use std::fs;
use std::io::{self, Write};
use std::net::TcpStream;
use std::path::Path;

use crate::config::{Config, ConfigStore};
use crate::rewrite::escape_json_string;

const MAX_HAR_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct MuseHarConfig {
    pub ws_url: Option<String>,
    pub base_url: Option<String>,
    pub access_token: Option<String>,
    pub notary_token: Option<String>,
    pub vm_id: Option<String>,
    pub cookie: Option<String>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum HarExtractError {
    InvalidJson(String),
    NotHar,
    NoMuseEntries,
}

impl MuseHarConfig {
    pub fn found_fields(&self) -> usize {
        [
            self.ws_url.is_some(),
            self.base_url.is_some(),
            self.access_token.is_some(),
            self.notary_token.is_some(),
            self.vm_id.is_some(),
            self.cookie.is_some(),
        ]
        .iter()
        .filter(|v| **v)
        .count()
    }

    pub fn keep_fields(&self, current: &Config) -> Vec<&'static str> {
        let checks: [(&Option<String>, &'static str); 6] = [
            (&self.ws_url, "ws_url"),
            (&self.base_url, "base_url"),
            (&self.access_token, "access_token"),
            (&self.notary_token, "notary_token"),
            (&self.vm_id, "vm_id"),
            (&self.cookie, "cookie"),
        ];
        checks
            .iter()
            .filter(|(found, _)| found.is_none())
            .map(|(_, name)| *name)
            .filter(|name| current_field_present(current, name))
            .collect()
    }
}

fn current_field_present(config: &Config, name: &str) -> bool {
    match name {
        "ws_url" => !config.museai_ws_url.is_empty(),
        "base_url" => !config.museai_base_url.is_empty(),
        "access_token" => !config.museai_access_token.is_empty(),
        "notary_token" => !config.museai_notary_token.is_empty(),
        "vm_id" => !config.museai_vm_id.is_empty(),
        "cookie" => !config.museai_cookie.is_empty(),
        _ => false,
    }
}

pub fn extract_muse_config_from_har(bytes: &[u8]) -> Result<MuseHarConfig, HarExtractError> {
    if bytes.len() > MAX_HAR_BYTES {
        return Err(HarExtractError::InvalidJson(
            "HAR exceeds 16 MB limit".to_string(),
        ));
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| HarExtractError::InvalidJson(e.to_string()))?;
    let entries = value
        .get("log")
        .and_then(|l| l.get("entries"))
        .and_then(|e| e.as_array())
        .ok_or(HarExtractError::NotHar)?;

    let mut out = MuseHarConfig::default();
    for entry in entries {
        let url = match entry
            .get("request")
            .and_then(|r| r.get("url"))
            .and_then(|u| u.as_str())
        {
            Some(u) => u,
            None => continue,
        };
        if let Some(ws) = parse_ws_muse_entry(url) {
            // Freshest (last) non-empty value wins; an entry missing a param
            // keeps the value found in an earlier entry.
            if let Some(v) = ws.ws_url {
                out.ws_url = Some(v);
            }
            if let Some(v) = ws.access_token {
                out.access_token = Some(v);
            }
            if let Some(v) = ws.notary_token {
                out.notary_token = Some(v);
            }
            if let Some(v) = ws.vm_id {
                out.vm_id = Some(v);
            }
            continue;
        }
        if let Some(rest) = parse_rest_muse_entry(url, entry) {
            out.base_url = Some(rest.base_url);
            out.cookie = rest.cookie;
            if rest.vm_id.is_some() {
                out.vm_id = rest.vm_id;
            }
        }
    }

    // Sanitize all extracted values: strip ASCII control chars so a crafted
    // HAR cannot break out of an env line (e.g. via an encoded newline).
    if let Some(v) = &mut out.ws_url {
        *v = sanitize_value(v);
    }
    if let Some(v) = &mut out.base_url {
        *v = sanitize_value(v);
    }
    if let Some(v) = &mut out.access_token {
        *v = sanitize_value(v);
    }
    if let Some(v) = &mut out.notary_token {
        *v = sanitize_value(v);
    }
    if let Some(v) = &mut out.vm_id {
        *v = sanitize_value(v);
    }
    if let Some(v) = &mut out.cookie {
        *v = sanitize_value(v);
    }

    if out.found_fields() == 0 {
        return Err(HarExtractError::NoMuseEntries);
    }
    Ok(out)
}

struct WsEntry {
    ws_url: Option<String>,
    access_token: Option<String>,
    notary_token: Option<String>,
    vm_id: Option<String>,
}

fn parse_ws_muse_entry(url: &str) -> Option<WsEntry> {
    let (scheme, rest) = split_scheme(url)?;
    if !matches!(scheme, "wss" | "ws") {
        // Not a WebSocket URL — let the REST-entry parser handle it.
        return None;
    }
    let (authority_path, query) = match rest.find('?') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, ""),
    };
    let host = authority_path.split('/').next().unwrap_or("");
    let path = authority_path
        .find('/')
        .map(|i| &authority_path[i..])
        .unwrap_or("");
    // Heuristic: a Muse Noise WS endpoint lives on metaaivm.com (or any path containing /noise)
    let is_muse = host.contains("metaaivm.com") || path.contains("/noise");
    if !is_muse {
        return None;
    }
    let params = parse_query(query);
    let ws_url = Some(format!("{}://{authority_path}", scheme));
    Some(WsEntry {
        ws_url,
        access_token: params
            .get("auth_token")
            .or_else(|| params.get("access_token"))
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty()),
        notary_token: params
            .get("notary_token")
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty()),
        vm_id: params
            .get("vm_id")
            .or_else(|| params.get("vmName"))
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty()),
    })
}

struct RestEntry {
    base_url: String,
    cookie: Option<String>,
    vm_id: Option<String>,
}

fn parse_rest_muse_entry(url: &str, entry: &serde_json::Value) -> Option<RestEntry> {
    let (scheme, rest) = split_scheme(url)?;
    if scheme != "https" {
        return None;
    }
    let authority_path = rest.split('?').next().unwrap_or(rest);
    let host = authority_path.split('/').next().unwrap_or("");
    let is_muse = host == "muse.ai" || host.ends_with(".muse.ai");
    if !is_muse {
        return None;
    }
    let cookie = entry
        .get("request")
        .and_then(|r| r.get("headers"))
        .and_then(|h| h.as_array())
        .and_then(|headers| {
            headers
                .iter()
                .find(|h| h.get("name").and_then(|n| n.as_str()) == Some("Cookie"))
                .and_then(|h| h.get("value").and_then(|v| v.as_str()))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    let vm_id = entry
        .get("response")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|body| {
            body.get("vm_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
        });
    let base_url = format!("{scheme}://{host}");
    Some(RestEntry {
        base_url,
        cookie,
        vm_id,
    })
}

fn split_scheme(url: &str) -> Option<(&str, &str)> {
    url.split_once("://")
}

fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for pair in query.split('&') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.find('=') {
            Some(i) => (&pair[..i], &pair[i + 1..]),
            None => (pair, ""),
        };
        // last occurrence wins, matching "freshest" semantics
        out.insert(percent_decode(k), percent_decode(v));
    }
    out
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn sanitize_value(s: &str) -> String {
    // Env-file injection prevention: remove ASCII control characters.
    // (Do not allow newline, carriage return, or NUL.)
    s.chars().filter(|c| !c.is_ascii_control()).collect()
}

pub fn mask_secret(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        return "****".to_string();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

#[derive(Debug, Clone)]
pub(crate) struct ApplyReport {
    pub applied: Vec<AppliedField>,
    pub kept: Vec<&'static str>,
    pub env: EnvReport,
}

#[derive(Debug, Clone)]
pub(crate) struct AppliedField {
    pub key: &'static str,
    pub value: String,
    pub masked: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum EnvReport {
    Written { path: String, keys: Vec<String> },
    SkippedNoFields,
    Failed { path: String, message: String },
}

pub(crate) fn apply_har_config(
    client: &mut TcpStream,
    body: &[u8],
    store: &ConfigStore,
) -> io::Result<()> {
    let extracted = match extract_muse_config_from_har(body) {
        Ok(v) => v,
        Err(e) => {
            let msg = match e {
                HarExtractError::InvalidJson(_) => "HAR is not valid JSON".to_string(),
                HarExtractError::NotHar => "Not a HAR file (missing log.entries)".to_string(),
                HarExtractError::NoMuseEntries => {
                    "No Muse WebSocket or REST entries found in HAR".to_string()
                }
            };
            return write_json_error(client, 400, "Bad Request", &msg);
        }
    };

    let kept = {
        let mut guard = store.write_guard();
        let kept = extracted.keep_fields(&guard);
        if let Some(v) = &extracted.ws_url {
            guard.museai_ws_url = v.clone();
        }
        if let Some(v) = &extracted.base_url {
            guard.museai_base_url = v.clone();
        }
        if let Some(v) = &extracted.access_token {
            guard.museai_access_token = v.clone();
        }
        if let Some(v) = &extracted.notary_token {
            guard.museai_notary_token = v.clone();
        }
        if let Some(v) = &extracted.vm_id {
            guard.museai_vm_id = v.clone();
        }
        if let Some(v) = &extracted.cookie {
            guard.museai_cookie = v.clone();
        }
        kept
    };

    let applied = vec![
        extracted.ws_url.as_ref().map(|v| AppliedField {
            key: "ws_url",
            value: v.clone(),
            masked: false,
        }),
        extracted.base_url.as_ref().map(|v| AppliedField {
            key: "base_url",
            value: v.clone(),
            masked: false,
        }),
        extracted.access_token.as_ref().map(|v| AppliedField {
            key: "access_token",
            value: mask_secret(v),
            masked: true,
        }),
        extracted.notary_token.as_ref().map(|v| AppliedField {
            key: "notary_token",
            value: mask_secret(v),
            masked: true,
        }),
        extracted.vm_id.as_ref().map(|v| AppliedField {
            key: "vm_id",
            value: v.clone(),
            masked: false,
        }),
        extracted.cookie.as_ref().map(|v| AppliedField {
            key: "cookie",
            value: mask_secret(v),
            masked: true,
        }),
    ]
    .into_iter()
    .flatten()
    .collect();

    let env_pairs: Vec<(&str, &str)> = vec![
        extracted
            .ws_url
            .as_deref()
            .map(|s| ("GW_MUSEAI_WS_URL", s))
            .filter(|(_, v)| !v.is_empty()),
        extracted
            .base_url
            .as_deref()
            .map(|s| ("GW_MUSEAI_BASE_URL", s))
            .filter(|(_, v)| !v.is_empty()),
        extracted
            .access_token
            .as_deref()
            .map(|s| ("GW_MUSEAI_ACCESS_TOKEN", s))
            .filter(|(_, v)| !v.is_empty()),
        extracted
            .notary_token
            .as_deref()
            .map(|s| ("GW_MUSEAI_NOTARY_TOKEN", s))
            .filter(|(_, v)| !v.is_empty()),
        extracted
            .vm_id
            .as_deref()
            .map(|s| ("GW_MUSEAI_VM_ID", s))
            .filter(|(_, v)| !v.is_empty()),
        extracted
            .cookie
            .as_deref()
            .map(|s| ("GW_MUSEAI_COOKIE", s))
            .filter(|(_, v)| !v.is_empty()),
    ]
    .into_iter()
    .flatten()
    .collect();

    let env = if env_pairs.is_empty() {
        EnvReport::SkippedNoFields
    } else {
        let path = &store.env_file;
        match persist_muse_env(path, &env_pairs) {
            Ok(()) => EnvReport::Written {
                path: path.display().to_string(),
                keys: env_pairs.into_iter().map(|(k, _)| k.to_string()).collect(),
            },
            Err(e) => EnvReport::Failed {
                path: path.display().to_string(),
                message: e.to_string(),
            },
        }
    };

    let report = ApplyReport { applied, kept, env };
    let body = serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string());
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )?;
    client.flush()
}

pub(crate) fn write_json_error(
    client: &mut TcpStream,
    status: u16,
    reason: &str,
    message: &str,
) -> io::Result<()> {
    let body = format!(
        r#"{{"error":{{"message":"{}"}}}}"#,
        escape_json_string(message)
    );
    write!(
        client,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nX-Content-Type-Options: nosniff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )?;
    client.flush()
}

impl serde::Serialize for ApplyReport {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("ApplyReport", 3)?;
        s.serialize_field("applied", &self.applied)?;
        s.serialize_field("kept", &self.kept)?;
        s.serialize_field("env", &self.env)?;
        s.end()
    }
}

impl serde::Serialize for AppliedField {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("AppliedField", 3)?;
        s.serialize_field("key", self.key)?;
        s.serialize_field("value", &self.value)?;
        s.serialize_field("masked", &self.masked)?;
        s.end()
    }
}

impl serde::Serialize for EnvReport {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        match self {
            EnvReport::Written { path, keys } => {
                let mut s = serializer.serialize_struct("Env", 3)?;
                s.serialize_field("status", "written")?;
                s.serialize_field("path", path)?;
                s.serialize_field("keys", keys)?;
                s.end()
            }
            EnvReport::SkippedNoFields => {
                let mut s = serializer.serialize_struct("Env", 1)?;
                s.serialize_field("status", "skipped_no_fields")?;
                s.end()
            }
            EnvReport::Failed { path, message } => {
                let mut s = serializer.serialize_struct("Env", 3)?;
                s.serialize_field("status", "failed")?;
                s.serialize_field("path", path)?;
                s.serialize_field("message", message)?;
                s.end()
            }
        }
    }
}

pub fn persist_muse_env(path: &Path, fields: &[(&str, &str)]) -> io::Result<()> {
    const MANAGED: [&str; 6] = [
        "GW_MUSEAI_WS_URL",
        "GW_MUSEAI_BASE_URL",
        "GW_MUSEAI_ACCESS_TOKEN",
        "GW_MUSEAI_NOTARY_TOKEN",
        "GW_MUSEAI_VM_ID",
        "GW_MUSEAI_COOKIE",
    ];

    let managed: std::collections::HashMap<&str, &str> = fields
        .iter()
        .filter(|(k, v)| MANAGED.contains(k) && !v.is_empty())
        .map(|(k, v)| (*k, *v))
        .collect();

    if managed.is_empty() {
        return Ok(());
    }

    let lines: Vec<String> = if path.exists() {
        fs::read_to_string(path)?
            .lines()
            .map(|l| l.to_string())
            .collect()
    } else {
        vec![]
    };

    // `kept_in_place` tracks managed keys whose existing line we keep untouched
    // (no new value supplied). Managed keys with a new value are dropped here and
    // re-appended below in their freshly encoded form.
    let mut kept_in_place: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut kept: Vec<String> = Vec::with_capacity(lines.len());
    for line in lines {
        let key = line
            .trim_start_matches(|c: char| c.is_whitespace())
            .split('=')
            .next()
            .unwrap_or("");
        let bare = key.trim_start_matches("export ");
        if MANAGED.contains(&bare) {
            if managed.contains_key(bare) {
                // New value available — drop the old line, re-append fresh below.
            } else {
                kept_in_place.insert(bare.to_string());
                kept.push(line);
            }
        } else {
            kept.push(line);
        }
    }
    let mut lines = kept;

    for (key, value) in &managed {
        if kept_in_place.contains(*key) {
            continue;
        }
        let quoted = if value.contains(' ') || value.contains('"') {
            format!(
                "{}=\"{}\"",
                key,
                value.replace('\\', "\\\\").replace('"', "\\\"")
            )
        } else {
            format!("{key}={value}")
        };
        lines.push(quoted);
    }

    let mut content = lines.join("\n");
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    if content.is_empty() {
        return Ok(());
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
