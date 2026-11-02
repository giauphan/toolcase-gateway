use std::fs;
use std::path::Path;
use toolcase_gateway::gateway::installer::{
    cleanup_module_files, install_module, scan_and_register_modules, upgrade_module,
    InstallOptions, InstallSource,
};
use toolcase_gateway::gateway::modules::{ModuleRegistry, PersonalManifest};

fn build_stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = Vec::new();
    let mut central_entries = Vec::new();

    for (name, data) in entries {
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
    zip
}

#[test]
fn test_scan_and_auto_discover_standalone_manifest_module() {
    let temp_state = std::env::temp_dir().join(format!("test_state_{}", uuid::Uuid::new_v4()));
    let module_dir = temp_state.join("modules").join("custom-standalone-addon");
    fs::create_dir_all(&module_dir).unwrap();

    let manifest_json = r#"{
        "id": "custom-standalone-addon",
        "name": "Custom Standalone Addon",
        "version": "1.0.0",
        "namespace": "/custom/addon",
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
    assert!(reg.get_module("custom-standalone-addon").is_none());

    scan_and_register_modules(&mut reg, &temp_state);

    let desc = reg
        .get_module("custom-standalone-addon")
        .expect("module auto-discovered");
    assert_eq!(desc.id, "custom-standalone-addon");
    assert_eq!(desc.name, "Custom Standalone Addon");
    assert_eq!(desc.namespace, "/custom/addon");

    let pers = reg
        .get_personal_manifest("custom-standalone-addon")
        .unwrap();
    assert!(Path::new(&pers.entrypoint).is_absolute());
    assert!(pers.entrypoint.ends_with("run.sh"));

    cleanup_module_files("custom-standalone-addon", &temp_state);
    assert!(!module_dir.exists());
    let _ = fs::remove_dir_all(&temp_state);
}

#[test]
fn test_scan_and_auto_discover_addons_directory_module() {
    let addons_dir = Path::new("addons").join("test-dropin-addon");
    let Ok(()) = fs::create_dir_all(&addons_dir) else {
        return;
    };

    let manifest_json = r#"{
        "id": "test-dropin-addon",
        "name": "Test Dropin Addon",
        "version": "1.0.0",
        "namespace": "/custom/dropin-addon",
        "entrypoint": "run.sh",
        "protocol": "stdio-stream",
        "capabilities": ["chat_completions"]
    }"#;
    fs::write(addons_dir.join("module.json"), manifest_json).unwrap();
    fs::write(
        addons_dir.join("run.sh"),
        "#!/bin/sh\necho '{\"status\": 200}'\n",
    )
    .unwrap();

    let mut reg = ModuleRegistry::new();
    let temp_state = std::env::temp_dir().join(format!("test_state_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_state).unwrap();

    scan_and_register_modules(&mut reg, &temp_state);

    let desc = reg
        .get_module("test-dropin-addon")
        .expect("module auto-discovered from addons directory");
    assert_eq!(desc.id, "test-dropin-addon");
    assert_eq!(desc.namespace, "/custom/dropin-addon");

    let _ = fs::remove_dir_all(&addons_dir);
    let _ = fs::remove_dir_all(&temp_state);
}

#[test]
fn test_scan_and_auto_discover_in_tree_addons_echo_provider() {
    let mut reg = ModuleRegistry::new();
    let temp_state = std::env::temp_dir().join(format!("test_state_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_state).unwrap();

    scan_and_register_modules(&mut reg, &temp_state);

    if std::path::Path::new("addons/echo-provider/module.json").exists() {
        let desc = reg
            .get_module("echo-provider")
            .expect("in-tree addons/echo-provider auto-discovered");
        assert_eq!(desc.id, "echo-provider");
        assert_eq!(desc.namespace, "/custom/echo");
    }

    if std::path::Path::new("addons/tiktok-studio/module.json").exists() {
        let tiktok = reg
            .get_module("tiktok-studio")
            .expect("in-tree addons/tiktok-studio auto-discovered");
        assert_eq!(tiktok.id, "tiktok-studio");
        assert_eq!(tiktok.namespace, "/custom/tiktok");
    }

    if std::path::Path::new("addons/facebook-suite/module.json").exists() {
        let fb = reg
            .get_module("facebook-suite")
            .expect("in-tree addons/facebook-suite auto-discovered");
        assert_eq!(fb.id, "facebook-suite");
        assert_eq!(fb.namespace, "/custom/facebook");
    }

    let _ = fs::remove_dir_all(&temp_state);
}

#[test]
fn test_unregistered_module_is_not_resurrected_by_scan() {
    let mut reg = ModuleRegistry::new();
    let temp_state = std::env::temp_dir().join(format!("test_state_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_state).unwrap();

    let fake_entrypoint = temp_state.join("run.sh");
    fs::write(&fake_entrypoint, "#!/bin/sh\nexit 0\n").unwrap();

    let manifest = PersonalManifest {
        id: "test-uninstalled-module".to_string(),
        name: "Test Uninstalled".to_string(),
        short_name: None,
        version: "1.0.0".to_string(),
        namespace: "/custom/uninstalled".to_string(),
        entrypoint: fake_entrypoint.to_string_lossy().to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec![],
        icon: None,
        config_schema: None,
        permissions: vec![],
        source: None,
        group: None,
    };
    reg.register_personal_module(manifest).unwrap();
    assert!(reg.get_module("test-uninstalled-module").is_some());

    reg.unregister_personal_module("test-uninstalled-module")
        .unwrap();
    assert!(reg.get_module("test-uninstalled-module").is_none());
    assert!(reg.is_uninstalled("test-uninstalled-module"));

    let fake_addon_dir = temp_state.join("modules").join("test-uninstalled-module");
    fs::create_dir_all(&fake_addon_dir).unwrap();
    fs::write(
        fake_addon_dir.join("module.json"),
        r#"{"id":"test-uninstalled-module","name":"Test Uninstalled","version":"1.0.0","namespace":"/custom/uninstalled","entrypoint":"run.sh"}"#,
    )
    .unwrap();

    let newly = scan_and_register_modules(&mut reg, &temp_state);
    assert!(!newly.iter().any(|m| m.id == "test-uninstalled-module"));
    assert!(reg.get_module("test-uninstalled-module").is_none());

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

    let zip = build_stored_zip(&[
        ("zipped-plugin-main/module.json", &manifest_bytes[..]),
        ("zipped-plugin-main/run.sh", &script_bytes[..]),
    ]);

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

    let zip = build_stored_zip(&[
        ("repo-main/module.json", manifest_json.as_bytes()),
        ("repo-main/main.py", main_py.as_bytes()),
    ]);

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
fn test_scan_and_auto_discover_facebook_suite_addon() {
    if !std::path::Path::new("addons/facebook-suite/module.json").exists() {
        return;
    }
    let mut reg = ModuleRegistry::new();
    let state_dir = std::path::PathBuf::from(".gw_state");
    scan_and_register_modules(&mut reg, &state_dir);

    let fb = reg
        .get_module("facebook-suite")
        .expect("facebook-suite must be auto-discovered from addons/");
    assert_eq!(fb.id, "facebook-suite");
    assert_eq!(fb.namespace, "/custom/facebook");
    assert!(fb.capabilities.contains(&"studio".to_string()));
    assert!(fb.capabilities.contains(&"upload".to_string()));
    assert!(fb.capabilities.contains(&"publish".to_string()));
    assert!(fb.capabilities.contains(&"reels".to_string()));
}

#[test]
fn test_upgrade_monorepo_subpath() {
    let temp_state = std::env::temp_dir().join(format!("test_state_{}", uuid::Uuid::new_v4()));
    let initial_manifest = r#"{
        "id": "monorepo-sub",
        "name": "Sub Module",
        "version": "1.0.0",
        "namespace": "/custom/monorepo-sub",
        "entrypoint": "run.sh",
        "protocol": "stdio-stream",
        "capabilities": ["news"]
    }"#;
    let initial_script = b"#!/bin/sh\necho '{\"status\": 200, \"body\": \"v1\"}'\n";
    let zip_v1 = build_stored_zip(&[
        ("module.json", initial_manifest.as_bytes()),
        ("run.sh", &initial_script[..]),
    ]);

    let _ = install_module(
        InstallSource::ZipBytes(zip_v1),
        InstallOptions::default(),
        &temp_state,
    )
    .expect("initial install succeeds");

    let upgraded_manifest = r#"{
        "id": "monorepo-sub",
        "name": "Sub Module Upgraded",
        "version": "1.1.0",
        "namespace": "/custom/monorepo-sub",
        "entrypoint": "run.sh",
        "protocol": "stdio-stream",
        "capabilities": ["news", "discovery"]
    }"#;
    let upgraded_script = b"#!/bin/sh\necho '{\"status\": 200, \"body\": \"v2\"}'\n";

    let monorepo_dir = std::env::temp_dir().join(format!("test_monorepo_{}", uuid::Uuid::new_v4()));
    let sub_dir = monorepo_dir.join("modules").join("monorepo-sub");
    fs::create_dir_all(&sub_dir).unwrap();
    fs::write(sub_dir.join("module.json"), upgraded_manifest).unwrap();
    fs::write(sub_dir.join("run.sh"), upgraded_script).unwrap();

    let upgraded = upgrade_module(
        "monorepo-sub",
        InstallSource::LocalDir(monorepo_dir.clone()),
        InstallOptions {
            id: Some("monorepo-sub".to_string()),
            ..Default::default()
        },
        &temp_state,
        None,
    )
    .expect("monorepo subpath upgrade succeeds");

    assert_eq!(upgraded.version, "1.1.0");
    assert_eq!(upgraded.name, "Sub Module Upgraded");
    assert!(temp_state
        .join("modules")
        .join("monorepo-sub")
        .join("module.json")
        .is_file());

    cleanup_module_files("monorepo-sub", &temp_state);
    let _ = fs::remove_dir_all(&temp_state);
    let _ = fs::remove_dir_all(&monorepo_dir);
}
