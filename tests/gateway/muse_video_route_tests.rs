//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;

pub(crate) fn request_route(
    method: &str,
    path: &str,
    body: &str,
    config: Config,
) -> (u16, serde_json::Value) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store =
        toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));

    let handle = thread::spawn(move || {
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
    let head = read_response_head(&mut client).unwrap();
    let mut response_body = head.buffered_body;
    let length =
        header_value(&head.headers, "content-length").and_then(|value| value.parse::<usize>().ok());
    if let Some(length) = length {
        while response_body.len() < length {
            read_more(&mut client, &mut response_body).unwrap();
        }
        response_body.truncate(length);
    } else {
        let _ = client.read_to_end(&mut response_body);
    }
    handle.join().unwrap();
    (head.status, serde_json::from_slice(&response_body).unwrap())
}

#[test]
fn test_versioned_video_capability_listing_has_no_production_ready_native_provider() {
    let (status, body) = request_route("GET", "/video/v1/providers", "", test_config());

    assert_eq!(status, 200);
    assert_eq!(body["schema_version"], "video.transport.v1");
    assert_eq!(body["providers"], serde_json::json!([]));
}

#[test]
fn test_unsupported_video_task_is_rejected_before_upstream_call() {
    let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    upstream.set_nonblocking(true).unwrap();

    let mut config = test_config();
    config.target_host = "127.0.0.1".into();
    config.target_port = upstream_port;
    let body = serde_json::json!({
        "model": "unverified-model",
        "operation": "text_to_video",
        "prompt": "A cat"
    })
    .to_string();
    let (status, response) = request_route(
        "POST",
        "/video/v1/providers/unverified/tasks",
        &body,
        config,
    );

    assert_eq!(status, 422);
    assert!(upstream.accept().is_err(), "route must not call upstream");
    assert_eq!(response["error"]["code"], "UNSUPPORTED_CAPABILITY");
    assert_eq!(response["error"]["category"], "invalid_input");
    assert_eq!(response["error"]["retryable"], false);
    assert_eq!(response["error"]["submission_unknown"], false);
}

#[test]
fn test_versioned_video_errors_have_stable_json_envelope() {
    let body = serde_json::json!({
        "model": "unverified-model",
        "operation": "text_to_video",
        "prompt": "A cat"
    })
    .to_string();
    let (status, response) = request_route(
        "POST",
        "/video/v1/providers/unverified/tasks",
        &body,
        test_config(),
    );

    assert_eq!(status, 422);
    assert_eq!(
        response,
        serde_json::json!({
            "error": {
                "code": "UNSUPPORTED_CAPABILITY",
                "category": "invalid_input",
                "retryable": false,
                "submission_unknown": false,
                "message": "provider/model/operation is not production-ready"
            },
            "request_id": "redacted-request-id"
        })
    );
}

#[test]
fn test_video_template_trailing_slash_serves_page() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = thread::spawn(move || {
        let config = test_config();
        let store =
            toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
        let (client, _) = listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        client,
        "GET /video-template/ HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();

    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 200);
    assert_eq!(
        header_value(&head.headers, "content-type"),
        Some("text/html; charset=utf-8")
    );
    handle.join().unwrap();
}

#[test]
fn test_video_template_only_served_on_get() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = thread::spawn(move || {
        let config = test_config();
        let store =
            toolcase_gateway::config::ConfigStore::new(config, std::path::PathBuf::from(".env"));
        let (client, _) = listener.accept().unwrap();
        toolcase_gateway::routes::route_request(client, &store).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    // POST belongs to the video-template namespace and must be rejected locally,
    // never forwarded through the generic OmniRoute proxy.
    write!(
        client,
        "POST /video-template HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    client.flush().unwrap();

    let head = read_response_head(&mut client).unwrap();
    assert_eq!(head.status, 405);
    assert_ne!(
        header_value(&head.headers, "content-type"),
        Some("text/html; charset=utf-8")
    );
    handle.join().unwrap();
}

#[test]
fn test_versioned_video_task_status_returns_not_found_for_unknown_task() {
    let (status, response) = request_route(
        "GET",
        "/video/v1/providers/kling_official/tasks/unknown-task-id",
        "",
        test_config(),
    );

    assert_eq!(status, 404);
    assert_eq!(response["error"]["code"], "NOT_FOUND");
    assert_eq!(response["error"]["category"], "not_found");
    assert_eq!(response["error"]["retryable"], false);
}

#[test]
fn test_versioned_video_reconciliation_returns_unsupported_error() {
    let (status, response) = request_route(
        "GET",
        "/video/v1/providers/kling_official/tasks/by-key/my-idempotency-key",
        "",
        test_config(),
    );

    assert_eq!(status, 501);
    assert_eq!(response["error"]["code"], "RECONCILE_UNSUPPORTED");
    assert_eq!(response["error"]["category"], "unsupported");
    assert_eq!(response["error"]["retryable"], false);
}

#[test]
fn test_versioned_video_route_handles_tasks_as_task_id_and_key() {
    let (status, response) = request_route(
        "GET",
        "/video/v1/providers/kling_official/tasks/tasks",
        "",
        test_config(),
    );
    assert_eq!(status, 404);
    assert_eq!(response["error"]["code"], "NOT_FOUND");

    let (status, response) = request_route(
        "GET",
        "/video/v1/providers/kling_official/tasks/by-key/tasks",
        "",
        test_config(),
    );
    assert_eq!(status, 501);
    assert_eq!(response["error"]["code"], "RECONCILE_UNSUPPORTED");
}

#[test]
fn test_ssrf_and_url_validation_blocks_unsafe_targets() {
    use toolcase_gateway::features::video::types::validate_secure_url;

    // Disallowed / SSRF targets
    assert!(!validate_secure_url("http://google.com"));
    assert!(!validate_secure_url("ftp://google.com"));
    assert!(!validate_secure_url("file:///etc/passwd"));
    assert!(!validate_secure_url("https://localhost/video.mp4"));
    assert!(!validate_secure_url("https://localhost./video.mp4"));
    assert!(!validate_secure_url("https://127.0.0.1/video.mp4"));
    assert!(!validate_secure_url("https://127.0.0.1:8080/video.mp4"));
    assert!(!validate_secure_url("https://0.0.0.0/video.mp4"));
    assert!(!validate_secure_url("https://10.0.0.1/video.mp4"));
    assert!(!validate_secure_url("https://192.168.1.1/video.mp4"));
    assert!(!validate_secure_url("https://172.16.0.1/video.mp4"));
    assert!(!validate_secure_url(
        "https://169.254.169.254/latest/meta-data"
    ));
    assert!(!validate_secure_url("https://[::1]/video.mp4"));
    assert!(!validate_secure_url("https://[0:0:0:0:0:0:0:1]/video.mp4"));
    assert!(!validate_secure_url("https://[fe80::1]/video.mp4"));
    assert!(!validate_secure_url("https://api.internal/video.mp4"));
    assert!(!validate_secure_url("https://service.local/video.mp4"));

    // Allowed public HTTPS targets
    assert!(validate_secure_url("https://cdn.example.com/video.mp4"));
    assert!(validate_secure_url(
        "https://storage.googleapis.com/bucket/ref.png"
    ));
    assert!(validate_secure_url(
        "https://drive.google.com/file/d/123/view"
    ));
}

#[test]
fn test_shared_contract_v1_poll_response_descriptor_serialization() {
    use toolcase_gateway::features::video::types::{ArtifactDescriptor, VideoTaskStatusResponse};

    let resp = VideoTaskStatusResponse {
        schema_version: "video.transport.v1".into(),
        provider: "kling_official".into(),
        provider_task_id: "task-abc-123".into(),
        provider_status: "succeeded".into(),
        artifact_available: true,
        artifact_url_expires_at: Some(1730000000),
        media_validated: false,
        reconciliation_capability: "unsupported".into(),
        artifacts: vec![ArtifactDescriptor {
            kind: "video".into(),
            download_url: "https://provider-storage.example.com/generated.mp4".into(),
        }],
    };

    let serialized = serde_json::to_value(&resp).unwrap();
    assert_eq!(serialized["schema_version"], "video.transport.v1");
    assert_eq!(serialized["provider"], "kling_official");
    assert_eq!(serialized["provider_task_id"], "task-abc-123");
    assert_eq!(serialized["provider_status"], "succeeded");
    assert_eq!(serialized["artifact_available"], true);
    assert_eq!(serialized["artifact_url_expires_at"], 1730000000);
    assert_eq!(serialized["media_validated"], false);
    assert_eq!(serialized["artifacts"][0]["kind"], "video");
    assert_eq!(
        serialized["artifacts"][0]["download_url"],
        "https://provider-storage.example.com/generated.mp4"
    );
}

#[test]
fn test_video_task_submit_rejects_ssrf_reference_images() {
    let bad_body = serde_json::json!({
        "model": "kling-v1",
        "operation": "image_to_video",
        "prompt": "Animate character",
        "reference_images": [
            {
                "role": "first_frame",
                "url": "https://127.0.0.1/evil.png"
            }
        ]
    })
    .to_string();

    let (status, response) = request_route(
        "POST",
        "/video/v1/providers/kling_official/tasks",
        &bad_body,
        test_config(),
    );
    assert_eq!(status, 400);
    assert_eq!(response["error"]["code"], "INVALID_INPUT");
    assert_eq!(
        response["error"]["message"],
        "reference_images url failed security validation"
    );

    let ipv6_bad_body = serde_json::json!({
        "model": "kling-v1",
        "operation": "image_to_video",
        "prompt": "Animate character",
        "reference_images": [
            {
                "role": "first_frame",
                "url": "https://[::1]/evil.png"
            }
        ]
    })
    .to_string();

    let (status, response) = request_route(
        "POST",
        "/video/v1/providers/kling_official/tasks",
        &ipv6_bad_body,
        test_config(),
    );
    assert_eq!(status, 400);
    assert_eq!(response["error"]["code"], "INVALID_INPUT");

    let good_body = serde_json::json!({
        "model": "kling-v1",
        "operation": "image_to_video",
        "prompt": "Animate character",
        "reference_images": [
            {
                "role": "first_frame",
                "url": "https://storage.example.com/frame.png"
            }
        ]
    })
    .to_string();

    let (status, response) = request_route(
        "POST",
        "/video/v1/providers/kling_official/tasks",
        &good_body,
        test_config(),
    );
    assert_eq!(status, 422);
    assert_eq!(response["error"]["code"], "UNSUPPORTED_CAPABILITY");
}
