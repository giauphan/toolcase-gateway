use std::io::{self, Write};
use std::net::TcpStream;

use crate::config::{is_valid_muse_approval_mode, parse_muse_approval_mode, ConfigStore};
use crate::http::write_error;

use super::service::mask_key;

#[derive(serde::Deserialize)]
pub struct UpdateMuseConfigPayload {
    pub base_url: Option<String>,
    pub ws_url: Option<String>,
    pub vm_id: Option<String>,
    pub access_token: Option<String>,
    pub notary_token: Option<String>,
    pub cookie: Option<String>,
    pub default_video_prompt: Option<String>,
    pub approval_mode: Option<String>,
    pub allowed_destinations: Option<Vec<String>>,
    pub allowed_types: Option<Vec<String>>,
    pub prefer_allow_once: Option<bool>,
    pub allow_always: Option<bool>,
    pub max_decisions: Option<usize>,
    pub timeout_secs: Option<u64>,
}

pub fn handle_update_muse_config(
    client: &mut TcpStream,
    body: &[u8],
    store: &ConfigStore,
) -> io::Result<()> {
    let payload: UpdateMuseConfigPayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(err) => {
            return write_error(client, 400, "Bad Request", &format!("Invalid JSON: {err}"));
        }
    };

    if let Some(ref base) = payload.base_url {
        let trimmed = base.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("http://")
            && !trimmed.starts_with("https://")
        {
            return write_error(
                client,
                400,
                "Bad Request",
                "base_url must start with http:// or https://",
            );
        }
    }

    if let Some(ref ws) = payload.ws_url {
        let trimmed = ws.trim();
        if !trimmed.is_empty() && !trimmed.starts_with("ws://") && !trimmed.starts_with("wss://") {
            return write_error(
                client,
                400,
                "Bad Request",
                "ws_url must start with ws:// or wss://",
            );
        }
    }

    if let Some(ref mode) = payload.approval_mode {
        let trimmed = mode.trim();
        if !trimmed.is_empty() && !is_valid_muse_approval_mode(trimmed) {
            return write_error(
                client,
                400,
                "Bad Request",
                "approval_mode must be one of: 'off', 'observe', 'on'",
            );
        }
    }

    {
        let mut conf = store.write_guard();
        if let Some(base) = payload.base_url {
            let trimmed = base.trim();
            if !trimmed.is_empty() {
                conf.museai_base_url = trimmed.to_string();
            }
        }
        if let Some(ws) = payload.ws_url {
            let trimmed = ws.trim();
            if !trimmed.is_empty() {
                conf.museai_ws_url = trimmed.to_string();
            }
        }
        if let Some(vm) = payload.vm_id {
            conf.museai_vm_id = vm.trim().to_string();
        }
        if let Some(token) = payload.access_token {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                conf.museai_access_token = trimmed.to_string();
            }
        }
        if let Some(token) = payload.notary_token {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                conf.museai_notary_token = trimmed.to_string();
            }
        }
        if let Some(cookie) = payload.cookie {
            let trimmed = cookie.trim();
            if !trimmed.is_empty() {
                conf.museai_cookie = trimmed.to_string();
            }
        }
        if let Some(prompt) = payload.default_video_prompt {
            let trimmed = prompt.trim();
            if !trimmed.is_empty() {
                conf.default_video_prompt = trimmed.to_string();
            }
        }
        if let Some(mode) = payload.approval_mode {
            let trimmed = mode.trim();
            if !trimmed.is_empty() {
                conf.muse_approval.mode = parse_muse_approval_mode(trimmed);
            }
        }
        if let Some(dests) = payload.allowed_destinations {
            conf.muse_approval.allowed_destinations = dests
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(types) = payload.allowed_types {
            conf.muse_approval.allowed_types = types
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(pref) = payload.prefer_allow_once {
            conf.muse_approval.prefer_allow_once = pref;
        }
        if let Some(always) = payload.allow_always {
            conf.muse_approval.allow_always = always;
        }
        if let Some(max) = payload.max_decisions {
            conf.muse_approval.max_decisions_per_job = max;
        }
        if let Some(timeout) = payload.timeout_secs {
            conf.muse_approval.approval_timeout_secs = timeout;
        }
    }

    let _ = store.save_to_persistent_file();

    let snapshot = store.snapshot();
    let resp = serde_json::json!({
        "ok": true,
        "message": "Muse AI configuration updated and saved to persistent storage",
        "museai": {
            "base_url": snapshot.museai_base_url,
            "ws_url": snapshot.museai_ws_url,
            "vm_id": snapshot.museai_vm_id,
            "configured": !snapshot.museai_cookie.is_empty() || !snapshot.museai_access_token.is_empty(),
            "masked_cookie": mask_key(&snapshot.museai_cookie),
            "masked_access_token": mask_key(&snapshot.museai_access_token),
            "masked_notary_token": mask_key(&snapshot.museai_notary_token),
            "has_cookie": !snapshot.museai_cookie.is_empty(),
            "has_access_token": !snapshot.museai_access_token.is_empty(),
            "has_notary_token": !snapshot.museai_notary_token.is_empty(),
            "default_video_prompt": snapshot.default_video_prompt,
            "approval": {
                "mode": snapshot.muse_approval.mode.as_str(),
                "allowed_destinations": snapshot.muse_approval.allowed_destinations,
                "allowed_types": snapshot.muse_approval.allowed_types,
                "prefer_allow_once": snapshot.muse_approval.prefer_allow_once,
                "allow_always": snapshot.muse_approval.allow_always,
                "max_decisions": snapshot.muse_approval.max_decisions_per_job,
                "timeout_secs": snapshot.muse_approval.approval_timeout_secs,
            },
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
