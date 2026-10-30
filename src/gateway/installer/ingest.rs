//! Remote and local archive ingestion for gateway modules.

use super::InstallSource;
use crate::gateway::archive::extract_zip;
use crate::gateway::modules::ModuleError;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

pub(super) fn ingest_github(
    url: &str,
    branch: Option<&str>,
    target_dir: &Path,
) -> Result<(), ModuleError> {
    let clean_url = url.trim().trim_end_matches('/');
    let path = clean_url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("github.com/");
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() < 2 {
        return Err(ModuleError::InvalidManifest(format!(
            "Invalid GitHub URL: {url}"
        )));
    }
    let owner = parts[0];
    let repo = parts[1].trim_end_matches(".git");

    let inferred_branch = if branch.is_none() && parts.len() >= 4 && parts[2] == "tree" {
        Some(parts[3])
    } else {
        branch
    };

    let branches_to_try = if let Some(b) = inferred_branch {
        vec![b.to_string()]
    } else {
        vec!["main".to_string(), "master".to_string()]
    };

    let mut last_err = String::new();

    for b in branches_to_try {
        let zip_url = format!("https://codeload.github.com/{owner}/{repo}/zip/refs/heads/{b}");
        match fetch_url_bytes(&zip_url) {
            Ok(bytes) => {
                extract_zip(&bytes, target_dir)
                    .map_err(|e| ModuleError::InvalidManifest(e.to_string()))?;
                return Ok(());
            }
            Err(e) => {
                last_err = format!("Failed to download branch '{b}': {e}");
            }
        }
    }

    // If HTTP zip download failed, attempt git clone as fallback
    let mut git_cmd = std::process::Command::new("git");
    git_cmd
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null());
    git_cmd.args([
        "clone",
        "--depth",
        "1",
        url,
        target_dir.to_str().unwrap_or(""),
    ]);
    if let Ok(status) = git_cmd.status() {
        if status.success() {
            let _ = fs::remove_dir_all(target_dir.join(".git"));
            return Ok(());
        }
    }

    Err(ModuleError::InvalidManifest(format!(
        "Failed to download module from GitHub: {last_err}"
    )))
}

pub(super) fn ingest_zip_url(url: &str, target_dir: &Path) -> Result<(), ModuleError> {
    let bytes = fetch_url_bytes(url)
        .map_err(|e| ModuleError::InvalidManifest(format!("Failed to fetch zip URL: {e}")))?;
    extract_zip(&bytes, target_dir).map_err(|e| ModuleError::InvalidManifest(e.to_string()))?;
    Ok(())
}

pub(super) fn ingest_zip_file(path: &Path, target_dir: &Path) -> Result<(), ModuleError> {
    let bytes = fs::read(path)
        .map_err(|e| ModuleError::InvalidManifest(format!("Failed to read zip file: {e}")))?;
    extract_zip(&bytes, target_dir).map_err(|e| ModuleError::InvalidManifest(e.to_string()))?;
    Ok(())
}

fn fetch_url_bytes(url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .new_agent();

    let resp = agent
        .get(url)
        .header("User-Agent", "toolcase-gateway/0.1.0")
        .call()
        .map_err(|e| format!("HTTP request error: {e}"))?;

    let mut bytes = Vec::new();
    let reader = resp.into_body().into_reader();
    let mut limited = std::io::Read::take(reader, MAX_ARCHIVE_BYTES + 1);
    std::io::Read::read_to_end(&mut limited, &mut bytes)
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err("Downloaded archive exceeds maximum permitted size (64MB)".into());
    }
    Ok(bytes)
}

pub(super) fn unwrap_single_top_level_directory(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut dirs = Vec::new();
    let mut has_files = false;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "__MACOSX" {
            if name == "__MACOSX" {
                let _ = fs::remove_dir_all(entry.path());
            }
            continue;
        }
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => dirs.push(entry.path()),
            Ok(ft) if ft.is_file() => has_files = true,
            _ => {}
        }
    }
    if !has_files && dirs.len() == 1 {
        let single_dir = &dirs[0];
        let temp_stage = dir.join(format!(".tmp_unwrap_{}", uuid::Uuid::new_v4()));
        if fs::rename(single_dir, &temp_stage).is_ok() {
            if let Ok(re_read) = fs::read_dir(&temp_stage) {
                for item in re_read.flatten() {
                    let target = dir.join(item.file_name());
                    let _ = fs::rename(item.path(), target);
                }
            }
            let _ = fs::remove_dir_all(&temp_stage);
        }
    }
}

pub(super) fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Resolves a string parameter into an `InstallSource`.
pub fn resolve_source(raw: &str) -> InstallSource {
    let trimmed = raw.trim();
    if trimmed.starts_with("https://github.com/") || trimmed.starts_with("http://github.com/") {
        InstallSource::GitHubUrl(trimmed.to_string())
    } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        InstallSource::ZipUrl(trimmed.to_string())
    } else {
        let p = PathBuf::from(trimmed);
        if p.is_file() {
            InstallSource::ZipFile(p)
        } else {
            InstallSource::LocalDir(p)
        }
    }
}

/// Decodes a base64 encoded byte sequence.
pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    while clean.len() % 4 != 0 {
        clean.push(b'=');
    }
    let mut out = Vec::with_capacity((clean.len() / 4) * 3);
    for chunk in clean.chunks_exact(4) {
        let mut b = [0u32; 4];
        let mut pad = 0;
        for (i, &byte) in chunk.iter().enumerate() {
            if byte == b'=' {
                b[i] = 0;
                pad += 1;
            } else if let Some(pos) = TABLE.iter().position(|&t| t == byte) {
                b[i] = pos as u32;
            } else {
                return Err(format!("Invalid base64 character: {}", byte as char));
            }
        }
        let triple = (b[0] << 18) | (b[1] << 12) | (b[2] << 6) | b[3];
        out.push(((triple >> 16) & 0xFF) as u8);
        if pad < 2 {
            out.push(((triple >> 8) & 0xFF) as u8);
        }
        if pad < 1 {
            out.push((triple & 0xFF) as u8);
        }
    }
    Ok(out)
}
