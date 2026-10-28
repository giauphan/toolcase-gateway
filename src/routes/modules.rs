//! Platform module dispatcher for OmniRoute and out-of-tree personal modules.

use crate::config::{Config, ConfigStore};
use crate::gateway::modules::{ModuleKind, ModuleStatus};
use crate::http::write_error;
use crate::omniroute::handle_omniroute_proxy;
use std::io;
use std::net::TcpStream;

pub fn dispatch_module_route(
    client: &mut TcpStream,
    request: &crate::http::Request,
    store: &ConfigStore,
    config: &Config,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    let matched = {
        let reg = store.module_registry();
        let reg_guard = match reg.read() {
            Ok(g) => g,
            Err(e) => {
                write_error(client, 500, "Internal Error", &e.to_string())?;
                return Ok(Some(500));
            }
        };

        reg_guard.match_path(clean_path).map(|(d, s)| {
            (
                d.clone(),
                s.to_string(),
                reg_guard.get_personal_manifest(&d.id).cloned(),
            )
        })
    };

    if let Some((desc, subpath, personal_manifest)) = matched {
        if desc.status == ModuleStatus::Disabled {
            write_error(
                client,
                503,
                "Service Unavailable",
                &format!("Module '{}' is currently disabled", desc.id),
            )?;
            return Ok(Some(503));
        }

        if desc.kind == ModuleKind::Personal {
            if let Some(manifest) = personal_manifest.as_ref() {
                let status = crate::providers::personal::handle_personal_module_request(
                    client, request, manifest, &subpath,
                )?;
                return Ok(Some(status));
            }
        }

        if desc.id == "omniroute" {
            let mut req_clone = request.clone();
            let base = if subpath.is_empty() { "/" } else { &subpath };
            req_clone.path = match request.path.split_once('?') {
                Some((_, q)) => format!("{base}?{q}"),
                None => base.to_string(),
            };
            let status = handle_omniroute_proxy(client, &req_clone, config)?;
            return Ok(Some(status));
        }
    }

    Ok(None)
}
