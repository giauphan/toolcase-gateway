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

/// Runtime profile selected from `--mode` or `GATEWAY_ENV`/`GW_ENV`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    Development,
    Production,
}

impl Default for RuntimeMode {
    fn default() -> Self {
        if cfg!(debug_assertions) {
            Self::Development
        } else {
            Self::Production
        }
    }
}

impl RuntimeMode {
    pub fn from_value(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" => Ok(Self::default()),
            "dev" | "development" | "local" => Ok(Self::Development),
            "prod" | "production" | "release" => Ok(Self::Production),
            other => Err(format!(
                "invalid runtime mode '{other}': expected development or production"
            )),
        }
    }

    /// Resolve precedence: `--mode` > `GATEWAY_ENV` > `GW_ENV` > build profile.
    pub fn resolve(args: &[String]) -> Result<Self, String> {
        if let Some(position) = args.iter().position(|arg| arg == "--mode") {
            let value = args
                .get(position + 1)
                .ok_or_else(|| "--mode requires a value (development|production)".to_string())?;
            return Self::from_value(value);
        }
        if let Some(value) = args.iter().find_map(|arg| arg.strip_prefix("--mode=")) {
            return Self::from_value(value);
        }
        for key in ["GATEWAY_ENV", "GW_ENV"] {
            if let Ok(value) = env::var(key) {
                if !value.trim().is_empty() {
                    return Self::from_value(&value);
                }
            }
        }
        Ok(Self::default())
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
        }
    }

    pub fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }

    /// Publish the canonical mode so child modules (e.g. daily-news) inherit it.
    pub fn apply(self) {
        env::set_var("GATEWAY_ENV", self.as_str());
        env::set_var("GW_ENV", self.as_str());
    }
}

/// Enforce the production credential policy.
///
/// Production must not persist plaintext credentials, so startup fails closed
/// when no master key is configured. Legacy plaintext rows are re-encrypted by
/// the storage adapter once a key is available.
pub fn enforce_production_secret_policy(
    mode: RuntimeMode,
    encryption_key: &str,
) -> Result<(), String> {
    if mode.is_production() && encryption_key.trim().is_empty() {
        return Err(
            "production mode requires GW_ENCRYPTION_KEY; generate one with \
             `openssl rand -hex 32` and store it only in the environment"
                .to_string(),
        );
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::RuntimeMode;

    #[test]
    fn parses_mode_aliases_and_rejects_unknown() {
        assert_eq!(
            RuntimeMode::from_value("dev").expect("dev alias"),
            RuntimeMode::Development
        );
        assert_eq!(
            RuntimeMode::from_value("PRODUCTION").expect("production alias"),
            RuntimeMode::Production
        );
        assert_eq!(
            RuntimeMode::from_value("release").expect("release alias"),
            RuntimeMode::Production
        );
        assert_eq!(
            RuntimeMode::from_value("").expect("empty uses build default"),
            RuntimeMode::default()
        );
        assert!(RuntimeMode::from_value("staging").is_err());
    }

    #[test]
    fn cli_mode_overrides_build_default() {
        let args = vec![
            "toolcase-gateway".to_string(),
            "--mode".to_string(),
            "production".to_string(),
        ];
        assert_eq!(
            RuntimeMode::resolve(&args).expect("--mode value"),
            RuntimeMode::Production
        );

        let args = vec![
            "toolcase-gateway".to_string(),
            "--mode=development".to_string(),
        ];
        assert_eq!(
            RuntimeMode::resolve(&args).expect("--mode=value"),
            RuntimeMode::Development
        );
    }

    #[test]
    fn cli_mode_requires_value() {
        let args = vec!["toolcase-gateway".to_string(), "--mode".to_string()];
        assert!(RuntimeMode::resolve(&args).is_err());
    }

    #[test]
    fn production_fails_closed_without_encryption_key() {
        use super::enforce_production_secret_policy;

        assert!(
            enforce_production_secret_policy(RuntimeMode::Development, "").is_ok(),
            "development may run without a master key"
        );
        assert!(
            enforce_production_secret_policy(RuntimeMode::Production, "").is_err(),
            "production must fail closed without a master key"
        );
        assert!(
            enforce_production_secret_policy(RuntimeMode::Production, "   ").is_err(),
            "blank keys must not satisfy the production policy"
        );
        assert!(
            enforce_production_secret_policy(RuntimeMode::Production, "deadbeef").is_ok(),
            "production accepts a configured master key"
        );
    }
}
