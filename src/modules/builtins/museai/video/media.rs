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
