use crate::config::Config;
use crate::providers::museai::transport::send_museai_request;
use crate::providers::museai::MuseApprovalRequired;
use std::io::{self, Write};
use std::net::TcpStream;

pub use super::video::{
    handle_create_video, handle_create_video_request, handle_get_video, handle_get_video_content,
    handle_get_video_request, handle_update_video_prompt, is_sync_requested,
};

pub fn write_chat_completion(client: &mut TcpStream, response: &str) -> io::Result<()> {
    crate::http::write_json_ok(client, response)?;
    client.flush()
}

pub fn handle_museai_chat_completions(
    client: &mut TcpStream,
    request_body: &[u8],
    config: &Config,
) -> io::Result<()> {
    match crate::providers::museai::chat::request_museai_chat_completion(
        request_body,
        config,
        false,
    ) {
        Ok(response) => write_chat_completion(client, &response),
        Err(err) => {
            if err
                .get_ref()
                .and_then(|e| e.downcast_ref::<MuseApprovalRequired>())
                .is_some()
            {
                return crate::http::write_error(
                    client,
                    409,
                    "Conflict",
                    &format!("Muse chat completion requires scoped approval: {err}"),
                );
            }
            let (status, reason) = match err.kind() {
                io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => (400, "Bad Request"),
                io::ErrorKind::PermissionDenied => (401, "Unauthorized"),
                io::ErrorKind::TimedOut => (504, "Gateway Timeout"),
                _ => (502, "Bad Gateway"),
            };
            crate::http::write_error(
                client,
                status,
                reason,
                &format!("Muse chat completion failed: {err}"),
            )
        }
    }
}

pub fn handle_museai_v1(
    client: &mut TcpStream,
    request_body: &[u8],
    config: &Config,
) -> io::Result<()> {
    let (url, method, body_json) =
        match crate::providers::museai::business::build_museai_request(request_body, config) {
            Ok(tuple) => tuple,
            Err(e) => {
                return crate::http::write_error(
                    client,
                    400,
                    "Bad Request",
                    &format!("Invalid Muse request: {e}"),
                );
            }
        };

    let result = send_museai_request(&url, &method, &body_json, config);

    match result {
        Ok(upstream_resp) => {
            let status_text = crate::http::http_status_text(upstream_resp.status);
            let out = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                upstream_resp.status,
                status_text,
                upstream_resp.content_type,
                upstream_resp.body.len(),
                upstream_resp.body
            );
            client.write_all(out.as_bytes())?;
            client.flush()
        }
        Err(err) => crate::http::write_error(
            client,
            502,
            "Bad Gateway",
            &format!("Failed to proxy to Muse.ai: {}", err),
        ),
    }
}

pub fn handle_museai_thread_cleanup(
    client: &mut TcpStream,
    thread_id: &str,
    config: &Config,
) -> io::Result<()> {
    let base_url = if config.museai_base_url.is_empty() {
        "https://muse.ai"
    } else {
        config.museai_base_url.as_str()
    };

    crate::providers::museai::threads::unregister_thread(thread_id);
    match crate::providers::museai::threads::delete_muse_thread(base_url, thread_id, config) {
        Ok(()) => {
            let body = serde_json::json!({
                "object": "thread.cleanup",
                "thread_id": thread_id,
                "status": "deleted"
            });
            let response_str = serde_json::to_string(&body)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            crate::http::write_json_ok(client, &response_str)?;
            client.flush()
        }
        Err(e) => crate::http::write_error(
            client,
            502,
            "Bad Gateway",
            &format!("Failed to clean up thread: {e}"),
        ),
    }
}

pub fn handle_museai_threads_cleanup_run(
    client: &mut TcpStream,
    config: &Config,
    request_body: &[u8],
) -> io::Result<()> {
    let requested_ttl = if !request_body.is_empty() {
        serde_json::from_slice::<serde_json::Value>(request_body)
            .ok()
            .and_then(|v| {
                v.get("older_than_secs")
                    .or_else(|| v.get("ttl_secs"))
                    .or_else(|| v.get("retention_secs"))
                    .and_then(|n| n.as_u64())
            })
    } else {
        None
    };

    let ttl_secs = requested_ttl.unwrap_or_else(|| {
        std::env::var("GW_MUSEAI_THREAD_RETENTION_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(7200)
    });

    let (threads_purged, jobs_purged) =
        crate::providers::museai::threads::cleanup_cycle_with_config(ttl_secs, config);

    let body = serde_json::json!({
        "object": "threads.cleanup",
        "retention_secs": ttl_secs,
        "threads_purged": threads_purged,
        "jobs_purged": jobs_purged,
        "status": "completed"
    });
    let response_str =
        serde_json::to_string(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    crate::http::write_json_ok(client, &response_str)?;
    client.flush()
}
