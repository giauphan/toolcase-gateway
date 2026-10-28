//! Platform module definitions, manifests, and data transfer types.

use serde::{Deserialize, Serialize};

/// Classification of module origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleKind {
    BuiltIn,
    Personal,
}

/// Runtime availability of a module or sub-provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleStatus {
    Enabled,
    Disabled,
}

/// Errors occurring during module validation, registration, or lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleError {
    InvalidId(String),
    InvalidNamespace(String),
    ReservedNamespace(String),
    NamespaceCollision(String),
    NotFound(String),
    InvalidManifest(String),
}

impl std::fmt::Display for ModuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidId(id) => write!(f, "Invalid module id '{id}'"),
            Self::InvalidNamespace(ns) => write!(f, "Invalid namespace prefix '{ns}'"),
            Self::ReservedNamespace(ns) => write!(f, "Namespace '{ns}' is reserved by system"),
            Self::NamespaceCollision(ns) => write!(f, "Namespace collision on prefix '{ns}'"),
            Self::NotFound(id) => write!(f, "Module '{id}' not found"),
            Self::InvalidManifest(msg) => write!(f, "Invalid module manifest: {msg}"),
        }
    }
}

impl std::error::Error for ModuleError {}

/// Manifest for a small sub-provider within a module group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubProviderManifest {
    pub id: String,
    #[serde(default = "default_subprovider_name")]
    pub name: String,
    #[serde(default)]
    pub subpath: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

fn default_subprovider_name() -> String {
    "Sub Provider".to_string()
}

/// Manifest grouping configuration allowing one module package to bundle multiple providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleGroupManifest {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub providers: Vec<SubProviderManifest>,
}

fn default_true() -> bool {
    true
}

/// Manifest for an out-of-tree personal provider module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalManifest {
    pub id: String,
    #[serde(default = "default_module_name")]
    pub name: String,
    #[serde(default = "default_module_version")]
    pub version: String,
    pub namespace: String,
    pub entrypoint: String,
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub config_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<ModuleGroupManifest>,
}

fn default_module_name() -> String {
    "Custom Module".to_string()
}

fn default_module_version() -> String {
    "0.1.0".to_string()
}

fn default_protocol() -> String {
    "stdio-stream".to_string()
}

/// Runtime descriptor of a sub-provider inside a module group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubProviderDescriptor {
    pub id: String,
    pub name: String,
    pub subpath: String,
    pub capabilities: Vec<String>,
    pub status: ModuleStatus,
}

/// Public descriptor of an installed module or module group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleDescriptor {
    pub id: String,
    pub name: String,
    pub namespace: String,
    pub kind: ModuleKind,
    pub status: ModuleStatus,
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub providers: Option<Vec<SubProviderDescriptor>>,
}
