//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;

#[test]
fn museai_protobuf_encoding_decoding_round_trips() {
    use toolcase_gateway::museai::protocol::*;

    let req = encode_application_request(
        "POST",
        "/chat/stream",
        &[Header {
            key: "Content-Type".into(),
            value: "application/json".into(),
        }],
        b"{\"test\":true}",
        true,
    );

    let sframe = encode_service_frame_request(42, &req);
    let sreq = encode_service_request(SERVICE_DAEMON, &sframe);

    let tframes = encode_transport_frames(12345, &sreq).unwrap();
    assert_eq!(tframes.len(), 1);

    let tframe_decoded = decode_transport_frame(&tframes[0]).unwrap();
    assert_eq!(tframe_decoded.chunk_id, 12345);
    assert_eq!(tframe_decoded.chunk_index, 0);
    assert_eq!(tframe_decoded.total_chunks, 1);
    assert_eq!(tframe_decoded.payload, sreq);
}

#[test]
fn museai_builds_shared_noise_url_from_direct_vm_url() {
    let mut config = test_config();
    config.museai_ws_url = "wss://vm-123.metaaivm.com/".into();
    config.museai_access_token = "token +/=".into();

    let url = toolcase_gateway::museai::build_museai_ws_url(&config, "request-id").unwrap();
    assert!(url.starts_with("wss://hatch.metaaivm.com/v1/noise?"));
    assert!(url.contains("vm_id=vm-123"));
    assert!(url.contains("auth_token=token%20%2B%2F%3D"));
    assert!(url.contains("app_id=hatch-web"));
    assert!(url.contains("request_id=request-id"));
    assert!(!url.contains("notary_token="));
}

#[test]
fn museai_noise_url_preserves_captured_query_parameters() {
    let mut config = test_config();
    config.museai_ws_url =
        "wss://hatch.metaaivm.com/v1/noise?vm_id=captured&auth_token=captured-token&app_id=hatch-web"
            .into();
    config.museai_access_token.clear();
    config.museai_vm_id.clear();

    let url = toolcase_gateway::museai::build_museai_ws_url(&config, "request-id").unwrap();
    assert!(url.contains("vm_id=captured"));
    assert!(url.contains("auth_token=captured-token"));
    assert!(url.contains("app_id=hatch-web"));
    assert!(url.contains("request_id=request-id"));
    assert_eq!(url.matches("vm_id=").count(), 1);
    assert_eq!(url.matches("auth_token=").count(), 1);
    assert_eq!(url.matches("app_id=").count(), 1);
}

#[test]
fn museai_noise_url_rejects_missing_auth_token() {
    let mut config = test_config();
    config.museai_ws_url = "wss://hatch.metaaivm.com/v1/noise".into();
    config.museai_access_token.clear();

    let error = toolcase_gateway::museai::build_museai_ws_url(&config, "request-id").unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn museai_noise_url_rejects_untrusted_configured_hosts() {
    for ws_url in [
        "wss://attacker.example/v1/noise",
        "wss://attacker.example/metaaivm.com/v1/noise",
        "wss://metaaivm.com.evil.example/v1/noise",
        "https://hatch.metaaivm.com/v1/noise",
    ] {
        let mut config = test_config();
        config.museai_ws_url = ws_url.into();
        config.museai_access_token = "secret-token".into();

        let error =
            toolcase_gateway::museai::build_museai_ws_url(&config, "request-id").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput, "{ws_url}");
    }
}

#[test]
fn exhausted_models_report_final_model_and_status() {
    let client_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = client_listener.local_addr().unwrap().port();
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = upstream_port;
    config.fallbacks = vec!["fallback-model".into()];
    config.retry_base_delay_ms = 0;
    config.max_retry_delay_ms = 0;

    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let gateway = thread::spawn(move || {
        let (client, _) = client_listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });
    let upstream = thread::spawn(move || {
        for _ in 0..2 {
            let (mut socket, _) = upstream_listener.accept().unwrap();
            let _request = toolcase_gateway::http::read_request(&mut socket).unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        }
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    let request = b"{\"model\":\"primary-model\",\"messages\":[]}";
    write!(
        client,
        "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
        request.len(),
        std::str::from_utf8(request).unwrap()
    )
    .unwrap();
    client.flush().unwrap();

    let response = read_response_head(&mut client).unwrap();
    assert_eq!(response.status, 503);
    let mut body = response.buffered_body;
    client.read_to_end(&mut body).unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("fallback-model"));
    assert!(body.contains("HTTP 503"));

    gateway.join().unwrap();
    upstream.join().unwrap();
}

#[test]
fn unauthorized_upstream_fails_over_to_next_model() {
    let client_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = client_listener.local_addr().unwrap().port();
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = upstream_port;
    config.default_model = "primary-model".into();
    config.fallbacks = vec!["fallback-model".into()];
    config.retry_base_delay_ms = 0;
    config.max_retry_delay_ms = 0;

    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let gateway = thread::spawn(move || {
        let (client, _) = client_listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });
    let upstream = thread::spawn(move || {
        let (mut first, _) = upstream_listener.accept().unwrap();
        let first_request = toolcase_gateway::http::read_request(&mut first).unwrap();
        assert_eq!(
            json_string_value(std::str::from_utf8(&first_request.body).unwrap(), "model"),
            Some("primary-model".into())
        );
        first
            .write_all(
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();

        let (mut second, _) = upstream_listener.accept().unwrap();
        let second_request = toolcase_gateway::http::read_request(&mut second).unwrap();
        assert_eq!(
            json_string_value(std::str::from_utf8(&second_request.body).unwrap(), "model"),
            Some("fallback-model".into())
        );
        let body = b"{\"id\":\"fallback-response\"}";
        write!(
            second,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        second.write_all(body).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    let request = b"{\"model\":\"primary-model\",\"messages\":[]}";
    write!(
        client,
        "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
        request.len(),
        std::str::from_utf8(request).unwrap()
    )
    .unwrap();
    client.flush().unwrap();

    let response = read_response_head(&mut client).unwrap();
    assert_eq!(response.status, 200);
    let mut body = response.buffered_body;
    client.read_to_end(&mut body).unwrap();
    assert!(String::from_utf8_lossy(&body).contains("fallback-response"));

    gateway.join().unwrap();
    upstream.join().unwrap();
}

#[test]
fn candidate_models_repairs_unknown_model_to_configured_default() {
    use toolcase_gateway::omniroute::candidate_models;

    let fallbacks: Vec<String> = Vec::new();
    let default = "gpt-5.6-sol";

    // The reported Codex bug: an unknown model id (the "model-medium-hight"
    // typo) must be repaired to a model the upstream actually knows, not
    // forwarded verbatim where it 503s.
    assert_eq!(
        candidate_models(
            b"{\"model\":\"model-medium-hight\"}",
            &fallbacks,
            0,
            default
        ),
        vec![default.to_string()]
    );
    // A valid base with a recognised effort suffix is preserved.
    assert_eq!(
        candidate_models(b"{\"model\":\"gpt-5.6-sol-high\"}", &fallbacks, 0, default),
        vec!["gpt-5.6-sol-high".to_string()]
    );
    // An unknown base with a recognised effort suffix keeps the effort level.
    assert_eq!(
        candidate_models(b"{\"model\":\"weird-high\"}", &fallbacks, 0, default),
        vec!["gpt-5.6-sol-high".to_string()]
    );
    // Fallbacks stay operator-configured and are not rewritten.
    let cfg_fallbacks = vec!["gpt-5.6-sol".to_string()];
    assert_eq!(
        candidate_models(
            b"{\"model\":\"model-medium-hight\"}",
            &cfg_fallbacks,
            0,
            default
        ),
        vec![default.to_string()]
    );
}

#[test]
fn unknown_model_no_longer_exhausts_fallbacks() {
    // End-to-end: a request carrying an unknown model id is normalized to the
    // configured default, which the upstream accepts, so the operator no longer
    // sees "all upstream models exhausted ... HTTP 503".
    let client_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = client_listener.local_addr().unwrap().port();
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = upstream_port;
    config.default_model = "gpt-5.6-sol".into();
    config.fallbacks = vec![];
    config.retry_base_delay_ms = 0;
    config.max_retry_delay_ms = 0;

    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let gateway = thread::spawn(move || {
        let (client, _) = client_listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });
    let upstream = thread::spawn(move || {
        let (mut socket, _) = upstream_listener.accept().unwrap();
        let request = toolcase_gateway::http::read_request(&mut socket).unwrap();
        // The gateway must have repaired the unknown model to the default.
        let body = String::from_utf8(request.body).unwrap();
        assert!(body.contains("\"model\":\"gpt-5.6-sol\""), "body: {body}");
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    let request = b"{\"model\":\"model-medium-hight\",\"messages\":[]}";
    write!(
        client,
        "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
        request.len(),
        std::str::from_utf8(request).unwrap()
    )
    .unwrap();
    client.flush().unwrap();

    let response = read_response_head(&mut client).unwrap();
    assert_eq!(
        response.status, 200,
        "unknown model must be repaired, not 503"
    );

    gateway.join().unwrap();
    upstream.join().unwrap();
}

#[test]
#[ignore = "live upstream check; run only against a configured real Codex/Prism endpoint"]
fn live_codex_model_repair_smoke() {
    // Enabled with:
    //   GW_LIVE_HOST=<host> GW_LIVE_PORT=<port> GW_LIVE_MODEL=model-medium-hight \
    //   cargo test -- --ignored live_codex_model_repair_smoke
    let host = match std::env::var("GW_LIVE_HOST") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("GW_LIVE_HOST not set; skipping live smoke test");
            return;
        }
    };
    let port: u16 = std::env::var("GW_LIVE_PORT")
        .unwrap_or_else(|_| "20128".to_string())
        .parse()
        .unwrap();
    let model = std::env::var("GW_LIVE_MODEL").unwrap_or_else(|_| "model-medium-hight".into());

    use std::net::ToSocketAddrs;
    let addr = (host.as_str(), port)
        .to_socket_addrs()
        .unwrap()
        .next()
        .unwrap();

    let mut socket = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .expect("connect to live upstream");
    socket
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let body = format!(
        r#"{{"model":"{model}","messages":[{{"role":"user","content":"Reply with OK."}}]}}"#
    );
    write!(
        socket,
        "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
    socket.flush().unwrap();

    let head = read_response_head(&mut socket).unwrap();
    assert!(
        head.status < 500,
        "live upstream returned HTTP {status} for model \"{model}\"",
        status = head.status
    );
}

#[test]
fn museai_fails_over_to_fallback() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let fallback_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let fallback_port = fallback_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = fallback_port;
    config.fallbacks = vec!["good_model".into()];
    config.retry_base_delay_ms = 1;
    config.max_retry_delay_ms = 1;

    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });

    let fallback_handle = thread::spawn(move || {
        let (mut upstream, _) = fallback_listener.accept().unwrap();
        let head = toolcase_gateway::http::read_request(&mut upstream).unwrap();
        let body_str = String::from_utf8_lossy(&head.body).into_owned();
        assert!(body_str.contains("\"model\":\"gpt-5.6-sol\""));
        assert!(!body_str.contains("\"model\":\"muse\""));

        upstream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .unwrap();
        upstream.flush().unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let req = b"{\"model\":\"muse\",\"messages\":[]}";
    write!(
        client,
        "POST /v1/chat/completions HTTP/1.1\r\nHost: loc\r\nContent-Length: {}\r\n\r\n{}",
        req.len(),
        std::str::from_utf8(req).unwrap()
    )
    .unwrap();
    client.flush().unwrap();

    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);

    let body = if is_chunked(&head.headers) {
        toolcase_gateway::http::read_chunked_body(&mut client, head.buffered_body).unwrap()
    } else {
        head.buffered_body
    };
    assert_eq!(body, b"ok");

    handle.join().unwrap();
    fallback_handle.join().unwrap();
}

#[test]
fn test_museai_bootstrap_fetches_hatch_token() {
    let mock_hatch = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = mock_hatch.local_addr().unwrap().port();

    let server_thread = std::thread::spawn(move || {
        let (mut client, _) = mock_hatch.accept().unwrap();
        let request = toolcase_gateway::http::read_request(&mut client).unwrap();

        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/hatch/vm/wake");

        let response_body1 = r#"{"status":"wake_requested"}"#;
        use std::io::Write;
        write!(
            client,
            "HTTP/1.1 200 OK
Content-Type: application/json
Content-Length: {}
Connection: close

{}",
            response_body1.len(),
            response_body1
        )
        .unwrap();
        client.flush().unwrap();
        drop(client);

        // The current flow checks the browser session token before falling back
        // to the legacy hatch token endpoint.
        let (mut client, _) = mock_hatch.accept().unwrap();
        let request = toolcase_gateway::http::read_request(&mut client).unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/auth/check");
        client
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        drop(client);

        // now accept the legacy token request
        let (mut client, _) = mock_hatch.accept().unwrap();
        let request = toolcase_gateway::http::read_request(&mut client).unwrap();

        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/hatch/token");

        let response_body = r#"{"token":"test_access","notary_token":"test_notary"}"#;
        write!(
            client,
            "HTTP/1.1 200 OK
Content-Type: application/json
Content-Length: {}
Connection: close

{}",
            response_body.len(),
            response_body
        )
        .unwrap();
        client.flush().unwrap();
        drop(client);

        let (mut client, _) = mock_hatch.accept().unwrap();
        let request = toolcase_gateway::http::read_request(&mut client).unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/api/session");

        let session_body =
            r#"{"vm_id":"test_vm_id","endpoint_url":"wss://test.invalid/v1/noise","vms":[]}"#;
        write!(
            client,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            session_body.len(),
            session_body
        )
        .unwrap();
        client.flush().unwrap();
    });

    let config = toolcase_gateway::config::Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        default_model: "gpt-5.6-sol".into(),
        museai_base_url: format!("http://127.0.0.1:{}", port),
        museai_cookie: "test_cookie".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "test_vm_id".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
        jev_api_keys: vec![],
    };

    let bootstrapped = toolcase_gateway::museai::bootstrap_museai_config(&config).unwrap();
    assert_eq!(bootstrapped.museai_access_token, "test_access");
    assert_eq!(bootstrapped.museai_notary_token, "test_notary");
    server_thread.join().unwrap();
}

#[test]
fn test_museai_bootstrap_uses_auth_check_and_current_session_shape() {
    let mock_muse = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = mock_muse.local_addr().unwrap().port();

    let server_thread = std::thread::spawn(move || {
        let (mut client, _) = mock_muse.accept().unwrap();
        let request = toolcase_gateway::http::read_request(&mut client).unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/auth/check");
        let body = r#"{"access_token":"current-access","ok":true}"#;
        write!(
            client,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
        drop(client);

        let (mut client, _) = mock_muse.accept().unwrap();
        let request = toolcase_gateway::http::read_request(&mut client).unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/api/session");
        let body = r#"{
            "vms": [
                {"id":"fallback-id","endpoint_url":"wss://fallback.metaaivm.com/","is_preferred":false},
                {"vm_id":"preferred-vm","endpoint_url":"wss://preferred.metaaivm.com/","is_preferred":true}
            ]
        }"#;
        write!(
            client,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    let mut config = test_config();
    config.museai_base_url = format!("http://127.0.0.1:{port}");
    config.museai_notary_token = "preserved-notary".into();

    let bootstrapped = toolcase_gateway::museai::bootstrap_museai_config(&config).unwrap();
    assert_eq!(bootstrapped.museai_access_token, "current-access");
    assert_eq!(bootstrapped.museai_notary_token, "preserved-notary");
    assert_eq!(bootstrapped.museai_vm_id, "preferred-vm");
    assert_eq!(bootstrapped.museai_ws_url, "wss://preferred.metaaivm.com/");
    server_thread.join().unwrap();
}

#[test]
fn test_confidential_vm_message_three_payload() {
    let notary_token = "test_notary_token_12345";
    let payload =
        toolcase_gateway::museai::noise::encode_confidential_vm_message_three(notary_token);

    // Tag 1 (notary_token) = 10
    assert_eq!(payload[0], 10);

    // Length of token
    let token_len = notary_token.len() as u8;
    assert_eq!(payload[1], token_len);

    // Token content
    let token_end = 2 + token_len as usize;
    assert_eq!(&payload[2..token_end], notary_token.as_bytes());

    // Tag 2 (fresh_rv_key) = 18
    assert_eq!(payload[token_end], 18);

    // Length of fresh_rv_key = 32
    assert_eq!(payload[token_end + 1], 32);

    // Total len: tag(1) + len(1) + str(23) + tag(1) + len(1) + bytes(32) = 59
    assert_eq!(payload.len(), 59);

    // Ensure rv_key isn't all zeros
    assert_ne!(&payload[token_end + 2..], &[0u8; 32]);
}
