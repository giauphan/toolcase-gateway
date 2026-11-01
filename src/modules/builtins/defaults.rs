//! Default built-in provider module definitions and registry bootstrapping.

use super::super::manifest::{
    builtin_ui_defaults, default_menu_position, ModuleDescriptor, ModuleKind, ModuleStatus,
    SubProviderDescriptor,
};
use super::super::registry::ModuleRegistry;

/// Creates a sub-provider descriptor for a built-in provider group.
pub fn sub_provider(id: &str, name: &str, subpath: &str, caps: &[&str]) -> SubProviderDescriptor {
    SubProviderDescriptor {
        id: id.to_string(),
        name: name.to_string(),
        subpath: subpath.to_string(),
        capabilities: caps.iter().map(ToString::to_string).collect(),
        status: ModuleStatus::Enabled,
    }
}

/// Registers the default platform built-in modules (`omniroute`, `muse`, `jev`) into the registry.
pub fn register_default_builtins(registry: &mut ModuleRegistry) {
    register_builtin(
        registry,
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
        sub_provider(
            "chat",
            "Muse Chat Completions",
            "/chat",
            &["chat", "models"],
        ),
        sub_provider("image", "Muse Image Generation", "/image", &["image"]),
        sub_provider("video", "Muse Video Generation", "/video", &["video"]),
        sub_provider(
            "threads",
            "Muse Thread Memory & Cleanup Cron",
            "/threads",
            &["threads", "cron", "cleanup"],
        ),
    ];

    register_builtin(
        registry,
        "muse",
        "Muse AI Provider",
        "/muse-ai",
        vec![
            "chat".to_string(),
            "models".to_string(),
            "video".to_string(),
            "image".to_string(),
            "threads".to_string(),
            "cron".to_string(),
        ],
        Some(muse_providers),
    );

    let jev_providers = vec![
        sub_provider(
            "decisions",
            "Jev Decision Engine",
            "/decisions",
            &["decisions"],
        ),
        sub_provider("models", "Jev Model Sampler", "/models", &["models"]),
    ];

    register_builtin(
        registry,
        "jev",
        "Jev Decision Provider",
        "/jev",
        vec!["decisions".to_string(), "models".to_string()],
        Some(jev_providers),
    );
}

fn register_builtin(
    registry: &mut ModuleRegistry,
    id: &str,
    name: &str,
    namespace: &str,
    capabilities: Vec<String>,
    providers: Option<Vec<SubProviderDescriptor>>,
) {
    let clean_ns = namespace.trim_end_matches('/').to_string();
    let (order, icon, desc, config_url, tab, schema) = builtin_ui_defaults(id, name, &clean_ns);
    let short_name = match id {
        "omniroute" => Some("OmniRoute".to_string()),
        "muse" => Some("Muse AI".to_string()),
        "jev" => Some("Jev Decision".to_string()),
        _ => None,
    };
    registry.insert_builtin_descriptor(ModuleDescriptor {
        id: id.to_string(),
        name: name.to_string(),
        short_name,
        namespace: clean_ns,
        kind: ModuleKind::BuiltIn,
        status: ModuleStatus::Enabled,
        capabilities,
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        providers,
        menu_position: default_menu_position(),
        menu_order: order,
        icon: Some(icon),
        description: Some(desc),
        config_url: Some(config_url),
        settings_tab: Some(tab),
        config_schema: schema,
        source: None,
    });
}
