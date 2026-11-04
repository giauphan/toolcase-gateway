//! Adapter for out-of-tree personal provider modules executed out-of-process.

mod cmd;
mod http;
mod stdio;

pub(crate) use cmd::build_command;
pub(crate) use http::execute_http_payload;
pub(crate) use stdio::execute_stdio_payload;
pub(crate) use stdio::spawn_stderr_reader;

use crate::gateway::modules::PersonalManifest;
use crate::http::Request;
use std::io::{self, Write};
use std::net::TcpStream;

/// Dispatches an inbound HTTP request to an out-of-process personal module.
pub fn handle_personal_module_request(
    client: &mut TcpStream,
    request: &Request,
    manifest: &PersonalManifest,
    subpath: &str,
) -> io::Result<u16> {
    handle_personal_module_request_with_provider(client, request, manifest, subpath, None)
}

pub fn handle_personal_module_request_with_provider(
    client: &mut TcpStream,
    request: &Request,
    manifest: &PersonalManifest,
    subpath: &str,
    sub_provider_id: Option<&str>,
) -> io::Result<u16> {
    match manifest.protocol.as_str() {
        "local-http" => http::dispatch_local_http(client, request, manifest, subpath),
        _ => stdio::dispatch_stdio(client, request, manifest, subpath, sub_provider_id),
    }
}

pub(crate) fn write_personal_error(
    client: &mut TcpStream,
    status: u16,
    message: &str,
) -> io::Result<u16> {
    let error_body = serde_json::json!({
        "error": {
            "message": format!("toolcase-gateway: {message}"),
            "type": "personal_module_error",
            "code": status
        }
    });
    let data = serde_json::to_vec(&error_body).unwrap_or_default();
    let reason = match status {
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        504 => "Gateway Timeout",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        data.len()
    );
    client.write_all(head.as_bytes())?;
    client.write_all(&data)?;
    client.flush()?;
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::cmd::build_fallback_interpreter_command;
    use super::*;

    #[test]
    fn test_write_personal_error_format() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (mut server, _) = listener.accept().unwrap();

        let status = write_personal_error(&mut server, 502, "test fault").unwrap();
        assert_eq!(status, 502);
        drop(server);

        let mut buf = Vec::new();
        io::Read::read_to_end(&mut client, &mut buf).unwrap();
        let response_str = String::from_utf8_lossy(&buf);
        assert!(response_str.contains("502 Bad Gateway"));
        assert!(response_str.contains("personal_module_error"));
    }

    #[test]
    fn test_build_fallback_interpreter_command() {
        let temp_dir = std::env::temp_dir().join(format!("test_fb_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let sh_file = temp_dir.join("test.sh");
        std::fs::write(&sh_file, "#!/bin/sh\necho test\n").unwrap();
        let py_file = temp_dir.join("test.py");
        std::fs::write(&py_file, "print('test')\n").unwrap();

        let manifest_sh = PersonalManifest {
            id: "test-sh".to_string(),
            name: "Test Sh".to_string(),
            short_name: None,
            version: "1.0.0".to_string(),
            namespace: "/custom/sh".to_string(),
            entrypoint: sh_file.to_string_lossy().to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec![],
            icon: None,
            config_schema: None,
            permissions: vec![],
            group: None,
            source: None,
            tools: None,
        };
        let cmd_sh = build_fallback_interpreter_command(&manifest_sh, None).expect("sh fallback");
        assert_eq!(cmd_sh.get_program(), "/bin/sh");

        let mut manifest_py = manifest_sh.clone();
        manifest_py.entrypoint = py_file.to_string_lossy().to_string();
        let cmd_py = build_fallback_interpreter_command(&manifest_py, None).expect("py fallback");
        assert_eq!(cmd_py.get_program(), "python3");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    #[cfg(unix)]
    fn test_heal_legacy_adapter_script_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let temp_dir = std::env::temp_dir().join(format!("test_heal_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let run_sh = temp_dir.join("run.sh");
        let legacy_content = "#!/bin/sh\ncat << 'EOF' | python3\nprint('hello')\nEOF\n";
        std::fs::write(&run_sh, legacy_content).unwrap();

        let manifest = PersonalManifest {
            id: "test-legacy".to_string(),
            name: "Test Legacy".to_string(),
            short_name: None,
            version: "1.0.0".to_string(),
            namespace: "/custom/legacy".to_string(),
            entrypoint: run_sh.to_string_lossy().to_string(),
            protocol: "stdio-stream".to_string(),
            capabilities: vec![],
            icon: None,
            config_schema: None,
            permissions: vec![],
            group: None,
            source: None,
            tools: None,
        };

        let _cmd = build_command(&manifest, None);

        let run_meta = std::fs::metadata(&run_sh).unwrap();
        assert_eq!(run_meta.permissions().mode() & 0o777, 0o700);

        let adapter_py = temp_dir.join("adapter.py");
        assert!(adapter_py.is_file());
        let py_meta = std::fs::metadata(&adapter_py).unwrap();
        assert_eq!(py_meta.permissions().mode() & 0o777, 0o700);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
