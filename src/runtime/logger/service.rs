//! Logger public interface and global static singleton.

use super::archive::cleanup_expired_logs;
use super::types::LogConfig;
use super::writer::LogWriter;
use std::io;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

pub struct Logger {
    inner: Mutex<LogWriter>,
}

impl Logger {
    pub fn new(config: LogConfig) -> Self {
        Self {
            inner: Mutex::new(LogWriter::new(config)),
        }
    }

    pub fn log(&self, level: &str, target: &str, msg: &str) {
        if let Ok(mut writer) = self.inner.lock() {
            writer.write_entry(level, target, msg);
        }
    }

    pub fn log_access(&self, method: &str, path: &str, client_ip: &str) {
        if let Ok(mut writer) = self.inner.lock() {
            writer.write_access_entry(method, path, client_ip, None, None);
        }
    }

    pub fn log_access_with_status(
        &self,
        method: &str,
        path: &str,
        client_ip: &str,
        status: u16,
        duration_ms: u64,
    ) {
        if let Ok(mut writer) = self.inner.lock() {
            writer.write_access_entry(method, path, client_ip, Some(status), Some(duration_ms));
        }
    }

    pub fn set_dir<P: AsRef<Path>>(&self, dir: P) {
        if let Ok(mut writer) = self.inner.lock() {
            writer.set_dir(dir.as_ref().to_path_buf());
        }
    }

    pub fn cleanup_logs(&self) -> io::Result<usize> {
        if let Ok(writer) = self.inner.lock() {
            cleanup_expired_logs(&writer.config.dir, writer.config.retention_days)
        } else {
            Ok(0)
        }
    }
}

static GLOBAL_LOGGER: OnceLock<Logger> = OnceLock::new();

pub fn default_logger() -> &'static Logger {
    GLOBAL_LOGGER.get_or_init(|| Logger::new(LogConfig::default()))
}

/// Log a message with a given level and subsystem target to the global logger.
pub fn log_message(level: &str, target: &str, msg: &str) {
    default_logger().log(level, target, msg);
}

/// Log an HTTP request access entry to the global logger.
pub fn log_access(method: &str, path: &str, client_ip: &str) {
    default_logger().log_access(method, path, client_ip);
}

/// Log an HTTP request access entry with final response status and duration to the global logger.
pub fn log_access_with_status(
    method: &str,
    path: &str,
    client_ip: &str,
    status: u16,
    duration_ms: u64,
) {
    default_logger().log_access_with_status(method, path, client_ip, status, duration_ms);
}

/// Set the log output directory at runtime for the global logger.
pub fn set_log_dir<P: AsRef<Path>>(dir: P) {
    default_logger().set_dir(dir);
}

/// Trigger log retention cleanup on default log directory.
pub fn cleanup_expired_logs_default() -> io::Result<usize> {
    default_logger().cleanup_logs()
}
