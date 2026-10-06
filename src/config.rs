use std::env;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub(crate) fn env_or(key: &str, fallback: &str) -> String {
    env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| fallback.into())
}

pub(crate) fn env_or_duration_ms(key: &str, fallback: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(fallback)
}

pub(crate) fn resolve_env_file() -> PathBuf {
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

pub(crate) fn absolute_env_file(configured: PathBuf, current_dir: PathBuf) -> PathBuf {
    if configured.is_absolute() {
        configured
    } else {
        current_dir.join(configured)
    }
}

#[derive(Clone)]
pub(crate) struct Config {
    pub(crate) target_host: String,
    pub(crate) target_port: u16,
    pub(crate) fallbacks: Vec<String>,
    pub(crate) io_timeout: Option<Duration>,
    pub(crate) retry_base_delay_ms: u64,
    pub(crate) max_retry_delay_ms: u64,
    pub(crate) default_model: String,
    pub(crate) museai_base_url: String,
    pub(crate) museai_cookie: String,
    pub(crate) museai_ws_url: String,
    pub(crate) museai_access_token: String,
    pub(crate) museai_notary_token: String,
    pub(crate) museai_vm_id: String,
    pub(crate) museai_auto_cleanup_threads: bool,
    pub(crate) museai_thread_retention_secs: u64,
}

#[derive(Clone)]
pub(crate) struct ConfigStore {
    inner: Arc<RwLock<Config>>,
    pub(crate) env_file: PathBuf,
}

impl ConfigStore {
    pub(crate) fn new(config: Config, env_file: PathBuf) -> Self {
        Self {
            inner: Arc::new(RwLock::new(config)),
            env_file,
        }
    }

    pub(crate) fn snapshot(&self) -> Config {
        self.inner.read().unwrap().clone()
    }

    pub(crate) fn write_guard(&self) -> std::sync::RwLockWriteGuard<'_, Config> {
        self.inner.write().unwrap()
    }
}
