//! Thread-safe multi-stream framework logging infrastructure.
#![forbid(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct LogConfig {
    pub enabled: bool,
    pub console: bool,
    pub daily: bool,
    pub dir: PathBuf,
    pub file_name: String,
    pub retention_days: u64,
}

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
            .unwrap_or(14);

        Self {
            enabled,
            console,
            daily,
            dir,
            file_name,
            retention_days,
        }
    }
}

pub struct Logger {
    inner: Mutex<LogWriter>,
}

struct LogWriter {
    config: LogConfig,
    current_date: String,
    gateway_file: Option<File>,
    daily_gateway_file: Option<File>,
    error_file: Option<File>,
    daily_error_file: Option<File>,
}

impl Logger {
    pub fn new(config: LogConfig) -> Self {
        Self {
            inner: Mutex::new(LogWriter {
                config,
                current_date: String::new(),
                gateway_file: None,
                daily_gateway_file: None,
                error_file: None,
                daily_error_file: None,
            }),
        }
    }

    pub fn log(&self, level: &str, target: &str, msg: &str) {
        if let Ok(mut writer) = self.inner.lock() {
            writer.write_entry(level, target, msg);
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

fn open_append(dir: &Path, filename: &str) -> Option<File> {
    if let Err(e) = fs::create_dir_all(dir) {
        eprintln!("[logger] failed to create log dir {:?}: {e}", dir);
        return None;
    }
    let path = dir.join(filename);
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => Some(f),
        Err(e) => {
            eprintln!("[logger] failed to open log file {:?}: {e}", path);
            None
        }
    }
}

fn write_line(file: &mut Option<File>, dir: &Path, filename: &str, line: &str) {
    if file.is_none() {
        *file = open_append(dir, filename);
    }
    if let Some(ref mut f) = file {
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }
}

impl LogWriter {
    fn check_date_roll(&mut self, today: &str) {
        if self.current_date != today {
            self.current_date = today.to_string();
            self.daily_gateway_file = None;
            self.daily_error_file = None;
        }
    }

    fn write_entry(&mut self, level: &str, target: &str, msg: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let ts = format_timestamp(now);
        let today = format_date(now);
        self.check_date_roll(&today);

        let sanitized = sanitize_log_message(msg);
        let line = format!("[{ts}] [{level}] [{target}] {sanitized}\n");

        if self.config.console {
            eprint!("{line}");
        }

        if self.config.enabled {
            // 1. General gateway logs
            let main_name = self.config.file_name.clone();
            write_line(&mut self.gateway_file, &self.config.dir, &main_name, &line);
            if self.config.daily {
                let daily_name = format!("gateway-{today}.log");
                write_line(
                    &mut self.daily_gateway_file,
                    &self.config.dir,
                    &daily_name,
                    &line,
                );
            }

            // 2. Error streams for WARN and ERROR
            if level.eq_ignore_ascii_case("warn") || level.eq_ignore_ascii_case("error") {
                write_line(&mut self.error_file, &self.config.dir, "error.log", &line);
                if self.config.daily {
                    let daily_err = format!("error-{today}.log");
                    write_line(
                        &mut self.daily_error_file,
                        &self.config.dir,
                        &daily_err,
                        &line,
                    );
                }
            }
        }
    }

    fn set_dir(&mut self, dir: PathBuf) {
        self.config.dir = dir;
        self.gateway_file = None;
        self.daily_gateway_file = None;
        self.error_file = None;
        self.daily_error_file = None;
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

/// Set the log output directory at runtime for the global logger.
pub fn set_log_dir<P: AsRef<Path>>(dir: P) {
    default_logger().set_dir(dir);
}

/// Trigger log retention cleanup on default log directory.
pub fn cleanup_expired_logs_default() -> io::Result<usize> {
    default_logger().cleanup_logs()
}

/// Convert seconds since UNIX epoch to an ISO-8601 UTC date string (YYYY-MM-DD).
pub fn format_date(secs: u64) -> String {
    let ts = format_timestamp(secs);
    ts[..10].to_string()
}

/// Convert seconds since UNIX epoch to an ISO-8601 UTC timestamp string.
pub fn format_timestamp(secs: u64) -> String {
    let second = secs % 60;
    let minute = (secs / 60) % 60;
    let hour = (secs / 3600) % 24;
    let mut days = (secs / 86400) as i64;

    days += 719468;
    let era = (if days >= 0 { days } else { days - 146096 }) / 146097;
    let doe = (days - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hour, minute, second
    )
}

/// Clean up rotated daily logs older than `retention_days`.
pub fn cleanup_expired_logs(dir: &Path, retention_days: u64) -> io::Result<usize> {
    if !dir.is_dir() || retention_days == 0 {
        return Ok(0);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let max_age_secs = retention_days * 86400;
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Ok(metadata) = entry.metadata() {
                if let Ok(modified) = metadata.modified() {
                    let mtime = modified
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    if now > mtime && (now - mtime) > max_age_secs {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            // Only clean daily date-stamped files
                            if name.contains('-')
                                && name.ends_with(".log")
                                && fs::remove_file(&path).is_ok()
                            {
                                count += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(count)
}

/// Sanitize potential secrets (tokens, session cookies, api keys) from log messages.
pub fn sanitize_log_message(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut rest = msg;

    let patterns = [
        ("Bearer ", 7),
        ("sk-", 0),
        ("hatch_sess=", 11),
        ("datr=", 5),
    ];

    while !rest.is_empty() {
        let mut earliest: Option<(usize, &'static str, usize)> = None;
        for (pat, keep_len) in patterns {
            if let Some(pos) = rest.find(pat) {
                match earliest {
                    Some((min_pos, _, _)) if pos < min_pos => {
                        earliest = Some((pos, pat, keep_len));
                    }
                    None => {
                        earliest = Some((pos, pat, keep_len));
                    }
                    _ => {}
                }
            }
        }

        if let Some((pos, pat, keep_len)) = earliest {
            out.push_str(&rest[..pos + keep_len]);
            let after = &rest[pos + keep_len..];
            let val_len = match pat {
                "Bearer " | "sk-" => after
                    .find(|c: char| {
                        c.is_whitespace() || c == '"' || c == ',' || c == ';' || c == '&'
                    })
                    .unwrap_or(after.len()),
                _ => after
                    .find(|c: char| c.is_whitespace() || c == ';' || c == '"')
                    .unwrap_or(after.len()),
            };

            if (pat == "Bearer " || pat == "sk-") && val_len > 8 {
                let keep_prefix = if pat == "Bearer " { 4 } else { 7 };
                let prefix_len = keep_prefix.min(val_len);
                out.push_str(&after[..prefix_len]);
                out.push_str("...[MASKED]");
            } else {
                out.push_str("[MASKED]");
            }
            rest = &after[val_len..];
        } else {
            out.push_str(rest);
            break;
        }
    }
    out
}
