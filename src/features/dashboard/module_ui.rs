//! Dashboard handlers for dynamic module UI positioning and automatic schema-driven configuration.

use super::overview::mask_key;
use crate::config::ConfigStore;
use crate::gateway::modules::{
    default_module_config_schema, normalize_module_config_schema, ModuleStatus,
};
use crate::http::write_error;
use std::io::{self, Write};
use std::net::TcpStream;

pub fn handle_module_config_page(client: &mut TcpStream) -> io::Result<()> {
    handle_module_config_page_for_id(client, None)
}

pub fn handle_module_config_page_for_id(
    client: &mut TcpStream,
    _module_id: Option<&str>,
) -> io::Result<()> {
    let html = include_str!("../../../assets/module-config.html");
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-cache, no-store, must-revalidate\r\nAccess-Control-Allow-Origin: *\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    )?;
    client.flush()
}

pub fn handle_set_module_position(
    client: &mut TcpStream,
    body: &[u8],
    store: &ConfigStore,
) -> io::Result<()> {
    #[derive(serde::Deserialize)]
    struct PositionPayload {
        id: String,
        #[serde(default)]
        menu_position: Option<String>,
        #[serde(default)]
        menu_order: Option<i32>,
        #[serde(default)]
        direction: Option<String>,
    }

    let payload: PositionPayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(e) => return write_error(client, 400, "Bad Request", &format!("Invalid JSON: {e}")),
    };

    let reg = store.module_registry();
    let mut guard = match reg.write() {
        Ok(g) => g,
        Err(e) => return write_error(client, 500, "Internal Error", &e.to_string()),
    };

    let updated = if let Some(ref dir) = payload.direction {
        match guard.move_module_order(&payload.id, dir) {
            Ok(desc) => desc,
            Err(e) => return write_error(client, 400, "Bad Request", &e.to_string()),
        }
    } else {
        match guard.set_module_position(&payload.id, payload.menu_position, payload.menu_order) {
            Ok(desc) => desc,
            Err(e) => return write_error(client, 400, "Bad Request", &e.to_string()),
        }
    };

    let all_modules = guard.list_modules();
    drop(guard);

    if let Err(e) = store.save_to_persistent_file() {
        return write_error(
            client,
            500,
            "Internal Error",
            &format!("Module position updated but persistence failed: {e}"),
        );
    }

    let resp = serde_json::json!({
        "ok": true,
        "id": updated.id,
        "menu_position": updated.menu_position,
        "menu_order": updated.menu_order,
        "module": updated,
        "modules": all_modules,
    });
    write_json_ok(client, &resp)
}

fn extract_query_param(path: &str, key: &str) -> Option<String> {
    let (_, query) = path.split_once('?')?;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key && !v.trim().is_empty() {
            return Some(v.trim().to_string());
        }
    }
    None
}

fn builtin_masked_config(
    id: &str,
    store: &ConfigStore,
    mut base: serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let cfg = store.snapshot();
    match id {
        "muse" => {
            base.insert(
                "base_url".to_string(),
                serde_json::Value::String(cfg.museai_base_url),
            );
            base.insert(
                "masked_api_key".to_string(),
                serde_json::Value::String(mask_key(&cfg.museai_access_token)),
            );
            base.insert(
                "masked_cookie".to_string(),
                serde_json::Value::String(mask_key(&cfg.museai_cookie)),
            );
        }
        "jev" => {
            base.insert(
                "base_url".to_string(),
                serde_json::Value::String(cfg.jev_base_url),
            );
            base.insert(
                "default_model".to_string(),
                serde_json::Value::String(cfg.jev_openrouter_model),
            );
        }
        "omniroute" => {
            base.insert(
                "base_url".to_string(),
                serde_json::Value::String(format!(
                    "http://{}:{}",
                    cfg.target_host, cfg.target_port
                )),
            );
            base.insert(
                "default_model".to_string(),
                serde_json::Value::String(cfg.default_model),
            );
        }
        _ => {}
    }
    base
}

pub fn handle_get_module_config(
    client: &mut TcpStream,
    request_path: &str,
    store: &ConfigStore,
) -> io::Result<()> {
    let Some(module_id) = extract_query_param(request_path, "id") else {
        return write_error(client, 400, "Bad Request", "Missing 'id' query parameter");
    };

    let reg = store.module_registry();
    let guard = match reg.read() {
        Ok(g) => g,
        Err(e) => return write_error(client, 500, "Internal Error", &e.to_string()),
    };

    let resolved_id = guard
        .resolve_module_id(&module_id)
        .unwrap_or_else(|| module_id.clone());

    let Some(desc) = guard.get_module(&resolved_id).cloned() else {
        return write_error(
            client,
            404,
            "Not Found",
            &format!("Module '{module_id}' not found"),
        );
    };

    let masked = guard.get_masked_module_config(&resolved_id);
    drop(guard);

    let merged_cfg = builtin_masked_config(&resolved_id, store, masked);
    let schema = desc
        .config_schema
        .clone()
        .map(|s| normalize_module_config_schema(s, &desc.id, &desc.namespace))
        .unwrap_or_else(|| default_module_config_schema(&desc.id, &desc.name, &desc.namespace));

    let resp = serde_json::json!({
        "status": "ok",
        "id": desc.id,
        "name": desc.name,
        "kind": desc.kind,
        "namespace": desc.namespace,
        "module_status": desc.status,
        "menu_position": desc.menu_position,
        "menu_order": desc.menu_order,
        "icon": desc.icon,
        "description": desc.description,
        "config_url": desc.config_url,
        "settings_tab": desc.settings_tab,
        "config_schema": schema,
        "config": merged_cfg,
    });
    write_json_ok(client, &resp)
}

fn get_trimmed_config_str<'a>(
    config: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<&'a str> {
    config
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn apply_omniroute_builtin_config(
    conf: &mut crate::config::Config,
    config: &serde_json::Map<String, serde_json::Value>,
) {
    if let Some(m) = get_trimmed_config_str(config, "default_model") {
        conf.default_model = m.to_string();
    }
}

fn apply_jev_builtin_config(
    conf: &mut crate::config::Config,
    config: &serde_json::Map<String, serde_json::Value>,
) {
    if let Some(base) = get_trimmed_config_str(config, "base_url") {
        conf.jev_base_url = base.to_string();
    }
    if let Some(m) = get_trimmed_config_str(config, "default_model") {
        conf.jev_openrouter_model = m.to_string();
    }
}

fn apply_muse_builtin_config(
    conf: &mut crate::config::Config,
    config: &serde_json::Map<String, serde_json::Value>,
) {
    if let Some(base) = get_trimmed_config_str(config, "base_url") {
        conf.museai_base_url = base.to_string();
    }
    if let Some(k) = get_trimmed_config_str(config, "api_key") {
        conf.museai_access_token = k.to_string();
    }
    if let Some(c) = get_trimmed_config_str(config, "cookie") {
        conf.museai_cookie = c.to_string();
    }
}

fn apply_builtin_module_config_updates(
    store: &ConfigStore,
    resolved_id: &str,
    config: &serde_json::Map<String, serde_json::Value>,
) {
    match resolved_id {
        "omniroute" => {
            let mut conf = store.write_guard();
            apply_omniroute_builtin_config(&mut conf, config);
        }
        "jev" => {
            let mut conf = store.write_guard();
            apply_jev_builtin_config(&mut conf, config);
        }
        "muse" => {
            let mut conf = store.write_guard();
            apply_muse_builtin_config(&mut conf, config);
        }
        _ => {}
    }
}

pub fn handle_update_module_config(
    client: &mut TcpStream,
    body: &[u8],
    store: &ConfigStore,
) -> io::Result<()> {
    #[derive(serde::Deserialize)]
    struct UpdateConfigPayload {
        id: String,
        #[serde(default)]
        config: serde_json::Map<String, serde_json::Value>,
        #[serde(default)]
        menu_position: Option<String>,
        #[serde(default)]
        menu_order: Option<i32>,
    }

    let payload: UpdateConfigPayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(e) => return write_error(client, 400, "Bad Request", &format!("Invalid JSON: {e}")),
    };

    let reg = store.module_registry();
    let mut guard = match reg.write() {
        Ok(g) => g,
        Err(e) => return write_error(client, 500, "Internal Error", &e.to_string()),
    };

    let resolved_id = guard
        .resolve_module_id(&payload.id)
        .unwrap_or_else(|| payload.id.clone());

    if payload.menu_position.is_some() || payload.menu_order.is_some() {
        if let Err(e) =
            guard.set_module_position(&resolved_id, payload.menu_position, payload.menu_order)
        {
            return write_error(client, 400, "Bad Request", &e.to_string());
        }
    }

    apply_builtin_module_config_updates(store, &resolved_id, &payload.config);

    if let Err(e) = guard.update_module_config(&resolved_id, payload.config.clone()) {
        return write_error(client, 400, "Bad Request", &e.to_string());
    }

    if let Some(enabled) = payload.config.get("enabled").and_then(|v| v.as_bool()) {
        let _ = guard.set_module_status(
            &resolved_id,
            if enabled {
                ModuleStatus::Enabled
            } else {
                ModuleStatus::Disabled
            },
        );
    }

    let desc = guard.get_module(&resolved_id).cloned();
    let masked = guard.get_masked_module_config(&resolved_id);
    drop(guard);

    if let Err(e) = store.save_to_persistent_file() {
        return write_error(
            client,
            500,
            "Internal Error",
            &format!("Module config updated but persistence failed: {e}"),
        );
    }

    let merged_cfg = builtin_masked_config(&resolved_id, store, masked);
    let status = desc.as_ref().map(|d| d.status);
    let resp = serde_json::json!({
        "ok": true,
        "status": "ok",
        "id": resolved_id,
        "message": format!("Module '{}' configuration updated and saved.", resolved_id),
        "module_status": status,
        "module": desc,
        "config": merged_cfg,
    });
    write_json_ok(client, &resp)
}

fn write_json_ok(client: &mut TcpStream, value: &serde_json::Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    )?;
    client.write_all(&bytes)?;
    client.flush()
}
