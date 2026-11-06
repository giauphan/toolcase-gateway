//! Core MCP tool definitions and resource schemas.

use crate::api::mcp::protocol::{McpResource, McpTool};
use serde_json::json;

fn tool(name: &str, description: &str, input_schema: serde_json::Value) -> McpTool {
    McpTool {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn empty_object_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

fn id_object_schema(id_description: &str) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": id_description }
        },
        "required": ["id"],
        "additionalProperties": false
    })
}

fn json_resource(uri: &str, name: &str, description: &str) -> McpResource {
    McpResource {
        uri: uri.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        mime_type: Some("application/json".to_string()),
    }
}

/// Returns the registry of official core MCP management tools.
pub fn list_core_tools() -> Vec<McpTool> {
    vec![
        tool(
            "gateway_list_modules",
            "List all installed provider modules (built-in and personal), their URL namespaces, and enablement status.",
            empty_object_schema(),
        ),
        tool(
            "gateway_get_module_info",
            "Get detailed manifest and configuration information for a specific module by id.",
            id_object_schema("The module identifier (e.g. 'omniroute', 'muse', 'jev')"),
        ),
        tool(
            "gateway_set_module_state",
            "Enable or disable a gateway module dynamically.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The module identifier" },
                    "enabled": { "type": "boolean", "description": "True to enable, false to disable" }
                },
                "required": ["id", "enabled"],
                "additionalProperties": false
            }),
        ),
        tool(
            "gateway_install_personal_module",
            "Install and register an out-of-tree personal provider module manifest at runtime without recompiling the gateway.",
            json!({
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
        ),
        tool(
            "gateway_upgrade_personal_module",
            "Upgrade and recompile an installed personal provider module, preserving configuration while atomically replacing the binary.",
            json!({
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
        ),
        tool(
            "gateway_remove_personal_module",
            "Unregister and remove an installed personal provider module.",
            id_object_schema("The personal module identifier"),
        ),
        tool(
            "gateway_get_system_metrics",
            "Retrieve gateway metrics, admission state, and provider pool health.",
            empty_object_schema(),
        ),
    ]
}

/// Returns the registry of official core MCP resources.
pub fn list_core_resources() -> Vec<McpResource> {
    vec![
        json_resource(
            "gateway://modules",
            "Gateway Installed Modules",
            "Active registry of installed modules, status, and URL namespaces.",
        ),
        json_resource(
            "gateway://routes",
            "Gateway Active Routes",
            "Routing topology and module namespace bindings.",
        ),
        json_resource(
            "gateway://health",
            "Gateway Health and Admission",
            "Gateway health, account pools, and admission controller statistics.",
        ),
    ]
}
