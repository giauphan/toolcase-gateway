//! Thin backward-compatibility facade forwarding to canonical provider at `crate::providers::jev`.

pub use crate::providers::jev::accounts;
pub use crate::providers::jev::adapter;
pub use crate::providers::jev::business;
pub use crate::providers::jev::openrouter;

pub mod pool {
    pub use crate::providers::jev::accounts::*;
}

pub mod handlers {
    pub use crate::providers::jev::adapter::*;
}

pub use crate::providers::jev::accounts::{JevAccount, JevAccountPool, OpenRouterAccount};
pub use crate::providers::jev::adapter::{
    expected_jev_method, handle_jev_ai, handle_jev_models_catalog, map_jev_upstream_path,
};
pub use crate::providers::jev::openrouter::{
    prepare_openrouter_payload, send_openrouter_decision_request,
};
