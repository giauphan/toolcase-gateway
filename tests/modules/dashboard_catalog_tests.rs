use std::fs;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::thread;
use toolcase_gateway::config::{Config, ConfigStore};
use toolcase_gateway::gateway::installer::cleanup_module_files;
use toolcase_gateway::http::read_response_head;
use toolcase_gateway::routes::route_request;

#[test]
fn test_dashboard_module_api_lifecycle() {
    let temp_env = std::env::temp_dir().join(format!("test_store_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(Config::default(), temp_env.clone());

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store_clone = store.clone();
    let handle = thread::spawn(move || {
        for _ in 0..5 {
            let (client, _) = listener.accept().unwrap();
            let _ = route_request(client, &store_clone);
        }
    });

    // 1. GET /api/v1/dashboard/overview (initially includes personal & built-in modules)
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "GET /api/v1/dashboard/overview HTTP/1.1\r\nHost: localhost\r\n\r\n"
    )
    .unwrap();
    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);

    // 2. POST /api/v1/dashboard/modules/install with raw manifest
    let install_body = serde_json::json!({
        "id": "http-test-mod",
        "name": "HTTP Test Module",
        "version": "1.0.0",
        "namespace": "/custom/http-test",
        "entrypoint": "/bin/echo",
        "protocol": "stdio-stream",
        "capabilities": ["models"]
    })
    .to_string();

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "POST /api/v1/dashboard/modules/install HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
        install_body.len(),
        install_body
    )
    .unwrap();
    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);

    // 3. POST /api/v1/dashboard/modules/install with omitted optional fields (auto-infers id and entrypoint)
    let simple_install_body = serde_json::json!({
        "namespace": "/custom/auto-derived-tool"
    })
    .to_string();

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "POST /api/v1/dashboard/modules/install HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
        simple_install_body.len(),
        simple_install_body
    )
    .unwrap();
    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);

    // 4. POST /api/v1/dashboard/modules/state
    let toggle_body = serde_json::json!({"id": "http-test-mod", "enabled": false}).to_string();
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "POST /api/v1/dashboard/modules/state HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
        toggle_body.len(),
        toggle_body
    )
    .unwrap();
    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);

    // 5. POST /api/v1/dashboard/modules/remove
    let remove_body = serde_json::json!({"id": "http-test-mod"}).to_string();
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "POST /api/v1/dashboard/modules/remove HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
        remove_body.len(),
        remove_body
    )
    .unwrap();
    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);

    handle.join().unwrap();
    let _ = fs::remove_file(temp_env);
}

#[test]
fn test_system_catalog_and_one_click_installation() {
    let catalog = toolcase_gateway::gateway::modules::get_system_module_catalog();
    assert!(catalog.iter().any(|c| c.id == "cohere-provider"));
    assert!(catalog.iter().any(|c| c.id == "claude-adapter"));
    assert!(catalog.iter().any(|c| c.id == "groq-fast"));

    let state_dir = toolcase_gateway::gateway::installer::get_state_dir();
    cleanup_module_files("cohere-provider", &state_dir);
    cleanup_module_files("another-cohere", &state_dir);

    let temp_env = std::env::temp_dir().join(format!("cat_test_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(Config::default(), temp_env.clone());

    // 1. Install cohere-provider from catalog by ID
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store_clone = store.clone();

    let handle = thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        let payload = br#"{"id":"cohere-provider"}"#;
        toolcase_gateway::features::dashboard::handle_install_personal_module(
            &mut client,
            payload,
            &store_clone,
        )
        .unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let head = read_response_head(&mut client).unwrap();
    if head.status != 200 {
        panic!(
            "Failed: status={}, body={:?}",
            head.status,
            String::from_utf8_lossy(&head.buffered_body)
        );
    }
    assert_eq!(head.status, 200);
    handle.join().unwrap();

    // 2. Verify registered in module registry
    let reg = store.module_registry();
    let guard = reg.read().unwrap();
    let cohere_mod = guard
        .get_module("cohere-provider")
        .expect("cohere-provider registered");
    assert_eq!(cohere_mod.name, "Cohere Provider Adapter");
    assert_eq!(cohere_mod.namespace, "/custom/cohere");
    assert_eq!(
        cohere_mod.status,
        toolcase_gateway::gateway::modules::ModuleStatus::Enabled
    );
    drop(guard);

    // 2.5 Dispatch live request to the installed cohere module
    let listener_exec = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port_exec = listener_exec.local_addr().unwrap().port();
    let store_clone_exec = store.clone();
    let handle_exec = thread::spawn(move || {
        let (client, _) = listener_exec.accept().unwrap();
        let _ = route_request(client, &store_clone_exec);
    });

    let mut client_exec = TcpStream::connect(("127.0.0.1", port_exec)).unwrap();
    let body_req = br#"{"messages":[{"role":"user","content":"Hello Cohere"}]}"#;
    client_exec
        .write_all(
            format!(
                "POST /custom/cohere/v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n",
                body_req.len()
            )
            .as_bytes(),
        )
        .unwrap();
    client_exec.write_all(body_req).unwrap();
    let head_exec = read_response_head(&mut client_exec).unwrap();
    if head_exec.status != 200 {
        panic!(
            "Cohere dispatch failed: status={}, body={:?}",
            head_exec.status,
            String::from_utf8_lossy(&head_exec.buffered_body)
        );
    }
    assert_eq!(head_exec.status, 200);
    handle_exec.join().unwrap();

    // 3. Namespace collision returns 400 Bad Request
    let listener2 = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port2 = listener2.local_addr().unwrap().port();
    let store_clone2 = store.clone();

    let handle2 = thread::spawn(move || {
        let (mut client, _) = listener2.accept().unwrap();
        let payload = br#"{"id":"another-cohere","namespace":"/custom/cohere"}"#;
        let _ = toolcase_gateway::features::dashboard::handle_install_personal_module(
            &mut client,
            payload,
            &store_clone2,
        );
    });

    let mut client2 = TcpStream::connect(("127.0.0.1", port2)).unwrap();
    let head2 = read_response_head(&mut client2).unwrap();
    assert_eq!(head2.status, 400);
    handle2.join().unwrap();

    let _ = fs::remove_file(temp_env);
    let _ = fs::remove_file(store.config_file);
    cleanup_module_files("cohere-provider", &state_dir);
    cleanup_module_files("another-cohere", &state_dir);
}
