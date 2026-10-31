use super::config_storage_tests::{request_gateway, test_config};
use std::fs;
use toolcase_gateway::config::{load_persistent_config, ConfigStore};

#[test]
fn test_dashboard_module_position_and_reordering_persistence() {
    let tmp_env = std::env::temp_dir().join(format!("test_mod_pos_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    // 1. Pin jev to sidebar with menu_order 5 (first position)
    let pos_payload = serde_json::json!({
        "id": "jev",
        "menu_position": "sidebar",
        "menu_order": 5
    });
    let (head, body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/position",
        &serde_json::to_vec(&pos_payload).unwrap(),
        &store,
    );
    assert_eq!(head.status, 200);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["menu_position"], "sidebar");
    assert_eq!(json["menu_order"], 5);
    assert_eq!(json["modules"][0]["id"], "jev");

    // 2. Move jev down one slot
    let move_payload = serde_json::json!({
        "id": "jev",
        "direction": "down"
    });
    let (head2, body2) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/position",
        &serde_json::to_vec(&move_payload).unwrap(),
        &store,
    );
    assert_eq!(head2.status, 200);
    let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
    assert_eq!(json2["modules"][1]["id"], "jev");

    // 3. Verify persistence in gateway_config.json
    let persisted = load_persistent_config(&store.config_file).expect("persisted config exists");
    let pos_map = persisted
        .module_positions
        .expect("module_positions persisted");
    assert_eq!(pos_map.get("jev").unwrap().menu_position, "sidebar");

    let _ = fs::remove_file(&tmp_env);
    let _ = fs::remove_file(&store.config_file);
}

#[test]
fn test_automatic_module_ui_config_and_standalone_page() {
    let tmp_env = std::env::temp_dir().join(format!("test_mod_cfg_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    // Install tiktok-studio from system catalog
    let install_payload = serde_json::json!({
        "id": "tiktok-studio"
    });
    let (inst_head, _) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/install",
        &serde_json::to_vec(&install_payload).unwrap(),
        &store,
    );
    assert_eq!(inst_head.status, 200);

    // 1. Standalone automatic UI config page works for any module
    let (page_head, page_body) =
        request_gateway("GET", "/modules/tiktok-studio/config", b"", &store);
    assert_eq!(page_head.status, 200);
    let html = String::from_utf8_lossy(&page_body);
    assert!(html.contains("Module UI Configuration"));
    assert!(html.contains("/api/v1/dashboard/modules/config"));

    // 2. GET module config returns schema and masked config
    let (get_head, get_body) = request_gateway(
        "GET",
        "/api/v1/dashboard/modules/config?id=tiktok-studio",
        b"",
        &store,
    );
    assert_eq!(get_head.status, 200);
    let get_json: serde_json::Value = serde_json::from_slice(&get_body).unwrap();
    assert_eq!(get_json["id"], "tiktok-studio");
    assert!(get_json["config_schema"]["fields"].is_array());

    // 3. POST module config updates secret and non-secret values and masks secrets in response
    let update_payload = serde_json::json!({
        "id": "tiktok-studio",
        "menu_position": "providers",
        "menu_order": 15,
        "config": {
            "base_url": "https://studio-custom.tiktok.com",
            "cookie": "sessionid=super_secret_cookie_999999",
            "csrf_token": "csrf_secret_token_888888",
            "enabled": true
        }
    });
    let (post_head, post_body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/config",
        &serde_json::to_vec(&update_payload).unwrap(),
        &store,
    );
    assert_eq!(post_head.status, 200);
    let post_json: serde_json::Value = serde_json::from_slice(&post_body).unwrap();
    assert_eq!(post_json["ok"], true);
    assert_eq!(
        post_json["config"]["base_url"],
        "https://studio-custom.tiktok.com"
    );
    assert_eq!(post_json["config"]["masked_cookie"], "sessio...9999");
    assert!(post_json["config"].get("cookie").is_none());

    // Clean up installed module files
    let remove_payload = serde_json::json!({ "id": "tiktok-studio" });
    let _ = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/remove",
        &serde_json::to_vec(&remove_payload).unwrap(),
        &store,
    );

    let _ = fs::remove_file(&tmp_env);
    let _ = fs::remove_file(&store.config_file);
}

#[test]
fn test_tiktok_creator_studio_adapter_alias_and_config_sync() {
    let state_dir = toolcase_gateway::gateway::installer::get_state_dir();
    toolcase_gateway::gateway::installer::cleanup_module_files("tiktok-studio", &state_dir);
    toolcase_gateway::gateway::installer::cleanup_module_files(
        "TikTok-Creator-Studio-Adapter",
        &state_dir,
    );

    let tmp_env =
        std::env::temp_dir().join(format!("test_mod_tiktok_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    // Ensure state isolation in registry
    let _ = store
        .module_registry()
        .write()
        .unwrap()
        .unregister_personal_module("tiktok-studio");

    let manifest = toolcase_gateway::gateway::modules::PersonalManifest {
        id: "TikTok-Creator-Studio-Adapter".to_string(),
        name: "TikTok Creator Studio Adapter".to_string(),
        version: "1.0.0".to_string(),
        namespace: "/tiktok".to_string(),
        entrypoint: "/tmp/adapter.py".to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec![
            "studio".to_string(),
            "items".to_string(),
            "download".to_string(),
            "publish".to_string(),
            "upload".to_string(),
            "chat_completions".to_string(),
            "models".to_string(),
        ],
        icon: None,
        short_name: None,
        config_schema: Some(serde_json::json!({
            "module_id": "tiktok-studio",
            "namespace": "/custom/tiktok",
            "fields": [
                {
                    "key": "base_url",
                    "label": "Base URL",
                    "type": "url",
                    "secret": false,
                    "default": "https://creator.tiktok.com"
                },
                {
                    "key": "cookie",
                    "label": "Session Cookie",
                    "type": "password",
                    "secret": true
                }
            ],
            "har_endpoint": "/custom/tiktok/v1/config/har"
        })),
        permissions: vec![],
        group: None,
    };

    store
        .module_registry()
        .write()
        .unwrap()
        .register_personal_module(manifest)
        .expect("register personal module");

    // 1. GET module config by exact ID
    let (head1, body1) = request_gateway(
        "GET",
        "/api/v1/dashboard/modules/config?id=TikTok-Creator-Studio-Adapter",
        b"",
        &store,
    );
    assert_eq!(head1.status, 200);
    let json1: serde_json::Value = serde_json::from_slice(&body1).unwrap();
    assert_eq!(json1["id"], "TikTok-Creator-Studio-Adapter");
    // Verify normalized har_endpoint matching current namespace
    assert_eq!(
        json1["config_schema"]["har_endpoint"],
        "/tiktok/v1/config/har"
    );

    // 2. GET module config by catalog alias (tiktok-studio)
    let (head2, body2) = request_gateway(
        "GET",
        "/api/v1/dashboard/modules/config?id=tiktok-studio",
        b"",
        &store,
    );
    assert_eq!(head2.status, 200);
    let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
    assert!(
        json2["id"] == "TikTok-Creator-Studio-Adapter" || json2["id"] == "tiktok-studio",
        "expected resolved id to be TikTok-Creator-Studio-Adapter or tiktok-studio, got {:?}",
        json2["id"]
    );

    // 3. GET module config by namespace alias (tiktok)
    let (head3, body3) = request_gateway(
        "GET",
        "/api/v1/dashboard/modules/config?id=tiktok",
        b"",
        &store,
    );
    assert_eq!(head3.status, 200);
    let json3: serde_json::Value = serde_json::from_slice(&body3).unwrap();
    assert!(
        json3["id"] == "TikTok-Creator-Studio-Adapter" || json3["id"] == "tiktok-studio",
        "expected resolved id to be TikTok-Creator-Studio-Adapter or tiktok-studio, got {:?}",
        json3["id"]
    );

    // 4. Standalone UI routes (/modules/TikTok-Creator-Studio-Adapter/config and /tiktok-config)
    let (h_page1, b_page1) = request_gateway(
        "GET",
        "/modules/TikTok-Creator-Studio-Adapter/config",
        b"",
        &store,
    );
    assert_eq!(h_page1.status, 200);
    assert!(h_page1
        .headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("cache-control") && v.contains("no-cache")));
    assert!(String::from_utf8_lossy(&b_page1).contains("Module UI Configuration"));

    let (h_page2, _) = request_gateway("GET", "/tiktok-config", b"", &store);
    assert_eq!(h_page2.status, 200);

    // 5. POST module config via alias 'tiktok-studio' and update settings
    let update_payload = serde_json::json!({
        "id": "tiktok-studio",
        "config": {
            "base_url": "https://custom-creator.tiktok.com",
            "cookie": "sessionid=tiktok_secret_val_12345",
            "enabled": false
        }
    });
    let (h_post, b_post) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/config",
        &serde_json::to_vec(&update_payload).unwrap(),
        &store,
    );
    assert_eq!(h_post.status, 200);
    let post_json: serde_json::Value = serde_json::from_slice(&b_post).unwrap();
    assert_eq!(post_json["ok"], true);
    assert_eq!(post_json["module_status"], "disabled");
    assert_eq!(
        post_json["config"]["base_url"],
        "https://custom-creator.tiktok.com"
    );
    assert_eq!(post_json["config"]["masked_cookie"], "sessio...2345");

    // 6. Verify effective module config
    let effective = store
        .module_registry()
        .read()
        .unwrap()
        .get_effective_module_config("TikTok-Creator-Studio-Adapter");
    assert_eq!(
        effective.get("base_url").and_then(|v| v.as_str()),
        Some("https://custom-creator.tiktok.com")
    );
    assert_eq!(
        effective.get("cookie").and_then(|v| v.as_str()),
        Some("sessionid=tiktok_secret_val_12345")
    );

    let _ = fs::remove_file(&tmp_env);
    let _ = fs::remove_file(&store.config_file);
}
