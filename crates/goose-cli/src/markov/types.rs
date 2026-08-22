//! Types shared across the seam: named in hook signatures, so they have to be
//! visible to the upstream crate even though the dialogs live in markov-cli.

use goose::agents::ExtensionConfig;

/// What an extension dialog changed, so a caller with a live agent can follow along.
#[derive(Debug, Clone)]
pub enum ExtensionChange {
    Connected(ExtensionConfig),
    Enabled(ExtensionConfig),
    Disabled(String),
    Removed(String),
}

/// What answers right now, whoever decided it.
pub struct Current {
    pub provider: String,
    pub model: String,
    /// A live session applies a pick to itself and leaves the file alone unless
    /// asked; standalone has nowhere to put a pick but the file.
    pub in_session: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Choice {
    pub provider: String,
    pub model: String,
}

impl From<&Current> for Choice {
    fn from(current: &Current) -> Self {
        Choice {
            provider: current.provider.clone(),
            model: current.model.clone(),
        }
    }
}

/// Having nothing to offer is not a failure of the prompt, and every caller has
/// its own sentence for it, so it travels back as an answer rather than an error.
pub enum SessionPick {
    Chosen(String),
    Cancelled,
    NoSessions,
}
