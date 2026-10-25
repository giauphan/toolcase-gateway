use crate::config::Config;
use crate::providers::museai::chat::request_museai_chat_completion;
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

mod media;
pub use media::*;
mod url;
pub use url::*;

pub fn create_video(request_body: &[u8], config: &Config) -> io::Result<serde_json::Value> {
    create_video_with_completion(request_body, config, request_museai_chat_completion)
}

pub struct ValidatedVideoRequest {
    pub allocated_job_id: String,
    pub prompt: String,
    pub upstream_model: String,
    pub requested_model: String,
    pub aspect_ratio: String,
    pub duration: u64,
    pub existing_job: Option<super::lifecycle::VideoJobRecord>,
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
        Some(id) => Some(super::lifecycle::load_video_job(id)?),
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

fn create_init_job_record(req: &ValidatedVideoRequest) -> super::lifecycle::VideoJobRecord {
    super::lifecycle::VideoJobRecord {
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

fn finish_video_request<F>(
    req: ValidatedVideoRequest,
    config: &Config,
    request_completion: F,
) -> io::Result<serde_json::Value>
where
    F: Fn(&[u8], &Config, bool) -> io::Result<String>,
{
    let completion = match request_completion(&req.request_json, config, true) {
        Ok(c) => c,
        Err(e) => {
            crate::runtime::logger::log_message(
                "ERROR",
                "museai_video",
                &format!(
                    "Video generation request failed for job {}: {e}",
                    req.allocated_job_id
                ),
            );
            if req.existing_job.is_none() {
                let (status, approval_status) = if e
                    .get_ref()
                    .and_then(|err| {
                        err.downcast_ref::<crate::providers::museai::MuseApprovalRequired>()
                    })
                    .is_some()
                {
                    ("approval_required", Some("manual_required"))
                } else if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::UnexpectedEof
                ) {
                    ("submission_unknown", None)
                } else {
                    ("failed", None)
                };
                let err_msg = e.to_string();
                if let Err(update_err) = super::lifecycle::update_video_job_with_error(
                    &req.allocated_job_id,
                    status,
                    approval_status,
                    None,
                    Some(&err_msg),
                ) {
                    super::lifecycle::debug_log_lifecycle(&format!(
                        "Failed to update job status after completion error: {update_err}"
                    ));
                    return Err(io::Error::new(
                        update_err.kind(),
                        format!(
                            "failed to update video job status to '{status}': {update_err} (completion error: {e})"
                        ),
                    ));
                }
            }
            return Err(e);
        }
    };

    let completion_json: serde_json::Value = match serde_json::from_str(&completion) {
        Ok(json) => json,
        Err(e) => {
            if req.existing_job.is_none() {
                let err_msg = format!("Failed to parse completion JSON: {e}");
                if let Err(update_err) = super::lifecycle::update_video_job_with_error(
                    &req.allocated_job_id,
                    "failed",
                    None,
                    None,
                    Some(&err_msg),
                ) {
                    super::lifecycle::debug_log_lifecycle(&format!(
                        "Failed to update job status after JSON parse error: {update_err}"
                    ));
                    return Err(io::Error::new(
                        update_err.kind(),
                        format!(
                            "failed to update video job status to 'failed': {update_err} (parse error: {e})"
                        ),
                    ));
                }
            }
            return Err(io::Error::new(io::ErrorKind::InvalidData, e));
        }
    };

    let content = completion_json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    let thread_id = completion_json
        .get("thread_id")
        .or_else(|| completion_json.get("session_id"))
        .and_then(|t| t.as_str())
        .map(str::to_string);

    let raw_video_url = extract_url_from_text(content);
    if raw_video_url.is_empty() {
        if req.existing_job.is_none() {
            let err_msg = if crate::providers::museai::chat::explicit_video_refusal(content) {
                format!("Muse video generation was declined: {content}")
            } else {
                "Muse video generation completed without a supported public HTTPS artifact"
                    .to_string()
            };
            if let Err(update_err) = super::lifecycle::update_video_job_with_error(
                &req.allocated_job_id,
                "failed",
                None,
                thread_id.as_deref(),
                Some(&err_msg),
            ) {
                super::lifecycle::debug_log_lifecycle(&format!(
                    "Failed to update job status after empty video url: {update_err}"
                ));
                return Err(io::Error::new(
                    update_err.kind(),
                    format!(
                        "failed to update video job status to 'failed': {update_err} (missing video url)"
                    ),
                ));
            }
        }
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Muse video generation completed without a supported public HTTPS artifact",
        ));
    }

    let is_vm = is_vm_media_url(&raw_video_url);
    let (video_url, media_validated) = if is_vm {
        let content_url = format!("/muse-ai/v1/videos/{}/content", req.allocated_job_id);
        let validated = has_video_media(&req.allocated_job_id);
        (content_url, validated)
    } else {
        (raw_video_url, false)
    };
    let rev_status = if media_validated {
        "completed"
    } else {
        "artifact_reported"
    };

    crate::runtime::logger::log_message(
        "INFO",
        "museai_video",
        &format!(
            "Video generation pipeline completed for job {} with URL: {} (status={})",
            req.allocated_job_id, video_url, rev_status
        ),
    );

    if let Some(parent) = req.existing_job {
        let updated = super::lifecycle::append_job_revision(
            &parent.job_id,
            &req.prompt,
            &video_url,
            rev_status,
        )?;
        let rev_id = updated
            .revisions
            .last()
            .map(|r| r.revision_id.clone())
            .unwrap_or_else(|| "rev-1".to_string());
        return Ok(serde_json::json!({
            "id": updated.job_id,
            "object": "video.generation",
            "created": updated.last_activity_at,
            "provider": "muse_chat",
            "resolved_provider": "muse_chat",
            "requested_model": updated.requested_model,
            "resolved_model": updated.model,
            "model": updated.model,
            "prompt": req.prompt,
            "requested_aspect_ratio": updated.aspect_ratio,
            "requested_duration": updated.duration,
            "status": updated.status,
            "provider_status": "artifact_url_reported",
            "output_verified": updated.output_verified,
            "media_validated": updated.media_validated,
            "video_url": updated.video_url,
            "revision_id": rev_id,
            "revisions": updated.revisions
        }));
    }

    build_video_result_with_job_id(
        req.allocated_job_id,
        &req.upstream_model,
        &req.requested_model,
        &req.prompt,
        &req.aspect_ratio,
        req.duration,
        content,
        "default-owner",
        thread_id,
        req.submission_key,
    )
}

pub fn create_video_with_completion<F>(
    request_body: &[u8],
    config: &Config,
    request_completion: F,
) -> io::Result<serde_json::Value>
where
    F: Fn(&[u8], &Config, bool) -> io::Result<String>,
{
    let req = parse_and_validate_video_request(request_body)?;

    if req.existing_job.is_none() {
        let init_record = create_init_job_record(&req);
        if let Some(existing) = super::lifecycle::reserve_video_job(&init_record)? {
            return video_job_response(existing);
        }
    }

    finish_video_request(req, config, request_completion)
}

pub fn submit_video(request_body: &[u8], config: &Config) -> io::Result<serde_json::Value> {
    submit_video_with_completion(request_body, config, request_museai_chat_completion)
}

pub fn submit_video_with_completion<F>(
    request_body: &[u8],
    config: &Config,
    request_completion: F,
) -> io::Result<serde_json::Value>
where
    F: Fn(&[u8], &Config, bool) -> io::Result<String> + Send + Sync + 'static,
{
    let req = parse_and_validate_video_request(request_body)?;

    if req.existing_job.is_some() {
        return finish_video_request(req, config, request_completion);
    }

    let init_record = create_init_job_record(&req);
    if let Some(existing) = super::lifecycle::reserve_video_job(&init_record)? {
        return video_job_response(existing);
    }

    crate::runtime::logger::log_message(
        "INFO",
        "museai_video",
        &format!(
            "Reserved and submitted video job {} for prompt '{}' (model={}, ratio={}, duration={}s)",
            req.allocated_job_id, req.prompt, req.requested_model, req.aspect_ratio, req.duration
        ),
    );

    let response = serde_json::json!({
        "id": req.allocated_job_id,
        "object": "video.generation",
        "created": req.now,
        "provider": "muse_chat",
        "resolved_provider": "muse_chat",
        "requested_model": req.requested_model,
        "resolved_model": req.upstream_model,
        "model": req.upstream_model,
        "prompt": req.prompt,
        "requested_aspect_ratio": req.aspect_ratio,
        "requested_duration": req.duration,
        "status": "submitting",
        "approval_status": serde_json::Value::Null,
        "approval_id": serde_json::Value::Null,
        "video_url": "",
        "revisions": []
    });

    let config_clone = config.clone();
    std::thread::spawn(move || {
        let _ = finish_video_request(req, &config_clone, request_completion);
    });

    Ok(response)
}

fn video_job_response(existing: super::lifecycle::VideoJobRecord) -> io::Result<serde_json::Value> {
    if existing.status == "approval_required" {
        return Err(io::Error::other(crate::providers::museai::MuseApprovalRequired(
            format!(
                "The current Muse thread '{}' is awaiting a user decision; do not resubmit the video prompt.",
                existing.thread_id.as_deref().unwrap_or("unknown")
            ),
        )));
    }

    if existing.status == "artifact_reported" || existing.status == "completed" {
        let rev_id = existing
            .revisions
            .last()
            .map(|r| r.revision_id.clone())
            .unwrap_or_else(|| "rev-1".to_string());
        return Ok(serde_json::json!({
            "id": existing.job_id,
            "object": "video.generation",
            "created": existing.created_at,
            "provider": "muse_chat",
            "resolved_provider": "muse_chat",
            "requested_model": existing.requested_model,
            "resolved_model": existing.model,
            "model": existing.model,
            "prompt": existing.prompt,
            "requested_aspect_ratio": existing.aspect_ratio,
            "requested_duration": existing.duration,
            "status": existing.status,
            "approval_status": existing.approval_status,
            "approval_id": existing.approval_id,
            "provider_status": "artifact_url_reported",
            "output_verified": existing.output_verified,
            "media_validated": existing.media_validated,
            "video_url": existing.video_url,
            "revision_id": rev_id,
            "revisions": existing.revisions
        }));
    }

    if existing.status == "submitting" {
        let now = super::lifecycle::now_secs();
        if super::lifecycle::is_submitting_stale(&existing, now) {
            let mut updated = existing.clone();
            super::lifecycle::recover_stale_submitting_job(&mut updated, now);
            let _ = super::lifecycle::save_video_job(&updated);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "video generation submission timed out (status=submission_unknown, job={}); retryable",
                    updated.job_id
                ),
            ));
        }
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            format!("video generation in progress (status={})", existing.status),
        ));
    }

    if existing.status == "submission_unknown" {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "video generation submission status is unknown (job_id={}); previous submission timed out or interrupted, retryable",
                existing.job_id
            ),
        ));
    }

    if existing.status == "failed" {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "video job previously failed; retry should create a new job",
        ));
    }

    Ok(serde_json::json!({
        "id": existing.job_id,
        "object": "video.generation",
        "created": existing.created_at,
        "provider": "muse_chat",
        "resolved_provider": "muse_chat",
        "requested_model": existing.requested_model,
        "resolved_model": existing.model,
        "model": existing.model,
        "prompt": existing.prompt,
        "requested_aspect_ratio": existing.aspect_ratio,
        "requested_duration": existing.duration,
        "status": existing.status,
        "approval_status": existing.approval_status,
        "approval_id": existing.approval_id,
        "video_url": "",
        "revisions": existing.revisions
    }))
}

pub fn build_video_result(
    model: &str,
    requested_model: &str,
    prompt: &str,
    aspect_ratio: &str,
    duration: u64,
    content: &str,
) -> io::Result<serde_json::Value> {
    build_video_result_with_owner(
        model,
        requested_model,
        prompt,
        aspect_ratio,
        duration,
        content,
        "default-owner",
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_video_result_with_owner(
    model: &str,
    requested_model: &str,
    prompt: &str,
    aspect_ratio: &str,
    duration: u64,
    content: &str,
    owner_id: &str,
    thread_id: Option<String>,
) -> io::Result<serde_json::Value> {
    let job_id = format!("video-{}", Uuid::new_v4());
    build_video_result_with_job_id(
        job_id,
        model,
        requested_model,
        prompt,
        aspect_ratio,
        duration,
        content,
        owner_id,
        thread_id,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_video_result_with_job_id(
    job_id: String,
    model: &str,
    requested_model: &str,
    prompt: &str,
    aspect_ratio: &str,
    duration: u64,
    content: &str,
    owner_id: &str,
    thread_id: Option<String>,
    submission_key: Option<String>,
) -> io::Result<serde_json::Value> {
    let raw_video_url = extract_url_from_text(content);
    if raw_video_url.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Muse video generation completed without a supported public HTTPS artifact",
        ));
    }

    let is_vm = is_vm_media_url(&raw_video_url);
    let (video_url, media_validated) = if is_vm {
        let content_url = format!("/muse-ai/v1/videos/{job_id}/content");
        let validated = has_video_media(&job_id);
        (content_url, validated)
    } else {
        (raw_video_url, false)
    };

    let output_verified = media_validated;
    let status = if media_validated {
        "completed"
    } else {
        "artifact_reported"
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let _guard = super::lifecycle::job_update_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let created_at = super::lifecycle::load_video_job(&job_id)
        .map(|existing| existing.created_at)
        .unwrap_or(now);

    let mut record = super::lifecycle::VideoJobRecord {
        job_id: job_id.clone(),
        owner_id: owner_id.to_string(),
        model: model.to_string(),
        requested_model: requested_model.to_string(),
        prompt: prompt.to_string(),
        aspect_ratio: aspect_ratio.to_string(),
        duration,
        thread_id,
        created_at,
        last_activity_at: now,
        pinned: false,
        status: status.to_string(),
        media_validated,
        output_verified,
        video_url: video_url.clone(),
        revisions: Vec::new(),
        approval_status: None,
        approval_id: None,
        submission_key,
        last_checked_at: Some(now),
        error: None,
    };

    if let Ok(existing) = super::lifecycle::load_video_job(&job_id) {
        record.pinned = existing.pinned;
        record.revisions = existing.revisions;
        record.approval_status = existing.approval_status;
        record.approval_id = existing.approval_id;
        record.submission_key = existing.submission_key;
        record.prompt = prompt.to_string();
        record.status = status.to_string();
    }
    if let Err(e) = super::lifecycle::save_video_job(&record) {
        crate::runtime::logger::log_message(
            "ERROR",
            "museai_video",
            &format!("Failed to persist video job {job_id}: {e}"),
        );
    }

    Ok(serde_json::json!({
        "id": job_id,
        "object": "video.generation",
        "created": created_at,
        "provider": "muse_chat",
        "resolved_provider": "muse_chat",
        "requested_model": requested_model,
        "resolved_model": model,
        "model": model,
        "prompt": prompt,
        "requested_aspect_ratio": aspect_ratio,
        "requested_duration": duration,
        "status": status,
        "provider_status": "artifact_url_reported",
        "output_verified": output_verified,
        "media_validated": media_validated,
        "video_url": video_url
    }))
}
