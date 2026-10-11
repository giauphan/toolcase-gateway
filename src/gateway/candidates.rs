use crate::rewrite::json_string_value;

const EFFORT_SUFFIXES: [&str; 4] = ["-xhigh", "-high", "-medium", "-low"];

pub fn normalize_model_id(requested: &str, default_model: &str) -> String {
    let trimmed = default_model.trim();
    if trimmed.is_empty() {
        return requested.to_string();
    }
    if requested == trimmed {
        return requested.to_string();
    }
    for suffix in EFFORT_SUFFIXES {
        if let Some(base) = requested.strip_suffix(suffix) {
            if base == trimmed {
                return requested.to_string();
            }
            return format!("{trimmed}{suffix}");
        }
    }
    trimmed.to_string()
}

/// Pure candidate planning: extracts model from request body, normalizes it,
/// and applies deterministic rotation across configured fallback models.
pub fn plan_candidates(
    body: &[u8],
    fallbacks: &[String],
    rotation: usize,
    default_model: &str,
) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Ok(text) = std::str::from_utf8(body) {
        if let Some(model) = json_string_value(text, "model") {
            let normalized = normalize_model_id(&model, default_model);
            candidates.push(normalized);
        }
    }
    if !fallbacks.is_empty() {
        let offset = rotation % fallbacks.len();
        for step in 0..fallbacks.len() {
            let fallback = &fallbacks[(offset + step) % fallbacks.len()];
            if !candidates.iter().any(|v| v == fallback) {
                candidates.push(fallback.clone());
            }
        }
    }
    if candidates.is_empty() {
        candidates.push(default_model.trim().to_string());
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candidate_planning() {
        let body = br#"{"model":"gpt-4o"}"#;
        let fallbacks = vec!["claude-3-5".to_string()];
        let list = plan_candidates(body, &fallbacks, 0, "default");
        assert_eq!(list[0], "default");
        assert_eq!(list[1], "claude-3-5");
    }
}
