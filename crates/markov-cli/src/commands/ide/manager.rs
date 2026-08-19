use super::paths::{BaseDirs, Os};
use super::targets::{Target, TargetId, TargetSelector};
use super::{
    Change, Configured, SetupOptions, apply, blocking_sandbox, ensure_extension, handle_ide_status,
    inspect, remove, resolve_command, vscode,
};
use anyhow::Result;
use console::style;
use goose_cli::markov::ui::{cancellable, multiselect, pad_to_display_width};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

const ELSEWHERE: &str = "Copy it in by hand — markov ide setup --print,  Esc to leave";

pub fn ide_dialog(name: &str) -> Result<()> {
    // The other managers refuse without a terminal. Here the status table is
    // already written and is exactly what a pipe or a CI job wants.
    if !std::io::stdin().is_terminal() {
        return handle_ide_status(name);
    }

    let dirs = BaseDirs::detect()?;
    let os = Os::current();
    let binary = dirs.cli_binary(os);

    cliclack::intro(style(" markov-ide ").on_cyan().black())?;

    let rows = survey(&dirs, os, name, &binary);
    cliclack::log::info(summary(&rows, &binary))?;
    cliclack::log::info(ELSEWHERE)?;

    // Leaving by Escape is not the same as saving a list that happened to be
    // unchanged, and the two must not print the same sentence.
    let Some(selected) = choose(&rows)? else {
        cliclack::outro("Left without saving")?;
        return Ok(());
    };

    let tally = apply_selection(&rows, &selected, &dirs, os, name)?;
    cliclack::outro(verdict(&tally))?;

    Ok(())
}

struct Row {
    target: &'static Target,
    path: PathBuf,
    connected: bool,
    blocked: Option<PathBuf>,
    annotation: String,
}

fn survey(dirs: &BaseDirs, os: Os, name: &str, binary: &Path) -> Vec<Row> {
    let command = binary.to_string_lossy();
    let extension_missing = !vscode::extension_installed(dirs);

    TargetSelector::All
        .expand()
        .into_iter()
        .map(|target| {
            let path = target.config_path(dirs, os);
            let state = inspect(target, &path, name, &command);
            let blocked = blocking_sandbox(target, dirs, os);

            Row {
                connected: state != Configured::Missing,
                annotation: annotation(
                    &state,
                    target.is_installed(dirs, os),
                    blocked.is_some(),
                    target.id == TargetId::Vscode && extension_missing,
                ),
                blocked,
                path,
                target,
            }
        })
        .collect()
}

/// The one thing worth saying about a row beyond its checkbox. Only the first
/// that applies is shown: a file we cannot read makes everything else moot.
pub fn annotation(
    state: &Configured,
    installed: bool,
    blocked: bool,
    extension_missing: bool,
) -> String {
    if matches!(state, Configured::Unreadable(_)) {
        return "settings file is not valid JSON".to_string();
    }
    if blocked {
        return "flatpak — cannot be connected".to_string();
    }
    if !installed {
        return "not installed".to_string();
    }
    if let Configured::Other(command) = state {
        return format!("points at {command}");
    }
    if extension_missing {
        return "extension will be installed".to_string();
    }
    String::new()
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    Connect,
    Disconnect,
    Blocked,
    Leave,
}

pub fn decide(wanted: bool, connected: bool, blocked: bool) -> Decision {
    match (wanted, blocked, connected) {
        (true, true, _) => Decision::Blocked,
        (true, false, _) => Decision::Connect,
        (false, _, true) => Decision::Disconnect,
        (false, _, false) => Decision::Leave,
    }
}

fn summary(rows: &[Row], binary: &Path) -> String {
    let connected = rows.iter().filter(|row| row.connected).count();
    let mut line = format!(
        "cli: {} · {connected} of {} connected",
        binary.display(),
        rows.len()
    );
    if !binary.is_file() {
        line.push_str(" · binary not found");
    }
    line
}

/// cliclack draws the hint only under the cursor, so what tells one row from
/// another lives in the label and the settings path becomes the hint.
fn choose(rows: &[Row]) -> Result<Option<Vec<TargetId>>> {
    let width = rows
        .iter()
        .map(|row| row.target.label.chars().count())
        .max()
        .unwrap_or(0);

    let items: Vec<(TargetId, String, String)> = rows
        .iter()
        .map(|row| {
            (
                row.target.id,
                label(row, width),
                row.path.display().to_string(),
            )
        })
        .collect();

    let connected: Vec<TargetId> = rows
        .iter()
        .filter(|row| row.connected)
        .map(|row| row.target.id)
        .collect();

    cancellable(
        multiselect(
            "Which IDEs should markov be available in? (space to toggle, enter to save)",
            &items,
        )
        .initial_values(connected)
        .required(false)
        .interact(),
    )
}

fn label(row: &Row, width: usize) -> String {
    match row.annotation.is_empty() {
        true => row.target.label.to_string(),
        false => format!(
            "{} {}",
            pad_to_display_width(row.target.label, width),
            row.annotation
        ),
    }
}

fn apply_selection(
    rows: &[Row],
    selected: &[TargetId],
    dirs: &BaseDirs,
    os: Os,
    name: &str,
) -> Result<Tally> {
    // Someone who came to untick boxes should not be stopped by a binary they
    // no longer have, so it is resolved only when something is being connected.
    let connecting = rows
        .iter()
        .any(|row| row.blocked.is_none() && selected.contains(&row.target.id));

    let command = match connecting {
        true => match resolve_command(dirs, os, None) {
            Ok(path) => Some(path.to_string_lossy().to_string()),
            Err(err) => {
                cliclack::log::error(format!("{err}"))?;
                None
            }
        },
        false => None,
    };

    let options = SetupOptions {
        name: name.to_string(),
        command: None,
        print: false,
        dry_run: false,
        vsix: None,
    };

    let mut tally = Tally::default();

    for row in rows {
        let wanted = selected.contains(&row.target.id);

        match decide(wanted, row.connected, row.blocked.is_some()) {
            Decision::Blocked => {
                let sandboxed = row.blocked.as_ref().expect("a blocked row has a path");
                cliclack::log::warning(format!(
                    "{} is a flatpak ({}) and may not be allowed to run the binary",
                    row.target.label,
                    sandboxed.display()
                ))?;
                tally.skipped += 1;
            }

            Decision::Connect => {
                let Some(command) = command.as_deref() else {
                    tally.skipped += 1;
                    continue;
                };
                match apply(row.target, &row.path, &options, command) {
                    Ok(change) => {
                        if change != Change::Unchanged {
                            tally.changes += 1;
                        }
                        if row.target.id == TargetId::Vscode {
                            report_extension(ensure_extension(dirs, os, None))?;
                        }
                    }
                    Err(err) => {
                        log_failure(row.target, err)?;
                        tally.skipped += 1;
                    }
                }
            }

            Decision::Disconnect => match remove(row.target, &row.path, name) {
                Ok(true) => tally.changes += 1,
                Ok(false) => {}
                Err(err) => {
                    log_failure(row.target, err)?;
                    tally.skipped += 1;
                }
            },

            Decision::Leave => {}
        }
    }

    Ok(tally)
}

#[derive(Default)]
pub struct Tally {
    pub changes: usize,
    pub skipped: usize,
}

/// "Saved, nothing was different" is a lie once something has failed, so the
/// count of what was left behind has to reach the last line.
pub fn verdict(tally: &Tally) -> String {
    match (tally.changes, tally.skipped) {
        (0, 0) => "Saved, nothing was different".to_string(),
        (0, skipped) => format!("{skipped} left unchanged"),
        (changes, 0) => format!("{changes} change(s) saved"),
        (changes, skipped) => format!("{changes} change(s) saved, {skipped} left unchanged"),
    }
}

/// One target failing is not a reason to tear the form down, and the message is
/// shown here rather than returned so it is not printed twice.
fn log_failure(target: &Target, err: anyhow::Error) -> Result<()> {
    cliclack::log::error(format!("{}: {err}", target.label))?;
    Ok(())
}

fn report_extension(outcome: super::ExtensionOutcome) -> Result<()> {
    let Some(message) = outcome.message() else {
        return Ok(());
    };
    match outcome.is_problem() {
        true => cliclack::log::warning(message)?,
        false => cliclack::log::success(message)?,
    }
    Ok(())
}
