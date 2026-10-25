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
    let headers = vec![
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
    ];

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
        .header("Accept", "application/json")
        .header("Origin", base_url)
        .header("Referer", &format!("{base_url}/"))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36")
        .header("sec-ch-ua", r#""Brave";v="153", "Not_A Brand";v="8", "Chromium";v="153""#)
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", r#""Windows""#)
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin");

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

pub fn purge_remote_cvm_threads_older_than(ttl_secs: u64, config: &Config) -> io::Result<usize> {
    if config.museai_cookie.is_empty() && config.museai_notary_token.is_empty() {
        return Ok(0);
    }

    let bootstrapped = match crate::providers::museai::session::bootstrap_museai_config(config) {
        Ok(c) => c,
        Err(e) => {
            debug_log(&format!("Remote CVM session sync skipped: {e}"));
            return Ok(0);
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
                return Ok(0);
            }
        };

    let mut socket =
        match crate::providers::museai::transport::MuseWebSocket::connect(&ws_url, base_origin, "")
        {
            Ok(s) => s,
            Err(e) => {
                debug_log(&format!("Failed to connect to CVM WebSocket: {e}"));
                return Ok(0);
            }
        };

    let mut session = match crate::providers::museai::noise::MuseNoiseSession::new_initiator(
        crate::providers::museai::noise::NOISE_PATTERN_XX,
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            debug_log(&format!("Noise initiator error: {e}"));
            return Ok(0);
        }
    };

    if let Err(e) = session.perform_client_handshake(&mut socket, &bootstrapped.museai_notary_token)
    {
        debug_log(&format!("Noise handshake error: {e}"));
        return Ok(0);
    }

    let mut stream_id = 1;
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
            &mut session,
            SERVICE_DAEMON,
            stream_id,
            "GET",
            "/api/session/list",
            &headers,
            &[],
        )
        .is_err()
    {
        return Ok(0);
    }

    let mut body_bytes = Vec::new();
    while let Ok(frame) = socket.read_encrypted_service_frame(&mut session) {
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

    let session_list: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(e) => {
            debug_log(&format!("Failed to parse session list JSON: {e}"));
            return Ok(0);
        }
    };

    let sessions_array = session_list
        .get("result")
        .and_then(|r| r.get("sessions"))
        .and_then(|s| s.as_array())
        .or_else(|| session_list.get("sessions").and_then(|s| s.as_array()))
        .or_else(|| session_list.as_array());

    let Some(sessions) = sessions_array else {
        return Ok(0);
    };

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

        if s.get("is_primary").and_then(|v| v.as_bool()) == Some(true) {
            continue;
        }

        let timestamp_sec = extract_session_timestamp(s);

        if timestamp_sec > 0 && now >= timestamp_sec.saturating_add(ttl_secs) {
            stream_id = stream_id.wrapping_add(2);
            let del_headers = vec![
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
            ];
            let del_payload = serde_json::json!({
                "method": "/api/session/delete",
                "session_id": sid,
                "id": sid,
            });
            if let Ok(bytes) = serde_json::to_vec(&del_payload) {
                if socket
                    .send_encrypted_service_request(
                        &mut session,
                        SERVICE_DAEMON,
                        stream_id,
                        "POST",
                        "/api/session/delete",
                        &del_headers,
                        &bytes,
                    )
                    .is_ok()
                {
                    let mut del_success = false;
                    while let Ok(frame) = socket.read_encrypted_service_frame(&mut session) {
                        if frame.stream_id != stream_id {
                            continue;
                        }
                        match frame.kind {
                            ServiceFrameKind::Response {
                                status, end_body, ..
                            } => {
                                if (200..300).contains(&status) {
                                    del_success = true;
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

                    if del_success {
                        unregister_thread(sid);
                        purged += 1;
                        crate::runtime::logger::log_message(
                            "INFO",
                            "museai_threads",
                            &format!(
                                "Purged remote CVM thread session {sid} older than {ttl_secs}s"
                            ),
                        );
                    } else {
                        // Fallback: Attempt archive if delete did not acknowledge 200
                        stream_id = stream_id.wrapping_add(2);
                        let arc_payload = serde_json::json!({
                            "method": "/api/session/archive",
                            "session_id": sid,
                            "id": sid,
                        });
                        if let Ok(arc_bytes) = serde_json::to_vec(&arc_payload) {
                            let _ = socket.send_encrypted_service_request(
                                &mut session,
                                SERVICE_DAEMON,
                                stream_id,
                                "POST",
                                "/api/session/archive",
                                &del_headers,
                                &arc_bytes,
                            );
                            while let Ok(frame) = socket.read_encrypted_service_frame(&mut session)
                            {
                                if frame.stream_id != stream_id {
                                    continue;
                                }
                                match frame.kind {
                                    ServiceFrameKind::Response {
                                        status, end_body, ..
                                    } => {
                                        if (200..300).contains(&status) {
                                            unregister_thread(sid);
                                            purged += 1;
                                            crate::runtime::logger::log_message(
                                                "INFO",
                                                "museai_threads",
                                                &format!("Archived remote CVM thread session {sid} older than {ttl_secs}s"),
                                            );
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
                        }
                    }
                }
            }
        }
    }

    Ok(purged)
}
