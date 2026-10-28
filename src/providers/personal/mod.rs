//! Adapter for out-of-tree personal provider modules executed out-of-process.

use crate::gateway::modules::PersonalManifest;
use crate::http::Request;
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

const DEFAULT_MODULE_TIMEOUT: Duration = Duration::from_secs(30);

/// Dispatches an inbound HTTP request to an out-of-process personal module.
pub fn handle_personal_module_request(
    client: &mut TcpStream,
    request: &Request,
    manifest: &PersonalManifest,
    subpath: &str,
) -> io::Result<u16> {
    match manifest.protocol.as_str() {
        "local-http" => dispatch_local_http(client, request, manifest, subpath),
        _ => dispatch_stdio(client, request, manifest, subpath),
    }
}

fn dispatch_stdio(
    client: &mut TcpStream,
    request: &Request,
    manifest: &PersonalManifest,
    subpath: &str,
) -> io::Result<u16> {
    let mut cmd = Command::new(&manifest.entrypoint);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    // Filter environment variables to isolate secrets
    cmd.env_clear();
    cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
    cmd.env("MODULE_ID", &manifest.id);
    cmd.env("MODULE_NAMESPACE", &manifest.namespace);

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return write_personal_error(
                client,
                502,
                &format!("Failed to spawn personal module '{}': {e}", manifest.id),
            );
        }
    };

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "Failed to open child stdin"))?;

    let payload = serde_json::json!({
        "method": request.method,
        "path": subpath,
        "headers": request.headers,
        "body": String::from_utf8_lossy(&request.body),
    });

    let mut payload_bytes = serde_json::to_vec(&payload).unwrap_or_default();
    payload_bytes.push(b'\n');

    let writer_handle = std::thread::spawn(move || {
        let _ = stdin.write_all(&payload_bytes);
        drop(stdin);
    });

    let stdout = child.stdout.take().ok_or_else(|| {
        io::Error::new(io::ErrorKind::BrokenPipe, "Failed to capture child stdout")
    })?;

    let (tx, rx) = std::sync::mpsc::channel();
    let reader_handle = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let res = reader.read_line(&mut line).map(|_| line);
        let _ = tx.send(res);
    });

    let response_line = match rx.recv_timeout(DEFAULT_MODULE_TIMEOUT) {
        Ok(Ok(line)) => {
            let _ = writer_handle.join();
            let _ = reader_handle.join();
            let _ = child.wait();
            line
        }
        Ok(Err(e)) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer_handle.join();
            let _ = reader_handle.join();
            return write_personal_error(
                client,
                502,
                &format!("Failed to read response from personal module: {e}"),
            );
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer_handle.join();
            let _ = reader_handle.join();
            return write_personal_error(
                client,
                504,
                "Personal module execution timed out (30s limit)",
            );
        }
    };

    if response_line.trim().is_empty() {
        return write_personal_error(
            client,
            502,
            "Personal module returned empty response or crashed",
        );
    }

    let resp_val: serde_json::Value = match serde_json::from_str(&response_line) {
        Ok(v) => v,
        Err(e) => {
            return write_personal_error(
                client,
                502,
                &format!("Personal module returned invalid JSON: {e}"),
            );
        }
    };

    let status = resp_val
        .get("status")
        .and_then(|s| s.as_u64())
        .filter(|s| (100..=599).contains(s))
        .unwrap_or(200) as u16;

    let body_str = resp_val.get("body").and_then(|b| b.as_str()).unwrap_or("");

    let content_type = resp_val
        .get("headers")
        .and_then(|h| h.get("content-type"))
        .and_then(|c| c.as_str())
        .filter(|c| !c.contains(['\r', '\n']))
        .unwrap_or("application/json");

    let reason = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Response",
    };

    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_str}",
        body_str.len()
    );

    client.write_all(response.as_bytes())?;
    client.flush()?;
    Ok(status)
}

fn dispatch_local_http(
    client: &mut TcpStream,
    request: &Request,
    manifest: &PersonalManifest,
    subpath: &str,
) -> io::Result<u16> {
    let base_url = manifest.entrypoint.trim_end_matches('/');
    let clean_sub = subpath.trim_start_matches('/');
    let target_url = format!("{base_url}/{clean_sub}");

    let agent_config = ureq::Agent::config_builder()
        .timeout_global(Some(DEFAULT_MODULE_TIMEOUT))
        .max_redirects(0)
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(agent_config);

    let is_post = request.method.eq_ignore_ascii_case("post");
    let is_get = request.method.eq_ignore_ascii_case("get");
    if !is_post && !is_get {
        return write_personal_error(
            client,
            405,
            &format!(
                "Method '{}' is not supported for local-http personal modules",
                request.method
            ),
        );
    }

    let resp = match if is_post {
        let mut req = agent.post(&target_url);
        for (k, v) in &request.headers {
            if !k.eq_ignore_ascii_case("host") && !k.eq_ignore_ascii_case("content-length") {
                req = req.header(k, v);
            }
        }
        req.send(&request.body)
    } else {
        let mut req = agent.get(&target_url);
        for (k, v) in &request.headers {
            if !k.eq_ignore_ascii_case("host") && !k.eq_ignore_ascii_case("content-length") {
                req = req.header(k, v);
            }
        }
        req.call()
    } {
        Ok(r) => r,
        Err(e) => {
            return write_personal_error(
                client,
                502,
                &format!("Upstream personal module error: {e}"),
            );
        }
    };

    let status = resp.status().as_u16();
    crate::http_provider::write_ureq_response(client, resp)?;
    Ok(status)
}

fn write_personal_error(client: &mut TcpStream, status: u16, message: &str) -> io::Result<u16> {
    let error_body = serde_json::json!({
        "error": {
            "message": format!("toolcase-gateway: {message}"),
            "type": "personal_module_error",
            "code": status
        }
    });
    let data = serde_json::to_vec(&error_body).unwrap_or_default();
    let reason = match status {
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        504 => "Gateway Timeout",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        data.len()
    );
    client.write_all(head.as_bytes())?;
    client.write_all(&data)?;
    client.flush()?;
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_personal_error_format() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (mut server, _) = listener.accept().unwrap();

        let status = write_personal_error(&mut server, 502, "test fault").unwrap();
        assert_eq!(status, 502);
        drop(server);

        let mut buf = Vec::new();
        io::Read::read_to_end(&mut client, &mut buf).unwrap();
        let response_str = String::from_utf8_lossy(&buf);
        assert!(response_str.contains("502 Bad Gateway"));
        assert!(response_str.contains("personal_module_error"));
    }
}
