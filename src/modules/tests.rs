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

    assert!(reg.get_module("tiktok").is_none());
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

fn sample_manifest(id: &str, name: &str, namespace: &str) -> PersonalManifest {
    PersonalManifest {
        id: id.to_string(),
        name: name.to_string(),
        short_name: None,
        version: "1.0.0".to_string(),
        namespace: namespace.to_string(),
        entrypoint: "/bin/sh".to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec!["chat".to_string()],
        icon: None,
        config_schema: None,
        permissions: Vec::new(),
        group: None,
        source: None,
    }
}

#[test]
fn test_personal_module_registration_and_isolation() {
    let mut reg = ModuleRegistry::new();
    let manifest = sample_manifest("my-test-mod", "My Test Module", "/custom/test-mod");

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
    let mut manifest = sample_manifest(
        "my-group-mod",
        "My Multi-Provider Group",
        "/custom/group-mod",
    );
    manifest.capabilities = vec!["chat".to_string(), "vision".to_string()];
    manifest.group = Some(ModuleGroupManifest {
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
    });

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
    let mut manifest =
        sample_manifest("duplicate-mod", "Duplicate Provider Mod", "/custom/dup-mod");
    manifest.group = Some(ModuleGroupManifest {
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
    });

    assert!(reg.register_personal_module(manifest).is_err());
}

#[test]
fn test_provider_prefix_required_and_rejects_empty_or_root() {
    let mut reg = ModuleRegistry::new();
    let mut manifest = sample_manifest("no-prefix-mod", "No Prefix Mod", "");

    // Empty namespace rejected
    assert!(reg.register_personal_module(manifest.clone()).is_err());

    // Whitespace namespace rejected
    manifest.namespace = "   ".to_string();
    assert!(reg.register_personal_module(manifest.clone()).is_err());

    // Root slash namespace rejected
    manifest.namespace = "/".to_string();
    assert!(reg.register_personal_module(manifest.clone()).is_err());

    // Multiple slashes rejected
    manifest.namespace = "///".to_string();
    assert!(reg.register_personal_module(manifest).is_err());
}

#[test]
fn test_provider_prefix_rejects_reserved_gateway_urls() {
    let mut reg = ModuleRegistry::new();
    let reserved_samples = [
        "/dashboard",
        "/home",
        "/api",
        "/mcp",
        "/sse",
        "/health",
        "/status",
        "/metrics",
        "/favicon.ico",
        "/sw.js",
        "/manifest.webmanifest",
        "/robots.txt",
        "/video",
        "/v1",
        "/gateway",
        "/gateway-config",
        "/gateway-ui",
        "/video-template",
        "/image-template",
        "/muse-config",
        "/dashboard/custom",
        "/api/custom",
    ];

    for (i, prefix) in reserved_samples.iter().enumerate() {
        let manifest = sample_manifest(
            &format!("reserved-test-mod-{i}"),
            "Reserved Test Mod",
            prefix,
        );
        let res = reg.register_personal_module(manifest);
        assert!(
            res.is_err(),
            "Expected prefix '{prefix}' to be rejected as reserved gateway URL"
        );
    }
}

#[test]
fn test_provider_prefix_rejects_builtin_provider_collisions() {
    let mut reg = ModuleRegistry::new();
    let builtin_samples = [
        "/omniroute",
        "/ominroute",
        "/muse-ai",
        "/muse",
        "/jev",
        "/jev-ai",
        "/omniroute/v1",
        "/muse-ai/v1",
        "/jev/v1",
    ];

    for (i, prefix) in builtin_samples.iter().enumerate() {
        let manifest = sample_manifest(
            &format!("builtin-collision-test-{i}"),
            "Collision Test Mod",
            prefix,
        );
        let res = reg.register_personal_module(manifest);
        assert!(
            res.is_err(),
            "Expected prefix '{prefix}' to be rejected as colliding with builtin provider"
        );
    }
}

#[test]
fn test_provider_prefix_rejects_malformed_syntax() {
    let mut reg = ModuleRegistry::new();
    let malformed_samples = [
        "custom/no-leading-slash",
        "/custom//double-slash",
        "/custom/space in path",
        "/custom/has?query",
        "/custom/has#fragment",
    ];

    for (i, prefix) in malformed_samples.iter().enumerate() {
        let manifest =
            sample_manifest(&format!("malformed-test-{i}"), "Malformed Test Mod", prefix);
        assert!(
            reg.register_personal_module(manifest).is_err(),
            "Expected prefix '{prefix}' to be rejected as malformed"
        );
    }
}

#[test]
fn test_sub_provider_requires_valid_non_root_subpath() {
    let mut reg = ModuleRegistry::new();
    let invalid_subpaths = ["/", "//", "/sub with space", "/sub?query"];

    for (i, subpath) in invalid_subpaths.iter().enumerate() {
        let mut manifest = sample_manifest(
            &format!("sub-invalid-mod-{i}"),
            "Invalid Subpath Mod",
            &format!("/custom/sub-invalid-{i}"),
        );
        manifest.group = Some(ModuleGroupManifest {
            enabled: true,
            providers: vec![SubProviderManifest {
                id: "sub-1".to_string(),
                name: "Sub 1".to_string(),
                subpath: subpath.to_string(),
                capabilities: vec![],
            }],
        });
        assert!(
            reg.register_personal_module(manifest).is_err(),
            "Expected subpath '{subpath}' to be rejected as invalid"
        );
    }
}

#[test]
fn test_module_ui_position_and_auto_config() {
    let mut reg = ModuleRegistry::new();
    let list = reg.list_modules();
    assert_eq!(list[0].id, "omniroute");
    assert_eq!(list[1].id, "muse");
    assert_eq!(list[2].id, "jev");

    // Move jev up above muse
    let moved = reg.move_module_order("jev", "up").unwrap();
    assert!(moved.menu_order < reg.get_module("muse").unwrap().menu_order);

    // Pin jev to sidebar with order 5 (first)
    let updated = reg
        .set_module_position("jev", Some("sidebar".to_string()), Some(5))
        .unwrap();
    assert_eq!(updated.menu_position, "sidebar");
    assert_eq!(updated.menu_order, 5);
    assert_eq!(reg.list_modules()[0].id, "jev");

    // Reject invalid menu_position
    assert!(reg
        .set_module_position("jev", Some("invalid_pos".to_string()), None)
        .is_err());

    // Update module config and verify secret masking
    let mut cfg = serde_json::Map::new();
    cfg.insert(
        "base_url".to_string(),
        serde_json::Value::String("https://api.example.com".to_string()),
    );
    cfg.insert(
        "api_key".to_string(),
        serde_json::Value::String("sk-secret-value-123456".to_string()),
    );
    reg.update_module_config("jev", cfg).unwrap();

    let masked = reg.get_masked_module_config("jev");
    assert_eq!(
        masked.get("base_url").and_then(|v| v.as_str()),
        Some("https://api.example.com")
    );
    assert_eq!(
        masked.get("masked_api_key").and_then(|v| v.as_str()),
        Some("sk-sec...3456")
    );
    assert!(masked.get("api_key").is_none());
}

#[test]
fn test_module_short_name_and_icon_preservation_and_defaults() {
    let mut reg = ModuleRegistry::new();

    // 1. Built-in modules have short names and distinct icons
    let omni = reg.get_module("omniroute").unwrap();
    assert_eq!(omni.short_name.as_deref(), Some("OmniRoute"));
    assert_eq!(omni.icon.as_deref(), Some("◎"));

    let muse = reg.get_module("muse").unwrap();
    assert_eq!(muse.short_name.as_deref(), Some("Muse AI"));
    assert_eq!(muse.icon.as_deref(), Some("✦"));

    // 2. Personal module with auto-detected known defaults (tiktok-studio, facebook-suite)
    let tt_manifest = sample_manifest(
        "tiktok-studio",
        "TikTok Creator Studio Adapter",
        "/custom/tiktok",
    );
    reg.register_personal_module(tt_manifest).unwrap();
    let tt_desc = reg.get_module("tiktok-studio").unwrap();
    assert_eq!(tt_desc.short_name.as_deref(), Some("TikTok Creator Studio"));
    assert_eq!(tt_desc.icon.as_deref(), Some("♬"));

    let tt_alias_manifest = sample_manifest(
        "TikTok-Creator-Studio-Adapter",
        "TikTok Creator Studio Adapter",
        "/custom/tiktok-alias",
    );
    reg.register_personal_module(tt_alias_manifest).unwrap();
    let tt_alias_desc = reg.get_module("TikTok-Creator-Studio-Adapter").unwrap();
    assert_eq!(
        tt_alias_desc.short_name.as_deref(),
        Some("TikTok Creator Studio")
    );
    assert_eq!(tt_alias_desc.icon.as_deref(), Some("♬"));

    let fb_manifest = sample_manifest(
        "facebook-suite",
        "Meta Business Suite / Facebook Creator Studio Adapter",
        "/custom/facebook",
    );
    reg.register_personal_module(fb_manifest).unwrap();
    let fb_desc = reg.get_module("facebook-suite").unwrap();
    assert_eq!(fb_desc.short_name.as_deref(), Some("Meta Business Suite"));
    assert_eq!(fb_desc.icon.as_deref(), Some("ⓕ"));

    // 3. Custom personal module preserving explicit icon and short_name
    let mut custom = sample_manifest(
        "custom-mod",
        "Custom Long Provider Module",
        "/custom/my-tool",
    );
    custom.short_name = Some("Custom Tool".to_string());
    custom.icon = Some("🛠".to_string());
    reg.register_personal_module(custom).unwrap();
    let custom_desc = reg.get_module("custom-mod").unwrap();
    assert_eq!(custom_desc.short_name.as_deref(), Some("Custom Tool"));
    assert_eq!(custom_desc.icon.as_deref(), Some("🛠"));
}

#[test]
fn test_upgrade_personal_module_in_registry() {
    let mut reg = ModuleRegistry::new();

    let mut m1 = sample_manifest("my-upg-mod", "My Upgrade Mod", "/custom/my-upg");
    m1.version = "1.0.0".to_string();
    m1.capabilities = vec!["chat".to_string()];
    m1.source = Some("https://github.com/my-org/my-upg".to_string());
    reg.register_personal_module(m1).unwrap();

    // Set custom position, order, and disabled status
    reg.set_module_position("my-upg-mod", Some("sidebar".to_string()), Some(42))
        .unwrap();
    reg.set_module_status("my-upg-mod", ModuleStatus::Disabled)
        .unwrap();

    let before = reg.get_module("my-upg-mod").unwrap();
    assert_eq!(before.status, ModuleStatus::Disabled);
    assert_eq!(before.menu_position, "sidebar");
    assert_eq!(before.menu_order, 42);
    assert_eq!(before.version.as_deref(), Some("1.0.0"));

    // Upgrade with v2 manifest
    let mut m2 = sample_manifest("my-upg-mod", "My Upgrade Mod v2", "/custom/my-upg");
    m2.version = "2.0.0".to_string();
    m2.capabilities = vec!["chat".to_string(), "embeddings".to_string()];

    let upgraded = reg.upgrade_personal_module(m2).unwrap();
    assert_eq!(upgraded.version.as_deref(), Some("2.0.0"));
    assert_eq!(upgraded.name, "My Upgrade Mod v2");
    assert_eq!(upgraded.status, ModuleStatus::Disabled); // Preserved!
    assert_eq!(upgraded.menu_position, "sidebar"); // Preserved!
    assert_eq!(upgraded.menu_order, 42); // Preserved!
    assert_eq!(
        upgraded.source.as_deref(),
        Some("https://github.com/my-org/my-upg")
    ); // Preserved from existing!
    assert!(upgraded.capabilities.contains(&"embeddings".to_string()));

    // Cannot upgrade built-in module
    let mut fake_builtin = sample_manifest("omniroute", "Fake Omni", "/ominroute");
    fake_builtin.version = "9.9.9".to_string();
    assert!(reg.upgrade_personal_module(fake_builtin).is_err());
}
