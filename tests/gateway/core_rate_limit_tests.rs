//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;

// ---------- Core admission, rate-limit, retry, and fail-closed route tests ----------

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

#[test]
fn request_is_rejected_when_byte_budget_is_exceeded() {
    let client_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = client_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_port = 65530;
    let store = toolcase_gateway::config::ConfigStore::new_with_limits(
        config,
        std::path::PathBuf::from(".env"),
        1,
        4,
    );
    let gateway = thread::spawn(move || {
        let (client, _) = client_listener.accept().unwrap();
        let _ = toolcase_gateway::routes::route_request(client, &store);
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    client
        .write_all(b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\nConnection: close\r\n\r\n12345")
        .unwrap();
    let response = read_response_head(&mut client).unwrap();

    gateway.join().unwrap();
    assert_eq!(response.status, 503);
}

#[test]
fn chunked_request_is_rejected_when_byte_budget_is_exceeded() {
    let client_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = client_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_port = 65530;
    let store = toolcase_gateway::config::ConfigStore::new_with_limits(
        config,
        std::path::PathBuf::from(".env"),
        1,
        4,
    );
    let gateway = thread::spawn(move || {
        let (client, _) = client_listener.accept().unwrap();
        let _ = toolcase_gateway::routes::route_request(client, &store);
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    client
        .write_all(b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\n12345\r\n0\r\n\r\n")
        .unwrap();
    let response = read_response_head(&mut client).unwrap();

    gateway.join().unwrap();
    assert_eq!(response.status, 503);
}

#[test]
fn oversized_request_returns_413() {
    let client_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = client_listener.local_addr().unwrap().port();
    let store = toolcase_gateway::config::ConfigStore::new(test_config(), ".env".into());
    let gateway = thread::spawn(move || {
        let (client, _) = client_listener.accept().unwrap();
        let _ = toolcase_gateway::routes::route_request(client, &store);
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    client
        .write_all(b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: 67108865\r\nConnection: close\r\n\r\n")
        .unwrap();
    let response = read_response_head(&mut client).unwrap();

    gateway.join().unwrap();
    assert_eq!(response.status, 413);
}

#[test]
fn status_400_does_not_retry_or_fail_over() {
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

    let upstream_accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let upstream_accepted_clone = upstream_accepted.clone();
    let upstream = thread::spawn(move || {
        let (mut socket, _) = upstream_listener.accept().unwrap();
        upstream_accepted_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _req = toolcase_gateway::http::read_request(&mut socket).unwrap();
        let body = b"bad request error";
        write!(
            socket,
            "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        socket.write_all(body).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    let req = b"{\"model\":\"primary-model\",\"messages\":[]}";
    write!(
        client,
        "POST /ominroute/v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
        req.len(),
        std::str::from_utf8(req).unwrap()
    )
    .unwrap();
    client.flush().unwrap();

    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 400);
    let mut body = head.buffered_body;
    let _ = client.read_to_end(&mut body);
    assert!(String::from_utf8_lossy(&body).contains("bad request error"));

    gateway.join().unwrap();
    upstream.join().unwrap();

    // Verify upstream was only hit once (no failover retry attempted for 400)
    assert_eq!(
        upstream_accepted.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[test]
fn retry_classification_handles_429_and_5xx_correctly() {
    use toolcase_gateway::omniroute::is_status_retryable;
    assert!(is_status_retryable(429));
    assert!(is_status_retryable(500));
    assert!(is_status_retryable(502));
    assert!(is_status_retryable(503));
    assert!(is_status_retryable(504));
    assert!(!is_status_retryable(400));
    assert!(!is_status_retryable(404));
}

#[test]
fn candidate_planning_determinism_and_edge_cases() {
    use toolcase_gateway::omniroute::candidate_models;
    let fallbacks = vec!["fb1".to_string(), "fb2".to_string()];

    // Empty body or missing model field falls back to configured fallback rotation.
    // (Default model only used when no candidates and no fallbacks.)
    let c1 = candidate_models(b"{}", &fallbacks, 0, "default-mod");
    assert_eq!(c1[0], "fb1");
    assert_eq!(c1[1], "fb2");

    // Rotation offset is deterministic
    let c2 = candidate_models(b"{}", &fallbacks, 1, "default-mod");
    assert_eq!(c2[0], "fb2");
    assert_eq!(c2[1], "fb1");

    // Malformed JSON does not panic, uses fallbacks rotation.
    let c3 = candidate_models(b"not-json", &fallbacks, 0, "default-mod");
    assert_eq!(c3[0], "fb1");
    assert_eq!(c3[1], "fb2");
}

#[test]
fn admission_controller_quota_guards_and_release() {
    use toolcase_gateway::runtime::admission::AdmissionController;

    let controller = AdmissionController::new(1, 100);
    {
        let conn_guard = controller.acquire_connection();
        assert!(conn_guard.is_some());
        // Second acquisition fails while guard is alive
        assert!(controller.acquire_connection().is_none());

        let byte_guard = controller.acquire_bytes(60);
        assert!(byte_guard.is_some());
        // Exceeds 100 limit
        assert!(controller.acquire_bytes(50).is_none());
    } // guards dropped here

    // Quotas must be fully released after drop
    let conn_guard2 = controller.acquire_connection();
    assert!(conn_guard2.is_some());

    let byte_guard2 = controller.acquire_bytes(100);
    assert!(byte_guard2.is_some());
}

#[test]
fn route_namespace_fail_closed_behavior_offline() {
    // Verify that requests to protected namespaces return proper error/status and don't fall through
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store =
        toolcase_gateway::config::ConfigStore::new(test_config(), std::path::PathBuf::from(".env"));
    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        let _ = toolcase_gateway::routes::route_request(client, &store);
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "POST /jev/v1/unknown-action HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();

    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 405); // Method not allowed / route gating
    handle.join().unwrap();
}

#[test]
fn unowned_route_fails_closed_without_upstream_contact() {
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    upstream_listener.set_nonblocking(true).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();

    let client_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = client_listener.local_addr().unwrap().port();
    let mut config = test_config();
    config.target_port = upstream_port;
    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let gateway = thread::spawn(move || {
        let (client, _) = client_listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    write!(
        client,
        "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    )
    .unwrap();
    client.flush().unwrap();

    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 404);
    assert_eq!(head.reason, "Not Found");
    gateway.join().unwrap();
    // Upstream should have received zero connections
    assert!(upstream_listener.accept().is_err());
}
