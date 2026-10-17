//! Tool-name casing gateway + failover retry library.
#![forbid(unsafe_code)]

use std::io::{self, ErrorKind};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

pub mod codecs;
pub mod config;
pub mod features;
pub mod gateway;
pub mod har_config;
pub mod http;
pub mod http_provider;
pub mod jev;
pub mod museai;
pub mod omniroute;
pub mod providers;
pub mod rewrite;
pub mod routes;
pub mod runtime;
pub mod test_helpers;
pub mod video_template;

pub use config::*;
pub use http::*;
pub use routes::*;

pub fn run() -> io::Result<()> {
    let _ = dotenvy::dotenv();

    let listen_host = env_or("GW_LISTEN_HOST", "127.0.0.1");
    let listen_port = env_or("GW_LISTEN_PORT", "20129").parse().unwrap_or(20129);
    let max_connections: usize = env_or("GW_MAX_CONNECTIONS", "256").parse().unwrap_or(256);
    let timeout_secs: u64 = env_or("GW_IO_TIMEOUT_SECS", "120").parse().unwrap_or(120);
    let config = Config {
        target_host: env_or("GW_TARGET_HOST", "127.0.0.1"),
        target_port: env_or("GW_TARGET_PORT", "20128").parse().unwrap_or(20128),
        fallbacks: env_or("GW_FALLBACK_MODELS", "fail-try")
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect(),
        io_timeout: (timeout_secs > 0).then(|| Duration::from_secs(timeout_secs)),
        retry_base_delay_ms: env_or_duration_ms("GW_RETRY_BASE_DELAY_MS", 100),
        max_retry_delay_ms: env_or_duration_ms("GW_MAX_RETRY_DELAY_MS", 5000),
        default_model: env_or("GW_DEFAULT_MODEL", ""),
        museai_base_url: env_or("GW_MUSEAI_BASE_URL", "https://muse.ai"),
        museai_cookie: env_or("GW_MUSEAI_COOKIE", ""),
        museai_ws_url: env_or("GW_MUSEAI_WS_URL", "wss://hatch.metaaivm.com/v1/noise"),
        museai_access_token: env_or("GW_MUSEAI_ACCESS_TOKEN", ""),
        museai_notary_token: env_or("GW_MUSEAI_NOTARY_TOKEN", ""),
        museai_vm_id: env_or("GW_MUSEAI_VM_ID", ""),
        museai_auto_cleanup_threads: env_or("GW_MUSEAI_AUTO_CLEANUP_THREADS", "true")
            .parse()
            .unwrap_or(true),
        museai_thread_retention_secs: env_or("GW_MUSEAI_THREAD_RETENTION_SECS", "86400")
            .parse()
            .unwrap_or(86400),
        jev_api_keys: crate::config::parse_jev_api_keys(&env_or("GW_JEV_API_KEYS", "")),
        jev_base_url: env_or("GW_JEV_BASE_URL", "https://jev-ai.org"),
        routing_mode: match env_or("GW_ROUTING_MODE", "existing").as_str() {
            "existing" => crate::config::RoutingMode::Existing,
            "round_robin" => crate::config::RoutingMode::RoundRobin,
            "rules" => crate::config::RoutingMode::Rules,
            "jev" => crate::config::RoutingMode::Jev,
            _ => crate::config::RoutingMode::Existing,
        },
        jev_enabled: env_or("GW_JEV_ENABLED", "false").eq_ignore_ascii_case("true"),
        jev_shadow_mode: env_or("GW_JEV_SHADOW_MODE", "true").eq_ignore_ascii_case("true"),
        default_tier: match env_or("GW_DEFAULT_TIER", "strong").as_str() {
            "fast" => crate::config::ModelTier::Fast,
            "strong" => crate::config::ModelTier::Strong,
            _ => crate::config::ModelTier::Strong,
        },
        muse_approval: {
            let mode =
                crate::config::parse_muse_approval_mode(&env_or("GW_MUSE_APPROVAL_MODE", "off"));
            let allowed_types = env_or("GW_MUSE_APPROVAL_ALLOWED_TYPES", "network")
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .collect();
            let allowed_destinations = env_or("GW_MUSE_APPROVAL_ALLOWED_DESTINATIONS", "")
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .collect();
            let prefer_allow_once =
                env_or("GW_MUSE_APPROVAL_PREFER_ALLOW_ONCE", "true").eq_ignore_ascii_case("true");
            let allow_always =
                env_or("GW_MUSE_APPROVAL_ALLOW_ALWAYS", "false").eq_ignore_ascii_case("true");
            let max_decisions = env_or("GW_MUSE_APPROVAL_MAX_DECISIONS", "3")
                .parse()
                .unwrap_or(3);
            let timeout_secs = env_or("GW_MUSE_APPROVAL_TIMEOUT_SECS", "30")
                .parse()
                .unwrap_or(30);

            crate::config::MuseApprovalConfig {
                mode,
                allowed_types,
                allowed_destinations,
                prefer_allow_once,
                allow_always,
                max_decisions_per_job: max_decisions,
                approval_timeout_secs: timeout_secs,
            }
        },
    };
    let store = ConfigStore::new_with_limits(
        config,
        resolve_env_file(),
        max_connections,
        crate::runtime::admission::DEFAULT_MAX_IN_FLIGHT_BYTES,
    );
    let listener = TcpListener::bind((listen_host.as_str(), listen_port))?;
    let config = store.snapshot();
    eprintln!(
        "[toolcase-gateway] {listen_host}:{listen_port} -> {}:{}",
        config.target_host, config.target_port
    );
    if listen_host != "127.0.0.1" && listen_host != "::1" && listen_host != "localhost" {
        eprintln!("[toolcase-gateway] WARNING: listening on {listen_host} exposes an unauthenticated proxy; put it behind an authenticating front end");
    }
    if config.museai_auto_cleanup_threads && config.museai_thread_retention_secs > 0 {
        crate::museai::start_cleanup_worker();
    }
    let admission = store.admission();
    for stream in listener.incoming() {
        let Ok(client) = stream else { continue };
        let Some(conn_guard) = admission.acquire_connection() else {
            let mut client = client;
            let _ = write_error(
                &mut client,
                503,
                "Service Unavailable",
                "too many connections",
            );
            continue;
        };
        if let Some(timeout) = config.io_timeout {
            let _ = client.set_read_timeout(Some(timeout));
            let _ = client.set_write_timeout(Some(timeout));
        }
        let store = store.clone();
        thread::spawn(move || {
            let _guard = conn_guard;
            if let Err(error) = route_request(client, &store) {
                if !matches!(
                    error.kind(),
                    ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::UnexpectedEof
                ) {
                    eprintln!("[toolcase-gateway] request failed: {}", error.kind());
                }
            }
        });
    }
    Ok(())
}
