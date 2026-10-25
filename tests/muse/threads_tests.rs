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
