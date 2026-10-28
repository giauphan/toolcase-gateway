use crate::config::Config;
use crate::providers::museai::protocol::{Header, ServiceFrameKind, SERVICE_DAEMON};
use crate::providers::museai::threads::registry::unregister_thread;
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

fn debug_log(msg: &str) {
    if std::env::var("MUSEAI_DEBUG").is_ok() {
        crate::runtime::logger::log_message("DEBUG", "museai_threads", msg);
    }
}

pub fn extract_session_timestamp(s: &serde_json::Value) -> u64 {
    let val = s
        .get("created_at")
        .or_else(|| s.get("created_at_ms"))
        .or_else(|| s.get("updated_at"))
        .or_else(|| s.get("updated_at_ms"));

    let Some(v) = val else { return 0 };

    let num = if let Some(n) = v.as_u64() {
        n
    } else if let Some(n) = v.as_i64() {
        if n <= 0 {
            0
        } else {
            n as u64
        }
    } else if let Some(n) = v.as_f64() {
        if n <= 0.0 {
            0
        } else {
            n as u64
        }
    } else if let Some(str_val) = v.as_str() {
        str_val.parse::<u64>().unwrap_or(0)
    } else {
        0
    };

    if num == 0 {
        return 0;
    }
    if num > 100_000_000_000 {
        num / 1000
    } else {
        num
    }
}

pub fn delete_cvm_session_over_noise(thread_id: &str, config: &Config) -> io::Result<()> {
    if thread_id.is_empty() {
        return Ok(());
    }
    if config.museai_cookie.is_empty() && config.museai_notary_token.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "No Muse credentials available for Noise session",
        ));
    }

    let bootstrapped = match crate::providers::museai::session::bootstrap_museai_config(config) {
        Ok(c) => c,
        Err(e) => return Err(io::Error::other(format!("Bootstrap failed: {e}"))),
    };

    let base_url = if bootstrapped.museai_base_url.is_empty() {
        "https://muse.ai"
    } else {
        bootstrapped.museai_base_url.as_str()
    };
    let base_origin = base_url.trim_end_matches('/');

    let req_id = uuid::Uuid::new_v4().to_string();
    let ws_url = crate::providers::museai::session::build_museai_ws_url(&bootstrapped, &req_id)?;

    let mut socket =
        crate::providers::museai::transport::MuseWebSocket::connect(&ws_url, base_origin, "")?;
    let mut session = crate::providers::museai::noise::MuseNoiseSession::new_initiator(
        crate::providers::museai::noise::NOISE_PATTERN_XX,
        None,
    )
    .map_err(|e| io::Error::other(format!("Noise initiator error: {e}")))?;

    session
        .perform_client_handshake(&mut socket, &bootstrapped.museai_notary_token)
        .map_err(|e| io::Error::other(format!("Noise handshake error: {e}")))?;

    let stream_id = 1;
    let headers = cvm_json_headers();

    let payload = serde_json::json!({
        "method": "/api/session/delete",
        "session_id": thread_id,
        "id": thread_id,
    });
    let payload_bytes =
        serde_json::to_vec(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    socket.send_encrypted_service_request(
        &mut session,
        SERVICE_DAEMON,
        stream_id,
        "POST",
        "/api/session/delete",
        &headers,
        &payload_bytes,
    )?;

    let mut del_status = 0;
    loop {
        let frame = socket.read_encrypted_service_frame(&mut session)?;
        if frame.stream_id != stream_id {
            continue;
        }
        match frame.kind {
            ServiceFrameKind::Response {
                status, end_body, ..
            } => {
                del_status = status;
                if end_body {
                    break;
                }
            }
            ServiceFrameKind::BodyChunk { end_body, .. } => {
                if end_body {
                    break;
                }
            }
            ServiceFrameKind::Reset { code, reason } => {
                return Err(io::Error::other(format!(
                    "CVM session deletion reset with code {code}: {reason}"
                )));
            }
        }
    }

    if (200..300).contains(&del_status) {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "CVM session deletion returned HTTP {del_status}"
        )))
    }
}

pub fn delete_muse_thread(base_url: &str, thread_id: &str, config: &Config) -> io::Result<()> {
    if thread_id.is_empty() {
        return Ok(());
    }

    // 1. Authoritative CVM session deletion over encrypted Noise WebSocket
    match delete_cvm_session_over_noise(thread_id, config) {
        Ok(_) => {
            crate::runtime::logger::log_message(
                "INFO",
                "museai_threads",
                &format!("Successfully purged upstream CVM thread {thread_id} via Noise session"),
            );
            debug_log("Successfully cleaned up thread via CVM Noise");
            return Ok(());
        }
        Err(e) => {
            crate::runtime::logger::log_message(
                "WARN",
                "museai_threads",
                &format!("Upstream CVM Noise session deletion for {thread_id} failed: {e}; falling back to HTTP"),
            );
            debug_log(&format!("Noise session deletion fallback: {e}"));
        }
    }

    // 2. HTTP fallback
    let delete_thread_url = format!("{base_url}/api/thread/{thread_id}");
    let mut req = ureq::delete(&delete_thread_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json");

    req = crate::providers::museai::transport::apply_muse_http_headers(req, base_url);

    if !config.museai_cookie.is_empty() {
        req = req.header("Cookie", &config.museai_cookie);
    }

    match req.call() {
        Ok(_) => {
            crate::runtime::logger::log_message(
                "INFO",
                "museai_threads",
                &format!("Successfully purged upstream thread {thread_id}"),
            );
            debug_log("Successfully cleaned up thread");
            Ok(())
        }
        Err(ureq::Error::StatusCode(status)) => {
            crate::runtime::logger::log_message(
                "WARN",
                "museai_threads",
                &format!("Upstream thread deletion for {thread_id} returned HTTP {status}"),
            );
            debug_log(&format!(
                "Note: Upstream thread deletion returned HTTP {status}"
            ));
            Ok(())
        }
        Err(e) => {
            crate::runtime::logger::log_message(
                "WARN",
                "museai_threads",
                &format!("Upstream thread deletion for {thread_id} failed: {e}"),
            );
            debug_log(&format!("Note: Upstream thread deletion failed: {e}"));
            Ok(())
        }
    }
}

fn init_remote_cvm_session(
    config: &Config,
) -> io::Result<
    Option<(
        crate::providers::museai::transport::MuseWebSocket,
        crate::providers::museai::noise::MuseNoiseSession,
    )>,
> {
    let bootstrapped = match crate::providers::museai::session::bootstrap_museai_config(config) {
        Ok(c) => c,
        Err(e) => {
            debug_log(&format!("Remote CVM session sync skipped: {e}"));
            return Ok(None);
        }
    };

    let base_url = if bootstrapped.museai_base_url.is_empty() {
        "https://muse.ai"
    } else {
        bootstrapped.museai_base_url.as_str()
    };
    let base_origin = base_url.trim_end_matches('/');

    let req_id = uuid::Uuid::new_v4().to_string();
    let ws_url =
        match crate::providers::museai::session::build_museai_ws_url(&bootstrapped, &req_id) {
            Ok(u) => u,
            Err(e) => {
                debug_log(&format!("Failed to build CVM ws url: {e}"));
                return Ok(None);
            }
        };

    let mut socket =
        match crate::providers::museai::transport::MuseWebSocket::connect(&ws_url, base_origin, "")
        {
            Ok(s) => s,
            Err(e) => {
                debug_log(&format!("Failed to connect to CVM WebSocket: {e}"));
                return Ok(None);
            }
        };

    let mut session = match crate::providers::museai::noise::MuseNoiseSession::new_initiator(
        crate::providers::museai::noise::NOISE_PATTERN_XX,
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            debug_log(&format!("Noise initiator error: {e}"));
            return Ok(None);
        }
    };

    if let Err(e) = session.perform_client_handshake(&mut socket, &bootstrapped.museai_notary_token)
    {
        debug_log(&format!("Noise handshake error: {e}"));
        return Ok(None);
    }

    Ok(Some((socket, session)))
}

fn fetch_remote_session_list(
    socket: &mut crate::providers::museai::transport::MuseWebSocket,
    session: &mut crate::providers::museai::noise::MuseNoiseSession,
    stream_id: u64,
) -> Option<Vec<serde_json::Value>> {
    let headers = vec![
        Header {
            key: "Host".to_string(),
            value: "hatch.metaaivm.com".to_string(),
        },
        Header {
            key: "Accept".to_string(),
            value: "application/json".to_string(),
        },
        Header {
            key: "x-request-id".to_string(),
            value: uuid::Uuid::new_v4().to_string(),
        },
        Header {
            key: "x-app-id".to_string(),
            value: "hatch-web".to_string(),
        },
    ];

    if socket
        .send_encrypted_service_request(
            session,
            SERVICE_DAEMON,
            stream_id,
            "GET",
            "/api/session/list",
            &headers,
            &[],
        )
        .is_err()
    {
        return None;
    }

    let mut body_bytes = Vec::new();
    while let Ok(frame) = socket.read_encrypted_service_frame(session) {
        if frame.stream_id != stream_id {
            continue;
        }
        match frame.kind {
            ServiceFrameKind::Response { body, end_body, .. } => {
                body_bytes.extend_from_slice(&body);
                if end_body {
                    break;
                }
            }
            ServiceFrameKind::BodyChunk { data, end_body } => {
                body_bytes.extend_from_slice(&data);
                if end_body {
                    break;
                }
            }
            ServiceFrameKind::Reset { code, reason } => {
                debug_log(&format!("Session list reset with code {code}: {reason}"));
                break;
            }
        }
    }

    let session_list: serde_json::Value = serde_json::from_slice(&body_bytes).ok()?;
    session_list
        .get("result")
        .and_then(|r| r.get("sessions"))
        .and_then(|s| s.as_array())
        .or_else(|| session_list.get("sessions").and_then(|s| s.as_array()))
        .or_else(|| session_list.as_array())
        .cloned()
}

fn cvm_json_headers() -> Vec<Header> {
    vec![
        Header {
            key: "Host".to_string(),
            value: "hatch.metaaivm.com".to_string(),
        },
        Header {
            key: "Content-Type".to_string(),
            value: "application/json".to_string(),
        },
        Header {
            key: "Accept".to_string(),
            value: "application/json".to_string(),
        },
        Header {
            key: "x-request-id".to_string(),
            value: uuid::Uuid::new_v4().to_string(),
        },
        Header {
            key: "x-app-id".to_string(),
            value: "hatch-web".to_string(),
        },
    ]
}

fn mutate_cvm_session_remote(
    socket: &mut crate::providers::museai::transport::MuseWebSocket,
    session: &mut crate::providers::museai::noise::MuseNoiseSession,
    action: &str,
    sid: &str,
    stream_id: &mut u64,
) -> bool {
    *stream_id = stream_id.wrapping_add(2);
    let headers = cvm_json_headers();
    let method = format!("/api/session/{action}");
    let payload = serde_json::json!({
        "method": method,
        "session_id": sid,
        "id": sid,
    });
    let Ok(bytes) = serde_json::to_vec(&payload) else {
        return false;
    };
    if socket
        .send_encrypted_service_request(
            session,
            SERVICE_DAEMON,
            *stream_id,
            "POST",
            &method,
            &headers,
            &bytes,
        )
        .is_err()
    {
        return false;
    }

    let mut success = false;
    while let Ok(frame) = socket.read_encrypted_service_frame(session) {
        if frame.stream_id != *stream_id {
            continue;
        }
        match frame.kind {
            ServiceFrameKind::Response {
                status, end_body, ..
            } => {
                if (200..300).contains(&status) {
                    success = true;
                }
                if end_body {
                    break;
                }
            }
            ServiceFrameKind::BodyChunk { end_body, .. } => {
                if end_body {
                    break;
                }
            }
            ServiceFrameKind::Reset { .. } => {
                break;
            }
        }
    }
    success
}

fn delete_cvm_session_remote(
    socket: &mut crate::providers::museai::transport::MuseWebSocket,
    session: &mut crate::providers::museai::noise::MuseNoiseSession,
    sid: &str,
    stream_id: &mut u64,
) -> bool {
    mutate_cvm_session_remote(socket, session, "delete", sid, stream_id)
}

fn archive_cvm_session_remote(
    socket: &mut crate::providers::museai::transport::MuseWebSocket,
    session: &mut crate::providers::museai::noise::MuseNoiseSession,
    sid: &str,
    stream_id: &mut u64,
) -> bool {
    mutate_cvm_session_remote(socket, session, "archive", sid, stream_id)
}

pub fn purge_remote_cvm_threads_older_than(ttl_secs: u64, config: &Config) -> io::Result<usize> {
    if config.museai_cookie.is_empty() && config.museai_notary_token.is_empty() {
        return Ok(0);
    }

    let Some((mut socket, mut session)) = init_remote_cvm_session(config)? else {
        return Ok(0);
    };

    let mut stream_id: u64 = 1;
    let Some(sessions) = fetch_remote_session_list(&mut socket, &mut session, stream_id) else {
        crate::runtime::logger::log_message(
            "INFO",
            "museai_threads",
            "No sessions array found in CVM session list response",
        );
        return Ok(0);
    };

    crate::runtime::logger::log_message(
        "INFO",
        "museai_threads",
        &format!(
            "Found {} remote CVM sessions in session list",
            sessions.len()
        ),
    );

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut purged = 0;
    for s in sessions {
        let sid = match s.get("session_id").and_then(|v| v.as_str()) {
            Some(id) if !id.is_empty() => id,
            _ => continue,
        };

        let is_primary = s.get("is_primary").and_then(|v| v.as_bool()) == Some(true);
        let timestamp_sec = extract_session_timestamp(&s);
        let age_secs = if timestamp_sec > 0 && now >= timestamp_sec {
            now - timestamp_sec
        } else {
            0
        };

        crate::runtime::logger::log_message(
            "INFO",
            "museai_threads",
            &format!(
                "Session {sid}: is_primary={is_primary}, ts={timestamp_sec}, age={age_secs}s, ttl={ttl_secs}s"
            ),
        );

        if is_primary {
            continue;
        }

        if timestamp_sec > 0 && now >= timestamp_sec.saturating_add(ttl_secs) {
            if delete_cvm_session_remote(&mut socket, &mut session, sid, &mut stream_id) {
                unregister_thread(sid);
                purged += 1;
                crate::runtime::logger::log_message(
                    "INFO",
                    "museai_threads",
                    &format!("Purged remote CVM thread session {sid} older than {ttl_secs}s"),
                );
            } else if archive_cvm_session_remote(&mut socket, &mut session, sid, &mut stream_id) {
                unregister_thread(sid);
                purged += 1;
                crate::runtime::logger::log_message(
                    "INFO",
                    "museai_threads",
                    &format!("Archived remote CVM thread session {sid} older than {ttl_secs}s"),
                );
            }
        }
    }

    Ok(purged)
}
