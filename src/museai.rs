use crate::config::Config;
use crate::museai_noise::{MuseNoiseSession, NOISE_PATTERN_XX};
use crate::museai_protocol::{Header, ServiceFrameKind, SERVICE_DAEMON};
use crate::museai_transport::{send_museai_request, MuseWebSocket};
use std::io::{self, Write};
use std::net::TcpStream;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

fn debug_log(msg: &str) {
    if std::env::var("MUSEAI_DEBUG").is_ok() {
        eprintln!("[museai] {msg}");
    }
}

fn append_query_component(url: &mut String, key: &str, value: &str) {
    url.push(if url.contains('?') { '&' } else { '?' });
    url.push_str(key);
    url.push('=');
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                url.push(char::from(byte));
            }
            _ => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                url.push('%');
                url.push(char::from(HEX[(byte >> 4) as usize]));
                url.push(char::from(HEX[(byte & 0x0f) as usize]));
            }
        }
    }
}

pub(crate) fn build_museai_ws_url(config: &Config, request_id: &str) -> io::Result<String> {
    let configured = config.museai_ws_url.as_str();

    let (base_url, query_str) = if let Some(idx) = configured.find('?') {
        (&configured[..idx], &configured[idx + 1..])
    } else {
        (configured, "")
    };

    let mut parsed_query = std::collections::HashMap::new();
    for pair in query_str.split('&').filter(|s| !s.is_empty()) {
        let mut parts = pair.splitn(2, '=');
        if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
            parsed_query.insert(k.to_string(), v.to_string());
        }
    }

    let shared = base_url.contains("metaaivm.com") && !base_url.contains("hatch.metaaivm.com");
    let mut url = if shared || base_url.is_empty() {
        "wss://hatch.metaaivm.com/v1/noise".to_string()
    } else {
        base_url.to_string()
    };
    if url.ends_with('/') {
        url.pop();
    }
    if !url.ends_with("/v1/noise") {
        url.push_str("/v1/noise");
    }

    let vm_id = if let Some(v) = parsed_query.get("vm_id") {
        v.to_string()
    } else if !config.museai_vm_id.is_empty() && config.museai_vm_id != "." {
        config.museai_vm_id.as_str().to_string()
    } else if shared {
        base_url
            .split("://")
            .nth(1)
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or(".")
            .to_string()
    } else {
        ".".to_string()
    };

    let auth_token = if let Some(v) = parsed_query.get("auth_token") {
        v.to_string()
    } else {
        config.museai_access_token.clone()
    };
    if auth_token.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Muse.ai WebSocket configuration is missing auth_token",
        ));
    }

    let notary_token = if let Some(v) = parsed_query.get("notary_token") {
        v.to_string()
    } else {
        config.museai_notary_token.clone()
    };

    let app_id = parsed_query
        .get("app_id")
        .map(|s| s.as_str())
        .unwrap_or("hatch-web");

    append_query_component(&mut url, "vm_id", &vm_id);
    append_query_component(&mut url, "auth_token", &auth_token);
    if !notary_token.is_empty() {
        append_query_component(&mut url, "notary_token", &notary_token);
    }
    append_query_component(&mut url, "app_id", app_id);
    append_query_component(&mut url, "request_id", request_id);

    Ok(url)
}

fn extract_user_prompt(request_body: &[u8]) -> io::Result<String> {
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

fn parse_assistant_content_from_json(val: &serde_json::Value) -> Option<String> {
    if let Some(event) = val.get("event").and_then(|e| e.as_str()) {
        if event == "delta.message_done"
            || event == "task.status"
            || event == "agent.status"
            || event == "approvals.snapshot"
        {
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
                    if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                        for c in content {
                            if let Some(text) = c.get("text").and_then(|t| t.as_str()) {
                                combined.push_str(text);
                            }
                        }
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
        .header("x-nextjs-data", "true");

    if !config.museai_cookie.is_empty() {
        req = req.header("Cookie", &config.museai_cookie);
    }

    let response = req
        .send("[]")
        .map_err(|e| io::Error::other(format!("Thread creation request failed: {e}")))?;
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
    let channel = format!("thread:{}", thread_id);
    Ok(channel)
}

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

pub(crate) fn bootstrap_museai_config(config: &Config) -> io::Result<Config> {
    if !config.museai_access_token.is_empty() && !config.museai_notary_token.is_empty() {
        return Ok(config.clone());
    }

    let base_url = if config.museai_base_url.is_empty() {
        "https://muse.ai"
    } else {
        config.museai_base_url.trim_end_matches('/')
    };

    let vm_address = if config.museai_ws_url.is_empty() {
        "wss://hatch.metaaivm.com/v1/noise".to_string()
    } else {
        let configured = config.museai_ws_url.clone();
        if let Some(idx) = configured.find('?') {
            configured[..idx].to_string()
        } else {
            configured
        }
    };

    let mut body = serde_json::Map::new();
    body.insert(
        "vmAddress".to_string(),
        serde_json::Value::String(vm_address),
    );

    let shared_vm = if config.museai_ws_url.is_empty() {
        if config.museai_base_url.contains("metaaivm.com")
            && !config.museai_base_url.contains("hatch.metaaivm.com")
        {
            config
                .museai_base_url
                .split("://")
                .nth(1)
                .unwrap_or("")
                .split('.')
                .next()
                .unwrap_or("")
                .to_string()
        } else {
            "".to_string()
        }
    } else {
        if config.museai_ws_url.contains("metaaivm.com")
            && !config.museai_ws_url.contains("hatch.metaaivm.com")
        {
            config
                .museai_ws_url
                .split("://")
                .nth(1)
                .unwrap_or("")
                .split('.')
                .next()
                .unwrap_or("")
                .to_string()
        } else {
            "".to_string()
        }
    };

    let active_vm_id = if !config.museai_vm_id.is_empty() && config.museai_vm_id != "." {
        config.museai_vm_id.clone()
    } else {
        shared_vm
    };

    if !active_vm_id.is_empty() {
        body.insert(
            "vmName".to_string(),
            serde_json::Value::String(active_vm_id.clone()),
        );

        let wake_body = serde_json::json!({
            "vm_id": active_vm_id.clone(),
            "retry_count": 0,
            "connect_attempt_id": uuid::Uuid::new_v4().to_string()
        });
        let mut wake_req = ureq::post(&format!("{base_url}/api/hatch/vm/wake"))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header("Origin", base_url)
            .header("Referer", &format!("{base_url}/"))
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36")
            .header("sec-ch-ua", r#""Brave";v="153", "Not_A Brand";v="8", "Chromium";v="153""#)
            .header("sec-ch-ua-mobile", "?0")
            .header("sec-ch-ua-platform", r#""Windows""#)
            .header("sec-fetch-dest", "empty")
            .header("sec-fetch-mode", "cors")
            .header("sec-fetch-site", "same-origin");

        if !config.museai_cookie.is_empty() {
            wake_req = wake_req.header("Cookie", &config.museai_cookie);
        }

        match wake_req.send_json(wake_body) {
            Ok(response) => {
                if let Ok(json) = response.into_body().read_json::<serde_json::Value>() {
                    debug_log(&format!("Note: Woke VM. Response: {}", json));
                }
            }
            Err(e) => {
                debug_log(&format!("Note: Failed to wake VM. Error: {}", e));
            }
        }
    }

    let mut request = ureq::post(&format!("{base_url}/api/hatch/token"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("Origin", base_url)
        .header("Referer", &format!("{base_url}/"))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36")
        .header("sec-ch-ua", r#""Brave";v="153", "Not_A Brand";v="8", "Chromium";v="153""#)
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", r#""Windows""#)
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin");

    if !config.museai_cookie.is_empty() {
        request = request.header("Cookie", &config.museai_cookie);
    }

    let mut bootstrapped = config.clone();

    match request.send_json(serde_json::Value::Object(body)) {
        Ok(response) => {
            if let Ok(session) = response.into_body().read_json::<serde_json::Value>() {
                if let Some(token) = session.get("token").and_then(|v| v.as_str()) {
                    bootstrapped.museai_access_token = token.to_owned();
                }
                if let Some(notary) = session.get("notary_token").and_then(|v| v.as_str()) {
                    bootstrapped.museai_notary_token = notary.to_owned();
                }
                if !bootstrapped.museai_access_token.is_empty() {
                    return Ok(bootstrapped);
                }
            }
        }
        Err(e) => {
            debug_log(&format!("Note: POST /api/hatch/token failed: {e}"));
        }
    }

    let mut request = ureq::get(&format!("{base_url}/api/session"))
        .header("Accept", "application/json")
        .header("Origin", base_url)
        .header("Referer", &format!("{base_url}/"));
    if !config.museai_cookie.is_empty() {
        request = request.header("Cookie", &config.museai_cookie);
    }
    let response = request
        .call()
        .map_err(|error| io::Error::other(format!("Muse.ai session bootstrap failed: {error}")))?;
    let session: serde_json::Value = response
        .into_body()
        .read_json()
        .map_err(|error| io::Error::other(format!("Muse.ai session response invalid: {error}")))?;

    if let Some(token) = session.get("auth_token").and_then(|value| value.as_str()) {
        bootstrapped.museai_access_token = token.to_owned();
    }
    if let Some(vm_id) = session.get("vm_id").and_then(|value| value.as_str()) {
        if bootstrapped.museai_vm_id.is_empty() {
            bootstrapped.museai_vm_id = vm_id.to_owned();
        }
    }
    if let Some(endpoint) = session.get("endpoint_url").and_then(|value| value.as_str()) {
        if bootstrapped.museai_ws_url.is_empty() {
            bootstrapped.museai_ws_url = endpoint.to_owned();
        }
    }
    Ok(bootstrapped)
}

pub(crate) fn request_museai_chat_completion(
    request_body: &[u8],
    config: &Config,
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

    let mut socket = MuseWebSocket::connect(&ws_url, base_origin, &config.museai_cookie)?;
    let mut session = MuseNoiseSession::new_initiator(NOISE_PATTERN_XX, None)
        .map_err(|e| io::Error::other(format!("Noise initiator error: {e}")))?;

    session
        .perform_client_handshake(&mut socket, &config.museai_notary_token)
        .map_err(|e| io::Error::other(format!("Noise handshake error: {e}")))?;

    let shared = base_url.contains("metaaivm.com") && !base_url.contains("hatch.metaaivm.com");
    let active_vm_id = if !config.museai_vm_id.is_empty() && config.museai_vm_id != "." {
        config.museai_vm_id.clone()
    } else if shared {
        base_url
            .split("://")
            .nth(1)
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or("")
            .to_string()
    } else {
        uuid::Uuid::new_v4().to_string()
    };

    // Spawn an isolated thread for this request to prevent cross-request context leakage
    let channel = spawn_muse_thread(base_origin, &config)
        .map_err(|e| io::Error::other(format!("Thread spawn error: {e}")))?;
    let thread_id = channel.strip_prefix("thread:").unwrap_or("").to_string();

    let chat_payload = serde_json::json!({
        "items": [
            {
                "type": "text",
                "text": prompt
            }
        ],
        "node_id": active_vm_id.clone(),
        "thread_id": thread_id.clone(),
        "session_id": thread_id.clone(),
        "chat_id": thread_id.clone(),
        "capabilities": [
            "chat_cancel",
            "delta_stream"
        ]
    });

    let payload_bytes = serde_json::to_vec(&chat_payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

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

    debug_log("Sending encrypted POST /chat/subscribe over Noise...");
    let sub_payload = serde_json::json!({
        "channel": channel
    });
    let sub_bytes = serde_json::to_vec(&sub_payload).unwrap();
    socket.send_encrypted_service_request(
        &mut session,
        SERVICE_DAEMON,
        2, // stream_id = 2 for subscribe
        "POST",
        "/chat/subscribe",
        &headers,
        &sub_bytes,
    )?;

    // Give it a small delay
    std::thread::sleep(std::time::Duration::from_millis(50));

    debug_log("Sending encrypted POST /api/chats/stream over Noise...");

    socket.send_encrypted_service_request(
        &mut session,
        SERVICE_DAEMON,
        stream_id,
        "POST",
        "/api/chats/stream",
        &headers,
        &payload_bytes,
    )?;

    let mut full_assistant_text = String::new();
    let mut received_any_text = false;

    debug_log("Waiting for encrypted response frames...");
    for _ in 0..1000 {
        let frame = match socket.read_encrypted_service_frame(&mut session) {
            Ok(f) => f,
            Err(e) if received_any_text => {
                debug_log(&format!("Error reading frame after text received: {}", e));
                break;
            }
            Err(e) => {
                // If it hits EOF/timeout, don't fail immediately if we got SOME text
                return Err(e);
            }
        };

        debug_log(&format!("Received encrypted frame: {:?}", frame.kind));

        match frame.kind {
            ServiceFrameKind::Response {
                body,
                end_body: _,
                status,
                headers: _,
            } => {
                debug_log(&format!("Response status: {}", status));
                if !body.is_empty() {
                    let s = String::from_utf8_lossy(&body);
                    debug_log(&format!("Response body length: {} bytes", s.len()));
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&body) {
                        if let Some(text) = parse_assistant_content_from_json(&val) {
                            debug_log(&format!("Parsed chunk length: {} chars", text.len()));
                            full_assistant_text.push_str(&text);
                            received_any_text = true;
                        }
                    }
                }
                // if end_body && received_any_text { break; }
            }
            ServiceFrameKind::BodyChunk { data, end_body } => {
                let s = String::from_utf8_lossy(&data);
                debug_log(&format!(
                    "BodyChunk length: {} bytes (end_body={})",
                    s.len(),
                    end_body
                ));
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&data) {
                    if let Some(text) = parse_assistant_content_from_json(&val) {
                        debug_log(&format!("Parsed chunk length: {} chars", text.len()));
                        full_assistant_text.push_str(&text);
                        received_any_text = true;
                    }

                    if let Some(event) = val.get("event").and_then(|e| e.as_str()) {
                        if (event == "delta.message_done" || event == "task.complete")
                            && received_any_text
                        {
                            debug_log("Stream completed successfully via event!");
                            break;
                        }
                    }

                    if let Some(status) = val
                        .get("payload")
                        .and_then(|p| p.get("status"))
                        .and_then(|s| s.as_str())
                    {
                        if status == "completed" && received_any_text {
                            debug_log("Stream completed successfully!");
                            break;
                        }
                    }
                }
            }
            ServiceFrameKind::Reset { .. } => {
                break;
            }
        }
    }

    if full_assistant_text.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "No assistant response received from Muse.ai",
        ));
    }

    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

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

const SUPPORTED_MODELS: [&str; 3] = ["gen-3", "gen-2", "kling"];
const SUPPORTED_ASPECT_RATIOS: [&str; 5] = ["16:9", "9:16", "1:1", "5:4", "4:3"];

pub(crate) fn create_video(request_body: &[u8], config: &Config) -> io::Result<serde_json::Value> {
    let body: serde_json::Value = if request_body.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(request_body).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid JSON: {e}"))
        })?
    };

    let prompt = body
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing 'prompt' field"))?
        .trim()
        .to_string();
    if prompt.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "prompt must not be empty",
        ));
    }

    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("gen-3");
    if !SUPPORTED_MODELS.contains(&model) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unsupported model '{model}'; supported: {}",
                SUPPORTED_MODELS.join(", ")
            ),
        ));
    }

    let aspect_ratio = body
        .get("aspect_ratio")
        .and_then(|v| v.as_str())
        .unwrap_or("16:9");
    if !SUPPORTED_ASPECT_RATIOS.contains(&aspect_ratio) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unsupported aspect_ratio '{aspect_ratio}'; supported: {}",
                SUPPORTED_ASPECT_RATIOS.join(", ")
            ),
        ));
    }

    let full_prompt = format!(
        "Create a video using the {model} model. Prompt: \"{prompt}\". Aspect ratio: {aspect_ratio}. Duration: 5 seconds. Output only a URL"
    );

    let request_json = serde_json::to_vec(&serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": full_prompt}
        ]
    }))
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let completion = request_museai_chat_completion(&request_json, config)?;

    let video_url = extract_url_from_text(&completion);
    let status = if video_url.is_empty() {
        "pending".to_string()
    } else {
        "completed".to_string()
    };

    Ok(serde_json::json!({
        "id": format!("video-{}", Uuid::new_v4()),
        "object": "video.generation",
        "created": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        "model": model,
        "prompt": prompt,
        "aspect_ratio": aspect_ratio,
        "status": status,
        "video_url": video_url
    }))
}

fn extract_url_from_text(text: &str) -> String {
    let urls: Vec<&str> = text
        .split_whitespace()
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
        .collect();
    for url in &urls {
        let cleaned = trim_trailing_punct(url);
        if cleaned.contains(".mp4") || cleaned.contains(".mov") || cleaned.contains(".webm") {
            return cleaned.to_string();
        }
    }
    if let Some(url) = urls.first() {
        return trim_trailing_punct(url).to_string();
    }
    String::new()
}

fn trim_trailing_punct(s: &str) -> &str {
    s.trim_end_matches(['"', '\'', ',', ';', ')', '}'])
}

pub(crate) fn write_chat_completion(client: &mut TcpStream, response: &str) -> io::Result<()> {
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.len(),
        response
    )?;
    client.flush()
}

pub(crate) fn handle_create_video(
    client: &mut TcpStream,
    request_body: &[u8],
    config: &Config,
) -> io::Result<()> {
    match create_video(request_body, config) {
        Ok(result) => {
            let response_str = serde_json::to_string(&result)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            write!(
                client,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_str.len(),
                response_str
            )?;
            client.flush()
        }
        Err(e) => {
            let (status_code, error_type) = match e.kind() {
                io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => (400, "Bad Request"),
                io::ErrorKind::PermissionDenied => (401, "Unauthorized"),
                io::ErrorKind::NotFound => (404, "Not Found"),
                io::ErrorKind::TimedOut => (504, "Gateway Timeout"),
                _ => (502, "Bad Gateway"),
            };
            crate::http::write_error(
                client,
                status_code,
                error_type,
                &format!("Failed to create video: {}", e),
            )
        }
    }
}

pub(crate) fn handle_museai_v1(
    client: &mut TcpStream,
    request_body: &[u8],
    config: &Config,
) -> io::Result<()> {
    let (url, method, body_json) =
        crate::museai_business::build_museai_request(request_body, config)?;

    let result = send_museai_request(&url, &method, &body_json, config);

    match result {
        Ok(upstream_resp) => {
            let status_text = match upstream_resp.status {
                200 => "OK",
                201 => "Created",
                204 => "No Content",
                400 => "Bad Request",
                401 => "Unauthorized",
                403 => "Forbidden",
                404 => "Not Found",
                500 => "Internal Server Error",
                _ => "OK",
            };

            let out = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                upstream_resp.status,
                status_text,
                upstream_resp.content_type,
                upstream_resp.body.len(),
                upstream_resp.body
            );
            client.write_all(out.as_bytes())?;
            client.flush()
        }
        Err(err) => crate::http::write_error(
            client,
            502,
            "Bad Gateway",
            &format!("Failed to proxy to Muse.ai: {}", err),
        ),
    }
}

#[cfg(test)]
mod museai_tests {
    use super::*;

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
        // Fails back to the first URL if no video extension is found.
        assert_eq!(
            extract_url_from_text(no_video_ext),
            "https://muse.ai/some-link"
        );

        let no_url = "Just some text without links";
        assert_eq!(extract_url_from_text(no_url), "");
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
            prism_base_url: "https://prism.openai.com".into(),
            prism_project_id: "".into(),
            prism_cookie: "".into(),
            prism_sandbox_token: "".into(),
            prism_user_id: "".into(),
            prism_default_model: "gpt-5.6-sol".into(),
            prism_system_prompt: "".into(),
            museai_base_url: "https://muse.ai".into(),
            museai_cookie: "".into(),
            museai_ws_url: "".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
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
}
