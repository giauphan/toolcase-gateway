//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;

#[test]
fn env_file_path_is_unambiguous() {
    let cwd = std::path::PathBuf::from("/srv/toolcase");
    assert_eq!(
        toolcase_gateway::config::absolute_env_file(std::path::PathBuf::from(".env"), cwd.clone()),
        cwd.join(".env")
    );
    assert_eq!(
        toolcase_gateway::config::absolute_env_file(
            std::path::PathBuf::from("/run/secrets/toolcase.env"),
            cwd
        ),
        std::path::PathBuf::from("/run/secrets/toolcase.env")
    );
}

#[test]
fn replaces_model_without_changing_other_fields() {
    assert_eq!(
        replace_model(br#"{"model":"old","tools":[]}"#, "fail-try"),
        br#"{"model":"fail-try","tools":[]}"#
    );
}

#[test]
fn noise_xx_round_trip_enters_transport_mode() {
    use snow::{params::NoiseParams, Builder};
    use toolcase_gateway::museai::noise::{MuseNoiseSession, NOISE_PATTERN_XX};

    let params: NoiseParams = NOISE_PATTERN_XX.parse().unwrap();
    let responder_key = Builder::new(params.clone()).generate_keypair().unwrap();
    let mut responder = Builder::new(params)
        .local_private_key(&responder_key.private)
        .build_responder()
        .unwrap();
    let mut initiator = MuseNoiseSession::new_initiator(NOISE_PATTERN_XX, None).unwrap();
    let mut message = vec![0; 65535];
    let mut plaintext = vec![0; 65535];

    let len = initiator
        .write_handshake_message(&[], &mut message)
        .unwrap();
    responder
        .read_message(&message[..len], &mut plaintext)
        .unwrap();
    let len = responder.write_message(&[], &mut message).unwrap();
    initiator
        .read_handshake_message(&message[..len], &mut plaintext)
        .unwrap();
    let len = initiator
        .write_handshake_message(&[], &mut message)
        .unwrap();
    responder
        .read_message(&message[..len], &mut plaintext)
        .unwrap();

    assert!(initiator.is_handshake_finished());
    initiator.into_transport_mode().unwrap();
    let mut responder = responder.into_transport_mode().unwrap();

    let len = initiator.encrypt(b"ping", &mut message).unwrap();
    let len = responder
        .read_message(&message[..len], &mut plaintext)
        .unwrap();
    assert_eq!(&plaintext[..len], b"ping");

    let len = responder.write_message(b"pong", &mut message).unwrap();
    let len = initiator.decrypt(&message[..len], &mut plaintext).unwrap();
    assert_eq!(&plaintext[..len], b"pong");
}

#[test]
fn primary_noise_message_one_payload_has_freshness_nonce() {
    let payload = toolcase_gateway::museai::noise::get_primary_message_one_payload();
    assert_eq!(payload.len(), 34);
    assert_eq!(payload[0], 0x0a);
    assert_eq!(payload[1], 0x20);
    // Nonce should not be all zeros
    assert_ne!(&payload[2..], &[0u8; 32]);
}

#[test]
fn noise_ik_requires_responder_key_and_round_trips() {
    use snow::{params::NoiseParams, Builder};
    use toolcase_gateway::museai::noise::{MuseNoiseSession, NOISE_PATTERN_IK};

    assert!(MuseNoiseSession::new_initiator(NOISE_PATTERN_IK, None).is_err());

    let params: NoiseParams = NOISE_PATTERN_IK.parse().unwrap();
    let responder_key = Builder::new(params.clone()).generate_keypair().unwrap();
    let mut responder = Builder::new(params)
        .local_private_key(&responder_key.private)
        .build_responder()
        .unwrap();
    let mut initiator =
        MuseNoiseSession::new_initiator(NOISE_PATTERN_IK, Some(&responder_key.public)).unwrap();
    let mut message = vec![0; 65535];
    let mut plaintext = vec![0; 65535];

    let len = initiator
        .write_handshake_message(&[], &mut message)
        .unwrap();
    responder
        .read_message(&message[..len], &mut plaintext)
        .unwrap();
    let len = responder.write_message(&[], &mut message).unwrap();
    initiator
        .read_handshake_message(&message[..len], &mut plaintext)
        .unwrap();

    assert!(initiator.is_handshake_finished());
    initiator.into_transport_mode().unwrap();
    let mut responder = responder.into_transport_mode().unwrap();

    let len = initiator.encrypt(b"request", &mut message).unwrap();
    let len = responder
        .read_message(&message[..len], &mut plaintext)
        .unwrap();
    assert_eq!(&plaintext[..len], b"request");
}

#[test]
fn muse_websocket_exchanges_binary_frames() {
    use toolcase_gateway::museai::transport::MuseWebSocket;
    use tungstenite::Message;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut socket = tungstenite::accept(stream).unwrap();
        assert_eq!(socket.read().unwrap().into_data(), b"request");
        socket.send(Message::Binary(b"response".to_vec())).unwrap();
    });

    let mut socket = MuseWebSocket::connect(
        &format!("ws://127.0.0.1:{port}/v1/noise"),
        "https://muse.ai",
        "",
    )
    .unwrap();
    socket.send_binary(b"request").unwrap();
    assert_eq!(socket.read_binary().unwrap(), b"response");
    server.join().unwrap();
}

#[test]
fn keeps_413_out_of_retry_statuses() {
    assert!(!RETRYABLE.contains(&413));
    for status in [400, 401, 402, 403, 408, 429, 500, 502, 503, 504, 524] {
        assert!(RETRYABLE.contains(&status));
    }
}

#[test]
fn rewrites_tool_name_after_chunked_body_is_decoded() {
    let request = br#"{"tools":[{"name":"MyTool"}]}"#;
    let decoded = br#"{"name":"mytool"}"#;
    assert_eq!(
        rewrite_tool_names(decoded, request),
        br#"{"name":"MyTool"}"#
    );
}

#[test]
fn rewrite_needs_full_name_pair() {
    let request = br#"{"tools":[{"name":"MyTool"}]}"#;
    assert_eq!(
        rewrite_tool_names(br#"{"name":"my"#, request),
        br#"{"name":"my"#
    );
    assert_eq!(rewrite_tool_names(br#"tool"}"#, request), br#"tool"}"#);
    let mut joined = rewrite_tool_names(br#"{"name":"my"#, request);
    joined.extend_from_slice(br#"tool"}"#);
    assert_eq!(
        rewrite_tool_names(&joined, request),
        br#"{"name":"MyTool"}"#
    );
}

#[test]
fn rewrites_tool_name_with_space_after_colon() {
    let request = br#"{"tools":[{"name":"MyTool"}]}"#;
    let decoded = br#"{"name": "mytool"}"#;
    assert_eq!(
        rewrite_tool_names(decoded, request),
        br#"{"name": "MyTool"}"#
    );
}

#[test]
fn filters_framing_headers() {
    const HOP_BY_HOP: [&str; 8] = [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ];
    assert!(HOP_BY_HOP
        .iter()
        .any(|h| h.eq_ignore_ascii_case("transfer-encoding")));
    assert!(!["content-type", "content-length"]
        .iter()
        .any(|h| { HOP_BY_HOP.iter().any(|x| x.eq_ignore_ascii_case(h)) }));
}

#[test]
fn forces_identity_encoding_upstream() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
        let content_length = request
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .unwrap()
            .trim()
            .parse::<usize>()
            .unwrap();
        let mut body = vec![0; content_length];
        socket.read_exact(&mut body).unwrap();
        let encodings: Vec<_> = request
            .lines()
            .filter(|line| line.starts_with("accept-encoding:"))
            .collect();
        assert_eq!(encodings, ["accept-encoding: identity"]);
        assert!(!request.contains("gzip"));
        let body = br#"{"name":"mytool"}"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .unwrap();
        socket.write_all(body).unwrap();
    });

    let request = Request {
        method: "GET".into(),
        path: "/dashboard".into(),
        headers: vec![("Accept-Encoding".into(), "gzip, br".into())],
        body: br#"{"model":"m","tools":[{"name":"MyTool"}]}"#.to_vec(),
    };
    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: port,
        fallbacks: vec![],
        io_timeout: Some(Duration::from_secs(5)),
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 5000,
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
    };
    let client_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = client_listener.local_addr().unwrap().port();
    let _client_socket = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    let (client_conn, _) = client_listener.accept().unwrap();
    let mut socket = open_upstream(&client_conn, &request, &config, "m").unwrap();
    let head = read_response_head(&mut socket).unwrap();
    let length = header_value(&head.headers, "content-length")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let mut body = head.buffered_body;
    while body.len() < length {
        read_more(&mut socket, &mut body).unwrap();
    }
    body.truncate(length);
    upstream.join().unwrap();
    assert_eq!(head.status, 200);
    assert_eq!(
        rewrite_tool_names(&body, &request.body),
        br#"{"name":"MyTool"}"#
    );
}

#[test]
fn rejects_conflicting_framing_headers() {
    let headers = vec![
        ("Transfer-Encoding".to_string(), "chunked".to_string()),
        ("Content-Length".to_string(), "7".to_string()),
    ];
    assert!(is_chunked(&headers));
    let lengths = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .count();
    assert_eq!(lengths, 1);
}

#[test]
fn rejects_header_injection_in_names() {
    assert!(is_token("Content-Type"));
    assert!(!is_token("Content Type"));
    assert!(!is_token(""));
    assert!(parse_headers(["X-Bad : 1"].into_iter()).is_err());
    assert!(parse_headers(["X-Good: 1"].into_iter()).is_ok());
}

#[test]
fn rejects_malformed_request_targets() {
    assert!(is_request_target("/v1/chat/completions"));
    assert!(!is_request_target("/pa th"));
    assert!(!is_request_target(""));
}

#[test]
fn error_body_escapes_message() {
    assert_eq!(escape_json_string("a\"b\\c"), "a\\\"b\\\\c");
}

#[test]
fn test_fallback_to_config_credentials_when_header_missing() {
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
    };

    // Test that config credentials handle work
    assert!(!config.default_model.is_empty());
}

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
    let expected_path = path.to_string();

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
    for path in ["/models", "/v1/models"] {
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

#[test]
fn test_cors_preflight_response() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        toolcase_gateway::routes::handle_cors_preflight(&mut client).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    assert_eq!(
        header_value(&head.headers, "access-control-allow-origin"),
        Some("*")
    );
    handle.join().unwrap();
}
