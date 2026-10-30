//! Dynamic Dashboard UI menu positioning and per-module configuration management.

use super::manifest::{
    is_secret_config_key, is_valid_menu_position, mask_config_map, ModuleDescriptor, ModuleError,
    ModuleUiPosition,
};
use super::registry::ModuleRegistry;
use std::collections::HashMap;

impl ModuleRegistry {
    /// Updates the Dashboard UI menu position and/or sort order for a module.
    pub fn set_module_position(
        &mut self,
        id: &str,
        menu_position: Option<String>,
        menu_order: Option<i32>,
    ) -> Result<ModuleDescriptor, ModuleError> {
        if let Some(ref pos) = menu_position {
            if !is_valid_menu_position(pos) {
                return Err(ModuleError::InvalidManifest(format!(
                    "Invalid menu_position '{pos}'. Expected one of: providers, sidebar, studios, overview, hidden"
                )));
            }
        }

        let desc = self
            .modules
            .get_mut(id)
            .ok_or_else(|| ModuleError::NotFound(id.to_string()))?;

        if let Some(pos) = menu_position {
            desc.menu_position = pos.trim().to_ascii_lowercase();
        }
        if let Some(order) = menu_order {
            desc.menu_order = order;
        }
        Ok(desc.clone())
    }

    /// Moves a module up or down in the Dashboard UI ordering by swapping with its neighbor.
    pub fn move_module_order(
        &mut self,
        id: &str,
        direction: &str,
    ) -> Result<ModuleDescriptor, ModuleError> {
        if !self.modules.contains_key(id) {
            return Err(ModuleError::NotFound(id.to_string()));
        }

        let sorted = self.list_modules();
        // Normalize orders so every module has a distinct step of 10 first
        for (idx, item) in sorted.iter().enumerate() {
            if let Some(m) = self.modules.get_mut(&item.id) {
                m.menu_order = ((idx as i32) + 1) * 10;
            }
        }

        let sorted = self.list_modules();
        let Some(pos_idx) = sorted.iter().position(|m| m.id == id) else {
            return Err(ModuleError::NotFound(id.to_string()));
        };

        let dir = direction.trim().to_ascii_lowercase();
        let target_idx = match dir.as_str() {
            "up" => pos_idx.checked_sub(1),
            "down" => {
                if pos_idx + 1 < sorted.len() {
                    Some(pos_idx + 1)
                } else {
                    None
                }
            }
            _ => {
                return Err(ModuleError::InvalidManifest(format!(
                    "Invalid move direction '{direction}'. Expected 'up' or 'down'"
                )))
            }
        };

        if let Some(other_idx) = target_idx {
            let cur_order = sorted[pos_idx].menu_order;
            let other_id = sorted[other_idx].id.clone();
            let other_order = sorted[other_idx].menu_order;
            if let Some(m) = self.modules.get_mut(id) {
                m.menu_order = other_order;
            }
            if let Some(other) = self.modules.get_mut(&other_id) {
                other.menu_order = cur_order;
            }
        }

        self.modules
            .get(id)
            .cloned()
            .ok_or_else(|| ModuleError::NotFound(id.to_string()))
    }

    /// Exports UI position settings for all registered modules.
    pub fn export_module_positions(&self) -> HashMap<String, ModuleUiPosition> {
        let mut out = HashMap::new();
        for (id, desc) in &self.modules {
            out.insert(
                id.clone(),
                ModuleUiPosition {
                    menu_position: desc.menu_position.clone(),
                    menu_order: desc.menu_order,
                },
            );
        }
        out
    }

    /// Applies persisted UI position settings to registered modules.
    pub fn apply_module_positions(&mut self, map: &HashMap<String, ModuleUiPosition>) {
        for (id, pos) in map {
            if let Some(desc) = self.modules.get_mut(id) {
                if is_valid_menu_position(&pos.menu_position) {
                    desc.menu_position = pos.menu_position.trim().to_ascii_lowercase();
                }
                desc.menu_order = pos.menu_order;
            }
        }
    }

    /// Resolves a module identifier by exact ID, case-insensitive ID, or route namespace alias.
    pub fn resolve_module_id(&self, id_or_alias: &str) -> Option<String> {
        let trimmed = id_or_alias.trim();
        if trimmed.is_empty() {
            return None;
        }
        if self.modules.contains_key(trimmed) {
            return Some(trimmed.to_string());
        }
        if let Some(m) = self
            .modules
            .values()
            .find(|m| m.id.eq_ignore_ascii_case(trimmed))
        {
            return Some(m.id.clone());
        }
        let stem = trimmed
            .trim_matches('/')
            .trim_end_matches("-config")
            .trim_end_matches("-studio");
        let direct_ns = format!("/{stem}");
        let custom_ns = format!("/custom/{stem}");
        if let Some(m) = self.modules.values().find(|m| {
            m.namespace.eq_ignore_ascii_case(&direct_ns)
                || m.namespace.eq_ignore_ascii_case(&custom_ns)
                || (!stem.is_empty()
                    && m.id
                        .to_ascii_lowercase()
                        .contains(&stem.to_ascii_lowercase()))
        }) {
            return Some(m.id.clone());
        }
        None
    }

    fn personal_module_config_path(&self, id: &str) -> Option<std::path::PathBuf> {
        let manifest = self.personal_manifests.get(id)?;
        if manifest.protocol == "local-http"
            || manifest.entrypoint.starts_with("http://")
            || manifest.entrypoint.starts_with("https://")
        {
            return None;
        }
        let ep = std::path::Path::new(&manifest.entrypoint);
        if let Some(parent) = ep.parent() {
            if parent.is_dir() {
                return Some(parent.join("config.json"));
            }
        }
        let fallback_dir = crate::gateway::installer::get_state_dir()
            .join("modules")
            .join(id);
        if fallback_dir.is_dir() {
            return Some(fallback_dir.join("config.json"));
        }
        None
    }

    /// Reads the personal module's `config.json` from its runtime directory if present.
    pub fn read_disk_module_config(
        &self,
        id: &str,
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        let path = self.personal_module_config_path(id)?;
        let content = std::fs::read_to_string(path).ok()?;
        let val: serde_json::Value = serde_json::from_str(&content).ok()?;
        val.as_object().cloned()
    }

    /// Synchronizes any settings stored in the personal module's `config.json` into `module_configs`.
    pub fn sync_module_config_from_disk(&mut self, id: &str) {
        let Some(disk_cfg) = self.read_disk_module_config(id) else {
            return;
        };
        let entry = self.module_configs.entry(id.to_string()).or_default();
        for (k, v) in disk_cfg {
            if is_secret_config_key(&k) {
                if let Some(s) = v.as_str() {
                    if !s.trim().is_empty() {
                        entry.insert(k, serde_json::Value::String(s.trim().to_string()));
                    }
                }
            } else {
                entry.insert(k, v);
            }
        }
    }

    fn write_disk_module_config(&self, id: &str) {
        let Some(path) = self.personal_module_config_path(id) else {
            return;
        };
        let Some(mem_cfg) = self.module_configs.get(id) else {
            return;
        };
        let mut merged = self.read_disk_module_config(id).unwrap_or_default();
        for (k, v) in mem_cfg {
            if is_secret_config_key(k) {
                if let Some(s) = v.as_str() {
                    if !s.trim().is_empty() {
                        merged.insert(k.clone(), serde_json::Value::String(s.trim().to_string()));
                    }
                }
            } else {
                merged.insert(k.clone(), v.clone());
            }
        }
        if let Ok(json_str) = serde_json::to_string_pretty(&merged) {
            let _ = std::fs::write(&path, json_str);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }

    /// Returns the effective merged configuration map for a module (disk + memory).
    pub fn get_effective_module_config(
        &self,
        id: &str,
    ) -> serde_json::Map<String, serde_json::Value> {
        let mut merged = self.read_disk_module_config(id).unwrap_or_default();
        if let Some(mem_cfg) = self.module_configs.get(id) {
            for (k, v) in mem_cfg {
                if is_secret_config_key(k) {
                    if let Some(s) = v.as_str() {
                        if !s.trim().is_empty() {
                            merged
                                .insert(k.clone(), serde_json::Value::String(s.trim().to_string()));
                        }
                    }
                } else {
                    merged.insert(k.clone(), v.clone());
                }
            }
        }
        merged
    }

    /// Returns the raw configuration map for a module.
    pub fn get_module_config(
        &self,
        id: &str,
    ) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.module_configs.get(id)
    }

    /// Returns a masked configuration map suitable for returning in Dashboard GET responses.
    pub fn get_masked_module_config(&self, id: &str) -> serde_json::Map<String, serde_json::Value> {
        let merged = self.get_effective_module_config(id);
        mask_config_map(&merged)
    }

    /// Updates the dynamic configuration for a module, preserving existing secrets when blank.
    pub fn update_module_config(
        &mut self,
        id: &str,
        updates: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), ModuleError> {
        if !self.modules.contains_key(id) {
            return Err(ModuleError::NotFound(id.to_string()));
        }
        self.sync_module_config_from_disk(id);
        let entry = self.module_configs.entry(id.to_string()).or_default();
        for (k, v) in updates {
            if is_secret_config_key(&k) {
                if let Some(s) = v.as_str() {
                    if !s.trim().is_empty() {
                        entry.insert(k, serde_json::Value::String(s.trim().to_string()));
                    }
                }
            } else {
                entry.insert(k, v);
            }
        }
        self.write_disk_module_config(id);
        Ok(())
    }

    /// Exports all per-module configurations for persistent storage.
    pub fn export_module_configs(
        &self,
    ) -> HashMap<String, serde_json::Map<String, serde_json::Value>> {
        let mut out = self.module_configs.clone();
        for id in self.personal_manifests.keys() {
            let eff = self.get_effective_module_config(id);
            if !eff.is_empty() {
                out.insert(id.clone(), eff);
            }
        }
        out
    }

    /// Applies persisted per-module configurations.
    pub fn apply_module_configs(
        &mut self,
        map: &HashMap<String, serde_json::Map<String, serde_json::Value>>,
    ) {
        for (id, cfg) in map {
            self.module_configs.insert(id.clone(), cfg.clone());
            self.write_disk_module_config(id);
        }
    }
}
