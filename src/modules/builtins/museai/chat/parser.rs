use super::stream::MuseChatStream;
use crate::providers::museai::video::{extract_url_from_text, is_raw_video_artifact_url};

fn is_ignorable_event(val: &serde_json::Value) -> bool {
    if let Some(event) = val.get("event").and_then(|e| e.as_str()) {
        matches!(event, "task.status" | "agent.status" | "approvals.snapshot")
    } else {
        false
    }
}

fn extract_direct_string_fields(val: &serde_json::Value) -> Option<String> {
    for key in &["content", "text"] {
        if let Some(s) = val.get(*key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    if let Some(delta) = val
        .get("delta")
        .and_then(|d| d.get("text"))
        .and_then(|t| t.as_str())
    {
        if !delta.is_empty() {
            return Some(delta.to_string());
        }
    }
    None
}

fn extract_transcript_messages(messages: &[serde_json::Value]) -> Option<String> {
    let mut combined = String::new();
    for msg in messages {
        if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
            continue;
        }
        if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
            for c in content {
                if let Some(text) = c.get("text").and_then(|t| t.as_str()) {
                    combined.push_str(text);
                }
            }
        } else if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
            combined.push_str(text);
        }
    }
    if !combined.is_empty() {
        Some(combined)
    } else {
        None
    }
}

fn extract_payload_content(val: &serde_json::Value) -> Option<String> {
    let payload = val.get("payload")?;
    for key in &["text", "content"] {
        if let Some(s) = payload.get(*key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    if let Some(transcript) = payload.get("transcript") {
        if let Some(messages) = transcript.get("messages").and_then(|m| m.as_array()) {
            if let Some(text) = extract_transcript_messages(messages) {
                return Some(text);
            }
        }
    }
    None
}

fn extract_fallback_video_text(val: &serde_json::Value) -> Option<String> {
    let fallback = val
        .get("payload")
        .and_then(|p| p.get("data"))
        .and_then(|d| d.get("fallback_text"))
        .and_then(|f| f.as_str())
        .or_else(|| {
            val.get("payload")
                .and_then(|p| p.get("fallback_text"))
                .and_then(|f| f.as_str())
        })
        .or_else(|| val.get("fallback_text").and_then(|f| f.as_str()))?;

    if !fallback.is_empty() {
        Some(format!("\nVideo: {fallback}\n"))
    } else {
        None
    }
}

fn extract_array_contents(val: &serde_json::Value, key: &str) -> Option<String> {
    let items = val.get(key).and_then(|v| v.as_array())?;
    let mut combined = String::new();
    for item in items {
        if let Some(text) = parse_assistant_content_from_json(item) {
            combined.push_str(&text);
        }
    }
    if !combined.is_empty() {
        Some(combined)
    } else {
        None
    }
}

pub fn parse_assistant_content_from_json(val: &serde_json::Value) -> Option<String> {
    if is_ignorable_event(val) {
        return None;
    }
    if let Some(text) = extract_direct_string_fields(val) {
        return Some(text);
    }
    if let Some(text) = extract_payload_content(val) {
        return Some(text);
    }
    if let Some(message) = val.get("message") {
        if let Some(text) = parse_assistant_content_from_json(message) {
            return Some(text);
        }
    }
    if let Some(url) = find_video_url_in_json(val, 0) {
        return Some(format!("\nVideo URL: {url}\n"));
    }
    if let Some(fallback) = extract_fallback_video_text(val) {
        return Some(fallback);
    }
    if let Some(events) = extract_array_contents(val, "events") {
        return Some(events);
    }
    if let Some(items) = extract_array_contents(val, "items") {
        return Some(items);
    }
    None
}

fn inspect_video_candidate(s: &str) -> Option<String> {
    let extracted = extract_url_from_text(s);
    let candidate = if !extracted.is_empty() {
        extracted
    } else if is_raw_video_artifact_url(s) {
        s.to_string()
    } else {
        String::new()
    };
    if !candidate.is_empty() && is_raw_video_artifact_url(&candidate) {
        Some(candidate)
    } else {
        None
    }
}

fn search_video_object(
    map: &serde_json::Map<String, serde_json::Value>,
    depth: usize,
) -> Option<String> {
    for key in &[
        "url",
        "video_url",
        "videoUrl",
        "uri",
        "src",
        "path",
        "download_url",
        "source_url",
        "media_url",
    ] {
        if let Some(serde_json::Value::String(s)) = map.get(*key) {
            if let Some(candidate) = inspect_video_candidate(s) {
                return Some(candidate);
            }
        }
    }
    for key in &[
        "data",
        "payload",
        "card",
        "media",
        "attachment",
        "artifact",
        "result",
        "output",
        "resources",
        "attachments",
        "cards",
        "items",
        "events",
        "metadata",
        "widgets",
        "presentations",
    ] {
        if let Some(child) = map.get(*key) {
            if let Some(url) = find_video_url_in_json(child, depth + 1) {
                return Some(url);
            }
        }
    }
    for (key, child) in map {
        if !matches!(
            key.as_str(),
            "data"
                | "payload"
                | "card"
                | "media"
                | "attachment"
                | "artifact"
                | "result"
                | "output"
                | "resources"
                | "attachments"
                | "cards"
                | "items"
                | "events"
                | "metadata"
                | "widgets"
                | "presentations"
                | "url"
                | "video_url"
                | "videoUrl"
                | "uri"
                | "src"
                | "path"
                | "download_url"
                | "source_url"
                | "media_url"
                | "session_id"
                | "thread_id"
                | "message_id"
                | "id"
        ) {
            if let Some(url) = find_video_url_in_json(child, depth + 1) {
                return Some(url);
            }
        }
    }
    None
}

fn search_video_array(arr: &[serde_json::Value], depth: usize) -> Option<String> {
    for item in arr {
        if let Some(url) = find_video_url_in_json(item, depth + 1) {
            return Some(url);
        }
    }
    None
}

pub fn find_video_url_in_json(val: &serde_json::Value, depth: usize) -> Option<String> {
    if depth > 10 {
        return None;
    }
    match val {
        serde_json::Value::String(s) => inspect_video_candidate(s),
        serde_json::Value::Object(map) => search_video_object(map, depth),
        serde_json::Value::Array(arr) => search_video_array(arr, depth),
        _ => None,
    }
}

pub fn extract_video_url_from_stream(stream: &MuseChatStream) -> Option<String> {
    let extracted = crate::providers::museai::video::extract_url_from_text(&stream.text);
    (!extracted.is_empty()).then_some(extracted)
}

pub fn explicit_video_refusal(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    let mentions_video_action = text.contains("video generation")
        || text.contains("generate a video")
        || text.contains("generate video")
        || text.contains("generate videos")
        || text.contains("create a video")
        || text.contains("create video")
        || text.contains("create videos");
    let mentions_inability = text.contains("unavailable")
        || text.contains("cannot")
        || text.contains("can't")
        || text.contains("not available")
        || text.contains("unable")
        || text.contains("don't have")
        || text.contains("do not have");
    mentions_video_action && mentions_inability
}
