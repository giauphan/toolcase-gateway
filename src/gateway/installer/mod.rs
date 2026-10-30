//! Platform module installer and converter.
//!
//! Supports installing self-contained modules from GitHub repositories,
//! zip archives, or local directories with automated manifest discovery,
//! conversion, path sanitization, and execution wrapping.

mod ingest;
mod manifest;

pub use ingest::{decode_base64, resolve_source};

use crate::gateway::archive::extract_zip;
use crate::gateway::modules::{ModuleError, PersonalManifest};
use ingest::{
    copy_dir_all, ingest_github, ingest_zip_file, ingest_zip_url, unwrap_single_top_level_directory,
};
use manifest::{discover_or_synthesize_manifest, read_module_manifest};
use std::fs;
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
