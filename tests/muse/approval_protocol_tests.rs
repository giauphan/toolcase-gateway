use toolcase_gateway::museai::approval::client::validate_approval_id;
use toolcase_gateway::museai::approval::types::{
    parse_approval_get, parse_approvals_list, ApprovalPayloadDetails, ApprovalRecord,
    ApprovalScope, ApprovalStatus, MAX_APPROVAL_PAYLOAD_BYTES,
};
use toolcase_gateway::museai::protocol::{
    encode_service_request, SERVICE_DAEMON, SERVICE_SENTINEL,
};

#[test]
fn test_service_sentinel_wire_encoding() {
    let dummy_frame = b"service_frame_content";
    let encoded_daemon = encode_service_request(SERVICE_DAEMON, dummy_frame);
    let encoded_sentinel = encode_service_request(SERVICE_SENTINEL, dummy_frame);

    // SERVICE_DAEMON (0) does not write field 1 (service ID)
    // SERVICE_SENTINEL (1) writes field 1 as varint (1)
    assert_ne!(encoded_daemon, encoded_sentinel);
    // Field 1 wire format: (1 << 3) | 0 = 0x08, followed by varint 1 = 0x01
    assert_eq!(&encoded_sentinel[..2], &[0x08, 0x01]);
    assert_eq!(
        &encoded_sentinel[encoded_sentinel.len() - dummy_frame.len()..],
        dummy_frame
    );
}

#[test]
fn test_validate_approval_id() {
    assert!(validate_approval_id("app-12345").is_ok());
    assert!(validate_approval_id("app_123.45-xyz").is_ok());

    assert!(validate_approval_id("").is_err());
    assert!(validate_approval_id("../traversal").is_err());
    assert!(validate_approval_id("id with spaces").is_err());
    assert!(validate_approval_id("id*with$special").is_err());
    assert!(validate_approval_id(&"a".repeat(129)).is_err());
}

#[test]
fn test_parse_approvals_list_valid() {
    let json = serde_json::json!({
        "ok": true,
        "result": [
            {
                "approval_id": "app-001",
                "status": "pending",
                "type": "network",
                "payload": {
                    "type": "network",
                    "host": "api.example.com",
                    "url": "https://api.example.com/data",
                    "method": "GET"
                },
                "scope": {
                    "thread_id": "thread-123",
                    "vm_id": "vm-abc"
                },
                "decision_options": [
                    {
                        "id": "opt-1",
                        "kind": "allow_once",
                        "label": "Allow once"
                    },
                    {
                        "id": "opt-2",
                        "kind": "allow_always",
                        "label": "Allow always"
                    }
                ],
                "request_expires_at_ms": 1700000050000_u64
            }
        ]
    });

    let bytes = serde_json::to_vec(&json).unwrap();
    let records = parse_approvals_list(&bytes).unwrap();
    assert_eq!(records.len(), 1);
    let r = &records[0];
    assert_eq!(r.approval_id, "app-001");
    assert_eq!(r.status, ApprovalStatus::Pending);
    assert_eq!(r.payload_type, "network");
    assert_eq!(r.payload.host.as_deref(), Some("api.example.com"));
    assert_eq!(r.target_destination(), Some("api.example.com"));
    assert!(r.scope.matches_session("thread-123"));
    assert!(!r.scope.matches_session("thread-other"));
    assert!(r.scope.matches_vm("vm-abc"));
    assert_eq!(r.decision_options.len(), 2);
    assert_eq!(r.decision_options[0].kind, "allow_once");
    assert_eq!(r.decision_options[1].kind, "allow_always");
}

#[test]
fn test_parse_live_sentinel_result_pending_approvals() {
    let body = serde_json::json!({
        "ok": true,
        "result": {
            "schema_version": 1,
            "pending_approvals": [{
                "approval_id": "app-001",
                "status": "pending",
                "payload": {"type": "network", "host": "example.com"},
                "scope": {"thread_id": "session-1", "vm_id": "vm-1"},
                "decision_options": [{"id": "opt-1", "kind": "allow_once"}]
            }],
            "recent_approvals": []
        }
    });
    let parsed = parse_approvals_list(&serde_json::to_vec(&body).unwrap()).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].approval_id, "app-001");
}

#[test]
fn test_parse_approvals_list_size_limit() {
    let large_body = vec![b'a'; MAX_APPROVAL_PAYLOAD_BYTES + 10];
    let err = parse_approvals_list(&large_body).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn test_parse_approval_get_ok_false() {
    let json = serde_json::json!({
        "ok": false,
        "error": {
            "message": "Approval not found"
        }
    });
    let bytes = serde_json::to_vec(&json).unwrap();
    let err = parse_approval_get(&bytes).unwrap_err();
    assert!(err.to_string().contains("Approval not found"));
}

#[test]
fn test_approval_record_expiration() {
    let record = ApprovalRecord {
        approval_id: "test".into(),
        status: ApprovalStatus::Pending,
        payload_type: "network".into(),
        payload: ApprovalPayloadDetails::default(),
        scope: ApprovalScope::default(),
        resource: None,
        decision_options: vec![],
        request_expires_at_ms: Some(1000),
        created_at_ms: Some(500),
    };

    assert!(!record.is_expired(999));
    assert!(record.is_expired(1000));
    assert!(record.is_expired(1001));
}

#[test]
fn test_parse_connector_live_schema() {
    let body = serde_json::json!({
        "approval_id": "connector-b542f8bd-53f6-4bf2-837a-6367fea47a7f",
        "status": "pending",
        "type": "connector",
        "payload": {
            "type": "connector",
            "connector_payload": {
                "host": "google_drive",
                "method": "permissions.create"
            }
        },
        "scope": {
            "thread_id": "3870c64e-f623-449d-82e4-a105656a5ae6",
            "session_scope_id": "0050a58e-707c-47d1-9546-87b38f728341"
        },
        "decision_options": [
            {
                "kind": "allow_once",
                "label": {"kind": "allow"},
                "label_text": "Allow"
            },
            {
                "kind": "deny",
                "label": {"kind": "deny_for_this_task"},
                "label_text": "Deny"
            }
        ]
    });
    let parsed =
        toolcase_gateway::museai::approval::types::parse_approval_record_json(&body).unwrap();
    assert_eq!(
        parsed.approval_id,
        "connector-b542f8bd-53f6-4bf2-837a-6367fea47a7f"
    );
    assert_eq!(parsed.status, ApprovalStatus::Pending);
    assert_eq!(parsed.payload_type, "connector");
    assert_eq!(parsed.target_destination(), Some("google_drive"));
    assert!(parsed
        .scope
        .matches_session("3870c64e-f623-449d-82e4-a105656a5ae6"));
    assert!(parsed.scope.matches_vm(""));
    assert_eq!(parsed.decision_options.len(), 2);
    assert_eq!(parsed.decision_options[0].id, "allow_once");
    assert_eq!(parsed.decision_options[0].kind, "allow_once");
    assert_eq!(parsed.decision_options[0].label.as_deref(), Some("allow"));
}

#[test]
fn test_parse_approval_get_live_envelope() {
    let body = serde_json::json!({
        "ok": true,
        "result": {
            "approval": {
                "approval_id": "connector-7b73c588-830f-486f-952c-feea4415b9ba",
                "status": "pending",
                "scheme": "connector",
                "host": "google_drive",
                "method": "permissions.create",
                "thread_id": "3870c64e-f623-449d-82e4-a105656a5ae6",
                "session_scope_id": "0050a58e-707c-47d1-9546-87b38f728341",
                "decision_options": ["allow_once", "deny"]
            }
        }
    });
    let parsed = parse_approval_get(&serde_json::to_vec(&body).unwrap()).unwrap();
    assert_eq!(
        parsed.approval_id,
        "connector-7b73c588-830f-486f-952c-feea4415b9ba"
    );
    assert_eq!(parsed.status, ApprovalStatus::Pending);
    assert_eq!(parsed.payload_type, "connector");
    assert_eq!(parsed.target_destination(), Some("google_drive"));
    assert_eq!(parsed.decision_options.len(), 2);
    assert_eq!(parsed.decision_options[0].id, "allow_once");
    assert_eq!(parsed.decision_options[0].kind, "allow_once");
}
