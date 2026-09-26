import re

with open("src/museai.rs", "r") as f:
    text = f.read()

# I will just rewrite the whole bootstrap_museai_config and request_museai_chat_completion functions safely.
# Find the start of bootstrap_museai_config
start_idx = text.find("pub(crate) fn bootstrap_museai_config")
# Find the start of write_chat_completion (which is right after request_museai_chat_completion)
end_idx = text.find("pub(crate) fn write_chat_completion")

if start_idx == -1 or end_idx == -1:
    print("Could not find functions to replace")
    exit(1)

head = text[:start_idx]
tail = text[end_idx:]

new_functions = """pub(crate) fn bootstrap_museai_config(config: &Config) -> io::Result<Config> {
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
    let node_id = active_vm_id.clone();
    
    let chat_payload = serde_json::json!({
        "items": [
            {
                "type": "text",
                "text": prompt
            }
        ],
        "node_id": node_id,
        "capabilities": [
            "chat_cancel",
            "delta_stream",
            "custom_reactions",
            "custom_reactions_facebook_thumbs_up_v1"
        ]
    });
    
    let payload_bytes = serde_json::to_vec(&chat_payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    let stream_id = 1;
    let headers = vec![
        Header {
            key: "Content-Type".to_string(),
            value: "application/json".to_string(),
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
    for i in 0..100 {
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

        if i == 0 {
            println!("[museai] Received first encrypted frame!");
        }

        match frame.kind {
            ServiceFrameKind::Response { body, end_body, .. } => {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&body) {
                    if let Some(text) = parse_assistant_content_from_json(&val) {
                        println!("[museai] Chunk: {}", text);
                        full_assistant_text.push_str(&text);
                        received_any_text = true;
                    }
                }
                if end_body && received_any_text {
                    break;
                }
            }
            ServiceFrameKind::BodyChunk { data, end_body } => {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&data) {
                    if let Some(text) = parse_assistant_content_from_json(&val) {
                        println!("[museai] Chunk: {}", text);
                        full_assistant_text.push_str(&text);
                        received_any_text = true;
                    }
                }
                if end_body && received_any_text {
                    break;
                }
            }
            ServiceFrameKind::Reset { .. } => {
                break;
            }
            _ => {}
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

"""

with open("src/museai.rs", "w") as f:
    f.write(head + new_functions + tail)

print("Patched src/museai.rs")
