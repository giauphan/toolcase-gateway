//! Gateway integration tests for Muse image generation routes.

use super::common::*;
use toolcase_gateway::config::Config;
use toolcase_gateway::museai::image::{save_image_job, save_image_media, ImageJobRecord};
use uuid::Uuid;

fn request_image_route(
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
fn test_create_image_route_rejects_non_post() {
    let (status, body) = request_image_route("GET", "/muse-ai/v1/create-image", "", test_config());
    assert_eq!(status, 405);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("only supports POST"));
}

#[test]
fn test_create_image_route_rejects_missing_prompt() {
    let (status, body) = request_image_route(
        "POST",
        "/muse-ai/v1/create-image",
        r#"{"aspect_ratio": "16:9"}"#,
        test_config(),
    );
    assert_eq!(status, 400);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("prompt is required"));
}

#[test]
fn test_create_image_route_rejects_invalid_aspect_ratio() {
    let (status, body) = request_image_route(
        "POST",
        "/muse-ai/v1/create-image",
        r#"{"prompt": "portrait", "aspect_ratio": "invalid-ratio"}"#,
        test_config(),
    );
    assert_eq!(status, 400);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unsupported aspect_ratio 'invalid-ratio'"));
}

#[test]
fn test_openai_images_generations_route_alias_validation() {
    let (status, body) = request_image_route(
        "POST",
        "/v1/images/generations",
        r#"{"prompt": "mountain landscape", "size": "unsupported-size"}"#,
        test_config(),
    );
    assert_eq!(status, 400);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unsupported size 'unsupported-size'"));
}

#[test]
fn test_create_image_async_submission_returns_submitting() {
    let unique_id = format!("route-img-{}", Uuid::new_v4());
    let payload = serde_json::json!({
        "id": unique_id,
        "prompt": "an oil painting of starry night with a lighthouse",
        "aspect_ratio": "16:9",
        "async": true
    })
    .to_string();

    let (status, body) = request_image_route(
        "POST",
        "/muse-ai/v1/create-image?async=true",
        &payload,
        test_config(),
    );

    assert_eq!(status, 200);
    assert_eq!(body["id"], unique_id);
    assert_eq!(body["status"], "submitting");
    assert_eq!(body["aspect_ratio"], "16:9");
    assert_eq!(body["object"], "image.generation");
}

#[test]
fn test_get_image_job_status_endpoint() {
    let unique_id = format!("route-get-{}", Uuid::new_v4());
    let record = ImageJobRecord {
        job_id: unique_id.clone(),
        owner_id: "default-owner".to_string(),
        model: "muse-image".to_string(),
        requested_model: "muse-image".to_string(),
        prompt: "a sunset over mountains".to_string(),
        aspect_ratio: "16:9".to_string(),
        thread_id: None,
        created_at: 1700000000,
        last_activity_at: 1700000000,
        status: "completed".to_string(),
        media_validated: true,
        output_verified: true,
        image_url: "https://muse.ai/files/888/999/art.png".to_string(),
        submission_key: None,
        error: None,
    };
    save_image_job(&record).unwrap();

    let path = format!("/muse-ai/v1/images/{unique_id}");
    let (status, body) = request_image_route("GET", &path, "", test_config());

    assert_eq!(status, 200);
    assert_eq!(body["id"], unique_id);
    assert_eq!(body["status"], "completed");
    assert_eq!(body["aspect_ratio"], "16:9");
    assert_eq!(body["image_url"], "https://muse.ai/files/888/999/art.png");
}

#[test]
fn test_get_image_job_status_not_found() {
    let (status, body) = request_image_route(
        "GET",
        "/muse-ai/v1/images/non-existent-image-id",
        "",
        test_config(),
    );
    assert_eq!(status, 404);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not found"));
}

#[test]
fn test_get_image_job_status_rejects_post() {
    let (status, body) =
        request_image_route("POST", "/muse-ai/v1/images/some-id", "", test_config());
    assert_eq!(status, 405);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("only supports GET"));
}

fn request_raw_image(
    method: &str,
    path: &str,
    body: &str,
    config: Config,
) -> (u16, Vec<(String, String)>, Vec<u8>) {
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
    let _ = client.read_to_end(&mut response_body);
    handle.join().unwrap();
    (head.status, head.headers, response_body)
}

#[test]
fn test_image_template_route_serves_html() {
    let (status, _headers, body) = request_raw_image("GET", "/image-template", "", test_config());
    assert_eq!(status, 200);
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Muse AI Image Studio"));
    assert!(html.contains("Aspect Ratio"));
    assert!(html.contains("Download Image"));
}

#[test]
fn test_image_content_and_download_local_media() {
    let unique_id = format!("route-media-{}", Uuid::new_v4());
    let dummy_png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDRtestimage";
    save_image_media(&unique_id, dummy_png, "png").unwrap();

    // 1. Test preview / content endpoint
    let content_path = format!("/muse-ai/v1/images/{unique_id}/content");
    let (status, headers, body) = request_raw_image("GET", &content_path, "", test_config());
    assert_eq!(status, 200);
    assert_eq!(body, dummy_png);
    let content_type = header_value(&headers, "content-type").unwrap();
    assert_eq!(content_type, "image/png");
    let disposition = header_value(&headers, "content-disposition").unwrap();
    assert!(disposition.contains("inline"));

    // 2. Test download endpoint
    let download_path = format!("/muse-ai/v1/images/{unique_id}/download");
    let (status, headers, body) = request_raw_image("GET", &download_path, "", test_config());
    assert_eq!(status, 200);
    assert_eq!(body, dummy_png);
    let disposition = header_value(&headers, "content-disposition").unwrap();
    assert!(disposition.contains("attachment"));
}

#[test]
fn test_image_content_redirects_external_url() {
    let unique_id = format!("route-redir-{}", Uuid::new_v4());
    let record = ImageJobRecord {
        job_id: unique_id.clone(),
        owner_id: "default-owner".to_string(),
        model: "muse-image".to_string(),
        requested_model: "muse-image".to_string(),
        prompt: "a landscape".to_string(),
        aspect_ratio: "16:9".to_string(),
        thread_id: None,
        created_at: 1700000000,
        last_activity_at: 1700000000,
        status: "completed".to_string(),
        media_validated: true,
        output_verified: true,
        image_url: "https://muse.ai/files/external/sample.png".to_string(),
        submission_key: None,
        error: None,
    };
    save_image_job(&record).unwrap();

    let content_path = format!("/muse-ai/v1/images/{unique_id}/content");
    let (status, headers, _body) = request_raw_image("GET", &content_path, "", test_config());
    assert_eq!(status, 302);
    let location = header_value(&headers, "location").unwrap();
    assert_eq!(location, "https://muse.ai/files/external/sample.png");
}

#[test]
fn test_get_image_job_status_head_request_omits_body() {
    let unique_id = format!("route-head-{}", Uuid::new_v4());
    let record = ImageJobRecord {
        job_id: unique_id.clone(),
        owner_id: "test-user".to_string(),
        model: "muse-image".to_string(),
        requested_model: "muse-image".to_string(),
        prompt: "art prompt".to_string(),
        aspect_ratio: "16:9".to_string(),
        thread_id: None,
        created_at: 1700000000,
        last_activity_at: 1700000000,
        status: "completed".to_string(),
        media_validated: true,
        output_verified: true,
        image_url: "https://muse.ai/files/888/999/art.png".to_string(),
        submission_key: None,
        error: None,
    };
    save_image_job(&record).unwrap();

    let path = format!("/muse-ai/v1/images/{unique_id}");
    let (status, headers, body) = request_raw_image("HEAD", &path, "", test_config());

    assert_eq!(status, 200);
    assert!(
        body.is_empty(),
        "RFC 9110 requires HEAD response body to be empty"
    );
    let content_len = header_value(&headers, "content-length").unwrap();
    assert_ne!(content_len, "0");
}

#[test]
fn test_get_image_rejects_malformed_job_id() {
    let (status, body) = request_image_route(
        "GET",
        "/muse-ai/v1/images/bad..id/content",
        "",
        test_config(),
    );
    assert_eq!(status, 400);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Invalid job id"));
}
