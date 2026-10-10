pub const ACCOUNT_RETRYABLE_STATUSES: [u16; 6] = [400, 401, 402, 403, 408, 429];

pub fn is_account_retryable(status: u16) -> bool {
    ACCOUNT_RETRYABLE_STATUSES.contains(&status)
}
