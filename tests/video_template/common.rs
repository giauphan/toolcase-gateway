//! Shared test helpers for video template and media integration tests.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use toolcase_gateway::config::{Config, ConfigStore};
use toolcase_gateway::http::{header_value, read_more, read_response_head};
use toolcase_gateway::routes::route_request;

pub(crate) fn test_config() -> Config {
    Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        museai_base_url: "https://muse.ai".into(),
        ..Config::default()
    }
}

pub(crate) fn request_route_raw(
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    config: Config,
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store = ConfigStore::new(config, std::path::PathBuf::from(".env"));

    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        route_request(client, &store).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut header_lines = String::new();
    for (k, v) in headers {
        header_lines.push_str(&format!("{k}: {v}\r\n"));
    }
    write!(
        client,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\n{header_lines}Connection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    if !body.is_empty() {
        client.write_all(body).unwrap();
    }
    client.flush().unwrap();

    let head = read_response_head(&mut client).unwrap();
    let mut response_body = head.buffered_body;
    if method.eq_ignore_ascii_case("head") {
        response_body.clear();
    } else {
        let length =
            header_value(&head.headers, "content-length").and_then(|val| val.parse::<usize>().ok());
        if let Some(length) = length {
            while response_body.len() < length {
                read_more(&mut client, &mut response_body).unwrap();
            }
            response_body.truncate(length);
        } else {
            let mut tail = Vec::new();
            let _ = client.read_to_end(&mut tail);
            response_body.extend_from_slice(&tail);
        }
    }
    handle.join().unwrap();
    (head.status, head.headers, response_body)
}

pub(crate) fn request_route_json(
    method: &str,
    path: &str,
    body: &str,
    config: Config,
) -> (u16, serde_json::Value) {
    let (status, _headers, raw_body) = request_route_raw(
        method,
        path,
        &[("Content-Type", "application/json")],
        body.as_bytes(),
        config,
    );
    let val: serde_json::Value = serde_json::from_slice(&raw_body).unwrap_or_else(|e| {
        panic!(
            "Failed to parse JSON response: {e}. Raw: {}",
            String::from_utf8_lossy(&raw_body)
        )
    });
    (status, val)
}
