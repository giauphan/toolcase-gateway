pub mod approval;
pub mod business;
pub mod chat;
pub mod handlers;
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
    handle_create_video, handle_get_video, handle_museai_thread_cleanup, handle_museai_v1,
    handle_update_video_prompt, write_chat_completion,
};
pub use lifecycle::{
    append_job_revision, is_submitting_stale, list_video_jobs, load_video_job,
    recover_stale_submitting_job, reserve_video_job, save_video_job, update_video_job,
    VideoJobRecord, VideoRevision, STALE_SUBMITTING_SECS,
};
pub use session::{bootstrap_museai_config, build_museai_ws_url};
pub use threads::spawn_muse_thread;
pub use threads::{delete_muse_thread, register_thread, start_cleanup_worker};
pub use video::build_video_prompt;

#[derive(Debug)]
pub struct MuseApprovalRequired(pub String);

impl std::fmt::Display for MuseApprovalRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for MuseApprovalRequired {}
