//! Tests for continuing chat conversations in existing Muse AI threads.

use super::common::*;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use toolcase_gateway::museai::chat::{
    build_muse_chat_request, build_muse_chat_request_with_session, extract_continuation_prompt,
    extract_thread_id, extract_thread_id_from_value, MuseChatStream,
};
use uuid::Uuid;

#[test]
fn test_extract_thread_id_from_various_field_names() {
    let cases = vec![
        (
            serde_json::json!({"reply_thread_id": "00000000-0000-4000-8000-000000000001"}),
            Some("00000000-0000-4000-8000-000000000001".to_string()),
        ),
        (
            serde_json::json!({"thread_id": "00000000-0000-4000-8000-000000000002"}),
            Some("00000000-0000-4000-8000-000000000002".to_string()),
        ),
        (
            serde_json::json!({"session_id": "00000000-0000-4000-8000-000000000003"}),
            Some("00000000-0000-4000-8000-000000000003".to_string()),
        ),
        (
            serde_json::json!({"reply_to_thread_id": "00000000-0000-4000-8000-000000000004"}),
            Some("00000000-0000-4000-8000-000000000004".to_string()),
        ),
        (
            serde_json::json!({
                "metadata": {
                    "reply_thread_id": "00000000-0000-4000-8000-000000000005"
                }
            }),
            Some("00000000-0000-4000-8000-000000000005".to_string()),
        ),
        (
            serde_json::json!({"thread_id": "thread_00000000-0000-4000-8000-000000000006"}),
            Some("00000000-0000-4000-8000-000000000006".to_string()),
        ),
        (
            serde_json::json!({"session_id": "session-00000000-0000-4000-8000-000000000007"}),
            Some("00000000-0000-4000-8000-000000000007".to_string()),
        ),
        (
            serde_json::json!({"messages": [{"role": "user", "content": "hello"}]}),
            None,
        ),
        (serde_json::json!({"thread_id": ""}), None),
        (serde_json::json!({"thread_id": "   "}), None),
    ];

    for (json, expected) in cases {
        let extracted = extract_thread_id_from_value(&json).unwrap();
        assert_eq!(extracted, expected);

        let bytes = serde_json::to_vec(&json).unwrap();
        let extracted_bytes = extract_thread_id(&bytes).unwrap();
        assert_eq!(extracted_bytes, expected);
    }
}

#[test]
fn test_extract_thread_id_rejects_invalid_values() {
    let invalid_cases = vec![
        serde_json::json!({"reply_thread_id": "not-a-uuid"}),
        serde_json::json!({"thread_id": "invalid-uuid-format"}),
        serde_json::json!({"session_id": 12345}),
        serde_json::json!({"metadata": {"reply_thread_id": "not-a-uuid"}}),
    ];

    for json in invalid_cases {
        assert!(
            extract_thread_id_from_value(&json).is_err(),
            "Expected error for {json}"
        );
        let bytes = serde_json::to_vec(&json).unwrap();
        assert!(
            extract_thread_id(&bytes).is_err(),
            "Expected error for bytes of {json}"
        );
    }
}

#[test]
fn test_build_muse_chat_request_with_session() {
    let custom_session = "00000000-0000-4000-8000-000000000001";
    let req = build_muse_chat_request_with_session("test prompt", Some(custom_session));

    assert_eq!(req["message"], "test prompt");
    assert_eq!(req["session_id"], custom_session);
    assert!(Uuid::parse_str(req["node_id"].as_str().unwrap()).is_ok());
    assert_eq!(
        req["capabilities"],
        serde_json::json!(["chat_cancel", "delta_stream"])
    );

    let fresh_req = build_muse_chat_request("fresh prompt");
    assert_eq!(fresh_req["message"], "fresh prompt");
    assert!(Uuid::parse_str(fresh_req["session_id"].as_str().unwrap()).is_ok());
    assert!(Uuid::parse_str(fresh_req["node_id"].as_str().unwrap()).is_ok());
    assert_ne!(fresh_req["session_id"], fresh_req["node_id"]);
}

#[test]
fn test_extract_continuation_prompt_handles_multiturn_messages() {
    let single_msg = serde_json::json!({
        "messages": [
            {"role": "user", "content": "What is the weather?"}
        ],
        "reply_thread_id": "00000000-0000-4000-8000-000000000001"
    });
    let bytes = serde_json::to_vec(&single_msg).unwrap();
    assert_eq!(
        extract_continuation_prompt(&bytes).unwrap(),
        "What is the weather?"
    );

    let multi_turn = serde_json::json!({
        "messages": [
            {"role": "user", "content": "My name is Bob"},
            {"role": "assistant", "content": "Hello Bob!"},
            {"role": "user", "content": "What was my name again?"}
        ],
        "reply_thread_id": "00000000-0000-4000-8000-000000000001"
    });
    let bytes = serde_json::to_vec(&multi_turn).unwrap();
    assert_eq!(
        extract_continuation_prompt(&bytes).unwrap(),
        "What was my name again?"
    );

    let array_content = serde_json::json!({
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": "Part 1 "},
                    {"type": "text", "text": "Part 2"}
                ]
            }
        ],
        "reply_thread_id": "00000000-0000-4000-8000-000000000001"
    });
    let bytes = serde_json::to_vec(&array_content).unwrap();
    assert_eq!(
        extract_continuation_prompt(&bytes).unwrap(),
        "Part 1 Part 2"
    );
}

#[test]
fn test_stream_with_expected_session() {
    let expected = "00000000-0000-4000-8000-000000000001";
    let mut stream = MuseChatStream::with_expected_session(Some(expected.to_string()));

    let matching_ack = serde_json::json!({
        "session_id": expected,
        "is_thread": true
    });
    assert!(stream.record(&matching_ack).is_ok());
    assert_eq!(stream.session_id.as_deref(), Some(expected));

    let mut stream_mismatch = MuseChatStream::with_expected_session(Some(expected.to_string()));
    let different_ack = serde_json::json!({
        "session_id": "00000000-0000-4000-8000-000000000002",
        "is_thread": true
    });
    let err = stream_mismatch.record(&different_ack).unwrap_err();
    assert!(err.to_string().contains("Muse chat session mismatch"));

    stream
        .record(&serde_json::json!({
            "type": "response",
            "payload": {
                "subscribed": true,
                "session_id": expected
            }
        }))
        .unwrap();

    stream
        .record(&serde_json::json!({
            "event": "delta.text_append",
            "payload": {"text": "Continued reply"}
        }))
        .unwrap();

    stream
        .record(&serde_json::json!({
            "event": "delta.message_done",
            "payload": {}
        }))
        .unwrap();

    let (finished_thread_id, finished_text) = stream.finish().unwrap();
    assert_eq!(finished_thread_id, expected);
    assert_eq!(finished_text, "Continued reply");
}

#[test]
fn test_muse_chat_completions_route_rejects_malformed_reply_thread_id() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = Config {
        museai_cookie: "mock-cookie".to_string(),
        ..Config::default()
    };
    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));

    let handle = std::thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        let _ = toolcase_gateway::routes::route_request(client, &store);
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let payload = serde_json::json!({
        "messages": [{"role": "user", "content": "hi"}],
        "reply_thread_id": "malformed-thread-id"
    })
    .to_string();

    write!(
        client,
        "POST /muse-ai/v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    )
    .unwrap();
    client.flush().unwrap();

    let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 400);
    let body = String::from_utf8_lossy(&head.buffered_body);
    assert!(body.contains("Invalid thread_id / reply_thread_id 'malformed-thread-id'"));
    handle.join().unwrap();
}

#[test]
fn test_thread_registration_updates_activity_timestamp_and_deduplicates() {
    let thread_id = format!("test-dedup-{}", Uuid::new_v4());
    let base_url = "https://muse.ai".to_string();
    let config = Config {
        museai_thread_retention_secs: 7200,
        ..Config::default()
    };

    toolcase_gateway::museai::threads::register_thread(
        thread_id.clone(),
        base_url.clone(),
        config.clone(),
    );

    let first_ts = {
        let threads = toolcase_gateway::museai::threads::tracked_threads()
            .lock()
            .unwrap();
        let found = threads.iter().find(|t| t.id == thread_id).unwrap();
        found.created_at
    };

    std::thread::sleep(std::time::Duration::from_millis(50));

    toolcase_gateway::museai::threads::register_thread(
        thread_id.clone(),
        base_url.clone(),
        config.clone(),
    );

    {
        let threads = toolcase_gateway::museai::threads::tracked_threads()
            .lock()
            .unwrap();
        let matching: Vec<_> = threads.iter().filter(|t| t.id == thread_id).collect();
        assert_eq!(matching.len(), 1, "Thread must not be duplicated in memory");
        assert!(
            matching[0].created_at >= first_ts,
            "Activity timestamp should be refreshed"
        );
    }

    toolcase_gateway::museai::threads::unregister_thread(&thread_id);
}

#[test]
fn test_stream_filters_historical_replay_events_on_continuation() {
    let expected = "00000000-0000-4000-8000-000000000001";
    let mut stream = MuseChatStream::with_expected_turn(
        Some(expected.to_string()),
        Some("What is the capital of France?".to_string()),
    );

    // Initial ack
    stream
        .record(&serde_json::json!({
            "session_id": expected,
            "is_thread": true
        }))
        .unwrap();

    stream
        .record(&serde_json::json!({
            "type": "response",
            "payload": {
                "subscribed": true,
                "session_id": expected
            }
        }))
        .unwrap();

    // Turn 1 historical replay (should be skipped!)
    stream
        .record(&serde_json::json!({
            "event": "message.user",
            "payload": {
                "chat_event_seq": 100,
                "content": "user:\nTell me a joke"
            }
        }))
        .unwrap();

    stream
        .record(&serde_json::json!({
            "event": "delta.text_append",
            "payload": {
                "message_seq": 101,
                "text": "Why did the chicken cross the road?"
            }
        }))
        .unwrap();

    stream
        .record(&serde_json::json!({
            "event": "delta.message_done",
            "payload": {
                "message_seq": 101,
                "transcript": {
                    "messages": [
                        {"role": "assistant", "content": "Why did the chicken cross the road?"}
                    ]
                }
            }
        }))
        .unwrap();

    assert!(
        !stream.completed,
        "Turn 1 historical replay must not complete the stream"
    );
    assert!(
        stream.text.is_empty(),
        "Turn 1 historical replay text must be discarded"
    );

    // Turn 2 active user message
    stream
        .record(&serde_json::json!({
            "event": "message.user",
            "payload": {
                "chat_event_seq": 102,
                "content": "user:\nWhat is the capital of France?"
            }
        }))
        .unwrap();

    // Delayed late replay of Turn 1 completion (seq 101 <= 102)
    stream
        .record(&serde_json::json!({
            "event": "delta.message_done",
            "payload": {
                "message_seq": 101,
                "transcript": {
                    "messages": [
                        {"role": "assistant", "content": "Late Turn 1 done"}
                    ]
                }
            }
        }))
        .unwrap();

    assert!(
        !stream.completed,
        "Delayed historical message_done must be ignored based on sequence numbers"
    );

    // Turn 2 tokens and completion
    stream
        .record(&serde_json::json!({
            "event": "delta.text_append",
            "payload": {
                "message_seq": 103,
                "text": "Paris."
            }
        }))
        .unwrap();

    stream
        .record(&serde_json::json!({
            "event": "delta.message_done",
            "payload": {
                "message_seq": 103,
                "transcript": {
                    "messages": [
                        {"role": "assistant", "content": "Paris."}
                    ]
                }
            }
        }))
        .unwrap();

    assert!(
        stream.completed,
        "Turn 2 completion must mark stream completed"
    );
    let (finished_id, finished_text) = stream.finish().unwrap();
    assert_eq!(finished_id, expected);
    assert_eq!(finished_text, "Paris.");
}

#[test]
fn test_stream_delta_message_done_does_not_duplicate_text() {
    let mut stream = MuseChatStream::default();
    stream
        .record(&serde_json::json!({
            "session_id": "00000000-0000-4000-8000-000000000001",
            "is_thread": true
        }))
        .unwrap();

    stream
        .record(&serde_json::json!({
            "event": "delta.text_append",
            "payload": {"text": "Hello world!"}
        }))
        .unwrap();

    stream
        .record(&serde_json::json!({
            "event": "delta.message_done",
            "payload": {
                "transcript": {
                    "messages": [
                        {"role": "assistant", "content": "Hello world!"}
                    ]
                }
            }
        }))
        .unwrap();

    let (_, text) = stream.finish().unwrap();
    assert_eq!(
        text, "Hello world!",
        "Assistant text should not be duplicated when transcript is present"
    );
}
