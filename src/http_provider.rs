use crate::http::write_error;
use crate::omniroute::calculate_retry_delay;
use std::io::{self, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateKind {
    Direct,
    OpenRouterApiKey,
    OpenRouterBrowser,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailoverCandidate {
    pub api_key: String,
    pub label: String,
    pub kind: CandidateKind,
    pub model: Option<String>,
}

impl FailoverCandidate {
    pub fn direct(api_key: String, label: String) -> Self {
        Self {
            api_key,
            label,
            kind: CandidateKind::Direct,
            model: None,
        }
    }

    pub fn openrouter(cookie: String, label: String) -> Self {
        Self {
            api_key: cookie,
            label,
            kind: CandidateKind::OpenRouterBrowser,
            model: None,
        }
    }

    pub fn openrouter_api_key(api_key: String, label: String, model: Option<String>) -> Self {
        Self {
            api_key,
            label,
            kind: CandidateKind::OpenRouterApiKey,
            model,
        }
    }

    pub fn openrouter_browser(cookie: String, label: String, model: Option<String>) -> Self {
        Self {
            api_key: cookie,
            label,
            kind: CandidateKind::OpenRouterBrowser,
            model,
        }
    }
}

pub struct FailoverConfig {
    pub timeout: Option<Duration>,
    pub retry_base_delay_ms: u64,
    pub max_retry_delay_ms: u64,
}

pub fn write_ureq_response(
    client: &mut TcpStream,
    response: ureq::http::Response<ureq::Body>,
) -> io::Result<()> {
    let status = response.status().as_u16();
    let reason = response
        .status()
        .canonical_reason()
        .unwrap_or("Upstream Response");
    let body_bytes = match response.into_body().read_to_vec() {
        Ok(body) => body,
        Err(error) => {
            return write_error(
                client,
                502,
                "Bad Gateway",
                &format!("toolcase-gateway: upstream response read failed: {error}"),
            );
        }
    };
    write!(
        client,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body_bytes.len()
    )?;
    client.write_all(&body_bytes)?;
    client.flush()
}

pub fn execute_http_failover<F, R>(
    client: &mut TcpStream,
    candidates: &[FailoverCandidate],
    config: &FailoverConfig,
    provider_name: &str,
    is_retryable_status: F,
    make_request: R,
) -> io::Result<()>
where
    F: Fn(u16) -> bool,
    R: Fn(
        &FailoverCandidate,
        &ureq::Agent,
    ) -> Result<ureq::http::Response<ureq::Body>, ureq::Error>,
{
    execute_http_failover_with_callback(
        client,
        candidates,
        config,
        provider_name,
        is_retryable_status,
        |_, _| {},
        make_request,
    )
}

pub fn execute_http_failover_with_callback<F, C, R>(
    client: &mut TcpStream,
    candidates: &[FailoverCandidate],
    config: &FailoverConfig,
    provider_name: &str,
    is_retryable_status: F,
    mut on_candidate_status: C,
    make_request: R,
) -> io::Result<()>
where
    F: Fn(u16) -> bool,
    C: FnMut(&FailoverCandidate, u16),
    R: Fn(
        &FailoverCandidate,
        &ureq::Agent,
    ) -> Result<ureq::http::Response<ureq::Body>, ureq::Error>,
{
    let agent_config = ureq::Agent::config_builder()
        .timeout_global(config.timeout)
        .max_redirects(0)
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(agent_config);
    let mut last_status: Option<u16> = None;

    for (index, candidate) in candidates.iter().enumerate() {
        let attempt_number = index + 1;
        let last = attempt_number == candidates.len();

        match make_request(candidate, &agent) {
            Ok(res) => {
                let status = res.status().as_u16();
                last_status = Some(status);
                on_candidate_status(candidate, status);
                if is_retryable_status(status) && !last {
                    let delay = calculate_retry_delay(
                        index,
                        config.retry_base_delay_ms,
                        config.max_retry_delay_ms,
                        None,
                    );
                    eprintln!(
                        "[toolcase-gateway] {provider_name} account {} returned HTTP {status}. Failing over to account {} (retry in {}ms)...",
                        candidate.label,
                        candidates[index + 1].label,
                        delay.as_millis()
                    );
                    if !delay.is_zero() {
                        thread::sleep(delay);
                    }
                    continue;
                }
                return write_ureq_response(client, res);
            }
            Err(e) => {
                if !last {
                    let delay = calculate_retry_delay(
                        index,
                        config.retry_base_delay_ms,
                        config.max_retry_delay_ms,
                        None,
                    );
                    eprintln!(
                        "[toolcase-gateway] {provider_name} account {} transport error ({e}). Failing over to account {} (retry in {}ms)...",
                        candidate.label,
                        candidates[index + 1].label,
                        delay.as_millis()
                    );
                    if !delay.is_zero() {
                        thread::sleep(delay);
                    }
                    continue;
                }
                return write_error(
                    client,
                    502,
                    "Bad Gateway",
                    &format!("toolcase-gateway: {provider_name} upstream transport error: {e}"),
                );
            }
        }
    }

    write_error(
        client,
        last_status.unwrap_or(502),
        "Bad Gateway",
        &format!(
            "toolcase-gateway: all {provider_name} accounts exhausted; last status HTTP {}",
            last_status.unwrap_or(502)
        ),
    )
}
