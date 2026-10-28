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

/// Returns the available catalog of standard provider modules ready for installation.
pub fn get_system_module_catalog() -> Vec<SystemCatalogModule> {
    vec![
        SystemCatalogModule {
            id: "cohere-provider".to_string(),
            name: "Cohere Provider Adapter".to_string(),
            description: "Cohere Command-R & Embeddings provider adapter".to_string(),
            namespace: "/custom/cohere".to_string(),
            version: "1.0.0".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec!["chat_completions".to_string(), "models".to_string()],
        },
        SystemCatalogModule {
            id: "claude-adapter".to_string(),
            name: "Anthropic Claude Adapter".to_string(),
            description: "Anthropic Claude 3.5 Sonnet & Haiku bridge".to_string(),
            namespace: "/custom/claude".to_string(),
            version: "1.0.0".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec![
                "chat_completions".to_string(),
                "models".to_string(),
                "streaming".to_string(),
            ],
        },
        SystemCatalogModule {
            id: "groq-fast".to_string(),
            name: "Groq High-Speed LLM".to_string(),
            description: "Ultra-low latency inference via Groq LPU".to_string(),
            namespace: "/custom/groq".to_string(),
            version: "1.0.0".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec!["chat_completions".to_string(), "models".to_string()],
        },
        SystemCatalogModule {
            id: "mistral-ai".to_string(),
            name: "Mistral AI Provider".to_string(),
            description: "Mistral Large, Codestral & Mixtral models".to_string(),
            namespace: "/custom/mistral".to_string(),
            version: "1.0.0".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec!["chat_completions".to_string(), "models".to_string()],
        },
        SystemCatalogModule {
            id: "local-echo".to_string(),
            name: "Local Echo Test Module".to_string(),
            description: "Diagnostic echo adapter for local routing testing".to_string(),
            namespace: "/custom/echo".to_string(),
            version: "1.0.0".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec!["chat_completions".to_string()],
        },
    ]
}
