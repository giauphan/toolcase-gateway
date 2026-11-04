//! Local HTTP module request dispatching over loopback socket.

use super::stdio::DEFAULT_MODULE_TIMEOUT;
use super::write_personal_error;
use crate::gateway::modules::PersonalManifest;
use crate::http::Request;
use std::io;
use std::net::TcpStream;

pub(crate) fn dispatch_local_http(
    client: &mut TcpStream,
    request: &Request,
    manifest: &PersonalManifest,
    subpath: &str,
) -> io::Result<u16> {
    let base_url = manifest.entrypoint.trim_end_matches('/');
    let clean_sub = subpath.trim_start_matches('/');
    let target_url = match request.path.split_once('?') {
        Some((_, q)) if !q.is_empty() && !clean_sub.contains('?') => {
            format!("{base_url}/{clean_sub}?{q}")
        }
        _ => format!("{base_url}/{clean_sub}"),
    };

    let agent_config = ureq::Agent::config_builder()
        .timeout_global(Some(DEFAULT_MODULE_TIMEOUT))
        .max_redirects(0)
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(agent_config);

    let is_post = request.method.eq_ignore_ascii_case("post");
    let is_get = request.method.eq_ignore_ascii_case("get");
    if !is_post && !is_get {
        return write_personal_error(
            client,
            405,
            &format!(
                "Method '{}' is not supported for local-http personal modules",
                request.method
            ),
        );
    }

    let resp = match if is_post {
        let mut req = agent.post(&target_url);
        for (k, v) in &request.headers {
            if !k.eq_ignore_ascii_case("host") && !k.eq_ignore_ascii_case("content-length") {
                req = req.header(k, v);
            }
        }
        req.send(&request.body)
    } else {
        let mut req = agent.get(&target_url);
        for (k, v) in &request.headers {
            if !k.eq_ignore_ascii_case("host") && !k.eq_ignore_ascii_case("content-length") {
                req = req.header(k, v);
            }
        }
        req.call()
    } {
        Ok(r) => r,
        Err(e) => {
            return write_personal_error(
                client,
                502,
                &format!("Upstream personal module error: {e}"),
            );
        }
    };

    let status = resp.status().as_u16();
    crate::http_provider::write_ureq_response(client, resp)?;
    Ok(status)
}

/// Dispatches an out-of-process personal module tool request over local HTTP.
pub fn execute_http_payload(
    manifest: &PersonalManifest,
    subpath: &str,
    method: &str,
    body: &str,
) -> Result<String, (u16, String)> {
    let base_url = manifest.entrypoint.trim_end_matches('/');
    let clean_sub = subpath.trim_start_matches('/');
    let target_url = format!("{base_url}/{clean_sub}");

    let agent_config = ureq::Agent::config_builder()
        .timeout_global(Some(DEFAULT_MODULE_TIMEOUT))
        .max_redirects(0)
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(agent_config);

    let is_post = method.eq_ignore_ascii_case("post");
    let resp = if is_post {
        agent
            .post(&target_url)
            .header("content-type", "application/json")
            .send(body.as_bytes())
    } else {
        agent
            .get(&target_url)
            .header("content-type", "application/json")
            .call()
    };

    let mut response =
        resp.map_err(|e| (502, format!("Upstream personal module HTTP error: {e}")))?;
    let status = response.status().as_u16();
    let body_str = response
        .body_mut()
        .read_to_string()
        .map_err(|e| (502, format!("Failed to read upstream response body: {e}")))?;

    if status >= 400 {
        return Err((status, body_str));
    }

    Ok(body_str)
}
