//! Multipart form-data parser for video creation requests.
//!
//! Enables standard multipart/form-data uploads (`curl -F`, SDKs, HTML forms)
//! alongside application/json payloads, converting binary files to validated data URIs.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use std::collections::HashMap;
use std::io;

const MAX_IMAGE_PARTS: usize = 5;
const MAX_FILE_SIZE: usize = 10 * 1024 * 1024; // 10 MB per file

/// Parses a multipart/form-data body into a JSON Value compatible with `parse_and_validate_video_request`.
pub fn parse_multipart_form(body: &[u8]) -> io::Result<serde_json::Value> {
    let boundary_line = extract_boundary_line(body).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "malformed multipart/form-data: missing boundary delimiter",
        )
    })?;

    let parts = split_multipart_parts(body, &boundary_line)?;
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut images: Vec<String> = Vec::new();

    for (headers, part_body) in parts {
        let (name, filename, content_type) = parse_part_headers(&headers);
        let Some(field_name) = name else {
            continue;
        };

        if is_file_field(&field_name, filename.is_some()) {
            if part_body.is_empty() {
                continue;
            }
            if part_body.len() > MAX_FILE_SIZE {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "image part exceeds size limit of 10 MB ({})",
                        part_body.len()
                    ),
                ));
            }
            if images.len() >= MAX_IMAGE_PARTS {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("too many image attachments: maximum is {MAX_IMAGE_PARTS}"),
                ));
            }
            let mime = detect_image_mime(part_body, content_type.as_deref())?;
            let data_uri = format!("data:{mime};base64,{}", BASE64.encode(part_body));
            images.push(data_uri);
        } else {
            let val = String::from_utf8(part_body.to_vec()).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("invalid UTF-8 in field '{field_name}': {e}"),
                )
            })?;
            fields.insert(field_name, val.trim().to_string());
        }
    }

    build_json_from_fields(fields, images)
}

fn extract_boundary_line(body: &[u8]) -> Option<Vec<u8>> {
    let non_ws_idx = body.iter().position(|b| !b.is_ascii_whitespace())?;
    let slice = &body[non_ws_idx..];
    if !slice.starts_with(b"--") {
        return None;
    }
    let end_idx = slice
        .iter()
        .position(|&b| b == b'\r' || b == b'\n')
        .unwrap_or(slice.len());
    let line = &slice[..end_idx];
    if line.len() > 2 {
        Some(line.to_vec())
    } else {
        None
    }
}

fn split_multipart_parts<'a>(
    body: &'a [u8],
    boundary_line: &[u8],
) -> io::Result<Vec<(Vec<u8>, &'a [u8])>> {
    let mut parts = Vec::new();
    let mut search_from = 0;

    let delimiter = boundary_line;

    while let Some(pos) = find_subslice(&body[search_from..], delimiter) {
        let abs_pos = search_from + pos;
        let after_delim = abs_pos + delimiter.len();

        if body.get(after_delim..after_delim + 2) == Some(b"--") {
            break;
        }

        let part_start = skip_line_break(&body[after_delim..]) + after_delim;
        search_from = part_start;

        let next_pos = match find_subslice(&body[search_from..], delimiter) {
            Some(p) => search_from + p,
            None => break,
        };

        let raw_part = &body[part_start..next_pos];
        let trimmed_part = trim_trailing_line_break(raw_part);

        if let Some((headers, part_body)) = split_headers_and_body(trimmed_part) {
            parts.push((headers, part_body));
        }

        search_from = next_pos;
    }

    Ok(parts)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn skip_line_break(slice: &[u8]) -> usize {
    if slice.starts_with(b"\r\n") {
        2
    } else if slice.starts_with(b"\n") {
        1
    } else {
        0
    }
}

fn trim_trailing_line_break(slice: &[u8]) -> &[u8] {
    if slice.ends_with(b"\r\n") {
        &slice[..slice.len() - 2]
    } else if slice.ends_with(b"\n") {
        &slice[..slice.len() - 1]
    } else {
        slice
    }
}

fn split_headers_and_body(part: &[u8]) -> Option<(Vec<u8>, &[u8])> {
    if let Some(pos) = find_subslice(part, b"\r\n\r\n") {
        Some((part[..pos].to_vec(), &part[pos + 4..]))
    } else if let Some(pos) = find_subslice(part, b"\n\n") {
        Some((part[..pos].to_vec(), &part[pos + 2..]))
    } else {
        None
    }
}

fn parse_part_headers(headers: &[u8]) -> (Option<String>, Option<String>, Option<String>) {
    let text = String::from_utf8_lossy(headers);
    let mut name = None;
    let mut filename = None;
    let mut content_type = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Content-Disposition:") {
            name = extract_header_param(rest, "name=");
            filename = extract_header_param(rest, "filename=");
        } else if let Some(rest) = trimmed.strip_prefix("Content-Type:") {
            content_type = Some(rest.trim().to_ascii_lowercase());
        }
    }

    (name, filename, content_type)
}

fn extract_header_param(header_val: &str, param: &str) -> Option<String> {
    let idx = header_val.find(param)?;
    let rest = &header_val[idx + param.len()..];
    let trimmed = rest.trim();
    if let Some(stripped) = trimmed.strip_prefix('"') {
        let end = stripped.find('"').unwrap_or(stripped.len());
        Some(stripped[..end].to_string())
    } else {
        let end = trimmed.find(';').unwrap_or(trimmed.len());
        Some(trimmed[..end].trim().to_string())
    }
}

fn is_file_field(name: &str, has_filename: bool) -> bool {
    has_filename
        || name == "image"
        || name == "images"
        || name == "file"
        || name == "attachment"
        || name.starts_with("image[")
        || name.starts_with("images[")
}

fn detect_image_mime(bytes: &[u8], content_type: Option<&str>) -> io::Result<String> {
    if let Some(ct) = content_type {
        let ct_lower = ct.to_ascii_lowercase();
        if ct_lower.contains("image/png") {
            return Ok("image/png".to_string());
        }
        if ct_lower.contains("image/jpeg") || ct_lower.contains("image/jpg") {
            return Ok("image/jpeg".to_string());
        }
        if ct_lower.contains("image/webp") {
            return Ok("image/webp".to_string());
        }
    }

    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok("image/png".to_string())
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Ok("image/jpeg".to_string())
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Ok("image/webp".to_string())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "uploaded file is not a supported image (expected image/png, image/jpeg, or image/webp)",
        ))
    }
}

fn build_json_from_fields(
    mut fields: HashMap<String, String>,
    images: Vec<String>,
) -> io::Result<serde_json::Value> {
    let mut map = serde_json::Map::new();

    if let Some(prompt) = fields.remove("prompt") {
        map.insert("prompt".to_string(), serde_json::Value::String(prompt));
    }
    if let Some(aspect_ratio) = fields.remove("aspect_ratio") {
        map.insert(
            "aspect_ratio".to_string(),
            serde_json::Value::String(aspect_ratio),
        );
    }
    if let Some(duration) = fields.remove("duration") {
        if let Ok(d_num) = duration.parse::<u64>() {
            map.insert("duration".to_string(), serde_json::json!(d_num));
        } else {
            map.insert("duration".to_string(), serde_json::Value::String(duration));
        }
    }
    if let Some(model) = fields.remove("model") {
        map.insert("model".to_string(), serde_json::Value::String(model));
    }
    if let Some(job_id) = fields.remove("job_id").or_else(|| fields.remove("id")) {
        map.insert("job_id".to_string(), serde_json::Value::String(job_id));
    }
    if let Some(key) = fields
        .remove("idempotency_key")
        .or_else(|| fields.remove("submission_key"))
    {
        map.insert("submission_key".to_string(), serde_json::Value::String(key));
    }
    if let Some(wait) = fields.remove("wait").or_else(|| fields.remove("sync")) {
        map.insert("wait".to_string(), serde_json::Value::String(wait));
    }

    for (k, v) in fields {
        map.insert(k, serde_json::Value::String(v));
    }

    if !images.is_empty() {
        map.insert("images".to_string(), serde_json::json!(images));
    }

    Ok(serde_json::Value::Object(map))
}
