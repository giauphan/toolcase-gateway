//! Integration tests for the Jev AI multi-account pool and retryable-status logic.

use toolcase_gateway::jev::business::is_account_retryable;
use toolcase_gateway::jev::pool::JevAccountPool;

#[test]
fn test_jev_account_pool_round_robin() {
    let keys = vec!["key1".to_string(), "key2".to_string(), "key3".to_string()];
    let pool = JevAccountPool::new(&keys);
    assert_eq!(pool.len(), 3);
    assert!(!pool.is_empty());

    let candidates1 = pool.candidate_accounts();
    assert_eq!(candidates1.len(), 3);
    assert_eq!(candidates1[0].api_key, "key1");
    assert_eq!(candidates1[1].api_key, "key2");
    assert_eq!(candidates1[2].api_key, "key3");

    let candidates2 = pool.candidate_accounts();
    assert_eq!(candidates2[0].api_key, "key2");
    assert_eq!(candidates2[1].api_key, "key3");
    assert_eq!(candidates2[2].api_key, "key1");
}

#[test]
fn test_jev_account_pool_empty() {
    let pool = JevAccountPool::new(&[]);
    assert!(pool.is_empty());
    assert_eq!(pool.len(), 0);
    assert!(pool.next_account().is_none());
    assert!(pool.candidate_accounts().is_empty());
}

#[test]
fn test_account_retryable_statuses() {
    assert!(is_account_retryable(401));
    assert!(is_account_retryable(402));
    assert!(is_account_retryable(403));
    assert!(is_account_retryable(429));
    assert!(!is_account_retryable(200));
    assert!(!is_account_retryable(404));
    assert!(!is_account_retryable(500));
}
