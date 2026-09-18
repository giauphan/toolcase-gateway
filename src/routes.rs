use crate::config::Config;
use crate::http::read_request;
use crate::omniroute::handle_omniroute_proxy;
use crate::prism::handle_prism_chat_completion;
use std::io::{self, Write};
use std::net::TcpStream;

pub(crate) fn is_models_catalog_route(path: &str) -> bool {
    path == "/v1/models" || path == "/models" || path == "/prism-openai/v1/models"
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

pub(crate) fn handle_models_catalog(client: &mut TcpStream, config: &Config) -> io::Result<()> {
    let mut base_models = vec![
        "gpt-5.6-terra".to_string(),
        "gpt-5.6-sol".to_string(),
    ];
    if !config.prism_default_model.is_empty() {
        base_models.push(config.prism_default_model.clone());
    }
    for fb in &config.fallbacks {
        if !fb.is_empty() && fb != "fail-try" {
            base_models.push(fb.clone());
        }
    }

    base_models.sort();
    base_models.dedup();

    let efforts = ["low", "medium", "high", "xhigh"];
    let mut model_entries = Vec::new();

    for bm in base_models {
        model_entries.push(format!(
            r#"{{"id":"{}","object":"model","created":1700000000,"owned_by":"system"}}"#,
            bm
        ));
        for effort in efforts {
            model_entries.push(format!(
                r#"{{"id":"{}-{}","object":"model","created":1700000000,"owned_by":"system"}}"#,
                bm, effort
            ));
        }
    }

    let body = format!(r#"{{"object":"list","data":[{}]}}"#, model_entries.join(","));
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
    let request = match read_request(&mut client) {
        Ok(req) => req,
        Err(e) => return Err(e),
    };

    let clean_path = request.path.split('?').next().unwrap_or("").trim_end_matches('/');

    if request.method.eq_ignore_ascii_case("options") {
        return handle_cors_preflight(&mut client);
    }

    if request.method.eq_ignore_ascii_case("get") && is_models_catalog_route(clean_path) {
        return handle_models_catalog(&mut client, config);
    }

    if is_prism_completions_route(clean_path) {
        return handle_prism_chat_completion(&mut client, &request.body, config, &request.headers);
    }

    handle_omniroute_proxy(&mut client, &request, config)
}
