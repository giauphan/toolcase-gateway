//! Gateway platform module definitions, manifests, and registry.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Classification of module origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleKind {
    BuiltIn,
    Personal,
}

/// Runtime availability of a module.
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

/// Public descriptor of an installed module.
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
}

/// System-reserved prefixes that cannot be claimed by provider modules.
const RESERVED_PREFIXES: &[&str] = &[
    "/api",
    "/dashboard",
    "/assets",
    "/mcp",
    "/health",
    "/favicon.ico",
    "/jev-ai",
    "/video",
    "/v1",
    "/muse-config",
    "/gateway-config",
    "/video-template",
    "/image-template",
];

/// Registry of authoritative built-in and dynamically installed personal modules.
#[derive(Debug, Clone, Default)]
pub struct ModuleRegistry {
    modules: HashMap<String, ModuleDescriptor>,
    personal_manifests: HashMap<String, PersonalManifest>,
}

impl ModuleRegistry {
    /// Creates a registry initialized with default built-in platform modules.
    pub fn new() -> Self {
        let mut registry = Self {
            modules: HashMap::new(),
            personal_manifests: HashMap::new(),
        };

        registry.register_builtin(
            "omniroute",
            "OmniRoute LLM Proxy",
            "/ominroute",
            vec![
                "chat_completions".to_string(),
                "models".to_string(),
                "failover".to_string(),
            ],
        );

        registry.register_builtin(
            "muse",
            "Muse AI Provider",
            "/muse-ai",
            vec![
                "chat".to_string(),
                "models".to_string(),
                "video".to_string(),
                "image".to_string(),
            ],
        );

        registry.register_builtin(
            "jev",
            "Jev Decision Provider",
            "/jev",
            vec!["decisions".to_string(), "models".to_string()],
        );

        registry
    }

    fn register_builtin(
        &mut self,
        id: &str,
        name: &str,
        namespace: &str,
        capabilities: Vec<String>,
    ) {
        self.modules.insert(
            id.to_string(),
            ModuleDescriptor {
                id: id.to_string(),
                name: name.to_string(),
                namespace: namespace.trim_end_matches('/').to_string(),
                kind: ModuleKind::BuiltIn,
                status: ModuleStatus::Enabled,
                capabilities,
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            },
        );
    }

    /// Validates and registers an out-of-tree personal module manifest.
    pub fn register_personal_module(
        &mut self,
        manifest: PersonalManifest,
    ) -> Result<(), ModuleError> {
        let id = manifest.id.trim();
        validate_module_id(id, &self.modules)?;

        let ns = manifest.namespace.trim().trim_end_matches('/');
        validate_module_namespace(ns, &manifest.namespace, id, &self.modules)?;

        validate_module_protocol(&manifest.protocol, &manifest.entrypoint)?;

        let desc = ModuleDescriptor {
            id: id.to_string(),
            name: manifest.name.clone(),
            namespace: ns.to_string(),
            kind: ModuleKind::Personal,
            status: ModuleStatus::Enabled,
            capabilities: manifest.capabilities.clone(),
            version: Some(manifest.version.clone()),
        };

        self.modules.insert(id.to_string(), desc);
        self.personal_manifests.insert(id.to_string(), manifest);
        Ok(())
    }

    /// Unregisters an installed personal module by id.
    pub fn unregister_personal_module(
        &mut self,
        id: &str,
    ) -> Result<PersonalManifest, ModuleError> {
        if let Some(desc) = self.modules.get(id) {
            if desc.kind == ModuleKind::BuiltIn {
                return Err(ModuleError::InvalidManifest(format!(
                    "Cannot unregister built-in module '{id}'"
                )));
            }
        } else {
            return Err(ModuleError::NotFound(id.to_string()));
        }

        self.modules.remove(id);
        self.personal_manifests
            .remove(id)
            .ok_or_else(|| ModuleError::NotFound(id.to_string()))
    }

    /// Sets the enablement status of a module.
    pub fn set_module_status(&mut self, id: &str, status: ModuleStatus) -> Result<(), ModuleError> {
        let desc = self
            .modules
            .get_mut(id)
            .ok_or_else(|| ModuleError::NotFound(id.to_string()))?;
        desc.status = status;
        Ok(())
    }

    /// Matches a normalized URL path against active module namespaces.
    /// Returns the matched module descriptor and the stripped subpath.
    pub fn match_path<'a>(&'a self, path: &'a str) -> Option<(&'a ModuleDescriptor, &'a str)> {
        let clean = path.split('?').next().unwrap_or(path);
        let trimmed = clean.trim_end_matches('/');

        // Find the longest matching namespace prefix
        let mut matched: Option<(&'a ModuleDescriptor, &'a str)> = None;
        let mut longest_len = 0;

        for desc in self.modules.values() {
            let ns_str = desc.namespace.as_str();
            let prefixes: &[&str] = if desc.id == "omniroute" {
                &["/ominroute", "/omniroute"]
            } else {
                std::slice::from_ref(&ns_str)
            };

            for &ns in prefixes {
                if trimmed == ns {
                    if ns.len() > longest_len {
                        longest_len = ns.len();
                        matched = Some((desc, ""));
                    }
                } else if trimmed.starts_with(ns)
                    && trimmed[ns.len()..].starts_with('/')
                    && ns.len() > longest_len
                {
                    longest_len = ns.len();
                    let subpath = &trimmed[ns.len()..];
                    matched = Some((desc, subpath));
                }
            }
        }

        matched
    }

    /// Returns a list of all registered module descriptors.
    pub fn list_modules(&self) -> Vec<ModuleDescriptor> {
        let mut list: Vec<ModuleDescriptor> = self.modules.values().cloned().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }

    /// Retrieves a module descriptor by id.
    pub fn get_module(&self, id: &str) -> Option<&ModuleDescriptor> {
        self.modules.get(id)
    }

    /// Retrieves a personal module manifest by id.
    pub fn get_personal_manifest(&self, id: &str) -> Option<&PersonalManifest> {
        self.personal_manifests.get(id)
    }

    /// Exports all personal manifests for persistent storage.
    pub fn export_personal_manifests(&self) -> Vec<PersonalManifest> {
        let mut list: Vec<PersonalManifest> = self.personal_manifests.values().cloned().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }

    /// Applies enablement states from configuration.
    pub fn apply_enablement_map(&mut self, map: &HashMap<String, bool>) {
        for (id, enabled) in map {
            if let Some(desc) = self.modules.get_mut(id) {
                desc.status = if *enabled {
                    ModuleStatus::Enabled
                } else {
                    ModuleStatus::Disabled
                };
            }
        }
    }
}

fn validate_module_id(
    id: &str,
    modules: &HashMap<String, ModuleDescriptor>,
) -> Result<(), ModuleError> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ModuleError::InvalidId(id.to_string()));
    }
    if let Some(existing) = modules.get(id) {
        if existing.kind == ModuleKind::BuiltIn {
            return Err(ModuleError::InvalidId(format!(
                "Cannot overwrite built-in module '{id}'"
            )));
        }
    }
    Ok(())
}

fn validate_module_namespace(
    ns: &str,
    raw_ns: &str,
    id: &str,
    modules: &HashMap<String, ModuleDescriptor>,
) -> Result<(), ModuleError> {
    if !ns.starts_with('/') || ns.len() < 2 {
        return Err(ModuleError::InvalidNamespace(raw_ns.to_string()));
    }
    for reserved in RESERVED_PREFIXES {
        if ns == *reserved || ns.starts_with(&format!("{reserved}/")) {
            return Err(ModuleError::ReservedNamespace(raw_ns.to_string()));
        }
    }
    const OMNIROUTE_ALIASES: &[&str] = &["/ominroute", "/omniroute"];
    if OMNIROUTE_ALIASES
        .iter()
        .any(|a| ns == *a || ns.starts_with(&format!("{a}/")) || a.starts_with(&format!("{ns}/")))
    {
        return Err(ModuleError::NamespaceCollision(raw_ns.to_string()));
    }
    for (other_id, desc) in modules {
        if other_id != id
            && (desc.namespace == ns
                || ns.starts_with(&format!("{}/", desc.namespace))
                || desc.namespace.starts_with(&format!("{ns}/")))
        {
            return Err(ModuleError::NamespaceCollision(raw_ns.to_string()));
        }
    }
    Ok(())
}

fn validate_module_protocol(protocol: &str, entrypoint: &str) -> Result<(), ModuleError> {
    let ep = entrypoint.trim();
    if ep.is_empty() {
        return Err(ModuleError::InvalidManifest("Empty entrypoint".to_string()));
    }
    match protocol {
        "stdio-stream" => {
            if !std::path::Path::new(ep).is_absolute() {
                return Err(ModuleError::InvalidManifest(format!(
                    "stdio-stream entrypoint must be an absolute path: '{ep}'"
                )));
            }
        }
        "local-http" => {
            if !is_loopback_http_endpoint(ep) {
                return Err(ModuleError::InvalidManifest(format!(
                    "local-http entrypoint must target a local loopback HTTP endpoint (127.0.0.1, localhost, [::1]): '{ep}'"
                )));
            }
        }
        other => {
            return Err(ModuleError::InvalidManifest(format!(
                "Unsupported protocol '{other}'. Supported protocols are 'stdio-stream' and 'local-http'"
            )));
        }
    }
    Ok(())
}

fn is_loopback_http_endpoint(ep: &str) -> bool {
    let lower = ep.to_ascii_lowercase();
    lower.starts_with("http://") && crate::http::is_loopback_origin(ep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_registry_has_builtins() {
        let reg = ModuleRegistry::new();
        let modules = reg.list_modules();
        assert_eq!(modules.len(), 3);
        assert!(reg.get_module("omniroute").is_some());
        assert!(reg.get_module("muse").is_some());
        assert!(reg.get_module("jev").is_some());
    }

    #[test]
    fn test_match_path_builtins() {
        let reg = ModuleRegistry::new();
        let (omni, sub) = reg.match_path("/ominroute/v1/chat/completions").unwrap();
        assert_eq!(omni.id, "omniroute");
        assert_eq!(sub, "/v1/chat/completions");

        let (muse, sub_muse) = reg.match_path("/muse-ai/v1/chat/completions").unwrap();
        assert_eq!(muse.id, "muse");
        assert_eq!(sub_muse, "/v1/chat/completions");

        // Exact match
        let (jev, sub_jev) = reg.match_path("/jev").unwrap();
        assert_eq!(jev.id, "jev");
        assert_eq!(sub_jev, "");

        // Lookalike path must NOT match
        assert!(reg.match_path("/ominroute-other/test").is_none());
        assert!(reg.match_path("/unknown/path").is_none());
    }

    #[test]
    fn test_personal_module_registration_and_isolation() {
        let mut reg = ModuleRegistry::new();
        let manifest = PersonalManifest {
            id: "custom-provider".to_string(),
            name: "Custom Provider".to_string(),
            version: "1.0.0".to_string(),
            namespace: "/custom/cohere".to_string(),
            entrypoint: "/opt/bin/cohere-adapter".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec!["chat".to_string()],
            config_schema: None,
            permissions: vec![],
        };

        reg.register_personal_module(manifest).unwrap();
        assert!(reg.get_module("custom-provider").is_some());

        let (m, sub) = reg.match_path("/custom/cohere/v1/generate").unwrap();
        assert_eq!(m.id, "custom-provider");
        assert_eq!(sub, "/v1/generate");

        // OmniRoute alias match
        let (omni, omni_sub) = reg.match_path("/omniroute/v1/chat/completions").unwrap();
        assert_eq!(omni.id, "omniroute");
        assert_eq!(omni_sub, "/v1/chat/completions");

        // Collision detection with existing module
        let dup = PersonalManifest {
            id: "dup-provider".to_string(),
            name: "Duplicate".to_string(),
            version: "0.1.0".to_string(),
            namespace: "/custom/cohere".to_string(),
            entrypoint: "/bin/echo".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec![],
            config_schema: None,
            permissions: vec![],
        };
        assert!(matches!(
            reg.register_personal_module(dup),
            Err(ModuleError::NamespaceCollision(_))
        ));

        // Reserved prefix rejection
        for reserved_prefix in [
            "/api",
            "/dashboard",
            "/assets",
            "/mcp",
            "/health",
            "/favicon.ico",
            "/jev-ai",
            "/video",
            "/v1",
            "/muse-config",
            "/gateway-config",
            "/video-template",
            "/image-template",
        ] {
            let reserved = PersonalManifest {
                id: format!("bad-{}", reserved_prefix.replace(['/', '.'], "-")),
                name: "Bad".to_string(),
                version: "0.1.0".to_string(),
                namespace: format!("{reserved_prefix}/custom"),
                entrypoint: "/bin/echo".to_string(),
                protocol: "stdio-stream".to_string(),
                capabilities: vec![],
                config_schema: None,
                permissions: vec![],
            };
            assert!(matches!(
                reg.register_personal_module(reserved),
                Err(ModuleError::ReservedNamespace(_))
            ));
        }

        // Built-in ID reuse rejection
        for builtin_id in ["omniroute", "muse", "jev"] {
            let reuse = PersonalManifest {
                id: builtin_id.to_string(),
                name: "Takeover".to_string(),
                version: "0.1.0".to_string(),
                namespace: "/custom/takeover".to_string(),
                entrypoint: "/bin/echo".to_string(),
                protocol: "stdio-stream".to_string(),
                capabilities: vec![],
                config_schema: None,
                permissions: vec![],
            };
            assert!(matches!(
                reg.register_personal_module(reuse),
                Err(ModuleError::InvalidId(_))
            ));
        }

        // Test protocol and entrypoint validation
        let relative_stdio = PersonalManifest {
            id: "rel-mod".to_string(),
            name: "Relative".to_string(),
            version: "1.0.0".to_string(),
            namespace: "/custom/rel".to_string(),
            entrypoint: "./relative/bin".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec![],
            config_schema: None,
            permissions: vec![],
        };
        assert!(matches!(
            reg.register_personal_module(relative_stdio),
            Err(ModuleError::InvalidManifest(_))
        ));

        let bad_http = PersonalManifest {
            id: "bad-http".to_string(),
            name: "Bad Http".to_string(),
            version: "1.0.0".to_string(),
            namespace: "/custom/bad-http".to_string(),
            entrypoint: "http://example.com/api".to_string(),
            protocol: "local-http".to_string(),
            capabilities: vec![],
            config_schema: None,
            permissions: vec![],
        };
        assert!(matches!(
            reg.register_personal_module(bad_http),
            Err(ModuleError::InvalidManifest(_))
        ));

        let good_http = PersonalManifest {
            id: "good-http".to_string(),
            name: "Good Http".to_string(),
            version: "1.0.0".to_string(),
            namespace: "/custom/good-http".to_string(),
            entrypoint: "http://127.0.0.1:8080".to_string(),
            protocol: "local-http".to_string(),
            capabilities: vec![],
            config_schema: None,
            permissions: vec![],
        };
        assert!(reg.register_personal_module(good_http).is_ok());

        // Rejection of omniroute alias collision
        let omni_collide = PersonalManifest {
            id: "omni-collide".to_string(),
            name: "Omni Collide".to_string(),
            version: "1.0.0".to_string(),
            namespace: "/omniroute/v1".to_string(),
            entrypoint: "/bin/echo".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec![],
            config_schema: None,
            permissions: vec![],
        };
        assert!(matches!(
            reg.register_personal_module(omni_collide),
            Err(ModuleError::NamespaceCollision(_))
        ));

        // Rejection of SSRF domain spoofing in local-http
        let ssrf_http = PersonalManifest {
            id: "ssrf-http".to_string(),
            name: "SSRF Http".to_string(),
            version: "1.0.0".to_string(),
            namespace: "/custom/ssrf".to_string(),
            entrypoint: "http://localhost.evil.com/api".to_string(),
            protocol: "local-http".to_string(),
            capabilities: vec![],
            config_schema: None,
            permissions: vec![],
        };
        assert!(matches!(
            reg.register_personal_module(ssrf_http),
            Err(ModuleError::InvalidManifest(_))
        ));

        // Unregister
        let removed = reg.unregister_personal_module("custom-provider").unwrap();
        assert_eq!(removed.id, "custom-provider");
        assert!(reg.get_module("custom-provider").is_none());
    }
}
