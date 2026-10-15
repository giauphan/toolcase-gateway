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
