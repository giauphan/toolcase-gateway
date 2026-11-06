//! Unit and integration tests for Hatch CVM native attachment items and workspace conditioning.

use toolcase_gateway::museai::chat::{
    build_muse_chat_request_with_items, extract_attachment_items,
};
use toolcase_gateway::museai::video::parse_and_validate_video_request;

#[test]
fn test_chat_request_payload_without_items_uses_canonical_message_field() {
    let payload =
        build_muse_chat_request_with_items("Hello from test", Some("test-session-123"), None);
    assert_eq!(payload["message"], "Hello from test");
    assert_eq!(payload["session_id"], "test-session-123");
    assert!(payload.get("items").is_none());
    assert!(payload["capabilities"].as_array().is_some());
}

#[test]
fn test_chat_request_payload_with_empty_items_uses_canonical_message_field() {
    let empty: Vec<serde_json::Value> = Vec::new();
    let payload = build_muse_chat_request_with_items(
        "Generate a sunset video",
        Some("test-session-456"),
        Some(&empty),
    );
    assert_eq!(payload["message"], "Generate a sunset video");
    assert_eq!(payload["session_id"], "test-session-456");
    assert!(payload.get("items").is_none());
}

#[test]
fn test_chat_request_payload_with_file_ref_items_matches_hatch_wire_schema() {
    let items = vec![serde_json::json!({
        "type": "file_ref",
        "mime_type": "image/png",
        "filename": "owl.png",
        "path": "workspace/your_files/owl.png"
    })];

    let payload = build_muse_chat_request_with_items(
        "Animate this owl in the moonlight",
        Some("session-789"),
        Some(&items),
    );

    // When items are present, Hatch CVM requires the top-level "items" array, not "message"
    assert!(payload.get("message").is_none());
    assert_eq!(payload["session_id"], "session-789");

    let items_arr = payload["items"]
        .as_array()
        .expect("items array must be present");
    assert_eq!(items_arr.len(), 2);

    // Item 0 is the file_ref attachment
    assert_eq!(items_arr[0]["type"], "file_ref");
    assert_eq!(items_arr[0]["mime_type"], "image/png");
    assert_eq!(items_arr[0]["filename"], "owl.png");
    assert_eq!(items_arr[0]["path"], "workspace/your_files/owl.png");

    // Item 1 is the text prompt
    assert_eq!(items_arr[1]["type"], "text");
    assert_eq!(items_arr[1]["text"], "Animate this owl in the moonlight");
}

#[test]
fn test_video_request_generates_attachment_items_metadata() {
    let payload = serde_json::json!({
        "prompt": "Animate owl flapping wings",
        "image": "/muse-ai/v1/images/img-owl-1234/content"
    });

    let req = parse_and_validate_video_request(&serde_json::to_vec(&payload).unwrap())
        .expect("video request parsing must succeed");

    let items = extract_attachment_items(&req.request_json)
        .expect("attachment_items must be extracted from request_json");
    assert_eq!(items.len(), 1);

    let first = &items[0];
    assert_eq!(first["type"], "file_ref");
    assert_eq!(first["mime_type"], "image/png");
    assert_eq!(first["filename"], "img-owl-1234.png");
    assert_eq!(first["path"], "workspace/your_files/img-owl-1234.png");
    assert_eq!(first["internal_job_id"], "img-owl-1234");
    assert_eq!(
        first["internal_ref"],
        "/muse-ai/v1/images/img-owl-1234/content"
    );
}

#[test]
fn test_multipart_video_request_populates_attachment_items() {
    let boundary = "---------------------------9876543210";
    let fake_png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15c4";

    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"prompt\"\r\n\r\n");
    body.extend_from_slice(b"Animate the moonlight owl glowing\r\n");

    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"image\"; filename=\"Moonlit Owl.png\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body.extend_from_slice(fake_png);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    let req = parse_and_validate_video_request(&body)
        .expect("multipart video request parsing must succeed");

    let items = extract_attachment_items(&req.request_json)
        .expect("attachment_items must be extracted from multipart request_json");
    assert_eq!(items.len(), 1);

    let first = &items[0];
    assert_eq!(first["type"], "file_ref");
    assert_eq!(first["mime_type"], "image/png");
    let fname = first["filename"].as_str().unwrap();
    assert!(fname.starts_with("img-upload-"));
    assert!(fname.ends_with(".png"));

    let vpath = first["path"].as_str().unwrap();
    assert!(vpath.starts_with("workspace/your_files/img-upload-"));
    assert!(vpath.ends_with(".png"));
}
