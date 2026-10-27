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
    access_file: Option<File>,
    daily_access_file: Option<File>,
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
                access_file: None,
                daily_access_file: None,
            }),
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

/// Creates a standard PKZIP format archive in-memory containing a single file entry compressed with DEFLATE.
pub fn create_zip_archive(entry_name: &str, uncompressed_data: &[u8]) -> Vec<u8> {
    let mut crc = flate2::Crc::new();
    crc.update(uncompressed_data);
    let crc32 = crc.sum();

    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = encoder.write_all(uncompressed_data);
    let compressed_data = encoder.finish().unwrap_or_default();

    let mut zip = Vec::with_capacity(compressed_data.len() + 256);

    let name_bytes = entry_name.as_bytes();
    let name_len = name_bytes.len() as u16;
    let comp_len = compressed_data.len() as u32;
    let uncomp_len = uncompressed_data.len() as u32;

    // 1. Local file header
    zip.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
    zip.extend_from_slice(&20u16.to_le_bytes()); // version needed: 2.0
    zip.extend_from_slice(&0u16.to_le_bytes()); // flags
    zip.extend_from_slice(&8u16.to_le_bytes()); // compression: 8 (Deflate)
    zip.extend_from_slice(&0u16.to_le_bytes()); // mod time
    zip.extend_from_slice(&0u16.to_le_bytes()); // mod date
    zip.extend_from_slice(&crc32.to_le_bytes());
    zip.extend_from_slice(&comp_len.to_le_bytes());
    zip.extend_from_slice(&uncomp_len.to_le_bytes());
    zip.extend_from_slice(&name_len.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
    zip.extend_from_slice(name_bytes);
    zip.extend_from_slice(&compressed_data);

    let central_dir_offset = zip.len() as u32;

    // 2. Central directory file header
    zip.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
    zip.extend_from_slice(&20u16.to_le_bytes()); // version made by
    zip.extend_from_slice(&20u16.to_le_bytes()); // version needed: 2.0
    zip.extend_from_slice(&0u16.to_le_bytes()); // flags
    zip.extend_from_slice(&8u16.to_le_bytes()); // compression: 8 (Deflate)
    zip.extend_from_slice(&0u16.to_le_bytes()); // mod time
    zip.extend_from_slice(&0u16.to_le_bytes()); // mod date
    zip.extend_from_slice(&crc32.to_le_bytes());
    zip.extend_from_slice(&comp_len.to_le_bytes());
    zip.extend_from_slice(&uncomp_len.to_le_bytes());
    zip.extend_from_slice(&name_len.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
    zip.extend_from_slice(&0u16.to_le_bytes()); // comment length
    zip.extend_from_slice(&0u16.to_le_bytes()); // disk number start
    zip.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
    zip.extend_from_slice(&0u32.to_le_bytes()); // external attributes
    zip.extend_from_slice(&0u32.to_le_bytes()); // relative offset of local header
    zip.extend_from_slice(name_bytes);

    let central_dir_size = (zip.len() as u32) - central_dir_offset;

    // 3. End of central directory record (EOCD)
    zip.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
    zip.extend_from_slice(&0u16.to_le_bytes()); // disk number
    zip.extend_from_slice(&0u16.to_le_bytes()); // disk where central dir starts
    zip.extend_from_slice(&1u16.to_le_bytes()); // records on this disk
    zip.extend_from_slice(&1u16.to_le_bytes()); // total records
    zip.extend_from_slice(&central_dir_size.to_le_bytes());
    zip.extend_from_slice(&central_dir_offset.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes()); // comment length

    zip
}

fn resolve_archive_path(dir: &Path, filename: &str, zip_enabled: bool) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let date_str = format_date(now);
    let stem = filename.strip_suffix(".log").unwrap_or(filename);
    let base_stem = if extract_date_from_filename(stem).is_some() {
        stem.to_string()
    } else {
        format!("{stem}-{date_str}")
    };

    let ext = if zip_enabled { "zip" } else { "log" };
    let mut idx = 1;
    loop {
        let candidate = format!("{base_stem}-{idx}.{ext}");
        let p = dir.join(&candidate);
        if !p.exists() {
            return p;
        }
        idx += 1;
    }
}

fn write_archive_file(
    archive_path: &Path,
    filename: &str,
    data: &[u8],
    zip_enabled: bool,
) -> io::Result<()> {
    if zip_enabled {
        let zip_bytes = create_zip_archive(filename, data);
        fs::write(archive_path, &zip_bytes)
    } else {
        fs::write(archive_path, data)
    }
}

fn perform_log_rotation(target_path: &Path, dir: &Path, filename: &str, zip_enabled: bool) {
    let Ok(data) = fs::read(target_path) else {
        return;
    };
    if data.is_empty() {
        return;
    }
    let archive_path = resolve_archive_path(dir, filename, zip_enabled);
    if write_archive_file(&archive_path, filename, &data, zip_enabled).is_ok() {
        let _ = fs::remove_file(target_path);
    }
}

fn rotate_if_needed(
    file: &mut Option<File>,
    dir: &Path,
    filename: &str,
    additional_bytes: usize,
    max_size_bytes: u64,
    zip_enabled: bool,
) {
    if max_size_bytes == 0 {
        return;
    }
    let target_path = dir.join(filename);
    let current_len = file
        .as_ref()
        .and_then(|f| f.metadata().ok())
        .map(|m| m.len())
        .unwrap_or_else(|| fs::metadata(&target_path).map(|m| m.len()).unwrap_or(0));

    if current_len == 0 || current_len + (additional_bytes as u64) <= max_size_bytes {
        return;
    }

    if let Some(mut f) = file.take() {
        let _ = f.flush();
    }
    perform_log_rotation(&target_path, dir, filename, zip_enabled);
}

fn write_line(
    file: &mut Option<File>,
    dir: &Path,
    filename: &str,
    line: &str,
    max_size_bytes: u64,
    zip_enabled: bool,
) {
    rotate_if_needed(file, dir, filename, line.len(), max_size_bytes, zip_enabled);
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
            self.daily_access_file = None;
            let _ = cleanup_expired_logs(&self.config.dir, self.config.retention_days);
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
            let max_bytes = self.config.max_size_bytes;
            let zip_en = self.config.zip_enabled;

            // 1. General gateway logs
            let main_name = self.config.file_name.clone();
            write_line(
                &mut self.gateway_file,
                &self.config.dir,
                &main_name,
                &line,
                max_bytes,
                zip_en,
            );
            if self.config.daily {
                let daily_name = format!("gateway-{today}.log");
                write_line(
                    &mut self.daily_gateway_file,
                    &self.config.dir,
                    &daily_name,
                    &line,
                    max_bytes,
                    zip_en,
                );
            }

            // 2. Error streams for WARN and ERROR
            if level.eq_ignore_ascii_case("warn") || level.eq_ignore_ascii_case("error") {
                write_line(
                    &mut self.error_file,
                    &self.config.dir,
                    "error.log",
                    &line,
                    max_bytes,
                    zip_en,
                );
                if self.config.daily {
                    let daily_err = format!("error-{today}.log");
                    write_line(
                        &mut self.daily_error_file,
                        &self.config.dir,
                        &daily_err,
                        &line,
                        max_bytes,
                        zip_en,
                    );
                }
            }
        }
    }

    fn write_access_entry(
        &mut self,
        method: &str,
        path: &str,
        client_ip: &str,
        status: Option<u16>,
        duration_ms: Option<u64>,
    ) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let ts = format_timestamp(now);
        let today = format_date(now);
        self.check_date_roll(&today);

        let sanitized_path = sanitize_log_message(path);
        let line = match (status, duration_ms) {
            (Some(s), Some(d)) => {
                format!("[{ts}] [ACCESS] {client_ip} \"{method} {sanitized_path}\" {s} {d}ms\n")
            }
            (Some(s), None) => {
                format!("[{ts}] [ACCESS] {client_ip} \"{method} {sanitized_path}\" {s}\n")
            }
            _ => format!("[{ts}] [ACCESS] {client_ip} \"{method} {sanitized_path}\"\n"),
        };

        if self.config.console {
            eprint!("{line}");
        }

        if self.config.enabled {
            let max_bytes = self.config.max_size_bytes;
            let zip_en = self.config.zip_enabled;

            // 1. Access logs
            write_line(
                &mut self.access_file,
                &self.config.dir,
                "access.log",
                &line,
                max_bytes,
                zip_en,
            );
            if self.config.daily {
                let daily_acc = format!("access-{today}.log");
                write_line(
                    &mut self.daily_access_file,
                    &self.config.dir,
                    &daily_acc,
                    &line,
                    max_bytes,
                    zip_en,
                );
            }

            // 2. Mirror to gateway log
            let main_name = self.config.file_name.clone();
            write_line(
                &mut self.gateway_file,
                &self.config.dir,
                &main_name,
                &line,
                max_bytes,
                zip_en,
            );
            if self.config.daily {
                let daily_name = format!("gateway-{today}.log");
                write_line(
                    &mut self.daily_gateway_file,
                    &self.config.dir,
                    &daily_name,
                    &line,
                    max_bytes,
                    zip_en,
                );
            }
        }
    }

    fn set_dir(&mut self, dir: PathBuf) {
        self.config.dir = dir;
        self.gateway_file = None;
        self.daily_gateway_file = None;
        self.error_file = None;
        self.daily_error_file = None;
        self.access_file = None;
        self.daily_access_file = None;
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

pub fn extract_date_from_filename(name: &str) -> Option<&str> {
    let bytes = name.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    for i in 0..=bytes.len() - 10 {
        let sub = &bytes[i..i + 10];
        if sub[4] == b'-'
            && sub[7] == b'-'
            && sub[0..4].iter().all(u8::is_ascii_digit)
            && sub[5..7].iter().all(u8::is_ascii_digit)
            && sub[8..10].iter().all(u8::is_ascii_digit)
        {
            return Some(&name[i..i + 10]);
        }
    }
    None
}

pub fn parse_date_to_epoch_days(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if s.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (
        s[0..4].parse().ok()?,
        s[5..7].parse().ok()?,
        s[8..10].parse().ok()?,
    );
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || y < 1970 {
        return None;
    }
    let y_adj = if m <= 2 { y - 1 } else { y };
    let era = (if y_adj >= 0 { y_adj } else { y_adj - 399 }) / 400;
    let yoe = (y_adj - era * 400) as u64;
    let m_adj = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * m_adj + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy as u64;
    let days = era * 146097 + doe as i64 - 719468;
    (days >= 0).then_some(days as u64)
}

/// Clean up rotated daily logs and zip archives older than `retention_days`.
pub fn cleanup_expired_logs(dir: &Path, retention_days: u64) -> io::Result<usize> {
    if !dir.is_dir() || retention_days == 0 {
        return Ok(0);
    }
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (now_days, max_age_secs) = (now_secs / 86400, retention_days * 86400);
    let mut count = 0;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(std::ffi::OsStr::to_str) else {
                continue;
            };
            let is_log = name.ends_with(".log");
            let is_zip = name.ends_with(".zip");
            if !is_log && !is_zip {
                continue;
            }
            // Never clean primary active log files like gateway.log, error.log, access.log
            if name == "gateway.log" || name == "error.log" || name == "access.log" {
                continue;
            }
            let Some(date_str) = extract_date_from_filename(name) else {
                continue;
            };

            let by_date = parse_date_to_epoch_days(date_str)
                .is_some_and(|d| now_days > d && now_days - d >= retention_days);
            let by_mtime = entry.metadata().and_then(|m| m.modified()).is_ok_and(|m| {
                let mtime = m.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                now_secs > mtime && now_secs - mtime >= max_age_secs
            });

            if (by_date || by_mtime) && fs::remove_file(&path).is_ok() {
                count += 1;
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
