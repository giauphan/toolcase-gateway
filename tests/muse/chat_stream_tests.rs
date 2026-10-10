//! Muse chat stream integration tests.

use super::common::*;
use toolcase_gateway::museai::transport::muse_ws_operation_deadline_secs;

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
        .record(&serde_json::json!({"event": "delta.text_append", "payload": {"text": "partial"}}))
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
    let result = stream.record(
        &serde_json::json!({"event": "approvals.snapshot", "payload": {
            "pending_approvals": [{"scope": {"thread_id": "00000000-0000-4000-8000-000000000001"},
                "payload": {"private": "secret"}}]
        }}),
    );
    assert!(
        result.is_err(),
        "approvals.snapshot with scoped pending approval must error the stream"
    );
    let err = result.unwrap_err();
    assert!(err
        .get_ref()
        .and_then(|e| e.downcast_ref::<MuseApprovalRequired>())
        .is_some());
    assert!(
        stream.text.is_empty(),
        "pending approval should not add text to stream"
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
    assert!(muse_ws_operation_deadline_secs() >= MUSE_POST_COMPLETION_WAIT_SECS);
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
        .and_then(|e| e.downcast_ref::<MuseApprovalRequired>())
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
fn museai_stream_request_uses_fresh_draft_session_id() {
    let first = build_muse_chat_request("sanitized test prompt");
    let second = build_muse_chat_request("sanitized test prompt");
    assert_eq!(first["message"], "sanitized test prompt");
    assert!(uuid::Uuid::parse_str(first["session_id"].as_str().unwrap()).is_ok());
    assert!(uuid::Uuid::parse_str(first["node_id"].as_str().unwrap()).is_ok());
    assert_ne!(first["session_id"], second["session_id"]);
    assert_ne!(first["session_id"], first["node_id"]);
    assert!(first.get("chat_id").is_none());
    assert!(first.get("channel").is_none());
}
