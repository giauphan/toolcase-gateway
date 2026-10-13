use crate::config::{Config, ConfigStore};
use crate::http::{read_request_with_admission, write_error};
use crate::omniroute::handle_omniroute_proxy;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogPlatform {
    Muse,
}

pub fn catalog_route(path: &str) -> Option<CatalogPlatform> {
    match path {
        "/muse-ai/models" | "/muse-ai/v1/models" => Some(CatalogPlatform::Muse),
        _ => None,
    }
}

pub fn handle_cors_preflight(client: &mut TcpStream) -> io::Result<()> {
    let response = "HTTP/1.1 200 OK\r\n\
                    Access-Control-Allow-Origin: *\r\n\
                    Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
                    Access-Control-Allow-Headers: *\r\n\
                    Content-Length: 0\r\n\
                    Connection: close\r\n\r\n";
    client.write_all(response.as_bytes())?;
    client.flush()
}

/// Handles the models catalog for the Muse-AI service
fn handle_muse_models_catalog(client: &mut TcpStream) -> io::Result<()> {
    let body = r#"{"object":"list","data":[{"id":"muse","object":"model","created":1700000000,"owned_by":"system"}]}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    client.write_all(response.as_bytes())?;
    client.flush()
}

pub fn handle_models_catalog(client: &mut TcpStream, platform: CatalogPlatform) -> io::Result<()> {
    match platform {
        CatalogPlatform::Muse => handle_muse_models_catalog(client),
    }
}

fn write_rejection_and_drain(
    client: &mut TcpStream,
    status: u16,
    reason: &str,
    message: &str,
) -> io::Result<()> {
    write_error(client, status, reason, message)?;
    client.flush()?;
    let _ = client.shutdown(Shutdown::Write);
    let _ = client.set_read_timeout(Some(Duration::from_millis(50)));
    let mut remaining = 64 * 1024;
    let mut buffer = [0u8; 8192];
    while remaining > 0 {
        let read_len = remaining.min(buffer.len());
        match client.read(&mut buffer[..read_len]) {
            Ok(0) | Err(_) => break,
            Ok(count) => remaining -= count,
        }
    }
    Ok(())
}

pub fn route_request(mut client: TcpStream, store: &ConfigStore) -> io::Result<()> {
    let (request, _byte_guard) = match read_request_with_admission(&mut client, &store.admission())
    {
        Ok(request) => request,
        Err(error) if error.kind() == io::ErrorKind::Other => {
            return write_rejection_and_drain(
                &mut client,
                503,
                "Service Unavailable",
                "too many in-flight bytes",
            );
        }
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
            return write_rejection_and_drain(
                &mut client,
                413,
                "Payload Too Large",
                &error.to_string(),
            );
        }
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return write_rejection_and_drain(&mut client, 400, "Bad Request", &error.to_string());
        }
        Err(error) => return Err(error),
    };
    let clean_path = request
        .path
        .split('?')
        .next()
        .unwrap_or("")
        .trim_end_matches('/');

    if request.method.eq_ignore_ascii_case("options") {
        return handle_cors_preflight(&mut client);
    }

    // Runtime-config routes operate on the live store and never fall through to
    // the generic OmniRoute proxy.
    if clean_path == "/muse-config" {
        return if request.method.eq_ignore_ascii_case("get") {
            crate::video_template::handle_muse_config_page(&mut client)
        } else {
            write_error(
                &mut client,
                405,
                "Method Not Allowed",
                "muse-config only supports GET",
            )
        };
    }
    if clean_path == "/muse-ai/v1/config" {
        return if request.method.eq_ignore_ascii_case("get") {
            let snapshot = store.snapshot();
            crate::har_config::write_current_config(&mut client, &snapshot)
        } else {
            write_error(
                &mut client,
                405,
                "Method Not Allowed",
                "Muse configuration route only supports GET",
            )
        };
    }
    if clean_path == "/muse-ai/v1/config/har" {
        return if request.method.eq_ignore_ascii_case("post") {
            crate::har_config::apply_har_config(&mut client, &request.body, store)
        } else {
            write_error(
                &mut client,
                405,
                "Method Not Allowed",
                "Muse HAR configuration only supports POST",
            )
        };
    }

    let snapshot = store.snapshot();
    route_request_impl(&mut client, &request, &snapshot, &store.jev_pool())
}

fn route_request_impl(
    client: &mut TcpStream,
    request: &crate::http::Request,
    config: &Config,
    jev_pool: &crate::providers::jev::accounts::JevAccountPool,
) -> io::Result<()> {
    let clean_path = request
        .path
        .split('?')
        .next()
        .unwrap_or("")
        .trim_end_matches('/');

    if let Some(platform) = catalog_route(clean_path) {
        return if request.method.eq_ignore_ascii_case("get") {
            handle_models_catalog(client, platform)
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "model catalogs only support GET",
            )
        };
    }

    if clean_path == "/muse-ai/v1" {
        return if request.method.eq_ignore_ascii_case("post") {
            crate::museai::handle_museai_v1(client, &request.body, config)
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "Muse chat completions only support POST",
            )
        };
    }

    if clean_path == "/muse-ai/v1/create-video" {
        return if request.method.eq_ignore_ascii_case("post") {
            crate::museai::handle_create_video(client, &request.body, config)
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "Muse video creation only supports POST",
            )
        };
    }

    if clean_path.starts_with("/muse-ai/v1/threads/") {
        return if request.method.eq_ignore_ascii_case("delete") {
            let thread_id = clean_path.split('/').next_back().unwrap_or("");
            crate::museai::handle_museai_thread_cleanup(client, thread_id, config)
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "Muse thread cleanup only supports DELETE",
            )
        };
    }
    if clean_path == "/muse-ai" || clean_path.starts_with("/muse-ai/") {
        return write_error(client, 404, "Not Found", "unknown Muse route");
    }

    if clean_path == "/jev/v1/models"
        || clean_path == "/jev/models"
        || clean_path == "/jev-ai/v1/models"
    {
        return if request.method.eq_ignore_ascii_case("get") {
            crate::jev::handle_jev_models_catalog(client, request, config, jev_pool)
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "Jev models catalog only supports GET",
            )
        };
    }
    if clean_path == "/jev/v1" || clean_path.starts_with("/jev/v1/") {
        let expected = crate::providers::jev::adapter::expected_jev_method(clean_path);
        if expected.is_some()
            && expected.is_some_and(|method| request.method.eq_ignore_ascii_case(method))
        {
            return crate::jev::handle_jev_ai(client, request, config, jev_pool);
        }
        return write_error(
            client,
            405,
            "Method Not Allowed",
            "Jev Decision API routes: GET /requests/{id}, POST /systemone, /cancel",
        );
    }
    if clean_path == "/jev-ai/v1" || clean_path.starts_with("/jev-ai/v1/") {
        let expected = crate::providers::jev::adapter::expected_jev_method(clean_path);
        if expected.is_some()
            && expected.is_some_and(|method| request.method.eq_ignore_ascii_case(method))
        {
            return crate::jev::handle_jev_ai(client, request, config, jev_pool);
        }
        return write_error(
            client,
            405,
            "Method Not Allowed",
            "Jev Decision API routes: GET /requests/{id}, POST /systemone, /cancel",
        );
    }

    if clean_path == "/video/v1/providers" {
        return if request.method.eq_ignore_ascii_case("get") {
            crate::features::video::handle_video_providers_catalog(client)
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "video providers catalog only supports GET",
            )
        };
    }

    let parts: Vec<&str> = clean_path.split('/').collect();
    if parts.len() == 6
        && parts.get(1) == Some(&"video")
        && parts.get(2) == Some(&"v1")
        && parts.get(3) == Some(&"providers")
        && parts.get(5) == Some(&"tasks")
    {
        let provider = parts.get(4).copied().unwrap_or("");
        return if request.method.eq_ignore_ascii_case("post") {
            crate::features::video::handle_video_task_submit(client, &request.body, provider)
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "video task submit only supports POST",
            )
        };
    }

    if parts.len() == 8
        && parts.get(1) == Some(&"video")
        && parts.get(2) == Some(&"v1")
        && parts.get(3) == Some(&"providers")
        && parts.get(5) == Some(&"tasks")
        && parts.get(6) == Some(&"by-key")
    {
        let provider = parts.get(4).copied().unwrap_or("");
        let key = parts.get(7).copied().unwrap_or("");
        return if request.method.eq_ignore_ascii_case("get") {
            crate::features::video::handle_video_task_reconciliation(client, provider, key)
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "reconciliation lookup only supports GET",
            )
        };
    }

    if parts.len() == 7
        && parts.get(1) == Some(&"video")
        && parts.get(2) == Some(&"v1")
        && parts.get(3) == Some(&"providers")
        && parts.get(5) == Some(&"tasks")
    {
        let provider = parts.get(4).copied().unwrap_or("");
        let task_id = parts.get(6).copied().unwrap_or("");
        return if request.method.eq_ignore_ascii_case("get") {
            crate::features::video::handle_video_task_status(client, provider, task_id)
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "video task status only supports GET",
            )
        };
    }
    if clean_path == "/video-template" || clean_path.starts_with("/video-template/") {
        return if request.method.eq_ignore_ascii_case("get") {
            crate::video_template::handle_video_template_page(client)
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "video templates only support GET",
            )
        };
    }

    handle_omniroute_proxy(client, request, config)
}
