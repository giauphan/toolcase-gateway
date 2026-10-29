//! MCP Server transport adapters: Stdio and Streamable HTTP.

use crate::api::mcp::protocol::{
    JsonRpcRequest, JsonRpcResponse, INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST,
    MCP_PROTOCOL_VERSION, METHOD_NOT_FOUND, PARSE_ERROR,
};
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

/// Handles inbound HTTP requests destined for the MCP control plane (`/mcp`, `/mcp/sse`, `/mcp/messages`, `/sse`).
pub fn handle_http_mcp(
    client: &mut TcpStream,
    request: &Request,
    store: &ConfigStore,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    if clean_path != "/mcp"
        && clean_path != "/mcp/sse"
        && clean_path != "/mcp/messages"
        && clean_path != "/sse"
    {
        return Ok(None);
    }

    let origin_hdr = request
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("origin"))
        .map(|(_, v)| v.as_str());

    let cors_header = if let Some(origin) = origin_hdr {
        if !crate::http::is_loopback_origin(origin) {
            crate::http::write_error(
                client,
                403,
                "Forbidden",
                "Cross-origin requests from untrusted origins are prohibited",
            )?;
            return Ok(Some(403));
        }
        format!("Access-Control-Allow-Origin: {origin}\r\n")
    } else {
        String::new()
    };

    if request.method.eq_ignore_ascii_case("get") {
        let is_sse_requested = clean_path == "/mcp/sse" || clean_path == "/sse";

        if is_sse_requested {
            let status = super::sse::handle_sse_stream(client, &cors_header)?;
            return Ok(Some(status));
        }

        let body = r#"{"error":"GET not allowed on /mcp; use POST /mcp for Streamable HTTP or GET /mcp/sse for SSE"}"#;
        let resp = format!(
            "HTTP/1.1 405 Method Not Allowed\r\nAllow: POST, GET\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            cors_header,
            body.len(),
            body
        );
        client.write_all(resp.as_bytes())?;
        client.flush()?;
        return Ok(Some(405));
    }

    if request.method.eq_ignore_ascii_case("post") {
        if origin_hdr.is_some() {
            let ct_ok = request
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                .is_some_and(|(_, ct)| ct.to_ascii_lowercase().starts_with("application/json"));
            if !ct_ok {
                crate::http::write_error(
                    client,
                    415,
                    "Unsupported Media Type",
                    "MCP endpoint expects Content-Type: application/json",
                )?;
                return Ok(Some(415));
            }
        } else if let Some((_, ct)) = request
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        {
            if !ct.to_ascii_lowercase().starts_with("application/json") {
                crate::http::write_error(
                    client,
                    415,
                    "Unsupported Media Type",
                    "MCP endpoint expects Content-Type: application/json",
                )?;
                return Ok(Some(415));
            }
        }

        let session_id = super::sse::extract_session_id(request);
        if let Some(session_id) = &session_id {
            if !super::sse::has_session(session_id) {
                let body = r#"{"error":"SSE session not found or expired"}"#;
                let resp = format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    cors_header,
                    body.len(),
                    body
                );
                client.write_all(resp.as_bytes())?;
                client.flush()?;
                return Ok(Some(404));
            }
        }

        let req_str = String::from_utf8_lossy(&request.body);
        let resp_json_opt = process_json_rpc(&req_str, store);

        // Check if this request is targeted to an active SSE session
        if let Some(session_id) = session_id {
            if let Some(resp_json) = resp_json_opt {
                if super::sse::send_to_session(&session_id, resp_json) {
                    let resp = format!(
                        "HTTP/1.1 202 Accepted\r\n{}Content-Length: 0\r\nConnection: close\r\n\r\n",
                        cors_header
                    );
                    client.write_all(resp.as_bytes())?;
                    client.flush()?;
                    return Ok(Some(202));
                }
                let body = r#"{"error":"SSE session not found or expired"}"#;
                let resp = format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    cors_header,
                    body.len(),
                    body
                );
                client.write_all(resp.as_bytes())?;
                client.flush()?;
                return Ok(Some(404));
            }
            // Notification under SSE returns 202 Accepted
            let resp = format!(
                "HTTP/1.1 202 Accepted\r\n{}Content-Length: 0\r\nConnection: close\r\n\r\n",
                cors_header
            );
            client.write_all(resp.as_bytes())?;
            client.flush()?;
            return Ok(Some(202));
        }

        // Streamable HTTP: Direct bidirectional HTTP response
        if let Some(resp_json) = resp_json_opt {
            let data = resp_json.as_bytes();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n",
                cors_header,
                data.len()
            );
            client.write_all(head.as_bytes())?;
            client.write_all(data)?;
            client.flush()?;
            return Ok(Some(200));
        }
        // Notifications return 202 Accepted with no body in Streamable HTTP MCP
        let resp = format!(
            "HTTP/1.1 202 Accepted\r\n{}Content-Length: 0\r\nConnection: close\r\n\r\n",
            cors_header
        );
        client.write_all(resp.as_bytes())?;
        client.flush()?;
        return Ok(Some(202));
    }

    let body = r#"{"error":"Method not allowed; use POST /mcp or GET /mcp/sse"}"#;
    let resp = format!(
        "HTTP/1.1 405 Method Not Allowed\r\nAllow: POST, GET\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        cors_header,
        body.len(),
        body
    );
    client.write_all(resp.as_bytes())?;
    client.flush()?;
    Ok(Some(405))
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

        // 6. invalid jsonrpc version without id
        let bad_ver_no_id = r#"{"jsonrpc":"1.0","method":"ping"}"#;
        let bad_resp = process_json_rpc(bad_ver_no_id, &store).unwrap();
        let val: Value = serde_json::from_str(&bad_resp).unwrap();
        assert_eq!(val["error"]["code"], INVALID_REQUEST);
        assert!(val["id"].is_null());
    }
}
