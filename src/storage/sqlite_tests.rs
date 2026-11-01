//! Unit tests for SQLite storage adapter and migration logic.

use super::sqlite::SqliteStorage;
use super::{DatabaseEngine, StorageError, StorageRepository, StoredAccount};
use crate::gateway::modules::PersonalManifest;
use std::fs;
use std::path::PathBuf;

#[test]
fn test_sqlite_storage_lifecycle_and_migration() {
    let temp_dir = std::env::temp_dir().join(format!("test_sqlite_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_dir).unwrap();

    // 1. Create a legacy accounts.json file
    let legacy_accounts = temp_dir.join("accounts.json");
    fs::write(
        &legacy_accounts,
        r#"{"accounts": [{"id": "acc_1", "provider": "openrouter", "key": "sk-or-v1-test", "label": "Dev"}]}"#,
    )
    .unwrap();

    // 2. Open SQLite storage
    let storage =
        SqliteStorage::open(DatabaseEngine::Sqlite(PathBuf::from(":memory:")), &temp_dir).unwrap();

    // 3. Test settings roundtrip
    storage
        .set_setting("gateway_config", r#"{"listen_port": 8080}"#)
        .unwrap();
    let cfg = storage.get_setting("gateway_config").unwrap();
    assert_eq!(cfg.as_deref(), Some(r#"{"listen_port": 8080}"#));

    // 4. Test module storage
    let manifest = PersonalManifest {
        id: "my-tool".into(),
        name: "My Tool".into(),
        short_name: None,
        version: "1.0.0".into(),
        namespace: "/custom/tool".into(),
        entrypoint: "/bin/echo".into(),
        protocol: "stdio-stream".into(),
        capabilities: vec!["chat".into()],
        icon: None,
        config_schema: None,
        permissions: vec![],
        group: None,
        source: None,
    };
    storage.save_module(&manifest).unwrap();
    let modules = storage.get_modules().unwrap();
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].id, "my-tool");

    // 5. Test module removal
    storage.remove_module("my-tool").unwrap();
    assert!(storage.get_modules().unwrap().is_empty());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_sqlite_account_credential_encryption_and_decryption() {
    let temp_dir = std::env::temp_dir().join(format!("test_sqlite_enc_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_dir).unwrap();

    let master_key = "test-master-encryption-key-999".to_string();
    let storage = SqliteStorage::open_with_key(
        DatabaseEngine::Sqlite(PathBuf::from(":memory:")),
        &temp_dir,
        master_key,
    )
    .unwrap();

    let acc = StoredAccount {
        id: "acc_enc_1".into(),
        provider: "openrouter".into(),
        key: "sk-or-v1-my-secret-credential".into(),
        label: Some("Encrypted Account".into()),
        is_active: true,
    };

    storage.save_accounts(&[acc]).unwrap();

    // Verify stored in DB as enc:v1:
    {
        let conn = storage.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT key FROM accounts WHERE id = 'acc_enc_1'")
            .unwrap();
        let raw_key: String = stmt.query_row([], |row| row.get(0)).unwrap();
        assert!(raw_key.starts_with("enc:v1:"));
        assert!(!raw_key.contains("sk-or-v1-my-secret-credential"));
    }

    // Verify decrypted in memory via get_accounts()
    let retrieved = storage.get_accounts().unwrap();
    assert_eq!(retrieved.len(), 1);
    assert_eq!(retrieved[0].key, "sk-or-v1-my-secret-credential");

    // Verify legacy unencrypted row compatibility
    {
        let conn = storage.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, provider, key, label, is_active)
             VALUES ('legacy_acc', 'jev', 'sk-jev-legacy-plain', 'Legacy Plain', 1)",
            [],
        )
        .unwrap();
    }

    let all_accs = storage.get_accounts().unwrap();
    assert_eq!(all_accs.len(), 2);
    let legacy = all_accs.iter().find(|a| a.id == "legacy_acc").unwrap();
    assert_eq!(legacy.key, "sk-jev-legacy-plain");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_sqlite_module_enabled_status_lifecycle() {
    let temp_dir = std::env::temp_dir().join(format!("test_sqlite_mod_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_dir).unwrap();

    let storage =
        SqliteStorage::open(DatabaseEngine::Sqlite(PathBuf::from(":memory:")), &temp_dir).unwrap();

    let manifest = PersonalManifest {
        id: "video-generator".into(),
        name: "Video Generator".into(),
        short_name: None,
        version: "1.0.0".into(),
        namespace: "/custom/video".into(),
        entrypoint: "/bin/video".into(),
        protocol: "stdio-stream".into(),
        capabilities: vec!["video".into()],
        icon: None,
        config_schema: None,
        permissions: vec![],
        group: None,
        source: None,
    };

    // 1. Save module with status false (disabled)
    storage.save_module_with_status(&manifest, false).unwrap();

    let stored = storage.get_stored_modules().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].manifest.id, "video-generator");
    assert!(!stored[0].enabled);

    // 2. Toggle module to true (enabled)
    storage.set_module_enabled("video-generator", true).unwrap();
    let stored_after_enable = storage.get_stored_modules().unwrap();
    assert_eq!(stored_after_enable.len(), 1);
    assert!(stored_after_enable[0].enabled);

    // 3. Toggle back to false (disabled)
    storage
        .set_module_enabled("video-generator", false)
        .unwrap();
    let stored_after_disable = storage.get_stored_modules().unwrap();
    assert!(!stored_after_disable[0].enabled);

    // 4. Update manifest using save_module (conflict update); verify disabled status is preserved!
    let mut updated_manifest = manifest.clone();
    updated_manifest.name = "Video Generator Pro".into();
    storage.save_module(&updated_manifest).unwrap();

    let stored_after_update = storage.get_stored_modules().unwrap();
    assert_eq!(stored_after_update[0].manifest.name, "Video Generator Pro");
    assert!(
        !stored_after_update[0].enabled,
        "save_module must preserve existing enabled=false flag on conflict update"
    );

    // 5. Toggle non-existent module returns NotFound
    let res = storage.set_module_enabled("non-existent-module", true);
    assert!(matches!(res, Err(StorageError::NotFound(_))));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_sqlite_legacy_modules_migration_from_gateway_config() {
    let temp_dir = std::env::temp_dir().join(format!("test_sqlite_mig_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_dir).unwrap();

    let config_content = r#"{
        "personal_modules": [
            {
                "id": "migrated-mod-1",
                "name": "Migrated Module 1",
                "version": "1.0.0",
                "namespace": "/custom/migrated1",
                "entrypoint": "python3 main.py",
                "protocol": "stdio-stream",
                "capabilities": ["chat"]
            },
            {
                "id": "migrated-mod-2",
                "name": "Migrated Module 2",
                "version": "2.0.0",
                "namespace": "/custom/migrated2",
                "entrypoint": "python3 main2.py",
                "protocol": "stdio-stream",
                "capabilities": ["tools"]
            }
        ],
        "enabled_modules": {
            "migrated-mod-1": true,
            "migrated-mod-2": false
        }
    }"#;
    fs::write(temp_dir.join("gateway_config.json"), config_content).unwrap();

    let storage =
        SqliteStorage::open(DatabaseEngine::Sqlite(PathBuf::from(":memory:")), &temp_dir).unwrap();

    let stored = storage.get_stored_modules().unwrap();
    assert_eq!(stored.len(), 2);

    let m1 = stored
        .iter()
        .find(|m| m.manifest.id == "migrated-mod-1")
        .expect("migrated-mod-1 should exist");
    assert!(m1.enabled);
    assert_eq!(m1.manifest.namespace, "/custom/migrated1");

    let m2 = stored
        .iter()
        .find(|m| m.manifest.id == "migrated-mod-2")
        .expect("migrated-mod-2 should exist");
    assert!(!m2.enabled);
    assert_eq!(m2.manifest.namespace, "/custom/migrated2");

    let _ = fs::remove_dir_all(&temp_dir);
}
