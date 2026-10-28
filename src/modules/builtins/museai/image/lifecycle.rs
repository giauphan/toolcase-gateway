//! Muse image generation job persistence and state tracking.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn image_job_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ImageJobRecord {
    pub job_id: String,
    pub owner_id: String,
    pub model: String,
    pub requested_model: String,
    #[serde(default)]
    pub prompt: String,
    pub aspect_ratio: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    pub created_at: u64,
    pub last_activity_at: u64,
    pub status: String,
    pub media_validated: bool,
    pub output_verified: bool,
    #[serde(default)]
    pub image_url: String,
    #[serde(default)]
    pub submission_key: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn state_dir() -> PathBuf {
    std::env::var("GW_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".gw_state"))
}

pub fn is_valid_job_id(job_id: &str) -> bool {
    !job_id.is_empty()
        && job_id.len() <= 128
        && job_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn ensure_image_state_dir() -> io::Result<PathBuf> {
    let dir = state_dir().join("image_jobs");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn image_job_file_path(job_id: &str) -> io::Result<PathBuf> {
    let dir = ensure_image_state_dir()?;
    if !is_valid_job_id(job_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid job id",
        ));
    }
    Ok(dir.join(format!("{job_id}.json")))
}

pub fn save_image_job(record: &ImageJobRecord) -> io::Result<()> {
    let path = image_job_file_path(&record.job_id)?;
    let temp_path = path.with_extension("tmp");
    let json = serde_json::to_string_pretty(record)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(&temp_path, json)?;
    fs::rename(temp_path, path)?;
    Ok(())
}

pub fn load_image_job(job_id: &str) -> io::Result<ImageJobRecord> {
    let path = image_job_file_path(job_id)?;
    let data = fs::read_to_string(&path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => io::Error::new(io::ErrorKind::NotFound, "image job not found"),
        _ => e,
    })?;
    serde_json::from_str::<ImageJobRecord>(&data)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn find_image_job_by_submission_key(dir: &Path, sub_key: &str) -> Option<ImageJobRecord> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(std::ffi::OsStr::to_str) == Some("json") {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(existing) = serde_json::from_str::<ImageJobRecord>(&content) {
                    if existing.submission_key.as_deref() == Some(sub_key) {
                        return Some(existing);
                    }
                }
            }
        }
    }
    None
}

pub fn reserve_image_job(record: &ImageJobRecord) -> io::Result<Option<ImageJobRecord>> {
    let _guard = image_job_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if let Some(ref sub_key) = record.submission_key {
        if !sub_key.is_empty() {
            let dir = ensure_image_state_dir()?;
            if let Some(existing) = find_image_job_by_submission_key(&dir, sub_key) {
                return Ok(Some(existing));
            }
        }
    }

    if let Ok(existing) = load_image_job(&record.job_id) {
        return Ok(Some(existing));
    }

    save_image_job(record)?;
    Ok(None)
}

pub fn update_image_job(
    job_id: &str,
    status: &str,
    image_url: Option<&str>,
    error: Option<&str>,
) -> io::Result<()> {
    let _guard = image_job_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut record = load_image_job(job_id)?;
    record.status = status.to_string();
    if let Some(url) = image_url {
        record.image_url = url.to_string();
        record.output_verified = !url.is_empty();
        record.media_validated = !url.is_empty();
    }
    if let Some(err) = error {
        record.error = Some(err.to_string());
    }
    record.last_activity_at = now_secs();
    save_image_job(&record)
}
