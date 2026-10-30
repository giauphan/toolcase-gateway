//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;

// ---------- Muse protocol framing, Noise URL, and bootstrap tests ----------

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

        let session_body = r#"{"vm_state":"RUNNING","vm_id":"test_vm_id","endpoint_url":"wss://test.invalid/v1/noise","vms":[]}"#;
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
        jev_base_url: "https://jev-ai.org".into(),
        ..Config::default()
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
            "vm_state": "RUNNING",
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
