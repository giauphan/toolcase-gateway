use std::fs;
use std::io;
use std::path::Path;

pub fn persist_muse_env(path: &Path, fields: &[(&str, &str)]) -> io::Result<()> {
    const MANAGED: [&str; 6] = [
        "GW_MUSEAI_WS_URL",
        "GW_MUSEAI_BASE_URL",
        "GW_MUSEAI_ACCESS_TOKEN",
        "GW_MUSEAI_NOTARY_TOKEN",
        "GW_MUSEAI_VM_ID",
        "GW_MUSEAI_COOKIE",
    ];

    let managed: std::collections::HashMap<&str, &str> = fields
        .iter()
        .filter(|(k, v)| MANAGED.contains(k) && !v.is_empty())
        .map(|(k, v)| (*k, *v))
        .collect();

    if managed.is_empty() {
        return Ok(());
    }

    let lines: Vec<String> = if path.exists() {
        fs::read_to_string(path)?
            .lines()
            .map(ToString::to_string)
            .collect()
    } else {
        vec![]
    };

    // `kept_in_place` tracks managed keys whose existing line we keep untouched
    // (no new value supplied). Managed keys with a new value are dropped here and
    // re-appended below in their freshly encoded form.
    let mut kept_in_place: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut kept: Vec<String> = Vec::with_capacity(lines.len());
    for line in lines {
        let key = line
            .trim_start_matches(|c: char| c.is_whitespace())
            .split('=')
            .next()
            .unwrap_or("");
        let bare = key.trim_start_matches("export ");
        if MANAGED.contains(&bare) {
            if managed.contains_key(bare) {
                // New value available — drop the old line, re-append fresh below.
            } else {
                kept_in_place.insert(bare.to_string());
                kept.push(line);
            }
        } else {
            kept.push(line);
        }
    }
    let mut lines = kept;

    for (key, value) in &managed {
        if kept_in_place.contains(*key) {
            continue;
        }
        let quoted = if value.contains(' ') || value.contains('"') {
            format!(
                "{}=\"{}\"",
                key,
                value.replace('\\', "\\\\").replace('"', "\\\"")
            )
        } else {
            format!("{key}={value}")
        };
        lines.push(quoted);
    }

    let mut content = lines.join("\n");
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    if content.is_empty() {
        return Ok(());
    }

    let parent = path.parent().unwrap_or(Path::new("."));
    if !parent.exists() {
        fs::create_dir_all(parent).ok();
    }
    let tmp = parent.join(format!(
        ".{}~tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    fs::write(&tmp, content.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, path)?;
    Ok(())
}
