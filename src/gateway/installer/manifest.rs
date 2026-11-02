//! Manifest discovery, synthesis, and entrypoint wrapper generation.

use super::InstallOptions;
use crate::gateway::modules::{normalize_module_config_schema, ModuleError, PersonalManifest};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

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

const CANDIDATE_MANIFESTS: &[&str] = &[
    "module.json",
    "manifest.json",
    "__manifest__.json",
    "toolcase-module.json",
];

fn try_read_manifest_file(path: &Path, options: &InstallOptions) -> Option<PersonalManifest> {
    if !path.is_file() {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    let mut manifest = serde_json::from_str::<PersonalManifest>(&content).ok()?;
    apply_overrides(&mut manifest, options);
    Some(manifest)
}

fn find_manifest_in_dir(dir: &Path, options: &InstallOptions) -> Option<PersonalManifest> {
    CANDIDATE_MANIFESTS
        .iter()
        .find_map(|name| try_read_manifest_file(&dir.join(name), options))
}

fn collect_candidate_subdirs(dir: &Path, target_id: Option<&str>) -> Vec<PathBuf> {
    let mut candidate_subdirs = Vec::new();
    if let Some(target_id) = target_id {
        candidate_subdirs.push(dir.join("modules").join(target_id));
        candidate_subdirs.push(dir.join(target_id));
    }
    if let Ok(entries) = fs::read_dir(dir.join("modules")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                candidate_subdirs.push(path);
            }
        }
    }
    candidate_subdirs
}

fn find_manifest_in_subdirs(dir: &Path, options: &InstallOptions) -> Option<PersonalManifest> {
    let candidate_subdirs = collect_candidate_subdirs(dir, options.id.as_deref());
    for subdir in candidate_subdirs {
        if let Some(manifest) = find_manifest_in_dir(&subdir, options) {
            let _ = super::ingest::copy_dir_all(&subdir, dir);
            return Some(manifest);
        }
    }
    None
}

fn synthesize_id(dir: &Path, options: &InstallOptions) -> String {
    if let Some(ref id) = options.id {
        return id.clone();
    }
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
}

fn synthesize_name(id: &str, options: &InstallOptions) -> String {
    if let Some(ref name) = options.name {
        return name.clone();
    }
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
}

fn auto_synthesize_manifest(
    dir: &Path,
    options: &InstallOptions,
) -> Result<PersonalManifest, ModuleError> {
    let id = synthesize_id(dir, options);
    let name = synthesize_name(&id, options);
    let namespace = options
        .namespace
        .clone()
        .unwrap_or_else(|| format!("/custom/{id}"));
    let protocol = options
        .protocol
        .clone()
        .unwrap_or_else(|| "stdio-stream".to_string());
    let entrypoint = match options.entrypoint {
        Some(ref ep) => ep.clone(),
        None => detect_or_generate_entrypoint(dir)?,
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
        source: None,
    })
}

pub(super) fn discover_or_synthesize_manifest(
    dir: &Path,
    options: &InstallOptions,
) -> Result<PersonalManifest, ModuleError> {
    if let Some(manifest) = find_manifest_in_dir(dir, options) {
        return Ok(manifest);
    }
    if let Some(manifest) = find_manifest_in_subdirs(dir, options) {
        return Ok(manifest);
    }
    auto_synthesize_manifest(dir, options)
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

/// Resolves the module entrypoint to an absolute path inside target_dir and sets executable permissions on Unix.
pub(super) fn finalize_module_entrypoint(
    manifest: &mut PersonalManifest,
    target_dir: &Path,
) -> Result<(), ModuleError> {
    let abs_target_dir = if target_dir.is_relative() {
        std::env::current_dir()
            .ok()
            .map(|cwd| cwd.join(target_dir))
            .unwrap_or_else(|| target_dir.to_path_buf())
    } else {
        target_dir.to_path_buf()
    };

    let is_url_entrypoint = manifest.protocol == "local-http"
        || manifest.entrypoint.starts_with("http://")
        || manifest.entrypoint.starts_with("https://");
    if !is_url_entrypoint {
        let ep_path = Path::new(&manifest.entrypoint);
        let resolved_ep = if ep_path.is_relative() {
            abs_target_dir.join(ep_path)
        } else {
            ep_path.to_path_buf()
        };

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if resolved_ep.is_file() {
                let _ = fs::set_permissions(&resolved_ep, fs::Permissions::from_mode(0o700));
            } else if let Ok(can_ep) = resolved_ep.canonicalize() {
                if can_ep.is_file() {
                    let _ = fs::set_permissions(&can_ep, fs::Permissions::from_mode(0o700));
                }
            }
            if let Some(parent) = resolved_ep.parent() {
                let adapter_py = parent.join("adapter.py");
                if adapter_py.is_file() {
                    let _ = fs::set_permissions(&adapter_py, fs::Permissions::from_mode(0o700));
                }
            }
        }

        manifest.entrypoint = resolved_ep.to_string_lossy().to_string();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_manifest_in_root() {
        let temp_dir = std::env::temp_dir().join(format!("test_mf_{}", uuid::Uuid::new_v4()));
        let _ = fs::create_dir_all(&temp_dir);
        let mpath = temp_dir.join("module.json");
        let manifest_json = r#"{
            "id": "my-mod",
            "name": "My Mod",
            "version": "1.0.0",
            "namespace": "/custom/my-mod",
            "entrypoint": "main.py",
            "protocol": "stdio-stream",
            "capabilities": []
        }"#;
        fs::write(&mpath, manifest_json).unwrap();

        let options = InstallOptions::default();
        let manifest = discover_or_synthesize_manifest(&temp_dir, &options).expect("discovered");
        assert_eq!(manifest.id, "my-mod");
        assert_eq!(manifest.name, "My Mod");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_discover_manifest_in_subdir() {
        let temp_dir = std::env::temp_dir().join(format!("test_mf_sub_{}", uuid::Uuid::new_v4()));
        let sub = temp_dir.join("modules").join("sub-mod");
        let _ = fs::create_dir_all(&sub);
        let mpath = sub.join("module.json");
        let manifest_json = r#"{
            "id": "sub-mod",
            "name": "Sub Mod",
            "version": "1.0.0",
            "namespace": "/custom/sub-mod",
            "entrypoint": "app.py",
            "protocol": "stdio-stream",
            "capabilities": []
        }"#;
        fs::write(&mpath, manifest_json).unwrap();

        let options = InstallOptions {
            id: Some("sub-mod".to_string()),
            ..Default::default()
        };
        let manifest = discover_or_synthesize_manifest(&temp_dir, &options).expect("discovered");
        assert_eq!(manifest.id, "sub-mod");
        assert_eq!(manifest.name, "Sub Mod");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_synthesize_manifest_fallback() {
        let temp_dir = std::env::temp_dir().join(format!("test_mf_syn_{}", uuid::Uuid::new_v4()));
        let _ = fs::create_dir_all(&temp_dir);
        let py = temp_dir.join("main.py");
        fs::write(&py, "print('hi')").unwrap();

        let options = InstallOptions {
            id: Some("syn-mod".to_string()),
            namespace: Some("/custom/syn".to_string()),
            ..Default::default()
        };
        let manifest = discover_or_synthesize_manifest(&temp_dir, &options).expect("synthesized");
        assert_eq!(manifest.id, "syn-mod");
        assert_eq!(manifest.namespace, "/custom/syn");
        assert_eq!(manifest.entrypoint, "run.sh");

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
