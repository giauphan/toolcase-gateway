//! Pre-configured provider modules ready for 1-click installation.

use serde::{Deserialize, Serialize};

/// Catalog item representing a pre-configured provider module available for 1-click installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemCatalogModule {
    pub id: String,
    pub name: String,
    pub description: String,
    pub namespace: String,
    pub version: String,
    pub protocol: String,
    pub capabilities: Vec<String>,
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
        description: description.to_string(),
        namespace: namespace.to_string(),
        version: "1.0.0".to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: capabilities.iter().map(ToString::to_string).collect(),
        source: None,
        config_schema: None,
    }
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
            description: "Diagnostic echo adapter for local routing testing".to_string(),
            namespace: "/custom/echo".to_string(),
            version: "1.0.0".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec!["chat_completions".to_string(), "models".to_string()],
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
            source: Some("https://github.com/giauphan/TikTok-Creator-Studio-Adapter.git".to_string()),
            config_schema: Some(serde_json::json!({
                "title": "TikTok Creator Studio Config & HAR Import",
                "module_id": "tiktok-studio",
                "namespace": "/custom/tiktok",
                "har_endpoint": "/custom/tiktok/v1/config/har",
                "har_hint": "Drop download-tiktok.har here or select file (.har, .json) to auto-extract Cookie and CSRF token",
                "fields": [
                    {
                        "key": "base_url",
                        "label": "TikTok Base URL",
                        "type": "text",
                        "placeholder": "https://www.tiktok.com",
                        "description": "Primary TikTok Creator Studio upstream URL"
                    },
                    {
                        "key": "enabled",
                        "label": "Module Status",
                        "type": "boolean",
                        "default": true,
                        "description": "Enable or disable TikTok Creator Studio requests"
                    },
                    {
                        "key": "cookie",
                        "label": "Session Cookie",
                        "type": "password",
                        "secret": true,
                        "placeholder": "Leave empty to keep existing cookie",
                        "description": "TikTok Creator Studio authenticated session cookie"
                    },
                    {
                        "key": "csrf_token",
                        "label": "CSRF Token (tt-csrf-token)",
                        "type": "password",
                        "secret": true,
                        "placeholder": "Leave empty to keep existing CSRF token",
                        "description": "TikTok CSRF protection token extracted from x-tt-csrf-token or cookies"
                    }
                ]
            })),
        },
        SystemCatalogModule {
            id: "facebook-suite".to_string(),
            name: "Meta Business Suite / Facebook Creator Studio Adapter".to_string(),
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
            source: None,
            config_schema: Some(serde_json::json!({
                "title": "Meta Business Suite Config & HAR Import",
                "module_id": "facebook-suite",
                "namespace": "/custom/facebook",
                "har_endpoint": "/custom/facebook/v1/config/har",
                "har_hint": "Drop download-facebook.har here or select file (.har, .json) to auto-extract credentials and Comet parameters",
                "fields": [
                    {
                        "key": "base_url",
                        "label": "Meta Business Base URL",
                        "type": "text",
                        "placeholder": "https://business.facebook.com",
                        "description": "Primary Meta Business Suite upstream URL"
                    },
                    {
                        "key": "enabled",
                        "label": "Module Status",
                        "type": "boolean",
                        "default": true,
                        "description": "Enable or disable Meta Business Suite requests"
                    },
                    {
                        "key": "cookie",
                        "label": "Session Cookie",
                        "type": "password",
                        "secret": true,
                        "placeholder": "Leave empty to keep existing cookie",
                        "description": "Facebook authenticated session cookie"
                    },
                    {
                        "key": "fb_dtsg",
                        "label": "CSRF Token (fb_dtsg)",
                        "type": "password",
                        "secret": true,
                        "placeholder": "Leave empty to keep existing fb_dtsg token",
                        "description": "Comet anti-CSRF token extracted from HAR or page payload"
                    },
                    {
                        "key": "user_id",
                        "label": "Facebook User ID (__user)",
                        "type": "text",
                        "placeholder": "1000...",
                        "description": "Authenticated Facebook User ID"
                    },
                    {
                        "key": "actor_id",
                        "label": "Actor ID (av)",
                        "type": "text",
                        "placeholder": "1435...",
                        "description": "Active acting identity or Page ID"
                    },
                    {
                        "key": "target_id",
                        "label": "Target Page ID",
                        "type": "text",
                        "placeholder": "1435...",
                        "description": "Default target Facebook Page for uploads and reels"
                    },
                    {
                        "key": "business_id",
                        "label": "Business ID (__bid)",
                        "type": "text",
                        "placeholder": "2567...",
                        "description": "Associated Meta Business Account ID"
                    }
                ]
            })),
        },
    ]
}
