//! URL classification and extraction shared by image and video artifacts.

fn is_media_url_token(value: &str) -> bool {
    value.starts_with("https://")
        || value.starts_with("http://")
        || value.starts_with("sandbox://workspace/")
        || value.starts_with("sandbox:///workspace/")
        || value.starts_with("/fs/raw/workspace/")
        || value.starts_with("workspace/")
        || value.starts_with("/workspace/")
}

fn next_media_url_start(text: &str) -> Option<usize> {
    ["https://", "http://", "sandbox://"]
        .iter()
        .filter_map(|scheme| text.find(scheme))
        .min()
}

pub fn extract_candidate_urls(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for word in text.split_whitespace() {
        let cleaned = trim_trailing_punct(word);
        if is_media_url_token(cleaned) {
            if !urls.iter().any(|u| u == cleaned) {
                urls.push(cleaned.to_string());
            }
        } else if let Some(idx) = next_media_url_start(word) {
            let slice = trim_trailing_punct(&word[idx..]);
            if is_media_url_token(slice) && !urls.iter().any(|u| u == slice) {
                urls.push(slice.to_string());
            }
        }
    }
    let mut rest = text;
    while let Some(idx) = next_media_url_start(rest) {
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
        if is_media_url_token(candidate) && !urls.iter().any(|u| u == candidate) {
            urls.push(candidate.to_string());
        }
        rest = &after[end.max(1)..];
    }
    urls
}

pub fn is_vm_media_host(authority: &str) -> bool {
    let host = authority
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host == "metaaivm.com"
        || host.ends_with(".metaaivm.com")
        || host == "willow606.com"
        || host.ends_with(".willow606.com")
        || host == "exe.xyz"
        || host.ends_with(".exe.xyz")
}

pub fn is_vm_media_url(url: &str) -> bool {
    is_vm_media_host(url_authority(url))
        || url.starts_with("/fs/raw/workspace/")
        || url.starts_with("/workspace/")
        || url.starts_with("workspace/")
        || url.starts_with("sandbox://workspace/")
        || url.starts_with("sandbox:///workspace/")
}

pub fn extract_vm_media_path(url: &str) -> Option<String> {
    let w_idx = url.find("workspace/")?;
    let path = &url[w_idx..];
    let clean = path
        .split('?')
        .next()
        .unwrap_or(path)
        .split('#')
        .next()
        .unwrap_or(path)
        .trim();
    if clean.starts_with("workspace/") {
        Some(clean.to_string())
    } else {
        None
    }
}

pub fn is_trusted_media_cdn_host(authority: &str) -> bool {
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

/// Normalizes a URL by stripping scheme/host, query strings, and fragments, returning a lowercase clean path.
pub fn clean_url_path(url: &str) -> String {
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
    path_str
        .split('?')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .trim_end_matches('/')
        .to_ascii_lowercase()
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
