use crate::dashboard_tests::{request_gateway, test_config};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use toolcase_gateway::config::ConfigStore;
use toolcase_gateway::features::dashboard::mask_key;

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
        "jev_openrouter_fallback_models": ["model-one", "model-two"],
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
    assert_eq!(
        parsed["config"]["jev_openrouter_fallback_models"],
        serde_json::json!(["model-one", "model-two"])
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
    assert_eq!(
        snapshot.jev_openrouter_fallback_models,
        vec!["model-one".to_string(), "model-two".to_string()]
    );
    assert_eq!(snapshot.jev_credit_cooldown_secs, 600);

    // Verify persistent config file has settings
    let persistent = toolcase_gateway::config::load_persistent_config(&store.config_file).unwrap();
    assert_eq!(
        persistent.jev_openrouter_fallback_models.as_ref().unwrap(),
        &vec!["model-one".to_string(), "model-two".to_string()]
    );
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
