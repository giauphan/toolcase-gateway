//! Platform module installer and converter.
//!
//! Supports installing self-contained modules from GitHub repositories,
//! zip archives, or local directories with automated manifest discovery,
//! conversion, path sanitization, and execution wrapping.

use crate::gateway::archive::extract_zip;
use crate::gateway::modules::{ModuleError, PersonalManifest};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Source descriptor for module installation.
#[derive(Debug, Clone)]
pub enum InstallSource {
    GitHubUrl(String),
    ZipUrl(String),
    ZipFile(PathBuf),
    ZipBytes(Vec<u8>),
    LocalDir(PathBuf),
}

/// Optional configuration overrides when installing a module.
#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    pub id: Option<String>,
    pub name: Option<String>,
    pub namespace: Option<String>,
    pub entrypoint: Option<String>,
    pub protocol: Option<String>,
    pub branch: Option<String>,
}

/// Returns the runtime state directory from `GW_STATE_DIR` or defaults to `.gw_state`.
pub fn get_state_dir() -> PathBuf {
    std::env::var("GW_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".gw_state"))
}

/// Auto-discovers self-contained modules from directories (Odoo / WordPress paradigm).
pub fn scan_and_register_modules(
    registry: &mut crate::gateway::modules::ModuleRegistry,
    state_dir: &Path,
) {
    let search_roots = [state_dir.join("modules"), PathBuf::from("modules")];
    for root in &search_roots {
        scan_directory_for_modules(root, registry);
    }
}

fn scan_directory_for_modules(root: &Path, registry: &mut crate::gateway::modules::ModuleRegistry) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            try_register_module_from_dir(&path, registry);
        }
    }
}

fn try_register_module_from_dir(
    path: &Path,
    registry: &mut crate::gateway::modules::ModuleRegistry,
) {
    const CANDIDATES: &[&str] = &[
        "module.json",
        "manifest.json",
        "__manifest__.json",
        "toolcase-module.json",
    ];

    for name in CANDIDATES {
        let mpath = path.join(name);
        if let Some(manifest) = read_module_manifest(&mpath, path) {
            if registry.get_module(&manifest.id).is_none() {
                let _ = registry.register_personal_module(manifest);
            }
            break;
        }
    }
}

fn read_module_manifest(mpath: &Path, base_dir: &Path) -> Option<PersonalManifest> {
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
    Some(manifest)
}

/// Installs a module from any supported source into `$GW_STATE_DIR/modules/<id>`.
/// Discovers or synthesizes a self-contained `PersonalManifest`.
pub fn install_module(
    source: InstallSource,
    options: InstallOptions,
    state_dir: &Path,
) -> Result<PersonalManifest, ModuleError> {
    let modules_root = state_dir.join("modules");
    fs::create_dir_all(&modules_root)
        .map_err(|e| ModuleError::InvalidManifest(format!("Failed to create modules root: {e}")))?;

    // Create a temporary staging directory to unpack the payload
    let staging_id = format!("stage_{}", uuid::Uuid::new_v4());
    let staging_dir = modules_root.join(&staging_id);
    fs::create_dir_all(&staging_dir)
        .map_err(|e| ModuleError::InvalidManifest(format!("Failed to create staging dir: {e}")))?;

    // Ingest the source files into staging_dir
    let ingest_result = match source {
        InstallSource::GitHubUrl(url) => {
            ingest_github(&url, options.branch.as_deref(), &staging_dir)
        }
        InstallSource::ZipUrl(url) => ingest_zip_url(&url, &staging_dir),
        InstallSource::ZipFile(path) => ingest_zip_file(&path, &staging_dir),
        InstallSource::ZipBytes(bytes) => extract_zip(&bytes, &staging_dir)
            .map(|_| ())
            .map_err(|e| ModuleError::InvalidManifest(e.to_string())),
        InstallSource::LocalDir(dir) => copy_dir_all(&dir, &staging_dir)
            .map_err(|e| ModuleError::InvalidManifest(format!("Failed to copy directory: {e}"))),
    };

    if let Err(e) = ingest_result {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(e);
    }

    unwrap_single_top_level_directory(&staging_dir);

    // Discover or synthesize the manifest
    let mut manifest = match discover_or_synthesize_manifest(&staging_dir, &options) {
        Ok(m) => m,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(e);
        }
    };

    // Validate ID strictly to prevent path traversal or malformed paths
    let id_valid = !manifest.id.is_empty()
        && manifest
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !id_valid {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(ModuleError::InvalidId(manifest.id.clone()));
    }

    // Validate Namespace (path URL prefix is required and must start with /)
    let ns_trimmed = manifest.namespace.trim().trim_end_matches('/');
    if let Err(e) = crate::modules::validation::validate_module_namespace(
        ns_trimmed,
        &manifest.namespace,
        &manifest.id,
        &std::collections::HashMap::new(),
    ) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(e);
    }
    manifest.namespace = ns_trimmed.to_string();

    if let Some(ref grp) = manifest.group {
        if let Err(e) = crate::modules::validation::validate_sub_providers(&grp.providers) {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(e);
        }
    }

    // Determine final module directory
    let target_dir = modules_root.join(&manifest.id);
    if target_dir.exists() {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(ModuleError::InvalidManifest(format!(
            "Module directory for '{}' already exists; remove the module first",
            manifest.id
        )));
    }

    if let Err(e) = fs::rename(&staging_dir, &target_dir) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(ModuleError::InvalidManifest(format!(
            "Failed to finalize module directory: {e}"
        )));
    }

    let abs_target_dir = if target_dir.is_relative() {
        std::env::current_dir()
            .ok()
            .map(|cwd| cwd.join(&target_dir))
            .unwrap_or_else(|| target_dir.clone())
    } else {
        target_dir.clone()
    };

    // Resolve entrypoint to absolute path inside target_dir for local executables
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
            if let (Ok(can_ep), Ok(can_target)) =
                (resolved_ep.canonicalize(), abs_target_dir.canonicalize())
            {
                if can_ep.starts_with(&can_target) && can_ep.is_file() {
                    let _ = fs::set_permissions(&can_ep, fs::Permissions::from_mode(0o700));
                }
            }
        }

        manifest.entrypoint = resolved_ep.to_string_lossy().to_string();
    }

    // Persist finalized manifest.json into the module directory
    let manifest_path = target_dir.join("module.json");
    if let Ok(json) = serde_json::to_string_pretty(&manifest) {
        let _ = fs::write(manifest_path, json);
    }

    Ok(manifest)
}

/// Cleans up extracted module files from `$GW_STATE_DIR/modules/<id>`.
pub fn cleanup_module_files(id: &str, state_dir: &Path) {
    let target = state_dir.join("modules").join(id);
    if target.exists() {
        let _ = fs::remove_dir_all(&target);
    }
}

fn ingest_github(url: &str, branch: Option<&str>, target_dir: &Path) -> Result<(), ModuleError> {
    let clean_url = url.trim().trim_end_matches('/');
    let path = clean_url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("github.com/");
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() < 2 {
        return Err(ModuleError::InvalidManifest(format!(
            "Invalid GitHub URL: {url}"
        )));
    }
    let owner = parts[0];
    let repo = parts[1].trim_end_matches(".git");

    let inferred_branch = if branch.is_none() && parts.len() >= 4 && parts[2] == "tree" {
        Some(parts[3])
    } else {
        branch
    };

    let branches_to_try = if let Some(b) = inferred_branch {
        vec![b.to_string()]
    } else {
        vec!["main".to_string(), "master".to_string()]
    };

    let mut last_err = String::new();

    for b in branches_to_try {
        let zip_url = format!("https://codeload.github.com/{owner}/{repo}/zip/refs/heads/{b}");
        match fetch_url_bytes(&zip_url) {
            Ok(bytes) => {
                extract_zip(&bytes, target_dir)
                    .map_err(|e| ModuleError::InvalidManifest(e.to_string()))?;
                return Ok(());
            }
            Err(e) => {
                last_err = format!("Failed to download branch '{b}': {e}");
            }
        }
    }

    // If HTTP zip download failed, attempt git clone as fallback
    let mut git_cmd = std::process::Command::new("git");
    git_cmd
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null());
    git_cmd.args([
        "clone",
        "--depth",
        "1",
        url,
        target_dir.to_str().unwrap_or(""),
    ]);
    if let Ok(status) = git_cmd.status() {
        if status.success() {
            let _ = fs::remove_dir_all(target_dir.join(".git"));
            return Ok(());
        }
    }

    Err(ModuleError::InvalidManifest(format!(
        "Failed to download module from GitHub: {last_err}"
    )))
}

fn ingest_zip_url(url: &str, target_dir: &Path) -> Result<(), ModuleError> {
    let bytes = fetch_url_bytes(url)
        .map_err(|e| ModuleError::InvalidManifest(format!("Failed to fetch zip URL: {e}")))?;
    extract_zip(&bytes, target_dir).map_err(|e| ModuleError::InvalidManifest(e.to_string()))?;
    Ok(())
}

fn ingest_zip_file(path: &Path, target_dir: &Path) -> Result<(), ModuleError> {
    let bytes = fs::read(path)
        .map_err(|e| ModuleError::InvalidManifest(format!("Failed to read zip file: {e}")))?;
    extract_zip(&bytes, target_dir).map_err(|e| ModuleError::InvalidManifest(e.to_string()))?;
    Ok(())
}

const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

fn fetch_url_bytes(url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .new_agent();

    let resp = agent
        .get(url)
        .header("User-Agent", "toolcase-gateway/0.1.0")
        .call()
        .map_err(|e| format!("HTTP request error: {e}"))?;

    let mut bytes = Vec::new();
    let reader = resp.into_body().into_reader();
    let mut limited = std::io::Read::take(reader, MAX_ARCHIVE_BYTES + 1);
    std::io::Read::read_to_end(&mut limited, &mut bytes)
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err("Downloaded archive exceeds maximum permitted size (64MB)".into());
    }
    Ok(bytes)
}

fn unwrap_single_top_level_directory(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut dirs = Vec::new();
    let mut has_files = false;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "__MACOSX" {
            if name == "__MACOSX" {
                let _ = fs::remove_dir_all(entry.path());
            }
            continue;
        }
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => dirs.push(entry.path()),
            Ok(ft) if ft.is_file() => has_files = true,
            _ => {}
        }
    }
    if !has_files && dirs.len() == 1 {
        let single_dir = &dirs[0];
        let temp_stage = dir.join(format!(".tmp_unwrap_{}", uuid::Uuid::new_v4()));
        if fs::rename(single_dir, &temp_stage).is_ok() {
            if let Ok(re_read) = fs::read_dir(&temp_stage) {
                for item in re_read.flatten() {
                    let target = dir.join(item.file_name());
                    let _ = fs::rename(item.path(), target);
                }
            }
            let _ = fs::remove_dir_all(&temp_stage);
        }
    }
}

fn discover_or_synthesize_manifest(
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
        version: "0.1.0".to_string(),
        namespace,
        entrypoint,
        protocol,
        capabilities: vec!["chat_completions".to_string(), "models".to_string()],
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
        manifest.id = id.clone();
    }
    if let Some(ref name) = options.name {
        manifest.name = name.clone();
    }
    if let Some(ref ns) = options.namespace {
        manifest.namespace = ns.clone();
    }
    if let Some(ref ep) = options.entrypoint {
        manifest.entrypoint = ep.clone();
    }
    if let Some(ref proto) = options.protocol {
        manifest.protocol = proto.clone();
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

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Resolves a string parameter into an `InstallSource`.
pub fn resolve_source(raw: &str) -> InstallSource {
    let trimmed = raw.trim();
    if trimmed.starts_with("https://github.com/") || trimmed.starts_with("http://github.com/") {
        InstallSource::GitHubUrl(trimmed.to_string())
    } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        InstallSource::ZipUrl(trimmed.to_string())
    } else {
        let p = PathBuf::from(trimmed);
        if p.is_file() {
            InstallSource::ZipFile(p)
        } else {
            InstallSource::LocalDir(p)
        }
    }
}

/// Decodes a base64 encoded byte sequence.
pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    while clean.len() % 4 != 0 {
        clean.push(b'=');
    }
    let mut out = Vec::with_capacity((clean.len() / 4) * 3);
    for chunk in clean.chunks_exact(4) {
        let mut b = [0u32; 4];
        let mut pad = 0;
        for (i, &byte) in chunk.iter().enumerate() {
            if byte == b'=' {
                b[i] = 0;
                pad += 1;
            } else if let Some(pos) = TABLE.iter().position(|&t| t == byte) {
                b[i] = pos as u32;
            } else {
                return Err(format!("Invalid base64 character: {}", byte as char));
            }
        }
        let triple = (b[0] << 18) | (b[1] << 12) | (b[2] << 6) | b[3];
        out.push(((triple >> 16) & 0xFF) as u8);
        if pad < 2 {
            out.push(((triple >> 8) & 0xFF) as u8);
        }
        if pad < 1 {
            out.push((triple & 0xFF) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_decode_valid() {
        let input = "SGVsbG8gV29ybGQ=";
        let decoded = decode_base64(input).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "Hello World");
    }

    #[test]
    fn test_resolve_source_github_and_zip() {
        match resolve_source("https://github.com/my-org/my-module") {
            InstallSource::GitHubUrl(u) => assert!(u.contains("my-org/my-module")),
            _ => panic!("Expected GitHubUrl"),
        }

        match resolve_source("https://example.com/archive.zip") {
            InstallSource::ZipUrl(u) => assert!(u.contains("archive.zip")),
            _ => panic!("Expected ZipUrl"),
        }
    }

    #[test]
    fn test_install_module_from_local_dir_with_python_script() {
        let temp_state = std::env::temp_dir().join(format!("gw_state_{}", uuid::Uuid::new_v4()));
        let source_dir = std::env::temp_dir().join(format!("src_mod_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&source_dir).unwrap();

        let py_script = source_dir.join("main.py");
        fs::write(&py_script, "print('{\"status\": 200}')").unwrap();

        let options = InstallOptions {
            id: Some("test-py-module".to_string()),
            name: Some("Test Py Module".to_string()),
            namespace: Some("/custom/py".to_string()),
            ..Default::default()
        };

        let manifest = install_module(
            InstallSource::LocalDir(source_dir.clone()),
            options,
            &temp_state,
        )
        .expect("install succeeds");

        assert_eq!(manifest.id, "test-py-module");
        assert_eq!(manifest.namespace, "/custom/py");
        assert!(manifest.entrypoint.ends_with("run.sh"));
        assert!(Path::new(&manifest.entrypoint).is_file());

        cleanup_module_files("test-py-module", &temp_state);
        assert!(!temp_state.join("modules").join("test-py-module").exists());

        let _ = fs::remove_dir_all(&source_dir);
        let _ = fs::remove_dir_all(&temp_state);
    }

    #[test]
    fn test_install_module_local_http_preserves_url_entrypoint() {
        let temp_state = std::env::temp_dir().join(format!("gw_state_{}", uuid::Uuid::new_v4()));
        let source_dir = std::env::temp_dir().join(format!("src_mod_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&source_dir).unwrap();

        let options = InstallOptions {
            id: Some("test-http-module".to_string()),
            name: Some("Test HTTP Module".to_string()),
            namespace: Some("/custom/http".to_string()),
            entrypoint: Some("http://127.0.0.1:8080".to_string()),
            protocol: Some("local-http".to_string()),
            ..Default::default()
        };

        let manifest = install_module(
            InstallSource::LocalDir(source_dir.clone()),
            options,
            &temp_state,
        )
        .expect("install succeeds");

        assert_eq!(manifest.id, "test-http-module");
        assert_eq!(manifest.entrypoint, "http://127.0.0.1:8080");
        assert_eq!(manifest.protocol, "local-http");

        let mod_dir = temp_state.join("modules").join("test-http-module");
        let loaded =
            read_module_manifest(&mod_dir.join("module.json"), &mod_dir).expect("reads manifest");
        assert_eq!(loaded.entrypoint, "http://127.0.0.1:8080");

        cleanup_module_files("test-http-module", &temp_state);
        let _ = fs::remove_dir_all(&source_dir);
        let _ = fs::remove_dir_all(&temp_state);
    }
}
