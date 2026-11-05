//! Official gateway control plane tools and resources for MCP.

mod definitions;
mod handlers;

pub use definitions::{list_core_resources, list_core_tools};
pub use handlers::{call_core_tool, read_core_resource};
