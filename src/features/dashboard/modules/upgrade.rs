//! Module upgrade and bulk upgrade request handlers.

use super::queue::{get_in_flight_upgrades, try_acquire_upgrade};
use super::source::resolve_upgrade_source_candidate;
use crate::config::ConfigStore;
use crate::gateway::installer::{get_state_dir, InstallOptions};
use crate::gateway::modules::ModuleKind;
use crate::http::write_error;
use std::io::{self, Write};
use std::net::TcpStream;

/// Upgrades an individual personal module.
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

    if existing_desc.kind == ModuleKind::BuiltIn {
        drop(read_guard);
        return write_error(
            client,
            400,
            "Bad Request",
            &format!("Built-in module '{resolved_id}' cannot be upgraded via this endpoint"),
        );
    }

    let source_candidate =
        resolve_upgrade_source_candidate(payload.source.as_deref(), &existing_desc, &resolved_id);

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
    let registered_mods = read_guard.modules().clone();
    drop(read_guard);

    let _upgrade_guard = match try_acquire_upgrade(&resolved_id) {
        Ok(g) => g,
        Err(msg) => return write_error(client, 409, "Conflict", &msg),
    };

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
        Some(&registered_mods),
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
    let updated_desc = match write_guard.upgrade_personal_module(new_manifest.clone()) {
        Ok(d) => d,
        Err(e) => {
            drop(write_guard);
            return write_error(client, 400, "Bad Request", &e.to_string());
        }
    };
    drop(write_guard);

    let _ = store.storage.save_module(&new_manifest);
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

/// Discovers or bulk-upgrades all eligible personal modules installed in the gateway.
pub fn handle_upgrade_all_modules(
    client: &mut TcpStream,
    body: &[u8],
    store: &ConfigStore,
) -> io::Result<()> {
    #[derive(serde::Deserialize, Default)]
    struct UpgradeAllPayload {
        #[serde(default)]
        execute: bool,
    }

    let payload: UpgradeAllPayload = serde_json::from_slice(body).unwrap_or_default();

    let reg = store.module_registry();
    let read_guard = match reg.read() {
        Ok(g) => g,
        Err(e) => return write_error(client, 500, "Internal Error", &e.to_string()),
    };

    let modules = read_guard.list_modules();
    let mut eligible = Vec::new();
    for m in &modules {
        if m.kind == ModuleKind::Personal {
            let source_opt = resolve_upgrade_source_candidate(None, m, &m.id);
            if let Some(src) = source_opt {
                eligible.push(serde_json::json!({
                    "id": m.id,
                    "name": m.name,
                    "version": m.version,
                    "source": src,
                    "eligible": true,
                }));
            }
        }
    }
    drop(read_guard);

    if !payload.execute {
        let resp = serde_json::json!({
            "ok": true,
            "total_eligible": eligible.len(),
            "modules": eligible,
        });
        let bytes = serde_json::to_vec(&resp).unwrap_or_default();
        write!(
            client,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        )?;
        client.write_all(&bytes)?;
        return client.flush();
    }

    // Execute bulk upgrade sequentially
    let mut upgraded = Vec::new();
    let mut failed = Vec::new();
    let state_dir = get_state_dir();

    for item in eligible {
        let mod_id = item["id"].as_str().unwrap_or_default().to_string();
        let source_str = item["source"].as_str().unwrap_or_default().to_string();

        let _guard = match try_acquire_upgrade(&mod_id) {
            Ok(g) => g,
            Err(msg) => {
                failed.push(serde_json::json!({ "id": mod_id, "error": msg }));
                continue;
            }
        };

        let read_guard = match reg.read() {
            Ok(g) => g,
            Err(e) => {
                failed.push(serde_json::json!({ "id": mod_id, "error": e.to_string() }));
                continue;
            }
        };
        let registered_mods = read_guard.modules().clone();
        drop(read_guard);

        let install_source = crate::gateway::installer::resolve_source(&source_str);
        let opts = InstallOptions {
            id: Some(mod_id.clone()),
            name: None,
            namespace: None,
            entrypoint: None,
            protocol: None,
            branch: None,
        };

        let new_manifest = match crate::gateway::installer::upgrade_module(
            &mod_id,
            install_source,
            opts,
            &state_dir,
            Some(&registered_mods),
        ) {
            Ok(m) => m,
            Err(e) => {
                failed.push(serde_json::json!({ "id": mod_id, "error": e.to_string() }));
                continue;
            }
        };

        let mut write_guard = match reg.write() {
            Ok(g) => g,
            Err(e) => {
                failed.push(serde_json::json!({ "id": mod_id, "error": e.to_string() }));
                continue;
            }
        };
        let updated_desc = match write_guard.upgrade_personal_module(new_manifest.clone()) {
            Ok(d) => d,
            Err(e) => {
                drop(write_guard);
                failed.push(serde_json::json!({ "id": mod_id, "error": e.to_string() }));
                continue;
            }
        };
        drop(write_guard);

        let _ = store.storage.save_module(&new_manifest);
        upgraded.push(serde_json::json!({
            "id": mod_id,
            "version": updated_desc.version,
            "name": updated_desc.name,
        }));
    }

    let _ = store.save_to_persistent_file();

    let resp = serde_json::json!({
        "ok": true,
        "upgraded": upgraded,
        "failed": failed,
    });
    let bytes = serde_json::to_vec(&resp).unwrap_or_default();
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    )?;
    client.write_all(&bytes)?;
    client.flush()
}

/// Returns current in-flight upgrade status.
pub fn handle_get_upgrade_queue_status(client: &mut TcpStream) -> io::Result<()> {
    let in_flight = get_in_flight_upgrades();
    let resp = serde_json::json!({
        "ok": true,
        "in_flight": in_flight,
        "count": in_flight.len()
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
