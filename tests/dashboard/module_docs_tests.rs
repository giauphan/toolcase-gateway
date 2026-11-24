use super::config_storage_tests::{request_gateway, test_config};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use toolcase_gateway::config::ConfigStore;
use toolcase_gateway::gateway::modules::{get_system_module_catalog, PersonalManifest};
use toolcase_gateway::http::{read_response_head, ResponseHead};
use toolcase_gateway::routes::route_request;

fn request_gateway_with_headers(
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    store: &ConfigStore,
) -> (ResponseHead, Vec<u8>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store_clone = store.clone();
    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        let _ = route_request(client, &store_clone);
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut header_bytes =
        format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    for (k, v) in headers {
        header_bytes.push_str(&format!("{k}: {v}\r\n"));
    }
    header_bytes.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    client.write_all(header_bytes.as_bytes()).unwrap();
    if !body.is_empty() {
        client.write_all(body).unwrap();
    }
    client.flush().unwrap();

    let head = read_response_head(&mut client).unwrap();
    let mut resp_body = head.buffered_body.clone();
    let _ = client.read_to_end(&mut resp_body);
    handle.join().unwrap();
    (head, resp_body)
}

#[test]
fn test_builtin_modules_have_default_documentation() {
    let tmp_env =
        std::env::temp_dir().join(format!("test_docs_builtin_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env);
    let reg = store.module_registry();
    let guard = reg.read().unwrap();

    // 1. OmniRoute built-in docs
    let omni = guard.get_module("omniroute").expect("omniroute registered");
    assert_eq!(omni.doc_url.as_deref(), Some("/modules/omniroute/doc"));
    let omni_content = omni.doc_content.as_deref().expect("omniroute doc content");
    assert!(omni_content.contains("# OmniRoute LLM Proxy"));
    assert!(omni_content.contains("/ominroute/v1/chat/completions"));

    // 2. Muse built-in docs
    let muse = guard.get_module("muse").expect("muse registered");
    assert_eq!(muse.doc_url.as_deref(), Some("/modules/muse/doc"));
    let muse_content = muse.doc_content.as_deref().expect("muse doc content");
    assert!(muse_content.contains("# Muse AI Provider"));
    assert!(muse_content.contains("/muse-ai/video"));

    // 3. Jev built-in docs
    let jev = guard.get_module("jev").expect("jev registered");
    assert_eq!(jev.doc_url.as_deref(), Some("/modules/jev/doc"));
    let jev_content = jev.doc_content.as_deref().expect("jev doc content");
    assert!(jev_content.contains("# Jev Decision Provider"));
    assert!(jev_content.contains("/jev/decisions"));
}

#[test]
fn test_system_catalog_modules_have_documentation_metadata() {
    let catalog = get_system_module_catalog();
    assert!(!catalog.is_empty(), "Catalog must not be empty");

    for item in &catalog {
        assert!(
            item.doc_url.is_some(),
            "Catalog item '{}' must have a doc_url",
            item.id
        );
        let url = item.doc_url.as_deref().unwrap();
        assert!(
            url.starts_with("http://")
                || url.starts_with("https://")
                || url.starts_with("/modules/"),
            "Invalid doc_url '{}' for item '{}'",
            url,
            item.id
        );

        assert!(
            item.doc_content.is_some(),
            "Catalog item '{}' must have a doc_content block",
            item.id
        );
        let content = item.doc_content.as_deref().unwrap();
        assert!(
            content.contains(&item.name) || content.contains(&item.id),
            "doc_content for '{}' should mention name or id",
            item.id
        );
    }
}

#[test]
fn test_personal_module_registration_and_upgrade_with_docs() {
    let tmp_env = std::env::temp_dir().join(format!("test_docs_pers_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env);

    let reg = store.module_registry();
    let mut guard = reg.write().unwrap();

    let manifest = PersonalManifest {
        id: "doc-test-tool".to_string(),
        name: "Documentation Test Tool".to_string(),
        short_name: Some("DocTool".to_string()),
        version: "1.0.0".to_string(),
        namespace: "/custom/doc-test".to_string(),
        entrypoint: "/bin/echo".to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec!["chat".to_string()],
        icon: Some("📖".to_string()),
        config_schema: None,
        permissions: Vec::new(),
        group: None,
        source: Some("https://github.com/example/doc-test-tool".to_string()),
        tools: None,
        doc_url: Some("https://example.com/docs/doctool".to_string()),
        doc_content: Some("# Documentation Test Tool\n\nCustom guide content.".to_string()),
    };

    guard.register_personal_module(manifest).unwrap();
    let desc = guard.get_module("doc-test-tool").expect("registered");
    assert_eq!(
        desc.doc_url.as_deref(),
        Some("https://example.com/docs/doctool")
    );
    assert_eq!(
        desc.doc_content.as_deref(),
        Some("# Documentation Test Tool\n\nCustom guide content.")
    );

    // Upgrade preserving existing docs
    let upgrade_manifest = PersonalManifest {
        id: "doc-test-tool".to_string(),
        name: "Documentation Test Tool V2".to_string(),
        short_name: Some("DocTool".to_string()),
        version: "2.0.0".to_string(),
        namespace: "/custom/doc-test".to_string(),
        entrypoint: "/bin/echo".to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec!["chat".to_string()],
        icon: Some("📖".to_string()),
        config_schema: None,
        permissions: Vec::new(),
        group: None,
        source: Some("https://github.com/example/doc-test-tool".to_string()),
        tools: None,
        doc_url: None,     // Omitted to test preservation
        doc_content: None, // Omitted to test preservation
    };

    let upgraded = guard.upgrade_personal_module(upgrade_manifest).unwrap();
    assert_eq!(upgraded.version.as_deref(), Some("2.0.0"));
    assert_eq!(
        upgraded.doc_url.as_deref(),
        Some("https://example.com/docs/doctool"),
        "doc_url must be preserved across upgrade"
    );
    assert_eq!(
        upgraded.doc_content.as_deref(),
        Some("# Documentation Test Tool\n\nCustom guide content."),
        "doc_content must be preserved across upgrade"
    );
}

#[test]
fn test_update_module_config_updates_docs() {
    let tmp_env = std::env::temp_dir().join(format!("test_docs_cfg_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env);

    // Register simple tool
    {
        let reg = store.module_registry();
        let mut guard = reg.write().unwrap();
        let manifest = PersonalManifest {
            id: "cfg-doc-tool".to_string(),
            name: "Config Doc Tool".to_string(),
            short_name: None,
            version: "1.0.0".to_string(),
            namespace: "/custom/cfg-doc".to_string(),
            entrypoint: "/bin/echo".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec![],
            icon: None,
            config_schema: None,
            permissions: Vec::new(),
            group: None,
            source: None,
            tools: None,
            doc_url: None,
            doc_content: None,
        };
        guard.register_personal_module(manifest).unwrap();
    }

    // Update config with doc_url and doc_content
    let payload = serde_json::json!({
        "id": "cfg-doc-tool",
        "config": {
            "doc_url": "https://docs.mytool.io",
            "doc_content": "# Dynamic Config Guide\n\n- Step 1: Configure credentials\n- Step 2: Test endpoints"
        }
    })
    .to_string();

    let (head, body) = request_gateway(
        "POST",
        "/api/v1/dashboard/modules/config",
        payload.as_bytes(),
        &store,
    );
    assert_eq!(
        head.status,
        200,
        "Update config response: {}",
        String::from_utf8_lossy(&body)
    );

    // Verify GET config response includes updated doc metadata
    let (head, body) = request_gateway(
        "GET",
        "/api/v1/dashboard/modules/config?id=cfg-doc-tool",
        &[],
        &store,
    );
    assert_eq!(head.status, 200);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["doc_url"], "https://docs.mytool.io");
    assert!(json["doc_content"]
        .as_str()
        .unwrap()
        .contains("# Dynamic Config Guide"));
}

#[test]
fn test_get_module_doc_html_rendering_and_content() {
    let tmp_env = std::env::temp_dir().join(format!("test_docs_html_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env);

    // GET /modules/omniroute/doc
    let (head, body) = request_gateway("GET", "/modules/omniroute/doc", &[], &store);
    assert_eq!(head.status, 200);
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("<!DOCTYPE html>"));
    assert!(html.contains("OmniRoute LLM Proxy Documentation"));
    assert!(html.contains("<h1>OmniRoute LLM Proxy</h1>"));
    assert!(html.contains("/ominroute/v1/chat/completions"));
    assert!(html.contains("/dashboard"));

    // GET /modules/muse/docs
    let (head, body) = request_gateway("GET", "/modules/muse/docs", &[], &store);
    assert_eq!(head.status, 200);
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Muse AI Provider Documentation"));
    assert!(html.contains("<h1>Muse AI Provider</h1>"));
}

#[test]
fn test_get_module_doc_json_endpoints() {
    let tmp_env = std::env::temp_dir().join(format!("test_docs_json_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env);

    // 1. Query via API route: /api/v1/dashboard/modules/doc?id=jev
    let (head, body) = request_gateway("GET", "/api/v1/dashboard/modules/doc?id=jev", &[], &store);
    assert_eq!(head.status, 200);
    let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(val["ok"], true);
    assert_eq!(val["id"], "jev");
    assert_eq!(val["doc_url"], "/modules/jev/doc");
    assert!(val["doc_content"]
        .as_str()
        .unwrap()
        .contains("Jev Decision Provider"));

    // 2. Query /modules/jev/doc with Accept: application/json
    let (head, body) = request_gateway_with_headers(
        "GET",
        "/modules/jev/doc",
        &[("Accept", "application/json")],
        &[],
        &store,
    );
    assert_eq!(head.status, 200);
    let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(val["ok"], true);
    assert_eq!(val["id"], "jev");
}

#[test]
fn test_get_module_doc_redirects_for_external_url_without_content_block() {
    let tmp_env =
        std::env::temp_dir().join(format!("test_docs_redir_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env);

    // Register module with external doc_url and NO inline doc_content
    {
        let reg = store.module_registry();
        let mut guard = reg.write().unwrap();
        let manifest = PersonalManifest {
            id: "ext-doc-tool".to_string(),
            name: "External Doc Tool".to_string(),
            short_name: None,
            version: "1.0.0".to_string(),
            namespace: "/custom/ext-doc".to_string(),
            entrypoint: "/bin/echo".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec![],
            icon: None,
            config_schema: None,
            permissions: Vec::new(),
            group: None,
            source: None,
            tools: None,
            doc_url: Some("https://external-docs.example.com/readme".to_string()),
            doc_content: None,
        };
        guard.register_personal_module(manifest).unwrap();
    }

    let (head, _) = request_gateway("GET", "/modules/ext-doc-tool/doc", &[], &store);
    assert_eq!(head.status, 302, "Expected 302 redirect to external URL");
    let loc = head
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("location"))
        .map(|(_, v)| v.as_str());
    assert_eq!(loc, Some("https://external-docs.example.com/readme"));
}

#[test]
fn test_get_module_doc_not_found() {
    let tmp_env = std::env::temp_dir().join(format!("test_docs_404_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env);

    let (head, body) = request_gateway("GET", "/modules/nonexistent-xyz/doc", &[], &store);
    assert_eq!(head.status, 404);
    assert!(String::from_utf8_lossy(&body).contains("not found"));
}

#[test]
fn test_mcp_core_tools_expose_module_docs() {
    let tmp_env = std::env::temp_dir().join(format!("test_docs_mcp_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env);

    // 1. Call gateway_list_modules
    let res = toolcase_gateway::features::mcp::tools::call_tool(
        "gateway_list_modules",
        &serde_json::json!({}),
        &store,
    )
    .expect("gateway_list_modules succeeds");

    let text = res["content"][0]["text"].as_str().expect("text content");
    assert!(text.contains("doc_url"));
    assert!(text.contains("/modules/omniroute/doc"));
    assert!(text.contains("doc_content"));

    // 2. Call gateway_get_module_info for omniroute
    let res = toolcase_gateway::features::mcp::tools::call_tool(
        "gateway_get_module_info",
        &serde_json::json!({ "id": "omniroute" }),
        &store,
    )
    .expect("gateway_get_module_info succeeds");

    let text = res["content"][0]["text"].as_str().expect("text content");
    assert!(text.contains("/modules/omniroute/doc"));
    assert!(text.contains("OmniRoute LLM Proxy"));
}

#[test]
fn test_apply_module_configs_blank_documentation_resets_defaults() {
    let tmp_env =
        std::env::temp_dir().join(format!("test_docs_blank_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env);
    let reg = store.module_registry();

    let manifest = PersonalManifest {
        id: "blank-test-mod".to_string(),
        name: "Blank Test Module".to_string(),
        short_name: None,
        version: "1.0.0".to_string(),
        namespace: "/custom/blank-test".to_string(),
        entrypoint: "/bin/echo".to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec![],
        icon: None,
        config_schema: None,
        permissions: Vec::new(),
        group: None,
        source: None,
        tools: None,
        doc_url: Some("https://example.com/custom-doc".to_string()),
        doc_content: Some("# Initial Custom Doc".to_string()),
    };

    {
        let mut guard = reg.write().unwrap();
        guard.register_personal_module(manifest).unwrap();
        let desc = guard.get_module("blank-test-mod").unwrap();
        assert_eq!(
            desc.doc_url.as_deref(),
            Some("https://example.com/custom-doc")
        );
        assert_eq!(desc.doc_content.as_deref(), Some("# Initial Custom Doc"));
    }

    // Apply configuration with blank doc_url and doc_content
    {
        let mut guard = reg.write().unwrap();
        let mut updates = std::collections::HashMap::new();
        let mut cfg = serde_json::Map::new();
        cfg.insert(
            "doc_url".to_string(),
            serde_json::Value::String("   ".to_string()),
        );
        cfg.insert(
            "doc_content".to_string(),
            serde_json::Value::String("".to_string()),
        );
        updates.insert("blank-test-mod".to_string(), cfg);

        guard.apply_module_configs(&updates);

        let desc = guard.get_module("blank-test-mod").unwrap();
        assert_eq!(
            desc.doc_url.as_deref(),
            Some("/modules/blank-test-mod/doc"),
            "Blank doc_url must reset to canonical path"
        );
        assert_eq!(
            desc.doc_content, None,
            "Blank doc_content must clear to None"
        );
        let manifests = guard.export_personal_manifests();
        let manifest_entry = manifests.iter().find(|m| m.id == "blank-test-mod").unwrap();
        assert_eq!(
            manifest_entry.doc_url.as_deref(),
            Some("/modules/blank-test-mod/doc")
        );
        assert_eq!(manifest_entry.doc_content, None);
    }
}

#[test]
fn test_sync_module_config_from_disk_projects_documentation_to_metadata() {
    let tmp_env =
        std::env::temp_dir().join(format!("test_docs_disk_proj_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env);
    let reg = store.module_registry();

    let manifest = PersonalManifest {
        id: "disk-proj-mod".to_string(),
        name: "Disk Proj Module".to_string(),
        short_name: None,
        version: "1.0.0".to_string(),
        namespace: "/custom/disk-proj".to_string(),
        entrypoint: "/bin/echo".to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec![],
        icon: None,
        config_schema: None,
        permissions: Vec::new(),
        group: None,
        source: None,
        tools: None,
        doc_url: None,
        doc_content: None,
    };

    {
        let mut guard = reg.write().unwrap();
        guard.register_personal_module(manifest).unwrap();
    }

    // Update config via update_module_config which writes to disk and projects
    {
        let mut guard = reg.write().unwrap();
        let mut updates = serde_json::Map::new();
        updates.insert(
            "doc_url".to_string(),
            serde_json::Value::String("https://wiki.internal/disk-proj".to_string()),
        );
        updates.insert(
            "doc_content".to_string(),
            serde_json::Value::String("# Disk Configured Content".to_string()),
        );
        guard
            .update_module_config("disk-proj-mod", updates)
            .unwrap();

        // Verify get_module, export_personal_manifests, and get_effective_module_config all match
        let desc = guard.get_module("disk-proj-mod").unwrap();
        assert_eq!(
            desc.doc_url.as_deref(),
            Some("https://wiki.internal/disk-proj")
        );
        assert_eq!(
            desc.doc_content.as_deref(),
            Some("# Disk Configured Content")
        );

        let manifests = guard.export_personal_manifests();
        let m = manifests.iter().find(|m| m.id == "disk-proj-mod").unwrap();
        assert_eq!(
            m.doc_url.as_deref(),
            Some("https://wiki.internal/disk-proj")
        );
        assert_eq!(m.doc_content.as_deref(), Some("# Disk Configured Content"));

        let eff = guard.get_effective_module_config("disk-proj-mod");
        assert_eq!(
            eff.get("doc_url").and_then(|v| v.as_str()),
            Some("https://wiki.internal/disk-proj")
        );
        assert_eq!(
            eff.get("doc_content").and_then(|v| v.as_str()),
            Some("# Disk Configured Content")
        );
    }
}

#[test]
fn test_universal_documentation_auto_population_for_all_modules() {
    let tmp_env = std::env::temp_dir().join(format!("test_docs_univ_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(test_config(), tmp_env);
    let reg = store.module_registry();

    // 1. Register arbitrary personal module with zero doc metadata
    let custom_manifest = PersonalManifest {
        id: "zero-doc-mod".to_string(),
        name: "Zero Doc Module".to_string(),
        short_name: None,
        version: "1.0.0".to_string(),
        namespace: "/custom/zero-doc".to_string(),
        entrypoint: "/bin/echo".to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec!["chat".to_string()],
        icon: None,
        config_schema: None,
        permissions: Vec::new(),
        group: None,
        source: None,
        tools: None,
        doc_url: None,
        doc_content: None,
    };

    {
        let mut guard = reg.write().unwrap();
        guard.register_personal_module(custom_manifest).unwrap();
        let desc = guard.get_module("zero-doc-mod").unwrap();
        assert_eq!(
            desc.doc_url.as_deref(),
            Some("/modules/zero-doc-mod/doc"),
            "Default doc_url should point to canonical doc path"
        );
        let content = desc
            .doc_content
            .as_deref()
            .expect("synthesized doc_content");
        assert!(content.contains("# Zero Doc Module"));
        assert!(content.contains("/custom/zero-doc"));
        assert!(content.contains("chat"));
    }

    // 2. Register catalog module (facebook-suite) without explicit doc metadata in manifest
    let fb_manifest = PersonalManifest {
        id: "facebook-suite".to_string(),
        name: "Meta Business Suite".to_string(),
        short_name: None,
        version: "1.0.0".to_string(),
        namespace: "/custom/facebook".to_string(),
        entrypoint: "/bin/echo".to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec!["studio".to_string()],
        icon: None,
        config_schema: None,
        permissions: Vec::new(),
        group: None,
        source: None,
        tools: None,
        doc_url: None,
        doc_content: None,
    };

    {
        let mut guard = reg.write().unwrap();
        guard.register_personal_module(fb_manifest).unwrap();
        let desc = guard.get_module("facebook-suite").unwrap();
        let url = desc.doc_url.as_deref().expect("resolved catalog doc_url");
        assert!(url.contains("facebook-suite"));
        assert!(url.starts_with("http"));
        let content = desc.doc_content.as_deref().expect("resolved doc_content");
        assert!(!content.trim().is_empty());
    }

    // 3. Verify GET /api/v1/dashboard/overview returns non-empty doc_url and doc_content for ALL modules
    let (head, body) = request_gateway("GET", "/api/v1/dashboard/overview", &[], &store);
    assert_eq!(head.status, 200);
    let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let modules = val["modules"].as_array().expect("modules array");
    assert!(!modules.is_empty());
    for m in modules {
        let m_id = m["id"].as_str().unwrap_or("unknown");
        let doc_url = m.get("doc_url").and_then(|v| v.as_str());
        assert!(
            doc_url.is_some() && !doc_url.unwrap().is_empty(),
            "Module '{m_id}' must have a valid doc_url"
        );
        let doc_content = m.get("doc_content").and_then(|v| v.as_str());
        assert!(
            doc_content.is_some() && !doc_content.unwrap().is_empty(),
            "Module '{m_id}' must have a valid doc_content"
        );
    }
}
