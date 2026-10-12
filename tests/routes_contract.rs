//! Offline regression tests for public router exports and synthetic configuration.

use toolcase_gateway::routes::{catalog_route, CatalogPlatform};
use toolcase_gateway::Config;

#[test]
fn test_catalog_routing_offline() {
    assert_eq!(
        catalog_route("/muse-ai/models"),
        Some(CatalogPlatform::Muse)
    );
    assert_eq!(
        catalog_route("/muse-ai/v1/models"),
        Some(CatalogPlatform::Muse)
    );
    assert_eq!(catalog_route("/unknown-route"), None);
}

#[test]
fn test_synthetic_config_defaults() {
    let cfg = Config {
        target_port: 8080,
        fallbacks: vec!["fail-try".into()],
        ..Config::default()
    };
    assert_eq!(cfg.target_port, 8080);
    assert_eq!(cfg.fallbacks, vec!["fail-try".to_string()]);
}
