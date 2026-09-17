use crate::config::Config;
use crate::http::write_error;
use std::io;
use std::net::TcpStream;

// Note: Stubs. Need to be completed.
pub(crate) fn handle_prism_chat_completion(
    client: &mut TcpStream,
    _body: &[u8],
    _config: &Config,
    _headers: &[(String, String)],
) -> io::Result<()> {
    write_error(
        client,
        500,
        "Not Implemented",
        "Prism functionality is in progress",
    )
}
