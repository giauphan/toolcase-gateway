//! Versioned media provider transport feature.
//! Implements strict capability routing and unverified provider rejection per Shared Contract v1.

pub mod service;
pub mod types;

pub use service::{
    handle_video_providers_catalog, handle_video_task_reconciliation, handle_video_task_status,
    handle_video_task_submit,
};
