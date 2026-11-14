//! Types shared by the Muse artifact consumers.

/// Which deferred artifact the caller wants the executor to keep reading for
/// after Muse completes the assistant message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactWait {
    None,
    Video,
    Image,
}
