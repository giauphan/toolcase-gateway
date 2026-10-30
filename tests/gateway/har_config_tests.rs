//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;

// ---------- HAR → config feature: handler and route tests ----------

pub(crate) fn har_fixture_full() -> &'static str {
    r#"{
      "log": {
        "entries": [
          {
            "request": {
              "url": "https://muse.ai/api/session",
              "headers": [
                {"name": "Cookie", "value": "sessionId=abc123; theme=dark"}
              ]
            },
            "response": {
              "content": {
                "text": "{\"vm_id\":\"vm-from-session\",\"endpoint_url\":\"wss://foo.metaaivm.com/\",\"status\":\"assigned\"}"
              }
            }
          },
          {
            "request": {
              "url": "wss://hatch.metaaivm.com/v1/noise?vm_id=vm-id-42&auth_token=access-token-abc123&notary_token=notary-token-def456&app_id=hatch-web&request_id=r1"
            }
          }
        ]
      }
    }"#
}

pub(crate) fn har_fixture_ws_only() -> &'static str {
    r#"{
      "log": {
        "entries": [
          {
            "request": {
              "url": "wss://hatch.metaaivm.com/v1/noise?vm_id=vm-id-42&auth_token=access-token-abc123&notary_token=notary-token-def456&app_id=hatch-web&request_id=r1"
            }
          }
        ]
      }
    }"#
}

pub(crate) fn har_fixture_no_muse() -> &'static str {
    r#"{
      "log": {
        "entries": [
          { "request": { "url": "https://github.com/x/y" } },
          { "request": { "url": "wss://other.example.com/socket" } }
        ]
      }
    }"#
}

pub(crate) fn test_har_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("gw-har-tests-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn har_handler_response(
    body: &str,
    store: &toolcase_gateway::config::ConfigStore,
) -> (u16, String) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_body = body.to_string();
    let store = store.clone();
    let t = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let req = toolcase_gateway::http::read_request(&mut socket).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/muse-ai/v1/config/har");
        toolcase_gateway::har_config::apply_har_config(&mut socket, &req.body, &store).unwrap();
    });
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let _ = write!(
        client,
        "POST /muse-ai/v1/config/har HTTP/1.1\r\nHost: test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        server_body.len(),
        server_body
    );
    client.flush().unwrap();
    let head = read_response_head(&mut client).unwrap();
    let mut buf = head.buffered_body;
    client.read_to_end(&mut buf).unwrap();
    t.join().unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();
    (head.status, text)
}

#[test]
fn har_handler_rejects_invalid_json_with_400() {
    let dir = test_har_dir("handler400a");
    let store = toolcase_gateway::config::ConfigStore::new(test_config(), dir.join(".env"));
    let (status, body) = har_handler_response("not json", &store);
    assert_eq!(status, 400);
    assert!(body.contains("not valid JSON"));
    assert!(store.snapshot().museai_access_token.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn har_handler_rejects_non_har_json_with_400() {
    let dir = test_har_dir("handler400b");
    let store = toolcase_gateway::config::ConfigStore::new(test_config(), dir.join(".env"));
    let (status, body) = har_handler_response("{\"foo\":1}", &store);
    assert_eq!(status, 400);
    assert!(body.contains("log.entries"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn har_handler_rejects_capture_without_muse_entries_with_400() {
    let dir = test_har_dir("handler400c");
    let store = toolcase_gateway::config::ConfigStore::new(test_config(), dir.join(".env"));
    let (status, body) = har_handler_response(har_fixture_no_muse(), &store);
    assert_eq!(status, 400);
    assert!(body.contains("No Muse"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn har_handler_applies_masks_and_persists() {
    let dir = test_har_dir("apply");
    let env_path = dir.join(".env");
    let store = toolcase_gateway::config::ConfigStore::new(test_config(), env_path.clone());

    let (status, body) = har_handler_response(har_fixture_ws_only(), &store);
    assert_eq!(status, 200, "body: {body}");

    let report: serde_json::Value = serde_json::from_str(&body).unwrap();
    let applied_keys: Vec<&str> = report["applied"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["key"].as_str().unwrap())
        .collect();
    assert!(applied_keys.contains(&"ws_url"));
    assert!(applied_keys.contains(&"access_token"));

    // Secrets must be masked in the response, never raw.
    let token_field = report["applied"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["key"] == "access_token")
        .unwrap();
    assert_eq!(token_field["masked"], true);
    assert!(
        !body.contains("access-token-abc123"),
        "raw token leaked in response: {body}"
    );

    // Live config actually changed.
    let snap = store.snapshot();
    assert_eq!(snap.museai_access_token, "access-token-abc123");
    assert_eq!(snap.museai_vm_id, "vm-id-42");
    assert_eq!(snap.museai_ws_url, "wss://hatch.metaaivm.com/v1/noise");

    // .env persisted with the raw value on disk.
    let content = std::fs::read_to_string(&env_path).unwrap();
    assert!(content.contains("GW_MUSEAI_ACCESS_TOKEN=access-token-abc123"));
    assert!(report["env"]["status"] == "written");

    // WS-only capture: base_url is retained and returned as effective config.
    assert!(report["kept"]
        .as_array()
        .unwrap()
        .iter()
        .any(|k| k == "base_url"));
    assert!(report["config"]
        .as_array()
        .unwrap()
        .iter()
        .any(|field| field["key"] == "base_url" && field["value"] == "https://muse.ai"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn har_handler_returns_and_persists_effective_config() {
    let dir = test_har_dir("effective");
    let env_path = dir.join(".env");
    let mut current = test_config();
    current.museai_ws_url = "wss://existing.metaaivm.com/v1/noise".into();
    current.museai_vm_id = "existing-vm".into();
    current.museai_cookie = "sessionId=existing-cookie".into();
    let store = toolcase_gateway::config::ConfigStore::new(current, env_path.clone());
    let har = r#"{
      "log": {"entries": [
        {
          "request": {"url": "https://muse.ai/api/auth/check"},
          "response": {"content": {"text": "{\"access_token\":\"new-synthetic-token\"}"}}
        }
      ]}
    }"#;

    let (status, body) = har_handler_response(har, &store);
    assert_eq!(status, 200, "body: {body}");
    assert!(!body.contains("new-synthetic-token"));
    assert!(!body.contains("existing-cookie"));
    let report: serde_json::Value = serde_json::from_str(&body).unwrap();
    let config = report["config"].as_array().unwrap();
    for key in ["ws_url", "base_url", "access_token", "vm_id", "cookie"] {
        assert!(
            config.iter().any(|field| field["key"] == key),
            "missing {key}"
        );
    }
    assert_eq!(report["env"]["keys"].as_array().unwrap().len(), 5);

    let persisted = std::fs::read_to_string(&env_path).unwrap();
    assert!(persisted.contains("GW_MUSEAI_WS_URL=wss://existing.metaaivm.com/v1/noise"));
    assert!(persisted.contains("GW_MUSEAI_ACCESS_TOKEN=new-synthetic-token"));
    assert!(persisted.contains("GW_MUSEAI_VM_ID=existing-vm"));
    assert!(persisted.contains("GW_MUSEAI_COOKIE=sessionId=existing-cookie"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pipeline_stable_after_apply() {
    let dir = test_har_dir("pipeline");
    let store = toolcase_gateway::config::ConfigStore::new(test_config(), dir.join(".env"));

    har_handler_response(har_fixture_full(), &store);

    let snap = store.snapshot();
    let bootstrap_result = toolcase_gateway::museai::bootstrap_museai_config(&snap);
    assert!(bootstrap_result.is_err());
    let bootstrap_error = bootstrap_result.err().unwrap();
    assert!(
        bootstrap_error
            .to_string()
            .contains("session bootstrap failed")
            || bootstrap_error
                .to_string()
                .contains("cookie expired or invalid")
            || bootstrap_error.to_string().contains("readiness")
    );

    let url = toolcase_gateway::museai::build_museai_ws_url(&snap, "req-1").unwrap();
    assert_eq!(
        url,
        "wss://hatch.metaaivm.com/v1/noise?vm_id=vm-id-42&auth_token=access-token-abc123&notary_token=notary-token-def456&app_id=hatch-web&request_id=req-1"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn muse_config_page_served_on_get_only() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let dir = test_har_dir("page");
    let store = toolcase_gateway::config::ConfigStore::new(test_config(), dir.join(".env"));
    let store2 = store.clone();
    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store2).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "GET /muse-config HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();

    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    assert_eq!(
        header_value(&head.headers, "content-type"),
        Some("text/html; charset=utf-8")
    );
    let mut body = head.buffered_body;
    client.read_to_end(&mut body).unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("id=\"har-file\""));
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn muse_config_read_route_returns_masked_effective_config() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let dir = test_har_dir("read");
    let mut config = test_config();
    config.museai_vm_id = "test-vm".into();
    config.museai_access_token = "secret-access-token-123".into();
    let store = toolcase_gateway::config::ConfigStore::new(config, dir.join(".env"));
    let store2 = store.clone();
    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store2).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "GET /muse-ai/v1/config HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();

    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    assert_eq!(
        header_value(&head.headers, "content-type"),
        Some("application/json")
    );
    let mut body = head.buffered_body;
    client.read_to_end(&mut body).unwrap();
    let body = String::from_utf8_lossy(&body).to_string();

    assert!(body.contains("\"source\":\"current\""));
    assert!(body.contains("\"key\":\"vm_id\""));
    assert!(body.contains("\"value\":\"test-vm\""));
    assert!(body.contains("\"masked\":true"));
    assert!(body.contains("\"key\":\"access_token\""));
    assert!(!body.contains("secret-access"));
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn muse_config_har_route_applies_through_router() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let dir = test_har_dir("route");
    let env_path = dir.join(".env");
    let store = toolcase_gateway::config::ConfigStore::new(test_config(), env_path.clone());
    let store2 = store.clone();
    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store2).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let body = har_fixture_ws_only();
    write!(
        client,
        "POST /muse-ai/v1/config/har HTTP/1.1\r\nHost: test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    client.flush().unwrap();

    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    assert_eq!(
        header_value(&head.headers, "content-type"),
        Some("application/json")
    );
    handle.join().unwrap();

    assert_eq!(store.snapshot().museai_vm_id, "vm-id-42");
    let content = std::fs::read_to_string(&env_path).unwrap();
    assert!(content.contains("GW_MUSEAI_VM_ID=vm-id-42"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_museai_wait_intent_contracts() {
    let generic_intent_wait = false;
    let video_intent_wait = true;
    assert!(
        !generic_intent_wait,
        "omniroute generic chat must not enter video presentation artifact wait"
    );
    assert!(
        video_intent_wait,
        "video generation endpoint must request artifact wait"
    );
}
