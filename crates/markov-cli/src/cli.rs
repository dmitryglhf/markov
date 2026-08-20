use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Subcommand};
use goose_cli::cli::{ExtensionOptions, SessionOptions};
use goose_cli::markov::resume::resume_identifier;
use goose_mcp::mcp_server_runner::McpCommand;
use std::path::PathBuf;

use crate::commands::ide::manager::ide_dialog;
use crate::commands::ide::{handle_ide_remove, handle_ide_setup, SetupOptions, TargetSelector};

/// Subcommands we keep working but no longer advertise.
///
/// `plugin`: installing clones a git repository and can keep updating it behind
/// the session, and what arrives that way is not manageable yet — a plugin's
/// servers reach no list, its hooks reach none either, and its skills read as
/// your own. Discovery is untouched, so a directory dropped into
/// `.agents/plugins` still works for anyone who knows to do it.
const HIDDEN: &[&str] = &["plugin"];

#[derive(Subcommand)]
pub enum MarkovCommand {
    /// Choose the provider and model markov starts with
    #[command(about = "Choose the default provider and model", alias = "models")]
    Model {},

    /// Turn extensions of every kind on and off
    #[command(about = "Turn extensions on and off", alias = "extension")]
    Extensions {},

    /// Register markov as an ACP agent in your IDE
    #[command(
        about = "Connect markov to your IDE",
        long_about = "Registers markov as an ACP agent in Zed, JetBrains IDEs and VS Code.\n\n\
                      Usage:\n  \
                        markov ide                what is installed and what is already set up\n  \
                        markov ide setup          configure every IDE that is installed\n  \
                        markov ide setup zed      configure one of them\n  \
                        markov ide setup --print  show the snippet instead of writing it\n  \
                        markov ide remove         take our entry back out"
    )]
    Ide {
        #[command(subcommand)]
        command: Option<IdeCommand>,

        /// Name the entry appears under in the IDE
        #[arg(
            long,
            global = true,
            default_value = "markov",
            help = "Name the entry appears under in the IDE"
        )]
        name: String,
    },

    /// Manage MCP servers, or run one of the servers bundled with markov
    #[command(about = "Manage MCP servers, or run one bundled with markov")]
    Mcp {
        /// Run this bundled server on stdio instead of opening the manager
        #[arg(value_parser = clap::value_parser!(McpCommand))]
        server: Option<McpCommand>,
    },

    /// Look through skills and edit them
    #[command(about = "Manage skills")]
    Skills {
        /// Print the list instead of opening the manager
        #[command(subcommand)]
        command: Option<SkillsCommand>,
    },

    /// Pick up a previous session
    #[command(
        about = "Pick up a previous session",
        long_about = "Continue a previous session. Names it or gives its id to go straight there; otherwise offers a choice of recent sessions."
    )]
    Resume {
        #[arg(
            value_name = "SESSION",
            help = "Name or id of the session; omit to choose from a list"
        )]
        session: Option<String>,

        #[arg(
            long,
            help = "Fork instead of continuing (creates a new session with copied history)"
        )]
        fork: bool,

        #[arg(
            long,
            help = "Edit the session conversation in $EDITOR before starting"
        )]
        edit: bool,

        #[arg(
            long = "no-history",
            action = clap::ArgAction::SetFalse,
            help = "Skip reprinting previous messages when resuming"
        )]
        history: bool,

        #[command(flatten)]
        session_opts: SessionOptions,

        #[command(flatten)]
        extension_opts: ExtensionOptions,
    },
}

#[derive(Subcommand)]
pub enum SkillsCommand {
    /// List all skills available to the markov agent
    #[command(about = "List all skills available to the markov agent")]
    List,
}

#[derive(Subcommand)]
pub enum IdeCommand {
    /// Write the agent entry into an IDE's settings
    #[command(
        about = "Write the agent entry into an IDE's settings",
        long_about = "Adds markov to the IDE's list of ACP agents.\n\n\
                      Existing settings are preserved, comments and all, and running\n\
                      this again only updates our own entry."
    )]
    Setup {
        /// Which IDE to configure
        #[arg(value_enum, default_value = "all")]
        target: TargetSelector,

        /// Binary the IDE should launch, when it is not where install.sh puts it
        #[arg(long, help = "Binary the IDE should launch")]
        command: Option<PathBuf>,

        /// Print the snippet and the file it belongs in, change nothing
        #[arg(long, help = "Print the snippet instead of writing it")]
        print: bool,

        /// Report what would change without touching anything
        #[arg(long = "dry-run", help = "Report what would change, write nothing")]
        dry_run: bool,

        /// Install the VS Code extension from a file instead of the marketplace
        #[arg(long, help = "Install the VS Code extension from a .vsix")]
        vsix: Option<PathBuf>,
    },

    /// Remove our entry from an IDE's settings
    #[command(about = "Remove our entry from an IDE's settings")]
    Remove {
        /// Which IDE to clean up
        #[arg(value_enum, default_value = "all")]
        target: TargetSelector,
    },
}

/// Names we answer to, canonical form — aliases resolve to these in matches.
const OURS: &[&str] = &["model", "extensions", "ide", "mcp", "skills", "resume"];

fn ours() -> Vec<clap::Command> {
    MarkovCommand::augment_subcommands(clap::Command::new("markov"))
        .get_subcommands()
        .cloned()
        .collect()
}

/// Upstream's tree with ours grafted on: a name upstream already uses is
/// replaced outright, the rest are added. Replacing rather than adding matters —
/// two subcommands under one name panic in debug and silently pick upstream's in
/// release.
pub fn command_tree() -> clap::Command {
    let mut tree = goose_cli::Cli::command().name("markov");
    for sub in ours() {
        let name = sub.get_name().to_string();
        tree = if tree.find_subcommand(&name).is_some() {
            tree.mut_subcommand(name, move |_| sub)
        } else {
            tree.subcommand(sub)
        };
    }
    for hidden in HIDDEN {
        tree = tree.mut_subcommand(hidden, |c| c.hide(true));
    }
    tree
}

pub async fn run() -> Result<()> {
    let matches = command_tree().get_matches();
    match matches.subcommand_name() {
        Some(name) if OURS.contains(&name) => {
            dispatch(MarkovCommand::from_arg_matches(&matches)?).await
        }
        _ => goose_cli::cli::run_matches(&matches).await,
    }
}

async fn dispatch(command: MarkovCommand) -> Result<()> {
    match command {
        MarkovCommand::Model {} => crate::commands::models::handle_model_command().await,
        MarkovCommand::Extensions {} => crate::commands::extensions::handle_extensions_command(),
        MarkovCommand::Mcp { server } => match server {
            Some(server) => goose_cli::cli::handle_mcp_command(server).await,
            None => crate::commands::mcp_manager::mcp_dialog().await.map(|_| ()),
        },
        MarkovCommand::Resume {
            session,
            fork,
            edit,
            history,
            session_opts,
            extension_opts,
        } => {
            goose_cli::cli::handle_interactive_session(
                resume_identifier(session),
                true,
                fork,
                edit,
                history,
                session_opts,
                extension_opts,
            )
            .await
        }
        MarkovCommand::Skills { command } => match command {
            Some(SkillsCommand::List) => goose_cli::commands::skills::handle_skills_list().await,
            None => crate::commands::skills::skills_dialog().await,
        },
        MarkovCommand::Ide { command, name } => match command {
            Some(IdeCommand::Setup {
                target,
                command,
                print,
                dry_run,
                vsix,
            }) => handle_ide_setup(
                target,
                SetupOptions {
                    name,
                    command,
                    print,
                    dry_run,
                    vsix,
                },
            ),
            Some(IdeCommand::Remove { target }) => handle_ide_remove(target, &name),
            None => ide_dialog(&name),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> MarkovCommand {
        let matches = command_tree()
            .try_get_matches_from(args)
            .expect("parse failed");
        MarkovCommand::from_arg_matches(&matches).expect("not one of ours")
    }

    #[test]
    fn skills_command_accepts_list_subcommand() {
        assert!(matches!(
            parse(&["markov", "skills", "list"]),
            MarkovCommand::Skills {
                command: Some(SkillsCommand::List)
            }
        ));
    }

    #[test]
    fn skills_command_without_subcommand_opens_the_manager() {
        assert!(matches!(
            parse(&["markov", "skills"]),
            MarkovCommand::Skills { command: None }
        ));
    }

    /// A name we share with upstream has to be *replaced*, not added: two
    /// subcommands under one name panic in debug and silently pick upstream's in
    /// release, which would hand `skills` back to the listing we grew past.
    #[test]
    fn a_name_upstream_also_uses_resolves_to_ours() {
        let tree = command_tree();

        for name in OURS {
            let matching: Vec<_> = tree
                .get_subcommands()
                .filter(|sub| sub.get_name() == *name)
                .collect();
            assert_eq!(matching.len(), 1, "{name} is defined more than once");
        }

        let skills = tree.find_subcommand("skills").expect("skills command");
        assert_eq!(
            skills.get_about().map(|about| about.to_string()),
            Some("Manage skills".to_string())
        );
    }

    /// Without an argument there is nothing to resume by, which is what sends
    /// the command to the picker.
    #[test]
    fn a_bare_resume_asks_for_the_list() {
        let MarkovCommand::Resume { session, fork, .. } = parse(&["markov", "resume"]) else {
            panic!("expected resume");
        };

        assert!(session.is_none());
        assert!(!fork);
    }

    /// `--fork` is spelled with `--resume` on the session command; here resuming
    /// is the command, so requiring the flag it no longer has would reject this.
    #[test]
    fn resume_takes_the_flags_that_used_to_need_the_resume_flag() {
        let MarkovCommand::Resume {
            session,
            fork,
            edit,
            history,
            ..
        } = parse(&["markov", "resume", "project-x", "--fork", "--no-history"])
        else {
            panic!("expected resume");
        };

        assert_eq!(session.as_deref(), Some("project-x"));
        assert!(fork);
        assert!(!history);
        assert!(!edit);
    }

    /// Resuming replays the conversation unasked; the flag is there to stop it.
    #[test]
    fn resuming_replays_the_conversation_by_default() {
        let MarkovCommand::Resume { history, .. } = parse(&["markov", "resume"]) else {
            panic!("expected resume");
        };

        assert!(history);
    }

    #[test]
    fn a_hidden_command_still_runs() {
        let tree = command_tree();
        let plugin = tree.find_subcommand("plugin").expect("plugin command");

        assert!(plugin.is_hide_set());
        assert!(command_tree()
            .try_get_matches_from(["markov", "plugin", "update", "some-plugin"])
            .is_ok());
    }
}
