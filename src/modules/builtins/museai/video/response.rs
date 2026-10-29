//! Video job response formatting, state serialization, and result builders.

use super::media::has_video_media;
use super::url::{extract_url_from_text, is_vm_media_url};
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub(crate) fn video_job_response(
    existing: super::super::lifecycle::VideoJobRecord,
) -> io::Result<serde_json::Value> {
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
        let now = super::super::lifecycle::now_secs();
        if super::super::lifecycle::is_submitting_stale(&existing, now) {
            let mut updated = existing.clone();
            super::super::lifecycle::recover_stale_submitting_job(&mut updated, now);
            let _ = super::super::lifecycle::save_video_job(&updated);
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

    let _guard = super::super::lifecycle::job_update_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let created_at = super::super::lifecycle::load_video_job(&job_id)
        .map(|existing| existing.created_at)
        .unwrap_or(now);

    let mut record = super::super::lifecycle::VideoJobRecord {
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

    if let Ok(existing) = super::super::lifecycle::load_video_job(&job_id) {
        record.pinned = existing.pinned;
        record.revisions = existing.revisions;
        record.approval_status = existing.approval_status;
        record.approval_id = existing.approval_id;
        record.submission_key = existing.submission_key;
        record.prompt = prompt.to_string();
        record.status = status.to_string();
    }
    if let Err(e) = super::super::lifecycle::save_video_job(&record) {
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
