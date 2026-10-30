use super::debug_log;
use super::parser::parse_assistant_content_from_json;
use crate::providers::museai::MuseApprovalRequired;
use std::io;
use uuid::Uuid;

pub struct MuseChatStream {
    pub pending: Vec<u8>,
    pub session_id: Option<String>,
    pub expected_session_id: Option<String>,
    pub expected_prompt: Option<String>,
    pub current_turn_active: bool,
    pub latest_user_seq: Option<u64>,
    pub text: String,
    pub completed: bool,
    pub approved_approval_ids: Vec<String>,
    #[allow(clippy::type_complexity)]
    pub approval_handler: Option<Box<dyn FnMut(&str, &serde_json::Value) -> io::Result<bool>>>,
}

impl Default for MuseChatStream {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            session_id: None,
            expected_session_id: None,
            expected_prompt: None,
            current_turn_active: true,
            latest_user_seq: None,
            text: String::new(),
            completed: false,
            approved_approval_ids: Vec::new(),
            approval_handler: None,
        }
    }
}

fn matches_user_prompt(payload: &serde_json::Value, prompt: &str) -> bool {
    let clean_prompt = prompt.trim();
    if clean_prompt.is_empty() {
        return true;
    }
    let check_str = |s: &str| {
        let clean = s.strip_prefix("user:\n").unwrap_or(s).trim();
        if clean == clean_prompt || clean.contains(clean_prompt) || clean_prompt.contains(clean) {
            return true;
        }
        let clean_lc = clean.to_lowercase();
        let prompt_lc = clean_prompt.to_lowercase();
        clean_lc == prompt_lc || clean_lc.contains(&prompt_lc) || prompt_lc.contains(&clean_lc)
    };
    if let Some(content) = payload.get("content") {
        if let Some(s) = content.as_str() {
            if check_str(s) {
                return true;
            }
        } else if let Some(arr) = content.as_array() {
            for item in arr {
                if let Some(s) = item
                    .get("text")
                    .and_then(|v| v.as_str())
                    .or_else(|| item.as_str())
                {
                    if check_str(s) {
                        return true;
                    }
                }
            }
        }
    }
    if let Some(display) = payload.get("display_text").and_then(|v| v.as_str()) {
        if check_str(display) {
            return true;
        }
    }
    false
}

impl MuseChatStream {
    pub fn with_expected_session(session_id: Option<String>) -> Self {
        Self::with_expected_turn(session_id, None)
    }

    pub fn with_expected_turn(session_id: Option<String>, expected_prompt: Option<String>) -> Self {
        let is_continuation = session_id.is_some() && expected_prompt.is_some();
        Self {
            expected_session_id: session_id,
            expected_prompt,
            current_turn_active: !is_continuation,
            ..Default::default()
        }
    }

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

    fn check_record_status(
        &self,
        record: &serde_json::Value,
        payload: &serde_json::Value,
    ) -> io::Result<bool> {
        let status = payload.get("status").and_then(|value| value.as_str());
        if let (Some(expected), Some(actual)) = (
            self.session_id.as_deref(),
            payload.get("session_id").and_then(|value| value.as_str()),
        ) {
            if actual != expected && record["type"] != "response" {
                return Ok(false);
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
        Ok(true)
    }

    fn try_init_session(&mut self, record: &serde_json::Value) -> io::Result<bool> {
        if self.session_id.is_some() {
            return Ok(false);
        }
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
            if let Some(ref expected) = self.expected_session_id {
                if expected != id {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Muse chat session mismatch: expected {expected}, got {id}"),
                    ));
                }
            }
            self.session_id = Some(id.to_owned());
            debug_log(&format!("Muse session acknowledged: record={record}"));
            return Ok(true);
        }
        Ok(false)
    }

    fn check_subscription_response(&self, payload: &serde_json::Value) -> io::Result<()> {
        if payload["subscribed"].as_bool() != Some(true)
            || payload["session_id"].as_str() != self.session_id.as_deref()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Muse subscription was not acknowledged for the requested session",
            ));
        }
        Ok(())
    }

    fn handle_user_message(&mut self, record: &serde_json::Value, payload: &serde_json::Value) {
        let user_seq = payload
            .get("chat_event_seq")
            .or_else(|| payload.get("seq"))
            .or_else(|| record.get("seq"))
            .and_then(|v| v.as_u64());

        if let Some(seq) = user_seq {
            if seq >= self.latest_user_seq.unwrap_or(0) {
                self.latest_user_seq = Some(seq);
            }
        }

        if !self.current_turn_active {
            if let Some(ref prompt) = self.expected_prompt {
                if matches_user_prompt(payload, prompt) {
                    debug_log(&format!(
                        "Matched expected prompt for current turn: seq={user_seq:?}"
                    ));
                    self.current_turn_active = true;
                    self.text.clear();
                }
            } else {
                self.current_turn_active = true;
                self.text.clear();
            }
        }
    }

    fn is_event_applicable(
        &self,
        record: &serde_json::Value,
        payload: &serde_json::Value,
        event: &str,
    ) -> bool {
        if !self.current_turn_active {
            debug_log(&format!(
                "Skipping historical event before active turn: category={event}"
            ));
            return false;
        }

        let msg_seq = payload
            .get("message_seq")
            .or_else(|| payload.get("seq"))
            .or_else(|| record.get("seq"))
            .and_then(|v| v.as_u64());

        if let (Some(user_seq), Some(mseq)) = (self.latest_user_seq, msg_seq) {
            if mseq <= user_seq {
                debug_log(&format!(
                    "Skipping historical message event with seq {mseq} <= latest user seq {user_seq}"
                ));
                return false;
            }
        }
        true
    }

    fn handle_approvals_snapshot(
        &mut self,
        record: &serde_json::Value,
        payload: &serde_json::Value,
    ) -> io::Result<()> {
        let pending = payload
            .get("pending_approvals")
            .and_then(|v| v.as_array())
            .map(Vec::as_slice)
            .unwrap_or_default();

        let sid = self.session_id.as_deref();
        let is_scoped = |app: &serde_json::Value| {
            ["thread_id", "session_scope_id"]
                .iter()
                .any(|k| app["scope"][k].as_str() == sid && sid.is_some())
        };

        let mut new_scoped_ids = Vec::new();
        let mut has_unapproved = false;
        for app in pending.iter().filter(|a| is_scoped(a)) {
            match app.get("approval_id").and_then(|id| id.as_str()) {
                Some(id) => {
                    if !self.approved_approval_ids.iter().any(|a| a == id) {
                        has_unapproved = true;
                        new_scoped_ids.push(id.to_string());
                    }
                }
                None => {
                    debug_log("Approval snapshot contains pending scoped approval without approval_id; failing closed");
                    return Err(io::Error::other(MuseApprovalRequired(
                        "Scoped Muse permission approval requires manual intervention (missing approval_id)".to_string(),
                    )));
                }
            }
        }

        if !has_unapproved {
            debug_log("Approval snapshot contains no unapproved requests for this session; continuing stream");
            return Ok(());
        }

        debug_log("Approval snapshot requires_approval=true");
        if let Some(ref mut handler) = self.approval_handler {
            match handler(sid.unwrap_or(""), record) {
                Ok(true) => {
                    for id in new_scoped_ids {
                        if !self.approved_approval_ids.contains(&id) {
                            self.approved_approval_ids.push(id);
                        }
                    }
                    debug_log("Scoped Muse permission approval auto-approved and confirmed; continuing stream");
                    return Ok(());
                }
                Ok(false) => {
                    debug_log("Scoped Muse permission approval requires manual intervention");
                    return Err(io::Error::other(MuseApprovalRequired(
                        "Scoped Muse permission approval requires manual intervention".to_string(),
                    )));
                }
                Err(e) => {
                    debug_log(&format!("Scoped Muse approval handling error: {e}"));
                    return Err(io::Error::other(MuseApprovalRequired(e.to_string())));
                }
            }
        }

        debug_log("Scoped Muse permission approval is pending");
        Err(io::Error::other(MuseApprovalRequired(
            "Scoped Muse permission approval is pending".to_string(),
        )))
    }

    fn handle_content_event(&mut self, record: &serde_json::Value, event: &str) {
        if matches!(event, "delta.message_done") {
            if self.text.is_empty() {
                if let Some(text) = parse_assistant_content_from_json(record) {
                    self.text.push_str(&text);
                }
            }
        } else if matches!(
            event,
            "delta.text_append"
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
    }

    pub fn record(&mut self, record: &serde_json::Value) -> io::Result<()> {
        if std::env::var("MUSE_LIVE_RESUME_OWNED").ok().as_deref() == Some("1") {
            crate::test_helpers::log_owned_record_schema(record);
        }
        let payload = record.get("payload").unwrap_or(record);
        if !self.check_record_status(record, payload)? {
            return Ok(());
        }

        if self.try_init_session(record)? {
            return Ok(());
        }

        if record["type"] == "response" {
            return self.check_subscription_response(payload);
        }

        let event = record
            .get("event")
            .and_then(|value| value.as_str())
            .unwrap_or("");

        if event == "message.user" {
            self.handle_user_message(record, payload);
            return Ok(());
        }

        if !self.is_event_applicable(record, payload, event) {
            return Ok(());
        }

        if event == "approvals.snapshot" {
            return self.handle_approvals_snapshot(record, payload);
        }

        self.handle_content_event(record, event);
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
