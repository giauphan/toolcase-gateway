use crate::config::Config;
use crate::http::read_request;
use crate::omniroute::handle_omniroute_proxy;
use crate::prism::{extract_credentials, handle_prism_chat_completion};
use std::io::{self, Write};
use std::net::TcpStream;

pub(crate) fn is_models_catalog_route(path: &str) -> bool {
    path.ends_with("/v1/models") || path.ends_with("/models")
}

pub(crate) fn is_prism_completions_route(path: &str) -> bool {
    path == "/prism-openai/v1/chat/completions"
}

pub(crate) fn is_prism_web_api_route(path: &str) -> bool {
    path == "/prism-web-api/v1/response"
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

pub(crate) fn handle_prism_web_api(
    client: &mut TcpStream,
    body: &[u8],
    config: &Config,
    inbound_headers: &[(String, String)],
) -> io::Result<()> {
    let prism_url = format!(
        "{}/api/llm/response_with_tools_start",
        config.prism_base_url
    );
    let creds = extract_credentials(inbound_headers, config);
    let sentinel_token = crate::http::header_value(inbound_headers, "openai-sentinel-token")
        .map(str::to_string)
        .or(creds.sentinel_token);
    let mut ureq_builder = ureq::post(&prism_url)
        .header("Content-Type", "application/json")
        .header("Origin", &config.prism_base_url)
        .header(
            "Referer",
            &format!("{}/?u={}", config.prism_base_url, creds.project_id),
        );

    if !creds.cookie.is_empty() {
        ureq_builder = ureq_builder.header("Cookie", &creds.cookie);
    }

    if let Some(sentinel_token) = sentinel_token {
        ureq_builder = ureq_builder.header("openai-sentinel-token", sentinel_token);
    }

    let response = ureq_builder.send(body);

    let mut response = match response {
        Ok(res) => res,
        Err(ureq::Error::StatusCode(code)) => {
            return crate::http::write_error(
                client,
                code,
                "Upstream Prism Error",
                &format!("Prism rejected request with HTTP {}", code),
            );
        }
        Err(_) => {
            return Err(io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Prism request failed",
            ));
        }
    };

    let response_text = response.body_mut().read_to_string().unwrap_or_default();
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response_text.len(),
    );
    client.write_all(head.as_bytes())?;
    client.write_all(response_text.as_bytes())?;
    client.flush()
}

pub(crate) fn handle_models_catalog(client: &mut TcpStream, _config: &Config) -> io::Result<()> {
    let base_models = vec!["gpt-5.6-sol".to_string()];

    let mut base_models = base_models;
    base_models.sort();
    base_models.dedup();

    let efforts = ["low", "medium", "high", "xhigh"];
    let mut model_entries = Vec::new();

    for bm in base_models {
        model_entries.push(format!(
            r#"{{"id":"{}","object":"model","created":1700000000,"owned_by":"system"}}"#,
            bm
        ));
        // Only apply effort permutations to models that explicitly support reasoning
        if bm.starts_with("gpt-5.6-") {
            for effort in efforts {
                model_entries.push(format!(
                    r#"{{"id":"{}-{}","object":"model","created":1700000000,"owned_by":"system"}}"#,
                    bm, effort
                ));
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

pub(crate) fn route_request(mut client: TcpStream, config: &Config) -> io::Result<()> {
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

    if request.method.eq_ignore_ascii_case("get") && is_models_catalog_route(clean_path) {
        return handle_models_catalog(&mut client, config);
    }

    if is_prism_completions_route(clean_path) {
        return handle_prism_chat_completion(&mut client, &request.body, config, &request.headers);
    }

    if is_prism_web_api_route(clean_path) {
        return handle_prism_web_api(&mut client, &request.body, config, &request.headers);
    }

    handle_omniroute_proxy(&mut client, &request, config)
}
