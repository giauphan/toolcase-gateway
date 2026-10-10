//! Shared imports and helpers for Muse integration test modules.

pub use toolcase_gateway::config::Config;
pub use toolcase_gateway::museai::chat::{
    build_muse_chat_request, explicit_video_refusal, extract_video_url_from_stream, MuseChatStream,
    MUSE_POST_COMPLETION_WAIT_SECS,
};
pub use toolcase_gateway::museai::threads::{
    cleanup_tracked_threads_once, extract_new_thread_id, register_thread, tracked_threads,
};
pub use toolcase_gateway::museai::video::{
    build_video_prompt, build_video_result, create_video, extract_url_from_text,
    normalize_video_model,
};
pub use toolcase_gateway::museai::MuseApprovalRequired;

pub fn test_config() -> Config {
    Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        fallbacks: vec![],
        io_timeout: None,
        retry_base_delay_ms: 100,
        max_retry_delay_ms: 1000,
        default_model: "gpt-5.6-sol".into(),
        museai_base_url: "https://muse.ai".into(),
        museai_cookie: "".into(),
        museai_ws_url: "".into(),
        museai_access_token: "".into(),
        museai_notary_token: "".into(),
        museai_vm_id: "".into(),
        museai_auto_cleanup_threads: true,
        museai_thread_retention_secs: 86400,
        jev_api_keys: vec![],
    }
}

pub fn acknowledged_chat_stream() -> MuseChatStream {
    let mut stream = MuseChatStream::default();
    stream
        .record(&serde_json::json!({
            "session_id": "00000000-0000-4000-8000-000000000001", "is_thread": true
        }))
        .unwrap();
    stream
}
