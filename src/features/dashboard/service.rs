pub use super::accounts::{
    handle_save_provider_settings, handle_save_provider_settings as handle_add_provider_accounts,
    persist_jev_keys,
};
pub use super::overview::{
    handle_dashboard_asset, handle_dashboard_overview, handle_dashboard_page, mask_key,
};

use crate::config::ConfigStore;
use crate::http::write_error;
use std::io::{self, Write};
use std::net::TcpStream;
use std::time::Duration;

#[derive(serde::Deserialize)]
pub struct UpdateConfigPayload {
    pub target_host: Option<String>,
    pub target_port: Option<u16>,
    pub fallbacks: Option<Vec<String>>,
    pub default_model: Option<String>,
    pub io_timeout_secs: Option<u64>,
    pub jev_base_url: Option<String>,
    pub jev_openrouter_base_url: Option<String>,
    pub jev_openrouter_api_key: Option<String>,
    pub jev_openrouter_cookie: Option<String>,
    pub jev_openrouter_model: Option<String>,
    pub jev_openrouter_fallback_models: Option<serde_json::Value>,
    pub jev_openrouter_enabled: Option<bool>,
}

pub fn handle_update_gateway_config(
    client: &mut TcpStream,
    body: &[u8],
    store: &ConfigStore,
) -> io::Result<()> {
    let payload: UpdateConfigPayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(err) => {
            return write_error(client, 400, "Bad Request", &format!("Invalid JSON: {err}"));
        }
    };

    if let Some(ref host) = payload.target_host {
        if host.trim().is_empty() {
            return write_error(client, 400, "Bad Request", "target_host cannot be empty");
        }
    }
    if let Some(port) = payload.target_port {
        if port == 0 {
            return write_error(
                client,
                400,
                "Bad Request",
                "target_port must be greater than 0",
            );
        }
    }

    {
        let mut conf = store.write_guard();
        if let Some(host) = payload.target_host {
            conf.target_host = host.trim().to_string();
        }
        if let Some(port) = payload.target_port {
            conf.target_port = port;
        }
        if let Some(fallbacks) = payload.fallbacks {
            conf.fallbacks = fallbacks
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(model) = payload.default_model {
            conf.default_model = model.trim().to_string();
        }
        if let Some(timeout) = payload.io_timeout_secs {
            conf.io_timeout = (timeout > 0).then(|| Duration::from_secs(timeout));
        }
        if let Some(base) = payload.jev_base_url {
            let t = base.trim();
            if !t.is_empty() {
                conf.jev_base_url = t.to_string();
            }
        }
        if let Some(base) = payload.jev_openrouter_base_url {
            let t = base.trim();
            if !t.is_empty() {
                conf.jev_openrouter_base_url = t.to_string();
            }
        }
        if let Some(key) = payload.jev_openrouter_api_key {
            let t = key.trim();
            if !t.is_empty() {
                conf.jev_openrouter_api_key = t.to_string();
            }
        }
        if let Some(cookie) = payload.jev_openrouter_cookie {
            let t = cookie.trim();
            if !t.is_empty() {
                conf.jev_openrouter_cookie = t.to_string();
            }
        }
        if let Some(model) = payload.jev_openrouter_model {
            let t = model.trim();
            if !t.is_empty() {
                conf.jev_openrouter_model = t.to_string();
            }
        }
        if let Some(ref val) = payload.jev_openrouter_fallback_models {
            conf.jev_openrouter_fallback_models = crate::config::parse_fallback_models(val);
        }
        if let Some(enabled) = payload.jev_openrouter_enabled {
            conf.jev_openrouter_enabled = enabled;
        }
    }

    let _ = store.save_to_persistent_file();

    let snapshot = store.snapshot();
    let resp = serde_json::json!({
        "ok": true,
        "message": "Gateway configuration saved to persistent storage",
        "config": {
            "target_host": snapshot.target_host,
            "target_port": snapshot.target_port,
            "fallbacks": snapshot.fallbacks,
            "default_model": snapshot.default_model,
            "io_timeout_secs": snapshot.io_timeout.map(|d| d.as_secs()).unwrap_or(0),
            "config_storage": store.config_file.file_name().unwrap_or_default().to_string_lossy(),
        }
    });

    let resp_bytes = serde_json::to_vec(&resp).unwrap_or_default();
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        resp_bytes.len()
    )?;
    client.write_all(&resp_bytes)?;
    client.flush()
}
