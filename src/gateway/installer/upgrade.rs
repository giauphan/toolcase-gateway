//! Module upgrade and atomic replacement logic.
//!
//! Upgrades an existing module in place, preserving existing configuration,
//! secrets, and custom settings while safely rolling back on failure.

use super::ingest::{
    copy_dir_all, ingest_github, ingest_zip_file, ingest_zip_url, unwrap_single_top_level_directory,
};
use super::manifest::discover_or_synthesize_manifest;
use super::{InstallOptions, InstallSource};
use crate::gateway::archive::extract_zip;
use crate::gateway::modules::{ModuleError, PersonalManifest};
use std::fs;
use std::path::{Path, PathBuf};

/// Upgrades an installed module from a supported source into `$GW_STATE_DIR/modules/<id>`.
///
/// Preserves existing `config.json` and persistent data files, and performs
/// an atomic filesystem replacement with automated rollback on failure.
pub fn upgrade_module(
    module_id: &str,
    source: InstallSource,
    mut options: InstallOptions,
    state_dir: &Path,
) -> Result<PersonalManifest, ModuleError> {
    let clean_id = module_id.trim();
    if clean_id.is_empty()
        || !clean_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ModuleError::InvalidId(clean_id.to_string()));
    }
    options.id = Some(clean_id.to_string());

    let modules_root = state_dir.join("modules");
    fs::create_dir_all(&modules_root)
        .map_err(|e| ModuleError::InvalidManifest(format!("Failed to create modules root: {e}")))?;

    let target_dir = modules_root.join(clean_id);
    let existing_dir = find_existing_module_dir(clean_id, &target_dir);
    if existing_dir.is_none() {
        return Err(ModuleError::NotFound(format!(
            "Module '{clean_id}' is not currently installed; cannot upgrade"
        )));
    }

    // Create temporary staging directory for the new code
    let staging_id = format!("stage_upg_{}", uuid::Uuid::new_v4());
    let staging_dir = modules_root.join(&staging_id);
    fs::create_dir_all(&staging_dir)
        .map_err(|e| ModuleError::InvalidManifest(format!("Failed to create staging dir: {e}")))?;

    let source_str = extract_source_url(&source);

    // Ingest source files into staging_dir
    let ingest_result = match source {
        InstallSource::GitHubUrl(ref url) => {
            ingest_github(url, options.branch.as_deref(), &staging_dir)
        }
        InstallSource::ZipUrl(ref url) => ingest_zip_url(url, &staging_dir),
        InstallSource::ZipFile(ref path) => ingest_zip_file(path, &staging_dir),
        InstallSource::ZipBytes(ref bytes) => extract_zip(bytes, &staging_dir)
            .map(|_| ())
            .map_err(|e| ModuleError::InvalidManifest(e.to_string())),
        InstallSource::LocalDir(ref dir) => copy_dir_all(dir, &staging_dir)
            .map_err(|e| ModuleError::InvalidManifest(format!("Failed to copy directory: {e}"))),
    };

    if let Err(e) = ingest_result {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(e);
    }

    unwrap_single_top_level_directory(&staging_dir);

    // Discover or synthesize the updated manifest
    let mut manifest = match discover_or_synthesize_manifest(&staging_dir, &options) {
        Ok(m) => m,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(e);
        }
    };

    manifest.id = clean_id.to_string();
    if let Some(src) = source_str {
        manifest.source = Some(src);
    }

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

    // Preserve existing config.json and custom configuration files
    if let Some(ref prev_dir) = existing_dir {
        preserve_module_configuration(prev_dir, &staging_dir);
    }

    // Atomic directory replacement with rollback
    perform_atomic_directory_swap(&staging_dir, &target_dir, &modules_root)?;

    // Resolve entrypoint and set executable permissions
    finalize_module_entrypoint(&mut manifest, &target_dir)?;

    // Persist updated module.json into target directory
    let manifest_path = target_dir.join("module.json");
    if let Ok(json) = serde_json::to_string_pretty(&manifest) {
        let _ = fs::write(manifest_path, json);
    }

    Ok(manifest)
}

fn find_existing_module_dir(clean_id: &str, target_dir: &Path) -> Option<PathBuf> {
    if target_dir.is_dir() {
        return Some(target_dir.to_path_buf());
    }
    let addons_dir = Path::new("addons").join(clean_id);
    if addons_dir.is_dir() {
        return Some(addons_dir);
    }
    let modules_dir = Path::new("modules").join(clean_id);
    if modules_dir.is_dir() {
        return Some(modules_dir);
    }
    None
}

fn extract_source_url(source: &InstallSource) -> Option<String> {
    match source {
        InstallSource::GitHubUrl(u) | InstallSource::ZipUrl(u) => Some(u.clone()),
        InstallSource::LocalDir(p) | InstallSource::ZipFile(p) => {
            Some(p.to_string_lossy().to_string())
        }
        InstallSource::ZipBytes(_) => None,
    }
}

/// Preserves runtime `config.json` and persistent data files from the previous module directory.
fn preserve_module_configuration(prev_dir: &Path, staging_dir: &Path) {
    let prev_config = prev_dir.join("config.json");
    if prev_config.is_file() {
        let _ = fs::copy(&prev_config, staging_dir.join("config.json"));
    }

    let Ok(entries) = fs::read_dir(prev_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Preserve secrets, environment files, and local sqlite databases
        let is_user_data = name_str == ".env"
            || name_str.ends_with(".db")
            || name_str.ends_with(".sqlite")
            || name_str.ends_with(".sqlite3")
            || name_str == "data";

        if is_user_data {
            let dest = staging_dir.join(&name);
            if !dest.exists() {
                if path.is_dir() {
                    let _ = copy_dir_all(&path, &dest);
                } else if path.is_file() {
                    let _ = fs::copy(&path, &dest);
                }
            }
        }
    }
}

/// Atomically replaces `target_dir` with `staging_dir`.
fn perform_atomic_directory_swap(
    staging_dir: &Path,
    target_dir: &Path,
    modules_root: &Path,
) -> Result<(), ModuleError> {
    if target_dir.exists() {
        let backup_id = format!("backup_upg_{}", uuid::Uuid::new_v4());
        let backup_dir = modules_root.join(&backup_id);

        fs::rename(target_dir, &backup_dir).map_err(|e| {
            let _ = fs::remove_dir_all(staging_dir);
            ModuleError::InvalidManifest(format!("Failed to backup current module directory: {e}"))
        })?;

        if let Err(e) = fs::rename(staging_dir, target_dir) {
            let _ = fs::rename(&backup_dir, target_dir);
            let _ = fs::remove_dir_all(staging_dir);
            return Err(ModuleError::InvalidManifest(format!(
                "Failed to replace module directory (rolled back): {e}"
            )));
        }

        let _ = fs::remove_dir_all(&backup_dir);
    } else if let Err(e) = fs::rename(staging_dir, target_dir) {
        let _ = fs::remove_dir_all(staging_dir);
        return Err(ModuleError::InvalidManifest(format!(
            "Failed to finalize module directory: {e}"
        )));
    }
    Ok(())
}

fn finalize_module_entrypoint(
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::installer::install_module;

    #[test]
    fn test_upgrade_module_updates_version_and_preserves_config() {
        let temp_state =
            std::env::temp_dir().join(format!("gw_upg_state_{}", uuid::Uuid::new_v4()));
        let v1_dir = std::env::temp_dir().join(format!("v1_src_{}", uuid::Uuid::new_v4()));
        let v2_dir = std::env::temp_dir().join(format!("v2_src_{}", uuid::Uuid::new_v4()));

        fs::create_dir_all(&v1_dir).unwrap();
        fs::create_dir_all(&v2_dir).unwrap();

        // Create v1 module files
        let v1_manifest = serde_json::json!({
            "id": "test-upgrade-module",
            "name": "Upgrade Test Module v1",
            "version": "1.0.0",
            "namespace": "/custom/upg-test",
            "entrypoint": "run.sh",
            "protocol": "stdio-stream",
            "capabilities": ["chat"]
        });
        fs::write(v1_dir.join("module.json"), v1_manifest.to_string()).unwrap();
        fs::write(v1_dir.join("run.sh"), "#!/bin/sh\necho v1").unwrap();

        // Create v2 module files
        let v2_manifest = serde_json::json!({
            "id": "test-upgrade-module",
            "name": "Upgrade Test Module v2",
            "version": "2.1.0",
            "namespace": "/custom/upg-test",
            "entrypoint": "run.sh",
            "protocol": "stdio-stream",
            "capabilities": ["chat", "video"]
        });
        fs::write(v2_dir.join("module.json"), v2_manifest.to_string()).unwrap();
        fs::write(v2_dir.join("run.sh"), "#!/bin/sh\necho v2").unwrap();

        // 1. Install v1
        let opts = InstallOptions {
            id: Some("test-upgrade-module".to_string()),
            ..Default::default()
        };
        let m1 = install_module(
            InstallSource::LocalDir(v1_dir.clone()),
            opts.clone(),
            &temp_state,
        )
        .expect("install v1 succeeds");
        assert_eq!(m1.version, "1.0.0");

        // 2. Add user configuration and credentials into module directory
        let target_dir = temp_state.join("modules").join("test-upgrade-module");
        let user_config = serde_json::json!({
            "api_key": "sk-super-secret-12345",
            "base_url": "https://api.example.com",
            "timeout_secs": 60
        });
        fs::write(
            target_dir.join("config.json"),
            serde_json::to_string_pretty(&user_config).unwrap(),
        )
        .unwrap();
        fs::write(target_dir.join(".env"), "CUSTOM_ENV_TOKEN=val_token").unwrap();

        // 3. Perform upgrade to v2
        let upgraded = upgrade_module(
            "test-upgrade-module",
            InstallSource::LocalDir(v2_dir.clone()),
            opts,
            &temp_state,
        )
        .expect("upgrade succeeds");

        assert_eq!(upgraded.version, "2.1.0");
        assert_eq!(upgraded.name, "Upgrade Test Module v2");
        assert!(upgraded.capabilities.contains(&"video".to_string()));

        // 4. Verify config.json and .env were strictly preserved
        let config_str = fs::read_to_string(target_dir.join("config.json")).unwrap();
        let parsed_cfg: serde_json::Value = serde_json::from_str(&config_str).unwrap();
        assert_eq!(
            parsed_cfg.get("api_key").and_then(|v| v.as_str()),
            Some("sk-super-secret-12345")
        );
        assert_eq!(
            parsed_cfg.get("timeout_secs").and_then(|v| v.as_i64()),
            Some(60)
        );

        let env_str = fs::read_to_string(target_dir.join(".env")).unwrap();
        assert_eq!(env_str, "CUSTOM_ENV_TOKEN=val_token");

        // Cleanup
        let _ = fs::remove_dir_all(&v1_dir);
        let _ = fs::remove_dir_all(&v2_dir);
        let _ = fs::remove_dir_all(&temp_state);
    }

    #[test]
    fn test_upgrade_module_atomic_rollback_on_failure() {
        let temp_state =
            std::env::temp_dir().join(format!("gw_upg_state_{}", uuid::Uuid::new_v4()));
        let v1_dir = std::env::temp_dir().join(format!("v1_src_{}", uuid::Uuid::new_v4()));
        let bad_v2_dir = std::env::temp_dir().join(format!("bad_v2_{}", uuid::Uuid::new_v4()));

        fs::create_dir_all(&v1_dir).unwrap();
        fs::create_dir_all(&bad_v2_dir).unwrap();

        fs::write(
            v1_dir.join("module.json"),
            serde_json::json!({
                "id": "mod-rollback",
                "name": "Rollback Test",
                "version": "1.0.0",
                "namespace": "/custom/rollback",
                "entrypoint": "run.sh",
                "protocol": "stdio-stream",
                "capabilities": ["chat"]
            })
            .to_string(),
        )
        .unwrap();
        fs::write(v1_dir.join("run.sh"), "#!/bin/sh\necho original").unwrap();

        let opts = InstallOptions {
            id: Some("mod-rollback".to_string()),
            ..Default::default()
        };
        install_module(
            InstallSource::LocalDir(v1_dir.clone()),
            opts.clone(),
            &temp_state,
        )
        .expect("install succeeds");

        let target_dir = temp_state.join("modules").join("mod-rollback");
        fs::write(target_dir.join("config.json"), r#"{"secret": "keep_me"}"#).unwrap();

        // Bad v2 has invalid namespace with double slashes
        fs::write(
            bad_v2_dir.join("module.json"),
            serde_json::json!({
                "id": "mod-rollback",
                "name": "Rollback Test",
                "version": "2.0.0",
                "namespace": "/custom//invalid",
                "entrypoint": "run.sh",
                "protocol": "stdio-stream"
            })
            .to_string(),
        )
        .unwrap();

        let res = upgrade_module(
            "mod-rollback",
            InstallSource::LocalDir(bad_v2_dir.clone()),
            opts,
            &temp_state,
        );
        assert!(res.is_err());

        // Assert v1 module directory is still intact and preserved
        assert!(target_dir.is_dir());
        let config_str = fs::read_to_string(target_dir.join("config.json")).unwrap();
        assert!(config_str.contains("keep_me"));

        let _ = fs::remove_dir_all(&v1_dir);
        let _ = fs::remove_dir_all(&bad_v2_dir);
        let _ = fs::remove_dir_all(&temp_state);
    }

    #[test]
    fn test_upgrade_module_not_found() {
        let temp_state =
            std::env::temp_dir().join(format!("gw_upg_state_{}", uuid::Uuid::new_v4()));
        let dummy_dir = std::env::temp_dir().join(format!("dummy_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dummy_dir).unwrap();

        let opts = InstallOptions::default();
        let res = upgrade_module(
            "non-existent-module",
            InstallSource::LocalDir(dummy_dir.clone()),
            opts,
            &temp_state,
        );
        assert!(matches!(res, Err(ModuleError::NotFound(_))));

        let _ = fs::remove_dir_all(&dummy_dir);
        let _ = fs::remove_dir_all(&temp_state);
    }
}
