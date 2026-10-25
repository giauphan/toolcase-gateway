//! Muse thread ID extraction, registration, and retention tests.

use super::common::*;
use std::sync::Mutex;

static TEST_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn test_extract_new_thread_id() {
    let valid_rsc = r#"something something /thread/1234abcd-5678-efgh-ijkl-9876mnopqrst"\n"#;
    assert_eq!(
        extract_new_thread_id(valid_rsc),
        "1234abcd-5678-efgh-ijkl-9876mnopqrst"
    );

    let valid_rsc_2 = r#"/thread/some-id-1234"#;
    assert_eq!(extract_new_thread_id(valid_rsc_2), "some-id-1234");

    let empty_rsc = r#"/thread/ "#;
    assert_eq!(extract_new_thread_id(empty_rsc), "");

    let no_thread = r#"something else"#;
    assert_eq!(extract_new_thread_id(no_thread), "");
}

#[test]
fn test_thread_registration_and_retention() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let config = test_config();

    let thread_id = "test-thread-id-12345".to_string();
    register_thread(
        thread_id.clone(),
        "https://muse.ai".to_string(),
        config.clone(),
    );

    let threads = tracked_threads().lock().unwrap();
    let found = threads.iter().find(|t| t.id == thread_id);
    assert!(found.is_some());
    assert_eq!(found.unwrap().config.museai_thread_retention_secs, 86400);
}

#[test]
fn test_thread_cleanup_active_vs_expired() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let config_active = test_config();

    let mut config_expired = config_active.clone();
    config_expired.museai_thread_retention_secs = 0;

    let active_id = "thread-active-xyz".to_string();
    let expired_id = "thread-expired-abc".to_string();

    register_thread(
        active_id.clone(),
        "https://muse.ai".to_string(),
        config_active,
    );
    register_thread(
        expired_id.clone(),
        "https://muse.ai".to_string(),
        config_expired,
    );

    let deleted = cleanup_tracked_threads_once();
    assert!(deleted >= 1);

    let threads = tracked_threads().lock().unwrap();
    assert!(!threads.iter().any(|t| t.id == expired_id));
    assert!(threads.iter().any(|t| t.id == active_id));
}

#[test]
fn test_thread_persistence_and_restart_recovery() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let config = test_config();
    let thread_id = "test-persist-recover-thread-999".to_string();

    register_thread(thread_id.clone(), "https://muse.ai".to_string(), config);

    let disk_file = toolcase_gateway::museai::threads::thread_state_dir()
        .join(format!("thread-{thread_id}.json"));
    assert!(disk_file.exists());

    // Simulate process restart by clearing in-memory registry
    {
        let mut threads = tracked_threads().lock().unwrap();
        threads.retain(|t| t.id != thread_id);
        assert!(!threads.iter().any(|t| t.id == thread_id));
    }

    // Running cleanup with TTL 0 should recover from disk and delete the expired thread
    let deleted = toolcase_gateway::museai::threads::cleanup_tracked_threads_with_ttl(0);
    assert!(deleted >= 1);
    assert!(!disk_file.exists());
}

#[test]
fn test_unregister_thread_removes_from_disk_and_memory() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let config = test_config();
    let thread_id = "test-unregister-thread-888".to_string();

    register_thread(thread_id.clone(), "https://muse.ai".to_string(), config);

    let disk_file = toolcase_gateway::museai::threads::thread_state_dir()
        .join(format!("thread-{thread_id}.json"));
    assert!(disk_file.exists());

    toolcase_gateway::museai::threads::unregister_thread(&thread_id);

    assert!(!disk_file.exists());
    let threads = tracked_threads().lock().unwrap();
    assert!(!threads.iter().any(|t| t.id == thread_id));
}

#[test]
fn test_delete_cvm_session_over_noise_guards() {
    let config = test_config();
    // Empty thread ID succeeds as no-op
    let res_empty = toolcase_gateway::museai::delete_cvm_session_over_noise("", &config);
    assert!(res_empty.is_ok());

    // Empty credentials returns NotFound error without hanging or panicking
    let res_no_creds =
        toolcase_gateway::museai::delete_cvm_session_over_noise("session-123", &config);
    assert!(res_no_creds.is_err());
    assert_eq!(
        res_no_creds.unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
}

#[test]
fn test_delete_muse_thread_graceful_handling() {
    let config = test_config();
    // Empty thread ID succeeds as no-op
    let res_empty = toolcase_gateway::museai::delete_muse_thread("https://muse.ai", "", &config);
    assert!(res_empty.is_ok());

    // Non-existent or offline thread logs warning and returns Ok(())
    let res_offline = toolcase_gateway::museai::delete_muse_thread(
        "https://127.0.0.1:9999",
        "offline-thread-1",
        &config,
    );
    assert!(res_offline.is_ok());
}

#[test]
fn test_purge_remote_cvm_threads_older_than_guards() {
    let config = test_config();
    let res = toolcase_gateway::museai::purge_remote_cvm_threads_older_than(7200, &config);
    assert!(res.is_ok());
    assert_eq!(res.unwrap(), 0);
}

#[test]
fn test_cleanup_cycle_with_ttl_run() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let (_threads, _jobs) = toolcase_gateway::museai::cleanup_cycle_with_ttl(7200);
}

#[test]
fn test_cleanup_cycle_with_config_run() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let config = test_config();
    let (_threads, _jobs) = toolcase_gateway::museai::cleanup_cycle_with_config(7200, &config);
}

#[test]
fn test_extract_session_timestamp() {
    // 1. Seconds scale (< 1e11)
    let s_sec = serde_json::json!({
        "session_id": "s1",
        "created_at": 1729828800
    });
    assert_eq!(
        toolcase_gateway::museai::extract_session_timestamp(&s_sec),
        1729828800
    );

    // 2. Milliseconds scale (>= 1e11)
    let s_ms = serde_json::json!({
        "session_id": "s2",
        "created_at": 1792908302000u64
    });
    assert_eq!(
        toolcase_gateway::museai::extract_session_timestamp(&s_ms),
        1792908302
    );

    // 3. Float timestamp
    let s_float = serde_json::json!({
        "session_id": "s3",
        "created_at": 1729828800.5
    });
    assert_eq!(
        toolcase_gateway::museai::extract_session_timestamp(&s_float),
        1729828800
    );

    // 4. String timestamp
    let s_str = serde_json::json!({
        "session_id": "s4",
        "created_at": "1792908302000"
    });
    assert_eq!(
        toolcase_gateway::museai::extract_session_timestamp(&s_str),
        1792908302
    );

    // 5. Fallback to updated_at when created_at is missing
    let s_updated = serde_json::json!({
        "session_id": "s5",
        "updated_at": 1729829900
    });
    assert_eq!(
        toolcase_gateway::museai::extract_session_timestamp(&s_updated),
        1729829900
    );

    // 6. Missing timestamp returns 0
    let s_empty = serde_json::json!({
        "session_id": "s6"
    });
    assert_eq!(
        toolcase_gateway::museai::extract_session_timestamp(&s_empty),
        0
    );
}

#[test]
fn test_session_list_structure_compatibility() {
    // Ensure both nested {"ok": true, "result": {"sessions": [...]}} and top-level {"sessions": [...]} work
    let nested = serde_json::json!({
        "ok": true,
        "result": {
            "sessions": [
                {
                    "session_id": "nested-1",
                    "is_primary": false,
                    "is_thread": true,
                    "created_at": 1792908302000u64
                }
            ]
        }
    });

    let sessions_nested = nested
        .get("result")
        .and_then(|r| r.get("sessions"))
        .and_then(|s| s.as_array())
        .or_else(|| nested.get("sessions").and_then(|s| s.as_array()));

    assert!(sessions_nested.is_some());
    assert_eq!(sessions_nested.unwrap().len(), 1);

    let flat = serde_json::json!({
        "sessions": [
            {
                "session_id": "flat-1",
                "is_primary": false,
                "is_thread": true,
                "created_at": 1729828800
            }
        ]
    });

    let sessions_flat = flat
        .get("result")
        .and_then(|r| r.get("sessions"))
        .and_then(|s| s.as_array())
        .or_else(|| flat.get("sessions").and_then(|s| s.as_array()));

    assert!(sessions_flat.is_some());
    assert_eq!(sessions_flat.unwrap().len(), 1);
}

#[test]
fn test_handle_museai_threads_cleanup_run() {
    use std::io::Read;
    use std::net::{TcpListener, TcpStream};

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let client_handle = std::thread::spawn(move || {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).unwrap();
        String::from_utf8_lossy(&buf).to_string()
    });

    let (mut server_stream, _) = listener.accept().unwrap();
    let config = test_config();
    let body = br#"{"older_than_secs": 7200}"#;

    let res = toolcase_gateway::museai::handle_museai_threads_cleanup_run(
        &mut server_stream,
        &config,
        body,
    );
    assert!(res.is_ok());
    drop(server_stream);

    let client_res = client_handle.join().unwrap();
    assert!(client_res.starts_with("HTTP/1.1 200 OK"));
    assert!(client_res.contains(r#""object":"threads.cleanup""#));
    assert!(client_res.contains(r#""retention_secs":7200"#));
    assert!(client_res.contains(r#""status":"completed""#));
}
