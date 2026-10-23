use crate::config::Config;
use crate::http::{header_value, write_error, Request};
use crate::http_provider::{
    execute_http_failover_with_callback, write_ureq_response, CandidateKind, FailoverConfig,
};
use crate::providers::jev::accounts::JevAccountPool;
use crate::providers::jev::business::is_retryable;
use crate::providers::jev::openrouter::{
    prepare_openrouter_payload, send_openrouter_decision_request,
};
use crate::providers::jev::route::resolve_jev_route;
use std::io;
use std::net::TcpStream;

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
        if !keys.is_empty() {
            fallback_pool = JevAccountPool::new(&keys);
            &fallback_pool
        } else {
            jev_pool
        }
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
        if !keys.is_empty() {
            fallback_pool = JevAccountPool::new(&keys);
            &fallback_pool
        } else {
            jev_pool
        }
    };

    let mut candidates = pool.candidate_failover_targets(config);

    if let Some(inbound_or_key) = header_value(&request.headers, "x-openrouter-api-key")
        .or_else(|| {
            header_value(&request.headers, "authorization")
                .and_then(|a| {
                    a.strip_prefix("Bearer ")
                        .or_else(|| a.strip_prefix("bearer "))
                })
                .filter(|token| token.trim().starts_with("sk-or-"))
        })
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
    {
        candidates.insert(
            0,
            crate::http_provider::FailoverCandidate::openrouter_api_key(
                inbound_or_key.clone(),
                "inbound-openrouter-key".into(),
                None,
            ),
        );
        let alt_models = pool.rotated_fallback_models(config);
        for (idx, alt_model) in alt_models.into_iter().enumerate() {
            candidates.insert(
                1 + idx,
                crate::http_provider::FailoverCandidate::openrouter_api_key(
                    inbound_or_key.clone(),
                    format!("inbound-openrouter-key-alt-{}", idx + 1),
                    Some(alt_model),
                ),
            );
        }
    }

    if candidates.is_empty() {
        return write_error(
            client,
            401,
            "Unauthorized",
            "No Jev AI or OpenRouter accounts configured. Set GW_JEV_API_KEYS, GW_JEV_OPENROUTER_API_KEY, GW_JEV_OPENROUTER_COOKIE, or pass Authorization: Bearer <key>",
        );
    }

    let route = match resolve_jev_route(&request.path) {
        Some(route) => route,
        None => return write_error(client, 404, "Not Found", "unknown Jev route"),
    };
    let upstream_path = route.upstream_path;

    let base_url = config.jev_base_url.trim_end_matches('/');
    let target_url = format!("{base_url}{upstream_path}");
    let or_base_url = if config.jev_openrouter_base_url.trim().is_empty() {
        "https://openrouter.ai"
    } else {
        config.jev_openrouter_base_url.trim()
    };
    let or_model = if config.jev_openrouter_model.trim().is_empty() {
        "typesafe/jev-1.13"
    } else {
        config.jev_openrouter_model.trim()
    };
    let idempotency = header_value(&request.headers, "idempotency-key");

    let failover_config = FailoverConfig {
        timeout: config.io_timeout,
        retry_base_delay_ms: config.retry_base_delay_ms,
        max_retry_delay_ms: config.max_retry_delay_ms,
    };

    execute_http_failover_with_callback(
        client,
        &candidates,
        &failover_config,
        "Jev",
        is_retryable,
        |candidate, status| {
            if status == 402 {
                pool.mark_credit_exhausted(&candidate.api_key);
            } else if status == 200 {
                pool.clear_credit_exhausted(&candidate.api_key);
            }
        },
        |candidate, agent| match candidate.kind {
            CandidateKind::Direct => {
                if request.method.eq_ignore_ascii_case("get") {
                    let mut req = agent.get(&target_url);
                    req = req.header("Authorization", format!("Bearer {}", candidate.api_key));
                    if let Some(idemp) = idempotency {
                        req = req.header("Idempotency-Key", idemp);
                    }
                    req.call()
                } else {
                    let mut req = agent.post(&target_url);
                    req = req.header("Authorization", format!("Bearer {}", candidate.api_key));
                    req = req.header("Content-Type", "application/json");
                    if let Some(idemp) = idempotency {
                        req = req.header("Idempotency-Key", idemp);
                    }
                    req.send(&request.body)
                }
            }
            CandidateKind::OpenRouterApiKey | CandidateKind::OpenRouterBrowser => {
                let model_to_use = candidate.model.as_deref().unwrap_or(or_model);
                let payload = prepare_openrouter_payload(&request.body, model_to_use);
                send_openrouter_decision_request(
                    agent,
                    or_base_url,
                    &candidate.api_key,
                    (!config.jev_openrouter_cookie.is_empty())
                        .then_some(config.jev_openrouter_cookie.as_str()),
                    &payload,
                    idempotency,
                    None,
                    None,
                )
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
