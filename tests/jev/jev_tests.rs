//! Integration tests for the Jev AI multi-account pool and retryable-status logic.

use toolcase_gateway::jev::business::{is_account_retryable, is_retryable, is_transient_retryable};
use toolcase_gateway::jev::handlers::map_jev_upstream_path;
use toolcase_gateway::jev::pool::JevAccountPool;

#[test]
fn test_jev_account_pool_round_robin() {
    let keys = vec!["key1".to_string(), "key2".to_string(), "key3".to_string()];
    let pool = JevAccountPool::new(&keys);
    assert_eq!(pool.len(), 3);
    assert!(!pool.is_empty());

    let candidates1 = pool.candidate_accounts();
    assert_eq!(candidates1.len(), 3);
    assert_eq!(candidates1[0].api_key, "key1");
    assert_eq!(candidates1[1].api_key, "key2");
    assert_eq!(candidates1[2].api_key, "key3");

    let candidates2 = pool.candidate_accounts();
    assert_eq!(candidates2[0].api_key, "key2");
    assert_eq!(candidates2[1].api_key, "key3");
    assert_eq!(candidates2[2].api_key, "key1");
}

#[test]
fn test_jev_account_pool_empty() {
    let pool = JevAccountPool::new(&[]);
    assert!(pool.is_empty());
    assert_eq!(pool.len(), 0);
    assert!(pool.next_account().is_none());
    assert!(pool.candidate_accounts().is_empty());
}

#[test]
fn test_account_retryable_statuses() {
    assert!(is_account_retryable(401));
    assert!(is_account_retryable(402));
    assert!(is_account_retryable(403));
    assert!(is_account_retryable(429));
    assert!(!is_account_retryable(200));
    assert!(!is_account_retryable(400));
    assert!(!is_account_retryable(404));
    assert!(!is_account_retryable(499));
    assert!(!is_account_retryable(500));
    assert!(is_transient_retryable(499));
    assert!(is_transient_retryable(502));
    assert!(is_transient_retryable(503));
    assert!(is_transient_retryable(504));
    assert!(is_retryable(429));
    assert!(is_retryable(503));
    assert!(is_retryable(504));
    assert!(!is_retryable(400));
}

#[test]
fn test_jev_documented_paths_map_to_canonical_endpoints() {
    assert_eq!(
        map_jev_upstream_path("/jev/v1/models").as_deref(),
        Some("/api/v1/models/")
    );
    assert_eq!(
        map_jev_upstream_path("/jev/v1/systemone").as_deref(),
        Some("/api/v1/systemone/")
    );
    assert_eq!(
        map_jev_upstream_path("/jev-ai/v1/systemone/").as_deref(),
        Some("/api/v1/systemone/")
    );
    assert_eq!(
        map_jev_upstream_path("/jev/v1/requests/dec_123").as_deref(),
        Some("/api/v1/requests/dec_123/")
    );
    assert_eq!(
        map_jev_upstream_path("/jev/v1/requests/dec_123/cancel").as_deref(),
        Some("/api/v1/requests/dec_123/cancel/")
    );
    assert_eq!(map_jev_upstream_path("/jev/v1/chat/completions"), None);
}

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};
use toolcase_gateway::config::{Config, ConfigStore};
use toolcase_gateway::http::{header_value, read_request, Request};

fn jev_config(upstream_port: u16, keys: &[&str]) -> Config {
    Config {
        target_host: "127.0.0.1".into(),
        target_port: 1,
        fallbacks: vec![],
        io_timeout: Some(Duration::from_secs(3)),
        retry_base_delay_ms: 0,
        max_retry_delay_ms: 0,
        default_model: "unrelated-model".into(),
        museai_base_url: String::new(),
        museai_cookie: String::new(),
        museai_ws_url: String::new(),
        museai_access_token: String::new(),
        museai_notary_token: String::new(),
        museai_vm_id: String::new(),
        museai_auto_cleanup_threads: false,
        museai_thread_retention_secs: 0,
        jev_api_keys: keys.iter().map(|key| (*key).to_string()).collect(),
        jev_base_url: format!("http://127.0.0.1:{upstream_port}"),
    }
}

fn round_trip(
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    keys: &[&str],
    statuses: &[u16],
) -> (String, Vec<Request>) {
    let upstream_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    upstream_listener.set_nonblocking(true).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();
    let statuses = statuses.to_vec();
    let upstream = thread::spawn(move || {
        let mut requests = Vec::new();
        for status in statuses {
            let deadline = Instant::now() + Duration::from_secs(3);
            let (mut socket, _) = loop {
                match upstream_listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "gateway did not reach Jev upstream"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("upstream accept failed: {error}"),
                }
            };
            requests.push(read_request(&mut socket).unwrap());
            let reason = match status {
                200 => "OK",
                400 => "Bad Request",
                401 => "Unauthorized",
                409 => "Conflict",
                429 => "Too Many Requests",
                _ => "Error",
            };
            let response = format!("{{\"upstream_status\":{status}}}");
            write!(
                socket,
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .unwrap();
        }
        requests
    });

    let gateway_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let gateway_port = gateway_listener.local_addr().unwrap().port();
    let store = ConfigStore::new(jev_config(upstream_port, keys), ".env".into());
    let gateway = thread::spawn(move || {
        let (socket, _) = gateway_listener.accept().unwrap();
        toolcase_gateway::routes::route_request(socket, &store).unwrap();
    });
    let mut client = TcpStream::connect(("127.0.0.1", gateway_port)).unwrap();
    write!(
        client,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    )
    .unwrap();
    for (name, value) in headers {
        write!(client, "{name}: {value}\r\n").unwrap();
    }
    client.write_all(b"\r\n").unwrap();
    client.write_all(body).unwrap();
    let mut raw_response = String::new();
    client.read_to_string(&mut raw_response).unwrap();
    gateway.join().unwrap();
    let requests = upstream.join().unwrap();
    (raw_response, requests)
}

#[test]
fn decision_request_forwards_body_idempotency_and_configured_key() {
    let body = br#"{"model":"jev-1.13","state":"damaged","questions":{"refund":{"type":"noul","instructions":"Refund?"}}}"#;
    let (response, requests) = round_trip(
        "POST",
        "/jev/v1/systemone",
        &[
            ("Idempotency-Key", "ticket-8814-20261010"),
            ("Authorization", "Bearer ignored"),
        ],
        body,
        &["synthetic-key"],
        &[200],
    );
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response:?}");
    assert!(response.contains("\r\n\r\n{\"upstream_status\":200}"));
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/api/v1/systemone/");
    assert_eq!(requests[0].body, body);
    assert_eq!(
        header_value(&requests[0].headers, "authorization"),
        Some("Bearer synthetic-key")
    );
    assert_eq!(
        header_value(&requests[0].headers, "idempotency-key"),
        Some("ticket-8814-20261010")
    );
    assert_eq!(
        header_value(&requests[0].headers, "content-type"),
        Some("application/json")
    );
}

#[test]
fn status_lookup_and_cancel_use_documented_methods_for_both_prefixes() {
    for prefix in ["jev", "jev-ai"] {
        for (method, suffix, upstream_path) in [
            ("GET", "requests/dec_123", "/api/v1/requests/dec_123/"),
            (
                "POST",
                "requests/dec_123/cancel",
                "/api/v1/requests/dec_123/cancel/",
            ),
        ] {
            let path = format!("/{prefix}/v1/{suffix}");
            let (response, requests) =
                round_trip(method, &path, &[], &[], &["synthetic-key"], &[200]);
            assert!(
                response.starts_with("HTTP/1.1 200 OK\r\n"),
                "{path}: {response:?}"
            );
            assert_eq!(requests[0].method, method);
            assert_eq!(requests[0].path, upstream_path);
        }
    }
}

#[test]
fn models_catalog_uses_inbound_api_key_and_never_returns_fabricated_data() {
    let (response, requests) = round_trip(
        "GET",
        "/jev/v1/models",
        &[("x-api-key", "synthetic-key")],
        &[],
        &[],
        &[401],
    );
    assert!(
        response.starts_with("HTTP/1.1 401 Unauthorized\r\n"),
        "{response:?}"
    );
    assert!(response.contains("\r\n\r\n{\"upstream_status\":401}"));
    assert_eq!(requests[0].path, "/api/v1/models/");
    assert_eq!(
        header_value(&requests[0].headers, "authorization"),
        Some("Bearer synthetic-key")
    );
}

#[test]
fn account_failover_reuses_exact_idempotency_key_and_body() {
    let body =
        br#"{"state":"damaged","questions":{"refund":{"type":"noul","instructions":"Refund?"}}}"#;
    let (response, requests) = round_trip(
        "POST",
        "/jev/v1/systemone",
        &[("Idempotency-Key", "ticket-8814-20261010")],
        body,
        &["first-key", "second-key"],
        &[429, 200],
    );
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response:?}");
    assert_eq!(requests.len(), 2);
    assert_eq!(
        header_value(&requests[0].headers, "authorization"),
        Some("Bearer first-key")
    );
    assert_eq!(
        header_value(&requests[1].headers, "authorization"),
        Some("Bearer second-key")
    );
    for request in requests {
        assert_eq!(request.body, body);
        assert_eq!(
            header_value(&request.headers, "idempotency-key"),
            Some("ticket-8814-20261010")
        );
    }
}

#[test]
fn validation_errors_pass_through_without_failing_over() {
    for status in [400, 409] {
        let (response, requests) = round_trip(
            "POST",
            "/jev/v1/systemone",
            &[],
            b"{}",
            &["first-key", "second-key"],
            &[status],
        );
        assert!(
            response.starts_with(&format!("HTTP/1.1 {status} ")),
            "{response:?}"
        );
        assert!(response.contains("\r\n\r\n"), "{response:?}");
        assert!(response.ends_with(&format!("{{\"upstream_status\":{status}}}")));
        assert_eq!(requests.len(), 1);
    }
}

#[test]
fn unrecognized_jev_routes_never_submit_a_decision() {
    for path in [
        "/jev/v1/chat/completions",
        "/jev-ai/v1/unknown",
        "/jev/v1/requests/dec_123/other",
    ] {
        let (response, requests) = round_trip("POST", path, &[], b"{}", &["synthetic-key"], &[]);
        assert!(
            response.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"),
            "{path}: {response:?}"
        );
        assert!(requests.is_empty());
    }
}
