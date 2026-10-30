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
        catalog_item(
            "local-echo",
            "Local Echo Test Module",
            "Diagnostic echo adapter for local routing testing",
            "/custom/echo",
            &["chat_completions"],
        ),
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
    ]
}
