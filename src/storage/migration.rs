//! Clean Architecture migration service: imports legacy JSON configurations into `StorageRepository`.

use super::{StorageError, StorageRepository, StoredAccount};
use crate::gateway::modules::PersonalManifest;
use std::fs;
use std::path::Path;

/// Automatically migrates accounts, modules, and settings from legacy JSON files into the repository on startup.
pub fn migrate_legacy_json_files(
    storage: &dyn StorageRepository,
    state_dir: &Path,
) -> Result<(), StorageError> {
    migrate_legacy_accounts(storage, state_dir)?;
    migrate_legacy_modules(storage, state_dir)?;
    migrate_legacy_setting(storage, state_dir, "gateway_config.json", "gateway_config")?;
    migrate_legacy_setting(storage, state_dir, "muse_config.json", "muse_config")?;
    Ok(())
}

/// Migrates personal module configurations from legacy `gateway_config.json` into the repository.
pub fn migrate_legacy_modules(
    storage: &dyn StorageRepository,
    state_dir: &Path,
) -> Result<(), StorageError> {
    let config_path = state_dir.join("gateway_config.json");
    if !config_path.is_file() {
        return Ok(());
    }

    let existing = storage.get_stored_modules()?;
    let Ok(content) = fs::read_to_string(&config_path) else {
        return Ok(());
    };

    let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Ok(());
    };

    let enabled_map: std::collections::HashMap<String, bool> = val
        .get("enabled_modules")
        .and_then(|m| serde_json::from_value(m.clone()).ok())
        .unwrap_or_default();

    if let Some(personal_arr) = val.get("personal_modules").and_then(|v| v.as_array()) {
        for item in personal_arr {
            if let Ok(manifest) = serde_json::from_value::<PersonalManifest>(item.clone()) {
                let already_in_db = existing.iter().any(|m| m.manifest.id == manifest.id);
                if !already_in_db {
                    let is_enabled = enabled_map.get(&manifest.id).copied().unwrap_or(true);
                    if let Err(e) = storage.save_module_with_status(&manifest, is_enabled) {
                        eprintln!(
                            "warning: failed to save module '{}' during legacy migration: {e}",
                            manifest.id
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

/// Migrates accounts from legacy `accounts.json` into the repository.
pub fn migrate_legacy_accounts(
    storage: &dyn StorageRepository,
    state_dir: &Path,
) -> Result<(), StorageError> {
    let accounts_file = state_dir.join("accounts.json");
    if !accounts_file.is_file() || !storage.get_accounts()?.is_empty() {
        return Ok(());
    }
    let Ok(content) = fs::read_to_string(&accounts_file) else {
        return Ok(());
    };
    let to_migrate = parse_legacy_accounts(&content);
    if !to_migrate.is_empty() {
        let _ = storage.save_accounts(&to_migrate);
    }
    Ok(())
}

/// Migrates a single legacy configuration JSON file into a setting key.
pub fn migrate_legacy_setting(
    storage: &dyn StorageRepository,
    state_dir: &Path,
    file_name: &str,
    setting_key: &str,
) -> Result<(), StorageError> {
    let file_path = state_dir.join(file_name);
    if file_path.is_file() && storage.get_setting(setting_key)?.is_none() {
        if let Ok(content) = fs::read_to_string(&file_path) {
            let _ = storage.set_setting(setting_key, &content);
        }
    }
    Ok(())
}

/// Parses accounts array from legacy JSON file content.
pub fn parse_legacy_accounts(content: &str) -> Vec<StoredAccount> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let Some(arr) = val.get("accounts").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    arr.iter()
        .enumerate()
        .filter_map(|(idx, item)| {
            let key = item.get("key").and_then(|v| v.as_str())?;
            if key.is_empty() {
                return None;
            }
            Some(StoredAccount {
                id: item
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("acc_{idx}")),
                provider: item
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or("openrouter")
                    .to_string(),
                key: key.to_string(),
                label: item
                    .get("label")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                is_active: true,
            })
        })
        .collect()
}
