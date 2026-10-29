use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use toolcase_gateway::config::{Config, ConfigStore};
use toolcase_gateway::http::{header_value, read_request};
use toolcase_gateway::http_provider::CandidateKind;
use toolcase_gateway::jev::handlers::{expected_jev_method, map_jev_upstream_path};
use toolcase_gateway::jev::pool::JevAccountPool;
use toolcase_gateway::providers::jev::accounts::OpenRouterAccount;

#[test]
fn test_jev_decision_route_aliases_resolve_properly() {
    let routes = [
        "/jev/v1/systemone",
        "/jev-ai/v1/systemone",
        "/jev/v1/decisions",
        "/jev-ai/v1/decisions",
        "/jev/v1/decision",
        "/jev-ai/v1/decision",
        "/jev/decisions",
        "/jev/decision",
    ];

    for r in routes {
        let upstream = map_jev_upstream_path(r).unwrap_or_else(|| panic!("failed to map {r}"));
        assert_eq!(upstream, "/api/v1/systemone/");
        let method = expected_jev_method(r).unwrap_or_else(|| panic!("failed method {r}"));
        assert_eq!(method, "POST");
    }
}

#[test]
fn test_jev_round_robin_on_subsequent_decision_requests_after_failure() {
    let direct_keys = vec!["direct-key-1".to_string(), "direct-key-2".to_string()];
    let or_accounts = vec![
        OpenRouterAccount {
            api_key: "sk-or-v1-key-a".to_string(),
            cookie: "".to_string(),
            label: "or-key-a".to_string(),
        },
        OpenRouterAccount {
            api_key: "sk-or-v1-key-b".to_string(),
            cookie: "".to_string(),
            label: "or-key-b".to_string(),
        },
        OpenRouterAccount {
            api_key: "".to_string(),
            cookie: "or_cookie_session_1=abc".to_string(),
            label: "or-browser-1".to_string(),
        },
        OpenRouterAccount {
            api_key: "".to_string(),
            cookie: "or_cookie_session_2=xyz".to_string(),
            label: "or-browser-2".to_string(),
        },
    ];

    let pool = JevAccountPool::new_with_all(
        toolcase_gateway::config::parse_jev_accounts("direct-key-1, direct-key-2"),
        or_accounts,
        300,
    );

    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 1,
        jev_api_keys: direct_keys,
        jev_openrouter_enabled: true,
        jev_credit_cooldown_secs: 300,
        jev_openrouter_fallback_models: vec!["anthropic/claude-3-haiku".to_string()],
        ..Config::default()
    };

    // 1. Initial round-robin when both direct accounts are healthy
    let t1 = pool.candidate_failover_targets(&config);
    assert_eq!(t1[0].api_key, "direct-key-1");
    assert_eq!(t1[1].api_key, "direct-key-2");

    let t2 = pool.candidate_failover_targets(&config);
    assert_eq!(t2[0].api_key, "direct-key-2");
    assert_eq!(t2[1].api_key, "direct-key-1");

    // 2. Direct account 1 fails with credit exhaustion -> demoted behind healthy accounts
    pool.mark_credit_exhausted("direct-key-1");
    assert!(pool.is_credit_exhausted("direct-key-1"));
    assert!(!pool.is_credit_exhausted("direct-key-2"));

    // Request after failure starts immediately with direct-key-2
    let t_after1 = pool.candidate_failover_targets(&config);
    assert_eq!(t_after1[0].api_key, "direct-key-2");
    // OpenRouter tier follows healthy direct
    assert_eq!(t_after1[1].api_key, "sk-or-v1-key-a");
    assert_eq!(t_after1[1].kind, CandidateKind::OpenRouterApiKey);
    assert_eq!(t_after1[1].model, None); // default JEV model

    // 3. When direct-key-2 also exhausts, all direct accounts are demoted during cooldown
    pool.mark_credit_exhausted("direct-key-2");
    assert_eq!(pool.exhausted_accounts_count(), 2);

    // Request after all direct accounts fail starts immediately with OpenRouter API keys in round-robin
    let t_or1 = pool.candidate_failover_targets(&config);
    assert_eq!(t_or1[0].api_key, "sk-or-v1-key-b");
    assert_eq!(t_or1[0].kind, CandidateKind::OpenRouterApiKey);
    assert_eq!(t_or1[0].model, None); // Tier 2: JEV openrouter
    assert_eq!(t_or1[1].api_key, "sk-or-v1-key-a");

    // Tier 3: Different decision models
    assert_eq!(t_or1[2].api_key, "sk-or-v1-key-b");
    assert_eq!(t_or1[2].model, Some("anthropic/claude-3-haiku".to_string()));
    assert_eq!(t_or1[3].api_key, "sk-or-v1-key-a");
    assert_eq!(t_or1[3].model, Some("anthropic/claude-3-haiku".to_string()));

    // Tier 4: Browser session cookies
    assert_eq!(t_or1[4].api_key, "or_cookie_session_2=xyz");
    assert_eq!(t_or1[4].kind, CandidateKind::OpenRouterBrowser);
    assert_eq!(t_or1[5].api_key, "or_cookie_session_1=abc");

    // Next request to decision path ("after") round-robins OpenRouter API keys and browser sessions!
    let t_or2 = pool.candidate_failover_targets(&config);
    assert_eq!(t_or2[0].api_key, "sk-or-v1-key-a"); // Rotated!
    assert_eq!(t_or2[1].api_key, "sk-or-v1-key-b");
    // Browser sessions also rotated!
    assert_eq!(t_or2[4].api_key, "or_cookie_session_1=abc"); // Rotated!
    assert_eq!(t_or2[5].api_key, "or_cookie_session_2=xyz");

    // 4. OpenRouter API key exhaustion demotes the key behind healthy keys and to browser sessions
    pool.mark_credit_exhausted("sk-or-v1-key-a");
    assert!(pool.is_credit_exhausted("sk-or-v1-key-a"));
    let t_or_demoted = pool.candidate_failover_targets(&config);
    // Only healthy key-b remains in Tier 2
    assert_eq!(t_or_demoted[0].api_key, "sk-or-v1-key-b");
    assert_eq!(t_or_demoted[1].api_key, "sk-or-v1-key-b"); // Tier 3 different decision
    assert_eq!(t_or_demoted[2].kind, CandidateKind::OpenRouterBrowser); // Tier 4 follows immediately!

    // 5. Reset/cooldown expiration restores direct keys to priority
    pool.reset_credit_exhausted();
    let t_restored = pool.candidate_failover_targets(&config);
    assert!(t_restored[0].api_key.starts_with("direct-key"));
    assert!(t_restored[1].api_key.starts_with("direct-key"));
}

#[test]
fn test_jev_decision_failover_cascade_direct_to_openrouter_to_alt_to_browser() {
    let direct_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let direct_port = direct_listener.local_addr().unwrap().port();
    direct_listener.set_nonblocking(true).unwrap();

    let openrouter_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let openrouter_port = openrouter_listener.local_addr().unwrap().port();
    openrouter_listener.set_nonblocking(true).unwrap();

    // 1. Mock Direct Jev upstream (returns 402 Payment Required)
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

    // 2. Mock OpenRouter upstream receives cascade:
    //    Req 1: Jev model with API key -> returns 429
    //    Req 2: Different decision model with API key -> returns 402
    //    Req 3: OpenRouter browser cookie -> returns 200 OK!
    let openrouter_upstream = thread::spawn(move || {
        let mut reqs = Vec::new();

        // Sub-step 1: OpenRouter with JEV model -> 429
        {
            let deadline = Instant::now() + Duration::from_secs(3);
            let (mut socket, _) = loop {
                match openrouter_listener.accept() {
                    Ok(conn) => break conn,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "openrouter 1 timeout");
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("openrouter accept failed: {e}"),
                }
            };
            reqs.push(read_request(&mut socket).unwrap());
            let body = "{\"error\":\"rate_limited\"}";
            write!(
                socket,
                "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ).unwrap();
        }

        // Sub-step 2: OpenRouter with different decision model -> 402
        {
            let deadline = Instant::now() + Duration::from_secs(3);
            let (mut socket, _) = loop {
                match openrouter_listener.accept() {
                    Ok(conn) => break conn,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "openrouter 2 timeout");
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("openrouter accept failed: {e}"),
                }
            };
            reqs.push(read_request(&mut socket).unwrap());
            let body = "{\"error\":\"insufficient_credits\"}";
            write!(
                socket,
                "HTTP/1.1 402 Payment Required\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ).unwrap();
        }

        // Sub-step 3: OpenRouter browser cookie -> 200 OK
        {
            let deadline = Instant::now() + Duration::from_secs(3);
            let (mut socket, _) = loop {
                match openrouter_listener.accept() {
                    Ok(conn) => break conn,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "openrouter 3 timeout");
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("openrouter accept failed: {e}"),
                }
            };
            reqs.push(read_request(&mut socket).unwrap());
            let body = "{\"decision\":{\"confidence\":0.98,\"approved\":true}}";
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
        jev_api_keys: vec!["direct-sk-fail".into()],
        jev_base_url: format!("http://127.0.0.1:{direct_port}"),
        jev_openrouter_base_url: format!("http://127.0.0.1:{openrouter_port}"),
        jev_openrouter_api_key: "sk-or-v1-failover-key".into(),
        jev_openrouter_cookie: "or_browser_session_cookie=valid".into(),
        jev_openrouter_model: "typesafe/jev-1.13".into(),
        jev_openrouter_fallback_models: vec!["anthropic/claude-3-haiku".into()],
        jev_openrouter_enabled: true,
        jev_credit_cooldown_secs: 300,
        ..Config::default()
    };

    let store = ConfigStore::new(config, ".env.test_jev".into());

    let gateway_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let gateway_port = gateway_listener.local_addr().unwrap().port();

    let store_clone = store.clone();
    let gateway = thread::spawn(move || {
        let (socket, _) = gateway_listener.accept().unwrap();
        toolcase_gateway::routes::route_request(socket, &store_clone).unwrap();
    });

    let payload = br#"{"state":{"task":"verify cascade"},"questions":{"check":{"type":"noul"}}}"#;
    let mut client = TcpStream::connect(("127.0.0.1", gateway_port)).unwrap();
    write!(
        client,
        "POST /jev/v1/decisions HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    ).unwrap();
    client.write_all(payload).unwrap();

    let mut resp = String::new();
    client.read_to_string(&mut resp).unwrap();
    assert!(resp.starts_with("HTTP/1.1 200 OK\r\n"), "Failed: {resp}");
    assert!(resp.contains("\"approved\":true"));

    let direct_req = direct_upstream.join().unwrap();
    assert_eq!(direct_req.path, "/api/v1/systemone/");
    assert_eq!(
        header_value(&direct_req.headers, "authorization"),
        Some("Bearer direct-sk-fail")
    );

    let or_reqs = openrouter_upstream.join().unwrap();
    assert_eq!(or_reqs.len(), 3);

    // Check step 1: Jev OpenRouter with API key
    assert_eq!(
        header_value(&or_reqs[0].headers, "authorization"),
        Some("Bearer sk-or-v1-failover-key")
    );
    let body0: serde_json::Value = serde_json::from_slice(&or_reqs[0].body).unwrap();
    assert_eq!(body0["model"], "typesafe/jev-1.13");

    // Check step 2: OpenRouter with different decision model
    assert_eq!(
        header_value(&or_reqs[1].headers, "authorization"),
        Some("Bearer sk-or-v1-failover-key")
    );
    let body1: serde_json::Value = serde_json::from_slice(&or_reqs[1].body).unwrap();
    assert_eq!(body1["model"], "anthropic/claude-3-haiku");

    // Check step 3: OpenRouter browser cookie session
    assert_eq!(
        header_value(&or_reqs[2].headers, "cookie"),
        Some("or_browser_session_cookie=valid")
    );
    let body2: serde_json::Value = serde_json::from_slice(&or_reqs[2].body).unwrap();
    assert_eq!(body2["model"], "typesafe/jev-1.13");

    gateway.join().ok();
}

#[test]
fn test_step3_alternate_decision_models_default_and_round_robin() {
    let direct_keys = vec![];
    let or_accounts = vec![
        OpenRouterAccount {
            api_key: "sk-or-v1-key-1".to_string(),
            cookie: "".to_string(),
            label: "or-account-1".to_string(),
        },
        OpenRouterAccount {
            api_key: "sk-or-v1-key-2".to_string(),
            cookie: "".to_string(),
            label: "or-account-2".to_string(),
        },
    ];

    let pool = JevAccountPool::new_with_all(vec![], or_accounts, 300);

    // Empty fallback models -> should default to baseten/mammoth-1:free and nace-ai/drex-max
    let config = Config {
        jev_api_keys: direct_keys,
        jev_openrouter_enabled: true,
        jev_credit_cooldown_secs: 300,
        jev_openrouter_fallback_models: vec![],
        ..Config::default()
    };

    // Request 1: Primary in Step 3 should be baseten/mammoth-1:free, followed by nace-ai/drex-max
    let t1 = pool.candidate_failover_targets(&config);
    // Candidates layout:
    // [0..2]: Tier 2 (OpenRouter Jev model)
    // [2..6]: Tier 3 (Different decision models)
    let alt_candidates1: Vec<_> = t1
        .iter()
        .filter(|c| c.kind == CandidateKind::OpenRouterApiKey && c.model.is_some())
        .collect();
    assert_eq!(alt_candidates1.len(), 4); // 2 models x 2 keys
    assert_eq!(
        alt_candidates1[0].model,
        Some("baseten/mammoth-1:free".to_string())
    );
    assert_eq!(
        alt_candidates1[1].model,
        Some("baseten/mammoth-1:free".to_string())
    );
    assert_eq!(
        alt_candidates1[2].model,
        Some("nace-ai/drex-max".to_string())
    );
    assert_eq!(
        alt_candidates1[3].model,
        Some("nace-ai/drex-max".to_string())
    );

    // Request 2: Round-robin rotation puts nace-ai/drex-max first, then baseten/mammoth-1:free
    let t2 = pool.candidate_failover_targets(&config);
    let alt_candidates2: Vec<_> = t2
        .iter()
        .filter(|c| c.kind == CandidateKind::OpenRouterApiKey && c.model.is_some())
        .collect();
    assert_eq!(alt_candidates2.len(), 4);
    assert_eq!(
        alt_candidates2[0].model,
        Some("nace-ai/drex-max".to_string())
    );
    assert_eq!(
        alt_candidates2[1].model,
        Some("nace-ai/drex-max".to_string())
    );
    assert_eq!(
        alt_candidates2[2].model,
        Some("baseten/mammoth-1:free".to_string())
    );
    assert_eq!(
        alt_candidates2[3].model,
        Some("baseten/mammoth-1:free".to_string())
    );

    // Request 3: Rotates back to baseten/mammoth-1:free
    let t3 = pool.candidate_failover_targets(&config);
    let alt_candidates3: Vec<_> = t3
        .iter()
        .filter(|c| c.kind == CandidateKind::OpenRouterApiKey && c.model.is_some())
        .collect();
    assert_eq!(
        alt_candidates3[0].model,
        Some("baseten/mammoth-1:free".to_string())
    );
    assert_eq!(
        alt_candidates3[2].model,
        Some("nace-ai/drex-max".to_string())
    );
}

#[test]
fn test_step3_custom_fallback_models_multi_round_robin() {
    let or_accounts = vec![OpenRouterAccount {
        api_key: "sk-or-v1-single-key".to_string(),
        cookie: "".to_string(),
        label: "or-account".to_string(),
    }];

    let pool = JevAccountPool::new_with_all(vec![], or_accounts, 300);

    let config = Config {
        jev_openrouter_enabled: true,
        jev_openrouter_fallback_models: vec![
            "model-alpha".to_string(),
            "model-beta".to_string(),
            "model-gamma".to_string(),
        ],
        ..Config::default()
    };

    let rot1 = pool.rotated_fallback_models(&config);
    assert_eq!(rot1, vec!["model-alpha", "model-beta", "model-gamma"]);

    let rot2 = pool.rotated_fallback_models(&config);
    assert_eq!(rot2, vec!["model-beta", "model-gamma", "model-alpha"]);

    let rot3 = pool.rotated_fallback_models(&config);
    assert_eq!(rot3, vec!["model-gamma", "model-alpha", "model-beta"]);

    let rot4 = pool.rotated_fallback_models(&config);
    assert_eq!(rot4, vec!["model-alpha", "model-beta", "model-gamma"]);
}

#[test]
fn test_step3_round_robin_cascade_with_default_models() {
    let openrouter_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let openrouter_port = openrouter_listener.local_addr().unwrap().port();

    let openrouter_upstream = thread::spawn(move || {
        let mut reqs = Vec::new();
        // Request 1: OpenRouter JEV fails (429) -> Step 3 tries baseten/mammoth-1:free (200 OK)
        for i in 0..2 {
            let (mut socket, _) = openrouter_listener.accept().unwrap();
            let req = read_request(&mut socket).unwrap();
            reqs.push(req);
            if i == 0 {
                let body = "{\"error\":{\"message\":\"Rate limited\"}}";
                write!(
                    socket,
                    "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                ).unwrap();
            } else {
                let body = "{\"decision\":{\"confidence\":0.99,\"approved\":true}}";
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                ).unwrap();
            }
        }

        // Request 2: OpenRouter JEV fails (429) -> Step 3 tries nace-ai/drex-max (200 OK)
        for i in 0..2 {
            let (mut socket, _) = openrouter_listener.accept().unwrap();
            let req = read_request(&mut socket).unwrap();
            reqs.push(req);
            if i == 0 {
                let body = "{\"error\":{\"message\":\"Rate limited\"}}";
                write!(
                    socket,
                    "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                ).unwrap();
            } else {
                let body = "{\"decision\":{\"confidence\":0.99,\"approved\":true}}";
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                ).unwrap();
            }
        }
        reqs
    });

    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 1,
        io_timeout: Some(Duration::from_secs(3)),
        retry_base_delay_ms: 0,
        max_retry_delay_ms: 0,
        jev_openrouter_base_url: format!("http://127.0.0.1:{openrouter_port}"),
        jev_openrouter_api_key: "sk-or-v1-step3-key".into(),
        jev_openrouter_model: "typesafe/jev-1.13".into(),
        jev_openrouter_fallback_models: vec![], // use default 2 models
        jev_openrouter_enabled: true,
        jev_credit_cooldown_secs: 300,
        ..Config::default()
    };

    let store = ConfigStore::new(config, ".env.test_jev".into());

    let gateway_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let gateway_port = gateway_listener.local_addr().unwrap().port();

    let store_clone1 = store.clone();
    let store_clone2 = store.clone();
    let gateway = thread::spawn(move || {
        let (socket1, _) = gateway_listener.accept().unwrap();
        toolcase_gateway::routes::route_request(socket1, &store_clone1).unwrap();
        let (socket2, _) = gateway_listener.accept().unwrap();
        toolcase_gateway::routes::route_request(socket2, &store_clone2).unwrap();
    });

    let payload =
        br#"{"state":{"task":"verify step 3 round robin"},"questions":{"eval":{"type":"noul"}}}"#;

    // Send Request 1
    let mut client1 = TcpStream::connect(("127.0.0.1", gateway_port)).unwrap();
    write!(
        client1,
        "POST /jev/v1/decisions HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    ).unwrap();
    client1.write_all(payload).unwrap();
    let mut resp1 = String::new();
    client1.read_to_string(&mut resp1).unwrap();
    assert!(resp1.starts_with("HTTP/1.1 200 OK\r\n"));

    // Send Request 2
    let mut client2 = TcpStream::connect(("127.0.0.1", gateway_port)).unwrap();
    write!(
        client2,
        "POST /jev/v1/decisions HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    ).unwrap();
    client2.write_all(payload).unwrap();
    let mut resp2 = String::new();
    client2.read_to_string(&mut resp2).unwrap();
    assert!(resp2.starts_with("HTTP/1.1 200 OK\r\n"));

    let or_reqs = openrouter_upstream.join().unwrap();
    assert_eq!(or_reqs.len(), 4);

    // Request 1:
    // req 0 is typesafe/jev-1.13
    let body0: serde_json::Value = serde_json::from_slice(&or_reqs[0].body).unwrap();
    assert_eq!(body0["model"], "typesafe/jev-1.13");
    // req 1 is baseten/mammoth-1:free
    let body1: serde_json::Value = serde_json::from_slice(&or_reqs[1].body).unwrap();
    assert_eq!(body1["model"], "baseten/mammoth-1:free");

    // Request 2:
    // req 2 is typesafe/jev-1.13
    let body2: serde_json::Value = serde_json::from_slice(&or_reqs[2].body).unwrap();
    assert_eq!(body2["model"], "typesafe/jev-1.13");
    // req 3 is nace-ai/drex-max (ROTATED!)
    let body3: serde_json::Value = serde_json::from_slice(&or_reqs[3].body).unwrap();
    assert_eq!(body3["model"], "nace-ai/drex-max");

    gateway.join().ok();
}
