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
    handle_personal_module_request_with_provider(client, request, manifest, subpath, None)
}

pub fn handle_personal_module_request_with_provider(
    client: &mut TcpStream,
    request: &Request,
    manifest: &PersonalManifest,
    subpath: &str,
    sub_provider_id: Option<&str>,
) -> io::Result<u16> {
    match manifest.protocol.as_str() {
        "local-http" => dispatch_local_http(client, request, manifest, subpath),
        _ => dispatch_stdio(client, request, manifest, subpath, sub_provider_id),
    }
}

fn heal_legacy_adapter_script(path: &std::path::Path) {
    if let Ok(content) = std::fs::read_to_string(path) {
        if content.contains("cat << 'EOF' | python3") {
            if let Some(parent) = path.parent() {
                let adapter_py = parent.join("adapter.py");
                if let Some(py_code) = content
                    .split("cat << 'EOF' | python3")
                    .nth(1)
                    .and_then(|s| s.split_once('\n'))
                    .and_then(|(_, rest)| rest.split("\nEOF").next())
                {
                    let _ = std::fs::write(&adapter_py, py_code);
                    let fixed_run_sh = r#"#!/bin/sh
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec python3 "$SCRIPT_DIR/adapter.py" "$@"
"#;
                    let _ = std::fs::write(path, fixed_run_sh);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ =
                            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
                        let _ = std::fs::set_permissions(
                            &adapter_py,
                            std::fs::Permissions::from_mode(0o700),
                        );
                    }
                }
            }
        }
    }
}

fn build_command(manifest: &PersonalManifest, sub_provider_id: Option<&str>) -> Command {
    let entry_path = std::path::Path::new(&manifest.entrypoint);
    let resolved_entrypoint = if entry_path.is_file() {
        heal_legacy_adapter_script(entry_path);
        entry_path
            .canonicalize()
            .unwrap_or_else(|_| entry_path.to_path_buf())
    } else {
        entry_path.to_path_buf()
    };
    let mut cmd = Command::new(&resolved_entrypoint);
    if let Some(parent) = resolved_entrypoint.parent() {
        if parent.is_dir() {
            cmd.current_dir(parent);
        }
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Isolate secrets by only inheriting safe runtime environment variables
    cmd.env_clear();
    const ALLOWED_ENV_VARS: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL", "TMPDIR", "TZ", "USER"];
    for key in ALLOWED_ENV_VARS {
        if let Ok(val) = std::env::var(key) {
            cmd.env(key, val);
        }
    }
    cmd.env("MODULE_ID", &manifest.id);
    cmd.env("MODULE_NAME", &manifest.name);
    cmd.env("MODULE_NAMESPACE", &manifest.namespace);
    if let Some(sub_id) = sub_provider_id {
        cmd.env("MODULE_SUB_PROVIDER_ID", sub_id);
    }
    cmd
}

fn spawn_stderr_reader(
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

fn wait_for_module_response(
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

fn format_crash_message(rx_err: std::sync::mpsc::Receiver<String>) -> String {
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

fn dispatch_stdio(
    client: &mut TcpStream,
    request: &Request,
    manifest: &PersonalManifest,
    subpath: &str,
    sub_provider_id: Option<&str>,
) -> io::Result<u16> {
    let mut cmd = build_command(manifest, sub_provider_id);

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

fn dispatch_local_http(
    client: &mut TcpStream,
    request: &Request,
    manifest: &PersonalManifest,
    subpath: &str,
) -> io::Result<u16> {
    let base_url = manifest.entrypoint.trim_end_matches('/');
    let clean_sub = subpath.trim_start_matches('/');
    let target_url = match request.path.split_once('?') {
        Some((_, q)) if !q.is_empty() && !clean_sub.contains('?') => {
            format!("{base_url}/{clean_sub}?{q}")
        }
        _ => format!("{base_url}/{clean_sub}"),
    };

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
