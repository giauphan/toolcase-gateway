//! Template route dispatcher (Video & Image studio).

use crate::http::write_error;
use std::io;
use std::net::TcpStream;

pub fn dispatch_template_route(
    client: &mut TcpStream,
    request: &crate::http::Request,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    let is_video = clean_path == "/video-template" || clean_path.starts_with("/video-template/");
    let is_image = clean_path == "/image-template" || clean_path.starts_with("/image-template/");
    if !is_video && !is_image {
        return Ok(None);
    }
    let allowed =
        request.method.eq_ignore_ascii_case("get") || request.method.eq_ignore_ascii_case("head");
    if allowed {
        if is_video {
            crate::video_template::handle_video_template_page_with_method(client, &request.method)?;
        } else {
            crate::video_template::handle_image_template_page_with_method(client, &request.method)?;
        }
        Ok(Some(200))
    } else {
        let name = if is_video {
            "video templates"
        } else {
            "image templates"
        };
        write_error(
            client,
            405,
            "Method Not Allowed",
            &format!("{name} only support GET and HEAD"),
        )?;
        Ok(Some(405))
    }
}
