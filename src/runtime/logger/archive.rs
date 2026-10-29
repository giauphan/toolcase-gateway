//! PKZIP in-memory compression, file rotation, and retention cleanup.

use super::format::{extract_date_from_filename, format_date, parse_date_to_epoch_days};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Creates a standard PKZIP format archive in-memory containing a single file entry compressed with DEFLATE.
pub fn create_zip_archive(entry_name: &str, uncompressed_data: &[u8]) -> Vec<u8> {
    let mut crc = flate2::Crc::new();
    crc.update(uncompressed_data);
    let crc32 = crc.sum();

    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = encoder.write_all(uncompressed_data);
    let compressed_data = encoder.finish().unwrap_or_default();

    let mut zip = Vec::with_capacity(compressed_data.len() + 256);

    let name_bytes = entry_name.as_bytes();
    let name_len = name_bytes.len() as u16;
    let comp_len = compressed_data.len() as u32;
    let uncomp_len = uncompressed_data.len() as u32;

    // 1. Local file header
    zip.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
    zip.extend_from_slice(&20u16.to_le_bytes()); // version needed: 2.0
    zip.extend_from_slice(&0u16.to_le_bytes()); // flags
    zip.extend_from_slice(&8u16.to_le_bytes()); // compression: 8 (Deflate)
    zip.extend_from_slice(&0u16.to_le_bytes()); // mod time
    zip.extend_from_slice(&0u16.to_le_bytes()); // mod date
    zip.extend_from_slice(&crc32.to_le_bytes());
    zip.extend_from_slice(&comp_len.to_le_bytes());
    zip.extend_from_slice(&uncomp_len.to_le_bytes());
    zip.extend_from_slice(&name_len.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
    zip.extend_from_slice(name_bytes);
    zip.extend_from_slice(&compressed_data);

    let central_dir_offset = zip.len() as u32;

    // 2. Central directory file header
    zip.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
    zip.extend_from_slice(&20u16.to_le_bytes()); // version made by
    zip.extend_from_slice(&20u16.to_le_bytes()); // version needed: 2.0
    zip.extend_from_slice(&0u16.to_le_bytes()); // flags
    zip.extend_from_slice(&8u16.to_le_bytes()); // compression: 8 (Deflate)
    zip.extend_from_slice(&0u16.to_le_bytes()); // mod time
    zip.extend_from_slice(&0u16.to_le_bytes()); // mod date
    zip.extend_from_slice(&crc32.to_le_bytes());
    zip.extend_from_slice(&comp_len.to_le_bytes());
    zip.extend_from_slice(&uncomp_len.to_le_bytes());
    zip.extend_from_slice(&name_len.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
    zip.extend_from_slice(&0u16.to_le_bytes()); // comment length
    zip.extend_from_slice(&0u16.to_le_bytes()); // disk number start
    zip.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
    zip.extend_from_slice(&0u32.to_le_bytes()); // external attributes
    zip.extend_from_slice(&0u32.to_le_bytes()); // relative offset of local header
    zip.extend_from_slice(name_bytes);

    let central_dir_size = (zip.len() as u32) - central_dir_offset;

    // 3. End of central directory record (EOCD)
    zip.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
    zip.extend_from_slice(&0u16.to_le_bytes()); // disk number
    zip.extend_from_slice(&0u16.to_le_bytes()); // disk where central dir starts
    zip.extend_from_slice(&1u16.to_le_bytes()); // records on this disk
    zip.extend_from_slice(&1u16.to_le_bytes()); // total records
    zip.extend_from_slice(&central_dir_size.to_le_bytes());
    zip.extend_from_slice(&central_dir_offset.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes()); // comment length

    zip
}

pub fn resolve_archive_path(dir: &Path, filename: &str, zip_enabled: bool) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let date_str = format_date(now);
    let stem = filename.strip_suffix(".log").unwrap_or(filename);
    let base_stem = if extract_date_from_filename(stem).is_some() {
        stem.to_string()
    } else {
        format!("{stem}-{date_str}")
    };

    let ext = if zip_enabled { "zip" } else { "log" };
    let mut idx = 1;
    loop {
        let candidate = format!("{base_stem}-{idx}.{ext}");
        let p = dir.join(&candidate);
        if !p.exists() {
            return p;
        }
        idx += 1;
    }
}

pub fn write_archive_file(
    archive_path: &Path,
    filename: &str,
    data: &[u8],
    zip_enabled: bool,
) -> io::Result<()> {
    if zip_enabled {
        let zip_bytes = create_zip_archive(filename, data);
        fs::write(archive_path, &zip_bytes)
    } else {
        fs::write(archive_path, data)
    }
}

pub fn perform_log_rotation(target_path: &Path, dir: &Path, filename: &str, zip_enabled: bool) {
    let Ok(data) = fs::read(target_path) else {
        return;
    };
    if data.is_empty() {
        return;
    }
    let archive_path = resolve_archive_path(dir, filename, zip_enabled);
    if write_archive_file(&archive_path, filename, &data, zip_enabled).is_ok() {
        let _ = fs::remove_file(target_path);
    }
}

pub fn rotate_if_needed(
    file: &mut Option<File>,
    dir: &Path,
    filename: &str,
    additional_bytes: usize,
    max_size_bytes: u64,
    zip_enabled: bool,
) {
    if max_size_bytes == 0 {
        return;
    }
    let target_path = dir.join(filename);
    let current_len = file
        .as_ref()
        .and_then(|f| f.metadata().ok())
        .map(|m| m.len())
        .unwrap_or_else(|| fs::metadata(&target_path).map(|m| m.len()).unwrap_or(0));

    if current_len == 0 || current_len + (additional_bytes as u64) <= max_size_bytes {
        return;
    }

    if let Some(mut f) = file.take() {
        let _ = f.flush();
    }
    perform_log_rotation(&target_path, dir, filename, zip_enabled);
}

/// Clean up rotated daily logs and zip archives older than `retention_days`.
pub fn cleanup_expired_logs(dir: &Path, retention_days: u64) -> io::Result<usize> {
    if !dir.is_dir() || retention_days == 0 {
        return Ok(0);
    }
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (now_days, max_age_secs) = (now_secs / 86400, retention_days * 86400);
    let mut count = 0;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(std::ffi::OsStr::to_str) else {
                continue;
            };
            let is_log = name.ends_with(".log");
            let is_zip = name.ends_with(".zip");
            if !is_log && !is_zip {
                continue;
            }
            // Never clean primary active log files like gateway.log, error.log, access.log
            if name == "gateway.log" || name == "error.log" || name == "access.log" {
                continue;
            }
            let Some(date_str) = extract_date_from_filename(name) else {
                continue;
            };

            let by_date = parse_date_to_epoch_days(date_str)
                .is_some_and(|d| now_days > d && now_days - d >= retention_days);
            let by_mtime = entry.metadata().and_then(|m| m.modified()).is_ok_and(|m| {
                let mtime = m.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                now_secs > mtime && now_secs - mtime >= max_age_secs
            });

            if (by_date || by_mtime) && fs::remove_file(&path).is_ok() {
                count += 1;
            }
        }
    }
    Ok(count)
}
