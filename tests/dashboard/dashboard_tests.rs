use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use toolcase_gateway::config::{Config, ConfigStore};
use toolcase_gateway::features::dashboard::{mask_key, persist_jev_keys};
use toolcase_gateway::http::{header_value, read_response_head, ResponseHead};
use toolcase_gateway::providers::jev::accounts::JevAccountPool;
use toolcase_gateway::routes::route_request;

fn test_config() -> Config {
    Config {
        target_host: "127.0.0.1".into(),
        target_port: 20128,
        fallbacks: vec!["fail-try".into()],
        io_timeout: Some(Duration::from_secs(5)),
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 5000,
        default_model: "gpt-5.6-sol".into(),
        museai_base_url: "https://muse.ai".into(),
        museai_cookie: "test-cookie".into(),
        museai_ws_url: "wss://hatch.metaaivm.com/v1/noise".into(),
        museai_access_token: "test-token".into(),
        museai_notary_token: "test-notary".into(),
        museai_vm_id: "test-vm".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
        jev_api_keys: vec![
            "sk-glm5-00bc9802bf5fd3545cbd00745429569fa4e871adb0c1d74b".into(),
            "sk-glm5-secondkey1234567890abcdef".into(),
        ],
        jev_base_url: "https://jev-ai.org".into(),
        ..Config::default()
    }
}

fn request_gateway(
    method: &str,
    path: &str,
    body: &[u8],
    store: &ConfigStore,
) -> (ResponseHead, Vec<u8>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store_clone = store.clone();
    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        let _ = route_request(client, &store_clone);
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    if !body.is_empty() {
        client.write_all(body).unwrap();
    }
    client.flush().unwrap();
    let head = read_response_head(&mut client).unwrap();
    let mut resp_body = head.buffered_body.clone();
    let _ = client.read_to_end(&mut resp_body);
    handle.join().unwrap();
    (head, resp_body)
}

#[test]
fn dashboard_html_structure_and_assets() {
    let html = include_str!("../../assets/dashboard.html");
    assert!(html.contains("Gateway Dashboard"));
    assert!(html.contains("OmniRoute Proxy"));
    assert!(html.contains("Jev AI Decision API"));
    assert!(html.contains("MuseAI Subsystem"));
    assert!(html.contains("Jev AI Provider Multi-Account Pool"));
    assert!(html.contains("Add Multi API Keys / Accounts"));
    assert!(html.contains("/api/v1/dashboard/overview"));
    assert!(html.contains("/api/v1/dashboard/accounts"));
}

#[test]
fn dashboard_page_route_serves_html_and_rejects_post() {
    let store = ConfigStore::new(test_config(), PathBuf::from(".env.test_dashboard"));

    // GET /dashboard
    let (head, body) = request_gateway("GET", "/dashboard", b"", &store);
    assert_eq!(head.status, 200);
    let content_type = header_value(&head.headers, "content-type").unwrap();
    assert!(content_type.contains("text/html"));
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("Gateway Dashboard"));

    // GET /dashboard/ (with trailing slash)
    let (head_slash, body_slash) = request_gateway("GET", "/dashboard/", b"", &store);
    assert_eq!(head_slash.status, 200);
    assert!(String::from_utf8_lossy(&body_slash).contains("Gateway Dashboard"));

    // POST /dashboard -> 405 Method Not Allowed
    let (head_post, _) = request_gateway("POST", "/dashboard", b"", &store);
    assert_eq!(head_post.status, 405);
}

#[test]
fn dashboard_overview_returns_masked_keys_and_providers() {
    let store = ConfigStore::new(test_config(), PathBuf::from(".env.test_dashboard"));

    // GET /api/v1/dashboard/overview
    let (head, body) = request_gateway("GET", "/api/v1/dashboard/overview", b"", &store);
    assert_eq!(head.status, 200);
    let content_type = header_value(&head.headers, "content-type").unwrap();
    assert!(content_type.contains("application/json"));

    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["providers"]["jev"]["total_accounts"], 2);

    let accounts = json["providers"]["jev"]["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 2);
    assert_eq!(accounts[0]["label"], "account-1 (..d74b)");
    assert_eq!(accounts[0]["masked_key"], "sk-glm...d74b");
    assert_eq!(accounts[1]["label"], "account-2 (..cdef)");
    assert_eq!(accounts[1]["masked_key"], "sk-glm...cdef");

    // Full key must NEVER be leaked in JSON output
    let body_str = String::from_utf8_lossy(&body);
    assert!(!body_str.contains("00bc9802bf5fd3545cbd00745429569fa4e871adb0c1d74b"));
    assert!(!body_str.contains("secondkey1234567890abcdef"));

    // POST /api/v1/dashboard/overview -> 405 Method Not Allowed
    let (head_post, _) = request_gateway("POST", "/api/v1/dashboard/overview", b"", &store);
    assert_eq!(head_post.status, 405);
}

#[test]
fn dashboard_accounts_add_and_live_pool_update() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    fs::write(&tmp_env, "GW_LISTEN_PORT=20129\n").unwrap();

    let mut cfg = test_config();
    cfg.jev_api_keys = vec!["sk-glm5-existing1111111111".into()];
    let store = ConfigStore::new(cfg, tmp_env.clone());

    assert_eq!(store.jev_pool().len(), 1);

    let payload =
        br#"{"provider":"jev","api_keys":["sk-glm5-added2222222222","sk-glm5-added3333333333"]}"#;
    let (head, body) = request_gateway("POST", "/api/v1/dashboard/accounts", payload, &store);
    assert_eq!(head.status, 200);

    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["provider"], "jev");
    assert_eq!(json["added_count"], 2);
    assert_eq!(json["total_accounts"], 3);

    // Live pool reflects newly added accounts immediately
    assert_eq!(store.jev_pool().len(), 3);
    assert_eq!(store.snapshot().jev_api_keys.len(), 3);
    let all_accounts = store.jev_pool().accounts();
    assert_eq!(all_accounts.len(), 3);
    assert_eq!(all_accounts[1].api_key, "sk-glm5-added2222222222");
    assert_eq!(all_accounts[2].api_key, "sk-glm5-added3333333333");

    // Live candidate accounts can rotate across all 3
    let candidates = store.jev_pool().candidate_accounts();
    assert_eq!(candidates.len(), 3);

    // Verify .env persistence
    let env_content = fs::read_to_string(&tmp_env).unwrap();
    assert!(env_content.contains("GW_LISTEN_PORT=20129"));
    assert!(env_content.contains("GW_JEV_API_KEYS=\"sk-glm5-existing1111111111,sk-glm5-added2222222222,sk-glm5-added3333333333\""));

    let _ = fs::remove_file(tmp_env);
}

#[test]
fn dashboard_accounts_accepts_comma_and_newline_delimited_string() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    let mut cfg = test_config();
    cfg.jev_api_keys = vec![];
    let store = ConfigStore::new(cfg, tmp_env.clone());

    let payload = br#"{"provider":"jev","keys":"sk-key-alpha12345678, sk-key-beta12345678\nsk-key-gamma12345678"}"#;
    let (head, body) = request_gateway("POST", "/api/v1/dashboard/accounts", payload, &store);
    assert_eq!(head.status, 200);

    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["added_count"], 3);
    assert_eq!(store.jev_pool().len(), 3);

    let _ = fs::remove_file(tmp_env);
}

#[test]
fn dashboard_accounts_rejects_malformed_and_unsupported_providers() {
    let store = ConfigStore::new(test_config(), PathBuf::from(".env.test_dashboard"));

    // Malformed JSON -> 400
    let (head_bad_json, _) =
        request_gateway("POST", "/api/v1/dashboard/accounts", b"not-json", &store);
    assert_eq!(head_bad_json.status, 400);

    // Unsupported provider -> 400
    let payload_unsupported = br#"{"provider":"unknown_provider","api_keys":["key1"]}"#;
    let (head_unsupported, body_unsupported) = request_gateway(
        "POST",
        "/api/v1/dashboard/accounts",
        payload_unsupported,
        &store,
    );
    assert_eq!(head_unsupported.status, 400);
    assert!(String::from_utf8_lossy(&body_unsupported).contains("Unsupported provider"));

    // Empty keys -> 400
    let payload_empty = br#"{"provider":"jev","api_keys":[]}"#;
    let (head_empty, body_empty) =
        request_gateway("POST", "/api/v1/dashboard/accounts", payload_empty, &store);
    assert_eq!(head_empty.status, 400);
    assert!(String::from_utf8_lossy(&body_empty).contains("No valid non-empty API keys"));

    // GET /api/v1/dashboard/accounts -> 405 Method Not Allowed
    let (head_get, _) = request_gateway("GET", "/api/v1/dashboard/accounts", b"", &store);
    assert_eq!(head_get.status, 405);
}

#[test]
fn dashboard_accounts_handles_duplicate_keys_gracefully() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    let mut cfg = test_config();
    cfg.jev_api_keys = vec!["sk-glm5-existing1111111111".into()];
    let store = ConfigStore::new(cfg, tmp_env.clone());

    // Submit the duplicate key
    let payload = br#"{"provider":"jev","api_keys":["sk-glm5-existing1111111111"]}"#;
    let (head, body) = request_gateway("POST", "/api/v1/dashboard/accounts", payload, &store);
    assert_eq!(head.status, 200);

    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["added_count"], 0);
    assert_eq!(json["total_accounts"], 1);
    assert_eq!(store.jev_pool().len(), 1);

    let _ = fs::remove_file(tmp_env);
}

#[test]
fn dashboard_accounts_case_insensitive_provider_aliases() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    for alias in ["JEV", "Jev-AI", "jev-ai"] {
        let payload = format!(r#"{{"provider":"{alias}","api_keys":["sk-alias-key-{alias}"]}}"#);
        let (head, body) = request_gateway(
            "POST",
            "/api/v1/dashboard/accounts",
            payload.as_bytes(),
            &store,
        );
        assert_eq!(head.status, 200, "alias {alias} must be accepted");
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["ok"], true);
    }
    let _ = fs::remove_file(tmp_env);
}

#[test]
fn dashboard_cors_preflight_supported() {
    let store = ConfigStore::new(test_config(), PathBuf::from(".env.test_dashboard"));
    for path in [
        "/dashboard",
        "/api/v1/dashboard/overview",
        "/api/v1/dashboard/accounts",
    ] {
        let (head, _) = request_gateway("OPTIONS", path, b"", &store);
        assert_eq!(head.status, 200, "OPTIONS {path} must return 200");
        let allow_origin = header_value(&head.headers, "access-control-allow-origin").unwrap();
        assert_eq!(allow_origin, "*");
    }
}

#[test]
fn mask_key_security_unit_tests() {
    assert_eq!(mask_key(""), "");
    assert_eq!(mask_key("   "), "");
    assert_eq!(mask_key("123"), "***");
    assert_eq!(mask_key("1234567890"), "***");
    assert_eq!(
        mask_key("sk-glm5-00bc9802bf5fd3545cbd00745429569fa4e871adb0c1d74b"),
        "sk-glm...d74b"
    );
    assert_eq!(mask_key("abcdefghijklmnop"), "abcdef...mnop");
}

#[test]
fn persist_jev_keys_preserves_other_variables_and_creates_clean_file() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    let initial_content =
        "GW_LISTEN_PORT=20129\nexport GW_JEV_API_KEYS=\"old-key\"\nGW_JEV_BASE_URL=https://jev-ai.org\n";
    fs::write(&tmp_env, initial_content).unwrap();

    let keys = vec!["key-1".to_string(), "key-2".to_string()];
    persist_jev_keys(&tmp_env, &keys).unwrap();

    let content = fs::read_to_string(&tmp_env).unwrap();
    assert!(content.contains("GW_LISTEN_PORT=20129"));
    assert!(content.contains("GW_JEV_BASE_URL=https://jev-ai.org"));
    assert!(content.contains("GW_JEV_API_KEYS=\"key-1,key-2\""));
    assert!(!content.contains("old-key"));

    // Empty keys does nothing
    persist_jev_keys(&tmp_env, &[]).unwrap();
    let _ = fs::remove_file(&tmp_env);

    // Missing file creation
    let new_tmp = std::env::temp_dir().join(format!("test_env_new_{}", uuid::Uuid::new_v4()));
    persist_jev_keys(&new_tmp, &["fresh-key".to_string()]).unwrap();
    let content_new = fs::read_to_string(&new_tmp).unwrap();
    assert!(content_new.contains("GW_JEV_API_KEYS=\"fresh-key\""));
    let _ = fs::remove_file(new_tmp);
}

#[test]
fn jev_account_pool_short_key_labeling() {
    let pool = JevAccountPool::new(&["tiny".into()]);
    assert_eq!(pool.len(), 1);
    let accounts = pool.accounts();
    assert_eq!(accounts[0].label, "account-1");
    assert_eq!(accounts[0].api_key, "tiny");
}

#[test]
fn dashboard_account_addition_enables_jev_upstream_forwarding_live() {
    // 1. Spin up a mock Jev upstream
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();
    let upstream_handle = thread::spawn(move || {
        let (mut socket, _) = upstream_listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let req_str = String::from_utf8_lossy(&request).to_ascii_lowercase();
        assert!(req_str.contains("authorization: bearer sk-dynamic-jev-key-999"));
        let body = br#"{"object":"list","data":[]}"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        socket.write_all(body).unwrap();
    });

    // 2. Start gateway with empty keys and target pointed at mock Jev
    let mut config = test_config();
    config.jev_api_keys = vec![];
    config.jev_base_url = format!("http://127.0.0.1:{upstream_port}");
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(config, tmp_env.clone());

    // 3. Before adding key, models catalog without auth header returns empty list or rejects
    assert_eq!(store.jev_pool().len(), 0);

    // 4. Ingest key via dashboard API
    let payload = br#"{"provider":"jev","api_keys":["sk-dynamic-jev-key-999"]}"#;
    let (head, _) = request_gateway("POST", "/api/v1/dashboard/accounts", payload, &store);
    assert_eq!(head.status, 200);
    assert_eq!(store.jev_pool().len(), 1);

    // 5. Subsequent call to /jev/v1/models automatically uses the newly added key!
    let (head_models, _) = request_gateway("GET", "/jev/v1/models", b"", &store);
    assert_eq!(head_models.status, 200);

    upstream_handle.join().unwrap();
    let _ = fs::remove_file(tmp_env);
}
