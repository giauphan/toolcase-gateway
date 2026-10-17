use crate::config::Config;
use crate::providers::museai::chat::request_museai_chat_completion;
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const SUPPORTED_MODELS: [&str; 10] = [
    "muse",
    "muse-video",
    "gen-3",
    "gen-2",
    "kling",
    "gen-4",
    "gen-4.5",
    "gen-4-turbo",
    "aleph-2.0",
    "ruby",
];
pub const SUPPORTED_ASPECT_RATIOS: [&str; 5] = ["16:9", "9:16", "1:1", "5:4", "4:3"];

pub fn is_muse_video_alias(model: &str) -> bool {
    matches!(
        model,
        "gen-3" | "gen-2" | "kling" | "gen-4" | "gen-4.5" | "gen-4-turbo" | "aleph-2.0" | "ruby"
    )
}

pub fn build_video_prompt(model: &str, prompt: &str, aspect_ratio: &str, duration: u64) -> String {
    let _ = (model, aspect_ratio, duration);
    format!(
        "Create a video from this description if video generation is available: \"{prompt}\". Return a video artifact link when ready. Do not change sharing permissions."
    )
}

pub fn create_video(request_body: &[u8], config: &Config) -> io::Result<serde_json::Value> {
    create_video_with_completion(request_body, config, request_museai_chat_completion)
}

pub fn create_video_with_completion<F>(
    request_body: &[u8],
    config: &Config,
    request_completion: F,
) -> io::Result<serde_json::Value>
where
    F: Fn(&[u8], &Config, bool) -> io::Result<String>,
{
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
        .filter(|s| !s.is_empty());

    let requested_model = body
        .get("model")
        .and_then(|v| v.as_str())
        .or_else(|| existing_job.as_ref().map(|j| j.requested_model.as_str()))
        .unwrap_or("muse-video");
    if !SUPPORTED_MODELS.contains(&requested_model) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unsupported model '{requested_model}'; supported: {}",
                SUPPORTED_MODELS.join(", ")
            ),
        ));
    }
    let upstream_model = normalize_video_model(requested_model);

    let aspect_ratio = body
        .get("aspect_ratio")
        .and_then(|v| v.as_str())
        .or_else(|| existing_job.as_ref().map(|j| j.aspect_ratio.as_str()))
        .unwrap_or("16:9");
    if !SUPPORTED_ASPECT_RATIOS.contains(&aspect_ratio) {
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
        build_video_prompt(upstream_model, &prompt, aspect_ratio, duration)
    };

    let request_json = serde_json::to_vec(&serde_json::json!({
        "model": "muse",
        "messages": [
            {"role": "user", "content": full_prompt}
        ]
    }))
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let allocated_job_id = body
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("video-{}", Uuid::new_v4()));

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if existing_job.is_none() {
        let init_record = super::lifecycle::VideoJobRecord {
            job_id: allocated_job_id.clone(),
            owner_id: "default-owner".to_string(),
            model: upstream_model.to_string(),
            requested_model: requested_model.to_string(),
            prompt: prompt.clone(),
            aspect_ratio: aspect_ratio.to_string(),
            duration,
            thread_id: None,
            created_at: now,
            last_activity_at: now,
            pinned: false,
            status: "submitting".to_string(),
            media_validated: false,
            output_verified: false,
            video_url: String::new(),
            revisions: Vec::new(),
            approval_status: None,
            approval_id: None,
            submission_key: submission_key.map(str::to_string),
            last_checked_at: Some(now),
        };

        if let Some(existing) = super::lifecycle::reserve_video_job(&init_record)? {
            return video_job_response(existing);
        }
    }

    let completion = match request_completion(&request_json, config, true) {
        Ok(c) => c,
        Err(e) => {
            if existing_job.is_none() {
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
                if let Err(update_err) = super::lifecycle::update_video_job(
                    &allocated_job_id,
                    status,
                    approval_status,
                    None,
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
            if existing_job.is_none() {
                if let Err(update_err) =
                    super::lifecycle::update_video_job(&allocated_job_id, "failed", None, None)
                {
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

    let video_url = extract_url_from_text(content);
    if video_url.is_empty() {
        if existing_job.is_none() {
            if let Err(update_err) = super::lifecycle::update_video_job(
                &allocated_job_id,
                "failed",
                None,
                thread_id.as_deref(),
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

    if let Some(parent) = existing_job {
        let updated = super::lifecycle::append_job_revision(
            &parent.job_id,
            &prompt,
            &video_url,
            "artifact_reported",
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
            "prompt": prompt,
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
        allocated_job_id,
        upstream_model,
        requested_model,
        &prompt,
        aspect_ratio,
        duration,
        content,
        "default-owner",
        thread_id,
        submission_key.map(str::to_string),
    )
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
    let video_url = extract_url_from_text(content);
    if video_url.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Muse video generation completed without a supported public HTTPS artifact",
        ));
    }

    let output_verified = false;
    let media_validated = false;
    let status = if media_validated {
        "completed"
    } else {
        "artifact_reported"
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let created_at = super::lifecycle::load_video_job(&job_id)
        .map(|existing| existing.created_at)
        .unwrap_or(now);

    let record = super::lifecycle::VideoJobRecord {
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
    };
    if let Err(e) = super::lifecycle::save_video_job(&record) {
        eprintln!("[museai_video] failed to persist video job {job_id}: {e}");
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

pub fn extract_url_from_text(text: &str) -> String {
    let urls: Vec<&str> = text
        .split_whitespace()
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
        .collect();
    for url in &urls {
        let cleaned = trim_trailing_punct(url);
        if is_google_drive_file_path(cleaned) {
            return cleaned.to_string();
        }
    }
    for url in &urls {
        let cleaned = trim_trailing_punct(url);
        if is_supported_video_url(cleaned) {
            return cleaned.to_string();
        }
    }
    String::new()
}

pub fn is_supported_video_url(url: &str) -> bool {
    url.starts_with("https://")
        && (is_muse_files_url(url)
            || (is_google_drive_host(url) && is_google_drive_file_path(url))
            || (has_supported_video_extension(url) && !is_spoofed_host(url)))
}

pub fn is_muse_files_url(url: &str) -> bool {
    url.starts_with("https://")
        && url_authority(url) == "muse.ai"
        && url
            .strip_prefix("https://")
            .and_then(|rest| rest.split_once('/').map(|(_, path)| path))
            .is_some_and(|path| path.starts_with("files/") && has_supported_video_extension(url))
}

fn is_spoofed_host(url: &str) -> bool {
    let authority = url_authority(url);
    let dots = authority.matches('.').count();
    let base = authority
        .split('.')
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .join(".");
    base == "ai" || dots != 2
}

pub fn has_supported_video_extension(url: &str) -> bool {
    let path = url
        .strip_prefix("https://")
        .and_then(|rest| rest.split_once('/').map(|(_, path)| path))
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    path.ends_with(".mp4") || path.ends_with(".mov") || path.ends_with(".webm")
}

pub fn is_google_drive_host(url: &str) -> bool {
    let authority = url_authority(url);
    matches!(authority, "drive.google.com" | "docs.google.com")
}

pub fn is_google_drive_file_path(url: &str) -> bool {
    url.starts_with("https://")
        && is_google_drive_host(url)
        && url
            .strip_prefix("https://")
            .and_then(|rest| rest.split_once('/').map(|(_, path)| path))
            .is_some_and(|path| path.starts_with("file/") || path.starts_with("uc?"))
}

pub fn url_authority(url: &str) -> &str {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .and_then(|host| host.split('?').next())
        .and_then(|host| host.split('#').next())
        .unwrap_or("")
}

pub fn trim_trailing_punct(s: &str) -> &str {
    s.trim_matches([
        '"', '\'', ',', ';', ')', '}', '(', '{', ']', '[', '\\', '\n', '\r', ' ',
    ])
}

pub fn normalize_video_model(model: &str) -> &str {
    match model {
        "muse-video" | "muse" => "muse-video",
        "gen-3" | "gen-2" | "kling" | "gen-4" | "gen-4.5" | "gen-4-turbo" | "aleph-2.0"
        | "ruby" => "muse-video",
        _ => model,
    }
}
