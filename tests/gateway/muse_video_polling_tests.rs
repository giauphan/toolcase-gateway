//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;
use super::muse_video_route_tests::request_route;

// ---------- Muse video lifecycle, prompt update, and polling tests ----------

#[test]
fn test_create_video_parent_job_not_found_returns_404() {
    let body = serde_json::json!({
        "job_id": "video-missing-parent-12345",
        "prompt": "make it brighter"
    })
    .to_string();

    let (status, response) =
        request_route("POST", "/muse-ai/v1/create-video", &body, test_config());
    assert_eq!(status, 404);
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("video job not found"));
}

#[test]
fn test_put_update_video_prompt_success() {
    let job_id = format!("video-put-test-{}", uuid::Uuid::new_v4());
    let record = toolcase_gateway::museai::VideoJobRecord {
        job_id: job_id.clone(),
        owner_id: "user-123".into(),
        model: "muse-video".into(),
        requested_model: "muse-video".into(),
        prompt: "original prompt".into(),
        aspect_ratio: "16:9".into(),
        duration: 5,
        thread_id: None,
        created_at: 1000,
        last_activity_at: 1000,
        pinned: false,
        status: "artifact_reported".into(),
        media_validated: false,
        output_verified: false,
        video_url: "https://muse.ai/files/v1.mp4".into(),
        revisions: vec![],
        ..Default::default()
    };
    toolcase_gateway::museai::save_video_job(&record).unwrap();

    let body = serde_json::json!({
        "prompt": "Updated video generation prompt with enhanced cinematic details"
    })
    .to_string();

    let (status, response) = request_route(
        "PUT",
        &format!("/muse-ai/v1/videos/{job_id}"),
        &body,
        test_config(),
    );
    assert_eq!(status, 200);
    assert_eq!(response["id"], job_id);
    assert_eq!(response["status"], "updated");
    assert_eq!(
        response["prompt"],
        "Updated video generation prompt with enhanced cinematic details"
    );
    assert!(response.get("video_url").is_none());

    let reloaded = toolcase_gateway::museai::load_video_job(&job_id).unwrap();
    assert_eq!(
        reloaded.prompt,
        "Updated video generation prompt with enhanced cinematic details"
    );
    assert_eq!(reloaded.video_url, "https://muse.ai/files/v1.mp4");
    assert!(reloaded.revisions.is_empty());
}

#[test]
fn test_put_update_video_prompt_nonexistent_returns_404() {
    let body = serde_json::json!({
        "prompt": "Updated prompt"
    })
    .to_string();

    let (status, _response) = request_route(
        "PUT",
        "/muse-ai/v1/videos/video-nonexistent-99999",
        &body,
        test_config(),
    );
    assert_eq!(status, 404);
}

#[test]
fn test_put_update_video_prompt_empty_returns_400() {
    let body = serde_json::json!({
        "prompt": "   "
    })
    .to_string();

    let (status, _response) = request_route(
        "PUT",
        "/muse-ai/v1/videos/video-some-id",
        &body,
        test_config(),
    );
    assert_eq!(status, 400);
}

#[test]
fn test_put_update_video_prompt_invalid_id_returns_400() {
    let body = serde_json::json!({
        "prompt": "Valid prompt"
    })
    .to_string();

    let (status, response) = request_route(
        "PUT",
        "/muse-ai/v1/videos/invalid.id.with.dots",
        &body,
        test_config(),
    );
    assert_eq!(status, 400);
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("invalid job id"));
}

#[test]
fn test_get_video_job_status_success() {
    let job_id = format!("video-get-test-{}", uuid::Uuid::new_v4());
    let record = toolcase_gateway::museai::VideoJobRecord {
        job_id: job_id.clone(),
        owner_id: "user-get".into(),
        model: "muse-video".into(),
        requested_model: "muse-video".into(),
        prompt: "get video test prompt".into(),
        aspect_ratio: "16:9".into(),
        duration: 5,
        thread_id: Some("thread-xyz".into()),
        created_at: 1000,
        last_activity_at: 1000,
        pinned: false,
        status: "artifact_reported".into(),
        media_validated: false,
        output_verified: false,
        video_url: "https://muse.ai/files/get-v1.mp4".into(),
        revisions: vec![],
        approval_status: Some("approved".into()),
        approval_id: Some("app-123".into()),
        ..Default::default()
    };
    toolcase_gateway::museai::save_video_job(&record).unwrap();

    let (status, response) = request_route(
        "GET",
        &format!("/muse-ai/v1/videos/{job_id}"),
        "",
        test_config(),
    );

    assert_eq!(status, 200);
    assert_eq!(response["id"], job_id);
    assert_eq!(response["status"], "artifact_reported");
    assert_eq!(response["approval_status"], "approved");
    assert_eq!(response["approval_id"], "app-123");
    assert_eq!(response["video_url"], "https://muse.ai/files/get-v1.mp4");
}

#[test]
fn test_get_video_job_nonexistent_returns_404() {
    let (status, _response) = request_route(
        "GET",
        "/muse-ai/v1/videos/video-nonexistent-88888",
        "",
        test_config(),
    );
    assert_eq!(status, 404);
}

#[test]
fn test_video_route_unsupported_method_returns_405() {
    let (status, _response) = request_route(
        "DELETE",
        "/muse-ai/v1/videos/video-any-id",
        "",
        test_config(),
    );
    assert_eq!(status, 405);
}

#[test]
fn test_curl_api_lifecycle_workflow_for_video_template() {
    let job_id = format!("video-ui-workflow-{}", uuid::Uuid::new_v4());
    let now = toolcase_gateway::museai::lifecycle::now_secs();
    let init_record = toolcase_gateway::museai::VideoJobRecord {
        job_id: job_id.clone(),
        owner_id: "default-owner".to_string(),
        model: "muse-video".to_string(),
        requested_model: "muse-video".to_string(),
        prompt: "A cozy bakery at dawn".to_string(),
        aspect_ratio: "9:16".to_string(),
        duration: 10,
        thread_id: None,
        created_at: now,
        last_activity_at: now,
        pinned: false,
        status: "submitting".to_string(),
        media_validated: false,
        output_verified: false,
        video_url: String::new(),
        revisions: Vec::new(),
        approval_status: None,
        approval_id: None,
        submission_key: None,
        last_checked_at: Some(now),
        ..Default::default()
    };
    toolcase_gateway::museai::save_video_job(&init_record).unwrap();

    let (status, polling_resp) = request_route(
        "GET",
        &format!("/muse-ai/v1/videos/{job_id}"),
        "",
        test_config(),
    );
    assert_eq!(status, 200);
    assert_eq!(polling_resp["id"], job_id);
    assert_eq!(polling_resp["status"], "submitting");
    assert_eq!(polling_resp["video_url"], "");

    let artifact_url = "https://muse.ai/files/completed-video.mp4";
    let completed_record = toolcase_gateway::museai::VideoJobRecord {
        status: "completed".to_string(),
        media_validated: true,
        output_verified: true,
        video_url: artifact_url.to_string(),
        last_activity_at: 1010,
        ..init_record.clone()
    };
    toolcase_gateway::museai::save_video_job(&completed_record).unwrap();

    let (status, final_resp) = request_route(
        "GET",
        &format!("/muse-ai/v1/videos/{job_id}"),
        "",
        test_config(),
    );
    assert_eq!(status, 200);
    assert_eq!(final_resp["id"], job_id);
    assert_eq!(final_resp["status"], "completed");
    assert_eq!(final_resp["media_validated"], true);
    assert_eq!(final_resp["video_url"], artifact_url);

    if let Ok(path) = toolcase_gateway::museai::lifecycle::job_file_path(&job_id) {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn test_post_create_video_route_submits_job_and_extracts_meta_cdn_artifact() {
    let job_id = format!("video-route-cdn-{}", uuid::Uuid::new_v4());
    let cdn_url = "https://video.xx.fbcdn.net/v/t39.25447-2/345678_n.mp4?ccb=1-7";

    let init_record = toolcase_gateway::museai::VideoJobRecord {
        job_id: job_id.clone(),
        status: "artifact_reported".to_string(),
        video_url: cdn_url.to_string(),
        media_validated: false,
        output_verified: false,
        ..Default::default()
    };
    toolcase_gateway::museai::save_video_job(&init_record).unwrap();

    let (status, resp) = request_route(
        "GET",
        &format!("/muse-ai/v1/videos/{job_id}"),
        "",
        test_config(),
    );
    assert_eq!(status, 200);
    assert_eq!(resp["id"], job_id);
    assert_eq!(resp["status"], "artifact_reported");
    assert_eq!(resp["video_url"], cdn_url);

    if let Ok(path) = toolcase_gateway::museai::lifecycle::job_file_path(&job_id) {
        let _ = std::fs::remove_file(path);
    }
}
