use crate::config::Config;
use crate::museai_noise::{MuseNoiseSession, NOISE_PATTERN_XX};
use crate::museai_protocol::{Header, ServiceFrameKind, SERVICE_DAEMON};
use crate::museai_transport::{send_museai_request, MuseWebSocket};
use std::io::{self, Write};
use std::net::TcpStream;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

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
        for msg in messages.iter().rev() {
            if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                    return Ok(content.to_string());
                }
            }
        }
    }

    if let Some(prompt) = json.get("prompt").and_then(|p| p.as_str()) {
        return Ok(prompt.to_string());
    }

    Ok("Hello".to_string())
}

fn parse_assistant_content_from_json(val: &serde_json::Value) -> Option<String> {
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
    if let Some(delta) = val.get("delta").and_then(|d| d.get("text")).and_then(|t| t.as_str()) {
        if !delta.is_empty() {
            return Some(delta.to_string());
        }
    }
    if let Some(payload) = val.get("payload") {
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
    body.insert("vmAddress".to_string(), serde_json::Value::String(vm_address));

    let shared_vm = if config.museai_ws_url.is_empty() {
        if config.museai_base_url.contains("metaaivm.com") && !config.museai_base_url.contains("hatch.metaaivm.com") {
            config.museai_base_url.split("://").nth(1).unwrap_or("").split('.').next().unwrap_or("").to_string()
        } else {
            "".to_string()
        }
    } else {
        if config.museai_ws_url.contains("metaaivm.com") && !config.museai_ws_url.contains("hatch.metaaivm.com") {
            config.museai_ws_url.split("://").nth(1).unwrap_or("").split('.').next().unwrap_or("").to_string()
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
        body.insert("vmName".to_string(), serde_json::Value::String(active_vm_id.clone()));

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
                    println!("[museai] Note: Woke VM. Response: {}", json);
                }
            }
            Err(e) => {
                println!("[museai] Note: Failed to wake VM. Error: {}", e);
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
            println!("[museai] Note: POST /api/hatch/token failed: {e}");
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
        base_url.split("://").nth(1).unwrap_or("").split('.').next().unwrap_or("").to_string()
    } else {
        uuid::Uuid::new_v4().to_string()
    };
    // // let node_id = active_vm_id.clone();
    
    let chat_payload = serde_json::json!({
        "items": [
            {
                "type": "text",
                "text": prompt
            }
        ],
        "node_id": active_vm_id.clone(),
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

    
    println!("[museai] Sending encrypted POST /chat/subscribe over Noise...");
    let sub_payload = serde_json::json!({
        "channel": "main"
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

    println!("[museai] Sending encrypted POST /chat/stream over Noise...");

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

    println!("[museai] Waiting for encrypted response frames...");
    for _ in 0..1000 {
        let frame = match socket.read_encrypted_service_frame(&mut session) {
            Ok(f) => f,
            Err(e) if received_any_text => {
                println!("[museai] Error reading frame after text received: {}", e);
                break;
            },
            Err(e) => {
                // If it hits EOF/timeout, don't fail immediately if we got SOME text
                return Err(e);
            }
        };

        println!("[museai] Received encrypted frame: {:?}", frame.kind);

        match frame.kind {
            ServiceFrameKind::Response { body, end_body: _, status, headers: _ } => {
                println!("[museai] Response status: {}", status);
                if !body.is_empty() {
                    let s = String::from_utf8_lossy(&body);
                    println!("[museai] RAW Response Body: {}", s);
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&body) {
                        if let Some(text) = parse_assistant_content_from_json(&val) {
                            println!("[museai] Parsed Chunk: {}", text);
                            full_assistant_text.push_str(&text);
                            received_any_text = true;
                        }
                    }
                }
                // if end_body && received_any_text { break; }
            }
                        ServiceFrameKind::BodyChunk { data, end_body } => {
                let s = String::from_utf8_lossy(&data);
                println!("[museai] RAW BodyChunk (end_body={}): {}", end_body, s);
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&data) {
                    if let Some(text) = parse_assistant_content_from_json(&val) {
                        println!("[museai] Parsed Chunk: {}", text);
                        full_assistant_text.push_str(&text);
                        received_any_text = true;
                    }
                    
                    if let Some(status) = val.get("payload").and_then(|p| p.get("status")).and_then(|s| s.as_str()) {
                        if status == "completed" {
                            println!("[museai] Stream completed successfully!");
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

pub(crate) fn write_chat_completion(client: &mut TcpStream, response: &str) -> io::Result<()> {
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.len(),
        response
    )?;
    client.flush()
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
