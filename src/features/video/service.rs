use std::io::{self, Write};
use std::net::TcpStream;

use crate::features::video::types::{validate_secure_url, VideoCapabilityCatalog};

fn write_json_response(
    client: &mut TcpStream,
    status: u16,
    reason: &str,
    body: &serde_json::Value,
) -> io::Result<()> {
    let body_str =
        serde_json::to_string(body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body_str.len(),
        body_str
    );
    client.write_all(response.as_bytes())?;
    client.flush()
}

fn write_video_error(
    client: &mut TcpStream,
    status: u16,
    reason: &str,
    code: &str,
    category: &str,
    message: &str,
) -> io::Result<()> {
    let err_body = serde_json::json!({
        "error": {
            "code": code,
            "category": category,
            "retryable": false,
            "submission_unknown": false,
            "message": message
        },
        "request_id": "redacted-request-id"
    });
    write_json_response(client, status, reason, &err_body)
}

fn write_invalid_input(client: &mut TcpStream, message: &str) -> io::Result<()> {
    write_video_error(
        client,
        400,
        "Bad Request",
        "INVALID_INPUT",
        "invalid_input",
        message,
    )
}

pub fn handle_video_providers_catalog(client: &mut TcpStream) -> io::Result<()> {
    let catalog = VideoCapabilityCatalog {
        schema_version: "video.transport.v1".into(),
        providers: vec![],
    };
    let body =
        serde_json::to_value(catalog).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_json_response(client, 200, "OK", &body)
}

pub fn handle_video_task_submit(
    client: &mut TcpStream,
    request_body: &[u8],
    provider: &str,
) -> io::Result<()> {
    let body: serde_json::Value = if request_body.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(request_body).unwrap_or_else(|_| serde_json::json!({}))
    };

    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let prompt = body.get("prompt").and_then(|v| v.as_str()).unwrap_or("");

    if model.is_empty() || prompt.trim().is_empty() || provider.is_empty() {
        return write_invalid_input(client, "missing required model, prompt, or provider");
    }

    if let Some(images) = body.get("reference_images").and_then(|v| v.as_array()) {
        for img in images {
            if let Some(url) = img.get("url").and_then(|v| v.as_str()) {
                if !validate_secure_url(url) {
                    return write_invalid_input(
                        client,
                        "reference_images url failed security validation",
                    );
                }
            }
        }
    }

    write_video_error(
        client,
        422,
        "Unprocessable Entity",
        "UNSUPPORTED_CAPABILITY",
        "invalid_input",
        "provider/model/operation is not production-ready",
    )
}

pub fn handle_video_task_status(
    client: &mut TcpStream,
    provider: &str,
    task_id: &str,
) -> io::Result<()> {
    if provider.is_empty() || task_id.is_empty() {
        return write_invalid_input(client, "missing provider or task id");
    }

    write_video_error(
        client,
        404,
        "Not Found",
        "NOT_FOUND",
        "not_found",
        "task not found on upstream provider",
    )
}

pub fn handle_video_task_reconciliation(
    client: &mut TcpStream,
    provider: &str,
    idempotency_key: &str,
) -> io::Result<()> {
    if provider.is_empty() || idempotency_key.is_empty() {
        return write_invalid_input(client, "missing provider or idempotency key");
    }

    write_video_error(
        client,
        501,
        "Not Implemented",
        "RECONCILE_UNSUPPORTED",
        "unsupported",
        "provider does not support key-based reconciliation lookup",
    )
}
