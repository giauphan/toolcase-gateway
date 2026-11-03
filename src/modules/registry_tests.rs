//! Unit tests for module UI positioning, icon/short-name defaults, and registry upgrades.

use super::manifest::ModuleStatus;
use super::registry::ModuleRegistry;
use super::tests::sample_manifest;

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
