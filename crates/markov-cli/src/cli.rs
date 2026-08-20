use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Subcommand};
use goose_mcp::mcp_server_runner::McpCommand;
use std::path::PathBuf;

use crate::commands::ide::manager::ide_dialog;
use crate::commands::ide::{handle_ide_remove, handle_ide_setup, SetupOptions, TargetSelector};

/// Subcommands we keep working but no longer advertise.
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
const OURS: &[&str] = &["model", "extensions", "ide", "mcp"];

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
