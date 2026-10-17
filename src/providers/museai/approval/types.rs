use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;

pub const MAX_APPROVAL_PAYLOAD_BYTES: usize = 512 * 1024; // 512 KB limit

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Expired,
    #[serde(untagged)]
    Other(String),
}

impl ApprovalStatus {
    pub fn is_approved(&self) -> bool {
        matches!(self, ApprovalStatus::Approved)
    }

    pub fn is_pending(&self) -> bool {
        matches!(self, ApprovalStatus::Pending)
    }
}

impl fmt::Display for ApprovalStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Approved => write!(f, "approved"),
            Self::Denied => write!(f, "denied"),
            Self::Expired => write!(f, "expired"),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DecisionOption {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApprovalScope {
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub session_scope_id: Option<String>,
    #[serde(default)]
    pub vm_id: Option<String>,
    #[serde(default)]
    pub owner_id: Option<String>,
}

impl ApprovalScope {
    pub fn matches_session(&self, expected_session_id: &str) -> bool {
        if expected_session_id.is_empty() {
            return false;
        }
        self.thread_id.as_deref() == Some(expected_session_id)
            || self.session_scope_id.as_deref() == Some(expected_session_id)
    }

    pub fn matches_vm(&self, expected_vm_id: &str) -> bool {
        if let Some(vm_id) = &self.vm_id {
            !expected_vm_id.is_empty() && vm_id == expected_vm_id
        } else {
            true
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApprovalPayloadDetails {
    #[serde(rename = "type", default)]
    pub approval_type: String,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub destination: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub approval_id: String,
    pub status: ApprovalStatus,
    #[serde(default)]
    pub payload_type: String,
    #[serde(default)]
    pub payload: ApprovalPayloadDetails,
    #[serde(default)]
    pub scope: ApprovalScope,
    #[serde(default)]
    pub resource: Option<String>,
    #[serde(default)]
    pub decision_options: Vec<DecisionOption>,
    #[serde(default)]
    pub request_expires_at_ms: Option<u64>,
    #[serde(default)]
    pub created_at_ms: Option<u64>,
}

impl ApprovalRecord {
    pub fn target_destination(&self) -> Option<&str> {
        self.payload
            .host
            .as_deref()
            .or(self.payload.destination.as_deref())
            .or(self.resource.as_deref())
    }

    pub fn is_expired(&self, now_ms: u64) -> bool {
        if let Some(exp) = self.request_expires_at_ms {
            exp <= now_ms
        } else {
            false
        }
    }
}

pub fn parse_approval_record_json(val: &serde_json::Value) -> io::Result<ApprovalRecord> {
    let mut obj = val;
    if let Some(res) = obj.get("result") {
        obj = res;
    }
    if let Some(app) = obj.get("approval") {
        obj = app;
    }

    let approval_id = obj
        .get("approval_id")
        .or_else(|| obj.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing approval_id"))?
        .to_string();

    let status_str = obj
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("pending");
    let status = match status_str.to_ascii_lowercase().as_str() {
        "pending" => ApprovalStatus::Pending,
        "approved" => ApprovalStatus::Approved,
        "denied" | "rejected" => ApprovalStatus::Denied,
        "expired" => ApprovalStatus::Expired,
        other => ApprovalStatus::Other(other.to_string()),
    };

    let payload_val = obj.get("payload");
    let payload_type = payload_val
        .and_then(|p| p.get("type"))
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("type").and_then(|v| v.as_str()))
        .or_else(|| obj.get("scheme").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    let mut payload_details = ApprovalPayloadDetails {
        approval_type: payload_type.clone(),
        ..Default::default()
    };
    if let Some(host) = obj.get("host").and_then(|v| v.as_str()) {
        payload_details.host = Some(host.to_string());
    }
    if let Some(url) = obj.get("url").and_then(|v| v.as_str()) {
        payload_details.url = Some(url.to_string());
    }
    if let Some(method) = obj.get("method").and_then(|v| v.as_str()) {
        payload_details.method = Some(method.to_string());
    }
    if let Some(dest) = obj.get("destination").and_then(|v| v.as_str()) {
        payload_details.destination = Some(dest.to_string());
    }
    if let Some(p) = payload_val {
        if let Some(conn) = p.get("connector_payload") {
            if let Some(host) = conn.get("host").and_then(|v| v.as_str()) {
                payload_details.host = Some(host.to_string());
            }
            if let Some(url) = conn.get("url").and_then(|v| v.as_str()) {
                payload_details.url = Some(url.to_string());
            }
            if let Some(method) = conn.get("method").and_then(|v| v.as_str()) {
                payload_details.method = Some(method.to_string());
            }
        }
        if let Some(host) = p.get("host").and_then(|v| v.as_str()) {
            payload_details.host = Some(host.to_string());
        }
        if let Some(url) = p.get("url").and_then(|v| v.as_str()) {
            payload_details.url = Some(url.to_string());
        }
        if let Some(method) = p.get("method").and_then(|v| v.as_str()) {
            payload_details.method = Some(method.to_string());
        }
        if let Some(dest) = p.get("destination").and_then(|v| v.as_str()) {
            payload_details.destination = Some(dest.to_string());
        }
    }

    let mut scope = ApprovalScope::default();
    if let Some(thread_id) = obj.get("thread_id").and_then(|v| v.as_str()) {
        scope.thread_id = Some(thread_id.to_string());
    }
    if let Some(session_scope_id) = obj.get("session_scope_id").and_then(|v| v.as_str()) {
        scope.session_scope_id = Some(session_scope_id.to_string());
    }
    if let Some(vm_id) = obj.get("vm_id").and_then(|v| v.as_str()) {
        scope.vm_id = Some(vm_id.to_string());
    }
    if let Some(owner_id) = obj.get("owner_id").and_then(|v| v.as_str()) {
        scope.owner_id = Some(owner_id.to_string());
    }
    if let Some(s) = obj.get("scope") {
        if let Some(thread_id) = s.get("thread_id").and_then(|v| v.as_str()) {
            scope.thread_id = Some(thread_id.to_string());
        }
        if let Some(session_scope_id) = s.get("session_scope_id").and_then(|v| v.as_str()) {
            scope.session_scope_id = Some(session_scope_id.to_string());
        }
        if let Some(vm_id) = s.get("vm_id").and_then(|v| v.as_str()) {
            scope.vm_id = Some(vm_id.to_string());
        }
        if let Some(owner_id) = s.get("owner_id").and_then(|v| v.as_str()) {
            scope.owner_id = Some(owner_id.to_string());
        }
    }

    let resource = obj
        .get("resource")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let mut decision_options = Vec::new();
    if let Some(opts) = obj.get("decision_options").and_then(|v| v.as_array()) {
        for opt in opts {
            if let Some(s) = opt.as_str() {
                decision_options.push(DecisionOption {
                    id: s.to_string(),
                    kind: s.to_string(),
                    label: Some(s.to_string()),
                    scope: None,
                });
            } else if let Some(map) = opt.as_object() {
                let kind = map
                    .get("kind")
                    .or_else(|| map.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut id = map
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if id.is_empty() {
                    id = kind.clone();
                }
                let label = map
                    .get("label")
                    .and_then(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .or_else(|| v.get("kind").and_then(|k| k.as_str()).map(str::to_string))
                    })
                    .or_else(|| {
                        map.get("label_text")
                            .and_then(|v| v.as_str())
                            .map(str::to_string)
                    });
                let scope = map
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                decision_options.push(DecisionOption {
                    id,
                    kind,
                    label,
                    scope,
                });
            }
        }
    }

    let request_expires_at_ms = obj
        .get("request_expires_at_ms")
        .or_else(|| obj.get("expires_at_ms"))
        .and_then(|v| v.as_u64());

    let created_at_ms = obj
        .get("created_at_ms")
        .or_else(|| obj.get("created_at"))
        .and_then(|v| v.as_u64());

    Ok(ApprovalRecord {
        approval_id,
        status,
        payload_type,
        payload: payload_details,
        scope,
        resource,
        decision_options,
        request_expires_at_ms,
        created_at_ms,
    })
}

pub fn parse_approvals_list(body: &[u8]) -> io::Result<Vec<ApprovalRecord>> {
    if body.len() > MAX_APPROVAL_PAYLOAD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Approval response body exceeds size limit",
        ));
    }

    let val: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Invalid JSON: {e}")))?;

    if let Some(false) = val.get("ok").and_then(|v| v.as_bool()) {
        let msg = val
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("Sentinel returned ok=false");
        return Err(io::Error::other(format!("Sentinel error: {msg}")));
    }

    let items_array = if let Some(arr) = val.as_array() {
        arr
    } else if let Some(arr) = val.get("result").and_then(|v| v.as_array()) {
        arr
    } else if let Some(arr) = val
        .get("result")
        .and_then(|v| v.get("pending_approvals"))
        .and_then(|v| v.as_array())
    {
        arr
    } else if let Some(arr) = val
        .get("result")
        .and_then(|v| v.get("pending"))
        .and_then(|v| v.as_array())
    {
        arr
    } else if let Some(arr) = val.get("approvals").and_then(|v| v.as_array()) {
        arr
    } else if let Some(arr) = val.get("pending").and_then(|v| v.as_array()) {
        arr
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Approval list response does not contain an array",
        ));
    };

    let mut records = Vec::with_capacity(items_array.len());
    for item in items_array {
        records.push(parse_approval_record_json(item)?);
    }

    Ok(records)
}

pub fn parse_approval_get(body: &[u8]) -> io::Result<ApprovalRecord> {
    if body.len() > MAX_APPROVAL_PAYLOAD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Approval response body exceeds size limit",
        ));
    }

    let val: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Invalid JSON: {e}")))?;

    if let Some(false) = val.get("ok").and_then(|v| v.as_bool()) {
        let msg = val
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("Sentinel returned ok=false");
        return Err(io::Error::other(format!("Sentinel error: {msg}")));
    }

    parse_approval_record_json(&val)
}
