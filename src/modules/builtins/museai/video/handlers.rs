//! HTTP route handlers for Muse AI Video generation, status polling, and content streaming.

use crate::config::Config;
use crate::providers::museai::video::submit_video;
use crate::providers::museai::MuseApprovalRequired;
use std::io::{self, Write};
use std::net::TcpStream;

fn is_truthy_flag(val: &str) -> bool {
    val.is_empty()
        || val.eq_ignore_ascii_case("true")
        || val == "1"
        || val.eq_ignore_ascii_case("yes")
}

fn is_falsy_flag(val: &str) -> bool {
    val.eq_ignore_ascii_case("false") || val == "0" || val.eq_ignore_ascii_case("no")
}

fn query_signals_explicit_async(query: &str) -> bool {
    for param in query.split('&') {
        let mut parts = param.split('=');
        let key = parts.next().unwrap_or("").trim();
        let val = parts.next().unwrap_or("").trim();
        if key.eq_ignore_ascii_case("async") && is_truthy_flag(val) {
            return true;
        }
        if (key.eq_ignore_ascii_case("wait") || key.eq_ignore_ascii_case("sync"))
            && is_falsy_flag(val)
        {
            return true;
        }
    }
    false
}

fn body_signals_explicit_async(body: &[u8]) -> bool {
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    if let Some(async_val) = json.get("async") {
        if async_val.as_bool() == Some(true)
            || async_val
                .as_str()
                .map(|s| is_truthy_flag(s.trim()))
                .unwrap_or(false)
        {
            return true;
        }
    }
    if let Some(wait_val) = json.get("wait").or_else(|| json.get("sync")) {
        if wait_val.as_bool() == Some(false)
            || wait_val
                .as_str()
                .map(|s| is_falsy_flag(s.trim()))
                .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

fn query_signals_explicit_sync(query: &str) -> bool {
    for param in query.split('&') {
        let mut parts = param.split('=');
        let key = parts.next().unwrap_or("").trim();
        let val = parts.next().unwrap_or("").trim();
        if (key.eq_ignore_ascii_case("wait") || key.eq_ignore_ascii_case("sync"))
            && is_truthy_flag(val)
        {
            return true;
        }
    }
    false
}

fn headers_signal_sync(headers: &[(String, String)]) -> bool {
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("prefer") {
            let lower = v.to_ascii_lowercase();
            if lower.contains("respond-async=false") || lower.contains("wait") {
                return true;
            }
        }
        if k.eq_ignore_ascii_case("x-wait") || k.eq_ignore_ascii_case("x-sync") {
            let lower = v.trim().to_ascii_lowercase();
            if !lower.is_empty() && is_truthy_flag(&lower) {
                return true;
            }
        }
        if k.eq_ignore_ascii_case("user-agent") && v.to_ascii_lowercase().starts_with("curl/") {
            return true;
        }
    }
    false
}

fn body_signals_explicit_sync(body: &[u8]) -> bool {
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    if let Some(wait_val) = json.get("wait").or_else(|| json.get("sync")) {
        return wait_val.as_bool() == Some(true)
            || wait_val
                .as_str()
                .map(|s| is_truthy_flag(s.trim()))
                .unwrap_or(false);
    }
    false
}

pub fn is_sync_requested(path: &str, headers: &[(String, String)], body: &[u8]) -> bool {
    let query = path.split('?').nth(1).unwrap_or("");
    if query_signals_explicit_async(query) || body_signals_explicit_async(body) {
        return false;
    }
    if query_signals_explicit_sync(query)
        || headers_signal_sync(headers)
        || body_signals_explicit_sync(body)
    {
        return true;
    }
    false
}

pub fn handle_create_video(
    client: &mut TcpStream,
    request_body: &[u8],
    config: &Config,
) -> io::Result<()> {
    let wait = is_sync_requested("", &[], request_body);
    handle_create_video_internal(client, request_body, config, wait, None)
}

pub fn handle_create_video_request(
    client: &mut TcpStream,
    request: &crate::http::Request,
    config: &Config,
) -> io::Result<()> {
    let host = crate::http::header_value(&request.headers, "host");
    let wait = is_sync_requested(&request.path, &request.headers, &request.body);
    handle_create_video_internal(client, &request.body, config, wait, host)
}

fn handle_create_video_internal(
    client: &mut TcpStream,
    request_body: &[u8],
    config: &Config,
    wait: bool,
    host: Option<&str>,
) -> io::Result<()> {
    if wait {
        let _ = client.set_write_timeout(Some(std::time::Duration::from_secs(360)));
    }

    let result = if wait {
        crate::providers::museai::video::create_video(request_body, config)
    } else {
        submit_video(request_body, config)
    };

    match result {
        Ok(mut result_json) => {
            if let Some(host_str) = host {
                if let Some(v_url) = result_json.get("video_url").and_then(|u| u.as_str()) {
                    if v_url.starts_with('/') {
                        let full = format!("http://{}{}", host_str, v_url);
                        result_json["full_video_url"] = serde_json::Value::String(full);
                    } else if !v_url.is_empty() {
                        result_json["full_video_url"] =
                            serde_json::Value::String(v_url.to_string());
                    }
                }
            }
            let response_str = serde_json::to_string(&result_json)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            crate::http::write_json_ok(client, &response_str)?;
            client.flush()
        }
        Err(e) => {
            if e.get_ref()
                .and_then(|e| e.downcast_ref::<MuseApprovalRequired>())
                .is_some()
            {
                return crate::http::write_error(
                    client,
                    409,
                    "Conflict",
                    &format!(
                        "Failed to create video: a scoped Muse permission approval is pending; grant it in the browser and retry. Details: {}",
                        e
                    ),
                );
            }
            let (status_code, error_type) = crate::http::io_error_to_http_status(e.kind());
            crate::http::write_error(
                client,
                status_code,
                error_type,
                &format!("Failed to create video: {}", e),
            )
        }
    }
}

pub fn handle_update_video_prompt(
    client: &mut TcpStream,
    job_id: &str,
    request_body: &[u8],
) -> io::Result<()> {
    let body: serde_json::Value = match serde_json::from_slice(request_body) {
        Ok(v) => v,
        Err(e) => {
            return crate::http::write_error(
                client,
                400,
                "Bad Request",
                &format!("Invalid JSON payload: {e}"),
            );
        }
    };

    let prompt = match body.get("prompt").and_then(|v| v.as_str()) {
        Some(p) if !p.trim().is_empty() => p.trim(),
        _ => {
            return crate::http::write_error(
                client,
                400,
                "Bad Request",
                "missing or empty 'prompt' field",
            );
        }
    };

    match crate::providers::museai::lifecycle::update_job_prompt(job_id, prompt) {
        Ok(record) => {
            let response = serde_json::json!({
                "id": record.job_id,
                "object": "video.generation",
                "updated_at": record.last_activity_at,
                "prompt": record.prompt,
                "status": "updated"
            });
            let response_str = serde_json::to_string(&response)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            crate::http::write_json_ok(client, &response_str)?;
            client.flush()
        }
        Err(e) => write_video_job_lookup_error(client, e),
    }
}

pub fn handle_get_video(client: &mut TcpStream, job_id: &str) -> io::Result<()> {
    handle_get_video_with_host(client, job_id, None)
}

pub fn handle_get_video_request(
    client: &mut TcpStream,
    job_id: &str,
    request: &crate::http::Request,
) -> io::Result<()> {
    let host = crate::http::header_value(&request.headers, "host");
    handle_get_video_with_host(client, job_id, host)
}

fn handle_get_video_with_host(
    client: &mut TcpStream,
    job_id: &str,
    host: Option<&str>,
) -> io::Result<()> {
    match crate::providers::museai::lifecycle::load_video_job(job_id) {
        Ok(record) => {
            let mut response = serde_json::json!({
                "id": record.job_id,
                "object": "video.generation",
                "created": record.created_at,
                "provider": "muse_chat",
                "status": record.status,
                "approval_status": record.approval_status,
                "approval_id": record.approval_id,
                "output_verified": record.output_verified,
                "media_validated": record.media_validated,
                "video_url": record.video_url,
                "prompt": record.prompt,
                "model": record.model,
                "requested_model": record.requested_model,
                "aspect_ratio": record.aspect_ratio,
                "duration": record.duration,
                "thread_id": record.thread_id,
                "revisions": record.revisions,
                "last_activity_at": record.last_activity_at,
            });
            if let Some(host_str) = host {
                if record.video_url.starts_with('/') {
                    let full = format!("http://{}{}", host_str, record.video_url);
                    response["full_video_url"] = serde_json::Value::String(full);
                } else if !record.video_url.is_empty() {
                    response["full_video_url"] =
                        serde_json::Value::String(record.video_url.clone());
                }
            }
            if let Some(ref err) = record.error {
                response["error"] = serde_json::json!({
                    "message": err,
                    "code": "VIDEO_GENERATION_FAILED"
                });
                response["error_message"] = serde_json::Value::String(err.clone());
            }
            let response_str = serde_json::to_string(&response)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            crate::http::write_json_ok(client, &response_str)?;
            client.flush()
        }
        Err(e) => write_video_job_lookup_error(client, e),
    }
}

pub fn handle_get_video_content(
    client: &mut TcpStream,
    job_id: &str,
    request: &crate::http::Request,
) -> io::Result<()> {
    if !crate::providers::museai::video::has_video_media(job_id) {
        return crate::http::write_error(
            client,
            404,
            "Not Found",
            &format!("video content not found for job: {job_id}"),
        );
    }

    let bytes = match crate::providers::museai::video::load_video_media(job_id) {
        Ok(b) => b,
        Err(e) => {
            return crate::http::write_error(
                client,
                500,
                "Internal Server Error",
                &format!("failed to read video media: {e}"),
            );
        }
    };

    let total_len = bytes.len();
    let is_head = request.method.eq_ignore_ascii_case("head");

    let range_header = request
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("range"))
        .map(|(_, v)| v.as_str());

    if let Some(range_val) = range_header {
        if let Some(range_spec) = range_val.strip_prefix("bytes=") {
            let mut parts = range_spec.split('-');
            let start_str = parts.next().unwrap_or("").trim();
            let end_str = parts.next().unwrap_or("").trim();

            let (start, end) = if start_str.is_empty() {
                let suffix_len: usize = end_str.parse().unwrap_or(0);
                let start = total_len.saturating_sub(suffix_len);
                (start, total_len.saturating_sub(1))
            } else {
                let start: usize = start_str.parse().unwrap_or(0);
                let end = if end_str.is_empty() {
                    total_len.saturating_sub(1)
                } else {
                    end_str
                        .parse()
                        .unwrap_or(total_len.saturating_sub(1))
                        .min(total_len.saturating_sub(1))
                };
                (start, end)
            };

            if start <= end && start < total_len {
                let slice = &bytes[start..=end];
                let content_len = slice.len();
                let header = format!(
                    "HTTP/1.1 206 Partial Content\r\n\
                     Content-Type: video/mp4\r\n\
                     Content-Range: bytes {start}-{end}/{total_len}\r\n\
                     Content-Length: {content_len}\r\n\
                     Accept-Ranges: bytes\r\n\
                     Access-Control-Allow-Origin: *\r\n\
                     Access-Control-Allow-Headers: Range, Content-Type, Accept\r\n\
                     Access-Control-Expose-Headers: Content-Range, Content-Length, Accept-Ranges\r\n\
                     Connection: close\r\n\r\n"
                );
                client.write_all(header.as_bytes())?;
                if !is_head {
                    client.write_all(slice)?;
                }
                return client.flush();
            }
        }
    }

    let header = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: video/mp4\r\n\
         Content-Length: {total_len}\r\n\
         Accept-Ranges: bytes\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Headers: Range, Content-Type, Accept\r\n\
         Access-Control-Expose-Headers: Content-Range, Content-Length, Accept-Ranges\r\n\
         Content-Disposition: inline; filename=\"{job_id}.mp4\"\r\n\
         Connection: close\r\n\r\n"
    );
    client.write_all(header.as_bytes())?;
    if !is_head {
        client.write_all(&bytes)?;
    }
    client.flush()
}

fn write_video_job_lookup_error(client: &mut TcpStream, e: io::Error) -> io::Result<()> {
    match e.kind() {
        io::ErrorKind::NotFound => crate::http::write_error(
            client,
            404,
            "Not Found",
            &format!("video job not found: {e}"),
        ),
        io::ErrorKind::InvalidInput => {
            crate::http::write_error(client, 400, "Bad Request", &format!("invalid job id: {e}"))
        }
        _ => crate::http::write_error(client, 500, "Internal Server Error", &e.to_string()),
    }
}
