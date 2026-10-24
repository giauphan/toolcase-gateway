use super::types::{HarExtractError, MuseHarConfig, RestEntry, WsEntry};

pub const MAX_HAR_BYTES: usize = 16 * 1024 * 1024;

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
            if rest.cookie.is_some() {
                out.cookie = rest.cookie;
            }
            if rest.ws_url.is_some() {
                out.ws_url = rest.ws_url;
            }
            if rest.access_token.is_some() {
                out.access_token = rest.access_token;
            }
            if rest.notary_token.is_some() {
                out.notary_token = rest.notary_token;
            }
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

pub(crate) fn parse_ws_muse_entry(url: &str) -> Option<WsEntry> {
    let (scheme, rest) = split_scheme(url)?;
    if scheme != "wss" {
        return None;
    }
    let (authority_path, query) = match rest.find('?') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, ""),
    };
    if !is_allowed_muse_ws_authority(authority_path) {
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

pub(crate) fn is_allowed_muse_ws_authority(authority_path: &str) -> bool {
    let host = authority_path
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");
    host == "metaaivm.com" || host.ends_with(".metaaivm.com")
}

pub(crate) fn is_allowed_muse_ws_url(url: &str) -> bool {
    let Some((scheme, rest)) = split_scheme(url) else {
        return false;
    };
    if scheme != "wss" {
        return false;
    }
    let authority_path = rest.split('?').next().unwrap_or(rest);
    is_allowed_muse_ws_authority(authority_path)
}

pub(crate) fn parse_rest_muse_entry(url: &str, entry: &serde_json::Value) -> Option<RestEntry> {
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
    let request = entry.get("request");
    let cookie = request
        .and_then(|r| r.get("headers"))
        .and_then(|h| h.as_array())
        .and_then(|headers| {
            headers
                .iter()
                .find(|h| {
                    h.get("name")
                        .and_then(|n| n.as_str())
                        .is_some_and(|name| name.eq_ignore_ascii_case("cookie"))
                })
                .and_then(|h| h.get("value").and_then(|v| v.as_str()))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| request.and_then(extract_har_cookie_array));
    let response_body = entry
        .get("response")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
    let response_string = |key: &str| {
        response_body
            .as_ref()
            .and_then(|body| body.get(key))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let selected_vm = response_body
        .as_ref()
        .and_then(|body| body.get("vms"))
        .and_then(|value| value.as_array())
        .and_then(|vms| {
            vms.iter()
                .find(|vm| vm.get("is_preferred").and_then(|value| value.as_bool()) == Some(true))
                .or_else(|| vms.iter().find(|vm| vm.get("endpoint_url").is_some()))
                .or_else(|| vms.first())
        });
    let selected_vm_string = |key: &str| {
        selected_vm
            .and_then(|vm| vm.get(key))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let ws_url = response_string("endpoint_url")
        .or_else(|| selected_vm_string("endpoint_url"))
        .filter(|value| is_allowed_muse_ws_url(value));
    let access_token = response_string("access_token")
        .or_else(|| response_string("auth_token"))
        .or_else(|| response_string("token"));
    let notary_token = response_string("notary_token");
    let vm_id = response_string("vm_id")
        .or_else(|| selected_vm_string("vm_id"))
        .or_else(|| selected_vm_string("id"));
    let base_url = format!("{scheme}://{host}");
    Some(RestEntry {
        base_url,
        cookie,
        ws_url,
        access_token,
        notary_token,
        vm_id,
    })
}

pub(crate) fn extract_har_cookie_array(request: &serde_json::Value) -> Option<String> {
    let cookies = request.get("cookies")?.as_array()?;
    let pairs: Vec<String> = cookies
        .iter()
        .filter_map(|cookie| {
            let name = cookie.get("name")?.as_str()?.trim();
            let value = cookie.get("value")?.as_str()?.trim();
            if name.is_empty() || value.is_empty() {
                None
            } else {
                Some(format!("{name}={value}"))
            }
        })
        .collect();
    (!pairs.is_empty()).then(|| pairs.join("; "))
}

pub(crate) fn split_scheme(url: &str) -> Option<(&str, &str)> {
    url.split_once("://")
}

pub(crate) fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
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

pub(crate) fn percent_decode(s: &str) -> String {
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

pub(crate) fn sanitize_value(s: &str) -> String {
    // Env-file injection prevention: remove ASCII control characters.
    // (Do not allow newline, carriage return, or NUL.)
    s.chars().filter(|c| !c.is_ascii_control()).collect()
}
