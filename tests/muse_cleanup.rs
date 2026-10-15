// GW_STATE_DIR changes below must stay in a single-test integration binary.
use toolcase_gateway::museai::{load_video_job, save_video_job, VideoJobRecord};

#[test]
fn cleanup_expired_video_jobs_removes_only_eligible_records() {
    let state_dir = std::env::temp_dir().join(format!(
        "toolcase-video-cleanup-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::env::set_var("GW_STATE_DIR", &state_dir);

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
    assert_eq!(cleaned, 1);
    assert!(load_video_job(&expired_id).is_err());
    assert!(load_video_job(&pinned_id).is_ok());
    assert!(load_video_job(&active_id).is_ok());

    std::fs::remove_dir_all(state_dir).unwrap();
}
