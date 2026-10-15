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
        "Create a video from this description if video generation is available: \"{prompt}\". Return a public Google Drive link when ready."
    )
}

pub fn create_video(request_body: &[u8], config: &Config) -> io::Result<serde_json::Value> {
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

    let completion = request_museai_chat_completion(&request_json, config, true)?;

    let completion_json: serde_json::Value = serde_json::from_str(&completion)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let content = completion_json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    let video_url = extract_url_from_text(content);
    if video_url.is_empty() {
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

    build_video_result(
        upstream_model,
        requested_model,
        &prompt,
        aspect_ratio,
        duration,
        content,
    )
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

    let job_id = format!("video-{}", Uuid::new_v4());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let record = super::lifecycle::VideoJobRecord {
        job_id: job_id.clone(),
        owner_id: owner_id.to_string(),
        model: model.to_string(),
        requested_model: requested_model.to_string(),
        prompt: prompt.to_string(),
        aspect_ratio: aspect_ratio.to_string(),
        duration,
        thread_id: thread_id.clone(),
        created_at: now,
        last_activity_at: now,
        pinned: false,
        status: status.to_string(),
        media_validated,
        output_verified,
        video_url: video_url.clone(),
        revisions: Vec::new(),
    };
    if let Err(e) = super::lifecycle::save_video_job(&record) {
        super::lifecycle::debug_log_lifecycle(&format!("persistence skipped for {job_id}: {e}"));
    }

    Ok(serde_json::json!({
        "id": job_id,
        "object": "video.generation",
        "created": now,
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
