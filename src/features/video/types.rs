use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VideoCapabilityCatalog {
    pub schema_version: String,
    pub providers: Vec<ProviderCapability>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProviderCapability {
    pub provider: String,
    pub verified: bool,
    pub supported_models: Vec<String>,
    pub supported_operations: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ArtifactDescriptor {
    pub kind: String,
    pub download_url: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VideoTaskStatusResponse {
    pub schema_version: String,
    pub provider: String,
    pub provider_task_id: String,
    pub provider_status: String,
    pub artifact_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_url_expires_at: Option<u64>,
    pub media_validated: bool,
    pub reconciliation_capability: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub artifacts: Vec<ArtifactDescriptor>,
}

pub fn validate_secure_url(url_str: &str) -> bool {
    if !url_str.starts_with("https://") {
        return false;
    }
    let rest = &url_str["https://".len()..];
    let authority = rest.split('/').next().unwrap_or(rest);
    let host = authority.split('@').next_back().unwrap_or(authority);
    let host_clean = host.split(':').next().unwrap_or(host).to_ascii_lowercase();

    if host_clean.is_empty()
        || host_clean == "localhost"
        || host_clean.starts_with("127.")
        || host_clean.starts_with("10.")
        || host_clean.starts_with("192.168.")
        || host_clean.starts_with("172.")
        || host_clean == "0.0.0.0"
        || host_clean == "::1"
        || host_clean == "[::1]"
        || host_clean.ends_with(".internal")
        || host_clean.ends_with(".local")
    {
        return false;
    }
    true
}
