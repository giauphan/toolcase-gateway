pub mod approval;
pub mod business;
pub mod chat;
pub mod handlers;
pub mod image;
pub mod lifecycle;
pub mod noise;
pub mod protocol;
pub mod session;
pub mod threads;
pub mod transport;
pub mod video;

pub use approval::{ApprovalCoordinator, CoordinateResult, MuseSentinelClient, SentinelClient};
pub use chat::request_museai_chat_completion;
pub use handlers::{
    handle_create_video, handle_create_video_request, handle_get_video, handle_get_video_content,
    handle_get_video_request, handle_museai_chat_completions, handle_museai_thread_cleanup,
    handle_museai_threads_cleanup_run, handle_museai_v1, handle_update_video_prompt,
    is_sync_requested, write_chat_completion,
};
pub use image::{
    build_image_prompt, create_image, create_image_with_completion, extract_image_url_from_text,
    find_image_media, handle_create_image, handle_create_image_request, handle_get_image,
    handle_get_image_content, handle_get_image_request, has_image_media, load_image_job,
    load_image_media, normalize_image_model, parse_and_validate_image_request, save_image_job,
    save_image_media, submit_image, submit_image_with_completion, ImageJobRecord,
    ValidatedImageRequest, DEFAULT_IMAGE_ASPECT_RATIO, SUPPORTED_IMAGE_ASPECT_RATIOS,
    SUPPORTED_IMAGE_MODELS,
};
pub use lifecycle::{
    append_job_revision, is_submitting_stale, list_video_jobs, load_video_job,
    recover_stale_submitting_job, reserve_video_job, save_video_job, update_video_job,
    update_video_job_with_error, VideoJobRecord, VideoRevision, STALE_SUBMITTING_SECS,
};
pub use session::{bootstrap_museai_config, build_museai_ws_url};
pub use threads::spawn_muse_thread;
pub use threads::{
    cleanup_cycle_once, cleanup_cycle_with_config, cleanup_cycle_with_ttl,
    delete_cvm_session_over_noise, delete_muse_thread, extract_session_timestamp,
    purge_remote_cvm_threads_older_than, register_thread, start_cleanup_worker,
    start_cleanup_worker_with_interval, start_cleanup_worker_with_store, unregister_thread,
};
pub use video::build_video_prompt;

#[derive(Debug)]
pub struct MuseApprovalRequired(pub String);

impl std::fmt::Display for MuseApprovalRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for MuseApprovalRequired {}
