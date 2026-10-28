//! Authoritative registry for built-in and dynamically installed modules.

use super::manifest::{
    ModuleDescriptor, ModuleError, ModuleKind, ModuleStatus, PersonalManifest,
    SubProviderDescriptor,
};
use super::validation::{
    validate_module_id, validate_module_namespace, validate_module_protocol, validate_sub_providers,
};
use std::collections::HashMap;

/// Registry of authoritative built-in and dynamically installed personal modules.
#[derive(Debug, Clone, Default)]
pub struct ModuleRegistry {
    modules: HashMap<String, ModuleDescriptor>,
    personal_manifests: HashMap<String, PersonalManifest>,
}

impl ModuleRegistry {
    /// Creates a registry initialized with default built-in platform modules.
    pub fn new() -> Self {
        let mut registry = Self {
            modules: HashMap::new(),
            personal_manifests: HashMap::new(),
        };

        registry.register_builtin(
            "omniroute",
            "OmniRoute LLM Proxy",
            "/ominroute",
            vec![
                "chat_completions".to_string(),
                "models".to_string(),
                "failover".to_string(),
            ],
            None,
        );

        let muse_providers = vec![
            SubProviderDescriptor {
                id: "chat".to_string(),
                name: "Muse Chat Completions".to_string(),
                subpath: "/chat".to_string(),
                capabilities: vec!["chat".to_string(), "models".to_string()],
                status: ModuleStatus::Enabled,
            },
            SubProviderDescriptor {
                id: "image".to_string(),
                name: "Muse Image Generation".to_string(),
                subpath: "/image".to_string(),
                capabilities: vec!["image".to_string()],
                status: ModuleStatus::Enabled,
            },
            SubProviderDescriptor {
                id: "video".to_string(),
                name: "Muse Video Generation".to_string(),
                subpath: "/video".to_string(),
                capabilities: vec!["video".to_string()],
                status: ModuleStatus::Enabled,
            },
            SubProviderDescriptor {
                id: "threads".to_string(),
                name: "Muse Thread Memory".to_string(),
                subpath: "/threads".to_string(),
                capabilities: vec!["threads".to_string()],
                status: ModuleStatus::Enabled,
            },
        ];

        registry.register_builtin(
            "muse",
            "Muse AI Provider",
            "/muse-ai",
            vec![
                "chat".to_string(),
                "models".to_string(),
                "video".to_string(),
                "image".to_string(),
            ],
            Some(muse_providers),
        );

        let jev_providers = vec![
            SubProviderDescriptor {
                id: "decisions".to_string(),
                name: "Jev Decision Engine".to_string(),
                subpath: "/decisions".to_string(),
                capabilities: vec!["decisions".to_string()],
                status: ModuleStatus::Enabled,
            },
            SubProviderDescriptor {
                id: "models".to_string(),
                name: "Jev Model Sampler".to_string(),
                subpath: "/models".to_string(),
                capabilities: vec!["models".to_string()],
                status: ModuleStatus::Enabled,
            },
        ];

        registry.register_builtin(
            "jev",
            "Jev Decision Provider",
            "/jev",
            vec!["decisions".to_string(), "models".to_string()],
            Some(jev_providers),
        );

        registry
    }

    fn register_builtin(
        &mut self,
        id: &str,
        name: &str,
        namespace: &str,
        capabilities: Vec<String>,
        providers: Option<Vec<SubProviderDescriptor>>,
    ) {
        self.modules.insert(
            id.to_string(),
            ModuleDescriptor {
                id: id.to_string(),
                name: name.to_string(),
                namespace: namespace.trim_end_matches('/').to_string(),
                kind: ModuleKind::BuiltIn,
                status: ModuleStatus::Enabled,
                capabilities,
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                providers,
            },
        );
    }

    /// Validates and registers an out-of-tree personal module manifest.
    pub fn register_personal_module(
        &mut self,
        manifest: PersonalManifest,
    ) -> Result<(), ModuleError> {
        let id = manifest.id.trim();
        validate_module_id(id, &self.modules)?;

        let ns = manifest.namespace.trim().trim_end_matches('/');
        validate_module_namespace(ns, &manifest.namespace, id, &self.modules)?;

        validate_module_protocol(&manifest.protocol, &manifest.entrypoint)?;

        let sub_providers = if let Some(ref grp) = manifest.group {
            validate_sub_providers(&grp.providers)?;
            let desc_list: Vec<SubProviderDescriptor> = grp
                .providers
                .iter()
                .map(|p| {
                    let clean_subpath = if p.subpath.is_empty() {
                        format!("/{}", p.id.trim())
                    } else if !p.subpath.starts_with('/') {
                        format!("/{}", p.subpath.trim_end_matches('/'))
                    } else {
                        p.subpath.trim_end_matches('/').to_string()
                    };
                    SubProviderDescriptor {
                        id: p.id.trim().to_string(),
                        name: p.name.clone(),
                        subpath: clean_subpath,
                        capabilities: p.capabilities.clone(),
                        status: if grp.enabled {
                            ModuleStatus::Enabled
                        } else {
                            ModuleStatus::Disabled
                        },
                    }
                })
                .collect();
            Some(desc_list)
        } else {
            None
        };

        let desc = ModuleDescriptor {
            id: id.to_string(),
            name: manifest.name.clone(),
            namespace: ns.to_string(),
            kind: ModuleKind::Personal,
            status: ModuleStatus::Enabled,
            capabilities: manifest.capabilities.clone(),
            version: Some(manifest.version.clone()),
            providers: sub_providers,
        };

        self.modules.insert(id.to_string(), desc);
        self.personal_manifests.insert(id.to_string(), manifest);
        Ok(())
    }

    /// Unregisters an installed personal module by id.
    pub fn unregister_personal_module(
        &mut self,
        id: &str,
    ) -> Result<PersonalManifest, ModuleError> {
        if let Some(desc) = self.modules.get(id) {
            if desc.kind == ModuleKind::BuiltIn {
                return Err(ModuleError::InvalidManifest(format!(
                    "Cannot unregister built-in module '{id}'"
                )));
            }
        } else {
            return Err(ModuleError::NotFound(id.to_string()));
        }

        self.modules.remove(id);
        self.personal_manifests
            .remove(id)
            .ok_or_else(|| ModuleError::NotFound(id.to_string()))
    }

    /// Sets the enablement status of a module.
    pub fn set_module_status(&mut self, id: &str, status: ModuleStatus) -> Result<(), ModuleError> {
        let desc = self
            .modules
            .get_mut(id)
            .ok_or_else(|| ModuleError::NotFound(id.to_string()))?;
        desc.status = status;
        if let Some(ref mut providers) = desc.providers {
            for p in providers {
                p.status = status;
            }
        }
        Ok(())
    }

    /// Sets the enablement status of a specific sub-provider within a module group.
    pub fn set_sub_provider_status(
        &mut self,
        module_id: &str,
        sub_provider_id: &str,
        status: ModuleStatus,
    ) -> Result<(), ModuleError> {
        let desc = self
            .modules
            .get_mut(module_id)
            .ok_or_else(|| ModuleError::NotFound(module_id.to_string()))?;
        let providers = desc.providers.as_mut().ok_or_else(|| {
            ModuleError::NotFound(format!("Module '{module_id}' has no sub-providers"))
        })?;
        let provider = providers
            .iter_mut()
            .find(|p| p.id == sub_provider_id)
            .ok_or_else(|| {
                ModuleError::NotFound(format!(
                    "Sub-provider '{sub_provider_id}' not found in module '{module_id}'"
                ))
            })?;
        provider.status = status;
        Ok(())
    }

    /// Matches a normalized URL path against active module namespaces and sub-providers.
    /// Returns the matched module descriptor and the stripped subpath.
    pub fn match_path<'a>(&'a self, path: &'a str) -> Option<(&'a ModuleDescriptor, &'a str)> {
        self.match_path_with_provider(path)
            .map(|(desc, subpath, _)| (desc, subpath))
    }

    /// Matches a normalized URL path against active module namespaces and sub-providers,
    /// returning the matched module descriptor, subpath, and matched sub-provider if applicable.
    pub fn match_path_with_provider<'a>(
        &'a self,
        path: &'a str,
    ) -> Option<(
        &'a ModuleDescriptor,
        &'a str,
        Option<&'a SubProviderDescriptor>,
    )> {
        let clean = path.split('?').next().unwrap_or(path);
        let trimmed = clean.trim_end_matches('/');

        let mut matched: Option<(
            &'a ModuleDescriptor,
            &'a str,
            Option<&'a SubProviderDescriptor>,
        )> = None;
        let mut longest_len = 0;

        for desc in self.modules.values() {
            let ns_str = desc.namespace.as_str();
            let prefixes: &[&str] = if desc.id == "omniroute" {
                &["/ominroute", "/omniroute"]
            } else if desc.id == "jev" {
                &["/jev", "/jev-ai"]
            } else if desc.id == "muse" {
                &[
                    "/muse-ai",
                    "/v1/images/generations",
                    "/video/v1/providers",
                    "/v1/models",
                ]
            } else {
                std::slice::from_ref(&ns_str)
            };

            for &ns in prefixes {
                let (prefix_matched, subpath) = if trimmed == ns {
                    (true, "")
                } else if trimmed.starts_with(ns) && trimmed[ns.len()..].starts_with('/') {
                    (true, &trimmed[ns.len()..])
                } else {
                    (false, "")
                };

                if prefix_matched && ns.len() > longest_len {
                    longest_len = ns.len();

                    // Check if a sub-provider matches within the module group
                    let sub_prov = desc.providers.as_ref().and_then(|providers| {
                        providers.iter().find(|p| {
                            subpath == p.subpath.as_str()
                                || (subpath.starts_with(&p.subpath)
                                    && subpath[p.subpath.len()..].starts_with('/'))
                        })
                    });

                    matched = Some((desc, subpath, sub_prov));
                }
            }
        }

        matched
    }

    /// Returns a list of all registered module descriptors.
    pub fn list_modules(&self) -> Vec<ModuleDescriptor> {
        let mut list: Vec<ModuleDescriptor> = self.modules.values().cloned().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }

    /// Retrieves a module descriptor by id.
    pub fn get_module(&self, id: &str) -> Option<&ModuleDescriptor> {
        self.modules.get(id)
    }

    /// Retrieves a personal module manifest by id.
    pub fn get_personal_manifest(&self, id: &str) -> Option<&PersonalManifest> {
        self.personal_manifests.get(id)
    }

    /// Exports all personal manifests for persistent storage.
    pub fn export_personal_manifests(&self) -> Vec<PersonalManifest> {
        let mut list: Vec<PersonalManifest> = self.personal_manifests.values().cloned().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }

    /// Applies enablement states from configuration.
    pub fn apply_enablement_map(&mut self, map: &HashMap<String, bool>) {
        for (id, enabled) in map {
            if let Some(desc) = self.modules.get_mut(id) {
                let status = if *enabled {
                    ModuleStatus::Enabled
                } else {
                    ModuleStatus::Disabled
                };
                desc.status = status;
                if let Some(ref mut providers) = desc.providers {
                    for p in providers {
                        p.status = status;
                    }
                }
            }
        }
    }
}
