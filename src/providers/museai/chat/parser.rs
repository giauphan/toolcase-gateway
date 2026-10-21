use super::stream::MuseChatStream;
use crate::providers::museai::video::{extract_url_from_text, is_supported_video_url};

pub fn parse_assistant_content_from_json(val: &serde_json::Value) -> Option<String> {
    if let Some(event) = val.get("event").and_then(|e| e.as_str()) {
        if event == "task.status" || event == "agent.status" || event == "approvals.snapshot" {
            return None;
        }
    }

    if let Some(content) = val.get("content").and_then(|c| c.as_str()) {
        if !content.is_empty() {
            return Some(content.to_string());
        }
    }
    if let Some(text) = val.get("text").and_then(|t| t.as_str()) {
        if !text.is_empty() {
            return Some(text.to_string());
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

    if let Some(payload) = val.get("payload") {
        if let Some(text) = payload.get("text").and_then(|t| t.as_str()) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
        if let Some(content) = payload.get("content").and_then(|c| c.as_str()) {
            if !content.is_empty() {
                return Some(content.to_string());
            }
        }
        if let Some(transcript) = payload.get("transcript") {
            if let Some(messages) = transcript.get("messages").and_then(|m| m.as_array()) {
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
                    return Some(combined);
                }
            }
        }
    }
    if let Some(message) = val.get("message") {
        if let Some(text) = parse_assistant_content_from_json(message) {
            return Some(text);
        }
    }

    if let Some(url) = find_video_url_in_json(val, 0) {
        return Some(format!("\nVideo URL: {url}\n"));
    }

    if let Some(fallback) = val
        .get("payload")
        .and_then(|p| p.get("data"))
        .and_then(|d| d.get("fallback_text"))
        .and_then(|f| f.as_str())
        .or_else(|| {
            val.get("payload")
                .and_then(|p| p.get("fallback_text"))
                .and_then(|f| f.as_str())
        })
        .or_else(|| val.get("fallback_text").and_then(|f| f.as_str()))
    {
        if !fallback.is_empty() {
            return Some(format!("\nVideo: {fallback}\n"));
        }
    }
    if let Some(events) = val.get("events").and_then(|e| e.as_array()) {
        let mut combined = String::new();
        for ev in events {
            if let Some(text) = parse_assistant_content_from_json(ev) {
                combined.push_str(&text);
            }
        }
        if !combined.is_empty() {
            return Some(combined);
        }
    }
    if let Some(items) = val.get("items").and_then(|i| i.as_array()) {
        let mut combined = String::new();
        for item in items {
            if let Some(text) = parse_assistant_content_from_json(item) {
                combined.push_str(&text);
            }
        }
        if !combined.is_empty() {
            return Some(combined);
        }
    }
    None
}

pub fn find_video_url_in_json(val: &serde_json::Value, depth: usize) -> Option<String> {
    if depth > 10 {
        return None;
    }
    match val {
        serde_json::Value::String(s) => {
            let extracted = extract_url_from_text(s);
            if !extracted.is_empty() && is_supported_video_url(&extracted) {
                Some(extracted)
            } else {
                None
            }
        }
        serde_json::Value::Object(map) => {
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
                    let extracted = extract_url_from_text(s);
                    if !extracted.is_empty() && is_supported_video_url(&extracted) {
                        return Some(extracted);
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
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(url) = find_video_url_in_json(item, depth + 1) {
                    return Some(url);
                }
            }
            None
        }
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
