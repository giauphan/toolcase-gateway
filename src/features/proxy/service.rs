use crate::config::Config;
use crate::http::Request;
use crate::providers::omniroute::OmniRouteAdapter;
use crate::rewrite::json_string_value;
use std::io;
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
                crate::runtime::logger::log_message(
                    "WARN",
                    "gateway_proxy",
                    &format!(
                        "Upstream connection failed for model '{}' on path {}: {e}",
                        model, request.path
                    ),
                );
                if !last {
                    return Ok(None);
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
                return Ok(Some(502));
            }
        };

        let head = match crate::http::read_response_head(&mut upstream) {
            Ok(h) => h,
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
                    return Ok(None);
                }
                crate::runtime::logger::log_message(
                    "ERROR",
                    "gateway_proxy",
                    &format!("All upstream models exhausted; upstream read head error: {e}"),
                );
                crate::http::write_error(
                    client,
                    502,
                    "Bad Gateway",
                    &format!("toolcase-gateway: upstream read head failed: {e}"),
                )?;
                return Ok(Some(502));
            }
        };

        *last_status = Some(head.status);
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
                return Ok(None);
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
            return Ok(Some(head.status));
        }

        crate::runtime::logger::log_message(
            "INFO",
            "gateway_proxy",
            &format!(
                "Upstream model '{}' responded with HTTP {} {} for path {}",
                model, head.status, head.reason, request.path
            ),
        );
        let mut head = head;
        if let Some(pfx) = prefix {
            if (300..=399).contains(&head.status) {
                let target_host = &config.target_host;
                let target_port = config.target_port;
                for (name, val) in &mut head.headers {
                    if name.eq_ignore_ascii_case("location") {
                        *val = rewrite_location_header(val, pfx, target_host, target_port);
                    }
                }
            }
            // Mount the upstream web app under the module prefix so its own
            // paths (/dashboard/providers, /_next/...) never collide with gateway routes.
            if super::prefix_rewrite::is_rewritable(&head) && request.method != "HEAD" {
                let body = super::prefix_rewrite::read_full_body(&mut upstream, &head)?;
                let rewritten = super::prefix_rewrite::rewrite_root_relative(
                    &String::from_utf8_lossy(&body),
                    pfx,
                );
                let status = head.status;
                super::prefix_rewrite::write_rewritten(client, &head, rewritten.as_bytes())?;
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

/// Rewrites redirect `Location` headers from an upstream service to remain within the mounted module prefix.
pub fn rewrite_location_header(
    location: &str,
    prefix: &str,
    upstream_host: &str,
    upstream_port: u16,
) -> String {
    let upstream_prefix_http = format!("http://{upstream_host}:{upstream_port}");
    let upstream_prefix_https = format!("https://{upstream_host}:{upstream_port}");
    let path = if let Some(stripped) = location.strip_prefix(&upstream_prefix_http) {
        stripped
    } else if let Some(stripped) = location.strip_prefix(&upstream_prefix_https) {
        stripped
    } else if location.starts_with('/') {
        location
    } else {
        return location.to_string();
    };

    if path == "/" || path.is_empty() {
        format!("{prefix}/dashboard")
    } else if path.starts_with(prefix) {
        path.to_string()
    } else {
        format!("{prefix}{path}")
    }
}
