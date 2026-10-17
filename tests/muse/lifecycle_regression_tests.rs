use std::fs;
use toolcase_gateway::museai::lifecycle::{
    job_file_path, list_video_jobs, reserve_video_job, save_video_job, VideoJobRecord,
};

#[test]
fn isolated_lifecycle_regressions() {
    let state_dir = std::env::temp_dir().join(format!("muse-lifecycle-{}", uuid::Uuid::new_v4()));
    std::env::set_var("GW_STATE_DIR", &state_dir);
    listing_valid_jobs_skips_corrupt_json_files();
    keyed_reservation_skips_corrupt_job_records();
    reservation_rejects_existing_job_id_without_overwriting_record();
    std::fs::remove_dir_all(state_dir).unwrap();
}

fn listing_valid_jobs_skips_corrupt_json_files() {
    let job_id = format!("video-corrupt-scan-{}", uuid::Uuid::new_v4());
    let valid = VideoJobRecord {
        job_id: job_id.clone(),
        status: "artifact_reported".into(),
        ..Default::default()
    };
    save_video_job(&valid).unwrap();
    let corrupt_path = job_file_path(&format!("corrupt-{}", uuid::Uuid::new_v4())).unwrap();
    fs::write(&corrupt_path, "not json").unwrap();

    let jobs = list_video_jobs().unwrap();

    assert!(jobs.iter().any(|job| job.job_id == job_id));
    fs::remove_file(corrupt_path).unwrap();
}

fn keyed_reservation_skips_corrupt_job_records() {
    let corrupt_path = job_file_path(&format!("corrupt-{}", uuid::Uuid::new_v4())).unwrap();
    fs::write(&corrupt_path, "not json").unwrap();
    let record = VideoJobRecord {
        job_id: format!("video-keyed-{}", uuid::Uuid::new_v4()),
        submission_key: Some(format!("key-{}", uuid::Uuid::new_v4())),
        ..Default::default()
    };

    let result = reserve_video_job(&record);

    fs::remove_file(corrupt_path).unwrap();
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
    assert!(toolcase_gateway::museai::load_video_job(&record.job_id).is_ok());
}

fn reservation_rejects_existing_job_id_without_overwriting_record() {
    let job_id = format!("video-collision-{}", uuid::Uuid::new_v4());
    let original = VideoJobRecord {
        job_id: job_id.clone(),
        prompt: "original prompt".into(),
        video_url: "https://muse.ai/files/original.mp4".into(),
        status: "artifact_reported".into(),
        ..Default::default()
    };
    save_video_job(&original).unwrap();

    let collision = VideoJobRecord {
        job_id: job_id.clone(),
        prompt: "replacement prompt".into(),
        status: "submitting".into(),
        ..Default::default()
    };
    let error = reserve_video_job(&collision).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    let stored = toolcase_gateway::museai::load_video_job(&job_id).unwrap();
    assert_eq!(stored.prompt, "original prompt");
    assert_eq!(stored.video_url, "https://muse.ai/files/original.mp4");
}
