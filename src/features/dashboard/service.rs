use crate::config::ConfigStore;
use crate::http::write_error;
use std::fs;
use std::io::{self, Write};
use std::net::TcpStream;
use std::path::Path;

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

    let overview = serde_json::json!({
        "status": "ok",
        "providers": {
            "jev": {
                "name": "Jev Decision API",
                "base_url": config.jev_base_url,
                "total_accounts": jev_accounts.len(),
                "accounts": jev_accounts,
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
                "configured": !config.museai_cookie.is_empty() || !config.museai_access_token.is_empty(),
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
    api_keys: Option<serde_json::Value>,
    keys: Option<serde_json::Value>,
}

fn extract_keys_from_value(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        serde_json::Value::String(s) => s
            .split(&['\n', '\r', ','][..])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

pub fn handle_add_provider_accounts(
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

    if provider != "jev" && provider != "jev-ai" {
        return write_error(
            client,
            400,
            "Bad Request",
            &format!("Unsupported provider '{provider}'. Supported providers: 'jev'"),
        );
    }

    let mut keys = Vec::new();
    if let Some(ref val) = payload.api_keys {
        keys.extend(extract_keys_from_value(val));
    }
    if let Some(ref val) = payload.keys {
        keys.extend(extract_keys_from_value(val));
    }

    if keys.is_empty() {
        return write_error(
            client,
            400,
            "Bad Request",
            "No valid non-empty API keys provided",
        );
    }

    let added_count = store.add_jev_keys(&keys);
    let all_keys = store.jev_pool().keys();
    let _ = persist_jev_keys(&store.env_file, &all_keys);

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

pub fn persist_jev_keys(path: &Path, keys: &[String]) -> io::Result<()> {
    if keys.is_empty() {
        return Ok(());
    }

    let joined_keys = keys.join(",");
    let new_line = format!("GW_JEV_API_KEYS=\"{joined_keys}\"");

    let lines: Vec<String> = if path.exists() {
        fs::read_to_string(path)?
            .lines()
            .map(|l| l.to_string())
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
