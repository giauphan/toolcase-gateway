use std::env;
use std::time::Duration;

pub(crate) fn env_or(key: &str, fallback: &str) -> String {
    env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| fallback.into())
}

pub(crate) struct Config {
    pub(crate) target_host: String,
    pub(crate) target_port: u16,
    pub(crate) fallbacks: Vec<String>,
    pub(crate) io_timeout: Option<Duration>,
}
