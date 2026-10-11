use crate::config::Config;
use crate::http::{header_value, Request};
use std::io;
use std::net::TcpStream;
use std::time::Duration;

pub const RETRYABLE: [u16; 11] = [400, 401, 402, 403, 408, 429, 500, 502, 503, 504, 524];

/// Pure gateway decision: check if an HTTP status code is retryable.
pub fn is_status_retryable(status: u16) -> bool {
    crate::gateway::retry::is_safe_to_retry(status)
}

pub fn parse_retry_after(headers: &[(String, String)]) -> Option<Duration> {
    let value = header_value(headers, "retry-after")?;
    let trimmed = value.trim();
    if let Ok(seconds) = trimmed.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    None
}

pub fn calculate_retry_delay(
    attempt: usize,
    base_delay_ms: u64,
    max_delay_ms: u64,
    headers: Option<&[(String, String)]>,
) -> Duration {
    if let Some(hdrs) = headers {
        if let Some(delay) = parse_retry_after(hdrs) {
            let max_dur = Duration::from_millis(max_delay_ms);
            return if delay > max_dur { max_dur } else { delay };
        }
    }
    let multiplier = 1u64.checked_shl(attempt as u32).unwrap_or(u64::MAX);
    let computed_ms = base_delay_ms.saturating_mul(multiplier);
    let final_ms = computed_ms.min(max_delay_ms);
    Duration::from_millis(final_ms)
}

pub fn open_upstream(
    client: &TcpStream,
    request: &Request,
    config: &Config,
    model: &str,
) -> io::Result<TcpStream> {
    crate::providers::omniroute::OmniRouteAdapter::open_upstream(client, request, config, model)
}

pub fn handle_omniroute_proxy(
    client: &mut TcpStream,
    request: &Request,
    config: &Config,
) -> io::Result<()> {
    crate::features::proxy::ProxyService::forward(client, request, config)
}

pub fn candidate_models(
    body: &[u8],
    fallbacks: &[String],
    rotation: usize,
    default_model: &str,
) -> Vec<String> {
    crate::gateway::candidates::plan_candidates(body, fallbacks, rotation, default_model)
}
