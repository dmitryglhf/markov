use crate::ui::{cancellable, multiselect, require_terminal, select};
use anyhow::{Context, Result};
use console::style;
use goose::custom_requests::{SourceEntry, SourceType};
use goose::skills::{
    discover_all_skills, global_skills_dir, project_skills_dir, skill_argument_hint,
    skill_name_problem,
};
use goose::sources::{create_source, delete_source};
use goose::token_counter::{create_token_counter, TokenCounter};
use goose_cli::session::editor::{launch_editor, resolve_editor_or_default};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const DESCRIPTION_PREVIEW_CHARS: usize = 50;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Browse,
    Create,
    Edit,
    Remove,
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Global,
    Project,
}

/// One copy of a skill on disk. Several copies can share a name, and only the
/// first one discovery meets is ever used, so each copy carries the path of the
/// one that beat it.
struct Found {
    entry: SourceEntry,
    shadowed_by: Option<String>,
}

impl Found {
    fn editable(&self) -> bool {
        self.entry.source_type == SourceType::Skill && self.entry.writable
    }

    fn origin(&self) -> &'static str {
        if self.entry.source_type == SourceType::BuiltinSkill {
            return "builtin";
        }
        // A skill a plugin brought keeps its source of truth in that plugin's
        // repository, so saying "global" here would invite editing something the
        // next update overwrites.
        if !self.entry.writable {
            return "from a plugin";
        }
        match self.entry.global {
            true => "global",
            false => "project",
        }
    }

    fn file(&self) -> PathBuf {
        Path::new(&self.entry.path).join("SKILL.md")
    }
}

pub async fn skills_dialog() -> Result<()> {
    require_terminal("markov skills", Some("markov skills list"))?;
    cliclack::intro(style(" markov-skills ").on_cyan().black())?;

    let counter = create_token_counter().await.map_err(anyhow::Error::msg)?;
    let working_dir = std::env::current_dir()?;
    let mut changes = 0usize;

    loop {
        let found = discover(&working_dir);
        cliclack::log::info(summary(&found))?;

        let Some(action) = cancellable(
            cliclack::select("What would you like to do?")
                .item(
                    Action::Browse,
                    "Browse and inspect",
                    "What a skill says and where it lives",
                )
                .item(
                    Action::Create,
                    "Create a new one",
                    "Then open it in your editor",
                )
                .item(Action::Edit, "Edit a skill", "Open it in your editor")
                .item(Action::Remove, "Remove a skill", "Delete its directory")
                .item(Action::Done, "Done", "Leave the manager")
                .interact(),
        )?
        else {
            break;
        };

        if action != Action::Create && action != Action::Done && found.is_empty() {
            cliclack::log::warning("Create a skill first")?;
            continue;
        }

        match action {
            Action::Browse => browse(&found, &counter)?,
            Action::Create => changes += usize::from(create(&working_dir)?),
            Action::Edit => changes += usize::from(edit(&found, &working_dir)?),
            Action::Remove => changes += remove(&found)?,
            Action::Done => break,
        }
    }

    match changes {
        0 => cliclack::outro("Nothing changed")?,
        count => cliclack::outro(format!("{} change(s) saved", style(count).green()))?,
    }
    Ok(())
}

fn discover(working_dir: &Path) -> Vec<Found> {
    with_shadows(discover_all_skills(Some(working_dir)))
}

fn with_shadows(entries: Vec<SourceEntry>) -> Vec<Found> {
    let mut winners: HashMap<String, String> = HashMap::new();
    let mut found = Vec::new();

    for skill in entries {
        let winner = winners
            .entry(skill.name.clone())
            .or_insert_with(|| skill.path.clone())
            .clone();
        let shadowed_by = (winner != skill.path).then_some(winner);
        found.push(Found {
            entry: skill,
            shadowed_by,
        });
    }

    found
}

fn summary(found: &[Found]) -> String {
    if found.is_empty() {
        return "No skills found yet".to_string();
    }

    let shadowed = found
        .iter()
        .filter(|skill| skill.shadowed_by.is_some())
        .count();
    match shadowed {
        0 => format!("{} skill(s)", found.len()),
        count => format!("{} skill(s), {} shadowed", found.len(), count),
    }
}

/// Copies of one skill share a name, and cliclack shows the hint only for the
/// row under the cursor, so the label has to carry what tells them apart.
fn label(skill: &Found) -> String {
    match skill.shadowed_by.is_some() {
        true => format!("{} ({}, shadowed)", skill.entry.name, skill.origin()),
        false => format!("{} ({})", skill.entry.name, skill.origin()),
    }
}

fn hint(skill: &Found) -> String {
    description_preview(&skill.entry.description)
}

fn items(found: &[&Found]) -> Vec<(usize, String, String)> {
    found
        .iter()
        .enumerate()
        .map(|(index, skill)| (index, label(skill), hint(skill)))
        .collect()
}

fn browse(found: &[Found], counter: &TokenCounter) -> Result<()> {
    let listed: Vec<&Found> = found.iter().collect();
    let Some(index) = cancellable(select("Which skill?", &items(&listed)).interact())? else {
        return Ok(());
    };

    let skill = listed[index];
    cliclack::note(&skill.entry.name, card(skill, found, counter))?;
    Ok(())
}

fn card(skill: &Found, found: &[Found], counter: &TokenCounter) -> String {
    let mut lines = vec![
        row("Path", &skill.entry.path),
        row("Origin", skill.origin()),
        row(
            "Tokens",
            format!(
                "description {} · content {}",
                counter.count_tokens(&skill.entry.description),
                counter.count_tokens(&skill.entry.content)
            ),
        ),
    ];

    if let Some(arguments) = skill_argument_hint(&skill.entry) {
        lines.push(row("Arguments", arguments));
    }

    for file in &skill.entry.supporting_files {
        lines.push(row("File", file));
    }

    match &skill.shadowed_by {
        Some(winner) => lines.push(row("Not used", format!("{winner} answers to this name"))),
        None => {
            for other in found.iter().filter(|other| {
                other.entry.name == skill.entry.name && other.entry.path != skill.entry.path
            }) {
                lines.push(row("Also at", format!("{} (not used)", other.entry.path)));
            }
        }
    }

    lines.join("\n")
}

fn row(label: &str, value: impl std::fmt::Display) -> String {
    format!("{label:<10} {value}")
}

fn create(working_dir: &Path) -> Result<bool> {
    let Some(name): Option<String> = cancellable(
        cliclack::input("Name")
            .placeholder("lowercase-with-hyphens")
            .validate(|input: &String| match skill_name_problem(input.trim()) {
                Some(problem) => Err(problem),
                None => Ok(()),
            })
            .interact(),
    )?
    else {
        return Ok(false);
    };
    let name = name.trim().to_string();

    let Some(description): Option<String> = cancellable(
        cliclack::input("Description")
            .placeholder("When the agent should reach for this skill")
            .validate(|input: &String| match input.trim().is_empty() {
                true => Err("The description is how the agent decides to load a skill"),
                false => Ok(()),
            })
            .interact(),
    )?
    else {
        return Ok(false);
    };
    let description = description.trim().to_string();

    let project_dir = project_skills_dir(working_dir);
    let global_dir = global_skills_dir();
    let Some(scope) = cancellable(
        cliclack::select("Where should it live?")
            .item(
                Scope::Global,
                "Everywhere",
                global_dir
                    .as_ref()
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_default(),
            )
            .item(
                Scope::Project,
                "This project only",
                project_dir.display().to_string(),
            )
            .interact(),
    )?
    else {
        return Ok(false);
    };

    let entry = create_source(
        SourceType::Skill,
        &name,
        &description,
        "",
        scope == Scope::Global,
        Some(working_dir.to_string_lossy().as_ref()),
        HashMap::new(),
    )?;

    let file = Path::new(&entry.path).join("SKILL.md");
    cliclack::log::success(format!("Created {}", file.display()))?;
    open_in_editor(&file)?;
    report_after_edit(&entry.path, working_dir)?;
    Ok(true)
}

fn edit(found: &[Found], working_dir: &Path) -> Result<bool> {
    let editable: Vec<&Found> = found.iter().filter(|skill| skill.editable()).collect();
    if editable.is_empty() {
        cliclack::log::warning(
            "Nothing here is yours to change: built-in and plugin skills are read-only",
        )?;
        return Ok(false);
    }

    let Some(index) = cancellable(select("Edit which skill?", &items(&editable)).interact())?
    else {
        return Ok(false);
    };

    let skill = editable[index];
    open_in_editor(&skill.file())?;
    report_after_edit(&skill.entry.path, working_dir)?;
    Ok(true)
}

fn remove(found: &[Found]) -> Result<usize> {
    let removable: Vec<&Found> = found.iter().filter(|skill| skill.editable()).collect();
    if removable.is_empty() {
        cliclack::log::warning(
            "Nothing here is yours to change: built-in and plugin skills are read-only",
        )?;
        return Ok(0);
    }

    let Some(selected) = cancellable(
        multiselect(
            "Remove which skills? (space to toggle, enter to submit)",
            &items(&removable),
        )
        .required(false)
        .interact(),
    )?
    else {
        return Ok(0);
    };

    if selected.is_empty() {
        cliclack::log::info("Nothing removed")?;
        return Ok(0);
    }

    // A skill is a directory, and removal takes the whole of it. Say which ones
    // before asking, not after.
    let doomed: Vec<&Found> = selected.iter().map(|index| removable[*index]).collect();
    cliclack::log::warning(
        doomed
            .iter()
            .map(|skill| skill.entry.path.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    )?;

    let Some(confirmed) = cancellable(
        cliclack::confirm(match doomed.len() {
            1 => "Delete this directory?".to_string(),
            count => format!("Delete these {count} directories?"),
        })
        .initial_value(false)
        .interact(),
    )?
    else {
        return Ok(0);
    };
    if !confirmed {
        return Ok(0);
    }

    let mut removed = 0;
    for skill in doomed {
        match delete_source(SourceType::Skill, &skill.entry.path) {
            Ok(()) => removed += 1,
            Err(err) => cliclack::log::error(format!("{}: {err}", skill.entry.path))?,
        }
    }

    cliclack::log::success(format!("Removed {} skill(s)", style(removed).green()))?;
    Ok(removed)
}

fn open_in_editor(file: &Path) -> Result<()> {
    let editor = resolve_editor_or_default();
    launch_editor(&editor, &file.to_path_buf())
        .with_context(|| format!("failed to launch editor '{editor}'"))
}

/// A skill whose frontmatter stops parsing drops out of discovery without a
/// word, and a name that exists twice loses to whichever copy comes first. Both
/// are invisible from the editor, so they are said out loud here.
fn report_after_edit(path: &str, working_dir: &Path) -> Result<()> {
    let found = discover(working_dir);
    let Some(skill) = found.iter().find(|skill| skill.entry.path == path) else {
        cliclack::log::warning(
            "This skill is no longer picked up: frontmatter needs a name and a description",
        )?;
        return Ok(());
    };

    if let Some(winner) = &skill.shadowed_by {
        cliclack::log::warning(format!(
            "Saved, but not in use: {winner} already answers to \"{}\"",
            skill.entry.name
        ))?;
    }
    Ok(())
}

fn description_preview(description: &str) -> String {
    let normalized = description.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_to_chars(&normalized, DESCRIPTION_PREVIEW_CHARS)
}

fn truncate_to_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }

    let mut output = text.chars().take(max_chars - 3).collect::<String>();
    output.push_str("...");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, path: &str, global: bool) -> SourceEntry {
        SourceEntry {
            source_type: SourceType::Skill,
            name: name.to_string(),
            description: format!("what {name} is for"),
            path: path.to_string(),
            global,
            writable: true,
            ..Default::default()
        }
    }

    fn found(entry: SourceEntry) -> Found {
        Found {
            entry,
            shadowed_by: None,
        }
    }

    #[test]
    fn the_first_copy_of_a_name_wins_and_the_rest_say_who_beat_them() {
        let found = with_shadows(vec![
            entry("review", "/project/.agents/skills/review", false),
            entry("review", "/home/.claude/skills/review", true),
            entry("deploy", "/home/.agents/skills/deploy", true),
        ]);

        assert_eq!(found[0].shadowed_by, None);
        assert_eq!(
            found[1].shadowed_by.as_deref(),
            Some("/project/.agents/skills/review")
        );
        assert_eq!(found[2].shadowed_by, None);
    }

    #[test]
    fn the_summary_counts_the_copies_nobody_uses() {
        let found = with_shadows(vec![
            entry("review", "/a/review", false),
            entry("review", "/b/review", true),
        ]);

        assert_eq!(summary(&found), "2 skill(s), 1 shadowed");
        assert_eq!(summary(&[]), "No skills found yet");
        assert_eq!(
            summary(&with_shadows(vec![entry("review", "/a/review", false)])),
            "1 skill(s)"
        );
    }

    /// Editing a plugin's copy is undone by the plugin's next update, and
    /// removing it deletes a directory inside somebody else's checkout, so
    /// neither is offered.
    #[test]
    fn only_what_we_own_is_offered_for_editing() {
        let mine = found(entry("review", "/home/.agents/skills/review", true));

        let mut from_plugin = entry("release-notes", "/home/.agents/plugins/kit/skills", true);
        from_plugin.writable = false;

        let mut builtin = entry("goose-doc-guide", "builtin://skills/goose-doc-guide", true);
        builtin.source_type = SourceType::BuiltinSkill;

        assert!(mine.editable());
        assert!(!found(from_plugin.clone()).editable());
        assert!(!found(builtin.clone()).editable());

        assert_eq!(mine.origin(), "global");
        assert_eq!(found(from_plugin).origin(), "from a plugin");
        assert_eq!(found(builtin).origin(), "builtin");
    }

    #[test]
    fn a_shadowed_copy_is_told_apart_by_its_label() {
        let found = with_shadows(vec![
            entry("review", "/a/review", false),
            entry("review", "/b/review", true),
        ]);

        assert_eq!(label(&found[0]), "review (project)");
        assert_eq!(label(&found[1]), "review (global, shadowed)");
        assert_eq!(hint(&found[0]), "what review is for");
    }

    #[tokio::test]
    async fn the_card_names_the_copy_on_either_side_of_the_shadow() {
        let counter = create_token_counter().await.expect("token counter");
        let found = with_shadows(vec![
            entry("review", "/a/review", false),
            entry("review", "/b/review", true),
        ]);

        let winner = card(&found[0], &found, &counter);
        assert!(winner.contains("Also at"), "{winner}");
        assert!(winner.contains("/b/review"), "{winner}");

        let loser = card(&found[1], &found, &counter);
        assert!(loser.contains("Not used"), "{loser}");
        assert!(loser.contains("/a/review"), "{loser}");
    }

    #[test]
    fn built_in_skills_are_not_offered_for_editing() {
        let mut builtin = entry("goose-doc-guide", "builtin://skills/goose-doc-guide", true);
        builtin.source_type = SourceType::BuiltinSkill;
        let found = with_shadows(vec![builtin, entry("review", "/a/review", false)]);

        assert!(!found[0].editable());
        assert_eq!(found[0].origin(), "builtin");
        assert!(found[1].editable());
    }
}
