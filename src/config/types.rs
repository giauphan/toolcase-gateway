//! Core domain configuration types and enums.

use std::time::Duration;

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

impl MuseApprovalMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Observe => "observe",
            Self::ScopedAuto => "on",
        }
    }
}

pub fn is_valid_muse_approval_mode(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "off" | "observe" | "on" | "scoped_auto" | "scoped-auto"
    )
}

pub fn parse_muse_approval_mode(value: &str) -> MuseApprovalMode {
    match value.trim().to_ascii_lowercase().as_str() {
        "observe" => MuseApprovalMode::Observe,
        "on" | "scoped_auto" | "scoped-auto" => MuseApprovalMode::ScopedAuto,
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
    /// Labeled accounts loaded from persistent storage.
    /// When non-empty, pool is initialized from these (with labels), then
    /// any extra keys from `jev_api_keys` that are not already present are
    /// added with auto-generated labels.
    pub jev_labeled_accounts: Vec<crate::providers::jev::accounts::JevAccount>,
    pub jev_base_url: String,
    pub jev_openrouter_base_url: String,
    pub jev_openrouter_api_key: String,
    pub jev_openrouter_cookie: String,
    pub jev_openrouter_model: String,
    pub jev_openrouter_fallback_models: Vec<String>,
    pub jev_openrouter_enabled: bool,
    pub jev_credit_cooldown_secs: u64,
    pub jev_openrouter_accounts: Vec<crate::providers::jev::accounts::OpenRouterAccount>,
    pub routing_mode: RoutingMode,
    pub jev_enabled: bool,
    pub jev_shadow_mode: bool,
    pub default_tier: ModelTier,
    pub muse_approval: MuseApprovalConfig,
    pub default_video_prompt: String,
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
            museai_thread_retention_secs: 172800,
            jev_api_keys: vec![],
            jev_labeled_accounts: vec![],
            jev_base_url: "https://jev-ai.org".into(),
            jev_openrouter_base_url: "https://openrouter.ai".into(),
            jev_openrouter_api_key: "".into(),
            jev_openrouter_cookie: "".into(),
            jev_openrouter_model: "typesafe/jev-1.13".into(),
            jev_openrouter_fallback_models: vec![],
            jev_openrouter_enabled: true,
            jev_credit_cooldown_secs: 300,
            jev_openrouter_accounts: vec![],
            routing_mode: RoutingMode::Existing,
            jev_enabled: false,
            jev_shadow_mode: true,
            default_tier: ModelTier::Strong,
            muse_approval: MuseApprovalConfig::default(),
            default_video_prompt:
                "A colorful butterfly landing gently on a blooming sunflower, macro video".into(),
        }
    }
}
