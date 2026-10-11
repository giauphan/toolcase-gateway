use crate::config::Config;
use crate::http::{read_response_head, stream_response, Request, ResponseHead};
use crate::rewrite::replace_model;
use std::io::{self, Write};
use std::net::TcpStream;

const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

pub struct OmniRouteAdapter;

impl OmniRouteAdapter {
    pub fn open_upstream(
        client: &TcpStream,
        request: &Request,
        config: &Config,
        model: &str,
    ) -> io::Result<TcpStream> {
        let host = config.target_host.as_str();
        let port = config.target_port;
        let mut upstream = TcpStream::connect((host, port))?;
        if let Some(timeout) = config.io_timeout {
            upstream.set_read_timeout(Some(timeout))?;
            upstream.set_write_timeout(Some(timeout))?;
        }
        let body = replace_model(&request.body, model);
        write!(upstream, "{} {} HTTP/1.1\r\n", request.method, request.path)?;
        for (name, value) in &request.headers {
            if name.eq_ignore_ascii_case("host")
                || name.eq_ignore_ascii_case("content-length")
                || name.eq_ignore_ascii_case("accept-encoding")
                || HOP_BY_HOP.iter().any(|h| name.eq_ignore_ascii_case(h))
            {
                continue;
            }
            write!(upstream, "{name}: {value}\r\n")?;
        }
        let client_ip = client
            .peer_addr()
            .map(|a| a.ip().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        write!(
            upstream,
            "Host: {host}:{port}\r\nAccept-Encoding: identity\r\nX-Forwarded-For: {client_ip}\r\nX-Real-Ip: {client_ip}\r\nX-Forwarded-Proto: http\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
        upstream.write_all(&body)?;
        upstream.flush()?;
        Ok(upstream)
    }

    pub fn forward_attempt(
        client: &mut TcpStream,
        request: &Request,
        config: &Config,
        model: &str,
    ) -> io::Result<ResponseHead> {
        let mut upstream = Self::open_upstream(client, request, config, model)?;
        let head = read_response_head(&mut upstream)?;
        if !crate::gateway::retry::is_safe_to_retry(head.status) {
            stream_response(client, &mut upstream, head.clone(), &request.body)?;
        }
        Ok(head)
    }
}
