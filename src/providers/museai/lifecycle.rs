//! Video job persistence, revision history, and lifecycle retention management.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn job_update_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct VideoRevision {
    pub revision_id: String,
    pub prompt: String,
    pub created_at: u64,
    pub video_url: String,
    pub status: String,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct VideoJobRecord {
    pub job_id: String,
    pub owner_id: String,
    pub model: String,
    pub requested_model: String,
    #[serde(default)]
    pub prompt: String,
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
    #[serde(default)]
    pub approval_status: Option<String>,
    #[serde(default)]
    pub approval_id: Option<String>,
    #[serde(default)]
    pub submission_key: Option<String>,
    #[serde(default)]
    pub last_checked_at: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
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
    if job_id.is_empty()
        || !job_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid job id",
        ));
    }
    Ok(dir.join(format!("{job_id}.json")))
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

pub fn update_video_job_with_error(
    job_id: &str,
    status: &str,
    approval_status: Option<&str>,
    thread_id: Option<&str>,
    error: Option<&str>,
) -> io::Result<()> {
    let _guard = job_update_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut record = load_video_job(job_id)?;
    record.status = status.to_string();
    record.approval_status = approval_status.map(str::to_string);
    if let Some(tid) = thread_id {
        record.thread_id = Some(tid.to_string());
    }
    if let Some(err) = error {
        record.error = Some(err.to_string());
    }
    record.last_activity_at = now_secs();
    record.last_checked_at = Some(record.last_activity_at);
    save_video_job(&record)
}

pub fn update_video_job(
    job_id: &str,
    status: &str,
    approval_status: Option<&str>,
    thread_id: Option<&str>,
) -> io::Result<()> {
    update_video_job_with_error(job_id, status, approval_status, thread_id, None)
}

pub const STALE_SUBMITTING_SECS: u64 = 300;

pub fn is_submitting_stale(record: &VideoJobRecord, now: u64) -> bool {
    if record.status != "submitting" {
        return false;
    }
    let ref_time = record.last_checked_at.unwrap_or(record.created_at);
    if ref_time == 0 {
        return false;
    }
    now.saturating_sub(ref_time) >= STALE_SUBMITTING_SECS
}

pub fn recover_stale_submitting_job(record: &mut VideoJobRecord, now: u64) -> bool {
    if is_submitting_stale(record, now) {
        record.status = "submission_unknown".to_string();
        record.last_activity_at = now;
        record.last_checked_at = Some(now);
        true
    } else {
        false
    }
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
    let mut record: VideoJobRecord =
        serde_json::from_str(&data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let now = now_secs();
    if recover_stale_submitting_job(&mut record, now) {
        let _ = save_video_job(&record);
    }
    Ok(record)
}

pub fn list_video_jobs() -> io::Result<Vec<VideoJobRecord>> {
    scan_video_jobs()
}

fn scan_video_jobs() -> io::Result<Vec<VideoJobRecord>> {
    let dir = ensure_state_dir()?;
    let mut jobs = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            match fs::read_to_string(&path).and_then(|data| {
                serde_json::from_str::<VideoJobRecord>(&data).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "Invalid video job record")
                })
            }) {
                Ok(record) => jobs.push(record),
                Err(_) => eprintln!("[museai_lifecycle] Skipping unreadable video job record"),
            }
        }
    }
    Ok(jobs)
}

pub fn reserve_video_job(record: &VideoJobRecord) -> io::Result<Option<VideoJobRecord>> {
    let _guard = job_update_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if let Some(submission_key) = record.submission_key.as_deref() {
        if let Some(existing) = scan_video_jobs()?
            .into_iter()
            .find(|job| job.submission_key.as_deref() == Some(submission_key))
        {
            if existing.status != "failed" {
                return Ok(Some(existing));
            }

            let old_path = job_file_path(&existing.job_id)?;
            let _ = fs::remove_file(old_path);
        }
    }

    if job_file_path(&record.job_id)?.try_exists()? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "video job id already exists",
        ));
    }
    save_video_job(record)?;
    Ok(None)
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn debug_log_lifecycle(msg: &str) {
    if std::env::var("MUSEAI_DEBUG").is_ok() {
        crate::runtime::logger::log_message("DEBUG", "museai_lifecycle", msg);
    }
}

pub fn evaluate_job_retention(record: &VideoJobRecord, now: u64, ttl_secs: u64) -> bool {
    if record.pinned {
        return false;
    }
    // Respect active/pending/in-flight states
    match record.status.as_str() {
        "queued" | "submitting" | "generating" | "validating" | "approval_pending"
        | "approval_required" => return false,
        _ => {}
    }
    let age = now.saturating_sub(record.last_activity_at);
    age >= ttl_secs
}

pub fn cleanup_expired_video_jobs(ttl_secs: u64) -> io::Result<usize> {
    cleanup_expired_video_jobs_with_callback(ttl_secs, |_| {})
}

pub fn cleanup_expired_video_jobs_with_callback<F>(
    ttl_secs: u64,
    mut on_delete: F,
) -> io::Result<usize>
where
    F: FnMut(&VideoJobRecord),
{
    let jobs = list_video_jobs()?;
    let now = now_secs();
    let mut cleaned = 0;
    for job in jobs {
        if evaluate_job_retention(&job, now, ttl_secs) {
            on_delete(&job);
            let media_path = crate::providers::museai::video::video_media_path(&job.job_id);
            if media_path.is_file() {
                let _ = fs::remove_file(media_path);
            }
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
    let _guard = job_update_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

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
    record.prompt = new_prompt.to_string();
    record.video_url = video_url.to_string();
    record.status = status.to_string();
    save_video_job(&record)?;
    Ok(record)
}

pub fn update_job_prompt(job_id: &str, prompt: &str) -> io::Result<VideoJobRecord> {
    let _guard = job_update_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let mut record = load_video_job(job_id)?;
    let now = now_secs();
    record.prompt = prompt.to_string();
    record.last_activity_at = now;
    save_video_job(&record)?;
    Ok(record)
}
