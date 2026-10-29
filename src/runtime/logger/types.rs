//! Logger configuration options and default constants.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct LogConfig {
    pub enabled: bool,
    pub console: bool,
    pub daily: bool,
    pub dir: PathBuf,
    pub file_name: String,
    pub retention_days: u64,
    pub max_size_bytes: u64,
    pub zip_enabled: bool,
}

pub const DEFAULT_LOG_MAX_SIZE_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

impl Default for LogConfig {
    fn default() -> Self {
        let enabled = std::env::var("GW_LOG_ENABLED")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(true);
        let console = std::env::var("GW_LOG_CONSOLE")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(true);
        let daily = std::env::var("GW_LOG_DAILY")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(true);
        let dir = PathBuf::from(std::env::var("GW_LOG_DIR").unwrap_or_else(|_| "logs".to_string()));
        let file_name = std::env::var("GW_LOG_FILE").unwrap_or_else(|_| "gateway.log".to_string());
        let retention_days = std::env::var("GW_LOG_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2);
        let max_size_bytes = std::env::var("GW_LOG_MAX_SIZE_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| {
                let mb: u64 = std::env::var("GW_LOG_MAX_SIZE_MB")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(10);
                mb * 1024 * 1024
            });
        let zip_enabled = std::env::var("GW_LOG_ZIP_ENABLED")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(true);

        Self {
            enabled,
            console,
            daily,
            dir,
            file_name,
            retention_days,
            max_size_bytes,
            zip_enabled,
        }
    }
}
