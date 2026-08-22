//! Resuming by name, and the decision to offer a list instead.
//!
//! Both sides need these: the command lives in markov-cli, while the choice of
//! whether to offer a picker is taken deep inside the upstream session handler.

use crate::cli::Identifier;
use std::io::IsTerminal;

/// `markov resume <session>` takes one word for both doors, because the lookup
/// behind it already matches a session by name or by id.
pub fn resume_identifier(session: Option<String>) -> Option<Identifier> {
    session.map(|name| Identifier {
        name: Some(name),
        session_id: None,
        path: None,
    })
}

/// A bare `--resume` offers a choice of sessions. Anything not on a terminal
/// keeps the old behaviour of taking the most recent one, so scripts stay scripts.
pub fn resume_selection_needed(resume: bool, identifier: &Option<Identifier>) -> bool {
    resume
        && identifier.is_none()
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One word, both doors: the lookup behind `name` matches an id too, and
    /// routing it as an id instead would lose everything named.
    #[test]
    fn a_named_session_arrives_as_a_name_and_nothing_else() {
        let identifier = resume_identifier(Some("project-x".to_string())).expect("identifier");

        assert_eq!(identifier.name.as_deref(), Some("project-x"));
        assert!(identifier.session_id.is_none());
        assert!(identifier.path.is_none());
        assert!(resume_identifier(None).is_none());
    }

    /// Naming a session is what there is to resume by; without it there is
    /// nothing to look up, which is what sends the command to the picker.
    #[test]
    fn nothing_named_means_nothing_to_resume_by() {
        assert!(resume_identifier(None).is_none());
    }
}
