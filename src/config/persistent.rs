//! Persistent JSON configuration model, serialization, and disk storage.

use super::types::{parse_muse_approval_mode, Config};
use std::time::Duration;

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
    pub jev_accounts: Option<Vec<crate::providers::jev::accounts::JevAccount>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_openrouter_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_openrouter_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_openrouter_cookie: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_openrouter_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_openrouter_fallback_models: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_openrouter_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_credit_cooldown_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_openrouter_accounts: Option<Vec<crate::providers::jev::accounts::OpenRouterAccount>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub museai_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub museai_cookie: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub museai_ws_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub museai_access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub museai_notary_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub museai_vm_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_video_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muse_approval_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muse_approval_allowed_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muse_approval_allowed_destinations: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muse_approval_prefer_allow_once: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muse_approval_allow_always: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muse_approval_max_decisions: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muse_approval_timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled_modules: Option<std::collections::HashMap<String, bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub personal_modules: Option<Vec<crate::gateway::modules::PersonalManifest>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_positions:
        Option<std::collections::HashMap<String, crate::gateway::modules::ModuleUiPosition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_configs:
        Option<std::collections::HashMap<String, serde_json::Map<String, serde_json::Value>>>,
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
            jev_accounts: None,
            jev_base_url: Some(config.jev_base_url.clone()),
            jev_openrouter_base_url: (!config.jev_openrouter_base_url.is_empty())
                .then(|| config.jev_openrouter_base_url.clone()),
            jev_openrouter_api_key: (!config.jev_openrouter_api_key.is_empty())
                .then(|| config.jev_openrouter_api_key.clone()),
            jev_openrouter_cookie: (!config.jev_openrouter_cookie.is_empty())
                .then(|| config.jev_openrouter_cookie.clone()),
            jev_openrouter_model: (!config.jev_openrouter_model.is_empty())
                .then(|| config.jev_openrouter_model.clone()),
            jev_openrouter_fallback_models: (!config.jev_openrouter_fallback_models.is_empty())
                .then(|| config.jev_openrouter_fallback_models.clone()),
            jev_openrouter_enabled: Some(config.jev_openrouter_enabled),
            jev_credit_cooldown_secs: Some(config.jev_credit_cooldown_secs),
            jev_openrouter_accounts: None,
            museai_base_url: (!config.museai_base_url.is_empty())
                .then(|| config.museai_base_url.clone()),
            museai_cookie: (!config.museai_cookie.is_empty()).then(|| config.museai_cookie.clone()),
            museai_ws_url: (!config.museai_ws_url.is_empty()).then(|| config.museai_ws_url.clone()),
            museai_access_token: (!config.museai_access_token.is_empty())
                .then(|| config.museai_access_token.clone()),
            museai_notary_token: (!config.museai_notary_token.is_empty())
                .then(|| config.museai_notary_token.clone()),
            museai_vm_id: (!config.museai_vm_id.is_empty()).then(|| config.museai_vm_id.clone()),
            default_video_prompt: (!config.default_video_prompt.is_empty())
                .then(|| config.default_video_prompt.clone()),
            muse_approval_mode: Some(config.muse_approval.mode.as_str().to_string()),
            muse_approval_allowed_types: Some(config.muse_approval.allowed_types.clone()),
            muse_approval_allowed_destinations: Some(
                config.muse_approval.allowed_destinations.clone(),
            ),
            muse_approval_prefer_allow_once: Some(config.muse_approval.prefer_allow_once),
            muse_approval_allow_always: Some(config.muse_approval.allow_always),
            muse_approval_max_decisions: Some(config.muse_approval.max_decisions_per_job),
            muse_approval_timeout_secs: Some(config.muse_approval.approval_timeout_secs),
            enabled_modules: None,
            personal_modules: None,
            module_positions: None,
            module_configs: None,
        }
    }

    pub fn apply_to(&self, config: &mut Config) {
        self.apply_proxy_to(config);
        self.apply_jev_to(config);
        self.apply_muse_to(config);
    }

    fn apply_proxy_to(&self, config: &mut Config) {
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
    }

    fn apply_jev_to(&self, config: &mut Config) {
        if let Some(ref accounts) = self.jev_accounts {
            if !accounts.is_empty() {
                config.jev_labeled_accounts = accounts.clone();
                let stored: std::collections::HashSet<&str> =
                    accounts.iter().map(|a| a.api_key.as_str()).collect();
                let extra: Vec<String> = config
                    .jev_api_keys
                    .iter()
                    .filter(|k| !stored.contains(k.as_str()))
                    .cloned()
                    .collect();
                let mut merged: Vec<String> = accounts.iter().map(|a| a.api_key.clone()).collect();
                merged.extend(extra);
                config.jev_api_keys = merged;
            }
        } else if let Some(ref keys) = self.jev_api_keys {
            config.jev_api_keys = keys.clone();
        }
        if let Some(ref url) = self.jev_base_url {
            config.jev_base_url = url.clone();
        }
        if let Some(ref url) = self.jev_openrouter_base_url {
            config.jev_openrouter_base_url = url.clone();
        }
        if let Some(ref key) = self.jev_openrouter_api_key {
            config.jev_openrouter_api_key = key.clone();
        }
        if let Some(ref cookie) = self.jev_openrouter_cookie {
            config.jev_openrouter_cookie = cookie.clone();
        }
        if let Some(ref model) = self.jev_openrouter_model {
            config.jev_openrouter_model = model.clone();
        }
        if let Some(ref f) = self.jev_openrouter_fallback_models {
            config.jev_openrouter_fallback_models = f.clone();
        }
        if let Some(enabled) = self.jev_openrouter_enabled {
            config.jev_openrouter_enabled = enabled;
        }
        if let Some(secs) = self.jev_credit_cooldown_secs {
            config.jev_credit_cooldown_secs = secs;
        }
        if let Some(ref or_accounts) = self.jev_openrouter_accounts {
            if !or_accounts.is_empty() {
                config.jev_openrouter_accounts = or_accounts.clone();
            }
        }
    }

    fn apply_muse_to(&self, config: &mut Config) {
        if let Some(ref url) = self.museai_base_url {
            config.museai_base_url = url.clone();
        }
        if let Some(ref cookie) = self.museai_cookie {
            config.museai_cookie = cookie.clone();
        }
        if let Some(ref ws) = self.museai_ws_url {
            config.museai_ws_url = ws.clone();
        }
        if let Some(ref token) = self.museai_access_token {
            config.museai_access_token = token.clone();
        }
        if let Some(ref token) = self.museai_notary_token {
            config.museai_notary_token = token.clone();
        }
        if let Some(ref vm) = self.museai_vm_id {
            config.museai_vm_id = vm.clone();
        }
        if let Some(ref prompt) = self.default_video_prompt {
            config.default_video_prompt = prompt.clone();
        }
        if let Some(ref mode) = self.muse_approval_mode {
            config.muse_approval.mode = parse_muse_approval_mode(mode);
        }
        if let Some(ref types) = self.muse_approval_allowed_types {
            config.muse_approval.allowed_types = types.clone();
        }
        if let Some(ref dests) = self.muse_approval_allowed_destinations {
            config.muse_approval.allowed_destinations = dests.clone();
        }
        if let Some(prefer) = self.muse_approval_prefer_allow_once {
            config.muse_approval.prefer_allow_once = prefer;
        }
        if let Some(always) = self.muse_approval_allow_always {
            config.muse_approval.allow_always = always;
        }
        if let Some(max) = self.muse_approval_max_decisions {
            config.muse_approval.max_decisions_per_job = max;
        }
        if let Some(timeout) = self.muse_approval_timeout_secs {
            config.muse_approval.approval_timeout_secs = timeout;
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
