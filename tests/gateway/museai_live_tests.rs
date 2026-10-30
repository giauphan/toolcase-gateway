//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;

// ---------- Muse live-capture auth, sentinel, and helper tests ----------

use toolcase_gateway::museai::session::apply_muse_session_metadata;

pub(crate) fn live_capture_config() -> Config {
    let _ = dotenvy::dotenv();
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let har_bytes = std::fs::read(manifest.join("target/muse.ai.har"))
        .expect("live Muse HAR capture is required");
    let extracted = toolcase_gateway::har_config::extract_muse_config_from_har(&har_bytes)
        .unwrap_or_else(|_| panic!("live Muse HAR could not be parsed"));
    let cookie = std::env::var("GW_MUSEAI_COOKIE")
        .ok()
        .filter(|cookie| !cookie.trim().is_empty())
        .or_else(|| {
            std::fs::read(manifest.join("target/muse-header.txt"))
                .ok()
                .and_then(|bytes| parse_cookie_from_header_dump(&bytes))
        })
        .or(extracted.cookie)
        .filter(|cookie| !cookie.is_empty())
        .expect("live Muse capture has no cookie");
    let har: serde_json::Value = serde_json::from_slice(&har_bytes)
        .unwrap_or_else(|_| panic!("live Muse HAR must be valid JSON"));
    let request = har["log"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| &entry["request"])
        .find(|request| {
            request["method"] == "POST" && request["url"] == "https://muse.ai/api/auth/check"
        })
        .expect("capture must evidence the Muse authentication endpoint");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .max_redirects(0)
        .build()
        .into();
    let mut auth = agent
        .post("https://muse.ai/api/auth/check")
        .header("Cookie", &cookie);
    for header in request["headers"].as_array().unwrap() {
        let Some(name) = header["name"].as_str() else {
            continue;
        };
        if [
            "accept",
            "accept-language",
            "origin",
            "referer",
            "user-agent",
            "sec-fetch-site",
            "sec-fetch-mode",
            "sec-fetch-dest",
            "sec-ch-ua",
            "sec-ch-ua-mobile",
            "sec-ch-ua-platform",
        ]
        .iter()
        .any(|allowed| name.eq_ignore_ascii_case(allowed))
        {
            if let Some(value) = header["value"].as_str() {
                auth = auth.header(name, value);
            }
        }
    }
    let auth = auth
        .send_empty()
        .unwrap_or_else(|error| match error {
            ureq::Error::StatusCode(status) => panic!("live Muse authentication HTTP {status}"),
            _ => panic!("live Muse authentication transport failed (details suppressed)"),
        })
        .into_body()
        .read_json::<serde_json::Value>()
        .unwrap_or_else(|_| panic!("invalid live Muse authentication JSON"));
    assert_eq!(
        auth["ok"].as_bool(),
        Some(true),
        "live Muse authentication rejected"
    );
    let token = auth["access_token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .expect("live Muse authentication returned no token")
        .to_owned();
    let session = agent
        .get("https://muse.ai/api/session")
        .header("Cookie", &cookie)
        .call()
        .unwrap_or_else(|_| panic!("live Muse session request failed"))
        .into_body()
        .read_json::<serde_json::Value>()
        .unwrap_or_else(|_| panic!("invalid live Muse session JSON"));
    let mut config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        default_model: "gpt-5.6-sol".into(),
        museai_base_url: "https://muse.ai".into(),
        museai_cookie: cookie,
        museai_ws_url: String::new(),
        museai_access_token: token,
        museai_notary_token: extracted.notary_token.unwrap_or_default(),
        museai_vm_id: String::new(),
        museai_auto_cleanup_threads: false,
        museai_thread_retention_secs: 0,
        jev_api_keys: vec![],
        jev_base_url: "https://jev-ai.org".into(),
        ..Config::default()
    };
    apply_muse_session_metadata(&mut config, &session);
    config
}

#[test]
fn test_live_muse_auth_from_har_capture() {
    if std::env::var("MUSE_LIVE_TEST").ok().as_deref() != Some("1") {
        return;
    }
    let config = live_capture_config();
    assert!(!config.museai_access_token.is_empty());
    assert!(!config.museai_vm_id.is_empty());
}

#[test]
fn test_thread_cleanup_route_dispatch() {
    use std::net::TcpListener;
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let config = Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        default_model: "gpt-5.6-sol".into(),
        museai_base_url: "http://127.0.0.1:9".into(), // Will not hang, unreachable
        museai_cookie: "".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
        jev_api_keys: vec![],
        jev_base_url: "https://jev-ai.org".into(),
        ..Config::default()
    };

    let handle = std::thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        let req_in = toolcase_gateway::http::read_request(&mut client).unwrap();
        assert_eq!(req_in.method, "DELETE");
        assert_eq!(req_in.path, "/muse-ai/v1/threads/mock-thread-456");
        let clean_path = req_in
            .path
            .split('?')
            .next()
            .unwrap_or("")
            .trim_end_matches('/');
        let thread_id = clean_path.split('/').next_back().unwrap_or("");
        assert_eq!(thread_id, "mock-thread-456");
        toolcase_gateway::museai::handle_museai_thread_cleanup(&mut client, thread_id, &config)
            .unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "DELETE /muse-ai/v1/threads/mock-thread-456 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();

    let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    let mut body = head.buffered_body;
    if let Some(length_str) = toolcase_gateway::http::header_value(&head.headers, "content-length")
    {
        let length: usize = length_str.parse().unwrap();
        while body.len() < length {
            toolcase_gateway::http::read_more(&mut client, &mut body).unwrap();
        }
    }
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["object"], "thread.cleanup");
    assert_eq!(json["thread_id"], "mock-thread-456");
    assert_eq!(json["status"], "deleted");

    handle.join().unwrap();
}

pub(crate) fn live_http_request(
    method: &str,
    path: &str,
    body: &str,
    config: Config,
) -> (u16, serde_json::Value) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
    let handle = std::thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    client.flush().unwrap();

    let head = toolcase_gateway::http::read_response_head(&mut client).unwrap();
    let mut response_body = head.buffered_body;
    if let Some(length) = toolcase_gateway::http::header_value(&head.headers, "content-length")
        .and_then(|value| value.parse::<usize>().ok())
    {
        while response_body.len() < length {
            toolcase_gateway::http::read_more(&mut client, &mut response_body).unwrap();
        }
        response_body.truncate(length);
    }
    handle.join().unwrap();
    (
        head.status,
        serde_json::from_slice(&response_body).unwrap_or_else(|_| serde_json::json!({})),
    )
}

#[test]
fn test_live_sentinel_read_only_approvals() {
    if std::env::var("MUSE_LIVE_SENTINEL_PROBE").ok().as_deref() != Some("1") {
        return;
    }

    let config = live_capture_config();
    use toolcase_gateway::museai::approval::client::{MuseSentinelClient, SentinelClient};
    let mut client = MuseSentinelClient::connect(&config)
        .unwrap_or_else(|_| panic!("Live Sentinel connection failed (private details suppressed)"));
    let approvals = client
        .list_approvals()
        .unwrap_or_else(|_| panic!("Live Sentinel list failed (private details suppressed)"));
    println!(
        "Live Sentinel read-only probe succeeded: count={} approvals={approvals:?}",
        approvals.len()
    );
}

pub(crate) fn parse_cookie_from_header_dump(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i + 1 < lines.len() {
        if lines[i].trim().eq_ignore_ascii_case("cookie") {
            let value = lines[i + 1].trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
        i += 1;
    }
    None
}

#[test]
fn parse_cookie_from_header_dump_extracts_cookie_value() {
    let dump =
        b":authority\nmuse.ai\n:method\nGET\ncookie\nsite.example; session=abc\n:method2\nx\n";
    assert_eq!(
        parse_cookie_from_header_dump(dump),
        Some("site.example; session=abc".to_string())
    );
}

#[test]
fn parse_cookie_from_header_dump_without_cookie_is_none() {
    let dump = b":authority\nmuse.ai\n:method\nGET\nreferer\nhttps://muse.ai/thread/new\n";
    assert_eq!(parse_cookie_from_header_dump(dump), None);
}

#[test]
fn parse_cookie_from_header_dump_empty_value_is_none() {
    let dump = b"cookie\n\npriority\nu=1, i\n";
    assert_eq!(parse_cookie_from_header_dump(dump), None);
}

#[test]
fn parse_cookie_from_header_dump_case_insensitive_key() {
    let dump = b"Cookie\nsession=xyz\n";
    assert_eq!(
        parse_cookie_from_header_dump(dump),
        Some("session=xyz".to_string())
    );
}

pub(crate) fn safe_live_error(error: &std::io::Error) -> &'static str {
    if error.kind() == std::io::ErrorKind::PermissionDenied
        || error
            .get_ref()
            .and_then(
                <dyn std::error::Error + Send + Sync>::downcast_ref::<
                    toolcase_gateway::museai::MuseApprovalRequired,
                >,
            )
            .is_some()
    {
        "service approval required or access denied"
    } else {
        "incomplete or invalid service response (private details suppressed)"
    }
}
