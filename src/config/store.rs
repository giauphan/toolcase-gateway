//! Thread-safe runtime application state and configuration store.

use super::persistent::{load_persistent_config, save_persistent_config, PersistentConfig};
use super::types::Config;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct ConfigStore {
    inner: Arc<RwLock<Config>>,
    pub env_file: PathBuf,
    pub config_file: PathBuf,
    pub jev_pool: Arc<crate::providers::jev::accounts::JevAccountPool>,
    admission: Arc<crate::runtime::admission::AdmissionController>,
    pub module_registry: Arc<RwLock<crate::gateway::modules::ModuleRegistry>>,
    pub storage: Arc<dyn crate::storage::StorageRepository>,
}

impl ConfigStore {
    pub fn new(config: Config, env_file: PathBuf) -> Self {
        let config_file = if env_file.file_name().map(|n| n == ".env").unwrap_or(false) {
            env_file.with_file_name("gateway_config.json")
        } else {
            env_file.with_extension("config.json")
        };
        Self::new_with_limits_and_config_file(
            config,
            env_file,
            config_file,
            256,
            crate::runtime::admission::DEFAULT_MAX_IN_FLIGHT_BYTES,
        )
    }

    pub fn new_with_limits(
        config: Config,
        env_file: PathBuf,
        max_connections: usize,
        max_in_flight_bytes: usize,
    ) -> Self {
        let config_file = if env_file.file_name().map(|n| n == ".env").unwrap_or(false) {
            env_file.with_file_name("gateway_config.json")
        } else {
            env_file.with_extension("config.json")
        };
        Self::new_with_limits_and_config_file(
            config,
            env_file,
            config_file,
            max_connections,
            max_in_flight_bytes,
        )
    }

    pub fn new_with_limits_and_config_file(
        config: Config,
        env_file: PathBuf,
        config_file: PathBuf,
        max_connections: usize,
        max_in_flight_bytes: usize,
    ) -> Self {
        let state_dir = resolve_state_dir(&config_file);
        let engine = resolve_database_engine(&config_file, &state_dir);
        let storage: Arc<dyn crate::storage::StorageRepository> =
            match crate::storage::SqliteStorage::open(engine, &state_dir) {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    eprintln!(
                        "warning: failed to open sqlite storage at {:?}: {e}; using memory fallback",
                        state_dir
                    );
                    Arc::new(
                        crate::storage::SqliteStorage::open(
                            crate::storage::DatabaseEngine::Sqlite(PathBuf::from(":memory:")),
                            &state_dir,
                        )
                        .expect("in-memory sqlite storage must initialize"),
                    )
                }
            };

        Self::new_with_limits_config_file_and_storage(
            config,
            env_file,
            config_file,
            max_connections,
            max_in_flight_bytes,
            storage,
        )
    }

    pub fn new_with_limits_config_file_and_storage(
        config: Config,
        env_file: PathBuf,
        config_file: PathBuf,
        max_connections: usize,
        max_in_flight_bytes: usize,
        storage: Arc<dyn crate::storage::StorageRepository>,
    ) -> Self {
        let persistent = load_persistent_config(&config_file);
        let pool = build_jev_account_pool(&config, persistent.as_ref());
        let state_dir = resolve_state_dir(&config_file);
        let reg = build_module_registry(storage.as_ref(), &state_dir, persistent.as_ref());

        Self {
            inner: Arc::new(RwLock::new(config)),
            env_file,
            config_file,
            jev_pool: pool,
            admission: Arc::new(crate::runtime::admission::AdmissionController::new(
                max_connections,
                max_in_flight_bytes,
            )),
            module_registry: Arc::new(RwLock::new(reg)),
            storage,
        }
    }

    pub fn storage(&self) -> Arc<dyn crate::storage::StorageRepository> {
        self.storage.clone()
    }

    pub fn with_storage(mut self, storage: Arc<dyn crate::storage::StorageRepository>) -> Self {
        self.storage = storage;
        self
    }

    pub fn module_registry(&self) -> Arc<RwLock<crate::gateway::modules::ModuleRegistry>> {
        self.module_registry.clone()
    }

    /// Automatically rescans `addons/` and `$GW_STATE_DIR/modules` on disk,
    /// registering and persisting any newly added modules.
    pub fn rescan_addons(&self) -> Vec<String> {
        let state_dir = resolve_state_dir(&self.config_file);
        let mut reg_guard = match self.module_registry.write() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let discovered =
            crate::gateway::installer::scan_and_register_modules(&mut reg_guard, &state_dir);
        drop(reg_guard);

        for manifest in &discovered {
            let _ = self.storage.save_module_with_status(manifest, true);
        }
        if !discovered.is_empty() {
            let _ = self.save_to_persistent_file();
        }
        discovered.into_iter().map(|m| m.id).collect()
    }

    pub fn snapshot(&self) -> Config {
        self.inner.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn jev_pool(&self) -> Arc<crate::providers::jev::accounts::JevAccountPool> {
        self.jev_pool.clone()
    }

    pub fn admission(&self) -> Arc<crate::runtime::admission::AdmissionController> {
        self.admission.clone()
    }

    pub fn write_guard(&self) -> std::sync::RwLockWriteGuard<'_, Config> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }

    pub fn add_jev_keys(&self, keys: &[String]) -> usize {
        let added = self.jev_pool.add_keys(keys);
        if added > 0 {
            let mut conf = self.inner.write().unwrap_or_else(|e| e.into_inner());
            conf.jev_api_keys = self.jev_pool.keys();
        }
        added
    }

    pub fn add_jev_accounts(&self, entries: &[(String, Option<String>)]) -> usize {
        let added = self.jev_pool.add_account_entries(entries);
        {
            let mut conf = self.inner.write().unwrap_or_else(|e| e.into_inner());
            conf.jev_api_keys = self.jev_pool.keys();
        }
        added
    }

    pub fn add_jev_openrouter_accounts(&self, entries: &[(String, Option<String>)]) -> usize {
        self.add_jev_openrouter_typed_accounts(entries, None)
    }

    pub fn add_jev_openrouter_typed_accounts(
        &self,
        entries: &[(String, Option<String>)],
        force_type: Option<bool>,
    ) -> usize {
        let added = self
            .jev_pool
            .add_openrouter_typed_entries(entries, force_type);
        {
            let mut conf = self.inner.write().unwrap_or_else(|e| e.into_inner());
            conf.jev_openrouter_accounts = self.jev_pool.openrouter_accounts();
        }
        added
    }

    pub fn save_to_persistent_file(&self) -> std::io::Result<()> {
        let mut persistent = PersistentConfig::from_config(&self.snapshot());
        persistent.jev_accounts = Some(self.jev_pool.accounts());
        persistent.jev_openrouter_accounts = Some(self.jev_pool.openrouter_accounts());
        let reg = self
            .module_registry
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut map = std::collections::HashMap::new();
        for m in reg.list_modules() {
            map.insert(
                m.id,
                m.status == crate::gateway::modules::ModuleStatus::Enabled,
            );
        }
        persistent.enabled_modules = Some(map);
        persistent.personal_modules = Some(reg.export_personal_manifests());
        persistent.module_positions = Some(reg.export_module_positions());
        persistent.module_configs = Some(reg.export_module_configs());
        drop(reg);
        save_persistent_config(&self.config_file, &persistent)
    }
}

fn resolve_base_accounts(
    config: &Config,
    persistent: Option<&PersistentConfig>,
) -> Vec<crate::providers::jev::accounts::JevAccount> {
    if !config.jev_labeled_accounts.is_empty() {
        config.jev_labeled_accounts.clone()
    } else {
        persistent
            .and_then(|p| p.jev_accounts.clone())
            .unwrap_or_default()
    }
}

fn resolve_openrouter_accounts(
    config: &Config,
    persistent: Option<&PersistentConfig>,
) -> Vec<crate::providers::jev::accounts::OpenRouterAccount> {
    let mut accounts = if !config.jev_openrouter_accounts.is_empty() {
        config.jev_openrouter_accounts.clone()
    } else {
        persistent
            .and_then(|p| p.jev_openrouter_accounts.clone())
            .unwrap_or_default()
    };

    let cfg_api_key = config.jev_openrouter_api_key.trim();
    if !cfg_api_key.is_empty() && !accounts.iter().any(|a| a.credential() == cfg_api_key) {
        accounts.push(crate::providers::jev::accounts::OpenRouterAccount {
            cookie: String::new(),
            api_key: cfg_api_key.to_string(),
            label: "openrouter-api-key".to_string(),
        });
    }

    let cfg_cookie = config.jev_openrouter_cookie.trim();
    if !cfg_cookie.is_empty() && !accounts.iter().any(|a| a.credential() == cfg_cookie) {
        accounts.push(crate::providers::jev::accounts::OpenRouterAccount {
            cookie: cfg_cookie.to_string(),
            api_key: String::new(),
            label: "openrouter-cookie".to_string(),
        });
    }

    accounts
}

fn build_jev_account_pool(
    config: &Config,
    persistent: Option<&PersistentConfig>,
) -> Arc<crate::providers::jev::accounts::JevAccountPool> {
    let base_accounts = resolve_base_accounts(config, persistent);
    let base_or_accounts = resolve_openrouter_accounts(config, persistent);
    let pool = Arc::new(
        crate::providers::jev::accounts::JevAccountPool::new_with_all(
            base_accounts,
            base_or_accounts,
            config.jev_credit_cooldown_secs,
        ),
    );
    pool.add_keys(&config.jev_api_keys);
    pool
}

fn resolve_state_dir(_config_file: &std::path::Path) -> PathBuf {
    crate::gateway::installer::get_state_dir()
}

fn resolve_database_engine(
    config_file: &std::path::Path,
    state_dir: &std::path::Path,
) -> crate::storage::DatabaseEngine {
    if let Some(url) = std::env::var("DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        let parsed = crate::storage::parse_database_url(Some(&url), state_dir).unwrap_or_else(|_| {
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

    if let Some(parent) = config_file
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
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

fn migrate_legacy_database(
    legacy: &std::path::Path,
    target: &std::path::Path,
) -> Result<(), String> {
    let Some(parent) = target.parent() else {
        return Err(format!("target database {:?} has no parent directory", target));
    };
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("create state dir {:?}: {error}", parent))?;
    if target.exists() {
        return Ok(());
    }

    let sequence = MIGRATION_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let staging = target.with_extension(format!(
        "db.migrating.{}.{}",
        std::process::id(),
        sequence
    ));
    if staging.exists() {
        std::fs::remove_file(&staging)
            .map_err(|error| format!("remove stale staging {:?}: {error}", staging))?;
    }

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
        return Err(format!("snapshot legacy database into {:?}: {error}", staging));
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

fn apply_stored_module_statuses(
    reg: &mut crate::gateway::modules::ModuleRegistry,
    stored_modules: &[crate::storage::StoredModule],
) {
    for sm in stored_modules {
        let status = if sm.enabled {
            crate::gateway::modules::ModuleStatus::Enabled
        } else {
            crate::gateway::modules::ModuleStatus::Disabled
        };
        let _ = reg.set_module_status(&sm.manifest.id, status);
    }
}

fn hydrate_stored_modules(
    reg: &mut crate::gateway::modules::ModuleRegistry,
    stored_modules: &[crate::storage::StoredModule],
) {
    for sm in stored_modules {
        if let Err(e) = reg.register_personal_module(sm.manifest.clone()) {
            eprintln!(
                "warning: skipping stored personal module '{}': {e}",
                sm.manifest.id
            );
        } else {
            let status = if sm.enabled {
                crate::gateway::modules::ModuleStatus::Enabled
            } else {
                crate::gateway::modules::ModuleStatus::Disabled
            };
            let _ = reg.set_module_status(&sm.manifest.id, status);
        }
    }
}

fn apply_persistent_fallback(
    reg: &mut crate::gateway::modules::ModuleRegistry,
    persistent: &PersistentConfig,
    stored_modules: &[crate::storage::StoredModule],
) {
    if let Some(ref personal) = persistent.personal_modules {
        for manifest in personal {
            if reg.get_module(&manifest.id).is_none() {
                if let Err(e) = reg.register_personal_module(manifest.clone()) {
                    eprintln!(
                        "warning: skipping persisted personal module '{}': {e}",
                        manifest.id
                    );
                }
            }
        }
    }
    if let Some(ref enabled_map) = persistent.enabled_modules {
        reg.apply_enablement_map(enabled_map);
    }
    // Re-apply SQLite stored module status to ensure SQLite remains authoritative for personal modules
    apply_stored_module_statuses(reg, stored_modules);

    if let Some(ref pos_map) = persistent.module_positions {
        reg.apply_module_positions(pos_map);
    }
    if let Some(ref cfg_map) = persistent.module_configs {
        reg.apply_module_configs(cfg_map);
    }
}

fn build_module_registry(
    storage: &dyn crate::storage::StorageRepository,
    state_dir: &std::path::Path,
    persistent: Option<&PersistentConfig>,
) -> crate::gateway::modules::ModuleRegistry {
    let mut reg = crate::gateway::modules::ModuleRegistry::new();
    let uninstalled = storage.get_uninstalled_modules().unwrap_or_default();
    reg.set_uninstalled(uninstalled);
    let stored_modules = storage.get_stored_modules().unwrap_or_default();

    // 1. Authoritative hydration from SQLite modules table
    hydrate_stored_modules(&mut reg, &stored_modules);

    // 2. Discover modules on disk (addons, modules dir)
    let discovered = crate::gateway::installer::scan_and_register_modules(&mut reg, state_dir);
    for manifest in &discovered {
        let _ = storage.save_module_with_status(manifest, true);
    }

    // 3. Fallback / supplementary configurations from persistent config (positions, configs, built-in enablement)
    if let Some(persistent) = persistent {
        apply_persistent_fallback(&mut reg, persistent, &stored_modules);
    }

    reg
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
