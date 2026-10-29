//! Mounts an upstream web app under a gateway module prefix (e.g. `/omniroute`).
//!
//! The upstream app emits root-relative URLs (`/dashboard/providers`, `/_next/...`).
//! Served as-is they would collide with gateway routes, so text responses are
//! rewritten to carry the module prefix. Only quoted string literals and CSS
//! `url(...)` references that start with a single `/` are touched.

use crate::http::{header_value, read_one_chunk, ResponseHead};
use std::io::{self, ErrorKind, Read, Write};
use std::net::TcpStream;

/// Largest text body that will be buffered for rewriting.
pub const MAX_REWRITE_BODY: usize = 16 * 1024 * 1024;

/// True when the response is HTML, JavaScript or CSS and therefore rewritable.
pub fn is_rewritable(head: &ResponseHead) -> bool {
    let Some(ct) = header_value(&head.headers, "content-type") else {
        return false;
    };
    let ct = ct.to_ascii_lowercase();
    ct.starts_with("text/html")
        || ct.starts_with("text/css")
        || ct.contains("javascript")
        || ct.contains("ecmascript")
}

/// Prefixes root-relative URL literals in `body` with `prefix`.
pub fn rewrite_root_relative(body: &str, prefix: &str) -> String {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len() + 256);
    let mut last = 0;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let quoted = matches!(c, b'"' | b'\'' | b'`');
        let css_url = c == b'(' && i >= 3 && &bytes[i - 3..i] == b"url";
        if (quoted || css_url) && is_root_relative(&body[i + 1..], prefix) {
            out.push_str(&body[last..=i]);
            out.push_str(prefix);
            last = i + 1;
        }
        i += 1;
    }
    out.push_str(&body[last..]);
    out
}

/// `/name...` (single slash followed by a path character) and not already prefixed.
fn is_root_relative(rest: &str, prefix: &str) -> bool {
    let mut chars = rest.chars();
    if chars.next() != Some('/') {
        return false;
    }
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() || c == '_' => {}
        _ => return false,
    }
    let already = rest
        .strip_prefix(prefix)
        .is_some_and(|r| r.is_empty() || r.starts_with(['/', '?', '#', '"', '\'', '`', ')']));
    !already
}

/// Reads the complete upstream body (chunked or length-delimited), bounded by [`MAX_REWRITE_BODY`].
pub fn read_full_body(upstream: &mut TcpStream, head: &ResponseHead) -> io::Result<Vec<u8>> {
    let mut buffered = head.buffered_body.clone();
    let mut body = Vec::new();
    let too_big = || io::Error::new(ErrorKind::InvalidData, "rewritable body too large");
    if crate::http::is_chunked(&head.headers) {
        while let Some(chunk) = read_one_chunk(upstream, &mut buffered)? {
            body.extend_from_slice(&chunk);
            if body.len() > MAX_REWRITE_BODY {
                return Err(too_big());
            }
        }
        return Ok(body);
    }
    let length: Option<usize> =
        header_value(&head.headers, "content-length").and_then(|v| v.parse().ok());
    body.append(&mut buffered);
    let mut temp = [0u8; 8192];
    while length.map_or(true, |n| body.len() < n) {
        let count = upstream.read(&mut temp)?;
        if count == 0 {
            break;
        }
        body.extend_from_slice(&temp[..count]);
        if body.len() > MAX_REWRITE_BODY {
            return Err(too_big());
        }
    }
    if let Some(n) = length {
        body.truncate(n);
    }
    Ok(body)
}

/// Writes `head` with a rewritten, length-delimited body.
pub fn write_rewritten(client: &mut TcpStream, head: &ResponseHead, body: &[u8]) -> io::Result<()> {
    write!(client, "HTTP/1.1 {} {}\r\n", head.status, head.reason)?;
    for (name, value) in &head.headers {
        if name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("content-encoding")
            || name.eq_ignore_ascii_case("etag")
            || name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("connection")
            || name.eq_ignore_ascii_case("keep-alive")
        {
            continue;
        }
        write!(client, "{name}: {value}\r\n")?;
    }
    write!(
        client,
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    client.write_all(body)?;
    client.flush()
}

#[cfg(test)]
mod tests {
    use super::rewrite_root_relative;

    #[test]
    fn prefixes_quoted_root_relative_paths() {
        let html = r#"<a href="/dashboard/providers"><script src='/_next/a.js'></script>"#;
        assert_eq!(
            rewrite_root_relative(html, "/omniroute"),
            r#"<a href="/omniroute/dashboard/providers"><script src='/omniroute/_next/a.js'></script>"#
        );
    }

    #[test]
    fn leaves_other_literals_alone() {
        for s in [
            r#""//cdn.example.com/x""#,
            r#""https://a.com/x""#,
            r#""/""#,
            r#""text/html""#,
            r#""/omniroute/dashboard""#,
        ] {
            assert_eq!(rewrite_root_relative(s, "/omniroute"), s);
        }
    }

    #[test]
    fn rewrites_css_urls_and_js_strings() {
        assert_eq!(
            rewrite_root_relative("a{background:url(/img/x.png)}", "/omniroute"),
            "a{background:url(/omniroute/img/x.png)}"
        );
        assert_eq!(
            rewrite_root_relative(r#"router.push("/home")"#, "/omniroute"),
            r#"router.push("/omniroute/home")"#
        );
    }
}
