/// Prepares the payload for OpenRouter's decision endpoint (`/api/alpha/decisions`).
/// Ensures that `"model"` is present and set to a supported OpenRouter model identifier
/// (defaulting to `default_model` e.g. `"typesafe/jev-1.13"` if omitted or generic `"jev-1.13"`).
pub fn prepare_openrouter_payload(raw_body: &[u8], default_model: &str) -> Vec<u8> {
    let mut val: serde_json::Value = match serde_json::from_slice(raw_body) {
        Ok(v) => v,
        Err(_) => return raw_body.to_vec(),
    };

    if let Some(obj) = val.as_object_mut() {
        let needs_model = match obj.get("model") {
            None => true,
            Some(serde_json::Value::Null) => true,
            Some(serde_json::Value::String(s)) => {
                let trimmed = s.trim();
                trimmed.is_empty()
                    || trimmed.eq_ignore_ascii_case("jev")
                    || trimmed.eq_ignore_ascii_case("jev-1.13")
                    || trimmed.eq_ignore_ascii_case("jev-ai")
            }
            _ => false,
        };

        if needs_model {
            let model_to_set = if default_model.trim().is_empty() {
                "typesafe/jev-1.13"
            } else {
                default_model.trim()
            };
            obj.insert(
                "model".to_string(),
                serde_json::Value::String(model_to_set.to_string()),
            );
        }
    }

    serde_json::to_vec(&val).unwrap_or_else(|_| raw_body.to_vec())
}

/// Determines whether a credential token represents an OpenRouter API key (e.g. `sk-or-v1-...`)
/// or a browser session cookie.
pub fn is_openrouter_api_key(credential: &str) -> bool {
    let trimmed = credential.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.starts_with("sk-or-")
        || trimmed.starts_with("sk-")
        || trimmed.starts_with("Bearer ")
        || trimmed.starts_with("bearer ")
}

/// Dispatches an evaluation request to the OpenRouter Decision API (`/api/alpha/decisions`).
/// Supports standard OpenRouter Bearer API key authentication (`Authorization: Bearer <key>`)
/// as documented in https://openrouter.ai/docs/quickstart, browser session cookies (`Cookie: ...`),
/// or both simultaneously.
#[allow(clippy::too_many_arguments)]
pub fn send_openrouter_decision_request(
    agent: &ureq::Agent,
    base_url: &str,
    credential: &str,
    fallback_cookie: Option<&str>,
    body: &[u8],
    idempotency_key: Option<&str>,
    site_url: Option<&str>,
    site_name: Option<&str>,
) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let clean_base = base_url.trim_end_matches('/');
    let target_url = format!("{clean_base}/api/alpha/decisions");
    let mut req = agent.post(&target_url);
    req = req.header("Content-Type", "application/json");
    req = req.header("Accept", "application/json");

    let clean_cred = credential.trim();
    if is_openrouter_api_key(clean_cred) {
        req = req.header("Authorization", format!("Bearer {clean_cred}"));
        let ref_url = site_url.unwrap_or("https://github.com/giauphan/toolcase-gateway");
        req = req.header("HTTP-Referer", ref_url);
        let title = site_name.unwrap_or("toolcase-gateway");
        req = req.header("X-Title", title);

        if let Some(cookie) = fallback_cookie {
            let c_clean = cookie.trim();
            if !c_clean.is_empty() {
                req = req.header("Cookie", c_clean);
            }
        }
    } else if !clean_cred.is_empty() {
        req = req.header("Cookie", clean_cred);
        req = req.header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        );
    }

    if let Some(idemp) = idempotency_key {
        let clean_idemp = idemp.trim();
        if !clean_idemp.is_empty() {
            req = req.header("Idempotency-Key", clean_idemp);
        }
    }

    req.send(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_openrouter_api_key_detection() {
        assert!(is_openrouter_api_key("sk-or-v1-abcdef1234567890"));
        assert!(is_openrouter_api_key("sk-ant-test-token"));
        assert!(is_openrouter_api_key("Bearer sk-or-token-here"));
        assert!(!is_openrouter_api_key(""));
        assert!(!is_openrouter_api_key(
            "or_statsig_stable_id=123; clerk_active_context=456"
        ));
        assert!(!is_openrouter_api_key("__session=ey12345="));
        assert!(!is_openrouter_api_key("or_session_11111111111"));
    }

    #[test]
    fn test_prepare_openrouter_payload_injects_model_when_missing() {
        let input = br#"{"state":{"task":"refund"},"questions":{"category":{"type":"choice"}}}"#;
        let output = prepare_openrouter_payload(input, "typesafe/jev-1.13");
        let val: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(val["model"], "typesafe/jev-1.13");
        assert_eq!(val["state"]["task"], "refund");
    }

    #[test]
    fn test_prepare_openrouter_payload_normalizes_generic_jev_model() {
        let input = br#"{"model":"jev-1.13","state":{"task":"refund"}}"#;
        let output = prepare_openrouter_payload(input, "typesafe/jev-1.13");
        let val: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(val["model"], "typesafe/jev-1.13");
    }

    #[test]
    fn test_prepare_openrouter_payload_preserves_custom_model() {
        let input = br#"{"model":"openai/gpt-4o","state":{"task":"refund"}}"#;
        let output = prepare_openrouter_payload(input, "typesafe/jev-1.13");
        let val: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(val["model"], "openai/gpt-4o");
    }

    #[test]
    fn test_prepare_openrouter_payload_handles_malformed_json_gracefully() {
        let input = b"invalid json content";
        let output = prepare_openrouter_payload(input, "typesafe/jev-1.13");
        assert_eq!(output, input.to_vec());
    }
}
