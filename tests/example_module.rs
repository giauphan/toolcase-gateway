use std::fs;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::thread;
use toolcase_gateway::gateway::installer::{
    cleanup_module_files, install_module, scan_and_register_modules, InstallOptions, InstallSource,
};
use toolcase_gateway::gateway::modules::{ModuleRegistry, PersonalManifest};
use toolcase_gateway::http::{read_response_head, Request};
use toolcase_gateway::providers::personal::handle_personal_module_request;

#[test]
fn test_example_echo_provider_structure_and_manifest() {
    let example_dir = Path::new("examples/modules/echo-provider");
    assert!(
        example_dir.is_dir(),
        "examples/modules/echo-provider directory must exist"
    );

    let manifest_path = example_dir.join("module.json");
    assert!(manifest_path.is_file(), "module.json must exist");

    let manifest_content = fs::read_to_string(&manifest_path).expect("read module.json");
    let val: serde_json::Value = serde_json::from_str(&manifest_content).expect("valid json");

    assert_eq!(val["id"], "echo-provider");
    assert_eq!(val["namespace"], "/custom/echo");
    assert_eq!(val["entrypoint"], "run.sh");
    assert_eq!(val["protocol"], "stdio-stream");

    let run_sh = example_dir.join("run.sh");
    let adapter_py = example_dir.join("adapter.py");
    let readme_md = example_dir.join("README.md");

    assert!(run_sh.is_file(), "run.sh must exist");
    assert!(adapter_py.is_file(), "adapter.py must exist");
    assert!(readme_md.is_file(), "README.md must exist");
}

#[test]
fn test_example_echo_provider_dispatch_stdio_flow() {
    let example_dir = Path::new("examples/modules/echo-provider");
    let run_sh_canonical = match example_dir.join("run.sh").canonicalize() {
        Ok(p) => p,
        Err(_) => return, // Skip if filesystem path cannot be canonicalized
    };

    let manifest = PersonalManifest {
        id: "echo-provider".to_string(),
        name: "Echo Provider Adapter".to_string(),
        version: "1.0.0".to_string(),
        namespace: "/custom/echo".to_string(),
        entrypoint: run_sh_canonical.to_string_lossy().to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec!["chat_completions".to_string(), "models".to_string()],
        config_schema: None,
        permissions: vec![],
    };

    let (mut client, mut server) = match create_stream_pair() {
        Some(pair) => pair,
        None => return, // Skip if local sockets unavailable in current environment
    };

    // 1. Test Chat Completions endpoint
    let chat_body =
        br#"{"model":"echo-model","messages":[{"role":"user","content":"Hello Toolcase"}]}"#;
    let req = Request {
        method: "POST".to_string(),
        path: "/custom/echo/v1/chat/completions".to_string(),
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: chat_body.to_vec(),
    };

    let server_handle = thread::spawn(move || {
        handle_personal_module_request(&mut server, &req, &manifest, "/v1/chat/completions")
    });

    let head = read_response_head(&mut client).expect("read response head");
    assert_eq!(head.status, 200);

    let mut full_body = head.buffered_body;
    let _ = client.read_to_end(&mut full_body);
    let resp_val: serde_json::Value =
        serde_json::from_slice(&full_body).expect("parse response body");

    assert_eq!(resp_val["object"], "chat.completion");
    assert_eq!(resp_val["model"], "echo-model");
    let content = resp_val["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default();
    assert!(
        content.contains("Echo: Hello Toolcase"),
        "expected content to contain echo message, got: {content}"
    );

    let exit_status = server_handle.join().unwrap().unwrap();
    assert_eq!(exit_status, 200);
}

#[test]
fn test_example_echo_provider_install_from_local_dir_and_discovery() {
    let temp_state = std::env::temp_dir().join(format!("test_echo_state_{}", uuid::Uuid::new_v4()));
    let example_dir = Path::new("examples/modules/echo-provider")
        .canonicalize()
        .unwrap();

    let install_opts = InstallOptions {
        id: Some("installed-echo-test".to_string()),
        namespace: Some("/custom/echo-test".to_string()),
        ..Default::default()
    };

    let manifest = install_module(
        InstallSource::LocalDir(example_dir),
        install_opts,
        &temp_state,
    )
    .expect("install example module from local directory");

    assert_eq!(manifest.id, "installed-echo-test");
    assert_eq!(manifest.namespace, "/custom/echo-test");
    assert!(Path::new(&manifest.entrypoint).is_file());

    let mut reg = ModuleRegistry::new();
    reg.register_personal_module(manifest.clone()).unwrap();

    let found = reg.get_module("installed-echo-test").unwrap();
    assert_eq!(found.namespace, "/custom/echo-test");

    // Test scan and discovery also discovers it in temp_state/modules
    let mut reg2 = ModuleRegistry::new();
    scan_and_register_modules(&mut reg2, &temp_state);
    let discovered = reg2.get_module("installed-echo-test");
    assert!(
        discovered.is_some(),
        "auto-discovery detected installed module"
    );

    cleanup_module_files("installed-echo-test", &temp_state);
    let _ = fs::remove_dir_all(&temp_state);
}

#[test]
fn test_personal_module_accepts_structured_json_body() {
    let temp_script =
        std::env::temp_dir().join(format!("test_echo_script_{}.sh", uuid::Uuid::new_v4()));
    fs::write(
        &temp_script,
        "#!/bin/sh\necho '{\"status\": 200, \"body\": {\"structured\": true, \"count\": 42}}'\n",
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&temp_script).unwrap().permissions();
        perms.set_mode(0o755);
        let _ = fs::set_permissions(&temp_script, perms);
    }

    let manifest = PersonalManifest {
        id: "test-structured".to_string(),
        name: "Test Structured".to_string(),
        version: "1.0.0".to_string(),
        namespace: "/custom/struct".to_string(),
        entrypoint: temp_script.to_string_lossy().to_string(),
        protocol: "stdio-stream".to_string(),
        capabilities: vec![],
        config_schema: None,
        permissions: vec![],
    };

    let (mut client, mut server) = match create_stream_pair() {
        Some(pair) => pair,
        None => return,
    };

    let req = Request {
        method: "GET".to_string(),
        path: "/custom/struct/ping".to_string(),
        headers: vec![],
        body: vec![],
    };

    let server_handle = thread::spawn(move || {
        handle_personal_module_request(&mut server, &req, &manifest, "/ping")
    });

    let head = read_response_head(&mut client).expect("read head");
    assert_eq!(head.status, 200);

    let mut body = head.buffered_body;
    let _ = client.read_to_end(&mut body);
    let json_val: serde_json::Value = serde_json::from_slice(&body).expect("parse json");
    assert_eq!(json_val["structured"], true);
    assert_eq!(json_val["count"], 42);

    let _ = server_handle.join().unwrap();
    let _ = fs::remove_file(temp_script);
}

fn create_stream_pair() -> Option<(TcpStream, TcpStream)> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let addr = listener.local_addr().ok()?;
    let client = TcpStream::connect(addr).ok()?;
    let (server, _) = listener.accept().ok()?;
    Some((client, server))
}
