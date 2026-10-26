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

    let req_id = Uuid::new_v4().to_string();
    let ws_url = build_museai_ws_url(&config, &req_id)?;

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
        .map_err(|e| io::Error::other(format!("Noise handshake error: {e}")))?;

    let stream_id = 1;
    let headers = vec![
        Header {
            key: "Host".to_string(),
            value: "hatch.metaaivm.com".to_string(),
        },
        Header {
            key: "Content-Type".to_string(),
            value: "application/json".to_string(),
        },
        Header {
            key: "Accept".to_string(),
            value: "text/event-stream".to_string(),
        },
        Header {
            key: "x-request-id".to_string(),
            value: Uuid::new_v4().to_string(),
        },
        Header {
            key: "x-app-id".to_string(),
            value: "hatch-web".to_string(),
        },
        Header {
            key: "Accept-Language".to_string(),
            value: "en-US".to_string(),
        },
    ];

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

    let mut stream = MuseChatStream::with_expected_turn(existing_thread_id, Some(prompt.clone()));
    let mut active_stream_id = stream_id;
    for _ in 0..1000 {
        let frame = socket.read_encrypted_service_frame(&mut session)?;
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
            let session_id = stream.session_id.as_deref().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Muse chat acknowledgement missing",
                )
            })?;
            if std::env::var("MUSE_LIVE_TEST").ok().as_deref() == Some("1") {
                crate::test_helpers::save_live_owned_session(session_id)?;
            }

            if config.muse_approval.mode == crate::config::MuseApprovalMode::ScopedAuto {
                let mut coordinator = crate::providers::museai::approval::ApprovalCoordinator::new(
                    &config, session_id,
                );
                stream.approval_handler = Some(Box::new(move |_sid, record| {
                    match coordinator.handle_approval_snapshot(record) {
                        Ok(crate::providers::museai::approval::CoordinateResult::Approved {
                            ..
                        }) => Ok(true),
                        Ok(
                            crate::providers::museai::approval::CoordinateResult::ManualRequired {
                                reason,
                                ..
                            },
                        ) => Err(io::Error::other(reason)),
                        Err(e) => Err(e),
                    }
                }));
            }

            let subscription = serde_json::json!({
                "session_id": session_id,
                "after_stream_seq": 0,
                "after_chat_event_seq": 0,
                "capabilities": ["chat_cancel", "delta_stream"]
            });
            active_stream_id = 2;
            socket.send_encrypted_service_request(
                &mut session,
                SERVICE_DAEMON,
                active_stream_id,
                "POST",
                "/chat/subscribe",
                &headers,
                subscription.to_string().as_bytes(),
            )?;
            continue;
        }
        if stream.completed || ended {
            break;
        }
    }
    let (thread_id, full_assistant_text) = stream.finish()?;

    let mut video_url: Option<String> = {
        let extracted =
            crate::providers::museai::video::extract_url_from_text(&full_assistant_text);
        (!extracted.is_empty()).then_some(extracted)
    };
    let image_url: Option<String> = {
        let extracted =
            crate::providers::museai::image::extract_image_url_from_text(&full_assistant_text);
        (!extracted.is_empty()).then_some(extracted)
    };
    if wait_for_video_artifact
        && video_url.is_none()
        && !explicit_video_refusal(&full_assistant_text)
    {
        let deadline =
            std::time::Instant::now() + Duration::from_secs(MUSE_POST_COMPLETION_WAIT_SECS);
        let mut presentation_stream = MuseChatStream {
            session_id: Some(thread_id.clone()),
            ..Default::default()
        };
        if config.muse_approval.mode == crate::config::MuseApprovalMode::ScopedAuto {
            let mut coordinator =
                crate::providers::museai::approval::ApprovalCoordinator::new(&config, &thread_id);
            presentation_stream.approval_handler = Some(Box::new(move |_sid, record| {
                match coordinator.handle_approval_snapshot(record) {
                    Ok(crate::providers::museai::approval::CoordinateResult::Approved {
                        ..
                    }) => Ok(true),
                    Ok(crate::providers::museai::approval::CoordinateResult::ManualRequired {
                        reason,
                        ..
                    }) => Err(io::Error::other(reason)),
                    Err(e) => Err(e),
                }
            }));
        }
        while std::time::Instant::now() < deadline && video_url.is_none() {
            match socket.read_encrypted_service_frame(&mut session) {
                Ok(frame) => {
                    let (bytes, ended) = match frame.kind {
                        ServiceFrameKind::Response { body, end_body, .. } => (body, end_body),
                        ServiceFrameKind::BodyChunk { data, end_body } => (data, end_body),
                        ServiceFrameKind::Reset { .. } => break,
                    };
                    presentation_stream.push(&bytes, ended)?;
                    if let Some(extracted) = extract_video_url_from_stream(&presentation_stream) {
                        debug_log("Video artifact URL received");
                        video_url = Some(extracted);
                        break;
                    }
                }
                Err(e) => {
                    debug_log(&format!("Video URL read error: {e}"));
                    break;
                }
            }
        }
    }

    if let Some(ref raw_url) = video_url {
        crate::runtime::logger::log_message(
            "INFO",
            "museai_chat",
            &format!("Resolved video artifact URL: {raw_url}"),
        );
        if crate::providers::museai::video::is_vm_media_url(raw_url) {
            if let Some(vm_path) = crate::providers::museai::video::extract_vm_media_path(raw_url) {
                crate::runtime::logger::log_message(
                    "INFO",
                    "museai_chat",
                    &format!("Fetching internal VM media over Noise: {vm_path}"),
                );
                let fetch_stream_id = active_stream_id.max(2) + 1;
                match crate::providers::museai::video::fetch_vm_media_over_noise(
                    &mut socket,
                    &mut session,
                    fetch_stream_id,
                    &vm_path,
                ) {
                    Ok(media_bytes) => {
                        crate::runtime::logger::log_message(
                            "INFO",
                            "museai_chat",
                            &format!(
                                "Fetched and cached {} bytes of VM video media",
                                media_bytes.len()
                            ),
                        );
                        if let Some(ref jid) = job_id_hint {
                            let _ = crate::providers::museai::video::save_video_media(
                                jid,
                                &media_bytes,
                            );
                        }
                    }
                    Err(e) => {
                        crate::runtime::logger::log_message(
                            "ERROR",
                            "museai_chat",
                            &format!("Failed to fetch VM media over Noise: {e}"),
                        );
                    }
                }
            }
        }
    }

    if let Some(ref raw_url) = image_url {
        crate::runtime::logger::log_message(
            "INFO",
            "museai_chat",
            &format!("Resolved image artifact URL: {raw_url}"),
        );
        if crate::providers::museai::video::is_vm_media_url(raw_url) {
            if let Some(vm_path) = crate::providers::museai::video::extract_vm_media_path(raw_url) {
                crate::runtime::logger::log_message(
                    "INFO",
                    "museai_chat",
                    &format!("Fetching internal VM image media over Noise: {vm_path}"),
                );
                let fetch_stream_id = active_stream_id.max(2) + 1;
                match crate::providers::museai::video::fetch_vm_media_over_noise(
                    &mut socket,
                    &mut session,
                    fetch_stream_id,
                    &vm_path,
                ) {
                    Ok(media_bytes) => {
                        crate::runtime::logger::log_message(
                            "INFO",
                            "museai_chat",
                            &format!(
                                "Fetched and cached {} bytes of VM image media",
                                media_bytes.len()
                            ),
                        );
                        if let Some(ref jid) = job_id_hint {
                            let ext = crate::providers::museai::image::guess_extension(&vm_path);
                            let _ = crate::providers::museai::image::save_image_media(
                                jid,
                                &media_bytes,
                                ext,
                            );
                        }
                    }
                    Err(e) => {
                        crate::runtime::logger::log_message(
                            "ERROR",
                            "museai_chat",
                            &format!("Failed to fetch VM image media over Noise: {e}"),
                        );
                    }
                }
            }
        }
    }

    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if config.museai_auto_cleanup_threads && !thread_id.is_empty() {
        if config.museai_thread_retention_secs == 0 {
            let delete_stream_id = active_stream_id.max(2) + 1;
            let delete_payload = serde_json::json!({
                "method": "/api/session/delete",
                "session_id": thread_id,
                "id": thread_id,
            });
            if let Ok(bytes) = serde_json::to_vec(&delete_payload) {
                let _ = socket.send_encrypted_service_request(
                    &mut session,
                    SERVICE_DAEMON,
                    delete_stream_id,
                    "POST",
                    "/api/session/delete",
                    &headers,
                    &bytes,
                );
            }
            crate::providers::museai::unregister_thread(&thread_id);
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
                thread_id.clone(),
                base_origin.to_string(),
                config.clone(),
            );
        }
    }

    let mut full_assistant_text = full_assistant_text;
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
