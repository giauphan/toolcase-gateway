//! Image media storage, inspection, and MIME type resolution.

use std::fs;
use std::io;
use std::path::PathBuf;

pub fn media_dir() -> PathBuf {
    super::lifecycle::state_dir().join("media")
}

pub fn guess_mime_type(path_or_url: &str) -> &'static str {
    let clean = path_or_url
        .split('?')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    if clean.ends_with(".png") {
        "image/png"
    } else if clean.ends_with(".jpg") || clean.ends_with(".jpeg") {
        "image/jpeg"
    } else if clean.ends_with(".webp") {
        "image/webp"
    } else if clean.ends_with(".gif") {
        "image/gif"
    } else if clean.ends_with(".svg") {
        "image/svg+xml"
    } else if clean.ends_with(".bmp") {
        "image/bmp"
    } else if clean.ends_with(".avif") {
        "image/avif"
    } else {
        "image/png"
    }
}

pub fn guess_extension(mime_or_url: &str) -> &'static str {
    let lower = mime_or_url.to_ascii_lowercase();
    if lower.contains("jpeg") || lower.contains("jpg") {
        "jpg"
    } else if lower.contains("webp") {
        "webp"
    } else if lower.contains("gif") {
        "gif"
    } else if lower.contains("svg") {
        "svg"
    } else if lower.contains("bmp") {
        "bmp"
    } else if lower.contains("avif") {
        "avif"
    } else {
        "png"
    }
}

pub fn find_image_media(job_id: &str) -> Option<(PathBuf, &'static str)> {
    if !super::lifecycle::is_valid_job_id(job_id) {
        return None;
    }
    let dir = media_dir();
    const EXTENSIONS: [(&str, &str); 7] = [
        ("png", "image/png"),
        ("jpg", "image/jpeg"),
        ("jpeg", "image/jpeg"),
        ("webp", "image/webp"),
        ("gif", "image/gif"),
        ("svg", "image/svg+xml"),
        ("avif", "image/avif"),
    ];
    for (ext, mime) in EXTENSIONS {
        let p = dir.join(format!("{job_id}.{ext}"));
        if p.is_file() {
            return Some((p, mime));
        }
    }
    None
}

pub fn has_image_media(job_id: &str) -> bool {
    find_image_media(job_id).is_some()
}

pub fn save_image_media(job_id: &str, bytes: &[u8], ext: &str) -> io::Result<PathBuf> {
    if !super::lifecycle::is_valid_job_id(job_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid job id",
        ));
    }
    let dir = media_dir();
    fs::create_dir_all(&dir)?;
    let clean_ext = ext.trim_start_matches('.');
    if clean_ext.is_empty()
        || !clean_ext.chars().all(|c| c.is_ascii_alphanumeric())
        || clean_ext.len() > 10
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid file extension",
        ));
    }
    let final_path = dir.join(format!("{job_id}.{clean_ext}"));
    let tmp_path = dir.join(format!(".tmp-img-{}-{}", job_id, uuid::Uuid::new_v4()));
    fs::write(&tmp_path, bytes)?;
    if let Err(e) = fs::rename(&tmp_path, &final_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(final_path)
}

pub fn load_image_media(job_id: &str) -> io::Result<(Vec<u8>, &'static str)> {
    if !super::lifecycle::is_valid_job_id(job_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid job id",
        ));
    }
    if let Some((path, mime)) = find_image_media(job_id) {
        let bytes = fs::read(path)?;
        Ok((bytes, mime))
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("image media not found for job: {job_id}"),
        ))
    }
}
