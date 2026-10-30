//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;

// ---------- OmniRoute caching, service worker, manifest, and module state tests ----------

#[test]
fn test_omniroute_304_preserves_etag_and_omits_body() {
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = upstream_port;
    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let gateway_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = gateway_listener.local_addr().unwrap().port();

    let gateway = thread::spawn(move || {
        let (client, _) = gateway_listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });

    let upstream = thread::spawn(move || {
        let (mut socket, _) = upstream_listener.accept().unwrap();
        let request = toolcase_gateway::http::read_request(&mut socket).unwrap();
        assert_eq!(request.path, "/sw.js?v=1792441366504");
        write!(
            socket,
            "HTTP/1.1 304 Not Modified\r\nETag: \"sw-v1792441366504\"\r\nCache-Control: public, max-age=0, must-revalidate\r\nContent-Type: application/javascript\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        socket.flush().unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    write!(
        client,
        "GET /omniroute/sw.js?v=1792441366504 HTTP/1.1\r\nHost: 127.0.0.1:{client_port}\r\nIf-None-Match: \"sw-v1792441366504\"\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();
    let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 304);
    assert_eq!(
        toolcase_gateway::http::header_value(&head.headers, "etag"),
        Some("\"sw-v1792441366504\"")
    );
    assert_eq!(
        toolcase_gateway::http::header_value(&head.headers, "service-worker-allowed"),
        Some("/omniroute/")
    );
    assert_eq!(
        toolcase_gateway::http::header_value(&head.headers, "content-length"),
        None
    );
    assert_eq!(
        toolcase_gateway::http::header_value(&head.headers, "transfer-encoding"),
        None
    );
    assert!(head.buffered_body.is_empty());

    let mut remaining = Vec::new();
    std::io::Read::read_to_end(&mut client, &mut remaining).unwrap();
    assert!(remaining.is_empty());

    gateway.join().unwrap();
    upstream.join().unwrap();
}

#[test]
fn test_sw_js_with_query_proxies_to_omniroute() {
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = upstream_port;
    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let gateway_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = gateway_listener.local_addr().unwrap().port();

    let gateway = thread::spawn(move || {
        let (client, _) = gateway_listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });

    let upstream = thread::spawn(move || {
        let (mut socket, _) = upstream_listener.accept().unwrap();
        let request = toolcase_gateway::http::read_request(&mut socket).unwrap();
        assert_eq!(request.path, "/sw.js?v=1792441366504");
        let script = r#"self.addEventListener('fetch', () => {});"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{script}",
            script.len()
        )
        .unwrap();
        socket.flush().unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    write!(
        client,
        "GET /sw.js?v=1792441366504 HTTP/1.1\r\nHost: 127.0.0.1:{client_port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();
    let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    assert_eq!(
        toolcase_gateway::http::header_value(&head.headers, "service-worker-allowed"),
        Some("/omniroute/")
    );
    let mut body = head.buffered_body.clone();
    std::io::Read::read_to_end(&mut client, &mut body).unwrap();
    assert_eq!(
        String::from_utf8(body).unwrap(),
        r#"self.addEventListener('fetch', () => {});"#
    );

    gateway.join().unwrap();
    upstream.join().unwrap();
}

#[test]
fn test_manifest_webmanifest_rewritten_under_prefix() {
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = upstream_port;
    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let gateway_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = gateway_listener.local_addr().unwrap().port();

    let gateway = thread::spawn(move || {
        let (client, _) = gateway_listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });

    let upstream = thread::spawn(move || {
        let (mut socket, _) = upstream_listener.accept().unwrap();
        let request = toolcase_gateway::http::read_request(&mut socket).unwrap();
        assert_eq!(request.path, "/manifest.webmanifest");
        let manifest =
            r#"{"start_url":"/dashboard","scope":"/","icons":[{"src":"/icon-192.png"}]}"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: application/manifest+json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{manifest}",
            manifest.len()
        )
        .unwrap();
        socket.flush().unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    write!(
        client,
        "GET /omniroute/manifest.webmanifest HTTP/1.1\r\nHost: 127.0.0.1:{client_port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();
    let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    let mut body = head.buffered_body.clone();
    std::io::Read::read_to_end(&mut client, &mut body).unwrap();
    let body_str = String::from_utf8(body).unwrap();
    assert!(body_str.contains(r#""start_url":"/omniroute/dashboard""#));
    assert!(body_str.contains(r#""src":"/omniroute/icon-192.png""#));

    gateway.join().unwrap();
    upstream.join().unwrap();
}

#[test]
fn test_sw_js_returns_empty_fallback_when_omniroute_disabled() {
    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = 20128;
    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let _ = store.module_registry().write().map(|mut reg| {
        reg.set_module_status(
            "omniroute",
            toolcase_gateway::gateway::modules::ModuleStatus::Disabled,
        )
    });

    let gateway_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = gateway_listener.local_addr().unwrap().port();

    let gateway = thread::spawn(move || {
        let (client, _) = gateway_listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    write!(
        client,
        "GET /sw.js?v=1792441366504 HTTP/1.1\r\nHost: 127.0.0.1:{client_port}\r\nReferer: http://127.0.0.1:{client_port}/omniroute/dashboard\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();
    let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    assert_eq!(
        toolcase_gateway::http::header_value(&head.headers, "content-type"),
        Some("application/javascript")
    );
    assert_eq!(
        toolcase_gateway::http::header_value(&head.headers, "content-length"),
        Some("0")
    );
    assert_eq!(
        toolcase_gateway::http::header_value(&head.headers, "service-worker-allowed"),
        None
    );

    gateway.join().unwrap();
}
