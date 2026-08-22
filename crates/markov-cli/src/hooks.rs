//! Our side of the seam: what the upstream REPL reaches for when it needs a
//! dialog that lives here.

use anyhow::Result;
use goose::session::{Session, SessionManager, SessionType};
use goose_cli::markov::hooks::MarkovHooks;
use goose_cli::markov::types::{Choice, Current, ExtensionChange, SessionPick};

pub struct Markov;

#[async_trait::async_trait(?Send)]
impl MarkovHooks for Markov {
    async fn mcp_dialog(&self) -> Result<Vec<ExtensionChange>> {
        crate::commands::mcp_manager::mcp_dialog().await
    }

    fn extensions_dialog(&self) -> Result<Vec<ExtensionChange>> {
        crate::commands::extensions::extensions_dialog()
    }

    async fn models_dialog(&self, current: Current) -> Result<Option<Choice>> {
        crate::commands::models::models_dialog(current).await
    }

    async fn handle_model_command(&self) -> Result<()> {
        crate::commands::models::handle_model_command().await
    }

    async fn skills_dialog(&self) -> Result<()> {
        crate::commands::skills::skills_dialog().await
    }

    fn session_removal_picker(&self, sessions: &[Session]) -> Result<Vec<Session>> {
        crate::commands::session::prompt_interactive_session_removal(sessions)
    }

    async fn session_picker(
        &self,
        session_manager: &SessionManager,
        prompt: &str,
        types: Option<&[SessionType]>,
    ) -> Result<SessionPick> {
        crate::commands::session::prompt_interactive_session_selection(
            session_manager,
            prompt,
            types,
        )
        .await
    }
}

pub static MARKOV: Markov = Markov;
