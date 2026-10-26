//! Muse image generation application service.

use crate::config::Config;
use crate::providers::museai::chat::request_museai_chat_completion;
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use super::lifecycle::{
    is_valid_job_id, load_image_job, reserve_image_job, update_image_job, ImageJobRecord,
};
use super::url::{
    build_image_prompt, extract_image_url_from_text, map_size_to_aspect_ratio,
    normalize_image_model, DEFAULT_IMAGE_ASPECT_RATIO, SUPPORTED_IMAGE_ASPECT_RATIOS,
};

#[derive(Debug)]
pub struct ValidatedImageRequest {
    pub allocated_job_id: String,
    pub prompt: String,
    pub upstream_model: String,
    pub requested_model: String,
    pub aspect_ratio: String,
    pub existing_job: Option<ImageJobRecord>,
    pub submission_key: Option<String>,
    pub request_json: Vec<u8>,
    pub now: u64,
}

pub fn parse_and_validate_image_request(request_body: &[u8]) -> io::Result<ValidatedImageRequest> {
    let body: serde_json::Value = if request_body.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "request body cannot be empty",
        ));
    } else {
        serde_json::from_slice(request_body)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
    };

    let prompt = body
        .get("prompt")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "prompt is required and must not be empty",
            )
        })?
        .to_string();

    let raw_ratio = if let Some(ratio_val) = body.get("aspect_ratio").and_then(|v| v.as_str()) {
        let trimmed = ratio_val.trim();
        if !SUPPORTED_IMAGE_ASPECT_RATIOS.contains(&trimmed) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "unsupported aspect_ratio '{trimmed}'; supported: {}",
                    SUPPORTED_IMAGE_ASPECT_RATIOS.join(", ")
                ),
            ));
        }
        trimmed
    } else if let Some(size_val) = body.get("size").and_then(|v| v.as_str()) {
        let trimmed = size_val.trim();
        map_size_to_aspect_ratio(trimmed).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported size '{trimmed}' for image aspect ratio"),
            )
        })?
    } else {
        DEFAULT_IMAGE_ASPECT_RATIO
    };

    let aspect_ratio = raw_ratio.to_string();

    let requested_model = body
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("muse-image")
        .to_string();
    let upstream_model = normalize_image_model(&requested_model).to_string();

    let submission_key = body
        .get("submission_key")
        .or_else(|| body.get("idempotency_key"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.len() > 256 {
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "submission_key exceeds maximum length of 256",
                ))
            } else {
                Ok(s.to_string())
            }
        })
        .transpose()?;

    let allocated_job_id = if let Some(id_val) = body.get("id").and_then(|v| v.as_str()) {
        let trimmed = id_val.trim();
        if !is_valid_job_id(trimmed) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid 'id' parameter; must contain only alphanumeric characters, '-', or '_', max 128 chars",
            ));
        }
        trimmed.to_string()
    } else {
        format!("image-{}", Uuid::new_v4())
    };

    let full_prompt = build_image_prompt(&upstream_model, &prompt, &aspect_ratio);

    let mut request_val = serde_json::json!({
        "model": "muse",
        "job_id": allocated_job_id,
        "messages": [
            {"role": "user", "content": full_prompt}
        ]
    });

    if let Ok(Some(tid)) = crate::providers::museai::chat::extract_thread_id(request_body) {
        request_val["thread_id"] = serde_json::Value::String(tid);
    }

    let request_json = serde_json::to_vec(&request_val)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let existing_job = load_image_job(&allocated_job_id).ok();

    Ok(ValidatedImageRequest {
        allocated_job_id,
        prompt,
        upstream_model,
        requested_model,
        aspect_ratio,
        existing_job,
        submission_key,
        request_json,
        now,
    })
}

fn create_init_job_record(req: &ValidatedImageRequest) -> ImageJobRecord {
    ImageJobRecord {
        job_id: req.allocated_job_id.clone(),
        owner_id: "default-owner".to_string(),
        model: req.upstream_model.clone(),
        requested_model: req.requested_model.clone(),
        prompt: req.prompt.clone(),
        aspect_ratio: req.aspect_ratio.clone(),
        thread_id: None,
        created_at: req.now,
        last_activity_at: req.now,
        status: "submitting".to_string(),
        media_validated: false,
        output_verified: false,
        image_url: String::new(),
        submission_key: req.submission_key.clone(),
        error: None,
    }
}

pub fn create_image(request_body: &[u8], config: &Config) -> io::Result<serde_json::Value> {
    create_image_with_completion(request_body, config, request_museai_chat_completion)
}

pub fn create_image_with_completion<F>(
    request_body: &[u8],
    config: &Config,
    request_completion: F,
) -> io::Result<serde_json::Value>
where
    F: Fn(&[u8], &Config, bool) -> io::Result<String>,
{
    let req = parse_and_validate_image_request(request_body)?;

    if let Some(existing) = &req.existing_job {
        if existing.status == "completed" && !existing.image_url.is_empty() {
            return Ok(image_job_response(existing));
        }
    }

    let init_record = create_init_job_record(&req);
    if let Some(existing) = reserve_image_job(&init_record)? {
        if existing.status == "completed" && !existing.image_url.is_empty() {
            return Ok(image_job_response(&existing));
        }
    }

    finish_image_request(req, config, request_completion)
}

pub fn submit_image(request_body: &[u8], config: &Config) -> io::Result<serde_json::Value> {
    submit_image_with_completion(request_body, config, request_museai_chat_completion)
}

pub fn submit_image_with_completion<F>(
    request_body: &[u8],
    config: &Config,
    request_completion: F,
) -> io::Result<serde_json::Value>
where
    F: Fn(&[u8], &Config, bool) -> io::Result<String> + Send + Sync + 'static,
{
    let req = parse_and_validate_image_request(request_body)?;

    if let Some(existing) = &req.existing_job {
        if existing.status == "completed" && !existing.image_url.is_empty() {
            return Ok(image_job_response(existing));
        }
    }

    let init_record = create_init_job_record(&req);
    if let Some(existing) = reserve_image_job(&init_record)? {
        if existing.status == "completed" && !existing.image_url.is_empty() {
            return Ok(image_job_response(&existing));
        }
    }

    crate::runtime::logger::log_message(
        "INFO",
        "museai_image",
        &format!(
            "Reserved and submitted image job {} for prompt '{}' (ratio={})",
            req.allocated_job_id, req.prompt, req.aspect_ratio
        ),
    );

    let response = serde_json::json!({
        "id": req.allocated_job_id,
        "object": "image.generation",
        "created": req.now,
        "provider": "muse_chat",
        "resolved_provider": "muse_chat",
        "requested_model": req.requested_model,
        "resolved_model": req.upstream_model,
        "model": req.upstream_model,
        "prompt": req.prompt,
        "aspect_ratio": req.aspect_ratio,
        "requested_aspect_ratio": req.aspect_ratio,
        "status": "submitting",
        "image_url": "",
        "data": []
    });

    let config_clone = config.clone();
    std::thread::spawn(move || {
        let _ = finish_image_request(req, &config_clone, request_completion);
    });

    Ok(response)
}

fn finish_image_request<F>(
    req: ValidatedImageRequest,
    config: &Config,
    request_completion: F,
) -> io::Result<serde_json::Value>
where
    F: Fn(&[u8], &Config, bool) -> io::Result<String>,
{
    let completion = match request_completion(&req.request_json, config, false) {
        Ok(c) => c,
        Err(e) => {
            crate::runtime::logger::log_message(
                "ERROR",
                "museai_image",
                &format!(
                    "Image generation request failed for job {}: {e}",
                    req.allocated_job_id
                ),
            );
            let status = if e
                .get_ref()
                .and_then(|err| {
                    err.downcast_ref::<crate::providers::museai::MuseApprovalRequired>()
                })
                .is_some()
            {
                "approval_required"
            } else {
                "failed"
            };
            let err_msg = e.to_string();
            let _ = update_image_job(&req.allocated_job_id, status, None, Some(&err_msg));
            return Err(e);
        }
    };

    let raw_image_url = extract_image_url_from_text(&completion);
    if raw_image_url.is_empty() {
        crate::runtime::logger::log_message(
            "WARN",
            "museai_image",
            &format!(
                "Image generation returned no URL for job {}. Raw response snippet: {}",
                req.allocated_job_id,
                &completion[..completion.len().min(500)]
            ),
        );
        let _ = update_image_job(
            &req.allocated_job_id,
            "failed",
            None,
            Some("Muse image generation completed without a supported public HTTPS artifact"),
        );
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Muse image generation completed without a supported public HTTPS artifact",
        ));
    }

    let is_vm = crate::providers::museai::video::is_vm_media_url(&raw_image_url);
    let (image_url, media_validated) = if is_vm {
        let content_url = format!("/muse-ai/v1/images/{}/content", req.allocated_job_id);
        let validated = super::media::has_image_media(&req.allocated_job_id);
        (content_url, validated)
    } else {
        (raw_image_url.clone(), true)
    };

    let _ = update_image_job(&req.allocated_job_id, "completed", Some(&image_url), None);

    let record = load_image_job(&req.allocated_job_id).unwrap_or_else(|_| ImageJobRecord {
        job_id: req.allocated_job_id.clone(),
        owner_id: "default-owner".to_string(),
        model: req.upstream_model.clone(),
        requested_model: req.requested_model.clone(),
        prompt: req.prompt.clone(),
        aspect_ratio: req.aspect_ratio.clone(),
        thread_id: None,
        created_at: req.now,
        last_activity_at: req.now,
        status: "completed".to_string(),
        media_validated,
        output_verified: true,
        image_url: image_url.clone(),
        submission_key: req.submission_key.clone(),
        error: None,
    });

    Ok(image_job_response(&record))
}

pub fn image_job_response(record: &ImageJobRecord) -> serde_json::Value {
    let mut data = Vec::new();
    if !record.image_url.is_empty() {
        data.push(serde_json::json!({
            "url": record.image_url
        }));
    }

    serde_json::json!({
        "id": record.job_id,
        "object": "image.generation",
        "created": record.created_at,
        "provider": "muse_chat",
        "resolved_provider": "muse_chat",
        "requested_model": record.requested_model,
        "resolved_model": record.model,
        "model": record.model,
        "prompt": record.prompt,
        "aspect_ratio": record.aspect_ratio,
        "requested_aspect_ratio": record.aspect_ratio,
        "status": record.status,
        "output_verified": record.output_verified,
        "media_validated": record.media_validated,
        "image_url": record.image_url,
        "data": data
    })
}
