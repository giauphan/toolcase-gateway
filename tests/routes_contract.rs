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
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec!["fail-try".into()],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        default_model: "gpt-5.6-sol".into(),
        museai_base_url: "https://muse.ai".into(),
        museai_cookie: "".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
        jev_api_keys: vec![],
        jev_base_url: "https://jev-ai.org".into(),
    };
    assert_eq!(cfg.target_port, 8080);
    assert_eq!(cfg.fallbacks, vec!["fail-try".to_string()]);
}
