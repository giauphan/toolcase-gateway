use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};
use toolcase_gateway::config::{Config, ConfigStore};
use toolcase_gateway::http::{header_value, read_request};
use toolcase_gateway::jev::pool::JevAccountPool;

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

    let store = ConfigStore::new(config, ".env.test_jev".into());

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

    let store = ConfigStore::new(config, ".env.test_jev".into());

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
