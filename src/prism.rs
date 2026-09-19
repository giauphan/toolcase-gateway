use crate::config::Config;
use serde::{Deserialize, Deserializer, Serialize};
use std::io::{self, ErrorKind, Write};
use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct OpenAiChatRequest {
    pub model: Option<String>,
    pub messages: Vec<OpenAiMessage>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[allow(dead_code)]
    pub temperature: Option<f64>,
    #[allow(dead_code)]
    pub top_p: Option<f64>,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OpenAiMessage {
    pub role: String,
    #[serde(deserialize_with = "deserialize_flexible_content")]
    pub content: String,
}

fn deserialize_flexible_content<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Array(arr) => {
            let mut combined = String::new();
            for item in arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    combined.push_str(text);
                } else if let Some(text) = item.as_str() {
                    combined.push_str(text);
                }
            }
            Ok(combined)
        }
        _ => Ok(value.to_string()),
    }
}

#[derive(Serialize)]
struct PrismStartRequest {
    input: Vec<PrismInputItem>,
    metadata: PrismMetadata,
    #[serde(rename = "conversationId")]
    conversation_id: String,
}

#[derive(Serialize)]
pub struct PrismInputItem {
    #[serde(rename = "type")]
    pub item_type: String,
    pub role: String,
    pub content: Vec<PrismContentItem>,
}

#[derive(Serialize)]
pub struct PrismContentItem {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Serialize)]
struct PrismMetadata {
    #[serde(rename = "projectId")]
    project_id: String,
    #[serde(rename = "userId")]
    user_id: String,
    model: String,
    reasoning_effort: String,
    sandbox_url: String,
    sandbox_token: String,
    frontend_origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    codex_listen_snapshot: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct PrismStatusRequest {
    request_id: String,
    turn_state: serde_json::Value,
}

#[derive(Deserialize)]
struct PrismResponse {
    status: Option<String>,
    request_id: Option<String>,
    turn_state: Option<serde_json::Value>,
    response: Option<PrismInnerResponse>,
}

#[derive(Deserialize)]
struct PrismInnerResponse {
    status: Option<String>,
    payload: Option<PrismPayload>,
}

#[derive(Deserialize)]
struct PrismPayload {
    output: Option<Vec<PrismOutputMessage>>,
    message: Option<String>,
    #[allow(dead_code)]
    reason: Option<String>,
}

#[derive(Deserialize)]
struct PrismOutputMessage {
    content: Option<Vec<PrismOutputContent>>,
}

#[derive(Deserialize)]
struct PrismOutputContent {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    content_type: Option<String>,
    text: Option<String>,
}

#[derive(Serialize)]
pub struct OpenAiChatResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<OpenAiChatChoice>,
    pub usage: OpenAiUsage,
}

#[derive(Serialize)]
pub struct OpenAiChatChoice {
    pub index: usize,
    pub message: OpenAiResponseMessage,
    pub finish_reason: String,
}

#[derive(Serialize)]
pub struct OpenAiResponseMessage {
    pub role: String,
    pub content: String,
}

#[derive(Serialize)]
pub struct OpenAiUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

#[derive(Serialize)]
struct OpenAiChatChunk {
    id: String,
    object: String,
    created: u64,
    model: String,
    choices: Vec<OpenAiChunkChoice>,
}

#[derive(Serialize)]
struct OpenAiChunkChoice {
    index: usize,
    delta: OpenAiChunkDelta,
    finish_reason: Option<String>,
}

#[derive(Serialize)]
struct OpenAiChunkDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}

pub(crate) struct PrismCredentials {
    pub(crate) cookie: String,
    pub(crate) sandbox_token: String,
    pub(crate) user_id: String,
    pub(crate) project_id: String,
}

pub(crate) fn extract_credentials(headers: &[(String, String)], config: &Config) -> PrismCredentials {
    let mut creds = PrismCredentials {
        cookie: config.prism_cookie.clone(),
        sandbox_token: config.prism_sandbox_token.clone(),
        user_id: config.prism_user_id.clone(),
        project_id: config.prism_project_id.clone(),
    };

    let auth_header = crate::http::header_value(headers, "authorization")
        .and_then(|v| v.strip_prefix("Bearer ").or(Some(v)))
        .or_else(|| crate::http::header_value(headers, "x-api-key"))
        .unwrap_or("");

    let delim = if auth_header.contains("|||") { "|||" } else { "," };
    let parts: Vec<&str> = auth_header.rsplitn(4, delim).collect();
    
    // rsplitn returns parts from right to left.
    // if length is 4: parts[0]=project_id, parts[1]=user_id, parts[2]=sandbox_token, parts[3]=cookie
    if parts.len() == 4 {
        creds.project_id = parts[0].trim().to_string();
        creds.user_id = parts[1].trim().to_string();
        creds.sandbox_token = parts[2].trim().to_string();
        
        // The rest is the cookie. We only strip Bearer prefix from cookie just in case it started with it
        let c = parts[3].trim();
        creds.cookie = c.strip_prefix("Bearer ").unwrap_or(c).to_string();
    }
    creds
}

pub fn handle_prism_chat_completion(
    client: &mut TcpStream,
    body: &[u8],
    config: &Config,
    inbound_headers: &[(String, String)],
) -> io::Result<()> {
    let req: OpenAiChatRequest = serde_json::from_slice(body).map_err(|e| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("Invalid JSON request: {e}"),
        )
    })?;

    // Fresh session / conversation ID per request to avoid noisy context
    let conv_uuid = Uuid::new_v4();
    let conversation_id = format!("cdx1_{conv_uuid}");
    let mut requested_model = req
        .model
        .clone()
        .unwrap_or_else(|| config.prism_default_model.clone());

    let mut inferred_reasoning = None;
    for suffix in &["-xhigh", "-high", "-medium", "-low"] {
        if requested_model.ends_with(suffix) {
            requested_model = requested_model.strip_suffix(suffix).unwrap().to_string();
            inferred_reasoning = Some(suffix.trim_start_matches('-').to_string());
            break;
        }
    }

    let input_items = build_injected_prism_inputs(&req.messages, &config.prism_system_prompt);

    let mut reasoning_effort = req
        .reasoning_effort
        .clone()
        .or(inferred_reasoning)
        .unwrap_or_else(|| "medium".to_string())
        .to_lowercase();

    if reasoning_effort == "xhight" {
        reasoning_effort = "xhigh".to_string();
    }
    if !["low", "medium", "high", "xhigh"].contains(&reasoning_effort.as_str()) {
        reasoning_effort = "medium".to_string();
    }

    let creds = extract_credentials(inbound_headers, config);

    let prism_start = PrismStartRequest {
        input: input_items,
        metadata: PrismMetadata {
            project_id: creds.project_id.clone(),
            user_id: creds.user_id.clone(),
            model: requested_model.clone(),
            reasoning_effort,
            sandbox_url: format!("{}/s/sandboxes/proxy/", config.prism_base_url.trim_end_matches('/')),
            sandbox_token: creds.sandbox_token.clone(),
            frontend_origin: config.prism_base_url.clone(),
            codex_listen_snapshot: None,
        },
        conversation_id,
    };

    let start_url = format!("{}/api/llm/response_with_tools_start", config.prism_base_url.trim_end_matches('/'));
    let status_url = format!("{}/api/llm/response_with_tools_status", config.prism_base_url.trim_end_matches('/'));

    let mut ureq_builder = ureq::post(&start_url)
        .header("Content-Type", "application/json")
        .header("Origin", &config.prism_base_url)
        .header("Referer", &format!("{}/?u={}", config.prism_base_url, creds.project_id));

    // Forward inbound Cookie if present, otherwise fallback to configured PRISM_COOKIE
    let cookie = &creds.cookie;

    if !cookie.is_empty() {
        ureq_builder = ureq_builder.header("Cookie", cookie);
    }

    if let Some((_, sentinel_val)) = inbound_headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("openai-sentinel-token")) {
        ureq_builder = ureq_builder.header("openai-sentinel-token", sentinel_val);
    }

    let start_body = serde_json::to_vec(&prism_start).map_err(|e| {
        io::Error::new(ErrorKind::InvalidInput, format!("Serialization error: {e}"))
    })?;

    let start_resp_res = ureq_builder.send(&start_body);
    let mut start_resp = match start_resp_res {
        Ok(mut res) => {
            let status = res.status();
            let status_u16 = u16::from(status);
            if status_u16 >= 400 {
                let error_body = res.body_mut().read_to_string().unwrap_or_default();
                return crate::http::write_error(
                    client,
                    status_u16,
                    "Upstream Prism Error",
                    &format!("Prism rejected request with HTTP {}: {}", status_u16, error_body),
                );
            }
            res
        }
        Err(ureq::Error::StatusCode(code)) => {
            return crate::http::write_error(
                client,
                code,
                "Upstream Prism Error",
                &format!("Prism rejected request with HTTP {}", code),
            );
        }
        Err(e) => {
            return crate::http::write_error(
                client,
                502,
                "Bad Gateway",
                &format!("Prism start request failed (network or timeout): {e}"),
            );
        }
    };

    let prism_res: PrismResponse = match start_resp.body_mut().read_json() {
        Ok(data) => data,
        Err(e) => {
            return crate::http::write_error(
                client,
                502,
                "Bad Gateway",
                &format!("Failed to parse Prism start response: {e}"),
            );
        }
    };

    let mut current_turn_state = prism_res.turn_state;
    let request_id = prism_res.request_id.unwrap_or_default();
    let mut final_output_text = String::new();

    if let Some(inner) = prism_res.response {
        if let Some(payload) = inner.payload {
            if let Some(msg) = payload.message {
                if inner.status.as_deref() == Some("error") {
                    let status_code = if msg.contains("403 Forbidden") { 403 } else { 400 };
                    let status_text = if status_code == 403 { "Forbidden" } else { "Bad Request" };
                    return crate::http::write_error(
                        client,
                        status_code,
                        status_text,
                        &format!("Prism error: {msg}"),
                    );
                }
            }
            if let Some(output) = payload.output {
                final_output_text = extract_text_from_output(&output);
            }
        }
    }

    // If response is still running or pending, poll status endpoint
    if final_output_text.is_empty() && current_turn_state.is_some() && !request_id.is_empty() {
        let max_attempts = 120;
        for _ in 0..max_attempts {
            std::thread::sleep(Duration::from_millis(1000));

            let status_req = PrismStatusRequest {
                request_id: request_id.clone(),
                turn_state: current_turn_state.clone().unwrap(),
            };

            let status_bytes = match serde_json::to_vec(&status_req) {
                Ok(b) => b,
                Err(_) => break,
            };

            let mut req_builder = ureq::post(&status_url)
                .header("Content-Type", "application/json")
                .header("Origin", &config.prism_base_url)
                .header("Referer", &format!("{}/?u={}", config.prism_base_url, creds.project_id));

            // Forward incoming openai-sentinel-token if present
            if let Some((_, sentinel_val)) = inbound_headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("openai-sentinel-token")) {
                req_builder = req_builder.header("openai-sentinel-token", sentinel_val);
            }

            if !cookie.is_empty() {
                req_builder = req_builder.header("Cookie", cookie);
            }

            let poll_resp_res = req_builder.send(&status_bytes);
            let mut poll_resp = match poll_resp_res {
                Ok(r) => {
                    let status = r.status();
                    let status_u16 = u16::from(status);
                    if status_u16 >= 400 {
                        let _ = r.into_body().read_to_string(); // exhaust it
                        continue;
                    }
                    r
                }
                Err(ureq::Error::StatusCode(_)) => continue,
                Err(_) => continue,
            };

            let poll_data: PrismResponse = match poll_resp.body_mut().read_json() {
                Ok(d) => d,
                Err(_) => continue,
            };

            if let Some(new_turn_state) = poll_data.turn_state {
                current_turn_state = Some(new_turn_state);
            }

            if let Some(inner) = poll_data.response {
                if let Some(payload) = inner.payload {
                    if let Some(msg) = payload.message {
                        if inner.status.as_deref() == Some("error") {
                            let status_code = if msg.contains("403 Forbidden") { 403 } else { 400 };
                            let status_text = if status_code == 403 { "Forbidden" } else { "Bad Request" };
                            return crate::http::write_error(
                                client,
                                status_code,
                                status_text,
                                &format!("Prism error: {msg}"),
                            );
                        }
                    }
                    if let Some(output) = payload.output {
                        final_output_text = extract_text_from_output(&output);
                        if !final_output_text.is_empty() {
                            break;
                        }
                    }
                }
            }

            if poll_data.status.as_deref() == Some("completed") {
                break;
            }
        }
    }

    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let response_id = format!("chatcmpl-prism-{}", Uuid::new_v4());

    if req.stream == Some(true) {
        // SSE Streaming format
        write!(
            client,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
        )?;

        // Delta chunk 1: role
        let chunk_start = OpenAiChatChunk {
            id: response_id.clone(),
            object: "chat.completion.chunk".to_string(),
            created,
            model: requested_model.clone(),
            choices: vec![OpenAiChunkChoice {
                index: 0,
                delta: OpenAiChunkDelta {
                    role: Some("assistant".to_string()),
                    content: Some("".to_string()),
                },
                finish_reason: None,
            }],
        };
        let _ = write!(client, "data: {}\n\n", serde_json::to_string(&chunk_start).unwrap_or_default());
        let _ = client.flush();

        // Delta chunk 2: text content
        if !final_output_text.is_empty() {
            let chunk_content = OpenAiChatChunk {
                id: response_id.clone(),
                object: "chat.completion.chunk".to_string(),
                created,
                model: requested_model.clone(),
                choices: vec![OpenAiChunkChoice {
                    index: 0,
                    delta: OpenAiChunkDelta {
                        role: None,
                        content: Some(final_output_text.clone()),
                    },
                    finish_reason: None,
                }],
            };
            let _ = write!(client, "data: {}\n\n", serde_json::to_string(&chunk_content).unwrap_or_default());
            let _ = client.flush();
        }

        // Delta chunk 3: stop
        let chunk_end = OpenAiChatChunk {
            id: response_id,
            object: "chat.completion.chunk".to_string(),
            created,
            model: requested_model,
            choices: vec![OpenAiChunkChoice {
                index: 0,
                delta: OpenAiChunkDelta {
                    role: None,
                    content: None,
                },
                finish_reason: Some("stop".to_string()),
            }],
        };
        let _ = write!(client, "data: {}\n\n", serde_json::to_string(&chunk_end).unwrap_or_default());
        let _ = write!(client, "data: [DONE]\n\n");
        let _ = client.flush();
        Ok(())
    } else {
        // Standard JSON format
        let prompt_tokens = body.len() / 4;
        let completion_tokens = final_output_text.len() / 4;
        let openai_resp = OpenAiChatResponse {
            id: response_id,
            object: "chat.completion".to_string(),
            created,
            model: requested_model,
            choices: vec![OpenAiChatChoice {
                index: 0,
                message: OpenAiResponseMessage {
                    role: "assistant".to_string(),
                    content: final_output_text,
                },
                finish_reason: "stop".to_string(),
            }],
            usage: OpenAiUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
            },
        };

        let resp_json = serde_json::to_string(&openai_resp).unwrap_or_default();
        write!(
            client,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            resp_json.len(),
            resp_json
        )?;
        client.flush()?;
        Ok(())
    }
}

fn extract_text_from_output(output: &[PrismOutputMessage]) -> String {
    let mut text_acc = String::new();
    for msg in output {
        if let Some(contents) = &msg.content {
            for item in contents {
                if let Some(t) = &item.text {
                    text_acc.push_str(t);
                }
            }
        }
    }
    text_acc
}

pub fn build_injected_prism_inputs(messages: &[OpenAiMessage], system_prompt_inject: &str) -> Vec<PrismInputItem> {
    let mut input_items = Vec::new();
    let mut has_injected_system = false;

    for msg in messages {
        let mut text_content = msg.content.clone();

        if msg.role == "system" && !has_injected_system {
            has_injected_system = true;
            text_content = format!("{}\n\n{}", system_prompt_inject, text_content);
        }

        input_items.push(PrismInputItem {
            item_type: "message".to_string(),
            role: msg.role.clone(),
            content: vec![PrismContentItem {
                content_type: "input_text".to_string(),
                text: text_content,
            }],
        });
    }

    if !has_injected_system {
        input_items.insert(0, PrismInputItem {
            item_type: "message".to_string(),
            role: "system".to_string(),
            content: vec![PrismContentItem {
                content_type: "input_text".to_string(),
                text: system_prompt_inject.to_string(),
            }],
        });
    }
    input_items
}
