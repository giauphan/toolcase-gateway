//! Muse thread ID extraction, registration, and retention tests.

use super::common::*;

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
