//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;

// ---------- OmniRoute routing, prefix mounting, and redirect tests ----------

#[test]
fn museai_fails_over_to_fallback() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let fallback_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let fallback_port = fallback_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = fallback_port;
    config.fallbacks = vec!["good_model".into()];
    config.retry_base_delay_ms = 1;
    config.max_retry_delay_ms = 1;

    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });

    let fallback_handle = thread::spawn(move || {
        let (mut upstream, _) = fallback_listener.accept().unwrap();
        let head = toolcase_gateway::http::read_request(&mut upstream).unwrap();
        let body_str = String::from_utf8_lossy(&head.body).into_owned();
        assert!(body_str.contains("\"model\":\"gpt-5.6-sol\""));
        assert!(!body_str.contains("\"model\":\"muse\""));

        upstream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .unwrap();
        upstream.flush().unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let req = b"{\"model\":\"muse\",\"messages\":[]}";
    write!(
        client,
        "POST /ominroute/v1/chat/completions HTTP/1.1\r\nHost: loc\r\nContent-Length: {}\r\n\r\n{}",
        req.len(),
        std::str::from_utf8(req).unwrap()
    )
    .unwrap();
    client.flush().unwrap();

    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);

    let body = if is_chunked(&head.headers) {
        toolcase_gateway::http::read_chunked_body(&mut client, head.buffered_body).unwrap()
    } else {
        head.buffered_body
    };
    assert_eq!(body, b"ok");

    handle.join().unwrap();
    fallback_handle.join().unwrap();
}

#[test]
fn test_omniroute_root_redirects_to_dashboard() {
    let client_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = client_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = 20128;

    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let store_clone = store.clone();

    let server_thread = thread::spawn(move || {
        for _ in 0..4 {
            let (client, _) = client_listener.accept().unwrap();
            let _ = toolcase_gateway::routes::route_request(client, &store_clone);
        }
    });

    for (path, expected_loc) in [
        ("/ominroute", "/ominroute/dashboard"),
        ("/ominroute/", "/ominroute/dashboard"),
        ("/omniroute", "/omniroute/dashboard"),
        ("/ominroute/dashbard", "/ominroute/dashboard"),
    ] {
        let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
        write!(
            client,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{client_port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        client.flush().unwrap();

        let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
        assert_eq!(head.status, 302, "Failed for path {path}");
        assert_eq!(
            toolcase_gateway::http::header_value(&head.headers, "location"),
            Some(expected_loc),
            "Location header mismatch for {path}"
        );
    }

    server_thread.join().unwrap();
}

#[test]
fn test_omniroute_upstream_redirect_rewrites_location_header() {
    let client_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = client_listener.local_addr().unwrap().port();
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = upstream_port;
    config.retry_base_delay_ms = 0;
    config.max_retry_delay_ms = 0;

    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let store_clone = store.clone();

    let gateway = thread::spawn(move || {
        for _ in 0..2 {
            let (client, _) = client_listener.accept().unwrap();
            let _ = toolcase_gateway::routes::route_request(client, &store_clone);
        }
    });

    let upstream = thread::spawn(move || {
        // First upstream request receives /dashboard and redirects to /
        let (mut socket, _) = upstream_listener.accept().unwrap();
        let _request = toolcase_gateway::http::read_request(&mut socket).unwrap();
        write!(
            socket,
            "HTTP/1.1 302 Found\r\nLocation: /\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();

        // Second upstream request redirects to /login
        let (mut socket, _) = upstream_listener.accept().unwrap();
        let _request = toolcase_gateway::http::read_request(&mut socket).unwrap();
        write!(
            socket,
            "HTTP/1.1 302 Found\r\nLocation: /login\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
    });

    // Request 1: should rewrite Location: / to /ominroute/dashboard
    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    write!(
        client,
        "GET /ominroute/dashboard HTTP/1.1\r\nHost: 127.0.0.1:{client_port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();
    let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 302);
    assert_eq!(
        toolcase_gateway::http::header_value(&head.headers, "location"),
        Some("/ominroute/dashboard")
    );

    // Request 2: should rewrite Location: /login to /ominroute/login
    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    write!(
        client,
        "GET /ominroute/dashboard HTTP/1.1\r\nHost: 127.0.0.1:{client_port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();
    let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 302);
    assert_eq!(
        toolcase_gateway::http::header_value(&head.headers, "location"),
        Some("/ominroute/login")
    );

    gateway.join().unwrap();
    upstream.join().unwrap();
}

#[test]
fn test_omniroute_rewrite_location_unit_test() {
    use toolcase_gateway::features::proxy::service::rewrite_location_header;

    assert_eq!(
        rewrite_location_header("/", "/ominroute", "127.0.0.1", 20128),
        "/ominroute/dashboard"
    );
    assert_eq!(
        rewrite_location_header("/dashboard", "/ominroute", "127.0.0.1", 20128),
        "/ominroute/dashboard"
    );
    assert_eq!(
        rewrite_location_header("/login", "/ominroute", "127.0.0.1", 20128),
        "/ominroute/login"
    );
    assert_eq!(
        rewrite_location_header("http://127.0.0.1:20128/", "/ominroute", "127.0.0.1", 20128),
        "/ominroute/dashboard"
    );
    assert_eq!(
        rewrite_location_header(
            "http://127.0.0.1:20128/dashboard",
            "/ominroute",
            "127.0.0.1",
            20128
        ),
        "/ominroute/dashboard"
    );
    assert_eq!(
        rewrite_location_header(
            "http://127.0.0.1:20128/login",
            "/ominroute",
            "127.0.0.1",
            20128
        ),
        "/ominroute/login"
    );
    assert_eq!(
        rewrite_location_header(
            "https://auth.example.com/oauth",
            "/ominroute",
            "127.0.0.1",
            20128
        ),
        "https://auth.example.com/oauth"
    );
}

#[test]
fn test_omniroute_html_is_mounted_under_prefix() {
    let client_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = client_listener.local_addr().unwrap().port();
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = upstream_port;
    config.retry_base_delay_ms = 0;
    config.max_retry_delay_ms = 0;

    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let store_clone = store.clone();

    let gateway = thread::spawn(move || {
        let (client, _) = client_listener.accept().unwrap();
        let _ = toolcase_gateway::routes::route_request(client, &store_clone);
    });

    let upstream = thread::spawn(move || {
        let (mut socket, _) = upstream_listener.accept().unwrap();
        let request = toolcase_gateway::http::read_request(&mut socket).unwrap();
        assert_eq!(request.path, "/dashboard/providers");
        let html = r#"<a href="/dashboard/providers"><script src="/_next/a.js"></script>"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
            html.len()
        )
        .unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    write!(
        client,
        "GET /omniroute/dashboard/providers HTTP/1.1\r\nHost: 127.0.0.1:{client_port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();
    let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    let mut body = head.buffered_body.clone();
    std::io::Read::read_to_end(&mut client, &mut body).unwrap();
    let body = String::from_utf8(body).unwrap();
    assert_eq!(
        body,
        r#"<a href="/omniroute/dashboard/providers"><script src="/omniroute/_next/a.js"></script>"#
    );

    gateway.join().unwrap();
    upstream.join().unwrap();
}

#[test]
fn test_omniroute_unprefixed_paths_stay_fail_closed() {
    let client_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = client_listener.local_addr().unwrap().port();
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    upstream_listener.set_nonblocking(true).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();

    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = upstream_port;
    config.retry_base_delay_ms = 0;
    config.max_retry_delay_ms = 0;

    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let store_clone = store.clone();

    let gateway = thread::spawn(move || {
        for _ in 0..2 {
            let (client, _) = client_listener.accept().unwrap();
            let _ = toolcase_gateway::routes::route_request(client, &store_clone);
        }
    });

    // Unprefixed OmniRoute paths must never reach the upstream, even with
    // an OmniRoute Referer or cookie: the gateway and the module do not share a path space.
    for extra in [
        "Referer: http://127.0.0.1/omniroute/dashboard",
        "Cookie: gw_module=omniroute",
    ] {
        let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
        write!(
            client,
            "GET /home HTTP/1.1\r\nHost: 127.0.0.1:{client_port}\r\n{extra}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        client.flush().unwrap();
        let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
        assert_eq!(head.status, 404);
    }

    gateway.join().unwrap();
    assert!(upstream_listener.accept().is_err());
}

#[test]
fn test_omniroute_home_redirects_to_dashboard() {
    let client_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = client_listener.local_addr().unwrap().port();
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    upstream_listener.set_nonblocking(true).unwrap();

    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = upstream_listener.local_addr().unwrap().port();

    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let store_clone = store.clone();

    let gateway = thread::spawn(move || {
        let (client, _) = client_listener.accept().unwrap();
        let _ = toolcase_gateway::routes::route_request(client, &store_clone);
    });

    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    write!(
        client,
        "GET /omniroute/home HTTP/1.1\r\nHost: 127.0.0.1:{client_port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();
    let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 302);
    assert_eq!(
        toolcase_gateway::http::header_value(&head.headers, "location"),
        Some("/omniroute/dashboard")
    );

    gateway.join().unwrap();
    assert!(upstream_listener.accept().is_err());
}
