//! Input validation and security checks for modules and sub-providers.

use super::manifest::{ModuleDescriptor, ModuleError, ModuleKind, SubProviderManifest};
use std::collections::{HashMap, HashSet};

/// System-reserved prefixes that cannot be claimed by provider modules.
pub const RESERVED_PREFIXES: &[&str] = &[
    "/api",
    "/dashboard",
    "/assets",
    "/mcp",
    "/health",
    "/favicon.ico",
    "/video",
    "/v1",
    "/muse-config",
    "/gateway-config",
    "/video-template",
    "/image-template",
];

pub fn validate_module_id(
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

pub fn validate_module_namespace(
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

pub fn validate_module_protocol(protocol: &str, entrypoint: &str) -> Result<(), ModuleError> {
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
                    "local-http entrypoint must target loopback (127.0.0.1 or localhost): '{ep}'"
                )));
            }
        }
        other => {
            return Err(ModuleError::InvalidManifest(format!(
                "Unsupported protocol '{other}'. Supported: 'stdio-stream', 'local-http'"
            )));
        }
    }
    Ok(())
}

pub fn validate_sub_providers(providers: &[SubProviderManifest]) -> Result<(), ModuleError> {
    let mut ids = HashSet::new();
    let mut subpaths = HashSet::new();

    for p in providers {
        let trimmed_id = p.id.trim();
        if trimmed_id.is_empty()
            || !trimmed_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(ModuleError::InvalidManifest(format!(
                "Invalid sub-provider id: '{}'",
                p.id
            )));
        }
        if !ids.insert(trimmed_id.to_string()) {
            return Err(ModuleError::InvalidManifest(format!(
                "Duplicate sub-provider id: '{trimmed_id}'"
            )));
        }

        let clean_subpath = if p.subpath.is_empty() {
            format!("/{trimmed_id}")
        } else if !p.subpath.starts_with('/') {
            format!("/{}", p.subpath.trim_end_matches('/'))
        } else {
            p.subpath.trim_end_matches('/').to_string()
        };

        if !subpaths.insert(clean_subpath.clone()) {
            return Err(ModuleError::InvalidManifest(format!(
                "Duplicate sub-provider subpath: '{clean_subpath}'"
            )));
        }
    }
    Ok(())
}

pub fn is_loopback_http_endpoint(ep: &str) -> bool {
    let lower = ep.to_ascii_lowercase();
    (lower.starts_with("http://127.0.0.1")
        || lower.starts_with("http://localhost")
        || lower.starts_with("https://127.0.0.1")
        || lower.starts_with("https://localhost"))
        && !lower.contains('@')
}
