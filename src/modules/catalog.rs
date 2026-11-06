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

#[allow(clippy::too_many_arguments)]
fn rich_catalog_item(
    id: &str,
    name: &str,
    short_name: Option<&str>,
    description: &str,
    namespace: &str,
    capabilities: &[&str],
    icon: Option<&str>,
    source: Option<&str>,
    config_schema: Option<serde_json::Value>,
) -> SystemCatalogModule {
    SystemCatalogModule {
        id: id.to_string(),
        name: name.to_string(),
        short_name: short_name.map(ToString::to_string),
        description: description.to_string(),
        namespace: namespace.to_string(),
        version: "1.0.0".to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: capabilities.iter().map(ToString::to_string).collect(),
        icon: icon.map(ToString::to_string),
        source: source.map(ToString::to_string),
        config_schema,
    }
}

fn catalog_item(
    id: &str,
    name: &str,
    description: &str,
    namespace: &str,
    capabilities: &[&str],
) -> SystemCatalogModule {
    rich_catalog_item(
        id,
        name,
        None,
        description,
        namespace,
        capabilities,
        None,
        None,
        None,
    )
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

fn enabled_field(module_name: &str) -> serde_json::Value {
    bool_field(
        "enabled",
        "Module Status",
        true,
        &format!("Enable or disable {module_name} requests"),
    )
}

fn cookie_field(module_name: &str) -> serde_json::Value {
    secret_field(
        "cookie",
        "Session Cookie",
        "Leave empty to keep existing cookie",
        &format!("{module_name} authenticated session cookie"),
    )
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
        rich_catalog_item(
            "local-echo",
            "Local Echo Test Module",
            None,
            "Diagnostic echo adapter for local routing testing",
            "/custom/echo",
            &["chat_completions", "models"],
            None,
            Some("examples/modules/echo-provider"),
            None,
        ),
        catalog_item(
            "suite-test",
            "Suite Provider Group",
            "Bundled multi-provider group (chat completions, vision, models)",
            "/custom/suite-test",
            &["chat_completions", "vision", "models"],
        ),
        rich_catalog_item(
            "tiktok-studio",
            "TikTok Creator Studio Adapter",
            Some("TikTok Creator Studio"),
            "TikTok Creator Studio video management, download extraction, and publishing adapter",
            "/custom/tiktok",
            &[
                "video_download",
                "items_manage",
                "publish",
                "studio",
            ],
            Some("♬"),
            Some("https://github.com/giauphan/TikTok-Creator-Studio-Adapter.git"),
            Some(config_schema(
                "TikTok Creator Studio Config & HAR Import",
                "tiktok-studio",
                "/custom/tiktok",
                "/custom/tiktok/v1/config/har",
                "Drop download-tiktok.har here or select file (.har, .json) to auto-extract Cookie and CSRF token",
                vec![
                    text_field("base_url", "TikTok Base URL", "https://www.tiktok.com", "Primary TikTok Creator Studio upstream URL"),
                    enabled_field("TikTok Creator Studio"),
                    cookie_field("TikTok Creator Studio"),
                    secret_field("csrf_token", "CSRF Token (tt-csrf-token)", "Leave empty to keep existing CSRF token", "TikTok CSRF protection token extracted from x-tt-csrf-token or cookies"),
                ],
            )),
        ),
        rich_catalog_item(
            "facebook-suite",
            "Meta Business Suite / Facebook Creator Studio Adapter",
            Some("Meta Business Suite"),
            "Meta Business Suite / Facebook Creator Studio video management, rupload resumable upload, and Reel/Story publishing adapter",
            "/custom/facebook",
            &[
                "video_upload",
                "rupload",
                "reels",
                "stories",
                "publish",
                "studio",
            ],
            Some("ⓕ"),
            Some("addons/facebook-suite"),
            Some(config_schema(
                "Meta Business Suite Config & HAR Import",
                "facebook-suite",
                "/custom/facebook",
                "/custom/facebook/v1/config/har",
                "Drop download-facebook.har here or select file (.har, .json) to auto-extract credentials and Comet parameters",
                vec![
                    text_field("base_url", "Meta Business Base URL", "https://business.facebook.com", "Primary Meta Business Suite upstream URL"),
                    enabled_field("Meta Business Suite"),
                    cookie_field("Facebook"),
                    secret_field("fb_dtsg", "CSRF Token (fb_dtsg)", "Leave empty to keep existing fb_dtsg token", "Comet anti-CSRF token extracted from HAR or page payload"),
                    text_field("user_id", "Facebook User ID (__user)", "1000...", "Authenticated Facebook User ID"),
                    text_field("actor_id", "Actor ID (av)", "1435...", "Active acting identity or Page ID"),
                    text_field("target_id", "Target Page ID", "1435...", "Default target Facebook Page for uploads and reels"),
                    text_field("business_id", "Business ID (__bid)", "2567...", "Associated Meta Business Account ID"),
                ],
            )),
        ),
        rich_catalog_item(
            "daily-news",
            "Daily News & Content Discovery Service",
            Some("Daily News"),
            "Daily automated news and latest content discovery, canonicalization, and tiered ranking adapter",
            "/custom/daily-news",
            &[
                "news",
                "content_discovery",
                "chat_completions",
                "models",
            ],
            Some("📰"),
            Some("https://github.com/giauphan/gateway-modules/tree/main/modules/daily-news"),
            Some(config_schema(
                "Daily News & Content Discovery Service Config",
                "daily-news",
                "/custom/daily-news",
                "/custom/daily-news/api/v1/config",
                "Configure upstream NewsNow API endpoint and freshness windows",
                vec![
                    text_field("newsnow_url", "NewsNow Base URL", "http://127.0.0.1:4444", "Primary upstream NewsNow feed service URL"),
                    bool_field("require_latest_news", "Strict Latest News", true, "Prioritize latest breaking news with verified timestamps"),
                    text_field("default_sources", "Default Feeds", "hackernews,github,producthunt", "Comma-separated feed sources to query"),
                ],
            )),
        ),
    ]
}
