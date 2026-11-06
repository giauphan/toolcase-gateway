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
            tools: None,
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
            tools: None,
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

    // Verify valid JSON parseability and CORS header on error response
    let parsed: serde_json::Value =
        serde_json::from_slice(&body).expect("Error response must be valid JSON");
    assert!(parsed["message"].is_string());
    assert_eq!(
        toolcase_gateway::http::header_value(&head.headers, "access-control-allow-origin"),
        Some("*")
    );

    let _ = fs::remove_dir_all(&mod_dir);
    let _ = fs::remove_file(&tmp_bin);
    let _ = fs::remove_file(&tmp_env);
    let _ = fs::remove_file(&store.config_file);
}

#[test]
fn test_write_error_multiline_json_integrity() {
    let msg_with_newlines =
        "Error on line 1: syntax error\n\tDetailed trace:\r\n  \"foo\": \"bar\"";
    let escaped = toolcase_gateway::rewrite::escape_json_string(msg_with_newlines);
    let raw_json = format!("{{\"error\":{{\"message\":\"{escaped}\"}},\"message\":\"{escaped}\"}}");

    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&raw_json);
    assert!(
        parsed.is_ok(),
        "Raw JSON must parse without control character errors: {:?}",
        parsed.err()
    );
    let json_val = parsed.unwrap();
    assert_eq!(json_val["message"].as_str().unwrap(), msg_with_newlines);
}

#[test]
fn test_dashboard_upgrade_queue_status_and_in_flight_guard() {
    let tmp_env =
        std::env::temp_dir().join(format!("test_queue_status_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    // 1. Initial queue status should report valid JSON
    let (head, body) = request_gateway(
        "GET",
        "/api/v1/dashboard/modules/upgrade/queue",
        &[],
        &store,
    );
    assert_eq!(head.status, 200);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["ok"], true);
    assert!(parsed["count"].is_number());

    // 2. Lock a module in the registry and verify queue status reports it
    {
        let guard = toolcase_gateway::features::dashboard::try_acquire_upgrade("simulated-mod")
            .expect("acquire succeeds");
        let (head_busy, body_busy) = request_gateway(
            "GET",
            "/api/v1/dashboard/modules/upgrade/status",
            &[],
            &store,
        );
        assert_eq!(head_busy.status, 200);
        let parsed_busy: serde_json::Value = serde_json::from_slice(&body_busy).unwrap();
        assert!(parsed_busy["in_flight"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "simulated-mod"));

        drop(guard);
    }

    let _ = fs::remove_file(&tmp_env);
    let _ = fs::remove_file(&store.config_file);
}

#[test]
fn test_upgrade_lock_stale_expiration_and_recovery() {
    use std::time::Duration;
    toolcase_gateway::features::dashboard::clear_all_upgrade_locks();

    let test_mod = "test-expire-mod";
    let short_ttl = Duration::from_millis(50);

    // Acquire lock with short TTL
    let _guard =
        toolcase_gateway::features::dashboard::try_acquire_upgrade_with_ttl(test_mod, short_ttl)
            .expect("initial acquire succeeds");

    // Immediate second acquire must fail with conflict
    let err =
        toolcase_gateway::features::dashboard::try_acquire_upgrade_with_ttl(test_mod, short_ttl);
    assert!(err.is_err());
    assert!(err.unwrap_err().contains("currently being upgraded"));

    // Sleep past TTL to simulate orphaned / stale lock
    std::thread::sleep(Duration::from_millis(70));

    // Next acquire should succeed and evict the stale lock
    let guard2 =
        toolcase_gateway::features::dashboard::try_acquire_upgrade_with_ttl(test_mod, short_ttl)
            .expect("acquire succeeds by evicting expired lock");
    drop(guard2);

    toolcase_gateway::features::dashboard::clear_all_upgrade_locks();
}

#[test]
fn test_copy_dir_all_preserves_modification_times() {
    let tmp_src = std::env::temp_dir().join(format!("test_src_mtime_{}", uuid::Uuid::new_v4()));
    let tmp_dst = std::env::temp_dir().join(format!("test_dst_mtime_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&tmp_src).unwrap();

    let file_path = tmp_src.join("sample.bin");
    fs::write(&file_path, b"hello binary").unwrap();

    // Set an older timestamp (1 hour ago)
    let older_time = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let f = fs::OpenOptions::new().write(true).open(&file_path).unwrap();
    f.set_modified(older_time).unwrap();
    drop(f);

    let src_mtime = fs::metadata(&file_path).unwrap().modified().unwrap();

    // Copy via copy_dir_all
    toolcase_gateway::gateway::installer::copy_dir_all(&tmp_src, &tmp_dst).unwrap();

    let dst_file = tmp_dst.join("sample.bin");
    assert!(dst_file.is_file());
    let dst_mtime = fs::metadata(&dst_file).unwrap().modified().unwrap();

    // Destination mtime should match source mtime (within 1 second tolerance for fs precision)
    let diff = if src_mtime > dst_mtime {
        src_mtime.duration_since(dst_mtime).unwrap()
    } else {
        dst_mtime.duration_since(src_mtime).unwrap()
    };
    assert!(
        diff < std::time::Duration::from_secs(2),
        "mtime diff too large: {:?}",
        diff
    );

    let _ = fs::remove_dir_all(&tmp_src);
    let _ = fs::remove_dir_all(&tmp_dst);
}
