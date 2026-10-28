//! Safe in-memory PKZIP extraction with path traversal protection.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

/// Error types occurring during zip archive decoding or extraction.
#[derive(Debug)]
pub enum ArchiveError {
    InvalidZip(String),
    PathTraversal(String),
    Io(io::Error),
    UnsupportedCompression(u16),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidZip(msg) => write!(f, "Invalid zip archive: {msg}"),
            Self::PathTraversal(path) => {
                write!(f, "Path traversal vulnerability rejected for: {path}")
            }
            Self::Io(e) => write!(f, "I/O error during archive extraction: {e}"),
            Self::UnsupportedCompression(method) => {
                write!(f, "Unsupported zip compression method: {method}")
            }
        }
    }
}

impl std::error::Error for ArchiveError {}

impl From<io::Error> for ArchiveError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

struct CentralEntry {
    name: String,
    compression_method: u16,
    compressed_size: usize,
    local_header_offset: usize,
    is_dir: bool,
}

/// Safely extracts a zip archive from a byte slice into `target_dir`.
/// Protects against Zip Slip (CWE-22) and automatically strips single
/// top-level root directories (standard in GitHub release archives).
pub fn extract_zip(zip_data: &[u8], target_dir: &Path) -> Result<Vec<PathBuf>, ArchiveError> {
    if zip_data.len() < 22 {
        return Err(ArchiveError::InvalidZip("File too small for zip".into()));
    }

    fs::create_dir_all(target_dir)?;
    let canonical_target = target_dir.canonicalize()?;

    let entries = parse_central_directory(zip_data)?;
    if entries.is_empty() {
        return Err(ArchiveError::InvalidZip("Archive has no entries".into()));
    }

    let common_prefix = find_common_root_prefix(&entries);
    let mut extracted = Vec::new();

    for entry in entries {
        let relative_str = if let Some(prefix) = &common_prefix {
            entry.name.strip_prefix(prefix).unwrap_or(&entry.name)
        } else {
            &entry.name
        };

        let sanitized_rel = sanitize_archive_path(relative_str)?;
        if sanitized_rel.as_os_str().is_empty() {
            continue;
        }

        let out_path = canonical_target.join(&sanitized_rel);

        // Security check: ensure path never escapes canonical target directory
        if !is_inside_target(&out_path, &canonical_target) {
            return Err(ArchiveError::PathTraversal(entry.name));
        }

        if entry.is_dir {
            fs::create_dir_all(&out_path)?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file_data = read_entry_data(zip_data, &entry)?;
        let mut file = File::create(&out_path)?;
        file.write_all(&file_data)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if is_script_or_executable(&out_path, &file_data) {
                let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(0o700));
            }
        }

        extracted.push(out_path);
    }

    Ok(extracted)
}

fn parse_central_directory(zip_data: &[u8]) -> Result<Vec<CentralEntry>, ArchiveError> {
    let eocd_pos = find_eocd(zip_data)
        .ok_or_else(|| ArchiveError::InvalidZip("End of Central Directory not found".into()))?;

    if eocd_pos + 22 > zip_data.len() {
        return Err(ArchiveError::InvalidZip("Malformed EOCD record".into()));
    }

    let total_records =
        u16::from_le_bytes([zip_data[eocd_pos + 10], zip_data[eocd_pos + 11]]) as usize;
    let cd_offset = u32::from_le_bytes([
        zip_data[eocd_pos + 16],
        zip_data[eocd_pos + 17],
        zip_data[eocd_pos + 18],
        zip_data[eocd_pos + 19],
    ]) as usize;

    let mut cursor = cd_offset;
    let mut entries = Vec::with_capacity(total_records);

    for _ in 0..total_records {
        if cursor + 46 > zip_data.len() {
            break;
        }

        if &zip_data[cursor..cursor + 4] != b"PK\x01\x02" {
            break;
        }

        let compression = u16::from_le_bytes([zip_data[cursor + 10], zip_data[cursor + 11]]);
        let comp_size = u32::from_le_bytes([
            zip_data[cursor + 20],
            zip_data[cursor + 21],
            zip_data[cursor + 22],
            zip_data[cursor + 23],
        ]) as usize;

        let name_len = u16::from_le_bytes([zip_data[cursor + 28], zip_data[cursor + 29]]) as usize;
        let extra_len = u16::from_le_bytes([zip_data[cursor + 30], zip_data[cursor + 31]]) as usize;
        let comment_len =
            u16::from_le_bytes([zip_data[cursor + 32], zip_data[cursor + 33]]) as usize;

        let local_offset = u32::from_le_bytes([
            zip_data[cursor + 42],
            zip_data[cursor + 43],
            zip_data[cursor + 44],
            zip_data[cursor + 45],
        ]) as usize;

        let name_start = cursor + 46;
        let name_end = name_start + name_len;
        if name_end > zip_data.len() {
            return Err(ArchiveError::InvalidZip("Entry name out of bounds".into()));
        }

        let raw_name = String::from_utf8_lossy(&zip_data[name_start..name_end]).to_string();
        let is_dir = raw_name.ends_with('/') || raw_name.ends_with('\\');

        entries.push(CentralEntry {
            name: raw_name,
            compression_method: compression,
            compressed_size: comp_size,
            local_header_offset: local_offset,
            is_dir,
        });

        cursor = name_end + extra_len + comment_len;
    }

    Ok(entries)
}

fn read_entry_data(zip_data: &[u8], entry: &CentralEntry) -> Result<Vec<u8>, ArchiveError> {
    let loc = entry.local_header_offset;
    if loc + 30 > zip_data.len() {
        return Err(ArchiveError::InvalidZip(
            "Local header out of bounds".into(),
        ));
    }

    if &zip_data[loc..loc + 4] != b"PK\x03\x04" {
        return Err(ArchiveError::InvalidZip(
            "Local header signature mismatch".into(),
        ));
    }

    let local_name_len = u16::from_le_bytes([zip_data[loc + 26], zip_data[loc + 27]]) as usize;
    let local_extra_len = u16::from_le_bytes([zip_data[loc + 28], zip_data[loc + 29]]) as usize;

    let data_start = loc + 30 + local_name_len + local_extra_len;
    let data_end = data_start + entry.compressed_size;

    if data_end > zip_data.len() {
        return Err(ArchiveError::InvalidZip(
            "Entry payload exceeds file bounds".into(),
        ));
    }

    let raw_slice = &zip_data[data_start..data_end];

    match entry.compression_method {
        0 => Ok(raw_slice.to_vec()),
        8 => {
            let mut decoder = flate2::read::DeflateDecoder::new(raw_slice);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed).map_err(|e| {
                ArchiveError::InvalidZip(format!("Deflate decompression failed: {e}"))
            })?;
            Ok(decompressed)
        }
        m => Err(ArchiveError::UnsupportedCompression(m)),
    }
}

fn find_eocd(zip_data: &[u8]) -> Option<usize> {
    let min_len = 22;
    if zip_data.len() < min_len {
        return None;
    }

    let max_search = (zip_data.len() - min_len).min(65535 + min_len);
    let start = zip_data.len() - min_len;
    let end = zip_data.len() - max_search;

    for i in (end..=start).rev() {
        if &zip_data[i..i + 4] == b"PK\x05\x06" {
            return Some(i);
        }
    }
    None
}

fn sanitize_archive_path(path_str: &str) -> Result<PathBuf, ArchiveError> {
    let normalized = path_str.replace('\\', "/");
    let mut clean = PathBuf::new();

    for comp in Path::new(&normalized).components() {
        match comp {
            Component::Normal(seg) => {
                let seg_str = seg.to_string_lossy();
                if seg_str == ".." || seg_str.contains(':') {
                    return Err(ArchiveError::PathTraversal(path_str.to_string()));
                }
                clean.push(seg);
            }
            Component::CurDir => continue,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ArchiveError::PathTraversal(path_str.to_string()));
            }
        }
    }

    Ok(clean)
}

fn is_inside_target(path: &Path, target: &Path) -> bool {
    let mut ancestor = path.to_path_buf();
    while !ancestor.exists() {
        if let Some(parent) = ancestor.parent() {
            ancestor = parent.to_path_buf();
        } else {
            break;
        }
    }

    if let Ok(canon_ancestor) = ancestor.canonicalize() {
        canon_ancestor.starts_with(target)
    } else {
        path.starts_with(target)
    }
}

fn find_common_root_prefix(entries: &[CentralEntry]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }

    let first = &entries[0].name.replace('\\', "/");
    let first_slash = first.find('/')?;
    let candidate = &first[..=first_slash];

    for entry in entries {
        let normalized = entry.name.replace('\\', "/");
        if !normalized.starts_with(candidate) {
            return None;
        }
    }

    Some(candidate.to_string())
}

#[cfg(unix)]
fn is_script_or_executable(path: &Path, data: &[u8]) -> bool {
    if data.starts_with(b"#!") {
        return true;
    }
    if let Some(ext) = path.extension().and_then(std::ffi::OsStr::to_str) {
        matches!(ext, "sh" | "py" | "js" | "rb" | "bin")
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_stored_and_deflated_zip() {
        let temp_dir = std::env::temp_dir().join(format!("test_zip_{}", uuid::Uuid::new_v4()));
        let mut zip = Vec::new();

        // 1. Deflated file entry
        let content = b"console.log('hello from test module');";
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(content).unwrap();
        let comp = encoder.finish().unwrap();

        let mut crc = flate2::Crc::new();
        crc.update(content);
        let crc32 = crc.sum();

        let name = "my-module-main/index.js";
        let local_offset = zip.len();

        zip.extend_from_slice(b"PK\x03\x04");
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&8u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&crc32.to_le_bytes());
        zip.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(content.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(name.as_bytes());
        zip.extend_from_slice(&comp);

        let cd_offset = zip.len();
        zip.extend_from_slice(b"PK\x01\x02");
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&8u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&crc32.to_le_bytes());
        zip.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(content.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&(local_offset as u32).to_le_bytes());
        zip.extend_from_slice(name.as_bytes());

        let cd_size = zip.len() - cd_offset;
        zip.extend_from_slice(b"PK\x05\x06");
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&1u16.to_le_bytes());
        zip.extend_from_slice(&1u16.to_le_bytes());
        zip.extend_from_slice(&(cd_size as u32).to_le_bytes());
        zip.extend_from_slice(&(cd_offset as u32).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());

        let extracted = extract_zip(&zip, &temp_dir).expect("extraction succeeds");
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].file_name().unwrap(), "index.js");

        let read_back = fs::read_to_string(&extracted[0]).unwrap();
        assert_eq!(read_back, "console.log('hello from test module');");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_rejects_path_traversal() {
        assert!(sanitize_archive_path("../../etc/passwd").is_err());
        assert!(sanitize_archive_path("/etc/shadow").is_err());
        assert!(sanitize_archive_path("foo/../../bar").is_err());
        assert!(sanitize_archive_path("C:\\windows\\system32").is_err());
    }
}
