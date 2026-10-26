//! Muse image generation subsystem.

pub mod handlers;
pub mod lifecycle;
pub mod media;
pub mod service;
pub mod url;

pub use handlers::{
    handle_create_image, handle_create_image_request, handle_get_image, handle_get_image_content,
    handle_get_image_request,
};
pub use lifecycle::{
    ensure_image_state_dir, is_valid_job_id, load_image_job, reserve_image_job, save_image_job,
    update_image_job, ImageJobRecord,
};
pub use media::{
    find_image_media, guess_extension, guess_mime_type, has_image_media, load_image_media,
    save_image_media,
};
pub use service::{
    create_image, create_image_with_completion, image_job_response,
    parse_and_validate_image_request, submit_image, submit_image_with_completion,
    ValidatedImageRequest,
};
pub use url::{
    build_image_prompt, extract_image_url_from_text, has_supported_image_extension,
    is_raw_image_artifact_url, is_supported_image_url, map_size_to_aspect_ratio,
    normalize_image_model, DEFAULT_IMAGE_ASPECT_RATIO, SUPPORTED_IMAGE_ASPECT_RATIOS,
    SUPPORTED_IMAGE_MODELS,
};
