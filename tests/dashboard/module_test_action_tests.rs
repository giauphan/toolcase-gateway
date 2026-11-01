use super::config_storage_tests::{request_gateway, test_config};
use std::fs;
use toolcase_gateway::config::ConfigStore;

#[test]
fn test_dashboard_test_module_personal_stdio_healthy() {
    let tmp_env = std::env::temp_dir().join(format!("test_mod_test_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    let state_dir = toolcase_gateway::gateway::installer::get_state_dir();
    let mod_dir = state_dir.join("modules").join("stdio-health-mod");
    let _ = fs::remove_dir_all(&mod_dir);

    let src_dir = std::env::temp_dir().join(format!("test_src_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&src_dir).unwrap();

    fs::write(
        src_dir.join("module.json"),
        serde_json::json!({
            "id": "stdio-health-mod",
            "name": "Health Test Module",
            "version": "1.0.0",
            "namespace": "/custom/health-mod",
            "entrypoint": "run.sh",
            "protocol": "stdio-stream",
            "capabilities": ["chat"]
        })
        .to_string(),
    )
    .unwrap();

    let script_content = "#!/bin/sh\nread line\necho '{\"status\":200,\"body\":\"ok\"}'\n";
    fs::write(src_dir.join("run.sh"), script_content).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(src_dir.join("run.sh"), fs::Permissions::from_mode(0o755));
    }

    // Install module via dashboard
    let install_payload = serde_json::json!({
        "id": "stdio-health-mod",
        "source": src_dir.to_string_lossy().to_string()
    });
    let (head, _) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/install",
        &serde_json::to_vec(&install_payload).unwrap(),
        &store,
    );
    assert_eq!(head.status, 200);

    // Test module via test endpoint
    let test_payload = serde_json::json!({ "id": "stdio-health-mod" });
    let (test_head, test_body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/test",
        &serde_json::to_vec(&test_payload).unwrap(),
        &store,
    );
    assert_eq!(test_head.status, 200);
    let res: serde_json::Value = serde_json::from_slice(&test_body).unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["id"], "stdio-health-mod");
    assert_eq!(res["status"], "healthy");
    assert_eq!(res["protocol"], "stdio-stream");
    assert!(res["message"]
        .as_str()
        .unwrap()
        .contains("Module responded successfully"));

    // Also test alias route /api/v1/dashboard/modules/check
    let (check_head, check_body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/check",
        &serde_json::to_vec(&test_payload).unwrap(),
        &store,
    );
    assert_eq!(check_head.status, 200);
    let check_res: serde_json::Value = serde_json::from_slice(&check_body).unwrap();
    assert_eq!(check_res["ok"], true);

    let _ = fs::remove_dir_all(&src_dir);
    let _ = fs::remove_dir_all(&mod_dir);
    let _ = fs::remove_file(&tmp_env);
}

#[test]
fn test_dashboard_test_module_personal_stdio_unhealthy_missing_entrypoint() {
    let tmp_env = std::env::temp_dir().join(format!("test_mod_test_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    let state_dir = toolcase_gateway::gateway::installer::get_state_dir();
    let mod_dir = state_dir.join("modules").join("missing-ep-mod");
    let _ = fs::remove_dir_all(&mod_dir);

    let src_dir = std::env::temp_dir().join(format!("test_src_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&src_dir).unwrap();

    fs::write(
        src_dir.join("module.json"),
        serde_json::json!({
            "id": "missing-ep-mod",
            "name": "Missing EP Module",
            "version": "1.0.0",
            "namespace": "/custom/missing-ep",
            "entrypoint": "run.sh",
            "protocol": "stdio-stream"
        })
        .to_string(),
    )
    .unwrap();
    fs::write(src_dir.join("run.sh"), "#!/bin/sh\necho hi\n").unwrap();

    let install_payload = serde_json::json!({
        "id": "missing-ep-mod",
        "source": src_dir.to_string_lossy().to_string()
    });
    let (head, _) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/install",
        &serde_json::to_vec(&install_payload).unwrap(),
        &store,
    );
    assert_eq!(head.status, 200);

    // Now delete the entrypoint file from the installed target directory
    let installed_ep = mod_dir.join("run.sh");
    let _ = fs::remove_file(installed_ep);

    let test_payload = serde_json::json!({ "id": "missing-ep-mod" });
    let (test_head, test_body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/test",
        &serde_json::to_vec(&test_payload).unwrap(),
        &store,
    );
    assert_eq!(test_head.status, 200);
    let res: serde_json::Value = serde_json::from_slice(&test_body).unwrap();
    assert_eq!(res["ok"], false);
    assert_eq!(res["status"], "unhealthy");
    assert!(res["message"].as_str().unwrap().contains("does not exist"));

    let _ = fs::remove_dir_all(&src_dir);
    let _ = fs::remove_dir_all(&mod_dir);
    let _ = fs::remove_file(&tmp_env);
}

#[test]
fn test_dashboard_test_module_builtin_healthy() {
    let tmp_env = std::env::temp_dir().join(format!("test_mod_test_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    let test_payload = serde_json::json!({ "id": "omniroute" });
    let (head, body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/test",
        &serde_json::to_vec(&test_payload).unwrap(),
        &store,
    );
    assert_eq!(head.status, 200);
    let res: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["id"], "omniroute");
    assert_eq!(res["status"], "healthy");
    assert_eq!(res["protocol"], "builtin");

    let _ = fs::remove_file(&tmp_env);
}

#[test]
fn test_dashboard_test_module_not_found() {
    let tmp_env = std::env::temp_dir().join(format!("test_mod_test_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    let test_payload = serde_json::json!({ "id": "unknown-nonexistent-mod" });
    let (head, body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/test",
        &serde_json::to_vec(&test_payload).unwrap(),
        &store,
    );
    assert_eq!(head.status, 404);
    assert!(String::from_utf8_lossy(&body).contains("not found in registry"));

    let _ = fs::remove_file(&tmp_env);
}

#[test]
fn test_dashboard_test_module_rejects_get() {
    let tmp_env = std::env::temp_dir().join(format!("test_mod_test_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    let (head, body) = request_gateway("GET", "/api/v1/dashboard/modules/test", &[], &store);
    assert_eq!(head.status, 405);
    assert!(String::from_utf8_lossy(&body).contains("only supports POST"));

    let _ = fs::remove_file(&tmp_env);
}
