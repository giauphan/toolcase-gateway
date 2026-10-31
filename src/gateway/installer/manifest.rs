//! Manifest discovery, synthesis, and entrypoint wrapper generation.

use super::InstallOptions;
use crate::gateway::modules::{normalize_module_config_schema, ModuleError, PersonalManifest};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

pub(super) fn read_module_manifest(mpath: &Path, base_dir: &Path) -> Option<PersonalManifest> {
    if !mpath.is_file() {
        return None;
    }
    let content = fs::read_to_string(mpath).ok()?;
    let mut manifest = serde_json::from_str::<PersonalManifest>(&content).ok()?;
    let abs_base = if base_dir.is_relative() {
        std::env::current_dir()
            .ok()
            .map(|cwd| cwd.join(base_dir))
            .unwrap_or_else(|| base_dir.to_path_buf())
    } else {
        base_dir.to_path_buf()
    };
    let is_url_entrypoint = manifest.protocol == "local-http"
        || manifest.entrypoint.starts_with("http://")
        || manifest.entrypoint.starts_with("https://");
    let ep = Path::new(&manifest.entrypoint);
    if !is_url_entrypoint && ep.is_relative() {
        manifest.entrypoint = abs_base.join(ep).to_string_lossy().to_string();
    }
    if let Some(schema) = manifest.config_schema.take() {
        manifest.config_schema = Some(normalize_module_config_schema(
            schema,
            &manifest.id,
            &manifest.namespace,
        ));
    }
    Some(manifest)
}

pub(super) fn discover_or_synthesize_manifest(
    dir: &Path,
    options: &InstallOptions,
) -> Result<PersonalManifest, ModuleError> {
    let candidate_manifests = [
        "module.json",
        "manifest.json",
        "__manifest__.json",
        "toolcase-module.json",
    ];

    for name in candidate_manifests {
        let manifest_file = dir.join(name);
        if manifest_file.is_file() {
            if let Ok(content) = fs::read_to_string(&manifest_file) {
                if let Ok(mut manifest) = serde_json::from_str::<PersonalManifest>(&content) {
                    apply_overrides(&mut manifest, options);
                    return Ok(manifest);
                }
            }
        }
    }

    // Auto-synthesize manifest from repository structure
    let id = options.id.clone().unwrap_or_else(|| {
        if let Some(ref ns) = options.namespace {
            let clean = ns.trim_matches('/').replace('/', "-");
            let filtered: String = clean
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !filtered.is_empty() {
                return filtered;
            }
        }
        let stem = dir
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("custom-module");
        sanitize_id(stem)
    });

    let name = options.name.clone().unwrap_or_else(|| {
        id.split(['-', '_'])
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    });

    let namespace = options
        .namespace
        .clone()
        .unwrap_or_else(|| format!("/custom/{id}"));

    let protocol = options
        .protocol
        .clone()
        .unwrap_or_else(|| "stdio-stream".to_string());

    let entrypoint = if let Some(ref ep) = options.entrypoint {
        ep.clone()
    } else {
        detect_or_generate_entrypoint(dir)?
    };

    Ok(PersonalManifest {
        id,
        name,
        short_name: None,
        version: "0.1.0".to_string(),
        namespace,
        entrypoint,
        protocol,
        capabilities: vec!["chat_completions".to_string(), "models".to_string()],
        icon: None,
        config_schema: None,
        permissions: Vec::new(),
        group: None,
    })
}

fn detect_or_generate_entrypoint(dir: &Path) -> Result<String, ModuleError> {
    let script_candidates = ["run.sh", "start.sh", "adapter.sh", "entrypoint.sh"];
    for script in script_candidates {
        let p = dir.join(script);
        if p.is_file() {
            return Ok(script.to_string());
        }
    }

    let py_candidates = ["main.py", "app.py", "server.py", "adapter.py"];
    for py in py_candidates {
        if dir.join(py).is_file() {
            let wrapper = dir.join("run.sh");
            let content = format!("#!/bin/sh\nexec python3 \"$(dirname \"$0\")/{py}\" \"$@\"\n");
            let mut f = File::create(&wrapper).map_err(|e| {
                ModuleError::InvalidManifest(format!("Failed to create wrapper: {e}"))
            })?;
            f.write_all(content.as_bytes()).map_err(|e| {
                ModuleError::InvalidManifest(format!("Failed to write wrapper: {e}"))
            })?;
            return Ok("run.sh".to_string());
        }
    }

    let js_candidates = ["index.js", "main.js", "server.js", "app.js"];
    for js in js_candidates {
        if dir.join(js).is_file() {
            let wrapper = dir.join("run.sh");
            let content = format!("#!/bin/sh\nexec node \"$(dirname \"$0\")/{js}\" \"$@\"\n");
            let mut f = File::create(&wrapper).map_err(|e| {
                ModuleError::InvalidManifest(format!("Failed to create wrapper: {e}"))
            })?;
            f.write_all(content.as_bytes()).map_err(|e| {
                ModuleError::InvalidManifest(format!("Failed to write wrapper: {e}"))
            })?;
            return Ok("run.sh".to_string());
        }
    }

    Err(ModuleError::InvalidManifest(
        "No manifest or recognizable entrypoint found (looked for module.json, run.sh, main.py, index.js)".to_string(),
    ))
}

fn apply_overrides(manifest: &mut PersonalManifest, options: &InstallOptions) {
    if let Some(ref id) = options.id {
        if !id.trim().is_empty() {
            manifest.id = id.trim().to_string();
        }
    }
    if let Some(ref name) = options.name {
        if !name.trim().is_empty() {
            manifest.name = name.trim().to_string();
        }
    }
    if let Some(ref ns) = options.namespace {
        if !ns.trim().is_empty() {
            manifest.namespace = ns.trim().to_string();
        }
    }
    if let Some(ref ep) = options.entrypoint {
        if !ep.trim().is_empty() {
            manifest.entrypoint = ep.trim().to_string();
        }
    }
    if let Some(ref proto) = options.protocol {
        if !proto.trim().is_empty() {
            manifest.protocol = proto.trim().to_string();
        }
    }
    if let Some(schema) = manifest.config_schema.take() {
        manifest.config_schema = Some(normalize_module_config_schema(
            schema,
            &manifest.id,
            &manifest.namespace,
        ));
    }
}

fn sanitize_id(raw: &str) -> String {
    let clean: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty() {
        "custom-module".to_string()
    } else {
        clean
    }
}
