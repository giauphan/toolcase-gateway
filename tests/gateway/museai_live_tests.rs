//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;

// ---------- Muse live-capture and route tests ----------

use toolcase_gateway::museai::noise::{MuseNoiseSession, NOISE_PATTERN_XX};
use toolcase_gateway::museai::protocol::{Header, ServiceFrameKind, SERVICE_DAEMON};
use toolcase_gateway::museai::session::apply_muse_session_metadata;
use toolcase_gateway::museai::transport::MuseWebSocket;
use uuid::Uuid;

fn live_capture_config() -> Config {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let har_bytes = std::fs::read(manifest.join("target/muse.ai.har"))
        .expect("live Muse HAR capture is required");
    let extracted = toolcase_gateway::har_config::extract_muse_config_from_har(&har_bytes)
        .unwrap_or_else(|_| panic!("live Muse HAR could not be parsed"));
    let cookie = std::fs::read(manifest.join("target/muse-header.txt"))
        .ok()
        .and_then(|bytes| parse_cookie_from_header_dump(&bytes))
        .or(extracted.cookie)
        .filter(|cookie| !cookie.is_empty())
        .expect("live Muse capture has no cookie");
    let har: serde_json::Value = serde_json::from_slice(&har_bytes)
        .unwrap_or_else(|_| panic!("live Muse HAR must be valid JSON"));
    let request = har["log"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| &entry["request"])
        .find(|request| {
            request["method"] == "POST" && request["url"] == "https://muse.ai/api/auth/check"
        })
        .expect("capture must evidence the Muse authentication endpoint");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .max_redirects(0)
        .build()
        .into();
    let mut auth = agent
        .post("https://muse.ai/api/auth/check")
        .header("Cookie", &cookie);
    for header in request["headers"].as_array().unwrap() {
        let Some(name) = header["name"].as_str() else {
            continue;
        };
        if [
            "accept",
            "accept-language",
            "origin",
            "referer",
            "user-agent",
            "sec-fetch-site",
            "sec-fetch-mode",
            "sec-fetch-dest",
            "sec-ch-ua",
            "sec-ch-ua-mobile",
            "sec-ch-ua-platform",
        ]
        .iter()
        .any(|allowed| name.eq_ignore_ascii_case(allowed))
        {
            if let Some(value) = header["value"].as_str() {
                auth = auth.header(name, value);
            }
        }
    }
    let auth = auth
        .send_empty()
        .unwrap_or_else(|error| match error {
            ureq::Error::StatusCode(status) => panic!("live Muse authentication HTTP {status}"),
            _ => panic!("live Muse authentication transport failed (details suppressed)"),
        })
        .into_body()
        .read_json::<serde_json::Value>()
        .unwrap_or_else(|_| panic!("invalid live Muse authentication JSON"));
    assert_eq!(
        auth["ok"].as_bool(),
        Some(true),
        "live Muse authentication rejected"
    );
    let token = auth["access_token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .expect("live Muse authentication returned no token")
        .to_owned();
    let session = agent
        .get("https://muse.ai/api/session")
        .header("Cookie", &cookie)
        .call()
        .unwrap_or_else(|_| panic!("live Muse session request failed"))
        .into_body()
        .read_json::<serde_json::Value>()
        .unwrap_or_else(|_| panic!("invalid live Muse session JSON"));
    let mut config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        default_model: "gpt-5.6-sol".into(),
        museai_base_url: "https://muse.ai".into(),
        museai_cookie: cookie,
        museai_ws_url: String::new(),
        museai_access_token: token,
        museai_notary_token: extracted.notary_token.unwrap_or_default(),
        museai_vm_id: String::new(),
        museai_auto_cleanup_threads: false,
        museai_thread_retention_secs: 0,
        jev_api_keys: vec![],
    };
    apply_muse_session_metadata(&mut config, &session);
    config
}

#[test]
fn test_live_muse_auth_from_har_capture() {
    if std::env::var("MUSE_LIVE_TEST").ok().as_deref() != Some("1") {
        return;
    }
    let config = live_capture_config();
    assert!(!config.museai_access_token.is_empty());
    assert!(!config.museai_vm_id.is_empty());
}

#[test]
fn test_live_muse_thread_spawn_and_cleanup() {
    let _ = dotenvy::dotenv();
    let cookie = std::env::var("GW_MUSEAI_COOKIE").unwrap_or_default();
    if cookie.is_empty() {
        println!("Skipping live test: GW_MUSEAI_COOKIE not set");
        return;
    }

    let base_url = std::env::var("GW_MUSEAI_BASE_URL").unwrap_or_else(|_| "https://muse.ai".into());
    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        default_model: "gpt-5.6-sol".into(),
        museai_base_url: base_url.clone(),
        museai_cookie: cookie,
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 0,
        jev_api_keys: vec![],
    };

    match toolcase_gateway::museai::threads::spawn_muse_thread(&base_url, &config) {
        Ok(channel) => {
            assert!(channel.starts_with("thread:"));
            let thread_id = channel.strip_prefix("thread:").unwrap();
            assert!(!thread_id.is_empty());

            let res = toolcase_gateway::museai::threads::delete_muse_thread(
                &base_url, thread_id, &config,
            );
            assert!(res.is_ok(), "Failed to delete live thread: {:?}", res);
        }
        Err(e) => {
            println!(
                "Live test note: upstream rejected thread creation or credentials expired: {e}"
            );
        }
    }
}

#[test]
fn test_thread_cleanup_route_dispatch() {
    use std::net::TcpListener;
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        default_model: "gpt-5.6-sol".into(),
        museai_base_url: "http://127.0.0.1:9".into(), // Will not hang, unreachable
        museai_cookie: "".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
        jev_api_keys: vec![],
    };

    let handle = std::thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        let req_in = toolcase_gateway::http::read_request(&mut client).unwrap();
        assert_eq!(req_in.method, "DELETE");
        assert_eq!(req_in.path, "/muse-ai/v1/threads/mock-thread-456");
        let clean_path = req_in
            .path
            .split('?')
            .next()
            .unwrap_or("")
            .trim_end_matches('/');
        let thread_id = clean_path.split('/').next_back().unwrap_or("");
        assert_eq!(thread_id, "mock-thread-456");
        toolcase_gateway::museai::handle_museai_thread_cleanup(&mut client, thread_id, &config)
            .unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "DELETE /muse-ai/v1/threads/mock-thread-456 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();

    let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    let mut body = head.buffered_body;
    if let Some(length_str) = toolcase_gateway::http::header_value(&head.headers, "content-length")
    {
        let length: usize = length_str.parse().unwrap();
        while body.len() < length {
            toolcase_gateway::http::read_more(&mut client, &mut body).unwrap();
        }
    }
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["object"], "thread.cleanup");
    assert_eq!(json["thread_id"], "mock-thread-456");
    assert_eq!(json["status"], "deleted");

    handle.join().unwrap();
}

fn parse_cookie_from_header_dump(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i + 1 < lines.len() {
        if lines[i].trim().eq_ignore_ascii_case("cookie") {
            let value = lines[i + 1].trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
        i += 1;
    }
    None
}

#[test]
fn parse_cookie_from_header_dump_extracts_cookie_value() {
    let dump =
        b":authority\nmuse.ai\n:method\nGET\ncookie\nsite.example; session=abc\n:method2\nx\n";
    assert_eq!(
        parse_cookie_from_header_dump(dump),
        Some("site.example; session=abc".to_string())
    );
}

#[test]
fn parse_cookie_from_header_dump_without_cookie_is_none() {
    let dump = b":authority\nmuse.ai\n:method\nGET\nreferer\nhttps://muse.ai/thread/new\n";
    assert_eq!(parse_cookie_from_header_dump(dump), None);
}

#[test]
fn parse_cookie_from_header_dump_empty_value_is_none() {
    let dump = b"cookie\n\npriority\nu=1, i\n";
    assert_eq!(parse_cookie_from_header_dump(dump), None);
}

#[test]
fn parse_cookie_from_header_dump_case_insensitive_key() {
    let dump = b"Cookie\nsession=xyz\n";
    assert_eq!(
        parse_cookie_from_header_dump(dump),
        Some("session=xyz".to_string())
    );
}

fn safe_live_error(error: &std::io::Error) -> &'static str {
    if error.kind() == std::io::ErrorKind::PermissionDenied
        || error
            .get_ref()
            .and_then(|e| e.downcast_ref::<toolcase_gateway::museai::MuseApprovalRequired>())
            .is_some()
    {
        "service approval required or access denied"
    } else {
        "incomplete or invalid service response (private details suppressed)"
    }
}

#[test]
fn test_live_muse_owned_session_events() {
    if std::env::var("MUSE_LIVE_RESUME_OWNED").ok().as_deref() != Some("1") {
        return;
    }
    use toolcase_gateway::museai::chat::{
        extract_video_url_from_stream, MuseChatStream, MUSE_POST_COMPLETION_WAIT_SECS,
    };
    use toolcase_gateway::museai::video::{extract_url_from_text, is_supported_video_url};

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/muse-live-owned-session.json");
    let bytes = std::fs::read(path).expect("no task-owned live session saved");
    let saved: serde_json::Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| panic!("invalid saved task-owned session"));
    let id = saved["session_id"]
        .as_str()
        .filter(|id| Uuid::parse_str(id).is_ok())
        .expect("invalid task-owned session identifier");
    let config = live_capture_config();
    let url = toolcase_gateway::museai::build_museai_ws_url(&config, &Uuid::new_v4().to_string())
        .unwrap_or_else(|_| panic!("invalid live Muse socket configuration"));
    let mut socket = MuseWebSocket::connect(&url, "https://muse.ai", "")
        .unwrap_or_else(|_| panic!("live Muse WebSocket connection failed"));
    let mut noise = MuseNoiseSession::new_initiator(NOISE_PATTERN_XX, None)
        .unwrap_or_else(|_| panic!("live Muse Noise initialization failed"));
    noise
        .perform_client_handshake(&mut socket, &config.museai_notary_token)
        .unwrap_or_else(|_| panic!("live Muse Noise handshake failed"));
    let payload = serde_json::json!({"session_id": id, "after_stream_seq": 0,
            "after_chat_event_seq": 0, "capabilities": ["chat_cancel", "delta_stream"]});
    let headers = vec![
        Header {
            key: "Content-Type".into(),
            value: "application/json".into(),
        },
        Header {
            key: "x-app-id".into(),
            value: "hatch-web".into(),
        },
    ];
    socket
        .send_encrypted_service_request(
            &mut noise,
            SERVICE_DAEMON,
            1,
            "POST",
            "/chat/subscribe",
            &headers,
            payload.to_string().as_bytes(),
        )
        .unwrap_or_else(|_| panic!("live Muse owned-session subscribe failed"));
    let mut stream = MuseChatStream {
        session_id: Some(id.to_owned()),
        ..Default::default()
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(MUSE_POST_COMPLETION_WAIT_SECS);
    while std::time::Instant::now() < deadline {
        let frame = match socket.read_encrypted_service_frame(&mut noise) {
            Ok(frame) => frame,
            Err(error) if stream.text.is_empty() => panic!(
                "live Muse owned-session read failed: {}",
                safe_live_error(&error)
            ),
            Err(_) => break,
        };
        assert_eq!(frame.stream_id, 1, "unexpected live Muse response stream");
        let (data, ended) = match frame.kind {
            ServiceFrameKind::Response {
                status,
                body,
                end_body,
                ..
            } => {
                println!("owned-session subscription HTTP status={status}");
                assert!(
                    (200..300).contains(&status),
                    "owned-session subscribe rejected"
                );
                (body, end_body)
            }
            ServiceFrameKind::BodyChunk { data, end_body } => (data, end_body),
            ServiceFrameKind::Reset { code, .. } => {
                panic!("live Muse stream reset code={code}")
            }
        };
        stream.push(&data, ended).unwrap_or_else(|error| {
            panic!(
                "live Muse owned-session result: {}",
                safe_live_error(&error)
            )
        });
        println!(
            "owned-session state: completed={} text_bytes={} pending_bytes={} ended={}",
            stream.completed,
            stream.text.len(),
            stream.pending.len(),
            ended
        );
        if stream.completed || ended || !extract_url_from_text(&stream.text).is_empty() {
            break;
        }
    }
    let mut text = stream.text;
    if extract_url_from_text(&text).is_empty() {
        let deadline =
            std::time::Instant::now() + Duration::from_secs(MUSE_POST_COMPLETION_WAIT_SECS);
        let mut presentation_stream = MuseChatStream {
            session_id: Some(id.to_owned()),
            ..Default::default()
        };
        while std::time::Instant::now() < deadline {
            let frame = match socket.read_encrypted_service_frame(&mut noise) {
                Ok(frame) => frame,
                Err(_) => break,
            };
            let (data, ended) = match frame.kind {
                ServiceFrameKind::Response { body, end_body, .. } => (body, end_body),
                ServiceFrameKind::BodyChunk { data, end_body } => (data, end_body),
                ServiceFrameKind::Reset { .. } => break,
            };
            presentation_stream
                .push(&data, ended)
                .unwrap_or_else(|error| {
                    panic!(
                        "live Muse owned-session presentation result: {}",
                        safe_live_error(&error)
                    )
                });
            if let Some(extracted) = extract_video_url_from_stream(&presentation_stream) {
                println!("Owned-session presentation video artifact received");
                text.push_str(&format!("\nVideo URL: {extracted}\n"));
                break;
            }
        }
    }
    let url = extract_url_from_text(&text);
    assert!(
        is_supported_video_url(&url),
        "owned session completed without a supported HTTPS video result"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/muse-video-result.json");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .unwrap_or_else(|_| panic!("cannot securely create live video result file"));
        let result = serde_json::json!({"object": "video.generation", "status": "completed", "video_url": url});
        file.write_all(result.to_string().as_bytes())
            .unwrap_or_else(|_| panic!("cannot write live video result file"));
        println!("Live video result saved to target/muse-video-result.json");
    }
    #[cfg(not(unix))]
    panic!("secure live-result storage requires an owner-only file ACL implementation");
}

#[test]
fn test_live_create_video_from_har_capture() {
    use toolcase_gateway::museai::video::create_video;

    if std::env::var("MUSE_LIVE_TEST").ok().as_deref() != Some("1") {
        return;
    }
    assert!(!std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/muse-live-owned-session.json").exists(),
            "a task-owned session is already saved; use the read-only owned-session test instead of generating again");
    let config = live_capture_config();
    let body = serde_json::json!({
        "prompt": "A single white square rotating slowly on a black background, 3D cartoon style",
        "model": "muse-video",
        "aspect_ratio": "16:9",
        "duration": 5
    });

    match create_video(body.to_string().as_bytes(), &config) {
        Ok(result) => {
            assert_eq!(result["object"], "video.generation");
            let status = result["status"].as_str().unwrap_or("");
            assert!(
                status == "completed",
                "live Muse did not return a completed video"
            );
            if status == "completed" {
                let url = result["video_url"].as_str().unwrap_or("");
                assert!(
                    url.starts_with("https://"),
                    "completed but video_url is not HTTPS"
                );
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("target/muse-video-result.json");
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(path)
                    .unwrap_or_else(|_| panic!("cannot securely create live video result file"));
                file.write_all(result.to_string().as_bytes())
                    .unwrap_or_else(|_| panic!("cannot write live video result file"));
                println!("Live video result saved to target/muse-video-result.json");
            }
            #[cfg(not(unix))]
            panic!("secure live-result storage requires an owner-only file ACL implementation");
        }
        Err(e) => {
            panic!(
                "Live Muse request failed at API pipeline stage: {} (private details suppressed)",
                e.kind()
            );
        }
    }
}
