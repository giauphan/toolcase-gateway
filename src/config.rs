use std::env;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub fn env_or(key: &str, fallback: &str) -> String {
    env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| fallback.into())
}

pub fn parse_jev_api_keys(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn env_or_duration_ms(key: &str, fallback: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(fallback)
}

pub fn resolve_env_file() -> PathBuf {
    let configured = env::var("GW_ENV_FILE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".env"));
    absolute_env_file(
        configured,
        env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    )
}

pub fn absolute_env_file(configured: PathBuf, current_dir: PathBuf) -> PathBuf {
    if configured.is_absolute() {
        configured
    } else {
        current_dir.join(configured)
    }
}

pub fn resolve_config_file() -> PathBuf {
    let configured = env::var("GW_CONFIG_FILE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("gateway_config.json"));
    absolute_config_file(
        configured,
        env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    )
}

pub fn absolute_config_file(configured: PathBuf, current_dir: PathBuf) -> PathBuf {
    if configured.is_absolute() {
        configured
    } else {
        current_dir.join(configured)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoutingMode {
    Existing,
    RoundRobin,
    Rules,
    Jev,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelTier {
    Fast,
    Strong,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MuseApprovalMode {
    Off,
    Observe,
    ScopedAuto,
}

pub fn parse_muse_approval_mode(value: &str) -> MuseApprovalMode {
    match value.trim().to_ascii_lowercase().as_str() {
        "observe" => MuseApprovalMode::Observe,
        "on" | "scoped_auto" => MuseApprovalMode::ScopedAuto,
        _ => MuseApprovalMode::Off,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MuseApprovalConfig {
    pub mode: MuseApprovalMode,
    pub allowed_types: Vec<String>,
    pub allowed_destinations: Vec<String>,
    pub prefer_allow_once: bool,
    pub allow_always: bool,
    pub max_decisions_per_job: usize,
    pub approval_timeout_secs: u64,
}

impl Default for MuseApprovalConfig {
    fn default() -> Self {
        Self {
            mode: MuseApprovalMode::Off,
            allowed_types: vec!["network".to_string()],
            allowed_destinations: Vec::new(),
            prefer_allow_once: true,
            allow_always: false,
            max_decisions_per_job: 3,
            approval_timeout_secs: 30,
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub target_host: String,
    pub target_port: u16,
    pub fallbacks: Vec<String>,
    pub io_timeout: Option<Duration>,
    pub retry_base_delay_ms: u64,
    pub max_retry_delay_ms: u64,
    pub default_model: String,
    pub museai_base_url: String,
    pub museai_cookie: String,
    pub museai_ws_url: String,
    pub museai_access_token: String,
    pub museai_notary_token: String,
    pub museai_vm_id: String,
    pub museai_auto_cleanup_threads: bool,
    pub museai_thread_retention_secs: u64,
    pub jev_api_keys: Vec<String>,
    pub jev_base_url: String,
    pub routing_mode: RoutingMode,
    pub jev_enabled: bool,
    pub jev_shadow_mode: bool,
    pub default_tier: ModelTier,
    pub muse_approval: MuseApprovalConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            target_host: "127.0.0.1".into(),
            target_port: 20128,
            fallbacks: vec!["fail-try".into()],
            io_timeout: Some(Duration::from_secs(120)),
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 5000,
            default_model: "".into(),
            museai_base_url: "https://muse.ai".into(),
            museai_cookie: "".into(),
            museai_ws_url: "wss://hatch.metaaivm.com/v1/noise".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 86400,
            jev_api_keys: vec![],
            jev_base_url: "https://jev-ai.org".into(),
            routing_mode: RoutingMode::Existing,
            jev_enabled: false,
            jev_shadow_mode: true,
            default_tier: ModelTier::Strong,
            muse_approval: MuseApprovalConfig::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct PersistentConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallbacks: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_api_keys: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_base_url: Option<String>,
}

impl PersistentConfig {
    pub fn from_config(config: &Config) -> Self {
        Self {
            target_host: Some(config.target_host.clone()),
            target_port: Some(config.target_port),
            fallbacks: Some(config.fallbacks.clone()),
            default_model: Some(config.default_model.clone()),
            io_timeout_secs: config.io_timeout.map(|d| d.as_secs()),
            jev_api_keys: Some(config.jev_api_keys.clone()),
            jev_base_url: Some(config.jev_base_url.clone()),
        }
    }

    pub fn apply_to(&self, config: &mut Config) {
        if let Some(ref h) = self.target_host {
            config.target_host = h.clone();
        }
        if let Some(p) = self.target_port {
            config.target_port = p;
        }
        if let Some(ref f) = self.fallbacks {
            config.fallbacks = f.clone();
        }
        if let Some(ref m) = self.default_model {
            config.default_model = m.clone();
        }
        if let Some(secs) = self.io_timeout_secs {
            config.io_timeout = (secs > 0).then(|| Duration::from_secs(secs));
        }
        if let Some(ref keys) = self.jev_api_keys {
            config.jev_api_keys = keys.clone();
        }
        if let Some(ref url) = self.jev_base_url {
            config.jev_base_url = url.clone();
        }
    }
}

pub fn load_persistent_config(path: &std::path::Path) -> Option<PersistentConfig> {
    if !path.exists() {
        return None;
    }
    let data = std::fs::read(path).ok()?;
    serde_json::from_slice(&data).ok()
}

pub fn save_persistent_config(
    path: &std::path::Path,
    config: &PersistentConfig,
) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    if !parent.exists() {
        std::fs::create_dir_all(parent).ok();
    }
    let tmp = parent.join(format!(
        ".{}~tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&tmp, &json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[derive(Clone)]
pub struct ConfigStore {
    inner: Arc<RwLock<Config>>,
    pub env_file: PathBuf,
    pub config_file: PathBuf,
    pub jev_pool: Arc<crate::providers::jev::accounts::JevAccountPool>,
    admission: Arc<crate::runtime::admission::AdmissionController>,
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
        let pool = Arc::new(crate::providers::jev::accounts::JevAccountPool::new(
            &config.jev_api_keys,
        ));
        Self {
            inner: Arc::new(RwLock::new(config)),
            env_file,
            config_file,
            jev_pool: pool,
            admission: Arc::new(crate::runtime::admission::AdmissionController::new(
                max_connections,
                max_in_flight_bytes,
            )),
        }
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

    pub fn save_to_persistent_file(&self) -> std::io::Result<()> {
        let persistent = PersistentConfig::from_config(&self.snapshot());
        save_persistent_config(&self.config_file, &persistent)
    }
}
