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
    assert!(html.contains("Muse AI Subsystem Configuration"));
    assert!(html.contains("Muse Sentinel Approval Mode"));
    assert!(html.contains("Default Video Prompt"));
    assert!(html.contains("/api/v1/dashboard/overview"));
    assert!(html.contains("/api/v1/dashboard/accounts"));
    assert!(html.contains("/api/v1/dashboard/muse-config"));
}

#[test]
fn dashboard_page_route_serves_html_and_rejects_post() {
    let store = ConfigStore::new(test_config(), PathBuf::from(".env.test_dashboard"));

    // GET /gateway-config/dashboard
    let (head, body) = request_gateway("GET", "/gateway-config/dashboard", b"", &store);
    assert_eq!(head.status, 200);
    let content_type = header_value(&head.headers, "content-type").unwrap();
    assert!(content_type.contains("text/html"));
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("Gateway Dashboard"));

    // GET /gateway-config/dashboard/ (with trailing slash)
    let (head_slash, body_slash) =
        request_gateway("GET", "/gateway-config/dashboard/", b"", &store);
    assert_eq!(head_slash.status, 200);
    assert!(String::from_utf8_lossy(&body_slash).contains("Gateway Dashboard"));

    // POST /gateway-config/dashboard -> 405 Method Not Allowed
    let (head_post, _) = request_gateway("POST", "/gateway-config/dashboard", b"", &store);
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

    // Verify JSON persistence instead of mutating .env
    assert!(store.config_file.exists());
    let json_content = fs::read_to_string(&store.config_file).unwrap();
    assert!(json_content.contains("sk-glm5-added2222222222"));
    assert!(json_content.contains("sk-glm5-added3333333333"));

    // .env remains untouched with its original content
    let env_content = fs::read_to_string(&tmp_env).unwrap();
    assert_eq!(env_content, "GW_LISTEN_PORT=20129\n");

    let _ = fs::remove_file(&tmp_env);
    let _ = fs::remove_file(&store.config_file);
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
        "/gateway-config/dashboard",
        "/api/v1/dashboard/overview",
        "/api/v1/dashboard/accounts",
        "/api/v1/dashboard/config",
        "/api/v1/dashboard/muse-config",
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

#[test]
fn dashboard_accounts_add_with_custom_label() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    fs::write(&tmp_env, "GW_LISTEN_PORT=20129\n").unwrap();

    let store = ConfigStore::new(test_config(), tmp_env.clone());
    let payload =
        br#"{"provider":"jev","label":"prod-primary","api_keys":["sk-glm5-custom11111"]}"#;
    let (head, body) = request_gateway("POST", "/api/v1/dashboard/accounts", payload, &store);
    assert_eq!(head.status, 200);

    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["added_count"], 1);

    let accounts = json["accounts"].as_array().unwrap();
    let added = accounts
        .iter()
        .find(|a| a["label"] == "prod-primary")
        .unwrap();
    assert!(added["masked_key"].as_str().unwrap().contains("..."));

    // Check pool directly
    let pool_accounts = store.jev_pool().accounts();
    let matched = pool_accounts
        .iter()
        .find(|a| a.label == "prod-primary")
        .unwrap();
    assert_eq!(matched.api_key, "sk-glm5-custom11111");

    let _ = fs::remove_file(tmp_env);
    let _ = fs::remove_file(store.config_file);
}

#[test]
fn dashboard_accounts_add_multiple_keys_with_common_label() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    fs::write(&tmp_env, "GW_LISTEN_PORT=20129\n").unwrap();

    let mut cfg = test_config();
    cfg.jev_api_keys = vec![];
    let store = ConfigStore::new(cfg, tmp_env.clone());

    let payload = br#"{"provider":"jev","label":"cluster-node","api_keys":["sk-k1-11111111","sk-k2-22222222"]}"#;
    let (head, body) = request_gateway("POST", "/api/v1/dashboard/accounts", payload, &store);
    assert_eq!(head.status, 200);

    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["added_count"], 2);

    let accounts = store.jev_pool().accounts();
    assert_eq!(accounts.len(), 2);
    assert_eq!(accounts[0].label, "cluster-node-1");
    assert_eq!(accounts[1].label, "cluster-node-2");

    let _ = fs::remove_file(tmp_env);
    let _ = fs::remove_file(store.config_file);
}

#[test]
fn dashboard_accounts_add_inline_labels_and_structured_accounts() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    fs::write(&tmp_env, "GW_LISTEN_PORT=20129\n").unwrap();

    let mut cfg = test_config();
    cfg.jev_api_keys = vec![];
    let store = ConfigStore::new(cfg, tmp_env.clone());

    // 1. Ingest via inline labels in textarea
    let payload = br#"{"provider":"jev","api_keys":["east-worker:sk-east-111111","west-worker=sk-west-222222"]}"#;
    let (head, _) = request_gateway("POST", "/api/v1/dashboard/accounts", payload, &store);
    assert_eq!(head.status, 200);

    let accounts = store.jev_pool().accounts();
    assert_eq!(accounts.len(), 2);
    assert_eq!(accounts[0].label, "east-worker");
    assert_eq!(accounts[0].api_key, "sk-east-111111");
    assert_eq!(accounts[1].label, "west-worker");
    assert_eq!(accounts[1].api_key, "sk-west-222222");

    // 2. Ingest via structured accounts JSON
    let struct_payload = br#"{"provider":"jev","accounts":[{"key":"sk-struct-333333","label":"struct-alpha"},{"api_key":"sk-struct-444444","label":"struct-beta"}]}"#;
    let (head2, _) = request_gateway("POST", "/api/v1/dashboard/accounts", struct_payload, &store);
    assert_eq!(head2.status, 200);

    let accounts2 = store.jev_pool().accounts();
    assert_eq!(accounts2.len(), 4);
    assert_eq!(accounts2[2].label, "struct-alpha");
    assert_eq!(accounts2[2].api_key, "sk-struct-333333");
    assert_eq!(accounts2[3].label, "struct-beta");
    assert_eq!(accounts2[3].api_key, "sk-struct-444444");

    // 3. Update existing label for an account
    let update_payload = br#"{"provider":"jev","accounts":[{"key":"sk-east-111111","label":"east-primary-promoted"}]}"#;
    let (head3, body3) =
        request_gateway("POST", "/api/v1/dashboard/accounts", update_payload, &store);
    assert_eq!(head3.status, 200);

    let json3: serde_json::Value = serde_json::from_slice(&body3).unwrap();
    assert_eq!(json3["total_accounts"], 4); // Not duplicated
    let accounts3 = store.jev_pool().accounts();
    assert_eq!(accounts3[0].label, "east-primary-promoted");

    // 4. Persistence across store reload
    let mut reload_cfg = test_config();
    reload_cfg.jev_api_keys = vec![]; // simulate fresh start; keys come from stored accounts only
    let loaded_store = ConfigStore::new_with_limits_and_config_file(
        reload_cfg,
        tmp_env.clone(),
        store.config_file.clone(),
        256,
        64 * 1024 * 1024,
    );
    let loaded_accounts = loaded_store.jev_pool().accounts();
    assert_eq!(loaded_accounts.len(), 4);
    assert_eq!(loaded_accounts[0].label, "east-primary-promoted");
    assert_eq!(loaded_accounts[0].api_key, "sk-east-111111");
    assert_eq!(loaded_accounts[1].label, "west-worker");
    assert_eq!(loaded_accounts[2].label, "struct-alpha");
    assert_eq!(loaded_accounts[3].label, "struct-beta");

    let _ = fs::remove_file(tmp_env);
    let _ = fs::remove_file(store.config_file);
}

#[test]
fn dashboard_accounts_persistent_labeled_and_extra_env_keys_merged() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    fs::write(&tmp_env, "GW_LISTEN_PORT=20129\n").unwrap();

    let mut cfg = test_config();
    cfg.jev_api_keys = vec![];
    let store = ConfigStore::new(cfg, tmp_env.clone());

    // Ingest 2 accounts with custom labels
    let payload = br#"{"provider":"jev","accounts":[{"key":"sk-stored-1","label":"primary-labeled"},{"key":"sk-stored-2","label":"backup-labeled"}]}"#;
    let (head, _) = request_gateway("POST", "/api/v1/dashboard/accounts", payload, &store);
    assert_eq!(head.status, 200);

    // Simulate restart where GW_JEV_API_KEYS has an extra key not in storage
    let mut restart_cfg = test_config();
    restart_cfg.jev_api_keys = vec!["sk-stored-1".into(), "sk-new-from-env".into()];
    let reloaded_store = ConfigStore::new_with_limits_and_config_file(
        restart_cfg,
        tmp_env.clone(),
        store.config_file.clone(),
        256,
        64 * 1024 * 1024,
    );

    let reloaded_accounts = reloaded_store.jev_pool().accounts();
    assert_eq!(reloaded_accounts.len(), 3);
    assert_eq!(reloaded_accounts[0].label, "primary-labeled");
    assert_eq!(reloaded_accounts[0].api_key, "sk-stored-1");
    assert_eq!(reloaded_accounts[1].label, "backup-labeled");
    assert_eq!(reloaded_accounts[1].api_key, "sk-stored-2");
    assert_eq!(reloaded_accounts[2].api_key, "sk-new-from-env");
    assert_eq!(reloaded_accounts[2].label, "account-3 (..-env)");

    let _ = fs::remove_file(tmp_env);
    let _ = fs::remove_file(store.config_file);
}

#[test]
fn dashboard_accounts_openrouter_ingest_and_persistence() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    fs::write(&tmp_env, "GW_LISTEN_PORT=20129\n").unwrap();

    let cfg = test_config();
    let store = ConfigStore::new(cfg, tmp_env.clone());

    let payload = br#"{"provider":"openrouter","api_keys":["or_session_11111111111","worker-2:or_session_22222222222","sk-or-v1-my-secret-key-123"]}"#;
    let (head, body) = request_gateway("POST", "/api/v1/dashboard/accounts", payload, &store);
    assert_eq!(head.status, 200);

    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["provider"], "openrouter/jev");
    assert_eq!(parsed["added_count"], 3);
    assert_eq!(parsed["total_accounts"], 3);

    let or_accounts = store.jev_pool().openrouter_accounts();
    assert_eq!(or_accounts.len(), 3);
    assert_eq!(or_accounts[0].cookie, "or_session_11111111111");
    assert_eq!(or_accounts[1].cookie, "or_session_22222222222");
    assert_eq!(or_accounts[1].label, "worker-2");
    assert_eq!(or_accounts[2].api_key, "sk-or-v1-my-secret-key-123");

    // Overview reports openrouter_jev with masked cookies and keys
    let (ov_head, ov_body) = request_gateway("GET", "/api/v1/dashboard/overview", b"", &store);
    assert_eq!(ov_head.status, 200);
    let ov_json: serde_json::Value = serde_json::from_slice(&ov_body).unwrap();
    assert_eq!(ov_json["providers"]["openrouter_jev"]["total_accounts"], 3);
    assert_eq!(
        ov_json["providers"]["openrouter_jev"]["accounts"][0]["label"],
        "openrouter-1 (..1111)"
    );

    // Persistence reload
    let reload_cfg = test_config();
    let reloaded_store = ConfigStore::new_with_limits_and_config_file(
        reload_cfg,
        tmp_env.clone(),
        store.config_file.clone(),
        256,
        64 * 1024 * 1024,
    );
    let reloaded_or = reloaded_store.jev_pool().openrouter_accounts();
    assert_eq!(reloaded_or.len(), 3);
    assert_eq!(reloaded_or[0].cookie, "or_session_11111111111");
    assert_eq!(reloaded_or[1].label, "worker-2");
    assert_eq!(reloaded_or[2].api_key, "sk-or-v1-my-secret-key-123");

    let _ = fs::remove_file(tmp_env);
    let _ = fs::remove_file(store.config_file);
}

#[test]
fn dashboard_update_jev_config_api_and_persistence() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    fs::write(&tmp_env, "GW_LISTEN_PORT=20129\n").unwrap();

    let cfg = test_config();
    let store = ConfigStore::new(cfg, tmp_env.clone());

    let payload = br#"{
        "jev_base_url": "https://custom-jev.ai",
        "jev_openrouter_base_url": "https://custom-or.ai",
        "jev_openrouter_api_key": "sk-or-v1-testkey123456",
        "jev_openrouter_cookie": "secret-session-cookie-12345",
        "jev_openrouter_model": "typesafe/jev-1.13",
        "jev_openrouter_enabled": true,
        "jev_credit_cooldown_secs": 600
    }"#;

    let (head, body) = request_gateway("POST", "/api/v1/dashboard/jev-config", payload, &store);
    assert_eq!(head.status, 200);

    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["config"]["jev_base_url"], "https://custom-jev.ai");
    assert_eq!(
        parsed["config"]["jev_openrouter_base_url"],
        "https://custom-or.ai"
    );
    assert_eq!(
        parsed["config"]["jev_openrouter_model"],
        "typesafe/jev-1.13"
    );
    assert_eq!(parsed["config"]["jev_openrouter_enabled"], true);
    assert_eq!(parsed["config"]["jev_credit_cooldown_secs"], 600);
    assert_eq!(
        parsed["config"]["jev_openrouter_cookie"],
        mask_key("secret-session-cookie-12345")
    );
    assert_eq!(
        parsed["config"]["jev_openrouter_api_key"],
        mask_key("sk-or-v1-testkey123456")
    );

    // Verify live store updated
    let snapshot = store.snapshot();
    assert_eq!(snapshot.jev_base_url, "https://custom-jev.ai");
    assert_eq!(snapshot.jev_openrouter_base_url, "https://custom-or.ai");
    assert_eq!(snapshot.jev_openrouter_api_key, "sk-or-v1-testkey123456");
    assert_eq!(
        snapshot.jev_openrouter_cookie,
        "secret-session-cookie-12345"
    );
    assert_eq!(snapshot.jev_openrouter_model, "typesafe/jev-1.13");
    assert_eq!(snapshot.jev_credit_cooldown_secs, 600);

    // Verify persistent config file has settings
    let persistent = toolcase_gateway::config::load_persistent_config(&store.config_file).unwrap();
    assert_eq!(
        persistent.jev_base_url.as_deref(),
        Some("https://custom-jev.ai")
    );
    assert_eq!(
        persistent.jev_openrouter_base_url.as_deref(),
        Some("https://custom-or.ai")
    );
    assert_eq!(
        persistent.jev_openrouter_api_key.as_deref(),
        Some("sk-or-v1-testkey123456")
    );
    assert_eq!(
        persistent.jev_openrouter_cookie.as_deref(),
        Some("secret-session-cookie-12345")
    );
    assert_eq!(
        persistent.jev_openrouter_model.as_deref(),
        Some("typesafe/jev-1.13")
    );
    assert_eq!(persistent.jev_credit_cooldown_secs, Some(600));

    let _ = fs::remove_file(tmp_env);
    let _ = fs::remove_file(store.config_file);
}

#[test]
fn dashboard_accounts_openrouter_explicit_key_and_cookie_ingest_and_persistence() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    fs::write(&tmp_env, "GW_LISTEN_PORT=20129\n").unwrap();

    let cfg = test_config();
    let store = ConfigStore::new(cfg, tmp_env.clone());

    // Ingest with explicit openrouter-key provider
    let key_payload =
        br#"{"provider":"openrouter-key","api_keys":["custom-label:sk-or-v1-explicit-key"]}"#;
    let (head1, body1) = request_gateway("POST", "/api/v1/dashboard/accounts", key_payload, &store);
    assert_eq!(head1.status, 200);
    let parsed1: serde_json::Value = serde_json::from_slice(&body1).unwrap();
    assert_eq!(parsed1["ok"], true);
    assert_eq!(parsed1["added_count"], 1);

    // Ingest with explicit openrouter-cookie provider
    let cookie_payload =
        br#"{"provider":"openrouter-cookie","api_keys":["session-label:or_cookie_token=val"]}"#;
    let (head2, body2) =
        request_gateway("POST", "/api/v1/dashboard/accounts", cookie_payload, &store);
    assert_eq!(head2.status, 200);
    let parsed2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
    assert_eq!(parsed2["ok"], true);
    assert_eq!(parsed2["added_count"], 1);

    let or_accounts = store.jev_pool().openrouter_accounts();
    assert_eq!(or_accounts.len(), 2);
    assert_eq!(or_accounts[0].api_key, "sk-or-v1-explicit-key");
    assert_eq!(or_accounts[0].cookie, "");
    assert_eq!(or_accounts[0].label, "custom-label");

    assert_eq!(or_accounts[1].api_key, "");
    assert_eq!(or_accounts[1].cookie, "or_cookie_token=val");
    assert_eq!(or_accounts[1].label, "session-label");

    let _ = fs::remove_file(tmp_env);
    let _ = fs::remove_file(store.config_file);
}
