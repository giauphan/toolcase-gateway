use std::fs;
use std::path::Path;
use toolcase_gateway::gateway::installer::{
    cleanup_module_files, install_module, scan_and_register_modules, InstallOptions, InstallSource,
};
use toolcase_gateway::gateway::modules::ModuleRegistry;

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
