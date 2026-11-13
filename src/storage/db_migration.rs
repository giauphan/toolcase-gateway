//! SQLite database location discovery and legacy database migration.
//!
//! Infrastructure concern: owns the SQLite driver and filesystem access so the
//! configuration/application layer only receives the resolved engine.

use std::path::PathBuf;

pub(crate) fn resolve_database_engine(
    config_file: &std::path::Path,
    state_dir: &std::path::Path,
) -> crate::storage::DatabaseEngine {
    if let Some(url) = std::env::var("DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        let parsed =
            crate::storage::parse_database_url(Some(&url), state_dir).unwrap_or_else(|_| {
                crate::storage::DatabaseEngine::Sqlite(state_dir.join("gateway.db"))
            });
        if let (Some(path_part), crate::storage::DatabaseEngine::Sqlite(target)) =
            (relative_sqlite_path(&url), &parsed)
        {
            let legacy = PathBuf::from(path_part);
            if legacy.as_path() != target.as_path() && legacy.is_file() && !target.exists() {
                match migrate_legacy_database(&legacy, target) {
                    Ok(()) => {}
                    Err(error) => {
                        eprintln!(
                            "warning: cannot migrate explicit relative database {:?} to {:?}: {error}; continuing with legacy database",
                            legacy, target
                        );
                        return crate::storage::DatabaseEngine::Sqlite(legacy);
                    }
                }
            }
        }
        return parsed;
    }

    let parent = config_file
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let legacy = parent.join("gateway.db");
    let target = state_dir.join("gateway.db");
    if legacy != target && legacy.is_file() && !target.exists() {
        match migrate_legacy_database(&legacy, &target) {
            Ok(()) => {}
            Err(error) => {
                eprintln!(
                    "warning: cannot migrate legacy database {:?} to {:?}: {error}; continuing with legacy database",
                    legacy, target
                );
                return crate::storage::DatabaseEngine::Sqlite(legacy);
            }
        }
    }

    crate::storage::parse_database_url(None, state_dir)
        .unwrap_or_else(|_| crate::storage::DatabaseEngine::Sqlite(state_dir.join("gateway.db")))
}

fn relative_sqlite_path(url: &str) -> Option<&str> {
    let path_part = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))?;
    if path_part.is_empty() || path_part == ":memory:" {
        return None;
    }
    std::path::Path::new(path_part)
        .is_relative()
        .then_some(path_part)
}

static MIGRATION_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn create_private_staging_file(path: &std::path::Path) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("create staging database {:?}: {error}", path))?;
    drop(file);
    Ok(())
}

fn migrate_legacy_database(
    legacy: &std::path::Path,
    target: &std::path::Path,
) -> Result<(), String> {
    let Some(parent) = target.parent() else {
        return Err(format!(
            "target database {:?} has no parent directory",
            target
        ));
    };
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("create state dir {:?}: {error}", parent))?;
    if target.exists() {
        return Ok(());
    }

    let sequence = MIGRATION_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let staging =
        target.with_extension(format!("db.migrating.{}.{}", std::process::id(), sequence));
    if staging.exists() {
        std::fs::remove_file(&staging)
            .map_err(|error| format!("remove stale staging {:?}: {error}", staging))?;
    }
    create_private_staging_file(&staging)?;

    let connection = rusqlite::Connection::open(legacy)
        .map_err(|error| format!("open legacy database {:?}: {error}", legacy))?;
    let staging_path = staging.to_string_lossy().into_owned();
    if let Err(error) = connection.execute("VACUUM INTO ?1", rusqlite::params![staging_path]) {
        if let Err(remove_error) = std::fs::remove_file(&staging) {
            eprintln!(
                "warning: cannot remove incomplete migration staging file {:?}: {remove_error}",
                staging
            );
        }
        return Err(format!(
            "snapshot legacy database into {:?}: {error}",
            staging
        ));
    }
    drop(connection);

    // Atomic publish: hard_link fails if another initializer already published the
    // database, so a slower migrator can never replace the active file.
    let publish_result = match std::fs::hard_link(&staging, target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(format!(
            "publish migrated database {:?} -> {:?}: {error}",
            staging, target
        )),
    };
    if let Err(remove_error) = std::fs::remove_file(&staging) {
        eprintln!(
            "warning: cannot remove migration staging file {:?}: {remove_error}",
            staging
        );
    }
    publish_result
}

#[cfg(test)]
mod tests {
    use super::migrate_legacy_database;

    #[test]
    fn migrates_legacy_database_into_state_dir() {
        let base = std::env::temp_dir().join(format!("gw_migrate_{}", uuid::Uuid::new_v4()));
        let project = base.join("project");
        let state = base.join("state");
        std::fs::create_dir_all(&project).expect("create project dir");
        let legacy = project.join("gateway.db");
        {
            let connection = rusqlite::Connection::open(&legacy).expect("open legacy db");
            connection
                .execute_batch(
                    "CREATE TABLE legacy_marker (id TEXT PRIMARY KEY); \
                     INSERT INTO legacy_marker VALUES ('kept');",
                )
                .expect("seed legacy db");
        }

        migrate_legacy_database(&legacy, &state.join("gateway.db"))
            .expect("migrate legacy database");

        let migrated = state.join("gateway.db");
        assert!(migrated.is_file(), "legacy db must be migrated");
        let connection = rusqlite::Connection::open(&migrated).expect("open migrated db");
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM legacy_marker", [], |row| row.get(0))
            .expect("count migrated rows");
        assert_eq!(count, 1);

        if let Err(error) = std::fs::remove_dir_all(&base) {
            eprintln!(
                "warning: cannot remove migration test dir {:?}: {error}",
                base
            );
        }
    }
}
