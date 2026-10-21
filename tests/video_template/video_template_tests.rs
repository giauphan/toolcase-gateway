//! Integration tests for HTML template asset serving.

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
