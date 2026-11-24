//! Pre-flight validation and configuration discovery for module packages.
//!
//! Inspects module sources (GitHub URLs, ZIP archives, or local directories)
//! non-destructively in staging, verifies manifest invariants, detects entrypoint
//! types, checks for routing conflicts, and synthesizes complete descriptors.

use super::ingest::{extract_source_url, ingest_source_into_staging};
use super::manifest::discover_or_synthesize_manifest;
use super::{InstallOptions, InstallSource};
use crate::gateway::modules::{
    validate_module_id, validate_module_namespace, validate_module_protocol,
    validate_sub_providers, ModuleDescriptor, ModuleError, PersonalManifest,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// RAII guard to guarantee cleanup of temporary validation staging directories.
struct StagingDirGuard<'a>(&'a Path);

impl<'a> Drop for StagingDirGuard<'a> {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = fs::remove_dir_all(self.0);
        }
    }
}

/// Detailed validation and configuration discovery report for a module candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleValidationReport {
    pub valid: bool,
    pub manifest: PersonalManifest,
    pub entrypoint_type: String,
    pub has_manifest_file: bool,
    pub is_update: bool,
    pub warnings: Vec<String>,
    pub diagnostics: Vec<String>,
}

fn detect_staging_entrypoint_type(staging_dir: &Path, manifest: &PersonalManifest) -> String {
    if manifest.protocol == "local-http"
        || manifest.entrypoint.starts_with("http://")
        || manifest.entrypoint.starts_with("https://")
    {
        return "local-http".to_string();
    }
    if staging_dir.join("Cargo.toml").is_file() {
        return "rust-crate".to_string();
    }
    let ep_filename = Path::new(&manifest.entrypoint)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");

    const PY_FILES: &[&str] = &["main.py", "app.py", "server.py", "adapter.py"];
    if ep_filename.ends_with(".py") || PY_FILES.iter().any(|f| staging_dir.join(f).is_file()) {
        return "python-script".to_string();
    }

    const JS_FILES: &[&str] = &["index.js", "main.js", "server.js", "app.js"];
    if ep_filename.ends_with(".js") || JS_FILES.iter().any(|f| staging_dir.join(f).is_file()) {
        return "node-script".to_string();
    }

    if ep_filename.ends_with(".sh") || staging_dir.join("run.sh").is_file() {
        return "shell-script".to_string();
    }
    "custom-binary".to_string()
}

fn check_manifest_file_exists(staging_dir: &Path) -> bool {
    const CANDIDATES: &[&str] = &[
        "module.json",
        "manifest.json",
        "__manifest__.json",
        "toolcase-module.json",
    ];
    CANDIDATES
        .iter()
        .any(|name| staging_dir.join(name).is_file())
}

/// Inspects and validates a module package from a supported source non-destructively.
pub fn validate_module_source(
    source: &InstallSource,
    options: &InstallOptions,
    state_dir: &Path,
    registered_modules: &HashMap<String, ModuleDescriptor>,
) -> Result<ModuleValidationReport, ModuleError> {
    let modules_root = state_dir.join("modules");
    fs::create_dir_all(&modules_root)
        .map_err(|e| ModuleError::InvalidManifest(format!("Failed to create modules root: {e}")))?;

    let staging_id = format!("stage_val_{}", uuid::Uuid::new_v4());
    let staging_dir = modules_root.join(&staging_id);
    fs::create_dir_all(&staging_dir)
        .map_err(|e| ModuleError::InvalidManifest(format!("Failed to create staging dir: {e}")))?;

    let _guard = StagingDirGuard(&staging_dir);

    // Ingest source into staging
    ingest_source_into_staging(source, options.branch.as_deref(), &staging_dir)?;
    let source_str = extract_source_url(source);

    let has_manifest_file = check_manifest_file_exists(&staging_dir);

    // Discover or synthesize the manifest
    let mut manifest = discover_or_synthesize_manifest(&staging_dir, options)?;
    if let Some(src) = source_str {
        if manifest.source.is_none() {
            manifest.source = Some(src);
        }
    }

    let mut warnings = Vec::new();
    let mut diagnostics = Vec::new();

    if has_manifest_file {
        diagnostics.push("Discovered declarative module.json manifest file in package".to_string());
    } else {
        warnings.push(
            "Package lacks module.json; configuration synthesized from folder structure and entrypoint"
                .to_string(),
        );
    }

    let entrypoint_type = detect_staging_entrypoint_type(&staging_dir, &manifest);
    diagnostics.push(format!("Detected entrypoint type: {entrypoint_type}"));

    // Check if this module is already installed
    let is_update = registered_modules.contains_key(&manifest.id);
    if is_update {
        warnings.push(format!(
            "Module '{}' is already registered; installing will replace existing code",
            manifest.id
        ));
    } else {
        validate_module_id(&manifest.id, registered_modules)?;
    }

    // Validate namespace
    let ns_trimmed = manifest.namespace.trim().trim_end_matches('/');
    validate_module_namespace(
        ns_trimmed,
        &manifest.namespace,
        &manifest.id,
        registered_modules,
    )?;
    manifest.namespace = ns_trimmed.to_string();

    // Validate protocol and entrypoint using staged resolution
    let mut staged_manifest = manifest.clone();
    super::manifest::finalize_module_entrypoint(&mut staged_manifest, &staging_dir)?;
    validate_module_protocol(&staged_manifest.protocol, &staged_manifest.entrypoint)?;

    // Validate sub-providers if grouped
    if let Some(ref grp) = manifest.group {
        validate_sub_providers(&grp.providers)?;
        diagnostics.push(format!(
            "Validated {} sub-providers in module group",
            grp.providers.len()
        ));
    }

    if manifest.capabilities.is_empty() {
        warnings.push(
            "No capabilities declared; module will not advertise specialized MCP tools".to_string(),
        );
    } else {
        diagnostics.push(format!(
            "Declared capabilities: {}",
            manifest.capabilities.join(", ")
        ));
    }

    Ok(ModuleValidationReport {
        valid: true,
        manifest,
        entrypoint_type,
        has_manifest_file,
        is_update,
        warnings,
        diagnostics,
    })
}

/// Validates a manifest descriptor directly without file ingestion.
pub fn validate_module_manifest_direct(
    manifest: &PersonalManifest,
    registered_modules: &HashMap<String, ModuleDescriptor>,
) -> Result<ModuleValidationReport, ModuleError> {
    let mut warnings = Vec::new();
    let mut diagnostics = Vec::new();

    let is_update = registered_modules.contains_key(&manifest.id);
    if is_update {
        warnings.push(format!(
            "Module '{}' is already registered; installing will replace existing configuration",
            manifest.id
        ));
    } else {
        validate_module_id(&manifest.id, registered_modules)?;
    }

    let ns_trimmed = manifest.namespace.trim().trim_end_matches('/');
    validate_module_namespace(
        ns_trimmed,
        &manifest.namespace,
        &manifest.id,
        registered_modules,
    )?;

    validate_module_protocol(&manifest.protocol, &manifest.entrypoint)?;

    if let Some(ref grp) = manifest.group {
        validate_sub_providers(&grp.providers)?;
        diagnostics.push(format!(
            "Validated {} sub-providers in module group",
            grp.providers.len()
        ));
    }

    let entrypoint_type = if manifest.protocol == "local-http"
        || manifest.entrypoint.starts_with("http://")
        || manifest.entrypoint.starts_with("https://")
    {
        "local-http".to_string()
    } else if manifest.entrypoint.ends_with(".sh") {
        "shell-script".to_string()
    } else if manifest.entrypoint.ends_with(".py") {
        "python-script".to_string()
    } else {
        "custom-binary".to_string()
    };

    Ok(ModuleValidationReport {
        valid: true,
        manifest: manifest.clone(),
        entrypoint_type,
        has_manifest_file: false,
        is_update,
        warnings,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_module_manifest_direct_valid() {
        let manifest = PersonalManifest {
            id: "test-valid-mod".to_string(),
            name: "Test Valid Mod".to_string(),
            short_name: None,
            version: "1.0.0".to_string(),
            namespace: "/custom/valid".to_string(),
            entrypoint: "/usr/local/bin/adapter.sh".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec!["chat_completions".to_string()],
            icon: None,
            config_schema: None,
            permissions: vec![],
            group: None,
            source: None,
            tools: None,
            doc_url: None,
            doc_content: None,
        };
        let registered = HashMap::new();
        let report = validate_module_manifest_direct(&manifest, &registered).expect("valid report");
        assert!(report.valid);
        assert_eq!(report.manifest.id, "test-valid-mod");
        assert_eq!(report.entrypoint_type, "shell-script");
        assert!(!report.is_update);
    }

    #[test]
    fn test_validate_module_manifest_direct_reserved_namespace() {
        let manifest = PersonalManifest {
            id: "test-bad-ns".to_string(),
            name: "Test Bad NS".to_string(),
            short_name: None,
            version: "1.0.0".to_string(),
            namespace: "/api/bad".to_string(),
            entrypoint: "/bin/sh".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec![],
            icon: None,
            config_schema: None,
            permissions: vec![],
            group: None,
            source: None,
            tools: None,
            doc_url: None,
            doc_content: None,
        };
        let registered = HashMap::new();
        let err = validate_module_manifest_direct(&manifest, &registered).unwrap_err();
        assert!(matches!(err, ModuleError::ReservedNamespace(_)));
    }

    #[test]
    fn test_validate_module_manifest_direct_invalid_id() {
        let manifest = PersonalManifest {
            id: "bad id with spaces".to_string(),
            name: "Bad ID".to_string(),
            short_name: None,
            version: "1.0.0".to_string(),
            namespace: "/custom/test".to_string(),
            entrypoint: "/bin/sh".to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec![],
            icon: None,
            config_schema: None,
            permissions: vec![],
            group: None,
            source: None,
            tools: None,
            doc_url: None,
            doc_content: None,
        };
        let registered = HashMap::new();
        let err = validate_module_manifest_direct(&manifest, &registered).unwrap_err();
        assert!(matches!(err, ModuleError::InvalidId(_)));
    }

    #[test]
    fn test_validate_module_source_local_dir_with_python_file() {
        let temp_state =
            std::env::temp_dir().join(format!("gw_val_state_{}", uuid::Uuid::new_v4()));
        let source_dir = std::env::temp_dir().join(format!("src_val_mod_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&source_dir).unwrap();

        fs::write(source_dir.join("main.py"), "print('hello')").unwrap();

        let options = InstallOptions {
            id: Some("val-py-mod".to_string()),
            namespace: Some("/custom/val-py".to_string()),
            ..Default::default()
        };
        let registered = HashMap::new();

        let report = validate_module_source(
            &InstallSource::LocalDir(source_dir.clone()),
            &options,
            &temp_state,
            &registered,
        )
        .expect("validation succeeds");

        assert!(report.valid);
        assert_eq!(report.manifest.id, "val-py-mod");
        assert_eq!(report.entrypoint_type, "python-script");
        assert!(!report.has_manifest_file);

        // Staging directory must be cleaned up
        let staging_pattern = temp_state.join("modules");
        if let Ok(entries) = fs::read_dir(staging_pattern) {
            let stage_count = entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("stage_val_"))
                .count();
            assert_eq!(stage_count, 0, "No staging directories should remain");
        }

        let _ = fs::remove_dir_all(&source_dir);
        let _ = fs::remove_dir_all(&temp_state);
    }
}
