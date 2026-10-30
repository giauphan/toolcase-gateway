use super::{apply_muse_session_metadata, debug_log, validate_muse_session_readiness};
use crate::config::Config;
use std::io;

fn extract_subdomain_vm(url: &str) -> String {
    if url.contains("metaaivm.com") && !url.contains("hatch.metaaivm.com") {
        url.split("://")
            .nth(1)
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    }
}

fn build_vm_address(config: &Config) -> String {
    if config.museai_ws_url.is_empty() {
        "wss://hatch.metaaivm.com/v1/noise".to_string()
    } else {
        let configured = config.museai_ws_url.clone();
        if let Some(idx) = configured.find('?') {
            configured[..idx].to_string()
        } else {
            configured
        }
    }
}

fn wake_vm_if_needed(base_url: &str, active_vm_id: &str, cookie: &str) {
    let wake_body = serde_json::json!({
        "vm_id": active_vm_id,
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

    if !cookie.is_empty() {
        wake_req = wake_req.header("Cookie", cookie);
    }

    match wake_req.send_json(wake_body) {
        Ok(response) => {
            if response.status() == 200 {
                let wake_status = response
                    .into_body()
                    .read_json::<serde_json::Value>()
                    .ok()
                    .and_then(|wake| {
                        wake.get("status")
                            .and_then(|value| value.as_str())
                            .map(str::to_owned)
                    });
                match wake_status.as_deref() {
                    Some("wake_requested" | "already_running" | "ready") => debug_log(
                        "Note: Muse VM wake acknowledged; readiness still requires RUNNING session",
                    ),
                    Some("failed") => debug_log("Note: Muse VM wake endpoint returned failed status"),
                    _ => debug_log(
                        "Note: Muse VM wake returned HTTP 200; readiness still requires RUNNING session",
                    ),
                }
            }
        }
        Err(e) => {
            debug_log(&format!("Note: Failed to wake VM. Error: {e}"));
        }
    }
}

fn fetch_auth_check_token(base_url: &str, cookie: &str) -> Option<String> {
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
    if !cookie.is_empty() {
        auth_request = auth_request.header("Cookie", cookie);
    }
    match auth_request.send_empty() {
        Ok(response) => {
            let auth = response.into_body().read_json::<serde_json::Value>().ok()?;
            auth.get("access_token")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        }
        Err(error) => {
            debug_log(&format!("Note: POST /api/auth/check failed: {error}"));
            None
        }
    }
}

fn fetch_hatch_token(
    base_url: &str,
    cookie: &str,
    body: serde_json::Value,
    bootstrapped: &mut Config,
) {
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

    if !cookie.is_empty() {
        request = request.header("Cookie", cookie);
    }

    match request.send_json(body) {
        Ok(response) => {
            if let Ok(session) = response.into_body().read_json::<serde_json::Value>() {
                if bootstrapped.museai_access_token.is_empty() {
                    if let Some(token) = session.get("token").and_then(|v| v.as_str()) {
                        bootstrapped.museai_access_token = token.to_owned();
                    }
                }
                if bootstrapped.museai_notary_token.is_empty() {
                    if let Some(notary) = session.get("notary_token").and_then(|v| v.as_str()) {
                        bootstrapped.museai_notary_token = notary.to_owned();
                    }
                }
            }
        }
        Err(e) => {
            debug_log(&format!("Note: POST /api/hatch/token failed: {e}"));
        }
    }
}

fn fetch_session_metadata(base_url: &str, cookie: &str) -> io::Result<serde_json::Value> {
    let mut request = ureq::get(&format!("{base_url}/api/session"))
        .header("Accept", "application/json")
        .header("Origin", base_url)
        .header("Referer", &format!("{base_url}/"));
    if !cookie.is_empty() {
        request = request.header("Cookie", cookie);
    }
    let mut response = match request.call() {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(401 | 403)) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Muse.ai session bootstrap failed: authentication rejected (cookie expired or invalid, HTTP 401 Unauthorized). Please update your session cookie in settings.",
            ));
        }
        Err(error) => {
            return Err(io::Error::other(format!(
                "Muse.ai session bootstrap failed: {error}"
            )));
        }
    };
    let status_code = response.status().as_u16();
    if status_code == 401 || status_code == 403 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Muse.ai session bootstrap failed: authentication rejected (cookie expired or invalid, HTTP 401 Unauthorized). Please update your session cookie in settings.",
        ));
    }
    if !(200..300).contains(&status_code) {
        return Err(io::Error::other(format!(
            "Muse.ai session bootstrap returned unexpected status HTTP {status_code}"
        )));
    }
    response
        .body_mut()
        .read_json()
        .map_err(|error| io::Error::other(format!("Muse.ai session response invalid: {error}")))
}

pub fn bootstrap_museai_config(config: &Config) -> io::Result<Config> {
    let base_url = if config.museai_base_url.is_empty() {
        "https://muse.ai"
    } else {
        config.museai_base_url.trim_end_matches('/')
    };

    let vm_address = build_vm_address(config);
    let shared_vm = if config.museai_ws_url.is_empty() {
        extract_subdomain_vm(&config.museai_base_url)
    } else {
        extract_subdomain_vm(&config.museai_ws_url)
    };

    let active_vm_id = if !config.museai_vm_id.is_empty() && config.museai_vm_id != "." {
        config.museai_vm_id.clone()
    } else {
        shared_vm
    };

    let mut body = serde_json::Map::new();
    body.insert(
        "vmAddress".to_string(),
        serde_json::Value::String(vm_address),
    );

    if !active_vm_id.is_empty() {
        body.insert(
            "vmName".to_string(),
            serde_json::Value::String(active_vm_id.clone()),
        );
        wake_vm_if_needed(base_url, &active_vm_id, &config.museai_cookie);
    }

    let mut bootstrapped = config.clone();
    if let Some(token) = fetch_auth_check_token(base_url, &config.museai_cookie) {
        bootstrapped.museai_access_token = token;
    }

    if bootstrapped.museai_access_token.is_empty() || bootstrapped.museai_notary_token.is_empty() {
        fetch_hatch_token(
            base_url,
            &config.museai_cookie,
            serde_json::Value::Object(body),
            &mut bootstrapped,
        );
    }

    let session = fetch_session_metadata(base_url, &config.museai_cookie)?;
    validate_muse_session_readiness(&session)?;
    apply_muse_session_metadata(&mut bootstrapped, &session);
    if bootstrapped.museai_access_token.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Muse.ai authentication did not return an access token",
        ));
    }
    Ok(bootstrapped)
}
