//! Thread-safe file and console logging infrastructure.
#![forbid(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct LogConfig {
    pub enabled: bool,
    pub console: bool,
    pub dir: PathBuf,
    pub file_name: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        let enabled = std::env::var("GW_LOG_ENABLED")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(true);
        let console = std::env::var("GW_LOG_CONSOLE")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(true);
        let dir = PathBuf::from(std::env::var("GW_LOG_DIR").unwrap_or_else(|_| "logs".to_string()));
        let file_name = std::env::var("GW_LOG_FILE").unwrap_or_else(|_| "gateway.log".to_string());

        Self {
            enabled,
            console,
            dir,
            file_name,
        }
    }
}

pub struct Logger {
    inner: Mutex<LogWriter>,
}

struct LogWriter {
    config: LogConfig,
    file: Option<File>,
}

impl Logger {
    pub fn new(config: LogConfig) -> Self {
        Self {
            inner: Mutex::new(LogWriter { config, file: None }),
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
}

impl LogWriter {
    fn write_entry(&mut self, level: &str, target: &str, msg: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let ts = format_timestamp(now);
        let sanitized = sanitize_log_message(msg);
        let line = format!("[{ts}] [{level}] [{target}] {sanitized}\n");

        if self.config.console {
            eprint!("{line}");
        }

        if self.config.enabled {
            self.write_to_file(&line);
        }
    }

    fn write_to_file(&mut self, line: &str) {
        if self.file.is_none() {
            if let Err(e) = fs::create_dir_all(&self.config.dir) {
                eprintln!(
                    "[logger] failed to create log dir {:?}: {e}",
                    self.config.dir
                );
                return;
            }
            let file_path = self.config.dir.join(&self.config.file_name);
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file_path)
            {
                Ok(f) => self.file = Some(f),
                Err(e) => {
                    eprintln!("[logger] failed to open log file {:?}: {e}", file_path);
                    return;
                }
            }
        }

        if let Some(ref mut f) = self.file {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
        }
    }

    fn set_dir(&mut self, dir: PathBuf) {
        self.config.dir = dir;
        self.file = None;
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

/// Convert seconds since UNIX epoch to an ISO-8601 UTC string.
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
