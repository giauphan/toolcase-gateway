//! Feature module for the MCP control plane.

pub mod tools;

#[cfg(test)]
mod tests;

pub use tools::{call_tool, list_resources, list_tools, read_resource};
