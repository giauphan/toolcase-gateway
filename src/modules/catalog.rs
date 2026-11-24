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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_content: Option<String>,
}

fn derive_catalog_docs(
    id: &str,
    name: &str,
    namespace: &str,
    capabilities: &[&str],
    source: Option<&str>,
) -> (Option<String>, Option<String>) {
    let doc_url = match id {
        "facebook-suite" => Some(
            "https://github.com/giauphan/gateway-modules/tree/main/modules/facebook-suite#readme"
                .to_string(),
        ),
        "tiktok-studio" => {
            Some("https://github.com/giauphan/TikTok-Creator-Studio-Adapter#readme".to_string())
        }
        "daily-news" => Some(
            "https://github.com/giauphan/gateway-modules/tree/main/modules/daily-news#readme"
                .to_string(),
        ),
        "cohere-provider" => Some("https://docs.cohere.com".to_string()),
        "claude-adapter" => Some("https://docs.anthropic.com".to_string()),
        "groq-fast" => Some("https://console.groq.com/docs".to_string()),
        "mistral-ai" => Some("https://docs.mistral.ai".to_string()),
        _ => source
            .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
            .map(|s| format!("{s}#readme"))
            .or_else(|| Some(format!("/modules/{id}/doc"))),
    };

    let caps_str = capabilities.join(", ");
    let doc_content = format!(
        "# {name}\n\n**Module ID**: `{id}`  \n**Route Prefix**: `{namespace}`  \n**Capabilities**: {caps_str}\n\n### Overview\nPre-configured gateway provider module ready for installation.\n\n### Endpoints\n- Route base: `{namespace}`\n"
    );

    (doc_url, Some(doc_content))
}

fn read_local_readme(id: &str) -> Option<String> {
    if id.contains("..") || id.contains('/') || id.contains('\\') {
        return None;
    }
    let state_dir = crate::gateway::installer::get_state_dir();
    let candidates = [
        state_dir.join("modules").join(id).join("README.md"),
        std::path::PathBuf::from("addons")
            .join(id)
            .join("README.md"),
        std::path::PathBuf::from("modules")
            .join(id)
            .join("README.md"),
    ];
    for cand in &candidates {
        if cand.is_file() {
            if let Ok(text) = std::fs::read_to_string(cand) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(text);
                }
            }
        }
    }
    None
}

fn is_archive_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let path = lower.split('?').next().unwrap_or(&lower);
    path.ends_with(".zip")
        || path.ends_with(".tar.gz")
        || path.ends_with(".tgz")
        || path.ends_with(".tar.bz2")
        || path.ends_with(".tar")
}

fn is_repo_url(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://"))
        && !is_archive_url(url)
        && (url.contains("github.com")
            || url.contains("gitlab.com")
            || url.contains("bitbucket.org")
            || url.ends_with(".git"))
}

/// Resolves comprehensive documentation URL and markdown document block for any module,
/// applying the intelligent hierarchy:
/// 1. Explicit override if already present and non-empty.
/// 2. Local README.md file in module state directory or addons.
/// 3. System catalog default documentation and external links.
/// 4. Clean synthesized markdown document block.
pub fn resolve_module_documentation(
    id: &str,
    name: &str,
    namespace: &str,
    capabilities: &[String],
    source: Option<&str>,
    existing_url: Option<String>,
    existing_content: Option<String>,
) -> (Option<String>, Option<String>) {
    let clean_existing_content = existing_content.filter(|s| !s.trim().is_empty());
    let clean_existing_url = existing_url.filter(|s| !s.trim().is_empty());

    let catalog_item = get_system_module_catalog().into_iter().find(|c| c.id == id);
    let catalog_doc_url = catalog_item.as_ref().and_then(|c| c.doc_url.clone());
    let catalog_doc_content = catalog_item.as_ref().and_then(|c| c.doc_content.clone());

    let default_canonical_url = format!("/modules/{id}/doc");
    let default_plural_url = format!("/modules/{id}/docs");

    let doc_url = if let Some(url) = clean_existing_url {
        let is_default_generated = url == default_canonical_url || url == default_plural_url;
        if is_default_generated {
            catalog_doc_url
                .or_else(|| {
                    source
                        .filter(|s| is_repo_url(s))
                        .map(|src| format!("{src}#readme"))
                })
                .or(Some(url))
        } else {
            // Preserve explicit local or external URLs configured by user or manifest
            Some(url)
        }
    } else if let Some(cat_url) = catalog_doc_url {
        Some(cat_url)
    } else if let Some(src) = source.filter(|s| is_repo_url(s)) {
        Some(format!("{src}#readme"))
    } else {
        Some(default_canonical_url)
    };

    let doc_content = if let Some(content) = clean_existing_content {
        Some(content)
    } else if let Some(readme) = read_local_readme(id) {
        Some(readme)
    } else if let Some(cat_content) = catalog_doc_content {
        Some(cat_content)
    } else if doc_url
        .as_ref()
        .map(|u| u.starts_with("http://") || u.starts_with("https://"))
        .unwrap_or(false)
    {
        None
    } else {
        let caps_str = if capabilities.is_empty() {
            "None declared".to_string()
        } else {
            capabilities.join(", ")
        };
        Some(format!(
            "# {name}\n\n**Module ID**: `{id}`  \n**Route Prefix**: `{namespace}`  \n**Capabilities**: {caps_str}\n\n### Overview\nActive Toolcase Gateway provider module routed at `{namespace}`.\n\n### Endpoints\n- Route base: `{namespace}`\n- Module UI Configuration: `/modules/{id}/config`\n- Documentation Viewer: `/modules/{id}/doc`\n"
        ))
    };

    (doc_url, doc_content)
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
    let (doc_url, doc_content) = derive_catalog_docs(id, name, namespace, capabilities, source);
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
        doc_url,
        doc_content,
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
            Some("https://github.com/giauphan/gateway-modules/tree/main/modules/facebook-suite"),
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
