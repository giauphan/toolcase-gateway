use crate::config::Config;
use crate::http::{read_request, read_response_head, stream_response, write_error, Request};
use crate::rewrite::{json_string_value, replace_model};
use std::io::{self, Write};

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

pub(crate) const RETRYABLE: [u16; 9] = [402, 403, 408, 429, 500, 502, 503, 504, 524];
static RR_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn serve(mut client: TcpStream, config: &Config) -> io::Result<()> {
    let request = read_request(&mut client)?;
    let rotation = RR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let candidates = request_candidates(&request.body, &config.fallbacks, rotation);
    eprintln!(
        "[toolcase-gateway] request {} {} candidates: {}",
        request.method,
        request.path,
        candidates
            .iter()
            .map(|model| if model.is_empty() { "<empty>" } else { model })
            .collect::<Vec<_>>()
            .join(", ")
    );
    for (index, model) in candidates.iter().enumerate() {
        let attempt_number = index + 1;
        let last = attempt_number == candidates.len();
        eprintln!(
            "[toolcase-gateway] attempt {attempt_number}/{} model \"{model}\"",
            candidates.len()
        );
        // Open the upstream and read only the status line + headers so the
        // failover decision happens before any byte is streamed to the client.
        let attempt = open_upstream(&request, config, model).and_then(|mut upstream| {
            let head = read_response_head(&mut upstream)?;
            Ok((upstream, head))
        });
        match attempt {
            Ok((_, head)) if RETRYABLE.contains(&head.status) && !last => {
                eprintln!("[toolcase-gateway] upstream model \"{model}\" returned HTTP {}. Failing over to \"{}\"...", head.status, candidates[index + 1]);
            }
            Ok((mut upstream, head)) => {
                if RETRYABLE.contains(&head.status) {
                    eprintln!(
                        "[toolcase-gateway] model \"{model}\" returned final HTTP {}; all {} candidates exhausted",
                        head.status,
                        candidates.len()
                    );
                } else {
                    eprintln!(
                        "[toolcase-gateway] model \"{model}\" accepted with HTTP {}",
                        head.status
                    );
                }
                // Committed to this model: stream its body straight through.
                return stream_response(&mut client, &mut upstream, head, &request.body);
            }
            Err(error) if !last => {
                eprintln!("[toolcase-gateway] upstream model \"{model}\" failed ({}). Failing over to \"{}\"...", error.kind(), candidates[index + 1]);
            }
            Err(error) => {
                eprintln!(
                    "[toolcase-gateway] model \"{model}\" failed on final attempt ({})",
                    error.kind()
                );
                return write_error(
                    &mut client,
                    502,
                    "Bad Gateway",
                    "toolcase-gateway: upstream unavailable",
                );
            }
        }
    }
    write_error(
        &mut client,
        502,
        "Bad Gateway",
        "toolcase-gateway: upstream unavailable",
    )
}

pub(crate) fn open_upstream(
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
    write!(
        upstream,
        "Host: {host}:{port}\r\nAccept-Encoding: identity\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
