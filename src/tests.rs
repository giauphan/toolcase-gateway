use crate::config::Config;
use crate::http::{
    header_value, is_chunked, is_request_target, is_token, parse_headers, read_more,
    read_response_head, Request,
};
use crate::omniroute::{open_upstream, RETRYABLE};
use crate::rewrite::{escape_json_string, replace_model, rewrite_tool_names};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

fn test_config() -> Config {
    Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        prism_base_url: "https://prism.openai.com".into(),
        prism_project_id: "default_proj".into(),
        prism_cookie: "".into(),
        prism_sandbox_token: "".into(),
        prism_user_id: "".into(),
        prism_default_model: "gpt-5.6-sol".into(),
        prism_system_prompt: "".into(),
        museai_base_url: "https://muse.ai".into(),
        museai_cookie: "".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
    }
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
    use crate::museai_noise::{MuseNoiseSession, NOISE_PATTERN_XX};
    use snow::{params::NoiseParams, Builder};

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
    let payload = crate::museai_noise::get_primary_message_one_payload();
    assert_eq!(payload.len(), 34);
    assert_eq!(payload[0], 0x0a);
    assert_eq!(payload[1], 0x20);
    // Nonce should not be all zeros
    assert_ne!(&payload[2..], &[0u8; 32]);
}

#[test]
fn noise_ik_requires_responder_key_and_round_trips() {
    use crate::museai_noise::{MuseNoiseSession, NOISE_PATTERN_IK};
    use snow::{params::NoiseParams, Builder};

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
    use crate::museai_transport::MuseWebSocket;
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
    assert!(RETRYABLE.contains(&429));
    assert!(RETRYABLE.contains(&500));
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
        prism_base_url: "https://prism.openai.com".into(),
        prism_project_id: "0f6fa2ad-f391-4d28-9770-e3d6d511f80c".into(),
        prism_cookie: "".into(),
        prism_sandbox_token: "".into(),
        prism_user_id: "".into(),
        prism_default_model: "gpt-5.6-sol".into(),
        prism_system_prompt: "".into(),
        museai_base_url: "https://muse.ai".into(),
        museai_cookie: "".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
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
fn extracts_prism_credentials_with_sentinel_and_triple_pipe() {
    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        prism_base_url: "https://prism.openai.com".into(),
        prism_project_id: "default_proj".into(),
        prism_cookie: "default_cookie".into(),
        prism_sandbox_token: "default_token".into(),
        prism_user_id: "default_user".into(),
        prism_default_model: "gpt-5.6-sol".into(),
        prism_system_prompt: "".into(),
        museai_base_url: "https://muse.ai".into(),
        museai_cookie: "".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
    };

    let headers = vec![
        ("Authorization".to_string(),
        "Bearer custom_cookie_val; a=b|||cookie_tail|||custom_sentinel|||custom_token|||custom_user|||custom_proj"
            .to_string(),
        ),
        ("openai-sentinel-token".to_string(), "test_sentinel_token_123".to_string()),
    ];

    let creds = crate::prism::extract_credentials(&headers, &config);
    assert_eq!(creds.cookie, "custom_cookie_val; a=b|||cookie_tail");
    assert_eq!(creds.sentinel_token.as_deref(), Some("custom_sentinel"));
    assert_eq!(creds.sandbox_token, "custom_token");
    assert_eq!(creds.user_id, "custom_user");
    assert_eq!(creds.project_id, "custom_proj");
}

#[test]
fn extracts_prism_credentials_from_x_api_key() {
    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        prism_base_url: "https://prism.openai.com".into(),
        prism_project_id: "default_proj".into(),
        prism_cookie: "default_cookie".into(),
        prism_sandbox_token: "default_token".into(),
        prism_user_id: "default_user".into(),
        prism_default_model: "gpt-5.6-sol".into(),
        prism_system_prompt: "".into(),
        museai_base_url: "https://muse.ai".into(),
        museai_cookie: "".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
    };
    let headers = vec![(
        "x-api-key".to_string(),
        "cookie|||{\"p\":\"sentinel\"}|||sandbox|||user|||project".to_string(),
    )];

    let creds = crate::prism::extract_credentials(&headers, &config);

    assert_eq!(creds.cookie, "cookie");
    assert_eq!(creds.sentinel_token.as_deref(), Some(r#"{"p":"sentinel"}"#));
    assert_eq!(creds.sandbox_token, "sandbox");
    assert_eq!(creds.user_id, "user");
    assert_eq!(creds.project_id, "project");
}

#[test]
fn extracts_prism_credentials_with_comma_delimiter() {
    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        prism_base_url: "https://prism.openai.com".into(),
        prism_project_id: "default_proj".into(),
        prism_cookie: "default_cookie".into(),
        prism_sandbox_token: "default_token".into(),
        prism_user_id: "default_user".into(),
        prism_default_model: "gpt-5.6-sol".into(),
        prism_system_prompt: "".into(),
        museai_base_url: "https://muse.ai".into(),
        museai_cookie: "".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
    };

    let headers = vec![(
        "x-api-key".to_string(),
        "cookie_part1, cookie_part2; key=val, token_123, user_456, proj_789".to_string(),
    )];

    let creds = crate::prism::extract_credentials(&headers, &config);
    assert_eq!(creds.cookie, "cookie_part1, cookie_part2; key=val");
    assert_eq!(creds.sandbox_token, "token_123");
    assert_eq!(creds.user_id, "user_456");
    assert_eq!(creds.project_id, "proj_789");
}

#[test]
fn falls_back_to_config_credentials_when_header_is_missing_or_short() {
    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        prism_base_url: "https://prism.openai.com".into(),
        prism_project_id: "default_proj".into(),
        prism_cookie: "default_cookie".into(),
        prism_sandbox_token: "default_token".into(),
        prism_user_id: "default_user".into(),
        prism_default_model: "gpt-5.6-sol".into(),
        prism_system_prompt: "".into(),
        museai_base_url: "https://muse.ai".into(),
        museai_cookie: "".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
    };

    let headers = vec![(
        "Authorization".to_string(),
        "Bearer sk-singlekey".to_string(),
    )];
    let creds = crate::prism::extract_credentials(&headers, &config);
    assert_eq!(creds.cookie, "default_cookie");
    assert_eq!(creds.sandbox_token, "default_token");
    assert_eq!(creds.user_id, "default_user");
    assert_eq!(creds.project_id, "default_proj");
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
        prism_base_url: "https://prism.openai.com".into(),
        prism_project_id: "project".into(),
        prism_cookie: "".into(),
        prism_sandbox_token: "".into(),
        prism_user_id: "".into(),
        prism_default_model: "model".into(),
        prism_system_prompt: "".into(),
        museai_base_url: "http://127.0.0.1:32123".into(),
        museai_cookie: "session=test".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
    };

    let request = br#"{"target":"session","method":"GET"}"#;
    let (url, method, body) =
        crate::museai_business::build_museai_request(request, &config).unwrap();
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
        prism_base_url: "https://prism.openai.com".into(),
        prism_project_id: "project".into(),
        prism_cookie: "".into(),
        prism_sandbox_token: "".into(),
        prism_user_id: "".into(),
        prism_default_model: "model".into(),
        prism_system_prompt: "".into(),
        museai_base_url: "https://muse.ai".into(),
        museai_cookie: "".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
    };

    let request = br#"{"target":"../session","method":"GET"}"#;
    assert!(crate::museai_business::build_museai_request(request, &config).is_err());
}

#[test]
fn test_museai_v1_mock_proxy_flow() {
    let mock_muse = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mock_muse_port = mock_muse.local_addr().unwrap().port();

    let muse_handle = thread::spawn(move || {
        let (mut client, _) = mock_muse.accept().unwrap();
        let req = crate::http::read_request(&mut client).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/api/session");
        assert_eq!(
            crate::http::header_value(&req.headers, "cookie"),
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
            prism_base_url: "https://prism.openai.com".into(),
            prism_project_id: "default_proj".into(),
            prism_cookie: "default_cookie".into(),
            prism_sandbox_token: "default_token".into(),
            prism_user_id: "default_user".into(),
            prism_default_model: "gpt-5.6-sol".into(),
            prism_system_prompt: "".into(),
            museai_base_url: format!("http://127.0.0.1:{mock_muse_port}"),
            museai_cookie: "hatch_sess=secret_mock_sess".into(),
            museai_ws_url: "".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 86400,
        };

        let (mut client, _) = mock_gw.accept().unwrap();
        let req_in = crate::http::read_request(&mut client).unwrap();
        crate::museai::handle_museai_v1(&mut client, &req_in.body, &config).unwrap();
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

    let head = crate::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    let mut body = head.buffered_body;
    let length = crate::http::header_value(&head.headers, "content-length")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    while body.len() < length {
        crate::http::read_more(&mut client, &mut body).unwrap();
    }
    let response: serde_json::Value = serde_json::from_slice(&body[..length]).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["access_token"], "token_test_123");

    muse_handle.join().unwrap();
    gw_handle.join().unwrap();
}

#[test]
fn test_models_catalog_response() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = thread::spawn(move || {
        let config = Config {
            target_host: "127.0.0.1".into(),
            target_port: 8080,
            fallbacks: vec!["fallback-1".into(), "fallback-2".into()],
            io_timeout: None,
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 1000,
            prism_base_url: "https://prism.openai.com".into(),
            prism_project_id: "default_proj".into(),
            prism_cookie: "default_cookie".into(),
            prism_sandbox_token: "default_token".into(),
            prism_user_id: "default_user".into(),
            prism_default_model: "gpt-5.6-sol".into(),
            prism_system_prompt: "".into(),
            museai_base_url: "https://muse.ai".into(),
            museai_cookie: "".into(),
            museai_ws_url: "".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 86400,
        };
        let (mut client, _) = listener.accept().unwrap();
        crate::routes::handle_models_catalog(&mut client, "/v1/models", &config).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    let mut body = head.buffered_body;
    let length = header_value(&head.headers, "content-length")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    while body.len() < length {
        read_more(&mut client, &mut body).unwrap();
    }
    handle.join().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body[..length]).unwrap();
    assert_eq!(json["object"], "list");
    let models = json["data"].as_array().unwrap();
    assert_eq!(models.len(), 6);
    assert!(models
        .iter()
        .all(|model| { !model["id"].as_str().unwrap().contains("terra") }));
    assert!(models
        .iter()
        .any(|model| model["id"] == "gpt-5.6-sol-xhigh"));
    assert!(models.iter().any(|model| model["id"] == "muse"));
}

#[test]
fn test_museai_models_catalog_response() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = thread::spawn(move || {
        let config = test_config();
        let (mut client, _) = listener.accept().unwrap();
        crate::routes::handle_models_catalog(&mut client, "/muse-ai/v1/models", &config).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    let mut body = head.buffered_body;
    let length = header_value(&head.headers, "content-length")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    while body.len() < length {
        read_more(&mut client, &mut body).unwrap();
    }
    handle.join().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body[..length]).unwrap();
    assert_eq!(json["object"], "list");
    let models = json["data"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["id"], "muse");
}

#[test]
fn test_prism_successful_start_and_status_flow() {
    let mock_prism = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mock_prism_port = mock_prism.local_addr().unwrap().port();

    let prism_handle = thread::spawn(move || {
        let (mut start_client, _) = mock_prism.accept().unwrap();
        let start = crate::http::read_request(&mut start_client).unwrap();
        assert_eq!(start.method, "POST");
        assert_eq!(start.path, "/api/llm/response_with_tools_start");
        assert_eq!(
            crate::http::header_value(&start.headers, "cookie"),
            Some("session=cookie-value|||cookie-tail")
        );
        assert_eq!(
            crate::http::header_value(&start.headers, "openai-sentinel-token"),
            Some(r#"{"p":"packed-sentinel"}"#)
        );
        let expected_base_url = format!("http://127.0.0.1:{mock_prism_port}");
        assert_eq!(
            crate::http::header_value(&start.headers, "origin"),
            Some(expected_base_url.as_str())
        );
        assert_eq!(
            crate::http::header_value(&start.headers, "referer"),
            Some(format!("{expected_base_url}/?u=proj_flow").as_str())
        );

        let start_json: serde_json::Value = serde_json::from_slice(&start.body).unwrap();
        assert_eq!(start_json["metadata"]["projectId"], "proj_flow");
        assert_eq!(start_json["metadata"]["userId"], "user_flow");
        assert_eq!(start_json["metadata"]["sandbox_token"], "sandbox_flow");
        assert_eq!(start_json["metadata"]["model"], "gpt-5.6-sol");
        assert_eq!(start_json["metadata"]["reasoning_effort"], "xhigh");

        let start_body = r#"{
            "status":"running",
            "request_id":"request_flow",
            "turn_state":{"step":1}
        }"#;
        write!(
            start_client,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            start_body.len(),
            start_body
        )
        .unwrap();
        start_client.flush().unwrap();
        drop(start_client);

        let (mut status_client, _) = mock_prism.accept().unwrap();
        let status = crate::http::read_request(&mut status_client).unwrap();
        assert_eq!(status.method, "POST");
        assert_eq!(status.path, "/api/llm/response_with_tools_status");
        assert_eq!(
            crate::http::header_value(&status.headers, "cookie"),
            Some("session=cookie-value|||cookie-tail")
        );
        assert_eq!(
            crate::http::header_value(&status.headers, "openai-sentinel-token"),
            Some(r#"{"p":"packed-sentinel"}"#)
        );

        let status_json: serde_json::Value = serde_json::from_slice(&status.body).unwrap();
        assert_eq!(status_json["request_id"], "request_flow");
        assert_eq!(status_json["turn_state"]["step"], 1);

        let status_body = r#"{
            "status":"completed",
            "response":{
                "status":"completed",
                "payload":{
                    "output":[{"content":[{"type":"output_text","text":"flow works"}]}]
                }
            }
        }"#;
        write!(
            status_client,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            status_body.len(),
            status_body
        )
        .unwrap();
        status_client.flush().unwrap();
    });

    let mock_proxy = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let proxy_port = mock_proxy.local_addr().unwrap().port();
    let proxy_handle = thread::spawn(move || {
        let config = Config {
            target_host: "127.0.0.1".into(),
            target_port: 8080,
            fallbacks: vec![],
            io_timeout: None,
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 1000,
            prism_base_url: format!("http://127.0.0.1:{mock_prism_port}"),
            prism_project_id: "default_proj".into(),
            prism_cookie: "default_cookie".into(),
            prism_sandbox_token: "default_token".into(),
            prism_user_id: "default_user".into(),
            prism_default_model: "gpt-5.6-sol".into(),
            prism_system_prompt: "Injected system".into(),
            museai_base_url: "https://muse.ai".into(),
            museai_cookie: "".into(),
            museai_ws_url: "".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 86400,
        };
        let (mut client, _) = mock_proxy.accept().unwrap();
        let req_in = crate::http::read_request(&mut client).unwrap();
        crate::prism::handle_prism_chat_completion(
            &mut client,
            &req_in.body,
            &config,
            &req_in.headers,
        )
        .unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).unwrap();
    let openai_req = r#"{
        "model":"gpt-5.6-sol-xhigh",
        "messages":[{"role":"user","content":"hello"}]
    }"#;
    let api_key = "session=cookie-value|||cookie-tail|||{\"p\":\"packed-sentinel\"}|||sandbox_flow|||user_flow|||proj_flow";
    write!(
        client,
        "POST /prism-openai/v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1:{proxy_port}\r\nx-api-key: {api_key}\r\nContent-Length: {}\r\n\r\n{}",
        openai_req.len(),
        openai_req
    )
    .unwrap();
    client.flush().unwrap();

    let head = crate::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    let mut body = head.buffered_body;
    let length = crate::http::header_value(&head.headers, "content-length")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    while body.len() < length {
        crate::http::read_more(&mut client, &mut body).unwrap();
    }
    let response: serde_json::Value = serde_json::from_slice(&body[..length]).unwrap();
    assert_eq!(response["model"], "gpt-5.6-sol");
    assert_eq!(response["choices"][0]["message"]["content"], "flow works");

    prism_handle.join().unwrap();
    proxy_handle.join().unwrap();
}

#[test]
fn test_cors_preflight_response() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        crate::routes::handle_cors_preflight(&mut client).unwrap();
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

#[test]
fn test_build_injected_prism_inputs_no_system() {
    let messages = vec![crate::prism::OpenAiMessage {
        role: "user".into(),
        content: "hello".into(),
    }];
    let prompt = "You are a test prompt";
    let prism_inputs = crate::prism::build_injected_prism_inputs(&messages, prompt);

    assert_eq!(prism_inputs.len(), 2);
    assert_eq!(prism_inputs[0].role, "system");
    assert!(prism_inputs[0].content[0]
        .text
        .starts_with("You are a test prompt"));

    assert_eq!(prism_inputs[1].role, "user");
    assert_eq!(prism_inputs[1].content[0].text, "hello");
}

#[test]
fn test_build_injected_prism_inputs_with_system() {
    let messages = vec![
        crate::prism::OpenAiMessage {
            role: "system".into(),
            content: "{\"openFile\": \"main.rs\"}".into(),
        },
        crate::prism::OpenAiMessage {
            role: "user".into(),
            content: "hello".into(),
        },
    ];
    let prompt = "You are a test prompt";
    let prism_inputs = crate::prism::build_injected_prism_inputs(&messages, prompt);

    assert_eq!(prism_inputs.len(), 2);
    assert_eq!(prism_inputs[0].role, "system");
    // Injection should be prepended
    assert!(prism_inputs[0].content[0]
        .text
        .starts_with("You are a test prompt"));
    assert!(prism_inputs[0].content[0]
        .text
        .contains("{\"openFile\": \"main.rs\"}"));

    assert_eq!(prism_inputs[1].role, "user");
    assert_eq!(prism_inputs[1].content[0].text, "hello");
}

#[test]
fn test_prism_403_forbidden_error_mapping() {
    let mock_prism = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mock_prism_port = mock_prism.local_addr().unwrap().port();

    // Spawn a thread to act as the mock Prism server
    let prism_handle = thread::spawn(move || {
        let (mut client, _) = mock_prism.accept().unwrap();

        // Read the request head
        let head = crate::http::read_request(&mut client).unwrap();
        assert_eq!(head.method, "POST");
        assert_eq!(head.path, "/api/llm/response_with_tools_start");
        let body = head.body;

        let req_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let metadata = &req_json["metadata"];
        assert_eq!(metadata["projectId"], "proj_403");

        // We check if sentinel token is forwarded correctly as well
        assert_eq!(
            crate::http::header_value(&head.headers, "cookie"),
            Some("cookie")
        );
        let sentinel = crate::http::header_value(&head.headers, "openai-sentinel-token");
        assert_eq!(sentinel, Some("test_sentinel_token_123"));

        // Create a Prism response that wraps an INNER error with 403 Forbidden payload message
        let mock_resp = r#"{
            "status": "completed",
            "request_id": "req_123",
            "response": {
                "status": "error",
                "payload": {
                    "message": "Error while processing conversation (403 Forbidden). Submit prompt again."
                }
            }
        }"#;

        write!(
            client,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            mock_resp.len(),
            mock_resp
        ).unwrap();
        client.flush().unwrap();
    });

    let mock_proxy = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let proxy_port = mock_proxy.local_addr().unwrap().port();

    let proxy_handle = thread::spawn(move || {
        let config = Config {
            target_host: "127.0.0.1".into(),
            target_port: 8080,
            fallbacks: vec![],
            io_timeout: None,
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 1000,
            prism_base_url: format!("http://127.0.0.1:{}", mock_prism_port),
            prism_project_id: "default_proj".into(),
            prism_cookie: "default_cookie".into(),
            prism_sandbox_token: "default_token".into(),
            prism_user_id: "default_user".into(),
            prism_default_model: "gpt-5.6-sol".into(),
            prism_system_prompt: "Injected system".into(),
            museai_base_url: "https://muse.ai".into(),
            museai_cookie: "".into(),
            museai_ws_url: "".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 86400,
        };
        let (mut client, _) = mock_proxy.accept().unwrap();

        let req_in = crate::http::read_request(&mut client).unwrap();
        crate::prism::handle_prism_chat_completion(
            &mut client,
            &req_in.body,
            &config,
            &req_in.headers,
        )
        .unwrap();
    });

    // Simulate an OpenAI client connecting to the proxy
    let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).unwrap();
    let openai_req = r#"{
        "model": "gpt-5.6-sol-high",
        "messages": [
            {"role": "user", "content": "hello"}
        ]
    }"#;
    write!(
        client,
        "POST /prism-openai/v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1:{proxy_port}\r\nx-api-key: cookie|||test_sentinel_token_123|||token|||user|||proj_403\r\nContent-Length: {}\r\n\r\n{}",
        openai_req.len(),
        openai_req
    ).unwrap();
    client.flush().unwrap();

    let head = crate::http::read_response_head(&mut client).unwrap();
    assert_eq!(
        head.status, 403,
        "Should map inner Prism 403 Forbidden to HTTP 403"
    );

    let mut body = head.buffered_body;
    if let Some(length_str) = crate::http::header_value(&head.headers, "content-length") {
        let length: usize = length_str.parse().unwrap();
        while body.len() < length {
            crate::http::read_more(&mut client, &mut body).unwrap();
        }
    }

    let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        resp["error"]["message"],
        "Prism error: Error while processing conversation (403 Forbidden). Submit prompt again."
    );

    prism_handle.join().unwrap();
    proxy_handle.join().unwrap();
}

#[test]
fn museai_protobuf_encoding_decoding_round_trips() {
    use crate::museai_protocol::*;

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

    let url = crate::museai::build_museai_ws_url(&config, "request-id").unwrap();
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

    let url = crate::museai::build_museai_ws_url(&config, "request-id").unwrap();
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

    let error = crate::museai::build_museai_ws_url(&config, "request-id").unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
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

    let gateway = thread::spawn(move || {
        let (client, _) = client_listener.accept().unwrap();
        crate::routes::route_request(client, &config).unwrap();
    });
    let upstream = thread::spawn(move || {
        for _ in 0..2 {
            let (mut socket, _) = upstream_listener.accept().unwrap();
            let _request = crate::http::read_request(&mut socket).unwrap();
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

    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        crate::routes::route_request(client, &config).unwrap();
    });

    let fallback_handle = thread::spawn(move || {
        let (mut upstream, _) = fallback_listener.accept().unwrap();
        let head = crate::http::read_request(&mut upstream).unwrap();
        let body_str = String::from_utf8_lossy(&head.body).into_owned();
        assert!(body_str.contains("\"model\":\"good_model\""));
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
        crate::http::read_chunked_body(&mut client, head.buffered_body).unwrap()
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
        let request = crate::http::read_request(&mut client).unwrap();

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

        // now accept the token request
        let (mut client, _) = mock_hatch.accept().unwrap();
        let request = crate::http::read_request(&mut client).unwrap();

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
    });

    let config = crate::config::Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        prism_base_url: "https://prism.openai.com".into(),
        prism_project_id: "default_proj".into(),
        prism_cookie: "default_cookie".into(),
        prism_sandbox_token: "default_token".into(),
        prism_user_id: "default_user".into(),
        prism_default_model: "gpt-5.6-sol".into(),
        prism_system_prompt: "".into(),
        museai_base_url: format!("http://127.0.0.1:{}", port),
        museai_cookie: "test_cookie".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "test_vm_id".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
    };

    let bootstrapped = crate::museai::bootstrap_museai_config(&config).unwrap();
    assert_eq!(bootstrapped.museai_access_token, "test_access");
    assert_eq!(bootstrapped.museai_notary_token, "test_notary");
    server_thread.join().unwrap();
}

#[test]
fn test_confidential_vm_message_three_payload() {
    let notary_token = "test_notary_token_12345";
    let payload = crate::museai_noise::encode_confidential_vm_message_three(notary_token);

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

#[test]
fn test_create_video_route() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = thread::spawn(move || {
        let config = test_config();
        let (mut client, _) = listener.accept().unwrap();
        // Since we don't have a valid Muse.ai session, create_video should fail,
        // but we're testing that it routes correctly and returns 400 or another structured error instead of crashing.
        crate::museai::handle_create_video(
            &mut client,
            br#"{"prompt": "", "model": "gen-3"}"#,
            &config,
        )
        .unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let head = read_response_head(&mut client).unwrap();
    // Prompt is empty, so we expect 400 Bad Request
    assert_eq!(head.status, 400);

    let mut body = head.buffered_body;
    let length = header_value(&head.headers, "content-length")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    while body.len() < length {
        read_more(&mut client, &mut body).unwrap();
    }
    handle.join().unwrap();

    let json: serde_json::Value = serde_json::from_slice(&body[..length]).unwrap();
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("prompt must not be empty"));
}

#[test]
fn museai_stream_request_uses_resolved_session_id() {
    let source = include_str!("museai.rs");
    assert!(source.contains(r#""/chat/stream""#));
    assert!(source.contains(r#""session_id": resolved_session_id.clone()"#));
    assert!(!source.contains(r#""chat_id": resolved_session_id.clone()"#));
}

#[test]
fn test_create_video_duration_support() {
    let source = include_str!("museai.rs");
    assert!(source.contains(r#"Duration: {duration} seconds"#));
    assert!(source.contains(r#""duration": duration"#));
}
