//! Feature module for the MCP control plane.

#[path = "core_tools/mod.rs"]
pub mod core_tools;
pub mod module_tools;
pub mod tools;

#[cfg(test)]
mod tests;

pub use tools::{call_tool, list_resources, list_tools, read_resource};
