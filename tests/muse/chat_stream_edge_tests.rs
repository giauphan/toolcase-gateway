//! Muse chat stream edge-case and structured artifact extraction tests.

use super::common::*;

#[test]
fn museai_chat_module_keeps_public_helpers_at_existing_path() {
    use toolcase_gateway::museai::chat::{
        build_muse_chat_request, explicit_video_refusal, extract_user_prompt,
        parse_assistant_content_from_json,
    };

    let request = br#"{"messages":[{"role":"user","content":"hello"}]}"#;
    assert_eq!(extract_user_prompt(request).unwrap(), "user:\nhello");
    assert_eq!(build_muse_chat_request("hello")["message"], "hello");
    assert_eq!(
        parse_assistant_content_from_json(&serde_json::json!({"content": "response"})),
        Some("response".to_string())
    );
    assert!(explicit_video_refusal("video generation is unavailable"));
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

#[test]
fn test_chat_stream_extracts_video_url_from_tool_done_resources() {
    let mut stream = acknowledged_chat_stream();
    let cdn_url = "https://video.xx.fbcdn.net/v/t39.25447-2/tool_result.mp4?ccb=1-7";
    stream
        .record(&serde_json::json!({
            "event": "delta.tool_done",
            "payload": {
                "name": "generate_video",
                "result": {
                    "resources": [
                        {
                            "kind": "video",
                            "uri": cdn_url
                        }
                    ]
                }
            }
        }))
        .unwrap();

    assert_eq!(
        extract_video_url_from_stream(&stream),
        Some(cdn_url.to_string())
    );
}

#[test]
fn test_chat_stream_extracts_video_url_from_card_event() {
    let mut stream = acknowledged_chat_stream();
    let cdn_url = "https://video-lax3-1.xx.fbcdn.net/v/card.mp4";
    stream
        .record(&serde_json::json!({
            "event": "delta.card",
            "payload": {
                "card": {
                    "title": "Generated Video",
                    "video_url": cdn_url
                }
            }
        }))
        .unwrap();

    assert_eq!(
        extract_video_url_from_stream(&stream),
        Some(cdn_url.to_string())
    );
}

#[test]
fn test_chat_stream_extracts_video_url_from_attachment_event() {
    let mut stream = acknowledged_chat_stream();
    let cdn_url = "https://cdn.muse.ai/files/attachment.mp4";
    stream
        .record(&serde_json::json!({
            "event": "delta.attachment",
            "payload": {
                "attachment": {
                    "type": "video",
                    "url": cdn_url
                }
            }
        }))
        .unwrap();

    assert_eq!(
        extract_video_url_from_stream(&stream),
        Some(cdn_url.to_string())
    );
}

#[test]
fn test_chat_stream_extracts_video_url_from_media_event() {
    let mut stream = acknowledged_chat_stream();
    let cdn_url = "https://scontent.xx.fbcdn.net/v/media.mp4";
    stream
        .record(&serde_json::json!({
            "event": "delta.media",
            "payload": {
                "media": {
                    "url": cdn_url
                }
            }
        }))
        .unwrap();

    assert_eq!(
        extract_video_url_from_stream(&stream),
        Some(cdn_url.to_string())
    );
}

#[test]
fn test_chat_stream_extracts_video_url_from_presentation_card() {
    let mut stream = acknowledged_chat_stream();
    let cdn_url = "https://cdn.fbsbx.com/v/presentation_card.mp4";
    stream
        .record(&serde_json::json!({
            "event": "delta.presentation",
            "payload": {
                "data": {
                    "card": {
                        "video_url": cdn_url
                    }
                }
            }
        }))
        .unwrap();

    assert_eq!(
        extract_video_url_from_stream(&stream),
        Some(cdn_url.to_string())
    );
}

#[test]
fn test_chat_stream_extracts_video_url_from_message_attachment() {
    let mut stream = acknowledged_chat_stream();
    let cdn_url = "https://video.meta.com/v/message_attachment.mp4";
    stream
        .record(&serde_json::json!({
            "event": "message.attachment",
            "payload": {
                "url": cdn_url
            }
        }))
        .unwrap();

    assert_eq!(
        extract_video_url_from_stream(&stream),
        Some(cdn_url.to_string())
    );
}

#[test]
fn test_chat_stream_ignores_non_video_and_spoofed_urls_in_json() {
    let mut stream = acknowledged_chat_stream();
    stream
        .record(&serde_json::json!({
            "event": "delta.presentation",
            "payload": {
                "data": {
                    "card": {
                        "url": "https://attacker.com/malware.exe",
                        "spoofed": "https://video.xx.fbcdn.net.evil.example/fake.mp4",
                        "non_video": "https://video.xx.fbcdn.net/v/page.html"
                    }
                }
            }
        }))
        .unwrap();

    assert_eq!(extract_video_url_from_stream(&stream), None);
}

#[test]
fn test_chat_stream_extracts_video_url_from_deeply_nested_card_actions() {
    let mut stream = acknowledged_chat_stream();
    let cdn_url = "https://video.xx.fbcdn.net/v/t39/deep_action.mp4";
    stream
        .record(&serde_json::json!({
            "event": "delta.presentation",
            "payload": {
                "data": {
                    "card": {
                        "actions": [
                            {
                                "label": "Download",
                                "url": cdn_url
                            }
                        ]
                    }
                }
            }
        }))
        .unwrap();

    assert_eq!(
        extract_video_url_from_stream(&stream),
        Some(cdn_url.to_string())
    );
}
