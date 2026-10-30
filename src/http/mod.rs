mod response;

pub use response::{
    http_status_text, io_error_to_http_status, is_loopback_origin, read_one_chunk, stream_response,
    write_chunk, write_error, write_json_ok, MAX_STREAM_BYTES,
};

use std::io::{self, ErrorKind, Read};
use std::net::TcpStream;

const MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024;
const MAX_HEAD_BYTES: usize = 64 * 1024;
pub const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct ResponseHead {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub buffered_body: Vec<u8>,
}

pub fn read_request(stream: &mut TcpStream) -> io::Result<Request> {
    let admission = crate::runtime::admission::AdmissionController::new(1, usize::MAX);
    read_request_with_admission(stream, &admission).map(|(request, _)| request)
}

pub fn read_request_with_admission(
    stream: &mut TcpStream,
    admission: &crate::runtime::admission::AdmissionController,
) -> io::Result<(Request, crate::runtime::admission::ByteGuard)> {
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
    let mut byte_guard = admission
        .acquire_bytes(length)
        .ok_or_else(|| io::Error::other("too many in-flight bytes"))?;
    let mut body = raw[split..].to_vec();
    if chunked {
        body = read_chunked_body_with_admission(stream, body, admission, &mut byte_guard)?;
    } else {
        if body.len() < length {
            let mut tail = vec![0; length - body.len()];
            stream.read_exact(&mut tail)?;
            body.extend_from_slice(&tail);
        }
        body.truncate(length);
    }
    Ok((
        Request {
            method,
            path,
            headers,
            body,
        },
        byte_guard,
    ))
}

pub fn parse_headers<'a>(
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
        };
        headers.push((name.to_owned(), value.trim().to_owned()));
    }
    Ok(headers)
}

pub fn is_chunked(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("transfer-encoding"))
        .any(|(_, v)| {
            v.split(',')
                .any(|x| x.trim().eq_ignore_ascii_case("chunked"))
        })
}

pub fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

pub fn is_request_target(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 8192
        && value
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'\\' && b != b'"')
}

pub fn read_until_headers(stream: &mut TcpStream, limit: usize) -> io::Result<Vec<u8>> {
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

pub fn read_chunked_body(stream: &mut TcpStream, buffered: Vec<u8>) -> io::Result<Vec<u8>> {
    let admission = crate::runtime::admission::AdmissionController::new(1, usize::MAX);
    let mut guard = admission
        .acquire_bytes(0)
        .ok_or_else(|| io::Error::other("failed to initialize admission byte guard"))?;
    read_chunked_body_with_admission(stream, buffered, &admission, &mut guard)
}

fn read_chunked_body_with_admission(
    stream: &mut TcpStream,
    mut buffered: Vec<u8>,
    admission: &crate::runtime::admission::AdmissionController,
    byte_guard: &mut crate::runtime::admission::ByteGuard,
) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        while !buffered.windows(2).any(|w| w == b"\r\n") {
            read_more(stream, &mut buffered)?;
            if buffered.len() > MAX_REQUEST_BYTES {
                return Err(io::Error::new(ErrorKind::InvalidInput, "request too large"));
            }
        }
        let size = parse_chunk_size(&mut buffered)?;
        if size == 0 {
            break;
        }
        let total = body
            .len()
            .checked_add(size)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "request too large"))?;
        if total > MAX_REQUEST_BYTES {
            return Err(io::Error::new(ErrorKind::InvalidInput, "request too large"));
        }
        if !admission.acquire_more_bytes(byte_guard, size) {
            return Err(io::Error::other("too many in-flight bytes"));
        }
        let framed_size = size
            .checked_add(2)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "invalid chunk size"))?;
        while buffered.len() < framed_size {
            read_more(stream, &mut buffered)?;
            if buffered.len() > MAX_REQUEST_BYTES {
                return Err(io::Error::new(ErrorKind::InvalidInput, "request too large"));
            }
        }
        body.extend_from_slice(&buffered[..size]);
        buffered.drain(..framed_size);
    }
    Ok(body)
}

pub fn read_more(stream: &mut TcpStream, buffer: &mut Vec<u8>) -> io::Result<()> {
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

pub fn read_response_head(stream: &mut TcpStream) -> io::Result<ResponseHead> {
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

pub(super) fn parse_chunk_size(buffered: &mut Vec<u8>) -> io::Result<usize> {
    let end = buffered
        .windows(2)
        .position(|w| w == b"\r\n")
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "missing chunk delimiter"))?;
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
    Ok(size)
}

pub fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}
