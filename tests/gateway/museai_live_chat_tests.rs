//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;
use super::museai_live_tests::{live_capture_config, safe_live_error};

// ---------- Muse live chat, thread continuation, and owned-session tests ----------

use toolcase_gateway::museai::noise::{MuseNoiseSession, NOISE_PATTERN_XX};
use toolcase_gateway::museai::protocol::{Header, ServiceFrameKind, SERVICE_DAEMON};
use toolcase_gateway::museai::transport::MuseWebSocket;
use uuid::Uuid;

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
        jev_base_url: "https://jev-ai.org".into(),
        ..Config::default()
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
fn test_live_muse_chat_multiturn_continuation() {
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
        default_model: "muse".into(),
        museai_base_url: base_url.clone(),
        museai_cookie: cookie,
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 7200,
        jev_api_keys: vec![],
        jev_base_url: "https://jev-ai.org".into(),
        ..Config::default()
    };

    println!("[Live Chat Test] Starting Turn 1...");
    let turn1_body = serde_json::json!({
        "model": "muse",
        "messages": [
            {
                "role": "user",
                "content": "The secret code word is BLUEBERRY. Acknowledge this code word in one brief sentence."
            }
        ]
    });

    let resp1_str = match toolcase_gateway::museai::request_museai_chat_completion(
        turn1_body.to_string().as_bytes(),
        &config,
        false,
    ) {
        Ok(s) => s,
        Err(e) => {
            println!("Live test note: upstream rejected Turn 1 or network error: {e}");
            return;
        }
    };

    let resp1: serde_json::Value =
        serde_json::from_str(&resp1_str).expect("Valid JSON response in Turn 1");
    let thread_id1 = resp1["reply_thread_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .expect("reply_thread_id in Turn 1")
        .to_string();
    let text1 = resp1["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("");
    println!("[Live Chat Test] Turn 1 completed! Thread ID: {thread_id1}, Assistant: {text1}");

    println!("[Live Chat Test] Starting Turn 2 in thread {thread_id1}...");
    let turn2_body = serde_json::json!({
        "model": "muse",
        "reply_thread_id": thread_id1,
        "messages": [
            {
                "role": "user",
                "content": "What was the secret code word I gave you in my previous message? Answer with just the code word."
            }
        ]
    });

    let resp2_str = match toolcase_gateway::museai::request_museai_chat_completion(
        turn2_body.to_string().as_bytes(),
        &config,
        false,
    ) {
        Ok(s) => s,
        Err(e) => {
            panic!("Live test failed in Turn 2 continuation: {e}");
        }
    };

    let resp2: serde_json::Value =
        serde_json::from_str(&resp2_str).expect("Valid JSON response in Turn 2");
    let thread_id2 = resp2["reply_thread_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .expect("reply_thread_id in Turn 2")
        .to_string();
    let text2 = resp2["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("");
    println!("[Live Chat Test] Turn 2 completed! Thread ID: {thread_id2}, Assistant: {text2}");

    assert_eq!(thread_id1, thread_id2, "Turn 2 thread ID must match Turn 1");
    assert!(
        text2.to_lowercase().contains("blueberry"),
        "Turn 2 assistant response must recall the secret code word BLUEBERRY from Turn 1, got: {text2}"
    );
    println!("[Live Chat Test] Thread continuation successfully verified live!");

    let _ = toolcase_gateway::museai::threads::delete_muse_thread(&base_url, &thread_id1, &config);
    println!("[Live Chat Test] Cleaned up test thread {thread_id1}");
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
