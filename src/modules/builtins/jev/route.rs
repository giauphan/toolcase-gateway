pub(crate) struct JevRoute {
    pub(crate) upstream_path: String,
    pub(crate) expected_method: &'static str,
}

pub(crate) fn resolve_jev_route(inbound_path: &str) -> Option<JevRoute> {
    let clean = inbound_path.split('?').next()?.trim_end_matches('/');
    let relative = if clean == "/jev/models" {
        "models"
    } else if clean == "/jev/decision" || clean == "/jev/decisions" {
        "systemone"
    } else {
        clean
            .strip_prefix("/jev/v1/")
            .or_else(|| clean.strip_prefix("/jev-ai/v1/"))
            .or_else(|| clean.strip_prefix("/jev/"))?
    };

    match relative {
        "models" => Some(JevRoute {
            upstream_path: "/api/v1/models/".to_string(),
            expected_method: "GET",
        }),
        "systemone" | "decision" | "decisions" => Some(JevRoute {
            upstream_path: "/api/v1/systemone/".to_string(),
            expected_method: "POST",
        }),
        _ => {
            let segments: Vec<&str> = relative.split('/').collect();
            if segments.first() != Some(&"requests") {
                return None;
            }
            let request_id = segments.get(1)?;
            if !is_valid_request_id(request_id) {
                return None;
            }
            match segments.len() {
                2 => Some(JevRoute {
                    upstream_path: format!("/api/v1/requests/{request_id}/"),
                    expected_method: "GET",
                }),
                3 if segments[2] == "cancel" => Some(JevRoute {
                    upstream_path: format!("/api/v1/requests/{request_id}/cancel/"),
                    expected_method: "POST",
                }),
                _ => None,
            }
        }
    }
}

fn is_valid_request_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
