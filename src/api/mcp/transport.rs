//! MCP Server transport adapters: Stdio and Streamable HTTP.

use crate::api::mcp::protocol::*;
use crate::config::ConfigStore;
use crate::http::Request;
use serde_json::json;
use std::io::{self, BufRead, Write};
use std::net::TcpStream;

/// Dispatches a JSON-RPC 2.0 string request and returns the serialized JSON-RPC 2.0 response.
/// Notifications that do not require responses return `None`.
pub fn process_json_rpc(raw_line: &str, store: &ConfigStore) -> Option<String> {
    let req: JsonRpcRequest = match serde_json::from_str(raw_line) {
        Ok(r) => r,
        Err(e) => {
            let resp =
                JsonRpcResponse::error(None, PARSE_ERROR, &format!("Parse error: {e}"), None);
            return Some(serde_json::to_string(&resp).unwrap_or_default());
        }
    };

    if req.jsonrpc != "2.0" {
        req.id.as_ref()?;
        let resp = JsonRpcResponse::error(
            req.id,
            INVALID_REQUEST,
            "Invalid JSON-RPC version: must be '2.0'",
            None,
        );
        return Some(serde_json::to_string(&resp).unwrap_or_default());
    }

    // In JSON-RPC 2.0, notifications (requests without an id) do not return a response.
    req.id.as_ref()?;

    match req.method.as_str() {
        "initialize" => {
            let result = json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "serverInfo": {
                    "name": "toolcase-gateway",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "tools": {},
                    "resources": {}
                }
            });
            let resp = JsonRpcResponse::success(req.id, result);
            Some(serde_json::to_string(&resp).unwrap_or_default())
        }
        "notifications/initialized" => {
            // Notifications do not return a response
            None
        }
        "ping" => {
            let resp = JsonRpcResponse::success(req.id, json!({}));
            Some(serde_json::to_string(&resp).unwrap_or_default())
        }
        "tools/list" => {
            let tools = crate::features::mcp::list_tools();
            let resp = JsonRpcResponse::success(req.id, json!({ "tools": tools }));
            Some(serde_json::to_string(&resp).unwrap_or_default())
        }
        "tools/call" => {
            let params = req.params.unwrap_or_else(|| json!({}));
            let name = match params.get("name").and_then(|v| v.as_str()) {
                Some(n) => n,
                None => {
                    let resp = JsonRpcResponse::error(
                        req.id,
                        INVALID_PARAMS,
                        "Missing tool 'name' in params",
                        None,
                    );
                    return Some(serde_json::to_string(&resp).unwrap_or_default());
                }
            };
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match crate::features::mcp::call_tool(name, &args, store) {
                Ok(val) => {
                    let resp = JsonRpcResponse::success(req.id, val);
                    Some(serde_json::to_string(&resp).unwrap_or_default())
                }
                Err(err_msg) => {
                    let resp = JsonRpcResponse::error(
                        req.id,
                        INTERNAL_ERROR,
                        &format!("Tool execution error: {err_msg}"),
                        None,
                    );
                    Some(serde_json::to_string(&resp).unwrap_or_default())
                }
            }
        }
        "resources/list" => {
            let resources = crate::features::mcp::list_resources();
            let resp = JsonRpcResponse::success(req.id, json!({ "resources": resources }));
            Some(serde_json::to_string(&resp).unwrap_or_default())
        }
        "resources/read" => {
            let params = req.params.unwrap_or_else(|| json!({}));
            let uri = match params.get("uri").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => {
                    let resp = JsonRpcResponse::error(
                        req.id,
                        INVALID_PARAMS,
                        "Missing 'uri' in params",
                        None,
                    );
                    return Some(serde_json::to_string(&resp).unwrap_or_default());
                }
            };
            match crate::features::mcp::read_resource(uri, store) {
                Ok(val) => {
                    let resp = JsonRpcResponse::success(req.id, val);
                    Some(serde_json::to_string(&resp).unwrap_or_default())
                }
                Err(err_msg) => {
                    let resp = JsonRpcResponse::error(
                        req.id,
                        INTERNAL_ERROR,
                        &format!("Resource read error: {err_msg}"),
                        None,
                    );
                    Some(serde_json::to_string(&resp).unwrap_or_default())
                }
            }
        }
        _ => {
            let resp = JsonRpcResponse::error(
                req.id,
                METHOD_NOT_FOUND,
                &format!("Method '{}' not found", req.method),
                None,
            );
            Some(serde_json::to_string(&resp).unwrap_or_default())
        }
    }
}

/// Runs the standard MCP stdio transport loop reading newline-delimited JSON-RPC from stdin.
pub fn run_stdio_transport(store: &ConfigStore) -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let reader = stdin.lock();

    for line_result in reader.lines() {
        let line = line_result?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(resp) = process_json_rpc(&line, store) {
            writeln!(stdout, "{resp}")?;
            stdout.flush()?;
        }
    }

    Ok(())
}

/// Handles inbound HTTP requests destined for the MCP control plane (`/mcp` and `/mcp/sse`).
pub fn handle_http_mcp(
    client: &mut TcpStream,
    request: &Request,
    store: &ConfigStore,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    if clean_path != "/mcp" && clean_path != "/mcp/sse" {
        return Ok(None);
    }

    if request.method.eq_ignore_ascii_case("post") {
        let req_str = String::from_utf8_lossy(&request.body);
        if let Some(resp_json) = process_json_rpc(&req_str, store) {
            let data = resp_json.as_bytes();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                data.len()
            );
            client.write_all(head.as_bytes())?;
            client.write_all(data)?;
            client.flush()?;
            return Ok(Some(200));
        } else {
            // Notifications return 202 Accepted with no body in Streamable HTTP MCP
            let resp = "HTTP/1.1 202 Accepted\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            client.write_all(resp.as_bytes())?;
            client.flush()?;
            return Ok(Some(202));
        }
    }

    if request.method.eq_ignore_ascii_case("get") {
        let info = json!({
            "service": "toolcase-gateway MCP Server",
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "transports": ["stdio", "http-post", "sse"],
            "endpoints": {
                "jsonrpc": "/mcp",
                "sse": "/mcp/sse"
            }
        });
        let data = serde_json::to_vec_pretty(&info).unwrap_or_default();
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            data.len()
        );
        client.write_all(head.as_bytes())?;
        client.write_all(&data)?;
        client.flush()?;
        return Ok(Some(200));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn test_mcp_initialize_and_ping_flow() {
        let store = ConfigStore::new(
            crate::config::Config::default(),
            std::path::PathBuf::from("/tmp/mcp_test.env"),
        );

        // 1. initialize
        let init_req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let init_resp = process_json_rpc(init_req, &store).unwrap();
        let val: Value = serde_json::from_str(&init_resp).unwrap();
        assert_eq!(val["id"], 1);
        assert_eq!(val["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);

        // 2. notifications/initialized
        let note = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
        assert!(process_json_rpc(note, &store).is_none());

        // 3. ping
        let ping = r#"{"jsonrpc":"2.0","id":2,"method":"ping","params":{}}"#;
        let ping_resp = process_json_rpc(ping, &store).unwrap();
        let val: Value = serde_json::from_str(&ping_resp).unwrap();
        assert_eq!(val["id"], 2);

        // 4. tools/list
        let tools_req = r#"{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}"#;
        let tools_resp = process_json_rpc(tools_req, &store).unwrap();
        let val: Value = serde_json::from_str(&tools_resp).unwrap();
        let tools = val["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "gateway_list_modules"));

        // 5. unknown method
        let unk = r#"{"jsonrpc":"2.0","id":4,"method":"unknownMethod","params":{}}"#;
        let unk_resp = process_json_rpc(unk, &store).unwrap();
        let val: Value = serde_json::from_str(&unk_resp).unwrap();
        assert_eq!(val["error"]["code"], METHOD_NOT_FOUND);
    }
}
