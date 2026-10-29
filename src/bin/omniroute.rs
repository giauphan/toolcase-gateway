//! OmniRoute binary alias for toolcase-gateway with MCP stdio support.
#![forbid(unsafe_code)]

fn main() -> std::io::Result<()> {
    toolcase_gateway::run()
}
