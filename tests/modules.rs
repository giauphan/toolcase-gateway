use std::fs;
use std::path::Path;
use toolcase_gateway::gateway::installer::{
    cleanup_module_files, install_module, scan_and_register_modules, InstallOptions, InstallSource,
};
use toolcase_gateway::gateway::modules::{ModuleError, ModuleRegistry, PersonalManifest};
use toolcase_gateway::storage::{
    parse_database_url, DatabaseEngine, SqliteStorage, StorageRepository, StoredAccount,
};

#[test]
fn test_scan_and_auto_discover_odoo_style_module() {
    let temp_state = std::env::temp_dir().join(format!("test_state_{}", uuid::Uuid::new_v4()));
    let module_dir = temp_state.join("modules").join("custom-odoo-addon");
    fs::create_dir_all(&module_dir).unwrap();

    let manifest_json = r#"{
        "id": "custom-odoo-addon",
        "name": "Custom Odoo Addon",
        "version": "1.0.0",
        "namespace": "/custom/odoo",
        "entrypoint": "run.sh",
        "protocol": "stdio-stream",
        "capabilities": ["chat_completions", "models"]
    }"#;
    fs::write(module_dir.join("__manifest__.json"), manifest_json).unwrap();
    fs::write(
        module_dir.join("run.sh"),
        "#!/bin/sh\necho '{\"status\": 200}'\n",
    )
    .unwrap();

    let mut reg = ModuleRegistry::new();
    assert!(reg.get_module("custom-odoo-addon").is_none());

    scan_and_register_modules(&mut reg, &temp_state);

    let desc = reg
        .get_module("custom-odoo-addon")
        .expect("module auto-discovered");
    assert_eq!(desc.id, "custom-odoo-addon");
    assert_eq!(desc.name, "Custom Odoo Addon");
    assert_eq!(desc.namespace, "/custom/odoo");

    let pers = reg.get_personal_manifest("custom-odoo-addon").unwrap();
    assert!(Path::new(&pers.entrypoint).is_absolute());
    assert!(pers.entrypoint.ends_with("run.sh"));

    cleanup_module_files("custom-odoo-addon", &temp_state);
    assert!(!module_dir.exists());
    let _ = fs::remove_dir_all(&temp_state);
}

#[test]
fn test_install_and_convert_script_without_manifest() {
    let temp_state = std::env::temp_dir().join(format!("test_state_{}", uuid::Uuid::new_v4()));
    let src_dir = std::env::temp_dir().join(format!("test_raw_repo_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&src_dir).unwrap();

    let py_script = src_dir.join("main.py");
    fs::write(
        &py_script,
        r#"import sys, json
for line in sys.stdin:
    if line.strip():
        req = json.loads(line)
        res = {"status": 200, "headers": {"content-type": "application/json"}, "body": "{\"ok\": true}"}
        sys.stdout.write(json.dumps(res) + "\n")
        sys.stdout.flush()
"#,
    )
    .unwrap();

    let options = InstallOptions {
        id: Some("auto-converted-py".to_string()),
        name: Some("Auto Converted Python Module".to_string()),
        namespace: Some("/custom/auto-py".to_string()),
        ..Default::default()
    };

    let manifest = install_module(
        InstallSource::LocalDir(src_dir.clone()),
        options,
        &temp_state,
    )
    .expect("auto-conversion succeeds");

    assert_eq!(manifest.id, "auto-converted-py");
    assert_eq!(manifest.namespace, "/custom/auto-py");
    assert!(manifest.entrypoint.ends_with("run.sh"));
    assert!(Path::new(&manifest.entrypoint).is_file());

    let installed_dir = temp_state.join("modules").join("auto-converted-py");
    assert!(installed_dir.join("module.json").is_file());
    assert!(installed_dir.join("main.py").is_file());

    cleanup_module_files("auto-converted-py", &temp_state);
    assert!(!installed_dir.exists());

    let _ = fs::remove_dir_all(&src_dir);
    let _ = fs::remove_dir_all(&temp_state);
}

#[test]
fn test_install_from_zip_bytes() {
    let temp_state = std::env::temp_dir().join(format!("test_state_{}", uuid::Uuid::new_v4()));

    // Create an in-memory zip archive with module.json and run.sh
    let mut zip = Vec::new();
    let manifest_bytes = br#"{
        "id": "zipped-plugin",
        "name": "Zipped Plugin Module",
        "version": "1.0.0",
        "namespace": "/custom/zipped",
        "entrypoint": "run.sh",
        "protocol": "stdio-stream",
        "capabilities": ["chat_completions"]
    }"#;

    let script_bytes = b"#!/bin/sh\necho '{\"status\": 200}'\n";

    // Build zip entries
    let files = [
        ("zipped-plugin-main/module.json", &manifest_bytes[..]),
        ("zipped-plugin-main/run.sh", &script_bytes[..]),
    ];

    let mut central_entries = Vec::new();

    for (name, data) in &files {
        let local_offset = zip.len();
        let name_bytes = name.as_bytes();

        let mut crc = flate2::Crc::new();
        crc.update(data);
        let crc32 = crc.sum();

        // Local header
        zip.extend_from_slice(b"PK\x03\x04");
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes()); // stored
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&crc32.to_le_bytes());
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(name_bytes);
        zip.extend_from_slice(data);

        central_entries.push((*name, local_offset, data.len(), crc32));
    }

    let cd_offset = zip.len();
    for (name, loc, size, crc32) in &central_entries {
        let name_bytes = name.as_bytes();
        zip.extend_from_slice(b"PK\x01\x02");
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&crc32.to_le_bytes());
        zip.extend_from_slice(&(*size as u32).to_le_bytes());
        zip.extend_from_slice(&(*size as u32).to_le_bytes());
        zip.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&(*loc as u32).to_le_bytes());
        zip.extend_from_slice(name_bytes);
    }

    let cd_size = zip.len() - cd_offset;
    zip.extend_from_slice(b"PK\x05\x06");
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip.extend_from_slice(&(central_entries.len() as u16).to_le_bytes());
    zip.extend_from_slice(&(central_entries.len() as u16).to_le_bytes());
    zip.extend_from_slice(&(cd_size as u32).to_le_bytes());
    zip.extend_from_slice(&(cd_offset as u32).to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());

    let manifest = install_module(
        InstallSource::ZipBytes(zip),
        InstallOptions::default(),
        &temp_state,
    )
    .expect("zip install succeeds");

    assert_eq!(manifest.id, "zipped-plugin");
    assert_eq!(manifest.namespace, "/custom/zipped");
    assert!(manifest.entrypoint.ends_with("run.sh"));
    assert!(Path::new(&manifest.entrypoint).is_file());

    cleanup_module_files("zipped-plugin", &temp_state);
    assert!(!temp_state.join("modules").join("zipped-plugin").exists());
    let _ = fs::remove_dir_all(&temp_state);
}

#[test]
fn test_install_from_zip_with_single_top_level_wrapper() {
    let temp_state = std::env::temp_dir().join(format!("test_state_{}", uuid::Uuid::new_v4()));
    let manifest_json = r#"{
        "id": "wrapped-module",
        "name": "Wrapped Module",
        "version": "1.0.0",
        "namespace": "/custom/wrapped",
        "entrypoint": "main.py",
        "protocol": "stdio-stream",
        "capabilities": ["models"]
    }"#;
    let main_py = "print('hello from wrapped')\n";

    let mut zip = Vec::new();
    let entries = [
        ("repo-main/module.json", manifest_json.as_bytes()),
        ("repo-main/main.py", main_py.as_bytes()),
    ];

    let mut central_entries = Vec::new();
    for (name, data) in &entries {
        let local_offset = zip.len();
        let name_bytes = name.as_bytes();
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let crc32 = crc.sum();

        zip.extend_from_slice(b"PK\x03\x04");
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&crc32.to_le_bytes());
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(name_bytes);
        zip.extend_from_slice(data);

        central_entries.push((*name, local_offset, data.len(), crc32));
    }

    let cd_offset = zip.len();
    for (name, loc, size, crc32) in &central_entries {
        let name_bytes = name.as_bytes();
        zip.extend_from_slice(b"PK\x01\x02");
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&crc32.to_le_bytes());
        zip.extend_from_slice(&(*size as u32).to_le_bytes());
        zip.extend_from_slice(&(*size as u32).to_le_bytes());
        zip.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&(*loc as u32).to_le_bytes());
        zip.extend_from_slice(name_bytes);
    }

    let cd_size = zip.len() - cd_offset;
    zip.extend_from_slice(b"PK\x05\x06");
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip.extend_from_slice(&(central_entries.len() as u16).to_le_bytes());
    zip.extend_from_slice(&(central_entries.len() as u16).to_le_bytes());
    zip.extend_from_slice(&(cd_size as u32).to_le_bytes());
    zip.extend_from_slice(&(cd_offset as u32).to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());

    let manifest = install_module(
        InstallSource::ZipBytes(zip),
        InstallOptions::default(),
        &temp_state,
    )
    .expect("wrapped zip unwraps and installs successfully");

    assert_eq!(manifest.id, "wrapped-module");
    assert_eq!(manifest.namespace, "/custom/wrapped");
    assert!(manifest.entrypoint.ends_with("main.py"));
    assert!(Path::new(&manifest.entrypoint).is_file());

    cleanup_module_files("wrapped-module", &temp_state);
    let _ = fs::remove_dir_all(&temp_state);
}

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
        config_schema: None,
        permissions: vec![],
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
        config_schema: None,
        permissions: vec![],
    };
    assert!(matches!(
        reg.register_personal_module(reserved_ns),
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
        config_schema: None,
        permissions: vec![],
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
        config_schema: None,
        permissions: vec![],
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
        config_schema: None,
        permissions: vec![],
    };
    assert!(matches!(
        reg.register_personal_module(overlap_ns),
        Err(ModuleError::NamespaceCollision(_))
    ));
}

#[test]
fn test_relative_entrypoint_resolution_and_canonicalization() {
    let temp_state = std::env::temp_dir().join(format!("test_state_{}", uuid::Uuid::new_v4()));
    let src_dir = std::env::temp_dir().join(format!("test_src_rel_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("main.py"), "print('hello from relative')").unwrap();

    let opts = InstallOptions {
        id: Some("relative-test-module".to_string()),
        namespace: Some("/custom/relative".to_string()),
        entrypoint: Some("main.py".to_string()),
        ..Default::default()
    };

    let manifest = install_module(InstallSource::LocalDir(src_dir.clone()), opts, &temp_state)
        .expect("install succeeds with relative entrypoint");

    assert!(std::path::Path::new(&manifest.entrypoint).is_absolute());
    assert!(manifest.entrypoint.ends_with("main.py"));

    let mut reg = ModuleRegistry::new();
    assert!(reg.register_personal_module(manifest).is_ok());

    cleanup_module_files("relative-test-module", &temp_state);
    let _ = fs::remove_dir_all(&temp_state);
    let _ = fs::remove_dir_all(&src_dir);
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
fn test_dashboard_module_api_lifecycle() {
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use toolcase_gateway::config::ConfigStore;
    use toolcase_gateway::http::read_response_head;
    use toolcase_gateway::routes::route_request;

    let temp_env = std::env::temp_dir().join(format!("test_store_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(
        toolcase_gateway::config::Config::default(),
        temp_env.clone(),
    );

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store_clone = store.clone();
    let handle = thread::spawn(move || {
        for _ in 0..5 {
            let (client, _) = listener.accept().unwrap();
            let _ = route_request(client, &store_clone);
        }
    });

    // 1. GET /api/v1/dashboard/overview (initially includes personal & built-in modules)
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "GET /api/v1/dashboard/overview HTTP/1.1\r\nHost: localhost\r\n\r\n"
    )
    .unwrap();
    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);

    // 2. POST /api/v1/dashboard/modules/install with raw manifest
    let install_body = serde_json::json!({
        "id": "http-test-mod",
        "name": "HTTP Test Module",
        "version": "1.0.0",
        "namespace": "/custom/http-test",
        "entrypoint": "/bin/echo",
        "protocol": "stdio-stream",
        "capabilities": ["models"]
    })
    .to_string();

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "POST /api/v1/dashboard/modules/install HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
        install_body.len(),
        install_body
    )
    .unwrap();
    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);

    // 3. POST /api/v1/dashboard/modules/install with omitted optional fields (auto-infers id and entrypoint)
    let simple_install_body = serde_json::json!({
        "namespace": "/custom/auto-derived-tool"
    })
    .to_string();

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "POST /api/v1/dashboard/modules/install HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
        simple_install_body.len(),
        simple_install_body
    )
    .unwrap();
    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);

    // 4. POST /api/v1/dashboard/modules/state
    let toggle_body = serde_json::json!({"id": "http-test-mod", "enabled": false}).to_string();
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "POST /api/v1/dashboard/modules/state HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
        toggle_body.len(),
        toggle_body
    )
    .unwrap();
    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);

    // 5. POST /api/v1/dashboard/modules/remove
    let remove_body = serde_json::json!({"id": "http-test-mod"}).to_string();
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "POST /api/v1/dashboard/modules/remove HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
        remove_body.len(),
        remove_body
    )
    .unwrap();
    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);

    handle.join().unwrap();
    let _ = fs::remove_file(temp_env);
}

#[test]
fn test_system_catalog_and_one_click_installation() {
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use toolcase_gateway::config::{Config, ConfigStore};
    use toolcase_gateway::http::read_response_head;

    let catalog = toolcase_gateway::gateway::modules::get_system_module_catalog();
    assert!(catalog.iter().any(|c| c.id == "cohere-provider"));
    assert!(catalog.iter().any(|c| c.id == "claude-adapter"));
    assert!(catalog.iter().any(|c| c.id == "groq-fast"));

    let state_dir = toolcase_gateway::gateway::installer::get_state_dir();
    cleanup_module_files("cohere-provider", &state_dir);
    cleanup_module_files("another-cohere", &state_dir);

    let temp_env = std::env::temp_dir().join(format!("cat_test_{}.env", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(Config::default(), temp_env.clone());

    // 1. Install cohere-provider from catalog by ID
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store_clone = store.clone();

    let handle = thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        let payload = br#"{"id":"cohere-provider"}"#;
        toolcase_gateway::features::dashboard::handle_install_personal_module(
            &mut client,
            payload,
            &store_clone,
        )
        .unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let head = read_response_head(&mut client).unwrap();
    if head.status != 200 {
        panic!(
            "Failed: status={}, body={:?}",
            head.status,
            String::from_utf8_lossy(&head.buffered_body)
        );
    }
    assert_eq!(head.status, 200);
    handle.join().unwrap();

    // 2. Verify registered in module registry
    let reg = store.module_registry();
    let guard = reg.read().unwrap();
    let cohere_mod = guard
        .get_module("cohere-provider")
        .expect("cohere-provider registered");
    assert_eq!(cohere_mod.name, "Cohere Provider Adapter");
    assert_eq!(cohere_mod.namespace, "/custom/cohere");
    assert_eq!(
        cohere_mod.status,
        toolcase_gateway::gateway::modules::ModuleStatus::Enabled
    );
    drop(guard);

    // 3. Namespace collision returns 400 Bad Request
    let listener2 = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port2 = listener2.local_addr().unwrap().port();
    let store_clone2 = store.clone();

    let handle2 = thread::spawn(move || {
        let (mut client, _) = listener2.accept().unwrap();
        let payload = br#"{"id":"another-cohere","namespace":"/custom/cohere"}"#;
        let _ = toolcase_gateway::features::dashboard::handle_install_personal_module(
            &mut client,
            payload,
            &store_clone2,
        );
    });

    let mut client2 = TcpStream::connect(("127.0.0.1", port2)).unwrap();
    let head2 = read_response_head(&mut client2).unwrap();
    assert_eq!(head2.status, 400);
    handle2.join().unwrap();

    let _ = fs::remove_file(temp_env);
    let _ = fs::remove_file(store.config_file);
    cleanup_module_files("cohere-provider", &state_dir);
    cleanup_module_files("another-cohere", &state_dir);
}
