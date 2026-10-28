use super::types::{
    parse_approval_get, parse_approvals_list, ApprovalRecord, MAX_APPROVAL_PAYLOAD_BYTES,
};
use crate::config::Config;
use crate::providers::museai::noise::{MuseNoiseSession, NOISE_PATTERN_XX};
use crate::providers::museai::protocol::{ServiceFrameKind, SERVICE_SENTINEL};
use crate::providers::museai::session::{bootstrap_museai_config, build_museai_ws_url};
use crate::providers::museai::transport::MuseWebSocket;
use std::io;
use uuid::Uuid;

pub trait SentinelClient {
    fn list_approvals(&mut self) -> io::Result<Vec<ApprovalRecord>>;
    fn get_approval(&mut self, approval_id: &str) -> io::Result<ApprovalRecord>;
    fn decide_approval(
        &mut self,
        approval_id: &str,
        decision_id: &str,
        decision_kind: &str,
    ) -> io::Result<ApprovalRecord>;
}

pub fn validate_approval_id(id: &str) -> io::Result<()> {
    if id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Approval ID cannot be empty",
        ));
    }
    if id.len() > 128 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Approval ID exceeds max length (128 chars)",
        ));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Approval ID contains invalid characters (only alphanumeric, -, _, . allowed)",
        ));
    }
    Ok(())
}

pub struct MuseSentinelClient {
    socket: MuseWebSocket,
    session: MuseNoiseSession,
    next_stream_id: u64,
}

impl MuseSentinelClient {
    pub fn connect(config: &Config) -> io::Result<Self> {
        let config = bootstrap_museai_config(config)?;
        let base_url = if config.museai_base_url.is_empty() {
            "https://muse.ai"
        } else {
            config.museai_base_url.as_str()
        };
        let base_origin = base_url.trim_end_matches('/');

        let req_id = Uuid::new_v4().to_string();
        let ws_url = build_museai_ws_url(&config, &req_id)?;

        let mut socket = MuseWebSocket::connect(&ws_url, base_origin, "")?;
        let mut session = MuseNoiseSession::new_initiator(NOISE_PATTERN_XX, None)
            .map_err(|e| io::Error::other(format!("Noise initiator error: {e}")))?;

        session
            .perform_client_handshake(&mut socket, &config.museai_notary_token)
            .map_err(|e| io::Error::other(format!("Noise handshake error: {e}")))?;

        Ok(Self {
            socket,
            session,
            next_stream_id: 1,
        })
    }

    pub fn execute_rpc(
        &mut self,
        verb: &str,
        path: &str,
        body: &[u8],
    ) -> io::Result<(u32, Vec<u8>)> {
        let stream_id = self.next_stream_id;
        self.next_stream_id = self.next_stream_id.wrapping_add(2);

        let headers =
            crate::providers::museai::protocol::standard_hatch_headers("application/json");

        self.socket.send_encrypted_service_request(
            &mut self.session,
            SERVICE_SENTINEL,
            stream_id,
            verb,
            path,
            &headers,
            body,
        )?;

        let mut response_status: Option<u32> = None;
        let mut accumulated_body = Vec::new();

        for _ in 0..100 {
            let frame = self
                .socket
                .read_encrypted_service_frame(&mut self.session)?;
            if frame.stream_id != stream_id {
                continue;
            }

            match frame.kind {
                ServiceFrameKind::Response {
                    status,
                    body: chunk,
                    end_body,
                    ..
                } => {
                    response_status = Some(status);
                    accumulated_body.extend_from_slice(&chunk);
                    if accumulated_body.len() > MAX_APPROVAL_PAYLOAD_BYTES {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Sentinel response exceeds body limit",
                        ));
                    }
                    if end_body {
                        return Ok((status, accumulated_body));
                    }
                }
                ServiceFrameKind::BodyChunk { data, end_body } => {
                    accumulated_body.extend_from_slice(&data);
                    if accumulated_body.len() > MAX_APPROVAL_PAYLOAD_BYTES {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Sentinel response exceeds body limit",
                        ));
                    }
                    if end_body {
                        let status = response_status.ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "Received body chunk before response header",
                            )
                        })?;
                        return Ok((status, accumulated_body));
                    }
                }
                ServiceFrameKind::Reset { code, reason } => {
                    return Err(io::Error::other(format!(
                        "Sentinel stream reset: code={code}, reason={reason}"
                    )));
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Sentinel RPC did not complete within frame limit",
        ))
    }
}

impl SentinelClient for MuseSentinelClient {
    fn list_approvals(&mut self) -> io::Result<Vec<ApprovalRecord>> {
        let (status, body) = self.execute_rpc("GET", "/approvals", &[])?;
        if status != 200 {
            return Err(io::Error::other(format!(
                "Sentinel GET /approvals failed with HTTP {status}"
            )));
        }
        parse_approvals_list(&body)
    }

    fn get_approval(&mut self, approval_id: &str) -> io::Result<ApprovalRecord> {
        validate_approval_id(approval_id)?;
        let path = format!("/approvals/{approval_id}");
        let (status, body) = self.execute_rpc("GET", &path, &[])?;
        if status != 200 {
            return Err(io::Error::other(format!(
                "Sentinel GET {path} failed with HTTP {status}"
            )));
        }
        parse_approval_get(&body)
    }

    fn decide_approval(
        &mut self,
        approval_id: &str,
        decision_id: &str,
        decision_kind: &str,
    ) -> io::Result<ApprovalRecord> {
        validate_approval_id(approval_id)?;
        let path = format!("/approvals/{approval_id}/decide");
        let payload = serde_json::json!({
            "decision_id": decision_id,
            "decision": decision_id,
            "kind": decision_kind,
        });
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        let (status, _body) = self.execute_rpc("POST", &path, &payload_bytes)?;
        if status != 200 && status != 204 {
            return Err(io::Error::other(format!(
                "Sentinel POST {path} failed with HTTP {status}"
            )));
        }

        // Always fetch and verify explicit approved status
        let updated = self.get_approval(approval_id)?;
        if !updated.status.is_approved() {
            return Err(io::Error::other(format!(
                "Sentinel decision submitted but approval status is '{}' (not approved)",
                updated.status
            )));
        }

        Ok(updated)
    }
}
