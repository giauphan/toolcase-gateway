use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use toolcase_gateway::config::{Config, ConfigStore};
use toolcase_gateway::http::read_request;
use toolcase_gateway::http_provider::CandidateKind;
use toolcase_gateway::jev::pool::JevAccountPool;
use toolcase_gateway::providers::jev::accounts::OpenRouterAccount;

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
