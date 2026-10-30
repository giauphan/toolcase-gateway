//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;
use super::har_config_tests::{
    har_fixture_full, har_fixture_no_muse, har_fixture_ws_only, test_har_dir,
};

// ---------- HAR → config feature: strict extraction & persistence tests ----------

fn har_fixture_empty_token() -> &'static str {
    r#"{
      "log": {
        "entries": [
          {
            "request": {
              "url": "wss://hatch.metaaivm.com/v1/noise?auth_token=&notary_token=notary-token-def456"
            }
          }
        ]
      }
    }"#
}

fn har_fixture_encoded_token() -> &'static str {
    r#"{
      "log": {
        "entries": [
          {
            "request": {
              "url": "wss://hatch.metaaivm.com/v1/noise?auth_token=%74%6F%6B&notary_token=n"
            }
          }
        ]
      }
    }"#
}

#[test]
fn har_extraction_full_capture_extracts_all_fields() {
    let parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har_fixture_full().as_bytes())
            .unwrap();
    assert_eq!(
        parsed.ws_url.as_deref(),
        Some("wss://hatch.metaaivm.com/v1/noise")
    );
    assert_eq!(parsed.base_url.as_deref(), Some("https://muse.ai"));
    assert_eq!(parsed.access_token.as_deref(), Some("access-token-abc123"));
    assert_eq!(parsed.notary_token.as_deref(), Some("notary-token-def456"));
    assert_eq!(parsed.vm_id.as_deref(), Some("vm-id-42"));
    assert_eq!(
        parsed.cookie.as_deref(),
        Some("sessionId=abc123; theme=dark")
    );
}

#[test]
fn har_extraction_rejects_untrusted_rest_derived_websocket_urls() {
    for endpoint_url in [
        "wss://attacker.example/v1/noise",
        "wss://metaaivm.com.evil.example/v1/noise",
        "ws://hatch.metaaivm.com/v1/noise",
        "https://hatch.metaaivm.com/v1/noise",
    ] {
        let har = serde_json::json!({
            "log": {"entries": [{
                "request": {"url": "https://muse.ai/api/session"},
                "response": {"content": {"text": format!("{{\"vm_id\":\"vm-from-response\",\"endpoint_url\":\"{endpoint_url}\"}}")}}
            }]}
        });
        let parsed =
            toolcase_gateway::har_config::extract_muse_config_from_har(har.to_string().as_bytes())
                .unwrap();
        assert_eq!(
            parsed.vm_id.as_deref(),
            Some("vm-from-response"),
            "{endpoint_url}"
        );
        assert_eq!(parsed.ws_url, None, "{endpoint_url}");
    }
}

#[test]
fn har_extraction_rejects_untrusted_noise_websocket_hosts() {
    for url in [
        "wss://attacker.example/v1/noise?auth_token=secret",
        "wss://metaaivm.com.evil.example/v1/noise?auth_token=secret",
        "ws://hatch.metaaivm.com/v1/noise?auth_token=secret",
    ] {
        let har = serde_json::json!({"log": {"entries": [{"request": {"url": url}}]}});
        let error =
            toolcase_gateway::har_config::extract_muse_config_from_har(har.to_string().as_bytes())
                .unwrap_err();
        assert!(matches!(
            error,
            toolcase_gateway::har_config::HarExtractError::NoMuseEntries
        ));
    }
}

#[test]
fn har_extraction_vm_id_from_session_response() {
    let har = r#"{
      "log": {"entries": [
        {"request": {"url": "https://muse.ai/api/session"}}
      ]}
    }"#;
    let mut parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har.as_bytes()).unwrap();
    assert_eq!(parsed.base_url.as_deref(), Some("https://muse.ai"));
    assert!(parsed.vm_id.is_none(), "no response body => no vm_id");

    let har_with_vm = r#"{
      "log": {"entries": [
        {
          "request": {"url": "https://muse.ai/api/session"},
          "response": {"content": {"text": "{\"vm_id\":\"vm-from-response\",\"endpoint_url\":\"wss://x.metaaivm.com/\"}"}}
        }
      ]}
    }"#;
    parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har_with_vm.as_bytes()).unwrap();
    assert_eq!(parsed.vm_id.as_deref(), Some("vm-from-response"));
    assert_eq!(parsed.ws_url.as_deref(), Some("wss://x.metaaivm.com/"));
    assert_eq!(parsed.base_url.as_deref(), Some("https://muse.ai"));
}

#[test]
fn har_extraction_selects_preferred_vm_from_current_session_shape() {
    let har = r#"{
      "log": {"entries": [
        {
          "request": {"url": "https://muse.ai/api/session"},
          "response": {"content": {"text": "{\"vms\":[{\"id\":\"fallback-id\",\"endpoint_url\":\"wss://fallback.invalid/\",\"is_preferred\":false},{\"vm_id\":\"preferred-vm\",\"endpoint_url\":\"wss://preferred.metaaivm.com/\",\"is_preferred\":true}]}"}}
        }
      ]}
    }"#;
    let parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har.as_bytes()).unwrap();
    assert_eq!(parsed.vm_id.as_deref(), Some("preferred-vm"));
    assert_eq!(
        parsed.ws_url.as_deref(),
        Some("wss://preferred.metaaivm.com/")
    );
    assert!(parsed.notary_token.is_none());
}

#[test]
fn har_extraction_reads_standard_cookie_arrays_and_preserves_them() {
    let har = r#"{
      "log": {"entries": [
        {
          "request": {
            "url": "https://muse.ai/api/session",
            "cookies": [
              {"name": "sessionId", "value": "synthetic-session"},
              {"name": "theme", "value": "dark"}
            ]
          }
        },
        {
          "request": {"url": "https://muse.ai/api/auth/check", "headers": []},
          "response": {"content": {"text": "{\"access_token\":\"synthetic-token\"}"}}
        }
      ]}
    }"#;
    let parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har.as_bytes()).unwrap();
    assert_eq!(
        parsed.cookie.as_deref(),
        Some("sessionId=synthetic-session; theme=dark")
    );
    assert_eq!(parsed.access_token.as_deref(), Some("synthetic-token"));
}

#[test]
fn har_extraction_accepts_case_insensitive_cookie_header() {
    let har = r#"{
      "log": {"entries": [
        {
          "request": {
            "url": "https://muse.ai/api/session",
            "headers": [{"name": "cookie", "value": "sessionId=synthetic"}]
          }
        }
      ]}
    }"#;
    let parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har.as_bytes()).unwrap();
    assert_eq!(parsed.cookie.as_deref(), Some("sessionId=synthetic"));
}

#[test]
fn har_extraction_access_token_from_auth_check_response() {
    let har = r#"{
      "log": {"entries": [
        {
          "request": {"url": "https://muse.ai/api/auth/check"},
          "response": {"content": {"text": "{\"access_token\":\"response-token\",\"ok\":true}"}}
        }
      ]}
    }"#;
    let parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har.as_bytes()).unwrap();
    assert_eq!(parsed.access_token.as_deref(), Some("response-token"));
}

#[test]
fn har_extraction_duplicates_keep_last_nonempty_value() {
    let har = serde_json::json!({
        "log": {"entries": [
            {"request": {"url": "wss://hatch.metaaivm.com/v1/noise?vm_id=first&auth_token=a1&notary_token=n1"}},
            {"request": {"url": "wss://hatch.metaaivm.com/v1/noise?vm_id=second&notary_token=n2"}}
        ]}
    });
    let parsed =
        toolcase_gateway::har_config::extract_muse_config_from_har(har.to_string().as_bytes())
            .unwrap();
    assert_eq!(parsed.vm_id.as_deref(), Some("second"));
    assert_eq!(parsed.access_token.as_deref(), Some("a1"));
    assert_eq!(parsed.notary_token.as_deref(), Some("n2"));
}

#[test]
fn har_extraction_empty_param_counts_as_missing() {
    let parsed = toolcase_gateway::har_config::extract_muse_config_from_har(
        har_fixture_empty_token().as_bytes(),
    )
    .unwrap();
    assert!(parsed.access_token.is_none());
    assert_eq!(parsed.notary_token.as_deref(), Some("notary-token-def456"));
}

#[test]
fn har_extraction_percent_decodes_query_values() {
    let parsed = toolcase_gateway::har_config::extract_muse_config_from_har(
        har_fixture_encoded_token().as_bytes(),
    )
    .unwrap();
    assert_eq!(parsed.access_token.as_deref(), Some("tok"));
}

#[test]
fn har_extraction_rejects_invalid_json() {
    let err = toolcase_gateway::har_config::extract_muse_config_from_har(b"not json").unwrap_err();
    assert!(matches!(
        err,
        toolcase_gateway::har_config::HarExtractError::InvalidJson(_)
    ));
}

#[test]
fn har_extraction_rejects_non_har_shapes() {
    for shape in [
        "{\"foo\":1}",
        "{\"log\":{}}",
        "{\"log\":{\"entries\":\"nope\"}}",
    ] {
        let err = toolcase_gateway::har_config::extract_muse_config_from_har(shape.as_bytes())
            .unwrap_err();
        assert!(
            matches!(err, toolcase_gateway::har_config::HarExtractError::NotHar),
            "unexpected error for {shape}: {err:?}"
        );
    }
}

#[test]
fn har_extraction_rejects_capture_without_muse_entries() {
    let err = toolcase_gateway::har_config::extract_muse_config_from_har(
        har_fixture_no_muse().as_bytes(),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        toolcase_gateway::har_config::HarExtractError::NoMuseEntries
    ));
}

#[test]
fn har_extraction_ws_only_capture_is_partial_ok() {
    let parsed = toolcase_gateway::har_config::extract_muse_config_from_har(
        har_fixture_ws_only().as_bytes(),
    )
    .unwrap();
    assert!(parsed.cookie.is_none());
    assert!(parsed.base_url.is_none());
    assert_eq!(parsed.vm_id.as_deref(), Some("vm-id-42"));
}

#[test]
fn mask_secret_masks_full_and_partial() {
    assert_eq!(
        toolcase_gateway::har_config::mask_secret("abcdef12"),
        "****"
    );
    assert_eq!(
        toolcase_gateway::har_config::mask_secret("abcdef12345"),
        "abcd…2345"
    );
    assert_eq!(
        toolcase_gateway::har_config::mask_secret("cafééééééé"),
        "café…éééé"
    );
}

#[test]
fn persist_env_creates_file_with_managed_keys_only() {
    let dir = test_har_dir("create");
    let path = dir.join(".env");
    toolcase_gateway::har_config::persist_muse_env(
        &path,
        &[
            ("GW_MUSEAI_VM_ID", "vm-id-42"),
            ("GW_MUSEAI_ACCESS_TOKEN", "tok-abc"),
        ],
    )
    .unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("GW_MUSEAI_VM_ID=vm-id-42"));
    assert!(content.contains("GW_MUSEAI_ACCESS_TOKEN=tok-abc"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persist_env_replaces_managed_key_preserves_unrelated() {
    let dir = test_har_dir("replace");
    let path = dir.join(".env");
    std::fs::write(
        &path,
        "# comment\nGW_LISTEN_PORT=20129\nGW_MUSEAI_ACCESS_TOKEN=old-token\n",
    )
    .unwrap();
    toolcase_gateway::har_config::persist_muse_env(
        &path,
        &[
            ("GW_MUSEAI_ACCESS_TOKEN", "new-token"),
            ("GW_MUSEAI_VM_ID", "vm-1"),
        ],
    )
    .unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("# comment"));
    assert!(content.contains("GW_LISTEN_PORT=20129"));
    assert!(content.contains("GW_MUSEAI_ACCESS_TOKEN=new-token"));
    assert!(content.contains("GW_MUSEAI_VM_ID=vm-1"));
    assert!(!content.contains("old-token"));
    assert_eq!(content.matches("GW_MUSEAI_ACCESS_TOKEN=").count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persist_env_keeps_existing_managed_key_without_new_value() {
    let dir = test_har_dir("keep");
    let path = dir.join(".env");
    std::fs::write(&path, "GW_MUSEAI_WS_URL=old-ws\nGW_LISTEN_PORT=1\n").unwrap();
    toolcase_gateway::har_config::persist_muse_env(&path, &[("GW_MUSEAI_VM_ID", "v")]).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("GW_MUSEAI_WS_URL=old-ws"));
    assert!(content.contains("GW_MUSEAI_VM_ID=v"));
    assert!(content.contains("GW_LISTEN_PORT=1"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persist_env_quotes_values_with_spaces() {
    let dir = test_har_dir("quote");
    let path = dir.join(".env");
    toolcase_gateway::har_config::persist_muse_env(
        &path,
        &[("GW_MUSEAI_COOKIE", "sessionId=abc; theme=dark")],
    )
    .unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("GW_MUSEAI_COOKIE=\"sessionId=abc; theme=dark\""));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persist_env_noop_when_no_managed_keys() {
    let dir = test_har_dir("noop");
    let path = dir.join(".env");
    toolcase_gateway::har_config::persist_muse_env(&path, &[("GW_LISTEN_PORT", "1")]).unwrap();
    assert!(!path.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
