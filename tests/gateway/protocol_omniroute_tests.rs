//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;

// ---------- OmniRoute failover and candidate model repair tests ----------

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
                    b"HTTP/1.1 503 Upstream Overloaded\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        }
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    let request = b"{\"model\":\"primary-model\",\"messages\":[]}";
    write!(
        client,
        "POST /ominroute/v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
        request.len(),
        std::str::from_utf8(request).unwrap()
    )
    .unwrap();
    client.flush().unwrap();

    let response = read_response_head(&mut client).unwrap();
    assert_eq!(response.status, 503);
    assert_eq!(response.reason, "Upstream Overloaded");
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
        "POST /ominroute/v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
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
fn candidate_models_repairs_unknown_model_when_default_empty() {
    use toolcase_gateway::omniroute::candidate_models;

    let fallbacks: Vec<String> = Vec::new();
    let empty_default = "";

    // When default_model is empty (default configuration), unknown model IDs
    // must still be repaired to a known upstream model (claude-3-5-sonnet),
    // not forwarded verbatim where they 503.
    assert_eq!(
        candidate_models(
            b"{\"model\":\"model-medium-hight\"}",
            &fallbacks,
            0,
            empty_default
        ),
        vec!["claude-3-5-sonnet".to_string()]
    );
    assert_eq!(
        candidate_models(
            b"{\"model\":\"model-only-plan\"}",
            &fallbacks,
            0,
            empty_default
        ),
        vec!["claude-3-5-sonnet".to_string()]
    );

    // Known models are preserved as-is.
    assert_eq!(
        candidate_models(b"{\"model\":\"gpt-4o\"}", &fallbacks, 0, empty_default),
        vec!["gpt-4o".to_string()]
    );
    assert_eq!(
        candidate_models(
            b"{\"model\":\"claude-3-5-haiku\"}",
            &fallbacks,
            0,
            empty_default
        ),
        vec!["claude-3-5-haiku".to_string()]
    );
    assert_eq!(
        candidate_models(
            b"{\"model\":\"claude-3-5-sonnet-20241022\"}",
            &fallbacks,
            0,
            empty_default
        ),
        vec!["claude-3-5-sonnet-20241022".to_string()]
    );

    // Unknown base with recognized effort suffix keeps effort level.
    assert_eq!(
        candidate_models(b"{\"model\":\"weird-high\"}", &fallbacks, 0, empty_default),
        vec!["claude-3-5-sonnet-high".to_string()]
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
        "POST /ominroute/v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
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
        "POST /ominroute/v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
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
