use std::io;
use uuid::Uuid;

pub fn extract_user_prompt(request_body: &[u8]) -> io::Result<String> {
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

fn extract_single_message_content(msg: &serde_json::Value) -> Option<String> {
    if let Some(content) = msg.get("content") {
        if let Some(s) = content.as_str() {
            return Some(s.to_string());
        } else if let Some(arr) = content.as_array() {
            let mut text = String::new();
            for item in arr {
                if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                    text.push_str(t);
                } else if let Some(t) = item.as_str() {
                    text.push_str(t);
                }
            }
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

pub fn extract_continuation_prompt(request_body: &[u8]) -> io::Result<String> {
    let json: serde_json::Value = serde_json::from_slice(request_body)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid JSON: {e}")))?;

    if let Some(prompt) = json.get("prompt").and_then(|p| p.as_str()) {
        let trimmed = prompt.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    if let Some(messages) = json.get("messages").and_then(|m| m.as_array()) {
        if let Some(last_user) = messages
            .iter()
            .rev()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        {
            if let Some(content) = extract_single_message_content(last_user) {
                let trimmed = content.trim().to_string();
                if !trimmed.is_empty() {
                    return Ok(trimmed);
                }
            }
        }
    }

    extract_user_prompt(request_body)
}

pub fn extract_thread_id(request_body: &[u8]) -> io::Result<Option<String>> {
    if request_body.is_empty() {
        return Ok(None);
    }
    let json: serde_json::Value = match serde_json::from_slice(request_body) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    extract_thread_id_from_value(&json)
}

pub fn extract_thread_id_from_value(json: &serde_json::Value) -> io::Result<Option<String>> {
    let candidate = json
        .get("reply_thread_id")
        .or_else(|| json.get("thread_id"))
        .or_else(|| json.get("session_id"))
        .or_else(|| json.get("reply_to_thread_id"))
        .or_else(|| {
            json.get("metadata").and_then(|m| {
                m.get("reply_thread_id")
                    .or_else(|| m.get("thread_id"))
                    .or_else(|| m.get("session_id"))
                    .or_else(|| m.get("reply_to_thread_id"))
            })
        });

    let raw = match candidate {
        Some(v) => match v.as_str() {
            Some(s) => s.trim(),
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "thread_id / reply_thread_id must be a string",
                ));
            }
        },
        None => return Ok(None),
    };

    if raw.is_empty() {
        return Ok(None);
    }

    let clean = raw
        .strip_prefix("thread_")
        .or_else(|| raw.strip_prefix("thread-"))
        .or_else(|| raw.strip_prefix("session_"))
        .or_else(|| raw.strip_prefix("session-"))
        .unwrap_or(raw);

    match Uuid::parse_str(clean) {
        Ok(parsed) => Ok(Some(parsed.to_string())),
        Err(e) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid thread_id / reply_thread_id '{raw}': {e}"),
        )),
    }
}

pub fn build_muse_chat_request(prompt: &str) -> serde_json::Value {
    build_muse_chat_request_with_session(prompt, None)
}

pub fn build_muse_chat_request_with_session(
    prompt: &str,
    session_id: Option<&str>,
) -> serde_json::Value {
    let session = session_id
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    serde_json::json!({
        "message": prompt,
        "node_id": Uuid::new_v4().to_string(),
        "session_id": session,
        "capabilities": ["chat_cancel", "delta_stream"]
    })
}
