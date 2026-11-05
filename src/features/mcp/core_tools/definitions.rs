//! Core MCP tool definitions and resource schemas.

use crate::api::mcp::protocol::{McpResource, McpTool};
use serde_json::json;

/// Returns the registry of official core MCP management tools.
pub fn list_core_tools() -> Vec<McpTool> {
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
            name: "gateway_upgrade_personal_module".to_string(),
            description: "Upgrade and recompile an installed personal provider module, preserving configuration while atomically replacing the binary.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The personal module identifier to upgrade" },
                    "source": { "type": "string", "description": "Optional upgrade source (GitHub URL, ZIP URL, or local path). If omitted, defaults to the module's registered source repository." },
                    "zip_base64": { "type": "string", "description": "Optional base64-encoded ZIP archive containing the updated module source" },
                    "branch": { "type": "string", "description": "Optional git branch for GitHub sources" }
                },
                "required": ["id"],
                "additionalProperties": false
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

/// Returns the registry of official core MCP resources.
pub fn list_core_resources() -> Vec<McpResource> {
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
