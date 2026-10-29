use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;
use toolcase_gateway::api::mcp::process_json_rpc;
use toolcase_gateway::config::{Config, ConfigStore};
use toolcase_gateway::gateway::modules::ModuleStatus;
use toolcase_gateway::routes::route_request;

fn test_store() -> ConfigStore {
    let tmp_env = std::env::temp_dir().join(format!("mcp_test_env_{}", uuid::Uuid::new_v4()));
    ConfigStore::new(Config::default(), tmp_env)
}

fn read_until(stream: &mut TcpStream, output: &mut String, marker: &str) {
    let mut buffer = [0u8; 1024];
    while !output.contains(marker) {
        let count = stream.read(&mut buffer).expect("SSE read timed out");
        assert!(count > 0, "SSE stream closed before {marker:?}");
        output.push_str(&String::from_utf8_lossy(&buffer[..count]));
    }
}

fn read_until_after(stream: &mut TcpStream, output: &mut String, start: usize, marker: &str) {
    while !output[start..].contains(marker) {
        let mut buffer = [0u8; 1024];
        let count = stream.read(&mut buffer).expect("SSE read timed out");
        assert!(count > 0, "SSE stream closed before {marker:?}");
        output.push_str(&String::from_utf8_lossy(&buffer[..count]));
    }
}

#[test]
fn test_mcp_option_1_stdio_json_rpc() {
    let store = test_store();

    // Initialize request
    let init_req = r#"{"jsonrpc":"2.0","id":"init-1","method":"initialize","params":{}}"#;
    let init_resp = process_json_rpc(init_req, &store).expect("response expected");
    let init_val: serde_json::Value = serde_json::from_str(&init_resp).unwrap();
    assert_eq!(init_val["id"], "init-1");
    assert_eq!(init_val["result"]["serverInfo"]["name"], "toolcase-gateway");

    // List tools request
    let list_req = r#"{"jsonrpc":"2.0","id":"tools-1","method":"tools/list","params":{}}"#;
    let list_resp = process_json_rpc(list_req, &store).expect("response expected");
    let list_val: serde_json::Value = serde_json::from_str(&list_resp).unwrap();
    let tools = list_val["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "gateway_list_modules"));

    // Notification: no response
    let note_req = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
    assert!(process_json_rpc(note_req, &store).is_none());

    // Call gateway_get_system_metrics tool
    let metrics_req = r#"{"jsonrpc":"2.0","id":"metrics-1","method":"tools/call","params":{"name":"gateway_get_system_metrics","arguments":{}}}"#;
    let metrics_resp = process_json_rpc(metrics_req, &store).expect("response expected");
    let metrics_val: serde_json::Value = serde_json::from_str(&metrics_resp).unwrap();
    let text = metrics_val["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("toolcase-gateway"));
    assert!(text.contains("active_connections"));
    assert!(text.contains("total_modules"));
}

#[test]
fn test_mcp_option_2_sse_transport_lifecycle() {
    let store = test_store();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();

    let running = Arc::new(AtomicBool::new(true));
    let r_clone = running.clone();
    let store_clone = store.clone();
    let server_handle = thread::spawn(move || {
        while r_clone.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((client, _)) => {
                    client.set_nonblocking(false).unwrap();
                    let sc = store_clone.clone();
                    thread::spawn(move || {
                        let _ = route_request(client, &sc);
                    });
                }
                Err(_) => {
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
    });

    // 1. Client connects via GET /mcp/sse to establish SSE stream
    let mut sse_client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        sse_client,
        "GET /mcp/sse HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n"
    )
    .unwrap();
    sse_client.flush().unwrap();
    sse_client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();

    // Read headers and initial endpoint event from SSE client
    let mut sse_output = String::new();
    read_until(&mut sse_client, &mut sse_output, "event: endpoint\r\ndata:");
    let endpoint_event_start = sse_output.find("event: endpoint").unwrap();
    read_until_after(
        &mut sse_client,
        &mut sse_output,
        endpoint_event_start,
        "\r\n\r\n",
    );

    assert!(sse_output.contains("HTTP/1.1 200 OK"));
    assert!(sse_output.contains("Content-Type: text/event-stream"));
    assert!(sse_output.contains("event: endpoint"));
    assert!(sse_output.contains("/mcp?sessionId="));

    // Extract sessionId from the event data
    let session_prefix = "data: /mcp?sessionId=";
    let session_idx = sse_output.find(session_prefix).unwrap();
    let after_prefix = &sse_output[session_idx + session_prefix.len()..];
    let session_id = after_prefix.lines().next().unwrap().trim();

    assert!(!session_id.is_empty());

    // 2. Open a separate connection to POST JSON-RPC to the announced endpoint with sessionId
    let (post_tx, post_rx) = mpsc::channel();
    let session_id_str = session_id.to_string();

    thread::spawn(move || {
        let mut post_client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let post_req = format!(
            "POST /mcp?sessionId={session_id_str} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 63\r\nConnection: close\r\n\r\n{{\"jsonrpc\":\"2.0\",\"id\":\"sse-test-1\",\"method\":\"ping\",\"params\":{{}}}}"
        );
        post_client.write_all(post_req.as_bytes()).unwrap();
        post_client.flush().unwrap();

        let mut post_buf = Vec::new();
        post_client.read_to_end(&mut post_buf).unwrap();
        let post_resp = String::from_utf8_lossy(&post_buf);
        post_tx.send(post_resp.to_string()).unwrap();
    });

    let post_response = post_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(post_response.contains("HTTP/1.1 202 Accepted"));

    // 3. Verify the JSON-RPC response was pushed to the open SSE stream
    let message_start = sse_output.len();
    read_until(&mut sse_client, &mut sse_output, "event: message\r\ndata:");
    read_until_after(&mut sse_client, &mut sse_output, message_start, "\r\n\r\n");
    assert!(sse_output.contains("event: message"));
    assert!(sse_output.contains("\"id\":\"sse-test-1\""));
    assert!(sse_output.contains("\"result\":{}"));

    // Drop SSE client and shut down server loop
    drop(sse_client);
    running.store(false, Ordering::Relaxed);
    let _ = server_handle.join();
}

#[test]
fn test_stale_sse_session_rejects_mutating_request_before_dispatch() {
    let store = test_store();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store_clone = store.clone();
    let server = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        let _ = route_request(client, &store_clone);
    });

    let body = r#"{"jsonrpc":"2.0","id":"stale-session","method":"tools/call","params":{"name":"gateway_set_module_state","arguments":{"id":"muse","enabled":false}}}"#;
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "POST /mcp?sessionId=missing-session HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
    client.flush().unwrap();
    let mut response = String::new();
    client.read_to_string(&mut response).unwrap();
    server.join().unwrap();

    assert!(response.starts_with("HTTP/1.1 404 Not Found"));
    let registry = store.module_registry();
    let registry = registry.read().unwrap();
    let muse = registry.get_module("muse").unwrap();
    assert_eq!(muse.status, ModuleStatus::Enabled);
}

#[test]
fn test_mcp_option_3_streamable_http() {
    let store = test_store();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let store_clone = store.clone();
    let server_handle = thread::spawn(move || {
        // Handle 2 requests
        for _ in 0..2 {
            if let Ok((client, _)) = listener.accept() {
                let _ = route_request(client, &store_clone);
            }
        }
    });

    // 1. POST /mcp with standard JSON-RPC request (ping)
    let mut client1 = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let payload = br#"{"jsonrpc":"2.0","id":"http-ping-1","method":"ping","params":{}}"#;
    write!(
        client1,
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )
    .unwrap();
    client1.write_all(payload).unwrap();
    client1.flush().unwrap();

    let mut buf1 = Vec::new();
    client1.read_to_end(&mut buf1).unwrap();
    let resp1 = String::from_utf8_lossy(&buf1);
    assert!(resp1.contains("HTTP/1.1 200 OK"));
    assert!(resp1.contains("Content-Type: application/json"));
    assert!(resp1.contains("\"id\":\"http-ping-1\""));

    // 2. GET /mcp remains a 405 even when a Streamable HTTP client accepts SSE.
    let mut client2 = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client2,
        "GET /mcp HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client2.flush().unwrap();

    let mut buf2 = Vec::new();
    client2.read_to_end(&mut buf2).unwrap();
    let resp2 = String::from_utf8_lossy(&buf2);
    assert!(resp2.contains("HTTP/1.1 405 Method Not Allowed"));
    assert!(resp2.contains("GET /mcp/sse for SSE"));

    let _ = server_handle.join();
}

#[test]
fn test_dashboard_overview_reports_three_mcp_options() {
    let store = test_store();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let store_clone = store.clone();
    let server_handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        let _ = route_request(client, &store_clone);
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "GET /api/v1/dashboard/overview HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();

    let mut buf = Vec::new();
    client.read_to_end(&mut buf).unwrap();
    let resp_str = String::from_utf8_lossy(&buf);
    let body_json = resp_str.split("\r\n\r\n").nth(1).unwrap();
    let val: serde_json::Value = serde_json::from_str(body_json).unwrap();

    let mcp = &val["mcp"];
    assert_eq!(mcp["status"], "enabled");

    let options = mcp["options"].as_array().expect("options array expected");
    assert_eq!(options.len(), 3);

    // Option 1: stdio
    assert_eq!(options[0]["name"], "stdio");
    assert_eq!(options[0]["command"], "toolcase-gateway --mcp");
    assert!(options[0]["description"]
        .as_str()
        .unwrap()
        .contains("toolcase-gateway --mcp"));

    // Option 2: sse
    assert_eq!(options[1]["name"], "sse");
    assert_eq!(options[1]["endpoint"], "/mcp/sse");

    // Option 3: streamable_http
    assert_eq!(options[2]["name"], "streamable_http");
    assert_eq!(options[2]["endpoint"], "/mcp");

    let _ = server_handle.join();
}
