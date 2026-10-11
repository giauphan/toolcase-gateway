pub const ACCOUNT_RETRYABLE_STATUSES: [u16; 4] = [401, 402, 403, 429];
pub const TRANSIENT_RETRYABLE_STATUSES: [u16; 4] = [499, 502, 503, 504];

pub fn is_account_retryable(status: u16) -> bool {
    ACCOUNT_RETRYABLE_STATUSES.contains(&status)
}

pub fn is_transient_retryable(status: u16) -> bool {
    TRANSIENT_RETRYABLE_STATUSES.contains(&status)
}

pub fn is_retryable(status: u16) -> bool {
    is_account_retryable(status) || is_transient_retryable(status)
}
