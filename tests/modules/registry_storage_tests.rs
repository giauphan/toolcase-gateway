use std::fs;
use toolcase_gateway::gateway::installer::{install_module, InstallOptions, InstallSource};
use toolcase_gateway::gateway::modules::{ModuleError, ModuleRegistry, PersonalManifest};
use toolcase_gateway::storage::{
    parse_database_url, DatabaseEngine, SqliteStorage, StorageRepository, StoredAccount,
};

#[test]
fn test_module_id_validation_rejects_traversal_and_invalid_chars() {
    let temp_state = std::env::temp_dir().join(format!("test_state_{}", uuid::Uuid::new_v4()));
    let src_dir = std::env::temp_dir().join(format!("test_src_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("main.py"), "print('hi')").unwrap();

    let invalid_ids = [
        "../../bad-id",
        "/absolute/path",
        "id with spaces",
        "id@special!",
        "..",
        "",
    ];

    for bad_id in invalid_ids {
        let opts = InstallOptions {
            id: Some(bad_id.to_string()),
            namespace: Some("/custom/test".to_string()),
            ..Default::default()
        };
        let res = install_module(InstallSource::LocalDir(src_dir.clone()), opts, &temp_state);
        assert!(res.is_err(), "Should reject invalid id: {bad_id}");
    }

    let _ = fs::remove_dir_all(&src_dir);
    let _ = fs::remove_dir_all(&temp_state);
}

#[test]
fn test_module_namespace_uniqueness_and_collision_prevention() {
    let mut reg = ModuleRegistry::new();

    // 1. Missing or invalid namespace fails
    let bad_ns = PersonalManifest {
        id: "tool-1".into(),
        name: "Tool 1".into(),
        version: "1.0.0".into(),
        namespace: "invalid-no-slash".into(),
        entrypoint: "/bin/echo".into(),
        protocol: "stdio-stream".into(),
        capabilities: vec![],
        icon: None,
        short_name: None,
        config_schema: None,
        permissions: vec![],
        group: None,
    };
    assert!(matches!(
        reg.register_personal_module(bad_ns),
        Err(ModuleError::InvalidNamespace(_))
    ));

    // 2. Reserved prefix fails
    let reserved_ns = PersonalManifest {
        id: "tool-2".into(),
        name: "Tool 2".into(),
        version: "1.0.0".into(),
        namespace: "/api/v1/custom".into(),
        entrypoint: "/bin/echo".into(),
        protocol: "stdio-stream".into(),
        capabilities: vec![],
        icon: None,
        short_name: None,
        config_schema: None,
        permissions: vec![],
        group: None,
    };
    assert!(matches!(
        reg.register_personal_module(reserved_ns),
        Err(ModuleError::ReservedNamespace(_))
    ));

    let ui_reserved_ns = PersonalManifest {
        id: "tool-ui".into(),
        name: "Tool UI".into(),
        version: "1.0.0".into(),
        namespace: "/gateway-ui/custom".into(),
        entrypoint: "/bin/echo".into(),
        protocol: "stdio-stream".into(),
        capabilities: vec![],
        icon: None,
        short_name: None,
        config_schema: None,
        permissions: vec![],
        group: None,
    };
    assert!(matches!(
        reg.register_personal_module(ui_reserved_ns),
        Err(ModuleError::ReservedNamespace(_))
    ));

    // 3. Valid registration succeeds
    let valid_1 = PersonalManifest {
        id: "tool-valid-1".into(),
        name: "Tool 1".into(),
        version: "1.0.0".into(),
        namespace: "/custom/search".into(),
        entrypoint: "/bin/echo".into(),
        protocol: "stdio-stream".into(),
        capabilities: vec![],
        icon: None,
        short_name: None,
        config_schema: None,
        permissions: vec![],
        group: None,
    };
    assert!(reg.register_personal_module(valid_1).is_ok());

    // 4. Duplicate namespace fails
    let dup_ns = PersonalManifest {
        id: "tool-valid-2".into(),
        name: "Tool 2".into(),
        version: "1.0.0".into(),
        namespace: "/custom/search".into(),
        entrypoint: "/bin/echo".into(),
        protocol: "stdio-stream".into(),
        capabilities: vec![],
        icon: None,
        short_name: None,
        config_schema: None,
        permissions: vec![],
        group: None,
    };
    assert!(matches!(
        reg.register_personal_module(dup_ns),
        Err(ModuleError::NamespaceCollision(_))
    ));

    // 5. Overlapping sub-path collision fails
    let overlap_ns = PersonalManifest {
        id: "tool-valid-3".into(),
        name: "Tool 3".into(),
        version: "1.0.0".into(),
        namespace: "/custom/search/extra".into(),
        entrypoint: "/bin/echo".into(),
        protocol: "stdio-stream".into(),
        capabilities: vec![],
        icon: None,
        short_name: None,
        config_schema: None,
        permissions: vec![],
        group: None,
    };
    assert!(matches!(
        reg.register_personal_module(overlap_ns),
        Err(ModuleError::NamespaceCollision(_))
    ));
}

#[test]
fn test_database_storage_url_parsing_and_sqlite_operations() {
    let temp_dir = std::env::temp_dir().join(format!("test_db_repo_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_dir).unwrap();

    // 1. Test parse_database_url schemes
    let sqlite_engine = parse_database_url(Some("sqlite://custom.db"), &temp_dir).unwrap();
    assert_eq!(
        sqlite_engine,
        DatabaseEngine::Sqlite(std::path::PathBuf::from("custom.db"))
    );

    let pg_engine =
        parse_database_url(Some("postgres://user:pass@localhost:5432/db"), &temp_dir).unwrap();
    assert_eq!(
        pg_engine,
        DatabaseEngine::Postgres("postgres://user:pass@localhost:5432/db".into())
    );

    let my_engine =
        parse_database_url(Some("mysql://user:pass@localhost:3306/db"), &temp_dir).unwrap();
    assert_eq!(
        my_engine,
        DatabaseEngine::Mysql("mysql://user:pass@localhost:3306/db".into())
    );

    let default_engine = parse_database_url(None, &temp_dir).unwrap();
    assert_eq!(
        default_engine,
        DatabaseEngine::Sqlite(temp_dir.join("gateway.db"))
    );

    let invalid = parse_database_url(Some("redis://localhost:6379"), &temp_dir);
    assert!(invalid.is_err());

    // 2. Test SqliteStorage operations
    let storage = SqliteStorage::open(
        DatabaseEngine::Sqlite(std::path::PathBuf::from(":memory:")),
        &temp_dir,
    )
    .unwrap();

    let accounts = vec![
        StoredAccount {
            id: "acc_1".into(),
            provider: "openrouter".into(),
            key: "sk-or-v1-key1".into(),
            label: Some("Primary".into()),
            is_active: true,
        },
        StoredAccount {
            id: "acc_2".into(),
            provider: "jev".into(),
            key: "jev_key_2".into(),
            label: None,
            is_active: false,
        },
    ];
    storage.save_accounts(&accounts).unwrap();
    let retrieved = storage.get_accounts().unwrap();
    assert_eq!(retrieved.len(), 2);
    assert_eq!(retrieved[0].id, "acc_1");
    assert_eq!(retrieved[1].key, "jev_key_2");

    let _ = fs::remove_dir_all(&temp_dir);
}
