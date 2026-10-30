//! Auto-carved gateway test submodule.
#![allow(unused_imports)]
use super::common::*;
use super::museai_live_tests::{live_capture_config, live_http_request};

// ---------- Muse live video pipeline and scoped auto-approval tests ----------

#[test]
fn test_live_http_video_create_edit_prompt_update_pipeline() {
    if std::env::var("MUSE_LIVE_HTTP_PIPELINE").ok().as_deref() != Some("1") {
        return;
    }

    let config = live_capture_config();
    let create_body = serde_json::json!({
        "prompt": "A small orange fox walks through a quiet moonlit forest, cinematic camera movement, no text, no logos.",
        "model": "muse-video",
        "aspect_ratio": "16:9",
        "duration": 5
    })
    .to_string();
    let (status, created) = live_http_request(
        "POST",
        "/muse-ai/v1/create-video",
        &create_body,
        config.clone(),
    );
    assert_eq!(status, 200, "live create returned HTTP {status}");
    assert_eq!(created["object"], "video.generation");
    assert_eq!(created["status"], "artifact_reported");
    assert_eq!(created["provider_status"], "artifact_url_reported");
    assert_eq!(created["output_verified"], false);
    assert_eq!(created["media_validated"], false);
    let first_url = created["video_url"].as_str().unwrap();
    assert!(toolcase_gateway::museai::video::is_supported_video_url(
        first_url
    ));
    let job_id = created["id"].as_str().unwrap().to_owned();
    println!("Created video artifact: {first_url}");

    let edit_body = serde_json::json!({
        "job_id": job_id,
        "prompt": "Add warm lantern light, gentle fog, and a slow camera orbit."
    })
    .to_string();
    let (status, edited) =
        live_http_request("POST", "/muse-ai/v1/create-video", &edit_body, config);
    assert_eq!(status, 200, "live edit returned HTTP {status}");
    assert_eq!(edited["object"], "video.generation");
    assert_eq!(edited["id"], job_id);
    assert_eq!(edited["status"], "artifact_reported");
    assert_eq!(edited["revision_id"], "rev-1");
    let second_url = edited["video_url"].as_str().unwrap();
    assert!(toolcase_gateway::museai::video::is_supported_video_url(
        second_url
    ));
    assert_ne!(first_url, second_url);
    println!("Edited video artifact: {second_url}");

    let prompt_body = serde_json::json!({
        "prompt": "Store this final prompt without generating another artifact."
    })
    .to_string();
    let (status, updated) = live_http_request(
        "PUT",
        &format!("/muse-ai/v1/videos/{job_id}"),
        &prompt_body,
        live_capture_config(),
    );
    assert_eq!(status, 200, "live prompt update returned HTTP {status}");
    assert_eq!(updated["id"], job_id);
    assert_eq!(updated["status"], "updated");
    assert_eq!(
        updated["prompt"],
        "Store this final prompt without generating another artifact."
    );
    assert!(updated.get("video_url").is_none());

    let persisted = toolcase_gateway::museai::load_video_job(&job_id).unwrap();
    assert_eq!(persisted.video_url, second_url);
    assert_eq!(persisted.revisions.len(), 1);
}

#[test]
fn test_live_scoped_auto_approval_video_link() {
    let _ = dotenvy::dotenv();

    if std::env::var("MUSE_LIVE_SCOPED_AUTO_APPROVAL")
        .ok()
        .as_deref()
        != Some("1")
    {
        return;
    }
    assert_eq!(
        std::env::var("MUSE_LIVE_TEST").ok().as_deref(),
        Some("1"),
        "live generation opt-in required"
    );
    let mode = toolcase_gateway::config::parse_muse_approval_mode(
        &std::env::var("GW_MUSE_APPROVAL_MODE").unwrap_or_default(),
    );
    assert_eq!(mode, toolcase_gateway::config::MuseApprovalMode::ScopedAuto);
    let destination = std::env::var("GW_MUSE_APPROVAL_ALLOWED_DESTINATIONS").unwrap_or_default();
    assert_eq!(
        destination, "muse.ai",
        "live approval destination must be muse.ai"
    );
    let allowed_types = std::env::var("GW_MUSE_APPROVAL_ALLOWED_TYPES").unwrap_or_default();
    assert_eq!(
        allowed_types, "network",
        "live approval type must be network"
    );
    assert_ne!(
        std::env::var("GW_MUSE_APPROVAL_ALLOW_ALWAYS")
            .ok()
            .as_deref(),
        Some("true")
    );
    assert!(
        !std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/muse-live-owned-session.json")
            .exists(),
        "owned session exists; inspect original session, do not generate twice"
    );
    let mut config = live_capture_config();
    config.muse_approval.mode = mode;
    config.muse_approval.allowed_types = allowed_types
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    config.muse_approval.allowed_destinations = destination
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    config.muse_approval.allow_always = false;
    config.muse_approval.max_decisions_per_job = 3;
    config.museai_auto_cleanup_threads = false;
    let prompt = std::env::var("MUSE_LIVE_VIDEO_PROMPT")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| {
            "A single white square rotating slowly on a black background, 3D cartoon style"
                .to_string()
        });

    let idempotency_key = std::env::var("MUSE_LIVE_IDEMPOTENCY_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .unwrap_or_else(|| format!("muse-scoped-auto-live-proof-{}", uuid::Uuid::new_v4()));

    let request = serde_json::json!({
        "prompt": prompt,
        "model": "muse-video",
        "idempotency_key": idempotency_key
    });
    let (status, result) = live_http_request(
        "POST",
        "/muse-ai/v1/create-video",
        &request.to_string(),
        config,
    );
    assert_eq!(
        status, 200,
        "live video creation did not complete without 409: {result}"
    );
    assert_eq!(result["status"], "artifact_reported");
    let url = result["video_url"].as_str().unwrap_or("");
    assert!(
        toolcase_gateway::museai::video::is_supported_video_url(url),
        "no supported video artifact reported"
    );
    println!("Live scoped auto video artifact: {url}");
}

#[test]
fn test_live_create_video_from_har_capture() {
    use toolcase_gateway::museai::video::create_video;

    if std::env::var("MUSE_LIVE_TEST").ok().as_deref() != Some("1") {
        return;
    }
    assert!(!std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/muse-live-owned-session.json").exists(),
            "a task-owned session is already saved; use the read-only owned-session test instead of generating again");
    let config = live_capture_config();
    let body = serde_json::json!({
        "prompt": "A single white square rotating slowly on a black background, 3D cartoon style",
        "model": "muse-video",
        "aspect_ratio": "16:9",
        "duration": 5
    });

    match create_video(body.to_string().as_bytes(), &config) {
        Ok(result) => {
            assert_eq!(result["object"], "video.generation");
            let status = result["status"].as_str().unwrap_or("");
            assert!(
                status == "completed" || status == "artifact_reported",
                "live Muse did not return a valid video status: {status}"
            );
            let url = result["video_url"].as_str().unwrap_or("");
            assert!(url.starts_with("https://"), "video_url is not HTTPS: {url}");
            assert!(
                toolcase_gateway::museai::video::is_supported_video_url(url),
                "video_url must be a supported CDN URL: {url}"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("target/muse-video-result.json");
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(path)
                    .unwrap_or_else(|_| panic!("cannot securely create live video result file"));
                file.write_all(result.to_string().as_bytes())
                    .unwrap_or_else(|_| panic!("cannot write live video result file"));
                println!("Live video result saved to target/muse-video-result.json");
            }
            let job_id = result["id"].as_str().unwrap_or("");
            if !job_id.is_empty() {
                let loaded = toolcase_gateway::museai::load_video_job(job_id)
                    .expect("live generated video job must be persisted in lifecycle store");
                assert_eq!(loaded.job_id, job_id);
                assert_eq!(loaded.video_url, url);
                assert_eq!(loaded.status, status);
            }
            let template_html = include_str!("../../assets/video-template.html");
            assert!(template_html.contains("showVideo(d.video_url)"));
            assert!(template_html.contains("player.src=url"));
            assert!(template_html.contains("download.href=url"));
            #[cfg(not(unix))]
            panic!("secure live-result storage requires an owner-only file ACL implementation");
        }
        Err(e) => {
            panic!(
                "Live Muse request failed at API pipeline stage: {} (private details suppressed)",
                e.kind()
            );
        }
    }
}
