use std::fs;
use toolcase_gateway::gateway::installer::builder::{
    detect_crate_binary_name, is_binary_fresh, is_cargo_available,
};
use toolcase_gateway::gateway::installer::{
    cleanup_module_files, install_module, upgrade_module, InstallOptions, InstallSource,
};

#[test]
fn test_builder_cargo_detected() {
    assert!(is_cargo_available());
}

#[test]
fn test_builder_crate_binary_detection() {
    let temp = std::env::temp_dir().join(format!("test_mod_bin_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp).unwrap();
    let cargo_toml = temp.join("Cargo.toml");

    fs::write(
        &cargo_toml,
        r#"
[package]
name = "default-pkg"
version = "0.1.0"

[[bin]]
name = "custom-bin-target"
path = "src/main.rs"
"#,
    )
    .unwrap();

    let name = detect_crate_binary_name(&cargo_toml, "fallback");
    assert_eq!(name, "custom-bin-target");

    let cargo_pkg_only = temp.join("Cargo_pkg.toml");
    fs::write(
        &cargo_pkg_only,
        r#"
[package]
name = "package-name-only"
version = "0.1.0"
"#,
    )
    .unwrap();

    let name2 = detect_crate_binary_name(&cargo_pkg_only, "fallback");
    assert_eq!(name2, "package-name-only");
    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn test_builder_binary_freshness_logic() {
    let temp = std::env::temp_dir().join(format!("test_mod_fresh_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp).unwrap();
    let bin_path = temp.join("test_bin");

    // Missing binary is not fresh
    assert!(!is_binary_fresh(&temp, &bin_path));

    // Create binary
    fs::write(&bin_path, b"dummy binary").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));

    // No source files => binary is fresh
    assert!(is_binary_fresh(&temp, &bin_path));

    // Update Cargo.toml after binary was created => binary is now stale
    std::thread::sleep(std::time::Duration::from_millis(10));
    fs::write(temp.join("Cargo.toml"), b"# updated").unwrap();
    assert!(!is_binary_fresh(&temp, &bin_path));
    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn test_upgrade_module_rolls_back_if_compilation_fails() {
    let temp_state = std::env::temp_dir().join(format!("test_state_{}", uuid::Uuid::new_v4()));
    let v1_dir = std::env::temp_dir().join(format!("test_v1_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&v1_dir).unwrap();

    let v1_manifest = r#"{
        "id": "failing-build-test",
        "name": "Failing Build Test v1",
        "version": "1.0.0",
        "namespace": "/custom/failing-build",
        "entrypoint": "run.sh",
        "protocol": "stdio-stream",
        "capabilities": ["test"]
    }"#;
    fs::write(v1_dir.join("module.json"), v1_manifest).unwrap();
    fs::write(
        v1_dir.join("run.sh"),
        b"#!/bin/sh\necho '{\"status\": 200, \"body\": \"v1\"}'\n",
    )
    .unwrap();

    // 1. Install v1 successfully
    let installed = install_module(
        InstallSource::LocalDir(v1_dir.clone()),
        InstallOptions::default(),
        &temp_state,
    )
    .expect("v1 install succeeds");
    assert_eq!(installed.version, "1.0.0");

    // 2. Prepare v2 with broken Rust syntax (fails to compile)
    let v2_dir = std::env::temp_dir().join(format!("test_v2_{}", uuid::Uuid::new_v4()));
    let v2_src = v2_dir.join("src");
    fs::create_dir_all(&v2_src).unwrap();

    let v2_manifest = r#"{
        "id": "failing-build-test",
        "name": "Failing Build Test v2",
        "version": "2.0.0",
        "namespace": "/custom/failing-build",
        "entrypoint": "run.sh",
        "protocol": "stdio-stream",
        "capabilities": ["test"]
    }"#;
    fs::write(v2_dir.join("module.json"), v2_manifest).unwrap();
    fs::write(
        v2_dir.join("Cargo.toml"),
        r#"
[package]
name = "failing-build-test"
version = "2.0.0"
edition = "2021"

[[bin]]
name = "failing-build-test"
path = "src/main.rs"
"#,
    )
    .unwrap();

    // Deliberate Rust syntax error
    fs::write(
        v2_src.join("main.rs"),
        b"fn main() { this is not valid rust code syntax; }",
    )
    .unwrap();

    // 3. Attempt upgrade to v2: must fail and roll back cleanly
    let upgrade_res = upgrade_module(
        "failing-build-test",
        InstallSource::LocalDir(v2_dir.clone()),
        InstallOptions::default(),
        &temp_state,
        None,
    );

    assert!(
        upgrade_res.is_err(),
        "Upgrade must fail due to compile error"
    );

    // 4. Verify active module in temp_state is still v1 (unmodified and rolled back)
    let active_manifest_path = temp_state
        .join("modules")
        .join("failing-build-test")
        .join("module.json");
    assert!(active_manifest_path.is_file());
    let active_content = fs::read_to_string(active_manifest_path).unwrap();
    assert!(active_content.contains("\"version\": \"1.0.0\""));
    assert!(active_content.contains("Failing Build Test v1"));

    // Cleanup
    cleanup_module_files("failing-build-test", &temp_state);
    let _ = fs::remove_dir_all(&temp_state);
    let _ = fs::remove_dir_all(&v1_dir);
    let _ = fs::remove_dir_all(&v2_dir);
}

#[test]
fn test_install_and_upgrade_rust_module_auto_compiles() {
    let temp_state = std::env::temp_dir().join(format!("test_state_{}", uuid::Uuid::new_v4()));
    let src_dir = std::env::temp_dir().join(format!("test_rust_mod_{}", uuid::Uuid::new_v4()));
    let src_code_dir = src_dir.join("src");
    fs::create_dir_all(&src_code_dir).unwrap();

    let cargo_toml = r#"
[package]
name = "autobuild-test"
version = "1.0.0"
edition = "2021"

[[bin]]
name = "autobuild-test"
path = "src/main.rs"
"#;
    let main_rs = r#"
fn main() {
    println!("autobuild-output-v1");
}
"#;
    let manifest = r#"{
        "id": "autobuild-test",
        "name": "AutoBuild Test",
        "version": "1.0.0",
        "namespace": "/custom/autobuild",
        "entrypoint": "run.sh",
        "protocol": "stdio-stream",
        "capabilities": ["test"]
    }"#;

    fs::write(src_dir.join("Cargo.toml"), cargo_toml).unwrap();
    fs::write(src_code_dir.join("main.rs"), main_rs).unwrap();
    fs::write(src_dir.join("module.json"), manifest).unwrap();

    // 1. Install source-only Rust module (no prebuilt binary provided)
    let installed = install_module(
        InstallSource::LocalDir(src_dir.clone()),
        InstallOptions::default(),
        &temp_state,
    )
    .expect("installing source-only Rust module succeeds");

    assert_eq!(installed.id, "autobuild-test");
    assert_eq!(installed.version, "1.0.0");

    let installed_bin = temp_state
        .join("modules")
        .join("autobuild-test")
        .join("autobuild-test");
    assert!(
        installed_bin.is_file(),
        "Compiled binary must exist at module root after install"
    );

    // Verify the binary executes and outputs v1
    let out = std::process::Command::new(&installed_bin)
        .output()
        .expect("binary executes");
    assert!(String::from_utf8_lossy(&out.stdout).contains("autobuild-output-v1"));

    // 2. Upgrade to v2 with modified source code
    let v2_manifest = r#"{
        "id": "autobuild-test",
        "name": "AutoBuild Test v2",
        "version": "2.0.0",
        "namespace": "/custom/autobuild",
        "entrypoint": "run.sh",
        "protocol": "stdio-stream",
        "capabilities": ["test"]
    }"#;
    let v2_main_rs = r#"
fn main() {
    println!("autobuild-output-v2");
}
"#;
    let v2_dir = std::env::temp_dir().join(format!("test_rust_mod_v2_{}", uuid::Uuid::new_v4()));
    let v2_src_code_dir = v2_dir.join("src");
    fs::create_dir_all(&v2_src_code_dir).unwrap();
    fs::write(
        v2_dir.join("Cargo.toml"),
        cargo_toml.replace("1.0.0", "2.0.0"),
    )
    .unwrap();
    fs::write(v2_src_code_dir.join("main.rs"), v2_main_rs).unwrap();
    fs::write(v2_dir.join("module.json"), v2_manifest).unwrap();

    let upgraded = upgrade_module(
        "autobuild-test",
        InstallSource::LocalDir(v2_dir.clone()),
        InstallOptions::default(),
        &temp_state,
        None,
    )
    .expect("upgrading source-only Rust module succeeds");

    assert_eq!(upgraded.version, "2.0.0");

    // Verify the recompiled binary executes and outputs v2
    let out_v2 = std::process::Command::new(&installed_bin)
        .output()
        .expect("binary executes v2");
    assert!(String::from_utf8_lossy(&out_v2.stdout).contains("autobuild-output-v2"));

    // Cleanup
    cleanup_module_files("autobuild-test", &temp_state);
    let _ = fs::remove_dir_all(&temp_state);
    let _ = fs::remove_dir_all(&src_dir);
    let _ = fs::remove_dir_all(&v2_dir);
}
