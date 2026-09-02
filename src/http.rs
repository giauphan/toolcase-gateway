use crate::rewrite::{escape_json_string, rewrite_tool_names};
use std::io::{self, ErrorKind, Read, Write};

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
use std::net::TcpStream;

pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct ResponseHead {
    pub(crate) status: u16,
    pub(crate) reason: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) buffered_body: Vec<u8>,
}

pub(crate) fn read_request(stream: &mut TcpStream) -> io::Result<Request> {
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

pub(crate) fn parse_headers<'a>(
    lines: impl Iterator<Item = &'a str>,
) -> io::Result<Vec<(String, String)>> {
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

pub(crate) fn is_chunked(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("transfer-encoding"))
        .any(|(_, v)| {
            v.split(',')
                .any(|x| x.trim().eq_ignore_ascii_case("chunked"))
        })
}

pub(crate) fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

pub(crate) fn is_request_target(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 8192
        && value
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'\\' && b != b'"')
}

pub(crate) fn read_until_headers(stream: &mut TcpStream, limit: usize) -> io::Result<Vec<u8>> {
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

pub(crate) fn read_chunked_body(
    stream: &mut TcpStream,
    mut buffered: Vec<u8>,
) -> io::Result<Vec<u8>> {
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

pub(crate) fn read_more(stream: &mut TcpStream, buffer: &mut Vec<u8>) -> io::Result<()> {
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

pub(crate) fn read_response_head(stream: &mut TcpStream) -> io::Result<ResponseHead> {
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

pub(crate) fn stream_response(
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

pub(crate) fn write_chunk(
    client: &mut TcpStream,
    body: &[u8],
    request_body: &[u8],
) -> io::Result<()> {
    if body.is_empty() {
        return Ok(());
    }
    let rewritten = rewrite_tool_names(body, request_body);
    write!(client, "{:X}\r\n", rewritten.len())?;
    client.write_all(&rewritten)?;
    client.write_all(b"\r\n")
}

pub(crate) fn read_one_chunk(
    stream: &mut TcpStream,
    buffered: &mut Vec<u8>,
) -> io::Result<Option<Vec<u8>>> {
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

pub(crate) fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

pub(crate) fn write_error(
    client: &mut TcpStream,
    status: u16,
    reason: &str,
    message: &str,
) -> io::Result<()> {
    let body = format!(
        "{{\"error\":{{\"message\":\"{}\"}}}}",
        escape_json_string(message)
    );
    write!(client, "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}", body.len())
}
