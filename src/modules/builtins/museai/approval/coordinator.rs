use super::client::{MuseSentinelClient, SentinelClient};
use super::policy::{ApprovalPolicyEvaluator, PolicyVerdict};
use crate::config::{Config, MuseApprovalConfig, MuseApprovalMode};
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoordinateResult {
    Approved {
        approval_id: String,
        decision_kind: String,
    },
    ManualRequired {
        reason: String,
        approval_id: Option<String>,
    },
}

pub struct ApprovalCoordinator {
    config: Config,
    approval_config: MuseApprovalConfig,
    client: Option<Box<dyn SentinelClient>>,
    session_id: String,
    decisions_count: usize,
}

impl ApprovalCoordinator {
    pub fn new(config: &Config, session_id: &str) -> Self {
        Self {
            config: config.clone(),
            approval_config: config.muse_approval.clone(),
            client: None,
            session_id: session_id.to_string(),
            decisions_count: 0,
        }
    }

    pub fn with_client(config: &Config, session_id: &str, client: Box<dyn SentinelClient>) -> Self {
        Self {
            config: config.clone(),
            approval_config: config.muse_approval.clone(),
            client: Some(client),
            session_id: session_id.to_string(),
            decisions_count: 0,
        }
    }

    fn ensure_client(&mut self) -> io::Result<()> {
        if self.client.is_none() {
            let sentinel = MuseSentinelClient::connect(&self.config)?;
            self.client = Some(Box::new(sentinel));
        }
        Ok(())
    }

    pub fn handle_approval_snapshot(
        &mut self,
        snapshot_val: &serde_json::Value,
    ) -> io::Result<CoordinateResult> {
        if self.approval_config.mode == MuseApprovalMode::Off {
            return Ok(CoordinateResult::ManualRequired {
                reason: "Approval coordinator is disabled (mode=off)".to_string(),
                approval_id: None,
            });
        }

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        if let Err(e) = self.ensure_client() {
            return Ok(CoordinateResult::ManualRequired {
                reason: format!("Failed to connect Sentinel client: {e}"),
                approval_id: None,
            });
        }

        let pending_list = match self.client.as_mut().unwrap().list_approvals() {
            Ok(list) => list,
            Err(e) => {
                return Ok(CoordinateResult::ManualRequired {
                    reason: format!("Failed to list pending approvals from Sentinel: {e}"),
                    approval_id: None,
                });
            }
        };

        let snapshot = snapshot_val.get("payload").unwrap_or(snapshot_val);
        let snapshot_ids: Vec<&str> = snapshot
            .get("pending_approvals")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter(|approval| {
                approval["scope"]["thread_id"].as_str() == Some(&self.session_id)
                    || approval["scope"]["session_scope_id"].as_str() == Some(&self.session_id)
            })
            .filter_map(|approval| approval.get("approval_id").and_then(|id| id.as_str()))
            .collect();
        if snapshot_ids.len() != 1 {
            return Ok(CoordinateResult::ManualRequired {
                reason: "Approval snapshot did not identify exactly one scoped approval".into(),
                approval_id: None,
            });
        }

        let session_records: Vec<_> = pending_list
            .into_iter()
            .filter(|rec| {
                rec.approval_id == snapshot_ids[0]
                    && rec.scope.matches_session(&self.session_id)
                    && rec.scope.matches_vm(&self.config.museai_vm_id)
            })
            .collect();
        let pending_records: Vec<_> = session_records
            .into_iter()
            .filter(|record| record.status.is_pending())
            .collect();

        if pending_records.is_empty() {
            return Ok(CoordinateResult::ManualRequired {
                reason: "No matching pending approval record found on Sentinel for this session"
                    .to_string(),
                approval_id: None,
            });
        }
        if pending_records.len() != 1 {
            return Ok(CoordinateResult::ManualRequired {
                reason:
                    "Multiple pending approval records match this session; manual review required"
                        .to_string(),
                approval_id: None,
            });
        }

        let mut last_approved = None;

        for record in pending_records {
            let evaluator = ApprovalPolicyEvaluator::new(
                &self.approval_config,
                &self.session_id,
                &self.config.museai_vm_id,
                now_ms,
                self.decisions_count,
            );

            let verdict = evaluator.evaluate(&record);

            match verdict {
                PolicyVerdict::Eligible {
                    decision_id,
                    decision_kind,
                } => {
                    let client = self.client.as_mut().unwrap();
                    match client.decide_approval(&record.approval_id, &decision_id, &decision_kind)
                    {
                        Ok(approved_rec)
                            if approved_rec.status.is_approved()
                                && approved_rec.approval_id == record.approval_id =>
                        {
                            self.decisions_count += 1;
                            last_approved = Some((record.approval_id, decision_kind));
                        }
                        Ok(rec) => {
                            return Ok(CoordinateResult::ManualRequired {
                                reason: format!(
                                    "Approval decided but final status is not approved ({})",
                                    rec.status
                                ),
                                approval_id: Some(record.approval_id),
                            });
                        }
                        Err(e) => {
                            return Ok(CoordinateResult::ManualRequired {
                                reason: format!("Failed to submit approval decision: {e}"),
                                approval_id: Some(record.approval_id),
                            });
                        }
                    }
                }
                PolicyVerdict::Ineligible { reason } => {
                    return Ok(CoordinateResult::ManualRequired {
                        reason,
                        approval_id: Some(record.approval_id),
                    });
                }
            }
        }

        if let Some((approval_id, decision_kind)) = last_approved {
            Ok(CoordinateResult::Approved {
                approval_id,
                decision_kind,
            })
        } else {
            Ok(CoordinateResult::ManualRequired {
                reason: "No approval was processed".to_string(),
                approval_id: None,
            })
        }
    }
}
