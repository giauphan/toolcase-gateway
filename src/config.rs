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
}

#[derive(Clone)]
pub struct ConfigStore {
    inner: Arc<RwLock<Config>>,
    pub env_file: PathBuf,
}

impl ConfigStore {
    pub fn new(config: Config, env_file: PathBuf) -> Self {
        Self {
            inner: Arc::new(RwLock::new(config)),
            env_file,
        }
    }

    pub fn snapshot(&self) -> Config {
        self.inner.read().unwrap().clone()
    }

    pub fn write_guard(&self) -> std::sync::RwLockWriteGuard<'_, Config> {
        self.inner.write().unwrap()
    }
}
