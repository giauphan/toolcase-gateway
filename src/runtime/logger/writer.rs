//! Multi-stream log writer for gateway, error, and access logs.

use super::archive::{cleanup_expired_logs, rotate_if_needed};
use super::format::{format_date, format_timestamp, sanitize_log_message};
use super::types::LogConfig;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct LogWriter {
    pub(crate) config: LogConfig,
    current_date: String,
    gateway_file: Option<File>,
    daily_gateway_file: Option<File>,
    error_file: Option<File>,
    daily_error_file: Option<File>,
    access_file: Option<File>,
    daily_access_file: Option<File>,
}

impl LogWriter {
    pub(crate) fn new(config: LogConfig) -> Self {
        Self {
            config,
            current_date: String::new(),
            gateway_file: None,
            daily_gateway_file: None,
            error_file: None,
            daily_error_file: None,
            access_file: None,
            daily_access_file: None,
        }
    }

    fn check_date_roll(&mut self, today: &str) {
        if self.current_date != today {
            self.current_date = today.to_string();
            self.daily_gateway_file = None;
            self.daily_error_file = None;
            self.daily_access_file = None;
            let _ = cleanup_expired_logs(&self.config.dir, self.config.retention_days);
        }
    }

    pub(crate) fn write_entry(&mut self, level: &str, target: &str, msg: &str) {
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

    pub(crate) fn write_access_entry(
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

    pub(crate) fn set_dir(&mut self, dir: PathBuf) {
        self.config.dir = dir;
        self.gateway_file = None;
        self.daily_gateway_file = None;
        self.error_file = None;
        self.daily_error_file = None;
        self.access_file = None;
        self.daily_access_file = None;
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
