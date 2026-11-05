use toolcase_gateway::museai::video::{
    parse_and_validate_video_request, validate_attachment_string, VideoAttachment,
};

#[test]
fn test_attachment_parsing_valid_external_https_url() {
    let payload = serde_json::json!({
        "prompt": "Cinematic pan across misty mountains",
        "image_url": "https://images.unsplash.com/photo-1506744038136-46273834b3fb.jpg",
        "aspect_ratio": "16:9",
        "duration": 5
    });
    let req = parse_and_validate_video_request(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert!(req.attachment.is_some());
    assert_eq!(
        req.attachment_url.as_deref(),
        Some("https://images.unsplash.com/photo-1506744038136-46273834b3fb.jpg")
    );

    let parsed_req_json: serde_json::Value = serde_json::from_slice(&req.request_json).unwrap();
    let prompt_sent = parsed_req_json["messages"][0]["content"].as_str().unwrap();
    assert!(prompt_sent.contains("animated from this reference image (https://images.unsplash.com/photo-1506744038136-46273834b3fb.jpg)"));
}

#[test]
fn test_attachment_parsing_valid_internal_gateway_image() {
    let payload = serde_json::json!({
        "prompt": "Animate character breathing and looking up",
        "image": "/muse-ai/v1/images/img-test-12345/content",
        "aspect_ratio": "9:16",
        "duration": 10
    });
    let req = parse_and_validate_video_request(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert!(matches!(
        req.attachment,
        Some(VideoAttachment::InternalImage { .. })
    ));
    assert_eq!(
        req.attachment_url.as_deref(),
        Some("/muse-ai/v1/images/img-test-12345/content")
    );

    let parsed_req_json: serde_json::Value = serde_json::from_slice(&req.request_json).unwrap();
    let prompt_sent = parsed_req_json["messages"][0]["content"].as_str().unwrap();
    assert!(prompt_sent.contains(
        "animated from this reference image (/muse-ai/v1/images/img-test-12345/content)"
    ));
}

#[test]
fn test_attachment_parsing_valid_data_uri() {
    // 1x1 transparent PNG base64
    let valid_data_uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    let payload = serde_json::json!({
        "prompt": "Animate floating particles",
        "attachment": valid_data_uri
    });
    let req = parse_and_validate_video_request(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert!(matches!(
        req.attachment,
        Some(VideoAttachment::DataUri { .. })
    ));
    assert!(req.attachment_url.is_some());
}

#[test]
fn test_attachment_ssrf_rejection() {
    let forbidden = [
        "https://localhost/test.png",
        "https://127.0.0.1/test.png",
        "https://192.168.1.50/test.png",
        "https://10.0.0.1/test.png",
        "https://172.16.0.5/test.png",
        "https://service.internal/test.png",
        "https://cluster.local/test.png",
        "http://insecure-plain-http.com/test.png",
    ];

    for url in forbidden {
        let err = validate_attachment_string(url).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }
}

#[test]
fn test_attachment_invalid_mime_and_corrupt_payload_rejection() {
    // Unsupported MIME
    let pdf_uri = "data:application/pdf;base64,JVBERi0xLjUK";
    let err = validate_attachment_string(pdf_uri).unwrap_err();
    assert!(err.to_string().contains("unsupported image MIME type"));

    // Corrupt base64
    let corrupt_uri = "data:image/png;base64,***NotValidBase64***";
    let err = validate_attachment_string(corrupt_uri).unwrap_err();
    assert!(err.to_string().contains("invalid base64 payload"));
}

#[test]
fn test_attachment_job_record_persistence_and_response() {
    let job_id = format!("video-att-test-{}", uuid::Uuid::new_v4());
    let payload = serde_json::json!({
        "id": job_id,
        "prompt": "Animate glowing flower",
        "image_url": "https://images.unsplash.com/photo-flower.png"
    });
    let req = parse_and_validate_video_request(&serde_json::to_vec(&payload).unwrap()).unwrap();
    let init_record = toolcase_gateway::museai::video::create_init_job_record(&req);
    assert_eq!(
        init_record.attachment_url.as_deref(),
        Some("https://images.unsplash.com/photo-flower.png")
    );

    toolcase_gateway::museai::save_video_job(&init_record).unwrap();
    let loaded = toolcase_gateway::museai::load_video_job(&job_id).unwrap();
    assert_eq!(
        loaded.attachment_url.as_deref(),
        Some("https://images.unsplash.com/photo-flower.png")
    );

    // Verify response JSON includes attachment_url when completed
    let mut completed = loaded.clone();
    completed.status = "completed".to_string();
    completed.video_url = "https://muse.ai/files/flower.mp4".to_string();
    let resp = toolcase_gateway::museai::video::video_job_response(completed).unwrap();
    assert_eq!(
        resp["attachment_url"],
        "https://images.unsplash.com/photo-flower.png"
    );
}

#[test]
fn test_backward_compatibility_without_attachment() {
    let payload = serde_json::json!({
        "prompt": "Just a standard text-to-video prompt",
        "aspect_ratio": "16:9",
        "duration": 5
    });
    let req = parse_and_validate_video_request(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert!(req.attachment.is_none());
    assert!(req.attachment_url.is_none());

    let parsed_req_json: serde_json::Value = serde_json::from_slice(&req.request_json).unwrap();
    let prompt_sent = parsed_req_json["messages"][0]["content"].as_str().unwrap();
    assert!(!prompt_sent.contains("animated from this reference image"));
    assert_eq!(
        prompt_sent,
        "Create a video from this description if video generation is available: \"Just a standard text-to-video prompt\". Return a video artifact link when ready. Do not change sharing permissions."
    );
}

#[test]
fn test_frontend_html_contains_image_attachment_elements() {
    let html = include_str!("../../assets/video-template.html");
    assert!(html.contains("id=\"image-attachment\""));
    assert!(html.contains("id=\"image-file-input\""));
    assert!(html.contains("id=\"btn-clear-image\""));
    assert!(html.contains("id=\"image-preview\""));
    assert!(html.contains("Reference Images (Image-to-Video"));
    assert!(html.contains("body.image = currentList[0];"));
}

#[test]
fn test_image_studio_to_video_pipeline_end_to_end_flow() {
    let img_id = format!("img-flow-{}", uuid::Uuid::new_v4());
    let img_record = toolcase_gateway::museai::ImageJobRecord {
        job_id: img_id.clone(),
        owner_id: "default-owner".to_string(),
        model: "muse-image".to_string(),
        requested_model: "muse-image".to_string(),
        prompt: "A mystical cosmic owl on a crystal branch".to_string(),
        aspect_ratio: "1:1".to_string(),
        thread_id: None,
        created_at: 1700000000,
        last_activity_at: 1700000005,
        status: "completed".to_string(),
        media_validated: true,
        output_verified: true,
        image_url: format!("https://muse.ai/files/{img_id}.png"),
        ..Default::default()
    };
    toolcase_gateway::museai::save_image_job(&img_record).unwrap();

    let video_job_id = format!("vid-from-img-{}", uuid::Uuid::new_v4());
    let video_req_payload = serde_json::json!({
        "id": video_job_id,
        "prompt": "Animate the owl spreading its wings and blinking glowing eyes",
        "image": format!("/muse-ai/v1/images/{img_id}/content"),
        "aspect_ratio": "16:9",
        "duration": 10
    });

    let req = parse_and_validate_video_request(&serde_json::to_vec(&video_req_payload).unwrap())
        .expect("video request with image attachment must parse successfully");

    // 1. Check attachment URL is extracted and normalized
    assert_eq!(
        req.attachment_url.as_deref(),
        Some(format!("/muse-ai/v1/images/{img_id}/content").as_str())
    );

    // 2. Check compiled Muse prompt explicitly conditions the model on the image
    let req_json: serde_json::Value = serde_json::from_slice(&req.request_json).unwrap();
    let prompt_content = req_json["messages"][0]["content"].as_str().unwrap();
    assert!(prompt_content.contains(&format!(
        "animated from this reference image (/muse-ai/v1/images/{img_id}/content)"
    )));
    assert!(
        prompt_content.contains("Animate the owl spreading its wings and blinking glowing eyes")
    );

    // 3. Check initialized job record captures attachment
    let init_rec = toolcase_gateway::museai::video::create_init_job_record(&req);
    assert_eq!(
        init_rec.attachment_url.as_deref(),
        Some(format!("/muse-ai/v1/images/{img_id}/content").as_str())
    );

    // 4. Check persistence and completion response
    toolcase_gateway::museai::save_video_job(&init_rec).unwrap();
    let mut completed_rec = toolcase_gateway::museai::load_video_job(&video_job_id).unwrap();
    completed_rec.status = "completed".to_string();
    completed_rec.video_url = format!("https://muse.ai/files/{video_job_id}.mp4");

    let resp = toolcase_gateway::museai::video::video_job_response(completed_rec).unwrap();
    assert_eq!(
        resp["attachment_url"],
        format!("/muse-ai/v1/images/{img_id}/content")
    );
    assert_eq!(
        resp["video_url"],
        format!("https://muse.ai/files/{video_job_id}.mp4")
    );
}

#[test]
fn test_attachment_parsing_valid_images_array() {
    let payload = serde_json::json!({
        "prompt": "Animate rotating 3D character from three camera angles",
        "images": [
            "https://images.unsplash.com/photo-front.jpg",
            "https://images.unsplash.com/photo-side.png",
            "/muse-ai/v1/images/img-iso-1/content"
        ],
        "aspect_ratio": "16:9",
        "duration": 10
    });

    let req = parse_and_validate_video_request(&serde_json::to_vec(&payload).unwrap())
        .expect("multiple images array must parse successfully");

    assert_eq!(req.attachments.len(), 3);
    assert_eq!(req.attachment_urls.len(), 3);
    assert_eq!(
        req.attachment_urls[0],
        "https://images.unsplash.com/photo-front.jpg"
    );
    assert_eq!(
        req.attachment_urls[1],
        "https://images.unsplash.com/photo-side.png"
    );
    assert_eq!(
        req.attachment_urls[2],
        "/muse-ai/v1/images/img-iso-1/content"
    );

    let parsed_req_json: serde_json::Value = serde_json::from_slice(&req.request_json).unwrap();
    let prompt_sent = parsed_req_json["messages"][0]["content"].as_str().unwrap();
    assert!(prompt_sent.contains("animated from these reference images (https://images.unsplash.com/photo-front.jpg, https://images.unsplash.com/photo-side.png, /muse-ai/v1/images/img-iso-1/content)"));
}

#[test]
fn test_attachment_parsing_reference_images_objects_array() {
    let payload = serde_json::json!({
        "prompt": "Animate smooth morph between two characters",
        "reference_images": [
            { "url": "https://images.unsplash.com/char-a.png" },
            { "url": "https://images.unsplash.com/char-b.webp" }
        ]
    });

    let req = parse_and_validate_video_request(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert_eq!(req.attachment_urls.len(), 2);
    assert_eq!(
        req.attachment_urls[0],
        "https://images.unsplash.com/char-a.png"
    );
    assert_eq!(
        req.attachment_urls[1],
        "https://images.unsplash.com/char-b.webp"
    );
}

#[test]
fn test_attachment_list_too_many_images_rejected() {
    let payload = serde_json::json!({
        "prompt": "Animate scene",
        "images": [
            "https://images.unsplash.com/1.png",
            "https://images.unsplash.com/2.png",
            "https://images.unsplash.com/3.png",
            "https://images.unsplash.com/4.png",
            "https://images.unsplash.com/5.png",
            "https://images.unsplash.com/6.png"
        ]
    });

    let err = parse_and_validate_video_request(&serde_json::to_vec(&payload).unwrap())
        .expect_err("exceeding max 5 images must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert!(err.to_string().contains("too many image attachments"));
}

#[test]
fn test_attachment_list_fail_closed_on_any_invalid_image() {
    let payload = serde_json::json!({
        "prompt": "Animate scene",
        "images": [
            "https://images.unsplash.com/valid.png",
            "https://127.0.0.1/evil-ssrf.png"
        ]
    });

    let err = parse_and_validate_video_request(&serde_json::to_vec(&payload).unwrap())
        .expect_err("any SSRF violation in image list must fail entire request");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn test_multi_image_job_record_persistence_and_response() {
    let job_id = format!("video-multi-{}", uuid::Uuid::new_v4());
    let payload = serde_json::json!({
        "id": job_id,
        "prompt": "Animate multi-angle shot",
        "images": [
            "https://images.unsplash.com/angle1.png",
            "https://images.unsplash.com/angle2.png"
        ]
    });

    let req = parse_and_validate_video_request(&serde_json::to_vec(&payload).unwrap()).unwrap();
    let init_rec = toolcase_gateway::museai::video::create_init_job_record(&req);
    assert_eq!(init_rec.attachment_urls.len(), 2);

    toolcase_gateway::museai::save_video_job(&init_rec).unwrap();
    let mut loaded = toolcase_gateway::museai::load_video_job(&job_id).unwrap();
    assert_eq!(loaded.attachment_urls.len(), 2);
    assert_eq!(
        loaded.attachment_urls[0],
        "https://images.unsplash.com/angle1.png"
    );

    loaded.status = "completed".to_string();
    loaded.video_url = format!("https://muse.ai/files/{job_id}.mp4");
    let resp = toolcase_gateway::museai::video::video_job_response(loaded).unwrap();

    let resp_urls = resp["attachment_urls"].as_array().unwrap();
    assert_eq!(resp_urls.len(), 2);
    assert_eq!(resp_urls[0], "https://images.unsplash.com/angle1.png");
    assert_eq!(resp_urls[1], "https://images.unsplash.com/angle2.png");
}

#[test]
fn test_frontend_html_contains_multi_image_elements() {
    let html = include_str!("../../assets/video-template.html");
    assert!(html.contains("id=\"btn-add-url\""));
    assert!(html.contains("id=\"image-gallery\""));
    assert!(html.contains("multiple"));
    assert!(html.contains("body.images = currentList;"));
    assert!(html.contains("Maximum of 5 images allowed per video request."));
}

#[test]
fn test_non_ascii_truncation_does_not_panic() {
    let non_ascii_payload = "é".repeat(35);
    let res = validate_attachment_string(&non_ascii_payload);
    assert!(res.is_err());
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("invalid image attachment"));
}

#[test]
fn test_secure_url_host_evasion_rejected() {
    // 1. Userinfo / @ bypass attempts
    assert!(validate_attachment_string("https://127.0.0.1#@images.unsplash.com/a.png").is_err());
    assert!(validate_attachment_string("https://user:pass@images.unsplash.com/a.png").is_err());
    assert!(validate_attachment_string("https://admin@images.unsplash.com/a.png").is_err());

    // 2. IPv4-mapped IPv6 loopback and private addresses
    assert!(validate_attachment_string("https://[::ffff:127.0.0.1]/a.png").is_err());
    assert!(validate_attachment_string("https://[::ffff:10.0.0.1]/a.png").is_err());
    assert!(validate_attachment_string("https://[::ffff:192.168.1.1]/a.png").is_err());
}

#[test]
fn test_multipart_form_data_single_image_upload() {
    let png_bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15c4";
    let boundary = "------------------------boundaryTest123";
    let mut body = Vec::new();

    // field: prompt
    body.extend_from_slice(format!("{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"prompt\"\r\n\r\n");
    body.extend_from_slice(b"Animate the owl spreading its wings and blinking glowing eye\r\n");

    // field: aspect_ratio
    body.extend_from_slice(format!("{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"aspect_ratio\"\r\n\r\n");
    body.extend_from_slice(b"16:9\r\n");

    // field: duration
    body.extend_from_slice(format!("{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"duration\"\r\n\r\n");
    body.extend_from_slice(b"10\r\n");

    // field: image file
    body.extend_from_slice(format!("{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"image\"; filename=\"Moonlit Owl with Radiant Wings.png\"\r\n");
    body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body.extend_from_slice(png_bytes);
    body.extend_from_slice(b"\r\n");

    // end
    body.extend_from_slice(format!("{boundary}--\r\n").as_bytes());

    let req = parse_and_validate_video_request(&body).expect("multipart parsing must succeed");
    assert_eq!(
        req.prompt,
        "Animate the owl spreading its wings and blinking glowing eye"
    );
    assert_eq!(req.aspect_ratio, "16:9");
    assert_eq!(req.duration, 10);
    assert!(req.attachment.is_some());
    assert_eq!(req.attachment_urls.len(), 1);
    assert!(req.attachment_urls[0].starts_with("data:image/png;base64,"));
}

#[test]
fn test_multipart_form_data_multiple_images() {
    let png_bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15c4";
    let jpeg_bytes =
        b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x01\x00`\x00`\x00\x00\xff\xdb\x00C\x00";
    let boundary = "------------------------multiBoundary789";
    let mut body = Vec::new();

    body.extend_from_slice(format!("{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"prompt\"\r\n\r\n");
    body.extend_from_slice(b"Two angles animation\r\n");

    body.extend_from_slice(format!("{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"images\"; filename=\"angle1.png\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body.extend_from_slice(png_bytes);
    body.extend_from_slice(b"\r\n");

    body.extend_from_slice(format!("{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"images\"; filename=\"angle2.jpg\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: image/jpeg\r\n\r\n");
    body.extend_from_slice(jpeg_bytes);
    body.extend_from_slice(b"\r\n");

    body.extend_from_slice(format!("{boundary}--\r\n").as_bytes());

    let req = parse_and_validate_video_request(&body).expect("multi image multipart must succeed");
    assert_eq!(req.attachment_urls.len(), 2);
    assert!(req.attachment_urls[0].starts_with("data:image/png;base64,"));
    assert!(req.attachment_urls[1].starts_with("data:image/jpeg;base64,"));
}

#[test]
fn test_multipart_form_data_unsupported_mime_rejected() {
    let pdf_bytes = b"%PDF-1.4\n%fake";
    let boundary = "------------------------boundaryPdf";
    let mut body = Vec::new();

    body.extend_from_slice(format!("{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"prompt\"\r\n\r\n");
    body.extend_from_slice(b"Should fail on pdf\r\n");

    body.extend_from_slice(format!("{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"image\"; filename=\"doc.pdf\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: application/pdf\r\n\r\n");
    body.extend_from_slice(pdf_bytes);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("{boundary}--\r\n").as_bytes());

    let res = parse_and_validate_video_request(&body);
    assert!(res.is_err());
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("not a supported image"));
}

#[test]
fn test_multipart_curl_form_exact_user_payload() {
    let owl_image_path = "/home/ubuntu/my-video-agent/plan/Moonlit Owl with Radiant Wings.png";
    let image_bytes = std::fs::read(owl_image_path).unwrap_or_else(|_| {
        b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15c4\x00\x00\x00\nIDATx\x9cc`\x00\x00\x00\x02\x00\x01H\xaf\xa4q\x00\x00\x00\x00IEND\xaeB`\x82".to_vec()
    });

    let boundary = "------------------------userCurlBoundary999";
    let mut body = Vec::new();

    // --form 'image=@"/home/ubuntu/my-video-agent/plan/Moonlit Owl with Radiant Wings.png"'
    body.extend_from_slice(format!("{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"image\"; filename=\"Moonlit Owl with Radiant Wings.png\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body.extend_from_slice(&image_bytes);
    body.extend_from_slice(b"\r\n");

    // --form 'prompt="Animate the owl spreading its wings and blinking glowing eye"'
    body.extend_from_slice(format!("{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"prompt\"\r\n\r\n");
    body.extend_from_slice(b"Animate the owl spreading its wings and blinking glowing eye\r\n");

    // --form 'aspect_ratio="16:9"'
    body.extend_from_slice(format!("{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"aspect_ratio\"\r\n\r\n");
    body.extend_from_slice(b"16:9\r\n");

    // --form 'duration="10"'
    body.extend_from_slice(format!("{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"duration\"\r\n\r\n");
    body.extend_from_slice(b"10\r\n");

    body.extend_from_slice(format!("{boundary}--\r\n").as_bytes());

    let req = parse_and_validate_video_request(&body)
        .expect("exact user curl payload must parse successfully");
    assert_eq!(
        req.prompt,
        "Animate the owl spreading its wings and blinking glowing eye"
    );
    assert_eq!(req.aspect_ratio, "16:9");
    assert_eq!(req.duration, 10);
    assert!(req.attachment.is_some());
    assert_eq!(req.attachment_urls.len(), 1);
    assert!(req.attachment_urls[0].starts_with("data:image/png;base64,"));

    // Pipeline submission simulation
    let config = toolcase_gateway::config::Config::default();
    let res = toolcase_gateway::museai::video::submit_video_with_completion(
        &body,
        &config,
        |_req_body, _cfg, _stream| {
            Ok(
                "{\"choices\":[{\"message\":{\"content\":\"Generating video https://muse.ai/files/owl-test.mp4\"}}]}"
                    .to_string(),
            )
        },
    )
    .expect("pipeline execution must succeed for user multipart payload");

    assert!(res["id"].as_str().is_some());
    assert_eq!(res["status"].as_str(), Some("submitting"));
    assert_eq!(res["attachment_urls"][0], req.attachment_urls[0]);
}
