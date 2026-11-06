//! Command construction, environment isolation, and interpreter fallback for personal modules.

use crate::gateway::modules::PersonalManifest;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

const ALLOWED_ENV_VARS: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL", "TMPDIR", "TZ", "USER"];

fn heal_legacy_adapter_script(path: &Path) {
    if let Ok(content) = std::fs::read_to_string(path) {
        if content.contains("cat << 'EOF' | python3") {
            if let Some(parent) = path.parent() {
                let adapter_py = parent.join("adapter.py");
                if let Some(py_code) = content
                    .split("cat << 'EOF' | python3")
                    .nth(1)
                    .and_then(|s| s.split_once('\n'))
                    .and_then(|(_, rest)| rest.split("\nEOF").next())
                {
                    let _ = std::fs::write(&adapter_py, py_code);
                    let fixed_run_sh = r#"#!/bin/sh
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec python3 "$SCRIPT_DIR/adapter.py" "$@"
"#;
                    let _ = std::fs::write(path, fixed_run_sh);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ =
                            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
                        let _ = std::fs::set_permissions(
                            &adapter_py,
                            std::fs::Permissions::from_mode(0o700),
                        );
                    }
                }
            }
        }
    }
}

fn resolve_entrypoint_path(entrypoint: &str) -> std::path::PathBuf {
    let entry_path = Path::new(entrypoint);
    if entry_path.is_file() {
        entry_path
            .canonicalize()
            .unwrap_or_else(|_| entry_path.to_path_buf())
    } else {
        entry_path.to_path_buf()
    }
}

fn configure_module_command(
    cmd: &mut Command,
    work_dir: Option<&Path>,
    manifest: &PersonalManifest,
    sub_provider_id: Option<&str>,
) {
    if let Some(parent) = work_dir {
        if parent.is_dir() {
            cmd.current_dir(parent);
        }
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Isolate secrets by only inheriting safe runtime environment variables
    cmd.env_clear();
    for key in ALLOWED_ENV_VARS {
        if let Ok(val) = std::env::var(key) {
            cmd.env(key, val);
        }
    }
    cmd.env("MODULE_ID", &manifest.id);
    cmd.env("MODULE_NAME", &manifest.name);
    cmd.env("MODULE_NAMESPACE", &manifest.namespace);
    if let Some(sub_id) = sub_provider_id {
        cmd.env("MODULE_SUB_PROVIDER_ID", sub_id);
    }
}

pub(crate) fn build_command(manifest: &PersonalManifest, sub_provider_id: Option<&str>) -> Command {
    let entry_path = Path::new(&manifest.entrypoint);
    let resolved_entrypoint = if entry_path.is_file() {
        heal_legacy_adapter_script(entry_path);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = entry_path.metadata() {
                let mode = metadata.permissions().mode();
                if mode & 0o111 == 0 {
                    let _ = std::fs::set_permissions(
                        entry_path,
                        std::fs::Permissions::from_mode(mode | 0o700),
                    );
                }
            }
        }
        entry_path
            .canonicalize()
            .unwrap_or_else(|_| entry_path.to_path_buf())
    } else {
        entry_path.to_path_buf()
    };

    let mut cmd = Command::new(&resolved_entrypoint);
    configure_module_command(
        &mut cmd,
        resolved_entrypoint.parent(),
        manifest,
        sub_provider_id,
    );
    cmd
}

/// Builds a fallback command using the appropriate system interpreter when direct execution fails
/// with permission errors (e.g. execution bit missing on disk or noexec mount).
pub(crate) fn build_fallback_interpreter_command(
    manifest: &PersonalManifest,
    sub_provider_id: Option<&str>,
) -> Option<Command> {
    let resolved = resolve_entrypoint_path(&manifest.entrypoint);

    if !resolved.is_file() {
        return None;
    }

    let ext = resolved
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("");
    let interpreter = if ext == "sh" {
        Some("/bin/sh")
    } else if ext == "py" {
        Some("python3")
    } else if ext == "js" {
        Some("node")
    } else {
        // Inspect shebang header
        std::fs::File::open(&resolved).ok().and_then(|file| {
            let mut reader = BufReader::new(file);
            let mut first_line = String::new();
            if reader.read_line(&mut first_line).is_ok() && first_line.starts_with("#!") {
                let shebang = first_line.trim_start_matches("#!").trim();
                if shebang.contains("python") {
                    Some("python3")
                } else if shebang.contains("node") {
                    Some("node")
                } else if shebang.contains("sh") || shebang.contains("bash") {
                    Some("/bin/sh")
                } else {
                    None
                }
            } else {
                None
            }
        })
    }?;

    let mut cmd = Command::new(interpreter);
    cmd.arg(&resolved);
    configure_module_command(&mut cmd, resolved.parent(), manifest, sub_provider_id);
    Some(cmd)
}
