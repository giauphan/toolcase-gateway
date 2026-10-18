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
