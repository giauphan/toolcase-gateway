//! Integration tests for module groups and sub-provider routing.

use std::fs;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::thread;
use toolcase_gateway::config::{Config, ConfigStore};
use toolcase_gateway::gateway::modules::{
    ModuleGroupManifest, ModuleStatus, PersonalManifest, SubProviderManifest,
};
use toolcase_gateway::http::read_response_head;

#[test]
fn test_personal_module_group_routing_and_sub_providers() {
    let temp_dir = std::env::temp_dir().join(format!("suite_test_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_dir).unwrap();
    let script_path = temp_dir.join("run.sh");

    let script = r#"#!/bin/sh
python3 -c '
import os, sys, json
for line in sys.stdin:
    if not line.strip(): continue
    req = json.loads(line)
    sub_id = os.environ.get("MODULE_SUB_PROVIDER_ID", "none")
    mod_id = os.environ.get("MODULE_ID", "none")
    out = {
        "status": 200,
        "headers": {"content-type": "application/json"},
        "body": json.dumps({"module": mod_id, "sub_provider": sub_id, "path": req.get("path")})
    }
    sys.stdout.write(json.dumps(out) + "\n")
    sys.stdout.flush()
'
"#;
    fs::write(&script_path, script).unwrap();
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o700);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    let manifest = PersonalManifest {
        id: "suite-test".to_string(),
        name: "Suite Provider Group".to_string(),
        version: "1.0.0".to_string(),
        namespace: "/custom/suite-test".to_string(),
        entrypoint: script_path.to_string_lossy().to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec!["chat".to_string(), "vision".to_string()],
        config_schema: None,
        permissions: Vec::new(),
        group: Some(ModuleGroupManifest {
            enabled: true,
            providers: vec![
                SubProviderManifest {
                    id: "p1-chat".to_string(),
                    name: "Chat Sub Provider".to_string(),
                    subpath: "/chat".to_string(),
                    capabilities: vec!["chat".to_string()],
                },
                SubProviderManifest {
                    id: "p2-vision".to_string(),
                    name: "Vision Sub Provider".to_string(),
                    subpath: "/vision".to_string(),
                    capabilities: vec!["vision".to_string()],
                },
            ],
        }),
    };

    let temp_env = std::env::temp_dir().join(format!("suite_env_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(Config::default(), temp_env.clone());
    store
        .module_registry()
        .write()
        .unwrap()
        .register_personal_module(manifest)
        .unwrap();

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store_clone = store.clone();

    let server_handle = thread::spawn(move || {
        for _ in 0..4 {
            if let Ok((client, _)) = listener.accept() {
                let _ = toolcase_gateway::routes::route_request(client, &store_clone);
            }
        }
    });

    // 1. Request to p1-chat
    let mut c1 = TcpStream::connect(("127.0.0.1", port)).unwrap();
    c1.write_all(b"POST /custom/suite-test/chat/completions HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 2\r\n\r\n{}").unwrap();
    let h1 = read_response_head(&mut c1).unwrap();
    assert_eq!(h1.status, 200);
    let b1: serde_json::Value = serde_json::from_slice(&h1.buffered_body).unwrap();
    assert_eq!(b1["sub_provider"], "p1-chat");

    // 2. Request to p2-vision
    let mut c2 = TcpStream::connect(("127.0.0.1", port)).unwrap();
    c2.write_all(b"POST /custom/suite-test/vision/analyze HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 2\r\n\r\n{}").unwrap();
    let h2 = read_response_head(&mut c2).unwrap();
    assert_eq!(h2.status, 200);
    let b2: serde_json::Value = serde_json::from_slice(&h2.buffered_body).unwrap();
    assert_eq!(b2["sub_provider"], "p2-vision");

    // 3. Disable p1-chat -> 503 on p1, 200 on p2
    store
        .module_registry()
        .write()
        .unwrap()
        .set_sub_provider_status("suite-test", "p1-chat", ModuleStatus::Disabled)
        .unwrap();

    let mut c3 = TcpStream::connect(("127.0.0.1", port)).unwrap();
    c3.write_all(b"POST /custom/suite-test/chat/completions HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 2\r\n\r\n{}").unwrap();
    let h3 = read_response_head(&mut c3).unwrap();
    assert_eq!(h3.status, 503);

    // 4. Disable whole group -> 503 on p2
    store
        .module_registry()
        .write()
        .unwrap()
        .set_module_status("suite-test", ModuleStatus::Disabled)
        .unwrap();

    let mut c4 = TcpStream::connect(("127.0.0.1", port)).unwrap();
    c4.write_all(b"POST /custom/suite-test/vision/analyze HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 2\r\n\r\n{}").unwrap();
    let h4 = read_response_head(&mut c4).unwrap();
    assert_eq!(h4.status, 503);

    server_handle.join().unwrap();
    let _ = fs::remove_file(temp_env);
    let _ = fs::remove_file(store.config_file);
    let _ = fs::remove_dir_all(temp_dir);
}
