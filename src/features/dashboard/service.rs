use crate::config::ConfigStore;
use crate::http::write_error;
use std::fs;
use std::io::{self, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

pub fn mask_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.len() > 10 {
        format!("{}...{}", &trimmed[..6], &trimmed[trimmed.len() - 4..])
    } else if !trimmed.is_empty() {
        "***".to_string()
    } else {
        String::new()
    }
}

pub fn handle_dashboard_page(client: &mut TcpStream) -> io::Result<()> {
    let html = include_str!("../../../assets/dashboard.html");
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    )?;
    client.flush()
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

#[derive(serde::Deserialize)]
struct AddAccountsPayload {
    provider: Option<String>,
    label: Option<String>,
    api_keys: Option<serde_json::Value>,
    keys: Option<serde_json::Value>,
    accounts: Option<serde_json::Value>,
}

fn parse_string_entries(
    s: &str,
    default_label: Option<&str>,
    is_cookie: bool,
) -> Vec<(String, Option<String>)> {
    let delimiters: &[char] = if is_cookie {
        &['\n', '\r']
    } else {
        &['\n', '\r', ',']
    };
    s.split(delimiters)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            if let Some((lbl, val)) = line.split_once(':') {
                let lbl = lbl.trim();
                let val = val.trim();
                if !lbl.is_empty() && !val.is_empty() && !lbl.contains('=') && !lbl.contains(';') {
                    return (val.to_string(), Some(lbl.to_string()));
                }
            }
            if !is_cookie {
                if let Some((lbl, key)) = line.split_once('=') {
                    let lbl = lbl.trim();
                    let key = key.trim();
                    if !lbl.is_empty() && !key.is_empty() && key.starts_with("sk-") {
                        return (key.to_string(), Some(lbl.to_string()));
                    }
                }
            }
            (line.to_string(), default_label.map(str::to_string))
        })
        .collect()
}

fn extract_entry_from_value(
    val: &serde_json::Value,
    default_label: Option<&str>,
    is_cookie: bool,
) -> Vec<(String, Option<String>)> {
    match val {
        serde_json::Value::Array(arr) => arr
            .iter()
            .flat_map(|item| extract_entry_from_value(item, default_label, is_cookie))
            .collect(),
        serde_json::Value::Object(map) => {
            let key = map
                .get("api_key")
                .or_else(|| map.get("key"))
                .or_else(|| map.get("cookie"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let label = map
                .get("label")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| default_label.map(str::to_string));
            if let Some(k) = key {
                vec![(k.to_string(), label)]
            } else {
                Vec::new()
            }
        }
        serde_json::Value::String(s) => parse_string_entries(s, default_label, is_cookie),
        _ => Vec::new(),
    }
}

fn extract_account_entries(
    payload: &AddAccountsPayload,
    is_cookie: bool,
) -> Vec<(String, Option<String>)> {
    let mut entries = Vec::new();
    let default_label = payload
        .label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if let Some(ref val) = payload.accounts {
        entries.extend(extract_entry_from_value(val, default_label, is_cookie));
    }
    if let Some(ref val) = payload.api_keys {
        entries.extend(extract_entry_from_value(val, default_label, is_cookie));
    }
    if let Some(ref val) = payload.keys {
        entries.extend(extract_entry_from_value(val, default_label, is_cookie));
    }

    if let Some(dl) = default_label {
        let count = entries
            .iter()
            .filter(|(_, l)| l.as_deref() == Some(dl))
            .count();
        if count > 1 {
            let mut sub_idx = 1;
            for (_, l) in &mut entries {
                if l.as_deref() == Some(dl) {
                    *l = Some(format!("{dl}-{sub_idx}"));
                    sub_idx += 1;
                }
            }
        }
    }

    entries
}

pub fn handle_save_provider_settings(
    client: &mut TcpStream,
    body: &[u8],
    store: &ConfigStore,
) -> io::Result<()> {
    let payload: AddAccountsPayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(err) => {
            return write_error(client, 400, "Bad Request", &format!("Invalid JSON: {err}"));
        }
    };

    let provider = payload
        .provider
        .as_deref()
        .unwrap_or("jev")
        .trim()
        .to_ascii_lowercase();

    let is_direct_jev = provider == "jev" || provider == "jev-ai";
    let is_openrouter_key = provider == "openrouter-key"
        || provider == "openrouter_key"
        || provider == "openrouter/key"
        || provider == "openroute-key"
        || provider == "openroute/key";
    let is_openrouter_cookie = provider == "openrouter-cookie"
        || provider == "openrouter_cookie"
        || provider == "openrouter/cookie"
        || provider == "openroute-cookie"
        || provider == "openroute/cookie"
        || provider == "jev-browser";
    let is_openrouter_general = provider == "openrouter"
        || provider == "openroute"
        || provider == "openroute/jev"
        || provider == "openrouter/jev";

    if !is_direct_jev && !is_openrouter_key && !is_openrouter_cookie && !is_openrouter_general {
        return write_error(
            client,
            400,
            "Bad Request",
            &format!("Unsupported provider '{provider}'. Supported providers: 'jev', 'openrouter-key', 'openrouter-cookie', 'openrouter'"),
        );
    }

    let entries = extract_account_entries(&payload, is_openrouter_cookie);

    if entries.is_empty() {
        return write_error(
            client,
            400,
            "Bad Request",
            "No valid non-empty API keys provided",
        );
    }

    if is_openrouter_key || is_openrouter_cookie || is_openrouter_general {
        let force_type = if is_openrouter_key {
            Some(true)
        } else if is_openrouter_cookie {
            Some(false)
        } else {
            None
        };
        let added_count = store.add_jev_openrouter_typed_accounts(&entries, force_type);
        let _ = store.save_to_persistent_file();

        let accounts: Vec<serde_json::Value> = store
            .jev_pool()
            .openrouter_accounts()
            .iter()
            .map(|acc| {
                serde_json::json!({
                    "label": acc.label,
                    "masked_key": mask_key(if !acc.api_key.is_empty() { &acc.api_key } else { &acc.cookie }),
                    "masked_cookie": mask_key(&acc.cookie),
                    "masked_api_key": mask_key(&acc.api_key),
                })
            })
            .collect();

        let resp = serde_json::json!({
            "ok": true,
            "provider": "openrouter/jev",
            "added_count": added_count,
            "total_accounts": accounts.len(),
            "accounts": accounts,
        });

        let resp_bytes = serde_json::to_vec(&resp).unwrap_or_default();
        write!(
            client,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            resp_bytes.len()
        )?;
        client.write_all(&resp_bytes)?;
        return client.flush();
    }

    let added_count = store.add_jev_accounts(&entries);
    let _ = store.save_to_persistent_file();

    let accounts: Vec<serde_json::Value> = store
        .jev_pool()
        .accounts()
        .iter()
        .map(|acc| {
            serde_json::json!({
                "label": acc.label,
                "masked_key": mask_key(&acc.api_key),
            })
        })
        .collect();

    let resp = serde_json::json!({
        "ok": true,
        "provider": "jev",
        "added_count": added_count,
        "total_accounts": accounts.len(),
        "accounts": accounts,
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

pub fn persist_jev_keys(path: &Path, keys: &[String]) -> io::Result<()> {
    if keys.is_empty() {
        return Ok(());
    }

    let joined_keys = keys.join(",");
    let new_line = format!("GW_JEV_API_KEYS=\"{joined_keys}\"");

    let lines: Vec<String> = if path.exists() {
        fs::read_to_string(path)?
            .lines()
            .map(ToString::to_string)
            .collect()
    } else {
        Vec::new()
    };

    let mut replaced = false;
    let mut updated_lines = Vec::with_capacity(lines.len() + 1);

    for line in lines {
        let trimmed = line.trim_start();
        let bare = trimmed.trim_start_matches("export ");
        if bare.starts_with("GW_JEV_API_KEYS=") {
            updated_lines.push(new_line.clone());
            replaced = true;
        } else {
            updated_lines.push(line);
        }
    }

    if !replaced {
        updated_lines.push(new_line);
    }

    let mut content = updated_lines.join("\n");
    if !content.ends_with('\n') {
        content.push('\n');
    }

    let parent = path.parent().unwrap_or(Path::new("."));
    if !parent.exists() {
        fs::create_dir_all(parent).ok();
    }

    let tmp = parent.join(format!(
        ".{}~tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));

    fs::write(&tmp, content.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

pub use handle_save_provider_settings as handle_add_provider_accounts;
