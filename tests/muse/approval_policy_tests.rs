use toolcase_gateway::config::{MuseApprovalConfig, MuseApprovalMode};
use toolcase_gateway::museai::approval::policy::{ApprovalPolicyEvaluator, PolicyVerdict};
use toolcase_gateway::museai::approval::types::{
    ApprovalPayloadDetails, ApprovalRecord, ApprovalScope, ApprovalStatus, DecisionOption,
};

fn create_sample_record(
    thread_id: &str,
    vm_id: &str,
    app_type: &str,
    host: &str,
    opts: Vec<DecisionOption>,
) -> ApprovalRecord {
    ApprovalRecord {
        approval_id: "app-test-1".to_string(),
        status: ApprovalStatus::Pending,
        payload_type: app_type.to_string(),
        payload: ApprovalPayloadDetails {
            approval_type: app_type.to_string(),
            host: Some(host.to_string()),
            ..Default::default()
        },
        scope: ApprovalScope {
            thread_id: Some(thread_id.to_string()),
            session_scope_id: None,
            vm_id: Some(vm_id.to_string()),
            owner_id: None,
        },
        resource: None,
        decision_options: opts,
        request_expires_at_ms: Some(10000),
        created_at_ms: Some(1000),
    }
}

#[test]
fn test_policy_mode_disabled() {
    let config = MuseApprovalConfig {
        mode: MuseApprovalMode::Off,
        ..Default::default()
    };

    let record = create_sample_record(
        "session-1",
        "vm-1",
        "network",
        "example.com",
        vec![DecisionOption {
            id: "opt-1".into(),
            kind: "allow_once".into(),
            label: None,
            scope: None,
        }],
    );

    let evaluator = ApprovalPolicyEvaluator::new(&config, "session-1", "vm-1", 5000, 0);
    let verdict = evaluator.evaluate(&record);
    assert!(matches!(verdict, PolicyVerdict::Ineligible { .. }));
}

#[test]
fn test_policy_mode_observe() {
    let config = MuseApprovalConfig {
        mode: MuseApprovalMode::Observe,
        ..Default::default()
    };

    let record = create_sample_record(
        "session-1",
        "vm-1",
        "network",
        "example.com",
        vec![DecisionOption {
            id: "opt-1".into(),
            kind: "allow_once".into(),
            label: None,
            scope: None,
        }],
    );

    let evaluator = ApprovalPolicyEvaluator::new(&config, "session-1", "vm-1", 5000, 0);
    let verdict = evaluator.evaluate(&record);
    assert!(matches!(verdict, PolicyVerdict::Ineligible { .. }));
}

#[test]
fn test_policy_scoped_auto_eligible() {
    let config = MuseApprovalConfig {
        mode: MuseApprovalMode::ScopedAuto,
        allowed_types: vec!["network".into()],
        allowed_destinations: vec!["example.com".into()],
        ..Default::default()
    };

    let record = create_sample_record(
        "session-1",
        "vm-1",
        "network",
        "example.com",
        vec![
            DecisionOption {
                id: "opt-1".into(),
                kind: "allow_once".into(),
                label: None,
                scope: None,
            },
            DecisionOption {
                id: "opt-2".into(),
                kind: "allow_always".into(),
                label: None,
                scope: None,
            },
        ],
    );

    let evaluator = ApprovalPolicyEvaluator::new(&config, "session-1", "vm-1", 5000, 0);
    let verdict = evaluator.evaluate(&record);
    assert_eq!(
        verdict,
        PolicyVerdict::Eligible {
            decision_id: "opt-1".to_string(),
            decision_kind: "allow_once".to_string()
        }
    );
}

#[test]
fn test_policy_session_mismatch() {
    let config = MuseApprovalConfig {
        mode: MuseApprovalMode::ScopedAuto,
        ..Default::default()
    };

    let record = create_sample_record(
        "session-OTHER",
        "vm-1",
        "network",
        "example.com",
        vec![DecisionOption {
            id: "opt-1".into(),
            kind: "allow_once".into(),
            label: None,
            scope: None,
        }],
    );

    let evaluator = ApprovalPolicyEvaluator::new(&config, "session-1", "vm-1", 5000, 0);
    let verdict = evaluator.evaluate(&record);
    assert!(matches!(verdict, PolicyVerdict::Ineligible { .. }));
}

#[test]
fn test_policy_unauthorized_type() {
    let config = MuseApprovalConfig {
        mode: MuseApprovalMode::ScopedAuto,
        allowed_types: vec!["network".into()],
        ..Default::default()
    };

    let record = create_sample_record(
        "session-1",
        "vm-1",
        "outgoing_media", // sensitive / not in allowed_types!
        "example.com",
        vec![DecisionOption {
            id: "opt-1".into(),
            kind: "allow_once".into(),
            label: None,
            scope: None,
        }],
    );

    let evaluator = ApprovalPolicyEvaluator::new(&config, "session-1", "vm-1", 5000, 0);
    let verdict = evaluator.evaluate(&record);
    assert!(matches!(verdict, PolicyVerdict::Ineligible { .. }));
}

#[test]
fn test_policy_requires_explicit_destination_allowlist() {
    let config = MuseApprovalConfig {
        mode: MuseApprovalMode::ScopedAuto,
        allowed_types: vec!["network".into()],
        allowed_destinations: vec![],
        ..Default::default()
    };
    let record = create_sample_record(
        "session-1",
        "vm-1",
        "network",
        "example.com",
        vec![DecisionOption {
            id: "opt-1".into(),
            kind: "allow_once".into(),
            label: None,
            scope: None,
        }],
    );

    let verdict =
        ApprovalPolicyEvaluator::new(&config, "session-1", "vm-1", 5000, 0).evaluate(&record);
    assert!(matches!(verdict, PolicyVerdict::Ineligible { .. }));
}

#[test]
fn test_policy_requires_vm_identity() {
    let config = MuseApprovalConfig {
        mode: MuseApprovalMode::ScopedAuto,
        allowed_destinations: vec!["example.com".into()],
        ..Default::default()
    };
    let record = create_sample_record(
        "session-1",
        "vm-1",
        "network",
        "example.com",
        vec![DecisionOption {
            id: "opt-1".into(),
            kind: "allow_once".into(),
            label: None,
            scope: None,
        }],
    );

    let verdict = ApprovalPolicyEvaluator::new(&config, "session-1", "", 5000, 0).evaluate(&record);
    assert!(matches!(verdict, PolicyVerdict::Ineligible { .. }));
}

#[test]
fn test_policy_destination_allowlist() {
    let config = MuseApprovalConfig {
        mode: MuseApprovalMode::ScopedAuto,
        allowed_types: vec!["network".into()],
        allowed_destinations: vec!["trusted.com".into()],
        ..Default::default()
    };

    let record_untrusted = create_sample_record(
        "session-1",
        "vm-1",
        "network",
        "evil.com",
        vec![DecisionOption {
            id: "opt-1".into(),
            kind: "allow_once".into(),
            label: None,
            scope: None,
        }],
    );

    let evaluator = ApprovalPolicyEvaluator::new(&config, "session-1", "vm-1", 5000, 0);
    assert!(matches!(
        evaluator.evaluate(&record_untrusted),
        PolicyVerdict::Ineligible { .. }
    ));

    let record_trusted = create_sample_record(
        "session-1",
        "vm-1",
        "network",
        "api.trusted.com",
        vec![DecisionOption {
            id: "opt-1".into(),
            kind: "allow_once".into(),
            label: None,
            scope: None,
        }],
    );

    assert!(matches!(
        evaluator.evaluate(&record_trusted),
        PolicyVerdict::Eligible { .. }
    ));
}

#[test]
fn test_policy_max_decisions_cap() {
    let config = MuseApprovalConfig {
        mode: MuseApprovalMode::ScopedAuto,
        max_decisions_per_job: 2,
        ..Default::default()
    };

    let record = create_sample_record(
        "session-1",
        "vm-1",
        "network",
        "example.com",
        vec![DecisionOption {
            id: "opt-1".into(),
            kind: "allow_once".into(),
            label: None,
            scope: None,
        }],
    );

    // With decisions_count = 2, max limit is reached!
    let evaluator = ApprovalPolicyEvaluator::new(&config, "session-1", "vm-1", 5000, 2);
    let verdict = evaluator.evaluate(&record);
    assert!(matches!(verdict, PolicyVerdict::Ineligible { .. }));
}

#[test]
fn test_policy_allow_always_only_when_opted_in() {
    let config_no_always = MuseApprovalConfig {
        mode: MuseApprovalMode::ScopedAuto,
        allowed_destinations: vec!["example.com".into()],
        prefer_allow_once: false,
        allow_always: false, // NOT opted in
        ..Default::default()
    };

    let record = create_sample_record(
        "session-1",
        "vm-1",
        "network",
        "example.com",
        vec![DecisionOption {
            id: "opt-always".into(),
            kind: "allow_always".into(),
            label: None,
            scope: None,
        }],
    );

    let evaluator = ApprovalPolicyEvaluator::new(&config_no_always, "session-1", "vm-1", 5000, 0);
    // When server only offers allow_always but config disables it, verdict is Ineligible!
    assert!(matches!(
        evaluator.evaluate(&record),
        PolicyVerdict::Ineligible { .. }
    ));

    // When allow_always is explicitly enabled:
    let config_with_always = MuseApprovalConfig {
        mode: MuseApprovalMode::ScopedAuto,
        allowed_destinations: vec!["example.com".into()],
        prefer_allow_once: false,
        allow_always: true,
        ..Default::default()
    };
    let evaluator_opt_in =
        ApprovalPolicyEvaluator::new(&config_with_always, "session-1", "vm-1", 5000, 0);
    assert_eq!(
        evaluator_opt_in.evaluate(&record),
        PolicyVerdict::Eligible {
            decision_id: "opt-always".to_string(),
            decision_kind: "allow_always".to_string()
        }
    );
}

#[test]
fn test_policy_rejects_connector_permission_changes() {
    let config = MuseApprovalConfig {
        mode: MuseApprovalMode::ScopedAuto,
        allowed_types: vec!["network".into(), "connector".into()],
        allowed_destinations: vec!["muse.ai".into(), "google_drive".into()],
        ..Default::default()
    };

    let record = ApprovalRecord {
        approval_id: "connector-b542f8bd-53f6-4bf2-837a-6367fea47a7f".into(),
        status: ApprovalStatus::Pending,
        payload_type: "connector".into(),
        payload: ApprovalPayloadDetails {
            approval_type: "connector".into(),
            host: Some("google_drive".into()),
            method: Some("permissions.create".into()),
            ..Default::default()
        },
        scope: ApprovalScope {
            thread_id: Some("session-1".into()),
            session_scope_id: Some("session-1".into()),
            vm_id: None,
            owner_id: None,
        },
        resource: None,
        decision_options: vec![DecisionOption {
            id: "allow_once".into(),
            kind: "allow_once".into(),
            label: Some("allow".into()),
            scope: None,
        }],
        request_expires_at_ms: Some(9999999999999),
        created_at_ms: Some(1000),
    };

    let verdict =
        ApprovalPolicyEvaluator::new(&config, "session-1", "vm-1", 5000, 0).evaluate(&record);
    assert!(matches!(verdict, PolicyVerdict::Ineligible { .. }));
}
