pub use crate::providers::museai::media::{
    clean_url_path, extract_candidate_urls, extract_vm_media_path, is_google_drive_file_path,
    is_google_drive_host, is_trusted_media_cdn_host as is_trusted_video_cdn_host, is_vm_media_host,
    is_vm_media_url, trim_trailing_punct, url_authority,
};

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
    build_video_prompt_with_attachments(model, prompt, aspect_ratio, duration, &[])
}

pub fn build_video_prompt_with_attachment(
    model: &str,
    prompt: &str,
    aspect_ratio: &str,
    duration: u64,
    attachment: Option<&str>,
) -> String {
    if let Some(att) = attachment.filter(|s| !s.trim().is_empty()) {
        build_video_prompt_with_attachments(
            model,
            prompt,
            aspect_ratio,
            duration,
            &[att.to_string()],
        )
    } else {
        build_video_prompt_with_attachments(model, prompt, aspect_ratio, duration, &[])
    }
}

pub fn build_video_prompt_with_attachments(
    model: &str,
    prompt: &str,
    aspect_ratio: &str,
    duration: u64,
    attachments: &[String],
) -> String {
    let _ = (model, aspect_ratio, duration);
    if attachments.is_empty() {
        format!(
            "Create a video from this description if video generation is available: \"{prompt}\". Return a video artifact link when ready. Do not change sharing permissions."
        )
    } else if attachments.len() == 1 {
        format!(
            "Create a video animated from this reference image ({}) and description if video generation is available: \"{prompt}\". Return a video artifact link when ready. Do not change sharing permissions.",
            attachments[0]
        )
    } else {
        let refs = attachments.join(", ");
        format!(
            "Create a video animated from these reference images ({refs}) and description if video generation is available: \"{prompt}\". Return a video artifact link when ready. Do not change sharing permissions."
        )
    }
}

pub fn extract_url_from_text(text: &str) -> String {
    let urls = extract_candidate_urls(text);
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
    for url in &urls {
        let cleaned = trim_trailing_punct(url);
        if is_raw_video_artifact_url(cleaned) {
            return cleaned.to_string();
        }
    }
    String::new()
}

pub fn is_gateway_video_content_url(url: &str) -> bool {
    url.contains("/muse-ai/v1/videos/") && (url.ends_with("/content") || url.ends_with("/download"))
}

pub fn is_raw_video_artifact_url(url: &str) -> bool {
    is_supported_video_url(url)
        || ((url.starts_with("https://")
            || url.starts_with("http://")
            || url.starts_with("sandbox://workspace/")
            || url.starts_with("sandbox:///workspace/")
            || url.starts_with('/')
            || url.starts_with("workspace/"))
            && is_vm_media_url(url)
            && has_supported_video_extension(url))
}

pub fn is_supported_video_url(url: &str) -> bool {
    is_gateway_video_content_url(url)
        || (url.starts_with("https://")
            && (is_muse_files_url(url)
                || (is_google_drive_host(url) && is_google_drive_file_path(url))
                || (has_supported_video_extension(url)
                    && !is_spoofed_host(url)
                    && !is_vm_media_url(url))))
}

pub fn is_muse_files_url(url: &str) -> bool {
    url.starts_with("https://")
        && url_authority(url) == "muse.ai"
        && url
            .strip_prefix("https://")
            .and_then(|rest| rest.split_once('/').map(|(_, path)| path))
            .is_some_and(|path| path.starts_with("files/") && has_supported_video_extension(url))
}

pub fn is_spoofed_host(url: &str) -> bool {
    let authority = url_authority(url);
    !is_trusted_video_cdn_host(authority)
}

pub fn has_supported_video_extension(url: &str) -> bool {
    let path = clean_url_path(url);
    path.ends_with(".mp4")
        || path.ends_with(".mov")
        || path.ends_with(".webm")
        || path.ends_with(".m4v")
        || path.ends_with(".ogv")
        || path.split('/').any(|segment| {
            segment.ends_with(".mp4")
                || segment.ends_with(".mov")
                || segment.ends_with(".webm")
                || segment.ends_with(".m4v")
                || segment.ends_with(".ogv")
        })
}

pub fn normalize_video_model(model: &str) -> &str {
    match model {
        "muse-video" | "muse" => "muse-video",
        "gen-3" | "gen-2" | "kling" | "gen-4" | "gen-4.5" | "gen-4-turbo" | "aleph-2.0"
        | "ruby" => "muse-video",
        _ => model,
    }
}
