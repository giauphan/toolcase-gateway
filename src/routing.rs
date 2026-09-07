use crate::config::Config;
use crate::http::{header_value, read_request, read_response_head, stream_response, write_error, Request};
use crate::rewrite::{json_string_value, replace_model};
use std::io::{self, Write};
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
use std::net::TcpStream;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) const RETRYABLE: [u16; 11] = [400, 401, 402, 403, 408, 429, 500, 502, 503, 504, 524];
static RR_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn parse_retry_after(headers: &[(String, String)]) -> Option<Duration> {
    header_value(headers, "retry-after")
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
}

fn calculate_retry_delay(
    attempt: usize,
    base_delay_ms: u64,
    max_delay_ms: u64,
    retry_after: Option<Duration>,
) -> Duration {
    retry_after.unwrap_or_else(|| {
        let exponential = base_delay_ms * 2u64.pow(attempt.saturating_sub(1) as u32);
        Duration::from_millis(exponential.min(max_delay_ms))
    })
}

pub(crate) fn serve(mut client: TcpStream, config: &Config) -> io::Result<()> {
    let request = read_request(&mut client)?;
    if request.method.eq_ignore_ascii_case("HEAD") && request.path == "/api/hello" {
        return write!(
            client,
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n"
        );
    }
    let rotation = RR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let candidates = request_candidates(&request.body, &config.fallbacks, rotation);
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
        // Open the upstream and read only the status line + headers so the
        // failover decision happens before any byte is streamed to the client.
        let attempt = open_upstream(&client, &request, config, model).and_then(|mut upstream| {
            let head = read_response_head(&mut upstream)?;
            Ok((upstream, head))
        });
        match attempt {
            Ok((_, head)) if RETRYABLE.contains(&head.status) && !last => {
                last_status = Some(head.status);
                let body_snippet = String::from_utf8_lossy(&head.buffered_body)
                    .trim()
                    .to_string();
                if !body_snippet.is_empty() {
                    eprintln!(
                        "[toolcase-gateway] request {} {} auth_header={} x_api_key={} upstream model \"{}\" returned HTTP {} with body preview: {}",
                        request.method, request.path, has_auth, has_x_api_key, model, head.status, body_snippet
                    );
                }
                let retry_after = parse_retry_after(&head.headers);
                let delay = calculate_retry_delay(attempt_number, config.retry_base_delay_ms, config.max_retry_delay_ms, retry_after);
                eprintln!("[toolcase-gateway] upstream model \"{model}\" returned HTTP {}. Failing over to \"{}\" (retry in {}ms)...", head.status, candidates[index + 1], delay.as_millis());
                thread::sleep(delay);
            }
            Ok((mut upstream, head)) => {
                if RETRYABLE.contains(&head.status) {
                    last_status = Some(head.status);
                    let body_snippet = String::from_utf8_lossy(&head.buffered_body)
                        .trim()
                        .to_string();
                    if !body_snippet.is_empty() {
                        eprintln!(
                            "[toolcase-gateway] model \"{model}\" returned final HTTP {} with body preview: {}",
                            head.status, body_snippet
                        );
                    }
                    eprintln!(
                        "[toolcase-gateway] model \"{model}\" returned final HTTP {}; all {} candidates exhausted",
                        head.status,
                        candidates.len()
                    );
                    return write_error(
                        &mut client,
                        head.status,
                        match head.status {
                            503 => "Service Unavailable",
                            502 => "Bad Gateway",
                            500 => "Internal Server Error",
                            504 => "Gateway Timeout",
                            _ => "Service Unavailable",
                        },
                        "toolcase-gateway: all upstream models exhausted",
                    );
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
                }
                // Committed to this model: stream its body straight through.
                return stream_response(&mut client, &mut upstream, head, &request.body);
            }
            Err(error) if !last => {
                let delay = calculate_retry_delay(attempt_number, config.retry_base_delay_ms, config.max_retry_delay_ms, None);
                eprintln!("[toolcase-gateway] upstream model \"{model}\" failed ({}). Failing over to \"{}\" (retry in {}ms)...", error.kind(), candidates[index + 1], delay.as_millis());
                thread::sleep(delay);
            }
            Err(error) => {
                eprintln!(
                    "[toolcase-gateway] model \"{model}\" failed on final attempt ({})",
                    error.kind()
                );
                let status = last_status.unwrap_or(502);
                return write_error(
                    &mut client,
                    status,
                    match status {
                        503 => "Service Unavailable",
                        502 => "Bad Gateway",
                        500 => "Internal Server Error",
                        504 => "Gateway Timeout",
                        _ => "Bad Gateway",
                    },
                    "toolcase-gateway: upstream unavailable",
                );
            }
        }
    }
    write_error(
        &mut client,
        last_status.unwrap_or(502),
        match last_status {
            Some(503) => "Service Unavailable",
            Some(502) => "Bad Gateway",
            Some(500) => "Internal Server Error",
            Some(504) => "Gateway Timeout",
            _ => "Bad Gateway",
        },
        "toolcase-gateway: all upstream models exhausted",
    )
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
    let client_ip = client.peer_addr().map(|a| a.ip().to_string()).unwrap_or_else(|_| "unknown".to_string());
    write!(
        upstream,
        "Host: {host}:{port}\r\nAccept-Encoding: identity\r\nX-Forwarded-For: {client_ip}\r\nX-Real-Ip: {client_ip}\r\nX-Forwarded-Proto: http\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    upstream.write_all(&body)?;
    upstream.flush()?;
    Ok(upstream)
}

pub(crate) fn request_candidates(
    body: &[u8],
    fallbacks: &[String],
    rotation: usize,
) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Ok(text) = std::str::from_utf8(body) {
        if let Some(model) = json_string_value(text, "model") {
            candidates.push(model);
        }
    }
    // Round-robin the fallback order so consecutive requests do not all hammer
    // the same secondary model after a primary failure.
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
