use std::io::{self, Write};
use std::net::TcpStream;

use crate::config::{Config, ConfigStore};
use crate::rewrite::escape_json_string;

use super::parser::extract_muse_config_from_har;
use super::storage::persist_muse_env;
use super::types::{
    mask_secret, safe_ws_url, AppliedField, ApplyReport, EnvReport, HarExtractError,
};

pub fn write_current_config(client: &mut TcpStream, config: &Config) -> io::Result<()> {
    let fields = vec![
        (!config.museai_ws_url.is_empty()).then(|| AppliedField {
            key: "ws_url",
            value: safe_ws_url(&config.museai_ws_url),
            masked: config.museai_ws_url.contains('?'),
        }),
        (!config.museai_base_url.is_empty()).then(|| AppliedField {
            key: "base_url",
            value: config.museai_base_url.clone(),
            masked: false,
        }),
        (!config.museai_access_token.is_empty()).then(|| AppliedField {
            key: "access_token",
            value: mask_secret(&config.museai_access_token),
            masked: true,
        }),
        (!config.museai_notary_token.is_empty()).then(|| AppliedField {
            key: "notary_token",
            value: mask_secret(&config.museai_notary_token),
            masked: true,
        }),
        (!config.museai_vm_id.is_empty()).then(|| AppliedField {
            key: "vm_id",
            value: config.museai_vm_id.clone(),
            masked: false,
        }),
        (!config.museai_cookie.is_empty()).then(|| AppliedField {
            key: "cookie",
            value: mask_secret(&config.museai_cookie),
            masked: true,
        }),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let body = serde_json::json!({"source": "current", "config": fields});
    let body = serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string());
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nX-Content-Type-Options: nosniff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )?;
    client.flush()
}

pub fn apply_har_config(
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

    let (kept, effective) = {
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
        (kept, guard.clone())
    };

    let applied = vec![
        extracted.ws_url.as_ref().map(|v| AppliedField {
            key: "ws_url",
            value: safe_ws_url(v),
            masked: v.contains('?'),
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

    let config = vec![
        (!effective.museai_ws_url.is_empty()).then(|| AppliedField {
            key: "ws_url",
            value: safe_ws_url(&effective.museai_ws_url),
            masked: effective.museai_ws_url.contains('?'),
        }),
        (!effective.museai_base_url.is_empty()).then(|| AppliedField {
            key: "base_url",
            value: effective.museai_base_url.clone(),
            masked: false,
        }),
        (!effective.museai_access_token.is_empty()).then(|| AppliedField {
            key: "access_token",
            value: mask_secret(&effective.museai_access_token),
            masked: true,
        }),
        (!effective.museai_notary_token.is_empty()).then(|| AppliedField {
            key: "notary_token",
            value: mask_secret(&effective.museai_notary_token),
            masked: true,
        }),
        (!effective.museai_vm_id.is_empty()).then(|| AppliedField {
            key: "vm_id",
            value: effective.museai_vm_id.clone(),
            masked: false,
        }),
        (!effective.museai_cookie.is_empty()).then(|| AppliedField {
            key: "cookie",
            value: mask_secret(&effective.museai_cookie),
            masked: true,
        }),
    ]
    .into_iter()
    .flatten()
    .collect();

    let env_pairs: Vec<(&str, &str)> = [
        ("GW_MUSEAI_WS_URL", effective.museai_ws_url.as_str()),
        ("GW_MUSEAI_BASE_URL", effective.museai_base_url.as_str()),
        (
            "GW_MUSEAI_ACCESS_TOKEN",
            effective.museai_access_token.as_str(),
        ),
        (
            "GW_MUSEAI_NOTARY_TOKEN",
            effective.museai_notary_token.as_str(),
        ),
        ("GW_MUSEAI_VM_ID", effective.museai_vm_id.as_str()),
        ("GW_MUSEAI_COOKIE", effective.museai_cookie.as_str()),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
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

    let _ = store.save_to_persistent_file();

    let report = ApplyReport {
        applied,
        config,
        kept,
        env,
    };
    let body = serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string());
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )?;
    client.flush()
}

pub fn write_json_error(
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
