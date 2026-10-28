//! MCP API adapters: JSON-RPC 2.0 protocol and transports.

pub mod protocol;
pub mod transport;

pub use protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse, McpResource, McpTool};
pub use transport::{handle_http_mcp, process_json_rpc, run_stdio_transport};
