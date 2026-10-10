use crate::config::Config;
use std::io;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn debug_log(msg: &str) {
    if std::env::var("MUSEAI_DEBUG").is_ok() {
        eprintln!("[museai_threads] {msg}");
    }
}

pub struct TrackedThread {
    pub id: String,
    pub created_at: u64,
    pub base_url: String,
    pub config: Config,
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
    if let Ok(mut threads) = tracked_threads().lock() {
        threads.push(TrackedThread {
            id: thread_id,
            created_at,
            base_url,
            config,
        });
    }
}

pub fn delete_muse_thread(base_url: &str, thread_id: &str, config: &Config) -> io::Result<()> {
    if thread_id.is_empty() {
        return Ok(());
    }

    let delete_thread_url = format!("{base_url}/api/thread/{thread_id}");
    let mut req = ureq::delete(&delete_thread_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("Origin", base_url)
        .header("Referer", &format!("{base_url}/"))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36")
        .header("sec-ch-ua", r#""Brave";v="153", "Not_A Brand";v="8", "Chromium";v="153""#)
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", r#""Windows""#)
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin");

    if !config.museai_cookie.is_empty() {
        req = req.header("Cookie", &config.museai_cookie);
    }

    match req.call() {
        Ok(_) => {
            debug_log(&format!("Successfully cleaned up thread: {thread_id}"));
            Ok(())
        }
        Err(e) => {
            debug_log(&format!("Note: Failed to clean up thread {thread_id}: {e}"));
            Ok(())
        }
    }
}

pub fn cleanup_tracked_threads_once() -> usize {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut to_delete = Vec::new();
    if let Ok(mut threads) = tracked_threads().lock() {
        let mut i = 0;
        while i < threads.len() {
            if now >= threads[i].created_at + threads[i].config.museai_thread_retention_secs {
                to_delete.push(threads.remove(i));
            } else {
                i += 1;
            }
        }
    }

    let deleted_count = to_delete.len();
    for thread in to_delete {
        let _ = delete_muse_thread(&thread.base_url, &thread.id, &thread.config);
    }
    deleted_count
}

pub fn start_cleanup_worker() {
    thread::spawn(|| loop {
        thread::sleep(Duration::from_secs(300));
        cleanup_tracked_threads_once();
    });
}

pub fn spawn_muse_thread(base_url: &str, config: &Config) -> io::Result<String> {
    let create_thread_url = format!("{base_url}/thread/new");
    let mut req = ureq::post(&create_thread_url)
        .header("Content-Type", "text/plain;charset=UTF-8")
        .header("Accept", "text/x-component")
        .header("Origin", base_url)
        .header("Referer", &format!("{base_url}/thread/new"))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36")
        .header("sec-ch-ua", r#""Brave";v="153", "Not_A Brand";v="8", "Chromium";v="153""#)
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", r#""Windows""#)
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin")
        .header("next-action", "00eaaa570484d537580e0bf4e19ee28a65ac048d62");

    if !config.museai_cookie.is_empty() {
        req = req.header("Cookie", &config.museai_cookie);
    }

    let response = match req.send("[]") {
        Ok(res) => res,
        Err(ureq::Error::StatusCode(401)) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Muse.ai thread creation returned 401 Unauthorized — credentials are invalid or expired",
            ));
        }
        Err(ureq::Error::StatusCode(403)) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Muse.ai thread creation returned 403 Forbidden — access denied",
            ));
        }
        Err(ureq::Error::StatusCode(404)) => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Muse.ai thread creation returned 404 — endpoint not found or cookie expired",
            ));
        }
        Err(ureq::Error::StatusCode(status)) => {
            return Err(io::Error::other(format!(
                "Muse.ai thread creation returned HTTP {status}"
            )));
        }
        Err(e) => {
            return Err(io::Error::other(format!(
                "Thread creation request failed: {e}"
            )));
        }
    };
    let mut reader = response.into_body();
    let text = reader
        .read_to_string()
        .map_err(|e| io::Error::other(format!("Failed to read thread response: {e}")))?;
    let thread_id = extract_new_thread_id(&text);
    if thread_id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "failed to extract thread ID from /thread/new response",
        ));
    }
    let channel = format!("thread:{thread_id}");
    Ok(channel)
}

pub fn extract_new_thread_id(rsc_text: &str) -> String {
    let needle = "/thread/";
    if let Some(idx) = rsc_text.rfind(needle) {
        let start = idx + needle.len();
        let remaining = &rsc_text[start..];
        let end = remaining
            .find(|c: char| !c.is_alphanumeric() && c != '-')
            .unwrap_or(remaining.len());
        return remaining[..end].to_string();
    }
    String::new()
}
