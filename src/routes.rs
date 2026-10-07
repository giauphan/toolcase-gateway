use crate::config::{Config, ConfigStore};
use crate::http::{read_request, write_error};
use crate::omniroute::handle_omniroute_proxy;
use std::io::{self, Write};
use std::net::TcpStream;

/// Checks if the path is for the main API models catalog
pub(crate) fn is_main_models_catalog_route(path: &str) -> bool {
    matches!(path, "/models" | "/v1/models")
}

/// Checks if the path is for the Muse-AI models catalog
/// Checks if the path is for the Muse-AI models catalog
pub(crate) fn is_muse_models_catalog_route(path: &str) -> bool {
    matches!(path, "/muse-ai/models" | "/muse-ai/v1/models")
}

pub(crate) fn handle_cors_preflight(client: &mut TcpStream) -> io::Result<()> {
    let response = "HTTP/1.1 200 OK\r\n\
                    Access-Control-Allow-Origin: *\r\n\
                    Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
                    Access-Control-Allow-Headers: *\r\n\
                    Content-Length: 0\r\n\
                    Connection: close\r\n\r\n";
    client.write_all(response.as_bytes())?;
    client.flush()
}

/// Handles the models catalog for the main API
fn handle_main_models_catalog(client: &mut TcpStream, config: &Config) -> io::Result<()> {
    let mut model_entries = Vec::new();

    // Detail the configured OmniRoute pipeline: the default upstream model
    // and every configured fallback.
    let mut base_models: Vec<String> = Vec::new();
    if !config.default_model.trim().is_empty() {
        base_models.push(config.default_model.trim().to_string());
    }
    for fallback in &config.fallbacks {
        if !fallback.trim().is_empty() {
            base_models.push(fallback.trim().to_string());
        }
    }
    base_models.sort();
    base_models.dedup();

    // Generate model entries for each base model
    for bm in &base_models {
        model_entries.push(format!(
            r#"{{"id":"{}","object":"model","created":1700000000,"owned_by":"system"}}"#,
            bm
        ));
    }

    let body = format!(
        r#"{{"object":"list","data":[{}]}}"#,
        model_entries.join(",")
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    client.write_all(response.as_bytes())?;
    client.flush()
}

/// Handles the models catalog for the Muse-AI service
fn handle_muse_models_catalog(client: &mut TcpStream) -> io::Result<()> {
    let mut model_entries = Vec::new();

    // For now, return an empty list of models
    // TODO: Implement proper Muse-AI models handling when ready
    let body = format!(
        r#"{{"object":"list","data":[{}]}}"#,
        model_entries.join(",")
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    client.write_all(response.as_bytes())?;
    client.flush()
}

pub(crate) fn handle_models_catalog(
    client: &mut TcpStream,
    clean_path: &str,
    config: &Config,
) -> io::Result<()> {
    // Route to the appropriate handler based on the path pattern
    if is_main_models_catalog_route(clean_path) {
        handle_main_models_catalog(client, config)
    } else if is_muse_models_catalog_route(clean_path) {
        handle_muse_models_catalog(client)
    } else {
        // Fallback to main models catalog if no specific handler found
        handle_main_models_catalog(client, config)
    }
}

pub(crate) fn route_request(mut client: TcpStream, store: &ConfigStore) -> io::Result<()> {
    let request = read_request(&mut client)?;
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
    route_request_impl(&mut client, &request, &snapshot)
}

fn route_request_impl(
    client: &mut TcpStream,
    request: &crate::http::Request,
    config: &Config,
) -> io::Result<()> {
    let clean_path = request
        .path
        .split('?')
        .next()
        .unwrap_or("")
        .trim_end_matches('/');

    if is_main_models_catalog_route(clean_path) {
        // Updated to use new function name
        return if request.method.eq_ignore_ascii_case("get") {
            handle_models_catalog(client, clean_path, config)
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
