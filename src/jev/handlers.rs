use crate::config::Config;
use crate::http::{header_value, read_response_head, stream_response, write_error, Request};
use crate::jev::business::is_account_retryable;
use crate::jev::pool::JevAccountPool;
use crate::omniroute::{calculate_retry_delay, open_upstream};
use std::io::{self, Write};
use std::net::TcpStream;
use std::thread;

pub fn handle_jev_models_catalog(client: &mut TcpStream) -> io::Result<()> {
    let body = r#"{"object":"list","data":[{"id":"jev","object":"model","created":1700000000,"owned_by":"jev-ai"},{"id":"gpt-4o-mini","object":"model","created":1700000000,"owned_by":"jev-ai"},{"id":"claude-3-5-sonnet","object":"model","created":1700000000,"owned_by":"jev-ai"}]}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    client.write_all(response.as_bytes())?;
    client.flush()
}

pub fn handle_jev_ai(client: &mut TcpStream, request: &Request, config: &Config) -> io::Result<()> {
    let mut keys = config.jev_api_keys.clone();
    if keys.is_empty() {
        if let Some(auth) = header_value(&request.headers, "authorization") {
            if let Some(token) = auth
                .strip_prefix("Bearer ")
                .or_else(|| auth.strip_prefix("bearer "))
            {
                let token_clean = token.trim().to_string();
                if !token_clean.is_empty() {
                    keys.push(token_clean);
                }
            }
        }
        if let Some(api_key) = header_value(&request.headers, "x-api-key") {
            let key_clean = api_key.trim().to_string();
            if !key_clean.is_empty() && !keys.contains(&key_clean) {
                keys.push(key_clean);
            }
        }
    }

    let pool = JevAccountPool::new(&keys);
    if pool.is_empty() {
        return write_error(
            client,
            401,
            "Unauthorized",
            "No Jev AI accounts configured. Set GW_JEV_API_KEYS or pass Authorization: Bearer <key>",
        );
    }

    let candidates = pool.candidate_accounts();
    let model = crate::rewrite::json_string_value(
        std::str::from_utf8(&request.body).unwrap_or(""),
        "model",
    )
    .unwrap_or_else(|| {
        if config.default_model.is_empty() {
            "jev".to_string()
        } else {
            config.default_model.clone()
        }
    });

    let mut normalized_request = Request {
        method: request.method.clone(),
        path: if request.path.starts_with("/jev/v1") {
            request.path.replacen("/jev/v1", "/v1", 1)
        } else if request.path.starts_with("/jev-ai/v1") {
            request.path.replacen("/jev-ai/v1", "/v1", 1)
        } else {
            request.path.clone()
        },
        headers: request.headers.clone(),
        body: request.body.clone(),
    };

    let mut last_status: Option<u16> = None;

    for (index, account) in candidates.iter().enumerate() {
        let attempt_number = index + 1;
        let last = attempt_number == candidates.len();
        eprintln!(
            "[toolcase-gateway] Jev attempt {attempt_number}/{} with account {}",
            candidates.len(),
            account.label
        );

        // Inject current account's Authorization header
        normalized_request.headers.retain(|(k, _)| {
            !k.eq_ignore_ascii_case("authorization") && !k.eq_ignore_ascii_case("x-api-key")
        });
        normalized_request.headers.push((
            "Authorization".to_string(),
            format!("Bearer {}", account.api_key),
        ));

        let attempt = open_upstream(client, &normalized_request, config, &model).and_then(
            |mut upstream| {
                let head = read_response_head(&mut upstream)?;
                last_status = Some(head.status);
                if is_account_retryable(head.status) {
                    let delay = calculate_retry_delay(
                        index,
                        config.retry_base_delay_ms,
                        config.max_retry_delay_ms,
                        Some(&head.headers),
                    );
                    if !last {
                        eprintln!(
                            "[toolcase-gateway] Jev account {} returned HTTP {}. Failing over to account {} (retry in {}ms)...",
                            account.label,
                            head.status,
                            candidates[index + 1].label,
                            delay.as_millis()
                        );
                        if !delay.is_zero() {
                            thread::sleep(delay);
                        }
                        return Ok(());
                    }
                    eprintln!(
                        "[toolcase-gateway] Jev account {} returned HTTP {} on final attempt",
                        account.label, head.status
                    );
                    write_error(
                        client,
                        head.status,
                        match head.status {
                            400 => "Bad Request",
                            401 => "Unauthorized",
                            402 => "Payment Required",
                            403 => "Forbidden",
                            408 => "Request Timeout",
                            429 => "Too Many Requests",
                            503 => "Service Unavailable",
                            502 => "Bad Gateway",
                            500 => "Internal Server Error",
                            504 => "Gateway Timeout",
                            _ => "Service Unavailable",
                        },
                        &format!(
                            "toolcase-gateway: all Jev accounts exhausted; account {} returned HTTP {}",
                            account.label, head.status
                        ),
                    )
                } else {
                    eprintln!(
                        "[toolcase-gateway] Jev account {} accepted with HTTP {}",
                        account.label, head.status
                    );
                    stream_response(client, &mut upstream, head, &normalized_request.body)?;
                    Ok(())
                }
            },
        );

        match attempt {
            Ok(()) => {
                if let Some(status) = last_status {
                    if !is_account_retryable(status) {
                        return Ok(());
                    }
                } else {
                    return Ok(());
                }
            }
            Err(error) if !last => {
                let delay = calculate_retry_delay(
                    index,
                    config.retry_base_delay_ms,
                    config.max_retry_delay_ms,
                    None,
                );
                eprintln!(
                    "[toolcase-gateway] Jev account {} failed ({}). Failing over to account {} (retry in {}ms)...",
                    account.label,
                    error.kind(),
                    candidates[index + 1].label,
                    delay.as_millis()
                );
                if !delay.is_zero() {
                    thread::sleep(delay);
                }
            }
            Err(error) => {
                eprintln!(
                    "[toolcase-gateway] Jev account {} failed on final attempt ({})",
                    account.label,
                    error.kind()
                );
                return write_error(
                    client,
                    502,
                    "Bad Gateway",
                    &format!(
                        "toolcase-gateway: all Jev accounts exhausted; last account error ({})",
                        error.kind()
                    ),
                );
            }
        }
    }

    write_error(
        client,
        last_status.unwrap_or(502),
        "Bad Gateway",
        &format!(
            "toolcase-gateway: all Jev accounts exhausted; last response {}",
            last_status
                .map(|s| format!("HTTP {s}"))
                .unwrap_or_else(|| "unavailable".to_string())
        ),
    )
}
