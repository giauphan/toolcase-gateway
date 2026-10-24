//! HAR configuration management facade.
//!
//! Decomposed into cohesive submodules under [`crate::features::har`]:
//! - `types`: Domain entities and reports (`MuseHarConfig`, `HarExtractError`, `ApplyReport`, etc.)
//! - `parser`: HAR entry extraction and URL sanitization
//! - `storage`: Safe disk persistence for managed environment variables
//! - `handler`: HTTP route endpoints (`apply_har_config`, `write_current_config`)

pub use crate::features::har::*;
