//! Gateway platform module definitions, manifests, and registry.

pub mod catalog;
pub mod manifest;
pub mod registry;
#[cfg(test)]
mod tests;
pub mod validation;

pub use catalog::*;
pub use manifest::*;
pub use registry::*;
pub use validation::*;
