//! Integration tests for HTML template asset serving and video display lifecycle.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use toolcase_gateway::config::{Config, ConfigStore};
use toolcase_gateway::http::{header_value, read_more, read_response_head};
use toolcase_gateway::routes::route_request;

fn test_config() -> Config {
    Config {
        target_host: "127.0.0.1".into(),
        target_port: 8080,
        museai_base_url: "https://muse.ai".into(),
        ..Config::default()
    }
}

fn request_route_raw(
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    config: Config,
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let store = ConfigStore::new(config, std::path::PathBuf::from(".env"));

    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        route_request(client, &store).unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut header_lines = String::new();
    for (k, v) in headers {
        header_lines.push_str(&format!("{k}: {v}\r\n"));
    }
    write!(
        client,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\n{header_lines}Connection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    if !body.is_empty() {
        client.write_all(body).unwrap();
    }
    client.flush().unwrap();

    let head = read_response_head(&mut client).unwrap();
    let mut response_body = head.buffered_body;
    let length =
        header_value(&head.headers, "content-length").and_then(|val| val.parse::<usize>().ok());
    if let Some(length) = length {
        while response_body.len() < length {
            read_more(&mut client, &mut response_body).unwrap();
        }
        response_body.truncate(length);
    } else {
        let _ = client.read_to_end(&mut response_body);
    }
    handle.join().unwrap();
    (head.status, head.headers, response_body)
}

fn request_route_json(
    method: &str,
    path: &str,
    body: &str,
    config: Config,
) -> (u16, serde_json::Value) {
    let (status, _headers, raw_body) = request_route_raw(
        method,
        path,
        &[("Content-Type", "application/json")],
        body.as_bytes(),
        config,
    );
    let val: serde_json::Value = serde_json::from_slice(&raw_body).unwrap_or_else(|e| {
        panic!(
            "Failed to parse JSON response: {e}. Raw: {}",
            String::from_utf8_lossy(&raw_body)
        )
    });
    (status, val)
}

#[test]
fn test_html_includes_form_and_fields() {
    let html = include_str!("../../assets/video-template.html");
    assert!(html.contains("name=\"topic\""));
    assert!(html.contains("name=\"character\""));
    assert!(html.contains("name=\"setting\""));
    assert!(html.contains("name=\"action\""));
    assert!(html.contains("name=\"camera\""));
    assert!(html.contains("name=\"sound\""));
    assert!(html.contains("/muse-ai/v1/create-video"));
}

#[test]
fn muse_config_fetches_masked_current_config() {
    let html = include_str!("../../assets/muse-config.html");
    assert!(html.contains("fetch(\"/muse-ai/v1/config\""));
    assert!(html.contains("Loading current config"));
}

#[test]
fn muse_config_explains_masking_and_safe_copying() {
    let html = include_str!("../../assets/muse-config.html");
    assert!(html.contains("GW_MUSEAI_ACCESS_TOKEN"));
    assert!(html.contains("GW_MUSEAI_COOKIE"));
    assert!(html.contains("do not copy the masked text"));
    assert!(html.contains("No manual copy is needed"));
    assert!(html.contains("if(!f.masked)"));
}

#[test]
fn video_template_labels_legacy_muse_as_experimental() {
    let html = include_str!("../../assets/video-template.html");
    assert!(html.contains("Experimental legacy Muse test harness"));
    assert!(html.contains("Legacy experimental"));
    assert!(html.contains("OpenMontage owns creative profiles"));
}

#[test]
fn video_template_previews_exact_submitted_settings_and_prompt() {
    let html = include_str!("../../assets/video-template.html");
    assert!(html.contains("id=\"settings-preview\""));
    assert!(html.contains("This exact compiled prompt and requested settings are submitted"));
    assert!(html.contains("prompt: buildPrompt()"));
    assert!(html.contains("model: state.model"));
    assert!(html.contains("aspect_ratio: state.aspect"));
    assert!(html.contains("duration: duration()"));
}

#[test]
fn video_template_surfaces_unverified_media_boundary() {
    let html = include_str!("../../assets/video-template.html");
    assert!(html.contains("Returned artifact URLs are unverified"));
    assert!(html.contains("Artifact URL returned; media not validated."));
    assert!(html.contains("d.media_validated===true"));
}

#[test]
fn video_template_reads_selected_duration_before_submission() {
    let html = include_str!("../../assets/video-template.html");
    assert!(html.contains(
        "function duration(){return Number.parseInt(document.getElementById(\"duration\").value,10)||10}"
    ));
}

#[test]
fn video_template_renders_returned_artifact_as_playable_downloadable_media() {
    let html = include_str!("../../assets/video-template.html");
    assert!(html.contains("<video id=\"video-player\" controls"));
    assert!(html.contains("<a id=\"video-download\" href=\"\" download"));
    assert!(html.contains("player.src=url"));
    assert!(html.contains("download.href=url"));
    assert!(html.contains("showVideo(d.video_url)"));
    assert!(html.contains("video-player"));
    assert!(html.contains("video-download"));
}

#[test]
fn video_template_tracks_processing_state_and_polls_job_lifecycle() {
    let html = include_str!("../../assets/video-template.html");
    assert!(html.contains("pollVideo"));
    assert!(html.contains("/muse-ai/v1/videos/"));
    assert!(html.contains("Video is processing…"));
    assert!(html.contains("d.status===\"submitting\""));
    assert!(html.contains("d.status===\"generating\""));
    assert!(html.contains("d.status===\"queued\""));
    assert!(html.contains("pollVideo(d.id || id,go)"));
}

#[test]
fn video_template_surfaces_actionable_error_on_failure() {
    let html = include_str!("../../assets/video-template.html");
    assert!(html.contains("d && d.error && d.error.message ? d.error.message"));
    assert!(html.contains("Video generation did not finish."));
    assert!(html.contains("Video generation requires approval."));
}

#[test]
fn test_local_video_template_route_serves_html_with_player_and_controls() {
    let (status, headers, body) =
        request_route_raw("GET", "/video-template", &[], &[], test_config());
    assert_eq!(status, 200);
    assert_eq!(
        header_value(&headers, "content-type"),
        Some("text/html; charset=utf-8")
    );

    let html = String::from_utf8(body).expect("served template HTML must be valid UTF-8");
    assert!(html.contains("<video id=\"video-player\" controls"));
    assert!(html.contains("<a id=\"video-download\" href=\"\" download"));
    assert!(html.contains("id=\"status\""));
    assert!(html.contains("id=\"video-url\""));
    assert!(html.contains("<button id=\"go\" type=\"button\">Generate video</button>"));
    assert!(html.contains("function showVideo(url)"));
    assert!(html.contains("function hideVideo()"));
    assert!(html.contains("function showResult(d)"));
    assert!(html.contains("function pollVideo(id, go)"));
}

#[test]
fn test_local_video_generation_returns_meta_cdn_url_and_activates_player() {
    let meta_cdn_url = "https://video.xx.fbcdn.net/v/t39.25447-2/99999999_123456789012345_n.mp4?_nc_cat=105&ccb=1-7&_nc_sid=555555&efg=eyJ2ZW5jb2RlX3RhZyI6Inhwdl9wcm9ncmVzc2l2ZS5GQkNPTlRFTlRfU1RSQVRFR1kuRkJfTk9STUFMX0hEMSJ9&_nc_ohc=abcde12345";
    let mock_completion_text = format!("Generation finished! View your video at: {meta_cdn_url}");

    let prompt = "Cinematic slow motion shot of an eagle soaring above snowy mountains";
    let req_payload = serde_json::json!({
        "prompt": prompt,
        "model": "muse-video",
        "aspect_ratio": "16:9",
        "duration": 10
    });

    let completion_resp = serde_json::json!({
        "choices": [{
            "message": {
                "content": mock_completion_text
            }
        }]
    })
    .to_string();

    let result = toolcase_gateway::museai::video::create_video_with_completion(
        &serde_json::to_vec(&req_payload).unwrap(),
        &test_config(),
        move |_request, _config, _wait_for_artifact| Ok(completion_resp.clone()),
    )
    .expect("video generation must succeed with valid artifact URL");

    let job_id = result["id"]
        .as_str()
        .expect("job id must be returned")
        .to_string();
    assert_eq!(result["status"], "artifact_reported");
    assert_eq!(result["video_url"], meta_cdn_url);

    // Retrieve via local HTTP route dispatch
    let (get_status, get_body) = request_route_json(
        "GET",
        &format!("/muse-ai/v1/videos/{job_id}"),
        "",
        test_config(),
    );
    assert_eq!(get_status, 200);
    assert_eq!(get_body["id"], job_id);
    assert_eq!(get_body["status"], "artifact_reported");
    assert_eq!(get_body["video_url"], meta_cdn_url);
    assert_eq!(get_body["media_validated"], false);

    // Verify contract with frontend video-template.html
    let html = include_str!("../../assets/video-template.html");
    // 1. showResult checks status and video_url
    assert!(html.contains(
        "d && (d.status===\"completed\" || d.status===\"artifact_reported\") && d.video_url"
    ));
    // 2. showResult calls showVideo with the extracted url
    assert!(html.contains("showVideo(d.video_url)"));
    // 3. showVideo binds player.src and download.href
    assert!(html.contains("player.src=url"));
    assert!(html.contains("download.href=url"));
    // 4. showVideo makes player and download visible
    assert!(html.contains("player.style.display=\"block\""));
    assert!(html.contains("download.style.display=\"inline-block\""));
    // 5. Unvalidated artifact status label
    assert!(html.contains("Artifact URL returned; media not validated."));
}

#[test]
fn test_local_video_generation_returns_muse_cdn_url_and_marks_completed_when_validated() {
    let muse_cdn_url = "https://muse.ai/files/timelapse-waves-4k.mp4";
    let mock_completion_text = format!("Your video render is ready: {muse_cdn_url}");

    let prompt = "Futuristic neon city cyberpunk flythrough";
    let req_payload = serde_json::json!({
        "prompt": prompt,
        "model": "muse-video",
        "aspect_ratio": "9:16",
        "duration": 5
    });

    let completion_resp = serde_json::json!({
        "choices": [{
            "message": {
                "content": mock_completion_text
            }
        }]
    })
    .to_string();

    let result = toolcase_gateway::museai::video::create_video_with_completion(
        &serde_json::to_vec(&req_payload).unwrap(),
        &test_config(),
        move |_request, _config, _wait_for_artifact| Ok(completion_resp.clone()),
    )
    .expect("video generation must succeed with muse.ai CDN URL");

    let job_id = result["id"]
        .as_str()
        .expect("job id must be returned")
        .to_string();
    assert_eq!(result["status"], "artifact_reported");
    assert_eq!(result["video_url"], muse_cdn_url);

    // Simulate completion with media validation verified
    let mut record =
        toolcase_gateway::museai::load_video_job(&job_id).expect("persisted record must exist");
    record.media_validated = true;
    record.output_verified = true;
    record.status = "completed".to_string();
    toolcase_gateway::museai::save_video_job(&record).expect("save updated record");

    let (get_status, get_body) = request_route_json(
        "GET",
        &format!("/muse-ai/v1/videos/{job_id}"),
        "",
        test_config(),
    );
    assert_eq!(get_status, 200);
    assert_eq!(get_body["id"], job_id);
    assert_eq!(get_body["status"], "completed");
    assert_eq!(get_body["media_validated"], true);
    assert_eq!(get_body["video_url"], muse_cdn_url);

    // Verify template logic for verified media: sets status to "Done." with class "ok"
    let html = include_str!("../../assets/video-template.html");
    assert!(html.contains("var verified=d.media_validated===true;"));
    assert!(html.contains("setStatus(verified?\"Done.\":\"Artifact URL returned; media not validated.\",verified?\"ok\":\"busy\");"));
}

#[test]
fn test_local_video_create_route_submission_and_polling_flow() {
    let job_id = format!("video-poll-test-{}", uuid::Uuid::new_v4());
    let req_payload = serde_json::json!({
        "id": job_id,
        "prompt": "Macro time-lapse blooming orchid flower",
        "model": "muse-video",
        "aspect_ratio": "1:1",
        "duration": 5
    })
    .to_string();

    let reported_url = "https://video.xx.fbcdn.net/v/t39.25447-2/blooming_orchid.mp4";
    let reported_clone = reported_url.to_string();

    let resp = toolcase_gateway::museai::video::submit_video_with_completion(
        req_payload.as_bytes(),
        &test_config(),
        move |_request, _config, _wait_for_artifact| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            Ok(serde_json::json!({
                "choices": [{"message": {"content": format!("Render ready: {reported_clone}")}}]
            })
            .to_string())
        },
    )
    .expect("submit_video_with_completion must succeed");
    assert_eq!(resp["id"], job_id);
    assert_eq!(resp["status"], "submitting");

    let (get_status, poll_1) = request_route_json(
        "GET",
        &format!("/muse-ai/v1/videos/{job_id}"),
        "",
        test_config(),
    );
    assert_eq!(get_status, 200);
    assert!(poll_1["status"] == "submitting" || poll_1["status"] == "artifact_reported");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut final_poll = poll_1;
    while std::time::Instant::now() < deadline {
        let (_, poll) = request_route_json(
            "GET",
            &format!("/muse-ai/v1/videos/{job_id}"),
            "",
            test_config(),
        );
        if poll["status"] == "artifact_reported" {
            final_poll = poll;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(final_poll["status"], "artifact_reported");
    assert_eq!(final_poll["video_url"], reported_url);

    let html = include_str!("../../assets/video-template.html");
    assert!(html.contains("if(showResult(d)){"));
    assert!(html.contains("go.disabled=false;"));
    assert!(html.contains("stopPolling();"));
}

#[test]
fn test_local_video_generation_refusal_shows_actionable_error_and_keeps_player_hidden() {
    let refusal_message =
        "I cannot generate videos with protected intellectual property or harmful themes.";
    let completion_resp = serde_json::json!({
        "choices": [{
            "message": {
                "content": refusal_message
            }
        }]
    })
    .to_string();

    let job_id = format!("video-refusal-test-{}", uuid::Uuid::new_v4());
    let req_payload = serde_json::json!({
        "id": job_id,
        "prompt": "prohibited prompt content",
        "model": "muse-video",
        "aspect_ratio": "16:9",
        "duration": 5
    });

    let err = toolcase_gateway::museai::video::create_video_with_completion(
        &serde_json::to_vec(&req_payload).unwrap(),
        &test_config(),
        move |_request, _config, _wait_for_artifact| Ok(completion_resp.clone()),
    )
    .expect_err("generation with refusal must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);

    // Check failed job status persisted
    let (get_status, get_body) = request_route_json(
        "GET",
        &format!("/muse-ai/v1/videos/{job_id}"),
        "",
        test_config(),
    );
    assert_eq!(get_status, 200);
    assert_eq!(get_body["status"], "failed");
    assert!(get_body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Muse video generation was declined"));

    // Template UI contract: surfaces error message, does not show video player
    let html = include_str!("../../assets/video-template.html");
    assert!(html.contains("if(d.status===\"failed\" || d.status===\"approval_required\" || d.status===\"submission_unknown\"){"));
    assert!(html.contains("setError(msg)"));
    assert!(html.contains("hideVideo();"));
}
