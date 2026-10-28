//! Platform module dispatcher for built-in providers, personal modules, and module groups.

use crate::config::{Config, ConfigStore};
use crate::gateway::modules::{ModuleKind, ModuleStatus, PersonalManifest};
use crate::http::write_error;
use crate::omniroute::handle_omniroute_proxy;
use std::io;
use std::net::TcpStream;

fn dispatch_builtin_omniroute(
    client: &mut TcpStream,
    request: &crate::http::Request,
    config: &Config,
    subpath: &str,
) -> io::Result<u16> {
    let mut req_clone = request.clone();
    let base = if subpath.is_empty() { "/" } else { subpath };
    req_clone.path = match request.path.split_once('?') {
        Some((_, q)) => format!("{base}?{q}"),
        None => base.to_string(),
    };
    handle_omniroute_proxy(client, &req_clone, config)
}

fn dispatch_builtin_muse(
    client: &mut TcpStream,
    request: &crate::http::Request,
    config: &Config,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    if let Some(code) = crate::routes::muse::dispatch_catalog_route(client, request, clean_path)? {
        return Ok(Some(code));
    }
    if let Some(code) =
        crate::routes::muse::dispatch_muse_chat_route(client, request, config, clean_path)?
    {
        return Ok(Some(code));
    }
    if let Some(code) =
        crate::routes::muse::dispatch_muse_image_route(client, request, config, clean_path)?
    {
        return Ok(Some(code));
    }
    if let Some(code) =
        crate::routes::muse::dispatch_muse_video_route(client, request, config, clean_path)?
    {
        return Ok(Some(code));
    }
    if let Some(code) =
        crate::routes::muse::dispatch_video_provider_route(client, request, clean_path)?
    {
        return Ok(Some(code));
    }
    Ok(None)
}

fn dispatch_builtin_jev(
    client: &mut TcpStream,
    request: &crate::http::Request,
    store: &ConfigStore,
    config: &Config,
    clean_path: &str,
) -> io::Result<Option<u16>> {
    crate::routes::jev::dispatch_jev_route(client, request, config, &store.jev_pool(), clean_path)
}

fn dispatch_personal(
    client: &mut TcpStream,
    request: &crate::http::Request,
    manifest: &PersonalManifest,
    subpath: &str,
    sub_provider_id: Option<&str>,
) -> io::Result<u16> {
    crate::providers::personal::handle_personal_module_request_with_provider(
        client,
        request,
        manifest,
        subpath,
        sub_provider_id,
    )
}

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

        reg_guard
            .match_path_with_provider(clean_path)
            .map(|(d, s, p)| {
                (
                    d.clone(),
                    s.to_string(),
                    p.cloned(),
                    reg_guard.get_personal_manifest(&d.id).cloned(),
                )
            })
    };

    let Some((desc, subpath, sub_prov, personal_manifest)) = matched else {
        return Ok(None);
    };

    if desc.status == ModuleStatus::Disabled {
        write_error(
            client,
            503,
            "Service Unavailable",
            &format!("Module '{}' is currently disabled", desc.id),
        )?;
        return Ok(Some(503));
    }

    if let Some(ref p) = sub_prov {
        if p.status == ModuleStatus::Disabled {
            write_error(
                client,
                503,
                "Service Unavailable",
                &format!(
                    "Sub-provider '{}' in module '{}' is currently disabled",
                    p.id, desc.id
                ),
            )?;
            return Ok(Some(503));
        }
    }

    match desc.kind {
        ModuleKind::Personal => {
            if let Some(manifest) = personal_manifest.as_ref() {
                let sub_id = sub_prov.as_ref().map(|p| p.id.as_str());
                let status = dispatch_personal(client, request, manifest, &subpath, sub_id)?;
                return Ok(Some(status));
            }
            Ok(None)
        }
        ModuleKind::BuiltIn => {
            if desc.id == "omniroute" {
                let status = dispatch_builtin_omniroute(client, request, config, &subpath)?;
                return Ok(Some(status));
            }
            if desc.id == "muse" {
                return dispatch_builtin_muse(client, request, config, clean_path);
            }
            if desc.id == "jev" {
                return dispatch_builtin_jev(client, request, store, config, clean_path);
            }
            Ok(None)
        }
    }
}
