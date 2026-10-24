use crate::config::Config;
use crate::http::Request;
use crate::providers::omniroute::OmniRouteAdapter;
use crate::rewrite::json_string_value;
use std::io;
use std::net::TcpStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

static RR_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct ProxyService;

impl ProxyService {
    pub fn forward(client: &mut TcpStream, request: &Request, config: &Config) -> io::Result<u16> {
        let rotation = RR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidates = crate::gateway::candidates::plan_candidates(
            &request.body,
            &config.fallbacks,
            rotation,
            &config.default_model,
        );
        let requested_model = std::str::from_utf8(&request.body)
            .ok()
            .and_then(|body| json_string_value(body, "model"))
            .unwrap_or_else(|| candidates.first().cloned().unwrap_or_default());
        let has_tools = std::str::from_utf8(&request.body)
            .map(|body| body.contains("\"tools\""))
            .unwrap_or(false);
        let decision = crate::gateway::policy::evaluate_routing_policy(
            &requested_model,
            candidates,
            config,
            has_tools,
        );
        if let Some(shadow) = decision.shadow_recommendation.as_deref() {
            eprintln!(
                "[toolcase-gateway] routing_shadow mode={:?} requested={} recommended={} reason={}",
                decision.strategy, requested_model, shadow, decision.reason
            );
        }
        let candidates = decision.candidates;

        crate::runtime::logger::log_message(
            "INFO",
            "gateway_proxy",
            &format!(
                "Routing request {} to model '{}' (candidates: {})",
                request.path,
                requested_model,
                candidates.join(", ")
            ),
        );

        let mut last_status: Option<u16> = None;

        for (index, model) in candidates.iter().enumerate() {
            let attempt_number = index + 1;
            let last = attempt_number == candidates.len();

            if model == "muse" {
                match crate::museai::request_museai_chat_completion(&request.body, config, false) {
                    Ok(response) => {
                        crate::runtime::logger::log_message(
                            "INFO",
                            "gateway_proxy",
                            &format!(
                                "Model 'muse' completed chat completion for path {}",
                                request.path
                            ),
                        );
                        crate::museai::write_chat_completion(client, &response)?;
                        return Ok(200);
                    }
                    Err(_error) if !last => {
                        let delay = crate::omniroute::calculate_retry_delay(
                            index,
                            config.retry_base_delay_ms,
                            config.max_retry_delay_ms,
                            None,
                        );
                        crate::runtime::logger::log_message(
                            "WARN",
                            "gateway_proxy",
                            &format!(
                                "Model 'muse' failed ({_error}); failing over to next candidate (retry in {}ms)",
                                delay.as_millis()
                            ),
                        );
                        if !delay.is_zero() {
                            thread::sleep(delay);
                        }
                        continue;
                    }
                    Err(error) => {
                        crate::runtime::logger::log_message(
                            "ERROR",
                            "gateway_proxy",
                            &format!(
                                "All upstream models exhausted; model 'muse' failed ({error})"
                            ),
                        );
                        crate::http::write_error(
                            client,
                            502,
                            "Bad Gateway",
                            &format!("toolcase-gateway: all upstream models exhausted; model \"muse\" failed ({error})"),
                        )?;
                        return Ok(502);
                    }
                }
            }

            crate::runtime::logger::log_message(
                "INFO",
                "gateway_proxy",
                &format!(
                    "Forwarding request {} to upstream {}:{} with model '{}' (attempt {}/{})",
                    request.path,
                    config.target_host,
                    config.target_port,
                    model,
                    attempt_number,
                    candidates.len()
                ),
            );

            match OmniRouteAdapter::open_upstream(client, request, config, model) {
                Ok(mut upstream) => match crate::http::read_response_head(&mut upstream) {
                    Ok(head) => {
                        last_status = Some(head.status);
                        if crate::gateway::retry::is_safe_to_retry(head.status) {
                            let delay = crate::omniroute::calculate_retry_delay(
                                index,
                                config.retry_base_delay_ms,
                                config.max_retry_delay_ms,
                                Some(&head.headers),
                            );
                            crate::runtime::logger::log_message(
                                "WARN",
                                "gateway_proxy",
                                &format!(
                                    "Upstream model '{}' returned HTTP {}; failing over (retry in {}ms)",
                                    model,
                                    head.status,
                                    delay.as_millis()
                                ),
                            );
                            if !last {
                                if !delay.is_zero() {
                                    thread::sleep(delay);
                                }
                                continue;
                            }
                            crate::runtime::logger::log_message(
                                "ERROR",
                                "gateway_proxy",
                                &format!(
                                    "All upstream models exhausted; final model '{}' returned HTTP {}",
                                    model, head.status
                                ),
                            );
                            crate::http::write_error(
                                client,
                                head.status,
                                &head.reason,
                                &format!("toolcase-gateway: all upstream models exhausted; model \"{model}\" returned HTTP {}", head.status),
                            )?;
                            return Ok(head.status);
                        } else {
                            crate::runtime::logger::log_message(
                                "INFO",
                                "gateway_proxy",
                                &format!(
                                    "Upstream model '{}' responded with HTTP {} {} for path {}",
                                    model, head.status, head.reason, request.path
                                ),
                            );
                            let status = head.status;
                            crate::http::stream_response(
                                client,
                                &mut upstream,
                                head,
                                &request.body,
                            )?;
                            return Ok(status);
                        }
                    }
                    Err(e) => {
                        crate::runtime::logger::log_message(
                            "WARN",
                            "gateway_proxy",
                            &format!(
                                "Upstream read head failed for model '{}' on path {}: {e}",
                                model, request.path
                            ),
                        );
                        if !last {
                            continue;
                        }
                        crate::runtime::logger::log_message(
                            "ERROR",
                            "gateway_proxy",
                            &format!(
                                "All upstream models exhausted; upstream read head error: {e}"
                            ),
                        );
                        crate::http::write_error(
                            client,
                            502,
                            "Bad Gateway",
                            &format!("toolcase-gateway: upstream read head failed: {e}"),
                        )?;
                        return Ok(502);
                    }
                },
                Err(e) => {
                    crate::runtime::logger::log_message(
                        "WARN",
                        "gateway_proxy",
                        &format!(
                            "Upstream connection failed for model '{}' on path {}: {e}",
                            model, request.path
                        ),
                    );
                    if !last {
                        continue;
                    }
                    crate::runtime::logger::log_message(
                        "ERROR",
                        "gateway_proxy",
                        &format!("All upstream models exhausted; upstream connection error: {e}"),
                    );
                    crate::http::write_error(
                        client,
                        502,
                        "Bad Gateway",
                        &format!("toolcase-gateway: upstream connection failed: {e}"),
                    )?;
                    return Ok(502);
                }
            }
        }

        let final_status = last_status.unwrap_or(502);
        crate::runtime::logger::log_message(
            "ERROR",
            "gateway_proxy",
            &format!(
                "All upstream models exhausted for path {}; returning HTTP {}",
                request.path, final_status
            ),
        );
        crate::http::write_error(
            client,
            final_status,
            "Bad Gateway",
            "toolcase-gateway: all upstream models exhausted",
        )?;
        Ok(final_status)
    }
}
