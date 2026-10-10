//! Shared imports and helpers for the gateway integration test modules.

pub use std::io::{Read, Write};
pub use std::net::{TcpListener, TcpStream};
pub use std::thread;
pub use std::time::Duration;
pub use toolcase_gateway::config::Config;
pub use toolcase_gateway::http::{
    header_value, is_chunked, is_request_target, is_token, parse_headers, read_more,
    read_response_head, Request,
};
pub use toolcase_gateway::omniroute::{open_upstream, RETRYABLE};
pub use toolcase_gateway::rewrite::{
    escape_json_string, json_string_value, replace_model, rewrite_tool_names,
};

pub fn test_config() -> Config {
    Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        default_model: "gpt-5.6-sol".into(),
        museai_base_url: "https://muse.ai".into(),
        museai_cookie: "".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
        jev_api_keys: vec![],
    }
}
