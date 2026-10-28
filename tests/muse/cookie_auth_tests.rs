use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use toolcase_gateway::config::Config;
use toolcase_gateway::http::{read_response_head, Request};
use toolcase_gateway::museai::{
    bootstrap_museai_config, handle_create_image_request, handle_create_video_request,
    handle_museai_chat_completions, handle_museai_v1, send_museai_request,
};

fn spawn_mock_server(status: u16, body: &'static str) -> (u16, Arc<AtomicBool>, JoinHandle<()>) {
    let mock_server = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mock_port = mock_server.local_addr().unwrap().port();
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();
    mock_server.set_nonblocking(true).unwrap();

    let mock_handle = thread::spawn(move || {
        while running_clone.load(Ordering::Relaxed) {
            match mock_server.accept() {
                Ok((mut stream, _)) => {
                    let mut buf = [0u8; 1024];
                    let _ = stream.read(&mut buf);
                    let resp = format!(
                        "HTTP/1.1 {status} Status\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });

    (mock_port, running, mock_handle)
}

fn call_handler<F>(handler: F) -> (u16, String)
where
    F: FnOnce(&mut TcpStream) + Send + 'static,
{
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        handler(&mut client);
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let head = read_response_head(&mut client).unwrap();
    let mut body_bytes = head.buffered_body;
    let _ = client.read_to_end(&mut body_bytes);
    handle.join().unwrap();
    (
        head.status,
        String::from_utf8_lossy(&body_bytes).to_string(),
    )
}

#[test]
fn test_muse_empty_cookie_returns_permission_denied_and_401() {
    let (mock_port, running, mock_handle) =
        spawn_mock_server(401, r#"{"error":"missing session cookie"}"#);
    let config = Config {
        museai_base_url: format!("http://127.0.0.1:{mock_port}"),
        museai_cookie: String::new(),
        ..Config::default()
    };

    let err = match bootstrap_museai_config(&config) {
        Ok(_) => panic!("expected Err on empty cookie"),
        Err(e) => e,
    };
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(err.to_string().contains("cookie expired or invalid"));

    let config_clone = config.clone();
    let (status, body) = call_handler(move |client| {
        let payload = br#"{"messages":[{"role":"user","content":"hello"}]}"#;
        let _ = handle_museai_chat_completions(client, payload, &config_clone);
    });
    assert_eq!(status, 401);
    assert!(body.contains("cookie expired or invalid"));

    running.store(false, Ordering::Relaxed);
    mock_handle.join().unwrap();
}

#[test]
fn test_muse_expired_cookie_returns_401_unauthorized() {
    let (mock_port, running, mock_handle) =
        spawn_mock_server(401, r#"{"error":"session has expired"}"#);
    let config = Config {
        museai_base_url: format!("http://127.0.0.1:{mock_port}"),
        museai_cookie: "expired_session_cookie=123".to_string(),
        ..Config::default()
    };

    let err = match bootstrap_museai_config(&config) {
        Ok(_) => panic!("expected Err on expired cookie"),
        Err(e) => e,
    };
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(err.to_string().contains("cookie expired or invalid"));

    let config_clone = config.clone();
    let (status, body) = call_handler(move |client| {
        let payload = br#"{"messages":[{"role":"user","content":"generate video"}]}"#;
        let _ = handle_museai_chat_completions(client, payload, &config_clone);
    });
    assert_eq!(status, 401);
    assert!(body.contains("cookie expired or invalid"));

    running.store(false, Ordering::Relaxed);
    mock_handle.join().unwrap();
}

#[test]
fn test_muse_video_request_expired_cookie_returns_401() {
    let (mock_port, running, mock_handle) =
        spawn_mock_server(401, r#"{"error":"session expired"}"#);
    let config = Config {
        museai_base_url: format!("http://127.0.0.1:{mock_port}"),
        museai_cookie: "expired_session_cookie=123".to_string(),
        ..Config::default()
    };

    let config_clone = config.clone();
    let (status, body) = call_handler(move |client| {
        let req = Request {
            method: "POST".to_string(),
            path: "/muse-ai/v1/video?wait=true".to_string(),
            headers: vec![],
            body: br#"{"prompt":"a cat in space"}"#.to_vec(),
        };
        let _ = handle_create_video_request(client, &req, &config_clone);
    });
    assert_eq!(status, 401);
    assert!(body.contains("cookie expired or invalid"));

    running.store(false, Ordering::Relaxed);
    mock_handle.join().unwrap();
}

#[test]
fn test_muse_image_request_expired_cookie_returns_401() {
    let (mock_port, running, mock_handle) =
        spawn_mock_server(401, r#"{"error":"session expired"}"#);
    let config = Config {
        museai_base_url: format!("http://127.0.0.1:{mock_port}"),
        museai_cookie: "expired_session_cookie=123".to_string(),
        ..Config::default()
    };

    let config_clone = config.clone();
    let (status, body) = call_handler(move |client| {
        let req = Request {
            method: "POST".to_string(),
            path: "/muse-ai/v1/images?wait=true".to_string(),
            headers: vec![],
            body: br#"{"prompt":"a mountain sunset"}"#.to_vec(),
        };
        let _ = handle_create_image_request(client, &req, &config_clone);
    });
    assert_eq!(status, 401);
    assert!(body.contains("cookie expired or invalid"));

    running.store(false, Ordering::Relaxed);
    mock_handle.join().unwrap();
}

#[test]
fn test_muse_v1_expired_cookie_returns_401() {
    let (mock_port, running, mock_handle) =
        spawn_mock_server(401, r#"{"error":"session expired"}"#);
    let config = Config {
        museai_base_url: format!("http://127.0.0.1:{mock_port}"),
        museai_cookie: "expired_token=abc".to_string(),
        ..Config::default()
    };

    let config_clone = config.clone();
    let (status, body) = call_handler(move |client| {
        let payload = br#"{"target":"api/proxy/action","method":"POST","body":{}}"#;
        let _ = handle_museai_v1(client, payload, &config_clone);
    });
    assert_eq!(status, 401);
    assert!(body.contains("cookie is expired, invalid, or missing"));

    running.store(false, Ordering::Relaxed);
    mock_handle.join().unwrap();
}

#[test]
fn test_muse_proxy_send_request_expired_cookie_returns_401() {
    let (mock_port, running, mock_handle) =
        spawn_mock_server(401, r#"{"error":"unauthenticated"}"#);
    let config = Config {
        museai_base_url: format!("http://127.0.0.1:{mock_port}"),
        museai_cookie: "expired_token=abc".to_string(),
        ..Config::default()
    };

    let target_url = format!("http://127.0.0.1:{mock_port}/api/proxy/action");
    let resp = send_museai_request(&target_url, "POST", "{}", &config).unwrap();
    assert_eq!(resp.status, 401);
    assert!(resp.body.contains("cookie is expired, invalid, or missing"));

    running.store(false, Ordering::Relaxed);
    mock_handle.join().unwrap();
}

#[test]
fn test_muse_network_failure_returns_502_bad_gateway() {
    let dummy_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let dead_port = dummy_listener.local_addr().unwrap().port();
    drop(dummy_listener);

    let config = Config {
        museai_base_url: format!("http://127.0.0.1:{dead_port}"),
        museai_cookie: "some_cookie_value".to_string(),
        ..Config::default()
    };

    let config_clone = config.clone();
    let (status, body) = call_handler(move |client| {
        let payload = br#"{"messages":[{"role":"user","content":"hello"}]}"#;
        let _ = handle_museai_chat_completions(client, payload, &config_clone);
    });
    assert_eq!(status, 502);
    assert!(body.contains("Bad Gateway") || body.contains("failed"));
}
