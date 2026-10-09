pub(crate) mod business;
pub(crate) mod chat;
pub(crate) mod handlers;
pub(crate) mod noise;
pub(crate) mod protocol;
pub(crate) mod session;
pub(crate) mod threads;
pub(crate) mod transport;
pub(crate) mod video;

pub(crate) use chat::request_museai_chat_completion;
#[cfg(test)]
pub(crate) use chat::{
    build_muse_chat_request, explicit_video_refusal, extract_video_url_from_stream, MuseChatStream,
    MUSE_POST_COMPLETION_WAIT_SECS,
};
pub(crate) use handlers::{
    handle_create_video, handle_museai_thread_cleanup, handle_museai_v1, write_chat_completion,
};
#[cfg(test)]
pub(crate) use session::{bootstrap_museai_config, build_museai_ws_url};
#[cfg(test)]
pub(crate) use threads::{cleanup_tracked_threads_once, tracked_threads};
pub(crate) use threads::{delete_muse_thread, register_thread, start_cleanup_worker};
#[cfg(test)]
pub(crate) use video::extract_url_from_text;
#[cfg(test)]
pub(crate) use video::{
    build_video_prompt, build_video_result, create_video, is_supported_video_url,
    normalize_video_model,
};

#[cfg(test)]
use crate::config::Config;
#[cfg(test)]
use std::io;

#[derive(Debug)]
pub(crate) struct MuseApprovalRequired(pub(crate) String);

impl std::fmt::Display for MuseApprovalRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for MuseApprovalRequired {}

// Retained for legacy HTTP-action fixture coverage, not the production chat flow.
#[cfg(test)]
pub(crate) fn spawn_muse_thread(base_url: &str, config: &Config) -> io::Result<String> {
    let create_thread_url = format!("{base_url}/thread/new");
    let mut req = ureq::post(&create_thread_url)
        .header("Content-Type", "text/plain;charset=UTF-8")
        .header("Accept", "text/x-component")
        .header("Origin", base_url)
        .header("Referer", &format!("{base_url}/thread/new"))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36")
        .header("sec-ch-ua", r#""Brave";v="153", "Not_A Brand";v="8", "Chromium";v="153""#)
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", r#""Windows""#)
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin")
        .header("next-action", "00eaaa570484d537580e0bf4e19ee28a65ac048d62");

    if !config.museai_cookie.is_empty() {
        req = req.header("Cookie", &config.museai_cookie);
    }

    let response = match req.send("[]") {
        Ok(res) => res,
        Err(ureq::Error::StatusCode(401)) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Muse.ai thread creation returned 401 Unauthorized — credentials are invalid or expired",
            ));
        }
        Err(ureq::Error::StatusCode(403)) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Muse.ai thread creation returned 403 Forbidden — access denied",
            ));
        }
        Err(ureq::Error::StatusCode(404)) => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Muse.ai thread creation returned 404 — endpoint not found or cookie expired",
            ));
        }
        Err(ureq::Error::StatusCode(status)) => {
            return Err(io::Error::other(format!(
                "Muse.ai thread creation returned HTTP {status}"
            )));
        }
        Err(e) => {
            return Err(io::Error::other(format!(
                "Thread creation request failed: {e}"
            )));
        }
    };
    let mut reader = response.into_body();
    let text = reader
        .read_to_string()
        .map_err(|e| io::Error::other(format!("Failed to read thread response: {e}")))?;
    let thread_id = extract_new_thread_id(&text);
    if thread_id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "failed to extract thread ID from /thread/new response",
        ));
    }
    let channel = format!("thread:{thread_id}");
    Ok(channel)
}

#[cfg(test)]
fn extract_new_thread_id(rsc_text: &str) -> String {
    let needle = "/thread/";
    if let Some(idx) = rsc_text.rfind(needle) {
        let start = idx + needle.len();
        let remaining = &rsc_text[start..];
        let end = remaining
            .find(|c: char| !c.is_alphanumeric() && c != '-')
            .unwrap_or(remaining.len());
        return remaining[..end].to_string();
    }
    String::new()
}

#[cfg(test)]
pub(crate) mod museai_tests {
    use super::*;
    use crate::museai::noise::{MuseNoiseSession, NOISE_PATTERN_XX};
    use crate::museai::protocol::{Header, ServiceFrameKind, SERVICE_DAEMON};
    use crate::museai::session::apply_muse_session_metadata;
    use crate::museai::transport::MuseWebSocket;
    use std::io::Write;
    use std::net::TcpStream;
    use std::time::Duration;
    use uuid::Uuid;

    fn acknowledged_chat_stream() -> MuseChatStream {
        let mut stream = MuseChatStream::default();
        stream
            .record(&serde_json::json!({
                "session_id": "00000000-0000-4000-8000-000000000001", "is_thread": true
            }))
            .unwrap();
        stream
    }

    #[test]
    fn muse_chat_records_handle_split_utf8_and_coalesced_lines() {
        let records = concat!(
            "{\"session_id\":\"00000000-0000-4000-8000-000000000001\",\"is_thread\":true}\n",
            "{\"event\":\"message.user\",\"payload\":{\"content\":\"do not echo\"}}\n",
            "{\"event\":\"delta.text_append\",\"payload\":{\"text\":\"Café\"}}\n",
            "{\"event\":\"delta.message_done\",\"payload\":{}}\n"
        );
        for split in 0..=records.len() {
            let mut stream = MuseChatStream::default();
            stream.push(&records.as_bytes()[..split], false).unwrap();
            stream.push(&records.as_bytes()[split..], true).unwrap();
            assert_eq!(stream.finish().unwrap().1, "Café");
        }
    }

    #[test]
    fn muse_chat_requires_semantic_thread_acknowledgement() {
        for ack in [
            serde_json::json!({"session_id": "not-a-thread", "is_thread": true}),
            serde_json::json!({"session_id": "00000000-0000-4000-8000-000000000001", "is_thread": false}),
            serde_json::json!({"ok": false, "error": "private-token"}),
        ] {
            let error = MuseChatStream::default().record(&ack).unwrap_err();
            assert!(!error.to_string().contains("private-token"));
        }
        let mut stream = acknowledged_chat_stream();
        assert!(stream
            .record(&serde_json::json!({"type": "response", "payload": {
                "subscribed": true, "session_id": "another-session"
            }}))
            .is_err());
    }

    #[test]
    fn muse_chat_rejects_incomplete_failed_and_oversized_records() {
        let mut partial = acknowledged_chat_stream();
        partial
            .record(
                &serde_json::json!({"event": "delta.text_append", "payload": {"text": "partial"}}),
            )
            .unwrap();
        assert!(partial.finish().is_err());
        let mut stream = acknowledged_chat_stream();
        assert!(stream.push(b"invalid\n", false).is_err());
        assert!(MuseChatStream::default()
            .push(&vec![b'x'; 1024 * 1024 + 1], false)
            .is_err());
        let mut stream = acknowledged_chat_stream();
        let error = stream
            .record(&serde_json::json!({"event": "task.status", "payload": {
                "status": "failed", "error": "https://example.invalid/?token=private"
            }}))
            .unwrap_err();
        assert!(!error.to_string().contains("token"));
    }

    #[test]
    fn muse_chat_ignores_other_sessions_and_requires_scoped_approval() {
        let mut stream = acknowledged_chat_stream();
        stream
            .record(
                &serde_json::json!({"event": "message.assistant", "payload": {
                    "session_id": "another-session", "content": "unrelated text"
                }}),
            )
            .unwrap();
        stream
            .record(
                &serde_json::json!({"event": "approvals.snapshot", "payload": {
                    "pending_approvals": [{"scope": {"thread_id": "another-session"}}]
                }}),
            )
            .unwrap();
        assert!(stream.text.is_empty());
        // Scoped pending approvals must return PermissionDenied rather than hanging.
        let result = stream.record(&serde_json::json!({"event": "approvals.snapshot", "payload": {
            "pending_approvals": [{"scope": {"thread_id": "00000000-0000-4000-8000-000000000001"},
                "payload": {"private": "secret"}}]
        }}));
        assert!(
            result.is_err(),
            "approvals.snapshot with scoped pending approval must error the stream"
        );
        let err = result.unwrap_err();
        assert!(err
            .get_ref()
            .and_then(|e| e.downcast_ref::<crate::museai::MuseApprovalRequired>())
            .is_some());
        assert!(
            stream.text.is_empty(),
            "pending approval should not add text to stream"
        );
    }

    #[test]
    fn test_extract_new_thread_id() {
        let valid_rsc = r#"something something /thread/1234abcd-5678-efgh-ijkl-9876mnopqrst"\n"#;
        assert_eq!(
            extract_new_thread_id(valid_rsc),
            "1234abcd-5678-efgh-ijkl-9876mnopqrst"
        );

        let valid_rsc_2 = r#"/thread/some-id-1234"#;
        assert_eq!(extract_new_thread_id(valid_rsc_2), "some-id-1234");

        let empty_rsc = r#"/thread/ "#;
        assert_eq!(extract_new_thread_id(empty_rsc), "");

        let no_thread = r#"something else"#;
        assert_eq!(extract_new_thread_id(no_thread), "");
    }

    #[test]
    fn test_extract_url_from_text() {
        let valid_text = "Here is your video: https://cdn.muse.ai/video/xyz123.mp4";
        assert_eq!(
            extract_url_from_text(valid_text),
            "https://cdn.muse.ai/video/xyz123.mp4"
        );

        let mov_text = "Watch this https://cdn.muse.ai/video/xyz123.mov, and enjoy!";
        assert_eq!(
            extract_url_from_text(mov_text),
            "https://cdn.muse.ai/video/xyz123.mov"
        );

        let no_video_ext = "Here is a link https://muse.ai/some-link";
        assert_eq!(extract_url_from_text(no_video_ext), "");

        let misleading_query = "Not a video https://example.com/help?next=clip.mp4";
        assert_eq!(extract_url_from_text(misleading_query), "");

        let http_video = "Do not accept http://cdn.muse.ai/video/xyz123.mp4";
        assert_eq!(extract_url_from_text(http_video), "");

        let no_url = "Just some text without links";
        assert_eq!(extract_url_from_text(no_url), "");

        let gdrive_url =
            "Video ready at https://drive.google.com/file/d/1a2b3c4d5e/view?usp=sharing enjoy!";
        assert_eq!(
            extract_url_from_text(gdrive_url),
            "https://drive.google.com/file/d/1a2b3c4d5e/view?usp=sharing"
        );

        let gdrive_multi =
            "Check https://example.com/site or Google Drive: https://drive.google.com/uc?id=xyz789";
        assert_eq!(
            extract_url_from_text(gdrive_multi),
            "https://drive.google.com/uc?id=xyz789"
        );

        let unsafe_drive = "Do not accept http://drive.google.com/uc?id=xyz789";
        assert_eq!(extract_url_from_text(unsafe_drive), "");

        let lookalike_drive = "Do not accept https://drive.google.com.evil.example/file/d/123";
        assert_eq!(extract_url_from_text(lookalike_drive), "");

        // Prefer public Google Drive URL over a direct Muse CDN video URL if both appear.
        let drive_and_direct = "Drive https://drive.google.com/file/d/drive123/view and direct https://cdn.muse.ai/video/direct.mp4";
        assert_eq!(
            extract_url_from_text(drive_and_direct),
            "https://drive.google.com/file/d/drive123/view"
        );
    }

    #[test]
    fn test_muse_chat_stream_allows_empty_text_on_completion() {
        let mut stream = acknowledged_chat_stream();
        stream
            .record(&serde_json::json!({
                "event": "task.complete",
                "payload": {}
            }))
            .unwrap();
        let (session_id, text) = stream
            .finish()
            .expect("finish must succeed on completion even if text is empty");
        assert_eq!(session_id, "00000000-0000-4000-8000-000000000001");
        assert_eq!(text, "");
    }

    #[test]
    fn test_muse_ws_operation_deadline_covers_post_completion_wait() {
        assert!(
            crate::museai::transport::muse_ws_operation_deadline_secs()
                >= MUSE_POST_COMPLETION_WAIT_SECS
        );
    }

    #[test]
    fn test_post_completion_parser_handles_split_presentation_records() {
        let session_id = "00000000-0000-4000-8000-000000000001";
        let first = br#"{"event":"delta.presentation","payload":{"data":{"url":"https://drive.google.com/file/d/video123/view"}}}"#;
        let second = b"\n";
        let chunks: &[&[u8]] = &[first, second];
        let mut stream = MuseChatStream {
            session_id: Some(session_id.to_string()),
            ..Default::default()
        };
        for chunk in chunks {
            stream.push(chunk, false).unwrap();
        }
        assert_eq!(
            extract_video_url_from_stream(&stream),
            Some("https://drive.google.com/file/d/video123/view".to_string())
        );
    }

    #[test]
    fn test_post_completion_parser_propagates_scoped_approval() {
        let mut stream = MuseChatStream {
            session_id: Some("00000000-0000-4000-8000-000000000001".to_string()),
            ..Default::default()
        };
        let record = br#"{"event":"approvals.snapshot","payload":{"pending_approvals":[{"scope":{"session_scope_id":"00000000-0000-4000-8000-000000000001"}}]}}
"#;
        let error = stream.push(record, false).unwrap_err();
        assert!(error
            .get_ref()
            .and_then(|e| e.downcast_ref::<crate::museai::MuseApprovalRequired>())
            .is_some());
    }

    #[test]
    fn test_delta_message_done_populates_transcript_when_deltas_empty() {
        let mut stream = acknowledged_chat_stream();
        stream
            .record(&serde_json::json!({
                "event": "delta.message_done",
                "payload": {
                    "transcript": {
                        "messages": [
                            {
                                "role": "user",
                                "content": [{"text": "create video"}]
                            },
                            {
                                "role": "assistant",
                                "content": [{"text": "Here is your video: https://cdn.muse.ai/video/abc.mp4"}]
                            }
                        ]
                    }
                }
            }))
            .unwrap();
        assert!(stream.completed);
        let (_, text) = stream.finish().unwrap();
        assert_eq!(
            text,
            "Here is your video: https://cdn.muse.ai/video/abc.mp4"
        );
        assert_eq!(
            extract_url_from_text(&text),
            "https://cdn.muse.ai/video/abc.mp4"
        );
    }

    #[test]
    fn test_status_completed_before_presentation_without_text() {
        let mut stream = acknowledged_chat_stream();
        stream
            .record(&serde_json::json!({
                "event": "task.status",
                "payload": {"status": "completed"}
            }))
            .unwrap();
        assert!(!stream.completed);

        stream
            .record(&serde_json::json!({
                "event": "delta.presentation",
                "payload": {
                    "data": {
                        "data": {
                            "url": "https://drive.google.com/file/d/123/view"
                        }
                    }
                }
            }))
            .unwrap();
        assert_eq!(
            stream.text,
            "\nVideo URL: https://drive.google.com/file/d/123/view\n"
        );

        stream
            .record(&serde_json::json!({
                "event": "delta.message_done",
                "payload": {}
            }))
            .unwrap();
        assert!(stream.completed);
        let (_, text) = stream.finish().unwrap();
        assert_eq!(
            text,
            "\nVideo URL: https://drive.google.com/file/d/123/view\n"
        );
        let result = build_video_result(
            "muse-video",
            "muse-video",
            "a rotating square",
            "16:9",
            5,
            &text,
        )
        .unwrap();
        assert_eq!(result["status"], "completed");
        assert_eq!(
            result["video_url"],
            "https://drive.google.com/file/d/123/view"
        );
    }

    #[test]
    fn test_task_status_completed_does_not_prematurely_finish_stream() {
        let mut stream = acknowledged_chat_stream();
        stream
            .record(&serde_json::json!({
                "event": "task.status",
                "payload": {"status": "completed"}
            }))
            .unwrap();
        assert!(
            !stream.completed,
            "task.status snapshot must not mark stream as completed"
        );

        stream
            .record(&serde_json::json!({
                "event": "delta.text_append",
                "payload": {"text": "Video generated: https://cdn.muse.ai/video/xyz.mp4"}
            }))
            .unwrap();
        assert_eq!(
            stream.text,
            "Video generated: https://cdn.muse.ai/video/xyz.mp4"
        );

        stream
            .record(&serde_json::json!({
                "event": "delta.message_done",
                "payload": {}
            }))
            .unwrap();
        assert!(stream.completed);
        let (_, text) = stream.finish().unwrap();
        assert_eq!(text, "Video generated: https://cdn.muse.ai/video/xyz.mp4");
    }

    #[test]
    fn test_completed_video_response_requires_supported_artifact() {
        let error = build_video_result(
            "muse-video",
            "muse-video",
            "a dancing cat",
            "16:9",
            5,
            "Video generation is unavailable in this environment.",
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn test_explicit_video_refusal_skips_delayed_artifact_wait() {
        assert!(explicit_video_refusal(
            "Video generation is unavailable in this environment."
        ));
        assert!(explicit_video_refusal("I cannot generate a video here."));
        assert!(explicit_video_refusal("I am unable to generate videos."));
        assert!(explicit_video_refusal("I cannot create videos right now."));
        assert!(!explicit_video_refusal("Your video is being prepared."));
    }

    #[test]
    fn test_normalize_video_model_aliases() {
        assert_eq!(normalize_video_model("muse"), "muse-video");
        assert_eq!(normalize_video_model("gen-3"), "muse-video");
        assert_eq!(normalize_video_model("kling"), "muse-video");
        assert_eq!(normalize_video_model("muse-video"), "muse-video");
    }

    #[test]
    fn test_stream_explicit_cancellation_and_error_events() {
        let mut stream_cancelled = acknowledged_chat_stream();
        let err_cancelled = stream_cancelled
            .record(&serde_json::json!({
                "event": "task.status",
                "payload": {"status": "cancelled"}
            }))
            .unwrap_err();
        assert_eq!(err_cancelled.kind(), std::io::ErrorKind::Other);

        let mut stream_error = acknowledged_chat_stream();
        let err_error = stream_error
            .record(&serde_json::json!({
                "event": "task.status",
                "payload": {"status": "error"}
            }))
            .unwrap_err();
        assert_eq!(err_error.kind(), std::io::ErrorKind::Other);
    }

    #[test]
    fn test_video_prompt_avoids_unsupported_capability_claims() {
        let prompt = build_video_prompt("gen-3", "a dancing cat", "16:9", 5);
        for unsupported in ["gen-3", "kling", "Muse Video", "16:9", "5 seconds"] {
            assert!(!prompt.contains(unsupported));
        }
        assert!(prompt.contains("a dancing cat"));
        assert!(prompt.contains("if video generation is available"));
        assert!(prompt.contains("public Google Drive link"));
    }

    #[test]
    fn test_create_video_validation() {
        let config = crate::config::Config {
            target_host: "127.0.0.1".into(),
            target_port: 8080,
            fallbacks: vec![],
            io_timeout: None,
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 1000,
            default_model: "gpt-5.6-sol".into(),
            museai_base_url: "https://muse.ai".into(),
            museai_cookie: "".into(),
            museai_ws_url: "".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 86400,
        };

        // 1. Empty prompt
        let empty_prompt = serde_json::json!({
            "prompt": "   ",
            "model": "gen-3"
        });
        let result = create_video(&serde_json::to_vec(&empty_prompt).unwrap(), &config);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);

        // 2. Invalid model
        let invalid_model = serde_json::json!({
            "prompt": "A cat",
            "model": "invalid-model"
        });
        let result = create_video(&serde_json::to_vec(&invalid_model).unwrap(), &config);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);

        // 3. Invalid aspect ratio
        let invalid_ar = serde_json::json!({
            "prompt": "A cat",
            "aspect_ratio": "4:5"
        });
        let result = create_video(&serde_json::to_vec(&invalid_ar).unwrap(), &config);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_thread_registration_and_retention() {
        let config = crate::config::Config {
            target_host: "127.0.0.1".into(),
            target_port: 8080,
            fallbacks: vec![],
            io_timeout: None,
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 1000,
            default_model: "gpt-5.6-sol".into(),
            museai_base_url: "https://muse.ai".into(),
            museai_cookie: "".into(),
            museai_ws_url: "".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 86400,
        };

        let thread_id = "test-thread-id-12345".to_string();
        register_thread(
            thread_id.clone(),
            "https://muse.ai".to_string(),
            config.clone(),
        );

        let threads = tracked_threads().lock().unwrap();
        let found = threads.iter().find(|t| t.id == thread_id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().config.museai_thread_retention_secs, 86400);
    }

    #[test]
    fn test_thread_cleanup_active_vs_expired() {
        let config_active = crate::config::Config {
            target_host: "127.0.0.1".into(),
            target_port: 8080,
            fallbacks: vec![],
            io_timeout: None,
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 1000,
            default_model: "gpt-5.6-sol".into(),
            museai_base_url: "https://muse.ai".into(),
            museai_cookie: "".into(),
            museai_ws_url: "".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 86400, // 24 hours
        };

        let mut config_expired = config_active.clone();
        config_expired.museai_thread_retention_secs = 0; // expires immediately

        let active_id = "thread-active-xyz".to_string();
        let expired_id = "thread-expired-abc".to_string();

        // Release the mutex lock from previous operations just to be safe
        register_thread(
            active_id.clone(),
            "https://muse.ai".to_string(),
            config_active,
        );
        register_thread(
            expired_id.clone(),
            "https://muse.ai".to_string(),
            config_expired,
        );

        // Run the cleanup once synchronously to verify it triggers correctly
        let deleted = cleanup_tracked_threads_once();

        // Check that at least the expired one got cleared
        assert!(deleted >= 1);

        let threads = tracked_threads().lock().unwrap();
        // The expired one should be gone
        assert!(!threads.iter().any(|t| t.id == expired_id));
        // The active one should still be tracked
        assert!(threads.iter().any(|t| t.id == active_id));
    }

    #[test]
    fn test_live_muse_thread_spawn_and_cleanup() {
        let _ = dotenvy::dotenv();
        let cookie = std::env::var("GW_MUSEAI_COOKIE").unwrap_or_default();
        if cookie.is_empty() {
            println!("Skipping live test: GW_MUSEAI_COOKIE not set");
            return;
        }

        let base_url =
            std::env::var("GW_MUSEAI_BASE_URL").unwrap_or_else(|_| "https://muse.ai".into());
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
        };

        match spawn_muse_thread(&base_url, &config) {
            Ok(channel) => {
                assert!(channel.starts_with("thread:"));
                let thread_id = channel.strip_prefix("thread:").unwrap();
                assert!(!thread_id.is_empty());

                let res = delete_muse_thread(&base_url, thread_id, &config);
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
        };

        let handle = std::thread::spawn(move || {
            let (mut client, _) = listener.accept().unwrap();
            let req_in = crate::http::read_request(&mut client).unwrap();
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
            handle_museai_thread_cleanup(&mut client, thread_id, &config).unwrap();
        });

        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            client,
            "DELETE /muse-ai/v1/threads/mock-thread-456 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        client.flush().unwrap();

        let head = crate::http::read_response_head(&mut client).unwrap();
        assert_eq!(head.status, 200);
        let mut body = head.buffered_body;
        if let Some(length_str) = crate::http::header_value(&head.headers, "content-length") {
            let length: usize = length_str.parse().unwrap();
            while body.len() < length {
                crate::http::read_more(&mut client, &mut body).unwrap();
            }
        }
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["object"], "thread.cleanup");
        assert_eq!(json["thread_id"], "mock-thread-456");
        assert_eq!(json["status"], "deleted");

        handle.join().unwrap();
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

    pub(crate) fn save_live_owned_session(id: &str) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/muse-live-owned-session.json");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)?;
            file.write_all(serde_json::json!({"session_id": id}).to_string().as_bytes())?;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = id;
            Err(io::Error::other(
                "secure live-session storage unavailable on this platform",
            ))
        }
    }

    fn live_capture_config() -> Config {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let har_bytes = std::fs::read(manifest.join("target/muse.ai.har"))
            .expect("live Muse HAR capture is required");
        let extracted = crate::har_config::extract_muse_config_from_har(&har_bytes)
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
    fn test_live_muse_owned_session_events() {
        if std::env::var("MUSE_LIVE_RESUME_OWNED").ok().as_deref() != Some("1") {
            return;
        }
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
        let url = build_museai_ws_url(&config, &Uuid::new_v4().to_string())
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
        let deadline =
            std::time::Instant::now() + Duration::from_secs(MUSE_POST_COMPLETION_WAIT_SECS);
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
            if stream.completed || ended {
                break;
            }
        }
        assert!(
            stream.completed,
            "owned session did not reach a terminal event"
        );
        let (_, mut text) = stream.finish().unwrap_or_else(|error| {
            panic!(
                "live Muse owned-session result: {}",
                safe_live_error(&error)
            )
        });
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
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/muse-video-result.json");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
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

    pub(crate) fn log_owned_record_schema(record: &serde_json::Value) {
        fn schema(value: &serde_json::Value, depth: usize) {
            if depth > 6 {
                return;
            }
            match value {
                serde_json::Value::Object(fields) => {
                    for (key, value) in fields {
                        let key = match key.as_str() {
                            "type" | "event" | "payload" | "status" | "session_id"
                            | "subscribed" | "text" | "content" | "message" | "messages"
                            | "transcript" | "data" | "url" | "role" | "items" | "events"
                            | "pending_approvals" | "scope" | "thread_id" | "session_scope_id"
                            | "result" | "error" | "ok" | "state" | "snapshot" | "format"
                            | "mime_type" | "completed" | "is_final" => key.as_str(),
                            _ => "other",
                        };
                        let kind = match value {
                            serde_json::Value::Null => "null",
                            serde_json::Value::Bool(_) => "bool",
                            serde_json::Value::Number(_) => "number",
                            serde_json::Value::String(_) => "string",
                            serde_json::Value::Array(_) => "array",
                            serde_json::Value::Object(_) => "object",
                        };
                        println!("owned-session schema depth={depth} field={key} type={kind}");
                        schema(value, depth + 1);
                    }
                }
                serde_json::Value::Array(items) => {
                    println!("owned-session schema depth={depth} items={}", items.len());
                    for item in items.iter().take(8) {
                        schema(item, depth + 1);
                    }
                }
                _ => {}
            }
        }
        let category = match record["event"].as_str().unwrap_or("") {
            event @ ("delta.text_append"
            | "delta.message_done"
            | "task.status"
            | "task.complete"
            | "agent.status"
            | "message.assistant"
            | "message.user"
            | "delta.presentation"
            | "delta.message_start"
            | "delta.tool_start"
            | "delta.tool_done"
            | "approvals.snapshot"
            | "delta.marker"
            | "delta.placeholder"
            | "chat.snapshot"
            | "session.snapshot") => event,
            _ => "other",
        };
        println!("owned-session event category={category}");
        schema(record, 0);
    }

    fn safe_live_error(error: &io::Error) -> &'static str {
        if error.kind() == io::ErrorKind::PermissionDenied
            || error
                .get_ref()
                .and_then(|e| e.downcast_ref::<crate::museai::MuseApprovalRequired>())
                .is_some()
        {
            "service approval required or access denied"
        } else {
            "incomplete or invalid service response (private details suppressed)"
        }
    }

    #[test]
    fn test_live_create_video_from_har_capture() {
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
                        .unwrap_or_else(|_| {
                            panic!("cannot securely create live video result file")
                        });
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
}
