//! Module source candidate resolution service for personal module upgrades.

use crate::gateway::installer::resolve_local_directory_path;
use crate::gateway::modules::{get_system_module_catalog, ModuleDescriptor};

pub(crate) fn resolve_candidate_source(source: Option<&str>) -> Option<String> {
    let raw = source?.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.starts_with("http://") || raw.starts_with("https://") || raw.starts_with("git@") {
        return Some(raw.to_string());
    }
    resolve_local_directory_path(raw).map(|p| p.to_string_lossy().to_string())
}

pub(crate) fn resolve_upgrade_source_candidate(
    payload_source: Option<&str>,
    existing_desc: &ModuleDescriptor,
    resolved_id: &str,
) -> Option<String> {
    if let Some(explicit) = payload_source.filter(|s| !s.trim().is_empty()) {
        let trimmed = explicit.trim();
        if trimmed.starts_with("http://")
            || trimmed.starts_with("https://")
            || trimmed.starts_with("git@")
        {
            return Some(trimmed.to_string());
        }
        return Some(
            resolve_local_directory_path(trimmed)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| trimmed.to_string()),
        );
    }

    resolve_candidate_source(existing_desc.source.as_deref())
        .or_else(|| {
            get_system_module_catalog()
                .into_iter()
                .find(|c| c.id == resolved_id || c.namespace == existing_desc.namespace)
                .and_then(|c| resolve_candidate_source(c.source.as_deref()))
        })
        .or_else(|| {
            [
                format!("addons/{resolved_id}"),
                format!("modules/{resolved_id}"),
                format!("gateway-modules/modules/{resolved_id}"),
                format!("../gateway-modules/modules/{resolved_id}"),
            ]
            .into_iter()
            .find_map(|rel| {
                resolve_local_directory_path(&rel).map(|p| p.to_string_lossy().to_string())
            })
        })
}
