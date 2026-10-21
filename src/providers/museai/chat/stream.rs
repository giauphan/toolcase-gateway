use super::debug_log;
use super::parser::parse_assistant_content_from_json;
use crate::providers::museai::MuseApprovalRequired;
use std::io;
use uuid::Uuid;

#[derive(Default)]
pub struct MuseChatStream {
    pub pending: Vec<u8>,
    pub session_id: Option<String>,
    pub text: String,
    pub completed: bool,
    pub approved_approval_ids: Vec<String>,
    #[allow(clippy::type_complexity)]
    pub approval_handler: Option<Box<dyn FnMut(&str, &serde_json::Value) -> io::Result<bool>>>,
}

impl MuseChatStream {
    pub fn push(&mut self, bytes: &[u8], ended: bool) -> io::Result<()> {
        const LIMIT: usize = 1024 * 1024;
        if self.pending.len().saturating_add(bytes.len()) > LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Muse chat record too large",
            ));
        }
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self
            .pending
            .iter()
            .position(|byte| *byte == b'\n')
            .or_else(|| (ended && !self.pending.is_empty()).then_some(self.pending.len()))
        {
            let line: Vec<u8> = self.pending.drain(..end).collect();
            if self.pending.first() == Some(&b'\n') {
                self.pending.remove(0);
            }
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let record: serde_json::Value = serde_json::from_slice(&line).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "Invalid Muse chat JSON record")
            })?;
            self.record(&record)?;
            if self.text.len() > LIMIT {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Muse chat output too large",
                ));
            }
        }
        Ok(())
    }

    pub fn record(&mut self, record: &serde_json::Value) -> io::Result<()> {
        if std::env::var("MUSE_LIVE_RESUME_OWNED").ok().as_deref() == Some("1") {
            crate::test_helpers::log_owned_record_schema(record);
        }
        let payload = record.get("payload").unwrap_or(record);
        let status = payload.get("status").and_then(|value| value.as_str());
        if let (Some(expected), Some(actual)) = (
            self.session_id.as_deref(),
            payload.get("session_id").and_then(|value| value.as_str()),
        ) {
            if actual != expected && record["type"] != "response" {
                return Ok(());
            }
        }

        if record.get("error").is_some_and(|error| !error.is_null())
            || record.get("ok").and_then(|value| value.as_bool()) == Some(false)
            || matches!(status, Some("failed" | "error" | "cancelled"))
        {
            return Err(io::Error::other(
                "Muse chat rejected or failed (private details suppressed)",
            ));
        }
        if self.session_id.is_none() {
            let result = record.get("result").unwrap_or(record);
            let id = result
                .get("session_id")
                .and_then(|value| value.as_str())
                .filter(|id| !id.is_empty());
            if let Some(id) = id {
                if Uuid::parse_str(id).is_err() || result["is_thread"].as_bool() != Some(true) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Muse did not acknowledge an isolated thread",
                    ));
                }
                self.session_id = Some(id.to_owned());
                debug_log("Fresh Muse session acknowledged");
                return Ok(());
            }
        }
        if record["type"] == "response" {
            if payload["subscribed"].as_bool() != Some(true)
                || payload["session_id"].as_str() != self.session_id.as_deref()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Muse subscription was not acknowledged for the requested session",
                ));
            }
            return Ok(());
        }
        let event = record
            .get("event")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let category = match event {
            "delta.text_append"
            | "delta.message_done"
            | "task.status"
            | "task.complete"
            | "agent.status"
            | "message.assistant"
            | "message.user"
            | "delta.presentation"
            | "delta.message_start"
            | "delta.tool_start"
            | "delta.tool_done"
            | "delta.card"
            | "delta.attachment"
            | "delta.media"
            | "message.attachment"
            | "approvals.snapshot"
            | "delta.marker"
            | "delta.placeholder" => event,
            _ => "other",
        };
        debug_log(&format!("Chat event category={category}"));
        if event == "approvals.snapshot" {
            let pending_approvals = payload
                .get("pending_approvals")
                .and_then(|value| value.as_array());

            let has_unapproved = pending_approvals.is_some_and(|approvals| {
                approvals.iter().any(|approval| {
                    let matches_scope = ["thread_id", "session_scope_id"].iter().any(|key| {
                        approval["scope"][key]
                            .as_str()
                            .is_some_and(|id| self.session_id.as_deref() == Some(id))
                    });
                    if !matches_scope {
                        return false;
                    }
                    match approval.get("approval_id").and_then(|id| id.as_str()) {
                        Some(id) => !self
                            .approved_approval_ids
                            .iter()
                            .any(|approved| approved == id),
                        None => true,
                    }
                })
            });

            if !has_unapproved {
                debug_log(
                    "Approval snapshot contains no unapproved requests for this session; continuing stream",
                );
                return Ok(());
            }

            let has_missing_approval_id = pending_approvals.is_some_and(|approvals| {
                approvals.iter().any(|approval| {
                    let matches_scope = ["thread_id", "session_scope_id"].iter().any(|key| {
                        approval["scope"][key]
                            .as_str()
                            .is_some_and(|id| self.session_id.as_deref() == Some(id))
                    });
                    matches_scope
                        && approval
                            .get("approval_id")
                            .and_then(|id| id.as_str())
                            .is_none()
                })
            });

            if has_missing_approval_id {
                debug_log(
                    "Approval snapshot contains pending scoped approval without approval_id; failing closed"
                );
                return Err(io::Error::other(MuseApprovalRequired(
                    "Scoped Muse permission approval requires manual intervention (missing approval_id)"
                        .to_string(),
                )));
            }

            let new_scoped_approval_ids: Vec<String> = pending_approvals
                .map(|approvals| {
                    approvals
                        .iter()
                        .filter_map(|approval| {
                            let matches_scope =
                                ["thread_id", "session_scope_id"].iter().any(|key| {
                                    approval["scope"][key]
                                        .as_str()
                                        .is_some_and(|id| self.session_id.as_deref() == Some(id))
                                });
                            if !matches_scope {
                                return None;
                            }
                            let id = approval.get("approval_id").and_then(|id| id.as_str())?;
                            if self.approved_approval_ids.iter().any(|app| app == id) {
                                None
                            } else {
                                Some(id.to_string())
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();

            debug_log("Approval snapshot requires_approval=true");
            if let Some(ref mut handler) = self.approval_handler {
                let sid = self.session_id.as_deref().unwrap_or("");
                match handler(sid, record) {
                    Ok(true) => {
                        for approval_id in new_scoped_approval_ids {
                            if !self.approved_approval_ids.contains(&approval_id) {
                                self.approved_approval_ids.push(approval_id);
                            }
                        }
                        debug_log(
                            "Scoped Muse permission approval auto-approved and confirmed; continuing stream",
                        );
                        return Ok(());
                    }
                    Ok(false) => {
                        debug_log("Scoped Muse permission approval requires manual intervention");
                        return Err(io::Error::other(MuseApprovalRequired(
                            "Scoped Muse permission approval requires manual intervention"
                                .to_string(),
                        )));
                    }
                    Err(e) => {
                        debug_log(&format!("Scoped Muse approval handling error: {e}"));
                        return Err(io::Error::other(MuseApprovalRequired(e.to_string())));
                    }
                }
            } else {
                debug_log("Scoped Muse permission approval is pending");
                return Err(io::Error::other(MuseApprovalRequired(
                    "Scoped Muse permission approval is pending".to_string(),
                )));
            }
        }
        if matches!(
            event,
            "delta.text_append"
                | "delta.message_done"
                | "message.assistant"
                | "delta.presentation"
                | "delta.tool_done"
                | "delta.card"
                | "delta.attachment"
                | "delta.media"
                | "message.attachment"
                | "delta.marker"
                | "delta.placeholder"
                | "task.complete"
        ) || (event.is_empty()
            && (record.get("content").is_some()
                || record.get("text").is_some()
                || record.get("payload").is_some()))
        {
            if let Some(text) = parse_assistant_content_from_json(record) {
                self.text.push_str(&text);
            }
        }
        if matches!(event, "delta.message_done" | "task.complete") {
            self.completed = true;
        }
        Ok(())
    }

    pub fn finish(self) -> io::Result<(String, String)> {
        if !self.completed || !self.pending.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Muse chat ended without a complete assistant response",
            ));
        }
        Ok((
            self.session_id.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Muse chat did not acknowledge the session",
                )
            })?,
            self.text,
        ))
    }
}
