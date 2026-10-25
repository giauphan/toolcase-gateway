use crate::config::Config;
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn debug_log(msg: &str) {
    if std::env::var("MUSEAI_DEBUG").is_ok() {
        crate::runtime::logger::log_message("DEBUG", "museai_threads", msg);
    }
}

pub struct TrackedThread {
    pub id: String,
    pub created_at: u64,
    pub base_url: String,
    pub config: Config,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersistedThreadRecord {
    pub id: String,
    pub created_at: u64,
    pub base_url: String,
    #[serde(default)]
    pub retention_secs: u64,
}

pub fn thread_state_dir() -> PathBuf {
    PathBuf::from(".gw_state").join("threads")
}

fn sanitize_thread_filename(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn save_thread_to_disk(thread: &TrackedThread) {
    let dir = thread_state_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        debug_log(&format!("Failed to create thread state directory: {e}"));
        return;
    }
    let filename = format!("thread-{}.json", sanitize_thread_filename(&thread.id));
    let path = dir.join(filename);
    let record = PersistedThreadRecord {
        id: thread.id.clone(),
        created_at: thread.created_at,
        base_url: thread.base_url.clone(),
        retention_secs: thread.config.museai_thread_retention_secs,
    };
    if let Ok(bytes) = serde_json::to_vec(&record) {
        let _ = std::fs::write(path, bytes);
    }
}

pub fn remove_thread_from_disk(id: &str) {
    let dir = thread_state_dir();
    let filename = format!("thread-{}.json", sanitize_thread_filename(id));
    let path = dir.join(filename);
    let _ = std::fs::remove_file(path);
}

pub fn sync_threads_from_disk() {
    let dir = thread_state_dir();
    if !dir.exists() {
        return;
    }
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut loaded = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(record) = serde_json::from_slice::<PersistedThreadRecord>(&bytes) {
                let config = Config {
                    museai_thread_retention_secs: record.retention_secs,
                    ..Default::default()
                };
                loaded.push(TrackedThread {
                    id: record.id,
                    created_at: record.created_at,
                    base_url: record.base_url,
                    config,
                });
            }
        }
    }
    if let Ok(mut threads) = tracked_threads().lock() {
        for item in loaded {
            if !threads.iter().any(|t| t.id == item.id) {
                threads.push(item);
            }
        }
    }
}

pub fn unregister_thread(thread_id: &str) {
    remove_thread_from_disk(thread_id);
    if let Ok(mut threads) = tracked_threads().lock() {
        threads.retain(|t| t.id != thread_id);
    }
}

pub fn tracked_threads() -> &'static Mutex<Vec<TrackedThread>> {
    static TRACKED_THREADS: OnceLock<Mutex<Vec<TrackedThread>>> = OnceLock::new();
    TRACKED_THREADS.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn register_thread(thread_id: String, base_url: String, config: Config) {
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let thread = TrackedThread {
        id: thread_id,
        created_at,
        base_url,
        config,
    };
    save_thread_to_disk(&thread);
    if let Ok(mut threads) = tracked_threads().lock() {
        threads.push(thread);
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
        crate::providers::museai::protocol::Header {
            key: "Host".to_string(),
            value: "hatch.metaaivm.com".to_string(),
        },
        crate::providers::museai::protocol::Header {
            key: "Content-Type".to_string(),
            value: "application/json".to_string(),
        },
        crate::providers::museai::protocol::Header {
            key: "Accept".to_string(),
            value: "application/json".to_string(),
        },
        crate::providers::museai::protocol::Header {
            key: "x-request-id".to_string(),
            value: uuid::Uuid::new_v4().to_string(),
        },
        crate::providers::museai::protocol::Header {
            key: "x-app-id".to_string(),
            value: "hatch-web".to_string(),
        },
    ];

    let payload = serde_json::json!({
        "session_id": thread_id,
        "id": thread_id,
    });
    let payload_bytes =
        serde_json::to_vec(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    socket.send_encrypted_service_request(
        &mut session,
        crate::providers::museai::protocol::SERVICE_DAEMON,
        stream_id,
        "POST",
        "/session/delete",
        &headers,
        &payload_bytes,
    )?;

    let frame = socket.read_encrypted_service_frame(&mut session)?;
    match frame.kind {
        crate::providers::museai::protocol::ServiceFrameKind::Response { status, .. } => {
            if (200..300).contains(&status) {
                Ok(())
            } else {
                Err(io::Error::other(format!(
                    "CVM session deletion returned HTTP {status}"
                )))
            }
        }
        _ => Ok(()),
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

pub fn cleanup_tracked_threads_once() -> usize {
    let global_ttl = std::env::var("GW_MUSEAI_THREAD_RETENTION_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(7200);

    cleanup_tracked_threads_with_ttl(global_ttl)
}

pub fn cleanup_tracked_threads_with_ttl(global_ttl: u64) -> usize {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    sync_threads_from_disk();

    let mut to_delete = Vec::new();
    if let Ok(mut threads) = tracked_threads().lock() {
        let mut i = 0;
        while i < threads.len() {
            let thread_ttl = if threads[i].config.museai_thread_retention_secs == 0 {
                0
            } else {
                std::cmp::min(threads[i].config.museai_thread_retention_secs, global_ttl)
            };
            if now >= threads[i].created_at.saturating_add(thread_ttl) {
                to_delete.push(threads.remove(i));
            } else {
                i += 1;
            }
        }
    }

    let deleted_count = to_delete.len();
    for thread in to_delete {
        remove_thread_from_disk(&thread.id);
        let _ = delete_muse_thread(&thread.base_url, &thread.id, &thread.config);
    }
    deleted_count
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
        crate::providers::museai::protocol::Header {
            key: "Host".to_string(),
            value: "hatch.metaaivm.com".to_string(),
        },
        crate::providers::museai::protocol::Header {
            key: "Accept".to_string(),
            value: "application/json".to_string(),
        },
        crate::providers::museai::protocol::Header {
            key: "x-request-id".to_string(),
            value: uuid::Uuid::new_v4().to_string(),
        },
        crate::providers::museai::protocol::Header {
            key: "x-app-id".to_string(),
            value: "hatch-web".to_string(),
        },
    ];

    if socket
        .send_encrypted_service_request(
            &mut session,
            crate::providers::museai::protocol::SERVICE_DAEMON,
            stream_id,
            "GET",
            "/api/session/list?include_peer_sidechats=true",
            &headers,
            &[],
        )
        .is_err()
    {
        return Ok(0);
    }

    let frame = match socket.read_encrypted_service_frame(&mut session) {
        Ok(f) => f,
        Err(_) => return Ok(0),
    };

    let body_bytes = match frame.kind {
        crate::providers::museai::protocol::ServiceFrameKind::Response { body, .. } => body,
        crate::providers::museai::protocol::ServiceFrameKind::BodyChunk { data, .. } => data,
        _ => return Ok(0),
    };

    let session_list: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(_) => return Ok(0),
    };

    let sessions_array = session_list
        .get("sessions")
        .and_then(|v| v.as_array())
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

        let created_at = s
            .get("created_at")
            .or_else(|| s.get("created_at_ms"))
            .and_then(|v| v.as_u64())
            .map(|t| if t > 100_000_000_000 { t / 1000 } else { t })
            .unwrap_or(0);

        if created_at > 0 && now >= created_at.saturating_add(ttl_secs) {
            stream_id += 1;
            let del_headers = vec![
                crate::providers::museai::protocol::Header {
                    key: "Host".to_string(),
                    value: "hatch.metaaivm.com".to_string(),
                },
                crate::providers::museai::protocol::Header {
                    key: "Content-Type".to_string(),
                    value: "application/json".to_string(),
                },
                crate::providers::museai::protocol::Header {
                    key: "Accept".to_string(),
                    value: "application/json".to_string(),
                },
                crate::providers::museai::protocol::Header {
                    key: "x-request-id".to_string(),
                    value: uuid::Uuid::new_v4().to_string(),
                },
                crate::providers::museai::protocol::Header {
                    key: "x-app-id".to_string(),
                    value: "hatch-web".to_string(),
                },
            ];
            let del_payload = serde_json::json!({
                "session_id": sid,
                "id": sid,
            });
            if let Ok(bytes) = serde_json::to_vec(&del_payload) {
                if socket
                    .send_encrypted_service_request(
                        &mut session,
                        crate::providers::museai::protocol::SERVICE_DAEMON,
                        stream_id,
                        "POST",
                        "/session/delete",
                        &del_headers,
                        &bytes,
                    )
                    .is_ok()
                {
                    let _ = socket.read_encrypted_service_frame(&mut session);
                    unregister_thread(sid);
                    purged += 1;
                    crate::runtime::logger::log_message(
                        "INFO",
                        "museai_threads",
                        &format!("Purged remote CVM thread session {sid} older than {ttl_secs}s"),
                    );
                }
            }
        }
    }

    Ok(purged)
}

pub fn cleanup_cycle_once() -> (usize, usize) {
    let ttl_secs = std::env::var("GW_MUSEAI_THREAD_RETENTION_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(7200);

    cleanup_cycle_with_ttl(ttl_secs)
}

pub fn cleanup_cycle_with_ttl(ttl_secs: u64) -> (usize, usize) {
    let base_url =
        std::env::var("GW_MUSEAI_BASE_URL").unwrap_or_else(|_| "https://muse.ai".to_string());
    let cookie = std::env::var("GW_MUSEAI_COOKIE").unwrap_or_default();
    let default_cfg = Config {
        museai_base_url: base_url.clone(),
        museai_cookie: cookie.clone(),
        ..Default::default()
    };

    let remote_purged = purge_remote_cvm_threads_older_than(ttl_secs, &default_cfg).unwrap_or(0);
    let threads_cleaned = cleanup_tracked_threads_with_ttl(ttl_secs);

    let jobs_cleaned =
        super::lifecycle::cleanup_expired_video_jobs_with_callback(ttl_secs, |job| {
            if let Some(ref tid) = job.thread_id {
                let cfg = Config {
                    museai_base_url: base_url.clone(),
                    museai_cookie: cookie.clone(),
                    ..Default::default()
                };
                let _ = delete_muse_thread(&base_url, tid, &cfg);
            }
        })
        .unwrap_or(0);

    let logs_cleaned = crate::runtime::logger::cleanup_expired_logs_default().unwrap_or(0);
    let total_threads_cleaned = threads_cleaned + remote_purged;
    if total_threads_cleaned > 0 || jobs_cleaned > 0 || logs_cleaned > 0 {
        crate::runtime::logger::log_message(
            "INFO",
            "museai:cleanup",
            &format!(
                "cycle purged {total_threads_cleaned} threads ({remote_purged} remote CVM, {threads_cleaned} local tracked), {jobs_cleaned} expired jobs, and {logs_cleaned} expired log files (TTL: {ttl_secs}s)"
            ),
        );
    }

    (total_threads_cleaned, jobs_cleaned)
}

pub fn start_cleanup_worker() {
    start_cleanup_worker_with_interval(Duration::from_secs(300));
}

pub fn start_cleanup_worker_with_interval(interval: Duration) {
    crate::runtime::logger::log_message(
        "INFO",
        "museai:cleanup",
        &format!(
            "background cleanup daemon worker started (interval: {:?})",
            interval
        ),
    );
    thread::spawn(move || {
        // Run immediately on start so threads older than 2 hours are cleared right away
        cleanup_cycle_once();
        loop {
            thread::sleep(interval);
            cleanup_cycle_once();
        }
    });
}

pub fn spawn_muse_thread(base_url: &str, config: &Config) -> io::Result<String> {
    let create_thread_url = format!("{base_url}/thread/new");
    let mut req = ureq::post(&create_thread_url)
        .header("Content-Type", "text/plain;charset=UTF-8")
        .header("Accept", "text/x-component")
        .header("Origin", base_url)
        .header("Referer", &format!("{base_url}/thread/new"))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36")
        .header("sec-ch-ua", r#""Brave";v="153", "Not_A Brand";v="8", "Chromium";v="153""#)
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", r#""Windows""#)
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin")
        .header("next-action", "00eaaa570484d537580e0bf4e19ee28a65ac048d62");

    if !config.museai_cookie.is_empty() {
        req = req.header("Cookie", &config.museai_cookie);
    }

    let response = match req.send("[]") {
        Ok(res) => res,
        Err(ureq::Error::StatusCode(401)) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Muse.ai thread creation returned 401 Unauthorized — credentials are invalid or expired",
            ));
        }
        Err(ureq::Error::StatusCode(403)) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Muse.ai thread creation returned 403 Forbidden — access denied",
            ));
        }
        Err(ureq::Error::StatusCode(404)) => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Muse.ai thread creation returned 404 — endpoint not found or cookie expired",
            ));
        }
        Err(ureq::Error::StatusCode(status)) => {
            return Err(io::Error::other(format!(
                "Muse.ai thread creation returned HTTP {status}"
            )));
        }
        Err(e) => {
            return Err(io::Error::other(format!(
                "Thread creation request failed: {e}"
            )));
        }
    };
    let mut reader = response.into_body();
    let text = reader
        .read_to_string()
        .map_err(|e| io::Error::other(format!("Failed to read thread response: {e}")))?;
    let thread_id = extract_new_thread_id(&text);
    if thread_id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "failed to extract thread ID from /thread/new response",
        ));
    }
    let channel = format!("thread:{thread_id}");
    Ok(channel)
}

pub fn extract_new_thread_id(rsc_text: &str) -> String {
    let needle = "/thread/";
    if let Some(idx) = rsc_text.rfind(needle) {
        let start = idx + needle.len();
        let remaining = &rsc_text[start..];
        let end = remaining
            .find(|c: char| !c.is_alphanumeric() && c != '-')
            .unwrap_or(remaining.len());
        return remaining[..end].to_string();
    }
    String::new()
}
