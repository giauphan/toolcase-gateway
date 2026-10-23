//! Integration tests for the Jev AI multi-account pool and retryable-status logic.

use toolcase_gateway::config::parse_jev_api_keys;
use toolcase_gateway::jev::business::{is_account_retryable, is_retryable, is_transient_retryable};
use toolcase_gateway::jev::handlers::{expected_jev_method, map_jev_upstream_path};
use toolcase_gateway::jev::pool::JevAccountPool;

#[test]
fn parse_jev_api_keys_trims_and_ignores_empty_entries() {
    assert_eq!(
        parse_jev_api_keys(" primary-key, ,secondary-key ,, "),
        vec!["primary-key", "secondary-key"]
    );
}

#[test]
fn parse_jev_accounts_parses_labels_and_generates_fallbacks() {
    let accounts = toolcase_gateway::config::parse_jev_accounts(
        "prod-primary:sk-111, staging=sk-222, sk-333333333333",
    );
    assert_eq!(accounts.len(), 3);
    assert_eq!(accounts[0].label, "prod-primary");
    assert_eq!(accounts[0].api_key, "sk-111");
    assert_eq!(accounts[1].label, "staging");
    assert_eq!(accounts[1].api_key, "sk-222");
    assert_eq!(accounts[2].label, "account-3 (..3333)");
    assert_eq!(accounts[2].api_key, "sk-333333333333");
}

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
    assert_eq!(
        map_jev_upstream_path("/jev/models").as_deref(),
        Some("/api/v1/models/")
    );
    assert_eq!(
        map_jev_upstream_path("/jev-ai/v1/requests/req-abc_123/cancel?dry_run=true").as_deref(),
        Some("/api/v1/requests/req-abc_123/cancel/")
    );
    for path in [
        "/jev/v1/chat/completions",
        "/jev/v1/requests",
        "/jev/v1/requests/",
        "/jev/v1/requests/dec_123/other",
        "/jev/v1/requests/invalid.id",
    ] {
        assert_eq!(map_jev_upstream_path(path), None, "{path}");
        assert_eq!(expected_jev_method(path), None, "{path}");
    }
    assert_eq!(expected_jev_method("/jev/v1/models"), Some("GET"));
    assert_eq!(expected_jev_method("/jev/v1/systemone"), Some("POST"));
    assert_eq!(expected_jev_method("/jev/v1/requests/dec_123"), Some("GET"));
    assert_eq!(
        expected_jev_method("/jev/v1/requests/dec_123/cancel"),
        Some("POST")
    );
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
        ..Config::default()
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

#[test]
fn test_jev_account_pool_openrouter_round_robin_and_failover_targets() {
    let pool = JevAccountPool::new(&["direct-key1".into(), "direct-key2".into()]);
    assert_eq!(pool.len(), 2);
    assert_eq!(pool.openrouter_len(), 0);

    let added =
        pool.add_openrouter_keys(&["cookie-session-a".into(), "team-b:cookie-session-b".into()]);
    assert_eq!(added, 2);
    assert_eq!(pool.openrouter_len(), 2);

    let or_accounts = pool.openrouter_accounts();
    assert_eq!(or_accounts[0].cookie, "cookie-session-a");
    assert_eq!(or_accounts[0].label, "openrouter-1 (..on-a)");
    assert_eq!(or_accounts[1].cookie, "cookie-session-b");
    assert_eq!(or_accounts[1].label, "team-b");

    let config = Config {
        jev_openrouter_enabled: true,
        ..Config::default()
    };

    // Direct accounts are healthy, so direct accounts come first (round-robin), then OpenRouter accounts
    let targets1 = pool.candidate_failover_targets(&config);
    assert_eq!(targets1.len(), 4);
    assert_eq!(targets1[0].api_key, "direct-key1");
    assert_eq!(targets1[1].api_key, "direct-key2");
    assert_eq!(targets1[2].api_key, "cookie-session-a");
    assert_eq!(targets1[3].api_key, "cookie-session-b");

    let targets2 = pool.candidate_failover_targets(&config);
    assert_eq!(targets2[0].api_key, "direct-key2");
    assert_eq!(targets2[1].api_key, "direct-key1");
    assert_eq!(targets2[2].api_key, "cookie-session-b");
    assert_eq!(targets2[3].api_key, "cookie-session-a");

    // Mark direct-key2 as credit exhausted
    pool.mark_credit_exhausted("direct-key2");
    assert!(pool.is_credit_exhausted("direct-key2"));
    assert!(!pool.is_credit_exhausted("direct-key1"));
    assert_eq!(pool.exhausted_accounts_count(), 1);

    // direct-key1 is healthy, direct-key2 is moved behind OpenRouter
    let targets3 = pool.candidate_failover_targets(&config);
    assert_eq!(targets3[0].api_key, "direct-key1");
    assert_eq!(targets3[1].api_key, "cookie-session-a");
    assert_eq!(targets3[2].api_key, "cookie-session-b");
    assert_eq!(targets3[3].api_key, "direct-key2");

    // When all direct keys are credit exhausted, OpenRouter automatically takes priority at the front
    pool.mark_credit_exhausted("direct-key1");
    let targets4 = pool.candidate_failover_targets(&config);
    assert_eq!(targets4[0].api_key, "cookie-session-b");
    assert_eq!(targets4[1].api_key, "cookie-session-a");
    assert!(targets4[2].api_key.starts_with("direct-key"));

    // Reset clears exhaustion
    pool.reset_credit_exhausted();
    assert_eq!(pool.exhausted_accounts_count(), 0);
    assert!(!pool.is_credit_exhausted("direct-key1"));

    // Add API key entries alongside cookies
    let added = pool.add_openrouter_entries(&[(
        "sk-or-v1-dedicated-test-key".to_string(),
        Some("dedicated-key".to_string()),
    )]);
    assert_eq!(added, 1);
    let all_or = pool.openrouter_accounts();
    let key_acc = all_or
        .iter()
        .find(|a| a.label == "dedicated-key")
        .expect("key acc found");
    assert_eq!(key_acc.api_key, "sk-or-v1-dedicated-test-key");
    assert_eq!(key_acc.cookie, "");
    assert_eq!(key_acc.credential(), "sk-or-v1-dedicated-test-key");
}

#[test]
fn test_jev_credit_limit_automatic_failover_to_openrouter_upstream() {
    let direct_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let direct_port = direct_listener.local_addr().unwrap().port();
    direct_listener.set_nonblocking(true).unwrap();

    let openrouter_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let openrouter_port = openrouter_listener.local_addr().unwrap().port();
    openrouter_listener.set_nonblocking(true).unwrap();

    // Spawn mock Direct Jev upstream (returns 402 on first request)
    let direct_upstream = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        let (mut socket, _) = loop {
            match direct_listener.accept() {
                Ok(conn) => break conn,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "direct upstream timeout");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("direct accept failed: {e}"),
            }
        };
        let req = read_request(&mut socket).unwrap();
        let body = "{\"error\":\"Credit limit reached\"}";
        write!(
            socket,
            "HTTP/1.1 402 Payment Required\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ).unwrap();
        req
    });

    // Spawn mock OpenRouter upstream (returns 200 OK on /api/alpha/decisions for both requests)
    let openrouter_upstream = thread::spawn(move || {
        let mut reqs = Vec::new();
        for _ in 0..2 {
            let deadline = Instant::now() + Duration::from_secs(3);
            let (mut socket, _) = loop {
                match openrouter_listener.accept() {
                    Ok(conn) => break conn,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "openrouter upstream timeout");
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("openrouter accept failed: {e}"),
                }
            };
            reqs.push(read_request(&mut socket).unwrap());
            let body = "{\"category\":{\"choice\":\"billing\",\"confidence\":1.0}}";
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ).unwrap();
        }
        reqs
    });

    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 1,
        io_timeout: Some(Duration::from_secs(3)),
        retry_base_delay_ms: 0,
        max_retry_delay_ms: 0,
        jev_api_keys: vec!["direct-sk-123".into()],
        jev_base_url: format!("http://127.0.0.1:{direct_port}"),
        jev_openrouter_base_url: format!("http://127.0.0.1:{openrouter_port}"),
        jev_openrouter_cookie: "or_session_token=test-cookie-123".into(),
        jev_openrouter_model: "typesafe/jev-1.13".into(),
        jev_openrouter_enabled: true,
        jev_credit_cooldown_secs: 300,
        ..Config::default()
    };

    let store = ConfigStore::new(config, ".env".into());

    let gateway_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let gateway_port = gateway_listener.local_addr().unwrap().port();
    let store_clone = store.clone();
    let gateway = thread::spawn(move || {
        for _ in 0..2 {
            let (socket, _) = gateway_listener.accept().unwrap();
            let store_ref = store_clone.clone();
            thread::spawn(move || {
                toolcase_gateway::routes::route_request(socket, &store_ref).unwrap();
            });
        }
    });

    let payload = br#"{"state":{"task":"billing duplicate charge"},"questions":{"category":{"type":"choice"}}}"#;

    // First request: Direct returns 402, fails over to OpenRouter
    let mut client1 = TcpStream::connect(("127.0.0.1", gateway_port)).unwrap();
    write!(
        client1,
        "POST /jev/v1/systemone HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    ).unwrap();
    client1.write_all(payload).unwrap();

    let mut resp1 = String::new();
    client1.read_to_string(&mut resp1).unwrap();
    assert!(
        resp1.starts_with("HTTP/1.1 200 OK\r\n"),
        "First request: {resp1:?}"
    );
    assert!(resp1.contains("\"category\":{\"choice\":\"billing\""));

    // Second request: Direct is credit-exhausted, automatically routes directly to OpenRouter
    let mut client2 = TcpStream::connect(("127.0.0.1", gateway_port)).unwrap();
    write!(
        client2,
        "POST /jev/v1/systemone HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    ).unwrap();
    client2.write_all(payload).unwrap();

    let mut resp2 = String::new();
    client2.read_to_string(&mut resp2).unwrap();
    assert!(
        resp2.starts_with("HTTP/1.1 200 OK\r\n"),
        "Second request: {resp2:?}"
    );
    assert!(resp2.contains("\"category\":{\"choice\":\"billing\""));

    let direct_req = direct_upstream.join().unwrap();
    assert_eq!(direct_req.path, "/api/v1/systemone/");
    assert_eq!(
        header_value(&direct_req.headers, "authorization"),
        Some("Bearer direct-sk-123")
    );

    let or_reqs = openrouter_upstream.join().unwrap();
    assert_eq!(or_reqs.len(), 2);
    for req in &or_reqs {
        assert_eq!(req.path, "/api/alpha/decisions");
        assert_eq!(
            header_value(&req.headers, "cookie"),
            Some("or_session_token=test-cookie-123")
        );
        let parsed_body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(parsed_body["model"], "typesafe/jev-1.13");
        assert_eq!(parsed_body["state"]["task"], "billing duplicate charge");
    }

    gateway.join().ok();
}

#[test]
fn test_jev_openrouter_api_key_bearer_authentication_and_headers() {
    let openrouter_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let openrouter_port = openrouter_listener.local_addr().unwrap().port();
    openrouter_listener.set_nonblocking(true).unwrap();

    let openrouter_upstream = thread::spawn(move || {
        let mut reqs = Vec::new();
        for _ in 0..2 {
            let deadline = Instant::now() + Duration::from_secs(3);
            let (mut socket, _) = loop {
                match openrouter_listener.accept() {
                    Ok(conn) => break conn,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "openrouter upstream timeout");
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("openrouter accept failed: {e}"),
                }
            };
            reqs.push(read_request(&mut socket).unwrap());
            let body = "{\"decision\":{\"confidence\":0.99,\"approved\":true}}";
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ).unwrap();
        }
        reqs
    });

    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 1,
        io_timeout: Some(Duration::from_secs(3)),
        retry_base_delay_ms: 0,
        max_retry_delay_ms: 0,
        jev_api_keys: vec![], // No direct keys
        jev_base_url: "http://127.0.0.1:1".into(),
        jev_openrouter_base_url: format!("http://127.0.0.1:{openrouter_port}"),
        jev_openrouter_api_key: "sk-or-v1-server-configured-key".into(),
        jev_openrouter_model: "typesafe/jev-1.13".into(),
        jev_openrouter_enabled: true,
        jev_credit_cooldown_secs: 300,
        ..Config::default()
    };

    let store = ConfigStore::new(config, ".env".into());

    let gateway_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let gateway_port = gateway_listener.local_addr().unwrap().port();

    let store_clone = store.clone();
    let gateway = thread::spawn(move || {
        for _ in 0..2 {
            let (socket, _) = gateway_listener.accept().unwrap();
            let store_ref = store_clone.clone();
            thread::spawn(move || {
                toolcase_gateway::routes::route_request(socket, &store_ref).unwrap();
            });
        }
    });

    // Request 1: uses configured OpenRouter API key and custom decision model
    let payload1 = br#"{"model":"anthropic/claude-3-haiku","state":{"task":"classify issue"},"questions":{"priority":{"type":"score"}}}"#;
    let mut client1 = TcpStream::connect(("127.0.0.1", gateway_port)).unwrap();
    write!(
        client1,
        "POST /jev/v1/systemone HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload1.len()
    ).unwrap();
    client1.write_all(payload1).unwrap();

    let mut resp1 = String::new();
    client1.read_to_string(&mut resp1).unwrap();
    assert!(
        resp1.starts_with("HTTP/1.1 200 OK\r\n"),
        "Req 1 failed: {resp1}"
    );
    assert!(resp1.contains("\"approved\":true"));

    // Request 2: client passes OpenRouter API key in Authorization header directly
    let payload2 = br#"{"state":{"task":"review order"},"questions":{"ok":{"type":"noul"}}}"#;
    let mut client2 = TcpStream::connect(("127.0.0.1", gateway_port)).unwrap();
    write!(
        client2,
        "POST /jev/v1/systemone HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer sk-or-v1-client-bearer-key\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload2.len()
    ).unwrap();
    client2.write_all(payload2).unwrap();

    let mut resp2 = String::new();
    client2.read_to_string(&mut resp2).unwrap();
    assert!(
        resp2.starts_with("HTTP/1.1 200 OK\r\n"),
        "Req 2 failed: {resp2}"
    );
    assert!(resp2.contains("\"approved\":true"));

    let reqs = openrouter_upstream.join().unwrap();
    assert_eq!(reqs.len(), 2);

    // Verify req 1
    assert_eq!(reqs[0].path, "/api/alpha/decisions");
    assert_eq!(
        header_value(&reqs[0].headers, "authorization"),
        Some("Bearer sk-or-v1-server-configured-key")
    );
    assert_eq!(
        header_value(&reqs[0].headers, "http-referer"),
        Some("https://github.com/giauphan/toolcase-gateway")
    );
    assert_eq!(
        header_value(&reqs[0].headers, "x-title"),
        Some("toolcase-gateway")
    );
    assert_eq!(
        header_value(&reqs[0].headers, "content-type"),
        Some("application/json")
    );
    let body1: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body1["model"], "anthropic/claude-3-haiku");

    // Verify req 2
    assert_eq!(reqs[1].path, "/api/alpha/decisions");
    assert_eq!(
        header_value(&reqs[1].headers, "authorization"),
        Some("Bearer sk-or-v1-client-bearer-key")
    );
    assert_eq!(
        header_value(&reqs[1].headers, "http-referer"),
        Some("https://github.com/giauphan/toolcase-gateway")
    );
    assert_eq!(
        header_value(&reqs[1].headers, "x-title"),
        Some("toolcase-gateway")
    );
    let body2: serde_json::Value = serde_json::from_slice(&reqs[1].body).unwrap();
    assert_eq!(body2["model"], "typesafe/jev-1.13");

    gateway.join().ok();
}
