//! Module upgrade and atomic replacement logic.
//!
//! Upgrades an existing module in place, preserving existing configuration,
//! secrets, and custom settings while safely rolling back on failure.

use super::ingest::{copy_dir_all, extract_source_url, ingest_source_into_staging};
use super::manifest::{discover_or_synthesize_manifest, finalize_module_entrypoint};
use super::{InstallOptions, InstallSource};
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
    registered_modules: Option<
        &std::collections::HashMap<String, crate::gateway::modules::ModuleDescriptor>,
    >,
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
    ingest_source_into_staging(&source, options.branch.as_deref(), &staging_dir)?;

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

    let empty_map = std::collections::HashMap::new();
    let mod_map = registered_modules.unwrap_or(&empty_map);
    let ns_trimmed = manifest.namespace.trim().trim_end_matches('/');
    if let Err(e) = crate::modules::validation::validate_module_namespace(
        ns_trimmed,
        &manifest.namespace,
        &manifest.id,
        mod_map,
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

    // Validate entrypoint and protocol in staging prior to atomic swap without corrupting manifest entrypoint
    let mut staged_manifest = manifest.clone();
    if let Err(e) = finalize_module_entrypoint(&mut staged_manifest, &staging_dir) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(e);
    }
    if let Err(e) = crate::modules::validation::validate_module_protocol(
        &staged_manifest.protocol,
        &staged_manifest.entrypoint,
    ) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(e);
    }

    // Preserve existing config.json and custom configuration files
    if let Some(ref prev_dir) = existing_dir {
        if let Err(e) = preserve_module_configuration(prev_dir, &staging_dir) {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(e);
        }
    }

    // Atomic directory replacement with rollback
    perform_atomic_directory_swap(&staging_dir, &target_dir, &modules_root)?;

    // Resolve finalized entrypoint path inside target_dir and set executable permissions
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

/// Preserves runtime `config.json` and persistent data files from the previous module directory.
fn preserve_module_configuration(prev_dir: &Path, staging_dir: &Path) -> Result<(), ModuleError> {
    let prev_config = prev_dir.join("config.json");
    if prev_config.is_file() {
        fs::copy(&prev_config, staging_dir.join("config.json")).map_err(|e| {
            ModuleError::InvalidManifest(format!("Failed to preserve config.json: {e}"))
        })?;
    }

    let entries = fs::read_dir(prev_dir).map_err(|e| {
        ModuleError::InvalidManifest(format!("Failed to read previous module directory: {e}"))
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Preserve secrets, environment files, and local sqlite databases (including WAL sidecars)
        let is_user_data = name_str == ".env"
            || name_str.ends_with(".db")
            || name_str.ends_with(".db-wal")
            || name_str.ends_with(".db-shm")
            || name_str.ends_with(".sqlite")
            || name_str.ends_with(".sqlite-wal")
            || name_str.ends_with(".sqlite-shm")
            || name_str.ends_with(".sqlite3")
            || name_str.ends_with(".sqlite3-wal")
            || name_str.ends_with(".sqlite3-shm")
            || name_str == "data";

        if is_user_data {
            let dest = staging_dir.join(&name);
            if path.is_dir() {
                copy_dir_all(&path, &dest).map_err(|e| {
                    ModuleError::InvalidManifest(format!(
                        "Failed to preserve directory {}: {e}",
                        name_str
                    ))
                })?;
            } else if path.is_file() {
                fs::copy(&path, &dest).map_err(|e| {
                    ModuleError::InvalidManifest(format!(
                        "Failed to preserve file {}: {e}",
                        name_str
                    ))
                })?;
            }
        }
    }
    Ok(())
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
            None,
        )
        .expect("upgrade succeeds");

        assert_eq!(upgraded.version, "2.1.0");
        assert_eq!(upgraded.name, "Upgrade Test Module v2");
        assert!(upgraded.capabilities.contains(&"video".to_string()));
        assert!(upgraded.entrypoint.contains("test-upgrade-module"));
        assert!(!upgraded.entrypoint.contains("stage_upg_"));

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
            None,
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
            None,
        );
        assert!(matches!(res, Err(ModuleError::NotFound(_))));

        let _ = fs::remove_dir_all(&dummy_dir);
        let _ = fs::remove_dir_all(&temp_state);
    }
}
