//! Clean Architecture Storage Repository Port & Pluggable Database Abstraction.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod sqlite;

pub use sqlite::SqliteStorage;

/// Domain error occurring during storage repository operations.
#[derive(Debug)]
pub enum StorageError {
    Database(String),
    NotFound(String),
    Serialization(String),
    Io(std::io::Error),
    UnsupportedEngine(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(msg) => write!(f, "Storage database error: {msg}"),
            Self::NotFound(key) => write!(f, "Storage record not found: {key}"),
            Self::Serialization(msg) => write!(f, "Storage serialization error: {msg}"),
            Self::Io(e) => write!(f, "Storage I/O error: {e}"),
            Self::UnsupportedEngine(e) => write!(f, "Unsupported database engine: {e}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<rusqlite::Error> for StorageError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Database(err.to_string())
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err.to_string())
    }
}

impl From<std::io::Error> for StorageError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Pluggable database engine configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseEngine {
    /// Local zero-configuration embedded SQLite database.
    Sqlite(PathBuf),
    /// PostgreSQL connection URL string for enterprise deployments.
    Postgres(String),
    /// MySQL connection URL string for enterprise deployments.
    Mysql(String),
}

/// Parses a database connection URL or falls back to local SQLite in `state_dir`.
pub fn parse_database_url(
    url: Option<&str>,
    state_dir: &Path,
) -> Result<DatabaseEngine, StorageError> {
    match url {
        Some(u) if u.starts_with("sqlite://") || u.starts_with("sqlite:") => {
            let path_part = u
                .trim_start_matches("sqlite://")
                .trim_start_matches("sqlite:");
            let path = if path_part.is_empty() || path_part == ":memory:" {
                PathBuf::from(":memory:")
            } else {
                PathBuf::from(path_part)
            };
            Ok(DatabaseEngine::Sqlite(path))
        }
        Some(u) if u.starts_with("postgres://") || u.starts_with("postgresql://") => {
            Ok(DatabaseEngine::Postgres(u.to_string()))
        }
        Some(u) if u.starts_with("mysql://") => Ok(DatabaseEngine::Mysql(u.to_string())),
        Some(other) if !other.trim().is_empty() => Err(StorageError::UnsupportedEngine(format!(
            "Scheme in '{other}' not recognized. Supported: sqlite://, postgres://, mysql://"
        ))),
        _ => {
            let default_sqlite_path = state_dir.join("gateway.db");
            Ok(DatabaseEngine::Sqlite(default_sqlite_path))
        }
    }
}

/// Generic account record stored in the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredAccount {
    pub id: String,
    pub provider: String,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub is_active: bool,
}

/// Outward Port declaring abstract repository persistence operations.
/// Domain services interact solely with this trait, decoupling all database engines.
pub trait StorageRepository: Send + Sync {
    /// Gets a generic configuration or state string by key.
    fn get_setting(&self, key: &str) -> Result<Option<String>, StorageError>;

    /// Sets or updates a generic configuration or state string by key.
    fn set_setting(&self, key: &str, value: &str) -> Result<(), StorageError>;

    /// Retrieves all persisted accounts.
    fn get_accounts(&self) -> Result<Vec<StoredAccount>, StorageError>;

    /// Saves or updates the account collection atomically.
    fn save_accounts(&self, accounts: &[StoredAccount]) -> Result<(), StorageError>;

    /// Retrieves all installed personal module manifests.
    fn get_modules(&self) -> Result<Vec<crate::gateway::modules::PersonalManifest>, StorageError>;

    /// Persists or updates a personal module manifest.
    fn save_module(
        &self,
        manifest: &crate::gateway::modules::PersonalManifest,
    ) -> Result<(), StorageError>;

    /// Removes an installed personal module by id.
    fn remove_module(&self, id: &str) -> Result<(), StorageError>;
}
