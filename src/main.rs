//! Tool-name casing gateway + failover retry binary.
#![forbid(unsafe_code)]

fn main() -> std::io::Result<()> {
    toolcase_gateway::run()
}
