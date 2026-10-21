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

pub fn extract_candidate_urls(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for word in text.split_whitespace() {
        let cleaned = trim_trailing_punct(word);
        if cleaned.starts_with("https://") || cleaned.starts_with("http://") {
            if !urls.iter().any(|u| u == cleaned) {
                urls.push(cleaned.to_string());
            }
        } else if let Some(idx) = word.find("https://").or_else(|| word.find("http://")) {
            let slice = trim_trailing_punct(&word[idx..]);
            if (slice.starts_with("https://") || slice.starts_with("http://"))
                && !urls.iter().any(|u| u == slice)
            {
                urls.push(slice.to_string());
            }
        }
    }
    let mut rest = text;
    while let Some(idx) = rest.find("https://").or_else(|| rest.find("http://")) {
        let after = &rest[idx..];
        let end = after
            .find(|c: char| {
                c.is_whitespace()
                    || matches!(
                        c,
                        '"' | '\'' | '<' | '>' | ')' | ']' | '}' | '\\' | ',' | ';'
                    )
            })
            .unwrap_or(after.len());
        let candidate = trim_trailing_punct(&after[..end]);
        if (candidate.starts_with("https://") || candidate.starts_with("http://"))
            && !urls.iter().any(|u| u == candidate)
        {
            urls.push(candidate.to_string());
        }
        rest = &after[end.max(1)..];
    }
    urls
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

pub fn is_trusted_video_cdn_host(authority: &str) -> bool {
    let host = authority
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    if host == "muse.ai" || host.ends_with(".muse.ai") {
        return true;
    }
    if host.ends_with(".fbcdn.net")
        || host == "fbsbx.com"
        || host.ends_with(".fbsbx.com")
        || host == "meta.com"
        || host.ends_with(".meta.com")
        || host == "metaaivm.com"
        || host.ends_with(".metaaivm.com")
        || host == "cdninstagram.com"
        || host.ends_with(".cdninstagram.com")
    {
        return true;
    }
    false
}

pub fn is_spoofed_host(url: &str) -> bool {
    let authority = url_authority(url);
    !is_trusted_video_cdn_host(authority)
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
        .trim_end_matches('/')
        .to_ascii_lowercase();
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
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let authority = without_scheme
        .split('/')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("");
    let host = authority.split('@').next_back().unwrap_or(authority);
    host.split(':').next().unwrap_or(host)
}

pub fn trim_trailing_punct(s: &str) -> &str {
    s.trim_matches([
        '"', '\'', ',', ';', ')', '}', '(', '{', ']', '[', '<', '>', '\\', '\n', '\r', ' ',
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
