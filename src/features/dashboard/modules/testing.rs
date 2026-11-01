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

fn parse_http_addr(raw: &str) -> Result<std::net::SocketAddr, String> {
    let without_proto = raw
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let host_port = without_proto.split('/').next().unwrap_or(without_proto);
    let resolved = if !host_port.contains(':') {
        format!("{host_port}:80")
    } else {
        host_port.to_string()
    };
    let addrs: Vec<std::net::SocketAddr> = resolved
        .to_socket_addrs()
        .map_err(|e| format!("Failed to resolve '{resolved}': {e}"))?
        .collect();
    addrs
        .into_iter()
        .next()
        .ok_or_else(|| format!("Could not resolve socket address for '{resolved}'"))
}

fn probe_local_http(manifest: &PersonalManifest) -> (bool, u64, String, serde_json::Value) {
    let start = Instant::now();
    let raw_ep = &manifest.entrypoint;
    let trimmed = raw_ep.trim();
    let url_str = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };

    let parsed = match parse_http_addr(&url_str) {
        Ok(addr) => addr,
        Err(e) => {
            return (
                false,
                0,
                format!("Invalid endpoint address: {e}"),
                serde_json::json!({ "endpoint": raw_ep }),
            );
        }
    };

    match TcpStream::connect_timeout(&parsed, Duration::from_secs(5)) {
        Ok(mut stream) => {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
            let probe_req =
                format!("HEAD / HTTP/1.1\r\nHost: {parsed}\r\nConnection: close\r\n\r\n");
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
            let ok = status_code
                .map(|code| (200..400).contains(&code))
                .unwrap_or(false);
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
                    "socket_addr": parsed.to_string(),
                    "response_preview": first_line,
                    "status_code": status_code,
                }),
            )
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            (
                false,
                latency_ms,
                format!("Failed to connect to local HTTP endpoint: {e}"),
                serde_json::json!({
                    "endpoint": raw_ep,
                    "socket_addr": parsed.to_string(),
                    "error": e.to_string(),
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
            (
                true,
                latency_ms,
                format!("Module responded successfully in {latency_ms}ms"),
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
    match desc.id.as_str() {
        "omniroute" => (
            true,
            0,
            "Built-in Omniroute gateway router is active and healthy".to_string(),
            serde_json::json!({
                "kind": "builtin",
                "namespace": desc.namespace,
            }),
        ),
        "museai" => {
            let conf = store.snapshot();
            let has_token = !conf.museai_access_token.is_empty() || !conf.museai_cookie.is_empty();
            let msg = if has_token {
                "Built-in MuseAI provider module is configured and ready".to_string()
            } else {
                "Built-in MuseAI provider module is enabled (credentials not yet configured)"
                    .to_string()
            };
            (
                true,
                0,
                msg,
                serde_json::json!({
                    "kind": "builtin",
                    "namespace": desc.namespace,
                    "configured": has_token,
                }),
            )
        }
        "jev" => {
            let conf = store.snapshot();
            let has_keys = !conf.jev_api_keys.is_empty();
            let msg = if has_keys {
                format!(
                    "Built-in Jev gateway module is configured with {} API key(s)",
                    conf.jev_api_keys.len()
                )
            } else {
                "Built-in Jev gateway module is enabled (no API keys configured)".to_string()
            };
            (
                true,
                0,
                msg,
                serde_json::json!({
                    "kind": "builtin",
                    "namespace": desc.namespace,
                    "configured": has_keys,
                }),
            )
        }
        _ => (
            true,
            0,
            format!("Built-in module '{}' is registered and healthy", desc.id),
            serde_json::json!({
                "kind": "builtin",
                "namespace": desc.namespace,
            }),
        ),
    }
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
