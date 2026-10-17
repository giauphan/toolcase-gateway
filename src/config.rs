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

#[derive(Clone)]
pub struct ConfigStore {
    inner: Arc<RwLock<Config>>,
    pub env_file: PathBuf,
    pub jev_pool: Arc<crate::providers::jev::accounts::JevAccountPool>,
    admission: Arc<crate::runtime::admission::AdmissionController>,
}

impl ConfigStore {
    pub fn new(config: Config, env_file: PathBuf) -> Self {
        Self::new_with_limits(
            config,
            env_file,
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
        let pool = Arc::new(crate::providers::jev::accounts::JevAccountPool::new(
            &config.jev_api_keys,
        ));
        Self {
            inner: Arc::new(RwLock::new(config)),
            env_file,
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
}
