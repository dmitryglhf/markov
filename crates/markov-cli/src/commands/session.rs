//! The session picker and the deletion menu.
//!
//! Upstream shows one line per session and orders them however the store hands
//! them over; these read newest first, say how long ago and how much was said,
//! and let the choice be cancelled instead of failing.

use anyhow::Result;
use chrono::{DateTime, Utc};
use cliclack::{multiselect, select};
use etcetera::home_dir;
use goose::session::{Session, SessionManager, SessionType};
use goose::utils::safe_truncate;
use goose_cli::markov::types::SessionPick;
use std::io;
use std::path::Path;

const TRUNCATED_DESC_LENGTH: usize = 60;
const SESSION_PICKER_LIMIT: usize = 20;
const SESSION_PICKER_ROWS: usize = 10;

fn display_path_with_tilde(path: &Path) -> String {
    #[cfg(not(target_os = "windows"))]
    if let Ok(home) = home_dir() {
        if let Ok(stripped) = path.strip_prefix(&home) {
            return format!("~/{}", stripped.display());
        }
    }
    path.display().to_string()
}

fn session_activity_at(session: &Session) -> DateTime<Utc> {
    session.last_message_at.unwrap_or(session.updated_at)
}

pub fn prompt_interactive_session_removal(sessions: &[Session]) -> Result<Vec<Session>> {
    if sessions.is_empty() {
        println!("No sessions to delete.");
        return Ok(vec![]);
    }

    // Everything is offered here, unlike the picker: a session you cannot see is
    // a session you cannot delete.
    let choices = session_picker_entries(sessions, sessions.len(), Utc::now());

    let mut selector = multiselect(
        "Select sessions to delete (use spacebar, Enter to confirm, Ctrl+C to cancel):",
    )
    .max_rows(SESSION_PICKER_ROWS);
    for choice in &choices {
        selector = selector.item(choice.id.clone(), &choice.label, &choice.working_dir);
    }

    let selected: Vec<String> = match selector.interact() {
        Ok(selected) => selected,
        Err(e) if e.kind() == io::ErrorKind::Interrupted => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };

    Ok(select_sessions_by_id(sessions, &choices, &selected))
}

/// Keeps the order the sessions were shown in, so the confirmation list reads
/// like the menu that produced it.
fn select_sessions_by_id(
    sessions: &[Session],
    choices: &[SessionChoice],
    selected: &[String],
) -> Vec<Session> {
    choices
        .iter()
        .filter(|choice| selected.contains(&choice.id))
        .filter_map(|choice| sessions.iter().find(|s| s.id == choice.id))
        .cloned()
        .collect()
}

struct SessionChoice {
    id: String,
    label: String,
    working_dir: String,
}

/// A human-scale stamp, because an exact timestamp is noise when you are looking
/// for the session you were in an hour ago.
fn time_ago(when: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let minutes = (now - when).num_minutes();
    match minutes {
        m if m < 1 => "just now".to_string(),
        m if m < 60 => format!("{m}m ago"),
        m if m < 60 * 24 => format!("{}h ago", m / 60),
        m if m < 60 * 24 * 7 => format!("{}d ago", m / (60 * 24)),
        _ => when.format("%Y-%m-%d").to_string(),
    }
}

fn session_picker_entries(
    sessions: &[Session],
    limit: usize,
    now: DateTime<Utc>,
) -> Vec<SessionChoice> {
    let mut ordered: Vec<&Session> = sessions.iter().collect();
    ordered.sort_by_key(|s| std::cmp::Reverse(session_activity_at(s)));
    ordered.truncate(limit);

    ordered
        .into_iter()
        .map(|s| {
            let name = match s.name.trim() {
                "" => "(no name)",
                name => name,
            };
            SessionChoice {
                id: s.id.clone(),
                // The count sits in the label rather than the hint because
                // cliclack only draws the hint under the cursor, and telling a
                // session apart from a false start is what you scan the list for.
                label: format!(
                    "{} · {} msg · {}",
                    time_ago(session_activity_at(s), now),
                    s.message_count,
                    safe_truncate(name, TRUNCATED_DESC_LENGTH)
                ),
                working_dir: display_path_with_tilde(&s.working_dir),
            }
        })
        .collect()
}

/// Prompt the user to interactively select a session, newest first.
///
/// `Err` is left for input that actually broke. Passing session types narrows
/// the list to the ones worth offering for the task at hand.
pub async fn prompt_interactive_session_selection(
    session_manager: &SessionManager,
    prompt: &str,
    types: Option<&[SessionType]>,
) -> Result<SessionPick> {
    let sessions = match types {
        Some(types) => session_manager.list_sessions_by_types(types).await?,
        None => session_manager.list_sessions().await?,
    };

    let choices = session_picker_entries(&sessions, SESSION_PICKER_LIMIT, Utc::now());
    if choices.is_empty() {
        return Ok(SessionPick::NoSessions);
    }

    let mut selector = select(prompt).max_rows(SESSION_PICKER_ROWS);
    for choice in &choices {
        selector = selector.item(Some(choice.id.clone()), &choice.label, &choice.working_dir);
    }
    selector = selector.item(None, "Cancel", "");

    match selector.interact() {
        Ok(Some(id)) => Ok(SessionPick::Chosen(id)),
        Ok(None) => Ok(SessionPick::Cancelled),
        Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(SessionPick::Cancelled),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::path::PathBuf;

    fn at(minutes: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 6, 12, 0, 0).unwrap() - chrono::Duration::minutes(minutes)
    }

    fn session(id: &str, name: &str, minutes_ago: i64) -> Session {
        Session {
            id: id.to_string(),
            name: name.to_string(),
            working_dir: PathBuf::from("/work"),
            last_message_at: Some(at(minutes_ago)),
            ..Default::default()
        }
    }

    #[test]
    fn the_picker_puts_the_freshest_session_first_every_time() {
        let sessions = vec![
            session("old", "Old One", 500),
            session("fresh", "Fresh One", 2),
            session("middle", "Middle One", 90),
        ];

        let ids: Vec<String> = session_picker_entries(&sessions, 10, at(0))
            .into_iter()
            .map(|c| c.id)
            .collect();

        assert_eq!(ids, vec!["fresh", "middle", "old"]);
        let again: Vec<String> = session_picker_entries(&sessions, 10, at(0))
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, again);
    }

    #[test]
    fn the_picker_keeps_only_the_most_recent_sessions_within_the_limit() {
        let sessions: Vec<Session> = (0..30)
            .map(|i| session(&format!("s{i}"), "Session", i as i64))
            .collect();

        let choices = session_picker_entries(&sessions, 20, at(0));

        assert_eq!(choices.len(), 20);
        assert_eq!(choices[0].id, "s0");
        assert_eq!(choices[19].id, "s19");
    }

    #[test]
    fn a_session_without_a_name_still_reads_as_something() {
        let choices = session_picker_entries(&[session("s1", "   ", 5)], 10, at(0));

        assert_eq!(choices[0].label, "5m ago · 0 msg · (no name)");
        assert_eq!(choices[0].working_dir, "/work");
    }

    #[test]
    fn a_long_name_is_cut_to_the_list_width() {
        let long_name = "n".repeat(TRUNCATED_DESC_LENGTH + 20);
        let choices = session_picker_entries(&[session("s1", &long_name, 5)], 10, at(0));

        let shown = choices[0]
            .label
            .strip_prefix("5m ago · 0 msg · ")
            .expect("stamp");
        assert!(shown.chars().count() <= TRUNCATED_DESC_LENGTH);
    }

    /// The one thing that separates a session worth returning to from a false
    /// start, so it has to survive into the row itself.
    #[test]
    fn the_row_says_how_much_was_said_in_the_session() {
        let mut worked_in = session("s1", "Real work", 5);
        worked_in.message_count = 42;

        let choices = session_picker_entries(&[worked_in], 10, at(0));

        assert_eq!(choices[0].label, "5m ago · 42 msg · Real work");
    }

    #[test]
    fn the_deletion_menu_offers_every_session_newest_first() {
        let sessions: Vec<Session> = (0..30)
            .map(|i| session(&format!("s{i}"), "Session", 30 - i as i64))
            .collect();

        let choices = session_picker_entries(&sessions, sessions.len(), at(0));

        assert_eq!(choices.len(), 30);
        assert_eq!(choices[0].id, "s29");
    }

    #[test]
    fn ticked_sessions_come_back_in_the_order_they_were_shown() {
        let sessions = vec![
            session("old", "Old One", 500),
            session("fresh", "Fresh One", 2),
            session("middle", "Middle One", 90),
        ];
        let choices = session_picker_entries(&sessions, 10, at(0));

        let picked = select_sessions_by_id(
            &sessions,
            &choices,
            &["old".to_string(), "fresh".to_string(), "gone".to_string()],
        );

        let ids: Vec<&str> = picked.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["fresh", "old"]);
    }

    #[test]
    fn time_ago_speaks_in_the_largest_unit_that_fits() {
        let now = at(0);
        assert_eq!(time_ago(at(0), now), "just now");
        assert_eq!(time_ago(at(59), now), "59m ago");
        assert_eq!(time_ago(at(60), now), "1h ago");
        assert_eq!(time_ago(at(60 * 24 - 1), now), "23h ago");
        assert_eq!(time_ago(at(60 * 24), now), "1d ago");
        assert_eq!(time_ago(at(60 * 24 * 7 - 1), now), "6d ago");
        assert_eq!(time_ago(at(60 * 24 * 7), now), "2026-07-30");
    }
}
