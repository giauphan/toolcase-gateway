use super::{
    build_muse_chat_request_with_session, debug_log, explicit_video_refusal,
    extract_continuation_prompt, extract_thread_id, extract_user_prompt,
    extract_video_url_from_stream, MuseChatStream,
};
use crate::config::Config;
use crate::providers::museai::noise::{MuseNoiseSession, NOISE_PATTERN_XX};
use crate::providers::museai::protocol::{Header, ServiceFrameKind, SERVICE_DAEMON};
use crate::providers::museai::session::{bootstrap_museai_config, build_museai_ws_url};
use crate::providers::museai::transport::MuseWebSocket;
use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const MUSE_POST_COMPLETION_WAIT_SECS: u64 = 300;

fn init_noise_session(
    config: &Config,
    base_origin: &str,
) -> io::Result<(MuseWebSocket, MuseNoiseSession)> {
    let req_id = Uuid::new_v4().to_string();
    let ws_url = build_museai_ws_url(config, &req_id)?;

    crate::runtime::logger::log_message(
        "INFO",
        "museai_chat",
        &format!("Connecting to Muse CVM WebSocket (origin: {base_origin})"),
    );

    let mut socket = MuseWebSocket::connect(&ws_url, base_origin, "")?;
    let mut session = MuseNoiseSession::new_initiator(NOISE_PATTERN_XX, None)
        .map_err(|e| io::Error::other(format!("Noise initiator error: {e}")))?;

    session
        .perform_client_handshake(&mut socket, &config.museai_notary_token)
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("401")
                || msg.contains("403")
                || msg.to_ascii_lowercase().contains("unauthorized")
            {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("Noise handshake authentication rejected: cookie or session token expired ({e})"),
                )
            } else {
                io::Error::other(format!("Noise handshake error: {e}"))
            }
        })?;

    Ok((socket, session))
}

fn drive_chat_stream(
    socket: &mut MuseWebSocket,
    session: &mut MuseNoiseSession,
    mut stream: MuseChatStream,
    config: &Config,
    headers: &[Header],
) -> io::Result<(String, String, u64)> {
    let stream_id: u64 = 1;
    let mut active_stream_id: u64 = stream_id;
    for _ in 0..1000 {
        let frame = socket.read_encrypted_service_frame(session)?;
        if frame.stream_id != active_stream_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unexpected Muse response stream",
            ));
        }
        let (bytes, ended) = match frame.kind {
            ServiceFrameKind::Response {
                status,
                body,
                end_body,
                ..
            } => {
                debug_log(&format!("Chat response HTTP status={status}"));
                if !(200..300).contains(&status) {
                    if status == 401 || status == 403 {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            format!("Muse chat stream authentication failed (HTTP {status}): cookie or session token expired"),
                        ));
                    }
                    return Err(io::Error::other(format!(
                        "Muse chat returned HTTP {status}"
                    )));
                }
                (body, end_body)
            }
            ServiceFrameKind::BodyChunk { data, end_body } => (data, end_body),
            ServiceFrameKind::Reset { code, .. } => {
                return Err(io::Error::other(format!("Muse chat reset code={code}")));
            }
        };
        stream.push(&bytes, ended)?;
        if ended && active_stream_id == stream_id && !stream.completed {
            let session_id = stream.session_id.clone().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Muse chat acknowledgement missing",
                )
            })?;
            if std::env::var("MUSE_LIVE_TEST").ok().as_deref() == Some("1") {
                crate::test_helpers::save_live_owned_session(&session_id)?;
            }

            attach_scoped_approval_handler(&mut stream, config, &session_id);

            let subscription = serde_json::json!({
                "session_id": session_id,
                "after_stream_seq": 0,
                "after_chat_event_seq": 0,
                "capabilities": ["chat_cancel", "delta_stream"]
            });
            active_stream_id = 2;
            socket.send_encrypted_service_request(
                session,
                SERVICE_DAEMON,
                active_stream_id,
                "POST",
                "/chat/subscribe",
                headers,
                subscription.to_string().as_bytes(),
            )?;
            continue;
        }
        if stream.completed || ended {
            break;
        }
    }
    let (thread_id, full_assistant_text) = stream.finish()?;
    Ok((thread_id, full_assistant_text, active_stream_id))
}

fn attach_scoped_approval_handler(stream: &mut MuseChatStream, config: &Config, session_id: &str) {
    if config.muse_approval.mode == crate::config::MuseApprovalMode::ScopedAuto {
        let mut coordinator =
            crate::providers::museai::approval::ApprovalCoordinator::new(config, session_id);
        stream.approval_handler = Some(Box::new(move |_sid, record| {
            match coordinator.handle_approval_snapshot(record) {
                Ok(crate::providers::museai::approval::CoordinateResult::Approved { .. }) => {
                    Ok(true)
                }
                Ok(crate::providers::museai::approval::CoordinateResult::ManualRequired {
                    reason,
                    ..
                }) => Err(io::Error::other(reason)),
                Err(e) => Err(e),
            }
        }));
    }
}

fn await_video_artifact(
    socket: &mut MuseWebSocket,
    session: &mut MuseNoiseSession,
    config: &Config,
    thread_id: &str,
) -> io::Result<Option<String>> {
    let deadline = std::time::Instant::now() + Duration::from_secs(MUSE_POST_COMPLETION_WAIT_SECS);
    let mut presentation_stream = MuseChatStream {
        session_id: Some(thread_id.to_string()),
        ..Default::default()
    };
    attach_scoped_approval_handler(&mut presentation_stream, config, thread_id);
    while std::time::Instant::now() < deadline {
        match socket.read_encrypted_service_frame(session) {
            Ok(frame) => {
                let (bytes, ended) = match frame.kind {
                    ServiceFrameKind::Response { body, end_body, .. } => (body, end_body),
                    ServiceFrameKind::BodyChunk { data, end_body } => (data, end_body),
                    ServiceFrameKind::Reset { .. } => break,
                };
                presentation_stream.push(&bytes, ended)?;
                if let Some(extracted) = extract_video_url_from_stream(&presentation_stream) {
                    debug_log("Video artifact URL received");
                    return Ok(Some(extracted));
                }
            }
            Err(e) => {
                debug_log(&format!("Video URL read error: {e}"));
                break;
            }
        }
    }
    Ok(None)
}

fn cache_vm_media(
    socket: &mut MuseWebSocket,
    session: &mut MuseNoiseSession,
    raw_url: &str,
    stream_id: u64,
    job_id_hint: Option<&str>,
    is_image: bool,
) {
    if !crate::providers::museai::video::is_vm_media_url(raw_url) {
        return;
    }
    let Some(vm_path) = crate::providers::museai::video::extract_vm_media_path(raw_url) else {
        return;
    };
    let kind = if is_image { "image" } else { "video" };
    crate::runtime::logger::log_message(
        "INFO",
        "museai_chat",
        &format!("Fetching internal VM {kind} media over Noise: {vm_path}"),
    );
    match crate::providers::museai::video::fetch_vm_media_over_noise(
        socket, session, stream_id, &vm_path,
    ) {
        Ok(media_bytes) => {
            crate::runtime::logger::log_message(
                "INFO",
                "museai_chat",
                &format!(
                    "Fetched and cached {} bytes of VM {kind} media",
                    media_bytes.len()
                ),
            );
            if let Some(jid) = job_id_hint {
                if is_image {
                    let ext = crate::providers::museai::image::guess_extension(&vm_path);
                    let _ =
                        crate::providers::museai::image::save_image_media(jid, &media_bytes, ext);
                } else {
                    let _ = crate::providers::museai::video::save_video_media(jid, &media_bytes);
                }
            }
        }
        Err(e) => {
            crate::runtime::logger::log_message(
                "ERROR",
                "museai_chat",
                &format!("Failed to fetch VM {kind} media over Noise: {e}"),
            );
        }
    }
}

fn handle_thread_cleanup(
    socket: &mut MuseWebSocket,
    session: &mut MuseNoiseSession,
    config: &Config,
    thread_id: &str,
    base_origin: &str,
    stream_id: u64,
    headers: &[Header],
) {
    if !config.museai_auto_cleanup_threads || thread_id.is_empty() {
        return;
    }
    if config.museai_thread_retention_secs == 0 {
        let delete_payload = serde_json::json!({
            "method": "/api/session/delete",
            "session_id": thread_id,
            "id": thread_id,
        });
        if let Ok(bytes) = serde_json::to_vec(&delete_payload) {
            let _ = socket.send_encrypted_service_request(
                session,
                SERVICE_DAEMON,
                stream_id,
                "POST",
                "/api/session/delete",
                headers,
                &bytes,
            );
        }
        crate::providers::museai::unregister_thread(thread_id);
        crate::runtime::logger::log_message(
            "INFO",
            "museai_chat",
            &format!("Purged ephemeral thread {thread_id} directly on CVM connection"),
        );
    } else {
        crate::runtime::logger::log_message(
            "INFO",
            "museai_chat",
            &format!(
                "Registered thread {} with retention TTL {}s",
                thread_id, config.museai_thread_retention_secs
            ),
        );
        crate::providers::museai::register_thread(
            thread_id.to_string(),
            base_origin.to_string(),
            config.clone(),
        );
    }
}

pub fn request_museai_chat_completion(
    request_body: &[u8],
    config: &Config,
    wait_for_video_artifact: bool,
) -> io::Result<String> {
    let existing_thread_id = extract_thread_id(request_body)?;
    let prompt = if existing_thread_id.is_some() {
        extract_continuation_prompt(request_body)?
    } else {
        extract_user_prompt(request_body)?
    };
    let job_id_hint = serde_json::from_slice::<serde_json::Value>(request_body)
        .ok()
        .and_then(|v| v.get("job_id").and_then(|j| j.as_str()).map(str::to_string));
    let config = bootstrap_museai_config(config)?;

    let base_url = if config.museai_base_url.is_empty() {
        "https://muse.ai"
    } else {
        config.museai_base_url.as_str()
    };
    let base_origin = base_url.trim_end_matches('/');

    let (mut socket, mut session) = init_noise_session(&config, base_origin)?;

    let stream_id = 1;
    let headers = crate::providers::museai::protocol::standard_hatch_headers("text/event-stream");

    let chat_payload = build_muse_chat_request_with_session(&prompt, existing_thread_id.as_deref());
    let payload_bytes = serde_json::to_vec(&chat_payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    crate::runtime::logger::log_message(
        "INFO",
        "museai_chat",
        "Sending encrypted POST /chat/stream over Noise XX session",
    );

    socket.send_encrypted_service_request(
        &mut session,
        SERVICE_DAEMON,
        stream_id,
        "POST",
        "/chat/stream",
        &headers,
        &payload_bytes,
    )?;

    let stream = MuseChatStream::with_expected_turn(existing_thread_id, Some(prompt.clone()));
    let (thread_id, mut full_assistant_text, active_stream_id) =
        drive_chat_stream(&mut socket, &mut session, stream, &config, &headers)?;

    let mut video_url = {
        let extracted =
            crate::providers::museai::video::extract_url_from_text(&full_assistant_text);
        (!extracted.is_empty()).then_some(extracted)
    };
    let image_url = {
        let extracted =
            crate::providers::museai::image::extract_image_url_from_text(&full_assistant_text);
        (!extracted.is_empty()).then_some(extracted)
    };

    if wait_for_video_artifact
        && video_url.is_none()
        && !explicit_video_refusal(&full_assistant_text)
    {
        if let Some(artifact) =
            await_video_artifact(&mut socket, &mut session, &config, &thread_id)?
        {
            video_url = Some(artifact);
        }
    }

    if let Some(ref raw_url) = video_url {
        crate::runtime::logger::log_message(
            "INFO",
            "museai_chat",
            &format!("Resolved video artifact URL: {raw_url}"),
        );
        cache_vm_media(
            &mut socket,
            &mut session,
            raw_url,
            active_stream_id.max(2) + 1,
            job_id_hint.as_deref(),
            false,
        );
    }

    if let Some(ref raw_url) = image_url {
        crate::runtime::logger::log_message(
            "INFO",
            "museai_chat",
            &format!("Resolved image artifact URL: {raw_url}"),
        );
        cache_vm_media(
            &mut socket,
            &mut session,
            raw_url,
            active_stream_id.max(2) + 1,
            job_id_hint.as_deref(),
            true,
        );
    }

    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    handle_thread_cleanup(
        &mut socket,
        &mut session,
        &config,
        &thread_id,
        base_origin,
        active_stream_id.max(2) + 1,
        &headers,
    );

    if let Some(url) = &video_url {
        if !full_assistant_text.contains(url.as_str()) {
            full_assistant_text.push_str(&format!("\nVideo URL: {url}\n"));
        }
    }
    if let Some(url) = &image_url {
        if !full_assistant_text.contains(url.as_str()) {
            full_assistant_text.push_str(&format!("\nImage URL: {url}\n"));
        }
    }

    let completion_response = serde_json::json!({
        "id": format!("chatcmpl-{}", Uuid::new_v4()),
        "object": "chat.completion",
        "created": created,
        "model": "muse",
        "thread_id": thread_id,
        "session_id": thread_id,
        "reply_thread_id": thread_id,
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": full_assistant_text
                },
                "finish_reason": "stop"
            }
        ],
        "usage": {
            "prompt_tokens": prompt.len() / 4 + 1,
            "completion_tokens": full_assistant_text.len() / 4 + 1,
            "total_tokens": (prompt.len() + full_assistant_text.len()) / 4 + 2
        }
    });

    serde_json::to_string(&completion_response)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
