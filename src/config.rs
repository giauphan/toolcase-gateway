use std::env;
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

#[derive(Clone)]
pub(crate) struct Config {
    pub(crate) target_host: String,
    pub(crate) target_port: u16,
    pub(crate) fallbacks: Vec<String>,
    pub(crate) io_timeout: Option<Duration>,
    pub(crate) retry_base_delay_ms: u64,
    pub(crate) max_retry_delay_ms: u64,
    pub(crate) prism_base_url: String,
    pub(crate) prism_project_id: String,
    pub(crate) prism_cookie: String,
    pub(crate) prism_sandbox_token: String,
    pub(crate) prism_user_id: String,
    pub(crate) prism_default_model: String,
    pub(crate) prism_system_prompt: String,
}
