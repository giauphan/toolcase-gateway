use crate::config::Config;
use serde::Deserialize;
use std::io;

#[derive(Debug, Deserialize)]
pub struct MuseAiRequest {
    pub target: String,
    pub method: Option<String>,
    pub body: Option<serde_json::Value>,
}

pub fn build_museai_request(
    request_body: &[u8],
    config: &Config,
) -> io::Result<(String, String, String)> {
    let api_request: MuseAiRequest = serde_json::from_slice(request_body)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid JSON: {e}")))?;

    let target = api_request.target.trim_matches('/');
    if target.is_empty() || target.contains("..") || target.contains('?') || target.contains('#') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid Muse.ai target",
        ));
    }

    let method = api_request
        .method
        .unwrap_or_else(|| "POST".to_string())
        .to_uppercase();
    if !matches!(method.as_str(), "GET" | "POST" | "PUT") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unsupported Muse.ai method",
        ));
    }

    let url = format!(
        "{}/api/{}",
        config.museai_base_url.trim_end_matches('/'),
        target
    );

    let body_json = match &api_request.body {
        Some(body) => serde_json::to_string(body).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Invalid Muse.ai body: {e}"),
            )
        })?,
        None => String::new(),
    };

    Ok((url, method, body_json))
}
