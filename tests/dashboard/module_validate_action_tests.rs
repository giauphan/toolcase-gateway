use super::config_storage_tests::{request_gateway, test_config};
use std::fs;
use toolcase_gateway::config::ConfigStore;

#[test]
fn test_dashboard_validate_module_config_valid_source() {
    let tmp_env = std::env::temp_dir().join(format!("test_val_env_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    let src_dir = std::env::temp_dir().join(format!("test_val_src_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&src_dir).unwrap();

    fs::write(
        src_dir.join("module.json"),
        serde_json::json!({
            "id": "val-demo-mod",
            "name": "Validation Demo Module",
            "version": "1.0.0",
            "namespace": "/custom/val-demo",
            "entrypoint": "run.sh",
            "protocol": "stdio-stream",
            "capabilities": ["chat_completions", "models"]
        })
        .to_string(),
    )
    .unwrap();

    let script_content = "#!/bin/sh\necho 'ok'\n";
    fs::write(src_dir.join("run.sh"), script_content).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(src_dir.join("run.sh"), fs::Permissions::from_mode(0o755));
    }

    let validate_payload = serde_json::json!({
        "source": src_dir.to_string_lossy().to_string()
    });
    let (head, body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/validate",
        &serde_json::to_vec(&validate_payload).unwrap(),
        &store,
    );
    assert_eq!(head.status, 200);
    let res: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["valid"], true);
    assert_eq!(res["module"]["id"], "val-demo-mod");
    assert_eq!(res["module"]["name"], "Validation Demo Module");
    assert_eq!(res["module"]["namespace"], "/custom/val-demo");
    assert_eq!(res["module"]["has_manifest_file"], true);

    // Verify non-destructive: module must NOT be in the registry
    let reg = store.module_registry();
    let reg_guard = reg.read().unwrap();
    assert!(reg_guard.get_module("val-demo-mod").is_none());

    let _ = fs::remove_dir_all(&src_dir);
    let _ = fs::remove_file(&tmp_env);
}

#[test]
fn test_dashboard_validate_module_config_reserved_namespace_rejection() {
    let tmp_env = std::env::temp_dir().join(format!("test_val_env_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    let src_dir = std::env::temp_dir().join(format!("test_val_src_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&src_dir).unwrap();

    fs::write(
        src_dir.join("module.json"),
        serde_json::json!({
            "id": "val-bad-ns",
            "name": "Bad Namespace Module",
            "version": "1.0.0",
            "namespace": "/dashboard/evil",
            "entrypoint": "run.sh",
            "protocol": "stdio-stream",
            "capabilities": ["chat"]
        })
        .to_string(),
    )
    .unwrap();

    let script_content = "#!/bin/sh\necho 'ok'\n";
    fs::write(src_dir.join("run.sh"), script_content).unwrap();

    let validate_payload = serde_json::json!({
        "source": src_dir.to_string_lossy().to_string()
    });
    let (head, body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/validate",
        &serde_json::to_vec(&validate_payload).unwrap(),
        &store,
    );
    assert_eq!(head.status, 200);
    let res: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(res["valid"], false);
    assert!(res["message"]
        .as_str()
        .unwrap()
        .contains("reserved by system"));

    let _ = fs::remove_dir_all(&src_dir);
    let _ = fs::remove_file(&tmp_env);
}

#[test]
fn test_dashboard_validate_module_config_auto_synthesis_without_manifest() {
    let tmp_env = std::env::temp_dir().join(format!("test_val_env_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    let src_dir = std::env::temp_dir().join(format!("test_synth_src_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&src_dir).unwrap();

    fs::write(src_dir.join("main.py"), "print('hello from python')").unwrap();

    let validate_payload = serde_json::json!({
        "source": src_dir.to_string_lossy().to_string()
    });
    let (head, body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/validate",
        &serde_json::to_vec(&validate_payload).unwrap(),
        &store,
    );
    assert_eq!(head.status, 200);
    let res: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["valid"], true);
    assert_eq!(res["module"]["entrypoint_type"], "python-script");
    assert_eq!(res["module"]["has_manifest_file"], false);
    assert!(res["module"]["namespace"]
        .as_str()
        .unwrap()
        .starts_with("/custom/"));

    let _ = fs::remove_dir_all(&src_dir);
    let _ = fs::remove_file(&tmp_env);
}

#[test]
fn test_dashboard_validate_module_direct_payload() {
    let tmp_env = std::env::temp_dir().join(format!("test_val_env_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    let validate_payload = serde_json::json!({
        "id": "direct-val-mod",
        "name": "Direct Module",
        "namespace": "/custom/direct",
        "entrypoint": "http://127.0.0.1:8080",
        "protocol": "local-http"
    });
    let (head, body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/validate",
        &serde_json::to_vec(&validate_payload).unwrap(),
        &store,
    );
    assert_eq!(head.status, 200);
    let res: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["valid"], true);
    assert_eq!(res["module"]["id"], "direct-val-mod");
    assert_eq!(res["module"]["entrypoint_type"], "local-http");

    let _ = fs::remove_file(&tmp_env);
}

#[test]
fn test_dashboard_validate_module_empty_source_string_treated_as_direct() {
    let tmp_env = std::env::temp_dir().join(format!("test_val_env_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env.clone());

    let validate_payload = serde_json::json!({
        "source": "   ",
        "id": "whitespace-src-mod",
        "namespace": "/custom/ws-src",
        "entrypoint": "http://127.0.0.1:9090",
        "protocol": "local-http",
        "capabilities": ["custom_cap_1", "custom_cap_2"]
    });
    let (head, body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/validate",
        &serde_json::to_vec(&validate_payload).unwrap(),
        &store,
    );
    assert_eq!(head.status, 200);
    let res: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["valid"], true);
    assert_eq!(res["module"]["id"], "whitespace-src-mod");
    assert_eq!(
        res["module"]["capabilities"],
        serde_json::json!(["custom_cap_1", "custom_cap_2"])
    );

    let _ = fs::remove_file(&tmp_env);
}
