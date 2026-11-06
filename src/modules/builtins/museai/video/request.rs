//! Video generation request parsing, validation, and initialization.

use super::attachment::{parse_and_validate_attachments, VideoAttachment};
use super::url::{
    build_video_prompt_with_attachments, normalize_video_model, SUPPORTED_ASPECT_RATIOS,
    SUPPORTED_MODELS,
};
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug)]
pub struct ValidatedVideoRequest {
    pub allocated_job_id: String,
    pub prompt: String,
    pub upstream_model: String,
    pub requested_model: String,
    pub aspect_ratio: String,
    pub duration: u64,
    pub existing_job: Option<super::super::lifecycle::VideoJobRecord>,
    pub submission_key: Option<String>,
    pub attachment: Option<VideoAttachment>,
    pub attachment_url: Option<String>,
    pub attachments: Vec<VideoAttachment>,
    pub attachment_urls: Vec<String>,
    pub request_json: Vec<u8>,
    pub now: u64,
}

fn parse_request_json(request_body: &[u8]) -> io::Result<serde_json::Value> {
    if request_body.is_empty() {
        return Ok(serde_json::json!({}));
    }

    let non_ws = request_body
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .take(2)
        .collect::<Vec<_>>();

    if non_ws == b"--" {
        return super::multipart::parse_multipart_form(request_body);
    }

    serde_json::from_slice(request_body)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid JSON: {e}")))
}

fn extract_prompt(body: &serde_json::Value) -> io::Result<String> {
    let prompt = body
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing 'prompt' field"))?
        .trim()
        .to_string();
    if prompt.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "prompt must not be empty",
        ));
    }
    Ok(prompt)
}

fn load_parent_job(
    body: &serde_json::Value,
) -> io::Result<Option<super::super::lifecycle::VideoJobRecord>> {
    let parent_job_id = body
        .get("job_id")
        .or_else(|| body.get("parent_id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    match parent_job_id {
        Some(id) => Ok(Some(super::super::lifecycle::load_video_job(id)?)),
        None => Ok(None),
    }
}

fn extract_submission_key(body: &serde_json::Value) -> Option<String> {
    body.get("idempotency_key")
        .or_else(|| body.get("submission_key"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn resolve_model(
    body: &serde_json::Value,
    existing_job: Option<&super::super::lifecycle::VideoJobRecord>,
) -> io::Result<(String, String)> {
    let requested_model = body
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| existing_job.map(|j| j.requested_model.clone()))
        .unwrap_or_else(|| "muse-video".to_string());

    if !SUPPORTED_MODELS.contains(&requested_model.as_str()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unsupported model '{requested_model}'; supported: {}",
                SUPPORTED_MODELS.join(", ")
            ),
        ));
    }
    let upstream_model = normalize_video_model(&requested_model).to_string();
    Ok((requested_model, upstream_model))
}

fn resolve_aspect_ratio(
    body: &serde_json::Value,
    existing_job: Option<&super::super::lifecycle::VideoJobRecord>,
) -> io::Result<String> {
    let aspect_ratio = body
        .get("aspect_ratio")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| existing_job.map(|j| j.aspect_ratio.clone()))
        .unwrap_or_else(|| "16:9".to_string());

    if !SUPPORTED_ASPECT_RATIOS.contains(&aspect_ratio.as_str()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unsupported aspect_ratio '{aspect_ratio}'; supported: {}",
                SUPPORTED_ASPECT_RATIOS.join(", ")
            ),
        ));
    }
    Ok(aspect_ratio)
}

fn resolve_duration(
    body: &serde_json::Value,
    existing_job: Option<&super::super::lifecycle::VideoJobRecord>,
) -> io::Result<u64> {
    match body.get("duration") {
        None => Ok(existing_job.map(|j| j.duration).unwrap_or(5)),
        Some(value) => {
            let parsed = value.as_u64().or_else(|| {
                value
                    .as_str()
                    .and_then(|text| text.strip_suffix('s').unwrap_or(text).parse::<u64>().ok())
            });
            parsed.filter(|seconds| *seconds > 0).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "duration must be a positive integer",
                )
            })
        }
    }
}

fn resolve_attachment_urls(
    attachments: &[VideoAttachment],
    existing_job: Option<&super::super::lifecycle::VideoJobRecord>,
) -> Vec<String> {
    let mut urls: Vec<String> = attachments
        .iter()
        .map(|a| a.as_reference().to_string())
        .collect();

    if urls.is_empty() {
        if let Some(parent) = existing_job {
            if !parent.attachment_urls.is_empty() {
                urls = parent.attachment_urls.clone();
            } else if let Some(ref single) = parent.attachment_url {
                urls = vec![single.clone()];
            }
        }
    }
    urls
}

fn build_full_prompt(
    prompt: &str,
    upstream_model: &str,
    aspect_ratio: &str,
    duration: u64,
    attachments: &[VideoAttachment],
    existing_job: Option<&super::super::lifecycle::VideoJobRecord>,
) -> String {
    let display_refs: Vec<String> = attachments
        .iter()
        .enumerate()
        .map(|(i, a)| match a {
            VideoAttachment::DataUri { .. } => format!("<attached image {}>", i + 1),
            _ => a.as_reference().to_string(),
        })
        .collect();

    if let Some(parent) = existing_job {
        if !display_refs.is_empty() {
            let refs = display_refs.join(", ");
            format!(
                "Edit this existing video (source: {}) with reference images ({refs}): \"{prompt}\". Return a public Google Drive link when ready.",
                parent.video_url
            )
        } else {
            format!(
                "Edit this existing video (source: {}): \"{prompt}\". Return a public Google Drive link when ready.",
                parent.video_url
            )
        }
    } else {
        build_video_prompt_with_attachments(
            upstream_model,
            prompt,
            aspect_ratio,
            duration,
            &display_refs,
        )
    }
}

fn build_attachment_item(i: usize, att: &VideoAttachment) -> serde_json::Value {
    match att {
        VideoAttachment::InternalImage { job_id, path } => {
            let filename = format!("{job_id}.png");
            let vm_path = format!("workspace/your_files/{filename}");
            serde_json::json!({
                "type": "file_ref",
                "mime_type": "image/png",
                "filename": filename,
                "path": vm_path,
                "internal_job_id": job_id,
                "internal_ref": path
            })
        }
        VideoAttachment::DataUri { mime_type, .. } => {
            let ext = if mime_type.contains("jpeg") || mime_type.contains("jpg") {
                "jpg"
            } else if mime_type.contains("webp") {
                "webp"
            } else {
                "png"
            };
            let filename = format!("upload_{}.{ext}", i + 1);
            let vm_path = format!("workspace/your_files/{filename}");
            serde_json::json!({
                "type": "file_ref",
                "mime_type": mime_type,
                "filename": filename,
                "path": vm_path,
                "data_uri": att.as_reference()
            })
        }
        VideoAttachment::ExternalUrl { url } => {
            let clean_url = url.split('?').next().unwrap_or(url);
            let filename = clean_url.rsplit('/').next().unwrap_or("input_image.png");
            let filename = if filename.is_empty() || !filename.contains('.') {
                "input_image.png"
            } else {
                filename
            };
            let vm_path = format!("workspace/your_files/{filename}");
            serde_json::json!({
                "type": "file_ref",
                "mime_type": "image/png",
                "filename": filename,
                "path": vm_path,
                "external_url": url
            })
        }
    }
}

fn build_request_payload(
    body: &serde_json::Value,
    full_prompt: &str,
    attachments: &[VideoAttachment],
    existing_job: Option<&super::super::lifecycle::VideoJobRecord>,
    request_body: &[u8],
) -> (String, serde_json::Value) {
    let allocated_job_id = body
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("video-{}", Uuid::new_v4()));

    let mut request_val = serde_json::json!({
        "model": "muse",
        "job_id": allocated_job_id,
        "messages": [
            {"role": "user", "content": full_prompt}
        ]
    });

    if !attachments.is_empty() {
        let items: Vec<serde_json::Value> = attachments
            .iter()
            .enumerate()
            .map(|(i, att)| build_attachment_item(i, att))
            .collect();
        request_val["attachment_items"] = serde_json::Value::Array(items);
    }

    if let Some(parent) = existing_job {
        if let Some(ref tid) = parent.thread_id {
            if !tid.is_empty() {
                request_val["thread_id"] = serde_json::Value::String(tid.clone());
            }
        }
    } else if let Ok(Some(tid)) = crate::providers::museai::chat::extract_thread_id(request_body) {
        request_val["thread_id"] = serde_json::Value::String(tid);
    }

    (allocated_job_id, request_val)
}

pub fn parse_and_validate_video_request(request_body: &[u8]) -> io::Result<ValidatedVideoRequest> {
    let body = parse_request_json(request_body)?;
    let prompt = extract_prompt(&body)?;
    let existing_job = load_parent_job(&body)?;
    let submission_key = extract_submission_key(&body);
    let (requested_model, upstream_model) = resolve_model(&body, existing_job.as_ref())?;
    let aspect_ratio = resolve_aspect_ratio(&body, existing_job.as_ref())?;
    let duration = resolve_duration(&body, existing_job.as_ref())?;

    let attachments = parse_and_validate_attachments(&body)?;
    let attachment_urls = resolve_attachment_urls(&attachments, existing_job.as_ref());
    let attachment_url = attachment_urls.first().cloned();
    let attachment = attachments.first().cloned();

    let full_prompt = build_full_prompt(
        &prompt,
        &upstream_model,
        &aspect_ratio,
        duration,
        &attachments,
        existing_job.as_ref(),
    );

    let (allocated_job_id, request_val) = build_request_payload(
        &body,
        &full_prompt,
        &attachments,
        existing_job.as_ref(),
        request_body,
    );

    let request_json = serde_json::to_vec(&request_val)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    Ok(ValidatedVideoRequest {
        allocated_job_id,
        prompt,
        upstream_model,
        requested_model,
        aspect_ratio,
        duration,
        existing_job,
        submission_key,
        attachment,
        attachment_url,
        attachments,
        attachment_urls,
        request_json,
        now,
    })
}

pub fn create_init_job_record(
    req: &ValidatedVideoRequest,
) -> super::super::lifecycle::VideoJobRecord {
    super::super::lifecycle::VideoJobRecord {
        job_id: req.allocated_job_id.clone(),
        owner_id: "default-owner".to_string(),
        model: req.upstream_model.to_string(),
        requested_model: req.requested_model.clone(),
        prompt: req.prompt.clone(),
        aspect_ratio: req.aspect_ratio.clone(),
        duration: req.duration,
        thread_id: None,
        created_at: req.now,
        last_activity_at: req.now,
        pinned: false,
        status: "submitting".to_string(),
        media_validated: false,
        output_verified: false,
        video_url: String::new(),
        revisions: Vec::new(),
        approval_status: None,
        approval_id: None,
        submission_key: req.submission_key.clone(),
        last_checked_at: Some(req.now),
        attachment_url: req.attachment_url.clone(),
        attachment_urls: req.attachment_urls.clone(),
        error: None,
    }
}
