use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use toolcase_gateway::config::{
    load_persistent_config, save_persistent_config, Config, ConfigStore, PersistentConfig,
};
use toolcase_gateway::http::{read_response_head, ResponseHead};
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
        jev_api_keys: vec!["sk-test-key-1".into()],
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
fn test_persistent_config_save_load_round_trip() {
    let tmp_file = std::env::temp_dir().join(format!("test_cfg_{}.json", uuid::Uuid::new_v4()));

    let original = PersistentConfig {
        target_host: Some("10.0.0.5".into()),
        target_port: Some(9999),
        fallbacks: Some(vec!["backup-1".into(), "backup-2".into()]),
        default_model: Some("model-custom".into()),
        io_timeout_secs: Some(45),
        jev_api_keys: Some(vec!["key-a".into(), "key-b".into()]),
        jev_base_url: Some("https://custom.jev".into()),
        museai_base_url: Some("https://muse.custom".into()),
        museai_cookie: Some("cookie-123".into()),
        museai_ws_url: Some("wss://ws.custom".into()),
        museai_access_token: Some("token-abc".into()),
        museai_notary_token: Some("notary-xyz".into()),
        museai_vm_id: Some("vm-999".into()),
    };

    save_persistent_config(&tmp_file, &original).unwrap();
    assert!(tmp_file.exists());

    let loaded = load_persistent_config(&tmp_file).expect("failed to load persistent config");
    assert_eq!(loaded, original);

    let mut cfg = Config::default();
    loaded.apply_to(&mut cfg);

    assert_eq!(cfg.target_host, "10.0.0.5");
    assert_eq!(cfg.target_port, 9999);
    assert_eq!(cfg.fallbacks, vec!["backup-1", "backup-2"]);
    assert_eq!(cfg.default_model, "model-custom");
    assert_eq!(cfg.io_timeout, Some(Duration::from_secs(45)));
    assert_eq!(cfg.jev_api_keys, vec!["key-a", "key-b"]);
    assert_eq!(cfg.jev_base_url, "https://custom.jev");
    assert_eq!(cfg.museai_base_url, "https://muse.custom");
    assert_eq!(cfg.museai_cookie, "cookie-123");
    assert_eq!(cfg.museai_ws_url, "wss://ws.custom");
    assert_eq!(cfg.museai_access_token, "token-abc");
    assert_eq!(cfg.museai_notary_token, "notary-xyz");
    assert_eq!(cfg.museai_vm_id, "vm-999");

    let _ = fs::remove_file(tmp_file);
}

#[test]
fn test_dashboard_update_gateway_config_api_and_json_persistence() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    let tmp_json = std::env::temp_dir().join(format!("test_cfg_{}.json", uuid::Uuid::new_v4()));
    let env_initial_content = "GW_LISTEN_PORT=20129\nGW_SOME_OTHER_VAR=unmodified\n";
    fs::write(&tmp_env, env_initial_content).unwrap();

    let store = ConfigStore::new_with_limits_and_config_file(
        test_config(),
        tmp_env.clone(),
        tmp_json.clone(),
        256,
        1024 * 1024,
    );

    let payload = serde_json::json!({
        "target_host": "upstream.proxy.local",
        "target_port": 8443,
        "default_model": "claude-3-7-sonnet",
        "io_timeout_secs": 60,
        "fallbacks": ["gpt-4o", "fail-try"]
    });
    let payload_bytes = serde_json::to_vec(&payload).unwrap();

    let (head, body) = request_gateway("POST", "/api/v1/dashboard/config", &payload_bytes, &store);
    assert_eq!(head.status, 200);

    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["config"]["target_host"], "upstream.proxy.local");
    assert_eq!(json["config"]["target_port"], 8443);
    assert_eq!(json["config"]["default_model"], "claude-3-7-sonnet");

    // In-memory config is updated immediately
    let snap = store.snapshot();
    assert_eq!(snap.target_host, "upstream.proxy.local");
    assert_eq!(snap.target_port, 8443);
    assert_eq!(snap.default_model, "claude-3-7-sonnet");
    assert_eq!(snap.io_timeout, Some(Duration::from_secs(60)));
    assert_eq!(snap.fallbacks, vec!["gpt-4o", "fail-try"]);

    // Settings are persisted in the JSON file
    assert!(tmp_json.exists());
    let saved_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&tmp_json).unwrap()).unwrap();
    assert_eq!(saved_json["target_host"], "upstream.proxy.local");
    assert_eq!(saved_json["target_port"], 8443);

    // .env file remains completely untouched!
    let env_after = fs::read_to_string(&tmp_env).unwrap();
    assert_eq!(env_after, env_initial_content);

    let _ = fs::remove_file(tmp_env);
    let _ = fs::remove_file(tmp_json);
}

#[test]
fn test_dashboard_update_gateway_config_validation() {
    let store = ConfigStore::new(test_config(), PathBuf::from(".env.test_validation"));

    // Empty target host -> 400
    let payload_bad_host = br#"{"target_host":""}"#;
    let (head1, body1) =
        request_gateway("POST", "/api/v1/dashboard/config", payload_bad_host, &store);
    assert_eq!(head1.status, 400);
    assert!(String::from_utf8_lossy(&body1).contains("target_host cannot be empty"));

    // Port 0 -> 400
    let payload_bad_port = br#"{"target_port":0}"#;
    let (head2, body2) =
        request_gateway("POST", "/api/v1/dashboard/config", payload_bad_port, &store);
    assert_eq!(head2.status, 400);
    assert!(String::from_utf8_lossy(&body2).contains("target_port must be greater than 0"));

    // Malformed JSON -> 400
    let (head3, _) = request_gateway("POST", "/api/v1/dashboard/config", b"not-json", &store);
    assert_eq!(head3.status, 400);

    // GET /api/v1/dashboard/config -> 405 Method Not Allowed
    let (head4, _) = request_gateway("GET", "/api/v1/dashboard/config", b"", &store);
    assert_eq!(head4.status, 405);
}

#[test]
fn test_overview_reports_standalone_json_storage() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    let tmp_json = std::env::temp_dir().join("custom_config.json");

    let store = ConfigStore::new_with_limits_and_config_file(
        test_config(),
        tmp_env.clone(),
        tmp_json.clone(),
        256,
        1024 * 1024,
    );

    let (head, body) = request_gateway("GET", "/api/v1/dashboard/overview", b"", &store);
    assert_eq!(head.status, 200);

    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["config"]["config_storage"], "custom_config.json");
    assert_eq!(json["config"]["target_host"], "127.0.0.1");
    assert_eq!(json["config"]["target_port"], 20128);
    assert_eq!(json["providers"]["museai"]["base_url"], "https://muse.ai");
    assert_eq!(
        json["providers"]["museai"]["ws_url"],
        "wss://hatch.metaaivm.com/v1/noise"
    );
    assert_eq!(json["providers"]["museai"]["has_cookie"], true);
    assert_eq!(json["providers"]["museai"]["has_access_token"], true);

    let _ = fs::remove_file(tmp_env);
}

#[test]
fn test_account_addition_persists_to_json_file_leaving_env_untouched() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    let tmp_json = std::env::temp_dir().join(format!("test_cfg_{}.json", uuid::Uuid::new_v4()));
    let env_initial_content = "GW_LISTEN_PORT=20129\n";
    fs::write(&tmp_env, env_initial_content).unwrap();

    let mut cfg = test_config();
    cfg.jev_api_keys = vec![];
    let store = ConfigStore::new_with_limits_and_config_file(
        cfg,
        tmp_env.clone(),
        tmp_json.clone(),
        256,
        1024 * 1024,
    );

    let payload = br#"{"provider":"jev","api_keys":["sk-json-persisted-key-1"]}"#;
    let (head, _) = request_gateway("POST", "/api/v1/dashboard/accounts", payload, &store);
    assert_eq!(head.status, 200);

    // Settings persisted to JSON file
    assert!(tmp_json.exists());
    let json_content = fs::read_to_string(&tmp_json).unwrap();
    assert!(json_content.contains("sk-json-persisted-key-1"));

    let _ = fs::remove_file(tmp_env);
    let _ = fs::remove_file(tmp_json);
}

#[test]
fn test_dashboard_update_muse_config_api_and_json_persistence() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    let tmp_json = std::env::temp_dir().join(format!("test_cfg_{}.json", uuid::Uuid::new_v4()));
    let env_initial_content = "GW_LISTEN_PORT=20129\n";
    fs::write(&tmp_env, env_initial_content).unwrap();

    let store = ConfigStore::new_with_limits_and_config_file(
        test_config(),
        tmp_env.clone(),
        tmp_json.clone(),
        256,
        1024 * 1024,
    );

    let payload = serde_json::json!({
        "base_url": "https://custom-muse.example.com",
        "ws_url": "wss://custom-ws.example.com/v1/noise",
        "vm_id": "vm-xyz-123",
        "access_token": "sk-secret-token-abcdef123456",
        "notary_token": "notary-secret-7890",
        "cookie": "session=auth-cookie-value"
    });
    let payload_bytes = serde_json::to_vec(&payload).unwrap();

    let (head, body) = request_gateway(
        "POST",
        "/api/v1/dashboard/muse-config",
        &payload_bytes,
        &store,
    );
    assert_eq!(head.status, 200);

    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(
        json["museai"]["base_url"],
        "https://custom-muse.example.com"
    );
    assert_eq!(
        json["museai"]["ws_url"],
        "wss://custom-ws.example.com/v1/noise"
    );
    assert_eq!(json["museai"]["vm_id"], "vm-xyz-123");
    assert_eq!(json["museai"]["configured"], true);
    assert_eq!(json["museai"]["has_access_token"], true);
    assert_eq!(json["museai"]["has_notary_token"], true);
    assert_eq!(json["museai"]["has_cookie"], true);
    assert_eq!(json["museai"]["masked_access_token"], "sk-sec...3456");

    // In-memory store snapshot updated
    let snapshot = store.snapshot();
    assert_eq!(snapshot.museai_base_url, "https://custom-muse.example.com");
    assert_eq!(
        snapshot.museai_ws_url,
        "wss://custom-ws.example.com/v1/noise"
    );
    assert_eq!(snapshot.museai_vm_id, "vm-xyz-123");
    assert_eq!(snapshot.museai_access_token, "sk-secret-token-abcdef123456");
    assert_eq!(snapshot.museai_notary_token, "notary-secret-7890");
    assert_eq!(snapshot.museai_cookie, "session=auth-cookie-value");

    // Persisted to standalone JSON file
    assert!(tmp_json.exists());
    let loaded = load_persistent_config(&tmp_json).expect("failed to read persisted json");
    assert_eq!(
        loaded.museai_base_url.as_deref(),
        Some("https://custom-muse.example.com")
    );
    assert_eq!(
        loaded.museai_ws_url.as_deref(),
        Some("wss://custom-ws.example.com/v1/noise")
    );
    assert_eq!(loaded.museai_vm_id.as_deref(), Some("vm-xyz-123"));
    assert_eq!(
        loaded.museai_access_token.as_deref(),
        Some("sk-secret-token-abcdef123456")
    );
    assert_eq!(
        loaded.museai_cookie.as_deref(),
        Some("session=auth-cookie-value")
    );

    // .env remains untouched
    let env_after = fs::read_to_string(&tmp_env).unwrap();
    assert_eq!(env_after, env_initial_content);

    let _ = fs::remove_file(tmp_env);
    let _ = fs::remove_file(tmp_json);
}

#[test]
fn test_dashboard_update_muse_config_validation_and_secrets_preservation() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    let tmp_json = std::env::temp_dir().join(format!("test_cfg_{}.json", uuid::Uuid::new_v4()));
    let store = ConfigStore::new_with_limits_and_config_file(
        test_config(),
        tmp_env.clone(),
        tmp_json.clone(),
        256,
        1024 * 1024,
    );

    // Initial credentials
    {
        let mut conf = store.write_guard();
        conf.museai_access_token = "existing-token".into();
        conf.museai_cookie = "existing-cookie".into();
    }

    // Update only vm_id leaving tokens blank
    let partial_payload = br#"{"vm_id":"vm-new-456","base_url":"","access_token":""}"#;
    let (head, body) = request_gateway(
        "POST",
        "/api/v1/dashboard/muse-config",
        partial_payload,
        &store,
    );
    assert_eq!(head.status, 200);

    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["museai"]["vm_id"], "vm-new-456");
    assert_eq!(json["museai"]["has_access_token"], true);
    assert_eq!(json["museai"]["has_cookie"], true);

    let snapshot = store.snapshot();
    assert_eq!(snapshot.museai_vm_id, "vm-new-456");
    assert_eq!(snapshot.museai_access_token, "existing-token");
    assert_eq!(snapshot.museai_cookie, "existing-cookie");

    // Invalid base_url
    let bad_base = br#"{"base_url":"ftp://invalid-url"}"#;
    let (head_bad_base, _) =
        request_gateway("POST", "/api/v1/dashboard/muse-config", bad_base, &store);
    assert_eq!(head_bad_base.status, 400);

    // Invalid ws_url
    let bad_ws = br#"{"ws_url":"http://not-a-websocket"}"#;
    let (head_bad_ws, _) = request_gateway("POST", "/api/v1/dashboard/muse-config", bad_ws, &store);
    assert_eq!(head_bad_ws.status, 400);

    // Invalid JSON
    let (head_bad_json, _) = request_gateway(
        "POST",
        "/api/v1/dashboard/muse-config",
        b"broken-json",
        &store,
    );
    assert_eq!(head_bad_json.status, 400);

    // Method Not Allowed
    let (head_get, _) = request_gateway("GET", "/api/v1/dashboard/muse-config", b"", &store);
    assert_eq!(head_get.status, 405);

    let _ = fs::remove_file(tmp_env);
    let _ = fs::remove_file(tmp_json);
}
