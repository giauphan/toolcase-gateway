//! Pre-configured provider modules ready for 1-click installation.

use serde::{Deserialize, Serialize};

/// Catalog item representing a pre-configured provider module available for 1-click installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemCatalogModule {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    pub description: String,
    pub namespace: String,
    pub version: String,
    pub protocol: String,
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_schema: Option<serde_json::Value>,
}

fn catalog_item(
    id: &str,
    name: &str,
    description: &str,
    namespace: &str,
    capabilities: &[&str],
) -> SystemCatalogModule {
    SystemCatalogModule {
        id: id.to_string(),
        name: name.to_string(),
        short_name: None,
        description: description.to_string(),
        namespace: namespace.to_string(),
        version: "1.0.0".to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: capabilities.iter().map(ToString::to_string).collect(),
        icon: None,
        source: None,
        config_schema: None,
    }
}

fn text_field(key: &str, label: &str, placeholder: &str, desc: &str) -> serde_json::Value {
    serde_json::json!({
        "key": key,
        "label": label,
        "type": "text",
        "placeholder": placeholder,
        "description": desc,
    })
}

fn secret_field(key: &str, label: &str, placeholder: &str, desc: &str) -> serde_json::Value {
    serde_json::json!({
        "key": key,
        "label": label,
        "type": "password",
        "secret": true,
        "placeholder": placeholder,
        "description": desc,
    })
}

fn bool_field(key: &str, label: &str, default_val: bool, desc: &str) -> serde_json::Value {
    serde_json::json!({
        "key": key,
        "label": label,
        "type": "boolean",
        "default": default_val,
        "description": desc,
    })
}

fn config_schema(
    title: &str,
    module_id: &str,
    namespace: &str,
    har_endpoint: &str,
    har_hint: &str,
    fields: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "title": title,
        "module_id": module_id,
        "namespace": namespace,
        "har_endpoint": har_endpoint,
        "har_hint": har_hint,
        "fields": fields,
    })
}

/// Returns the available catalog of standard provider modules ready for installation.
pub fn get_system_module_catalog() -> Vec<SystemCatalogModule> {
    vec![
        catalog_item(
            "cohere-provider",
            "Cohere Provider Adapter",
            "Cohere Command-R & Embeddings provider adapter template stub",
            "/custom/cohere",
            &["chat_completions", "models"],
        ),
        catalog_item(
            "claude-adapter",
            "Anthropic Claude Adapter",
            "Anthropic Claude 3.5 Sonnet & Haiku bridge template stub",
            "/custom/claude",
            &["chat_completions", "models", "streaming"],
        ),
        catalog_item(
            "groq-fast",
            "Groq High-Speed LLM",
            "Ultra-low latency inference template stub via Groq LPU",
            "/custom/groq",
            &["chat_completions", "models"],
        ),
        catalog_item(
            "mistral-ai",
            "Mistral AI Provider",
            "Mistral Large, Codestral & Mixtral models template stub",
            "/custom/mistral",
            &["chat_completions", "models"],
        ),
        SystemCatalogModule {
            id: "local-echo".to_string(),
            name: "Local Echo Test Module".to_string(),
            short_name: None,
            description: "Diagnostic echo adapter for local routing testing".to_string(),
            namespace: "/custom/echo".to_string(),
            version: "1.0.0".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec!["chat_completions".to_string(), "models".to_string()],
            icon: None,
            source: Some("examples/modules/echo-provider".to_string()),
            config_schema: None,
        },
        catalog_item(
            "suite-test",
            "Suite Provider Group",
            "Bundled multi-provider group (chat completions, vision, models)",
            "/custom/suite-test",
            &["chat_completions", "vision", "models"],
        ),
        SystemCatalogModule {
            id: "tiktok-studio".to_string(),
            name: "TikTok Creator Studio Adapter".to_string(),
            short_name: Some("TikTok Creator Studio".to_string()),
            description: "TikTok Creator Studio video management, download extraction, and publishing adapter".to_string(),
            namespace: "/custom/tiktok".to_string(),
            version: "1.0.0".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec![
                "video_download".to_string(),
                "items_manage".to_string(),
                "publish".to_string(),
                "studio".to_string(),
            ],
            icon: Some("♬".to_string()),
            source: Some("https://github.com/giauphan/TikTok-Creator-Studio-Adapter.git".to_string()),
            config_schema: Some(config_schema(
                "TikTok Creator Studio Config & HAR Import",
                "tiktok-studio",
                "/custom/tiktok",
                "/custom/tiktok/v1/config/har",
                "Drop download-tiktok.har here or select file (.har, .json) to auto-extract Cookie and CSRF token",
                vec![
                    text_field("base_url", "TikTok Base URL", "https://www.tiktok.com", "Primary TikTok Creator Studio upstream URL"),
                    bool_field("enabled", "Module Status", true, "Enable or disable TikTok Creator Studio requests"),
                    secret_field("cookie", "Session Cookie", "Leave empty to keep existing cookie", "TikTok Creator Studio authenticated session cookie"),
                    secret_field("csrf_token", "CSRF Token (tt-csrf-token)", "Leave empty to keep existing CSRF token", "TikTok CSRF protection token extracted from x-tt-csrf-token or cookies"),
                ],
            )),
        },
        SystemCatalogModule {
            id: "facebook-suite".to_string(),
            name: "Meta Business Suite / Facebook Creator Studio Adapter".to_string(),
            short_name: Some("Meta Business Suite".to_string()),
            description: "Meta Business Suite / Facebook Creator Studio video management, rupload resumable upload, and Reel/Story publishing adapter".to_string(),
            namespace: "/custom/facebook".to_string(),
            version: "1.0.0".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec![
                "video_upload".to_string(),
                "rupload".to_string(),
                "reels".to_string(),
                "stories".to_string(),
                "publish".to_string(),
                "studio".to_string(),
            ],
            icon: Some("ⓕ".to_string()),
            source: None,
            config_schema: Some(config_schema(
                "Meta Business Suite Config & HAR Import",
                "facebook-suite",
                "/custom/facebook",
                "/custom/facebook/v1/config/har",
                "Drop download-facebook.har here or select file (.har, .json) to auto-extract credentials and Comet parameters",
                vec![
                    text_field("base_url", "Meta Business Base URL", "https://business.facebook.com", "Primary Meta Business Suite upstream URL"),
                    bool_field("enabled", "Module Status", true, "Enable or disable Meta Business Suite requests"),
                    secret_field("cookie", "Session Cookie", "Leave empty to keep existing cookie", "Facebook authenticated session cookie"),
                    secret_field("fb_dtsg", "CSRF Token (fb_dtsg)", "Leave empty to keep existing fb_dtsg token", "Comet anti-CSRF token extracted from HAR or page payload"),
                    text_field("user_id", "Facebook User ID (__user)", "1000...", "Authenticated Facebook User ID"),
                    text_field("actor_id", "Actor ID (av)", "1435...", "Active acting identity or Page ID"),
                    text_field("target_id", "Target Page ID", "1435...", "Default target Facebook Page for uploads and reels"),
                    text_field("business_id", "Business ID (__bid)", "2567...", "Associated Meta Business Account ID"),
                ],
            )),
        },
    ]
}
