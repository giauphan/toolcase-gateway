use super::tools::*;
use crate::config::ConfigStore;
use serde_json::json;

#[test]
fn system_metrics_reports_a_poisoned_registry() {
    let env_path =
        std::env::temp_dir().join(format!("mcp_metrics_test_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(crate::config::Config::default(), env_path.clone());
    let registry = store.module_registry();
    let poison_target = registry.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poison_target.write().unwrap();
        panic!("poison module registry for test");
    })
    .join();

    let err = call_tool("gateway_get_system_metrics", &json!({}), &store).unwrap_err();
    assert!(err.contains("Module registry unavailable"));

    let _ = std::fs::remove_file(&store.config_file);
    let _ = std::fs::remove_file(&env_path);
}

#[test]
fn test_mcp_tools_and_resources_lifecycle() {
    let env_path =
        std::env::temp_dir().join(format!("mcp_tools_test_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(crate::config::Config::default(), env_path.clone());

    // 1. List tools
    let tools = list_tools(&store);
    assert!(tools.iter().any(|t| t.name == "gateway_list_modules"));
    assert!(tools
        .iter()
        .any(|t| t.name == "gateway_install_personal_module"));
    assert!(tools
        .iter()
        .any(|t| t.name == "gateway_upgrade_personal_module"));
    // Verify unadapted built-in modules do not advertise synthetic tools
    assert!(!tools.iter().any(|t| t.name.starts_with("jev__")));

    // 2. Call gateway_list_modules
    let res = call_tool("gateway_list_modules", &json!({}), &store).unwrap();
    let text = res["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("omniroute"));
    assert!(text.contains("muse"));
    assert!(text.contains("jev"));

    // 3. Install personal module
    let install_params = json!({
        "id": "my-claude-agent",
        "name": "Claude Agent Module",
        "version": "1.0.0",
        "namespace": "/personal/claude",
        "entrypoint": "/bin/echo",
        "protocol": "stdio-stream",
        "capabilities": ["chat"]
    });
    let install_res =
        call_tool("gateway_install_personal_module", &install_params, &store).unwrap();
    assert!(install_res["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("installed"));

    // 4. Verify module info
    let info_res = call_tool(
        "gateway_get_module_info",
        &json!({"id": "my-claude-agent"}),
        &store,
    )
    .unwrap();
    assert!(info_res["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("/personal/claude"));

    // 4b. Discover module tool and read module resource
    let updated_tools = list_tools(&store);
    assert!(updated_tools
        .iter()
        .any(|t| t.name == "my-claude-agent__chat"));
    let res_specific = read_resource("gateway://modules/my-claude-agent", &store).unwrap();
    assert!(res_specific["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("my-claude-agent"));

    // 5. Disable module
    let state_res = call_tool(
        "gateway_set_module_state",
        &json!({"id": "my-claude-agent", "enabled": false}),
        &store,
    )
    .unwrap();
    assert!(state_res["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Disabled"));

    // 5b. Verify calling a tool for a disabled module returns an error
    let disabled_err =
        call_tool("my-claude-agent__chat", &json!({"prompt": "hello"}), &store).unwrap_err();
    assert!(disabled_err.contains("is currently disabled"));

    // 6. Read resource gateway://modules
    let res_mod = read_resource("gateway://modules", &store).unwrap();
    let mod_text = res_mod["contents"][0]["text"].as_str().unwrap();
    assert!(mod_text.contains("my-claude-agent"));

    // 7. Remove personal module
    let remove_res = call_tool(
        "gateway_remove_personal_module",
        &json!({"id": "my-claude-agent"}),
        &store,
    )
    .unwrap();
    assert!(remove_res["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("removed"));

    // 8. Call gateway_get_system_metrics
    let metrics_res = call_tool("gateway_get_system_metrics", &json!({}), &store).unwrap();
    let metrics_text = metrics_res["content"][0]["text"].as_str().unwrap();
    assert!(metrics_text.contains("toolcase-gateway"));
    assert!(metrics_text.contains("active_connections"));
    assert!(metrics_text.contains("total_modules"));

    let _ = std::fs::remove_file(&store.config_file);
    let _ = std::fs::remove_file(&env_path);
}

#[test]
fn test_mcp_upgrade_personal_module_tool() {
    let env_path =
        std::env::temp_dir().join(format!("mcp_upgrade_test_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(crate::config::Config::default(), env_path.clone());

    // 1. Missing parameter 'id'
    let missing_id_err =
        call_tool("gateway_upgrade_personal_module", &json!({}), &store).unwrap_err();
    assert!(missing_id_err.contains("Missing parameter 'id'"));

    // 2. Nonexistent module
    let not_found_err = call_tool(
        "gateway_upgrade_personal_module",
        &json!({"id": "nonexistent-module"}),
        &store,
    )
    .unwrap_err();
    assert!(not_found_err.contains("is not installed; cannot upgrade"));

    // 3. Built-in module upgrade rejected
    let builtin_err = call_tool(
        "gateway_upgrade_personal_module",
        &json!({"id": "omniroute"}),
        &store,
    )
    .unwrap_err();
    assert!(builtin_err.contains("Built-in module 'omniroute' cannot be upgraded"));

    // 4. End-to-end upgrade flow
    let v1_dir = std::env::temp_dir().join(format!("v1_mcp_{}", uuid::Uuid::new_v4()));
    let v2_dir = std::env::temp_dir().join(format!("v2_mcp_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&v1_dir);
    let _ = std::fs::create_dir_all(&v2_dir);

    std::fs::write(
        v1_dir.join("module.json"),
        json!({
            "id": "my-upgradable-mod",
            "name": "My Upgradable Mod v1",
            "version": "1.0.0",
            "namespace": "/personal/upgradable",
            "entrypoint": "run.sh",
            "protocol": "stdio-stream",
            "capabilities": ["chat"]
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(v1_dir.join("run.sh"), "#!/bin/sh\necho v1").unwrap();

    std::fs::write(
        v2_dir.join("module.json"),
        json!({
            "id": "my-upgradable-mod",
            "name": "My Upgradable Mod v2",
            "version": "2.1.0",
            "namespace": "/personal/upgradable",
            "entrypoint": "run.sh",
            "protocol": "stdio-stream",
            "capabilities": ["chat", "decisions"]
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(v2_dir.join("run.sh"), "#!/bin/sh\necho v2").unwrap();

    // Install v1
    let install_res = call_tool(
        "gateway_install_personal_module",
        &json!({
            "id": "my-upgradable-mod",
            "source": v1_dir.to_string_lossy().to_string()
        }),
        &store,
    )
    .unwrap();
    assert!(install_res["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("installed"));

    // Check module info before upgrade
    let info_v1 = call_tool(
        "gateway_get_module_info",
        &json!({"id": "my-upgradable-mod"}),
        &store,
    )
    .unwrap();
    let text_v1 = info_v1["content"][0]["text"].as_str().unwrap();
    assert!(text_v1.contains("1.0.0"));

    // Upgrade to v2
    let upgrade_res = call_tool(
        "gateway_upgrade_personal_module",
        &json!({
            "id": "my-upgradable-mod",
            "source": v2_dir.to_string_lossy().to_string()
        }),
        &store,
    )
    .unwrap();
    let upg_text = upgrade_res["content"][0]["text"].as_str().unwrap();
    assert!(upg_text.contains("upgraded successfully to version 2.1.0"));

    // Verify module info after upgrade reflects 2.1.0
    let info_v2 = call_tool(
        "gateway_get_module_info",
        &json!({"id": "my-upgradable-mod"}),
        &store,
    )
    .unwrap();
    let text_v2 = info_v2["content"][0]["text"].as_str().unwrap();
    assert!(text_v2.contains("2.1.0"));
    assert!(text_v2.contains("decisions"));

    // Verify upgrade with invalid source fails closed and preserves v2
    let invalid_res = call_tool(
        "gateway_upgrade_personal_module",
        &json!({
            "id": "my-upgradable-mod",
            "source": "/nonexistent/path/for/upgrade"
        }),
        &store,
    );
    assert!(invalid_res.is_err());

    let info_post_fail = call_tool(
        "gateway_get_module_info",
        &json!({"id": "my-upgradable-mod"}),
        &store,
    )
    .unwrap();
    let text_post_fail = info_post_fail["content"][0]["text"].as_str().unwrap();
    assert!(text_post_fail.contains("2.1.0"));

    // Clean up
    let _ = call_tool(
        "gateway_remove_personal_module",
        &json!({"id": "my-upgradable-mod"}),
        &store,
    );
    let _ = std::fs::remove_dir_all(&v1_dir);
    let _ = std::fs::remove_dir_all(&v2_dir);
    let _ = std::fs::remove_file(&store.config_file);
    let _ = std::fs::remove_file(&env_path);
}
