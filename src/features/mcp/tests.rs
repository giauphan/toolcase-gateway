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
    let tools = list_tools();
    assert!(tools.iter().any(|t| t.name == "gateway_list_modules"));
    assert!(tools
        .iter()
        .any(|t| t.name == "gateway_install_personal_module"));

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
