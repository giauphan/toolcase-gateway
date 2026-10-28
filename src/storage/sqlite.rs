//! SQLite adapter implementing the `StorageRepository` port.

use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{DatabaseEngine, StorageError, StorageRepository, StoredAccount};
use crate::gateway::modules::PersonalManifest;

/// Embedded SQLite storage implementation.
pub struct SqliteStorage {
    conn: Mutex<Connection>,
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

            CREATE INDEX IF NOT EXISTS idx_accounts_provider ON accounts(provider);
            CREATE INDEX IF NOT EXISTS idx_modules_namespace ON modules(namespace);",
        )?;
        Ok(())
    }

    /// Automatically migrates accounts and settings from legacy JSON files into SQLite on first run.
    fn migrate_legacy_json_files(&self, state_dir: &Path) -> Result<(), StorageError> {
        self.migrate_legacy_accounts(state_dir)?;
        self.migrate_legacy_setting(state_dir, "gateway_config.json", "gateway_config")?;
        self.migrate_legacy_setting(state_dir, "muse_config.json", "muse_config")?;
        Ok(())
    }

    fn migrate_legacy_accounts(&self, state_dir: &Path) -> Result<(), StorageError> {
        let accounts_file = state_dir.join("accounts.json");
        if !accounts_file.is_file() || !self.get_accounts()?.is_empty() {
            return Ok(());
        }
        let Ok(content) = fs::read_to_string(&accounts_file) else {
            return Ok(());
        };
        let to_migrate = parse_legacy_accounts(&content);
        if !to_migrate.is_empty() {
            let _ = self.save_accounts(&to_migrate);
        }
        Ok(())
    }

    fn migrate_legacy_setting(
        &self,
        state_dir: &Path,
        file_name: &str,
        setting_key: &str,
    ) -> Result<(), StorageError> {
        let file_path = state_dir.join(file_name);
        if file_path.is_file() && self.get_setting(setting_key)?.is_none() {
            if let Ok(content) = fs::read_to_string(&file_path) {
                let _ = self.set_setting(setting_key, &content);
            }
        }
        Ok(())
    }

    /// Access the underlying path of the SQLite database.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}

fn parse_legacy_accounts(content: &str) -> Vec<StoredAccount> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let Some(arr) = val.get("accounts").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    arr.iter()
        .enumerate()
        .filter_map(|(idx, item)| {
            let key = item.get("key").and_then(|v| v.as_str())?;
            if key.is_empty() {
                return None;
            }
            Some(StoredAccount {
                id: item
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("acc_{idx}")),
                provider: item
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or("openrouter")
                    .to_string(),
                key: key.to_string(),
                label: item
                    .get("label")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                is_active: true,
            })
        })
        .collect()
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

    fn get_modules(&self) -> Result<Vec<PersonalManifest>, StorageError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT manifest_json FROM modules ORDER BY id ASC")?;
        let rows = stmt.query_map([], |row| {
            let json_str: String = row.get(0)?;
            Ok(json_str)
        })?;

        let mut list = Vec::new();
        for r in rows {
            let json_str = r?;
            if let Ok(manifest) = serde_json::from_str::<PersonalManifest>(&json_str) {
                list.push(manifest);
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
        Ok(())
    }

    fn remove_module(&self, id: &str) -> Result<(), StorageError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let affected = conn.execute("DELETE FROM modules WHERE id = ?1", params![id])?;
        if affected == 0 {
            Err(StorageError::NotFound(id.to_string()))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_storage_lifecycle_and_migration() {
        let temp_dir = std::env::temp_dir().join(format!("test_sqlite_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        // 1. Create a legacy accounts.json file
        let legacy_accounts = temp_dir.join("accounts.json");
        fs::write(
            &legacy_accounts,
            r#"{"accounts": [{"id": "acc_1", "provider": "openrouter", "key": "sk-or-v1-test", "label": "Dev"}]}"#,
        )
        .unwrap();

        // 2. Open SQLite storage
        let storage =
            SqliteStorage::open(DatabaseEngine::Sqlite(PathBuf::from(":memory:")), &temp_dir)
                .unwrap();

        // 3. Test settings roundtrip
        storage
            .set_setting("gateway_config", r#"{"listen_port": 8080}"#)
            .unwrap();
        let cfg = storage.get_setting("gateway_config").unwrap();
        assert_eq!(cfg.as_deref(), Some(r#"{"listen_port": 8080}"#));

        // 4. Test module storage
        let manifest = PersonalManifest {
            id: "my-tool".into(),
            name: "My Tool".into(),
            version: "1.0.0".into(),
            namespace: "/custom/tool".into(),
            entrypoint: "/bin/echo".into(),
            protocol: "stdio-stream".into(),
            capabilities: vec!["chat".into()],
            config_schema: None,
            permissions: vec![],
            group: None,
        };
        storage.save_module(&manifest).unwrap();
        let modules = storage.get_modules().unwrap();
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].id, "my-tool");

        // 5. Test module removal
        storage.remove_module("my-tool").unwrap();
        assert!(storage.get_modules().unwrap().is_empty());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_sqlite_account_credential_encryption_and_decryption() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_sqlite_enc_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        let master_key = "test-master-encryption-key-999".to_string();
        let storage = SqliteStorage::open_with_key(
            DatabaseEngine::Sqlite(PathBuf::from(":memory:")),
            &temp_dir,
            master_key,
        )
        .unwrap();

        let acc = StoredAccount {
            id: "acc_enc_1".into(),
            provider: "openrouter".into(),
            key: "sk-or-v1-my-secret-credential".into(),
            label: Some("Encrypted Account".into()),
            is_active: true,
        };

        storage.save_accounts(&[acc]).unwrap();

        // Verify stored in DB as enc:v1:
        {
            let conn = storage.conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT key FROM accounts WHERE id = 'acc_enc_1'")
                .unwrap();
            let raw_key: String = stmt.query_row([], |row| row.get(0)).unwrap();
            assert!(raw_key.starts_with("enc:v1:"));
            assert!(!raw_key.contains("sk-or-v1-my-secret-credential"));
        }

        // Verify decrypted in memory via get_accounts()
        let retrieved = storage.get_accounts().unwrap();
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].key, "sk-or-v1-my-secret-credential");

        // Verify legacy unencrypted row compatibility
        {
            let conn = storage.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, provider, key, label, is_active)
                 VALUES ('legacy_acc', 'jev', 'sk-jev-legacy-plain', 'Legacy Plain', 1)",
                [],
            )
            .unwrap();
        }

        let all_accs = storage.get_accounts().unwrap();
        assert_eq!(all_accs.len(), 2);
        let legacy = all_accs.iter().find(|a| a.id == "legacy_acc").unwrap();
        assert_eq!(legacy.key, "sk-jev-legacy-plain");

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
