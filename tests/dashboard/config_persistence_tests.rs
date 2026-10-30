use crate::config_storage_tests::{request_gateway, test_config};
use std::fs;
use toolcase_gateway::config::{load_persistent_config, ConfigStore};

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
        "cookie": "session=auth-cookie-value",
        "default_video_prompt": "Updated dynamic video prompt",
        "approval_mode": "on",
        "allowed_destinations": ["muse.ai", "api.muse.ai"],
        "allowed_types": ["network"],
        "timeout_secs": 45
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
    assert_eq!(
        json["museai"]["default_video_prompt"],
        "Updated dynamic video prompt"
    );
    assert_eq!(json["museai"]["approval"]["mode"], "on");
    assert_eq!(
        json["museai"]["approval"]["allowed_destinations"],
        serde_json::json!(["muse.ai", "api.muse.ai"])
    );
    assert_eq!(
        json["museai"]["approval"]["allowed_types"],
        serde_json::json!(["network"])
    );
    assert_eq!(json["museai"]["approval"]["timeout_secs"], 45);

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
    assert_eq!(
        snapshot.default_video_prompt,
        "Updated dynamic video prompt"
    );
    assert_eq!(
        snapshot.muse_approval.mode,
        toolcase_gateway::config::MuseApprovalMode::ScopedAuto
    );
    assert_eq!(
        snapshot.muse_approval.allowed_destinations,
        vec!["muse.ai", "api.muse.ai"]
    );
    assert_eq!(snapshot.muse_approval.allowed_types, vec!["network"]);
    assert_eq!(snapshot.muse_approval.approval_timeout_secs, 45);

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
    assert_eq!(
        loaded.default_video_prompt.as_deref(),
        Some("Updated dynamic video prompt")
    );
    assert_eq!(loaded.muse_approval_mode.as_deref(), Some("on"));
    assert_eq!(
        loaded.muse_approval_allowed_destinations,
        Some(vec!["muse.ai".into(), "api.muse.ai".into()])
    );
    assert_eq!(
        loaded.muse_approval_allowed_types,
        Some(vec!["network".into()])
    );
    assert_eq!(loaded.muse_approval_timeout_secs, Some(45));

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

    // Invalid approval_mode
    let bad_mode = br#"{"approval_mode":"super-allow-everything"}"#;
    let (head_bad_mode, _) =
        request_gateway("POST", "/api/v1/dashboard/muse-config", bad_mode, &store);
    assert_eq!(head_bad_mode.status, 400);

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

#[test]
fn dashboard_accounts_openrouter_explicit_key_and_cookie_ingest_and_persistence() {
    let tmp_env = std::env::temp_dir().join(format!("test_env_{}", uuid::Uuid::new_v4()));
    fs::write(&tmp_env, "GW_LISTEN_PORT=20129\n").unwrap();

    let cfg = crate::dashboard_tests::test_config();
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
