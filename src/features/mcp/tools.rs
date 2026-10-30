//! Application use cases for MCP control plane tools and resources.

use crate::api::mcp::protocol::{McpResource, McpTool};
use crate::config::ConfigStore;
use crate::gateway::modules::{ModuleStatus, PersonalManifest};
use serde_json::{json, Value};

/// Returns the registry of official MCP management tools.
pub fn list_tools() -> Vec<McpTool> {
    vec![
        McpTool {
            name: "gateway_list_modules".to_string(),
            description: "List all installed provider modules (built-in and personal), their URL namespaces, and enablement status.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        McpTool {
            name: "gateway_get_module_info".to_string(),
            description: "Get detailed manifest and configuration information for a specific module by id.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The module identifier (e.g. 'omniroute', 'muse', 'jev')" }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        },
        McpTool {
            name: "gateway_set_module_state".to_string(),
            description: "Enable or disable a gateway module dynamically.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The module identifier" },
                    "enabled": { "type": "boolean", "description": "True to enable, false to disable" }
                },
                "required": ["id", "enabled"],
                "additionalProperties": false
            }),
        },
        McpTool {
            name: "gateway_install_personal_module".to_string(),
            description: "Install and register an out-of-tree personal provider module manifest at runtime without recompiling the gateway.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "GitHub URL, ZIP URL, or local path" },
                    "zip_base64": { "type": "string", "description": "Base64-encoded ZIP archive" },
                    "branch": { "type": "string", "description": "Git branch for GitHub sources" },
                    "id": { "type": "string", "description": "Unique module identifier" },
                    "name": { "type": "string", "description": "Display name" },
                    "version": { "type": "string", "description": "SemVer version string" },
                    "namespace": { "type": "string", "description": "Public URL path prefix (e.g. '/custom/cohere')" },
                    "entrypoint": { "type": "string", "description": "Executable binary or command to spawn" },
                    "protocol": { "type": "string", "enum": ["stdio-stream", "local-http"], "default": "stdio-stream" },
                    "capabilities": { "type": "array", "items": { "type": "string" } },
                    "permissions": { "type": "array", "items": { "type": "string" } }
                },
                "additionalProperties": true
            }),
        },
        McpTool {
            name: "gateway_remove_personal_module".to_string(),
            description: "Unregister and remove an installed personal provider module.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The personal module identifier" }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        },
        McpTool {
            name: "gateway_get_system_metrics".to_string(),
            description: "Retrieve gateway metrics, admission state, and provider pool health.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
    ]
}

/// Returns the registry of official MCP resources.
pub fn list_resources() -> Vec<McpResource> {
    vec![
        McpResource {
            uri: "gateway://modules".to_string(),
            name: "Gateway Installed Modules".to_string(),
            description: "Active registry of installed modules, status, and URL namespaces."
                .to_string(),
            mime_type: Some("application/json".to_string()),
        },
        McpResource {
            uri: "gateway://routes".to_string(),
            name: "Gateway Active Routes".to_string(),
            description: "Routing topology and module namespace bindings.".to_string(),
            mime_type: Some("application/json".to_string()),
        },
        McpResource {
            uri: "gateway://health".to_string(),
            name: "Gateway Health and Admission".to_string(),
            description: "Gateway health, account pools, and admission controller statistics."
                .to_string(),
            mime_type: Some("application/json".to_string()),
        },
    ]
}

/// Executes an MCP tool call by name.
pub fn call_tool(name: &str, params: &Value, store: &ConfigStore) -> Result<Value, String> {
    match name {
        "gateway_list_modules" => {
            let reg = store.module_registry();
            let guard = reg.read().map_err(|e| e.to_string())?;
            let modules = guard.list_modules();
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&modules).unwrap_or_default()
                }]
            }))
        }
        "gateway_get_module_info" => {
            let id = params
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing parameter 'id'")?;
            let reg = store.module_registry();
            let guard = reg.read().map_err(|e| e.to_string())?;
            let module = guard
                .get_module(id)
                .ok_or_else(|| format!("Module '{id}' not found"))?;
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
        "gateway_set_module_state" => {
            let id = params
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing parameter 'id'")?;
            let enabled = params
                .get("enabled")
                .and_then(|v| v.as_bool())
                .ok_or("Missing parameter 'enabled'")?;
            let status = if enabled {
                ModuleStatus::Enabled
            } else {
                ModuleStatus::Disabled
            };

            {
                let reg = store.module_registry();
                let mut guard = reg.write().map_err(|e| e.to_string())?;
                guard
                    .set_module_status(id, status)
                    .map_err(|e| e.to_string())?;
            }
            store
                .save_to_persistent_file()
                .map_err(|e| format!("Module state updated but persistence failed: {e}"))?;

            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Module '{id}' state updated to {:?}", status)
                }]
            }))
        }
        "gateway_install_personal_module" => {
            let opts = extract_install_options(params);
            let state_dir = crate::gateway::installer::get_state_dir();
            let source_opt = if let Some(src) = params.get("source").and_then(|v| v.as_str()) {
                Some(crate::gateway::installer::resolve_source(src))
            } else if let Some(b64) = params.get("zip_base64").and_then(|v| v.as_str()) {
                let bytes = crate::gateway::installer::decode_base64(b64)
                    .map_err(|e| format!("Invalid zip_base64: {e}"))?;
                Some(crate::gateway::installer::InstallSource::ZipBytes(bytes))
            } else {
                None
            };

            let installed_from_source = source_opt.is_some();
            let manifest = if let Some(source) = source_opt {
                crate::gateway::installer::install_module(source, opts, &state_dir)
                    .map_err(|e| format!("Module installation failed: {e}"))?
            } else {
                serde_json::from_value::<PersonalManifest>(params.clone())
                    .map_err(|e| format!("Invalid personal module manifest parameters: {e}"))?
            };

            let id = manifest.id.clone();
            {
                let reg = store.module_registry();
                let mut guard = reg.write().map_err(|e| e.to_string())?;
                if let Err(e) = guard.register_personal_module(manifest) {
                    drop(guard);
                    if installed_from_source {
                        crate::gateway::installer::cleanup_module_files(&id, &state_dir);
                    }
                    return Err(e.to_string());
                }
            }
            store
                .save_to_persistent_file()
                .map_err(|e| format!("Module state updated but persistence failed: {e}"))?;

            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Personal module '{id}' installed and registered successfully")
                }]
            }))
        }
        "gateway_remove_personal_module" => {
            let id = params
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing parameter 'id'")?;
            {
                let reg = store.module_registry();
                let mut guard = reg.write().map_err(|e| e.to_string())?;
                guard
                    .unregister_personal_module(id)
                    .map_err(|e| e.to_string())?;
            }
            let state_dir = crate::gateway::installer::get_state_dir();
            crate::gateway::installer::cleanup_module_files(id, &state_dir);
            store
                .save_to_persistent_file()
                .map_err(|e| format!("Module state updated but persistence failed: {e}"))?;

            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Personal module '{id}' removed successfully")
                }]
            }))
        }
        "gateway_get_system_metrics" => {
            let config = store.snapshot();
            let admission = store.admission();
            let registry = store.module_registry();
            let reg = registry
                .read()
                .map_err(|e| format!("Module registry unavailable: {e}"))?;
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
        _ => Err(format!("Unknown tool '{name}'")),
    }
}

/// Reads resource contents by URI.
pub fn read_resource(uri: &str, store: &ConfigStore) -> Result<Value, String> {
    match uri {
        "gateway://modules" => {
            let reg = store.module_registry();
            let guard = reg.read().map_err(|e| e.to_string())?;
            let modules = guard.list_modules();
            Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": serde_json::to_string_pretty(&modules).unwrap_or_default()
                }]
            }))
        }
        "gateway://routes" => {
            let reg = store.module_registry();
            let guard = reg.read().map_err(|e| e.to_string())?;
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
            Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": serde_json::to_string_pretty(&routes).unwrap_or_default()
                }]
            }))
        }
        "gateway://health" => {
            let health = json!({
                "status": "pass",
                "version": env!("CARGO_PKG_VERSION"),
                "zero_async": true,
                "fail_closed": true
            });
            Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": serde_json::to_string_pretty(&health).unwrap_or_default()
                }]
            }))
        }
        _ => Err(format!("Resource '{uri}' not found")),
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
