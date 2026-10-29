//! Dashboard and configuration route dispatcher.

use crate::config::ConfigStore;
use crate::http::write_error;
use std::io::{self, Write};
use std::net::TcpStream;

pub fn dispatch_dashboard_route(
    client: &mut TcpStream,
    request: &crate::http::Request,
    store: &ConfigStore,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    if clean_path.starts_with("/gateway-ui/") {
        if !request.method.eq_ignore_ascii_case("get")
            && !request.method.eq_ignore_ascii_case("head")
        {
            write_error(
                client,
                405,
                "Method Not Allowed",
                "gateway UI assets only support GET and HEAD",
            )?;
            return Ok(Some(405));
        }
        if crate::features::dashboard::handle_dashboard_asset(
            client,
            clean_path,
            request.method.eq_ignore_ascii_case("get"),
        )? {
            return Ok(Some(200));
        }
        return Ok(None);
    }
    match clean_path {
        "" | "/" | "/gateway-config/dashboard" => {
            if request.method.eq_ignore_ascii_case("get")
                || request.method.eq_ignore_ascii_case("head")
            {
                let response = "HTTP/1.1 302 Found\r\nLocation: /dashboard\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                client.write_all(response.as_bytes())?;
                client.flush()?;
                Ok(Some(302))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "root endpoint only supports GET and HEAD",
                )?;
                Ok(Some(405))
            }
        }
        "/sw.js" => {
            let has_omni_context = request.path.contains('?')
                || crate::http::header_value(&request.headers, "referer")
                    .is_some_and(|r| r.contains("/omniroute") || r.contains("/ominroute"))
                || crate::http::header_value(&request.headers, "cookie")
                    .is_some_and(|c| c.contains("omniroute") || c.contains("ominroute"));

            if has_omni_context {
                let omni_enabled = store
                    .module_registry()
                    .read()
                    .ok()
                    .and_then(|reg| reg.get_module("omniroute").map(|m| m.status))
                    .is_some_and(|status| status == crate::gateway::modules::ModuleStatus::Enabled);

                if omni_enabled {
                    let snapshot = store.snapshot();
                    let status = crate::routes::modules::dispatch_builtin_omniroute(
                        client, request, &snapshot, "/sw.js",
                    )?;
                    return Ok(Some(status));
                }
            }

            let response = "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            client.write_all(response.as_bytes())?;
            client.flush()?;
            Ok(Some(200))
        }
        "/dashboard" => handle_get(
            client,
            request,
            "dashboard",
            crate::features::dashboard::handle_dashboard_page,
        ),
        "/api/v1/dashboard/overview" => handle_get(client, request, "dashboard overview", |c| {
            crate::features::dashboard::handle_dashboard_overview(c, store)
        }),
        "/api/v1/dashboard/accounts" => {
            handle_post(client, request, "dashboard accounts", |c, b| {
                crate::features::dashboard::handle_save_provider_settings(c, b, store)
            })
        }
        "/api/v1/dashboard/config" => handle_post(client, request, "dashboard config", |c, b| {
            crate::features::dashboard::handle_update_gateway_config(c, b, store)
        }),
        "/api/v1/dashboard/muse-config" => {
            handle_post(client, request, "dashboard muse-config", |c, b| {
                crate::features::dashboard::handle_update_muse_config(c, b, store)
            })
        }
        "/api/v1/dashboard/jev-config" => {
            handle_post(client, request, "dashboard jev-config", |c, b| {
                crate::features::dashboard::handle_update_jev_config(c, b, store)
            })
        }
        "/api/v1/dashboard/modules/state" => {
            handle_post(client, request, "dashboard modules/state", |c, b| {
                crate::features::dashboard::handle_set_module_state(c, b, store)
            })
        }
        "/api/v1/dashboard/modules/install" => {
            handle_post(client, request, "dashboard modules/install", |c, b| {
                crate::features::dashboard::handle_install_personal_module(c, b, store)
            })
        }
        "/api/v1/dashboard/modules/remove" | "/api/v1/dashboard/modules/uninstall" => {
            handle_post(client, request, "dashboard modules/remove", |c, b| {
                crate::features::dashboard::handle_remove_personal_module(c, b, store)
            })
        }
        _ => Ok(None),
    }
}

fn handle_get<F>(
    client: &mut TcpStream,
    request: &crate::http::Request,
    route_name: &str,
    handler: F,
) -> io::Result<Option<u16>>
where
    F: FnOnce(&mut TcpStream) -> io::Result<()>,
{
    if !request.method.eq_ignore_ascii_case("get") {
        write_error(
            client,
            405,
            "Method Not Allowed",
            &format!("{route_name} only supports GET"),
        )?;
        return Ok(Some(405));
    }
    handler(client)?;
    Ok(Some(200))
}

fn handle_post<F>(
    client: &mut TcpStream,
    request: &crate::http::Request,
    route_name: &str,
    handler: F,
) -> io::Result<Option<u16>>
where
    F: FnOnce(&mut TcpStream, &[u8]) -> io::Result<()>,
{
    if !request.method.eq_ignore_ascii_case("post") {
        write_error(
            client,
            405,
            "Method Not Allowed",
            &format!("{route_name} only supports POST"),
        )?;
        return Ok(Some(405));
    }
    if !validate_dashboard_mutation_request(client, request)? {
        return Ok(Some(403));
    }
    handler(client, &request.body)?;
    Ok(Some(200))
}

fn validate_dashboard_mutation_request(
    client: &mut TcpStream,
    request: &crate::http::Request,
) -> io::Result<bool> {
    if let Some((_, origin)) = request
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("origin"))
    {
        if !crate::http::is_loopback_origin(origin) {
            write_error(
                client,
                403,
                "Forbidden",
                "Cross-origin requests from untrusted origins are prohibited",
            )?;
            return Ok(false);
        }
        let ct_ok = request
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .is_some_and(|(_, ct)| ct.to_ascii_lowercase().starts_with("application/json"));
        if !ct_ok {
            write_error(
                client,
                415,
                "Unsupported Media Type",
                "Endpoint expects Content-Type: application/json",
            )?;
            return Ok(false);
        }
    } else if let Some((_, ct)) = request
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        if !ct.to_ascii_lowercase().starts_with("application/json") {
            write_error(
                client,
                415,
                "Unsupported Media Type",
                "Endpoint expects Content-Type: application/json",
            )?;
            return Ok(false);
        }
    }

    Ok(true)
}

pub fn dispatch_muse_config_route(
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
                    "muse config only supports GET",
                )?;
                Ok(Some(405))
            }
        }
        "/muse-ai/v1/config/har" | "/api/v1/muse-config/har" => {
            if request.method.eq_ignore_ascii_case("post") {
                crate::features::har::apply_har_config(client, &request.body, store)?;
                Ok(Some(200))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "muse config HAR ingestion only supports POST",
                )?;
                Ok(Some(405))
            }
        }
        "/muse-ai/v1/config" | "/api/v1/muse-config/current" => {
            if request.method.eq_ignore_ascii_case("get") {
                crate::features::har::write_current_config(client, &store.snapshot())?;
                Ok(Some(200))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "muse config read only supports GET",
                )?;
                Ok(Some(405))
            }
        }
        _ => Ok(None),
    }
}
