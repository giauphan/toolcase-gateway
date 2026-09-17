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

pub(crate) fn handle_models_catalog(client: &mut TcpStream) -> io::Result<()> {
    let body = r#"{"object":"list","data":[{"id":"gpt-5.6-terra","object":"model"},{"id":"gpt-5.6-sol","object":"model"}]}"#;
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
        return handle_models_catalog(&mut client);
    }

    if is_prism_completions_route(clean_path) {
        return handle_prism_chat_completion(&mut client, &request.body, config, &request.headers);
    }

    handle_omniroute_proxy(&mut client, &request, config)
}
