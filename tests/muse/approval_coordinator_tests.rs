use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use toolcase_gateway::config::{Config, MuseApprovalConfig, MuseApprovalMode};
use toolcase_gateway::museai::approval::client::SentinelClient;
use toolcase_gateway::museai::approval::coordinator::{ApprovalCoordinator, CoordinateResult};
use toolcase_gateway::museai::approval::types::{
    ApprovalPayloadDetails, ApprovalRecord, ApprovalScope, ApprovalStatus, DecisionOption,
};
use toolcase_gateway::museai::chat::MuseChatStream;
use toolcase_gateway::museai::MuseApprovalRequired;

struct MockSentinel {
    records: Vec<ApprovalRecord>,
    decide_calls: Arc<AtomicUsize>,
    should_approve_status: bool,
}

impl SentinelClient for MockSentinel {
    fn list_approvals(&mut self) -> io::Result<Vec<ApprovalRecord>> {
        Ok(self.records.clone())
    }

    fn get_approval(&mut self, approval_id: &str) -> io::Result<ApprovalRecord> {
        let mut rec = self
            .records
            .iter()
            .find(|r| r.approval_id == approval_id)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not found"))?;

        if self.should_approve_status {
            rec.status = ApprovalStatus::Approved;
        }
        Ok(rec)
    }

    fn decide_approval(
        &mut self,
        approval_id: &str,
        _decision_id: &str,
        _decision_kind: &str,
    ) -> io::Result<ApprovalRecord> {
        self.decide_calls.fetch_add(1, Ordering::SeqCst);
        let mut rec = self.get_approval(approval_id)?;
        if self.should_approve_status {
            rec.status = ApprovalStatus::Approved;
        }
        Ok(rec)
    }
}

fn test_record(session_id: &str) -> ApprovalRecord {
    ApprovalRecord {
        approval_id: "app-42".into(),
        status: ApprovalStatus::Pending,
        payload_type: "network".into(),
        payload: ApprovalPayloadDetails {
            approval_type: "network".into(),
            host: Some("example.com".into()),
            ..Default::default()
        },
        scope: ApprovalScope {
            thread_id: Some(session_id.into()),
            session_scope_id: None,
            vm_id: Some("vm-1".into()),
            owner_id: None,
        },
        resource: None,
        decision_options: vec![DecisionOption {
            id: "opt-once".into(),
            kind: "allow_once".into(),
            label: Some("Allow Once".into()),
            scope: None,
        }],
        request_expires_at_ms: Some(9999999999999),
        created_at_ms: Some(1000),
    }
}

fn approval_snapshot(session_id: &str, approval_id: &str) -> serde_json::Value {
    serde_json::json!({
        "event": "approvals.snapshot",
        "payload": {
            "session_id": session_id,
            "pending_approvals": [{
                "approval_id": approval_id,
                "scope": {"thread_id": session_id, "vm_id": "vm-1"}
            }]
        }
    })
}

#[test]
fn test_coordinator_rejects_unrelated_snapshot_id_without_deciding() {
    let config = Config {
        museai_vm_id: "vm-1".into(),
        muse_approval: MuseApprovalConfig {
            mode: MuseApprovalMode::ScopedAuto,
            allowed_destinations: vec!["example.com".into()],
            ..Default::default()
        },
        ..Default::default()
    };
    let counter = Arc::new(AtomicUsize::new(0));
    let mock = MockSentinel {
        records: vec![test_record("session-1")],
        decide_calls: counter.clone(),
        should_approve_status: true,
    };
    let mut coordinator = ApprovalCoordinator::with_client(&config, "session-1", Box::new(mock));
    let result = coordinator
        .handle_approval_snapshot(&approval_snapshot("session-1", "other-id"))
        .unwrap();
    assert!(matches!(result, CoordinateResult::ManualRequired { .. }));
    assert_eq!(counter.load(Ordering::SeqCst), 0);
}

struct MismatchedConfirmation;

impl SentinelClient for MismatchedConfirmation {
    fn list_approvals(&mut self) -> io::Result<Vec<ApprovalRecord>> {
        Ok(vec![test_record("session-1")])
    }
    fn get_approval(&mut self, _: &str) -> io::Result<ApprovalRecord> {
        Ok(ApprovalRecord {
            approval_id: "wrong-id".into(),
            status: ApprovalStatus::Approved,
            ..test_record("session-1")
        })
    }
    fn decide_approval(&mut self, id: &str, _: &str, _: &str) -> io::Result<ApprovalRecord> {
        self.get_approval(id)
    }
}

#[test]
fn test_coordinator_rejects_approved_confirmation_for_wrong_id() {
    let config = Config {
        museai_vm_id: "vm-1".into(),
        muse_approval: MuseApprovalConfig {
            mode: MuseApprovalMode::ScopedAuto,
            allowed_destinations: vec!["example.com".into()],
            ..Default::default()
        },
        ..Default::default()
    };
    let mut coordinator =
        ApprovalCoordinator::with_client(&config, "session-1", Box::new(MismatchedConfirmation));
    let result = coordinator
        .handle_approval_snapshot(&approval_snapshot("session-1", "app-42"))
        .unwrap();
    assert!(matches!(result, CoordinateResult::ManualRequired { .. }));
}

#[test]
fn test_coordinator_auto_approves_eligible_request() {
    let config = Config {
        museai_vm_id: "vm-1".into(),
        muse_approval: MuseApprovalConfig {
            mode: MuseApprovalMode::ScopedAuto,
            allowed_types: vec!["network".into()],
            allowed_destinations: vec!["example.com".into()],
            prefer_allow_once: true,
            allow_always: false,
            max_decisions_per_job: 3,
            approval_timeout_secs: 30,
        },
        ..Default::default()
    };

    let session_id = "test-session-123";
    let decide_counter = Arc::new(AtomicUsize::new(0));
    let mock = MockSentinel {
        records: vec![test_record(session_id)],
        decide_calls: decide_counter.clone(),
        should_approve_status: true,
    };

    let mut coordinator = ApprovalCoordinator::with_client(&config, session_id, Box::new(mock));
    let dummy_snapshot = approval_snapshot(session_id, "app-42");
    let res = coordinator
        .handle_approval_snapshot(&dummy_snapshot)
        .unwrap();

    assert_eq!(
        res,
        CoordinateResult::Approved {
            approval_id: "app-42".into(),
            decision_kind: "allow_once".into()
        }
    );
    assert_eq!(decide_counter.load(Ordering::SeqCst), 1);
}

#[test]
fn test_coordinator_fails_when_server_does_not_confirm_approved() {
    let config = Config {
        museai_vm_id: "vm-1".into(),
        muse_approval: MuseApprovalConfig {
            mode: MuseApprovalMode::ScopedAuto,
            allowed_types: vec!["network".into()],
            allowed_destinations: vec!["example.com".into()],
            prefer_allow_once: true,
            allow_always: false,
            max_decisions_per_job: 3,
            approval_timeout_secs: 30,
        },
        ..Default::default()
    };

    let session_id = "test-session-123";
    let decide_counter = Arc::new(AtomicUsize::new(0));
    let mock = MockSentinel {
        records: vec![test_record(session_id)],
        decide_calls: decide_counter.clone(),
        should_approve_status: false, // does NOT confirm approved!
    };

    let mut coordinator = ApprovalCoordinator::with_client(&config, session_id, Box::new(mock));
    let dummy_snapshot = approval_snapshot(session_id, "app-42");
    let res = coordinator
        .handle_approval_snapshot(&dummy_snapshot)
        .unwrap();

    assert!(matches!(res, CoordinateResult::ManualRequired { .. }));
}

#[test]
fn test_coordinator_fails_when_sentinel_has_no_matching_record() {
    let config = Config {
        museai_vm_id: "vm-1".into(),
        muse_approval: MuseApprovalConfig {
            mode: MuseApprovalMode::ScopedAuto,
            allowed_types: vec!["network".into()],
            allowed_destinations: vec!["example.com".into()],
            prefer_allow_once: true,
            allow_always: false,
            max_decisions_per_job: 3,
            approval_timeout_secs: 30,
        },
        ..Default::default()
    };

    let session_id = "test-session-123";
    let decide_counter = Arc::new(AtomicUsize::new(0));
    let mock = MockSentinel {
        // Record belongs to different session
        records: vec![test_record("different-session")],
        decide_calls: decide_counter.clone(),
        should_approve_status: true,
    };

    let mut coordinator = ApprovalCoordinator::with_client(&config, session_id, Box::new(mock));
    let dummy_snapshot = approval_snapshot(session_id, "app-42");
    let res = coordinator
        .handle_approval_snapshot(&dummy_snapshot)
        .unwrap();

    assert!(matches!(res, CoordinateResult::ManualRequired { .. }));
    assert_eq!(decide_counter.load(Ordering::SeqCst), 0);
}

#[test]
fn test_coordinator_fails_when_multiple_pending_records_match_session() {
    let config = Config {
        museai_vm_id: "vm-1".into(),
        muse_approval: MuseApprovalConfig {
            mode: MuseApprovalMode::ScopedAuto,
            allowed_types: vec!["network".into()],
            ..Default::default()
        },
        ..Default::default()
    };
    let session_id = "test-session-123";
    let decide_counter = Arc::new(AtomicUsize::new(0));
    let second_record = ApprovalRecord {
        approval_id: "app-43".into(),
        ..test_record(session_id)
    };
    let mock = MockSentinel {
        records: vec![test_record(session_id), second_record],
        decide_calls: decide_counter.clone(),
        should_approve_status: true,
    };

    let mut coordinator = ApprovalCoordinator::with_client(&config, session_id, Box::new(mock));
    let result = coordinator
        .handle_approval_snapshot(&serde_json::json!({"event": "approvals.snapshot"}))
        .unwrap();

    assert!(matches!(result, CoordinateResult::ManualRequired { .. }));
    assert_eq!(decide_counter.load(Ordering::SeqCst), 0);
}

#[test]
fn test_chat_stream_approval_handled_and_continued() {
    let mut stream = MuseChatStream::default();
    let session_id = "4f8a29a0-e29b-4e8a-b851-91d1e4c77112";

    // 1. Acknowledge session
    let init_record = serde_json::json!({
        "result": {
            "session_id": session_id,
            "is_thread": true
        }
    });
    stream.record(&init_record).unwrap();
    assert_eq!(stream.session_id.as_deref(), Some(session_id));

    // 2. Set approval handler that returns Ok(true) (approved)
    let approved_count = Arc::new(AtomicUsize::new(0));
    let approved_count_clone = approved_count.clone();
    stream.approval_handler = Some(Box::new(move |_sid, _rec| {
        approved_count_clone.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }));

    // 3. Receive approvals.snapshot for this session
    let snapshot_record = serde_json::json!({
        "event": "approvals.snapshot",
        "payload": {
            "session_id": session_id,
            "pending_approvals": [
                {
                    "approval_id": "app-42",
                    "scope": {
                        "thread_id": session_id
                    }
                }
            ]
        }
    });
    // This MUST succeed without returning MuseApprovalRequired!
    assert!(stream.record(&snapshot_record).is_ok());
    assert_eq!(approved_count.load(Ordering::SeqCst), 1);

    // 4. Stream continues and receives delta.presentation!
    let presentation_record = serde_json::json!({
        "event": "delta.presentation",
        "payload": {
            "session_id": session_id,
            "data": {
                "url": "https://muse.ai/files/test_vid.mp4"
            }
        }
    });
    assert!(stream.record(&presentation_record).is_ok());

    assert_eq!(stream.approved_approval_ids, vec!["app-42"]);

    // 5. Complete stream
    let done_record = serde_json::json!({
        "event": "delta.message_done",
        "payload": {
            "session_id": session_id
        }
    });
    assert!(stream.record(&done_record).is_ok());

    let (final_sid, final_text) = stream.finish().unwrap();
    assert_eq!(final_sid, session_id);
    assert!(final_text.contains("https://muse.ai/files/test_vid.mp4"));
}

#[test]
fn test_chat_stream_approval_unhandled_raises_error() {
    let mut stream = MuseChatStream::default();
    let session_id = "4f8a29a0-e29b-4e8a-b851-91d1e4c77112";

    let init_record = serde_json::json!({
        "result": {
            "session_id": session_id,
            "is_thread": true
        }
    });
    stream.record(&init_record).unwrap();

    // No approval_handler set (mode=off / default)
    let snapshot_record = serde_json::json!({
        "event": "approvals.snapshot",
        "payload": {
            "session_id": session_id,
            "pending_approvals": [
                {
                    "approval_id": "app-42",
                    "scope": {
                        "thread_id": session_id
                    }
                }
            ]
        }
    });

    let err = stream.record(&snapshot_record).unwrap_err();
    assert!(err
        .get_ref()
        .and_then(|e| e.downcast_ref::<MuseApprovalRequired>())
        .is_some());
}
