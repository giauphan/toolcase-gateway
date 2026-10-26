//! Image URL parsing, prompt construction, and aspect ratio handling.

use crate::providers::museai::video::{
    extract_candidate_urls, is_google_drive_file_path, is_trusted_video_cdn_host, is_vm_media_url,
    trim_trailing_punct, url_authority,
};

pub const SUPPORTED_IMAGE_MODELS: [&str; 6] = [
    "muse",
    "muse-image",
    "dall-e-3",
    "dall-e-2",
    "imagen",
    "flux",
];

pub const SUPPORTED_IMAGE_ASPECT_RATIOS: [&str; 9] = [
    "1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3", "5:4", "4:5",
];

pub const DEFAULT_IMAGE_ASPECT_RATIO: &str = "1:1";

pub fn normalize_image_model(model: &str) -> &str {
    match model {
        "muse" | "muse-image" | "dall-e-3" | "dall-e-2" | "imagen" | "flux" => "muse-image",
        _ => model,
    }
}

pub fn map_size_to_aspect_ratio(size: &str) -> Option<&'static str> {
    match size.trim() {
        "1024x1024" | "512x512" | "256x256" => Some("1:1"),
        "1792x1024" | "1920x1080" | "1280x720" => Some("16:9"),
        "1024x1792" | "1080x1920" | "720x1280" => Some("9:16"),
        "1024x768" => Some("4:3"),
        "768x1024" => Some("3:4"),
        "1200x800" => Some("3:2"),
        "800x1200" => Some("2:3"),
        _ => None,
    }
}

pub fn build_image_prompt(model: &str, prompt: &str, aspect_ratio: &str) -> String {
    let _ = model;
    format!(
        "Create an image with aspect ratio {aspect_ratio} from this description if image generation is available: \"{prompt}\". Return an image artifact link when ready. Do not change sharing permissions."
    )
}

pub fn has_supported_image_extension(url: &str) -> bool {
    let path_str = if let Some(after_scheme) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        after_scheme
            .split_once('/')
            .map(|(_, path)| path)
            .unwrap_or("")
    } else {
        url
    };
    let path = path_str
        .split('?')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .trim_end_matches('/')
        .to_ascii_lowercase();

    const EXTENSIONS: [&str; 8] = [
        ".png", ".jpg", ".jpeg", ".webp", ".gif", ".svg", ".bmp", ".avif",
    ];

    EXTENSIONS.iter().any(|ext| path.ends_with(ext))
        || path
            .split('/')
            .any(|segment| EXTENSIONS.iter().any(|ext| segment.ends_with(ext)))
}

pub fn is_supported_image_url(url: &str) -> bool {
    if is_gateway_image_content_url(url) {
        return true;
    }
    if url.starts_with("https://") {
        if is_google_drive_file_path(url) {
            return true;
        }
        if is_muse_image_files_url(url) {
            return true;
        }
        if has_supported_image_extension(url) && is_trusted_image_cdn_host(url_authority(url)) {
            return true;
        }
    }
    false
}

pub fn is_gateway_image_content_url(url: &str) -> bool {
    url.contains("/muse-ai/v1/images/") && (url.ends_with("/content") || url.ends_with("/download"))
}

pub fn is_muse_image_files_url(url: &str) -> bool {
    url.starts_with("https://")
        && url_authority(url) == "muse.ai"
        && url
            .strip_prefix("https://")
            .and_then(|rest| rest.split_once('/').map(|(_, path)| path))
            .is_some_and(|path| path.starts_with("files/") && has_supported_image_extension(url))
}

pub fn is_trusted_image_cdn_host(authority: &str) -> bool {
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
    if is_trusted_video_cdn_host(authority) {
        return true;
    }
    host == "images.unsplash.com"
        || host == "cdn.midjourney.com"
        || host == "oaidalleapiprodscus.blob.core.windows.net"
}

pub fn is_raw_image_artifact_url(url: &str) -> bool {
    is_supported_image_url(url)
        || ((url.starts_with("https://")
            || url.starts_with("http://")
            || url.starts_with('/')
            || url.starts_with("workspace/"))
            && (is_vm_media_url(url) || is_trusted_image_cdn_host(url_authority(url)))
            && has_supported_image_extension(url))
}

pub fn extract_image_url_from_text(text: &str) -> String {
    if let Some(url) = extract_markdown_image_url(text) {
        return url;
    }

    let urls = extract_candidate_urls(text);

    for url in &urls {
        let cleaned = trim_trailing_punct(url);
        if is_google_drive_file_path(cleaned) {
            return cleaned.to_string();
        }
    }

    for url in &urls {
        let cleaned = trim_trailing_punct(url);
        if is_supported_image_url(cleaned) {
            return cleaned.to_string();
        }
    }

    for url in &urls {
        let cleaned = trim_trailing_punct(url);
        if is_raw_image_artifact_url(cleaned) {
            return cleaned.to_string();
        }
    }

    for url in &urls {
        let cleaned = trim_trailing_punct(url);
        if (cleaned.starts_with("https://") || cleaned.starts_with("http://"))
            && has_supported_image_extension(cleaned)
        {
            return cleaned.to_string();
        }
    }

    String::new()
}

fn extract_markdown_image_url(text: &str) -> Option<String> {
    let mut search = text;
    while let Some(idx) = search.find("![") {
        let rest = &search[idx + 2..];
        if let Some(close_bracket) = rest.find(']') {
            let after_bracket = &rest[close_bracket + 1..];
            if after_bracket.starts_with('(') {
                if let Some(close_paren) = after_bracket.find(')') {
                    let target = after_bracket[1..close_paren].trim();
                    let url = target.split_whitespace().next().unwrap_or("");
                    let cleaned = trim_trailing_punct(url);
                    if (cleaned.starts_with("https://") || cleaned.starts_with("http://"))
                        && (has_supported_image_extension(cleaned)
                            || is_raw_image_artifact_url(cleaned))
                    {
                        return Some(cleaned.to_string());
                    }
                }
            }
        }
        search = rest;
    }
    None
}
