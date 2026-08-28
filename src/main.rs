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
mod rewrite;
mod routing;
#[cfg(test)]
mod tests;

pub(crate) use config::*;
pub(crate) use http::*;
pub(crate) use routing::*;

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

fn main() -> io::Result<()> {
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
            if let Err(error) = serve(client, &config) {
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
