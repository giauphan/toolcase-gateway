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

fn generate_module_adapter_script(target_dir: &std::path::Path) -> io::Result<String> {
    std::fs::create_dir_all(target_dir)?;
    let adapter_py = target_dir.join("adapter.py");
    let py_script = r#"import os, sys, json, time, uuid

M_ID = os.environ.get("MODULE_ID", "custom-module")
M_NAME = os.environ.get("MODULE_NAME", M_ID)

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
        path = req.get("path", "")
        raw_body = req.get("body", "")
        try:
            body = json.loads(raw_body) if raw_body else {}
        except Exception:
            body = {}

        if path.endswith("/models"):
            resp = {
                "object": "list",
                "data": [{"id": M_ID, "object": "model", "created": int(time.time()), "owned_by": M_ID}],
            }
        else:
            messages = body.get("messages", [])
            last_msg = ""
            for m in reversed(messages):
                if m.get("role") == "user":
                    last_msg = m.get("content", "")
                    break
            sub_id = os.environ.get("MODULE_SUB_PROVIDER_ID", "")
            prefix = f"[{M_NAME}/{sub_id}]" if sub_id else f"[{M_NAME}]"
            reply = f"{prefix} Echo: {last_msg}" if last_msg else f"{prefix} Ready."
            resp = {
                "id": f"chatcmpl-{uuid.uuid4().hex[:8]}",
                "object": "chat.completion",
                "created": int(time.time()),
                "model": M_ID,
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": reply},
                    "finish_reason": "stop",
                }],
                "usage": {"prompt_tokens": 5, "completion_tokens": 5, "total_tokens": 10},
            }
        out = {
            "status": 200,
            "headers": {"content-type": "application/json"},
            "body": json.dumps(resp),
        }
    except Exception as e:
        out = {
            "status": 500,
            "headers": {"content-type": "application/json"},
            "body": json.dumps({"error": str(e)}),
        }
    sys.stdout.write(json.dumps(out) + "\n")
    sys.stdout.flush()
"#;
    std::fs::write(&adapter_py, py_script)?;

    let run_sh = target_dir.join("run.sh");
    let sh_script = r#"#!/bin/sh
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec python3 "$SCRIPT_DIR/adapter.py" "$@"
"#;
    std::fs::write(&run_sh, sh_script)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&run_sh, std::fs::Permissions::from_mode(0o700));
        let _ = std::fs::set_permissions(&adapter_py, std::fs::Permissions::from_mode(0o700));
    }
    let can = run_sh.canonicalize().unwrap_or(run_sh);
    Ok(can.to_string_lossy().to_string())
}

fn resolve_entrypoint_path(payload_entrypoint: Option<String>, id: &str) -> String {
    if let Some(ep) = payload_entrypoint {
        let p = std::path::Path::new(&ep);
        if p.is_file() {
            return p
                .canonicalize()
                .unwrap_or_else(|_| p.to_path_buf())
                .to_string_lossy()
                .to_string();
        }
        if !ep.trim().is_empty() {
            return ep;
        }
    }
    let target_ep = get_state_dir().join("modules").join(id).join("run.sh");
    let abs_ep = if target_ep.is_relative() {
        std::env::current_dir().unwrap_or_default().join(target_ep)
    } else {
        target_ep
    };
    abs_ep.to_string_lossy().to_string()
}

fn parse_or_synthesize_manifest(
    body: &[u8],
    payload: &InstallPayload,
) -> Result<(PersonalManifest, bool), String> {
    if let Ok(m) = serde_json::from_slice::<PersonalManifest>(body) {
        return Ok((m, false));
    }
    if let Some(target_id) = payload.id.as_deref().or(payload.namespace.as_deref()) {
        if let Some(cat) = crate::gateway::modules::get_system_module_catalog()
            .into_iter()
            .find(|c| c.id == target_id || c.namespace == target_id)
        {
            if let Some(ref src) = cat.source {
                let state_dir = get_state_dir();
                let opts = InstallOptions {
                    id: Some(cat.id.clone()),
                    name: Some(cat.name.clone()),
                    namespace: payload
                        .namespace
                        .clone()
                        .or_else(|| Some(cat.namespace.clone())),
                    entrypoint: payload.entrypoint.clone(),
                    protocol: payload
                        .protocol
                        .clone()
                        .or_else(|| Some(cat.protocol.clone())),
                    branch: payload.branch.clone(),
                };
                let mut m = install_module(
                    crate::gateway::installer::resolve_source(src),
                    opts,
                    &state_dir,
                )
                .map_err(|e| format!("Catalog install from {src} failed: {e}"))?;
                if m.config_schema.is_none() {
                    m.config_schema = cat.config_schema.clone();
                }
                return Ok((m, true));
            }
            let ep = resolve_entrypoint_path(payload.entrypoint.clone(), &cat.id);
            let group = if cat.id == "suite-test" {
                Some(crate::gateway::modules::ModuleGroupManifest {
                    enabled: true,
                    providers: vec![
                        crate::gateway::modules::SubProviderManifest {
                            id: "chat".to_string(),
                            name: "Chat Provider".to_string(),
                            subpath: "/chat".to_string(),
                            capabilities: vec!["chat_completions".to_string()],
                        },
                        crate::gateway::modules::SubProviderManifest {
                            id: "vision".to_string(),
                            name: "Vision Provider".to_string(),
                            subpath: "/vision".to_string(),
                            capabilities: vec!["vision".to_string()],
                        },
                    ],
                })
            } else {
                None
            };
            return Ok((
                PersonalManifest {
                    id: cat.id,
                    name: cat.name,
                    version: cat.version,
                    namespace: cat.namespace,
                    entrypoint: ep,
                    protocol: cat.protocol,
                    capabilities: cat.capabilities,
                    config_schema: cat.config_schema,
                    permissions: Vec::new(),
                    group,
                },
                false,
            ));
        }
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
    let entrypoint = resolve_entrypoint_path(payload.entrypoint.clone(), &id);
    let protocol = payload
        .protocol
        .clone()
        .unwrap_or_else(|| "stdio-stream".to_string());
    Ok((
        PersonalManifest {
            id,
            name,
            version: "0.1.0".to_string(),
            namespace: ns.clone(),
            entrypoint,
            protocol,
            capabilities: vec!["chat_completions".to_string(), "models".to_string()],
            config_schema: None,
            permissions: Vec::new(),
            group: None,
        },
        false,
    ))
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
