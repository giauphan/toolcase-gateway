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
    if let Ok(configured) = env::var("GW_ENV_FILE") {
        let trimmed = configured.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            return if p.is_absolute() {
                p
            } else {
                env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(p)
            };
        }
    }

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let local_env = cwd.join(".env");
    if local_env.is_file() {
        return local_env;
    }

    let state_dir = crate::gateway::installer::get_state_dir();
    if let Some(parent) = state_dir.parent() {
        let parent_env = parent.join(".env");
        if parent_env.is_file() {
            return parent_env;
        }
    }

    local_env
}

pub fn absolute_env_file(configured: PathBuf, current_dir: PathBuf) -> PathBuf {
    if configured.is_absolute() {
        configured
    } else {
        current_dir.join(configured)
    }
}

pub fn resolve_config_file() -> PathBuf {
    if let Ok(configured) = env::var("GW_CONFIG_FILE") {
        let trimmed = configured.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            return if p.is_absolute() {
                p
            } else {
                env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(p)
            };
        }
    }

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let local_cfg = cwd.join("gateway_config.json");
    if local_cfg.is_file() {
        return local_cfg;
    }

    let state_dir = crate::gateway::installer::get_state_dir();
    if let Some(parent) = state_dir.parent() {
        let parent_cfg = parent.join("gateway_config.json");
        if parent_cfg.is_file() {
            return parent_cfg;
        }
    }
    let state_cfg = state_dir.join("gateway_config.json");
    if state_cfg.is_file() {
        return state_cfg;
    }

    local_cfg
}

pub fn absolute_config_file(configured: PathBuf, current_dir: PathBuf) -> PathBuf {
    if configured.is_absolute() {
        configured
    } else {
        current_dir.join(configured)
    }
}
