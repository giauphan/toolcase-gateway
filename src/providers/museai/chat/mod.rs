mod executor;
mod parser;
mod prompt;
mod stream;

pub use executor::{request_museai_chat_completion, MUSE_POST_COMPLETION_WAIT_SECS};
pub use parser::{
    explicit_video_refusal, extract_video_url_from_stream, find_video_url_in_json,
    parse_assistant_content_from_json,
};
pub use prompt::{build_muse_chat_request, extract_user_prompt};
pub use stream::MuseChatStream;

pub(super) fn debug_log(msg: &str) {
    if std::env::var("MUSEAI_DEBUG").is_ok() {
        eprintln!("[museai_chat] {msg}");
    }
}
