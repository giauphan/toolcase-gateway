//! Stdio-stream process communication, request forwarding, and response decoding.

use super::cmd::{build_command, build_fallback_interpreter_command};
use super::write_personal_error;
use crate::gateway::modules::PersonalManifest;
use crate::http::Request;
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

pub const DEFAULT_MODULE_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn format_crash_message(rx_err: std::sync::mpsc::Receiver<String>) -> String {
    let err_detail = rx_err
        .recv_timeout(Duration::from_millis(150))
        .unwrap_or_default();
    if !err_detail.trim().is_empty() {
        format!(
            "Personal module returned empty response or crashed: {}",
            err_detail.trim()
        )
    } else {
        "Personal module returned empty response or crashed".to_string()
    }
}

pub(crate) fn spawn_stderr_reader(
    stderr: Option<std::process::ChildStderr>,
) -> (
    std::sync::mpsc::Receiver<String>,
    std::thread::JoinHandle<()>,
) {
    let (tx_err, rx_err) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        if let Some(mut err_stream) = stderr {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while let Ok(n) = std::io::Read::read(&mut err_stream, &mut chunk) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > 4096 {
                    break;
                }
            }
            let _ = tx_err.send(String::from_utf8_lossy(&buf).to_string());
        }
    });
    (rx_err, handle)
}

pub(crate) fn wait_for_module_response(
    mut child: std::process::Child,
    rx: std::sync::mpsc::Receiver<std::io::Result<String>>,
    writer_handle: std::thread::JoinHandle<()>,
    reader_handle: std::thread::JoinHandle<()>,
    err_reader_handle: std::thread::JoinHandle<()>,
) -> Result<String, (u16, String)> {
    match rx.recv_timeout(DEFAULT_MODULE_TIMEOUT) {
        Ok(Ok(line)) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer_handle.join();
            let _ = reader_handle.join();
            let _ = err_reader_handle.join();
            Ok(line)
        }
        Ok(Err(e)) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer_handle.join();
            let _ = reader_handle.join();
            let _ = err_reader_handle.join();
            Err((
                502,
                format!("Failed to read response from personal module: {e}"),
            ))
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            drop(writer_handle);
            drop(reader_handle);
            drop(err_reader_handle);
            Err((
                504,
                "Personal module execution timed out (30s limit)".to_string(),
            ))
        }
    }
}

pub(crate) fn dispatch_stdio(
    client: &mut TcpStream,
    request: &Request,
    manifest: &PersonalManifest,
    subpath: &str,
    sub_provider_id: Option<&str>,
) -> io::Result<u16> {
    let mut cmd = build_command(manifest, sub_provider_id);

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            if let Some(mut fallback_cmd) =
                build_fallback_interpreter_command(manifest, sub_provider_id)
            {
                match fallback_cmd.spawn() {
                    Ok(child) => child,
                    Err(fallback_err) => {
                        return write_personal_error(
                            client,
                            502,
                            &format!(
                                "Failed to spawn personal module '{}': {e} (interpreter fallback error: {fallback_err})",
                                manifest.id
                            ),
                        );
                    }
                }
            } else {
                return write_personal_error(
                    client,
                    502,
                    &format!("Failed to spawn personal module '{}': {e}", manifest.id),
                );
            }
        }
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

    let full_subpath = match request.path.split_once('?') {
        Some((_, q)) if !q.is_empty() && !subpath.contains('?') => format!("{subpath}?{q}"),
        _ => subpath.to_string(),
    };

    let payload = serde_json::json!({
        "method": request.method,
        "path": full_subpath,
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

    let (rx_err, err_reader_handle) = spawn_stderr_reader(child.stderr.take());

    let response_line = match wait_for_module_response(
        child,
        rx,
        writer_handle,
        reader_handle,
        err_reader_handle,
    ) {
        Ok(line) => line,
        Err((code, msg)) => return write_personal_error(client, code, &msg),
    };

    if response_line.trim().is_empty() {
        let error_msg = format_crash_message(rx_err);
        crate::runtime::logger::log_message(
            "ERROR",
            "personal_module",
            &format!("[{}] {error_msg}", manifest.id),
        );
        return write_personal_error(client, 502, &error_msg);
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

    let body_owned;
    let body_str = match resp_val.get("body") {
        Some(serde_json::Value::String(s)) => s.as_str(),
        Some(v @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) => {
            body_owned = v.to_string();
            body_owned.as_str()
        }
        _ => "",
    };

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
