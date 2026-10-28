//! Unified Gateway Modules architecture.
//! Encapsulates built-in provider modules, personal out-of-process modules,
//! and the module engine (registry, manifest, catalog, validation).

pub mod builtins;
pub mod catalog;
pub mod manifest;
pub mod personal;
pub mod registry;
#[cfg(test)]
mod tests;
pub mod validation;

pub use catalog::*;
pub use manifest::*;
pub use registry::*;
pub use validation::*;
