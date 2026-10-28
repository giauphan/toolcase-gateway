//! Jev decision provider route dispatcher.

use crate::config::Config;
use crate::http::write_error;
use std::io;
use std::net::TcpStream;

pub fn dispatch_jev_route(
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
