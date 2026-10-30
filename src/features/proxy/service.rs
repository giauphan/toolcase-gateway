use crate::config::Config;
use crate::http::Request;
use crate::providers::omniroute::OmniRouteAdapter;
use crate::rewrite::json_string_value;
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

static RR_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct UpstreamAttempt<'a> {
    model: &'a str,
    index: usize,
    last: bool,
    prefix: Option<&'a str>,
}

pub struct ProxyService;

impl ProxyService {
    fn forward_muse(
        client: &mut TcpStream,
        request: &Request,
        config: &Config,
        index: usize,
        last: bool,
    ) -> io::Result<Option<u16>> {
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
                Ok(Some(200))
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
                Ok(None)
            }
            Err(error) => {
                crate::runtime::logger::log_message(
                    "ERROR",
                    "gateway_proxy",
                    &format!("All upstream models exhausted; model 'muse' failed ({error})"),
                );
                crate::http::write_error(
                    client,
                    502,
                    "Bad Gateway",
                    &format!("toolcase-gateway: all upstream models exhausted; model \"muse\" failed ({error})"),
                )?;
                Ok(Some(502))
            }
        }
    }

    fn forward_upstream(
        client: &mut TcpStream,
        request: &Request,
        config: &Config,
        attempt: UpstreamAttempt<'_>,
        last_status: &mut Option<u16>,
    ) -> io::Result<Option<u16>> {
        let UpstreamAttempt {
            model,
            index,
            last,
            prefix,
        } = attempt;
        let mut upstream = match OmniRouteAdapter::open_upstream(client, request, config, model) {
            Ok(u) => u,
            Err(e) => {
                return handle_upstream_error(client, &request.path, model, "connection", last, &e)
            }
        };

        let mut head = match crate::http::read_response_head(&mut upstream) {
            Ok(h) => h,
            Err(e) => {
                return handle_upstream_error(client, &request.path, model, "read head", last, &e)
            }
        };

        *last_status = Some(head.status);
        if crate::gateway::retry::is_safe_to_retry(head.status) {
            return handle_retryable_status(client, &head, model, index, last, config);
        }

        crate::runtime::logger::log_message(
            "INFO",
            "gateway_proxy",
            &format!(
                "Upstream model '{model}' responded with HTTP {} {} for path {}",
                head.status, head.reason, request.path
            ),
        );

        let allowed_scope = resolve_allowed_service_worker_scope(request, prefix);

        if head.status == 304 || head.status == 204 || (100..=199).contains(&head.status) {
            let status = head.status;
            handle_empty_body_forward(client, &head, allowed_scope.as_deref())?;
            return Ok(Some(status));
        }

        if let Some(pfx) = prefix {
            if let Some(status) = handle_prefix_rewrite(
                client,
                &mut upstream,
                &mut head,
                request,
                pfx,
                config,
                allowed_scope.as_deref(),
            )? {
                return Ok(Some(status));
            }
        }
        let status = head.status;
        crate::http::stream_response(client, &mut upstream, head, &request.body)?;
        Ok(Some(status))
    }

    pub fn forward(client: &mut TcpStream, request: &Request, config: &Config) -> io::Result<u16> {
        Self::forward_with_prefix(client, request, config, None)
    }

    pub fn forward_with_prefix(
        client: &mut TcpStream,
        request: &Request,
        config: &Config,
        prefix: Option<&str>,
    ) -> io::Result<u16> {
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
            let last = index + 1 == candidates.len();

            if model == "muse" {
                if let Some(status) = Self::forward_muse(client, request, config, index, last)? {
                    return Ok(status);
                }
                continue;
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
                    index + 1,
                    candidates.len()
                ),
            );

            let attempt = UpstreamAttempt {
                model,
                index,
                last,
                prefix,
            };
            if let Some(status) =
                Self::forward_upstream(client, request, config, attempt, &mut last_status)?
            {
                return Ok(status);
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

pub use super::prefix_rewrite::rewrite_location_header;

fn handle_upstream_error(
    client: &mut TcpStream,
    path: &str,
    model: &str,
    phase: &str,
    last: bool,
    err: &io::Error,
) -> io::Result<Option<u16>> {
    crate::runtime::logger::log_message(
        "WARN",
        "gateway_proxy",
        &format!("Upstream {phase} failed for model '{model}' on path {path}: {err}"),
    );
    if !last {
        return Ok(None);
    }
    crate::runtime::logger::log_message(
        "ERROR",
        "gateway_proxy",
        &format!("All upstream models exhausted; upstream {phase} error: {err}"),
    );
    crate::http::write_error(
        client,
        502,
        "Bad Gateway",
        &format!("toolcase-gateway: upstream {phase} failed: {err}"),
    )?;
    Ok(Some(502))
}

fn handle_retryable_status(
    client: &mut TcpStream,
    head: &crate::http::ResponseHead,
    model: &str,
    index: usize,
    last: bool,
    config: &Config,
) -> io::Result<Option<u16>> {
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
            "Upstream model '{model}' returned HTTP {}; failing over (retry in {}ms)",
            head.status,
            delay.as_millis()
        ),
    );
    if !last {
        if !delay.is_zero() {
            thread::sleep(delay);
        }
        return Ok(None);
    }
    crate::runtime::logger::log_message(
        "ERROR",
        "gateway_proxy",
        &format!(
            "All upstream models exhausted; final model '{model}' returned HTTP {}",
            head.status
        ),
    );
    crate::http::write_error(
        client,
        head.status,
        &head.reason,
        &format!(
            "toolcase-gateway: all upstream models exhausted; model \"{model}\" returned HTTP {}",
            head.status
        ),
    )?;
    Ok(Some(head.status))
}

fn resolve_allowed_service_worker_scope(request: &Request, prefix: Option<&str>) -> Option<String> {
    let is_sw = crate::http::header_value(&request.headers, "service-worker")
        .is_some_and(|v| v.eq_ignore_ascii_case("script"))
        || request.path.ends_with("/sw.js")
        || request.path.contains("/sw.js?");
    if is_sw {
        Some(match prefix {
            Some(pfx) => format!("{pfx}/"),
            None => "/".to_string(),
        })
    } else {
        None
    }
}

fn handle_empty_body_forward(
    client: &mut TcpStream,
    head: &crate::http::ResponseHead,
    allowed_scope: Option<&str>,
) -> io::Result<()> {
    let status = head.status;
    write!(client, "HTTP/1.1 {status} {}\r\n", head.reason)?;
    for (name, val) in &head.headers {
        if name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("transfer-encoding")
            || (allowed_scope.is_some() && name.eq_ignore_ascii_case("service-worker-allowed"))
            || crate::http::HOP_BY_HOP
                .iter()
                .any(|h| name.eq_ignore_ascii_case(h))
        {
            continue;
        }
        write!(client, "{name}: {val}\r\n")?;
    }
    if let Some(scope) = allowed_scope {
        write!(client, "Service-Worker-Allowed: {scope}\r\n")?;
    }
    write!(client, "Connection: close\r\n\r\n")?;
    client.flush()
}

fn handle_prefix_rewrite(
    client: &mut TcpStream,
    upstream: &mut TcpStream,
    head: &mut crate::http::ResponseHead,
    request: &Request,
    pfx: &str,
    config: &Config,
    allowed_scope: Option<&str>,
) -> io::Result<Option<u16>> {
    if (300..=399).contains(&head.status) {
        let target_host = &config.target_host;
        let target_port = config.target_port;
        for (name, val) in &mut head.headers {
            if name.eq_ignore_ascii_case("location") {
                *val = rewrite_location_header(val, pfx, target_host, target_port);
            }
        }
    }
    if super::prefix_rewrite::is_rewritable(head) && request.method != "HEAD" {
        let body = super::prefix_rewrite::read_full_body(upstream, head)?;
        let rewritten =
            super::prefix_rewrite::rewrite_root_relative(&String::from_utf8_lossy(&body), pfx);
        let status = head.status;
        super::prefix_rewrite::write_rewritten(client, head, rewritten.as_bytes(), allowed_scope)?;
        return Ok(Some(status));
    }
    Ok(None)
}
