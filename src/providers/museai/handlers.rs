use crate::config::Config;
use crate::providers::museai::transport::send_museai_request;
use crate::providers::museai::video::submit_video;
use crate::providers::museai::MuseApprovalRequired;
use std::io::{self, Write};
use std::net::TcpStream;

pub fn write_chat_completion(client: &mut TcpStream, response: &str) -> io::Result<()> {
    client.write_all(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .as_bytes(),
    )?;
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
            let status_text = match upstream_resp.status {
                200 => "OK",
                201 => "Created",
                204 => "No Content",
                400 => "Bad Request",
                401 => "Unauthorized",
                403 => "Forbidden",
                404 => "Not Found",
                408 => "Request Timeout",
                429 => "Too Many Requests",
                500 => "Internal Server Error",
                502 => "Bad Gateway",
                503 => "Service Unavailable",
                504 => "Gateway Timeout",
                _ => "Internal Server Error",
            };

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

pub fn is_sync_requested(path: &str, headers: &[(String, String)], body: &[u8]) -> bool {
    // 1. Check explicit async flags (takes precedence)
    if let Some(query) = path.split('?').nth(1) {
        for param in query.split('&') {
            let mut parts = param.split('=');
            let key = parts.next().unwrap_or("").trim();
            let val = parts.next().unwrap_or("").trim();
            if key.eq_ignore_ascii_case("async")
                && (val.is_empty()
                    || val.eq_ignore_ascii_case("true")
                    || val == "1"
                    || val.eq_ignore_ascii_case("yes"))
            {
                return false;
            }
            if (key.eq_ignore_ascii_case("wait") || key.eq_ignore_ascii_case("sync"))
                && (val.eq_ignore_ascii_case("false")
                    || val == "0"
                    || val.eq_ignore_ascii_case("no"))
            {
                return false;
            }
        }
    }

    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(async_val) = json.get("async") {
            if async_val.as_bool() == Some(true)
                || async_val
                    .as_str()
                    .map(|s| s.eq_ignore_ascii_case("true") || s == "1")
                    .unwrap_or(false)
            {
                return false;
            }
        }
        if let Some(wait_val) = json.get("wait").or_else(|| json.get("sync")) {
            if wait_val.as_bool() == Some(false)
                || wait_val
                    .as_str()
                    .map(|s| s.eq_ignore_ascii_case("false") || s == "0")
                    .unwrap_or(false)
            {
                return false;
            }
        }
    }

    // 2. Check query params in path
    if let Some(query) = path.split('?').nth(1) {
        for param in query.split('&') {
            let mut parts = param.split('=');
            let key = parts.next().unwrap_or("").trim();
            let val = parts.next().unwrap_or("").trim();
            if (key.eq_ignore_ascii_case("wait") || key.eq_ignore_ascii_case("sync"))
                && (val.is_empty()
                    || val.eq_ignore_ascii_case("true")
                    || val == "1"
                    || val.eq_ignore_ascii_case("yes"))
            {
                return true;
            }
        }
    }

    // 3. Check headers
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("prefer") {
            let lower = v.to_ascii_lowercase();
            if lower.contains("respond-async=false") || lower.contains("wait") {
                return true;
            }
        }
        if k.eq_ignore_ascii_case("x-wait") || k.eq_ignore_ascii_case("x-sync") {
            let lower = v.trim().to_ascii_lowercase();
            if lower == "true" || lower == "1" || lower == "yes" {
                return true;
            }
        }
    }

    // 4. Check JSON body
    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(wait_val) = json.get("wait").or_else(|| json.get("sync")) {
            if wait_val.as_bool() == Some(true)
                || wait_val
                    .as_str()
                    .map(|s| {
                        s.eq_ignore_ascii_case("true") || s == "1" || s.eq_ignore_ascii_case("yes")
                    })
                    .unwrap_or(false)
            {
                return true;
            }
        }
    }

    // 5. Default to sync for command-line clients (e.g. curl) if not explicitly set to async
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("user-agent") {
            let lower = v.to_ascii_lowercase();
            if lower.starts_with("curl/") {
                return true;
            }
        }
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
            client.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_str.len(),
                    response_str
                )
                .as_bytes(),
            )?;
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
            let (status_code, error_type) = match e.kind() {
                io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => (400, "Bad Request"),
                io::ErrorKind::PermissionDenied => (401, "Unauthorized"),
                io::ErrorKind::NotFound => (404, "Not Found"),
                io::ErrorKind::WouldBlock | io::ErrorKind::AlreadyExists => (409, "Conflict"),
                io::ErrorKind::Unsupported => (501, "Not Implemented"),
                io::ErrorKind::TimedOut => (504, "Gateway Timeout"),
                _ => (502, "Bad Gateway"),
            };
            crate::http::write_error(
                client,
                status_code,
                error_type,
                &format!("Failed to create video: {}", e),
            )
        }
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
            client.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_str.len(),
                    response_str
                )
                .as_bytes(),
            )?;
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
            client.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_str.len(),
                    response_str
                )
                .as_bytes(),
            )?;
            client.flush()
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => crate::http::write_error(
            client,
            404,
            "Not Found",
            &format!("video job not found: {e}"),
        ),
        Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
            crate::http::write_error(client, 400, "Bad Request", &format!("invalid job id: {e}"))
        }
        Err(e) => crate::http::write_error(client, 500, "Internal Server Error", &e.to_string()),
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
            client.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_str.len(),
                    response_str
                )
                .as_bytes(),
            )?;
            client.flush()
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => crate::http::write_error(
            client,
            404,
            "Not Found",
            &format!("video job not found: {e}"),
        ),
        Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
            crate::http::write_error(client, 400, "Bad Request", &format!("invalid job id: {e}"))
        }
        Err(e) => crate::http::write_error(client, 500, "Internal Server Error", &e.to_string()),
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
