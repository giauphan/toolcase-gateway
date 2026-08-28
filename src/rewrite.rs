pub(crate) fn escape_json_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(crate) fn json_string_value(text: &str, key: &str) -> Option<String> {
    let start = text.find(&format!("\"{key}\""))?;
    let rest = &text[start + key.len() + 2..];
    let open = rest.find('"')?;
    let value = &rest[open + 1..];
    Some(value[..value.find('"')?].to_owned())
}

pub(crate) fn replace_model(body: &[u8], model: &str) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(body) else {
        return body.to_vec();
    };
    let Some(start) = text.find("\"model\"") else {
        return body.to_vec();
    };
    let Some(colon) = text[start..].find(':') else {
        return body.to_vec();
    };
    let value_start = start + colon + 1;
    let Some(open) = text[value_start..].find('"') else {
        return body.to_vec();
    };
    let content_start = value_start + open + 1;
    let Some(close) = text[content_start..].find('"') else {
        return body.to_vec();
    };
    let content_end = content_start + close;
    format!(
        "{}{}{}",
        &text[..content_start],
        escape_json_string(model),
        &text[content_end..]
    )
    .into_bytes()
}

pub(crate) fn rewrite_tool_names(body: &[u8], request_body: &[u8]) -> Vec<u8> {
    let Ok(mut output) = String::from_utf8(body.to_vec()) else {
        return body.to_vec();
    };
    let Ok(request) = std::str::from_utf8(request_body) else {
        return body.to_vec();
    };
    let mut rest = request;
    while let Some(index) = rest.find("\"name\"") {
        rest = &rest[index + 6..];
        let Some(open) = rest.find('"') else { break };
        let value = &rest[open + 1..];
        let Some(close) = value.find('"') else { break };
        let original = &value[..close];
        rest = &value[close + 1..];
        if original.is_empty() || !original.chars().any(|c| c.is_ascii_uppercase()) {
            continue;
        }
        let lower = original.to_ascii_lowercase();
        let mut cursor = 0;
        while let Some(index) = output[cursor..].to_ascii_lowercase().find("\"name\"") {
            let abs_name = cursor + index;
            let after_key = abs_name + 6; // past "name"
            let Some(colon_pos) = output[after_key..].find(':') else {
                break;
            };
            let after_colon = after_key + colon_pos + 1;
            let rest_after_colon = &output[after_colon..];
            let trimmed_start = rest_after_colon.len() - rest_after_colon.trim_start().len();
            let value_open = after_colon + trimmed_start;
            if value_open >= output.len() || output.as_bytes()[value_open] != b'"' {
                break;
            }
            let value_content = value_open + 1;
            let Some(close_rel) = output[value_content..].find('"') else {
                break;
            };
            let value_end = value_content + close_rel;
            let current_value = &output[value_content..value_end];
            if current_value.eq_ignore_ascii_case(&lower) {
                let span = value_content..value_end;
                output.replace_range(span.clone(), original);
                cursor = value_content + original.len();
            } else {
                cursor = value_end + 1;
            }
        }
    }
    output.into_bytes()
}
