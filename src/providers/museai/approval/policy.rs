use super::types::{ApprovalRecord, ApprovalStatus, DecisionOption};
use crate::config::{MuseApprovalConfig, MuseApprovalMode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyVerdict {
    Eligible {
        decision_id: String,
        decision_kind: String,
    },
    Ineligible {
        reason: String,
    },
}

impl PolicyVerdict {
    pub fn is_eligible(&self) -> bool {
        matches!(self, PolicyVerdict::Eligible { .. })
    }
}

pub struct ApprovalPolicyEvaluator<'a> {
    config: &'a MuseApprovalConfig,
    expected_session_id: &'a str,
    expected_vm_id: &'a str,
    now_ms: u64,
    decisions_count: usize,
}

impl<'a> ApprovalPolicyEvaluator<'a> {
    pub fn new(
        config: &'a MuseApprovalConfig,
        expected_session_id: &'a str,
        expected_vm_id: &'a str,
        now_ms: u64,
        decisions_count: usize,
    ) -> Self {
        Self {
            config,
            expected_session_id,
            expected_vm_id,
            now_ms,
            decisions_count,
        }
    }

    pub fn evaluate(&self, record: &ApprovalRecord) -> PolicyVerdict {
        if self.config.mode != MuseApprovalMode::ScopedAuto {
            return PolicyVerdict::Ineligible {
                reason: format!("Approval mode is {:?} (not scoped_auto)", self.config.mode),
            };
        }

        if self.decisions_count >= self.config.max_decisions_per_job {
            return PolicyVerdict::Ineligible {
                reason: format!(
                    "Maximum decisions per job ({}) reached",
                    self.config.max_decisions_per_job
                ),
            };
        }

        if record.status != ApprovalStatus::Pending {
            return PolicyVerdict::Ineligible {
                reason: format!("Record status is not pending ({})", record.status),
            };
        }

        if record.is_expired(self.now_ms) {
            return PolicyVerdict::Ineligible {
                reason: "Approval request has expired".to_string(),
            };
        }

        // Scope correlation
        if !record.scope.matches_session(self.expected_session_id) {
            return PolicyVerdict::Ineligible {
                reason: format!(
                    "Scope session mismatch: expected '{}', got thread_id={:?}, session_scope_id={:?}",
                    self.expected_session_id, record.scope.thread_id, record.scope.session_scope_id
                ),
            };
        }

        if !record.scope.matches_vm(self.expected_vm_id) {
            return PolicyVerdict::Ineligible {
                reason: format!(
                    "Scope VM mismatch: expected '{}', got {:?}",
                    self.expected_vm_id, record.scope.vm_id
                ),
            };
        }

        // Approval type check
        let rec_type = if !record.payload_type.is_empty() {
            record.payload_type.as_str()
        } else if !record.payload.approval_type.is_empty() {
            record.payload.approval_type.as_str()
        } else {
            "unknown"
        };

        if rec_type.eq_ignore_ascii_case("connector") {
            return PolicyVerdict::Ineligible {
                reason: "Connector approvals require manual authorization".to_string(),
            };
        }

        let type_allowed = self
            .config
            .allowed_types
            .iter()
            .any(|t| t.eq_ignore_ascii_case(rec_type));
        if !type_allowed {
            return PolicyVerdict::Ineligible {
                reason: format!("Approval type '{rec_type}' is not in allowed_types"),
            };
        }

        let target_dest = record.target_destination().unwrap_or("");
        let dest_allowed = self.config.allowed_destinations.iter().any(|d| {
            !d.is_empty()
                && (d.eq_ignore_ascii_case(target_dest) || target_dest.ends_with(&format!(".{d}")))
        });
        if !dest_allowed {
            return PolicyVerdict::Ineligible {
                reason: format!(
                    "Destination '{target_dest}' is not in allowed_destinations allowlist"
                ),
            };
        }

        // Decision option resolution from server-offered options
        let chosen_option = self.select_decision_option(&record.decision_options);
        match chosen_option {
            Some(opt) => PolicyVerdict::Eligible {
                decision_id: opt.id.clone(),
                decision_kind: opt.kind.clone(),
            },
            None => PolicyVerdict::Ineligible {
                reason: "No acceptable server-offered decision options found".to_string(),
            },
        }
    }

    fn select_decision_option<'b>(
        &self,
        options: &'b [DecisionOption],
    ) -> Option<&'b DecisionOption> {
        if options.is_empty() {
            return None;
        }

        // 1. Try allow_once if prefer_allow_once
        if self.config.prefer_allow_once {
            if let Some(opt) = options
                .iter()
                .find(|o| o.kind.eq_ignore_ascii_case("allow_once"))
            {
                return Some(opt);
            }
        }

        // 2. Try allow_always only if allow_always is explicitly enabled
        if self.config.allow_always {
            if let Some(opt) = options
                .iter()
                .find(|o| o.kind.eq_ignore_ascii_case("allow_always"))
            {
                return Some(opt);
            }
        }

        // 3. Fallback: if not prefer_allow_once, try allow_once
        if let Some(opt) = options
            .iter()
            .find(|o| o.kind.eq_ignore_ascii_case("allow_once"))
        {
            return Some(opt);
        }

        None
    }
}
