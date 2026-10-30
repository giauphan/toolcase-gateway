use crate::config::ConfigStore;
use std::io::{self, Write};
use std::net::TcpStream;

pub fn mask_key(key: &str) -> String {
    let trimmed = key.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() > 10 {
        let head: String = chars[..6].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}...{tail}")
    } else if !chars.is_empty() {
        "***".to_string()
    } else {
        String::new()
    }
}

pub fn handle_dashboard_page(client: &mut TcpStream) -> io::Result<()> {
    let html = include_str!("../../../assets/dashboard.html");
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-cache, no-store, must-revalidate\r\nAccess-Control-Allow-Origin: *\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    )?;
    client.flush()
}

pub fn handle_dashboard_asset(
    client: &mut TcpStream,
    path: &str,
    include_body: bool,
) -> io::Result<bool> {
    let (content_type, body) = match path {
        "/gateway-ui/dashboard.css" => (
            "text/css; charset=utf-8",
            include_str!("../../../assets/ui/dashboard.css"),
        ),
        "/gateway-ui/dashboard.js" => (
            "application/javascript; charset=utf-8",
            include_str!("../../../assets/ui/dashboard.js"),
        ),
        "/gateway-ui/studio.css" => (
            "text/css; charset=utf-8",
            include_str!("../../../assets/ui/studio.css"),
        ),
        "/gateway-ui/studio.js" => (
            "application/javascript; charset=utf-8",
            include_str!("../../../assets/ui/studio.js"),
        ),
        _ => return Ok(false),
    };
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nCache-Control: no-cache, no-store, must-revalidate\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if include_body {
        client.write_all(body.as_bytes())?;
    }
    client.flush()?;
    Ok(true)
}

pub fn handle_dashboard_overview(client: &mut TcpStream, store: &ConfigStore) -> io::Result<()> {
    let config = store.snapshot();
    let pool = store.jev_pool();
    let jev_accounts: Vec<serde_json::Value> = pool
        .accounts()
        .iter()
        .map(|acc| {
            serde_json::json!({
                "label": acc.label,
                "masked_key": mask_key(&acc.api_key),
            })
        })
        .collect();

    let or_accounts: Vec<serde_json::Value> = pool
        .openrouter_accounts()
        .iter()
        .map(|acc| {
            serde_json::json!({
                "label": acc.label,
                "masked_cookie": mask_key(&acc.cookie),
                "masked_api_key": mask_key(&acc.api_key),
                "masked_key": mask_key(if !acc.api_key.is_empty() { &acc.api_key } else { &acc.cookie }),
            })
        })
        .collect();

    let overview = serde_json::json!({
        "status": "ok",
        "config": {
            "target_host": config.target_host,
            "target_port": config.target_port,
            "fallbacks": config.fallbacks,
            "default_model": config.default_model,
            "io_timeout_secs": config.io_timeout.map(|d| d.as_secs()).unwrap_or(0),
            "config_storage": store.config_file.file_name().unwrap_or_default().to_string_lossy(),
        },
        "providers": {
            "jev": {
                "name": "Jev Decision API",
                "base_url": config.jev_base_url,
                "total_accounts": jev_accounts.len(),
                "exhausted_accounts": pool.exhausted_accounts_count(),
                "accounts": jev_accounts,
            },
            "openrouter_jev": {
                "name": "OpenRouter JEV (Browser / Cookie / API Key)",
                "base_url": config.jev_openrouter_base_url,
                "model": config.jev_openrouter_model,
                "fallback_models": crate::providers::jev::accounts::effective_fallback_models(&config),
                "enabled": config.jev_openrouter_enabled,
                "credit_cooldown_secs": config.jev_credit_cooldown_secs,
                "masked_cookie": mask_key(&config.jev_openrouter_cookie),
                "masked_api_key": mask_key(&config.jev_openrouter_api_key),
                "total_accounts": or_accounts.len(),
                "accounts": or_accounts,
            },
            "omniroute": {
                "name": "OmniRoute Upstream Proxy",
                "target_host": config.target_host,
                "target_port": config.target_port,
                "fallbacks": config.fallbacks,
                "default_model": config.default_model,
            },
            "museai": {
                "name": "Muse AI Subsystem",
                "base_url": config.museai_base_url,
                "ws_url": config.museai_ws_url,
                "vm_id": config.museai_vm_id,
                "configured": !config.museai_cookie.is_empty() || !config.museai_access_token.is_empty(),
                "masked_cookie": mask_key(&config.museai_cookie),
                "masked_access_token": mask_key(&config.museai_access_token),
                "masked_notary_token": mask_key(&config.museai_notary_token),
                "has_cookie": !config.museai_cookie.is_empty(),
                "has_access_token": !config.museai_access_token.is_empty(),
                "has_notary_token": !config.museai_notary_token.is_empty(),
                "default_video_prompt": config.default_video_prompt,
                "approval": {
                    "mode": config.muse_approval.mode.as_str(),
                    "allowed_destinations": config.muse_approval.allowed_destinations,
                    "allowed_types": config.muse_approval.allowed_types,
                    "prefer_allow_once": config.muse_approval.prefer_allow_once,
                    "allow_always": config.muse_approval.allow_always,
                    "max_decisions": config.muse_approval.max_decisions_per_job,
                    "timeout_secs": config.muse_approval.approval_timeout_secs,
                },
            }
        },
        "modules": store.module_registry().read().map(|r| r.list_modules()).unwrap_or_default(),
        "module_configs": store.module_registry().read().map(|r| {
            let mut masked = serde_json::Map::new();
            for m in r.list_modules() {
                let cfg = r.get_masked_module_config(&m.id);
                if !cfg.is_empty() {
                    masked.insert(m.id, serde_json::Value::Object(cfg));
                }
            }
            masked
        }).unwrap_or_default(),
        "catalog": crate::gateway::modules::get_system_module_catalog(),
        "mcp": {
            "status": "enabled",
            "transports": ["stdio", "sse", "streamable_http", "http-sse"],
            "options": [
                {
                    "name": "stdio",
                    "type": "local",
                    "command": "toolcase-gateway --mcp",
                    "description": "Local — IDE spawns process via toolcase-gateway --mcp"
                },
                {
                    "name": "sse",
                    "type": "remote",
                    "endpoint": "/mcp/sse",
                    "description": "Remote — Server-Sent Events over HTTP"
                },
                {
                    "name": "streamable_http",
                    "type": "remote",
                    "endpoint": "/mcp",
                    "description": "Remote — Modern bidirectional HTTP"
                }
            ],
            "endpoints": {
                "jsonrpc": "/mcp",
                "sse": "/mcp/sse"
            },
            "tools": crate::features::mcp::list_tools(),
            "resources": crate::features::mcp::list_resources(),
        }
    });

    let body = serde_json::to_vec(&overview).unwrap_or_default();
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    client.write_all(&body)?;
    client.flush()
}
