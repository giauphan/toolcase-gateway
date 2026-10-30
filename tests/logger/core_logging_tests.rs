use std::fs;
use std::sync::Arc;
use std::thread;
use toolcase_gateway::runtime::logger::{
    cleanup_expired_logs, format_date, format_timestamp, sanitize_log_message, LogConfig, Logger,
};

#[test]
fn test_timestamp_and_date_formatting() {
    assert_eq!(format_timestamp(0), "1970-01-01T00:00:00Z");
    assert_eq!(format_date(0), "1970-01-01");

    let ts = format_timestamp(1792641171);
    assert!(ts.starts_with("2026-"));
    assert!(ts.ends_with('Z'));
    assert_eq!(ts.len(), 20);

    let d = format_date(1792641171);
    assert_eq!(d.len(), 10);
    assert!(ts.starts_with(&d));
}

#[test]
fn test_sanitize_log_message() {
    let raw =
        "User with Bearer secret-token-12345678 and key sk-glm5-00bc9802bf5fd3545 tried logging in";
    let sanitized = sanitize_log_message(raw);
    assert!(!sanitized.contains("secret-token-12345678"));
    assert!(!sanitized.contains("00bc9802bf5fd3545"));
    assert!(sanitized.contains("[MASKED]"));

    let cookie_raw = "Cookie: datr=fiDHauAIUxote5JFvHFGpMb0; hatch_sess=6ccee67c-06e0-47d5-b5b1-88f615c7c9b3; other=value";
    let sanitized_cookie = sanitize_log_message(cookie_raw);
    assert!(!sanitized_cookie.contains("fiDHauAIUxote5JFvHFGpMb0"));
    assert!(!sanitized_cookie.contains("6ccee67c-06e0-47d5-b5b1-88f615c7c9b3"));
    assert!(sanitized_cookie.contains("datr=[MASKED]"));
    assert!(sanitized_cookie.contains("hatch_sess=[MASKED]"));
    assert!(sanitized_cookie.contains("other=value"));
}

#[test]
fn test_framework_multi_stream_logging() {
    let test_dir = std::env::temp_dir().join(format!("gw_multi_log_test_{}", uuid::Uuid::new_v4()));
    let logger = Logger::new(LogConfig {
        enabled: true,
        console: false,
        daily: true,
        dir: test_dir.clone(),
        file_name: "gateway.log".to_string(),
        retention_days: 14,
        max_size_bytes: 10 * 1024 * 1024,
        zip_enabled: true,
    });

    let today = format_date(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    );

    // 1. Info log
    logger.log("INFO", "server", "Server successfully bound on port 20129");
    let gw_file = test_dir.join("gateway.log");
    let daily_gw_file = test_dir.join(format!("gateway-{today}.log"));
    assert!(gw_file.is_file(), "gateway.log must exist");
    assert!(daily_gw_file.is_file(), "gateway-YYYY-MM-DD.log must exist");
    let gw_content = fs::read_to_string(&gw_file).unwrap();
    assert!(gw_content.contains("[INFO]"));
    assert!(gw_content.contains("Server successfully bound on port 20129"));

    // 2. Error and Warning logs
    logger.log("WARN", "proxy", "Upstream target returned 429 slow down");
    logger.log("ERROR", "handler", "Database connection reset");
    let err_file = test_dir.join("error.log");
    let daily_err_file = test_dir.join(format!("error-{today}.log"));
    assert!(err_file.is_file(), "error.log must exist");
    assert!(daily_err_file.is_file(), "error-YYYY-MM-DD.log must exist");
    let err_content = fs::read_to_string(&err_file).unwrap();
    assert!(err_content.contains("[WARN]"));
    assert!(err_content.contains("[ERROR]"));
    assert!(err_content.contains("Upstream target returned 429"));
    assert!(err_content.contains("Database connection reset"));
    // Info message should NOT be in error.log
    assert!(!err_content.contains("Server successfully bound"));

    // 3. Access log
    logger.log_access("POST", "/muse-ai/v1/create-video", "192.168.1.100");
    let acc_file = test_dir.join("access.log");
    let daily_acc_file = test_dir.join(format!("access-{today}.log"));
    assert!(acc_file.is_file(), "access.log must exist");
    assert!(daily_acc_file.is_file(), "access-YYYY-MM-DD.log must exist");
    let acc_content = fs::read_to_string(&acc_file).unwrap();
    assert!(acc_content.contains("[ACCESS]"));
    assert!(acc_content.contains("POST /muse-ai/v1/create-video"));
    assert!(acc_content.contains("192.168.1.100"));

    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_cleanup_expired_daily_logs() {
    let test_dir =
        std::env::temp_dir().join(format!("gw_cleanup_log_test_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&test_dir).unwrap();

    let today = format_date(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    );

    // Active primary files
    let active_log = test_dir.join("gateway.log");
    let active_err = test_dir.join("error.log");
    let active_acc = test_dir.join("access.log");
    fs::write(&active_log, b"active content").unwrap();
    fs::write(&active_err, b"active errors").unwrap();
    fs::write(&active_acc, b"active access").unwrap();

    // Today's daily logs (must be preserved)
    let today_gw = test_dir.join(format!("gateway-{today}.log"));
    let today_acc = test_dir.join(format!("access-{today}.log"));
    fs::write(&today_gw, b"today gateway").unwrap();
    fs::write(&today_acc, b"today access").unwrap();

    // Old daily logs from >2 days ago (e.g. 2020-01-01) (must be purged)
    let old_daily_gw = test_dir.join("gateway-2020-01-01.log");
    let old_daily_acc = test_dir.join("access-2020-01-01.log");
    let old_daily_err = test_dir.join("error-2020-01-01.log");
    fs::write(&old_daily_gw, b"old content").unwrap();
    fs::write(&old_daily_acc, b"old access").unwrap();
    fs::write(&old_daily_err, b"old error").unwrap();

    // With retention 0, nothing is cleaned
    assert_eq!(cleanup_expired_logs(&test_dir, 0).unwrap(), 0);
    assert!(old_daily_gw.is_file());
    assert!(old_daily_acc.is_file());
    assert!(old_daily_err.is_file());

    // With retention 2, the 3 old daily logs are cleaned up
    let cleaned = cleanup_expired_logs(&test_dir, 2).unwrap();
    assert_eq!(cleaned, 3);
    assert!(!old_daily_gw.exists());
    assert!(!old_daily_acc.exists());
    assert!(!old_daily_err.exists());

    // Active primary log files and today's daily files must never be deleted
    assert!(active_log.is_file());
    assert!(active_err.is_file());
    assert!(active_acc.is_file());
    assert!(today_gw.is_file());
    assert!(today_acc.is_file());

    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_logger_concurrent_writes() {
    let test_dir =
        std::env::temp_dir().join(format!("gw_log_concurrent_test_{}", uuid::Uuid::new_v4()));
    let logger = Arc::new(Logger::new(LogConfig {
        enabled: true,
        console: false,
        daily: true,
        dir: test_dir.clone(),
        file_name: "gateway.log".to_string(),
        retention_days: 2,
        max_size_bytes: 10 * 1024 * 1024,
        zip_enabled: true,
    }));

    let thread_count = 10;
    let iterations_per_thread = 30;
    let mut handles = Vec::new();

    for t in 0..thread_count {
        let logger_clone = Arc::clone(&logger);
        let handle = thread::spawn(move || {
            for i in 0..iterations_per_thread {
                if i % 3 == 0 {
                    logger_clone.log(
                        "ERROR",
                        &format!("worker_{t}"),
                        &format!("thread iteration error {i}"),
                    );
                } else if i % 3 == 1 {
                    logger_clone.log(
                        "INFO",
                        &format!("worker_{t}"),
                        &format!("thread iteration info {i}"),
                    );
                } else {
                    logger_clone.log_access(
                        "GET",
                        &format!("/api/test/{i}"),
                        &format!("10.0.0.{t}"),
                    );
                }
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().unwrap();
    }

    let gw_file = test_dir.join("gateway.log");
    let err_file = test_dir.join("error.log");
    let acc_file = test_dir.join("access.log");

    assert!(gw_file.is_file());
    assert!(err_file.is_file());
    assert!(acc_file.is_file());

    let gw_lines = fs::read_to_string(&gw_file).unwrap().lines().count();
    let err_lines = fs::read_to_string(&err_file).unwrap().lines().count();
    let acc_lines = fs::read_to_string(&acc_file).unwrap().lines().count();

    assert_eq!(gw_lines, thread_count * iterations_per_thread);
    assert_eq!(err_lines, thread_count * (iterations_per_thread / 3));
    assert_eq!(acc_lines, thread_count * (iterations_per_thread / 3));

    let _ = fs::remove_dir_all(test_dir);
}
