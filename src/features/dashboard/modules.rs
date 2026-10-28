//! Dashboard personal module management handlers.

use crate::config::ConfigStore;
use crate::gateway::installer::{
    cleanup_module_files, get_state_dir, install_module, InstallOptions, InstallSource,
};
use crate::gateway::modules::{ModuleStatus, PersonalManifest};
use crate::http::write_error;
use std::io::{self, Write};
use std::net::TcpStream;

#[derive(serde::Deserialize, Default)]
struct InstallPayload {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    zip_base64: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    entrypoint: Option<String>,
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    branch: Option<String>,
}

fn parse_or_synthesize_manifest(
    body: &[u8],
    payload: &InstallPayload,
) -> Result<PersonalManifest, String> {
    if let Ok(m) = serde_json::from_slice::<PersonalManifest>(body) {
        return Ok(m);
    }
    let Some(ref ns) = payload.namespace else {
        return Err("Route Path URL Prefix (namespace) is required (e.g. /custom/my-tool)".into());
    };
    if ns.trim().is_empty() {
        return Err("Route Path URL Prefix (namespace) is required (e.g. /custom/my-tool)".into());
    }
    let id = payload.id.clone().unwrap_or_else(|| {
        let clean = ns.trim_matches('/').replace('/', "-");
        let filtered: String = clean
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if !filtered.is_empty() {
            filtered
        } else {
            format!("mod-{}", &uuid::Uuid::new_v4().to_string()[..8])
        }
    });
    let name = payload.name.clone().unwrap_or_else(|| id.clone());
    let entrypoint = payload
        .entrypoint
        .clone()
        .unwrap_or_else(|| "/bin/echo".to_string());
    let protocol = payload
        .protocol
        .clone()
        .unwrap_or_else(|| "stdio-stream".to_string());
    Ok(PersonalManifest {
        id,
        name,
        version: "0.1.0".to_string(),
        namespace: ns.clone(),
        entrypoint,
        protocol,
        capabilities: vec!["chat_completions".to_string(), "models".to_string()],
        config_schema: None,
        permissions: Vec::new(),
    })
}

pub fn handle_set_module_state(
    client: &mut TcpStream,
    body: &[u8],
    store: &ConfigStore,
) -> io::Result<()> {
    #[derive(serde::Deserialize)]
    struct StatePayload {
        id: String,
        enabled: bool,
    }
    let payload: StatePayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(e) => {
            return write_error(client, 400, "Bad Request", &format!("Invalid JSON: {e}"));
        }
    };
    let status = if payload.enabled {
        ModuleStatus::Enabled
    } else {
        ModuleStatus::Disabled
    };

    let reg = store.module_registry();
    let mut guard = match reg.write() {
        Ok(g) => g,
        Err(e) => return write_error(client, 500, "Internal Error", &e.to_string()),
    };
    if let Err(e) = guard.set_module_status(&payload.id, status) {
        return write_error(client, 400, "Bad Request", &e.to_string());
    }
    drop(guard);
    if let Err(e) = store.save_to_persistent_file() {
        return write_error(
            client,
            500,
            "Internal Error",
            &format!("Module state changed but persistence failed: {e}"),
        );
    }

    let resp = serde_json::json!({
        "ok": true,
        "id": payload.id,
        "status": status
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

pub fn handle_install_personal_module(
    client: &mut TcpStream,
    body: &[u8],
    store: &ConfigStore,
) -> io::Result<()> {
    let payload = serde_json::from_slice::<InstallPayload>(body).unwrap_or_default();
    let installed_from_source = payload.source.is_some() || payload.zip_base64.is_some();
    let manifest = if installed_from_source {
        let source = if let Some(ref s) = payload.source {
            crate::gateway::installer::resolve_source(s)
        } else if let Some(ref b64) = payload.zip_base64 {
            match crate::gateway::installer::decode_base64(b64) {
                Ok(bytes) => InstallSource::ZipBytes(bytes),
                Err(e) => return write_error(client, 400, "Bad Request", &e),
            }
        } else {
            return write_error(client, 400, "Bad Request", "Missing installation source");
        };

        let opts = InstallOptions {
            id: payload.id,
            name: payload.name,
            namespace: payload.namespace,
            entrypoint: payload.entrypoint,
            protocol: payload.protocol,
            branch: payload.branch,
        };
        let state_dir = get_state_dir();
        match install_module(source, opts, &state_dir) {
            Ok(m) => m,
            Err(e) => return write_error(client, 400, "Bad Request", &e.to_string()),
        }
    } else {
        match parse_or_synthesize_manifest(body, &payload) {
            Ok(m) => m,
            Err(e) => return write_error(client, 400, "Bad Request", &e),
        }
    };

    let id = manifest.id.clone();
    let reg = store.module_registry();
    let mut guard = match reg.write() {
        Ok(g) => g,
        Err(e) => return write_error(client, 500, "Internal Error", &e.to_string()),
    };
    if let Err(e) = guard.register_personal_module(manifest) {
        drop(guard);
        if installed_from_source {
            cleanup_module_files(&id, &get_state_dir());
        }
        return write_error(client, 400, "Bad Request", &e.to_string());
    }
    drop(guard);
    if let Err(e) = store.save_to_persistent_file() {
        return write_error(
            client,
            500,
            "Internal Error",
            &format!("Module state changed but persistence failed: {e}"),
        );
    }

    let resp = serde_json::json!({
        "ok": true,
        "id": id,
        "message": "Personal module installed successfully"
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

pub fn handle_remove_personal_module(
    client: &mut TcpStream,
    body: &[u8],
    store: &ConfigStore,
) -> io::Result<()> {
    #[derive(serde::Deserialize)]
    struct RemovePayload {
        id: String,
    }
    let payload: RemovePayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(e) => return write_error(client, 400, "Bad Request", &format!("Invalid JSON: {e}")),
    };
    let reg = store.module_registry();
    let mut guard = match reg.write() {
        Ok(g) => g,
        Err(e) => return write_error(client, 500, "Internal Error", &e.to_string()),
    };
    if let Err(e) = guard.unregister_personal_module(&payload.id) {
        return write_error(client, 400, "Bad Request", &e.to_string());
    }
    drop(guard);
    let state_dir = get_state_dir();
    cleanup_module_files(&payload.id, &state_dir);
    if let Err(e) = store.save_to_persistent_file() {
        return write_error(
            client,
            500,
            "Internal Error",
            &format!("Module removed but persistence failed: {e}"),
        );
    }
    let resp = serde_json::json!({
        "ok": true,
        "id": payload.id,
        "message": "Personal module removed successfully"
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
