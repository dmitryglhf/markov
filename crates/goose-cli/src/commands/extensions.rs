//! Every extension in one list, whatever kind it is.
//!
//! The specialised managers each own a slice of `extensions:` and say nothing
//! about the rest, so most of what a fresh install carries had no screen at all
//! and no way back off once `/builtin` had switched it on. This form shows the
//! whole list and does the one thing that means the same for every kind: turns
//! entries on and off.
//!
//! There is no menu in front of the list. The single operation is the screen,
//! and what a row is gets written on the row rather than hidden behind a card —
//! a menu whose only real item is the one you came for is a keystroke charged
//! for nothing.
//!
//! Adding, editing and credentials do *not* mean the same thing for every kind —
//! a server has an address and secrets, a builtin has neither and its set is
//! fixed in the binary — so they stay with the manager that understands them.
//! The header names that command instead of opening it: a list that can reach
//! into another list eventually has to grow everything that list has.

use anyhow::Result;
use console::style;
use goose::agents::extension_manager::is_hidden_extension;
use goose::agents::ExtensionConfig;
use goose::config::extensions::{get_all_extensions, set_extension_enabled};
use goose::config::ExtensionEntry;

use super::ui::{
    cancellable, multiselect, pad_to_display_width, require_terminal, terminal_width,
    truncate_to_display_width,
};

const ELSEWHERE: &str = "Add or edit a server — /mcp,  skills — /skills,  Esc to leave";

/// Enough for the kind and a name, leaving the rest of the line to say what the
/// thing is. Beyond this the description would be shaved to nothing, so a long
/// name is allowed to push instead.
const NAME_BUDGET: usize = 24;

/// Room for the widest word `ExtensionKind::label` can return.
const KIND_WIDTH: usize = 7;

/// What the dialog changed, so a caller with a live agent can follow along.
#[derive(Debug, Clone)]
pub enum ExtensionChange {
    Connected(ExtensionConfig),
    Enabled(ExtensionConfig),
    Disabled(String),
    Removed(String),
}

pub fn extensions_dialog() -> Result<Vec<ExtensionChange>> {
    require_terminal("markov extensions")?;

    cliclack::intro(style(" markov-extensions ").on_cyan().black())?;

    let entries = visible();
    if entries.is_empty() {
        cliclack::outro("Nothing configured yet")?;
        return Ok(Vec::new());
    }

    cliclack::log::info(summary(&entries))?;
    cliclack::log::info(ELSEWHERE)?;

    // Leaving and saving an unchanged list used to print the same sentence, so
    // a toggle that failed to register looked exactly like a deliberate exit.
    let Some(changes) = toggle(&entries)? else {
        cliclack::outro("Left without saving")?;
        return Ok(Vec::new());
    };

    match changes.is_empty() {
        true => cliclack::outro("Saved, nothing was different")?,
        false => cliclack::outro(format!("{} change(s) saved", style(changes.len()).green()))?,
    }

    Ok(changes)
}

pub fn handle_extensions_command() -> Result<()> {
    extensions_dialog().map(|_| ())
}

/// Hidden extensions are left out of the list the agent may enable, out of what
/// ACP reports and out of the catalogue of things to add. They reach this list
/// only because the migration seeds every platform definition into the config
/// file without asking whether it is meant to be seen.
fn visible() -> Vec<ExtensionEntry> {
    let mut entries: Vec<ExtensionEntry> = get_all_extensions()
        .into_iter()
        .filter(|entry| !is_hidden_extension(&entry.config.name()))
        .collect();
    entries.sort_by_key(|entry| {
        (
            entry.config.kind().display_rank(),
            entry.config.name().to_lowercase(),
        )
    });
    entries
}

/// `None` means the form was left by Escape, which is not the same as saving a
/// list that happened to be unchanged.
fn toggle(entries: &[ExtensionEntry]) -> Result<Option<Vec<ExtensionChange>>> {
    let items = items(entries);
    let enabled: Vec<String> = entries
        .iter()
        .filter(|entry| entry.enabled)
        .map(|entry| entry.config.key())
        .collect();

    let Some(selected) = cancellable(
        multiselect(
            "Which extensions should be on? (space to toggle, enter to save)",
            &items,
        )
        .initial_values(enabled)
        .required(false)
        .interact(),
    )?
    else {
        return Ok(None);
    };

    let mut changes = Vec::new();
    for entry in entries {
        let key = entry.config.key();
        let wanted = selected.contains(&key);
        if wanted == entry.enabled {
            continue;
        }

        set_extension_enabled(&key, wanted);
        changes.push(match wanted {
            true => ExtensionChange::Enabled(entry.config.clone()),
            false => ExtensionChange::Disabled(entry.config.name()),
        });
    }

    Ok(Some(changes))
}

fn items(entries: &[ExtensionEntry]) -> Vec<(String, String, String)> {
    let name_width = entries
        .iter()
        .map(|entry| entry.config.name().chars().count())
        .max()
        .unwrap_or(0)
        .min(NAME_BUDGET);

    entries
        .iter()
        .map(|entry| {
            (
                entry.config.key(),
                label(entry, name_width),
                hint(&entry.config, name_width),
            )
        })
        .collect()
}

/// What the label could not say: the description, whenever the label spent its
/// room on an address or had to cut the description short. When the label
/// already ends in the whole description there is nothing left to add, and
/// saying it again printed the same sentence twice on one row.
fn hint(config: &ExtensionConfig, name_width: usize) -> String {
    let described = config.description().trim();
    let detail = detail(config);
    let shown = truncate_to_display_width(&detail, description_budget(used(name_width)));

    match shown == described {
        true => String::new(),
        false => described.to_string(),
    }
}

/// Columns the kind, the name and the gaps around them take, so the label and
/// the hint agree on how much room the description had.
fn used(name_width: usize) -> usize {
    KIND_WIDTH + 1 + name_width + 1
}

/// cliclack draws the hint only under the cursor, so everything that tells one
/// row from another has to sit in the label: the kind, because a server and a
/// builtin follow different rules, and enough of the description to recognise a
/// name nobody has met before. The hint keeps the untruncated text for whoever
/// stops on the row.
fn label(entry: &ExtensionEntry, name_width: usize) -> String {
    let kind = pad_to_display_width(entry.config.kind().label(), KIND_WIDTH);
    let name = pad_to_display_width(&entry.config.name(), name_width);
    let detail = detail(&entry.config);

    match detail.is_empty() {
        true => format!("{kind} {name}").trim_end().to_string(),
        false => format!(
            "{kind} {name} {}",
            truncate_to_display_width(&detail, description_budget(used(name_width)))
        ),
    }
}

/// What the row is about: for a server the address it will dial, for everything
/// else the description it came with. The address wins because two servers with
/// dull names are told apart by where they point, and a server's description is
/// usually whatever the person typed while adding it.
fn detail(config: &ExtensionConfig) -> String {
    let described = config.description().trim();
    match config {
        ExtensionConfig::StreamableHttp { uri, .. } => uri.clone(),
        ExtensionConfig::Sse { uri, .. } => uri.clone().unwrap_or_default(),
        ExtensionConfig::Stdio { cmd, args, .. } => match args.is_empty() {
            true => cmd.clone(),
            false => format!("{cmd} {}", args.join(" ")),
        },
        _ => described.to_string(),
    }
}

/// cliclack spends a few columns of its own on the checkbox and the frame, and a
/// label that overruns the terminal wraps into the next row and breaks the list.
fn description_budget(used: usize) -> usize {
    const FRAME: usize = 8;
    terminal_width()
        .unwrap_or(80)
        .saturating_sub(used + FRAME)
        .max(12)
}

/// Counted by the word on screen, not by the variant behind it: `Platform` and
/// `Builtin` are one thing to a reader, and counting them apart printed the same
/// word twice with two numbers.
fn summary(entries: &[ExtensionEntry]) -> String {
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for entry in entries {
        let label = entry.config.kind().label();
        match counts.iter_mut().find(|(seen, _)| *seen == label) {
            Some((_, count)) => *count += 1,
            None => counts.push((label, 1)),
        }
    }

    let breakdown = counts
        .iter()
        .map(|(label, count)| format!("{count} {label}"))
        .collect::<Vec<_>>()
        .join(" · ");

    let mut line = format!("{} configured: {breakdown}", entries.len());
    if let Some(plugins) = plugin_extensions_line() {
        line.push_str(&format!(" · {plugins}"));
    }
    line
}

/// A plugin brings its own servers along, and they never reach the config file,
/// so this list cannot manage them. Naming them is still worth a clause: without
/// it, a tool that plainly works sits next to a list that has never heard of it.
/// Where they are switched off is deliberately left out — that lives in
/// `settings.json`, and no command reaches it yet.
pub fn plugin_extensions_line() -> Option<String> {
    let working_dir = std::env::current_dir().ok();
    let names: Vec<String> =
        goose::plugins::mcp_servers::enabled_plugin_mcp_servers(working_dir.as_deref())
            .iter()
            .map(|config| config.name().to_string())
            .collect();

    match names.is_empty() {
        true => None,
        false => Some(format!(
            "{} more from plugins, not managed here: {}",
            names.len(),
            names.join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use goose::agents::extension::Envs;
    use goose::agents::ExtensionKind;

    fn entry(config: ExtensionConfig, enabled: bool) -> ExtensionEntry {
        ExtensionEntry { enabled, config }
    }

    fn server(name: &str) -> ExtensionConfig {
        ExtensionConfig::Stdio {
            name: name.to_string(),
            description: "a server".to_string(),
            cmd: "npx".to_string(),
            args: vec!["-y".to_string(), "atlas-mcp".to_string()],
            envs: Envs::default(),
            env_keys: Vec::new(),
            timeout: None,
            cwd: None,
            bundled: None,
            available_tools: Vec::new(),
        }
    }

    fn platform(name: &str, description: &str) -> ExtensionConfig {
        ExtensionConfig::Platform {
            name: name.to_string(),
            description: description.to_string(),
            display_name: None,
            bundled: None,
            available_tools: Vec::new(),
        }
    }

    fn bundled(name: &str) -> ExtensionConfig {
        ExtensionConfig::Builtin {
            name: name.to_string(),
            description: String::new(),
            display_name: None,
            timeout: None,
            bundled: None,
            available_tools: Vec::new(),
        }
    }

    /// `developer` ships as `type: builtin` while its neighbours are
    /// `type: platform`, so counting by variant printed "10 builtin · 1 builtin".
    #[test]
    fn the_two_kinds_that_share_a_word_are_counted_as_one() {
        let entries = vec![
            entry(server("atlas"), true),
            entry(platform("todo", ""), true),
            entry(bundled("developer"), false),
        ];

        assert_eq!(summary(&entries), "3 configured: 1 mcp · 2 builtin");
    }

    #[test]
    fn a_bundled_row_does_not_fall_out_of_the_alphabet_of_its_neighbours() {
        let mut kinds = [
            ExtensionKind::Builtin.display_rank(),
            ExtensionKind::Platform.display_rank(),
        ];
        kinds.sort_unstable();
        assert_eq!(kinds[0], kinds[1]);
    }

    #[test]
    fn the_hint_never_echoes_the_address_the_label_already_shows() {
        let remote = ExtensionConfig::streamable_http(
            "remote",
            "https://example.test/mcp",
            "Semantic search",
            30u64,
        );
        assert_eq!(hint(&remote, 14), "Semantic search");
        assert!(!hint(&remote, 14).contains("https://"));
    }

    /// A builtin's label ends in its whole description, so a hint repeating it
    /// printed the same sentence twice on the row under the cursor.
    #[test]
    fn a_description_the_label_already_shows_whole_is_not_said_again() {
        assert_eq!(hint(&platform("todo", "Keeps a list"), 14), "");
    }

    #[test]
    fn a_description_the_label_had_to_cut_short_survives_in_the_hint() {
        let long = "Keeps a list of everything the agent has decided to do next";
        let cut = hint(&platform("todo", long), 200);
        assert_eq!(cut, long);
    }

    #[test]
    fn a_server_is_described_by_where_it_points_and_a_builtin_by_what_it_does() {
        assert_eq!(detail(&server("atlas")), "npx -y atlas-mcp");
        assert_eq!(
            detail(&ExtensionConfig::streamable_http(
                "remote",
                "https://example.test/mcp",
                "",
                30u64
            )),
            "https://example.test/mcp"
        );
        assert_eq!(detail(&platform("todo", "Keeps a list")), "Keeps a list");
    }

    #[test]
    fn the_label_carries_the_kind_and_lines_the_names_up() {
        let rows = [
            entry(
                platform("code_execution", "Runs with the agent's own rights"),
                false,
            ),
            entry(platform("todo", ""), true),
        ];
        let width = 14;

        assert!(label(&rows[0], width).starts_with("builtin code_execution Runs with"));
        assert_eq!(label(&rows[1], width), "builtin todo");
    }

    #[test]
    fn a_row_never_outgrows_the_terminal() {
        let long = "x".repeat(500);
        let row = entry(platform("verbose", &long), true);

        let rendered = label(&row, 10);
        assert!(rendered.chars().count() < 500);
        assert!(rendered.ends_with("..."));
    }
}
