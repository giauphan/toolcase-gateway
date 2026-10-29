//! Inbound HTTP routing and fail-closed dispatching.

pub mod dashboard;
pub mod jev;
pub mod modules;
pub mod muse;
pub mod templates;

pub use dashboard::*;
pub use jev::*;
pub use modules::*;
pub use muse::*;
pub use templates::*;

use crate::config::{Config, ConfigStore};
use crate::http::{read_request_with_admission, write_error};
use crate::omniroute::handle_omniroute_proxy;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::Duration;

fn write_rejection_and_drain(
    client: &mut TcpStream,
    status: u16,
    reason: &str,
    message: &str,
) -> io::Result<()> {
    write_error(client, status, reason, message)?;
    client.flush()?;
    let _ = client.shutdown(Shutdown::Write);
    let _ = client.set_read_timeout(Some(Duration::from_millis(50)));
    let mut remaining = 64 * 1024;
    let mut buffer = [0u8; 8192];
    while remaining > 0 {
        let read_len = remaining.min(buffer.len());
        match client.read(&mut buffer[..read_len]) {
            Ok(0) | Err(_) => break,
            Ok(count) => remaining -= count,
        }
    }
    Ok(())
}

pub fn route_request(mut client: TcpStream, store: &ConfigStore) -> io::Result<()> {
    let start = std::time::Instant::now();
    let peer_ip = client
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "-".into());

    let (request, _byte_guard) = match read_request_with_admission(&mut client, &store.admission())
    {
        Ok(request) => request,
        Err(error) if error.kind() == io::ErrorKind::Other => {
            let res = write_rejection_and_drain(
                &mut client,
                503,
                "Service Unavailable",
                "too many in-flight bytes",
            );
            crate::runtime::logger::log_access_with_status(
                "UNKNOWN",
                "-",
                &peer_ip,
                503,
                start.elapsed().as_millis() as u64,
            );
            return res;
        }
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
            let res = write_rejection_and_drain(
                &mut client,
                413,
                "Payload Too Large",
                &error.to_string(),
            );
            crate::runtime::logger::log_access_with_status(
                "UNKNOWN",
                "-",
                &peer_ip,
                413,
                start.elapsed().as_millis() as u64,
            );
            return res;
        }
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            let res =
                write_rejection_and_drain(&mut client, 400, "Bad Request", &error.to_string());
            crate::runtime::logger::log_access_with_status(
                "UNKNOWN",
                "-",
                &peer_ip,
                400,
                start.elapsed().as_millis() as u64,
            );
            return res;
        }
        Err(error) => return Err(error),
    };

    let result = route_request_dispatch(&mut client, &request, store);
    let duration_ms = start.elapsed().as_millis() as u64;
    let status = match &result {
        Ok(s) => *s,
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe || e.kind() == io::ErrorKind::ConnectionReset => 499,
        Err(_) => 500,
    };
    crate::runtime::logger::log_access_with_status(
        &request.method,
        &request.path,
        &peer_ip,
        status,
        duration_ms,
    );
    result.map(|_| ())
}

pub fn route_request_dispatch(
    client: &mut TcpStream,
    request: &crate::http::Request,
    store: &ConfigStore,
) -> io::Result<u16> {
    let clean_path = request
        .path
        .split('?')
        .next()
        .unwrap_or("")
        .trim_end_matches('/');

    if request.method.eq_ignore_ascii_case("options") {
        handle_cors_preflight(client)?;
        return Ok(200);
    }

    // 1. System Dashboard and configuration routes
    if let Some(code) = dispatch_dashboard_route(client, request, store, clean_path)? {
        return Ok(code);
    }
    if let Some(code) = dispatch_muse_config_route(client, request, store, clean_path)? {
        return Ok(code);
    }

    // 2. Model Context Protocol (MCP) HTTP / SSE endpoints
    if let Some(code) = crate::api::mcp::handle_http_mcp(client, request, store, clean_path)? {
        return Ok(code);
    }

    // 3. System template routes
    if let Some(code) = dispatch_template_route(client, request, clean_path)? {
        return Ok(code);
    }

    let snapshot = store.snapshot();

    // 4. Unified Module & Provider Dispatcher (handles built-in providers, personal modules, and module groups)
    if let Some(code) = dispatch_module_route(client, request, store, &snapshot, clean_path)? {
        return Ok(code);
    }

    // 5. Fail-closed fallback: unowned paths outside enabled module namespaces return 404
    write_error(
        client,
        404,
        "Not Found",
        &format!("No enabled module registered for route '{clean_path}'"),
    )?;
    Ok(404)
}

pub fn route_request_impl(
    client: &mut TcpStream,
    request: &crate::http::Request,
    config: &Config,
    jev_pool: &crate::providers::jev::accounts::JevAccountPool,
) -> io::Result<u16> {
    let clean_path = request
        .path
        .split('?')
        .next()
        .unwrap_or("")
        .trim_end_matches('/');

    if let Some(code) = dispatch_catalog_route(client, request, clean_path)? {
        return Ok(code);
    }
    if let Some(code) = dispatch_muse_chat_route(client, request, config, clean_path)? {
        return Ok(code);
    }
    if let Some(code) = dispatch_muse_image_route(client, request, config, clean_path)? {
        return Ok(code);
    }
    if let Some(code) = dispatch_muse_video_route(client, request, config, clean_path)? {
        return Ok(code);
    }
    if let Some(code) = dispatch_jev_route(client, request, config, jev_pool, clean_path)? {
        return Ok(code);
    }
    if let Some(code) = dispatch_video_provider_route(client, request, clean_path)? {
        return Ok(code);
    }
    if let Some(code) = dispatch_template_route(client, request, clean_path)? {
        return Ok(code);
    }

    handle_omniroute_proxy(client, request, config)
}
