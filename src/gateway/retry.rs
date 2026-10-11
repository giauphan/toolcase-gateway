use std::time::Duration;

/// V4 Pure Gateway Decision: determines if an HTTP status code is safe for replay/failover.
/// Forbids blind retries of 400 (Bad Request) which are deterministic client errors,
/// while permitting transient server errors (500, 502, 503, 504), rate limits (429),
/// and account credential/credit expiration (401, 402, 403).
pub fn is_safe_to_retry(status: u16) -> bool {
    matches!(
        status,
        401 | 402 | 403 | 408 | 429 | 499 | 500 | 502 | 503 | 504 | 524
    )
}

pub fn calculate_exponential_backoff(
    attempt: usize,
    base_delay_ms: u64,
    max_delay_ms: u64,
) -> Duration {
    let multiplier = 1u64.checked_shl(attempt as u32).unwrap_or(u64::MAX);
    let computed_ms = base_delay_ms.saturating_mul(multiplier);
    let final_ms = computed_ms.min(max_delay_ms);
    Duration::from_millis(final_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bad_request_not_retryable() {
        assert!(!is_safe_to_retry(400));
        assert!(!is_safe_to_retry(404));
        assert!(!is_safe_to_retry(405));
    }

    #[test]
    fn test_transient_and_auth_retryable() {
        assert!(is_safe_to_retry(401));
        assert!(is_safe_to_retry(429));
        assert!(is_safe_to_retry(500));
        assert!(is_safe_to_retry(502));
        assert!(is_safe_to_retry(503));
    }
}
