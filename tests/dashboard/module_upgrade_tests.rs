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

#[test]
fn test_dashboard_upgrade_module_resolves_catalog_and_addon_source() {
    let tmp_env = std::env::temp_dir().join(format!("test_fb_upg_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    // 1. Verify facebook-suite catalog entry has source declared
    let catalog = toolcase_gateway::gateway::modules::get_system_module_catalog();
    let fb_cat = catalog
        .into_iter()
        .find(|c| c.id == "facebook-suite")
        .expect("facebook-suite in catalog");
    assert_eq!(fb_cat.source.as_deref(), Some("addons/facebook-suite"));

    // 2. Register facebook-suite if not yet in registry
    let tmp_bin = std::env::temp_dir().join(format!("test_fb_ep_{}.sh", uuid::Uuid::new_v4()));
    let _ = fs::write(&tmp_bin, "#!/bin/sh\necho ok");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp_bin, fs::Permissions::from_mode(0o755));
    }

    let (prev_manifest, had_installed_fb) = {
        let reg = store.module_registry();
        let mut guard = reg.write().unwrap();
        let prev = guard.get_personal_manifest("facebook-suite").cloned();
        let _ = guard.unregister_personal_module("facebook-suite");
        let manifest = toolcase_gateway::gateway::modules::PersonalManifest {
            id: "facebook-suite".to_string(),
            name: "Meta Business Suite".to_string(),
            short_name: Some("Meta Business Suite".to_string()),
            version: "1.0.0".to_string(),
            namespace: "/custom/facebook".to_string(),
            entrypoint: tmp_bin.to_string_lossy().to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec!["studio".to_string()],
            icon: Some("ⓕ".to_string()),
            config_schema: None,
            permissions: Vec::new(),
            group: None,
            source: None, // Explicitly source-less to exercise catalog source fallback
        };
        guard.register_personal_module(manifest).unwrap();
        let installed_dir = toolcase_gateway::gateway::installer::get_state_dir()
            .join("modules")
            .join("facebook-suite");
        (prev, installed_dir.exists())
    };

    // 3. Upgrade facebook-suite with payload without explicit source
    // It should locate source via catalog / addons and not fail with missing source
    if std::path::Path::new("addons/facebook-suite").is_dir() {
        let (head, body) = request_gateway(
            "POST",
            "/api/v1/dashboard/modules/upgrade",
            br#"{"id": "facebook-suite"}"#,
            &store,
        );
        let resp_str = String::from_utf8_lossy(&body);
        assert_ne!(
            head.status, 400,
            "Upgrade must not fail with missing source 400: {}",
            resp_str
        );
        assert_eq!(head.status, 200, "Upgrade response: {}", resp_str);
        let upg_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(upg_json["ok"], true);
    } else {
        let (head, body) = request_gateway(
            "POST",
            "/api/v1/dashboard/modules/upgrade",
            br#"{"id": "facebook-suite"}"#,
            &store,
        );
        let resp_str = String::from_utf8_lossy(&body);
        assert_eq!(head.status, 400, "Expected 400 bad request: {}", resp_str);
        assert!(
            resp_str.contains("has no configured source repository")
                || resp_str.contains("Local module source directory")
                || resp_str.contains("Failed to copy directory")
                || resp_str.contains("is not currently installed"),
            "Expected failure due to missing source directory or installation in CI, got: {}",
            resp_str
        );
    }

    // 4. Teardown: restore previous registry state and remove test artifacts
    {
        let reg = store.module_registry();
        let mut guard = reg.write().unwrap();
        let _ = guard.unregister_personal_module("facebook-suite");
        if let Some(prev) = prev_manifest {
            let _ = guard.register_personal_module(prev);
        }
    }

    let installed_dir = toolcase_gateway::gateway::installer::get_state_dir()
        .join("modules")
        .join("facebook-suite");
    if !had_installed_fb && installed_dir.exists() {
        let _ = fs::remove_dir_all(&installed_dir);
    }

    let _ = fs::remove_file(&tmp_bin);
    let _ = fs::remove_file(&tmp_env);
    let _ = fs::remove_file(&store.config_file);
}

#[test]
fn test_dashboard_upgrade_module_with_nonexistent_local_dir() {
    let tmp_env =
        std::env::temp_dir().join(format!("test_nonexistent_upg_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    let tmp_bin = std::env::temp_dir().join(format!("test_entry_{}.sh", uuid::Uuid::new_v4()));
    fs::write(&tmp_bin, "#!/bin/sh\nexit 0\n").unwrap();

    let mod_dir = toolcase_gateway::gateway::installer::get_state_dir()
        .join("modules")
        .join("dummy-test-mod");
    fs::create_dir_all(&mod_dir).unwrap();
    fs::write(mod_dir.join("module.json"), "{}").unwrap();

    {
        let reg = store.module_registry();
        let mut guard = reg.write().unwrap();
        let manifest = toolcase_gateway::gateway::modules::PersonalManifest {
            id: "dummy-test-mod".to_string(),
            name: "Dummy Test Module".to_string(),
            short_name: None,
            version: "1.0.0".to_string(),
            namespace: "/custom/dummy".to_string(),
            entrypoint: tmp_bin.to_string_lossy().to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec!["test".to_string()],
            icon: None,
            config_schema: None,
            permissions: Vec::new(),
            group: None,
            source: None,
        };
        guard.register_personal_module(manifest).unwrap();
    }

    let (head, body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/upgrade",
        br#"{"id": "dummy-test-mod", "source": "addons/nonexistent-dummy-dir-xyz"}"#,
        &store,
    );
    let resp_str = String::from_utf8_lossy(&body);
    assert_eq!(head.status, 400, "Expected 400 bad request: {}", resp_str);
    assert!(
        resp_str.contains("does not exist or is not a directory"),
        "Expected clean not-found directory error, got: {}",
        resp_str
    );

    let _ = fs::remove_dir_all(&mod_dir);
    let _ = fs::remove_file(&tmp_bin);
    let _ = fs::remove_file(&tmp_env);
    let _ = fs::remove_file(&store.config_file);
}
