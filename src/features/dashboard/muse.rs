use std::io::{self, Write};
use std::net::TcpStream;

use crate::config::{is_valid_muse_approval_mode, parse_muse_approval_mode, Config, ConfigStore};
use crate::http::write_error;

use super::service::mask_key;

#[derive(serde::Deserialize, Default)]
pub struct MuseApprovalPayload {
    pub mode: Option<String>,
    pub allowed_destinations: Option<Vec<String>>,
    pub allowed_types: Option<Vec<String>>,
    pub prefer_allow_once: Option<bool>,
    pub allow_always: Option<bool>,
    pub max_decisions: Option<usize>,
    pub timeout_secs: Option<u64>,
}

#[derive(serde::Deserialize)]
pub struct UpdateMuseConfigPayload {
    pub base_url: Option<String>,
    pub ws_url: Option<String>,
    pub vm_id: Option<String>,
    pub access_token: Option<String>,
    pub notary_token: Option<String>,
    pub cookie: Option<String>,
    pub default_video_prompt: Option<String>,
    pub approval: Option<MuseApprovalPayload>,
    pub approval_mode: Option<String>,
    pub allowed_destinations: Option<Vec<String>>,
    pub allowed_types: Option<Vec<String>>,
    pub prefer_allow_once: Option<bool>,
    pub allow_always: Option<bool>,
    pub max_decisions: Option<usize>,
    pub timeout_secs: Option<u64>,
}

fn validate_muse_config_payload(payload: &UpdateMuseConfigPayload) -> Result<(), &'static str> {
    if let Some(ref base) = payload.base_url {
        let trimmed = base.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("http://")
            && !trimmed.starts_with("https://")
        {
            return Err("base_url must start with http:// or https://");
        }
    }
    if let Some(ref ws) = payload.ws_url {
        let trimmed = ws.trim();
        if !trimmed.is_empty() && !trimmed.starts_with("ws://") && !trimmed.starts_with("wss://") {
            return Err("ws_url must start with ws:// or wss://");
        }
    }
    let mode_candidate = payload
        .approval
        .as_ref()
        .and_then(|a| a.mode.as_ref())
        .or(payload.approval_mode.as_ref());
    if let Some(mode) = mode_candidate {
        let trimmed = mode.trim();
        if !trimmed.is_empty() && !is_valid_muse_approval_mode(trimmed) {
            return Err("approval_mode must be one of: 'off', 'observe', 'on'");
        }
    }
    Ok(())
}

fn update_non_empty_string(target: &mut String, val: Option<String>) {
    if let Some(v) = val {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            *target = trimmed.to_string();
        }
    }
}

fn clean_string_list(list: Option<Vec<String>>) -> Option<Vec<String>> {
    list.map(|items| {
        items
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    })
}

fn apply_muse_approval_settings(
    approval_conf: &mut crate::config::MuseApprovalConfig,
    app: MuseApprovalPayload,
) {
    if let Some(m) = app.mode {
        let trimmed = m.trim();
        if !trimmed.is_empty() {
            approval_conf.mode = parse_muse_approval_mode(trimmed);
        }
    }
    if let Some(clean_dests) = clean_string_list(app.allowed_destinations) {
        approval_conf.allowed_destinations = clean_dests;
    }
    if let Some(clean_types) = clean_string_list(app.allowed_types) {
        approval_conf.allowed_types = clean_types;
    }
    if let Some(pref) = app.prefer_allow_once {
        approval_conf.prefer_allow_once = pref;
    }
    if let Some(always) = app.allow_always {
        approval_conf.allow_always = always;
    }
    if let Some(max) = app.max_decisions {
        approval_conf.max_decisions_per_job = max;
    }
    if let Some(timeout) = app.timeout_secs {
        approval_conf.approval_timeout_secs = timeout;
    }
}

fn apply_muse_payload(conf: &mut Config, payload: UpdateMuseConfigPayload) {
    let UpdateMuseConfigPayload {
        base_url,
        ws_url,
        vm_id,
        access_token,
        notary_token,
        cookie,
        default_video_prompt,
        approval,
        approval_mode,
        allowed_destinations,
        allowed_types,
        prefer_allow_once,
        allow_always,
        max_decisions,
        timeout_secs,
    } = payload;

    update_non_empty_string(&mut conf.museai_base_url, base_url);
    update_non_empty_string(&mut conf.museai_ws_url, ws_url);
    if let Some(vm) = vm_id {
        let trimmed = vm.trim();
        if trimmed.is_empty() || trimmed == "vm-438" {
            conf.museai_vm_id.clear();
        } else {
            conf.museai_vm_id = trimmed.to_string();
        }
    }
    update_non_empty_string(&mut conf.museai_access_token, access_token);
    update_non_empty_string(&mut conf.museai_notary_token, notary_token);
    update_non_empty_string(&mut conf.museai_cookie, cookie);
    update_non_empty_string(&mut conf.default_video_prompt, default_video_prompt);

    let mut app = approval.unwrap_or_default();
    if app.mode.is_none() {
        app.mode = approval_mode;
    }
    if app.allowed_destinations.is_none() {
        app.allowed_destinations = allowed_destinations;
    }
    if app.allowed_types.is_none() {
        app.allowed_types = allowed_types;
    }
    if app.prefer_allow_once.is_none() {
        app.prefer_allow_once = prefer_allow_once;
    }
    if app.allow_always.is_none() {
        app.allow_always = allow_always;
    }
    if app.max_decisions.is_none() {
        app.max_decisions = max_decisions;
    }
    if app.timeout_secs.is_none() {
        app.timeout_secs = timeout_secs;
    }

    apply_muse_approval_settings(&mut conf.muse_approval, app);
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

    if let Err(msg) = validate_muse_config_payload(&payload) {
        return write_error(client, 400, "Bad Request", msg);
    }

    {
        let mut conf = store.write_guard();
        apply_muse_payload(&mut conf, payload);
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
