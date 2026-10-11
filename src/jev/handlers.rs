use crate::config::Config;
use crate::http::{header_value, write_error, Request};
use crate::http_provider::{
    execute_http_failover, write_ureq_response, FailoverCandidate, FailoverConfig,
};
use crate::jev::business::is_retryable;
use crate::jev::pool::JevAccountPool;
use std::io;
use std::net::TcpStream;

fn jev_route(inbound_path: &str) -> Option<(String, &'static str)> {
    let clean = inbound_path.split('?').next()?.trim_end_matches('/');
    let relative = if clean == "/jev/models" {
        "models"
    } else {
        clean
            .strip_prefix("/jev/v1/")
            .or_else(|| clean.strip_prefix("/jev-ai/v1/"))?
    };

    match relative {
        "models" => Some(("/api/v1/models/".to_string(), "GET")),
        "systemone" => Some(("/api/v1/systemone/".to_string(), "POST")),
        "requests" => None,
        _ => {
            let segments: Vec<&str> = relative.split('/').collect();
            if segments.first() != Some(&"requests") {
                return None;
            }
            let request_id = segments.get(1)?;
            if !is_valid_request_id(request_id) {
                return None;
            }
            match segments.len() {
                2 => Some((format!("/api/v1/requests/{request_id}/"), "GET")),
                3 if segments[2] == "cancel" => {
                    Some((format!("/api/v1/requests/{request_id}/cancel/"), "POST"))
                }
                _ => None,
            }
        }
    }
}

fn is_valid_request_id(request_id: &str) -> bool {
    !request_id.is_empty()
        && request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

pub fn map_jev_upstream_path(inbound_path: &str) -> Option<String> {
    jev_route(inbound_path).map(|(path, _)| path)
}

pub fn expected_jev_method(inbound_path: &str) -> Option<&'static str> {
    jev_route(inbound_path).map(|(_, method)| method)
}

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
) -> io::Result<()> {
    let keys = configured_jev_keys(request, config);
    let pool = JevAccountPool::new(&keys);
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

pub fn handle_jev_ai(client: &mut TcpStream, request: &Request, config: &Config) -> io::Result<()> {
    let keys = configured_jev_keys(request, config);

    let pool = JevAccountPool::new(&keys);
    if pool.is_empty() {
        return write_error(
            client,
            401,
            "Unauthorized",
            "No Jev AI accounts configured. Set JEV_API_KEY or GW_JEV_API_KEYS, or pass Authorization: Bearer <key>",
        );
    }

    let Some(upstream_path) = map_jev_upstream_path(&request.path) else {
        return write_error(client, 404, "Not Found", "unknown Jev Decision API route");
    };
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
