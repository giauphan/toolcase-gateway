use super::overview::mask_key;
use crate::config::ConfigStore;
use crate::http::write_error;
use std::fs;
use std::io::{self, Write};
use std::net::TcpStream;
use std::path::Path;

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
