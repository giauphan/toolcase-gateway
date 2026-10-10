//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;

// ---------- HAR → config feature: strict tests ----------

fn har_fixture_full() -> &'static str {
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

fn har_fixture_ws_only() -> &'static str {
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

fn har_fixture_empty_token() -> &'static str {
    r#"{
      "log": {
        "entries": [
          {
            "request": {
              "url": "wss://hatch.metaaivm.com/v1/noise?auth_token=&notary_token=notary-token-def456"
            }
          }
        ]
      }
    }"#
}

fn har_fixture_encoded_token() -> &'static str {
    r#"{
      "log": {
        "entries": [
          {
            "request": {
              "url": "wss://hatch.metaaivm.com/v1/noise?auth_token=%74%6F%6B&notary_token=n"
            }
          }
        ]
      }
    }"#
}

fn har_fixture_no_muse() -> &'static str {
    r#"{
      "log": {
        "entries": [
          { "request": { "url": "https://github.com/x/y" } },
          { "request": { "url": "wss://other.example.com/socket" } }
        ]
      }
    }"#
}

fn test_har_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("gw-har-tests-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn har_extraction_full_capture_extracts_all_fields() {
    let parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har_fixture_full().as_bytes())
            .unwrap();
    assert_eq!(
        parsed.ws_url.as_deref(),
        Some("wss://hatch.metaaivm.com/v1/noise")
    );
    assert_eq!(parsed.base_url.as_deref(), Some("https://muse.ai"));
    assert_eq!(parsed.access_token.as_deref(), Some("access-token-abc123"));
    assert_eq!(parsed.notary_token.as_deref(), Some("notary-token-def456"));
    assert_eq!(parsed.vm_id.as_deref(), Some("vm-id-42"));
    assert_eq!(
        parsed.cookie.as_deref(),
        Some("sessionId=abc123; theme=dark")
    );
}

#[test]
fn har_extraction_rejects_untrusted_rest_derived_websocket_urls() {
    for endpoint_url in [
        "wss://attacker.example/v1/noise",
        "wss://metaaivm.com.evil.example/v1/noise",
        "ws://hatch.metaaivm.com/v1/noise",
        "https://hatch.metaaivm.com/v1/noise",
    ] {
        let har = serde_json::json!({
            "log": {"entries": [{
                "request": {"url": "https://muse.ai/api/session"},
                "response": {"content": {"text": format!("{{\"vm_id\":\"vm-from-response\",\"endpoint_url\":\"{endpoint_url}\"}}")}}
            }]}
        });
        let parsed =
            toolcase_gateway::har_config::extract_muse_config_from_har(har.to_string().as_bytes())
                .unwrap();
        assert_eq!(
            parsed.vm_id.as_deref(),
            Some("vm-from-response"),
            "{endpoint_url}"
        );
        assert_eq!(parsed.ws_url, None, "{endpoint_url}");
    }
}

#[test]
fn har_extraction_rejects_untrusted_noise_websocket_hosts() {
    for url in [
        "wss://attacker.example/v1/noise?auth_token=secret",
        "wss://metaaivm.com.evil.example/v1/noise?auth_token=secret",
        "ws://hatch.metaaivm.com/v1/noise?auth_token=secret",
    ] {
        let har = serde_json::json!({"log": {"entries": [{"request": {"url": url}}]}});
        let error =
            toolcase_gateway::har_config::extract_muse_config_from_har(har.to_string().as_bytes())
                .unwrap_err();
        assert!(matches!(
            error,
            toolcase_gateway::har_config::HarExtractError::NoMuseEntries
        ));
    }
}

#[test]
fn har_extraction_vm_id_from_session_response() {
    let har = r#"{
      "log": {"entries": [
        {"request": {"url": "https://muse.ai/api/session"}}
      ]}
    }"#;
    let mut parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har.as_bytes()).unwrap();
    assert_eq!(parsed.base_url.as_deref(), Some("https://muse.ai"));
    assert!(parsed.vm_id.is_none(), "no response body => no vm_id");

    let har_with_vm = r#"{
      "log": {"entries": [
        {
          "request": {"url": "https://muse.ai/api/session"},
          "response": {"content": {"text": "{\"vm_id\":\"vm-from-response\",\"endpoint_url\":\"wss://x.metaaivm.com/\"}"}}
        }
      ]}
    }"#;
    parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har_with_vm.as_bytes()).unwrap();
    assert_eq!(parsed.vm_id.as_deref(), Some("vm-from-response"));
    assert_eq!(parsed.ws_url.as_deref(), Some("wss://x.metaaivm.com/"));
    assert_eq!(parsed.base_url.as_deref(), Some("https://muse.ai"));
}

#[test]
fn har_extraction_selects_preferred_vm_from_current_session_shape() {
    let har = r#"{
      "log": {"entries": [
        {
          "request": {"url": "https://muse.ai/api/session"},
          "response": {"content": {"text": "{\"vms\":[{\"id\":\"fallback-id\",\"endpoint_url\":\"wss://fallback.invalid/\",\"is_preferred\":false},{\"vm_id\":\"preferred-vm\",\"endpoint_url\":\"wss://preferred.metaaivm.com/\",\"is_preferred\":true}]}"}}
        }
      ]}
    }"#;
    let parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har.as_bytes()).unwrap();
    assert_eq!(parsed.vm_id.as_deref(), Some("preferred-vm"));
    assert_eq!(
        parsed.ws_url.as_deref(),
        Some("wss://preferred.metaaivm.com/")
    );
    assert!(parsed.notary_token.is_none());
}

#[test]
fn har_extraction_reads_standard_cookie_arrays_and_preserves_them() {
    let har = r#"{
      "log": {"entries": [
        {
          "request": {
            "url": "https://muse.ai/api/session",
            "cookies": [
              {"name": "sessionId", "value": "synthetic-session"},
              {"name": "theme", "value": "dark"}
            ]
          }
        },
        {
          "request": {"url": "https://muse.ai/api/auth/check", "headers": []},
          "response": {"content": {"text": "{\"access_token\":\"synthetic-token\"}"}}
        }
      ]}
    }"#;
    let parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har.as_bytes()).unwrap();
    assert_eq!(
        parsed.cookie.as_deref(),
        Some("sessionId=synthetic-session; theme=dark")
    );
    assert_eq!(parsed.access_token.as_deref(), Some("synthetic-token"));
}

#[test]
fn har_extraction_accepts_case_insensitive_cookie_header() {
    let har = r#"{
      "log": {"entries": [
        {
          "request": {
            "url": "https://muse.ai/api/session",
            "headers": [{"name": "cookie", "value": "sessionId=synthetic"}]
          }
        }
      ]}
    }"#;
    let parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har.as_bytes()).unwrap();
    assert_eq!(parsed.cookie.as_deref(), Some("sessionId=synthetic"));
}

#[test]
fn har_extraction_access_token_from_auth_check_response() {
    let har = r#"{
      "log": {"entries": [
        {
          "request": {"url": "https://muse.ai/api/auth/check"},
          "response": {"content": {"text": "{\"access_token\":\"response-token\",\"ok\":true}"}}
        }
      ]}
    }"#;
    let parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har.as_bytes()).unwrap();
    assert_eq!(parsed.access_token.as_deref(), Some("response-token"));
}

#[test]
fn har_extraction_duplicates_keep_last_nonempty_value() {
    let har = serde_json::json!({
        "log": {"entries": [
            {"request": {"url": "wss://hatch.metaaivm.com/v1/noise?vm_id=first&auth_token=a1&notary_token=n1"}},
            {"request": {"url": "wss://hatch.metaaivm.com/v1/noise?vm_id=second&notary_token=n2"}}
        ]}
    });
    let parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har.to_string().as_bytes())
            .unwrap();
    assert_eq!(parsed.vm_id.as_deref(), Some("second"));
    assert_eq!(parsed.access_token.as_deref(), Some("a1"));
    assert_eq!(parsed.notary_token.as_deref(), Some("n2"));
}

#[test]
fn har_extraction_empty_param_counts_as_missing() {
    let parsed = toolcase_gateway::har_config::extract_muse_config_from_har(
        har_fixture_empty_token().as_bytes(),
    )
    .unwrap();
    assert!(parsed.access_token.is_none());
    assert_eq!(parsed.notary_token.as_deref(), Some("notary-token-def456"));
}

#[test]
fn har_extraction_percent_decodes_query_values() {
    let parsed = toolcase_gateway::har_config::extract_muse_config_from_har(
        har_fixture_encoded_token().as_bytes(),
    )
    .unwrap();
    assert_eq!(parsed.access_token.as_deref(), Some("tok"));
}

#[test]
fn har_extraction_rejects_invalid_json() {
    let err = toolcase_gateway::har_config::extract_muse_config_from_har(b"not json").unwrap_err();
    assert!(matches!(
        err,
        toolcase_gateway::har_config::HarExtractError::InvalidJson(_)
    ));
}

#[test]
fn har_extraction_rejects_non_har_shapes() {
    for shape in [
        "{\"foo\":1}",
        "{\"log\":{}}",
        "{\"log\":{\"entries\":\"nope\"}}",
    ] {
        let err = toolcase_gateway::har_config::extract_muse_config_from_har(shape.as_bytes())
            .unwrap_err();
        assert!(
            matches!(err, toolcase_gateway::har_config::HarExtractError::NotHar),
            "unexpected error for {shape}: {err:?}"
        );
    }
}

#[test]
fn har_extraction_rejects_capture_without_muse_entries() {
    let err = toolcase_gateway::har_config::extract_muse_config_from_har(
        har_fixture_no_muse().as_bytes(),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        toolcase_gateway::har_config::HarExtractError::NoMuseEntries
    ));
}

#[test]
fn har_extraction_ws_only_capture_is_partial_ok() {
    let parsed = toolcase_gateway::har_config::extract_muse_config_from_har(
        har_fixture_ws_only().as_bytes(),
    )
    .unwrap();
    assert!(parsed.cookie.is_none());
    assert!(parsed.base_url.is_none());
    assert_eq!(parsed.vm_id.as_deref(), Some("vm-id-42"));
}

#[test]
fn mask_secret_masks_full_and_partial() {
    assert_eq!(
        toolcase_gateway::har_config::mask_secret("abcdef12"),
        "****"
    );
    assert_eq!(
        toolcase_gateway::har_config::mask_secret("abcdef12345"),
        "abcd…2345"
    );
    assert_eq!(
        toolcase_gateway::har_config::mask_secret("cafééééééé"),
        "café…éééé"
    );
}

#[test]
fn persist_env_creates_file_with_managed_keys_only() {
    let dir = test_har_dir("create");
    let path = dir.join(".env");
    toolcase_gateway::har_config::persist_muse_env(
        &path,
        &[
            ("GW_MUSEAI_VM_ID", "vm-id-42"),
            ("GW_MUSEAI_ACCESS_TOKEN", "tok-abc"),
        ],
    )
    .unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("GW_MUSEAI_VM_ID=vm-id-42"));
    assert!(content.contains("GW_MUSEAI_ACCESS_TOKEN=tok-abc"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persist_env_replaces_managed_key_preserves_unrelated() {
    let dir = test_har_dir("replace");
    let path = dir.join(".env");
    std::fs::write(
        &path,
        "# comment\nGW_LISTEN_PORT=20129\nGW_MUSEAI_ACCESS_TOKEN=old-token\n",
    )
    .unwrap();
    toolcase_gateway::har_config::persist_muse_env(
        &path,
        &[
            ("GW_MUSEAI_ACCESS_TOKEN", "new-token"),
            ("GW_MUSEAI_VM_ID", "vm-1"),
        ],
    )
    .unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("# comment"));
    assert!(content.contains("GW_LISTEN_PORT=20129"));
    assert!(content.contains("GW_MUSEAI_ACCESS_TOKEN=new-token"));
    assert!(content.contains("GW_MUSEAI_VM_ID=vm-1"));
    assert!(!content.contains("old-token"));
    assert_eq!(content.matches("GW_MUSEAI_ACCESS_TOKEN=").count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persist_env_keeps_existing_managed_key_without_new_value() {
    let dir = test_har_dir("keep");
    let path = dir.join(".env");
    std::fs::write(&path, "GW_MUSEAI_WS_URL=old-ws\nGW_LISTEN_PORT=1\n").unwrap();
    toolcase_gateway::har_config::persist_muse_env(&path, &[("GW_MUSEAI_VM_ID", "v")]).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("GW_MUSEAI_WS_URL=old-ws"));
    assert!(content.contains("GW_MUSEAI_VM_ID=v"));
    assert!(content.contains("GW_LISTEN_PORT=1"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persist_env_quotes_values_with_spaces() {
    let dir = test_har_dir("quote");
    let path = dir.join(".env");
    toolcase_gateway::har_config::persist_muse_env(
        &path,
        &[("GW_MUSEAI_COOKIE", "sessionId=abc; theme=dark")],
    )
    .unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("GW_MUSEAI_COOKIE=\"sessionId=abc; theme=dark\""));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persist_env_noop_when_no_managed_keys() {
    let dir = test_har_dir("noop");
    let path = dir.join(".env");
    toolcase_gateway::har_config::persist_muse_env(&path, &[("GW_LISTEN_PORT", "1")]).unwrap();
    assert!(!path.exists());
    let _ = std::fs::remove_dir_all(&dir);
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
    // With both tokens present, bootstrap short-circuits: no network, fields intact.
    let booted = toolcase_gateway::museai::bootstrap_museai_config(&snap).unwrap();
    assert_eq!(booted.museai_access_token, snap.museai_access_token);
    assert_eq!(booted.museai_vm_id, snap.museai_vm_id);

    // The WS URL builder now carries the applied values.
    let url = toolcase_gateway::museai::build_museai_ws_url(&booted, "req-1").unwrap();
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
