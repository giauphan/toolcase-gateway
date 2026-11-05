//! Execution handlers for core gateway MCP tools and resources.

use crate::config::ConfigStore;
use crate::gateway::modules::{ModuleStatus, PersonalManifest};
use serde_json::{json, Value};

/// Handles execution of core gateway tools, returning None if the tool is not a core tool.
pub fn call_core_tool(
    name: &str,
    params: &Value,
    store: &ConfigStore,
) -> Option<Result<Value, String>> {
    match name {
        "gateway_list_modules" => Some(handle_list_modules_tool(store)),
        "gateway_get_module_info" => Some(handle_get_module_info_tool(params, store)),
        "gateway_set_module_state" => Some(handle_set_module_state_tool(params, store)),
        "gateway_install_personal_module" => {
            Some(handle_install_personal_module_tool(params, store))
        }
        "gateway_upgrade_personal_module" => Some(handle_upgrade_personal_module(params, store)),
        "gateway_remove_personal_module" => Some(handle_remove_personal_module_tool(params, store)),
        "gateway_get_system_metrics" => Some(handle_get_system_metrics_tool(store)),
        _ => None,
    }
}

fn handle_list_modules_tool(store: &ConfigStore) -> Result<Value, String> {
    let reg = store.module_registry();
    let guard = match reg.read() {
        Ok(g) => g,
        Err(e) => return Err(e.to_string()),
    };
    let modules = guard.list_modules();
    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&modules).unwrap_or_default()
        }]
    }))
}

fn handle_get_module_info_tool(params: &Value, store: &ConfigStore) -> Result<Value, String> {
    let id = match params.get("id").and_then(|v| v.as_str()) {
        Some(i) => i,
        None => return Err("Missing parameter 'id'".to_string()),
    };
    let reg = store.module_registry();
    let guard = match reg.read() {
        Ok(g) => g,
        Err(e) => return Err(e.to_string()),
    };
    let module = match guard.get_module(id) {
        Some(m) => m,
        None => return Err(format!("Module '{id}' not found")),
    };
    let personal_manifest = guard.get_personal_manifest(id);

    let result = json!({
        "module": module,
        "personal_manifest": personal_manifest
    });
    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&result).unwrap_or_default()
        }]
    }))
}

fn handle_set_module_state_tool(params: &Value, store: &ConfigStore) -> Result<Value, String> {
    let id = match params.get("id").and_then(|v| v.as_str()) {
        Some(i) => i,
        None => return Err("Missing parameter 'id'".to_string()),
    };
    let enabled = match params.get("enabled").and_then(|v| v.as_bool()) {
        Some(b) => b,
        None => return Err("Missing parameter 'enabled'".to_string()),
    };
    let status = if enabled {
        ModuleStatus::Enabled
    } else {
        ModuleStatus::Disabled
    };

    {
        let reg = store.module_registry();
        let mut guard = match reg.write() {
            Ok(g) => g,
            Err(e) => return Err(e.to_string()),
        };
        if let Err(e) = guard.set_module_status(id, status) {
            return Err(e.to_string());
        }
    }
    if let Err(e) = store.save_to_persistent_file() {
        return Err(format!("Module state updated but persistence failed: {e}"));
    }

    Ok(json!({
        "content": [{
            "type": "text",
            "text": format!("Module '{id}' state updated to {:?}", status)
        }]
    }))
}

fn handle_install_personal_module_tool(
    params: &Value,
    store: &ConfigStore,
) -> Result<Value, String> {
    let opts = extract_install_options(params);
    let state_dir = crate::gateway::installer::get_state_dir();
    let source_opt = if let Some(src) = params.get("source").and_then(|v| v.as_str()) {
        Some(crate::gateway::installer::resolve_source(src))
    } else if let Some(b64) = params.get("zip_base64").and_then(|v| v.as_str()) {
        let bytes = match crate::gateway::installer::decode_base64(b64) {
            Ok(b) => b,
            Err(e) => return Err(format!("Invalid zip_base64: {e}")),
        };
        Some(crate::gateway::installer::InstallSource::ZipBytes(bytes))
    } else {
        None
    };

    let installed_from_source = source_opt.is_some();
    let manifest = if let Some(source) = source_opt {
        match crate::gateway::installer::install_module(source, opts, &state_dir) {
            Ok(m) => m,
            Err(e) => return Err(format!("Module installation failed: {e}")),
        }
    } else {
        match serde_json::from_value::<PersonalManifest>(params.clone()) {
            Ok(m) => m,
            Err(e) => return Err(format!("Invalid personal module manifest parameters: {e}")),
        }
    };

    let id = manifest.id.clone();
    {
        let reg = store.module_registry();
        let mut guard = match reg.write() {
            Ok(g) => g,
            Err(e) => return Err(e.to_string()),
        };
        if let Err(e) = guard.register_personal_module(manifest.clone()) {
            drop(guard);
            if installed_from_source {
                crate::gateway::installer::cleanup_module_files(&id, &state_dir);
            }
            return Err(e.to_string());
        }
    }
    let _ = store.storage.save_module_with_status(&manifest, true);
    if let Err(e) = store.save_to_persistent_file() {
        return Err(format!("Module state updated but persistence failed: {e}"));
    }

    Ok(json!({
        "content": [{
            "type": "text",
            "text": format!("Personal module '{id}' installed and registered successfully")
        }]
    }))
}

fn handle_remove_personal_module_tool(
    params: &Value,
    store: &ConfigStore,
) -> Result<Value, String> {
    let id = match params.get("id").and_then(|v| v.as_str()) {
        Some(i) => i,
        None => return Err("Missing parameter 'id'".to_string()),
    };
    {
        let reg = store.module_registry();
        let mut guard = match reg.write() {
            Ok(g) => g,
            Err(e) => return Err(e.to_string()),
        };
        if let Err(e) = guard.unregister_personal_module(id) {
            return Err(e.to_string());
        }
    }
    let state_dir = crate::gateway::installer::get_state_dir();
    crate::gateway::installer::cleanup_module_files(id, &state_dir);
    let _ = store.storage.remove_module(id);
    if let Err(e) = store.save_to_persistent_file() {
        return Err(format!("Module state updated but persistence failed: {e}"));
    }

    Ok(json!({
        "content": [{
            "type": "text",
            "text": format!("Personal module '{id}' removed successfully")
        }]
    }))
}

fn handle_get_system_metrics_tool(store: &ConfigStore) -> Result<Value, String> {
    let config = store.snapshot();
    let admission = store.admission();
    let registry = store.module_registry();
    let reg = match registry.read() {
        Ok(r) => r,
        Err(e) => return Err(format!("Module registry unavailable: {e}")),
    };
    let list = reg.list_modules();
    let total_modules = list.len();
    let enabled_modules = list
        .iter()
        .filter(|m| m.status == crate::gateway::modules::ModuleStatus::Enabled)
        .count();
    let metrics = json!({
        "status": "healthy",
        "system": "toolcase-gateway",
        "version": env!("CARGO_PKG_VERSION"),
        "target_host": config.target_host,
        "target_port": config.target_port,
        "active_connections": admission.active_connections(),
        "max_connections": admission.max_connections(),
        "in_flight_bytes": admission.in_flight_bytes(),
        "max_in_flight_bytes": admission.max_in_flight_bytes(),
        "total_modules": total_modules,
        "enabled_modules": enabled_modules,
        "jev_accounts_count": store.jev_pool().len(),
        "jev_exhausted_count": store.jev_pool().exhausted_accounts_count(),
        "openrouter_accounts_count": store.jev_pool().openrouter_accounts().len(),
    });
    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&metrics).unwrap_or_default()
        }]
    }))
}

/// Upgrades an installed personal provider module, maintaining config and persistent data.
fn handle_upgrade_personal_module(params: &Value, store: &ConfigStore) -> Result<Value, String> {
    let id = match params.get("id").and_then(|v| v.as_str()).map(|s| s.trim()) {
        Some(i) if !i.is_empty() => i,
        _ => return Err("Missing parameter 'id'".to_string()),
    };

    let reg = store.module_registry();
    let (existing_desc, registered_mods) = {
        let read_guard = match reg.read() {
            Ok(g) => g,
            Err(e) => return Err(format!("Module registry read error: {e}")),
        };
        let desc = match read_guard.get_module(id) {
            Some(d) => d.clone(),
            None => return Err(format!("Module '{id}' is not installed; cannot upgrade")),
        };
        if desc.kind == crate::gateway::modules::ModuleKind::BuiltIn {
            return Err(format!(
                "Built-in module '{id}' cannot be upgraded via this tool"
            ));
        }
        (desc, read_guard.modules().clone())
    };

    let _upgrade_guard = match crate::features::dashboard::try_acquire_upgrade(id) {
        Ok(g) => g,
        Err(msg) => return Err(format!("Upgrade conflict: {msg}")),
    };

    let install_source = if let Some(b64) = params.get("zip_base64").and_then(|v| v.as_str()) {
        let bytes = match crate::gateway::installer::decode_base64(b64) {
            Ok(b) => b,
            Err(e) => return Err(format!("Invalid zip_base64: {e}")),
        };
        crate::gateway::installer::InstallSource::ZipBytes(bytes)
    } else {
        let explicit_source = params.get("source").and_then(|v| v.as_str());
        let candidate = crate::features::dashboard::resolve_upgrade_source_candidate(
            explicit_source,
            &existing_desc,
            id,
        );
        let raw_source = match candidate {
            Some(s) => s,
            None => {
                return Err(format!(
                    "Module '{id}' has no configured source repository to upgrade from. Please provide a 'source' parameter."
                ))
            }
        };
        crate::gateway::installer::resolve_source(&raw_source)
    };

    let branch = params
        .get("branch")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let opts = crate::gateway::installer::InstallOptions {
        id: Some(id.to_string()),
        name: None,
        namespace: None,
        entrypoint: None,
        protocol: None,
        branch,
    };

    let state_dir = crate::gateway::installer::get_state_dir();
    let new_manifest = match crate::gateway::installer::upgrade_module(
        id,
        install_source,
        opts,
        &state_dir,
        Some(&registered_mods),
    ) {
        Ok(m) => m,
        Err(e) => return Err(format!("Module upgrade failed: {e}")),
    };

    let updated_desc = {
        let mut write_guard = match reg.write() {
            Ok(g) => g,
            Err(e) => return Err(format!("Module registry write error: {e}")),
        };
        match write_guard.upgrade_personal_module(new_manifest.clone()) {
            Ok(d) => d,
            Err(e) => return Err(e.to_string()),
        }
    };

    let _ = store.storage.save_module(&new_manifest);
    if let Err(e) = store.save_to_persistent_file() {
        return Err(format!("Module upgraded but persistence failed: {e}"));
    }

    let version_str = updated_desc.version.as_deref().unwrap_or("unknown");
    Ok(json!({
        "content": [{
            "type": "text",
            "text": format!("Personal module '{id}' upgraded successfully to version {version_str}")
        }],
        "module": updated_desc,
        "manifest": new_manifest
    }))
}

/// Reads official core gateway resources by URI.
pub fn read_core_resource(uri: &str, store: &ConfigStore) -> Option<Result<Value, String>> {
    match uri {
        "gateway://modules" => {
            let reg = store.module_registry();
            let guard = match reg.read() {
                Ok(g) => g,
                Err(e) => return Some(Err(e.to_string())),
            };
            let modules = guard.list_modules();
            Some(Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": serde_json::to_string_pretty(&modules).unwrap_or_default()
                }]
            })))
        }
        "gateway://routes" => {
            let reg = store.module_registry();
            let guard = match reg.read() {
                Ok(g) => g,
                Err(e) => return Some(Err(e.to_string())),
            };
            let routes: Vec<_> = guard
                .list_modules()
                .into_iter()
                .map(|m| {
                    json!({
                        "id": m.id,
                        "namespace": m.namespace,
                        "status": m.status,
                        "kind": m.kind
                    })
                })
                .collect();
            Some(Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": serde_json::to_string_pretty(&routes).unwrap_or_default()
                }]
            })))
        }
        "gateway://health" => {
            let health = json!({
                "status": "pass",
                "version": env!("CARGO_PKG_VERSION"),
                "zero_async": true,
                "fail_closed": true
            });
            Some(Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": serde_json::to_string_pretty(&health).unwrap_or_default()
                }]
            })))
        }
        _ => None,
    }
}

fn extract_install_options(params: &Value) -> crate::gateway::installer::InstallOptions {
    let opt_str = |key| {
        params
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    crate::gateway::installer::InstallOptions {
        id: opt_str("id"),
        name: opt_str("name"),
        namespace: opt_str("namespace"),
        entrypoint: opt_str("entrypoint"),
        protocol: opt_str("protocol"),
        branch: opt_str("branch"),
    }
}
