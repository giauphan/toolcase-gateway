use super::{header_value, is_chunked, parse_chunk_size, read_more, ResponseHead, HOP_BY_HOP};
use crate::rewrite::{escape_json_string, rewrite_tool_names};
use std::io::{self, ErrorKind, Read, Write};
use std::net::TcpStream;

const MAX_REWRITE_WINDOW: usize = 1024 * 1024;
pub const MAX_STREAM_BYTES: usize = 128 * 1024 * 1024; // 128 MB maximum streamed bytes guard

fn write_empty_body_response(client: &mut TcpStream, head: &ResponseHead) -> io::Result<()> {
    write!(client, "HTTP/1.1 {} {}\r\n", head.status, head.reason)?;
    for (name, value) in &head.headers {
        if name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("transfer-encoding")
            || HOP_BY_HOP.iter().any(|h| name.eq_ignore_ascii_case(h))
        {
            continue;
        }
        write!(client, "{name}: {value}\r\n")?;
    }
    write!(client, "Connection: close\r\n\r\n")?;
    client.flush()
}

fn write_chunked_response_headers(client: &mut TcpStream, head: &ResponseHead) -> io::Result<()> {
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
    )
}

fn pump_chunked_input(
    upstream: &mut TcpStream,
    buffered: &mut Vec<u8>,
    output: &mut Vec<u8>,
    total_streamed: &mut usize,
) -> io::Result<bool> {
    let decoded = read_one_chunk(upstream, buffered)?;
    let Some(decoded) = decoded else {
        return Ok(false);
    };
    *total_streamed = total_streamed.saturating_add(decoded.len());
    if *total_streamed > MAX_STREAM_BYTES {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "stream exceeded maximum byte budget",
        ));
    }
    output.extend_from_slice(&decoded);
    Ok(true)
}

fn pump_fixed_input(
    upstream: &mut TcpStream,
    buffered: &mut Vec<u8>,
    remaining: &mut Option<usize>,
    output: &mut Vec<u8>,
    total_streamed: &mut usize,
) -> io::Result<bool> {
    if *remaining == Some(0) {
        return Ok(false);
    }
    if buffered.is_empty() {
        let mut temp = [0u8; 8192];
        let count = upstream.read(&mut temp)?;
        if count == 0 {
            return Ok(false);
        }
        buffered.extend_from_slice(&temp[..count]);
    }
    let take = remaining.map_or(buffered.len(), |n| n.min(buffered.len()));
    *total_streamed = total_streamed.saturating_add(take);
    if *total_streamed > MAX_STREAM_BYTES {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "stream exceeded maximum byte budget",
        ));
    }
    output.extend(buffered.drain(..take));
    if let Some(n) = remaining {
        *n -= take;
    }
    Ok(true)
}

fn flush_rewrite_window(
    client: &mut TcpStream,
    output: &mut Vec<u8>,
    request_body: &[u8],
) -> io::Result<()> {
    if let Some(end) = output.iter().rposition(|b| *b == b'\n') {
        let rest = output.split_off(end + 1);
        write_chunk(client, output, request_body)?;
        *output = rest;
    } else if output.len() > MAX_REWRITE_WINDOW {
        write_chunk(client, output, request_body)?;
        output.clear();
    }
    Ok(())
}

pub fn stream_response(
    client: &mut TcpStream,
    upstream: &mut TcpStream,
    head: ResponseHead,
    request_body: &[u8],
) -> io::Result<()> {
    if head.status == 304 || head.status == 204 || (100..=199).contains(&head.status) {
        return write_empty_body_response(client, &head);
    }

    write_chunked_response_headers(client, &head)?;

    let mut total_streamed: usize = 0;
    let mut buffered = head.buffered_body;
    let chunked = is_chunked(&head.headers);
    let mut remaining = header_value(&head.headers, "content-length").and_then(|v| v.parse().ok());
    let mut output = Vec::with_capacity(8192);
    loop {
        let has_more = if chunked {
            pump_chunked_input(upstream, &mut buffered, &mut output, &mut total_streamed)?
        } else {
            pump_fixed_input(
                upstream,
                &mut buffered,
                &mut remaining,
                &mut output,
                &mut total_streamed,
            )?
        };

        if !has_more {
            break;
        }

        flush_rewrite_window(client, &mut output, request_body)?;
    }
    write_chunk(client, &output, request_body)?;
    client.write_all(b"0\r\n\r\n")
}

pub fn write_chunk(client: &mut TcpStream, body: &[u8], request_body: &[u8]) -> io::Result<()> {
    if body.is_empty() {
        return Ok(());
    }
    let rewritten = rewrite_tool_names(body, request_body);
    write!(client, "{:X}\r\n", rewritten.len())?;
    client.write_all(&rewritten)?;
    client.write_all(b"\r\n")
}

pub fn read_one_chunk(
    stream: &mut TcpStream,
    buffered: &mut Vec<u8>,
) -> io::Result<Option<Vec<u8>>> {
    while !buffered.windows(2).any(|w| w == b"\r\n") {
        read_more(stream, buffered)?;
    }
    let size = parse_chunk_size(buffered)?;
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

pub fn write_error(
    client: &mut TcpStream,
    status: u16,
    reason: &str,
    message: &str,
) -> io::Result<()> {
    let escaped = escape_json_string(message);
    let body = format!("{{\"error\":{{\"message\":\"{escaped}\"}},\"message\":\"{escaped}\"}}");
    write!(client, "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}", body.len())
}

/// Maps a numeric HTTP status code to its standard reason phrase.
pub fn http_status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        409 => "Conflict",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Internal Server Error",
    }
}

/// Maps an `io::ErrorKind` to its corresponding HTTP status code and reason phrase.
pub fn io_error_to_http_status(kind: io::ErrorKind) -> (u16, &'static str) {
    match kind {
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => (400, "Bad Request"),
        io::ErrorKind::PermissionDenied => (401, "Unauthorized"),
        io::ErrorKind::NotFound => (404, "Not Found"),
        io::ErrorKind::WouldBlock | io::ErrorKind::AlreadyExists => (409, "Conflict"),
        io::ErrorKind::Unsupported => (501, "Not Implemented"),
        io::ErrorKind::TimedOut => (504, "Gateway Timeout"),
        _ => (502, "Bad Gateway"),
    }
}

/// Writes a 200 OK JSON response body to the client stream.
pub fn write_json_ok(client: &mut TcpStream, body: &str) -> io::Result<()> {
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

/// Verifies whether an Origin header matches an exact loopback host.
pub fn is_loopback_origin(origin: &str) -> bool {
    let lower = origin.to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return false;
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}
