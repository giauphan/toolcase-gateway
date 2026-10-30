//! Structured video URL discovery and sync/async request classification tests.

use super::common::*;
use toolcase_gateway::museai::video::save_video_media;

#[test]
fn test_find_video_url_in_structured_json_events() {
    use toolcase_gateway::museai::chat::find_video_url_in_json;
    use toolcase_gateway::museai::video::is_raw_video_artifact_url;

    let vm_card_event = serde_json::json!({
        "event": "delta.presentation",
        "payload": {
            "card": {
                "title": "Generated Video",
                "url": "https://57004190-bb16-4e36-8c1f-695a4de1d5e1.metaaivm.com/fs/raw/workspace/your_files/cat/cat.mp4"
            }
        }
    });
    let found = find_video_url_in_json(&vm_card_event, 0).expect("must find VM URL in card");
    assert!(is_raw_video_artifact_url(&found));
    assert!(found.contains(".metaaivm.com"));

    let workspace_path_event = serde_json::json!({
        "event": "delta.media",
        "payload": {
            "media": {
                "path": "/fs/raw/workspace/your_files/animated_cube.mp4"
            }
        }
    });
    let found_path =
        find_video_url_in_json(&workspace_path_event, 0).expect("must find workspace path");
    assert!(is_raw_video_artifact_url(&found_path));
    assert!(found_path.contains("workspace/"));

    let public_cdn_event = serde_json::json!({
        "event": "delta.card",
        "payload": {
            "video_url": "https://video.xx.fbcdn.net/v/t39.25447-2/test.mp4"
        }
    });
    let found_cdn = find_video_url_in_json(&public_cdn_event, 0).expect("must find public CDN URL");
    assert!(is_raw_video_artifact_url(&found_cdn));
}

#[test]
fn test_is_sync_requested_classification() {
    use toolcase_gateway::museai::is_sync_requested;

    // 1. Query parameter ?wait=true / ?sync=true / ?wait
    assert!(is_sync_requested(
        "/muse-ai/v1/create-video?wait=true",
        &[],
        b""
    ));
    assert!(is_sync_requested(
        "/muse-ai/v1/create-video?wait=1",
        &[],
        b""
    ));
    assert!(is_sync_requested("/muse-ai/v1/create-video?wait", &[], b""));
    assert!(is_sync_requested(
        "/muse-ai/v1/create-video?sync=true",
        &[],
        b""
    ));
    assert!(is_sync_requested(
        "/muse-ai/v1/create-video?sync=1",
        &[],
        b""
    ));

    // 2. Query parameter explicit async override
    assert!(!is_sync_requested(
        "/muse-ai/v1/create-video?async=true",
        &[],
        b""
    ));
    assert!(!is_sync_requested(
        "/muse-ai/v1/create-video?wait=false",
        &[],
        b""
    ));
    assert!(!is_sync_requested(
        "/muse-ai/v1/create-video?sync=false",
        &[],
        b""
    ));

    // 3. Request headers
    let prefer_sync = vec![("Prefer".to_string(), "respond-async=false".to_string())];
    assert!(is_sync_requested(
        "/muse-ai/v1/create-video",
        &prefer_sync,
        b""
    ));

    let x_wait = vec![("X-Wait".to_string(), "true".to_string())];
    assert!(is_sync_requested("/muse-ai/v1/create-video", &x_wait, b""));

    let x_sync = vec![("X-Sync".to_string(), "1".to_string())];
    assert!(is_sync_requested("/muse-ai/v1/create-video", &x_sync, b""));

    // 4. User-Agent curl defaults to sync when no async flag is given
    let curl_agent = vec![("User-Agent".to_string(), "curl/8.5.0".to_string())];
    assert!(is_sync_requested(
        "/muse-ai/v1/create-video",
        &curl_agent,
        b""
    ));

    // But if curl explicitly specifies async, it respects it:
    assert!(!is_sync_requested(
        "/muse-ai/v1/create-video?async=true",
        &curl_agent,
        b""
    ));

    // 5. Browser User-Agent without wait/sync flags defaults to async
    let browser_agent = vec![(
        "User-Agent".to_string(),
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64)".to_string(),
    )];
    assert!(!is_sync_requested(
        "/muse-ai/v1/create-video",
        &browser_agent,
        b""
    ));

    // 6. JSON body
    assert!(is_sync_requested(
        "/muse-ai/v1/create-video",
        &[],
        br#"{"prompt":"test","wait":true}"#
    ));
    assert!(is_sync_requested(
        "/muse-ai/v1/create-video",
        &[],
        br#"{"prompt":"test","sync":true}"#
    ));
    assert!(!is_sync_requested(
        "/muse-ai/v1/create-video",
        &[],
        br#"{"prompt":"test","wait":false}"#
    ));
    assert!(!is_sync_requested(
        "/muse-ai/v1/create-video",
        &[],
        br#"{"prompt":"test","async":true}"#
    ));
}

#[test]
fn test_sync_video_creation_and_status_includes_full_video_url() {
    let test_job_id = format!("video-sync-full-{}", uuid::Uuid::new_v4());
    let dummy_mp4 = b"\x00\x00\x00 ftypisom\x00\x00\x02\x00isomiso2avc1mp41[SYNC_TEST_BYTES]";
    let _ = save_video_media(&test_job_id, dummy_mp4);

    // Save initial completed record
    let job_record = toolcase_gateway::museai::VideoJobRecord {
        job_id: test_job_id.clone(),
        owner_id: "default-owner".to_string(),
        model: "meta/animated-drawings".to_string(),
        requested_model: "muse-video".to_string(),
        prompt: "A dancing hedgehog".to_string(),
        aspect_ratio: "16:9".to_string(),
        duration: 5,
        thread_id: Some("thread-test".to_string()),
        created_at: 1700000000,
        last_activity_at: 1700000005,
        pinned: false,
        status: "completed".to_string(),
        media_validated: true,
        output_verified: true,
        video_url: format!("/muse-ai/v1/videos/{test_job_id}/content"),
        revisions: Vec::new(),
        approval_status: None,
        approval_id: None,
        submission_key: None,
        last_checked_at: Some(1700000005),
        error: None,
    };
    let _ = toolcase_gateway::museai::save_video_job(&job_record);

    // Test GET /muse-ai/v1/videos/:id includes full_video_url when Host is present
    let (get_status, _, get_body) = request_route_raw(
        "GET",
        &format!("/muse-ai/v1/videos/{test_job_id}"),
        &[("Host", "localhost:28130")],
        &[],
        test_config(),
    );
    assert_eq!(get_status, 200);
    let get_json: serde_json::Value = serde_json::from_slice(&get_body).unwrap();
    assert_eq!(get_json["id"], test_job_id);
    assert_eq!(get_json["status"], "completed");
    assert_eq!(
        get_json["video_url"],
        format!("/muse-ai/v1/videos/{test_job_id}/content")
    );
    let full_url = get_json["full_video_url"].as_str().unwrap();
    assert!(full_url.starts_with("http://"));
    assert!(full_url.ends_with(&format!("/muse-ai/v1/videos/{test_job_id}/content")));
}
