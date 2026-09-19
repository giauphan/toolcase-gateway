use crate::config::Config;
use crate::http::{header_value, read_response_head, stream_response, write_error, Request};
use crate::rewrite::{json_string_value, replace_model};
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

pub(crate) const RETRYABLE: [u16; 7] = [408, 429, 500, 502, 503, 504, 524];
static RR_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn parse_retry_after(headers: &[(String, String)]) -> Option<Duration> {
    let value = header_value(headers, "retry-after")?;
    let trimmed = value.trim();
    if let Ok(seconds) = trimmed.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    None
}

pub(crate) fn calculate_retry_delay(
    attempt: usize,
    base_delay_ms: u64,
    max_delay_ms: u64,
    headers: Option<&[(String, String)]>,
) -> Duration {
    if let Some(hdrs) = headers {
        if let Some(delay) = parse_retry_after(hdrs) {
            let max_dur = Duration::from_millis(max_delay_ms);
            return if delay > max_dur { max_dur } else { delay };
        }
    }
    let multiplier = 1u64.checked_shl(attempt as u32).unwrap_or(u64::MAX);
    let computed_ms = base_delay_ms.saturating_mul(multiplier);
    let final_ms = computed_ms.min(max_delay_ms);
    Duration::from_millis(final_ms)
}

pub(crate) fn open_upstream(
    client: &TcpStream,
    request: &Request,
    config: &Config,
    model: &str,
) -> io::Result<TcpStream> {
    let host = config.target_host.as_str();
    let port = config.target_port;
    let mut upstream = TcpStream::connect((host, port))?;
    if let Some(timeout) = config.io_timeout {
        upstream.set_read_timeout(Some(timeout))?;
        upstream.set_write_timeout(Some(timeout))?;
    }
    let body = replace_model(&request.body, model);
    write!(upstream, "{} {} HTTP/1.1\r\n", request.method, request.path)?;
    for (name, value) in &request.headers {
        if name.eq_ignore_ascii_case("host")
            || name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("accept-encoding")
            || HOP_BY_HOP.iter().any(|h| name.eq_ignore_ascii_case(h))
        {
            continue;
        }
        write!(upstream, "{name}: {value}\r\n")?;
    }
    let client_ip = client
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    write!(
        upstream,
        "Host: {host}:{port}\r\nAccept-Encoding: identity\r\nX-Forwarded-For: {client_ip}\r\nX-Real-Ip: {client_ip}\r\nX-Forwarded-Proto: http\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    upstream.write_all(&body)?;
    upstream.flush()?;
    Ok(upstream)
}

pub(crate) fn handle_omniroute_proxy(
    client: &mut TcpStream,
    request: &Request,
    config: &Config,
) -> io::Result<()> {
    let rotation = RR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let candidates = candidate_models(&request.body, &config.fallbacks, rotation);
    let has_auth = header_value(&request.headers, "authorization").is_some();
    let has_x_api_key = header_value(&request.headers, "x-api-key").is_some();
    eprintln!(
        "[toolcase-gateway] request {} {} (auth: auth_header={}, x_api_key={}) candidates: {}",
        request.method,
        request.path,
        has_auth,
        has_x_api_key,
        candidates
            .iter()
            .map(|model| if model.is_empty() { "<empty>" } else { model })
            .collect::<Vec<_>>()
            .join(", ")
    );

    let mut last_status: Option<u16> = None;

    for (index, model) in candidates.iter().enumerate() {
        let attempt_number = index + 1;
        let last = attempt_number == candidates.len();
        eprintln!(
            "[toolcase-gateway] attempt {attempt_number}/{} model \"{model}\"",
            candidates.len()
        );

        let attempt = open_upstream(client, request, config, model).and_then(|mut upstream| {
            let head = read_response_head(&mut upstream)?;
            last_status = Some(head.status);
            if RETRYABLE.contains(&head.status) {
                let delay = calculate_retry_delay(
                    index,
                    config.retry_base_delay_ms,
                    config.max_retry_delay_ms,
                    Some(&head.headers),
                );
                if !last {
                    eprintln!(
                        "[toolcase-gateway] upstream model \"{model}\" returned HTTP {}. Failing over to \"{}\" (retry in {}ms)...",
                        head.status,
                        candidates[index + 1],
                        delay.as_millis()
                    );
                    if !delay.is_zero() {
                        thread::sleep(delay);
                    }
                    return Ok(());
                }
                eprintln!(
                    "[toolcase-gateway] upstream model \"{model}\" returned HTTP {} on final attempt",
                    head.status
                );
                write_error(
                    client,
                    head.status,
                    match head.status {
                        408 => "Request Timeout",
                        429 => "Too Many Requests",
                        503 => "Service Unavailable",
                        502 => "Bad Gateway",
                        500 => "Internal Server Error",
                        504 => "Gateway Timeout",
                        _ => "Service Unavailable",
                    },
                    "toolcase-gateway: all upstream models exhausted",
                )
            } else {
                if head.status >= 400 {
                    let body_snippet = String::from_utf8_lossy(&head.buffered_body)
                        .trim()
                        .to_string();
                    if !body_snippet.is_empty() {
                        eprintln!(
                            "[toolcase-gateway] model \"{model}\" accepted with HTTP {} but body preview: {}",
                            head.status, body_snippet
                        );
                    }
                }
                eprintln!(
                    "[toolcase-gateway] model \"{model}\" accepted with HTTP {}",
                    head.status
                );
                stream_response(client, &mut upstream, head, &request.body)?;
                Ok(())
            }
        });

        match attempt {
            Ok(()) => {
                if let Some(status) = last_status {
                    if !RETRYABLE.contains(&status) {
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
                    "[toolcase-gateway] upstream model \"{model}\" failed ({}). Failing over to \"{}\" (retry in {}ms)...",
                    error.kind(),
                    candidates[index + 1],
                    delay.as_millis()
                );
                if !delay.is_zero() {
                    thread::sleep(delay);
                }
            }
            Err(error) => {
                eprintln!(
                    "[toolcase-gateway] model \"{model}\" failed on final attempt ({})",
                    error.kind()
                );
                return write_error(
                    client,
                    502,
                    match last_status {
                        Some(408) => "Request Timeout",
                        Some(429) => "Too Many Requests",
                        Some(503) => "Service Unavailable",
                        Some(502) => "Bad Gateway",
                        Some(500) => "Internal Server Error",
                        Some(504) => "Gateway Timeout",
                        _ => "Bad Gateway",
                    },
                    "toolcase-gateway: all upstream models exhausted",
                );
            }
        }
    }

    write_error(
        client,
        last_status.unwrap_or(502),
        match last_status {
            Some(408) => "Request Timeout",
            Some(429) => "Too Many Requests",
            Some(503) => "Service Unavailable",
            Some(502) => "Bad Gateway",
            Some(500) => "Internal Server Error",
            Some(504) => "Gateway Timeout",
            _ => "Bad Gateway",
        },
        "toolcase-gateway: all upstream models exhausted",
    )
}

pub(crate) fn candidate_models(body: &[u8], fallbacks: &[String], rotation: usize) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Ok(text) = std::str::from_utf8(body) {
        if let Some(model) = json_string_value(text, "model") {
            candidates.push(model);
        }
    }
    if !fallbacks.is_empty() {
        let offset = rotation % fallbacks.len();
        for step in 0..fallbacks.len() {
            let fallback = &fallbacks[(offset + step) % fallbacks.len()];
            if !candidates.iter().any(|v| v == fallback) {
                candidates.push(fallback.clone());
            }
        }
    }
    if candidates.is_empty() {
        candidates.push(String::new());
    }
    candidates
}
