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
        let base_accounts = if !config.jev_labeled_accounts.is_empty() {
            config.jev_labeled_accounts.clone()
        } else if let Some(persistent) = load_persistent_config(&config_file) {
            persistent.jev_accounts.unwrap_or_default()
        } else {
            Vec::new()
        };

        let mut base_or_accounts = if !config.jev_openrouter_accounts.is_empty() {
            config.jev_openrouter_accounts.clone()
        } else if let Some(persistent) = load_persistent_config(&config_file) {
            persistent.jev_openrouter_accounts.unwrap_or_default()
        } else {
            Vec::new()
        };
        let cfg_api_key = config.jev_openrouter_api_key.trim();
        if !cfg_api_key.is_empty()
            && !base_or_accounts
                .iter()
                .any(|a| a.credential() == cfg_api_key)
        {
            base_or_accounts.push(crate::providers::jev::accounts::OpenRouterAccount {
                cookie: String::new(),
                api_key: cfg_api_key.to_string(),
                label: "openrouter-api-key".to_string(),
            });
        }
        let cfg_cookie = config.jev_openrouter_cookie.trim();
        if !cfg_cookie.is_empty()
            && !base_or_accounts
                .iter()
                .any(|a| a.credential() == cfg_cookie)
        {
            base_or_accounts.push(crate::providers::jev::accounts::OpenRouterAccount {
                cookie: cfg_cookie.to_string(),
                api_key: String::new(),
                label: "openrouter-cookie".to_string(),
            });
        }

        let pool = Arc::new(
            crate::providers::jev::accounts::JevAccountPool::new_with_all(
                base_accounts,
                base_or_accounts,
                config.jev_credit_cooldown_secs,
            ),
        );
        pool.add_keys(&config.jev_api_keys);

        let mut reg = crate::gateway::modules::ModuleRegistry::new();
        if let Some(persistent) = load_persistent_config(&config_file) {
            if let Some(ref personal) = persistent.personal_modules {
                for manifest in personal {
                    if let Err(e) = reg.register_personal_module(manifest.clone()) {
                        eprintln!(
                            "warning: skipping persisted personal module '{}': {e}",
                            manifest.id
                        );
                    }
                }
            }
            let state_dir = crate::gateway::installer::get_state_dir();
            crate::gateway::installer::scan_and_register_modules(&mut reg, &state_dir);
            if let Some(ref enabled_map) = persistent.enabled_modules {
                reg.apply_enablement_map(enabled_map);
            }
            if let Some(ref pos_map) = persistent.module_positions {
                reg.apply_module_positions(pos_map);
            }
            if let Some(ref cfg_map) = persistent.module_configs {
                reg.apply_module_configs(cfg_map);
            }
        } else {
            let state_dir = crate::gateway::installer::get_state_dir();
            crate::gateway::installer::scan_and_register_modules(&mut reg, &state_dir);
        }
        let module_registry = Arc::new(RwLock::new(reg));

        Self {
            inner: Arc::new(RwLock::new(config)),
            env_file,
            config_file,
            jev_pool: pool,
            admission: Arc::new(crate::runtime::admission::AdmissionController::new(
                max_connections,
                max_in_flight_bytes,
            )),
            module_registry,
        }
    }

    pub fn module_registry(&self) -> Arc<RwLock<crate::gateway::modules::ModuleRegistry>> {
        self.module_registry.clone()
    }

    pub fn snapshot(&self) -> Config {
        self.inner.read().unwrap().clone()
    }

    pub fn jev_pool(&self) -> Arc<crate::providers::jev::accounts::JevAccountPool> {
        self.jev_pool.clone()
    }

    pub fn admission(&self) -> Arc<crate::runtime::admission::AdmissionController> {
        self.admission.clone()
    }

    pub fn write_guard(&self) -> std::sync::RwLockWriteGuard<'_, Config> {
        self.inner.write().unwrap()
    }

    pub fn add_jev_keys(&self, keys: &[String]) -> usize {
        let added = self.jev_pool.add_keys(keys);
        if added > 0 {
            let mut conf = self.inner.write().unwrap();
            conf.jev_api_keys = self.jev_pool.keys();
        }
        added
    }

    pub fn add_jev_accounts(&self, entries: &[(String, Option<String>)]) -> usize {
        let added = self.jev_pool.add_account_entries(entries);
        {
            let mut conf = self.inner.write().unwrap();
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
            let mut conf = self.inner.write().unwrap();
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
