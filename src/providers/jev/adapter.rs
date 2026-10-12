use crate::config::Config;
use crate::http::{header_value, write_error, Request};
use crate::http_provider::{
    execute_http_failover, write_ureq_response, FailoverCandidate, FailoverConfig,
};
use crate::providers::jev::accounts::JevAccountPool;
use crate::providers::jev::business::is_retryable;
use std::io;
use std::net::TcpStream;

use crate::providers::jev::route::resolve_jev_route;

fn configured_jev_keys(request: &Request, config: &Config) -> Vec<String> {
    if !config.jev_api_keys.is_empty() {
        return config.jev_api_keys.clone();
    }

    let mut keys = Vec::new();
    if let Some(auth) = header_value(&request.headers, "authorization") {
        if let Some(token) = auth
            .strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("bearer "))
        {
            let token_clean = token.trim().to_string();
            if !token_clean.is_empty() {
                keys.push(token_clean);
            }
        }
    }
    if let Some(api_key) = header_value(&request.headers, "x-api-key") {
        let key_clean = api_key.trim().to_string();
        if !key_clean.is_empty() && !keys.contains(&key_clean) {
            keys.push(key_clean);
        }
    }
    keys
}

pub fn handle_jev_models_catalog(
    client: &mut TcpStream,
    request: &Request,
    config: &Config,
    jev_pool: &JevAccountPool,
) -> io::Result<()> {
    let fallback_pool;
    let pool = if !jev_pool.is_empty() {
        jev_pool
    } else {
        let keys = configured_jev_keys(request, config);
        fallback_pool = JevAccountPool::new(&keys);
        &fallback_pool
    };
    let base_url = config.jev_base_url.trim_end_matches('/');
    let target_url = format!("{base_url}/api/v1/models/");
    let agent_config = ureq::Agent::config_builder()
        .timeout_global(config.io_timeout)
        .max_redirects(0)
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(agent_config);
    let mut req = agent.get(&target_url);
    if let Some(account) = pool.next_account() {
        req = req.header("Authorization", format!("Bearer {}", account.api_key));
    }

    match req.call() {
        Ok(response) => write_ureq_response(client, response),
        Err(error) => write_error(
            client,
            502,
            "Bad Gateway",
            &format!("toolcase-gateway: Jev models upstream transport error: {error}"),
        ),
    }
}

pub fn handle_jev_ai(
    client: &mut TcpStream,
    request: &Request,
    config: &Config,
    jev_pool: &JevAccountPool,
) -> io::Result<()> {
    let fallback_pool;
    let pool = if !jev_pool.is_empty() {
        jev_pool
    } else {
        let keys = configured_jev_keys(request, config);
        fallback_pool = JevAccountPool::new(&keys);
        &fallback_pool
    };

    if pool.is_empty() {
        return write_error(
            client,
            401,
            "Unauthorized",
            "No Jev AI accounts configured. Set GW_JEV_API_KEYS or pass Authorization: Bearer <key>",
        );
    }

    let route = match resolve_jev_route(&request.path) {
        Some(route) => route,
        None => return write_error(client, 404, "Not Found", "unknown Jev route"),
    };
    let upstream_path = route.upstream_path;

    let base_url = config.jev_base_url.trim_end_matches('/');
    let target_url = format!("{base_url}{upstream_path}");

    let candidates: Vec<FailoverCandidate> = pool
        .candidate_accounts()
        .into_iter()
        .map(|acc| FailoverCandidate {
            api_key: acc.api_key,
            label: acc.label,
        })
        .collect();

    let failover_config = FailoverConfig {
        timeout: config.io_timeout,
        retry_base_delay_ms: config.retry_base_delay_ms,
        max_retry_delay_ms: config.max_retry_delay_ms,
    };

    execute_http_failover(
        client,
        &candidates,
        &failover_config,
        "Jev",
        is_retryable,
        |candidate, agent| {
            if request.method.eq_ignore_ascii_case("get") {
                let mut req = agent.get(&target_url);
                req = req.header("Authorization", format!("Bearer {}", candidate.api_key));
                if let Some(idemp) = header_value(&request.headers, "idempotency-key") {
                    req = req.header("Idempotency-Key", idemp);
                }
                req.call()
            } else {
                let mut req = agent.post(&target_url);
                req = req.header("Authorization", format!("Bearer {}", candidate.api_key));
                req = req.header("Content-Type", "application/json");
                if let Some(idemp) = header_value(&request.headers, "idempotency-key") {
                    req = req.header("Idempotency-Key", idemp);
                }
                req.send(&request.body)
            }
        },
    )
}

pub fn expected_jev_method(inbound_path: &str) -> Option<&'static str> {
    resolve_jev_route(inbound_path).map(|route| route.expected_method)
}

pub fn map_jev_upstream_path(inbound_path: &str) -> Option<String> {
    resolve_jev_route(inbound_path).map(|route| route.upstream_path)
}
