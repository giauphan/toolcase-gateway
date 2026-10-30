use std::fs;
use std::process::Command;
use std::thread;
use toolcase_gateway::runtime::logger::{
    cleanup_expired_logs, create_zip_archive, format_date, LogConfig, Logger,
    DEFAULT_LOG_MAX_SIZE_BYTES,
};

#[test]
fn test_default_10mb_threshold_constants() {
    assert_eq!(DEFAULT_LOG_MAX_SIZE_BYTES, 10 * 1024 * 1024);
    let cfg = LogConfig::default();
    assert_eq!(cfg.max_size_bytes, 10 * 1024 * 1024);
    assert!(cfg.zip_enabled);
}

#[test]
fn test_create_zip_archive_format_integrity() {
    let tmp_dir = std::env::temp_dir().join(format!("gw_zip_test_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&tmp_dir).unwrap();

    let sample_text =
        "2026-10-23T11:00:00Z [INFO] [server] Hello test log content line!\n".repeat(500);
    let raw_bytes = sample_text.as_bytes();

    let zip_bytes = create_zip_archive("test_gateway.log", raw_bytes);
    assert!(!zip_bytes.is_empty());
    // Zip file signature PK\x03\x04
    assert_eq!(&zip_bytes[0..4], &[0x50, 0x4b, 0x03, 0x04]);

    let zip_path = tmp_dir.join("test.zip");
    fs::write(&zip_path, &zip_bytes).unwrap();

    // Verify integrity using system unzip
    let test_status = Command::new("unzip").arg("-t").arg(&zip_path).status();
    if let Ok(st) = test_status {
        assert!(st.success(), "unzip -t integrity test must pass");
    }

    // Verify extracted content matches exactly
    let out = Command::new("unzip").arg("-p").arg(&zip_path).output();
    if let Ok(extracted) = out {
        assert_eq!(extracted.stdout, raw_bytes);
    }

    let _ = fs::remove_dir_all(tmp_dir);
}

#[test]
fn test_log_rotation_and_zip_compression_when_exceeding_threshold() {
    let test_dir =
        std::env::temp_dir().join(format!("gw_log_rotation_test_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&test_dir).unwrap();

    // Set max_size_bytes to a small threshold for testing: 30 KB
    let threshold = 30 * 1024;
    let logger = Logger::new(LogConfig {
        enabled: true,
        console: false,
        daily: false,
        dir: test_dir.clone(),
        file_name: "gateway.log".to_string(),
        retention_days: 2,
        max_size_bytes: threshold,
        zip_enabled: true,
    });

    let line = "A".repeat(400); // ~400 bytes per log line

    // Write ~40 KB of logs (exceeding 30 KB threshold)
    for i in 0..100 {
        logger.log("INFO", "worker", &format!("line {i}: {line}"));
    }

    // Verify at least one .zip archive was created in test_dir
    let mut zip_count = 0;
    let mut zip_files = Vec::new();
    for entry in fs::read_dir(&test_dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".zip") {
            zip_count += 1;
            zip_files.push(entry.path());
        }
    }
    assert!(
        zip_count >= 1,
        "At least one .zip archive must be generated"
    );

    // Verify the zip archive can be extracted and contains uncompressed logs
    let out = Command::new("unzip")
        .arg("-p")
        .arg(&zip_files[0])
        .output()
        .unwrap();
    assert!(out.status.success());
    let extracted_str = String::from_utf8_lossy(&out.stdout);
    assert!(extracted_str.contains("line 0:"));

    // Verify current active gateway.log exists and is smaller than threshold
    let active_gw = test_dir.join("gateway.log");
    assert!(active_gw.is_file());
    let active_size = fs::metadata(&active_gw).unwrap().len();
    assert!(active_size < threshold);

    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_cleanup_expired_zip_archives() {
    let test_dir =
        std::env::temp_dir().join(format!("gw_cleanup_zip_test_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&test_dir).unwrap();

    let today = format_date(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    );

    // 1. Active files (must NOT be deleted)
    let active_log = test_dir.join("gateway.log");
    fs::write(&active_log, b"active").unwrap();

    // 2. Today's zip archive (must NOT be deleted)
    let today_zip = test_dir.join(format!("gateway-{today}-1.zip"));
    fs::write(&today_zip, b"PK\x03\x04...").unwrap();

    // 3. Old expired zip archives from 2020 (must be purged)
    let old_zip_1 = test_dir.join("gateway-2020-01-01-1.zip");
    let old_zip_2 = test_dir.join("access-2020-01-01-1.zip");
    fs::write(&old_zip_1, b"PK\x03\x04...").unwrap();
    fs::write(&old_zip_2, b"PK\x03\x04...").unwrap();

    // Clean up with retention = 2 days
    let cleaned = cleanup_expired_logs(&test_dir, 2).unwrap();
    assert_eq!(cleaned, 2);

    assert!(active_log.is_file());
    assert!(today_zip.is_file());
    assert!(!old_zip_1.exists());
    assert!(!old_zip_2.exists());

    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_log_access_with_status_and_duration() {
    let test_dir =
        std::env::temp_dir().join(format!("gw_access_status_test_{}", uuid::Uuid::new_v4()));
    let logger = Logger::new(LogConfig {
        enabled: true,
        console: false,
        daily: false,
        dir: test_dir.clone(),
        file_name: "gateway.log".to_string(),
        retention_days: 2,
        max_size_bytes: 10 * 1024 * 1024,
        zip_enabled: true,
    });

    // 1. Log with status and duration
    logger.log_access_with_status("POST", "/v1/responses", "127.0.0.1", 200, 45);
    logger.log_access_with_status("POST", "/v1/chat/completions", "192.168.1.50", 502, 120);

    let acc_file = test_dir.join("access.log");
    assert!(acc_file.is_file());
    let content = fs::read_to_string(&acc_file).unwrap();

    assert!(content.contains(r#"[ACCESS] 127.0.0.1 "POST /v1/responses" 200 45ms"#));
    assert!(content.contains(r#"[ACCESS] 192.168.1.50 "POST /v1/chat/completions" 502 120ms"#));

    // Also mirrored into gateway.log
    let gw_file = test_dir.join("gateway.log");
    assert!(gw_file.is_file());
    let gw_content = fs::read_to_string(&gw_file).unwrap();
    assert!(gw_content.contains(r#"[ACCESS] 127.0.0.1 "POST /v1/responses" 200 45ms"#));

    // 2. Legacy log without status preserves backwards-compatible format
    logger.log_access("GET", "/models", "10.0.0.1");
    let content_after = fs::read_to_string(&acc_file).unwrap();
    assert!(content_after.contains(r#"[ACCESS] 10.0.0.1 "GET /models""#));

    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_route_request_access_status_and_proxy_logs() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use toolcase_gateway::config::{Config, ConfigStore};
    use toolcase_gateway::routes::route_request;
    use toolcase_gateway::runtime::logger::set_log_dir;

    let test_dir = std::env::temp_dir().join(format!("gw_proxy_log_test_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&test_dir).unwrap();
    set_log_dir(&test_dir);

    // Mock upstream listener
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();

    let client_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client_port = client_listener.local_addr().unwrap().port();

    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: upstream_port,
        default_model: "mock-model".into(),
        fallbacks: vec![],
        retry_base_delay_ms: 0,
        max_retry_delay_ms: 0,
        ..Config::default()
    };

    let store = ConfigStore::new(config, std::path::PathBuf::from(".env"));

    // Server thread running route_request
    let store_clone = store.clone();
    let gateway_handle = thread::spawn(move || {
        let (client, _) = client_listener.accept().unwrap();
        route_request(client, &store_clone).unwrap();
    });

    // Upstream server thread
    let upstream_handle = thread::spawn(move || {
        let (mut socket, _) = upstream_listener.accept().unwrap();
        let upstream_req = toolcase_gateway::http::read_request(&mut socket).unwrap();
        assert_eq!(upstream_req.path, "/v1/responses");

        let resp = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 17\r\nConnection: close\r\n\r\n{\"output\":\"hello\"}";
        socket.write_all(resp).unwrap();
        socket.flush().unwrap();
    });

    // Client makes request to /ominroute/v1/responses
    let mut client = TcpStream::connect(("127.0.0.1", client_port)).unwrap();
    let req = b"POST /ominroute/v1/responses HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: 38\r\nConnection: close\r\n\r\n{\"model\":\"mock-model\",\"input\":\"hello\"}";
    client.write_all(req).unwrap();
    client.flush().unwrap();

    let mut response_buf = Vec::new();
    client.read_to_end(&mut response_buf).unwrap();
    let resp_str = String::from_utf8_lossy(&response_buf);
    assert!(resp_str.contains("200 OK"));

    gateway_handle.join().unwrap();
    upstream_handle.join().unwrap();

    // Verify access.log records status code and duration
    let acc_file = test_dir.join("access.log");
    assert!(acc_file.is_file(), "access.log must exist");
    let acc_content = fs::read_to_string(&acc_file).unwrap();
    assert!(
        acc_content.contains(r#""POST /ominroute/v1/responses" 200"#),
        "access.log must record response status 200: {acc_content}"
    );
    assert!(
        acc_content.contains("ms"),
        "access.log must record elapsed duration in ms: {acc_content}"
    );

    // Verify gateway.log contains structured proxy execution flow logs
    let gw_file = test_dir.join("gateway.log");
    assert!(gw_file.is_file(), "gateway.log must exist");
    let gw_content = fs::read_to_string(&gw_file).unwrap();
    assert!(
        gw_content
            .contains("[INFO] [gateway_proxy] Routing request /v1/responses to model 'mock-model'"),
        "gateway.log must contain routing log: {gw_content}"
    );
    assert!(
        gw_content.contains("[INFO] [gateway_proxy] Forwarding request /v1/responses to upstream"),
        "gateway.log must contain forwarding log: {gw_content}"
    );
    assert!(
        gw_content.contains(
            "Upstream model 'mock-model' responded with HTTP 200 OK for path /v1/responses"
        ),
        "gateway.log must contain upstream response log: {gw_content}"
    );

    let _ = fs::remove_dir_all(test_dir);
}
