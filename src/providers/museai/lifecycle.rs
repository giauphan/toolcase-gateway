//! Video job persistence, revision history, and lifecycle retention management.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct VideoRevision {
    pub revision_id: String,
    pub prompt: String,
    pub created_at: u64,
    pub video_url: String,
    pub status: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct VideoJobRecord {
    pub job_id: String,
    pub owner_id: String,
    pub model: String,
    pub requested_model: String,
    pub aspect_ratio: String,
    pub duration: u64,
    pub thread_id: Option<String>,
    pub created_at: u64,
    pub last_activity_at: u64,
    pub pinned: bool,
    pub status: String,
    pub media_validated: bool,
    pub output_verified: bool,
    pub video_url: String,
    pub revisions: Vec<VideoRevision>,
}

pub fn state_dir() -> PathBuf {
    std::env::var("GW_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".gw_state"))
}

pub fn ensure_state_dir() -> io::Result<PathBuf> {
    let dir = state_dir().join("video_jobs");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn job_file_path(job_id: &str) -> io::Result<PathBuf> {
    let dir = ensure_state_dir()?;
    // Sanitize job_id to prevent path traversal
    let clean_id: String = job_id
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if clean_id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid job id",
        ));
    }
    Ok(dir.join(format!("{clean_id}.json")))
}

pub fn save_video_job(record: &VideoJobRecord) -> io::Result<()> {
    let path = job_file_path(&record.job_id)?;
    let temp_path = path.with_extension("tmp");
    let json = serde_json::to_string_pretty(record)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(&temp_path, json)?;
    fs::rename(temp_path, path)?;
    Ok(())
}

pub fn load_video_job(job_id: &str) -> io::Result<VideoJobRecord> {
    let path = job_file_path(job_id)?;
    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "video job not found",
        ));
    }
    let data = fs::read_to_string(path)?;
    let record: VideoJobRecord =
        serde_json::from_str(&data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(record)
}

pub fn list_video_jobs() -> io::Result<Vec<VideoJobRecord>> {
    let dir = match ensure_state_dir() {
        Ok(d) => d,
        Err(_) => return Ok(Vec::new()),
    };
    let mut jobs = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(data) = fs::read_to_string(&path) {
                    if let Ok(record) = serde_json::from_str::<VideoJobRecord>(&data) {
                        jobs.push(record);
                    }
                }
            }
        }
    }
    Ok(jobs)
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn debug_log_lifecycle(msg: &str) {
    if std::env::var("MUSEAI_DEBUG").is_ok() {
        eprintln!("[museai_lifecycle] {msg}");
    }
}

pub fn evaluate_job_retention(record: &VideoJobRecord, now: u64, ttl_secs: u64) -> bool {
    if record.pinned {
        return false;
    }
    // Respect active/pending/in-flight states
    match record.status.as_str() {
        "queued" | "submitting" | "generating" | "validating" | "approval_required" => {
            return false
        }
        _ => {}
    }
    let age = now.saturating_sub(record.last_activity_at);
    age >= ttl_secs
}

pub fn cleanup_expired_video_jobs(ttl_secs: u64) -> io::Result<usize> {
    let jobs = list_video_jobs()?;
    let now = now_secs();
    let mut cleaned = 0;
    for job in jobs {
        if evaluate_job_retention(&job, now, ttl_secs) {
            let path = job_file_path(&job.job_id)?;
            if fs::remove_file(path).is_ok() {
                cleaned += 1;
                debug_log_lifecycle(&format!("Cleaned up expired video job: {}", job.job_id));
            }
        }
    }
    Ok(cleaned)
}

pub fn append_job_revision(
    job_id: &str,
    new_prompt: &str,
    video_url: &str,
    status: &str,
) -> io::Result<VideoJobRecord> {
    let mut record = load_video_job(job_id)?;
    let now = now_secs();
    let rev_count = record.revisions.len() + 1;
    let revision = VideoRevision {
        revision_id: format!("rev-{rev_count}"),
        prompt: new_prompt.to_string(),
        created_at: now,
        video_url: video_url.to_string(),
        status: status.to_string(),
    };
    record.revisions.push(revision);
    record.last_activity_at = now;
    record.video_url = video_url.to_string();
    record.status = status.to_string();
    save_video_job(&record)?;
    Ok(record)
}
