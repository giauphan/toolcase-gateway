use crate::config::Config;
use crate::museai_noise::MuseNoiseSession;
use crate::museai_protocol::Header;
use std::io;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

const MAX_MUSE_WS_FRAME_BYTES: usize = 16 * 1024 * 1024;

pub(crate) const MUSE_WS_OPERATION_DEADLINE_SECS: u64 = 360;

#[allow(dead_code)]
pub(crate) struct MuseWebSocket {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    deadline: Instant,
}

#[allow(dead_code)]
pub(crate) fn muse_ws_operation_deadline_secs() -> u64 {
    MUSE_WS_OPERATION_DEADLINE_SECS
}

#[allow(dead_code)]
impl MuseWebSocket {
    pub(crate) fn connect(url: &str, origin: &str, cookie: &str) -> io::Result<Self> {
        let mut request = url
            .into_client_request()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        request.headers_mut().insert(
            "Origin",
            HeaderValue::from_str(origin)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
        );
        request.headers_mut().insert(
            "User-Agent",
            HeaderValue::from_str("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36")
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
        );
        request.headers_mut().insert(
            "Accept-Language",
            HeaderValue::from_str("en-US,en;q=0.9")
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
        );
        request.headers_mut().insert(
            "Cache-Control",
            HeaderValue::from_str("no-cache")
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
        );
        request.headers_mut().insert(
            "Pragma",
            HeaderValue::from_str("no-cache")
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
        );
        if !cookie.is_empty() {
            request.headers_mut().insert(
                "Cookie",
                HeaderValue::from_str(cookie)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
            );
        }
        let (mut socket, _) =
            connect(request).map_err(|_| io::Error::other("Muse WebSocket connection failed"))?;
        match socket.get_mut() {
            tungstenite::stream::MaybeTlsStream::Plain(tcp) => {
                tcp.set_read_timeout(Some(Duration::from_secs(300)))?;
            }
            tungstenite::stream::MaybeTlsStream::Rustls(tls) => {
                tls.get_mut()
                    .set_read_timeout(Some(Duration::from_secs(300)))?;
            }
            _ => {}
        }
        Ok(Self {
            socket,
            deadline: Instant::now() + Duration::from_secs(MUSE_WS_OPERATION_DEADLINE_SECS),
        })
    }

    fn apply_deadline(&mut self) -> io::Result<()> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "Muse operation deadline exceeded")
            })?;
        let tcp = match self.socket.get_mut() {
            MaybeTlsStream::Plain(tcp) => tcp,
            MaybeTlsStream::Rustls(tls) => tls.get_mut(),
            _ => return Err(io::Error::other("Unsupported Muse TLS transport")),
        };
        tcp.set_read_timeout(Some(remaining))?;
        tcp.set_write_timeout(Some(remaining))
    }

    pub(crate) fn send_binary(&mut self, message: &[u8]) -> io::Result<()> {
        self.apply_deadline()?;
        println!(
            "[museai_transport] Sending binary frame (length: {})",
            message.len()
        );
        if message.len() > MAX_MUSE_WS_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Muse.ai WebSocket frame too large",
            ));
        }
        self.socket
            .send(Message::Binary(message.to_vec()))
            .map_err(io::Error::other)
    }

    pub(crate) fn read_binary(&mut self) -> io::Result<Vec<u8>> {
        loop {
            self.apply_deadline()?;
            match self.socket.read().map_err(|error| {
                let kind = match error {
                    tungstenite::Error::Io(error) => error.kind(),
                    _ => io::ErrorKind::Other,
                };
                io::Error::new(kind, "Muse WebSocket read failed")
            })? {
                Message::Binary(message) if message.len() <= MAX_MUSE_WS_FRAME_BYTES => {
                    return Ok(message)
                }
                Message::Binary(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Muse.ai WebSocket frame too large",
                    ));
                }
                Message::Close(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Muse.ai WebSocket closed",
                    ));
                }
                Message::Ping(payload) => self
                    .socket
                    .send(Message::Pong(payload))
                    .map_err(io::Error::other)?,
                Message::Text(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn send_encrypted_service_request(
        &mut self,
        session: &mut MuseNoiseSession,
        service: u64,
        stream_id: u64,
        verb: &str,
        path: &str,
        headers: &[Header],
        body: &[u8],
    ) -> io::Result<()> {
        let chunk_id = (uuid::Uuid::new_v4().as_u128() & 0x7FFF_FFFF_FFFF_FFFF) as i64;

        // Send single ApplicationRequest with body and end_body = true
        let app_req =
            crate::museai_protocol::encode_application_request(verb, path, headers, body, true);
        let frame_req = crate::museai_protocol::encode_service_frame_request(stream_id, &app_req);
        let svc_req = crate::museai_protocol::encode_service_request(service, &frame_req);

        let frames = crate::museai_protocol::encode_transport_frames(chunk_id, &svc_req)?;
        for frame in frames {
            let mut encrypted = vec![0u8; frame.len() + 64];
            let len = session
                .encrypt(&frame, &mut encrypted)
                .map_err(io::Error::other)?;
            println!(
                "[museai_transport] Sending ApplicationRequest frame with full body (len: {})",
                len
            );
            self.send_binary(&encrypted[..len])?;
        }

        Ok(())
    }

    pub(crate) fn read_encrypted_service_frame(
        &mut self,
        session: &mut crate::museai_noise::MuseNoiseSession,
    ) -> io::Result<crate::museai_protocol::ServiceFrame> {
        let mut assembled_payload: Vec<u8> = Vec::new();
        let mut expected_total: u32 = 0;
        let mut expected_id: i64 = 0;

        loop {
            let cipher = self.read_binary()?;
            let mut plain = vec![0u8; cipher.len() + 64];
            let len = session
                .decrypt(&cipher, &mut plain)
                .map_err(io::Error::other)?;
            let frame = crate::museai_protocol::decode_transport_frame(&plain[..len])?;
            if assembled_payload.is_empty() {
                expected_id = frame.chunk_id;
                expected_total = frame.total_chunks;
            } else if frame.chunk_id != expected_id {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Mismatched transport chunk ID",
                ));
            }
            assembled_payload.extend_from_slice(&frame.payload);
            if frame.chunk_index + 1 >= expected_total {
                break;
            }
        }

        let svc_resp = crate::museai_protocol::decode_service_response(&assembled_payload)?;
        crate::museai_protocol::decode_service_frame(&svc_resp)
    }
}

#[derive(Debug)]
pub(crate) struct UpstreamResponse {
    pub status: u16,
    pub body: String,
    pub content_type: String,
}

pub(crate) fn send_museai_request(
    url: &str,
    method: &str,
    body_json: &str,
    config: &Config,
) -> io::Result<UpstreamResponse> {
    let base = config.museai_base_url.trim_end_matches('/');

    let mut is_json = false;
    let has_body = !body_json.is_empty()
        && (method.eq_ignore_ascii_case("post") || method.eq_ignore_ascii_case("put"));
    if has_body {
        is_json = true;
    }

    let is_auth_check = url.ends_with("/api/auth/check") && method.eq_ignore_ascii_case("post");

    let response_res = if method.eq_ignore_ascii_case("get") {
        let mut b = ureq::get(url)
            .header("Accept", "application/json")
            .header("Origin", base)
            .header("Referer", &format!("{base}/"))
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Safari/537.36")
            .header("sec-ch-ua", r#""Chromium";v="154", "Brave";v="154", "Not A(Brand";v="99""#)
            .header("sec-ch-ua-mobile", "?0")
            .header("sec-ch-ua-platform", r#""Windows""#)
            .header("sec-fetch-dest", "empty")
            .header("sec-fetch-mode", "cors")
            .header("sec-fetch-site", "same-origin")
            .header("sec-gpc", "1")
            .header("cache-control", "no-cache")
            .header("pragma", "no-cache");
        if !config.museai_cookie.is_empty() {
            b = b.header("Cookie", &config.museai_cookie);
        }
        b.call()
    } else {
        let mut b = ureq::post(url)
            .header("Accept", "application/json")
            .header("Origin", base)
            .header("Referer", &format!("{base}/"))
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Safari/537.36")
            .header("sec-ch-ua", r#""Chromium";v="154", "Brave";v="154", "Not A(Brand";v="99""#)
            .header("sec-ch-ua-mobile", "?0")
            .header("sec-ch-ua-platform", r#""Windows""#)
            .header("sec-fetch-dest", "empty")
            .header("sec-fetch-mode", "cors")
            .header("sec-fetch-site", "same-origin")
            .header("sec-gpc", "1")
            .header("cache-control", "no-cache")
            .header("pragma", "no-cache");
        if !config.museai_cookie.is_empty() {
            b = b.header("Cookie", &config.museai_cookie);
        }
        if is_auth_check {
            b = b.header("next-action", "009276f6a217bd1a06954e5ac591c5cbd42c0afca5");
        }
        if is_json {
            b = b.header("Content-Type", "application/json");
            b.send(body_json)
        } else {
            b.send_empty()
        }
    };

    match response_res {
        Ok(mut res) => {
            let status = u16::from(res.status());
            let content_type = res
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/json")
                .to_string();

            let body = match res.body_mut().read_to_string() {
                Ok(b) => b,
                Err(e) => {
                    return Err(io::Error::other(format!(
                        "Failed to read upstream body: {e}"
                    )));
                }
            };

            Ok(UpstreamResponse {
                status,
                body,
                content_type,
            })
        }
        Err(e) => {
            let (status, content_type, body) = match e {
                ureq::Error::StatusCode(status) => {
                    let text = format!("{{\"error\": \"{status}\"}}");
                    (status, "application/json".to_string(), text)
                }
                _ => (
                    502,
                    "application/json".to_string(),
                    format!("{{\"error\": \"Upstream network error: {e}\"}}"),
                ),
            };

            Ok(UpstreamResponse {
                status,
                body,
                content_type,
            })
        }
    }
}
