use crate::session::user_projected_message_to_markdown;
use anyhow::{Context, Result};

use chrono::{DateTime, Utc};
use cliclack::{confirm, multiselect, select};
use etcetera::home_dir;
#[cfg(feature = "nostr")]
use goose::config::Config;
#[cfg(feature = "nostr")]
use goose::session::nostr_share;
use goose::session::{
    generate_diagnostics, DiagnosticsLevel, Session, SessionManager, SessionType,
};
use goose::utils::safe_truncate;
use regex::Regex;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::path::PathBuf;

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

async fn remove_sessions(session_manager: &SessionManager, sessions: Vec<Session>) -> Result<()> {
    println!("The following sessions will be removed:");
    for session in &sessions {
        println!("- {} {}", session.id, session.name);
    }

    let should_delete = confirm("Are you sure you want to delete these sessions?")
        .initial_value(false)
        .interact()?;

    if should_delete {
        for session in sessions {
            session_manager.delete_session(&session.id).await?;
            println!("Session `{}` removed.", session.id);
        }
    } else {
        println!("Skipping deletion of the sessions.");
    }

    Ok(())
}

fn prompt_interactive_session_removal(sessions: &[Session]) -> Result<Vec<Session>> {
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

pub async fn handle_session_remove(
    session_id: Option<String>,
    name: Option<String>,
    regex_string: Option<String>,
) -> Result<()> {
    let session_manager = SessionManager::instance();

    let matched_sessions: Vec<Session>;

    if let Some(id_val) = session_id {
        match session_manager.get_session(&id_val, false).await {
            Ok(session) => matched_sessions = vec![session],
            Err(_) => return Err(anyhow::anyhow!("Session ID '{}' not found.", id_val)),
        }
    } else if let Some(name_val) = name {
        let all_sessions = session_manager.list_all_sessions().await?;
        if let Some(session) = all_sessions.into_iter().find(|s| s.name == name_val) {
            matched_sessions = vec![session];
        } else {
            return Err(anyhow::anyhow!(
                "Session with name '{}' not found.",
                name_val
            ));
        }
    } else if let Some(regex_val) = regex_string {
        let session_regex = Regex::new(&regex_val)
            .with_context(|| format!("Invalid regex pattern '{}'", regex_val))?;

        let visible_sessions = session_manager.list_sessions().await?;
        matched_sessions = visible_sessions
            .into_iter()
            .filter(|session| session_regex.is_match(&session.id))
            .collect();

        if matched_sessions.is_empty() {
            println!("Regex string '{}' does not match any sessions", regex_val);
            return Ok(());
        }
    } else {
        let visible_sessions = session_manager.list_sessions().await?;
        if visible_sessions.is_empty() {
            return Err(anyhow::anyhow!("No sessions found."));
        }
        matched_sessions = prompt_interactive_session_removal(&visible_sessions)?;
    }

    if matched_sessions.is_empty() {
        return Ok(());
    }

    remove_sessions(&session_manager, matched_sessions).await
}

fn write_line_or_broken_pipe_ok<W: Write>(out: &mut W, line: &str) -> Result<bool> {
    match writeln!(out, "{line}") {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn session_activity_at(session: &Session) -> chrono::DateTime<chrono::Utc> {
    session.last_message_at.unwrap_or(session.updated_at)
}

pub async fn handle_session_list(
    format: String,
    ascending: bool,
    working_dir: Option<PathBuf>,
    limit: Option<usize>,
) -> Result<()> {
    let session_manager = SessionManager::instance();
    let mut sessions = session_manager.list_sessions().await?;

    if let Some(ref pat) = working_dir {
        let pat_lower = pat.to_string_lossy().to_lowercase();
        sessions.retain(|s| {
            s.working_dir
                .to_string_lossy()
                .to_lowercase()
                .contains(&pat_lower)
        });
    }

    if ascending {
        sessions.sort_by_key(session_activity_at);
    } else {
        sessions.sort_by_key(|b| std::cmp::Reverse(session_activity_at(b)));
    }

    if let Some(n) = limit {
        sessions.truncate(n);
    }

    let stdout = io::stdout();
    let mut out = stdout.lock();

    match format.as_str() {
        "json" => {
            let payload = serde_json::to_string(&sessions)?;
            if !write_line_or_broken_pipe_ok(&mut out, &payload)? {
                return Ok(());
            }
        }
        _ => {
            if sessions.is_empty() {
                if !write_line_or_broken_pipe_ok(&mut out, "No sessions found")? {
                    return Ok(());
                }
                return Ok(());
            }

            if !write_line_or_broken_pipe_ok(&mut out, "Available sessions:")? {
                return Ok(());
            }

            for session in sessions {
                let output = format!(
                    "{} - {} - {} - {}",
                    session.id,
                    session.name,
                    session_activity_at(&session),
                    display_path_with_tilde(&session.working_dir)
                );
                if !write_line_or_broken_pipe_ok(&mut out, &output)? {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

pub async fn handle_session_export(
    session_id: String,
    output_path: Option<PathBuf>,
    format: String,
    nostr: bool,
    #[cfg_attr(not(feature = "nostr"), allow(unused_variables))] relays: Vec<String>,
) -> Result<()> {
    let session_manager = SessionManager::instance();
    let session = match session_manager.get_session(&session_id, true).await {
        Ok(session) => session,
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Session '{}' not found or failed to read: {}",
                session_id,
                e
            ));
        }
    };

    let output = match format.as_str() {
        "json" => serde_json::to_string_pretty(&session)?,
        "yaml" => serde_yaml::to_string(&session)?,
        "markdown" => {
            let conversation = session
                .conversation
                .ok_or_else(|| anyhow::anyhow!("Session has no messages"))?;
            export_session_to_markdown(conversation.user_visible_messages(), &session.name)
        }
        _ => return Err(anyhow::anyhow!("Unsupported format: {}", format)),
    };

    #[cfg(feature = "nostr")]
    if nostr {
        if format != "json" {
            return Err(anyhow::anyhow!(
                "Nostr session sharing only supports --format json"
            ));
        }
        if output_path.is_some() {
            return Err(anyhow::anyhow!(
                "Nostr session sharing cannot be combined with --output"
            ));
        }

        let relays = nostr_share::resolve_relays(relays, Config::global());
        let share = nostr_share::publish_session_json(&output, relays).await?;
        println!("Session published to Nostr relays:");
        for relay in &share.relays {
            println!("- {}", relay);
        }
        println!("\nShare link:");
        println!("{}", share.deeplink);
        return Ok(());
    }
    #[cfg(not(feature = "nostr"))]
    if nostr {
        return Err(anyhow::anyhow!("goose was not built with nostr support"));
    }

    if let Some(output_path) = output_path {
        fs::write(&output_path, output).with_context(|| {
            format!("Failed to write to output file: {}", output_path.display())
        })?;
        println!("Session exported to {}", output_path.display());
    } else {
        println!("{}", output);
    }

    Ok(())
}

pub async fn handle_session_import(input: String, nostr: bool) -> Result<()> {
    let json = if nostr || input.starts_with("goose://sessions/nostr") {
        #[cfg(feature = "nostr")]
        {
            nostr_share::import_session_json_from_deeplink(&input).await?
        }
        #[cfg(not(feature = "nostr"))]
        return Err(anyhow::anyhow!("goose was not built with nostr support"));
    } else {
        fs::read_to_string(&input)
            .with_context(|| format!("Failed to read session import file: {input}"))?
    };

    let format = goose::session::import_formats::detect_format(&json);
    let label = match format {
        goose::session::import_formats::ImportFormat::Goose => "goose",
        goose::session::import_formats::ImportFormat::ClaudeCode => "Claude Code",
        goose::session::import_formats::ImportFormat::Codex => "Codex",
        goose::session::import_formats::ImportFormat::Pi => "Pi",
    };
    println!("Detected format: {}", label);

    let session_manager = SessionManager::instance();
    let session = session_manager
        .import_session(&json, Some(SessionType::User))
        .await?;

    println!("Session imported:");
    println!("{} - {}", session.id, session.name);

    Ok(())
}

pub async fn handle_diagnostics(session_id: &str, output_path: Option<PathBuf>) -> Result<()> {
    println!(
        "Generating diagnostics report for session '{}'...",
        session_id
    );

    let session_manager = SessionManager::instance();
    let diagnostics_report =
        generate_diagnostics(&session_manager, session_id, DiagnosticsLevel::Full)
            .await
            .with_context(|| {
                format!(
                    "Failed to generate diagnostics report for session '{}'",
                    session_id
                )
            })?;
    let diagnostics_data = serde_json::to_vec_pretty(&diagnostics_report)
        .context("Failed to serialize diagnostics report")?;

    let output_file = if let Some(path) = output_path {
        path.clone()
    } else {
        PathBuf::from(format!("diagnostics_{}.json", session_id))
    };

    let mut file = fs::File::create(&output_file).context(format!(
        "Failed to create output file: {}",
        output_file.display()
    ))?;

    file.write_all(&diagnostics_data)
        .context("Failed to write diagnostics data")?;

    println!("Diagnostics report saved to: {}", output_file.display());

    Ok(())
}

fn export_session_to_markdown(
    messages: Vec<goose::conversation::message::Message>,
    session_name: &String,
) -> String {
    let mut markdown_output = String::new();

    markdown_output.push_str(&format!("# Session Export: {}\n\n", session_name));

    if messages.is_empty() {
        markdown_output.push_str("*(This session has no messages)*\n");
        return markdown_output;
    }

    markdown_output.push_str(&format!("*Total messages: {}*\n\n---\n\n", messages.len()));

    // Track if the last message had tool requests to properly handle tool responses
    let mut skip_next_if_tool_response = false;

    for message in &messages {
        // Check if this is a User message containing only ToolResponses
        let is_only_tool_response = message.role == rmcp::model::Role::User
            && message.content.iter().all(|content| {
                matches!(
                    content,
                    goose::conversation::message::MessageContent::ToolResponse(_)
                )
            });

        // If the previous message had tool requests and this one is just tool responses,
        // don't create a new User section - we'll attach the responses to the tool calls
        if skip_next_if_tool_response && is_only_tool_response {
            // Export the tool responses without a User heading
            markdown_output.push_str(&user_projected_message_to_markdown(message));
            markdown_output.push_str("\n\n---\n\n");
            skip_next_if_tool_response = false;
            continue;
        }

        // Reset the skip flag - we'll update it below if needed
        skip_next_if_tool_response = false;

        // Output the role prefix except for tool response-only messages
        if !is_only_tool_response {
            let role_prefix = match message.role {
                rmcp::model::Role::User => "### User:\n",
                rmcp::model::Role::Assistant => "### Assistant:\n",
            };
            markdown_output.push_str(role_prefix);
        }

        // Add the message content
        markdown_output.push_str(&user_projected_message_to_markdown(message));
        markdown_output.push_str("\n\n---\n\n");

        // Check if this message has any tool requests, to handle the next message differently
        if message.content.iter().any(|content| {
            matches!(
                content,
                goose::conversation::message::MessageContent::ToolRequest(_)
            )
        }) {
            skip_next_if_tool_response = true;
        }
    }

    markdown_output
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
                label: format!(
                    "{} · {}",
                    time_ago(session_activity_at(s), now),
                    safe_truncate(name, TRUNCATED_DESC_LENGTH)
                ),
                working_dir: display_path_with_tilde(&s.working_dir),
            }
        })
        .collect()
}

/// Prompt the user to interactively select a session, newest first.
///
/// `Ok(None)` means the user backed out. Passing session types narrows the list
/// to the ones worth offering for the task at hand.
pub async fn prompt_interactive_session_selection(
    session_manager: &SessionManager,
    prompt: &str,
    types: Option<&[SessionType]>,
) -> Result<Option<String>> {
    let sessions = match types {
        Some(types) => session_manager.list_sessions_by_types(types).await?,
        None => session_manager.list_sessions().await?,
    };

    let choices = session_picker_entries(&sessions, SESSION_PICKER_LIMIT, Utc::now());
    if choices.is_empty() {
        return Err(anyhow::anyhow!("No sessions found"));
    }

    let mut selector = select(prompt).max_rows(SESSION_PICKER_ROWS);
    for choice in &choices {
        selector = selector.item(Some(choice.id.clone()), &choice.label, &choice.working_dir);
    }
    selector = selector.item(None, "Cancel", "");

    match selector.interact() {
        Ok(selected) => Ok(selected),
        Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use goose::conversation::message::Message;
    use goose::conversation::Conversation;
    use rmcp::model::{Annotations, ContentBlock, Role, TextContent};

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

        assert_eq!(choices[0].label, "5m ago · (no name)");
        assert_eq!(choices[0].working_dir, "/work");
    }

    #[test]
    fn a_long_name_is_cut_to_the_list_width() {
        let long_name = "n".repeat(TRUNCATED_DESC_LENGTH + 20);
        let choices = session_picker_entries(&[session("s1", &long_name, 5)], 10, at(0));

        let shown = choices[0].label.strip_prefix("5m ago · ").expect("stamp");
        assert!(shown.chars().count() <= TRUNCATED_DESC_LENGTH);
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

    #[test]
    fn markdown_export_preserves_user_audience_tool_output() {
        let user_output = ContentBlock::Text(
            TextContent::new("user-visible output")
                .with_annotations(Annotations::default().with_audience(vec![Role::User])),
        );
        let assistant_output = ContentBlock::Text(
            TextContent::new("assistant-only output")
                .with_annotations(Annotations::default().with_audience(vec![Role::Assistant])),
        );
        let conversation = Conversation::new_unvalidated([Message::user().with_tool_response(
            "tool-1",
            Ok(rmcp::model::CallToolResult::success(vec![
                user_output,
                assistant_output,
                ContentBlock::text("shared output"),
            ])),
        )]);

        let markdown = export_session_to_markdown(
            conversation.user_visible_messages(),
            &"Audience export".to_string(),
        );

        assert!(markdown.contains("user-visible output"));
        assert!(markdown.contains("shared output"));
        assert!(!markdown.contains("assistant-only output"));
    }
}
