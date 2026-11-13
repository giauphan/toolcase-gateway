//! Muse image generation unit and integration tests.

use toolcase_gateway::config::Config;
use toolcase_gateway::museai::chat::ArtifactWait;
use toolcase_gateway::museai::image::{
    build_image_prompt, create_image_with_completion, extract_image_url_from_text, load_image_job,
    map_size_to_aspect_ratio, normalize_image_model, parse_and_validate_image_request,
    save_image_job, submit_image_with_completion, ImageJobRecord, DEFAULT_IMAGE_ASPECT_RATIO,
    SUPPORTED_IMAGE_ASPECT_RATIOS,
};
use uuid::Uuid;

#[test]
fn test_image_request_validation_empty_body() {
    let result = parse_and_validate_image_request(b"");
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn test_image_request_validation_missing_prompt() {
    let payload = serde_json::json!({
        "aspect_ratio": "16:9"
    });
    let result = parse_and_validate_image_request(&serde_json::to_vec(&payload).unwrap());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert!(err.to_string().contains("prompt is required"));
}

#[test]
fn test_image_request_validation_empty_prompt() {
    let payload = serde_json::json!({
        "prompt": "   ",
        "aspect_ratio": "16:9"
    });
    let result = parse_and_validate_image_request(&serde_json::to_vec(&payload).unwrap());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert!(err.to_string().contains("prompt is required"));
}

#[test]
fn test_image_request_validation_default_aspect_ratio() {
    let payload = serde_json::json!({
        "prompt": "a cute baby otter swimming"
    });
    let req = parse_and_validate_image_request(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert_eq!(req.prompt, "a cute baby otter swimming");
    assert_eq!(req.aspect_ratio, DEFAULT_IMAGE_ASPECT_RATIO);
    assert_eq!(req.aspect_ratio, "1:1");
}

#[test]
fn test_image_request_validation_all_supported_aspect_ratios() {
    for ratio in SUPPORTED_IMAGE_ASPECT_RATIOS {
        let payload = serde_json::json!({
            "prompt": "a sunset over the ocean",
            "aspect_ratio": ratio
        });
        let req = parse_and_validate_image_request(&serde_json::to_vec(&payload).unwrap()).unwrap();
        assert_eq!(req.aspect_ratio, ratio);
    }
}

#[test]
fn test_image_request_validation_unsupported_aspect_ratio() {
    let payload = serde_json::json!({
        "prompt": "a sunset",
        "aspect_ratio": "21:9"
    });
    let result = parse_and_validate_image_request(&serde_json::to_vec(&payload).unwrap());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert!(err.to_string().contains("unsupported aspect_ratio '21:9'"));
    assert!(err.to_string().contains("1:1, 16:9, 9:16"));
}

#[test]
fn test_image_request_openai_size_mapping() {
    let cases = vec![
        ("1024x1024", "1:1"),
        ("512x512", "1:1"),
        ("1792x1024", "16:9"),
        ("1024x1792", "9:16"),
        ("1024x768", "4:3"),
        ("768x1024", "3:4"),
        ("1200x800", "3:2"),
        ("800x1200", "2:3"),
    ];

    for (size, expected_ratio) in cases {
        let payload = serde_json::json!({
            "prompt": "portrait of a painter",
            "size": size
        });
        let req = parse_and_validate_image_request(&serde_json::to_vec(&payload).unwrap()).unwrap();
        assert_eq!(req.aspect_ratio, expected_ratio);
        assert_eq!(map_size_to_aspect_ratio(size), Some(expected_ratio));
    }
}

#[test]
fn test_image_request_explicit_aspect_ratio_precedence_over_size() {
    let payload = serde_json::json!({
        "prompt": "portrait",
        "aspect_ratio": "16:9",
        "size": "1024x1024"
    });
    let req = parse_and_validate_image_request(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert_eq!(req.aspect_ratio, "16:9");
}

#[test]
fn test_image_request_custom_id_and_idempotency_key() {
    let custom_id = format!("img-custom-{}", Uuid::new_v4());
    let payload = serde_json::json!({
        "id": custom_id,
        "prompt": "futuristic city skyline",
        "submission_key": "sub-12345"
    });
    let req = parse_and_validate_image_request(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert_eq!(req.allocated_job_id, custom_id);
    assert_eq!(req.submission_key.as_deref(), Some("sub-12345"));
}

#[test]
fn test_build_image_prompt() {
    let prompt = build_image_prompt("muse-image", "a mystical forest path", "16:9");
    assert!(prompt.contains("a mystical forest path"));
    assert!(prompt.contains("aspect ratio 16:9"));
    assert!(prompt.contains("image generation is available"));
    assert!(prompt.contains("image artifact link"));
}

#[test]
fn test_normalize_image_model() {
    assert_eq!(normalize_image_model("muse"), "muse-image");
    assert_eq!(normalize_image_model("muse-image"), "muse-image");
    assert_eq!(normalize_image_model("dall-e-3"), "muse-image");
    assert_eq!(normalize_image_model("flux"), "muse-image");
    assert_eq!(normalize_image_model("custom-model"), "custom-model");
}

#[test]
fn test_extract_image_url_from_text() {
    let markdown = "Here is your generated image: ![A mountain](https://images.unsplash.com/photo-12345.jpg?auto=format)";
    assert_eq!(
        extract_image_url_from_text(markdown),
        "https://images.unsplash.com/photo-12345.jpg?auto=format"
    );

    let muse_files = "Your artwork is ready: https://muse.ai/files/100/200/art.png";
    assert_eq!(
        extract_image_url_from_text(muse_files),
        "https://muse.ai/files/100/200/art.png"
    );

    let gdrive = "Check Google Drive https://drive.google.com/file/d/1A2B3C/view";
    assert_eq!(
        extract_image_url_from_text(gdrive),
        "https://drive.google.com/file/d/1A2B3C/view"
    );

    let video_only = "Video link: https://cdn.muse.ai/video/clip.mp4";
    assert_eq!(extract_image_url_from_text(video_only), "");

    let no_links = "Sorry, unable to produce an image at this time.";
    assert_eq!(extract_image_url_from_text(no_links), "");
}

#[test]
fn test_create_image_synchronous_success() {
    let config = Config::default();
    let unique_id = format!("image-sync-{}", Uuid::new_v4());
    let payload = serde_json::json!({
        "id": unique_id,
        "prompt": "a cyberpunk street market at night",
        "aspect_ratio": "16:9"
    });

    let mock_completion =
        |_body: &[u8], _cfg: &Config, _wait: ArtifactWait| -> std::io::Result<String> {
            Ok("Generated image: https://muse.ai/files/111/222/cyberpunk.png".to_string())
        };

    let result = create_image_with_completion(
        &serde_json::to_vec(&payload).unwrap(),
        &config,
        mock_completion,
    )
    .unwrap();

    assert_eq!(result["id"], unique_id);
    assert_eq!(result["object"], "image.generation");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["aspect_ratio"], "16:9");
    assert_eq!(result["requested_aspect_ratio"], "16:9");
    assert_eq!(
        result["image_url"],
        "https://muse.ai/files/111/222/cyberpunk.png"
    );

    let data_arr = result["data"].as_array().unwrap();
    assert_eq!(data_arr.len(), 1);
    assert_eq!(
        data_arr[0]["url"],
        "https://muse.ai/files/111/222/cyberpunk.png"
    );

    let saved = load_image_job(&unique_id).unwrap();
    assert_eq!(saved.status, "completed");
    assert_eq!(
        saved.image_url,
        "https://muse.ai/files/111/222/cyberpunk.png"
    );
    assert_eq!(saved.aspect_ratio, "16:9");
}

#[test]
fn test_create_image_synchronous_missing_artifact_fails() {
    let config = Config::default();
    let unique_id = format!("image-fail-{}", Uuid::new_v4());
    let payload = serde_json::json!({
        "id": unique_id,
        "prompt": "an impossible concept",
        "aspect_ratio": "1:1"
    });

    let mock_completion =
        |_body: &[u8], _cfg: &Config, _wait: ArtifactWait| -> std::io::Result<String> {
            Ok("I cannot create this image right now.".to_string())
        };

    let result = create_image_with_completion(
        &serde_json::to_vec(&payload).unwrap(),
        &config,
        mock_completion,
    );

    assert!(result.is_err());
    let error = result.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
    assert!(
        error.to_string().contains("declined"),
        "refusals must be reported as declines, got: {error}"
    );

    let saved = load_image_job(&unique_id).unwrap();
    assert_eq!(saved.status, "failed");
    assert_eq!(
        saved.error.as_deref(),
        Some("Muse image generation was declined by Muse")
    );
}

#[test]
fn test_submit_image_asynchronous_success() {
    let config = Config::default();
    let unique_id = format!("image-async-{}", Uuid::new_v4());
    let payload = serde_json::json!({
        "id": unique_id,
        "prompt": "watercolor painting of lavender fields",
        "aspect_ratio": "4:3"
    });

    let mock_completion =
        |_body: &[u8], _cfg: &Config, _wait: ArtifactWait| -> std::io::Result<String> {
            std::thread::sleep(std::time::Duration::from_millis(50));
            Ok("Finished: https://muse.ai/files/333/444/lavender.jpg".to_string())
        };

    let result = submit_image_with_completion(
        &serde_json::to_vec(&payload).unwrap(),
        &config,
        mock_completion,
    )
    .unwrap();

    assert_eq!(result["id"], unique_id);
    assert_eq!(result["status"], "submitting");
    assert_eq!(result["aspect_ratio"], "4:3");

    // Wait for background worker to complete
    let mut completed = false;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        if let Ok(job) = load_image_job(&unique_id) {
            if job.status == "completed" {
                completed = true;
                assert_eq!(job.image_url, "https://muse.ai/files/333/444/lavender.jpg");
                break;
            }
        }
    }
    assert!(
        completed,
        "Background worker did not complete the job in time"
    );
}

#[test]
fn test_image_job_persistence_roundtrip() {
    let job_id = format!("img-roundtrip-{}", Uuid::new_v4());
    let record = ImageJobRecord {
        job_id: job_id.clone(),
        owner_id: "test-user".to_string(),
        model: "muse-image".to_string(),
        requested_model: "dall-e-3".to_string(),
        prompt: "test persistence prompt".to_string(),
        aspect_ratio: "16:9".to_string(),
        thread_id: Some("thread-test-1".to_string()),
        created_at: 1700000000,
        last_activity_at: 1700000005,
        status: "completed".to_string(),
        media_validated: true,
        output_verified: true,
        image_url: "https://example.com/image.png".to_string(),
        submission_key: Some("idem-key-99".to_string()),
        error: None,
    };

    save_image_job(&record).unwrap();
    let loaded = load_image_job(&job_id).unwrap();

    assert_eq!(loaded.job_id, job_id);
    assert_eq!(loaded.owner_id, "test-user");
    assert_eq!(loaded.model, "muse-image");
    assert_eq!(loaded.requested_model, "dall-e-3");
    assert_eq!(loaded.prompt, "test persistence prompt");
    assert_eq!(loaded.aspect_ratio, "16:9");
    assert_eq!(loaded.thread_id.as_deref(), Some("thread-test-1"));
    assert_eq!(loaded.status, "completed");
    assert_eq!(loaded.image_url, "https://example.com/image.png");
    assert_eq!(loaded.submission_key.as_deref(), Some("idem-key-99"));
}

#[test]
fn test_image_request_validation_client_id_security() {
    // Valid alphanumeric and dash/underscore
    let valid_payload = serde_json::json!({
        "prompt": "sunset",
        "id": "job_123-abc"
    });
    let req =
        parse_and_validate_image_request(&serde_json::to_vec(&valid_payload).unwrap()).unwrap();
    assert_eq!(req.allocated_job_id, "job_123-abc");

    // Invalid traversal or character attempts
    let invalid_ids = vec![
        "../traversal",
        "/absolute/path",
        "has space",
        "has$symbol",
        "",
    ];
    for bad_id in invalid_ids {
        let payload = serde_json::json!({
            "prompt": "sunset",
            "id": bad_id
        });
        let res = parse_and_validate_image_request(&serde_json::to_vec(&payload).unwrap());
        assert!(res.is_err(), "Expected id '{bad_id}' to be rejected");
    }
}

#[test]
fn test_image_request_validation_submission_key_limit() {
    let too_long_key = "a".repeat(257);
    let payload = serde_json::json!({
        "prompt": "sunset",
        "submission_key": too_long_key
    });
    let res = parse_and_validate_image_request(&serde_json::to_vec(&payload).unwrap());
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .to_string()
        .contains("exceeds maximum length"));
}

#[test]
fn test_image_media_path_traversal_hardening() {
    use toolcase_gateway::museai::image::{find_image_media, load_image_media, save_image_media};

    assert!(find_image_media("../../bad_job").is_none());
    assert!(load_image_media("../../bad_job").is_err());
    assert!(save_image_media("../../bad_job", b"test", "png").is_err());
}

#[test]
fn test_extract_image_url_from_plain_markdown_link_with_sandbox_artifact() {
    let completion = "Image generation is available, and I've generated your 9:16 image and verified it matches the brief — an iPad mini tablet on a futuristic minimalist desk.\n\n[Ipad Mini Futuristic Minimalist Desk](sandbox://workspace/your_files/ipad-mini-futuristic-minimalist-desk/ipad-mini-futuristic-minimalist-desk.webp)\n\nSpec: 1152x2048, 9:16.";
    assert_eq!(
        extract_image_url_from_text(completion),
        "sandbox://workspace/your_files/ipad-mini-futuristic-minimalist-desk/ipad-mini-futuristic-minimalist-desk.webp"
    );
}

#[test]
fn test_extract_image_url_from_bare_sandbox_artifact_and_markdown_image() {
    let bare = "Here is your image: sandbox://workspace/your_files/owl/owl.webp";
    assert_eq!(
        extract_image_url_from_text(bare),
        "sandbox://workspace/your_files/owl/owl.webp"
    );

    let markdown_image = "![Owl](sandbox://workspace/your_files/owl/owl.webp)";
    assert_eq!(
        extract_image_url_from_text(markdown_image),
        "sandbox://workspace/your_files/owl/owl.webp"
    );
}

#[test]
fn test_extract_image_url_from_markdown_link_to_public_https_artifact() {
    let completion =
        "Finished. [Final render](https://muse.ai/files/777/888/render.webp \"Final render\")";
    assert_eq!(
        extract_image_url_from_text(completion),
        "https://muse.ai/files/777/888/render.webp"
    );
}

#[test]
fn test_extract_image_url_ignores_markdown_links_to_non_image_targets() {
    let completion = "Read more in the [guide](https://muse.ai/help/article) or [video](https://muse.ai/files/1/2/clip.mp4).";
    assert_eq!(extract_image_url_from_text(completion), "");
}

#[test]
fn test_explicit_image_refusal_detection() {
    use toolcase_gateway::museai::image::explicit_image_refusal;

    assert!(explicit_image_refusal(
        "I cannot create this image right now."
    ));
    assert!(explicit_image_refusal(
        "Image generation is unavailable for this request."
    ));
    assert!(!explicit_image_refusal(
        "Image generation is available — I'll create your 9:16 data center image now."
    ));
    assert!(!explicit_image_refusal(
        "Image generation is available, and I'm packaging your 9:16 image as an artifact now. I'll send the artifact link here once it's ready."
    ));
}

#[test]
fn test_create_image_succeeds_with_sandbox_markdown_link_artifact() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let config = Config::default();
    let unique_id = format!("image-mdlink-{}", Uuid::new_v4());
    let payload = serde_json::json!({
        "id": unique_id,
        "prompt": "an ipad mini on a minimalist desk",
        "aspect_ratio": "9:16"
    });

    let requested_image_wait = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&requested_image_wait);
    let mock_completion =
        move |_body: &[u8], _cfg: &Config, wait: ArtifactWait| -> std::io::Result<String> {
            flag.store(wait == ArtifactWait::Image, Ordering::SeqCst);
            Ok("Here it is.\n\n[Desk](sandbox://workspace/your_files/ipad/ipad.webp)".to_string())
        };

    let result = create_image_with_completion(
        &serde_json::to_vec(&payload).unwrap(),
        &config,
        mock_completion,
    )
    .unwrap();

    assert!(
        requested_image_wait.load(Ordering::SeqCst),
        "image generation must request a deferred image artifact wait"
    );
    assert_eq!(result["status"], "completed");
    assert_eq!(
        result["image_url"],
        format!("/muse-ai/v1/images/{unique_id}/content")
    );
    assert_eq!(
        result["data"][0]["url"],
        format!("/muse-ai/v1/images/{unique_id}/content")
    );

    let saved = load_image_job(&unique_id).unwrap();
    assert_eq!(saved.status, "completed");
    assert_eq!(
        saved.image_url,
        format!("/muse-ai/v1/images/{unique_id}/content")
    );
}

#[test]
fn test_submit_image_completes_from_sandbox_markdown_link() {
    let config = Config::default();
    let unique_id = format!("image-mdlink-async-{}", Uuid::new_v4());
    let payload = serde_json::json!({
        "id": unique_id,
        "prompt": "a lighthouse at dusk",
        "aspect_ratio": "1:1"
    });

    let mock_completion = |_body: &[u8],
                           _cfg: &Config,
                           _wait: ArtifactWait|
     -> std::io::Result<String> {
        std::thread::sleep(std::time::Duration::from_millis(50));
        Ok("[Lighthouse](sandbox://workspace/your_files/lighthouse/lighthouse.webp)".to_string())
    };

    let result = submit_image_with_completion(
        &serde_json::to_vec(&payload).unwrap(),
        &config,
        mock_completion,
    )
    .unwrap();
    assert_eq!(result["status"], "submitting");

    let mut completed = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        if let Ok(job) = load_image_job(&unique_id) {
            if job.status == "completed" {
                completed = true;
                assert_eq!(
                    job.image_url,
                    format!("/muse-ai/v1/images/{unique_id}/content")
                );
                break;
            }
        }
    }
    assert!(
        completed,
        "Background worker did not complete the job from a sandbox markdown link"
    );
}

#[test]
fn test_create_image_recovers_from_real_completion_json_with_sandbox_link() {
    let config = Config::default();
    let unique_id = format!("image-real-json-{}", Uuid::new_v4());
    let payload = serde_json::json!({
        "id": unique_id,
        "prompt": "an iPad mini on a futuristic minimalist desk",
        "aspect_ratio": "9:16"
    });

    let completion = serde_json::json!({
        "choices": [{
            "finish_reason": "stop",
            "index": 0,
            "message": {
                "content": "Image generation is available, and I've generated your 9:16 image and verified it matches the brief — an iPad mini tablet on a futuristic minimalist desk, with glowing blue circuit light and dramatic studio lighting.\n\n[Ipad Mini Futuristic Minimalist Desk](sandbox://workspace/your_files/ipad-mini-futuristic-minimalist-desk/ipad-mini-futuristic-minimalist-desk.webp)\n\nSpec: 1152x2048, 9:16.",
                "role": "assistant"
            }
        }],
        "created": 1793253231,
        "id": "chatcmpl-72428dff-484a-49df-bf00-f2207707ab26",
        "model": "muse",
        "object": "chat.completion",
        "reply_thread_id": "194e1db0-1c3c-4986-9d92-ea4fd7f03d63",
        "session_id": "194e1db0-1c3c-4986-9d92-ea4fd7f03d63",
        "thread_id": "194e1db0-1c3c-4986-9d92-ea4fd7f03d63"
    })
    .to_string();

    let mock_completion = move |_body: &[u8],
                                _cfg: &Config,
                                _wait: ArtifactWait|
          -> std::io::Result<String> { Ok(completion.clone()) };

    let result = create_image_with_completion(
        &serde_json::to_vec(&payload).unwrap(),
        &config,
        mock_completion,
    )
    .expect("image generation must accept the recorded Muse completion shape");

    assert_eq!(result["status"], "completed");
    assert_eq!(
        result["image_url"],
        format!("/muse-ai/v1/images/{unique_id}/content")
    );
    assert_eq!(result["error"], serde_json::Value::Null);
}

#[test]
fn test_extract_image_url_prefers_markdown_image_over_plain_link() {
    let completion = "[Thumbnail](https://muse.ai/files/1/2/thumb.jpg)\n\n![Final render](sandbox://workspace/your_files/final/final.webp)";
    assert_eq!(
        extract_image_url_from_text(completion),
        "sandbox://workspace/your_files/final/final.webp"
    );
}

#[test]
fn test_extract_image_url_skips_non_image_workspace_markdown_links() {
    let completion =
        "[Source](sandbox://workspace/source.txt)\n\n[Image](sandbox://workspace/render.webp)";
    assert_eq!(
        extract_image_url_from_text(completion),
        "sandbox://workspace/render.webp"
    );

    let only_non_image = "[Source](sandbox://workspace/source.txt)";
    assert_eq!(extract_image_url_from_text(only_non_image), "");
}
