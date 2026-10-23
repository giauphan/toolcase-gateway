use std::io::{self, Write};
use std::net::TcpStream;

use crate::config::ConfigStore;
use crate::http::write_error;

use super::service::mask_key;

#[derive(serde::Deserialize)]
pub struct UpdateJevConfigPayload {
    pub jev_base_url: Option<String>,
    pub jev_openrouter_base_url: Option<String>,
    pub jev_openrouter_api_key: Option<String>,
    pub jev_openrouter_cookie: Option<String>,
    pub jev_openrouter_model: Option<String>,
    pub jev_openrouter_enabled: Option<bool>,
    pub jev_credit_cooldown_secs: Option<u64>,
}

pub fn handle_update_jev_config(
    client: &mut TcpStream,
    body: &[u8],
    store: &ConfigStore,
) -> io::Result<()> {
    let payload: UpdateJevConfigPayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(err) => {
            return write_error(client, 400, "Bad Request", &format!("Invalid JSON: {err}"));
        }
    };

    if let Some(ref base) = payload.jev_base_url {
        let trimmed = base.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("http://")
            && !trimmed.starts_with("https://")
        {
            return write_error(
                client,
                400,
                "Bad Request",
                "jev_base_url must start with http:// or https://",
            );
        }
    }

    if let Some(ref base) = payload.jev_openrouter_base_url {
        let trimmed = base.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("http://")
            && !trimmed.starts_with("https://")
        {
            return write_error(
                client,
                400,
                "Bad Request",
                "jev_openrouter_base_url must start with http:// or https://",
            );
        }
    }

    {
        let mut conf = store.write_guard();
        if let Some(base) = payload.jev_base_url {
            let trimmed = base.trim();
            if !trimmed.is_empty() {
                conf.jev_base_url = trimmed.to_string();
            }
        }
        if let Some(base) = payload.jev_openrouter_base_url {
            let trimmed = base.trim();
            if !trimmed.is_empty() {
                conf.jev_openrouter_base_url = trimmed.to_string();
            }
        }
        if let Some(key) = payload.jev_openrouter_api_key {
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                conf.jev_openrouter_api_key = trimmed.to_string();
            }
        }
        if let Some(cookie) = payload.jev_openrouter_cookie {
            let trimmed = cookie.trim();
            if !trimmed.is_empty() {
                conf.jev_openrouter_cookie = trimmed.to_string();
            }
        }
        if let Some(model) = payload.jev_openrouter_model {
            let trimmed = model.trim();
            if !trimmed.is_empty() {
                conf.jev_openrouter_model = trimmed.to_string();
            }
        }
        if let Some(enabled) = payload.jev_openrouter_enabled {
            conf.jev_openrouter_enabled = enabled;
        }
        if let Some(secs) = payload.jev_credit_cooldown_secs {
            if secs > 0 {
                conf.jev_credit_cooldown_secs = secs;
            }
        }
    }

    let _ = store.save_to_persistent_file();

    let snapshot = store.snapshot();
    let resp = serde_json::json!({
        "ok": true,
        "message": "Jev and OpenRouter configuration saved to persistent storage",
        "config": {
            "jev_base_url": snapshot.jev_base_url,
            "jev_openrouter_base_url": snapshot.jev_openrouter_base_url,
            "jev_openrouter_model": snapshot.jev_openrouter_model,
            "jev_openrouter_enabled": snapshot.jev_openrouter_enabled,
            "jev_credit_cooldown_secs": snapshot.jev_credit_cooldown_secs,
            "jev_openrouter_cookie": mask_key(&snapshot.jev_openrouter_cookie),
            "jev_openrouter_api_key": mask_key(&snapshot.jev_openrouter_api_key),
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
