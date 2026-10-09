use crate::config::Config;
use crate::museai::noise::{MuseNoiseSession, NOISE_PATTERN_XX};
use crate::museai::protocol::{Header, ServiceFrameKind, SERVICE_DAEMON};
use crate::museai::session::{bootstrap_museai_config, build_museai_ws_url};
use crate::museai::transport::MuseWebSocket;
use crate::museai::MuseApprovalRequired;
use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub(crate) const MUSE_POST_COMPLETION_WAIT_SECS: u64 = 300;

fn debug_log(msg: &str) {
    if std::env::var("MUSEAI_DEBUG").is_ok() {
        eprintln!("[museai_chat] {msg}");
    }
}

pub(crate) fn extract_user_prompt(request_body: &[u8]) -> io::Result<String> {
    let json: serde_json::Value = serde_json::from_slice(request_body)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid JSON: {e}")))?;

    if let Some(messages) = json.get("messages").and_then(|m| m.as_array()) {
        let mut full_prompt = String::new();
        for msg in messages {
            if let Some(role) = msg.get("role").and_then(|r| r.as_str()) {
                full_prompt.push_str(&format!("{}:\n", role));
            }
            if let Some(content) = msg.get("content") {
                if let Some(s) = content.as_str() {
                    full_prompt.push_str(s);
                } else if let Some(arr) = content.as_array() {
                    for item in arr {
                        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                            full_prompt.push_str(text);
                        } else if let Some(text) = item.as_str() {
                            full_prompt.push_str(text);
                        }
                    }
                }
            }
            full_prompt.push_str("\n\n");
        }
        let full_prompt = full_prompt.trim().to_string();
        if !full_prompt.is_empty() {
            return Ok(full_prompt);
        }
    }

    if let Some(prompt) = json.get("prompt").and_then(|p| p.as_str()) {
        return Ok(prompt.to_string());
    }

    Ok("Hello".to_string())
}

pub(crate) fn parse_assistant_content_from_json(val: &serde_json::Value) -> Option<String> {
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
        if let Some(data) = payload.get("data") {
            let url = data.get("url").and_then(|u| u.as_str()).or_else(|| {
                data.get("data")
                    .and_then(|nested| nested.get("url"))
                    .and_then(|u| u.as_str())
            });
            if let Some(url) = url {
                if !url.is_empty() {
                    return Some(format!("\nVideo URL: {url}\n"));
                }
            }
            if let Some(fallback) = data.get("fallback_text").and_then(|f| f.as_str()) {
                if !fallback.is_empty() {
                    return Some(format!("\nVideo: {fallback}\n"));
                }
            }
        }
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

#[derive(Default)]
pub(crate) struct MuseChatStream {
    pub(crate) pending: Vec<u8>,
    pub(crate) session_id: Option<String>,
    pub(crate) text: String,
    pub(crate) completed: bool,
}

impl MuseChatStream {
    pub(crate) fn push(&mut self, bytes: &[u8], ended: bool) -> io::Result<()> {
        const LIMIT: usize = 1024 * 1024;
        if self.pending.len().saturating_add(bytes.len()) > LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Muse chat record too large",
            ));
        }
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self
            .pending
            .iter()
            .position(|byte| *byte == b'\n')
            .or_else(|| (ended && !self.pending.is_empty()).then_some(self.pending.len()))
        {
            let line: Vec<u8> = self.pending.drain(..end).collect();
            if self.pending.first() == Some(&b'\n') {
                self.pending.remove(0);
            }
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let record: serde_json::Value = serde_json::from_slice(&line).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "Invalid Muse chat JSON record")
            })?;
            self.record(&record)?;
            if self.text.len() > LIMIT {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Muse chat output too large",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn record(&mut self, record: &serde_json::Value) -> io::Result<()> {
        #[cfg(test)]
        if std::env::var("MUSE_LIVE_RESUME_OWNED").ok().as_deref() == Some("1") {
            crate::museai::museai_tests::log_owned_record_schema(record);
        }
        let payload = record.get("payload").unwrap_or(record);
        let status = payload.get("status").and_then(|value| value.as_str());
        if let (Some(expected), Some(actual)) = (
            self.session_id.as_deref(),
            payload.get("session_id").and_then(|value| value.as_str()),
        ) {
            if actual != expected && record["type"] != "response" {
                return Ok(());
            }
        }

        if record.get("error").is_some_and(|error| !error.is_null())
            || record.get("ok").and_then(|value| value.as_bool()) == Some(false)
            || matches!(status, Some("failed" | "error" | "cancelled"))
        {
            return Err(io::Error::other(
                "Muse chat rejected or failed (private details suppressed)",
            ));
        }
        if self.session_id.is_none() {
            let result = record.get("result").unwrap_or(record);
            let id = result
                .get("session_id")
                .and_then(|value| value.as_str())
                .filter(|id| !id.is_empty());
            if let Some(id) = id {
                if Uuid::parse_str(id).is_err() || result["is_thread"].as_bool() != Some(true) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Muse did not acknowledge an isolated thread",
                    ));
                }
                self.session_id = Some(id.to_owned());
                debug_log("Fresh Muse session acknowledged");
                return Ok(());
            }
        }
        if record["type"] == "response" {
            if payload["subscribed"].as_bool() != Some(true)
                || payload["session_id"].as_str() != self.session_id.as_deref()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Muse subscription was not acknowledged for the requested session",
                ));
            }
            return Ok(());
        }
        let event = record
            .get("event")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let category = match event {
            "delta.text_append"
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
            | "delta.placeholder" => event,
            _ => "other",
        };
        debug_log(&format!("Chat event category={category}"));
        if event == "approvals.snapshot" {
            let requires_approval = payload
                .get("pending_approvals")
                .and_then(|value| value.as_array())
                .is_some_and(|approvals| {
                    approvals.iter().any(|approval| {
                        ["thread_id", "session_scope_id"].iter().any(|key| {
                            approval["scope"][key]
                                .as_str()
                                .is_some_and(|id| self.session_id.as_deref() == Some(id))
                        })
                    })
                });
            debug_log(&format!(
                "Approval snapshot requires_approval={requires_approval}"
            ));
            if requires_approval {
                debug_log("Scoped Muse permission approval is pending");
                return Err(io::Error::other(MuseApprovalRequired(
                    "Scoped Muse permission approval is pending".to_string(),
                )));
            }
        }
        if matches!(
            event,
            "delta.text_append" | "delta.message_done" | "message.assistant" | "delta.presentation"
        ) {
            if let Some(text) = parse_assistant_content_from_json(record) {
                self.text.push_str(&text);
            }
        }
        if matches!(event, "delta.message_done" | "task.complete") {
            self.completed = true;
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> io::Result<(String, String)> {
        if !self.completed || !self.pending.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Muse chat ended without a complete assistant response",
            ));
        }
        Ok((
            self.session_id.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Muse chat did not acknowledge the session",
                )
            })?,
            self.text,
        ))
    }
}

pub(crate) fn build_muse_chat_request(prompt: &str) -> serde_json::Value {
    serde_json::json!({
        "message": prompt,
        "node_id": Uuid::new_v4().to_string(),
        "session_id": Uuid::new_v4().to_string(),
        "capabilities": ["chat_cancel", "delta_stream"]
    })
}

pub(crate) fn extract_video_url_from_stream(stream: &MuseChatStream) -> Option<String> {
    let extracted = crate::museai::video::extract_url_from_text(&stream.text);
    (!extracted.is_empty()).then_some(extracted)
}

pub(crate) fn explicit_video_refusal(text: &str) -> bool {
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

pub(crate) fn request_museai_chat_completion(
    request_body: &[u8],
    config: &Config,
    wait_for_video_artifact: bool,
) -> io::Result<String> {
    let prompt = extract_user_prompt(request_body)?;
    let config = bootstrap_museai_config(config)?;

    let base_url = if config.museai_base_url.is_empty() {
        "https://muse.ai"
    } else {
        config.museai_base_url.as_str()
    };
    let base_origin = base_url.trim_end_matches('/');

    let req_id = Uuid::new_v4().to_string();
    let ws_url = build_museai_ws_url(&config, &req_id)?;

    let mut socket = MuseWebSocket::connect(&ws_url, base_origin, "")?;
    let mut session = MuseNoiseSession::new_initiator(NOISE_PATTERN_XX, None)
        .map_err(|e| io::Error::other(format!("Noise initiator error: {e}")))?;

    session
        .perform_client_handshake(&mut socket, &config.museai_notary_token)
        .map_err(|e| io::Error::other(format!("Noise handshake error: {e}")))?;

    let stream_id = 1;
    let headers = vec![
        Header {
            key: "Host".to_string(),
            value: "hatch.metaaivm.com".to_string(),
        },
        Header {
            key: "Content-Type".to_string(),
            value: "application/json".to_string(),
        },
        Header {
            key: "Accept".to_string(),
            value: "text/event-stream".to_string(),
        },
        Header {
            key: "x-request-id".to_string(),
            value: Uuid::new_v4().to_string(),
        },
        Header {
            key: "x-app-id".to_string(),
            value: "hatch-web".to_string(),
        },
        Header {
            key: "Accept-Language".to_string(),
            value: "en-US".to_string(),
        },
    ];

    let chat_payload = build_muse_chat_request(&prompt);

    let payload_bytes = serde_json::to_vec(&chat_payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    debug_log("Sending encrypted POST /chat/stream over Noise...");

    socket.send_encrypted_service_request(
        &mut session,
        SERVICE_DAEMON,
        stream_id,
        "POST",
        "/chat/stream",
        &headers,
        &payload_bytes,
    )?;

    let mut stream = MuseChatStream::default();
    let mut active_stream_id = stream_id;
    for _ in 0..1000 {
        let frame = socket.read_encrypted_service_frame(&mut session)?;
        if frame.stream_id != active_stream_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unexpected Muse response stream",
            ));
        }
        let (bytes, ended) = match frame.kind {
            ServiceFrameKind::Response {
                status,
                body,
                end_body,
                ..
            } => {
                debug_log(&format!("Chat response HTTP status={status}"));
                if !(200..300).contains(&status) {
                    return Err(io::Error::other(format!(
                        "Muse chat returned HTTP {status}"
                    )));
                }
                (body, end_body)
            }
            ServiceFrameKind::BodyChunk { data, end_body } => (data, end_body),
            ServiceFrameKind::Reset { code, .. } => {
                return Err(io::Error::other(format!("Muse chat reset code={code}")));
            }
        };
        stream.push(&bytes, ended)?;
        if ended && active_stream_id == stream_id && !stream.completed {
            let session_id = stream.session_id.as_deref().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Muse chat acknowledgement missing",
                )
            })?;
            #[cfg(test)]
            if std::env::var("MUSE_LIVE_TEST").ok().as_deref() == Some("1") {
                crate::museai::museai_tests::save_live_owned_session(session_id)?;
            }
            let subscription = serde_json::json!({
                "session_id": session_id,
                "after_stream_seq": 0,
                "after_chat_event_seq": 0,
                "capabilities": ["chat_cancel", "delta_stream"]
            });
            active_stream_id = 2;
            socket.send_encrypted_service_request(
                &mut session,
                SERVICE_DAEMON,
                active_stream_id,
                "POST",
                "/chat/subscribe",
                &headers,
                subscription.to_string().as_bytes(),
            )?;
            continue;
        }
        if stream.completed || ended {
            break;
        }
    }
    let (thread_id, full_assistant_text) = stream.finish()?;

    let mut video_url: Option<String> = {
        let extracted = crate::museai::video::extract_url_from_text(&full_assistant_text);
        (!extracted.is_empty()).then_some(extracted)
    };
    if wait_for_video_artifact
        && video_url.is_none()
        && !explicit_video_refusal(&full_assistant_text)
    {
        let deadline =
            std::time::Instant::now() + Duration::from_secs(MUSE_POST_COMPLETION_WAIT_SECS);
        let mut presentation_stream = MuseChatStream {
            session_id: Some(thread_id.clone()),
            ..Default::default()
        };
        while std::time::Instant::now() < deadline && video_url.is_none() {
            match socket.read_encrypted_service_frame(&mut session) {
                Ok(frame) => {
                    let (bytes, ended) = match frame.kind {
                        ServiceFrameKind::Response { body, end_body, .. } => (body, end_body),
                        ServiceFrameKind::BodyChunk { data, end_body } => (data, end_body),
                        ServiceFrameKind::Reset { .. } => break,
                    };
                    presentation_stream.push(&bytes, ended)?;
                    if let Some(extracted) = extract_video_url_from_stream(&presentation_stream) {
                        debug_log("Video artifact URL received");
                        video_url = Some(extracted);
                    }
                }
                Err(e) => {
                    debug_log(&format!("Video URL read error: {e}"));
                    break;
                }
            }
        }
    }

    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if config.museai_auto_cleanup_threads && !thread_id.is_empty() {
        if config.museai_thread_retention_secs == 0 {
            let _ = crate::museai::delete_muse_thread(base_origin, &thread_id, &config);
        } else {
            crate::museai::register_thread(thread_id, base_origin.to_string(), config.clone());
        }
    }

    let mut full_assistant_text = full_assistant_text;
    if let Some(url) = &video_url {
        if !full_assistant_text.contains(url.as_str()) {
            full_assistant_text.push_str(&format!("\nVideo URL: {url}\n"));
        }
    }
    let completion_response = serde_json::json!({
        "id": format!("chatcmpl-{}", Uuid::new_v4()),
        "object": "chat.completion",
        "created": created,
        "model": "muse",
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": full_assistant_text
                },
                "finish_reason": "stop"
            }
        ],
        "usage": {
            "prompt_tokens": prompt.len() / 4 + 1,
            "completion_tokens": full_assistant_text.len() / 4 + 1,
            "total_tokens": (prompt.len() + full_assistant_text.len()) / 4 + 2
        }
    });

    serde_json::to_string(&completion_response)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
