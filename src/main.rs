//! Tool-name casing gateway + failover retry.

#![forbid(unsafe_code)]

use std::env;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

const RETRYABLE: [u16; 9] = [402, 403, 408, 429, 500, 502, 503, 504, 524];
const MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024;
const MAX_HEAD_BYTES: usize = 64 * 1024;
const MAX_REWRITE_WINDOW: usize = 1024 * 1024;
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

fn env_or(key: &str, fallback: &str) -> String {
    env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| fallback.into())
}

/// Runtime limits and upstream target, resolved once at startup.
struct Config {
    target_host: String,
    target_port: u16,
    fallbacks: Vec<String>,
    io_timeout: Option<Duration>,
}

fn main() -> io::Result<()> {
    let listen_host = env_or("GW_LISTEN_HOST", "127.0.0.1");
    let listen_port = env_or("GW_LISTEN_PORT", "20129").parse().unwrap_or(20129);
    let max_connections: usize = env_or("GW_MAX_CONNECTIONS", "256").parse().unwrap_or(256);
    let timeout_secs: u64 = env_or("GW_IO_TIMEOUT_SECS", "120").parse().unwrap_or(120);
    let config = Config {
        target_host: env_or("GW_TARGET_HOST", "127.0.0.1"),
        target_port: env_or("GW_TARGET_PORT", "20128").parse().unwrap_or(20128),
        fallbacks: env_or("GW_FALLBACK_MODELS", "fail-try")
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect(),
        io_timeout: (timeout_secs > 0).then(|| Duration::from_secs(timeout_secs)),
    };
    let config = std::sync::Arc::new(config);
    let listener = TcpListener::bind((listen_host.as_str(), listen_port))?;
    eprintln!(
        "[toolcase-gateway] {listen_host}:{listen_port} -> {}:{}",
        config.target_host, config.target_port
    );
    if listen_host != "127.0.0.1" && listen_host != "::1" && listen_host != "localhost" {
        eprintln!(
            "[toolcase-gateway] WARNING: listening on {listen_host} exposes an unauthenticated proxy; put it behind an authenticating front end"
        );
    }
    for stream in listener.incoming() {
        let Ok(client) = stream else { continue };
        // Bound concurrency so a flood of idle sockets cannot exhaust threads.
        if ACTIVE.load(Ordering::Relaxed) >= max_connections {
            let mut client = client;
            let _ = write_error(
                &mut client,
                503,
                "Service Unavailable",
                "too many connections",
            );
            continue;
        }
        if let Some(timeout) = config.io_timeout {
            let _ = client.set_read_timeout(Some(timeout));
            let _ = client.set_write_timeout(Some(timeout));
        }
        ACTIVE.fetch_add(1, Ordering::Relaxed);
        let config = config.clone();
        thread::spawn(move || {
            if let Err(error) = serve(client, &config) {
                if !matches!(
                    error.kind(),
                    ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::UnexpectedEof
                ) {
                    eprintln!("[toolcase-gateway] request failed: {}", error.kind());
                }
            }
            ACTIVE.fetch_sub(1, Ordering::Relaxed);
        });
    }
    Ok(())
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

static RR_COUNTER: AtomicUsize = AtomicUsize::new(0);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

fn serve(mut client: TcpStream, config: &Config) -> io::Result<()> {
    let request = read_request(&mut client)?;
    let rotation = RR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let candidates = request_candidates(&request.body, &config.fallbacks, rotation);
    for (index, model) in candidates.iter().enumerate() {
        let last = index + 1 == candidates.len();
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
                // Committed to this model: stream its body straight through.
                return stream_response(&mut client, &mut upstream, head, &request.body);
            }
            Err(error) if !last => {
                eprintln!("[toolcase-gateway] upstream model \"{model}\" failed ({}). Failing over to \"{}\"...", error.kind(), candidates[index + 1]);
            }
            Err(_) => {
                return write_error(
                    &mut client,
                    502,
                    "Bad Gateway",
                    "toolcase-gateway: upstream unavailable",
                )
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

fn read_request(stream: &mut TcpStream) -> io::Result<Request> {
    let raw = read_until_headers(stream, MAX_HEAD_BYTES)?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .ok_or_else(|| io::Error::new(ErrorKind::UnexpectedEof, "incomplete request head"))?;
    let head = std::str::from_utf8(&raw[..split])
        .map_err(|_| io::Error::new(ErrorKind::InvalidData, "non-utf8 request head"))?;
    let mut lines = head.split("\r\n");
    let mut parts = lines.next().unwrap_or("POST /").split_whitespace();
    let method = parts.next().unwrap_or("POST").to_owned();
    let path = parts.next().unwrap_or("/").to_owned();
    if !is_token(&method) || !is_request_target(&path) {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "invalid request line",
        ));
    }
    let headers = parse_headers(lines)?;
    let chunked = is_chunked(&headers);
    let content_lengths: Vec<&str> = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .map(|(_, v)| v.as_str())
        .collect();
    // Reject request smuggling: conflicting or duplicated framing headers.
    if content_lengths.len() > 1 || (chunked && !content_lengths.is_empty()) {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "conflicting framing headers",
        ));
    }
    let mut body = raw[split..].to_vec();
    if chunked {
        body = read_chunked_body(stream, body)?;
    } else {
        let length = match content_lengths.first() {
            Some(value) => value
                .trim()
                .parse::<usize>()
                .map_err(|_| io::Error::new(ErrorKind::InvalidData, "invalid content-length"))?,
            None => 0,
        };
        if length > MAX_REQUEST_BYTES {
            return Err(io::Error::new(ErrorKind::InvalidInput, "request too large"));
        }
        if body.len() < length {
            let mut tail = vec![0; length - body.len()];
            stream.read_exact(&mut tail)?;
            body.extend_from_slice(&tail);
        }
        body.truncate(length);
    }
    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

fn parse_headers<'a>(lines: impl Iterator<Item = &'a str>) -> io::Result<Vec<(String, String)>> {
    let mut headers = Vec::new();
    for line in lines.take_while(|l| !l.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            return Err(io::Error::new(ErrorKind::InvalidData, "malformed header"));
        };
        // A space before the colon or a control char in the value is the classic
        // smuggling / header-injection vector; refuse instead of normalizing.
        if !is_token(name) || value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
            return Err(io::Error::new(ErrorKind::InvalidData, "malformed header"));
        }
        headers.push((name.to_owned(), value.trim().to_owned()));
    }
    Ok(headers)
}

fn is_chunked(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("transfer-encoding"))
        .any(|(_, v)| {
            v.split(',')
                .any(|x| x.trim().eq_ignore_ascii_case("chunked"))
        })
}

fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

fn is_request_target(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 8192
        && value
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'\\' && b != b'"')
}

fn read_until_headers(stream: &mut TcpStream, limit: usize) -> io::Result<Vec<u8>> {
    let mut data = Vec::with_capacity(4096);
    let mut chunk = [0; 8192];
    loop {
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..count]);
        if data.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if data.len() > limit {
            return Err(io::Error::new(ErrorKind::InvalidInput, "head too large"));
        }
    }
    Ok(data)
}

fn read_chunked_body(stream: &mut TcpStream, mut buffered: Vec<u8>) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        while !buffered.windows(2).any(|w| w == b"\r\n") {
            read_more(stream, &mut buffered)?;
        }
        let end = buffered.windows(2).position(|w| w == b"\r\n").unwrap();
        let size = usize::from_str_radix(
            String::from_utf8_lossy(&buffered[..end])
                .split(';')
                .next()
                .unwrap_or("")
                .trim(),
            16,
        )
        .map_err(|_| io::Error::new(ErrorKind::InvalidData, "invalid chunk size"))?;
        buffered.drain(..end + 2);
        if size == 0 {
            break;
        }
        while buffered.len() < size + 2 {
            read_more(stream, &mut buffered)?;
        }
        body.extend_from_slice(&buffered[..size]);
        buffered.drain(..size + 2);
        if body.len() > MAX_REQUEST_BYTES {
            return Err(io::Error::new(ErrorKind::InvalidInput, "request too large"));
        }
    }
    Ok(body)
}

fn read_more(stream: &mut TcpStream, buffer: &mut Vec<u8>) -> io::Result<()> {
    let mut chunk = [0; 8192];
    let count = stream.read(&mut chunk)?;
    if count == 0 {
        return Err(io::Error::new(
            ErrorKind::UnexpectedEof,
            "request ended early",
        ));
    }
    buffer.extend_from_slice(&chunk[..count]);
    Ok(())
}

struct ResponseHead {
    status: u16,
    reason: String,
    headers: Vec<(String, String)>,
    buffered_body: Vec<u8>,
}

fn open_upstream(request: &Request, config: &Config, model: &str) -> io::Result<TcpStream> {
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

fn read_response_head(stream: &mut TcpStream) -> io::Result<ResponseHead> {
    let raw = read_until_headers(stream, MAX_HEAD_BYTES)?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .ok_or_else(|| io::Error::new(ErrorKind::UnexpectedEof, "incomplete response head"))?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("HTTP/1.1 502 Bad Gateway");
    let mut status_parts = status_line.splitn(3, ' ');
    let _ = status_parts.next();
    let status = status_parts
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(502);
    let reason = status_parts
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    let headers = parse_headers(lines)?;
    Ok(ResponseHead {
        status,
        reason,
        headers,
        buffered_body: raw[split..].to_vec(),
    })
}

fn stream_response(
    client: &mut TcpStream,
    upstream: &mut TcpStream,
    head: ResponseHead,
    request_body: &[u8],
) -> io::Result<()> {
    write!(client, "HTTP/1.1 {} {}\r\n", head.status, head.reason)?;
    for (name, value) in &head.headers {
        if name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("content-encoding")
            || name.eq_ignore_ascii_case("etag")
            || HOP_BY_HOP.iter().any(|h| name.eq_ignore_ascii_case(h))
        {
            continue;
        }
        write!(client, "{name}: {value}\r\n")?;
    }
    write!(
        client,
        "Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    )?;

    let mut buffered = head.buffered_body;
    let chunked = is_chunked(&head.headers);
    let length = header_value(&head.headers, "content-length").and_then(|v| v.parse().ok());
    let mut remaining = length;
    let mut output = Vec::with_capacity(8192);
    loop {
        if chunked {
            let decoded = read_one_chunk(upstream, &mut buffered)?;
            let Some(decoded) = decoded else { break };
            output.extend_from_slice(&decoded);
        } else if remaining == Some(0) {
            break;
        } else {
            if buffered.is_empty() {
                let mut temp = [0u8; 8192];
                let count = upstream.read(&mut temp)?;
                if count == 0 {
                    break;
                }
                buffered.extend_from_slice(&temp[..count]);
            }
            let take = remaining.map_or(buffered.len(), |n| n.min(buffered.len()));
            output.extend(buffered.drain(..take));
            if let Some(n) = &mut remaining {
                *n -= take;
            }
        }
        // Only flush up to the last newline: a `"name":"..."` pair split across
        // two reads would otherwise escape rewriting. A body with no newline is
        // flushed once it exceeds the rewrite window so memory stays bounded.
        if let Some(end) = output.iter().rposition(|b| *b == b'\n') {
            let rest = output.split_off(end + 1);
            write_chunk(client, &output, request_body)?;
            output = rest;
        } else if output.len() > MAX_REWRITE_WINDOW {
            write_chunk(client, &output, request_body)?;
            output.clear();
        }
    }
    write_chunk(client, &output, request_body)?;
    client.write_all(b"0\r\n\r\n")
}

fn write_chunk(client: &mut TcpStream, body: &[u8], request_body: &[u8]) -> io::Result<()> {
    if body.is_empty() {
        return Ok(());
    }
    let rewritten = rewrite_tool_names(body, request_body);
    write!(client, "{:X}\r\n", rewritten.len())?;
    client.write_all(&rewritten)?;
    client.write_all(b"\r\n")
}

fn read_one_chunk(stream: &mut TcpStream, buffered: &mut Vec<u8>) -> io::Result<Option<Vec<u8>>> {
    while !buffered.windows(2).any(|w| w == b"\r\n") {
        read_more(stream, buffered)?;
    }
    let end = buffered.windows(2).position(|w| w == b"\r\n").unwrap();
    let size = usize::from_str_radix(
        String::from_utf8_lossy(&buffered[..end])
            .split(';')
            .next()
            .unwrap_or("")
            .trim(),
        16,
    )
    .map_err(|_| io::Error::new(ErrorKind::InvalidData, "invalid chunk size"))?;
    buffered.drain(..end + 2);
    if size == 0 {
        return Ok(None);
    }
    while buffered.len() < size + 2 {
        read_more(stream, buffered)?;
    }
    let body = buffered[..size].to_vec();
    buffered.drain(..size + 2);
    Ok(Some(body))
}

fn request_candidates(body: &[u8], fallbacks: &[String], rotation: usize) -> Vec<String> {
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

fn replace_model(body: &[u8], model: &str) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(body) else {
        return body.to_vec();
    };
    let Some(start) = text.find("\"model\"") else {
        return body.to_vec();
    };
    let Some(colon) = text[start..].find(':') else {
        return body.to_vec();
    };
    let value_start = start + colon + 1;
    let Some(open) = text[value_start..].find('"') else {
        return body.to_vec();
    };
    let content_start = value_start + open + 1;
    let Some(close) = text[content_start..].find('"') else {
        return body.to_vec();
    };
    let content_end = content_start + close;
    format!(
        "{}{}{}",
        &text[..content_start],
        escape_json_string(model),
        &text[content_end..]
    )
    .into_bytes()
}

fn rewrite_tool_names(body: &[u8], request_body: &[u8]) -> Vec<u8> {
    let Ok(mut output) = String::from_utf8(body.to_vec()) else {
        return body.to_vec();
    };
    let Ok(request) = std::str::from_utf8(request_body) else {
        return body.to_vec();
    };
    let mut rest = request;
    while let Some(index) = rest.find("\"name\"") {
        rest = &rest[index + 6..];
        let Some(open) = rest.find('"') else { break };
        let value = &rest[open + 1..];
        let Some(close) = value.find('"') else { break };
        let original = &value[..close];
        rest = &value[close + 1..];
        if original.is_empty() || !original.chars().any(|c| c.is_ascii_uppercase()) {
            continue;
        }
        let lower = original.to_ascii_lowercase();
        let mut cursor = 0;
        while let Some(index) = output[cursor..].to_ascii_lowercase().find("\"name\"") {
            let abs_name = cursor + index;
            let after_key = abs_name + 6; // past "name"
            let Some(colon_pos) = output[after_key..].find(':') else {
                break;
            };
            let after_colon = after_key + colon_pos + 1;
            let rest_after_colon = &output[after_colon..];
            let trimmed_start = rest_after_colon.len() - rest_after_colon.trim_start().len();
            let value_open = after_colon + trimmed_start;
            if value_open >= output.len() || output.as_bytes()[value_open] != b'"' {
                break;
            }
            let value_content = value_open + 1;
            let Some(close_rel) = output[value_content..].find('"') else {
                break;
            };
            let value_end = value_content + close_rel;
            let current_value = &output[value_content..value_end];
            if current_value.eq_ignore_ascii_case(&lower) {
                let span = value_content..value_end;
                output.replace_range(span.clone(), original);
                cursor = value_content + original.len();
            } else {
                cursor = value_end + 1;
            }
        }
    }
    output.into_bytes()
}

fn json_string_value(text: &str, key: &str) -> Option<String> {
    let start = text.find(&format!("\"{key}\""))?;
    let rest = &text[start + key.len() + 2..];
    let open = rest.find('"')?;
    let value = &rest[open + 1..];
    Some(value[..value.find('"')?].to_owned())
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn escape_json_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn write_error(client: &mut TcpStream, status: u16, reason: &str, message: &str) -> io::Result<()> {
    let body = format!(
        "{{\"error\":{{\"message\":\"{}\"}}}}",
        escape_json_string(message)
    );
    write!(client, "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}", body.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_model_without_changing_other_fields() {
        assert_eq!(
            replace_model(br#"{"model":"old","tools":[]}"#, "fail-try"),
            br#"{"model":"fail-try","tools":[]}"#
        );
    }

    #[test]
    fn keeps_413_out_of_retry_statuses() {
        assert!(!RETRYABLE.contains(&413));
        assert!(RETRYABLE.contains(&403));
    }

    #[test]
    fn rewrites_tool_name_after_chunked_body_is_decoded() {
        let request = br#"{"tools":[{"name":"MyTool"}]}"#;
        let decoded = br#"{"name":"mytool"}"#;
        assert_eq!(
            rewrite_tool_names(decoded, request),
            br#"{"name":"MyTool"}"#
        );
    }

    #[test]
    fn rewrite_needs_full_name_pair() {
        let request = br#"{"tools":[{"name":"MyTool"}]}"#;
        assert_eq!(
            rewrite_tool_names(br#"{"name":"my"#, request),
            br#"{"name":"my"#
        );
        assert_eq!(rewrite_tool_names(br#"tool"}"#, request), br#"tool"}"#);
        let mut joined = rewrite_tool_names(br#"{"name":"my"#, request);
        joined.extend_from_slice(br#"tool"}"#);
        assert_eq!(
            rewrite_tool_names(&joined, request),
            br#"{"name":"MyTool"}"#
        );
    }

    #[test]
    fn rewrites_tool_name_with_space_after_colon() {
        let request = br#"{"tools":[{"name":"MyTool"}]}"#;
        let decoded = br#"{"name": "mytool"}"#;
        assert_eq!(
            rewrite_tool_names(decoded, request),
            br#"{"name": "MyTool"}"#
        );
    }

    #[test]
    fn filters_framing_headers() {
        assert!(HOP_BY_HOP
            .iter()
            .any(|h| h.eq_ignore_ascii_case("transfer-encoding")));
        assert!(!["content-type", "content-length"]
            .iter()
            .any(|h| HOP_BY_HOP.iter().any(|x| x.eq_ignore_ascii_case(h))));
    }

    #[test]
    fn forces_identity_encoding_upstream() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let upstream = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
            let content_length = request
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse::<usize>()
                .unwrap();
            let mut body = vec![0; content_length];
            socket.read_exact(&mut body).unwrap();
            let encodings: Vec<_> = request
                .lines()
                .filter(|line| line.starts_with("accept-encoding:"))
                .collect();
            assert_eq!(encodings, ["accept-encoding: identity"]);
            assert!(!request.contains("gzip"));
            let body = br#"{"name":"mytool"}"#;
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .unwrap();
            socket.write_all(body).unwrap();
        });

        let request = Request {
            method: "GET".into(),
            path: "/dashboard".into(),
            headers: vec![("Accept-Encoding".into(), "gzip, br".into())],
            body: br#"{"model":"m","tools":[{"name":"MyTool"}]}"#.to_vec(),
        };
        let config = Config {
            target_host: "127.0.0.1".into(),
            target_port: port,
            fallbacks: vec![],
            io_timeout: Some(Duration::from_secs(5)),
        };
        let mut socket = open_upstream(&request, &config, "m").unwrap();
        let head = read_response_head(&mut socket).unwrap();
        let length = header_value(&head.headers, "content-length")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let mut body = head.buffered_body;
        while body.len() < length {
            read_more(&mut socket, &mut body).unwrap();
        }
        body.truncate(length);
        upstream.join().unwrap();
        assert_eq!(head.status, 200);
        assert_eq!(
            rewrite_tool_names(&body, &request.body),
            br#"{"name":"MyTool"}"#
        );
    }

    #[test]
    fn rejects_conflicting_framing_headers() {
        let headers = vec![
            ("Transfer-Encoding".to_string(), "chunked".to_string()),
            ("Content-Length".to_string(), "7".to_string()),
        ];
        assert!(is_chunked(&headers));
        let lengths = headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .count();
        assert_eq!(lengths, 1);
    }

    #[test]
    fn rejects_header_injection_in_names() {
        assert!(is_token("Content-Type"));
        assert!(!is_token("Content Type"));
        assert!(!is_token(""));
        assert!(parse_headers(["X-Bad : 1"].into_iter()).is_err());
        assert!(parse_headers(["X-Good: 1"].into_iter()).is_ok());
    }

    #[test]
    fn rejects_malformed_request_targets() {
        assert!(is_request_target("/v1/chat/completions"));
        assert!(!is_request_target("/pa th"));
        assert!(!is_request_target(""));
    }

    #[test]
    fn error_body_escapes_message() {
        assert_eq!(escape_json_string("a\"b\\c"), "a\\\"b\\\\c");
    }
}
