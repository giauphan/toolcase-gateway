//! Muse video-related integration tests.

use super::common::*;

#[test]
fn test_extract_url_from_text() {
    let valid_text = "Here is your video: https://cdn.muse.ai/video/xyz123.mp4";
    assert_eq!(
        extract_url_from_text(valid_text),
        "https://cdn.muse.ai/video/xyz123.mp4"
    );

    let mov_text = "Watch this https://cdn.muse.ai/video/xyz123.mov, and enjoy!";
    assert_eq!(
        extract_url_from_text(mov_text),
        "https://cdn.muse.ai/video/xyz123.mov"
    );

    let no_video_ext = "Here is a link https://muse.ai/some-link";
    assert_eq!(extract_url_from_text(no_video_ext), "");

    let misleading_query = "Not a video https://example.com/help?next=clip.mp4";
    assert_eq!(extract_url_from_text(misleading_query), "");

    let http_video = "Do not accept http://cdn.muse.ai/video/xyz123.mp4";
    assert_eq!(extract_url_from_text(http_video), "");

    let no_url = "Just some text without links";
    assert_eq!(extract_url_from_text(no_url), "");

    let gdrive_url =
        "Video ready at https://drive.google.com/file/d/1a2b3c4d5e/view?usp=sharing enjoy!";
    assert_eq!(
        extract_url_from_text(gdrive_url),
        "https://drive.google.com/file/d/1a2b3c4d5e/view?usp=sharing"
    );

    let gdrive_multi =
        "Check https://example.com/site or Google Drive: https://drive.google.com/uc?id=xyz789";
    assert_eq!(
        extract_url_from_text(gdrive_multi),
        "https://drive.google.com/uc?id=xyz789"
    );

    let unsafe_drive = "Do not accept http://drive.google.com/uc?id=xyz789";
    assert_eq!(extract_url_from_text(unsafe_drive), "");

    let lookalike_drive = "Do not accept https://drive.google.com.evil.example/file/d/123";
    assert_eq!(extract_url_from_text(lookalike_drive), "");

    let drive_and_direct = "Drive https://drive.google.com/file/d/drive123/view and direct https://cdn.muse.ai/video/direct.mp4";
    assert_eq!(
        extract_url_from_text(drive_and_direct),
        "https://drive.google.com/file/d/drive123/view"
    );

    let muse_files_url = "Video uploaded to https://muse.ai/files/1378866961972717/1124543089916741/er47bxzumhn4tcankbn87rkr/cheerful-3d-short.mp4 ready.";
    assert_eq!(
        extract_url_from_text(muse_files_url),
        "https://muse.ai/files/1378866961972717/1124543089916741/er47bxzumhn4tcankbn87rkr/cheerful-3d-short.mp4"
    );

    let spoofed_muse_files = "Do not accept https://muse.ai.evil.example/files/123/abc/file.mp4";
    assert_eq!(extract_url_from_text(spoofed_muse_files), "");
}

#[test]
fn test_video_prompt_avoids_unsupported_capability_claims() {
    let prompt = build_video_prompt("gen-3", "a dancing cat", "16:9", 5);
    for unsupported in ["gen-3", "kling", "Muse Video", "16:9", "5 seconds"] {
        assert!(!prompt.contains(unsupported));
    }
    assert!(prompt.contains("a dancing cat"));
    assert!(prompt.contains("if video generation is available"));
    assert!(prompt.contains("video artifact link"));
    assert!(!prompt.contains("Google Drive"));
    assert!(!prompt.contains("public"));
}

#[test]
fn test_normalize_video_model_aliases() {
    assert_eq!(normalize_video_model("muse"), "muse-video");
    assert_eq!(normalize_video_model("gen-3"), "muse-video");
    assert_eq!(normalize_video_model("kling"), "muse-video");
    assert_eq!(normalize_video_model("muse-video"), "muse-video");
}

#[test]
fn test_create_video_validation() {
    let config = test_config();

    let empty_prompt = serde_json::json!({
        "prompt": "   ",
        "model": "gen-3"
    });
    let result = create_video(&serde_json::to_vec(&empty_prompt).unwrap(), &config);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);

    let invalid_model = serde_json::json!({
        "prompt": "A cat",
        "model": "invalid-model"
    });
    let result = create_video(&serde_json::to_vec(&invalid_model).unwrap(), &config);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);

    let invalid_ar = serde_json::json!({
        "prompt": "A cat",
        "aspect_ratio": "4:5"
    });
    let result = create_video(&serde_json::to_vec(&invalid_ar).unwrap(), &config);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn test_duration_must_be_positive_and_positive_values_pass_validation() {
    let mut config = test_config();
    config.museai_access_token = "synthetic-token".into();
    config.museai_ws_url = "https://not-a-trusted-muse-host.example/noise".into();

    for duration in [
        serde_json::json!(0),
        serde_json::json!("0"),
        serde_json::json!(-1),
        serde_json::json!(null),
        serde_json::json!(""),
    ] {
        let request = serde_json::json!({"prompt": "A cat", "duration": duration});
        let error = create_video(&serde_json::to_vec(&request).unwrap(), &config).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "duration must be a positive integer");
    }

    for duration in [serde_json::json!(1), serde_json::json!("5s")] {
        let request = serde_json::json!({"prompt": "A cat", "duration": duration});
        let error = create_video(&serde_json::to_vec(&request).unwrap(), &config).unwrap_err();
        assert_ne!(error.to_string(), "duration must be a positive integer");
    }
}

#[test]
fn test_legacy_muse_artifact_reports_provider_and_media_validation_truthfully() {
    let result = build_video_result(
        "muse-video",
        "gen-3",
        "a dancing cat",
        "16:9",
        5,
        "Video ready: https://drive.google.com/file/d/legacy-artifact/view",
    )
    .unwrap();

    assert_eq!(result["provider"], "muse_chat");
    assert_eq!(result["resolved_provider"], "muse_chat");
    assert_eq!(result["requested_model"], "gen-3");
    assert_eq!(result["resolved_model"], "muse-video");
    assert_eq!(result["status"], "artifact_reported");
    assert_eq!(result["media_validated"], false);
    assert_eq!(result["output_verified"], false);
    assert_eq!(result["provider_status"], "artifact_url_reported");
}

#[test]
fn test_unverified_artifact_is_not_reported_as_completed() {
    let result = build_video_result(
        "muse-video",
        "muse-video",
        "a rotating square",
        "16:9",
        5,
        "Video ready: https://cdn.muse.ai/video/unverified.mp4",
    )
    .unwrap();

    assert_eq!(result["status"], "artifact_reported");
    assert_eq!(result["media_validated"], false);
    assert_eq!(result["output_verified"], false);
}

#[test]
fn test_create_edit_prompt_update_pipeline_preserves_video_history() {
    let initial_request = serde_json::json!({
        "prompt": "a paper boat crossing a moonlit lake",
        "model": "gen-3",
        "aspect_ratio": "16:9",
        "duration": 5
    });
    let initial_result = toolcase_gateway::museai::video::create_video_with_completion(
        &serde_json::to_vec(&initial_request).unwrap(),
        &test_config(),
        |request, _config, _wait_for_artifact| {
            let request = String::from_utf8_lossy(request);
            assert!(request.contains("a paper boat crossing a moonlit lake"));
            Ok(serde_json::json!({
                "choices": [{"message": {"content": "https://muse.ai/files/pipeline-v1.mp4"}}]
            })
            .to_string())
        },
    )
    .unwrap();
    let job_id = initial_result["id"].as_str().unwrap().to_string();
    assert_eq!(
        initial_result["video_url"],
        "https://muse.ai/files/pipeline-v1.mp4"
    );

    let edit_request = serde_json::json!({
        "job_id": job_id,
        "prompt": "add warm lantern light and gentle ripples"
    });
    let edited_result = toolcase_gateway::museai::video::create_video_with_completion(
        &serde_json::to_vec(&edit_request).unwrap(),
        &test_config(),
        |request, _config, _wait_for_artifact| {
            let request = String::from_utf8_lossy(request);
            assert!(request.contains("https://muse.ai/files/pipeline-v1.mp4"));
            assert!(request.contains("add warm lantern light and gentle ripples"));
            Ok(serde_json::json!({
                "choices": [{"message": {"content": "https://muse.ai/files/pipeline-v2.mp4"}}]
            })
            .to_string())
        },
    )
    .unwrap();
    assert_eq!(edited_result["id"], job_id);
    assert_eq!(edited_result["revision_id"], "rev-1");
    assert_eq!(
        edited_result["video_url"],
        "https://muse.ai/files/pipeline-v2.mp4"
    );

    let updated = toolcase_gateway::museai::lifecycle::update_job_prompt(
        &job_id,
        "store this prompt without regenerating the artifact",
    )
    .unwrap();
    assert_eq!(
        updated.prompt,
        "store this prompt without regenerating the artifact"
    );
    assert_eq!(updated.video_url, "https://muse.ai/files/pipeline-v2.mp4");
    assert_eq!(updated.revisions.len(), 1);
    assert_eq!(
        updated.revisions[0].prompt,
        "add warm lantern light and gentle ripples"
    );

    let persisted = toolcase_gateway::museai::load_video_job(&job_id).unwrap();
    assert_eq!(
        persisted.prompt,
        "store this prompt without regenerating the artifact"
    );
    assert_eq!(persisted.video_url, "https://muse.ai/files/pipeline-v2.mp4");
    assert_eq!(persisted.revisions.len(), 1);
}

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
