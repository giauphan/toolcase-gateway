//! Dashboard and configuration route dispatcher.

use crate::config::ConfigStore;
use crate::http::write_error;
use std::io;
use std::net::TcpStream;

pub fn dispatch_dashboard_route(
    client: &mut TcpStream,
    request: &crate::http::Request,
    store: &ConfigStore,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    match clean_path {
        "" | "/" | "/dashboard" | "/gateway-config/dashboard" => {
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
                if !validate_dashboard_mutation_request(client, request)? {
                    return Ok(Some(403));
                }
                crate::features::dashboard::handle_save_provider_settings(
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
                if !validate_dashboard_mutation_request(client, request)? {
                    return Ok(Some(403));
                }
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
                if !validate_dashboard_mutation_request(client, request)? {
                    return Ok(Some(403));
                }
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
                if !validate_dashboard_mutation_request(client, request)? {
                    return Ok(Some(403));
                }
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
        "/api/v1/dashboard/modules/state" => {
            if request.method.eq_ignore_ascii_case("post") {
                if !validate_dashboard_mutation_request(client, request)? {
                    return Ok(Some(403));
                }
                crate::features::dashboard::handle_set_module_state(client, &request.body, store)?;
                Ok(Some(200))
            } else {
                write_error(
                    client,
                    405,
                    "Method Not Allowed",
                    "dashboard modules/state only supports POST",
                )?;
                Ok(Some(405))
            }
        }
        "/api/v1/dashboard/modules/install" => {
            if request.method.eq_ignore_ascii_case("post") {
                if !validate_dashboard_mutation_request(client, request)? {
                    return Ok(Some(403));
                }
                crate::features::dashboard::handle_install_personal_module(
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
                    "dashboard modules/install only supports POST",
                )?;
                Ok(Some(405))
            }
        }
        "/api/v1/dashboard/modules/remove" | "/api/v1/dashboard/modules/uninstall" => {
            if request.method.eq_ignore_ascii_case("post") {
                if !validate_dashboard_mutation_request(client, request)? {
                    return Ok(Some(403));
                }
                crate::features::dashboard::handle_remove_personal_module(
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
                    "dashboard modules/remove only supports POST",
                )?;
                Ok(Some(405))
            }
        }
        _ => Ok(None),
    }
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
