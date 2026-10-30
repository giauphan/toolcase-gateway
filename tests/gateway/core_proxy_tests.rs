//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;

// ---------- Core proxy, Muse business proxy, and model catalog route tests ----------

#[test]
fn test_museai_business_builds_configured_request() {
    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        default_model: "gpt-5.6-sol".into(),
        museai_base_url: "http://127.0.0.1:32123".into(),
        museai_cookie: "session=test".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
        jev_api_keys: vec![],
        jev_base_url: "https://jev-ai.org".into(),
        ..Config::default()
    };

    let request = br#"{"target":"session","method":"GET"}"#;
    let (url, method, body) =
        toolcase_gateway::museai::business::build_museai_request(request, &config).unwrap();
    assert_eq!(url, "http://127.0.0.1:32123/api/session");
    assert_eq!(method, "GET");
    assert!(body.is_empty());
}

#[test]
fn test_museai_business_rejects_unsafe_target() {
    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        default_model: "gpt-5.6-sol".into(),
        museai_base_url: "https://muse.ai".into(),
        museai_cookie: "".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
        jev_api_keys: vec![],
        jev_base_url: "https://jev-ai.org".into(),
        ..Config::default()
    };

    let request = br#"{"target":"../session","method":"GET"}"#;
    assert!(toolcase_gateway::museai::business::build_museai_request(request, &config).is_err());
}

#[test]
fn test_museai_v1_mock_proxy_flow() {
    let mock_muse = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mock_muse_port = mock_muse.local_addr().unwrap().port();

    let muse_handle = thread::spawn(move || {
        let (mut client, _) = mock_muse.accept().unwrap();
        let req = toolcase_gateway::http::read_request(&mut client).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/api/session");
        assert_eq!(
            toolcase_gateway::http::header_value(&req.headers, "cookie"),
            Some("hatch_sess=secret_mock_sess")
        );
        let resp_body = r#"{"ok":true,"access_token":"token_test_123"}"#;
        write!(
            client,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            resp_body.len(),
            resp_body
        )
        .unwrap();
        client.flush().unwrap();
    });

    let mock_gw = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let gw_port = mock_gw.local_addr().unwrap().port();

    let gw_handle = thread::spawn(move || {
        let config = Config {
            target_host: "127.0.0.1".into(),
            target_port: 8080,
            fallbacks: vec![],
            io_timeout: None,
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 1000,
            default_model: "gpt-5.6-sol".into(),
            museai_base_url: format!("http://127.0.0.1:{mock_muse_port}"),
            museai_cookie: "hatch_sess=secret_mock_sess".into(),
            museai_ws_url: "".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 86400,
            jev_api_keys: vec![],
            jev_base_url: "https://jev-ai.org".into(),
            ..Config::default()
        };

        let (mut client, _) = mock_gw.accept().unwrap();
        let req_in = toolcase_gateway::http::read_request(&mut client).unwrap();
        toolcase_gateway::museai::handle_museai_v1(&mut client, &req_in.body, &config).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", gw_port)).unwrap();
    let gw_req = r#"{"target":"session","method":"GET"}"#;
    write!(
        client,
        "POST /muse-ai/v1 HTTP/1.1\r\nHost: 127.0.0.1:{gw_port}\r\nContent-Length: {}\r\n\r\n{}",
        gw_req.len(),
        gw_req
    )
    .unwrap();
    client.flush().unwrap();

    let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    let mut body = head.buffered_body;
    let length = toolcase_gateway::http::header_value(&head.headers, "content-length")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    while body.len() < length {
        toolcase_gateway::http::read_more(&mut client, &mut body).unwrap();
    }
    let response: serde_json::Value = serde_json::from_slice(&body[..length]).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["access_token"], "token_test_123");

    muse_handle.join().unwrap();
    gw_handle.join().unwrap();
}

#[test]
fn test_muse_chat_completions_handler_invalid_payload() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store =
        toolcase_gateway::config::ConfigStore::new(test_config(), std::path::PathBuf::from(".env"));
    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        let _ = toolcase_gateway::routes::route_request(client, &store);
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let body = "invalid-not-json";
    write!(
        client,
        "POST /muse-ai/v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
    client.flush().unwrap();

    let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 400);
    handle.join().unwrap();
}

fn routed_status(method: &str, path: &str) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store =
        toolcase_gateway::config::ConfigStore::new(test_config(), std::path::PathBuf::from(".env"));
    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();
    let status = read_response_head(&mut client).unwrap().status;
    handle.join().unwrap();
    status
}

#[test]
fn reserved_feature_routes_never_fall_through_to_omniroute() {
    assert_eq!(routed_status("GET", "/muse-ai/unknown"), 404);
    assert_eq!(routed_status("POST", "/muse-config"), 405);
    assert_eq!(routed_status("POST", "/video-template"), 405);
    assert_eq!(routed_status("POST", "/dashboard"), 405);
    assert_eq!(routed_status("GET", "/api/v1/dashboard/muse-config"), 405);
    assert_eq!(routed_status("GET", "/muse-ai/v1/chat/completions"), 405);
    assert_eq!(routed_status("PUT", "/muse-ai/v1/chat/completions"), 405);
    assert_eq!(routed_status("DELETE", "/muse-ai/v1/chat/completions"), 405);
    assert_eq!(routed_status("GET", "/muse-ai/chat/completions"), 405);
    assert_eq!(
        routed_status("OPTIONS", "/muse-ai/v1/chat/completions"),
        200
    );
}

#[test]
fn model_catalog_routes_have_exact_platform_owners() {
    for path in ["/muse-ai/models", "/muse-ai/v1/models"] {
        assert_eq!(
            toolcase_gateway::routes::catalog_route(path),
            Some(toolcase_gateway::routes::CatalogPlatform::Muse)
        );
    }
    for path in [
        "/models",
        "/v1/models",
        "/models/extra",
        "/muse-ai/models/extra",
        "/other/models",
        "/other/v1/models",
        "/muse-ai/other/models",
    ] {
        assert_eq!(toolcase_gateway::routes::catalog_route(path), None);
    }
}

fn assert_main_catalog_proxies_to_upstream(path: &str) {
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    upstream_listener.set_nonblocking(true).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();
    let expected_path = if let Some(stripped) = path.strip_prefix("/ominroute") {
        stripped.to_string()
    } else {
        path.to_string()
    };

    let upstream = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let (mut socket, _) = loop {
            match upstream_listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        panic!("gateway did not proxy {expected_path} to upstream");
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        };
        let request = toolcase_gateway::http::read_request(&mut socket).unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, expected_path);
        let body = br#"{"object":"list","data":[{"id":"real-upstream-model"}]}"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        socket.write_all(body).unwrap();
    });

    let gateway_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let gateway_port = gateway_listener.local_addr().unwrap().port();
    let mut config = test_config();
    config.target_port = upstream_port;
    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let gateway = thread::spawn(move || {
        let (client, _) = gateway_listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", gateway_port)).unwrap();
    write!(
        client,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();

    let response = read_response_head(&mut client).unwrap();
    assert_eq!(response.status, 200);
    let body = if is_chunked(&response.headers) {
        toolcase_gateway::http::read_chunked_body(&mut client, response.buffered_body).unwrap()
    } else {
        let mut body = response.buffered_body;
        let length = header_value(&response.headers, "content-length")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        while body.len() < length {
            read_more(&mut client, &mut body).unwrap();
        }
        body.truncate(length);
        body
    };
    assert_eq!(
        body,
        br#"{"object":"list","data":[{"id":"real-upstream-model"}]}"#
    );

    gateway.join().unwrap();
    upstream.join().unwrap();
}

#[test]
fn main_model_catalog_routes_proxy_to_upstream() {
    for path in ["/ominroute/models", "/ominroute/v1/models"] {
        assert_main_catalog_proxies_to_upstream(path);
    }
}

fn assert_muse_catalog_stays_local(path: &str) {
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    upstream_listener.set_nonblocking(true).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();
    let gateway_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let gateway_port = gateway_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_port = upstream_port;
    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let gateway = thread::spawn(move || {
        let (client, _) = gateway_listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", gateway_port)).unwrap();
    write!(
        client,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();

    let response = read_response_head(&mut client).unwrap();
    assert_eq!(response.status, 200);
    let mut body = response.buffered_body;
    let length = header_value(&response.headers, "content-length")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    while body.len() < length {
        read_more(&mut client, &mut body).unwrap();
    }
    body.truncate(length);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["object"], "list");
    assert_eq!(json["data"][0]["id"], "muse");

    gateway.join().unwrap();
    assert!(matches!(
        upstream_listener.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
}

#[test]
fn muse_model_catalog_routes_stay_local() {
    for path in ["/muse-ai/models", "/muse-ai/v1/models"] {
        assert_muse_catalog_stays_local(path);
    }
}
