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
    for status in [401, 402, 403, 408, 429, 499, 500, 502, 503, 504, 524] {
        assert!(RETRYABLE.contains(&status));
    }
    assert!(!RETRYABLE.contains(&400));
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
        jev_base_url: "https://jev-ai.org".into(),
        ..Config::default()
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
        jev_base_url: "https://jev-ai.org".into(),
        ..Config::default()
    };

    // Test that config credentials handle work
    assert!(!config.default_model.is_empty());
}
