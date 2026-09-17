use crate::config::Config;
use crate::http::write_error;
use std::io;
use std::net::TcpStream;

// Note: Stubs. Need to be completed.
pub(crate) fn handle_prism_chat_completion(
    client: &mut TcpStream,
    _body: &[u8],
    config: &Config,
    _headers: &[(String, String)],
) -> io::Result<()> {
    let _ = (
        &config.prism_base_url,
        &config.prism_project_id,
        &config.prism_cookie,
        &config.prism_sandbox_token,
        &config.prism_user_id,
        &config.prism_default_model,
    );
    write_error(
        client,
        500,
        "Not Implemented",
        "Prism functionality is in progress",
    )
}
