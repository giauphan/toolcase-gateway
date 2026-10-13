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
pub struct VideoTaskStatusResponse {
    pub schema_version: String,
    pub provider: String,
    pub provider_task_id: String,
    pub provider_status: String,
    pub media_validated: bool,
    pub reconciliation_capability: String,
}
