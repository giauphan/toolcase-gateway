use crate::config::Config;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn debug_log(msg: &str) {
    if std::env::var("MUSEAI_DEBUG").is_ok() {
        crate::runtime::logger::log_message("DEBUG", "museai_threads", msg);
    }
}

pub struct TrackedThread {
    pub id: String,
    pub created_at: u64,
    pub base_url: String,
    pub config: Config,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersistedThreadRecord {
    pub id: String,
    pub created_at: u64,
    pub base_url: String,
    #[serde(default)]
    pub retention_secs: u64,
}

pub fn thread_state_dir() -> PathBuf {
    PathBuf::from(".gw_state").join("threads")
}

pub fn sanitize_thread_filename(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn save_thread_to_disk(thread: &TrackedThread) {
    let dir = thread_state_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        debug_log(&format!("Failed to create thread state directory: {e}"));
        return;
    }
    let filename = format!("thread-{}.json", sanitize_thread_filename(&thread.id));
    let path = dir.join(filename);
    let record = PersistedThreadRecord {
        id: thread.id.clone(),
        created_at: thread.created_at,
        base_url: thread.base_url.clone(),
        retention_secs: thread.config.museai_thread_retention_secs,
    };
    if let Ok(bytes) = serde_json::to_vec(&record) {
        let _ = std::fs::write(path, bytes);
    }
}

pub fn remove_thread_from_disk(id: &str) {
    let dir = thread_state_dir();
    let filename = format!("thread-{}.json", sanitize_thread_filename(id));
    let path = dir.join(filename);
    let _ = std::fs::remove_file(path);
}

pub fn sync_threads_from_disk() {
    let dir = thread_state_dir();
    if !dir.exists() {
        return;
    }
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut loaded = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(record) = serde_json::from_slice::<PersistedThreadRecord>(&bytes) {
                let config = Config {
                    museai_thread_retention_secs: record.retention_secs,
                    ..Default::default()
                };
                loaded.push(TrackedThread {
                    id: record.id,
                    created_at: record.created_at,
                    base_url: record.base_url,
                    config,
                });
            }
        }
    }
    if let Ok(mut threads) = tracked_threads().lock() {
        for item in loaded {
            if !threads.iter().any(|t| t.id == item.id) {
                threads.push(item);
            }
        }
    }
}

pub fn unregister_thread(thread_id: &str) {
    remove_thread_from_disk(thread_id);
    if let Ok(mut threads) = tracked_threads().lock() {
        threads.retain(|t| t.id != thread_id);
    }
}

pub fn tracked_threads() -> &'static Mutex<Vec<TrackedThread>> {
    static TRACKED_THREADS: OnceLock<Mutex<Vec<TrackedThread>>> = OnceLock::new();
    TRACKED_THREADS.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn register_thread(thread_id: String, base_url: String, config: Config) {
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let thread = TrackedThread {
        id: thread_id,
        created_at,
        base_url,
        config,
    };
    save_thread_to_disk(&thread);
    if let Ok(mut threads) = tracked_threads().lock() {
        threads.retain(|t| t.id != thread.id);
        threads.push(thread);
    }
}
