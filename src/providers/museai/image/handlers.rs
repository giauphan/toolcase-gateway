//! HTTP handlers for Muse AI image generation and job querying.

use crate::config::Config;
use crate::http::{header_value, write_error, Request};
use crate::providers::museai::handlers::is_sync_requested;
use crate::providers::museai::MuseApprovalRequired;
use std::io::{self, Write};
use std::net::TcpStream;

use super::lifecycle::{is_valid_job_id, load_image_job};
use super::service::{create_image, image_job_response, submit_image};

fn sanitize_host_header(host: Option<&str>) -> Option<&str> {
    host.and_then(|h| {
        let trimmed = h.trim();
        if !trimmed.is_empty()
            && trimmed.len() <= 253
            && trimmed
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == ':' || c == '-')
        {
            Some(trimmed)
        } else {
            None
        }
    })
}

pub fn handle_create_image(
    client: &mut TcpStream,
    request_body: &[u8],
    config: &Config,
) -> io::Result<()> {
    let wait = is_sync_requested("", &[], request_body);
    handle_create_image_internal(client, request_body, config, wait, None)
}

pub fn handle_create_image_request(
    client: &mut TcpStream,
    request: &Request,
    config: &Config,
) -> io::Result<()> {
    let host = sanitize_host_header(header_value(&request.headers, "host"));
    let wait = is_sync_requested(&request.path, &request.headers, &request.body);
    handle_create_image_internal(client, &request.body, config, wait, host)
}

fn handle_create_image_internal(
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
        create_image(request_body, config)
    } else {
        submit_image(request_body, config)
    };

    match result {
        Ok(mut result_json) => {
            let job_id = result_json
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();
            if !job_id.is_empty() {
                let preview = format!("/muse-ai/v1/images/{job_id}/content");
                let download = format!("/muse-ai/v1/images/{job_id}/download");
                result_json["preview_url"] = serde_json::Value::String(preview.clone());
                result_json["download_url"] = serde_json::Value::String(download.clone());

                if let Some(host_str) = host {
                    result_json["full_preview_url"] =
                        serde_json::Value::String(format!("http://{host_str}{preview}"));
                    result_json["full_download_url"] =
                        serde_json::Value::String(format!("http://{host_str}{download}"));
                }
            }

            if let Some(host_str) = host {
                if let Some(img_url) = result_json.get("image_url").and_then(|u| u.as_str()) {
                    if img_url.starts_with('/') {
                        let full = format!("http://{host_str}{img_url}");
                        result_json["full_image_url"] = serde_json::Value::String(full);
                    } else if !img_url.is_empty() {
                        result_json["full_image_url"] =
                            serde_json::Value::String(img_url.to_string());
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
                .and_then(|err| err.downcast_ref::<MuseApprovalRequired>())
                .is_some()
            {
                return write_error(
                    client,
                    409,
                    "Conflict",
                    &format!(
                        "Failed to create image: a scoped Muse permission approval is pending; grant it in the browser and retry. Details: {}",
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
            write_error(
                client,
                status_code,
                error_type,
                &format!("Failed to create image: {}", e),
            )
        }
    }
}

pub fn handle_get_image(client: &mut TcpStream, job_id: &str) -> io::Result<()> {
    if !is_valid_job_id(job_id) {
        return write_error(client, 400, "Bad Request", "Invalid job id");
    }
    handle_get_image_with_host(client, job_id, None, false)
}

pub fn handle_get_image_request(
    client: &mut TcpStream,
    job_id: &str,
    request: &Request,
) -> io::Result<()> {
    if !is_valid_job_id(job_id) {
        return write_error(client, 400, "Bad Request", "Invalid job id");
    }
    let host = sanitize_host_header(header_value(&request.headers, "host"));
    let is_head = request.method.eq_ignore_ascii_case("head");
    handle_get_image_with_host(client, job_id, host, is_head)
}

fn handle_get_image_with_host(
    client: &mut TcpStream,
    job_id: &str,
    host: Option<&str>,
    is_head: bool,
) -> io::Result<()> {
    match load_image_job(job_id) {
        Ok(record) => {
            let mut response = image_job_response(&record);
            let preview = format!("/muse-ai/v1/images/{job_id}/content");
            let download = format!("/muse-ai/v1/images/{job_id}/download");
            response["preview_url"] = serde_json::Value::String(preview.clone());
            response["download_url"] = serde_json::Value::String(download.clone());

            if let Some(host_str) = host {
                response["full_preview_url"] =
                    serde_json::Value::String(format!("http://{host_str}{preview}"));
                response["full_download_url"] =
                    serde_json::Value::String(format!("http://{host_str}{download}"));
                if let Some(img_url) = response.get("image_url").and_then(|u| u.as_str()) {
                    if img_url.starts_with('/') {
                        let full = format!("http://{host_str}{img_url}");
                        response["full_image_url"] = serde_json::Value::String(full);
                    } else if !img_url.is_empty() {
                        response["full_image_url"] = serde_json::Value::String(img_url.to_string());
                    }
                }
            }
            let body = serde_json::to_string(&response)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            client.write_all(header.as_bytes())?;
            if !is_head {
                client.write_all(body.as_bytes())?;
            }
            client.flush()
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            write_error(client, 404, "Not Found", "Image job not found")
        }
        Err(e) => write_error(
            client,
            500,
            "Internal Server Error",
            &format!("Failed to read image job: {e}"),
        ),
    }
}

pub fn handle_get_image_content(
    client: &mut TcpStream,
    job_id: &str,
    request: &Request,
) -> io::Result<()> {
    if !is_valid_job_id(job_id) {
        return write_error(client, 400, "Bad Request", "Invalid job id");
    }
    let is_download = request.path.ends_with("/download");
    let is_head = request.method.eq_ignore_ascii_case("head");

    // 1. If local image media is saved in .gw_state/media/{job_id}.{ext}
    if let Ok((bytes, mime)) = super::media::load_image_media(job_id) {
        let ext = super::media::guess_extension(mime);
        let disposition = if is_download {
            format!("attachment; filename=\"{job_id}.{ext}\"")
        } else {
            format!("inline; filename=\"{job_id}.{ext}\"")
        };
        let header = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: {mime}\r\n\
             Content-Length: {}\r\n\
             Accept-Ranges: bytes\r\n\
             Access-Control-Allow-Origin: *\r\n\
             Access-Control-Allow-Headers: Range, Content-Type, Accept\r\n\
             Content-Disposition: {disposition}\r\n\
             Connection: close\r\n\r\n",
            bytes.len()
        );
        client.write_all(header.as_bytes())?;
        if !is_head {
            client.write_all(&bytes)?;
        }
        return client.flush();
    }

    // 2. Fallback to persisted job record and redirect to upstream/external image URL
    match load_image_job(job_id) {
        Ok(record) => {
            if !record.image_url.is_empty()
                && (record.image_url.starts_with("http://")
                    || record.image_url.starts_with("https://"))
            {
                let header = format!(
                    "HTTP/1.1 302 Found\r\n\
                     Location: {}\r\n\
                     Access-Control-Allow-Origin: *\r\n\
                     Content-Length: 0\r\n\
                     Connection: close\r\n\r\n",
                    record.image_url
                );
                client.write_all(header.as_bytes())?;
                return client.flush();
            }
            write_error(
                client,
                404,
                "Not Found",
                &format!("image media not yet ready or verified for job: {job_id}"),
            )
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => write_error(
            client,
            404,
            "Not Found",
            &format!("image content not found for job: {job_id}"),
        ),
        Err(e) => write_error(
            client,
            500,
            "Internal Server Error",
            &format!("failed to read image job: {e}"),
        ),
    }
}
