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
    let start = std::time::Instant::now();
    let peer_ip = client
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "-".into());

    let (request, _byte_guard) = match read_request_with_admission(&mut client, &store.admission())
    {
        Ok(request) => request,
        Err(error) if error.kind() == io::ErrorKind::Other => {
            let res = write_rejection_and_drain(
                &mut client,
                503,
                "Service Unavailable",
                "too many in-flight bytes",
            );
            crate::runtime::logger::log_access_with_status(
                "UNKNOWN",
                "-",
                &peer_ip,
                503,
                start.elapsed().as_millis() as u64,
            );
            return res;
        }
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
            let res = write_rejection_and_drain(
                &mut client,
                413,
                "Payload Too Large",
                &error.to_string(),
            );
            crate::runtime::logger::log_access_with_status(
                "UNKNOWN",
                "-",
                &peer_ip,
                413,
                start.elapsed().as_millis() as u64,
            );
            return res;
        }
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            let res =
                write_rejection_and_drain(&mut client, 400, "Bad Request", &error.to_string());
            crate::runtime::logger::log_access_with_status(
                "UNKNOWN",
                "-",
                &peer_ip,
                400,
                start.elapsed().as_millis() as u64,
            );
            return res;
        }
        Err(error) => return Err(error),
    };

    let result = route_request_dispatch(&mut client, &request, store);
    let duration_ms = start.elapsed().as_millis() as u64;
    let status = match &result {
        Ok(s) => *s,
        Err(_) => 500,
    };
    crate::runtime::logger::log_access_with_status(
        &request.method,
        &request.path,
        &peer_ip,
        status,
        duration_ms,
    );
    result.map(|_| ())
}

fn dispatch_dashboard_route(
    client: &mut TcpStream,
    request: &crate::http::Request,
    store: &ConfigStore,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    match clean_path {
        "/gateway-config/dashboard" => {
            if request.method.eq_ignore_ascii_case("get") {
                crate::features::dashboard::handle_dashboard_page(client)?;
                Ok(Some(200))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "dashboard only supports GET",
                )?;
                Ok(Some(405))
            }
        }
        "/api/v1/dashboard/overview" => {
            if request.method.eq_ignore_ascii_case("get") {
                crate::features::dashboard::handle_dashboard_overview(client, store)?;
                Ok(Some(200))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "dashboard overview only supports GET",
                )?;
                Ok(Some(405))
            }
        }
        "/api/v1/dashboard/accounts" => {
            if request.method.eq_ignore_ascii_case("post") {
                crate::features::dashboard::handle_add_provider_accounts(
                    client,
                    &request.body,
                    store,
                )?;
                Ok(Some(200))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "dashboard accounts only supports POST",
                )?;
                Ok(Some(405))
            }
        }
        "/api/v1/dashboard/config" => {
            if request.method.eq_ignore_ascii_case("post") {
                crate::features::dashboard::handle_update_gateway_config(
                    client,
                    &request.body,
                    store,
                )?;
                Ok(Some(200))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "dashboard config only supports POST",
                )?;
                Ok(Some(405))
            }
        }
        "/api/v1/dashboard/muse-config" => {
            if request.method.eq_ignore_ascii_case("post") {
                crate::features::dashboard::handle_update_muse_config(
                    client,
                    &request.body,
                    store,
                )?;
                Ok(Some(200))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "dashboard muse-config only supports POST",
                )?;
                Ok(Some(405))
            }
        }
        "/api/v1/dashboard/jev-config" => {
            if request.method.eq_ignore_ascii_case("post") {
                crate::features::dashboard::handle_update_jev_config(client, &request.body, store)?;
                Ok(Some(200))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "dashboard jev-config only supports POST",
                )?;
                Ok(Some(405))
            }
        }
        _ => Ok(None),
    }
}

fn dispatch_muse_config_route(
    client: &mut TcpStream,
    request: &crate::http::Request,
    store: &ConfigStore,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    match clean_path {
        "/muse-config" => {
            if request.method.eq_ignore_ascii_case("get") {
                crate::video_template::handle_muse_config_page(client)?;
                Ok(Some(200))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "muse-config only supports GET",
                )?;
                Ok(Some(405))
            }
        }
        "/muse-ai/v1/config" => {
            if request.method.eq_ignore_ascii_case("get") {
                let snapshot = store.snapshot();
                crate::har_config::write_current_config(client, &snapshot)?;
                Ok(Some(200))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "Muse configuration route only supports GET",
                )?;
                Ok(Some(405))
            }
        }
        "/muse-ai/v1/config/har" => {
            if request.method.eq_ignore_ascii_case("post") {
                crate::har_config::apply_har_config(client, &request.body, store)?;
                Ok(Some(200))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "Muse HAR configuration only supports POST",
                )?;
                Ok(Some(405))
            }
        }
        _ => Ok(None),
    }
}

fn route_request_dispatch(
    client: &mut TcpStream,
    request: &crate::http::Request,
    store: &ConfigStore,
) -> io::Result<u16> {
    let clean_path = request
        .path
        .split('?')
        .next()
        .unwrap_or("")
        .trim_end_matches('/');

    if request.method.eq_ignore_ascii_case("options") {
        handle_cors_preflight(client)?;
        return Ok(200);
    }

    // Runtime-config routes operate on the live store and never fall through to
    // the generic OmniRoute proxy.
    if let Some(code) = dispatch_dashboard_route(client, request, store, clean_path)? {
        return Ok(code);
    }
    if let Some(code) = dispatch_muse_config_route(client, request, store, clean_path)? {
        return Ok(code);
    }

    let snapshot = store.snapshot();
    route_request_impl(client, request, &snapshot, &store.jev_pool())
}

fn dispatch_catalog_route(
    client: &mut TcpStream,
    request: &crate::http::Request,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    let Some(platform) = catalog_route(clean_path) else {
        return Ok(None);
    };
    if request.method.eq_ignore_ascii_case("get") {
        handle_models_catalog(client, platform)?;
        Ok(Some(200))
    } else {
        write_error(
            client,
            405,
            "Method Not Allowed",
            "model catalogs only support GET",
        )?;
        Ok(Some(405))
    }
}

fn dispatch_muse_chat_route(
    client: &mut TcpStream,
    request: &crate::http::Request,
    config: &Config,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    if clean_path == "/muse-ai/v1/chat/completions" || clean_path == "/muse-ai/chat/completions" {
        return if request.method.eq_ignore_ascii_case("post") {
            crate::museai::handle_museai_chat_completions(client, &request.body, config)?;
            Ok(Some(200))
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "Muse chat completions only support POST",
            )?;
            Ok(Some(405))
        };
    }

    if clean_path == "/muse-ai/v1" {
        return if request.method.eq_ignore_ascii_case("post") {
            crate::museai::handle_museai_v1(client, &request.body, config)?;
            Ok(Some(200))
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "Muse endpoint only supports POST",
            )?;
            Ok(Some(405))
        };
    }

    if clean_path == "/muse-ai/v1/threads/cleanup" || clean_path == "/muse-ai/threads/cleanup" {
        return if request.method.eq_ignore_ascii_case("post") {
            crate::museai::handle_museai_threads_cleanup_run(
                client,
                config,
                request.body.as_slice(),
            )?;
            Ok(Some(200))
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "Muse threads cleanup only supports POST",
            )?;
            Ok(Some(405))
        };
    }

    if clean_path.starts_with("/muse-ai/v1/threads/") {
        return if request.method.eq_ignore_ascii_case("delete") {
            let thread_id = clean_path.split('/').next_back().unwrap_or("");
            crate::museai::handle_museai_thread_cleanup(client, thread_id, config)?;
            Ok(Some(200))
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "Muse thread cleanup only supports DELETE",
            )?;
            Ok(Some(405))
        };
    }

    Ok(None)
}

fn dispatch_muse_image_route(
    client: &mut TcpStream,
    request: &crate::http::Request,
    config: &Config,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    if clean_path == "/muse-ai/v1/create-image"
        || clean_path == "/muse-ai/create-image"
        || clean_path == "/v1/images/generations"
    {
        return if request.method.eq_ignore_ascii_case("post") {
            crate::museai::handle_create_image_request(client, request, config)?;
            Ok(Some(200))
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "Muse image creation only supports POST",
            )?;
            Ok(Some(405))
        };
    }

    if clean_path.starts_with("/muse-ai/v1/images/") {
        let sub = clean_path.strip_prefix("/muse-ai/v1/images/").unwrap_or("");
        if let Some(job_id) = sub
            .strip_suffix("/content")
            .or_else(|| sub.strip_suffix("/download"))
        {
            let allowed = request.method.eq_ignore_ascii_case("get")
                || request.method.eq_ignore_ascii_case("head");
            return if allowed {
                crate::museai::handle_get_image_content(client, job_id, request)?;
                Ok(Some(200))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "Muse image content only supports GET and HEAD",
                )?;
                Ok(Some(405))
            };
        }

        let job_id = sub.trim_matches('/');
        return if request.method.eq_ignore_ascii_case("get")
            || request.method.eq_ignore_ascii_case("head")
        {
            crate::museai::handle_get_image_request(client, job_id, request)?;
            Ok(Some(200))
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "Muse image job status only supports GET and HEAD",
            )?;
            Ok(Some(405))
        };
    }

    Ok(None)
}

fn dispatch_muse_video_route(
    client: &mut TcpStream,
    request: &crate::http::Request,
    config: &Config,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    if clean_path == "/muse-ai/v1/create-video" {
        return if request.method.eq_ignore_ascii_case("post") {
            crate::museai::handle_create_video_request(client, request, config)?;
            Ok(Some(200))
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "Muse video creation only supports POST",
            )?;
            Ok(Some(405))
        };
    }

    if clean_path.starts_with("/muse-ai/v1/videos/") {
        let sub = clean_path.strip_prefix("/muse-ai/v1/videos/").unwrap_or("");
        if let Some(job_id) = sub
            .strip_suffix("/content")
            .or_else(|| sub.strip_suffix("/download"))
        {
            let allowed = request.method.eq_ignore_ascii_case("get")
                || request.method.eq_ignore_ascii_case("head");
            return if allowed {
                crate::museai::handle_get_video_content(client, job_id, request)?;
                Ok(Some(200))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "video content only supports GET and HEAD",
                )?;
                Ok(Some(405))
            };
        }
        if request.method.eq_ignore_ascii_case("put") {
            crate::museai::handle_update_video_prompt(client, sub, &request.body)?;
            return Ok(Some(200));
        }
        if request.method.eq_ignore_ascii_case("get") {
            crate::museai::handle_get_video_request(client, sub, request)?;
            return Ok(Some(200));
        }
        write_error(
            client,
            405,
            "Method Not Allowed",
            "video route only supports GET and PUT",
        )?;
        return Ok(Some(405));
    }

    if clean_path == "/muse-ai" || clean_path.starts_with("/muse-ai/") {
        write_error(client, 404, "Not Found", "unknown Muse route")?;
        return Ok(Some(404));
    }

    Ok(None)
}

fn dispatch_jev_route(
    client: &mut TcpStream,
    request: &crate::http::Request,
    config: &Config,
    jev_pool: &crate::providers::jev::accounts::JevAccountPool,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    if clean_path == "/jev/v1/models"
        || clean_path == "/jev/models"
        || clean_path == "/jev-ai/v1/models"
    {
        return if request.method.eq_ignore_ascii_case("get") {
            crate::jev::handle_jev_models_catalog(client, request, config, jev_pool).map(Some)
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "Jev models catalog only supports GET",
            )?;
            Ok(Some(405))
        };
    }
    let is_jev_v1 = clean_path == "/jev/v1"
        || clean_path.starts_with("/jev/v1/")
        || clean_path == "/jev/decision"
        || clean_path == "/jev/decisions"
        || clean_path.starts_with("/jev/decision/")
        || clean_path.starts_with("/jev/decisions/");
    let is_jev_ai_v1 = clean_path == "/jev-ai/v1" || clean_path.starts_with("/jev-ai/v1/");

    if is_jev_v1 || is_jev_ai_v1 {
        let expected = crate::providers::jev::adapter::expected_jev_method(clean_path);
        if expected.is_some()
            && expected.is_some_and(|method| request.method.eq_ignore_ascii_case(method))
        {
            return crate::jev::handle_jev_ai(client, request, config, jev_pool).map(Some);
        }
        write_error(
            client,
            405,
            "Method Not Allowed",
            "Jev Decision API routes: GET /requests/{id}, POST /systemone, /cancel",
        )?;
        return Ok(Some(405));
    }

    Ok(None)
}

fn dispatch_video_provider_route(
    client: &mut TcpStream,
    request: &crate::http::Request,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    if clean_path == "/video/v1/providers" {
        return if request.method.eq_ignore_ascii_case("get") {
            crate::features::video::handle_video_providers_catalog(client)?;
            Ok(Some(200))
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "video providers catalog only supports GET",
            )?;
            Ok(Some(405))
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
            crate::features::video::handle_video_task_submit(client, &request.body, provider)?;
            Ok(Some(200))
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "video task submit only supports POST",
            )?;
            Ok(Some(405))
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
            crate::features::video::handle_video_task_reconciliation(client, provider, key)?;
            Ok(Some(200))
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "reconciliation lookup only supports GET",
            )?;
            Ok(Some(405))
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
            crate::features::video::handle_video_task_status(client, provider, task_id)?;
            Ok(Some(200))
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "video task status only supports GET",
            )?;
            Ok(Some(405))
        };
    }

    Ok(None)
}

fn dispatch_template_route(
    client: &mut TcpStream,
    request: &crate::http::Request,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    if clean_path == "/video-template" || clean_path.starts_with("/video-template/") {
        return if request.method.eq_ignore_ascii_case("get")
            || request.method.eq_ignore_ascii_case("head")
        {
            crate::video_template::handle_video_template_page_with_method(client, &request.method)?;
            Ok(Some(200))
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "video templates only support GET and HEAD",
            )?;
            Ok(Some(405))
        };
    }

    if clean_path == "/image-template" || clean_path.starts_with("/image-template/") {
        return if request.method.eq_ignore_ascii_case("get")
            || request.method.eq_ignore_ascii_case("head")
        {
            crate::video_template::handle_image_template_page_with_method(client, &request.method)?;
            Ok(Some(200))
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "image templates only support GET and HEAD",
            )?;
            Ok(Some(405))
        };
    }

    Ok(None)
}

fn route_request_impl(
    client: &mut TcpStream,
    request: &crate::http::Request,
    config: &Config,
    jev_pool: &crate::providers::jev::accounts::JevAccountPool,
) -> io::Result<u16> {
    let clean_path = request
        .path
        .split('?')
        .next()
        .unwrap_or("")
        .trim_end_matches('/');

    if let Some(code) = dispatch_catalog_route(client, request, clean_path)? {
        return Ok(code);
    }
    if let Some(code) = dispatch_muse_chat_route(client, request, config, clean_path)? {
        return Ok(code);
    }
    if let Some(code) = dispatch_muse_image_route(client, request, config, clean_path)? {
        return Ok(code);
    }
    if let Some(code) = dispatch_muse_video_route(client, request, config, clean_path)? {
        return Ok(code);
    }
    if let Some(code) = dispatch_jev_route(client, request, config, jev_pool, clean_path)? {
        return Ok(code);
    }
    if let Some(code) = dispatch_video_provider_route(client, request, clean_path)? {
        return Ok(code);
    }
    if let Some(code) = dispatch_template_route(client, request, clean_path)? {
        return Ok(code);
    }

    handle_omniroute_proxy(client, request, config)
}
