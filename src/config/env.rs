//! Environment variable parsing and file path resolution helpers.

use std::env;
use std::path::PathBuf;

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

pub fn parse_openrouter_api_keys(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn parse_fallback_models(val: &serde_json::Value) -> Vec<String> {
    match val {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        serde_json::Value::String(s) => s
            .split(&[',', '\n', '\r'][..])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

pub fn parse_jev_accounts(value: &str) -> Vec<crate::providers::jev::accounts::JevAccount> {
    let keys = parse_jev_api_keys(value);
    crate::providers::jev::accounts::JevAccountPool::new(&keys).accounts()
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
