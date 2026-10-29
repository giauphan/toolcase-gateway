//! Video generation request parsing, validation, and initialization.

use super::url::{
    build_video_prompt, normalize_video_model, SUPPORTED_ASPECT_RATIOS, SUPPORTED_MODELS,
};
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub struct ValidatedVideoRequest {
    pub allocated_job_id: String,
    pub prompt: String,
    pub upstream_model: String,
    pub requested_model: String,
    pub aspect_ratio: String,
    pub duration: u64,
    pub existing_job: Option<super::super::lifecycle::VideoJobRecord>,
    pub submission_key: Option<String>,
    pub request_json: Vec<u8>,
    pub now: u64,
}

pub fn parse_and_validate_video_request(request_body: &[u8]) -> io::Result<ValidatedVideoRequest> {
    let body: serde_json::Value = if request_body.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(request_body).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid JSON: {e}"))
        })?
    };

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

    let parent_job_id = body
        .get("job_id")
        .or_else(|| body.get("parent_id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let existing_job = match parent_job_id {
        Some(id) => Some(super::super::lifecycle::load_video_job(id)?),
        None => None,
    };

    let submission_key = body
        .get("idempotency_key")
        .or_else(|| body.get("submission_key"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let requested_model = body
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| existing_job.as_ref().map(|j| j.requested_model.clone()))
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

    let aspect_ratio = body
        .get("aspect_ratio")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| existing_job.as_ref().map(|j| j.aspect_ratio.clone()))
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

    let duration = match body.get("duration") {
        None => existing_job.as_ref().map(|j| j.duration).unwrap_or(5),
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
            })?
        }
    };

    let full_prompt = if let Some(ref parent) = existing_job {
        format!(
            "Edit this existing video (source: {}): \"{prompt}\". Return a public Google Drive link when ready.",
            parent.video_url
        )
    } else {
        build_video_prompt(&upstream_model, &prompt, &aspect_ratio, duration)
    };

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
    if let Some(ref parent) = existing_job {
        if let Some(ref tid) = parent.thread_id {
            if !tid.is_empty() {
                request_val["thread_id"] = serde_json::Value::String(tid.clone());
            }
        }
    } else if let Ok(Some(tid)) = crate::providers::museai::chat::extract_thread_id(request_body) {
        request_val["thread_id"] = serde_json::Value::String(tid);
    }

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
        request_json,
        now,
    })
}

pub(crate) fn create_init_job_record(
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
        error: None,
    }
}
