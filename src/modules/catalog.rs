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
    }
}

/// Returns the available catalog of standard provider modules ready for installation.
pub fn get_system_module_catalog() -> Vec<SystemCatalogModule> {
    vec![
        catalog_item(
            "cohere-provider",
            "Cohere Provider Adapter",
            "Cohere Command-R & Embeddings provider adapter",
            "/custom/cohere",
            &["chat_completions", "models"],
        ),
        catalog_item(
            "claude-adapter",
            "Anthropic Claude Adapter",
            "Anthropic Claude 3.5 Sonnet & Haiku bridge",
            "/custom/claude",
            &["chat_completions", "models", "streaming"],
        ),
        catalog_item(
            "groq-fast",
            "Groq High-Speed LLM",
            "Ultra-low latency inference via Groq LPU",
            "/custom/groq",
            &["chat_completions", "models"],
        ),
        catalog_item(
            "mistral-ai",
            "Mistral AI Provider",
            "Mistral Large, Codestral & Mixtral models",
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
    ]
}
