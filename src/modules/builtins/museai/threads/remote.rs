use crate::config::Config;
use crate::providers::museai::protocol::{Header, ServiceFrameKind, SERVICE_DAEMON};
use std::io;

pub(super) fn debug_log(msg: &str) {
    if std::env::var("MUSEAI_DEBUG").is_ok() {
        crate::runtime::logger::log_message("DEBUG", "museai_threads", msg);
    }
}

pub(super) fn cvm_json_headers() -> Vec<Header> {
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
