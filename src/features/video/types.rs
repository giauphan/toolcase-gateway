use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VideoCapabilityCatalog {
    pub schema_version: String,
    pub providers: Vec<ProviderCapability>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProviderCapability {
    pub provider: String,
    pub verified: bool,
    pub supported_models: Vec<String>,
    pub supported_operations: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ArtifactDescriptor {
    pub kind: String,
    pub download_url: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VideoTaskStatusResponse {
    pub schema_version: String,
    pub provider: String,
    pub provider_task_id: String,
    pub provider_status: String,
    pub artifact_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_url_expires_at: Option<u64>,
    pub media_validated: bool,
    pub reconciliation_capability: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub artifacts: Vec<ArtifactDescriptor>,
}

pub fn validate_secure_url(url_str: &str) -> bool {
    let Some(host_trimmed) = secure_url_host(url_str) else {
        return false;
    };

    if let Ok(ip) = host_trimmed.parse::<IpAddr>() {
        return is_public_ip(ip);
    }

    !(host_trimmed.is_empty()
        || host_trimmed == "localhost"
        || host_trimmed.ends_with(".internal")
        || host_trimmed.ends_with(".local"))
}

fn secure_url_host(url_str: &str) -> Option<String> {
    let rest = url_str.strip_prefix("https://")?;
    let authority = rest.split('/').next().unwrap_or(rest);
    let host = authority.split('@').next_back().unwrap_or(authority);
    let host = if host.starts_with('[') {
        host.split(']').next()?.trim_start_matches('[')
    } else {
        host.split(':').next().unwrap_or(host)
    };
    Some(host.to_ascii_lowercase().trim_end_matches('.').to_string())
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast())
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        || first & 0xfe00 == 0xfc00
        || first & 0xffc0 == 0xfe80)
}
