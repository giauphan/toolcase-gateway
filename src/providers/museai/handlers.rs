use crate::config::Config;
use crate::providers::museai::transport::send_museai_request;
use crate::providers::museai::video::create_video;
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

pub fn handle_museai_v1(
    client: &mut TcpStream,
    request_body: &[u8],
    config: &Config,
) -> io::Result<()> {
    let (url, method, body_json) =
        crate::providers::museai::business::build_museai_request(request_body, config)?;

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

pub fn handle_create_video(
    client: &mut TcpStream,
    request_body: &[u8],
    config: &Config,
) -> io::Result<()> {
    match create_video(request_body, config) {
        Ok(result) => {
            let response_str = serde_json::to_string(&result)
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
    match crate::providers::museai::lifecycle::load_video_job(job_id) {
        Ok(record) => {
            let response = serde_json::json!({
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
