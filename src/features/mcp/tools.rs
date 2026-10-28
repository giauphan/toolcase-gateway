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
                    "id": { "type": "string", "description": "Unique module identifier" },
                    "name": { "type": "string", "description": "Display name" },
                    "version": { "type": "string", "description": "SemVer version string" },
                    "namespace": { "type": "string", "description": "Public URL path prefix (e.g. '/custom/cohere')" },
                    "entrypoint": { "type": "string", "description": "Executable binary or command to spawn" },
                    "protocol": { "type": "string", "enum": ["stdio-stream", "local-http"], "default": "stdio-stream" },
                    "capabilities": { "type": "array", "items": { "type": "string" } },
                    "permissions": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["id", "name", "version", "namespace", "entrypoint"],
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
            let manifest: PersonalManifest = serde_json::from_value(params.clone())
                .map_err(|e| format!("Invalid personal module manifest parameters: {e}"))?;

            let id = manifest.id.clone();
            {
                let reg = store.module_registry();
                let mut guard = reg.write().map_err(|e| e.to_string())?;
                guard
                    .register_personal_module(manifest)
                    .map_err(|e| e.to_string())?;
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
            let metrics = json!({
                "status": "healthy",
                "target_host": config.target_host,
                "target_port": config.target_port,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_tools_and_resources_lifecycle() {
        let store = ConfigStore::new(
            crate::config::Config::default(),
            std::path::PathBuf::from("/tmp/mcp_tools_test.env"),
        );

        // 1. List tools
        let tools = list_tools();
        assert!(tools.iter().any(|t| t.name == "gateway_list_modules"));
        assert!(tools
            .iter()
            .any(|t| t.name == "gateway_install_personal_module"));

        // 2. Call gateway_list_modules
        let res = call_tool("gateway_list_modules", &json!({}), &store).unwrap();
        let text = res["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("omniroute"));
        assert!(text.contains("muse"));
        assert!(text.contains("jev"));

        // 3. Install personal module
        let install_params = json!({
            "id": "my-claude-agent",
            "name": "Claude Agent Module",
            "version": "1.0.0",
            "namespace": "/personal/claude",
            "entrypoint": "echo",
            "protocol": "stdio-stream",
            "capabilities": ["chat"]
        });
        let install_res =
            call_tool("gateway_install_personal_module", &install_params, &store).unwrap();
        assert!(install_res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("installed"));

        // 4. Verify module info
        let info_res = call_tool(
            "gateway_get_module_info",
            &json!({"id": "my-claude-agent"}),
            &store,
        )
        .unwrap();
        assert!(info_res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("/personal/claude"));

        // 5. Disable module
        let state_res = call_tool(
            "gateway_set_module_state",
            &json!({"id": "my-claude-agent", "enabled": false}),
            &store,
        )
        .unwrap();
        assert!(state_res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Disabled"));

        // 6. Read resource gateway://modules
        let res_mod = read_resource("gateway://modules", &store).unwrap();
        let mod_text = res_mod["contents"][0]["text"].as_str().unwrap();
        assert!(mod_text.contains("my-claude-agent"));

        // 7. Remove personal module
        let remove_res = call_tool(
            "gateway_remove_personal_module",
            &json!({"id": "my-claude-agent"}),
            &store,
        )
        .unwrap();
        assert!(remove_res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("removed"));
    }
}
