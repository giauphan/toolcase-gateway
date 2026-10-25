use crate::config::Config;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod registry;
pub use registry::*;
mod remote;
pub use remote::*;
mod spawn;
pub use spawn::*;

pub fn cleanup_tracked_threads_once() -> usize {
    let global_ttl = std::env::var("GW_MUSEAI_THREAD_RETENTION_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(7200);

    cleanup_tracked_threads_with_ttl(global_ttl)
}

pub fn cleanup_tracked_threads_with_ttl(global_ttl: u64) -> usize {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    sync_threads_from_disk();

    let mut to_delete = Vec::new();
    if let Ok(mut threads) = tracked_threads().lock() {
        let mut i = 0;
        while i < threads.len() {
            let thread_ttl = if threads[i].config.museai_thread_retention_secs == 0 {
                0
            } else {
                std::cmp::min(threads[i].config.museai_thread_retention_secs, global_ttl)
            };
            if now >= threads[i].created_at.saturating_add(thread_ttl) {
                to_delete.push(threads.remove(i));
            } else {
                i += 1;
            }
        }
    }

    let deleted_count = to_delete.len();
    for thread in to_delete {
        remove_thread_from_disk(&thread.id);
        let _ = delete_muse_thread(&thread.base_url, &thread.id, &thread.config);
    }
    deleted_count
}

pub fn config_from_env() -> Config {
    Config {
        museai_base_url: std::env::var("GW_MUSEAI_BASE_URL")
            .unwrap_or_else(|_| "https://muse.ai".to_string()),
        museai_cookie: std::env::var("GW_MUSEAI_COOKIE").unwrap_or_default(),
        museai_ws_url: std::env::var("GW_MUSEAI_WS_URL")
            .unwrap_or_else(|_| "wss://hatch.metaaivm.com/v1/noise".to_string()),
        museai_access_token: std::env::var("GW_MUSEAI_ACCESS_TOKEN").unwrap_or_default(),
        museai_notary_token: std::env::var("GW_MUSEAI_NOTARY_TOKEN").unwrap_or_default(),
        museai_vm_id: std::env::var("GW_MUSEAI_VM_ID").unwrap_or_default(),
        museai_auto_cleanup_threads: std::env::var("GW_MUSEAI_AUTO_CLEANUP_THREADS")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(true),
        museai_thread_retention_secs: std::env::var("GW_MUSEAI_THREAD_RETENTION_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7200),
        ..Default::default()
    }
}

pub fn cleanup_cycle_once() -> (usize, usize) {
    let ttl_secs = std::env::var("GW_MUSEAI_THREAD_RETENTION_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(7200);

    let cfg = config_from_env();
    cleanup_cycle_with_config(ttl_secs, &cfg)
}

pub fn cleanup_cycle_with_ttl(ttl_secs: u64) -> (usize, usize) {
    let cfg = config_from_env();
    cleanup_cycle_with_config(ttl_secs, &cfg)
}

pub fn cleanup_cycle_with_config(ttl_secs: u64, config: &Config) -> (usize, usize) {
    let remote_purged = purge_remote_cvm_threads_older_than(ttl_secs, config).unwrap_or(0);
    let threads_cleaned = cleanup_tracked_threads_with_ttl(ttl_secs);

    let base_url = if config.museai_base_url.is_empty() {
        "https://muse.ai".to_string()
    } else {
        config.museai_base_url.clone()
    };
    let cfg_clone = config.clone();

    let jobs_cleaned =
        super::lifecycle::cleanup_expired_video_jobs_with_callback(ttl_secs, move |job| {
            if let Some(ref tid) = job.thread_id {
                let _ = delete_muse_thread(&base_url, tid, &cfg_clone);
            }
        })
        .unwrap_or(0);

    let logs_cleaned = crate::runtime::logger::cleanup_expired_logs_default().unwrap_or(0);
    let total_threads_cleaned = threads_cleaned + remote_purged;
    if total_threads_cleaned > 0 || jobs_cleaned > 0 || logs_cleaned > 0 {
        crate::runtime::logger::log_message(
            "INFO",
            "museai:cleanup",
            &format!(
                "cycle purged {total_threads_cleaned} threads ({remote_purged} remote CVM, {threads_cleaned} local tracked), {jobs_cleaned} expired jobs, and {logs_cleaned} expired log files (TTL: {ttl_secs}s)"
            ),
        );
    }

    (total_threads_cleaned, jobs_cleaned)
}

pub fn start_cleanup_worker() {
    start_cleanup_worker_with_interval(Duration::from_secs(300));
}

pub fn start_cleanup_worker_with_interval(interval: Duration) {
    crate::runtime::logger::log_message(
        "INFO",
        "museai:cleanup",
        &format!(
            "background cleanup daemon worker started (interval: {:?})",
            interval
        ),
    );
    thread::spawn(move || {
        // Run immediately on start so threads older than 2 hours are cleared right away
        cleanup_cycle_once();
        loop {
            thread::sleep(interval);
            cleanup_cycle_once();
        }
    });
}
