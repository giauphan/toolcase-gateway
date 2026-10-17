use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use toolcase_gateway::config::Config;
use toolcase_gateway::museai::lifecycle::{
    evaluate_job_retention, load_video_job, save_video_job, VideoJobRecord,
};
use toolcase_gateway::museai::video::create_video_with_completion;
use toolcase_gateway::museai::MuseApprovalRequired;

fn sample_completion_json(video_url: &str) -> String {
    serde_json::json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "thread_id": "thread-ack-123",
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": format!("Here is your video: {video_url}")
                }
            }
        ]
    })
    .to_string()
}

#[test]
fn test_video_creation_idempotency() {
    let config = Config::default();
    let idempotency_key = format!("idem-{}", uuid::Uuid::new_v4());
    let req = serde_json::json!({
        "prompt": "sunset over mountains",
        "idempotency_key": idempotency_key,
    });
    let req_bytes = serde_json::to_vec(&req).unwrap();

    let completion_calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = completion_calls.clone();

    // First call: runs completion
    let res1 = create_video_with_completion(&req_bytes, &config, move |_body, _cfg, _wait| {
        calls_clone.fetch_add(1, Ordering::SeqCst);
        Ok(sample_completion_json("https://muse.ai/files/test-v1.mp4"))
    })
    .unwrap();

    assert_eq!(completion_calls.load(Ordering::SeqCst), 1);
    let job_id = res1["id"].as_str().unwrap();
    assert_eq!(res1["status"], "artifact_reported");
    assert_eq!(res1["video_url"], "https://muse.ai/files/test-v1.mp4");

    // Second call with same idempotency_key: returns cached result without running completion!
    let calls_clone2 = completion_calls.clone();
    let res2 = create_video_with_completion(&req_bytes, &config, move |_body, _cfg, _wait| {
        calls_clone2.fetch_add(1, Ordering::SeqCst);
        Ok(sample_completion_json(
            "https://muse.ai/files/different.mp4",
        ))
    })
    .unwrap();

    // Verify completion was NOT called again!
    assert_eq!(completion_calls.load(Ordering::SeqCst), 1);
    assert_eq!(res2["id"], job_id);
    assert_eq!(res2["video_url"], "https://muse.ai/files/test-v1.mp4");
}

#[test]
fn test_video_creation_persists_approval_required_on_conflict() {
    let config = Config::default();
    let custom_id = format!("video-app-req-{}", uuid::Uuid::new_v4());
    let req = serde_json::json!({
        "id": custom_id,
        "prompt": "generate landscape",
    });
    let req_bytes = serde_json::to_vec(&req).unwrap();

    let err = create_video_with_completion(&req_bytes, &config, |_body, _cfg, _wait| {
        Err(io::Error::other(MuseApprovalRequired(
            "Scoped Muse permission approval is pending".to_string(),
        )))
    })
    .unwrap_err();

    assert!(err
        .get_ref()
        .and_then(|e| e.downcast_ref::<MuseApprovalRequired>())
        .is_some());

    // Verify job was persisted with status approval_required
    let job = load_video_job(&custom_id).unwrap();
    assert_eq!(job.status, "approval_required");
    assert_eq!(job.approval_status.as_deref(), Some("manual_required"));
}

#[test]
fn test_replay_for_inflight_idempotency_key_never_resubmits() {
    let config = Config::default();
    let idempotency_key = format!("idem-inflight-{}", uuid::Uuid::new_v4());
    let job_id = format!("video-inflight-{}", uuid::Uuid::new_v4());
    save_video_job(&VideoJobRecord {
        job_id: job_id.clone(),
        status: "submitting".into(),
        submission_key: Some(idempotency_key.clone()),
        ..Default::default()
    })
    .unwrap();

    let req = serde_json::json!({
        "prompt": "same prompt must not submit twice",
        "idempotency_key": idempotency_key,
    });

    let completion_calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = completion_calls.clone();

    let err = create_video_with_completion(
        &serde_json::to_vec(&req).unwrap(),
        &config,
        move |_body, _cfg, _wait| {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Ok(sample_completion_json(
                "https://muse.ai/files/should-not-exist.mp4",
            ))
        },
    )
    .unwrap_err();

    assert_eq!(completion_calls.load(Ordering::SeqCst), 0);
    assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
}

#[test]
fn test_malformed_completion_marks_reserved_job_failed() {
    let job_id = format!("video-malformed-completion-{}", uuid::Uuid::new_v4());
    let request = serde_json::json!({
        "id": job_id,
        "prompt": "a harmless test video",
    });
    let error = create_video_with_completion(
        &serde_json::to_vec(&request).unwrap(),
        &Config::default(),
        |_body, _config, _wait| Ok("not valid json".into()),
    )
    .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let job = load_video_job(request["id"].as_str().unwrap()).unwrap();
    assert_eq!(job.status, "failed");
}

#[test]
fn test_lifecycle_retention_protects_approval_and_pending_states() {
    let now = 1000000_u64;
    let mut rec = VideoJobRecord {
        job_id: "job-prot".into(),
        status: "approval_pending".into(),
        last_activity_at: now - 999999, // very old
        ..Default::default()
    };

    // approval_pending must NOT be cleaned up
    assert!(!evaluate_job_retention(&rec, now, 86400));

    // approval_required must NOT be cleaned up
    rec.status = "approval_required".into();
    assert!(!evaluate_job_retention(&rec, now, 86400));

    // submission_unknown CAN be cleaned up if older than TTL, but stays protected within TTL
    rec.status = "submission_unknown".into();
    rec.last_activity_at = now - 999999;
    assert!(evaluate_job_retention(&rec, now, 86400));
    rec.last_activity_at = now - 100;
    assert!(!evaluate_job_retention(&rec, now, 86400));

    // submitting must NOT be cleaned up
    rec.status = "submitting".into();
    rec.last_activity_at = now - 999999;
    assert!(!evaluate_job_retention(&rec, now, 86400));

    // artifact_reported CAN be cleaned up if older than TTL
    rec.status = "artifact_reported".into();
    rec.last_activity_at = now - 999999;
    assert!(evaluate_job_retention(&rec, now, 86400));
}

#[test]
fn test_submission_unknown_cleanup_releases_submission_key() {
    let key = format!("idem-cleanup-{}", uuid::Uuid::new_v4());
    let old_job_id = format!("video-old-{}", uuid::Uuid::new_v4());
    let now = toolcase_gateway::museai::lifecycle::now_secs();

    let old_job = VideoJobRecord {
        job_id: old_job_id.clone(),
        status: "submission_unknown".into(),
        submission_key: Some(key.clone()),
        created_at: now.saturating_sub(100_000),
        last_activity_at: now.saturating_sub(100_000),
        ..Default::default()
    };
    save_video_job(&old_job).unwrap();

    // Prior to cleanup, reservation with same submission_key finds existing job
    let duplicate = VideoJobRecord {
        job_id: format!("video-dup-{}", uuid::Uuid::new_v4()),
        status: "submitting".into(),
        submission_key: Some(key.clone()),
        ..Default::default()
    };
    let found = toolcase_gateway::museai::lifecycle::reserve_video_job(&duplicate)
        .unwrap()
        .unwrap();
    assert_eq!(found.job_id, old_job_id);

    // Run cleanup with TTL = 86400 (1 day)
    let cleaned = toolcase_gateway::museai::lifecycle::cleanup_expired_video_jobs(86400).unwrap();
    assert!(cleaned >= 1);
    assert!(load_video_job(&old_job_id).is_err());

    // After cleanup, reservation with same submission_key succeeds as a fresh reservation
    let fresh = VideoJobRecord {
        job_id: format!("video-fresh-{}", uuid::Uuid::new_v4()),
        status: "submitting".into(),
        submission_key: Some(key),
        ..Default::default()
    };
    let reserved = toolcase_gateway::museai::lifecycle::reserve_video_job(&fresh).unwrap();
    assert!(reserved.is_none(), "fresh reservation must be accepted");
}
