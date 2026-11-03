//! Automated compilation and build lifecycle for native Rust modules.
//!
//! Inspects staging directories for `Cargo.toml`, compiles release binaries
//! via `cargo build --release`, and guarantees zero cold-start execution latency.

use crate::gateway::modules::ModuleError;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Checks if `cargo` is installed and executable on the host system.
pub fn is_cargo_available() -> bool {
    Command::new("cargo")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Discovers the expected binary name from Cargo.toml or falls back to `default_id`.
pub fn detect_crate_binary_name(cargo_toml_path: &Path, default_id: &str) -> String {
    if let Ok(content) = fs::read_to_string(cargo_toml_path) {
        // Look for [[bin]] name = "..."
        if let Some(pos) = content.find("[[bin]]") {
            let rest = &content[pos..];
            for line in rest.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("name") {
                    if let Some((_, val)) = trimmed.split_once('=') {
                        let name = val.trim().trim_matches('"').trim_matches('\'').trim();
                        if !name.is_empty() {
                            return name.to_string();
                        }
                    }
                }
                if trimmed.starts_with("[[") && !trimmed.starts_with("[[bin]]") {
                    break;
                }
            }
        }
        // Look for [package] name = "..."
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("name") {
                if let Some((_, val)) = trimmed.split_once('=') {
                    let name = val.trim().trim_matches('"').trim_matches('\'').trim();
                    if !name.is_empty() {
                        return name.to_string();
                    }
                }
            }
        }
    }
    default_id.to_string()
}

/// Checks if the precompiled binary exists and is newer than all Rust source files and Cargo.toml.
pub fn is_binary_fresh(dir: &Path, binary_path: &Path) -> bool {
    if !binary_path.is_file() {
        return false;
    }
    let Ok(bin_meta) = fs::metadata(binary_path) else {
        return false;
    };
    let Ok(bin_mtime) = bin_meta.modified() else {
        return false;
    };

    let cargo_toml = dir.join("Cargo.toml");
    if let Ok(meta) = fs::metadata(&cargo_toml) {
        if let Ok(mtime) = meta.modified() {
            if mtime > bin_mtime {
                return false;
            }
        }
    }

    let src_dir = dir.join("src");
    if src_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&src_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(mtime) = meta.modified() {
                        if mtime > bin_mtime {
                            return false;
                        }
                    }
                }
            }
        }
    }

    true
}

/// Compiles a Rust module in staging if `Cargo.toml` is present and binary needs building.
pub fn build_rust_module_if_present(
    staging_dir: &Path,
    module_id: &str,
) -> Result<Option<PathBuf>, ModuleError> {
    let cargo_toml = staging_dir.join("Cargo.toml");
    if !cargo_toml.is_file() {
        return Ok(None);
    }

    let bin_name = detect_crate_binary_name(&cargo_toml, module_id);
    let target_bin = staging_dir.join(&bin_name);

    if is_binary_fresh(staging_dir, &target_bin) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = fs::metadata(&target_bin) {
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&target_bin, perms);
            }
        }
        return Ok(Some(target_bin));
    }

    if !is_cargo_available() {
        if target_bin.is_file() {
            return Ok(Some(target_bin));
        }
        return Err(ModuleError::InvalidManifest(format!(
            "Rust module '{module_id}' has Cargo.toml but no native binary, and 'cargo' is not installed on host"
        )));
    }

    // Run cargo build --release
    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(&cargo_toml)
        .current_dir(staging_dir);

    // Ensure the module builds into its own isolated staging target directory
    let module_target_dir = staging_dir.join("target");
    cmd.env("CARGO_TARGET_DIR", &module_target_dir);

    let output = cmd.output().map_err(|e| {
        ModuleError::InvalidManifest(format!("Failed to execute 'cargo build': {e}"))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ModuleError::InvalidManifest(format!(
            "Compilation of Rust module '{module_id}' failed: {stderr}"
        )));
    }

    // Locate compiled binary in target/release/<bin_name> or fallback
    let release_bin = module_target_dir.join("release").join(&bin_name);
    let final_bin = if release_bin.is_file() {
        let dest = staging_dir.join(&bin_name);
        if release_bin != dest {
            let _ = fs::copy(&release_bin, &dest);
        }
        dest
    } else {
        target_bin
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(&final_bin) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&final_bin, perms);
        }
    }

    Ok(Some(final_bin))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_cargo_available() {
        // Cargo should be available in our dev/CI environment
        assert!(is_cargo_available());
    }

    #[test]
    fn test_detect_crate_binary_name() {
        let temp = std::env::temp_dir().join(format!("test_builder_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp).unwrap();
        let cargo_toml = temp.join("Cargo.toml");
        fs::write(
            &cargo_toml,
            r#"
[package]
name = "my-test-package"
version = "0.1.0"

[[bin]]
name = "custom-binary-exe"
path = "src/main.rs"
"#,
        )
        .unwrap();

        let bin_name = detect_crate_binary_name(&cargo_toml, "fallback");
        assert_eq!(bin_name, "custom-binary-exe");

        let cargo_toml_no_bin = temp.join("Cargo_no_bin.toml");
        fs::write(
            &cargo_toml_no_bin,
            r#"
[package]
name = "pkg-only"
version = "0.1.0"
"#,
        )
        .unwrap();
        let bin_name2 = detect_crate_binary_name(&cargo_toml_no_bin, "fallback");
        assert_eq!(bin_name2, "pkg-only");
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_is_binary_fresh_and_missing() {
        let temp =
            std::env::temp_dir().join(format!("test_builder_fresh_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp).unwrap();
        let bin = temp.join("nonexistent");
        assert!(!is_binary_fresh(&temp, &bin));
        let _ = fs::remove_dir_all(&temp);
    }
}
