use std::io::Write;
use std::net::TcpStream;

pub(crate) fn handle_muse_config_page(client: &mut TcpStream) -> std::io::Result<()> {
    let html = include_str!("../assets/muse-config.html");
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    )?;
    client.flush()
}

pub(crate) fn handle_video_template_page(client: &mut TcpStream) -> std::io::Result<()> {
    let html = include_str!("../assets/video-template.html");
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    )?;
    client.flush()
}

#[cfg(test)]
mod template_tests {
    #[test]
    fn test_html_includes_form_and_fields() {
        let html = include_str!("../assets/video-template.html");
        assert!(html.contains("name=\"topic\""));
        assert!(html.contains("name=\"character\""));
        assert!(html.contains("name=\"setting\""));
        assert!(html.contains("name=\"action\""));
        assert!(html.contains("name=\"camera\""));
        assert!(html.contains("name=\"sound\""));
        assert!(html.contains("/muse-ai/v1/create-video"));
    }

    #[test]
    fn muse_config_fetches_masked_current_config() {
        let html = include_str!("../assets/muse-config.html");
        assert!(html.contains("fetch(\"/muse-ai/v1/config\""));
        assert!(html.contains("Loading current config"));
    }

    #[test]
    fn muse_config_explains_masking_and_safe_copying() {
        let html = include_str!("../assets/muse-config.html");
        assert!(html.contains("GW_MUSEAI_ACCESS_TOKEN"));
        assert!(html.contains("GW_MUSEAI_COOKIE"));
        assert!(html.contains("do not copy the masked text"));
        assert!(html.contains("No manual copy is needed"));
        assert!(html.contains("if(!f.masked)"));
    }
}
