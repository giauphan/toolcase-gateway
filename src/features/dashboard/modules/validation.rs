//! Validation endpoint handler for candidate module packages and manifests.

use super::synthesis::InstallPayload;
use crate::config::ConfigStore;
use crate::gateway::installer::{
    get_state_dir, validate_module_manifest_direct, validate_module_source, InstallOptions,
    InstallSource,
};
use crate::http::write_error;
use std::io::{self, Write};
use std::net::TcpStream;

pub fn handle_validate_module_config(
    client: &mut TcpStream,
    body: &[u8],
    store: &ConfigStore,
) -> io::Result<()> {
    let payload = serde_json::from_slice::<InstallPayload>(body).unwrap_or_default();
    let clean_source = payload
        .source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let clean_zip = payload
        .zip_base64
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let has_direct_source = clean_source.is_some() || clean_zip.is_some();

    let reg = store.module_registry();
    let reg_guard = match reg.read() {
        Ok(g) => g,
        Err(e) => return write_error(client, 500, "Internal Error", &e.to_string()),
    };
    let registered_modules = reg_guard.modules().clone();
    drop(reg_guard);

    let state_dir = get_state_dir();

    let report_result = if has_direct_source {
        let source = if let Some(ref s) = clean_source {
            crate::gateway::installer::resolve_source(s)
        } else if let Some(ref b64) = clean_zip {
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

        validate_module_source(&source, &opts, &state_dir, &registered_modules)
    } else {
        let catalog_source = payload
            .id
            .as_deref()
            .or(payload.namespace.as_deref())
            .and_then(|target_id| {
                crate::gateway::modules::get_system_module_catalog()
                    .into_iter()
                    .find(|c| c.id == target_id || c.namespace == target_id)
                    .and_then(|c| c.source.map(|s| (c.id, c.name, c.namespace, c.protocol, s)))
            });

        if let Some((cat_id, cat_name, cat_ns, cat_proto, cat_src)) = catalog_source {
            let source = crate::gateway::installer::resolve_source(&cat_src);
            let opts = InstallOptions {
                id: payload.id.or(Some(cat_id)),
                name: payload.name.or(Some(cat_name)),
                namespace: payload.namespace.or(Some(cat_ns)),
                entrypoint: payload.entrypoint,
                protocol: payload.protocol.or(Some(cat_proto)),
                branch: payload.branch,
            };
            validate_module_source(&source, &opts, &state_dir, &registered_modules)
        } else {
            match super::synthesis::parse_or_synthesize_manifest_non_destructive(body, &payload) {
                Ok(manifest) => validate_module_manifest_direct(&manifest, &registered_modules),
                Err(e) => {
                    let resp = serde_json::json!({
                        "ok": false,
                        "valid": false,
                        "message": e,
                        "diagnostics": vec![format!("Failed to parse or synthesize configuration: {e}")],
                    });
                    let resp_bytes = serde_json::to_vec(&resp).unwrap_or_default();
                    write!(
                        client,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        resp_bytes.len()
                    )?;
                    client.write_all(&resp_bytes)?;
                    return client.flush();
                }
            }
        }
    };

    match report_result {
        Ok(report) => {
            let resp = serde_json::json!({
                "ok": true,
                "valid": report.valid,
                "report": report,
                "module": {
                    "id": report.manifest.id,
                    "name": report.manifest.name,
                    "namespace": report.manifest.namespace,
                    "entrypoint": report.manifest.entrypoint,
                    "protocol": report.manifest.protocol,
                    "capabilities": report.manifest.capabilities,
                    "entrypoint_type": report.entrypoint_type,
                    "has_manifest_file": report.has_manifest_file,
                    "is_update": report.is_update,
                    "config_schema": report.manifest.config_schema,
                },
                "warnings": report.warnings,
                "diagnostics": report.diagnostics,
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
        Err(e) => {
            let resp = serde_json::json!({
                "ok": false,
                "valid": false,
                "message": e.to_string(),
                "diagnostics": vec![format!("Validation rejected: {e}")],
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
    }
}
