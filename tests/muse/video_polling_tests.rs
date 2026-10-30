//! Muse video revision, session readiness, and CDN artifact URL integration tests.

use super::common::*;

#[test]
fn test_create_video_parent_id_alias_appends_revision() {
    let job_id = format!("video-api-parent-edit-{}", uuid::Uuid::new_v4());
    let record = toolcase_gateway::museai::VideoJobRecord {
        job_id: job_id.clone(),
        owner_id: "user-test".into(),
        model: "muse-video".into(),
        requested_model: "gen-3".into(),
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
        video_url: "https://muse.ai/files/parent-v1.mp4".into(),
        revisions: vec![],
        ..Default::default()
    };
    toolcase_gateway::museai::save_video_job(&record).unwrap();

    let request = serde_json::json!({"parent_id": job_id, "prompt": "new prompt"});
    let result = toolcase_gateway::museai::video::create_video_with_completion(
        &serde_json::to_vec(&request).unwrap(),
        &test_config(),
        |_request, _config, _wait_for_artifact| {
            Ok(serde_json::json!({
                "choices": [{"message": {"content": "https://muse.ai/files/parent-v2.mp4"}}]
            })
            .to_string())
        },
    )
    .unwrap();

    assert_eq!(result["id"], record.job_id);
    assert_eq!(result["video_url"], "https://muse.ai/files/parent-v2.mp4");
    assert_eq!(result["revisions"].as_array().unwrap().len(), 1);
}

#[test]
fn test_create_video_parent_job_not_found_returns_not_found() {
    let request = serde_json::json!({
        "job_id": format!("video-missing-{}", uuid::Uuid::new_v4()),
        "prompt": "new prompt"
    });
    let error = toolcase_gateway::museai::video::create_video_with_completion(
        &serde_json::to_vec(&request).unwrap(),
        &test_config(),
        |_request, _config, _wait_for_artifact| {
            panic!("provider must not be called for missing parent job")
        },
    )
    .unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn test_create_video_parent_job_request_appends_revision() {
    let job_id = format!("video-api-edit-{}", uuid::Uuid::new_v4());
    let record = toolcase_gateway::museai::VideoJobRecord {
        job_id: job_id.clone(),
        owner_id: "user-test".into(),
        model: "muse-video".into(),
        requested_model: "gen-3".into(),
        prompt: "original prompt".into(),
        aspect_ratio: "16:9".into(),
        duration: 5,
        thread_id: Some("thread-original".into()),
        created_at: 1000,
        last_activity_at: 1000,
        pinned: false,
        status: "artifact_reported".into(),
        media_validated: false,
        output_verified: false,
        video_url: "https://muse.ai/files/v1.mp4".into(),
        revisions: vec![toolcase_gateway::museai::VideoRevision {
            revision_id: "rev-1".into(),
            prompt: "original prompt".into(),
            created_at: 1000,
            video_url: "https://muse.ai/files/v1.mp4".into(),
            status: "artifact_reported".into(),
        }],
        ..Default::default()
    };
    toolcase_gateway::museai::save_video_job(&record).unwrap();

    let request = serde_json::json!({
        "job_id": job_id,
        "prompt": "make it brighter"
    });
    let config = test_config();
    let result = toolcase_gateway::museai::video::create_video_with_completion(
        &serde_json::to_vec(&request).unwrap(),
        &config,
        |_request, _config, _wait_for_artifact| {
            Ok(serde_json::json!({
                "choices": [{
                    "message": {
                        "content": "Updated video: https://muse.ai/files/v2.mp4"
                    }
                }]
            })
            .to_string())
        },
    )
    .unwrap();

    assert_eq!(result["id"], record.job_id);
    assert_eq!(result["revision_id"], "rev-2");
    assert_eq!(result["video_url"], "https://muse.ai/files/v2.mp4");
    assert_eq!(result["revisions"].as_array().unwrap().len(), 2);

    let saved = toolcase_gateway::museai::load_video_job(&record.job_id).unwrap();
    assert_eq!(saved.video_url, "https://muse.ai/files/v2.mp4");
    assert_eq!(saved.revisions.len(), 2);
}

#[test]
fn test_session_readiness_requires_running_vm_state() {
    validate_muse_session_readiness(&serde_json::json!({"vm_state":"RUNNING"})).unwrap();

    let error = validate_muse_session_readiness(&serde_json::json!({})).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

    let error =
        validate_muse_session_readiness(&serde_json::json!({"vm_state": null})).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

    for state in ["UPDATING", "STARTING", "STOPPING", "STOPPED", "ERROR"] {
        let error =
            validate_muse_session_readiness(&serde_json::json!({"vm_state": state})).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
    }

    let error =
        validate_muse_session_readiness(&serde_json::json!({"vm_state":"UNKNOWN"})).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn test_completed_video_response_requires_supported_artifact() {
    let error = build_video_result(
        "muse-video",
        "muse-video",
        "a dancing cat",
        "16:9",
        5,
        "Video generation is unavailable in this environment.",
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
}

#[test]
fn test_muse_files_url_is_supported() {
    let url = "https://muse.ai/files/1378868031972717/1124543089916741/er47bxzumhn4tcankbn87rkr/cheerful-3d-short.mp4";
    assert!(is_supported_video_url(url));

    let spoofed = "https://muse.ai.evil.example/files/123/abc/file.mp4";
    assert!(!is_supported_video_url(spoofed));

    let wrong_ext = "https://muse.ai/files/123/abc/file.txt";
    assert!(!is_supported_video_url(wrong_ext));
}

#[test]
fn test_submit_video_missing_url_persists_artifact_error() {
    let job_id = format!("video-missing-url-{}", uuid::Uuid::new_v4());
    let request = serde_json::json!({"id": job_id, "prompt": "a paper boat"});
    let response = toolcase_gateway::museai::video::submit_video_with_completion(
        &serde_json::to_vec(&request).unwrap(),
        &test_config(),
        |_request, _config, _wait_for_artifact| {
            Ok(serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "I am working on your video now."
                    }
                }]
            })
            .to_string())
        },
    )
    .unwrap();
    assert_eq!(response["status"], "submitting");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let record = toolcase_gateway::museai::load_video_job(&job_id).unwrap();
        if record.status == "failed" {
            assert!(
                record.error.as_deref().unwrap().contains("artifact"),
                "error must mention artifact: {:?}",
                record.error
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "provider failure was not persisted"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn test_apply_muse_session_metadata_extracts_notary_token() {
    let mut config = test_config();
    config.museai_notary_token.clear();
    let session = serde_json::json!({
        "vm_id": "vm-123",
        "endpoint_url": "wss://hatch.metaaivm.com/v1/noise",
        "auth_token": "token-123",
        "notary_token": "notary-abc-789"
    });
    toolcase_gateway::museai::session::apply_muse_session_metadata(&mut config, &session);
    assert_eq!(config.museai_notary_token, "notary-abc-789");
}

#[test]
fn test_meta_and_muse_cdn_video_urls_are_supported() {
    let valid_urls = [
        "https://video.xx.fbcdn.net/v/t39.25447-2/123456_n.mp4?_nc_cat=100&ccb=1-7",
        "https://video-lax3-1.xx.fbcdn.net/v/t39/cheerful.mp4",
        "https://scontent.xx.fbcdn.net/v/video.mp4",
        "https://cdn.fbsbx.com/v/t39/video.mp4",
        "https://video.meta.com/v/test.mp4",
        "https://video.cdninstagram.com/v/test.mp4",
        "https://cdn.muse.ai/files/v1.mp4",
        "https://files.muse.ai/video/clip.webm",
        "https://muse.ai/files/123/video.mov",
        "https://video.xx.fbcdn.net/v/t39/UPPERCASE.MP4",
        "https://video.xx.fbcdn.net/v/t39/caps.MOV",
        "https://video.xx.fbcdn.net/v/t39/clip.WEBM",
    ];
    for url in valid_urls {
        assert!(
            is_supported_video_url(url),
            "expected supported video URL: {url}"
        );
    }

    let invalid_urls = [
        "https://video.xx.fbcdn.net.evil.example/video.mp4",
        "https://evilfbcdn.net/video.mp4",
        "https://notmuse.ai/video.mp4",
        "https://attacker.com/video.mp4",
        "https://muse.ai.evil.example/files/123/video.mp4",
        "http://video.xx.fbcdn.net/v/t39/video.mp4",
        "https://video.xx.fbcdn.net/v/t39/image.jpg",
        "https://video.xx.fbcdn.net/v/t39/script.js",
        "https://video.xx.fbcdn.net/v/t39/style.css",
        "ftp://video.xx.fbcdn.net/v/t39/video.mp4",
        "https://127.0.0.1/video.mp4",
        "https://192.168.1.1/video.mp4",
        "video.xx.fbcdn.net/v/t39/video.mp4",
        "",
    ];
    for url in invalid_urls {
        assert!(
            !is_supported_video_url(url),
            "expected rejected video URL: {url}"
        );
    }
}

#[test]
fn test_extract_url_from_markdown_and_embedded_text() {
    let md = "Here is your video: [Watch](https://video.xx.fbcdn.net/v/t39/sample.mp4) now!";
    assert_eq!(
        extract_url_from_text(md),
        "https://video.xx.fbcdn.net/v/t39/sample.mp4"
    );

    let angle = "Video at <https://video.xx.fbcdn.net/v/t39/sample.mp4>.";
    assert_eq!(
        extract_url_from_text(angle),
        "https://video.xx.fbcdn.net/v/t39/sample.mp4"
    );

    let quoted = "{\"url\": \"https://video.xx.fbcdn.net/v/t39/sample.mp4\"}";
    assert_eq!(
        extract_url_from_text(quoted),
        "https://video.xx.fbcdn.net/v/t39/sample.mp4"
    );

    let parens = "Video ready (https://video.xx.fbcdn.net/v/t39/sample.mp4)";
    assert_eq!(
        extract_url_from_text(parens),
        "https://video.xx.fbcdn.net/v/t39/sample.mp4"
    );
}

#[test]
fn test_simulated_muse_production_response_shape_extracts_video_url() {
    let cdn_url = "https://video.xx.fbcdn.net/v/t39.25447-2/345678_n.mp4?ccb=1-7";
    let simulated_body = serde_json::json!({
        "prompt": "A cheerful cartoon fox rides a skateboard",
        "model": "muse-video"
    });
    let result = toolcase_gateway::museai::video::create_video_with_completion(
        &serde_json::to_vec(&simulated_body).unwrap(),
        &test_config(),
        |_req, _cfg, _wait| {
            Ok(serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": format!("Video generation is available — here's your video:\n\nVideo URL: {cdn_url}\n")
                    }
                }]
            }).to_string())
        },
    )
    .unwrap();

    assert_eq!(result["status"], "artifact_reported");
    assert_eq!(result["provider_status"], "artifact_url_reported");
    assert_eq!(result["video_url"], cdn_url);
}
