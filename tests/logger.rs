use std::fs;
use std::sync::Arc;
use std::thread;
use toolcase_gateway::runtime::logger::{
    format_timestamp, sanitize_log_message, LogConfig, Logger,
};

#[test]
fn test_timestamp_formatting() {
    // 0 is 1970-01-01T00:00:00Z
    assert_eq!(format_timestamp(0), "1970-01-01T00:00:00Z");

    // 1792641171 (from previous session timestamps in 2026)
    let ts = format_timestamp(1792641171);
    assert!(ts.starts_with("2026-"));
    assert!(ts.ends_with('Z'));
    assert_eq!(ts.len(), 20);
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
fn test_logger_creates_directory_and_writes_file() {
    let test_dir = std::env::temp_dir().join(format!("gw_log_test_{}", uuid::Uuid::new_v4()));
    let logger = Logger::new(LogConfig {
        enabled: true,
        console: false,
        dir: test_dir.clone(),
        file_name: "gateway.log".to_string(),
    });

    let test_msg = "Gateway server test initialization message";
    logger.log("INFO", "test_target", test_msg);

    let log_file = test_dir.join("gateway.log");
    assert!(
        log_file.is_file(),
        "Expected log file to exist at {:?}",
        log_file
    );

    let content = fs::read_to_string(&log_file).unwrap();
    assert!(content.contains("[INFO]"));
    assert!(content.contains("[test_target]"));
    assert!(content.contains(test_msg));

    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_logger_concurrent_writes() {
    let test_dir =
        std::env::temp_dir().join(format!("gw_log_concurrent_test_{}", uuid::Uuid::new_v4()));
    let logger = Arc::new(Logger::new(LogConfig {
        enabled: true,
        console: false,
        dir: test_dir.clone(),
        file_name: "gateway.log".to_string(),
    }));

    let thread_count = 10;
    let iterations_per_thread = 20;
    let mut handles = Vec::new();

    for t in 0..thread_count {
        let logger_clone = Arc::clone(&logger);
        let handle = thread::spawn(move || {
            for i in 0..iterations_per_thread {
                logger_clone.log(
                    "DEBUG",
                    &format!("worker_{t}"),
                    &format!("thread iteration message {i}"),
                );
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().unwrap();
    }

    let log_file = test_dir.join("gateway.log");
    assert!(log_file.is_file());

    let content = fs::read_to_string(&log_file).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), thread_count * iterations_per_thread);

    let _ = fs::remove_dir_all(test_dir);
}
