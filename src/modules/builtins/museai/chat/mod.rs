mod executor;
mod parser;
mod prompt;
mod stream;

pub use crate::providers::museai::media::ArtifactWait;
pub use executor::{
    request_museai_chat_completion, MUSE_POST_COMPLETION_IMAGE_WAIT_SECS,
    MUSE_POST_COMPLETION_WAIT_SECS,
};
pub use parser::{
    explicit_video_refusal, extract_video_url_from_stream, find_image_url_in_json,
    find_video_url_in_json, parse_assistant_content_from_json,
};
pub use prompt::{
    build_muse_chat_request, build_muse_chat_request_with_items,
    build_muse_chat_request_with_session, extract_attachment_items, extract_continuation_prompt,
    extract_thread_id, extract_thread_id_from_value, extract_user_prompt,
};
pub use stream::MuseChatStream;

pub(super) fn debug_log(msg: &str) {
    if std::env::var("MUSEAI_DEBUG").is_ok() {
        crate::runtime::logger::log_message("DEBUG", "museai_chat", msg);
    }
}
