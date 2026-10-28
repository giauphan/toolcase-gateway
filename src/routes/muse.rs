//! Muse AI provider route dispatcher.

use crate::config::Config;
use crate::http::write_error;
use std::io::{self, Write};
use std::net::TcpStream;

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

pub fn dispatch_catalog_route(
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

pub fn dispatch_muse_chat_route(
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

fn handle_media_content_request<F>(
    client: &mut TcpStream,
    request: &crate::http::Request,
    sub: &str,
    media_name: &str,
    handler: F,
) -> io::Result<Option<u16>>
where
    F: FnOnce(&mut TcpStream, &str, &crate::http::Request) -> io::Result<()>,
{
    if let Some(job_id) = sub
        .strip_suffix("/content")
        .or_else(|| sub.strip_suffix("/download"))
    {
        let allowed = request.method.eq_ignore_ascii_case("get")
            || request.method.eq_ignore_ascii_case("head");
        return if allowed {
            handler(client, job_id, request)?;
            Ok(Some(200))
        } else {
            write_error(
                client,
                405,
                "Method Not Allowed",
                &format!("{media_name} only supports GET and HEAD"),
            )?;
            Ok(Some(405))
        };
    }
    Ok(None)
}

pub fn dispatch_muse_image_route(
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
        if let Some(status) = handle_media_content_request(
            client,
            request,
            sub,
            "Muse image content",
            crate::museai::handle_get_image_content,
        )? {
            return Ok(Some(status));
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

pub fn dispatch_muse_video_route(
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
        if let Some(status) = handle_media_content_request(
            client,
            request,
            sub,
            "video content",
            crate::museai::handle_get_video_content,
        )? {
            return Ok(Some(status));
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

pub fn dispatch_video_provider_route(
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
    if parts.len() >= 6
        && parts.get(1) == Some(&"video")
        && parts.get(2) == Some(&"v1")
        && parts.get(3) == Some(&"providers")
        && parts.get(5) == Some(&"tasks")
    {
        let provider = parts.get(4).copied().unwrap_or("");
        if parts.len() == 6 {
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
        if parts.len() == 7 {
            let task_id = parts.get(6).copied().unwrap_or("");
            return handle_get_only_route(client, request, "video task status", |c| {
                crate::features::video::handle_video_task_status(c, provider, task_id)
            });
        }
        if parts.len() == 8 && parts.get(6) == Some(&"by-key") {
            let key = parts.get(7).copied().unwrap_or("");
            return handle_get_only_route(client, request, "reconciliation lookup", |c| {
                crate::features::video::handle_video_task_reconciliation(c, provider, key)
            });
        }
    }

    Ok(None)
}

fn handle_get_only_route<F>(
    client: &mut TcpStream,
    request: &crate::http::Request,
    action_desc: &str,
    handler: F,
) -> io::Result<Option<u16>>
where
    F: FnOnce(&mut TcpStream) -> io::Result<()>,
{
    if request.method.eq_ignore_ascii_case("get") {
        handler(client)?;
        Ok(Some(200))
    } else {
        write_error(
            client,
            405,
            "Method Not Allowed",
            &format!("{action_desc} only supports GET"),
        )?;
        Ok(Some(405))
    }
}
