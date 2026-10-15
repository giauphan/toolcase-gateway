use toolcase_gateway::museai::{
    list_video_jobs, load_video_job, save_video_job, VideoJobRecord, VideoRevision,
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
    assert_eq!(updated.revisions[1].prompt, "edit: make it brighter and add cinematic lighting");
    assert_eq!(updated.revisions[1].video_url, "https://muse.ai/files/v2.mp4");
}

#[test]
fn test_cleanup_expired_video_jobs_removes_only_eligible_records() {
    let expired_id = format!("video-expired-{}", uuid::Uuid::new_v4());
    let pinned_id = format!("video-pinned-{}", uuid::Uuid::new_v4());
    let active_id = format!("video-active-{}", uuid::Uuid::new_v4());

    for (job_id, status, pinned) in [
        (expired_id.clone(), "artifact_reported", false),
        (pinned_id.clone(), "artifact_reported", true),
        (active_id.clone(), "generating", false),
    ] {
        save_video_job(&VideoJobRecord {
            job_id,
            owner_id: "user-cleanup".into(),
            model: "muse-video".into(),
            requested_model: "gen-3".into(),
            prompt: "cleanup test".into(),
            aspect_ratio: "16:9".into(),
            duration: 5,
            thread_id: None,
            created_at: 1,
            last_activity_at: 1,
            pinned,
            status: status.into(),
            media_validated: false,
            output_verified: false,
            video_url: "https://muse.ai/files/cleanup.mp4".into(),
            revisions: vec![],
        })
        .unwrap();
    }

    let cleaned = toolcase_gateway::museai::lifecycle::cleanup_expired_video_jobs(1).unwrap();
    assert!(cleaned >= 1);
    assert!(load_video_job(&expired_id).is_err());
    assert!(load_video_job(&pinned_id).is_ok());
    assert!(load_video_job(&active_id).is_ok());
}

#[test]
fn test_job_id_validation_rejects_invalid_characters() {
    use toolcase_gateway::museai::lifecycle::job_file_path;

    for bad_id in ["", "id.with.dots", "../traversal", "has spaces", "bad/slash", "a*b"] {
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
