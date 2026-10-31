//! Dashboard personal module management handlers and route dispatchers.

mod synthesis;

pub use synthesis::{generate_module_adapter_script, parse_or_synthesize_manifest, InstallPayload};

use crate::config::ConfigStore;
use crate::gateway::installer::{
    cleanup_module_files, get_state_dir, install_module, InstallOptions, InstallSource,
};
use crate::gateway::modules::ModuleStatus;
use crate::http::write_error;
use std::io::{self, Write};
use std::net::TcpStream;

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
    let has_direct_source = payload.source.is_some() || payload.zip_base64.is_some();
    let (manifest, installed_from_source) = if has_direct_source {
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
            Ok(m) => (m, true),
            Err(e) => return write_error(client, 400, "Bad Request", &e.to_string()),
        }
    } else {
        match parse_or_synthesize_manifest(body, &payload) {
            Ok(res) => res,
            Err(e) => return write_error(client, 400, "Bad Request", &e),
        }
    };

    let id = manifest.id.clone();
    let reg = store.module_registry();
    let mut guard = match reg.write() {
        Ok(g) => g,
        Err(e) => return write_error(client, 500, "Internal Error", &e.to_string()),
    };
    if let Err(e) = guard.register_personal_module(manifest.clone()) {
        drop(guard);
        if installed_from_source {
            cleanup_module_files(&id, &get_state_dir());
        }
        return write_error(client, 400, "Bad Request", &e.to_string());
    }
    drop(guard);

    if !installed_from_source {
        let target_dir = get_state_dir().join("modules").join(&id);
        if !target_dir.join("run.sh").is_file() {
            let _ = generate_module_adapter_script(&target_dir);
        }
        let _ = std::fs::write(
            target_dir.join("module.json"),
            serde_json::to_string_pretty(&manifest).unwrap_or_default(),
        );
    }
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

pub fn handle_upgrade_personal_module(
    client: &mut TcpStream,
    body: &[u8],
    store: &ConfigStore,
) -> io::Result<()> {
    #[derive(serde::Deserialize)]
    struct UpgradePayload {
        id: String,
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        branch: Option<String>,
    }

    let payload: UpgradePayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(e) => return write_error(client, 400, "Bad Request", &format!("Invalid JSON: {e}")),
    };

    let id = payload.id.trim();
    if id.is_empty() {
        return write_error(client, 400, "Bad Request", "Module ID is required");
    }

    let reg = store.module_registry();
    let read_guard = match reg.read() {
        Ok(g) => g,
        Err(e) => return write_error(client, 500, "Internal Error", &e.to_string()),
    };

    let resolved_id = read_guard
        .resolve_module_id(id)
        .unwrap_or_else(|| id.to_string());

    let existing_desc = match read_guard.get_module(&resolved_id) {
        Some(d) => d.clone(),
        None => {
            drop(read_guard);
            return write_error(
                client,
                404,
                "Not Found",
                &format!("Module '{resolved_id}' is not installed"),
            );
        }
    };

    if existing_desc.kind == crate::gateway::modules::ModuleKind::BuiltIn {
        drop(read_guard);
        return write_error(
            client,
            400,
            "Bad Request",
            &format!("Built-in module '{resolved_id}' cannot be upgraded via this endpoint"),
        );
    }

    let source_candidate = payload
        .source
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .or_else(|| existing_desc.source.clone())
        .or_else(|| {
            crate::gateway::modules::get_system_module_catalog()
                .into_iter()
                .find(|c| c.id == resolved_id || c.namespace == existing_desc.namespace)
                .and_then(|c| c.source)
        });

    let raw_source = match source_candidate {
        Some(s) => s,
        None => {
            drop(read_guard);
            return write_error(
                client,
                400,
                "Bad Request",
                &format!(
                    "Module '{resolved_id}' has no configured source repository to upgrade from. Please provide a 'source' parameter."
                ),
            );
        }
    };
    drop(read_guard);

    let install_source = crate::gateway::installer::resolve_source(&raw_source);
    let opts = InstallOptions {
        id: Some(resolved_id.clone()),
        name: None,
        namespace: None,
        entrypoint: None,
        protocol: None,
        branch: payload.branch,
    };

    let state_dir = get_state_dir();
    let new_manifest = match crate::gateway::installer::upgrade_module(
        &resolved_id,
        install_source,
        opts,
        &state_dir,
    ) {
        Ok(m) => m,
        Err(e) => {
            return write_error(
                client,
                400,
                "Bad Request",
                &format!("Module upgrade failed: {e}"),
            )
        }
    };

    let mut write_guard = match reg.write() {
        Ok(g) => g,
        Err(e) => return write_error(client, 500, "Internal Error", &e.to_string()),
    };
    let updated_desc = match write_guard.upgrade_personal_module(new_manifest) {
        Ok(d) => d,
        Err(e) => {
            drop(write_guard);
            return write_error(client, 400, "Bad Request", &e.to_string());
        }
    };
    drop(write_guard);

    if let Err(e) = store.save_to_persistent_file() {
        return write_error(
            client,
            500,
            "Internal Error",
            &format!("Module upgraded but persistence failed: {e}"),
        );
    }

    let resp = serde_json::json!({
        "ok": true,
        "id": resolved_id,
        "version": updated_desc.version,
        "message": format!(
            "Module '{}' upgraded successfully to version {}.",
            resolved_id,
            updated_desc.version.as_deref().unwrap_or("latest")
        ),
        "module": updated_desc
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
