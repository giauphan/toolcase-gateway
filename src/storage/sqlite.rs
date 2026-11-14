//! SQLite adapter implementing the `StorageRepository` port.

use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{DatabaseEngine, StorageError, StorageRepository, StoredAccount, StoredModule};
use crate::gateway::modules::PersonalManifest;

impl From<rusqlite::Error> for StorageError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Database(err.to_string())
    }
}

/// Settings key recording that the plaintext-credential scrub is still owed.
pub(crate) const PLAINTEXT_CLEANUP_MARKER: &str = "storage_plaintext_cleanup_pending";

/// Embedded SQLite storage implementation.
pub struct SqliteStorage {
    pub(crate) conn: Mutex<Connection>,
    db_path: PathBuf,
    encryption_key: String,
}

impl SqliteStorage {
    /// Opens or creates an embedded SQLite database, initializing schema and migrating legacy files.
    pub fn open(engine: DatabaseEngine, state_dir: &Path) -> Result<Self, StorageError> {
        let key = std::env::var("GW_ENCRYPTION_KEY").unwrap_or_default();
        Self::open_with_key(engine, state_dir, key)
    }

    /// Opens or creates an embedded SQLite database with an explicit master encryption key.
    pub fn open_with_key(
        engine: DatabaseEngine,
        state_dir: &Path,
        encryption_key: String,
    ) -> Result<Self, StorageError> {
        let db_path = match engine {
            DatabaseEngine::Sqlite(path) => {
                if path.to_string_lossy() == ":memory:" {
                    path
                } else {
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    path
                }
            }
            other => {
                return Err(StorageError::UnsupportedEngine(format!(
                    "Cannot initialize embedded SQLite with non-sqlite engine: {other:?}"
                )))
            }
        };

        let conn = if db_path.to_string_lossy() == ":memory:" {
            Connection::open_in_memory()?
        } else {
            Connection::open(&db_path)?
        };

        // Enable WAL mode and foreign keys for durability and concurrent performance
        let _ = conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA synchronous = NORMAL;",
        );

        let storage = Self {
            conn: Mutex::new(conn),
            db_path,
            encryption_key,
        };

        storage.init_schema()?;
        storage.migrate_legacy_json_files(state_dir)?;
        storage.encrypt_plaintext_credentials()?;

        Ok(storage)
    }

    /// Initializes tables and indexes if they do not exist.
    fn init_schema(&self) -> Result<(), StorageError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS accounts (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                key TEXT NOT NULL,
                label TEXT,
                is_active INTEGER NOT NULL DEFAULT 1,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS modules (
                id TEXT PRIMARY KEY,
                namespace TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                manifest_json TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS uninstalled_modules (
                id TEXT PRIMARY KEY,
                uninstalled_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_accounts_provider ON accounts(provider);
            CREATE INDEX IF NOT EXISTS idx_modules_namespace ON modules(namespace);",
        )?;
        Ok(())
    }

    /// Automatically migrates accounts, modules, and settings from legacy JSON files into SQLite on first run.
    fn migrate_legacy_json_files(&self, state_dir: &Path) -> Result<(), StorageError> {
        super::migration::migrate_legacy_json_files(self, state_dir)
    }

    /// Encrypts credentials that older builds stored as plaintext.
    ///
    /// Runs once per startup when a master key is configured, so production
    /// databases stop carrying readable secrets regardless of their history.
    pub fn encrypt_plaintext_credentials(&self) -> Result<usize, StorageError> {
        if self.encryption_key.trim().is_empty() {
            return Ok(0);
        }

        let cleanup_pending = self.get_setting(PLAINTEXT_CLEANUP_MARKER)?.as_deref() == Some("1");

        let mut conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let tx = conn.transaction()?;
        let pending = {
            let mut stmt = tx.prepare("SELECT id, key FROM accounts")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        let mut migrated = 0usize;
        for (id, stored) in pending {
            if stored.is_empty() || super::crypto::is_protected_secret(&stored) {
                continue;
            }
            let protected = super::crypto::protect_secret(&stored, &self.encryption_key);
            if protected == stored {
                continue;
            }
            tx.execute(
                "UPDATE accounts SET key = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
                params![protected, id],
            )?;
            migrated += 1;
        }
        if migrated > 0 {
            // Persist the intent in the same transaction: a crash before the
            // scrub completes must not let the next startup skip cleanup just
            // because every row already looks encrypted.
            tx.execute(
                "INSERT INTO settings (key, value, updated_at)
                 VALUES (?1, '1', CURRENT_TIMESTAMP)
                 ON CONFLICT(key) DO UPDATE SET value = '1', updated_at = CURRENT_TIMESTAMP",
                params![PLAINTEXT_CLEANUP_MARKER],
            )?;
        }
        tx.commit()?;

        if migrated > 0 || cleanup_pending {
            // WAL retains the pre-update pages until a checkpoint, and freed
            // pages can still hold the old bytes. Rewrite the database so no
            // readable credential survives on disk before startup completes.
            Self::scrub_plaintext_pages(&conn)?;
            conn.execute(
                "DELETE FROM settings WHERE key = ?1",
                params![PLAINTEXT_CLEANUP_MARKER],
            )?;
        }
        Ok(migrated)
    }

    /// Flushes and rewrites the database file so no plaintext page survives.
    fn scrub_plaintext_pages(conn: &Connection) -> Result<(), StorageError> {
        Self::checkpoint_truncate(conn)?;
        conn.execute_batch("VACUUM;")?;
        Self::checkpoint_truncate(conn)
    }

    /// Runs `wal_checkpoint(TRUNCATE)` and fails when SQLite reports it blocked.
    fn checkpoint_truncate(conn: &Connection) -> Result<(), StorageError> {
        for attempt in 0..5 {
            let busy = {
                let mut stmt = conn.prepare("PRAGMA wal_checkpoint(TRUNCATE)")?;
                stmt.query_row([], |row| row.get::<_, i64>(0))?
            };
            if busy == 0 {
                return Ok(());
            }
            if attempt < 4 {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        Err(StorageError::Database(
            "wal_checkpoint(TRUNCATE) was blocked; plaintext cleanup remains pending".to_string(),
        ))
    }

    /// Access the underlying path of the SQLite database.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}

impl StorageRepository for SqliteStorage {
    fn get_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            let val: String = row.get(0)?;
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    fn set_setting(&self, key: &str, value: &str) -> Result<(), StorageError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
            params![key, value],
        )?;
        Ok(())
    }

    fn get_accounts(&self) -> Result<Vec<StoredAccount>, StorageError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT id, provider, key, label, is_active FROM accounts ORDER BY updated_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let is_active_int: i32 = row.get(4)?;
            let raw_key: String = row.get(2)?;
            let decrypted_key =
                crate::storage::crypto::unprotect_secret(&raw_key, &self.encryption_key);
            Ok(StoredAccount {
                id: row.get(0)?,
                provider: row.get(1)?,
                key: decrypted_key,
                label: row.get(3)?,
                is_active: is_active_int != 0,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    fn save_accounts(&self, accounts: &[StoredAccount]) -> Result<(), StorageError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM accounts", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO accounts (id, provider, key, label, is_active, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)",
            )?;
            for acc in accounts {
                let stored_key =
                    crate::storage::crypto::protect_secret(&acc.key, &self.encryption_key);
                stmt.execute(params![
                    acc.id,
                    acc.provider,
                    stored_key,
                    acc.label,
                    if acc.is_active { 1 } else { 0 }
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn get_stored_modules(&self) -> Result<Vec<StoredModule>, StorageError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let mut stmt =
            conn.prepare("SELECT manifest_json, enabled FROM modules ORDER BY id ASC")?;
        let rows = stmt.query_map([], |row| {
            let json_str: String = row.get(0)?;
            let enabled_int: i32 = row.get(1)?;
            Ok((json_str, enabled_int != 0))
        })?;

        let mut list = Vec::new();
        for r in rows {
            let (json_str, enabled) = r?;
            if let Ok(manifest) = serde_json::from_str::<PersonalManifest>(&json_str) {
                list.push(StoredModule { manifest, enabled });
            }
        }
        Ok(list)
    }

    fn save_module(&self, manifest: &PersonalManifest) -> Result<(), StorageError> {
        let json_str = serde_json::to_string(manifest)?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO modules (id, namespace, name, manifest_json, enabled, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, CURRENT_TIMESTAMP)
             ON CONFLICT(id) DO UPDATE SET
                namespace = excluded.namespace,
                name = excluded.name,
                manifest_json = excluded.manifest_json,
                updated_at = CURRENT_TIMESTAMP",
            params![manifest.id, manifest.namespace, manifest.name, json_str],
        )?;
        let _ = conn.execute(
            "DELETE FROM uninstalled_modules WHERE id = ?1",
            params![manifest.id],
        );
        Ok(())
    }

    fn save_module_with_status(
        &self,
        manifest: &PersonalManifest,
        enabled: bool,
    ) -> Result<(), StorageError> {
        let json_str = serde_json::to_string(manifest)?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO modules (id, namespace, name, manifest_json, enabled, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)
             ON CONFLICT(id) DO UPDATE SET
                namespace = excluded.namespace,
                name = excluded.name,
                manifest_json = excluded.manifest_json,
                enabled = excluded.enabled,
                updated_at = CURRENT_TIMESTAMP",
            params![
                manifest.id,
                manifest.namespace,
                manifest.name,
                json_str,
                if enabled { 1 } else { 0 }
            ],
        )?;
        let _ = conn.execute(
            "DELETE FROM uninstalled_modules WHERE id = ?1",
            params![manifest.id],
        );
        Ok(())
    }

    fn set_module_enabled(&self, id: &str, enabled: bool) -> Result<(), StorageError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let affected = conn.execute(
            "UPDATE modules SET enabled = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            params![if enabled { 1 } else { 0 }, id],
        )?;
        if affected == 0 {
            Err(StorageError::NotFound(id.to_string()))
        } else {
            Ok(())
        }
    }

    fn remove_module(&self, id: &str) -> Result<(), StorageError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let _ = conn.execute(
            "INSERT OR REPLACE INTO uninstalled_modules (id) VALUES (?1)",
            params![id],
        );
        let affected = conn.execute("DELETE FROM modules WHERE id = ?1", params![id])?;
        if affected == 0 {
            Err(StorageError::NotFound(id.to_string()))
        } else {
            Ok(())
        }
    }

    fn get_uninstalled_modules(&self) -> Result<Vec<String>, StorageError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT id FROM uninstalled_modules")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    fn clear_module_uninstalled(&self, id: &str) -> Result<(), StorageError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let _ = conn.execute("DELETE FROM uninstalled_modules WHERE id = ?1", params![id]);
        Ok(())
    }
}
