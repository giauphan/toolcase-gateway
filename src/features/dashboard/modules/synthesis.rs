//! Synthesis and adapter generation for dynamic dashboard modules.

use crate::gateway::installer::{get_state_dir, install_module, InstallOptions};
use crate::gateway::modules::PersonalManifest;
use std::io;

#[derive(serde::Deserialize, Default)]
pub struct InstallPayload {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub zip_base64: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub entrypoint: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

pub fn generate_module_adapter_script(target_dir: &std::path::Path) -> io::Result<String> {
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

pub fn resolve_entrypoint_path(payload_entrypoint: Option<String>, id: &str) -> String {
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
    for root in &["addons", "modules"] {
        let p = std::path::Path::new(root).join(id).join("run.sh");
        if p.is_file() {
            return p.canonicalize().unwrap_or(p).to_string_lossy().to_string();
        }
    }
    let target = get_state_dir().join("modules").join(id).join("run.sh");
    let abs = if target.is_relative() {
        std::env::current_dir().unwrap_or_default().join(target)
    } else {
        target
    };
    abs.to_string_lossy().to_string()
}

pub fn parse_or_synthesize_manifest(
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
                if let Ok(mut m) = install_module(
                    crate::gateway::installer::resolve_source(src),
                    opts,
                    &state_dir,
                ) {
                    if m.config_schema.is_none() {
                        m.config_schema = cat.config_schema.clone();
                    }
                    return Ok((m, true));
                }
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
                    short_name: cat.short_name,
                    version: cat.version,
                    namespace: cat.namespace,
                    entrypoint: ep,
                    protocol: cat.protocol,
                    capabilities: cat.capabilities,
                    icon: cat.icon,
                    config_schema: cat.config_schema,
                    permissions: Vec::new(),
                    group,
                },
                false,
            ));
        }
    }
    let ns = payload
        .namespace
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "Route Path URL Prefix (namespace) is required (e.g. /custom/my-tool)".to_string()
        })?;
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
            short_name: None,
            version: "0.1.0".to_string(),
            namespace: ns.to_string(),
            entrypoint,
            protocol,
            capabilities: vec!["chat_completions".to_string(), "models".to_string()],
            icon: None,
            config_schema: None,
            permissions: Vec::new(),
            group: None,
        },
        false,
    ))
}
