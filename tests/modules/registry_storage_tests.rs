use std::fs;
use toolcase_gateway::config::{Config, ConfigStore};
use toolcase_gateway::gateway::installer::{install_module, InstallOptions, InstallSource};
use toolcase_gateway::gateway::modules::{
    ModuleError, ModuleRegistry, ModuleStatus, PersonalManifest,
};
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
        source: None,
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
        source: None,
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
        source: None,
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
        source: None,
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
        source: None,
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
        source: None,
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

#[test]
fn test_config_store_sqlite_module_hydration_and_toggle_persistence() {
    let temp_dir = std::env::temp_dir().join(format!("test_store_hydr_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_dir).unwrap();

    // 1. Pre-populate SQLite with a disabled module
    let db_engine = DatabaseEngine::Sqlite(temp_dir.join("gateway.db"));
    let storage = SqliteStorage::open(db_engine.clone(), &temp_dir).unwrap();
    let manifest = PersonalManifest {
        id: "hydrated-tool".into(),
        name: "Hydrated Tool".into(),
        short_name: None,
        version: "1.0.0".into(),
        namespace: "/custom/hydrated".into(),
        entrypoint: "/bin/echo".into(),
        protocol: "stdio-stream".into(),
        capabilities: vec!["chat".into()],
        icon: None,
        config_schema: None,
        permissions: vec![],
        group: None,
        source: None,
    };
    storage.save_module_with_status(&manifest, false).unwrap();
    drop(storage);

    // 2. Initialize ConfigStore with storage connected to the same SQLite database
    let storage_repo =
        std::sync::Arc::new(SqliteStorage::open(db_engine.clone(), &temp_dir).unwrap());
    let config_file = temp_dir.join("gateway_config.json");
    let env_file = temp_dir.join(".env");
    let store = ConfigStore::new_with_limits_config_file_and_storage(
        Config::default(),
        env_file.clone(),
        config_file.clone(),
        100,
        1024 * 1024,
        storage_repo,
    );

    // Verify registry hydrated the module from SQLite as disabled
    {
        let reg = store.module_registry();
        let guard = reg.read().unwrap();
        let desc = guard
            .get_module("hydrated-tool")
            .expect("hydrated-tool should be loaded");
        assert_eq!(desc.status, ModuleStatus::Disabled);
    }

    // 3. Toggle to enabled via storage and registry
    store
        .storage
        .set_module_enabled("hydrated-tool", true)
        .unwrap();
    {
        let reg = store.module_registry();
        let mut guard = reg.write().unwrap();
        guard
            .set_module_status("hydrated-tool", ModuleStatus::Enabled)
            .unwrap();
    }

    // 4. Restart simulation: create a new ConfigStore pointing to the same DB
    let restart_storage = std::sync::Arc::new(SqliteStorage::open(db_engine, &temp_dir).unwrap());
    let restart_store = ConfigStore::new_with_limits_config_file_and_storage(
        Config::default(),
        env_file,
        config_file,
        100,
        1024 * 1024,
        restart_storage,
    );

    {
        let reg = restart_store.module_registry();
        let guard = reg.read().unwrap();
        let desc = guard
            .get_module("hydrated-tool")
            .expect("hydrated-tool should survive restart");
        assert_eq!(
            desc.status,
            ModuleStatus::Enabled,
            "Module should hydrate with enabled status after reboot"
        );
    }

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_config_store_migrates_legacy_json_into_sqlite() {
    let temp_dir = std::env::temp_dir().join(format!("test_store_mig_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_dir).unwrap();

    let legacy_json = r#"{
        "personal_modules": [
            {
                "id": "legacy-auto-mig",
                "name": "Legacy Auto Migrated",
                "version": "1.0.0",
                "namespace": "/custom/legacy",
                "entrypoint": "/bin/run.py",
                "protocol": "stdio-stream",
                "capabilities": ["chat"]
            }
        ],
        "enabled_modules": {
            "legacy-auto-mig": false
        }
    }"#;
    let config_file = temp_dir.join("gateway_config.json");
    fs::write(&config_file, legacy_json).unwrap();

    let db_engine = DatabaseEngine::Sqlite(temp_dir.join("gateway.db"));
    let storage = std::sync::Arc::new(SqliteStorage::open(db_engine, &temp_dir).unwrap());
    let env_file = temp_dir.join(".env");

    let store = ConfigStore::new_with_limits_config_file_and_storage(
        Config::default(),
        env_file,
        config_file,
        100,
        1024 * 1024,
        storage,
    );

    // Verify module was migrated into SQLite storage
    let stored = store.storage.get_stored_modules().unwrap();
    let mod_entry = stored
        .iter()
        .find(|m| m.manifest.id == "legacy-auto-mig")
        .expect("legacy-auto-mig should be in SQLite modules");
    assert!(!mod_entry.enabled);

    // Verify registry hydrated it with disabled status
    {
        let reg = store.module_registry();
        let guard = reg.read().unwrap();
        let desc = guard
            .get_module("legacy-auto-mig")
            .expect("legacy-auto-mig should be registered");
        assert_eq!(desc.status, ModuleStatus::Disabled);
    }

    let _ = fs::remove_dir_all(&temp_dir);
}
