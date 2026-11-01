//! Diagnostic test and health probe handlers for gateway modules.

use crate::config::ConfigStore;
use crate::gateway::modules::{ModuleDescriptor, ModuleKind, PersonalManifest};
use crate::http::write_error;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(serde::Deserialize)]
pub struct TestModulePayload {
    pub id: String,
}

#[derive(serde::Serialize, Debug, Clone, PartialEq, Eq)]
pub struct TestModuleResult {
    pub ok: bool,
    pub id: String,
    pub name: String,
    pub status: String,
    pub protocol: String,
    pub latency_ms: u64,
    pub message: String,
    pub details: serde_json::Value,
}

fn resolve_http_target(raw: &str) -> Result<(Vec<std::net::SocketAddr>, String, String), String> {
    let without_proto = raw.trim_start_matches("http://");
    let (host_port, path) = match without_proto.split_once('/') {
        Some((hp, p)) => (hp, format!("/{p}")),
        None => (without_proto, "/".to_string()),
    };
    let resolved = if !host_port.contains(':') {
        format!("{host_port}:80")
    } else {
        host_port.to_string()
    };
    let addrs: Vec<std::net::SocketAddr> = resolved
        .to_socket_addrs()
        .map_err(|e| format!("Failed to resolve '{resolved}': {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("Could not resolve socket address for '{resolved}'"));
    }
    Ok((addrs, host_port.to_string(), path))
}

fn probe_local_http(manifest: &PersonalManifest) -> (bool, u64, String, serde_json::Value) {
    let start = Instant::now();
    let raw_ep = &manifest.entrypoint;
    let trimmed = raw_ep.trim();
    if trimmed.starts_with("https://") {
        return (
            false,
            0,
            "HTTPS probing is not supported for local-http protocol; use http://".to_string(),
            serde_json::json!({ "endpoint": raw_ep }),
        );
    }
    let url_str = if trimmed.starts_with("http://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };

    let (addrs, host_header, path) = match resolve_http_target(&url_str) {
        Ok(t) => t,
        Err(e) => {
            return (
                false,
                0,
                format!("Invalid endpoint address: {e}"),
                serde_json::json!({ "endpoint": raw_ep }),
            );
        }
    };

    let mut connected = None;
    let mut last_err = String::new();
    for addr in &addrs {
        match TcpStream::connect_timeout(addr, Duration::from_secs(2)) {
            Ok(s) => {
                connected = Some((*addr, s));
                break;
            }
            Err(e) => last_err = e.to_string(),
        }
    }

    match connected {
        Some((connected_addr, mut stream)) => {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
            let probe_req =
                format!("GET {path} HTTP/1.1\r\nHost: {host_header}\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(probe_req.as_bytes());
            let mut resp_buf = [0u8; 128];
            let n = stream.read(&mut resp_buf).unwrap_or(0);
            let latency_ms = start.elapsed().as_millis() as u64;
            let first_line = String::from_utf8_lossy(&resp_buf[..n])
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            let status_code = first_line
                .split_whitespace()
                .nth(1)
                .and_then(|code| code.parse::<u16>().ok());
            let ok = status_code.is_some_and(|code| (200..400).contains(&code));
            let msg = if !first_line.is_empty() {
                if ok {
                    format!("Local HTTP server reachable ({first_line})")
                } else {
                    format!("Local HTTP server returned error status ({first_line})")
                }
            } else {
                "Local HTTP server closed connection without HTTP response".to_string()
            };
            (
                ok,
                latency_ms,
                msg,
                serde_json::json!({
                    "endpoint": raw_ep,
                    "socket_addr": connected_addr.to_string(),
                    "response_preview": first_line,
                    "status_code": status_code,
                }),
            )
        }
        None => {
            let latency_ms = start.elapsed().as_millis() as u64;
            (
                false,
                latency_ms,
                format!("Failed to connect to local HTTP endpoint: {last_err}"),
                serde_json::json!({
                    "endpoint": raw_ep,
                    "attempted_addrs": addrs.iter().map(ToString::to_string).collect::<Vec<_>>(),
                    "error": last_err,
                }),
            )
        }
    }
}

fn probe_stdio_module(manifest: &PersonalManifest) -> (bool, u64, String, serde_json::Value) {
    let ep_path = Path::new(&manifest.entrypoint);
    if !ep_path.exists() {
        return (
            false,
            0,
            format!("Entrypoint file does not exist: {}", manifest.entrypoint),
            serde_json::json!({
                "entrypoint": manifest.entrypoint,
                "exists": false,
            }),
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(ep_path) {
            if meta.permissions().mode() & 0o111 == 0 {
                return (
                    false,
                    0,
                    format!(
                        "Entrypoint file lacks executable permissions: {}",
                        manifest.entrypoint
                    ),
                    serde_json::json!({
                        "entrypoint": manifest.entrypoint,
                        "executable": false,
                    }),
                );
            }
        }
    }

    let start = Instant::now();
    let mut cmd = crate::modules::personal::build_command(manifest, None);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return (
                false,
                0,
                format!("Failed to spawn module process: {e}"),
                serde_json::json!({
                    "entrypoint": manifest.entrypoint,
                    "error": e.to_string(),
                }),
            );
        }
    };

    let probe_payload = serde_json::json!({
        "method": "GET",
        "path": "/health",
        "headers": {},
        "body": "",
        "probe": true
    });
    let mut payload_bytes = serde_json::to_vec(&probe_payload).unwrap_or_default();
    payload_bytes.push(b'\n');

    if let Some(mut stdin) = child.stdin.take() {
        let _ = std::thread::spawn(move || {
            let _ = stdin.write_all(&payload_bytes);
            drop(stdin);
        });
    }

    let stdout = child.stdout.take();
    let (tx, rx) = std::sync::mpsc::channel();
    if let Some(out) = stdout {
        std::thread::spawn(move || {
            let mut reader = BufReader::new(out);
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let _ = tx.send(line);
        });
    }

    let (rx_err, err_handle) = crate::modules::personal::spawn_stderr_reader(child.stderr.take());

    let probe_timeout = Duration::from_secs(5);
    let result = match rx.recv_timeout(probe_timeout) {
        Ok(line) if !line.trim().is_empty() => {
            let _ = child.kill();
            let _ = child.wait();
            let latency_ms = start.elapsed().as_millis() as u64;
            let trimmed = line.trim();
            let parsed_json: Option<serde_json::Value> = serde_json::from_str(trimmed).ok();
            let has_error = parsed_json.as_ref().map_or(true, |val| {
                let status_err = val
                    .get("status")
                    .and_then(|s| s.as_f64())
                    .is_some_and(|code| code < 200.0 || code >= 400.0);
                let ok_err = val.get("ok").and_then(|o| o.as_bool()).is_some_and(|o| !o);
                let healthy_err = val
                    .get("healthy")
                    .and_then(|h| h.as_bool())
                    .is_some_and(|h| !h);
                let explicit_err = val.get("error").is_some();
                status_err || ok_err || healthy_err || explicit_err
            });
            let ok = !has_error;
            let msg = if ok {
                format!("Module responded successfully in {latency_ms}ms")
            } else {
                format!("Module returned unhealthy status: {trimmed}")
            };
            (
                ok,
                latency_ms,
                msg,
                serde_json::json!({
                    "entrypoint": manifest.entrypoint,
                    "response_preview": trimmed.chars().take(80).collect::<String>(),
                }),
            )
        }
        probe_res => {
            let _ = child.kill();
            let exit_status = child.wait().ok();
            let latency_ms = start.elapsed().as_millis() as u64;
            let err_msg = rx_err
                .recv_timeout(Duration::from_millis(150))
                .unwrap_or_default();
            let is_timeout = matches!(probe_res, Err(std::sync::mpsc::RecvTimeoutError::Timeout));
            let msg = if !err_msg.trim().is_empty() {
                format!("Module probe failed: {}", err_msg.trim())
            } else if is_timeout {
                format!("Module execution timed out ({probe_timeout:?})")
            } else if let Some(code) = exit_status.and_then(|s| s.code()) {
                format!("Module process exited prematurely with status code {code}")
            } else {
                "Module process closed stdout without responding".to_string()
            };
            (
                false,
                latency_ms,
                msg,
                serde_json::json!({
                    "entrypoint": manifest.entrypoint,
                    "timeout": is_timeout,
                    "stderr": err_msg.trim(),
                }),
            )
        }
    };
    // Do not join: a grandchild process can keep the stderr pipe open after `kill()`.
    drop(err_handle);
    result
}

fn probe_builtin_module(
    desc: &ModuleDescriptor,
    store: &ConfigStore,
) -> (bool, u64, String, serde_json::Value) {
    let (msg, extra) = match desc.id.as_str() {
        "omniroute" => (
            "Built-in Omniroute gateway router is active and healthy".to_string(),
            serde_json::json!({}),
        ),
        "museai" => {
            let conf = store.snapshot();
            let has_token = !conf.museai_access_token.is_empty() || !conf.museai_cookie.is_empty();
            let text = if has_token {
                "Built-in MuseAI provider module is configured and ready"
            } else {
                "Built-in MuseAI provider module is enabled (credentials not yet configured)"
            };
            (
                text.to_string(),
                serde_json::json!({ "configured": has_token }),
            )
        }
        "jev" => {
            let conf = store.snapshot();
            let has_keys = !conf.jev_api_keys.is_empty();
            let text = if has_keys {
                format!(
                    "Built-in Jev gateway module is configured with {} API key(s)",
                    conf.jev_api_keys.len()
                )
            } else {
                "Built-in Jev gateway module is enabled (no API keys configured)".to_string()
            };
            (text, serde_json::json!({ "configured": has_keys }))
        }
        _ => (
            format!("Built-in module '{}' is registered and healthy", desc.id),
            serde_json::json!({}),
        ),
    };
    let mut details = serde_json::json!({ "kind": "builtin", "namespace": desc.namespace });
    if let (Some(m), Some(e)) = (details.as_object_mut(), extra.as_object()) {
        m.extend(e.clone());
    }
    (true, 0, msg, details)
}

pub fn handle_test_personal_module(
    client: &mut TcpStream,
    body: &[u8],
    store: &ConfigStore,
) -> io::Result<()> {
    let payload: TestModulePayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(e) => return write_error(client, 400, "Bad Request", &format!("Invalid JSON: {e}")),
    };

    let clean_id = payload.id.trim();
    if clean_id.is_empty() {
        return write_error(client, 400, "Bad Request", "Module id must not be empty");
    }

    let reg = store.module_registry();
    let read_guard = match reg.read() {
        Ok(g) => g,
        Err(e) => return write_error(client, 500, "Internal Error", &e.to_string()),
    };

    let desc = match read_guard.get_module(clean_id) {
        Some(d) => d.clone(),
        None => {
            drop(read_guard);
            return write_error(
                client,
                404,
                "Not Found",
                &format!("Module '{clean_id}' not found in registry"),
            );
        }
    };

    let personal_manifest = read_guard.get_personal_manifest(clean_id).cloned();
    drop(read_guard);

    let (ok, latency_ms, message, details) = if let Some(manifest) = personal_manifest.as_ref() {
        if manifest.protocol == "local-http" {
            probe_local_http(manifest)
        } else {
            probe_stdio_module(manifest)
        }
    } else {
        probe_builtin_module(&desc, store)
    };

    let status = if ok { "healthy" } else { "unhealthy" };
    let resp = TestModuleResult {
        ok,
        id: desc.id.clone(),
        name: desc.name.clone(),
        status: status.to_string(),
        protocol: if desc.kind == ModuleKind::BuiltIn {
            "builtin".to_string()
        } else {
            personal_manifest
                .as_ref()
                .map(|m| m.protocol.clone())
                .unwrap_or_else(|| "stdio-stream".to_string())
        },
        latency_ms,
        message,
        details,
    };

    let resp_bytes = serde_json::to_vec(&resp).unwrap_or_default();
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        resp_bytes.len()
    )?;
    client.write_all(&resp_bytes)?;
    client.flush()
}
