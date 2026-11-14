//! Shared media artifact kernel for the Muse image and video pipelines.
//!
//! Image and video both deal with the same artifact sources (public CDNs, the
//! Muse file store, the CVM workspace, and `sandbox://` links). Keeping those
//! rules here stops the two bounded contexts from depending on each other.

pub mod types;
pub mod url;

pub use types::ArtifactWait;
pub use url::{
    clean_url_path, extract_candidate_urls, extract_vm_media_path, is_google_drive_file_path,
    is_google_drive_host, is_trusted_media_cdn_host, is_vm_media_host, is_vm_media_url,
    trim_trailing_punct, url_authority,
};
