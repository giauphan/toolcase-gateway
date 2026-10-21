use std::io::Write;
use std::net::TcpStream;

pub fn handle_muse_config_page(client: &mut TcpStream) -> std::io::Result<()> {
    let html = include_str!("../assets/muse-config.html");
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    )?;
    client.flush()
}

pub fn handle_video_template_page(client: &mut TcpStream) -> std::io::Result<()> {
    handle_video_template_page_with_method(client, "GET")
}

pub fn handle_video_template_page_with_method(
    client: &mut TcpStream,
    method: &str,
) -> std::io::Result<()> {
    let html = include_str!("../assets/video-template.html");
    if method.eq_ignore_ascii_case("head") {
        write!(
            client,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            html.len()
        )?;
    } else {
        write!(
            client,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        )?;
    }
    client.flush()
}
