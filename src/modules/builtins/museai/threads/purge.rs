use super::remote::{cvm_json_headers, debug_log, extract_session_timestamp};
use crate::config::Config;
use crate::providers::museai::protocol::{Header, ServiceFrameKind, SERVICE_DAEMON};
use crate::providers::museai::threads::registry::unregister_thread;
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

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
