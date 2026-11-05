//! Video image attachment domain value object, validation, and SSRF prevention.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use std::io;
use std::net::IpAddr;

pub const MAX_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024; // 10 MB

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VideoAttachment {
    InternalImage {
        job_id: String,
        path: String,
    },
    ExternalUrl {
        url: String,
    },
    DataUri {
        mime_type: String,
        reference: String,
    },
}

impl VideoAttachment {
    pub fn as_reference(&self) -> &str {
        match self {
            Self::InternalImage { path, .. } => path.as_str(),
            Self::ExternalUrl { url } => url.as_str(),
            Self::DataUri { reference, .. } => reference.as_str(),
        }
    }
}

pub const MAX_ATTACHMENTS_PER_REQUEST: usize = 5;

pub fn parse_and_validate_attachment(
    val: &serde_json::Value,
) -> io::Result<Option<VideoAttachment>> {
    let list = parse_and_validate_attachments(val)?;
    Ok(list.into_iter().next())
}

pub fn parse_and_validate_attachments(val: &serde_json::Value) -> io::Result<Vec<VideoAttachment>> {
    let mut raw_strings = Vec::new();

    if let Some(arr) = val.get("images").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    raw_strings.push(trimmed.to_string());
                }
            } else if let Some(u) = item.get("url").and_then(|v| v.as_str()) {
                let trimmed = u.trim();
                if !trimmed.is_empty() {
                    raw_strings.push(trimmed.to_string());
                }
            }
        }
    } else if let Some(arr) = val.get("reference_images").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(u) = item.get("url").and_then(|v| v.as_str()) {
                let trimmed = u.trim();
                if !trimmed.is_empty() {
                    raw_strings.push(trimmed.to_string());
                }
            } else if let Some(s) = item.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    raw_strings.push(trimmed.to_string());
                }
            }
        }
    } else if let Some(arr) = val.get("image_urls").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    raw_strings.push(trimmed.to_string());
                }
            }
        }
    } else {
        let single = val
            .get("image_url")
            .or_else(|| val.get("image"))
            .or_else(|| val.get("attachment"))
            .or_else(|| val.get("reference_image"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        if let Some(s) = single {
            raw_strings.push(s.to_string());
        }
    }

    if raw_strings.len() > MAX_ATTACHMENTS_PER_REQUEST {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "too many image attachments: maximum is {MAX_ATTACHMENTS_PER_REQUEST}, got {}",
                raw_strings.len()
            ),
        ));
    }

    let mut result = Vec::with_capacity(raw_strings.len());
    for s in &raw_strings {
        result.push(validate_attachment_string(s)?);
    }

    Ok(result)
}

pub fn validate_attachment_string(raw: &str) -> io::Result<VideoAttachment> {
    if raw.starts_with("data:") {
        return parse_data_uri(raw);
    }

    if let Some(internal) = parse_internal_gateway_image(raw) {
        return Ok(internal);
    }

    if raw.starts_with("https://") {
        return validate_external_image_url(raw);
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "invalid image attachment: expected https:// URL, internal /muse-ai/v1/images/ path, or data:image/* data URI; got '{}'",
            if raw.len() > 60 { &raw[..60] } else { raw }
        ),
    ))
}

fn parse_data_uri(raw: &str) -> io::Result<VideoAttachment> {
    let header_end = raw.find(";base64,").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "data URI must specify base64 encoding (e.g. data:image/png;base64,...)",
        )
    })?;

    let mime_type = &raw[5..header_end]; // skip "data:"
    let allowed_mimes = ["image/png", "image/jpeg", "image/jpg", "image/webp"];
    if !allowed_mimes.contains(&mime_type) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unsupported image MIME type '{mime_type}'; supported: {}",
                allowed_mimes.join(", ")
            ),
        ));
    }

    let b64_payload = &raw[header_end + 8..];
    if b64_payload.len() > MAX_ATTACHMENT_BYTES * 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "image payload exceeds maximum size ceiling of {} bytes",
                MAX_ATTACHMENT_BYTES
            ),
        ));
    }

    let decoded = BASE64.decode(b64_payload.trim()).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid base64 payload in data URI: {e}"),
        )
    })?;

    if decoded.len() > MAX_ATTACHMENT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "decoded image exceeds maximum size ceiling of {} bytes (got {} bytes)",
                MAX_ATTACHMENT_BYTES,
                decoded.len()
            ),
        ));
    }

    Ok(VideoAttachment::DataUri {
        mime_type: mime_type.to_string(),
        reference: format!("data:{mime_type};base64,<{} bytes>", decoded.len()),
    })
}

fn parse_internal_gateway_image(raw: &str) -> Option<VideoAttachment> {
    let marker = "/muse-ai/v1/images/";
    let idx = raw.find(marker)?;
    let after = &raw[idx + marker.len()..];
    let job_id = after.split('/').next().unwrap_or(after);

    if job_id.is_empty()
        || job_id.len() > 128
        || !job_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }

    Some(VideoAttachment::InternalImage {
        job_id: job_id.to_string(),
        path: format!("/muse-ai/v1/images/{job_id}/content"),
    })
}

fn validate_external_image_url(url_str: &str) -> io::Result<VideoAttachment> {
    let host = secure_url_host(url_str).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "malformed external image URL")
    })?;

    if !is_secure_host(&host) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("image URL host '{host}' rejected: loopback, private, or local destinations are forbidden"),
        ));
    }

    if !has_image_extension_or_trusted_host(url_str, &host) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "external image URL must have a supported extension (.png, .jpg, .jpeg, .webp) or originate from a trusted CDN",
        ));
    }

    Ok(VideoAttachment::ExternalUrl {
        url: url_str.to_string(),
    })
}

fn secure_url_host(url_str: &str) -> Option<String> {
    let rest = url_str.strip_prefix("https://")?;
    let authority = rest.split('/').next().unwrap_or(rest);
    let host = authority.split('@').next_back().unwrap_or(authority);
    let host = if host.starts_with('[') {
        host.split(']').next()?.trim_start_matches('[')
    } else {
        host.split(':').next().unwrap_or(host)
    };
    Some(host.to_ascii_lowercase().trim_end_matches('.').to_string())
}

fn is_secure_host(host: &str) -> bool {
    if host.is_empty()
        || host == "localhost"
        || host.ends_with(".internal")
        || host.ends_with(".local")
    {
        return false;
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(ipv4) => {
                !(ipv4.is_private()
                    || ipv4.is_loopback()
                    || ipv4.is_link_local()
                    || ipv4.is_unspecified()
                    || ipv4.is_broadcast())
            }
            IpAddr::V6(ipv6) => {
                let first = ipv6.segments()[0];
                !(ipv6.is_loopback()
                    || ipv6.is_unspecified()
                    || first & 0xfe00 == 0xfc00
                    || first & 0xffc0 == 0xfe80)
            }
        }
    } else {
        true
    }
}

fn has_image_extension_or_trusted_host(url_str: &str, host: &str) -> bool {
    const TRUSTED_DOMAINS: [&str; 6] = [
        "images.unsplash.com",
        "cdn.midjourney.com",
        "muse.ai",
        "oaidalleapiprodscus.blob.core.windows.net",
        "xx.fbcdn.net",
        "cdn.meta.com",
    ];

    if TRUSTED_DOMAINS
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
    {
        return true;
    }

    let path = url_str.split('?').next().unwrap_or(url_str);
    const EXTENSIONS: [&str; 4] = [".png", ".jpg", ".jpeg", ".webp"];
    EXTENSIONS
        .iter()
        .any(|ext| path.to_ascii_lowercase().ends_with(ext))
}
