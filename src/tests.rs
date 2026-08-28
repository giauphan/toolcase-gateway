use crate::config::Config;
use crate::http::{
    header_value, is_chunked, is_request_target, is_token, parse_headers, read_more,
    read_response_head, Request,
};
use crate::rewrite::{escape_json_string, replace_model, rewrite_tool_names};
use crate::routing::{open_upstream, RETRYABLE};
use std::io::{Read, Write};
use std::net::TcpListener;
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
    assert!(RETRYABLE.contains(&403));
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
    };
    let mut socket = open_upstream(&request, &config, "m").unwrap();
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
