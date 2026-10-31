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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    #[serde(default = "default_module_version")]
    pub version: String,
    pub namespace: String,
    pub entrypoint: String,
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
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

/// Persisted Dashboard UI menu position and ordering metadata for a module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleUiPosition {
    #[serde(default = "default_menu_position")]
    pub menu_position: String,
    #[serde(default = "default_menu_order")]
    pub menu_order: i32,
}

impl Default for ModuleUiPosition {
    fn default() -> Self {
        Self {
            menu_position: default_menu_position(),
            menu_order: default_menu_order(),
        }
    }
}

pub fn default_menu_position() -> String {
    "providers".to_string()
}

pub fn default_menu_order() -> i32 {
    100
}

pub fn is_valid_menu_position(pos: &str) -> bool {
    matches!(
        pos.trim().to_ascii_lowercase().as_str(),
        "providers" | "sidebar" | "studios" | "overview" | "hidden"
    )
}

/// Generates a default automatic UI configuration schema for a module.
pub fn default_module_config_schema(id: &str, name: &str, namespace: &str) -> serde_json::Value {
    serde_json::json!({
        "title": format!("{name} Configuration"),
        "module_id": id,
        "namespace": namespace,
        "fields": [
            {
                "key": "base_url",
                "label": "Upstream Base URL",
                "type": "text",
                "placeholder": "https://api.example.com",
                "description": "Primary upstream HTTP endpoint for this module"
            },
            {
                "key": "api_key",
                "label": "API Key / Bearer Token",
                "type": "password",
                "secret": true,
                "placeholder": "Leave empty to keep existing secret",
                "description": "Authentication token passed to upstream requests"
            },
            {
                "key": "cookie",
                "label": "Session Cookie",
                "type": "password",
                "secret": true,
                "placeholder": "Leave empty to keep existing cookie",
                "description": "Optional browser session cookie header"
            },
            {
                "key": "default_model",
                "label": "Default Model / Target",
                "type": "text",
                "placeholder": id,
                "description": "Default model identifier or target profile"
            },
            {
                "key": "timeout_secs",
                "label": "Request Timeout (seconds)",
                "type": "number",
                "default": 30,
                "description": "Maximum execution time in seconds"
            }
        ]
    })
}

/// Public descriptor of an installed module or module group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleDescriptor {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    pub namespace: String,
    pub kind: ModuleKind,
    pub status: ModuleStatus,
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub providers: Option<Vec<SubProviderDescriptor>>,
    #[serde(default = "default_menu_position")]
    pub menu_position: String,
    #[serde(default = "default_menu_order")]
    pub menu_order: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_tab: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_schema: Option<serde_json::Value>,
}

pub fn builtin_ui_defaults(
    id: &str,
    name: &str,
    namespace: &str,
) -> (
    i32,
    String,
    String,
    String,
    String,
    Option<serde_json::Value>,
) {
    match id {
        "omniroute" => (
            10,
            "◎".to_string(),
            "OpenAI-compatible proxy · model fallback and casing repair".to_string(),
            "/modules/omniroute/config".to_string(),
            "omni".to_string(),
            Some(default_module_config_schema(id, name, namespace)),
        ),
        "muse" => (
            20,
            "✦".to_string(),
            "Chat completions, image and video pipeline with Sentinel approvals".to_string(),
            "/muse-config".to_string(),
            "muse".to_string(),
            Some(default_module_config_schema(id, name, namespace)),
        ),
        "jev" => (
            30,
            "◇".to_string(),
            "Decision API, pooled multi-account rotation and failover".to_string(),
            "/modules/jev/config".to_string(),
            "jev".to_string(),
            Some(default_module_config_schema(id, name, namespace)),
        ),
        _ => (
            50,
            "◫".to_string(),
            format!("{name} built-in gateway module"),
            format!("/modules/{id}/config"),
            id.to_string(),
            Some(default_module_config_schema(id, name, namespace)),
        ),
    }
}

pub fn is_secret_config_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("key")
        || lower.contains("token")
        || lower.contains("cookie")
        || lower.contains("secret")
        || lower.contains("password")
        || lower.contains("csrf")
}

/// Normalizes a module's `config_schema` so `module_id`, `namespace`, and `har_endpoint`
/// always reflect the installed module identifier and route prefix.
pub fn normalize_module_config_schema(
    schema: serde_json::Value,
    id: &str,
    namespace: &str,
) -> serde_json::Value {
    let clean_ns = namespace.trim().trim_end_matches('/');
    let serde_json::Value::Object(mut obj) = schema else {
        return schema;
    };

    let prev_ns = obj
        .get("namespace")
        .and_then(serde_json::Value::as_str)
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .unwrap_or_default();

    obj.insert(
        "module_id".to_string(),
        serde_json::Value::String(id.to_string()),
    );
    obj.insert(
        "namespace".to_string(),
        serde_json::Value::String(clean_ns.to_string()),
    );

    if let Some(har_ep) = obj
        .get("har_endpoint")
        .and_then(serde_json::Value::as_str)
        .map(|s| s.trim().to_string())
    {
        let updated_har = if !prev_ns.is_empty() && har_ep.starts_with(&prev_ns) {
            format!("{clean_ns}{}", &har_ep[prev_ns.len()..])
        } else if har_ep.ends_with("/v1/config/har") && !har_ep.starts_with(clean_ns) {
            format!("{clean_ns}/v1/config/har")
        } else if har_ep.ends_with("/config/har") && !har_ep.starts_with(clean_ns) {
            format!("{clean_ns}/config/har")
        } else {
            har_ep
        };
        obj.insert(
            "har_endpoint".to_string(),
            serde_json::Value::String(updated_har),
        );
    }

    serde_json::Value::Object(obj)
}

pub fn mask_config_map(
    raw: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    for (k, v) in raw {
        if is_secret_config_key(k) {
            if let Some(s) = v.as_str() {
                let t = s.trim();
                let masked = crate::features::dashboard::mask_key(t);
                out.insert(format!("masked_{k}"), serde_json::Value::String(masked));
                out.insert(format!("has_{k}"), serde_json::Value::Bool(!t.is_empty()));
            }
        } else {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}
