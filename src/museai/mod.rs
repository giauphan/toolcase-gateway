pub(crate) mod business;
pub(crate) mod chat;
pub(crate) mod handlers;
pub(crate) mod noise;
pub(crate) mod protocol;
pub(crate) mod session;
pub(crate) mod threads;
pub(crate) mod transport;
pub(crate) mod video;

pub(crate) use chat::request_museai_chat_completion;
#[cfg(test)]
pub(crate) use chat::{
    build_muse_chat_request, explicit_video_refusal, extract_video_url_from_stream, MuseChatStream,
    MUSE_POST_COMPLETION_WAIT_SECS,
};
pub(crate) use handlers::{
    handle_create_video, handle_museai_thread_cleanup, handle_museai_v1, write_chat_completion,
};
#[cfg(test)]
pub(crate) use session::{bootstrap_museai_config, build_museai_ws_url};
#[cfg(test)]
pub(crate) use threads::{cleanup_tracked_threads_once, tracked_threads};
pub(crate) use threads::{delete_muse_thread, register_thread, start_cleanup_worker};
#[cfg(test)]
pub(crate) use video::extract_url_from_text;
#[cfg(test)]
pub(crate) use video::{
    build_video_prompt, build_video_result, create_video, is_supported_video_url,
    normalize_video_model,
};

#[cfg(test)]
use crate::config::Config;
#[cfg(test)]
use std::io;

#[derive(Debug)]
pub(crate) struct MuseApprovalRequired(pub(crate) String);

impl std::fmt::Display for MuseApprovalRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for MuseApprovalRequired {}

// Retained for legacy HTTP-action fixture coverage, not the production chat flow.
#[cfg(test)]
pub(crate) fn spawn_muse_thread(base_url: &str, config: &Config) -> io::Result<String> {
    let create_thread_url = format!("{base_url}/thread/new");
    let mut req = ureq::post(&create_thread_url)
        .header("Content-Type", "text/plain;charset=UTF-8")
        .header("Accept", "text/x-component")
        .header("Origin", base_url)
        .header("Referer", &format!("{base_url}/thread/new"))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36")
        .header("sec-ch-ua", r#""Brave";v="153", "Not_A Brand";v="8", "Chromium";v="153""#)
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", r#""Windows""#)
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin")
        .header("next-action", "00eaaa570484d537580e0bf4e19ee28a65ac048d62");

    if !config.museai_cookie.is_empty() {
        req = req.header("Cookie", &config.museai_cookie);
    }

    let response = match req.send("[]") {
        Ok(res) => res,
        Err(ureq::Error::StatusCode(401)) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Muse.ai thread creation returned 401 Unauthorized — credentials are invalid or expired",
            ));
        }
        Err(ureq::Error::StatusCode(403)) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Muse.ai thread creation returned 403 Forbidden — access denied",
            ));
        }
        Err(ureq::Error::StatusCode(404)) => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Muse.ai thread creation returned 404 — endpoint not found or cookie expired",
            ));
        }
        Err(ureq::Error::StatusCode(status)) => {
            return Err(io::Error::other(format!(
                "Muse.ai thread creation returned HTTP {status}"
            )));
        }
        Err(e) => {
            return Err(io::Error::other(format!(
                "Thread creation request failed: {e}"
            )));
        }
    };
    let mut reader = response.into_body();
    let text = reader
        .read_to_string()
        .map_err(|e| io::Error::other(format!("Failed to read thread response: {e}")))?;
    let thread_id = extract_new_thread_id(&text);
    if thread_id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "failed to extract thread ID from /thread/new response",
        ));
    }
    let channel = format!("thread:{thread_id}");
    Ok(channel)
}

#[cfg(test)]
fn extract_new_thread_id(rsc_text: &str) -> String {
    let needle = "/thread/";
    if let Some(idx) = rsc_text.rfind(needle) {
        let start = idx + needle.len();
        let remaining = &rsc_text[start..];
        let end = remaining
            .find(|c: char| !c.is_alphanumeric() && c != '-')
            .unwrap_or(remaining.len());
        return remaining[..end].to_string();
    }
    String::new()
}

#[cfg(test)]
pub(crate) mod museai_tests;
