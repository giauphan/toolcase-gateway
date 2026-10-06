use crate::config::{Config, ConfigStore};
use crate::http::read_request;
use crate::omniroute::handle_omniroute_proxy;
use crate::prism::handle_prism_chat_completion;
use std::io::{self, Write};
use std::net::TcpStream;

pub(crate) fn is_models_catalog_route(path: &str) -> bool {
    path.ends_with("/v1/models") || path.ends_with("/models")
}

pub(crate) fn is_prism_completions_route(path: &str) -> bool {
    path == "/prism-openai/v1/chat/completions"
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

pub(crate) fn handle_models_catalog(
    client: &mut TcpStream,
    clean_path: &str,
    config: &Config,
) -> io::Result<()> {
    let mut model_entries = Vec::new();

    if clean_path.starts_with("/muse-ai") {
        model_entries.push(
            r#"{"id":"muse","object":"model","created":1700000000,"owned_by":"system"}"#
                .to_string(),
        );
    } else {
        // Detail the configured OmniRoute pipeline: the default upstream model,
        // every configured fallback, and the local Muse provider.
        let mut base_models: Vec<String> = Vec::new();
        if !config.prism_default_model.trim().is_empty() {
            base_models.push(config.prism_default_model.trim().to_string());
        }
        for fallback in &config.fallbacks {
            if !fallback.trim().is_empty() {
                base_models.push(fallback.trim().to_string());
            }
        }
        base_models.push("muse".to_string());
        base_models.sort();
        base_models.dedup();

        let efforts = ["low", "medium", "high", "xhigh"];

        for bm in &base_models {
            model_entries.push(format!(
                r#"{{"id":"{}","object":"model","created":1700000000,"owned_by":"system"}}"#,
                bm
            ));
            if bm.starts_with("gpt-5.6-") {
                for effort in efforts {
                    model_entries.push(format!(
                        r#"{{"id":"{}-{}","object":"model","created":1700000000,"owned_by":"system"}}"#,
                        bm, effort
                    ));
                }
            }
        }
    }

    let body = format!(
        r#"{{"object":"list","data":[{}]}}"#,
        model_entries.join(",")
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\n\
        Content-Type: application/json\r\n\
        Access-Control-Allow-Origin: *\r\n\
        Content-Length: {}\r\n\
        Connection: close\r\n\r\n{}",
        body.len(),
        body
    );
    client.write_all(response.as_bytes())?;
    client.flush()
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

    // Runtime-config routes operate on the live store.
    if request.method.eq_ignore_ascii_case("get") && clean_path == "/muse-config" {
        return crate::video_template::handle_muse_config_page(&mut client);
    }
    if request.method.eq_ignore_ascii_case("post") && clean_path == "/muse-ai/v1/config/har" {
        return crate::har_config::apply_har_config(&mut client, &request.body, store);
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

    if request.method.eq_ignore_ascii_case("get") && is_models_catalog_route(clean_path) {
        return handle_models_catalog(client, clean_path, config);
    }

    if is_prism_completions_route(clean_path) {
        return handle_prism_chat_completion(client, &request.body, config, &request.headers);
    }

    if clean_path == "/muse-ai/v1" {
        return crate::museai::handle_museai_v1(client, &request.body, config);
    }

    if clean_path == "/muse-ai/v1/create-video" && request.method.eq_ignore_ascii_case("post") {
        return crate::museai::handle_create_video(client, &request.body, config);
    }

    // Thread cleanup endpoint
    if clean_path.starts_with("/muse-ai/v1/threads/")
        && request.method.eq_ignore_ascii_case("delete")
    {
        let thread_id = clean_path.split('/').next_back().unwrap_or("");
        return crate::museai::handle_museai_thread_cleanup(client, thread_id, config);
    }

    // Video template UI page (also serve on /video-template/<name>)
    if request.method.eq_ignore_ascii_case("get")
        && (clean_path == "/video-template" || clean_path.starts_with("/video-template/"))
    {
        return crate::video_template::handle_video_template_page(client);
    }

    handle_omniroute_proxy(client, request, config)
}
