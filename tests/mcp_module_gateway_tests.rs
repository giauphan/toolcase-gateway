use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;
use toolcase_gateway::api::mcp::process_json_rpc;
use toolcase_gateway::config::{Config, ConfigStore};
use toolcase_gateway::gateway::modules::{
    ModuleGroupManifest, ModuleStatus, ModuleToolDefinition, PersonalManifest, SubProviderManifest,
};
use toolcase_gateway::routes::route_request;

fn test_store() -> ConfigStore {
    let tmp_env = std::env::temp_dir().join(format!("mcp_mod_test_env_{}", uuid::Uuid::new_v4()));
    ConfigStore::new(Config::default(), tmp_env)
}

fn read_until(stream: &mut TcpStream, output: &mut String, marker: &str) {
    let mut buffer = [0u8; 1024];
    while !output.contains(marker) {
        let count = stream.read(&mut buffer).expect("read timed out");
        assert!(count > 0, "stream closed before {marker:?}");
        output.push_str(&String::from_utf8_lossy(&buffer[..count]));
    }
}

fn create_mock_module_script(temp_dir: &std::path::Path) -> std::path::PathBuf {
    let run_sh = temp_dir.join("run.sh");
    let script = r#"#!/bin/sh
read -r line
cat << 'EOF'
{"status": 200, "headers": {"content-type": "application/json"}, "body": "{\"result\": \"success\", \"info\": \"mock mcp response\"}"}
EOF
"#;
    std::fs::write(&run_sh, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&run_sh, std::fs::Permissions::from_mode(0o755));
    }
    run_sh
}

#[test]
fn test_mcp_module_gateway_tools_and_resources_stdio() {
    let store = test_store();
    let temp_dir = std::env::temp_dir().join(format!("mcp_mod_stdio_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let run_sh = create_mock_module_script(&temp_dir);

    // Register module with declared tools
    let manifest = PersonalManifest {
        id: "weather-provider".to_string(),
        name: "Weather Provider".to_string(),
        short_name: Some("Weather".to_string()),
        version: "1.0.0".to_string(),
        namespace: "/custom/weather".to_string(),
        entrypoint: run_sh.to_string_lossy().to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec!["chat".to_string()],
        icon: None,
        config_schema: None,
        permissions: vec![],
        group: None,
        source: None,
        tools: Some(vec![ModuleToolDefinition {
            name: "get_weather".to_string(),
            description: "Get weather for a city".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "city": { "type": "string" } },
                "required": ["city"]
            }),
        }]),
    };

    {
        let reg = store.module_registry();
        let mut guard = reg.write().unwrap();
        guard.register_personal_module(manifest).unwrap();
    }

    // 1. tools/list includes core gateway tools + module tools
    let list_req = r#"{"jsonrpc":"2.0","id":"tools-1","method":"tools/list","params":{}}"#;
    let list_resp = process_json_rpc(list_req, &store).expect("response expected");
    let list_val: serde_json::Value = serde_json::from_str(&list_resp).unwrap();
    let tools = list_val["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "gateway_list_modules"));
    assert!(tools
        .iter()
        .any(|t| t["name"] == "weather-provider__get_weather"));

    // 2. tools/call for the module tool
    let call_req = r#"{"jsonrpc":"2.0","id":"call-1","method":"tools/call","params":{"name":"weather-provider__get_weather","arguments":{"city":"London"}}}"#;
    let call_resp = process_json_rpc(call_req, &store).expect("response expected");
    let call_val: serde_json::Value = serde_json::from_str(&call_resp).unwrap();
    assert_eq!(call_val["id"], "call-1");
    let content = call_val["result"]["content"][0]["text"].as_str().unwrap();
    assert!(content.contains("mock mcp response"));

    // 3. resources/list includes gateway://modules/weather-provider
    let res_req = r#"{"jsonrpc":"2.0","id":"res-1","method":"resources/list","params":{}}"#;
    let res_resp = process_json_rpc(res_req, &store).expect("response expected");
    let res_val: serde_json::Value = serde_json::from_str(&res_resp).unwrap();
    let resources = res_val["result"]["resources"].as_array().unwrap();
    assert!(resources
        .iter()
        .any(|r| r["uri"] == "gateway://modules/weather-provider"));

    // 4. resources/read retrieves module manifest
    let read_req = r#"{"jsonrpc":"2.0","id":"read-1","method":"resources/read","params":{"uri":"gateway://modules/weather-provider"}}"#;
    let read_resp = process_json_rpc(read_req, &store).expect("response expected");
    let read_val: serde_json::Value = serde_json::from_str(&read_resp).unwrap();
    let res_text = read_val["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(res_text.contains("weather-provider"));

    // 5. Disable module and verify tool call and resource read rejection
    {
        let reg = store.module_registry();
        let mut guard = reg.write().unwrap();
        guard
            .set_module_status("weather-provider", ModuleStatus::Disabled)
            .unwrap();
    }
    let call_disabled = process_json_rpc(call_req, &store).expect("response expected");
    let err_val: serde_json::Value = serde_json::from_str(&call_disabled).unwrap();
    assert!(err_val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("is currently disabled"));

    let read_disabled = process_json_rpc(read_req, &store).expect("response expected");
    let read_err_val: serde_json::Value = serde_json::from_str(&read_disabled).unwrap();
    assert!(read_err_val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("is currently disabled"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mcp_module_gateway_subprovider_tools_and_capabilities_synthesis() {
    let store = test_store();
    let temp_dir = std::env::temp_dir().join(format!("mcp_mod_group_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let run_sh = create_mock_module_script(&temp_dir);

    // Register group module with sub-provider tool
    let manifest = PersonalManifest {
        id: "media-suite".to_string(),
        name: "Media Suite".to_string(),
        short_name: Some("Media".to_string()),
        version: "1.0.0".to_string(),
        namespace: "/custom/media".to_string(),
        entrypoint: run_sh.to_string_lossy().to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec![],
        icon: None,
        config_schema: None,
        permissions: vec![],
        group: Some(ModuleGroupManifest {
            enabled: true,
            providers: vec![SubProviderManifest {
                id: "image-gen".to_string(),
                name: "Image Generator".to_string(),
                subpath: "/image".to_string(),
                capabilities: vec!["image".to_string()],
                tools: Some(vec![ModuleToolDefinition {
                    name: "draw".to_string(),
                    description: "Draw image from prompt".to_string(),
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": { "prompt": { "type": "string" } }
                    }),
                }]),
            }],
        }),
        source: None,
        tools: None,
    };

    {
        let reg = store.module_registry();
        let mut guard = reg.write().unwrap();
        guard.register_personal_module(manifest).unwrap();
    }

    let list_req = r#"{"jsonrpc":"2.0","id":"tools-grp","method":"tools/list","params":{}}"#;
    let list_resp = process_json_rpc(list_req, &store).expect("response expected");
    let list_val: serde_json::Value = serde_json::from_str(&list_resp).unwrap();
    let tools = list_val["result"]["tools"].as_array().unwrap();
    assert!(tools
        .iter()
        .any(|t| t["name"] == "media-suite__image-gen__draw"));

    // 2. Execute sub-provider tool
    let call_req = r#"{"jsonrpc":"2.0","id":"sp-call","method":"tools/call","params":{"name":"media-suite__image-gen__draw","arguments":{"prompt":"sunset"}}}"#;
    let call_resp = process_json_rpc(call_req, &store).expect("response expected");
    let call_val: serde_json::Value = serde_json::from_str(&call_resp).unwrap();
    assert_eq!(call_val["id"], "sp-call");
    assert!(call_val["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("mock mcp response"));

    // 3. Reject unadvertised tool name
    let unadv_req = r#"{"jsonrpc":"2.0","id":"unadv","method":"tools/call","params":{"name":"media-suite__nonexistent","arguments":{}}}"#;
    let unadv_resp = process_json_rpc(unadv_req, &store).expect("response expected");
    let unadv_val: serde_json::Value = serde_json::from_str(&unadv_resp).unwrap();
    assert!(unadv_val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Unknown or unadvertised tool"));

    // 4. Built-in without adapter is not advertised and rejected
    assert!(!tools.iter().any(|t| t["name"] == "jev__evaluate"));
    let builtin_req = r#"{"jsonrpc":"2.0","id":"b-call","method":"tools/call","params":{"name":"jev__evaluate","arguments":{}}}"#;
    let builtin_resp = process_json_rpc(builtin_req, &store).expect("response expected");
    let builtin_val: serde_json::Value = serde_json::from_str(&builtin_resp).unwrap();
    assert!(builtin_val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Unknown or unadvertised tool"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mcp_module_tool_call_over_streamable_http() {
    let store = test_store();
    let temp_dir = std::env::temp_dir().join(format!("mcp_http_mod_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let run_sh = create_mock_module_script(&temp_dir);

    let manifest = PersonalManifest {
        id: "calculator-mod".to_string(),
        name: "Calculator Module".to_string(),
        short_name: None,
        version: "1.0.0".to_string(),
        namespace: "/custom/calc".to_string(),
        entrypoint: run_sh.to_string_lossy().to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec![],
        icon: None,
        config_schema: None,
        permissions: vec![],
        group: None,
        source: None,
        tools: Some(vec![ModuleToolDefinition {
            name: "add".to_string(),
            description: "Add two numbers".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "a": { "type": "number" },
                    "b": { "type": "number" }
                }
            }),
        }]),
    };

    {
        let reg = store.module_registry();
        let mut guard = reg.write().unwrap();
        guard.register_personal_module(manifest).unwrap();
    }

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let store_clone = store.clone();
    let server_handle = thread::spawn(move || {
        if let Ok((client, _)) = listener.accept() {
            let _ = route_request(client, &store_clone);
        }
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let payload = br#"{"jsonrpc":"2.0","id":"http-tool-1","method":"tools/call","params":{"name":"calculator-mod__add","arguments":{"a":5,"b":7}}}"#;
    write!(
        client,
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )
    .unwrap();
    client.write_all(payload).unwrap();
    client.flush().unwrap();

    let mut buf = Vec::new();
    client.read_to_end(&mut buf).unwrap();
    let resp = String::from_utf8_lossy(&buf);
    assert!(resp.contains("HTTP/1.1 200 OK"));
    assert!(resp.contains("\"id\":\"http-tool-1\""));
    assert!(resp.contains("mock mcp response"));

    let _ = server_handle.join();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mcp_module_tool_call_over_sse_transport() {
    let store = test_store();
    let temp_dir = std::env::temp_dir().join(format!("mcp_sse_mod_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let run_sh = create_mock_module_script(&temp_dir);

    let manifest = PersonalManifest {
        id: "sse-echo-mod".to_string(),
        name: "SSE Echo Module".to_string(),
        short_name: None,
        version: "1.0.0".to_string(),
        namespace: "/custom/sse-echo".to_string(),
        entrypoint: run_sh.to_string_lossy().to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec![],
        icon: None,
        config_schema: None,
        permissions: vec![],
        group: None,
        source: None,
        tools: Some(vec![ModuleToolDefinition {
            name: "echo_action".to_string(),
            description: "Echo action".to_string(),
            input_schema: serde_json::json!({ "type": "object" }),
        }]),
    };

    {
        let reg = store.module_registry();
        let mut guard = reg.write().unwrap();
        guard.register_personal_module(manifest).unwrap();
    }

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

    // 1. Establish SSE stream
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

    let mut sse_output = String::new();
    read_until(&mut sse_client, &mut sse_output, "data: /mcp?sessionId=");
    let session_idx = sse_output.find("data: /mcp?sessionId=").unwrap();
    let session_prefix_len = "data: /mcp?sessionId=".len();
    let after_prefix = &sse_output[session_idx + session_prefix_len..];
    let session_id = after_prefix.lines().next().unwrap().trim().to_string();

    // 2. POST JSON-RPC tools/call for the module tool to the announced session endpoint
    let (post_tx, post_rx) = mpsc::channel();
    let s_id = session_id.clone();
    thread::spawn(move || {
        let mut post_client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let body = r#"{"jsonrpc":"2.0","id":"sse-mod-call","method":"tools/call","params":{"name":"sse-echo-mod__echo_action","arguments":{}}}"#;
        let post_req = format!(
            "POST /mcp?sessionId={s_id} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        post_client.write_all(post_req.as_bytes()).unwrap();
        post_client.flush().unwrap();
        let mut post_buf = Vec::new();
        post_client.read_to_end(&mut post_buf).unwrap();
        post_tx
            .send(String::from_utf8_lossy(&post_buf).to_string())
            .unwrap();
    });

    let post_resp = post_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(post_resp.contains("HTTP/1.1 202 Accepted"));

    // 3. Verify pushed response event on open SSE stream
    read_until(&mut sse_client, &mut sse_output, "\"id\":\"sse-mod-call\"");
    assert!(sse_output.contains("mock mcp response"));

    drop(sse_client);
    running.store(false, Ordering::Relaxed);
    let _ = server_handle.join();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mcp_module_gateway_local_http_tool_execution() {
    let store = test_store();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let server_handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 46\r\nConnection: close\r\n\r\n{\"content\":[{\"type\":\"text\",\"text\":\"http ok\"}]}";
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    let manifest = PersonalManifest {
        id: "http-provider".to_string(),
        name: "HTTP Provider".to_string(),
        short_name: None,
        version: "1.0.0".to_string(),
        namespace: "/custom/http-provider".to_string(),
        entrypoint: format!("http://127.0.0.1:{port}"),
        protocol: "local-http".to_string(),
        capabilities: vec![],
        icon: None,
        config_schema: None,
        permissions: vec![],
        group: None,
        source: None,
        tools: Some(vec![ModuleToolDefinition {
            name: "fetch_data".to_string(),
            description: "Fetch remote data".to_string(),
            input_schema: serde_json::json!({ "type": "object" }),
        }]),
    };

    {
        let reg = store.module_registry();
        let mut guard = reg.write().unwrap();
        guard.register_personal_module(manifest).unwrap();
    }

    let call_req = r#"{"jsonrpc":"2.0","id":"http-test-1","method":"tools/call","params":{"name":"http-provider__fetch_data","arguments":{}}}"#;
    let call_resp = process_json_rpc(call_req, &store).expect("response expected");
    let call_val: serde_json::Value = serde_json::from_str(&call_resp).unwrap();
    assert_eq!(call_val["id"], "http-test-1");
    let text = call_val["result"]["content"][0]["text"].as_str().unwrap();
    assert_eq!(text, "http ok");

    let _ = server_handle.join();
}
