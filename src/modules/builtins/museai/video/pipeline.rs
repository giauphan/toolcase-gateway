//! Video generation execution pipeline, asynchronous submission, and error recovery.

use super::media::has_video_media;
use super::request::{
    create_init_job_record, parse_and_validate_video_request, ValidatedVideoRequest,
};
use super::response::{build_video_result_with_job_id, video_job_response};
use super::url::{extract_url_from_text, is_vm_media_url};
use crate::config::Config;
use crate::providers::museai::chat::request_museai_chat_completion;
use std::io;

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
    let req = parse_and_validate_video_request(request_body)?;

    if req.existing_job.is_none() {
        let init_record = create_init_job_record(&req);
        if let Some(existing) = super::super::lifecycle::reserve_video_job(&init_record)? {
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
    if let Some(existing) = super::super::lifecycle::reserve_video_job(&init_record)? {
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

fn handle_video_request_error(e: &io::Error, req: &ValidatedVideoRequest) -> io::Result<()> {
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
            .and_then(
                <dyn std::error::Error + Send + Sync>::downcast_ref::<
                    crate::providers::museai::MuseApprovalRequired,
                >,
            )
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
        if let Err(update_err) = super::super::lifecycle::update_video_job_with_error(
            &req.allocated_job_id,
            status,
            approval_status,
            None,
            Some(&err_msg),
        ) {
            super::super::lifecycle::debug_log_lifecycle(&format!(
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
    Ok(())
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
            handle_video_request_error(&e, &req)?;
            return Err(e);
        }
    };

    let completion_json: serde_json::Value = match serde_json::from_str(&completion) {
        Ok(json) => json,
        Err(e) => {
            if req.existing_job.is_none() {
                let err_msg = format!("Failed to parse completion JSON: {e}");
                if let Err(update_err) = super::super::lifecycle::update_video_job_with_error(
                    &req.allocated_job_id,
                    "failed",
                    None,
                    None,
                    Some(&err_msg),
                ) {
                    super::super::lifecycle::debug_log_lifecycle(&format!(
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
            if let Err(update_err) = super::super::lifecycle::update_video_job_with_error(
                &req.allocated_job_id,
                "failed",
                None,
                thread_id.as_deref(),
                Some(&err_msg),
            ) {
                super::super::lifecycle::debug_log_lifecycle(&format!(
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
        let updated = super::super::lifecycle::append_job_revision(
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
