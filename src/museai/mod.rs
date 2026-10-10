pub mod business;
pub mod chat;
pub mod handlers;
pub mod noise;
pub mod protocol;
pub mod session;
pub mod threads;
pub mod transport;
pub mod video;

pub use chat::request_museai_chat_completion;
pub use handlers::{
    handle_create_video, handle_museai_thread_cleanup, handle_museai_v1, write_chat_completion,
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
