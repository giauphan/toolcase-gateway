//! Tests for thread registration deduplication and continuation stream replay recovery.

use super::common::*;
use toolcase_gateway::museai::chat::MuseChatStream;
use uuid::Uuid;

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
