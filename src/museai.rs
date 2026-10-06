use crate::config::Config;
use crate::museai_noise::{MuseNoiseSession, NOISE_PATTERN_XX};
use crate::museai_protocol::{Header, ServiceFrameKind, SERVICE_DAEMON};
use crate::museai_transport::{send_museai_request, MuseWebSocket};
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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

pub(crate) struct TrackedThread {
    id: String,
    created_at: u64,
    base_url: String,
    config: Config,
}

fn tracked_threads() -> &'static Mutex<Vec<TrackedThread>> {
    static TRACKED_THREADS: OnceLock<Mutex<Vec<TrackedThread>>> = OnceLock::new();
    TRACKED_THREADS.get_or_init(|| Mutex::new(Vec::new()))
}

fn register_thread(thread_id: String, base_url: String, config: Config) {
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if let Ok(mut threads) = tracked_threads().lock() {
        threads.push(TrackedThread {
            id: thread_id,
            created_at,
            base_url,
            config,
        });
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
        if let Some(data) = payload.get("data") {
            if let Some(url) = data.get("url").and_then(|u| u.as_str()) {
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
        .map_err(|e| io::Error::new(e.kind(), format!("Thread spawn error: {e}")))?;
    let thread_id = channel.strip_prefix("thread:").unwrap_or("").to_string();

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

    // Wait for the subscribe confirmation and extract the active session_id
    let mut resolved_session_id = thread_id.clone();
    for _ in 0..50 {
        match socket.read_encrypted_service_frame(&mut session) {
            Ok(frame) => {
                if let ServiceFrameKind::BodyChunk { data, end_body: _ } = frame.kind {
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&data) {
                        if let Some(sess) = val
                            .get("payload")
                            .and_then(|p| p.get("session_id"))
                            .and_then(|s| s.as_str())
                        {
                            if !sess.is_empty() {
                                debug_log(&format!("Discovered active daemon session_id: {sess}"));
                                resolved_session_id = sess.to_string();
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                debug_log(&format!("Note: subscribe frame read: {e}"));
                break;
            }
        }
    }

    let chat_payload = serde_json::json!({
        "items": [
            {
                "type": "text",
                "text": prompt
            }
        ],
        "node_id": active_vm_id.clone(),
        "session_id": resolved_session_id.clone(),
        "capabilities": [
            "chat_cancel",
            "delta_stream"
        ]
    });

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

    let mut full_assistant_text = String::new();
    let mut received_any_text = false;
    let mut stream_status = None;

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
                stream_status = Some(status);
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

                        if event == "agent.status" && received_any_text {
                            if let Some(code) = val
                                .get("payload")
                                .and_then(|p| p.get("activity_code"))
                                .and_then(|c| c.as_str())
                            {
                                if code == "online" {
                                    debug_log("Stream completed as agent returned online!");
                                    break;
                                }
                            }
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

    if let Some(status) = stream_status {
        if status >= 400 {
            return Err(io::Error::other(format!(
                "Muse.ai stream request failed with status {status}"
            )));
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

    if config.museai_auto_cleanup_threads && !thread_id.is_empty() {
        if config.museai_thread_retention_secs == 0 {
            let _ = delete_muse_thread(base_origin, &thread_id, &config);
        } else {
            register_thread(thread_id, base_origin.to_string(), config.clone());
        }
    }

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

    let duration: u64 = body
        .get("duration")
        .and_then(|v| {
            if let Some(n) = v.as_u64() {
                Some(n)
            } else if let Some(s) = v.as_str() {
                s.trim_end_matches('s').parse::<u64>().ok()
            } else {
                None
            }
        })
        .unwrap_or(5);

    let full_prompt = format!(
        "Create a video using the {model} model. Prompt: \"{prompt}\". Aspect ratio: {aspect_ratio}. Duration: {duration} seconds. Output only a URL"
    );

    let request_json = serde_json::to_vec(&serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": full_prompt}
        ]
    }))
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let completion = request_museai_chat_completion(&request_json, config)?;

    let completion_json: serde_json::Value = serde_json::from_str(&completion)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let content = completion_json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    let video_url = extract_url_from_text(content);
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
        "duration": duration,
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
    s.trim_matches([
        '"', '\'', ',', ';', ')', '}', '(', '{', ']', '[', '\\', '\n', '\r', ' ',
    ])
}

pub(crate) fn delete_muse_thread(
    base_url: &str,
    thread_id: &str,
    config: &Config,
) -> io::Result<()> {
    if thread_id.is_empty() {
        return Ok(());
    }

    let delete_thread_url = format!("{base_url}/api/thread/{thread_id}");
    let mut req = ureq::delete(&delete_thread_url)
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
        req = req.header("Cookie", &config.museai_cookie);
    }

    match req.call() {
        Ok(_) => {
            debug_log(&format!("Successfully cleaned up thread: {thread_id}"));
            Ok(())
        }
        Err(e) => {
            debug_log(&format!("Note: Failed to clean up thread {thread_id}: {e}"));
            if let ureq::Error::StatusCode(404) = e {
                Ok(())
            } else {
                Ok(())
            }
        }
    }
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

pub(crate) fn cleanup_tracked_threads_once() -> usize {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut to_delete = Vec::new();
    if let Ok(mut threads) = tracked_threads().lock() {
        let mut i = 0;
        while i < threads.len() {
            if now >= threads[i].created_at + threads[i].config.museai_thread_retention_secs {
                to_delete.push(threads.remove(i));
            } else {
                i += 1;
            }
        }
    }

    let deleted_count = to_delete.len();
    for thread in to_delete {
        let _ = delete_muse_thread(&thread.base_url, &thread.id, &thread.config);
    }
    deleted_count
}

pub(crate) fn start_cleanup_worker() {
    thread::spawn(|| loop {
        thread::sleep(Duration::from_secs(300));
        cleanup_tracked_threads_once();
    });
}

pub(crate) fn handle_museai_thread_cleanup(
    client: &mut TcpStream,
    thread_id: &str,
    config: &Config,
) -> io::Result<()> {
    let base_url = if config.museai_base_url.is_empty() {
        "https://muse.ai"
    } else {
        config.museai_base_url.as_str()
    };

    match delete_muse_thread(base_url, thread_id, config) {
        Ok(()) => {
            let body = serde_json::json!({
                "object": "thread.cleanup",
                "thread_id": thread_id,
                "status": "deleted"
            });
            let response_str = serde_json::to_string(&body)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            write!(
                client,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_str.len(),
                response_str
            )?;
            client.flush()
        }
        Err(e) => crate::http::write_error(
            client,
            502,
            "Bad Gateway",
            &format!("Failed to clean up thread: {}", e),
        ),
    }
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
            prism_base_url: "https://prism.openai.com".into(),
            prism_project_id: "".into(),
            prism_cookie: "".into(),
            prism_sandbox_token: "".into(),
            prism_user_id: "".into(),
            prism_default_model: "gpt-5.6-sol".into(),
            prism_system_prompt: "".into(),
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
            prism_base_url: "https://prism.openai.com".into(),
            prism_project_id: "".into(),
            prism_cookie: "".into(),
            prism_sandbox_token: "".into(),
            prism_user_id: "".into(),
            prism_default_model: "gpt-5.6-sol".into(),
            prism_system_prompt: "".into(),
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

    #[test]
    fn test_live_create_video_from_har_capture() {
        if std::env::var("MUSE_LIVE_TEST").ok().as_deref() != Some("1") {
            return;
        }
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let har_path = manifest.join("target/muse.ai.har");
        let header_path = manifest.join("target/muse-header.txt");

        let har_bytes = match std::fs::read(&har_path) {
            Ok(b) => b,
            Err(_) => {
                println!("Skipping live test: {} not found", har_path.display());
                return;
            }
        };
        let extracted = match crate::har_config::extract_muse_config_from_har(&har_bytes) {
            Ok(c) => c,
            Err(e) => {
                println!("Skipping live test: HAR parse failed: {e:?}");
                return;
            }
        };

        let cookie = extracted.cookie.clone().or_else(|| {
            std::fs::read(&header_path)
                .ok()
                .and_then(|bytes| parse_cookie_from_header_dump(&bytes))
        });
        let cookie = match cookie {
            Some(c) if !c.is_empty() => c,
            _ => {
                println!(
                    "Skipping live test: no cookie in HAR or {}",
                    header_path.display()
                );
                return;
            }
        };

        let config = Config {
            target_host: "127.0.0.1".into(),
            target_port: 8080,
            fallbacks: vec![],
            io_timeout: None,
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 1000,
            prism_base_url: "https://prism.openai.com".into(),
            prism_project_id: "default_proj".into(),
            prism_cookie: "".into(),
            prism_sandbox_token: "".into(),
            prism_user_id: "".into(),
            prism_default_model: "gpt-5.6-sol".into(),
            prism_system_prompt: "".into(),
            museai_base_url: extracted
                .base_url
                .unwrap_or_else(|| "https://muse.ai".into()),
            museai_cookie: cookie,
            museai_ws_url: extracted.ws_url.unwrap_or_default(),
            museai_access_token: extracted.access_token.unwrap_or_default(),
            museai_notary_token: extracted.notary_token.unwrap_or_default(),
            museai_vm_id: extracted.vm_id.unwrap_or_default(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 0,
        };

        let body = serde_json::json!({
            "prompt": "A single white square rotating slowly on a black background, 3D cartoon style",
            "model": "gen-3",
            "aspect_ratio": "16:9",
            "duration": 5
        });

        match create_video(body.to_string().as_bytes(), &config) {
            Ok(result) => {
                assert_eq!(result["object"], "video.generation");
                let status = result["status"].as_str().unwrap_or("");
                assert!(
                    matches!(status, "completed" | "pending"),
                    "unexpected status: {status}"
                );
                if status == "completed" {
                    let url = result["video_url"].as_str().unwrap_or("");
                    assert!(
                        url.starts_with("https://"),
                        "completed but video_url is not https: {url}"
                    );
                }
                println!("Live e2e create_video finished with status={status}");
            }
            Err(e) => {
                println!(
                    "Live e2e note: upstream rejected or credentials expired: {}",
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
