use crate::config::Config;
use std::io;

fn debug_log(msg: &str) {
    if std::env::var("MUSEAI_DEBUG").is_ok() {
        eprintln!("[museai_session] {msg}");
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
}

pub fn bootstrap_museai_config(config: &Config) -> io::Result<Config> {
    if !config.museai_access_token.is_empty() {
        return Ok(config.clone());
    }

    let base_url = if config.museai_base_url.is_empty() {
        "https://muse.ai"
    } else {
        config.museai_base_url.trim_end_matches('/')
    };

    let vm_address = if config.museai_ws_url.is_empty() {
        "wss://hatch.metaaivm.com/v1/noise".to_string()
    } else {
        let configured = config.museai_ws_url.clone();
        if let Some(idx) = configured.find('?') {
            configured[..idx].to_string()
        } else {
            configured
        }
    };

    let mut body = serde_json::Map::new();
    body.insert(
        "vmAddress".to_string(),
        serde_json::Value::String(vm_address),
    );

    let shared_vm = if config.museai_ws_url.is_empty() {
        if config.museai_base_url.contains("metaaivm.com")
            && !config.museai_base_url.contains("hatch.metaaivm.com")
        {
            config
                .museai_base_url
                .split("://")
                .nth(1)
                .unwrap_or("")
                .split('.')
                .next()
                .unwrap_or("")
                .to_string()
        } else {
            "".to_string()
        }
    } else {
        if config.museai_ws_url.contains("metaaivm.com")
            && !config.museai_ws_url.contains("hatch.metaaivm.com")
        {
            config
                .museai_ws_url
                .split("://")
                .nth(1)
                .unwrap_or("")
                .split('.')
                .next()
                .unwrap_or("")
                .to_string()
        } else {
            "".to_string()
        }
    };

    let active_vm_id = if !config.museai_vm_id.is_empty() && config.museai_vm_id != "." {
        config.museai_vm_id.clone()
    } else {
        shared_vm
    };

    if !active_vm_id.is_empty() {
        body.insert(
            "vmName".to_string(),
            serde_json::Value::String(active_vm_id.clone()),
        );

        let wake_body = serde_json::json!({
            "vm_id": active_vm_id.clone(),
            "retry_count": 0,
            "connect_attempt_id": uuid::Uuid::new_v4().to_string()
        });
        let mut wake_req = ureq::post(&format!("{base_url}/api/hatch/vm/wake"))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header("Origin", base_url)
            .header("Referer", &format!("{base_url}/"))
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36")
            .header("sec-ch-ua", r#""Brave";v="153", "Not_A Brand";v="8", "Chromium";v="153""#)
            .header("sec-ch-ua-mobile", "?0")
            .header("sec-ch-ua-platform", r#""Windows""#)
            .header("sec-fetch-dest", "empty")
            .header("sec-fetch-mode", "cors")
            .header("sec-fetch-site", "same-origin");

        if !config.museai_cookie.is_empty() {
            wake_req = wake_req.header("Cookie", &config.museai_cookie);
        }

        match wake_req.send_json(wake_body) {
            Ok(response) => {
                if response.status() == 200 {
                    debug_log("Note: Woke VM successfully");
                }
            }
            Err(e) => {
                debug_log(&format!("Note: Failed to wake VM. Error: {}", e));
            }
        }
    }

    let mut request = ureq::post(&format!("{base_url}/api/hatch/token"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("Origin", base_url)
        .header("Referer", &format!("{base_url}/"))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36")
        .header("sec-ch-ua", r#""Brave";v="153", "Not_A Brand";v="8", "Chromium";v="153""#)
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", r#""Windows""#)
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin");

    if !config.museai_cookie.is_empty() {
        request = request.header("Cookie", &config.museai_cookie);
    }

    let mut bootstrapped = config.clone();

    let mut auth_request = ureq::post(&format!("{base_url}/api/auth/check"))
        .header("Accept", "application/json")
        .header("Origin", base_url)
        .header("Referer", &format!("{base_url}/"))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Safari/537.36")
        .header("sec-ch-ua", r#""Chromium";v="154", "Brave";v="154", "Not A(Brand";v="99""#)
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", r#""Windows""#)
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin")
        .header("sec-gpc", "1")
        .header("cache-control", "no-cache")
        .header("pragma", "no-cache")
        .header("next-action", "009276f6a217bd1a06954e5ac591c5cbd42c0afca5");
    if !config.museai_cookie.is_empty() {
        auth_request = auth_request.header("Cookie", &config.museai_cookie);
    }
    match auth_request.send_empty() {
        Ok(response) => {
            if let Ok(auth) = response.into_body().read_json::<serde_json::Value>() {
                if let Some(token) = auth.get("access_token").and_then(|value| value.as_str()) {
                    bootstrapped.museai_access_token = token.to_owned();
                }
            }
        }
        Err(error) => debug_log(&format!("Note: POST /api/auth/check failed: {error}")),
    }

    if bootstrapped.museai_access_token.is_empty() {
        match request.send_json(serde_json::Value::Object(body)) {
            Ok(response) => {
                if let Ok(session) = response.into_body().read_json::<serde_json::Value>() {
                    if let Some(token) = session.get("token").and_then(|v| v.as_str()) {
                        bootstrapped.museai_access_token = token.to_owned();
                    }
                    if let Some(notary) = session.get("notary_token").and_then(|v| v.as_str()) {
                        bootstrapped.museai_notary_token = notary.to_owned();
                    }
                }
            }
            Err(e) => {
                debug_log(&format!("Note: POST /api/hatch/token failed: {e}"));
            }
        }
    }

    let mut request = ureq::get(&format!("{base_url}/api/session"))
        .header("Accept", "application/json")
        .header("Origin", base_url)
        .header("Referer", &format!("{base_url}/"));
    if !config.museai_cookie.is_empty() {
        request = request.header("Cookie", &config.museai_cookie);
    }
    let response = request
        .call()
        .map_err(|error| io::Error::other(format!("Muse.ai session bootstrap failed: {error}")))?;
    let session: serde_json::Value = response
        .into_body()
        .read_json()
        .map_err(|error| io::Error::other(format!("Muse.ai session response invalid: {error}")))?;

    apply_muse_session_metadata(&mut bootstrapped, &session);
    if bootstrapped.museai_access_token.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Muse.ai authentication did not return an access token",
        ));
    }
    Ok(bootstrapped)
}
