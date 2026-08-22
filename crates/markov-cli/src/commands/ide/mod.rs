pub mod jsonc;
pub mod manager;
pub mod paths;
pub mod targets;
pub mod vscode;

use anyhow::{anyhow, bail, Result};
use console::style;
use jsonc::Document;
use paths::{BaseDirs, Os};
use std::path::{Path, PathBuf};
use targets::{Target, TargetId};

pub use targets::TargetSelector;

pub struct SetupOptions {
    pub name: String,
    pub command: Option<PathBuf>,
    pub print: bool,
    pub dry_run: bool,
    pub vsix: Option<PathBuf>,
}

#[derive(Debug, PartialEq)]
pub enum Configured {
    Missing,
    Ours,
    Other(String),
    Unreadable(String),
}

#[derive(Debug, PartialEq)]
pub enum Change {
    Added,
    Updated,
    Unchanged,
}

pub fn handle_ide_status(name: &str) -> Result<()> {
    let dirs = BaseDirs::detect()?;
    let os = Os::current();
    let binary = dirs.cli_binary(os);

    println!();
    if binary.is_file() {
        println!("  {}  {}", style("cli").dim(), binary.display());
    } else {
        println!(
            "  {}  {} {}",
            style("cli").dim(),
            binary.display(),
            style("(not found — install it first)").red()
        );
    }

    for target in TargetSelector::All.expand() {
        let path = target.config_path(&dirs, os);
        println!();
        println!("  {}", style(target.label).bold());

        if !target.is_installed(&dirs, os) {
            println!("  {}", style("not installed").dim());
            continue;
        }

        match inspect(target, &path, name, &binary.to_string_lossy()) {
            Configured::Ours => println!("  {}  {}", style("configured").green(), path.display()),
            Configured::Missing => {
                println!("  {}  {}", style("not configured").yellow(), path.display())
            }
            Configured::Other(command) => {
                println!("  {}  {}", style("configured").green(), path.display());
                println!("  {}", style(format!("points at {command}")).dim());
            }
            Configured::Unreadable(err) => println!("  {}", style(err).red()),
        }

        if target.id == TargetId::Vscode && !vscode::extension_installed(&dirs) {
            println!(
                "  {}",
                style(format!("extension {} is missing", vscode::EXTENSION_ID)).yellow()
            );
        }
    }

    if os == Os::Windows {
        println!();
        println!(
            "  {}",
            style("Agents running inside WSL are not set up by this command.").dim()
        );
    }

    println!();
    Ok(())
}

pub fn handle_ide_setup(selector: TargetSelector, opts: SetupOptions) -> Result<()> {
    let dirs = BaseDirs::detect()?;
    let os = Os::current();

    let command = if opts.print {
        opts.command
            .clone()
            .unwrap_or_else(|| dirs.cli_binary(os))
            .to_string_lossy()
            .to_string()
    } else {
        resolve_command(&dirs, os, opts.command.as_deref())?
            .to_string_lossy()
            .to_string()
    };

    // An explicitly named target is always attempted: detection is a heuristic
    // and the user asking by name knows better than it does.
    let explicit = selector != TargetSelector::All;
    let mut failed = false;

    for target in selector.expand() {
        let path = target.config_path(&dirs, os);
        println!();
        println!("  {}", style(target.label).bold());

        if !explicit && !target.is_installed(&dirs, os) {
            println!("  {}", style("not installed, skipped").dim());
            continue;
        }

        if let Some(sandboxed) = blocking_sandbox(target, &dirs, os) {
            println!(
                "  {}",
                style(format!(
                    "installed as a flatpak ({}), which may not be allowed to run {command}",
                    sandboxed.display()
                ))
                .yellow()
            );
            println!("  {}", style("skipped — configure it by hand").dim());
            continue;
        }

        if opts.print {
            match snippet(target, &opts.name, &command) {
                Ok(text) => {
                    println!("  {}", path.display());
                    println!();
                    println!("{text}");
                }
                Err(err) => {
                    println!("  {}", style(err).red());
                    failed = true;
                }
            }
            continue;
        }

        match apply(target, &path, &opts, &command) {
            Ok(Change::Added) if opts.dry_run => {
                println!("  {}  {}", style("would add").yellow(), path.display())
            }
            Ok(Change::Updated) if opts.dry_run => {
                println!("  {}  {}", style("would update").yellow(), path.display())
            }
            Ok(Change::Unchanged) => {
                println!("  {}  {}", style("already set up").green(), path.display())
            }
            Ok(Change::Added) => println!("  {}  {}", style("added").green(), path.display()),
            Ok(Change::Updated) => println!("  {}  {}", style("updated").green(), path.display()),
            Err(err) => {
                println!("  {}", style(err).red());
                failed = true;
                continue;
            }
        }

        if target.id == TargetId::Vscode && !opts.dry_run {
            let outcome = ensure_extension(&dirs, os, opts.vsix.as_deref());
            if let Some(message) = outcome.message() {
                match outcome.is_problem() {
                    true => println!("  {}", style(message).yellow()),
                    false => println!("  {}", style(message).green()),
                }
            }
        }
    }

    println!();
    if failed {
        bail!("some targets were left unchanged");
    }
    Ok(())
}

pub fn handle_ide_remove(selector: TargetSelector, name: &str) -> Result<()> {
    let dirs = BaseDirs::detect()?;
    let os = Os::current();
    let mut failed = false;

    for target in selector.expand() {
        let path = target.config_path(&dirs, os);
        println!();
        println!("  {}", style(target.label).bold());

        match remove(target, &path, name) {
            Ok(true) => println!("  {}  {}", style("removed").green(), path.display()),
            Ok(false) => println!("  {}", style("nothing to remove").dim()),
            Err(err) => {
                println!("  {}", style(err).red());
                failed = true;
            }
        }
    }

    println!();
    if failed {
        bail!("some targets were left unchanged");
    }
    Ok(())
}

pub fn inspect(target: &Target, path: &Path, name: &str, command: &str) -> Configured {
    if !path.exists() {
        return Configured::Missing;
    }
    let doc = match Document::load(path) {
        Ok(doc) => doc,
        Err(err) => return Configured::Unreadable(err.to_string()),
    };
    let Some(container) = doc.container(target.container) else {
        return Configured::Missing;
    };
    let Some(current) = jsonc::get_entry(&container, name) else {
        return Configured::Missing;
    };

    if Some(&current) == jsonc::entry_as_json(target.entry(command)).as_ref() {
        Configured::Ours
    } else {
        let pointed_at = current
            .get("command")
            .and_then(|value| value.as_str())
            .unwrap_or("an unrecognised entry");
        Configured::Other(pointed_at.to_string())
    }
}

pub fn apply(target: &Target, path: &Path, opts: &SetupOptions, command: &str) -> Result<Change> {
    let doc = Document::load(path)?;
    let container = doc.container_or_create(target.container)?;

    let wanted = jsonc::entry_as_json(target.entry(command))
        .ok_or_else(|| anyhow!("could not render the {} entry", target.label))?;
    let current = jsonc::get_entry(&container, &opts.name);

    if current.as_ref() == Some(&wanted) {
        return Ok(Change::Unchanged);
    }

    let change = if current.is_some() {
        Change::Updated
    } else {
        Change::Added
    };

    if !opts.dry_run {
        jsonc::set_entry(&container, &opts.name, target.entry(command));
        doc.save()?;
    }

    Ok(change)
}

pub fn remove(target: &Target, path: &Path, name: &str) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let doc = Document::load(path)?;
    let Some(container) = doc.container(target.container) else {
        return Ok(false);
    };
    if !jsonc::remove_entry(&container, name) {
        return Ok(false);
    }
    doc.save()?;
    Ok(true)
}

pub fn snippet(target: &Target, name: &str, command: &str) -> Result<String> {
    let doc = Document::scratch();
    let container = doc.container_or_create(target.container)?;
    jsonc::set_entry(&container, name, target.entry(command));
    Ok(doc.to_text())
}

fn resolve_command(dirs: &BaseDirs, os: Os, given: Option<&Path>) -> Result<PathBuf> {
    let path = given
        .map(Path::to_path_buf)
        .unwrap_or_else(|| dirs.cli_binary(os));

    if !path.is_file() {
        bail!(
            "no markov binary at {}\ninstall it with install.sh, or point at one with --command",
            path.display()
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(&path)?.permissions().mode() & 0o111 == 0 {
            bail!("{} is not executable", path.display());
        }
    }

    Ok(path)
}

/// A flatpak IDE keeps its settings elsewhere and, more to the point, may not be
/// allowed to execute a binary from the user's home directory.
fn blocking_sandbox(target: &Target, dirs: &BaseDirs, os: Os) -> Option<PathBuf> {
    if os != Os::Linux {
        return None;
    }
    let sandboxed = match target.id {
        TargetId::Zed => dirs
            .home
            .join(".var/app/dev.zed.Zed/config/zed/settings.json"),
        TargetId::Vscode => dirs
            .home
            .join(".var/app/com.visualstudio.code/config/Code/User/settings.json"),
        TargetId::Jetbrains => return None,
    };
    let standard = target.config_path(dirs, os);
    (sandboxed.is_file() && !standard.exists()).then_some(sandboxed)
}

/// Best effort: a missing extension leaves the settings we wrote inert, but it
/// is not a reason to call the whole setup a failure. The outcome is returned
/// rather than printed, because the manager draws inside a cliclack frame that
/// a stray `println!` would break.
fn ensure_extension(dirs: &BaseDirs, os: Os, vsix: Option<&Path>) -> ExtensionOutcome {
    if vsix.is_none() && vscode::extension_installed(dirs) {
        return ExtensionOutcome::AlreadyThere;
    }

    let Some(cli) = vscode::find_cli(dirs, os) else {
        return ExtensionOutcome::NoCli;
    };

    match vscode::install_extension(&cli, vsix) {
        Ok(()) => ExtensionOutcome::Installed,
        Err(err) => ExtensionOutcome::Failed(err.to_string()),
    }
}

#[derive(Debug, PartialEq)]
pub enum ExtensionOutcome {
    AlreadyThere,
    Installed,
    NoCli,
    Failed(String),
}

impl ExtensionOutcome {
    /// What the user should be told, and `None` when there is nothing to say.
    pub fn message(&self) -> Option<String> {
        match self {
            ExtensionOutcome::AlreadyThere => None,
            ExtensionOutcome::Installed => {
                Some(format!("extension installed: {}", vscode::EXTENSION_ID))
            }
            ExtensionOutcome::NoCli => Some(format!(
                "install the {} extension from the marketplace — no code CLI found",
                vscode::EXTENSION_ID
            )),
            ExtensionOutcome::Failed(err) => Some(err.clone()),
        }
    }

    pub fn is_problem(&self) -> bool {
        matches!(self, ExtensionOutcome::NoCli | ExtensionOutcome::Failed(_))
    }
}
