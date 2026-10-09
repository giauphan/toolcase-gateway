use crate::config::Config;
use crate::museai_chat::request_museai_chat_completion;
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub(crate) const SUPPORTED_MODELS: [&str; 10] = [
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
pub(crate) const SUPPORTED_ASPECT_RATIOS: [&str; 5] = ["16:9", "9:16", "1:1", "5:4", "4:3"];

pub(crate) fn build_video_prompt(
    model: &str,
    prompt: &str,
    aspect_ratio: &str,
    duration: u64,
) -> String {
    let _ = (model, aspect_ratio, duration);
    format!(
        "Create a video from this description if video generation is available: \"{prompt}\". Return a public Google Drive link when ready."
    )
}

pub(crate) fn create_video(request_body: &[u8], config: &Config) -> io::Result<serde_json::Value> {
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

    let requested_model = body
        .get("model")
        .and_then(|v| v.as_str())
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

    let duration: u64 = body
        .get("duration")
        .and_then(|v| {
            if let Some(n) = v.as_u64() {
                Some(n)
            } else if let Some(s) = v.as_str() {
                s.trim_end_matches('s').parse::<u64>().ok()
            } else {
                None
            }
        })
        .unwrap_or(5);

    let full_prompt = build_video_prompt(upstream_model, &prompt, aspect_ratio, duration);

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

    build_video_result(
        upstream_model,
        requested_model,
        &prompt,
        aspect_ratio,
        duration,
        content,
    )
}

pub(crate) fn build_video_result(
    model: &str,
    requested_model: &str,
    prompt: &str,
    aspect_ratio: &str,
    duration: u64,
    content: &str,
) -> io::Result<serde_json::Value> {
    let video_url = extract_url_from_text(content);
    if video_url.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Muse video generation completed without a supported public HTTPS artifact",
        ));
    }

    Ok(serde_json::json!({
        "id": format!("video-{}", Uuid::new_v4()),
        "object": "video.generation",
        "created": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        "model": model,
        "requested_model": requested_model,
        "prompt": prompt,
        "requested_aspect_ratio": aspect_ratio,
        "requested_duration": duration,
        "status": "completed",
        "video_url": video_url
    }))
}

pub(crate) fn extract_url_from_text(text: &str) -> String {
    let urls: Vec<&str> = text
        .split_whitespace()
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
        .collect();
    // Prioritize public Google Drive URLs if output by upstream.
    for url in &urls {
        let cleaned = trim_trailing_punct(url);
        if is_google_drive_file_path(cleaned) {
            return cleaned.to_string();
        }
    }
    // Fall back to direct video file URLs (.mp4, .mov, .webm).
    for url in &urls {
        let cleaned = trim_trailing_punct(url);
        if is_supported_video_url(cleaned) {
            return cleaned.to_string();
        }
    }
    String::new()
}

pub(crate) fn is_supported_video_url(url: &str) -> bool {
    url.starts_with("https://")
        && ((is_google_drive_host(url) && is_google_drive_file_path(url))
            || has_supported_video_extension(url))
}

pub(crate) fn has_supported_video_extension(url: &str) -> bool {
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

pub(crate) fn is_google_drive_host(url: &str) -> bool {
    let authority = url_authority(url);
    matches!(authority, "drive.google.com" | "docs.google.com")
}

pub(crate) fn is_google_drive_file_path(url: &str) -> bool {
    url.starts_with("https://")
        && is_google_drive_host(url)
        && url
            .strip_prefix("https://")
            .and_then(|rest| rest.split_once('/').map(|(_, path)| path))
            .is_some_and(|path| path.starts_with("file/") || path.starts_with("uc?"))
}

pub(crate) fn url_authority(url: &str) -> &str {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .and_then(|host| host.split('?').next())
        .and_then(|host| host.split('#').next())
        .unwrap_or("")
}

pub(crate) fn trim_trailing_punct(s: &str) -> &str {
    s.trim_matches([
        '"', '\'', ',', ';', ')', '}', '(', '{', ']', '[', '\\', '\n', '\r', ' ',
    ])
}

pub(crate) fn normalize_video_model(model: &str) -> &str {
    match model {
        "muse-video" | "muse" => "muse-video",
        "gen-3" | "gen-2" | "kling" | "gen-4" | "gen-4.5" | "gen-4-turbo" | "aleph-2.0"
        | "ruby" => "muse-video",
        _ => model,
    }
}
