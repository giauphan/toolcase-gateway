use crate::config::Config;
use crate::museai_noise::{MuseNoiseSession, NOISE_PATTERN_XX};
use crate::museai_protocol::{Header, ServiceFrameKind, SERVICE_DAEMON};
use crate::museai_transport::{send_museai_request, MuseWebSocket};
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

fn debug_log(msg: &str) {
    if std::env::var("MUSEAI_DEBUG").is_ok() {
        eprintln!("[museai] {msg}");
    }
}

fn append_query_component(url: &mut String, key: &str, value: &str) {
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

#[derive(Debug)]
pub(crate) struct MuseApprovalRequired(pub(crate) String);

impl std::fmt::Display for MuseApprovalRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for MuseApprovalRequired {}

pub(crate) struct TrackedThread {
    id: String,
    created_at: u64,
    base_url: String,
    config: Config,
}

fn tracked_threads() -> &'static Mutex<Vec<TrackedThread>> {
    static TRACKED_THREADS: OnceLock<Mutex<Vec<TrackedThread>>> = OnceLock::new();
    TRACKED_THREADS.get_or_init(|| Mutex::new(Vec::new()))
}

fn register_thread(thread_id: String, base_url: String, config: Config) {
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if let Ok(mut threads) = tracked_threads().lock() {
        threads.push(TrackedThread {
            id: thread_id,
            created_at,
            base_url,
            config,
        });
    }
}

fn is_allowed_muse_ws_authority(authority_path: &str) -> bool {
    let host = authority_path
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");
    host == "metaaivm.com" || host.ends_with(".metaaivm.com")
}

pub(crate) fn build_museai_ws_url(config: &Config, request_id: &str) -> io::Result<String> {
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

fn extract_user_prompt(request_body: &[u8]) -> io::Result<String> {
    let json: serde_json::Value = serde_json::from_slice(request_body)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid JSON: {e}")))?;

    if let Some(messages) = json.get("messages").and_then(|m| m.as_array()) {
        let mut full_prompt = String::new();
        for msg in messages {
            if let Some(role) = msg.get("role").and_then(|r| r.as_str()) {
                full_prompt.push_str(&format!("{}:\n", role));
            }
            if let Some(content) = msg.get("content") {
                if let Some(s) = content.as_str() {
                    full_prompt.push_str(s);
                } else if let Some(arr) = content.as_array() {
                    for item in arr {
                        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                            full_prompt.push_str(text);
                        } else if let Some(text) = item.as_str() {
                            full_prompt.push_str(text);
                        }
                    }
                }
            }
            full_prompt.push_str("\n\n");
        }
        let full_prompt = full_prompt.trim().to_string();
        if !full_prompt.is_empty() {
            return Ok(full_prompt);
        }
    }

    if let Some(prompt) = json.get("prompt").and_then(|p| p.as_str()) {
        return Ok(prompt.to_string());
    }

    Ok("Hello".to_string())
}

fn parse_assistant_content_from_json(val: &serde_json::Value) -> Option<String> {
    if let Some(event) = val.get("event").and_then(|e| e.as_str()) {
        if event == "task.status" || event == "agent.status" || event == "approvals.snapshot" {
            return None;
        }
    }

    if let Some(content) = val.get("content").and_then(|c| c.as_str()) {
        if !content.is_empty() {
            return Some(content.to_string());
        }
    }
    if let Some(text) = val.get("text").and_then(|t| t.as_str()) {
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    if let Some(delta) = val
        .get("delta")
        .and_then(|d| d.get("text"))
        .and_then(|t| t.as_str())
    {
        if !delta.is_empty() {
            return Some(delta.to_string());
        }
    }

    if let Some(payload) = val.get("payload") {
        if let Some(data) = payload.get("data") {
            if let Some(url) = data.get("url").and_then(|u| u.as_str()) {
                if !url.is_empty() {
                    return Some(format!("\nVideo URL: {url}\n"));
                }
            }
            if let Some(fallback) = data.get("fallback_text").and_then(|f| f.as_str()) {
                if !fallback.is_empty() {
                    return Some(format!("\nVideo: {fallback}\n"));
                }
            }
        }
        if let Some(text) = payload.get("text").and_then(|t| t.as_str()) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
        if let Some(content) = payload.get("content").and_then(|c| c.as_str()) {
            if !content.is_empty() {
                return Some(content.to_string());
            }
        }
        if let Some(transcript) = payload.get("transcript") {
            if let Some(messages) = transcript.get("messages").and_then(|m| m.as_array()) {
                let mut combined = String::new();
                for msg in messages {
                    if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                        continue;
                    }
                    if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                        for c in content {
                            if let Some(text) = c.get("text").and_then(|t| t.as_str()) {
                                combined.push_str(text);
                            }
                        }
                    } else if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
                        combined.push_str(text);
                    }
                }
                if !combined.is_empty() {
                    return Some(combined);
                }
            }
        }
    }
    if let Some(message) = val.get("message") {
        if let Some(text) = parse_assistant_content_from_json(message) {
            return Some(text);
        }
    }
    if let Some(events) = val.get("events").and_then(|e| e.as_array()) {
        let mut combined = String::new();
        for ev in events {
            if let Some(text) = parse_assistant_content_from_json(ev) {
                combined.push_str(&text);
            }
        }
        if !combined.is_empty() {
            return Some(combined);
        }
    }
    if let Some(items) = val.get("items").and_then(|i| i.as_array()) {
        let mut combined = String::new();
        for item in items {
            if let Some(text) = parse_assistant_content_from_json(item) {
                combined.push_str(&text);
            }
        }
        if !combined.is_empty() {
            return Some(combined);
        }
    }
    None
}

// Retained for legacy HTTP-action fixture coverage, not the production chat flow.
#[cfg(test)]
pub(crate) fn spawn_muse_thread(base_url: &str, config: &Config) -> io::Result<String> {
    let create_thread_url = format!("{base_url}/thread/new");
    let mut req = ureq::post(&create_thread_url)
        .header("Content-Type", "text/plain;charset=UTF-8")
        .header("Accept", "text/x-component")
        .header("Origin", base_url)
        .header("Referer", &format!("{base_url}/thread/new"))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36")
        .header("sec-ch-ua", r#""Brave";v="153", "Not_A Brand";v="8", "Chromium";v="153""#)
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", r#""Windows""#)
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin")
        .header("next-action", "00eaaa570484d537580e0bf4e19ee28a65ac048d62");

    if !config.museai_cookie.is_empty() {
        req = req.header("Cookie", &config.museai_cookie);
    }

    let response = match req.send("[]") {
        Ok(res) => res,
        Err(ureq::Error::StatusCode(401)) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Muse.ai thread creation returned 401 Unauthorized — credentials are invalid or expired",
            ));
        }
        Err(ureq::Error::StatusCode(403)) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Muse.ai thread creation returned 403 Forbidden — access denied",
            ));
        }
        Err(ureq::Error::StatusCode(404)) => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Muse.ai thread creation returned 404 — endpoint not found or cookie expired",
            ));
        }
        Err(ureq::Error::StatusCode(status)) => {
            return Err(io::Error::other(format!(
                "Muse.ai thread creation returned HTTP {status}"
            )));
        }
        Err(e) => {
            return Err(io::Error::other(format!(
                "Thread creation request failed: {e}"
            )));
        }
    };
    let mut reader = response.into_body();
    let text = reader
        .read_to_string()
        .map_err(|e| io::Error::other(format!("Failed to read thread response: {e}")))?;
    let thread_id = extract_new_thread_id(&text);
    if thread_id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "failed to extract thread ID from /thread/new response",
        ));
    }
    let channel = format!("thread:{thread_id}");
    Ok(channel)
}

#[cfg(test)]
fn extract_new_thread_id(rsc_text: &str) -> String {
    let needle = "/thread/";
    if let Some(idx) = rsc_text.rfind(needle) {
        let start = idx + needle.len();
        let remaining = &rsc_text[start..];
        let end = remaining
            .find(|c: char| !c.is_alphanumeric() && c != '-')
            .unwrap_or(remaining.len());
        return remaining[..end].to_string();
    }
    String::new()
}

fn apply_muse_session_metadata(config: &mut Config, session: &serde_json::Value) {
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

pub(crate) fn bootstrap_museai_config(config: &Config) -> io::Result<Config> {
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

#[derive(Default)]
struct MuseChatStream {
    pending: Vec<u8>,
    session_id: Option<String>,
    text: String,
    completed: bool,
}

impl MuseChatStream {
    fn push(&mut self, bytes: &[u8], ended: bool) -> io::Result<()> {
        const LIMIT: usize = 1024 * 1024;
        if self.pending.len().saturating_add(bytes.len()) > LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Muse chat record too large",
            ));
        }
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self
            .pending
            .iter()
            .position(|byte| *byte == b'\n')
            .or_else(|| (ended && !self.pending.is_empty()).then_some(self.pending.len()))
        {
            let line: Vec<u8> = self.pending.drain(..end).collect();
            if self.pending.first() == Some(&b'\n') {
                self.pending.remove(0);
            }
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let record: serde_json::Value = serde_json::from_slice(&line).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "Invalid Muse chat JSON record")
            })?;
            self.record(&record)?;
            if self.text.len() > LIMIT {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Muse chat output too large",
                ));
            }
        }
        Ok(())
    }

    fn record(&mut self, record: &serde_json::Value) -> io::Result<()> {
        #[cfg(test)]
        if std::env::var("MUSE_LIVE_RESUME_OWNED").ok().as_deref() == Some("1") {
            museai_tests::log_owned_record_schema(record);
        }
        let payload = record.get("payload").unwrap_or(record);
        let status = payload.get("status").and_then(|value| value.as_str());
        if let (Some(expected), Some(actual)) = (
            self.session_id.as_deref(),
            payload.get("session_id").and_then(|value| value.as_str()),
        ) {
            if actual != expected && record["type"] != "response" {
                return Ok(());
            }
        }

        if record.get("error").is_some_and(|error| !error.is_null())
            || record.get("ok").and_then(|value| value.as_bool()) == Some(false)
            || matches!(status, Some("failed" | "error" | "cancelled"))
        {
            return Err(io::Error::other(
                "Muse chat rejected or failed (private details suppressed)",
            ));
        }
        if self.session_id.is_none() {
            let result = record.get("result").unwrap_or(record);
            let id = result
                .get("session_id")
                .and_then(|value| value.as_str())
                .filter(|id| !id.is_empty());
            if let Some(id) = id {
                if Uuid::parse_str(id).is_err() || result["is_thread"].as_bool() != Some(true) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Muse did not acknowledge an isolated thread",
                    ));
                }
                self.session_id = Some(id.to_owned());
                debug_log("Fresh Muse session acknowledged");
                return Ok(());
            }
        }
        if record["type"] == "response" {
            if payload["subscribed"].as_bool() != Some(true)
                || payload["session_id"].as_str() != self.session_id.as_deref()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Muse subscription was not acknowledged for the requested session",
                ));
            }
            return Ok(());
        }
        let event = record
            .get("event")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let category = match event {
            "delta.text_append"
            | "delta.message_done"
            | "task.status"
            | "task.complete"
            | "agent.status"
            | "message.assistant"
            | "message.user"
            | "delta.presentation"
            | "delta.message_start"
            | "delta.tool_start"
            | "delta.tool_done"
            | "approvals.snapshot"
            | "delta.marker"
            | "delta.placeholder" => event,
            _ => "other",
        };
        debug_log(&format!("Chat event category={category}"));
        if event == "approvals.snapshot" {
            let requires_approval = payload
                .get("pending_approvals")
                .and_then(|value| value.as_array())
                .is_some_and(|approvals| {
                    approvals.iter().any(|approval| {
                        ["thread_id", "session_scope_id"].iter().any(|key| {
                            approval["scope"][key]
                                .as_str()
                                .is_some_and(|id| self.session_id.as_deref() == Some(id))
                        })
                    })
                });
            debug_log(&format!(
                "Approval snapshot requires_approval={requires_approval}"
            ));
            if requires_approval {
                debug_log("Scoped Muse permission approval is pending");
                return Err(io::Error::other(MuseApprovalRequired(
                    "Scoped Muse permission approval is pending".to_string(),
                )));
            }
        }
        if matches!(
            event,
            "delta.text_append" | "delta.message_done" | "message.assistant" | "delta.presentation"
        ) {
            if let Some(text) = parse_assistant_content_from_json(record) {
                self.text.push_str(&text);
            }
        }
        if matches!(event, "delta.message_done" | "task.complete") {
            self.completed = true;
        }
        Ok(())
    }

    fn finish(self) -> io::Result<(String, String)> {
        if !self.completed || !self.pending.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Muse chat ended without a complete assistant response",
            ));
        }
        Ok((
            self.session_id.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Muse chat did not acknowledge the session",
                )
            })?,
            self.text,
        ))
    }
}

pub(crate) fn build_muse_chat_request(prompt: &str) -> serde_json::Value {
    // The first-party draft flow creates the session on chat.stream, not via a
    // Next server action or a subscription to a nonexistent thread channel.
    serde_json::json!({
        "message": prompt,
        "node_id": Uuid::new_v4().to_string(),
        "session_id": Uuid::new_v4().to_string(),
        "capabilities": ["chat_cancel", "delta_stream"]
    })
}

pub(crate) fn request_museai_chat_completion(
    request_body: &[u8],
    config: &Config,
) -> io::Result<String> {
    let prompt = extract_user_prompt(request_body)?;
    let config = bootstrap_museai_config(config)?;

    let base_url = if config.museai_base_url.is_empty() {
        "https://muse.ai"
    } else {
        config.museai_base_url.as_str()
    };
    let base_origin = base_url.trim_end_matches('/');

    let req_id = Uuid::new_v4().to_string();
    let ws_url = build_museai_ws_url(&config, &req_id)?;

    // Browser site cookies are not credentials for the separate VM host.
    let mut socket = MuseWebSocket::connect(&ws_url, base_origin, "")?;
    let mut session = MuseNoiseSession::new_initiator(NOISE_PATTERN_XX, None)
        .map_err(|e| io::Error::other(format!("Noise initiator error: {e}")))?;

    session
        .perform_client_handshake(&mut socket, &config.museai_notary_token)
        .map_err(|e| io::Error::other(format!("Noise handshake error: {e}")))?;

    let stream_id = 1;
    let headers = vec![
        Header {
            key: "Host".to_string(),
            value: "hatch.metaaivm.com".to_string(),
        },
        Header {
            key: "Content-Type".to_string(),
            value: "application/json".to_string(),
        },
        Header {
            key: "Accept".to_string(),
            value: "text/event-stream".to_string(),
        },
        Header {
            key: "x-request-id".to_string(),
            value: Uuid::new_v4().to_string(),
        },
        Header {
            key: "x-app-id".to_string(),
            value: "hatch-web".to_string(),
        },
        Header {
            key: "Accept-Language".to_string(),
            value: "en-US".to_string(),
        },
    ];

    let chat_payload = build_muse_chat_request(&prompt);

    let payload_bytes = serde_json::to_vec(&chat_payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    debug_log("Sending encrypted POST /chat/stream over Noise...");

    socket.send_encrypted_service_request(
        &mut session,
        SERVICE_DAEMON,
        stream_id,
        "POST",
        "/chat/stream",
        &headers,
        &payload_bytes,
    )?;

    let mut stream = MuseChatStream::default();
    let mut active_stream_id = stream_id;
    for _ in 0..1000 {
        let frame = socket.read_encrypted_service_frame(&mut session)?;
        if frame.stream_id != active_stream_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unexpected Muse response stream",
            ));
        }
        let (bytes, ended) = match frame.kind {
            ServiceFrameKind::Response {
                status,
                body,
                end_body,
                ..
            } => {
                debug_log(&format!("Chat response HTTP status={status}"));
                if !(200..300).contains(&status) {
                    return Err(io::Error::other(format!(
                        "Muse chat returned HTTP {status}"
                    )));
                }
                (body, end_body)
            }
            ServiceFrameKind::BodyChunk { data, end_body } => (data, end_body),
            ServiceFrameKind::Reset { code, .. } => {
                return Err(io::Error::other(format!("Muse chat reset code={code}")));
            }
        };
        stream.push(&bytes, ended)?;
        if ended && active_stream_id == stream_id && !stream.completed {
            let session_id = stream.session_id.as_deref().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Muse chat acknowledgement missing",
                )
            })?;
            #[cfg(test)]
            if std::env::var("MUSE_LIVE_TEST").ok().as_deref() == Some("1") {
                museai_tests::save_live_owned_session(session_id)?;
            }
            let subscription = serde_json::json!({
                "session_id": session_id,
                "after_stream_seq": 0,
                "after_chat_event_seq": 0,
                "capabilities": ["chat_cancel", "delta_stream"]
            });
            active_stream_id = 2;
            socket.send_encrypted_service_request(
                &mut session,
                SERVICE_DAEMON,
                active_stream_id,
                "POST",
                "/chat/subscribe",
                &headers,
                subscription.to_string().as_bytes(),
            )?;
            continue;
        }
        if stream.completed || ended {
            break;
        }
    }
    let (thread_id, full_assistant_text) = stream.finish()?;

    // After task.complete, the daemon may still push delta.presentation events
    // on stream 2 carrying the generated video URL. Continue reading for up to
    // 300 seconds to collect it (as noted in live observations, ~300s).
    let mut video_url: Option<String> = None;
    {
        let deadline =
            std::time::Instant::now() + Duration::from_secs(MUSE_POST_COMPLETION_WAIT_SECS);
        let mut presentation_stream = MuseChatStream {
            session_id: Some(thread_id.clone()),
            ..Default::default()
        };
        while std::time::Instant::now() < deadline && video_url.is_none() {
            match socket.read_encrypted_service_frame(&mut session) {
                Ok(frame) => {
                    let (bytes, ended) = match frame.kind {
                        ServiceFrameKind::Response { body, end_body, .. } => (body, end_body),
                        ServiceFrameKind::BodyChunk { data, end_body } => (data, end_body),
                        ServiceFrameKind::Reset { .. } => break,
                    };
                    presentation_stream.push(&bytes, ended)?;
                    if let Some(extracted) = extract_video_url_from_stream(&presentation_stream) {
                        debug_log("Video artifact URL received");
                        video_url = Some(extracted);
                    }
                }
                Err(e) => {
                    debug_log(&format!("Video URL read error: {e}"));
                    break;
                }
            }
        }
    }

    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if config.museai_auto_cleanup_threads && !thread_id.is_empty() {
        if config.museai_thread_retention_secs == 0 {
            let _ = delete_muse_thread(base_origin, &thread_id, &config);
        } else {
            register_thread(thread_id, base_origin.to_string(), config.clone());
        }
    }

    let mut full_assistant_text = full_assistant_text;
    if let Some(url) = &video_url {
        if !full_assistant_text.contains(url.as_str()) {
            full_assistant_text.push_str(&format!("\nVideo URL: {url}\n"));
        }
    }
    let completion_response = serde_json::json!({
        "id": format!("chatcmpl-{}", Uuid::new_v4()),
        "object": "chat.completion",
        "created": created,
        "model": "muse",
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": full_assistant_text
                },
                "finish_reason": "stop"
            }
        ],
        "usage": {
            "prompt_tokens": prompt.len() / 4 + 1,
            "completion_tokens": full_assistant_text.len() / 4 + 1,
            "total_tokens": (prompt.len() + full_assistant_text.len()) / 4 + 2
        }
    });

    serde_json::to_string(&completion_response)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

const SUPPORTED_MODELS: [&str; 10] = [
    "muse",
    "muse-video",
    "gen-3",
    "gen-2",
    "kling",
    "gen-4",
    "gen-4.5",
    "gen-4-turbo",
    "aleph-2.0",
    "ruby",
];
const SUPPORTED_ASPECT_RATIOS: [&str; 5] = ["16:9", "9:16", "1:1", "5:4", "4:3"];
const MUSE_POST_COMPLETION_WAIT_SECS: u64 = 300;

fn extract_video_url_from_stream(stream: &MuseChatStream) -> Option<String> {
    let extracted = extract_url_from_text(&stream.text);
    (!extracted.is_empty()).then_some(extracted)
}

pub(crate) fn build_video_prompt(
    model: &str,
    prompt: &str,
    aspect_ratio: &str,
    duration: u64,
) -> String {
    let _ = model;
    format!(
        "Create a video with Muse Video. Prompt: \"{prompt}\". Requested aspect ratio: {aspect_ratio}. Requested duration: {duration} seconds. Return the generated public video link when ready."
    )
}

pub(crate) fn create_video(request_body: &[u8], config: &Config) -> io::Result<serde_json::Value> {
    let body: serde_json::Value = if request_body.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(request_body).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid JSON: {e}"))
        })?
    };

    let prompt = body
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing 'prompt' field"))?
        .trim()
        .to_string();
    if prompt.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "prompt must not be empty",
        ));
    }

    let requested_model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("muse-video");
    if !SUPPORTED_MODELS.contains(&requested_model) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unsupported model '{requested_model}'; supported: {}",
                SUPPORTED_MODELS.join(", ")
            ),
        ));
    }
    let upstream_model = normalize_video_model(requested_model);

    let aspect_ratio = body
        .get("aspect_ratio")
        .and_then(|v| v.as_str())
        .unwrap_or("16:9");
    if !SUPPORTED_ASPECT_RATIOS.contains(&aspect_ratio) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unsupported aspect_ratio '{aspect_ratio}'; supported: {}",
                SUPPORTED_ASPECT_RATIOS.join(", ")
            ),
        ));
    }

    let duration: u64 = body
        .get("duration")
        .and_then(|v| {
            if let Some(n) = v.as_u64() {
                Some(n)
            } else if let Some(s) = v.as_str() {
                s.trim_end_matches('s').parse::<u64>().ok()
            } else {
                None
            }
        })
        .unwrap_or(5);

    let full_prompt = build_video_prompt(upstream_model, &prompt, aspect_ratio, duration);

    let request_json = serde_json::to_vec(&serde_json::json!({
        "model": "muse",
        "messages": [
            {"role": "user", "content": full_prompt}
        ]
    }))
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let completion = request_museai_chat_completion(&request_json, config)?;

    let completion_json: serde_json::Value = serde_json::from_str(&completion)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let content = completion_json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    let video_url = extract_url_from_text(content);
    let status = if video_url.is_empty() {
        "pending".to_string()
    } else {
        "completed".to_string()
    };

    Ok(serde_json::json!({
        "id": format!("video-{}", Uuid::new_v4()),
        "object": "video.generation",
        "created": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        "model": upstream_model,
        "requested_model": requested_model,
        "prompt": prompt,
        "requested_aspect_ratio": aspect_ratio,
        "requested_duration": duration,
        "status": status,
        "video_url": video_url
    }))
}

fn extract_url_from_text(text: &str) -> String {
    let urls: Vec<&str> = text
        .split_whitespace()
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
        .collect();
    // Prioritize public Google Drive URLs if output by upstream.
    for url in &urls {
        let cleaned = trim_trailing_punct(url);
        if is_google_drive_file_path(cleaned) {
            return cleaned.to_string();
        }
    }
    // Fall back to direct video file URLs (.mp4, .mov, .webm).
    for url in &urls {
        let cleaned = trim_trailing_punct(url);
        if is_supported_video_url(cleaned) {
            return cleaned.to_string();
        }
    }
    String::new()
}

fn is_supported_video_url(url: &str) -> bool {
    url.starts_with("https://")
        && ((is_google_drive_host(url) && is_google_drive_file_path(url))
            || has_supported_video_extension(url))
}

fn has_supported_video_extension(url: &str) -> bool {
    let path = url
        .strip_prefix("https://")
        .and_then(|rest| rest.split_once('/').map(|(_, path)| path))
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    path.ends_with(".mp4") || path.ends_with(".mov") || path.ends_with(".webm")
}

fn is_google_drive_host(url: &str) -> bool {
    let authority = url_authority(url);
    matches!(authority, "drive.google.com" | "docs.google.com")
}

fn is_google_drive_file_path(url: &str) -> bool {
    url.starts_with("https://")
        && is_google_drive_host(url)
        && url
            .strip_prefix("https://")
            .and_then(|rest| rest.split_once('/').map(|(_, path)| path))
            .is_some_and(|path| path.starts_with("file/") || path.starts_with("uc?"))
}

fn url_authority(url: &str) -> &str {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .and_then(|host| host.split('?').next())
        .and_then(|host| host.split('#').next())
        .unwrap_or("")
}

fn trim_trailing_punct(s: &str) -> &str {
    s.trim_matches([
        '"', '\'', ',', ';', ')', '}', '(', '{', ']', '[', '\\', '\n', '\r', ' ',
    ])
}

fn normalize_video_model(model: &str) -> &str {
    match model {
        "muse-video" | "muse" => "muse-video",
        "gen-3" | "gen-2" | "kling" | "gen-4" | "gen-4.5" | "gen-4-turbo" | "aleph-2.0"
        | "ruby" => "muse-video",
        _ => model,
    }
}

pub(crate) fn delete_muse_thread(
    base_url: &str,
    thread_id: &str,
    config: &Config,
) -> io::Result<()> {
    if thread_id.is_empty() {
        return Ok(());
    }

    let delete_thread_url = format!("{base_url}/api/thread/{thread_id}");
    let mut req = ureq::delete(&delete_thread_url)
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
        req = req.header("Cookie", &config.museai_cookie);
    }

    match req.call() {
        Ok(_) => {
            debug_log(&format!("Successfully cleaned up thread: {thread_id}"));
            Ok(())
        }
        Err(e) => {
            debug_log(&format!("Note: Failed to clean up thread {thread_id}: {e}"));
            if let ureq::Error::StatusCode(404) = e {
                Ok(())
            } else {
                Ok(())
            }
        }
    }
}

pub(crate) fn write_chat_completion(client: &mut TcpStream, response: &str) -> io::Result<()> {
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.len(),
        response
    )?;
    client.flush()
}

pub(crate) fn cleanup_tracked_threads_once() -> usize {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut to_delete = Vec::new();
    if let Ok(mut threads) = tracked_threads().lock() {
        let mut i = 0;
        while i < threads.len() {
            if now >= threads[i].created_at + threads[i].config.museai_thread_retention_secs {
                to_delete.push(threads.remove(i));
            } else {
                i += 1;
            }
        }
    }

    let deleted_count = to_delete.len();
    for thread in to_delete {
        let _ = delete_muse_thread(&thread.base_url, &thread.id, &thread.config);
    }
    deleted_count
}

pub(crate) fn start_cleanup_worker() {
    thread::spawn(|| loop {
        thread::sleep(Duration::from_secs(300));
        cleanup_tracked_threads_once();
    });
}

pub(crate) fn handle_museai_thread_cleanup(
    client: &mut TcpStream,
    thread_id: &str,
    config: &Config,
) -> io::Result<()> {
    let base_url = if config.museai_base_url.is_empty() {
        "https://muse.ai"
    } else {
        config.museai_base_url.as_str()
    };

    match delete_muse_thread(base_url, thread_id, config) {
        Ok(()) => {
            let body = serde_json::json!({
                "object": "thread.cleanup",
                "thread_id": thread_id,
                "status": "deleted"
            });
            let response_str = serde_json::to_string(&body)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            write!(
                client,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_str.len(),
                response_str
            )?;
            client.flush()
        }
        Err(e) => crate::http::write_error(
            client,
            502,
            "Bad Gateway",
            &format!("Failed to clean up thread: {}", e),
        ),
    }
}

pub(crate) fn handle_create_video(
    client: &mut TcpStream,
    request_body: &[u8],
    config: &Config,
) -> io::Result<()> {
    match create_video(request_body, config) {
        Ok(result) => {
            let response_str = serde_json::to_string(&result)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            write!(
                client,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_str.len(),
                response_str
            )?;
            client.flush()
        }
        Err(e) => {
            if e.get_ref()
                .and_then(|e| e.downcast_ref::<MuseApprovalRequired>())
                .is_some()
            {
                return crate::http::write_error(
                    client,
                    409,
                    "Conflict",
                    &format!(
                        "Failed to create video: a scoped Muse permission approval is pending; grant it in the browser and retry. Details: {}",
                        e
                    ),
                );
            }
            let (status_code, error_type) = match e.kind() {
                io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => (400, "Bad Request"),
                io::ErrorKind::PermissionDenied => (401, "Unauthorized"),
                io::ErrorKind::NotFound => (404, "Not Found"),
                io::ErrorKind::TimedOut => (504, "Gateway Timeout"),
                _ => (502, "Bad Gateway"),
            };
            crate::http::write_error(
                client,
                status_code,
                error_type,
                &format!("Failed to create video: {}", e),
            )
        }
    }
}

pub(crate) fn handle_museai_v1(
    client: &mut TcpStream,
    request_body: &[u8],
    config: &Config,
) -> io::Result<()> {
    let (url, method, body_json) =
        crate::museai_business::build_museai_request(request_body, config)?;

    let result = send_museai_request(&url, &method, &body_json, config);

    match result {
        Ok(upstream_resp) => {
            let status_text = match upstream_resp.status {
                200 => "OK",
                201 => "Created",
                204 => "No Content",
                400 => "Bad Request",
                401 => "Unauthorized",
                403 => "Forbidden",
                404 => "Not Found",
                500 => "Internal Server Error",
                _ => "OK",
            };

            let out = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                upstream_resp.status,
                status_text,
                upstream_resp.content_type,
                upstream_resp.body.len(),
                upstream_resp.body
            );
            client.write_all(out.as_bytes())?;
            client.flush()
        }
        Err(err) => crate::http::write_error(
            client,
            502,
            "Bad Gateway",
            &format!("Failed to proxy to Muse.ai: {}", err),
        ),
    }
}

#[cfg(test)]
mod museai_tests {
    use super::*;

    fn acknowledged_chat_stream() -> MuseChatStream {
        let mut stream = MuseChatStream::default();
        stream
            .record(&serde_json::json!({
                "session_id": "00000000-0000-4000-8000-000000000001", "is_thread": true
            }))
            .unwrap();
        stream
    }

    #[test]
    fn muse_chat_records_handle_split_utf8_and_coalesced_lines() {
        let records = concat!(
            "{\"session_id\":\"00000000-0000-4000-8000-000000000001\",\"is_thread\":true}\n",
            "{\"event\":\"message.user\",\"payload\":{\"content\":\"do not echo\"}}\n",
            "{\"event\":\"delta.text_append\",\"payload\":{\"text\":\"Café\"}}\n",
            "{\"event\":\"delta.message_done\",\"payload\":{}}\n"
        );
        for split in 0..=records.len() {
            let mut stream = MuseChatStream::default();
            stream.push(&records.as_bytes()[..split], false).unwrap();
            stream.push(&records.as_bytes()[split..], true).unwrap();
            assert_eq!(stream.finish().unwrap().1, "Café");
        }
    }

    #[test]
    fn muse_chat_requires_semantic_thread_acknowledgement() {
        for ack in [
            serde_json::json!({"session_id": "not-a-thread", "is_thread": true}),
            serde_json::json!({"session_id": "00000000-0000-4000-8000-000000000001", "is_thread": false}),
            serde_json::json!({"ok": false, "error": "private-token"}),
        ] {
            let error = MuseChatStream::default().record(&ack).unwrap_err();
            assert!(!error.to_string().contains("private-token"));
        }
        let mut stream = acknowledged_chat_stream();
        assert!(stream
            .record(&serde_json::json!({"type": "response", "payload": {
                "subscribed": true, "session_id": "another-session"
            }}))
            .is_err());
    }

    #[test]
    fn muse_chat_rejects_incomplete_failed_and_oversized_records() {
        let mut partial = acknowledged_chat_stream();
        partial
            .record(
                &serde_json::json!({"event": "delta.text_append", "payload": {"text": "partial"}}),
            )
            .unwrap();
        assert!(partial.finish().is_err());
        let mut stream = acknowledged_chat_stream();
        assert!(stream.push(b"invalid\n", false).is_err());
        assert!(MuseChatStream::default()
            .push(&vec![b'x'; 1024 * 1024 + 1], false)
            .is_err());
        let mut stream = acknowledged_chat_stream();
        let error = stream
            .record(&serde_json::json!({"event": "task.status", "payload": {
                "status": "failed", "error": "https://example.invalid/?token=private"
            }}))
            .unwrap_err();
        assert!(!error.to_string().contains("token"));
    }

    #[test]
    fn muse_chat_ignores_other_sessions_and_requires_scoped_approval() {
        let mut stream = acknowledged_chat_stream();
        stream
            .record(
                &serde_json::json!({"event": "message.assistant", "payload": {
                    "session_id": "another-session", "content": "unrelated text"
                }}),
            )
            .unwrap();
        stream
            .record(
                &serde_json::json!({"event": "approvals.snapshot", "payload": {
                    "pending_approvals": [{"scope": {"thread_id": "another-session"}}]
                }}),
            )
            .unwrap();
        assert!(stream.text.is_empty());
        // Scoped pending approvals must return PermissionDenied rather than hanging.
        let result = stream.record(&serde_json::json!({"event": "approvals.snapshot", "payload": {
            "pending_approvals": [{"scope": {"thread_id": "00000000-0000-4000-8000-000000000001"},
                "payload": {"private": "secret"}}]
        }}));
        assert!(
            result.is_err(),
            "approvals.snapshot with scoped pending approval must error the stream"
        );
        let err = result.unwrap_err();
        assert!(err
            .get_ref()
            .and_then(|e| e.downcast_ref::<crate::museai::MuseApprovalRequired>())
            .is_some());
        assert!(
            stream.text.is_empty(),
            "pending approval should not add text to stream"
        );
    }

    #[test]
    fn test_extract_new_thread_id() {
        let valid_rsc = r#"something something /thread/1234abcd-5678-efgh-ijkl-9876mnopqrst"\n"#;
        assert_eq!(
            extract_new_thread_id(valid_rsc),
            "1234abcd-5678-efgh-ijkl-9876mnopqrst"
        );

        let valid_rsc_2 = r#"/thread/some-id-1234"#;
        assert_eq!(extract_new_thread_id(valid_rsc_2), "some-id-1234");

        let empty_rsc = r#"/thread/ "#;
        assert_eq!(extract_new_thread_id(empty_rsc), "");

        let no_thread = r#"something else"#;
        assert_eq!(extract_new_thread_id(no_thread), "");
    }

    #[test]
    fn test_extract_url_from_text() {
        let valid_text = "Here is your video: https://cdn.muse.ai/video/xyz123.mp4";
        assert_eq!(
            extract_url_from_text(valid_text),
            "https://cdn.muse.ai/video/xyz123.mp4"
        );

        let mov_text = "Watch this https://cdn.muse.ai/video/xyz123.mov, and enjoy!";
        assert_eq!(
            extract_url_from_text(mov_text),
            "https://cdn.muse.ai/video/xyz123.mov"
        );

        let no_video_ext = "Here is a link https://muse.ai/some-link";
        assert_eq!(extract_url_from_text(no_video_ext), "");

        let misleading_query = "Not a video https://example.com/help?next=clip.mp4";
        assert_eq!(extract_url_from_text(misleading_query), "");

        let http_video = "Do not accept http://cdn.muse.ai/video/xyz123.mp4";
        assert_eq!(extract_url_from_text(http_video), "");

        let no_url = "Just some text without links";
        assert_eq!(extract_url_from_text(no_url), "");

        let gdrive_url =
            "Video ready at https://drive.google.com/file/d/1a2b3c4d5e/view?usp=sharing enjoy!";
        assert_eq!(
            extract_url_from_text(gdrive_url),
            "https://drive.google.com/file/d/1a2b3c4d5e/view?usp=sharing"
        );

        let gdrive_multi =
            "Check https://example.com/site or Google Drive: https://drive.google.com/uc?id=xyz789";
        assert_eq!(
            extract_url_from_text(gdrive_multi),
            "https://drive.google.com/uc?id=xyz789"
        );

        let unsafe_drive = "Do not accept http://drive.google.com/uc?id=xyz789";
        assert_eq!(extract_url_from_text(unsafe_drive), "");

        let lookalike_drive = "Do not accept https://drive.google.com.evil.example/file/d/123";
        assert_eq!(extract_url_from_text(lookalike_drive), "");

        // Prefer public Google Drive URL over a direct Muse CDN video URL if both appear.
        let drive_and_direct = "Drive https://drive.google.com/file/d/drive123/view and direct https://cdn.muse.ai/video/direct.mp4";
        assert_eq!(
            extract_url_from_text(drive_and_direct),
            "https://drive.google.com/file/d/drive123/view"
        );
    }

    #[test]
    fn test_muse_chat_stream_allows_empty_text_on_completion() {
        let mut stream = acknowledged_chat_stream();
        stream
            .record(&serde_json::json!({
                "event": "task.complete",
                "payload": {}
            }))
            .unwrap();
        let (session_id, text) = stream
            .finish()
            .expect("finish must succeed on completion even if text is empty");
        assert_eq!(session_id, "00000000-0000-4000-8000-000000000001");
        assert_eq!(text, "");
    }

    #[test]
    fn test_muse_ws_operation_deadline_covers_post_completion_wait() {
        assert!(
            crate::museai_transport::muse_ws_operation_deadline_secs()
                >= MUSE_POST_COMPLETION_WAIT_SECS
        );
    }

    #[test]
    fn test_post_completion_parser_handles_split_presentation_records() {
        let session_id = "00000000-0000-4000-8000-000000000001";
        let first = br#"{"event":"delta.presentation","payload":{"data":{"url":"https://drive.google.com/file/d/video123/view"}}}"#;
        let second = b"\n";
        let chunks: &[&[u8]] = &[first, second];
        let mut stream = MuseChatStream {
            session_id: Some(session_id.to_string()),
            ..Default::default()
        };
        for chunk in chunks {
            stream.push(chunk, false).unwrap();
        }
        assert_eq!(
            extract_video_url_from_stream(&stream),
            Some("https://drive.google.com/file/d/video123/view".to_string())
        );
    }

    #[test]
    fn test_post_completion_parser_propagates_scoped_approval() {
        let mut stream = MuseChatStream {
            session_id: Some("00000000-0000-4000-8000-000000000001".to_string()),
            ..Default::default()
        };
        let record = br#"{"event":"approvals.snapshot","payload":{"pending_approvals":[{"scope":{"session_scope_id":"00000000-0000-4000-8000-000000000001"}}]}}
"#;
        let error = stream.push(record, false).unwrap_err();
        assert!(error
            .get_ref()
            .and_then(|e| e.downcast_ref::<crate::museai::MuseApprovalRequired>())
            .is_some());
    }

    #[test]
    fn test_delta_message_done_populates_transcript_when_deltas_empty() {
        let mut stream = acknowledged_chat_stream();
        stream
            .record(&serde_json::json!({
                "event": "delta.message_done",
                "payload": {
                    "transcript": {
                        "messages": [
                            {
                                "role": "user",
                                "content": [{"text": "create video"}]
                            },
                            {
                                "role": "assistant",
                                "content": [{"text": "Here is your video: https://cdn.muse.ai/video/abc.mp4"}]
                            }
                        ]
                    }
                }
            }))
            .unwrap();
        assert!(stream.completed);
        let (_, text) = stream.finish().unwrap();
        assert_eq!(
            text,
            "Here is your video: https://cdn.muse.ai/video/abc.mp4"
        );
        assert_eq!(
            extract_url_from_text(&text),
            "https://cdn.muse.ai/video/abc.mp4"
        );
    }

    #[test]
    fn test_task_status_completed_does_not_prematurely_finish_stream() {
        let mut stream = acknowledged_chat_stream();
        stream
            .record(&serde_json::json!({
                "event": "task.status",
                "payload": {"status": "completed"}
            }))
            .unwrap();
        assert!(
            !stream.completed,
            "task.status snapshot must not mark stream as completed"
        );

        stream
            .record(&serde_json::json!({
                "event": "delta.text_append",
                "payload": {"text": "Video generated: https://cdn.muse.ai/video/xyz.mp4"}
            }))
            .unwrap();
        assert_eq!(
            stream.text,
            "Video generated: https://cdn.muse.ai/video/xyz.mp4"
        );

        stream
            .record(&serde_json::json!({
                "event": "delta.message_done",
                "payload": {}
            }))
            .unwrap();
        assert!(stream.completed);
        let (_, text) = stream.finish().unwrap();
        assert_eq!(text, "Video generated: https://cdn.muse.ai/video/xyz.mp4");
    }

    #[test]
    fn test_video_prompt_avoids_fake_model_injection() {
        let prompt_legacy = build_video_prompt("gen-3", "a dancing cat", "16:9", 5);
        assert!(!prompt_legacy.contains("gen-3 model"));
        assert!(!prompt_legacy.contains("kling model"));
        assert!(prompt_legacy.contains("a dancing cat"));

        let prompt_canon = build_video_prompt("muse-video", "a dancing cat", "16:9", 5);
        assert!(!prompt_canon.contains("muse-video model"));
        assert!(prompt_canon.contains("a dancing cat"));
    }

    #[test]
    fn test_video_prompt_requests_public_google_drive_delivery() {
        let prompt = build_video_prompt("muse-video", "a fox skating", "16:9", 10);
        assert!(prompt.contains("Muse Video"));
        assert!(prompt.contains("public video link"));
        assert!(prompt.contains("a fox skating"));
    }

    #[test]
    fn test_create_video_validation() {
        let config = crate::config::Config {
            target_host: "127.0.0.1".into(),
            target_port: 8080,
            fallbacks: vec![],
            io_timeout: None,
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 1000,
            default_model: "gpt-5.6-sol".into(),
            museai_base_url: "https://muse.ai".into(),
            museai_cookie: "".into(),
            museai_ws_url: "".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 86400,
        };

        // 1. Empty prompt
        let empty_prompt = serde_json::json!({
            "prompt": "   ",
            "model": "gen-3"
        });
        let result = create_video(&serde_json::to_vec(&empty_prompt).unwrap(), &config);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);

        // 2. Invalid model
        let invalid_model = serde_json::json!({
            "prompt": "A cat",
            "model": "invalid-model"
        });
        let result = create_video(&serde_json::to_vec(&invalid_model).unwrap(), &config);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);

        // 3. Invalid aspect ratio
        let invalid_ar = serde_json::json!({
            "prompt": "A cat",
            "aspect_ratio": "4:5"
        });
        let result = create_video(&serde_json::to_vec(&invalid_ar).unwrap(), &config);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_thread_registration_and_retention() {
        let config = crate::config::Config {
            target_host: "127.0.0.1".into(),
            target_port: 8080,
            fallbacks: vec![],
            io_timeout: None,
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 1000,
            default_model: "gpt-5.6-sol".into(),
            museai_base_url: "https://muse.ai".into(),
            museai_cookie: "".into(),
            museai_ws_url: "".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 86400,
        };

        let thread_id = "test-thread-id-12345".to_string();
        register_thread(
            thread_id.clone(),
            "https://muse.ai".to_string(),
            config.clone(),
        );

        let threads = tracked_threads().lock().unwrap();
        let found = threads.iter().find(|t| t.id == thread_id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().config.museai_thread_retention_secs, 86400);
    }

    #[test]
    fn test_thread_cleanup_active_vs_expired() {
        let config_active = crate::config::Config {
            target_host: "127.0.0.1".into(),
            target_port: 8080,
            fallbacks: vec![],
            io_timeout: None,
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 1000,
            default_model: "gpt-5.6-sol".into(),
            museai_base_url: "https://muse.ai".into(),
            museai_cookie: "".into(),
            museai_ws_url: "".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 86400, // 24 hours
        };

        let mut config_expired = config_active.clone();
        config_expired.museai_thread_retention_secs = 0; // expires immediately

        let active_id = "thread-active-xyz".to_string();
        let expired_id = "thread-expired-abc".to_string();

        // Release the mutex lock from previous operations just to be safe
        register_thread(
            active_id.clone(),
            "https://muse.ai".to_string(),
            config_active,
        );
        register_thread(
            expired_id.clone(),
            "https://muse.ai".to_string(),
            config_expired,
        );

        // Run the cleanup once synchronously to verify it triggers correctly
        let deleted = cleanup_tracked_threads_once();

        // Check that at least the expired one got cleared
        assert!(deleted >= 1);

        let threads = tracked_threads().lock().unwrap();
        // The expired one should be gone
        assert!(!threads.iter().any(|t| t.id == expired_id));
        // The active one should still be tracked
        assert!(threads.iter().any(|t| t.id == active_id));
    }

    #[test]
    fn test_live_muse_thread_spawn_and_cleanup() {
        let _ = dotenvy::dotenv();
        let cookie = std::env::var("GW_MUSEAI_COOKIE").unwrap_or_default();
        if cookie.is_empty() {
            println!("Skipping live test: GW_MUSEAI_COOKIE not set");
            return;
        }

        let base_url =
            std::env::var("GW_MUSEAI_BASE_URL").unwrap_or_else(|_| "https://muse.ai".into());
        let config = Config {
            target_host: "127.0.0.1".into(),
            target_port: 8080,
            fallbacks: vec![],
            io_timeout: None,
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 1000,
            default_model: "gpt-5.6-sol".into(),
            museai_base_url: base_url.clone(),
            museai_cookie: cookie,
            museai_ws_url: "".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 0,
        };

        match spawn_muse_thread(&base_url, &config) {
            Ok(channel) => {
                assert!(channel.starts_with("thread:"));
                let thread_id = channel.strip_prefix("thread:").unwrap();
                assert!(!thread_id.is_empty());

                let res = delete_muse_thread(&base_url, thread_id, &config);
                assert!(res.is_ok(), "Failed to delete live thread: {:?}", res);
            }
            Err(e) => {
                println!(
                    "Live test note: upstream rejected thread creation or credentials expired: {e}"
                );
            }
        }
    }

    #[test]
    fn test_thread_cleanup_route_dispatch() {
        use std::net::TcpListener;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        let config = Config {
            target_host: "127.0.0.1".into(),
            target_port: 8080,
            fallbacks: vec![],
            io_timeout: None,
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 1000,
            default_model: "gpt-5.6-sol".into(),
            museai_base_url: "http://127.0.0.1:9".into(), // Will not hang, unreachable
            museai_cookie: "".into(),
            museai_ws_url: "".into(),
            museai_access_token: "".into(),
            museai_notary_token: "".into(),
            museai_vm_id: "".into(),
            museai_auto_cleanup_threads: true,
            museai_thread_retention_secs: 86400,
        };

        let handle = std::thread::spawn(move || {
            let (mut client, _) = listener.accept().unwrap();
            let req_in = crate::http::read_request(&mut client).unwrap();
            assert_eq!(req_in.method, "DELETE");
            assert_eq!(req_in.path, "/muse-ai/v1/threads/mock-thread-456");
            let clean_path = req_in
                .path
                .split('?')
                .next()
                .unwrap_or("")
                .trim_end_matches('/');
            let thread_id = clean_path.split('/').next_back().unwrap_or("");
            assert_eq!(thread_id, "mock-thread-456");
            handle_museai_thread_cleanup(&mut client, thread_id, &config).unwrap();
        });

        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            client,
            "DELETE /muse-ai/v1/threads/mock-thread-456 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        client.flush().unwrap();

        let head = crate::http::read_response_head(&mut client).unwrap();
        assert_eq!(head.status, 200);
        let mut body = head.buffered_body;
        if let Some(length_str) = crate::http::header_value(&head.headers, "content-length") {
            let length: usize = length_str.parse().unwrap();
            while body.len() < length {
                crate::http::read_more(&mut client, &mut body).unwrap();
            }
        }
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["object"], "thread.cleanup");
        assert_eq!(json["thread_id"], "mock-thread-456");
        assert_eq!(json["status"], "deleted");

        handle.join().unwrap();
    }

    #[test]
    fn parse_cookie_from_header_dump_extracts_cookie_value() {
        let dump =
            b":authority\nmuse.ai\n:method\nGET\ncookie\nsite.example; session=abc\n:method2\nx\n";
        assert_eq!(
            parse_cookie_from_header_dump(dump),
            Some("site.example; session=abc".to_string())
        );
    }

    #[test]
    fn parse_cookie_from_header_dump_without_cookie_is_none() {
        let dump = b":authority\nmuse.ai\n:method\nGET\nreferer\nhttps://muse.ai/thread/new\n";
        assert_eq!(parse_cookie_from_header_dump(dump), None);
    }

    #[test]
    fn parse_cookie_from_header_dump_empty_value_is_none() {
        let dump = b"cookie\n\npriority\nu=1, i\n";
        assert_eq!(parse_cookie_from_header_dump(dump), None);
    }

    #[test]
    fn parse_cookie_from_header_dump_case_insensitive_key() {
        let dump = b"Cookie\nsession=xyz\n";
        assert_eq!(
            parse_cookie_from_header_dump(dump),
            Some("session=xyz".to_string())
        );
    }

    pub(super) fn save_live_owned_session(id: &str) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/muse-live-owned-session.json");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)?;
            file.write_all(serde_json::json!({"session_id": id}).to_string().as_bytes())?;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = id;
            Err(io::Error::other(
                "secure live-session storage unavailable on this platform",
            ))
        }
    }

    fn live_capture_config() -> Config {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let har_bytes = std::fs::read(manifest.join("target/muse.ai.har"))
            .expect("live Muse HAR capture is required");
        let extracted = crate::har_config::extract_muse_config_from_har(&har_bytes)
            .unwrap_or_else(|_| panic!("live Muse HAR could not be parsed"));
        let cookie = std::fs::read(manifest.join("target/muse-header.txt"))
            .ok()
            .and_then(|bytes| parse_cookie_from_header_dump(&bytes))
            .or(extracted.cookie)
            .filter(|cookie| !cookie.is_empty())
            .expect("live Muse capture has no cookie");
        let har: serde_json::Value = serde_json::from_slice(&har_bytes)
            .unwrap_or_else(|_| panic!("live Muse HAR must be valid JSON"));
        let request = har["log"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| &entry["request"])
            .find(|request| {
                request["method"] == "POST" && request["url"] == "https://muse.ai/api/auth/check"
            })
            .expect("capture must evidence the Muse authentication endpoint");
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .max_redirects(0)
            .build()
            .into();
        let mut auth = agent
            .post("https://muse.ai/api/auth/check")
            .header("Cookie", &cookie);
        for header in request["headers"].as_array().unwrap() {
            let Some(name) = header["name"].as_str() else {
                continue;
            };
            if [
                "accept",
                "accept-language",
                "origin",
                "referer",
                "user-agent",
                "sec-fetch-site",
                "sec-fetch-mode",
                "sec-fetch-dest",
                "sec-ch-ua",
                "sec-ch-ua-mobile",
                "sec-ch-ua-platform",
            ]
            .iter()
            .any(|allowed| name.eq_ignore_ascii_case(allowed))
            {
                if let Some(value) = header["value"].as_str() {
                    auth = auth.header(name, value);
                }
            }
        }
        let auth = auth
            .send_empty()
            .unwrap_or_else(|error| match error {
                ureq::Error::StatusCode(status) => panic!("live Muse authentication HTTP {status}"),
                _ => panic!("live Muse authentication transport failed (details suppressed)"),
            })
            .into_body()
            .read_json::<serde_json::Value>()
            .unwrap_or_else(|_| panic!("invalid live Muse authentication JSON"));
        assert_eq!(
            auth["ok"].as_bool(),
            Some(true),
            "live Muse authentication rejected"
        );
        let token = auth["access_token"]
            .as_str()
            .filter(|token| !token.is_empty())
            .expect("live Muse authentication returned no token")
            .to_owned();
        // Verified from the original capture and live first-party session flow;
        // a later browser capture may omit this read-only request.
        let session = agent
            .get("https://muse.ai/api/session")
            .header("Cookie", &cookie)
            .call()
            .unwrap_or_else(|_| panic!("live Muse session request failed"))
            .into_body()
            .read_json::<serde_json::Value>()
            .unwrap_or_else(|_| panic!("invalid live Muse session JSON"));
        let mut config = Config {
            target_host: "127.0.0.1".into(),
            target_port: 8080,
            fallbacks: vec![],
            io_timeout: None,
            retry_base_delay_ms: 100,
            max_retry_delay_ms: 1000,
            default_model: "gpt-5.6-sol".into(),
            museai_base_url: "https://muse.ai".into(),
            museai_cookie: cookie,
            museai_ws_url: String::new(),
            museai_access_token: token,
            museai_notary_token: extracted.notary_token.unwrap_or_default(),
            museai_vm_id: String::new(),
            museai_auto_cleanup_threads: false,
            museai_thread_retention_secs: 0,
        };
        apply_muse_session_metadata(&mut config, &session);
        config
    }

    #[test]
    fn test_live_muse_auth_from_har_capture() {
        if std::env::var("MUSE_LIVE_TEST").ok().as_deref() != Some("1") {
            return;
        }
        let config = live_capture_config();
        assert!(!config.museai_access_token.is_empty());
        assert!(!config.museai_vm_id.is_empty());
    }

    #[test]
    fn test_live_muse_owned_session_events() {
        if std::env::var("MUSE_LIVE_RESUME_OWNED").ok().as_deref() != Some("1") {
            return;
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/muse-live-owned-session.json");
        let bytes = std::fs::read(path).expect("no task-owned live session saved");
        let saved: serde_json::Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| panic!("invalid saved task-owned session"));
        let id = saved["session_id"]
            .as_str()
            .filter(|id| Uuid::parse_str(id).is_ok())
            .expect("invalid task-owned session identifier");
        let config = live_capture_config();
        let url = build_museai_ws_url(&config, &Uuid::new_v4().to_string())
            .unwrap_or_else(|_| panic!("invalid live Muse socket configuration"));
        let mut socket = MuseWebSocket::connect(&url, "https://muse.ai", "")
            .unwrap_or_else(|_| panic!("live Muse WebSocket connection failed"));
        let mut noise = MuseNoiseSession::new_initiator(NOISE_PATTERN_XX, None)
            .unwrap_or_else(|_| panic!("live Muse Noise initialization failed"));
        noise
            .perform_client_handshake(&mut socket, &config.museai_notary_token)
            .unwrap_or_else(|_| panic!("live Muse Noise handshake failed"));
        let payload = serde_json::json!({"session_id": id, "after_stream_seq": 0,
            "after_chat_event_seq": 0, "capabilities": ["chat_cancel", "delta_stream"]});
        let headers = vec![
            Header {
                key: "Content-Type".into(),
                value: "application/json".into(),
            },
            Header {
                key: "x-app-id".into(),
                value: "hatch-web".into(),
            },
        ];
        socket
            .send_encrypted_service_request(
                &mut noise,
                SERVICE_DAEMON,
                1,
                "POST",
                "/chat/subscribe",
                &headers,
                payload.to_string().as_bytes(),
            )
            .unwrap_or_else(|_| panic!("live Muse owned-session subscribe failed"));
        let mut stream = MuseChatStream {
            session_id: Some(id.to_owned()),
            ..Default::default()
        };
        let deadline =
            std::time::Instant::now() + Duration::from_secs(MUSE_POST_COMPLETION_WAIT_SECS);
        while std::time::Instant::now() < deadline {
            let frame = match socket.read_encrypted_service_frame(&mut noise) {
                Ok(frame) => frame,
                Err(error) if stream.text.is_empty() => panic!(
                    "live Muse owned-session read failed: {}",
                    safe_live_error(&error)
                ),
                Err(_) => break,
            };
            assert_eq!(frame.stream_id, 1, "unexpected live Muse response stream");
            let (data, ended) = match frame.kind {
                ServiceFrameKind::Response {
                    status,
                    body,
                    end_body,
                    ..
                } => {
                    println!("owned-session subscription HTTP status={status}");
                    assert!(
                        (200..300).contains(&status),
                        "owned-session subscribe rejected"
                    );
                    (body, end_body)
                }
                ServiceFrameKind::BodyChunk { data, end_body } => (data, end_body),
                ServiceFrameKind::Reset { code, .. } => {
                    panic!("live Muse stream reset code={code}")
                }
            };
            stream.push(&data, ended).unwrap_or_else(|error| {
                panic!(
                    "live Muse owned-session result: {}",
                    safe_live_error(&error)
                )
            });
            println!(
                "owned-session state: completed={} text_bytes={} pending_bytes={} ended={}",
                stream.completed,
                stream.text.len(),
                stream.pending.len(),
                ended
            );
            if stream.completed || ended {
                break;
            }
        }
        assert!(
            stream.completed,
            "owned session did not reach a terminal event"
        );
        let (_, mut text) = stream.finish().unwrap_or_else(|error| {
            panic!(
                "live Muse owned-session result: {}",
                safe_live_error(&error)
            )
        });
        if extract_url_from_text(&text).is_empty() {
            let deadline =
                std::time::Instant::now() + Duration::from_secs(MUSE_POST_COMPLETION_WAIT_SECS);
            let mut presentation_stream = MuseChatStream {
                session_id: Some(id.to_owned()),
                ..Default::default()
            };
            while std::time::Instant::now() < deadline {
                let frame = match socket.read_encrypted_service_frame(&mut noise) {
                    Ok(frame) => frame,
                    Err(_) => break,
                };
                let (data, ended) = match frame.kind {
                    ServiceFrameKind::Response { body, end_body, .. } => (body, end_body),
                    ServiceFrameKind::BodyChunk { data, end_body } => (data, end_body),
                    ServiceFrameKind::Reset { .. } => break,
                };
                presentation_stream
                    .push(&data, ended)
                    .unwrap_or_else(|error| {
                        panic!(
                            "live Muse owned-session presentation result: {}",
                            safe_live_error(&error)
                        )
                    });
                if let Some(extracted) = extract_video_url_from_stream(&presentation_stream) {
                    println!("Owned-session presentation video artifact received");
                    text.push_str(&format!("\nVideo URL: {extracted}\n"));
                    break;
                }
            }
        }
        let url = extract_url_from_text(&text);
        assert!(
            url.starts_with("https://"),
            "owned session completed without an HTTPS video result"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/muse-video-result.json");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .unwrap_or_else(|_| panic!("cannot securely create live video result file"));
            let result = serde_json::json!({"object": "video.generation", "status": "completed", "video_url": url});
            file.write_all(result.to_string().as_bytes())
                .unwrap_or_else(|_| panic!("cannot write live video result file"));
            println!("Live video result saved to target/muse-video-result.json");
        }
        #[cfg(not(unix))]
        panic!("secure live-result storage requires an owner-only file ACL implementation");
    }

    pub(crate) fn log_owned_record_schema(record: &serde_json::Value) {
        fn schema(value: &serde_json::Value, depth: usize) {
            if depth > 6 {
                return;
            }
            match value {
                serde_json::Value::Object(fields) => {
                    for (key, value) in fields {
                        let key = match key.as_str() {
                            "type" | "event" | "payload" | "status" | "session_id"
                            | "subscribed" | "text" | "content" | "message" | "messages"
                            | "transcript" | "data" | "url" | "role" | "items" | "events"
                            | "pending_approvals" | "scope" | "thread_id" | "session_scope_id"
                            | "result" | "error" | "ok" | "state" | "snapshot" | "format"
                            | "mime_type" | "completed" | "is_final" => key.as_str(),
                            _ => "other",
                        };
                        let kind = match value {
                            serde_json::Value::Null => "null",
                            serde_json::Value::Bool(_) => "bool",
                            serde_json::Value::Number(_) => "number",
                            serde_json::Value::String(_) => "string",
                            serde_json::Value::Array(_) => "array",
                            serde_json::Value::Object(_) => "object",
                        };
                        println!("owned-session schema depth={depth} field={key} type={kind}");
                        schema(value, depth + 1);
                    }
                }
                serde_json::Value::Array(items) => {
                    println!("owned-session schema depth={depth} items={}", items.len());
                    for item in items.iter().take(8) {
                        schema(item, depth + 1);
                    }
                }
                _ => {}
            }
        }
        let category = match record["event"].as_str().unwrap_or("") {
            event @ ("delta.text_append"
            | "delta.message_done"
            | "task.status"
            | "task.complete"
            | "agent.status"
            | "message.assistant"
            | "message.user"
            | "delta.presentation"
            | "delta.message_start"
            | "delta.tool_start"
            | "delta.tool_done"
            | "approvals.snapshot"
            | "delta.marker"
            | "delta.placeholder"
            | "chat.snapshot"
            | "session.snapshot") => event,
            _ => "other",
        };
        println!("owned-session event category={category}");
        schema(record, 0);
    }

    fn safe_live_error(error: &io::Error) -> &'static str {
        if error.kind() == io::ErrorKind::PermissionDenied
            || error
                .get_ref()
                .and_then(|e| e.downcast_ref::<crate::museai::MuseApprovalRequired>())
                .is_some()
        {
            "service approval required or access denied"
        } else {
            "incomplete or invalid service response (private details suppressed)"
        }
    }

    #[test]
    fn test_live_create_video_from_har_capture() {
        if std::env::var("MUSE_LIVE_TEST").ok().as_deref() != Some("1") {
            return;
        }
        assert!(!std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/muse-live-owned-session.json").exists(),
            "a task-owned session is already saved; use the read-only owned-session test instead of generating again");
        let config = live_capture_config();
        let body = serde_json::json!({
            "prompt": "A single white square rotating slowly on a black background, 3D cartoon style",
            "model": "muse-video",
            "aspect_ratio": "16:9",
            "duration": 5
        });

        match create_video(body.to_string().as_bytes(), &config) {
            Ok(result) => {
                assert_eq!(result["object"], "video.generation");
                let status = result["status"].as_str().unwrap_or("");
                assert!(
                    status == "completed",
                    "live Muse did not return a completed video"
                );
                if status == "completed" {
                    let url = result["video_url"].as_str().unwrap_or("");
                    assert!(
                        url.starts_with("https://"),
                        "completed but video_url is not HTTPS"
                    );
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("target/muse-video-result.json");
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(path)
                        .unwrap_or_else(|_| {
                            panic!("cannot securely create live video result file")
                        });
                    file.write_all(result.to_string().as_bytes())
                        .unwrap_or_else(|_| panic!("cannot write live video result file"));
                    println!("Live video result saved to target/muse-video-result.json");
                }
                #[cfg(not(unix))]
                panic!("secure live-result storage requires an owner-only file ACL implementation");
            }
            Err(e) => {
                panic!(
                    "Live Muse request failed at API pipeline stage: {} (private details suppressed)",
                    e.kind()
                );
            }
        }
    }

    fn parse_cookie_from_header_dump(bytes: &[u8]) -> Option<String> {
        let text = String::from_utf8_lossy(bytes);
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while i + 1 < lines.len() {
            if lines[i].trim().eq_ignore_ascii_case("cookie") {
                let value = lines[i + 1].trim();
                return (!value.is_empty()).then(|| value.to_string());
            }
            i += 1;
        }
        None
    }
}
