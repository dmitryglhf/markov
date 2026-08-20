//! The seam between the upstream CLI and markov-cli.
//!
//! Crate cycles are forbidden, so the upstream crate cannot call into markov-cli
//! directly. It calls through this trait instead; markov-cli installs the
//! implementation before dispatching. Every method has a default, so adding a
//! hook never touches an upstream file twice.

use super::types::{Choice, Current, ExtensionChange, SessionPick};
use anyhow::Result;
use goose::session::{Session, SessionManager, SessionType};
use std::sync::OnceLock;

// cliclack's prompt handles are not Send and live across awaits inside the
// dialogs, so the boxed futures cannot carry a Send bound.
#[async_trait::async_trait(?Send)]
pub trait MarkovHooks: Send + Sync {
    async fn mcp_dialog(&self) -> Result<Vec<ExtensionChange>> {
        Ok(Vec::new())
    }

    fn extensions_dialog(&self) -> Result<Vec<ExtensionChange>> {
        Ok(Vec::new())
    }

    async fn models_dialog(&self, _current: Current) -> Result<Option<Choice>> {
        Ok(None)
    }

    async fn handle_model_command(&self) -> Result<()> {
        Ok(())
    }

    async fn skills_dialog(&self) -> Result<()> {
        Ok(())
    }

    /// Defaults delegate to the upstream dialogs, which stay where they are.
    fn session_removal_picker(&self, sessions: &[Session]) -> Result<Vec<Session>> {
        crate::commands::session::prompt_interactive_session_removal(sessions)
    }

    async fn session_picker(
        &self,
        session_manager: &SessionManager,
        _prompt: &str,
        _types: Option<&[SessionType]>,
    ) -> Result<SessionPick> {
        crate::commands::session::prompt_interactive_session_selection(session_manager)
            .await
            .map(SessionPick::Chosen)
    }
}

/// Stands in when nothing was installed, so a vanilla build of this crate runs
/// without markov-cli present.
struct Absent;
impl MarkovHooks for Absent {}

static HOOKS: OnceLock<&'static dyn MarkovHooks> = OnceLock::new();

pub fn install(hooks: &'static dyn MarkovHooks) {
    let _ = HOOKS.set(hooks);
}

pub fn hooks() -> &'static dyn MarkovHooks {
    static ABSENT: Absent = Absent;
    *HOOKS
        .get()
        .unwrap_or(&(&ABSENT as &'static dyn MarkovHooks))
}
