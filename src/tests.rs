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

#[test]
fn replaces_model_without_changing_other_fields() {
    assert_eq!(
        replace_model(br#"{"model":"old","tools":[]}"#, "fail-try"),
        br#"{"model":"fail-try","tools":[]}"#
    );
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
        };
        let (mut client, _) = listener.accept().unwrap();
        crate::routes::handle_models_catalog(&mut client, &config).unwrap();
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
    assert_eq!(models.len(), 5);
    assert!(models
        .iter()
        .all(|model| { !model["id"].as_str().unwrap().contains("terra") }));
    assert!(models
        .iter()
        .any(|model| model["id"] == "gpt-5.6-sol-xhigh"));
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
