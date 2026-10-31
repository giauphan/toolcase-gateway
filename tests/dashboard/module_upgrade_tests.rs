use super::config_storage_tests::{request_gateway, test_config};
use std::fs;
use toolcase_gateway::config::ConfigStore;
use toolcase_gateway::gateway::modules::ModuleStatus;

#[test]
fn test_dashboard_upgrade_module_endpoint_and_preservation() {
    let tmp_env = std::env::temp_dir().join(format!("test_mod_upg_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    let state_dir = toolcase_gateway::gateway::installer::get_state_dir();
    let mod_dir = state_dir.join("modules").join("my-dynamic-mod");
    let _ = fs::remove_dir_all(&mod_dir);

    let v1_dir = std::env::temp_dir().join(format!("v1_src_{}", uuid::Uuid::new_v4()));
    let v2_dir = std::env::temp_dir().join(format!("v2_src_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&v1_dir).unwrap();
    fs::create_dir_all(&v2_dir).unwrap();

    // Create v1 module files
    fs::write(
        v1_dir.join("module.json"),
        serde_json::json!({
            "id": "my-dynamic-mod",
            "name": "My Dynamic Mod v1",
            "version": "1.0.0",
            "namespace": "/custom/dyn-mod",
            "entrypoint": "run.sh",
            "protocol": "stdio-stream",
            "capabilities": ["chat"]
        })
        .to_string(),
    )
    .unwrap();
    fs::write(v1_dir.join("run.sh"), "#!/bin/sh\necho v1").unwrap();

    // Create v2 module files
    fs::write(
        v2_dir.join("module.json"),
        serde_json::json!({
            "id": "my-dynamic-mod",
            "name": "My Dynamic Mod v2",
            "version": "2.5.0",
            "namespace": "/custom/dyn-mod",
            "entrypoint": "run.sh",
            "protocol": "stdio-stream",
            "capabilities": ["chat", "video", "search"]
        })
        .to_string(),
    )
    .unwrap();
    fs::write(v2_dir.join("run.sh"), "#!/bin/sh\necho v2").unwrap();

    // 1. Install v1 via dashboard endpoint
    let install_payload = serde_json::json!({
        "id": "my-dynamic-mod",
        "source": v1_dir.to_string_lossy().to_string()
    });
    let (head, body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/install",
        &serde_json::to_vec(&install_payload).unwrap(),
        &store,
    );
    assert_eq!(
        head.status,
        200,
        "Install response: {}",
        String::from_utf8_lossy(&body)
    );

    // 2. Configure secret and custom settings for the module
    let config_payload = serde_json::json!({
        "id": "my-dynamic-mod",
        "config": {
            "api_key": "sk-secret-credentials-987",
            "base_url": "https://api.custom.com",
            "enabled": false
        },
        "menu_position": "sidebar",
        "menu_order": 25
    });
    let (head_cfg, body_cfg) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/config",
        &serde_json::to_vec(&config_payload).unwrap(),
        &store,
    );
    assert_eq!(
        head_cfg.status,
        200,
        "Config response: {}",
        String::from_utf8_lossy(&body_cfg)
    );

    // Verify initial settings before upgrade
    {
        let reg = store.module_registry();
        let guard = reg.read().unwrap();
        let desc = guard
            .get_module("my-dynamic-mod")
            .expect("module registered");
        assert_eq!(desc.version.as_deref(), Some("1.0.0"));
        assert_eq!(desc.status, ModuleStatus::Disabled);
        assert_eq!(desc.menu_position, "sidebar");
        assert_eq!(desc.menu_order, 25);
    }

    // 3. Upgrade to v2 using /api/v1/dashboard/modules/upgrade
    let upgrade_payload = serde_json::json!({
        "id": "my-dynamic-mod",
        "source": v2_dir.to_string_lossy().to_string()
    });
    let (head_upg, body_upg) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/upgrade",
        &serde_json::to_vec(&upgrade_payload).unwrap(),
        &store,
    );
    assert_eq!(
        head_upg.status,
        200,
        "Upgrade response: {}",
        String::from_utf8_lossy(&body_upg)
    );
    let upg_json: serde_json::Value = serde_json::from_slice(&body_upg).unwrap();
    assert_eq!(upg_json["ok"], true);
    assert_eq!(upg_json["version"], "2.5.0");
    assert_eq!(upg_json["module"]["name"], "My Dynamic Mod v2");

    // 4. Verify that registry updated the version and capabilities, but preserved position, order, and status
    {
        let reg = store.module_registry();
        let guard = reg.read().unwrap();
        let desc = guard
            .get_module("my-dynamic-mod")
            .expect("module registered");
        assert_eq!(desc.version.as_deref(), Some("2.5.0"));
        assert_eq!(desc.status, ModuleStatus::Disabled); // Preserved!
        assert_eq!(desc.menu_position, "sidebar"); // Preserved!
        assert_eq!(desc.menu_order, 25); // Preserved!
        assert!(desc.capabilities.contains(&"search".to_string())); // Updated!
        assert!(desc.capabilities.contains(&"video".to_string()));

        // Verify secret credentials survived the upgrade
        // Verify secret credentials survived the upgrade
        let disk_cfg = guard
            .read_disk_module_config("my-dynamic-mod")
            .expect("disk config exists");
        assert_eq!(
            disk_cfg.get("api_key").and_then(|v| v.as_str()),
            Some("sk-secret-credentials-987")
        );
        let masked = guard.get_masked_module_config("my-dynamic-mod");
        assert_eq!(
            masked.get("has_api_key").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            masked.get("base_url").and_then(|v| v.as_str()),
            Some("https://api.custom.com")
        );
        assert_eq!(
            masked.get("base_url").and_then(|v| v.as_str()),
            Some("https://api.custom.com")
        );
    }

    // 5. Test nonexistent module returns 404
    let (head_404, _) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/upgrade",
        br#"{"id": "non-existent-module"}"#,
        &store,
    );
    assert_eq!(head_404.status, 404);

    // 6. Test built-in module upgrade is rejected with 400
    let (head_builtin, _) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/upgrade",
        br#"{"id": "omniroute"}"#,
        &store,
    );
    assert_eq!(head_builtin.status, 400);

    // Cleanup
    let _ = fs::remove_dir_all(&v1_dir);
    let _ = fs::remove_dir_all(&v2_dir);
    let _ = fs::remove_dir_all(&mod_dir);
    let _ = fs::remove_file(&tmp_env);
    let _ = fs::remove_file(&store.config_file);
}
