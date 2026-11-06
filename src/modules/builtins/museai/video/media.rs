use crate::providers::museai::noise::MuseNoiseSession;
use crate::providers::museai::protocol::{Header, ServiceFrameKind, SERVICE_DAEMON};
use crate::providers::museai::transport::MuseWebSocket;
use std::fs;
use std::io;
use std::path::PathBuf;

pub const DEFAULT_MEDIA_DIR: &str = ".gw_state/media";

pub fn media_dir() -> PathBuf {
    std::env::var("GW_STATE_DIR")
        .map(|s| PathBuf::from(s).join("media"))
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_MEDIA_DIR))
}

pub fn video_media_path(job_id: &str) -> PathBuf {
    media_dir().join(format!("{job_id}.mp4"))
}

pub fn has_video_media(job_id: &str) -> bool {
    let p = video_media_path(job_id);
    p.is_file()
}

pub fn save_video_media(job_id: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let dir = media_dir();
    fs::create_dir_all(&dir)?;
    let final_path = video_media_path(job_id);
    let tmp_path = dir.join(format!(".tmp-{}-{}", job_id, uuid::Uuid::new_v4()));
    fs::write(&tmp_path, bytes)?;
    if let Err(e) = fs::rename(&tmp_path, &final_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(final_path)
}

pub fn load_video_media(job_id: &str) -> io::Result<Vec<u8>> {
    fs::read(video_media_path(job_id))
}

pub fn fetch_vm_media_over_noise(
    socket: &mut MuseWebSocket,
    noise: &mut MuseNoiseSession,
    stream_id: u64,
    workspace_path: &str,
) -> io::Result<Vec<u8>> {
    let clean_path = workspace_path.trim_start_matches('/');
    let encoded_path = clean_path
        .split('/')
        .map(|segment| {
            let mut s = String::new();
            for b in segment.bytes() {
                if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
                    s.push(b as char);
                } else if b == b' ' {
                    s.push_str("%20");
                } else {
                    use std::fmt::Write;
                    let _ = write!(s, "%{:02X}", b);
                }
            }
            s
        })
        .collect::<Vec<_>>()
        .join("/");
    let raw_request_path = format!("/fs/raw/{encoded_path}");
    let headers = vec![
        Header {
            key: "x-app-id".into(),
            value: "hatch-web".into(),
        },
        Header {
            key: "x-request-id".into(),
            value: uuid::Uuid::new_v4().to_string(),
        },
        Header {
            key: "Accept".into(),
            value: "*/*".into(),
        },
    ];
    socket.send_encrypted_service_request(
        noise,
        SERVICE_DAEMON,
        stream_id,
        "GET",
        &raw_request_path,
        &headers,
        &[],
    )?;

    let mut body = Vec::new();
    loop {
        let frame = socket.read_encrypted_service_frame(noise)?;
        if frame.stream_id != stream_id {
            continue;
        }
        match frame.kind {
            ServiceFrameKind::Response {
                status,
                body: initial_body,
                end_body,
                ..
            } => {
                if status != 200 {
                    return Err(io::Error::other(format!(
                        "fs.raw returned HTTP status {status}"
                    )));
                }
                body.extend_from_slice(&initial_body);
                if end_body {
                    break;
                }
            }
            ServiceFrameKind::BodyChunk { data, end_body } => {
                body.extend_from_slice(&data);
                if end_body {
                    break;
                }
            }
            ServiceFrameKind::Reset { code, reason } => {
                return Err(io::Error::other(format!(
                    "fs.raw reset stream with code {code}: {reason}"
                )));
            }
        }
    }
    Ok(body)
}

pub fn write_vm_file_over_noise(
    socket: &mut MuseWebSocket,
    noise: &mut MuseNoiseSession,
    stream_id_counter: &mut u64,
    workspace_path: &str,
    data_bytes: &[u8],
) -> io::Result<()> {
    const CHUNK_SIZE: usize = 512 * 1024; // 524,288 bytes (Hatch standard 512 KB chunk)
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;

    let clean_path = workspace_path.trim_start_matches('/');
    let total_chunks = if data_bytes.is_empty() {
        1
    } else {
        data_bytes.len().div_ceil(CHUNK_SIZE)
    };

    for chunk_idx in 0..total_chunks {
        let start = chunk_idx * CHUNK_SIZE;
        let end = (start + CHUNK_SIZE).min(data_bytes.len());
        let slice = if data_bytes.is_empty() {
            &[][..]
        } else {
            &data_bytes[start..end]
        };
        let b64 = BASE64.encode(slice);
        let is_first = chunk_idx == 0;
        let payload = serde_json::json!({
            "path": clean_path,
            "data_base64": b64,
            "overwrite": is_first,
            "append": !is_first,
            "create_parent": is_first,
        });
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        let stream_id = *stream_id_counter;
        *stream_id_counter += 1;

        let headers = vec![
            Header {
                key: "Host".into(),
                value: "hatch.metaaivm.com".into(),
            },
            Header {
                key: "Content-Type".into(),
                value: "application/json".into(),
            },
            Header {
                key: "Accept".into(),
                value: "application/json".into(),
            },
            Header {
                key: "x-app-id".into(),
                value: "hatch-web".into(),
            },
            Header {
                key: "x-request-id".into(),
                value: uuid::Uuid::new_v4().to_string(),
            },
            Header {
                key: "Accept-Language".into(),
                value: "en-US".into(),
            },
        ];

        socket.send_encrypted_service_request(
            noise,
            SERVICE_DAEMON,
            stream_id,
            "POST",
            "/fs/write",
            &headers,
            &payload_bytes,
        )?;

        loop {
            let frame = socket.read_encrypted_service_frame(noise)?;
            if frame.stream_id != stream_id {
                continue;
            }
            match frame.kind {
                ServiceFrameKind::Response {
                    status, end_body, ..
                } => {
                    if status != 200 {
                        return Err(io::Error::other(format!(
                            "fs.write returned HTTP status {status}"
                        )));
                    }
                    if end_body {
                        break;
                    }
                }
                ServiceFrameKind::BodyChunk { end_body, .. } => {
                    if end_body {
                        break;
                    }
                }
                ServiceFrameKind::Reset { code, reason } => {
                    return Err(io::Error::other(format!(
                        "fs.write reset stream with code {code}: {reason}"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn resolve_attachment_bytes(item: &serde_json::Value) -> Option<Vec<u8>> {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;

    if let Some(job_id) = item.get("internal_job_id").and_then(|v| v.as_str()) {
        if let Ok((bytes, _)) = crate::providers::museai::image::load_image_media(job_id) {
            return Some(bytes);
        }
    }

    if let Some(r) = item.get("internal_ref").and_then(|v| v.as_str()) {
        let marker = "/muse-ai/v1/images/";
        if let Some(idx) = r.find(marker) {
            let after = &r[idx + marker.len()..];
            let job_id = after.split('/').next().unwrap_or(after);
            if let Ok((bytes, _)) = crate::providers::museai::image::load_image_media(job_id) {
                return Some(bytes);
            }
        }
    }

    if let Some(data_uri) = item.get("data_uri").and_then(|v| v.as_str()) {
        if let Some(idx) = data_uri.find(";base64,") {
            let b64 = &data_uri[idx + 8..];
            if let Ok(bytes) = BASE64.decode(b64.trim()) {
                return Some(bytes);
            }
        }
    }

    None
}

pub fn upload_attachments_to_vm_workspace(
    socket: &mut MuseWebSocket,
    noise: &mut MuseNoiseSession,
    attachment_items: &[serde_json::Value],
    next_stream_id: &mut u64,
) -> Vec<serde_json::Value> {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;

    let mut result = Vec::new();
    for item in attachment_items {
        let filename = item
            .get("filename")
            .and_then(|v| v.as_str())
            .unwrap_or("input_image.png");
        let mime_type = item
            .get("mime_type")
            .and_then(|v| v.as_str())
            .unwrap_or("image/png");
        let vm_path = item
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("workspace/your_files/{filename}"));

        let bytes_opt = resolve_attachment_bytes(item);
        if let Some(ref bytes) = bytes_opt {
            match write_vm_file_over_noise(socket, noise, next_stream_id, &vm_path, bytes) {
                Ok(_) => {
                    crate::runtime::logger::log_message(
                        "INFO",
                        "museai_chat",
                        &format!(
                            "Uploaded reference image to CVM workspace: {vm_path} ({} bytes)",
                            bytes.len()
                        ),
                    );
                    result.push(serde_json::json!({
                        "type": "file_ref",
                        "mime_type": mime_type,
                        "filename": filename,
                        "path": vm_path
                    }));
                }
                Err(e) => {
                    crate::runtime::logger::log_message(
                        "WARN",
                        "museai_chat",
                        &format!(
                            "Failed to write image to CVM workspace at {vm_path}: {e}; falling back to file payload"
                        ),
                    );
                    result.push(serde_json::json!({
                        "type": "file",
                        "mime_type": mime_type,
                        "filename": filename,
                        "data_base64": BASE64.encode(bytes)
                    }));
                }
            }
        } else {
            result.push(item.clone());
        }
    }
    result
}
