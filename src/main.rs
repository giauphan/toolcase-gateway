//! Tool-name casing gateway + failover retry.
#![forbid(unsafe_code)]

use std::io::{self, ErrorKind};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

mod config;
mod http;
mod omniroute;
mod prism;
mod rewrite;
mod routes;
#[cfg(test)]
mod tests;

pub(crate) use config::*;
pub(crate) use http::*;
pub(crate) use routes::*;

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

fn main() -> io::Result<()> {
    let _ = dotenvy::dotenv();

    let listen_host = env_or("GW_LISTEN_HOST", "127.0.0.1");
    let listen_port = env_or("GW_LISTEN_PORT", "20129").parse().unwrap_or(20129);
    let max_connections: usize = env_or("GW_MAX_CONNECTIONS", "256").parse().unwrap_or(256);
    let timeout_secs: u64 = env_or("GW_IO_TIMEOUT_SECS", "120").parse().unwrap_or(120);
    let config = Arc::new(Config {
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
        prism_base_url: env_or("GW_PRISM_BASE_URL", "https://prism.openai.com"),
        prism_project_id: env_or("GW_PRISM_PROJECT_ID", "0f6fa2ad-f391-4d28-9770-e3d6d511f80c"),
        prism_cookie: env_or("GW_PRISM_COOKIE", ""),
        prism_sandbox_token: env_or("GW_PRISM_SANDBOX_TOKEN", ""),
        prism_user_id: env_or("GW_PRISM_USER_ID", ""),
        prism_default_model: env_or("GW_PRISM_DEFAULT_MODEL", "gpt-5.6-sol"),
        prism_system_prompt: env_or(
            "GW_PRISM_SYSTEM_PROMPT",
            "You are ChatGPT, a large language model trained by OpenAI. Carefully follow the user's instructions. Implement the requested tasks perfectly and exactly as directed.",
        ),
    });
    let listener = TcpListener::bind((listen_host.as_str(), listen_port))?;
    eprintln!(
        "[toolcase-gateway] {listen_host}:{listen_port} -> {}:{}",
        config.target_host, config.target_port
    );
    if listen_host != "127.0.0.1" && listen_host != "::1" && listen_host != "localhost" {
        eprintln!("[toolcase-gateway] WARNING: listening on {listen_host} exposes an unauthenticated proxy; put it behind an authenticating front end");
    }
    for stream in listener.incoming() {
        let Ok(client) = stream else { continue };
        if ACTIVE.load(Ordering::Relaxed) >= max_connections {
            let mut client = client;
            let _ = write_error(
                &mut client,
                503,
                "Service Unavailable",
                "too many connections",
            );
            continue;
        }
        if let Some(timeout) = config.io_timeout {
            let _ = client.set_read_timeout(Some(timeout));
            let _ = client.set_write_timeout(Some(timeout));
        }
        ACTIVE.fetch_add(1, Ordering::Relaxed);
        let config = config.clone();
        thread::spawn(move || {
            if let Err(error) = route_request(client, &config) {
                if !matches!(
                    error.kind(),
                    ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::UnexpectedEof
                ) {
                    eprintln!("[toolcase-gateway] request failed: {}", error.kind());
                }
            }
            ACTIVE.fetch_sub(1, Ordering::Relaxed);
        });
    }
    Ok(())
}
