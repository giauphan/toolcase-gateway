use toolcase_gateway::museai::{
    list_video_jobs, load_video_job, reserve_video_job, save_video_job, VideoJobRecord,
    VideoRevision,
};

#[test]
fn test_video_job_persistence_and_revisions() {
    let job_id = format!("video-test-{}", uuid::Uuid::new_v4());
    let record = VideoJobRecord {
        job_id: job_id.clone(),
        owner_id: "user-123".into(),
        model: "muse-video".into(),
        requested_model: "gen-3".into(),
        prompt: "initial prompt".into(),
        aspect_ratio: "16:9".into(),
        duration: 5,
        thread_id: Some("thread-abc".into()),
        created_at: 1000,
        last_activity_at: 1000,
        pinned: false,
        status: "artifact_reported".into(),
        media_validated: false,
        output_verified: false,
        video_url: "https://muse.ai/files/test.mp4".into(),
        revisions: vec![VideoRevision {
            revision_id: "rev-1".into(),
            prompt: "initial prompt".into(),
            created_at: 1000,
            video_url: "https://muse.ai/files/test.mp4".into(),
            status: "artifact_reported".into(),
        }],
        ..Default::default()
    };

    save_video_job(&record).unwrap();
    let loaded = load_video_job(&job_id).unwrap();
    assert_eq!(loaded.job_id, job_id);
    assert_eq!(loaded.owner_id, "user-123");
    assert_eq!(loaded.revisions.len(), 1);
    assert_eq!(loaded.revisions[0].prompt, "initial prompt");

    let jobs = list_video_jobs().unwrap();
    assert!(jobs.iter().any(|j| j.job_id == job_id));
}

#[test]
fn reserve_video_job_returns_existing_record_for_duplicate_submission_key() {
    let existing_id = format!("video-reservation-existing-{}", uuid::Uuid::new_v4());
    let duplicate_id = format!("video-reservation-duplicate-{}", uuid::Uuid::new_v4());
    let key = format!("submission-{}", uuid::Uuid::new_v4());
    let existing = VideoJobRecord {
        job_id: existing_id.clone(),
        submission_key: Some(key.clone()),
        status: "submitting".into(),
        ..Default::default()
    };
    let duplicate = VideoJobRecord {
        job_id: duplicate_id.clone(),
        submission_key: Some(key),
        status: "submitting".into(),
        ..Default::default()
    };

    reserve_video_job(&existing).unwrap();
    let found = reserve_video_job(&duplicate).unwrap().unwrap();

    assert_eq!(found.job_id, existing_id);
    assert!(load_video_job(&duplicate_id).is_err());
}

#[test]
fn concurrent_video_job_reservations_are_deduplicated() {
    let key = format!("concurrent-submission-{}", uuid::Uuid::new_v4());
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let key = key.clone();
            std::thread::spawn(move || {
                let record = VideoJobRecord {
                    job_id: format!("video-reservation-{}", uuid::Uuid::new_v4()),
                    submission_key: Some(key),
                    status: "submitting".into(),
                    ..Default::default()
                };
                reserve_video_job(&record).unwrap().is_none()
            })
        })
        .collect();

    let newly_reserved = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .filter(|reserved| *reserved)
        .count();
    assert_eq!(newly_reserved, 1);
}

#[test]
fn test_retention_cleanup_rules() {
    let now = 1000000_u64;
    let mut record = VideoJobRecord {
        job_id: "video-retention-test".into(),
        owner_id: "user-123".into(),
        model: "muse-video".into(),
        requested_model: "muse-video".into(),
        prompt: "initial prompt".into(),
        aspect_ratio: "16:9".into(),
        duration: 5,
        thread_id: None,
        created_at: now - 90000, // over 24 hours ago
        last_activity_at: now - 90000,
        pinned: false,
        status: "artifact_reported".into(),
        media_validated: false,
        output_verified: false,
        video_url: "https://muse.ai/files/old.mp4".into(),
        revisions: vec![],
        ..Default::default()
    };

    // Unpinned, old -> should expire for 1-day TTL (86400)
    assert!(toolcase_gateway::museai::lifecycle::evaluate_job_retention(
        &record, now, 86400
    ));

    // Pinned -> should never expire
    record.pinned = true;
    assert!(!toolcase_gateway::museai::lifecycle::evaluate_job_retention(&record, now, 86400));

    // Active status -> should never expire
    record.pinned = false;
    record.status = "generating".into();
    assert!(!toolcase_gateway::museai::lifecycle::evaluate_job_retention(&record, now, 86400));
}

#[test]
fn test_append_job_revision() {
    let job_id = format!("video-edit-test-{}", uuid::Uuid::new_v4());
    let record = VideoJobRecord {
        job_id: job_id.clone(),
        owner_id: "user-456".into(),
        model: "muse-video".into(),
        requested_model: "gen-3".into(),
        prompt: "initial prompt".into(),
        aspect_ratio: "16:9".into(),
        duration: 5,
        thread_id: Some("thread-xyz".into()),
        created_at: 2000,
        last_activity_at: 2000,
        pinned: false,
        status: "artifact_reported".into(),
        media_validated: true,
        output_verified: true,
        video_url: "https://muse.ai/files/v1.mp4".into(),
        revisions: vec![VideoRevision {
            revision_id: "rev-1".into(),
            prompt: "initial prompt".into(),
            created_at: 2000,
            video_url: "https://muse.ai/files/v1.mp4".into(),
            status: "artifact_reported".into(),
        }],
        ..Default::default()
    };

    save_video_job(&record).unwrap();

    // Append edit revision
    let updated = toolcase_gateway::museai::append_job_revision(
        &job_id,
        "edit: make it brighter and add cinematic lighting",
        "https://muse.ai/files/v2.mp4",
        "artifact_reported",
    )
    .unwrap();

    assert_eq!(updated.revisions.len(), 2);
    assert_eq!(updated.video_url, "https://muse.ai/files/v2.mp4");
    assert_eq!(updated.revisions[1].revision_id, "rev-2");
    assert_eq!(
        updated.revisions[1].prompt,
        "edit: make it brighter and add cinematic lighting"
    );
    assert_eq!(
        updated.revisions[1].video_url,
        "https://muse.ai/files/v2.mp4"
    );
}

#[test]
fn test_job_id_validation_rejects_invalid_characters() {
    use toolcase_gateway::museai::lifecycle::job_file_path;

    for bad_id in [
        "",
        "id.with.dots",
        "../traversal",
        "has spaces",
        "bad/slash",
        "a*b",
    ] {
        let err = job_file_path(bad_id).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{bad_id}");
    }

    // Valid IDs succeed
    assert!(job_file_path("video-123_ABC-xyz").is_ok());
}

#[test]
fn test_concurrent_job_updates_are_synchronized() {
    let job_id = format!("video-concurrent-{}", uuid::Uuid::new_v4());
    let record = VideoJobRecord {
        job_id: job_id.clone(),
        owner_id: "user-test".into(),
        model: "muse-video".into(),
        requested_model: "gen-3".into(),
        prompt: "base prompt".into(),
        aspect_ratio: "16:9".into(),
        duration: 5,
        thread_id: None,
        created_at: 1000,
        last_activity_at: 1000,
        pinned: false,
        status: "artifact_reported".into(),
        media_validated: false,
        output_verified: false,
        video_url: "https://muse.ai/files/base.mp4".into(),
        revisions: vec![],
        ..Default::default()
    };
    save_video_job(&record).unwrap();

    let mut handles = vec![];
    for i in 0..5 {
        let jid = job_id.clone();
        handles.push(std::thread::spawn(move || {
            let prompt = format!("revision {i}");
            let url = format!("https://muse.ai/files/v{i}.mp4");
            toolcase_gateway::museai::append_job_revision(&jid, &prompt, &url, "artifact_reported")
                .unwrap();
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let final_record = toolcase_gateway::museai::load_video_job(&job_id).unwrap();
    assert_eq!(final_record.revisions.len(), 5);
}

#[test]
fn test_submitting_record_stale_detection_and_recovery() {
    use toolcase_gateway::museai::{is_submitting_stale, recover_stale_submitting_job};

    let now = 1_000_000_u64;

    // Stale record (> 300s since last_checked_at)
    let mut stale_record = VideoJobRecord {
        job_id: "video-stale-submitting".into(),
        status: "submitting".into(),
        created_at: now - 500,
        last_checked_at: Some(now - 400),
        ..Default::default()
    };
    assert!(is_submitting_stale(&stale_record, now));
    assert!(recover_stale_submitting_job(&mut stale_record, now));
    assert_eq!(stale_record.status, "submission_unknown");
    assert_eq!(stale_record.last_checked_at, Some(now));

    // Active record (< 300s)
    let mut active_record = VideoJobRecord {
        job_id: "video-active-submitting".into(),
        status: "submitting".into(),
        created_at: now - 60,
        last_checked_at: Some(now - 60),
        ..Default::default()
    };
    assert!(!is_submitting_stale(&active_record, now));
    assert!(!recover_stale_submitting_job(&mut active_record, now));
    assert_eq!(active_record.status, "submitting");

    // Non-submitting status is never submitting-stale
    let mut completed_record = VideoJobRecord {
        job_id: "video-completed".into(),
        status: "completed".into(),
        created_at: now - 5000,
        last_checked_at: Some(now - 5000),
        ..Default::default()
    };
    assert!(!is_submitting_stale(&completed_record, now));
    assert!(!recover_stale_submitting_job(&mut completed_record, now));
}

#[test]
fn test_load_video_job_recovers_stale_submitting_record() {
    let job_id = format!("video-stale-load-{}", uuid::Uuid::new_v4());
    let now = toolcase_gateway::museai::lifecycle::now_secs();

    let stale = VideoJobRecord {
        job_id: job_id.clone(),
        owner_id: "user-stale".into(),
        status: "submitting".into(),
        created_at: now.saturating_sub(600),
        last_activity_at: now.saturating_sub(600),
        last_checked_at: Some(now.saturating_sub(600)),
        ..Default::default()
    };
    save_video_job(&stale).unwrap();

    let loaded = load_video_job(&job_id).unwrap();
    assert_eq!(loaded.status, "submission_unknown");

    // Verify it was updated on disk as well
    let reloaded = load_video_job(&job_id).unwrap();
    assert_eq!(reloaded.status, "submission_unknown");
}

#[test]
fn test_stale_submitting_retry_does_not_block_with_would_block() {
    let idempotency_key = format!("idem-stale-{}", uuid::Uuid::new_v4());
    let job_id = format!("video-stale-retry-{}", uuid::Uuid::new_v4());
    let now = toolcase_gateway::museai::lifecycle::now_secs();

    save_video_job(&VideoJobRecord {
        job_id,
        status: "submitting".into(),
        submission_key: Some(idempotency_key.clone()),
        created_at: now.saturating_sub(500),
        last_activity_at: now.saturating_sub(500),
        last_checked_at: Some(now.saturating_sub(500)),
        ..Default::default()
    })
    .unwrap();

    let req = serde_json::json!({
        "prompt": "prompt with stale submitting key",
        "idempotency_key": idempotency_key,
    });

    let config = toolcase_gateway::config::Config::default();
    let err = toolcase_gateway::museai::video::create_video_with_completion(
        &serde_json::to_vec(&req).unwrap(),
        &config,
        |_body, _cfg, _wait| Ok("{}".into()),
    )
    .unwrap_err();

    // Must NOT be WouldBlock ("generation in progress")
    assert_ne!(err.kind(), std::io::ErrorKind::WouldBlock);
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    assert!(err.to_string().contains("submission_unknown"));
}

#[test]
fn test_status_update_failure_is_propagated_on_completion_error() {
    let job_id = format!("video-update-fail-{}", uuid::Uuid::new_v4());
    let req = serde_json::json!({
        "id": job_id.clone(),
        "prompt": "status update failure propagation test",
    });

    let config = toolcase_gateway::config::Config::default();
    let err = toolcase_gateway::museai::video::create_video_with_completion(
        &serde_json::to_vec(&req).unwrap(),
        &config,
        |_body, _cfg, _wait| {
            // Delete the reserved job file before completion returns error,
            // so update_video_job will fail with NotFound
            let path = toolcase_gateway::museai::lifecycle::job_file_path(&job_id).unwrap();
            let _ = std::fs::remove_file(path);
            Err(std::io::Error::other("upstream simulated timeout"))
        },
    )
    .unwrap_err();

    // Verify the error is propagated and mentions failure to update status
    assert!(
        err.to_string()
            .contains("failed to update video job status"),
        "error must mention status update failure: {err}"
    );
}
