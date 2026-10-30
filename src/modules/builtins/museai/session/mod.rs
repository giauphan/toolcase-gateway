mod bootstrap;

pub use bootstrap::bootstrap_museai_config;

use crate::config::Config;
use std::io;

pub(super) fn debug_log(msg: &str) {
    if std::env::var("MUSEAI_DEBUG").is_ok() {
        crate::runtime::logger::log_message("DEBUG", "museai_session", msg);
    }
}

pub fn append_query_component(url: &mut String, key: &str, value: &str) {
    url.push(if url.contains('?') { '&' } else { '?' });
    url.push_str(key);
    url.push('=');
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                url.push(char::from(byte));
            }
            _ => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                url.push('%');
                url.push(char::from(HEX[(byte >> 4) as usize]));
                url.push(char::from(HEX[(byte & 0x0f) as usize]));
            }
        }
    }
}

pub fn is_allowed_muse_ws_authority(authority_path: &str) -> bool {
    let host = authority_path
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");
    host == "metaaivm.com" || host.ends_with(".metaaivm.com")
}

pub fn build_museai_ws_url(config: &Config, request_id: &str) -> io::Result<String> {
    let configured = config.museai_ws_url.as_str();

    let (base_url, query_str) = if let Some(idx) = configured.find('?') {
        (&configured[..idx], &configured[idx + 1..])
    } else {
        (configured, "")
    };

    if !base_url.is_empty() {
        let (scheme, rest) = base_url.split_once("://").ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Invalid WebSocket URL scheme")
        })?;
        if scheme != "wss" || !is_allowed_muse_ws_authority(rest) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Untrusted Muse WebSocket URL host",
            ));
        }
    }

    let mut parsed_query = std::collections::HashMap::new();
    for pair in query_str.split('&').filter(|s| !s.is_empty()) {
        let mut parts = pair.splitn(2, '=');
        if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
            parsed_query.insert(k.to_string(), v.to_string());
        }
    }

    let shared = base_url.contains("metaaivm.com") && !base_url.contains("hatch.metaaivm.com");
    let mut url = if shared || base_url.is_empty() {
        "wss://hatch.metaaivm.com/v1/noise".to_string()
    } else {
        base_url.to_string()
    };
    if url.ends_with('/') {
        url.pop();
    }
    if !url.ends_with("/v1/noise") {
        url.push_str("/v1/noise");
    }

    let vm_id = if let Some(v) = parsed_query.get("vm_id") {
        v.to_string()
    } else if !config.museai_vm_id.is_empty() && config.museai_vm_id != "." {
        config.museai_vm_id.as_str().to_string()
    } else if shared {
        base_url
            .split("://")
            .nth(1)
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or(".")
            .to_string()
    } else {
        ".".to_string()
    };

    let auth_token = if let Some(v) = parsed_query.get("auth_token") {
        v.to_string()
    } else {
        config.museai_access_token.clone()
    };
    if auth_token.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Muse.ai WebSocket configuration is missing auth_token",
        ));
    }

    let notary_token = if let Some(v) = parsed_query.get("notary_token") {
        v.to_string()
    } else {
        config.museai_notary_token.clone()
    };

    let app_id = parsed_query
        .get("app_id")
        .map(|s| s.as_str())
        .unwrap_or("hatch-web");

    append_query_component(&mut url, "vm_id", &vm_id);
    append_query_component(&mut url, "auth_token", &auth_token);
    if !notary_token.is_empty() {
        append_query_component(&mut url, "notary_token", &notary_token);
    }
    append_query_component(&mut url, "app_id", app_id);
    append_query_component(&mut url, "request_id", request_id);

    Ok(url)
}

pub fn validate_muse_session_readiness(session: &serde_json::Value) -> io::Result<()> {
    let Some(state) = session.get("vm_state").and_then(|value| value.as_str()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Muse VM session is missing readiness state",
        ));
    };

    match state.to_ascii_uppercase().as_str() {
        "RUNNING" => Ok(()),
        "UPDATING" | "STARTING" | "STOPPING" | "STOPPED" | "ERROR" => {
            Err(io::Error::other("Muse VM session is not ready"))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Muse VM session returned an unsupported readiness state",
        )),
    }
}

pub fn apply_muse_session_metadata(config: &mut Config, session: &serde_json::Value) {
    let selected_vm = session
        .get("vms")
        .and_then(|value| value.as_array())
        .and_then(|vms| {
            vms.iter()
                .find(|vm| vm.get("is_preferred").and_then(|value| value.as_bool()) == Some(true))
                .or_else(|| vms.iter().find(|vm| vm.get("endpoint_url").is_some()))
                .or_else(|| vms.first())
        });

    if config.museai_vm_id.is_empty() {
        config.museai_vm_id = session
            .get("vm_id")
            .and_then(|value| value.as_str())
            .or_else(|| {
                selected_vm.and_then(|vm| {
                    vm.get("vm_id")
                        .or_else(|| vm.get("id"))
                        .and_then(|value| value.as_str())
                })
            })
            .unwrap_or_default()
            .to_owned();
    }
    if config.museai_ws_url.is_empty() {
        config.museai_ws_url = session
            .get("endpoint_url")
            .and_then(|value| value.as_str())
            .or_else(|| {
                selected_vm.and_then(|vm| vm.get("endpoint_url").and_then(|value| value.as_str()))
            })
            .unwrap_or_default()
            .to_owned();
    }
    if config.museai_access_token.is_empty() {
        config.museai_access_token = session
            .get("auth_token")
            .or_else(|| session.get("access_token"))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned();
    }
    if config.museai_notary_token.is_empty() {
        config.museai_notary_token = session
            .get("notary_token")
            .or_else(|| selected_vm.and_then(|vm| vm.get("notary_token")))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned();
    }
}
