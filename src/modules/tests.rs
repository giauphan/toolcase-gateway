//! Unit tests for module registry, manifests, and module groups.

use super::manifest::{
    ModuleGroupManifest, ModuleKind, ModuleStatus, PersonalManifest, SubProviderManifest,
};
use super::registry::ModuleRegistry;

#[test]
fn test_default_registry_has_builtins() {
    let reg = ModuleRegistry::new();
    let omni = reg.get_module("omniroute").expect("omniroute registered");
    assert_eq!(omni.kind, ModuleKind::BuiltIn);
    assert_eq!(omni.status, ModuleStatus::Enabled);

    let muse = reg.get_module("muse").expect("muse registered");
    assert_eq!(muse.kind, ModuleKind::BuiltIn);
    assert!(muse.providers.is_some());
    assert_eq!(muse.providers.as_ref().unwrap().len(), 4);

    let jev = reg.get_module("jev").expect("jev registered");
    assert_eq!(jev.kind, ModuleKind::BuiltIn);
    assert!(jev.providers.is_some());
    assert_eq!(jev.providers.as_ref().unwrap().len(), 2);
}

#[test]
fn test_match_path_builtins() {
    let reg = ModuleRegistry::new();
    let (desc, subpath) = reg
        .match_path("/ominroute/v1/chat/completions")
        .expect("omniroute matched");
    assert_eq!(desc.id, "omniroute");
    assert_eq!(subpath, "/v1/chat/completions");

    let (desc2, subpath2) = reg
        .match_path("/omniroute/v1/models")
        .expect("omniroute alias matched");
    assert_eq!(desc2.id, "omniroute");
    assert_eq!(subpath2, "/v1/models");

    let (muse_desc, muse_sub) = reg
        .match_path("/muse-ai/v1/chat/completions")
        .expect("muse matched");
    assert_eq!(muse_desc.id, "muse");
    assert_eq!(muse_sub, "/v1/chat/completions");

    let (jev_desc, jev_sub) = reg.match_path("/jev/v1/decisions").expect("jev matched");
    assert_eq!(jev_desc.id, "jev");
    assert_eq!(jev_sub, "/v1/decisions");
}

#[test]
fn test_personal_module_registration_and_isolation() {
    let mut reg = ModuleRegistry::new();
    let manifest = PersonalManifest {
        id: "my-test-mod".to_string(),
        name: "My Test Module".to_string(),
        version: "1.0.0".to_string(),
        namespace: "/custom/test-mod".to_string(),
        entrypoint: "/bin/sh".to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec!["chat".to_string()],
        config_schema: None,
        permissions: Vec::new(),
        group: None,
    };

    reg.register_personal_module(manifest).unwrap();
    let desc = reg.get_module("my-test-mod").unwrap();
    assert_eq!(desc.id, "my-test-mod");
    assert_eq!(desc.namespace, "/custom/test-mod");
    assert_eq!(desc.kind, ModuleKind::Personal);
    assert_eq!(desc.status, ModuleStatus::Enabled);

    let (matched_desc, subpath) = reg
        .match_path("/custom/test-mod/sub/route")
        .expect("matched");
    assert_eq!(matched_desc.id, "my-test-mod");
    assert_eq!(subpath, "/sub/route");

    reg.set_module_status("my-test-mod", ModuleStatus::Disabled)
        .unwrap();
    let desc = reg.get_module("my-test-mod").unwrap();
    assert_eq!(desc.status, ModuleStatus::Disabled);

    let unregistered = reg.unregister_personal_module("my-test-mod").unwrap();
    assert_eq!(unregistered.id, "my-test-mod");
    assert!(reg.get_module("my-test-mod").is_none());
}

#[test]
fn test_personal_module_group_registration_and_sub_providers() {
    let mut reg = ModuleRegistry::new();
    let manifest = PersonalManifest {
        id: "my-group-mod".to_string(),
        name: "My Multi-Provider Group".to_string(),
        version: "1.0.0".to_string(),
        namespace: "/custom/group-mod".to_string(),
        entrypoint: "/bin/sh".to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec!["chat".to_string(), "vision".to_string()],
        config_schema: None,
        permissions: Vec::new(),
        group: Some(ModuleGroupManifest {
            enabled: true,
            providers: vec![
                SubProviderManifest {
                    id: "chat-sub".to_string(),
                    name: "Chat Sub Provider".to_string(),
                    subpath: "/chat".to_string(),
                    capabilities: vec!["chat".to_string()],
                },
                SubProviderManifest {
                    id: "vision-sub".to_string(),
                    name: "Vision Sub Provider".to_string(),
                    subpath: "/vision".to_string(),
                    capabilities: vec!["vision".to_string()],
                },
            ],
        }),
    };

    reg.register_personal_module(manifest).unwrap();
    let desc = reg.get_module("my-group-mod").unwrap();
    assert_eq!(desc.kind, ModuleKind::Personal);
    assert!(desc.providers.is_some());
    let provs = desc.providers.as_ref().unwrap();
    assert_eq!(provs.len(), 2);
    assert_eq!(provs[0].id, "chat-sub");
    assert_eq!(provs[1].id, "vision-sub");

    // Match with provider
    let (matched_desc, subpath, sub_prov) = reg
        .match_path_with_provider("/custom/group-mod/chat/completions")
        .expect("matched");
    assert_eq!(matched_desc.id, "my-group-mod");
    assert_eq!(subpath, "/chat/completions");
    assert_eq!(sub_prov.unwrap().id, "chat-sub");

    // Toggle sub-provider
    reg.set_sub_provider_status("my-group-mod", "chat-sub", ModuleStatus::Disabled)
        .unwrap();
    let desc = reg.get_module("my-group-mod").unwrap();
    let provs = desc.providers.as_ref().unwrap();
    assert_eq!(provs[0].status, ModuleStatus::Disabled);
    assert_eq!(provs[1].status, ModuleStatus::Enabled);

    // Toggle whole module disables all
    reg.set_module_status("my-group-mod", ModuleStatus::Disabled)
        .unwrap();
    let desc = reg.get_module("my-group-mod").unwrap();
    assert_eq!(desc.status, ModuleStatus::Disabled);
    let provs = desc.providers.as_ref().unwrap();
    assert_eq!(provs[0].status, ModuleStatus::Disabled);
    assert_eq!(provs[1].status, ModuleStatus::Disabled);
}

#[test]
fn test_module_group_validation_rejects_duplicates() {
    let mut reg = ModuleRegistry::new();
    let manifest = PersonalManifest {
        id: "duplicate-mod".to_string(),
        name: "Duplicate Provider Mod".to_string(),
        version: "1.0.0".to_string(),
        namespace: "/custom/dup-mod".to_string(),
        entrypoint: "/bin/sh".to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec!["chat".to_string()],
        config_schema: None,
        permissions: Vec::new(),
        group: Some(ModuleGroupManifest {
            enabled: true,
            providers: vec![
                SubProviderManifest {
                    id: "p1".to_string(),
                    name: "Provider 1".to_string(),
                    subpath: "/chat".to_string(),
                    capabilities: vec![],
                },
                SubProviderManifest {
                    id: "p1".to_string(), // Duplicate ID!
                    name: "Provider 1 dup".to_string(),
                    subpath: "/chat2".to_string(),
                    capabilities: vec![],
                },
            ],
        }),
    };

    assert!(reg.register_personal_module(manifest).is_err());
}
