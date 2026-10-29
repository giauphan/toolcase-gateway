//! Input validation and security checks for modules and sub-providers.

use super::manifest::{ModuleDescriptor, ModuleError, ModuleKind, SubProviderManifest};
use std::collections::{HashMap, HashSet};

/// System-reserved prefixes that cannot be claimed by provider modules to protect gateway URLs.
pub const RESERVED_PREFIXES: &[&str] = &[
    "/",
    "/api",
    "/dashboard",
    "/home",
    "/assets",
    "/mcp",
    "/sse",
    "/health",
    "/status",
    "/metrics",
    "/favicon.ico",
    "/sw.js",
    "/manifest.webmanifest",
    "/robots.txt",
    "/video",
    "/v1",
    "/muse-config",
    "/gateway",
    "/gateway-config",
    "/gateway-ui",
    "/video-template",
    "/image-template",
];

/// Known built-in provider prefixes that cannot be claimed or shadowed by personal modules.
pub const BUILTIN_PROVIDER_PREFIXES: &[&str] = &[
    "/ominroute",
    "/omniroute",
    "/muse-ai",
    "/muse",
    "/jev",
    "/jev-ai",
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
    let trimmed = ns.trim().trim_end_matches('/');
    if trimmed.is_empty()
        || !trimmed.starts_with('/')
        || trimmed.len() < 2
        || trimmed.contains("//")
        || trimmed.chars().any(char::is_whitespace)
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '-' || c == '_' || c == '.')
    {
        return Err(ModuleError::InvalidNamespace(raw_ns.to_string()));
    }

    for reserved in RESERVED_PREFIXES {
        if *reserved == "/" {
            if trimmed == "/" {
                return Err(ModuleError::ReservedNamespace(raw_ns.to_string()));
            }
        } else if trimmed == *reserved
            || trimmed.starts_with(&format!("{reserved}/"))
            || reserved.starts_with(&format!("{trimmed}/"))
        {
            return Err(ModuleError::ReservedNamespace(raw_ns.to_string()));
        }
    }

    for builtin in BUILTIN_PROVIDER_PREFIXES {
        if trimmed == *builtin
            || trimmed.starts_with(&format!("{builtin}/"))
            || builtin.starts_with(&format!("{trimmed}/"))
        {
            return Err(ModuleError::NamespaceCollision(raw_ns.to_string()));
        }
    }

    for (other_id, desc) in modules {
        let other_ns = desc.namespace.trim_end_matches('/');
        if other_id != id
            && (other_ns == trimmed
                || trimmed.starts_with(&format!("{other_ns}/"))
                || other_ns.starts_with(&format!("{trimmed}/")))
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

        let clean_subpath = if p.subpath.trim().is_empty() {
            format!("/{trimmed_id}")
        } else {
            let s = p.subpath.trim().trim_end_matches('/');
            if !s.starts_with('/')
                || s.len() < 2
                || s.contains("//")
                || s.chars().any(char::is_whitespace)
                || !s.chars().all(|c| {
                    c.is_ascii_alphanumeric() || c == '/' || c == '-' || c == '_' || c == '.'
                })
            {
                return Err(ModuleError::InvalidManifest(format!(
                    "Sub-provider '{trimmed_id}' must specify a valid subpath prefix starting with '/' (got '{}')",
                    p.subpath
                )));
            }
            s.to_string()
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
    crate::http::is_loopback_origin(ep)
}
