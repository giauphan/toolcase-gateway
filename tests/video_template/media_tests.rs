use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use toolcase_gateway::config::{Config, ConfigStore};
use toolcase_gateway::http::{header_value, read_more, read_response_head};
use toolcase_gateway::museai::video::{
    create_video_with_completion, extract_vm_media_path, is_gateway_video_content_url,
    is_supported_video_url, is_vm_media_url, save_video_media,
};
use toolcase_gateway::routes::route_request;

fn test_config() -> Config {
    Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        museai_base_url: "https://muse.ai".into(),
        ..Config::default()
    }
}

fn request_route_raw(
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    config: Config,
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store = ConfigStore::new(config, std::path::PathBuf::from(".env"));

    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        route_request(client, &store).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut header_lines = String::new();
    for (k, v) in headers {
        header_lines.push_str(&format!("{k}: {v}\r\n"));
    }
    write!(
        client,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\n{header_lines}Connection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    if !body.is_empty() {
        client.write_all(body).unwrap();
    }
    client.flush().unwrap();

    let head = read_response_head(&mut client).unwrap();
    let mut response_body = head.buffered_body;
    if method.eq_ignore_ascii_case("head") {
        response_body.clear();
    } else {
        let length =
            header_value(&head.headers, "content-length").and_then(|val| val.parse::<usize>().ok());
        if let Some(length) = length {
            while response_body.len() < length {
                read_more(&mut client, &mut response_body).unwrap();
            }
            response_body.truncate(length);
        } else {
            let mut tail = Vec::new();
            let _ = client.read_to_end(&mut tail);
            response_body.extend_from_slice(&tail);
        }
    }
    handle.join().unwrap();
    (head.status, head.headers, response_body)
}

#[test]
fn test_vm_media_url_classification_and_extraction() {
    let vm_url = "https://57004190-bb16-4e36-8c1f-695a4de1d5e1.metaaivm.com/media/raw/workspace/your_files/cube/cube.mp4";
    assert!(is_vm_media_url(vm_url));
    assert!(is_vm_media_url(
        "https://willow606.com/fs/raw/workspace/demo.mp4"
    ));
    assert!(is_vm_media_url(
        "https://vm-123.exe.xyz/media/raw/workspace/sample.mp4"
    ));

    assert!(!is_vm_media_url(
        "https://video.xx.fbcdn.net/v/t39.25447-2/123_n.mp4"
    ));
    assert!(!is_vm_media_url("https://drive.google.com/file/d/123/view"));
    assert!(!is_vm_media_url("https://muse.ai/files/user/video.mp4"));

    let extracted = extract_vm_media_path(vm_url).expect("must extract workspace path");
    assert_eq!(extracted, "workspace/your_files/cube/cube.mp4");

    let raw_extracted =
        extract_vm_media_path("https://abc.metaaivm.com/fs/raw/workspace/models/out.mp4?token=123")
            .expect("must extract from /fs/raw/");
    assert_eq!(raw_extracted, "workspace/models/out.mp4");

    assert!(is_gateway_video_content_url(
        "/muse-ai/v1/videos/job-123/content"
    ));
    assert!(is_gateway_video_content_url(
        "/muse-ai/v1/videos/job-123/download"
    ));
    assert!(is_gateway_video_content_url(
        "http://127.0.0.1:20129/muse-ai/v1/videos/job-123/content"
    ));
    assert!(!is_gateway_video_content_url("/muse-ai/v1/videos/job-123"));
    assert!(is_supported_video_url("/muse-ai/v1/videos/job-123/content"));
}

#[test]
fn test_internal_vm_url_is_proxied_to_gateway_content_endpoint() {
    let test_job_id = format!("video-test-proxy-{}", uuid::Uuid::new_v4());
    let dummy_mp4 = b"\x00\x00\x00 ftypisom\x00\x00\x02\x00isomiso2avc1mp41[TEST_BYTES_DUMMY]";
    let _ = save_video_media(&test_job_id, dummy_mp4);

    let vm_url = format!(
        "https://57004190-bb16-4e36-8c1f-695a4de1d5e1.metaaivm.com/media/raw/workspace/your_files/test/{test_job_id}.mp4"
    );
    let mock_completion_text = format!("Your video is ready: {vm_url}");

    let req_payload = serde_json::json!({
        "id": test_job_id,
        "prompt": "Rotating glowing square in 3D cartoon style",
        "model": "muse-video",
        "aspect_ratio": "16:9",
        "duration": 5
    });

    let completion_resp = serde_json::json!({
        "choices": [{
            "message": {
                "content": mock_completion_text
            }
        }]
    })
    .to_string();

    let result = create_video_with_completion(
        &serde_json::to_vec(&req_payload).unwrap(),
        &test_config(),
        move |_request, _config, _wait_for_artifact| Ok(completion_resp.clone()),
    )
    .expect("video generation must succeed");

    assert_eq!(result["id"], test_job_id);
    let expected_content_url = format!("/muse-ai/v1/videos/{test_job_id}/content");
    assert_eq!(result["video_url"], expected_content_url);
    assert_eq!(result["status"], "completed");
    assert_eq!(result["media_validated"], true);
    assert_eq!(result["output_verified"], true);

    let (get_status, _, get_body_bytes) = request_route_raw(
        "GET",
        &format!("/muse-ai/v1/videos/{test_job_id}"),
        &[],
        &[],
        test_config(),
    );
    assert_eq!(get_status, 200);
    let get_json: serde_json::Value = serde_json::from_slice(&get_body_bytes).unwrap();
    assert_eq!(get_json["video_url"], expected_content_url);
    assert_eq!(get_json["status"], "completed");
    assert_eq!(get_json["media_validated"], true);
}

#[test]
fn test_video_content_endpoint_serves_mp4_with_range_and_head_support() {
    let job_id = format!("video-media-test-{}", uuid::Uuid::new_v4());
    let mut fake_mp4 = Vec::new();
    fake_mp4.extend_from_slice(b"\x00\x00\x00 ftypisom\x00\x00\x02\x00isomiso2avc1mp41");
    for i in 0..1024 {
        fake_mp4.push((i % 256) as u8);
    }
    save_video_media(&job_id, &fake_mp4).expect("saving media fixture must succeed");

    // 1. GET full content -> 200 OK
    let (status, headers, body) = request_route_raw(
        "GET",
        &format!("/muse-ai/v1/videos/{job_id}/content"),
        &[],
        &[],
        test_config(),
    );
    assert_eq!(status, 200);
    assert_eq!(header_value(&headers, "content-type"), Some("video/mp4"));
    assert_eq!(header_value(&headers, "accept-ranges"), Some("bytes"));
    assert_eq!(
        header_value(&headers, "content-length"),
        Some(&fake_mp4.len().to_string()[..])
    );
    assert_eq!(body, fake_mp4);

    // 2. GET partial content (Range) -> 206 Partial Content
    let (range_status, range_headers, range_body) = request_route_raw(
        "GET",
        &format!("/muse-ai/v1/videos/{job_id}/content"),
        &[("Range", "bytes=16-47")],
        &[],
        test_config(),
    );
    assert_eq!(range_status, 206);
    assert_eq!(
        header_value(&range_headers, "content-type"),
        Some("video/mp4")
    );
    assert_eq!(
        header_value(&range_headers, "content-range"),
        Some(&format!("bytes 16-47/{}", fake_mp4.len())[..])
    );
    assert_eq!(header_value(&range_headers, "content-length"), Some("32"));
    assert_eq!(range_body, &fake_mp4[16..=47]);

    // 3. HEAD content -> 200 OK with headers but no body
    let (head_status, head_headers, head_body) = request_route_raw(
        "HEAD",
        &format!("/muse-ai/v1/videos/{job_id}/content"),
        &[],
        &[],
        test_config(),
    );
    assert_eq!(head_status, 200);
    assert_eq!(
        header_value(&head_headers, "content-type"),
        Some("video/mp4")
    );
    assert_eq!(head_body.len(), 0);

    // 4. GET nonexistent job -> 404 Not Found
    let (not_found_status, _, _) = request_route_raw(
        "GET",
        "/muse-ai/v1/videos/nonexistent-media-job-9999/content",
        &[],
        &[],
        test_config(),
    );
    assert_eq!(not_found_status, 404);

    // 5. POST to content endpoint -> 405 Method Not Allowed
    let (disallowed_status, _, _) = request_route_raw(
        "POST",
        &format!("/muse-ai/v1/videos/{job_id}/content"),
        &[],
        &[],
        test_config(),
    );
    assert_eq!(disallowed_status, 405);
}

#[test]
fn test_persisted_live_video_job_content_is_served() {
    let job_id = "video-dc704c30-c65c-42d8-b055-6af81c7964ce";
    if !toolcase_gateway::museai::video::has_video_media(job_id) {
        return;
    }
    let (job_status, _, job_body) = request_route_raw(
        "GET",
        &format!("/muse-ai/v1/videos/{job_id}"),
        &[],
        &[],
        test_config(),
    );
    assert_eq!(job_status, 200);
    let job_json: serde_json::Value = serde_json::from_slice(&job_body).unwrap();
    assert_eq!(job_json["status"], "completed");
    assert_eq!(
        job_json["video_url"],
        format!("/muse-ai/v1/videos/{job_id}/content")
    );

    let (content_status, headers, body) = request_route_raw(
        "GET",
        &format!("/muse-ai/v1/videos/{job_id}/content"),
        &[],
        &[],
        test_config(),
    );
    assert_eq!(content_status, 200);
    assert_eq!(header_value(&headers, "content-type"), Some("video/mp4"));
    assert_eq!(body.len(), 1998471);
    assert_eq!(&body[4..12], b"ftypisom");
}
