//! Application use cases for MCP control plane tools and resources.
//! Facade orchestrating core gateway management tools and module-contributed tools.

use super::core_tools;
use super::module_tools;

pub use super::core_tools::{list_core_resources, list_core_tools};
pub use super::module_tools::{list_module_resources, list_module_tools};
use crate::api::mcp::protocol::{McpResource, McpTool};
use crate::config::ConfigStore;
use serde_json::Value;

/// Returns the complete aggregated registry of core gateway tools and active module tools.
pub fn list_tools(store: &ConfigStore) -> Vec<McpTool> {
    let mut tools = core_tools::list_core_tools();
    tools.extend(module_tools::list_module_tools(store));
    tools
}

/// Returns the complete aggregated registry of core gateway resources and module resources.
pub fn list_resources(store: &ConfigStore) -> Vec<McpResource> {
    let mut resources = core_tools::list_core_resources();
    resources.extend(module_tools::list_module_resources(store));
    resources
}

/// Executes an MCP tool call by name, delegating to core tools or module tools.
pub fn call_tool(name: &str, params: &Value, store: &ConfigStore) -> Result<Value, String> {
    if let Some(res) = core_tools::call_core_tool(name, params, store) {
        return res;
    }
    module_tools::call_module_tool(name, params, store)
}

/// Reads an MCP resource by URI, delegating to core resources or module resources.
pub fn read_resource(uri: &str, store: &ConfigStore) -> Result<Value, String> {
    if let Some(res) = core_tools::read_core_resource(uri, store) {
        return res;
    }
    module_tools::read_module_resource(uri, store)
}
