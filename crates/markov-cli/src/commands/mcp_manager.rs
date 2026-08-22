//! Interactive management of external MCP servers.
//!
//! The dialog knows nothing about a running session: it draws the form, checks
//! the connection, writes the config and reports what changed. Applying those
//! changes to a live agent is up to the caller, so `markov mcp` and the `/mcp`
//! slash command share the same code.

use anyhow::Result;
use console::style;
use futures::StreamExt;
use goose::agents::extension::Envs;
use goose::agents::{ExtensionConfig, ExtensionKind, ExtensionManager};
use goose::config::extensions::{
    get_all_extension_names, get_all_extensions, name_to_key, remove_extension, set_extension,
    set_extension_enabled,
};
use goose::config::paths::Paths;
use goose::config::{Config, ExtensionEntry, PermissionManager, DEFAULT_EXTENSION_TIMEOUT};
use std::collections::HashMap;
use std::sync::Arc;

use crate::commands::extensions::{bundled_servers_line, plugin_extensions_line};
use crate::commands::mcp_registry::{self, Candidate, Install};
use crate::ui::{cancellable, multiselect, require_terminal, select};
use goose_cli::commands::configure::try_store_secret;
use goose_cli::markov::types::ExtensionChange;

/// Only passed through to the server's `list_tools`, no session is created.
const PROBE_SESSION: &str = "mcp-connection-check";

/// A first `npx` or `uvx` start downloads the package before it can answer, and
/// the timeout kills that download, so a retry begins as cold as the attempt it
/// follows. The wait has to cover an install, not a handshake.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// Only reads the opening of a stream, so it does not need the probe's patience.
const SSE_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

const LEGACY_SSE_MESSAGE: &str =
    "the server speaks the retired HTTP+SSE protocol, which markov does not support. \
     Ask whoever runs it for a streamable_http endpoint";

enum ProbeOutcome {
    Connected(Vec<String>),
    Failed(String),
    Interrupted,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Connect,
    Inspect,
    Edit,
    Toggle,
    Remove,
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Pgpro,
    Registry,
    Manual,
}

/// What a catalogue entry knows about a server, in the shape the connect form
/// takes. Everything stays editable: an entry can be written badly.
#[derive(Default)]
struct Prefill {
    target: String,
    name: String,
    secrets: Vec<String>,
    plain: Vec<mcp_registry::Variable>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditTarget {
    Address,
    Credentials,
    Timeout,
    Tools,
    Back,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Transport {
    Stdio,
    Http,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FailedProbe {
    Retry,
    SaveAnyway,
    Discard,
}

pub async fn mcp_dialog() -> Result<Vec<ExtensionChange>> {
    require_terminal("markov mcp", None)?;

    cliclack::intro(style(" markov-mcp ").on_cyan().black())?;

    let mut changes: Vec<ExtensionChange> = Vec::new();
    loop {
        let servers = configured_servers();
        if servers.is_empty() {
            cliclack::log::info("No MCP servers configured yet")?;
        } else {
            cliclack::log::info(
                servers
                    .iter()
                    .map(describe)
                    .collect::<Vec<_>>()
                    .join("   ·   "),
            )?;
        }

        if let Some(line) = plugin_extensions_line() {
            cliclack::log::info(line)?;
        }

        if let Some(line) = bundled_servers_line() {
            cliclack::log::info(line)?;
        }

        // Every action is listed even with nothing configured: the first visit is
        // where people learn what the manager can do.
        let Some(action) = cancellable(
            cliclack::select("What would you like to do?")
                .item(
                    Action::Connect,
                    "Add a server",
                    "From a catalogue or by hand",
                )
                .item(
                    Action::Inspect,
                    "Show a server",
                    "Its settings and the tools it offers",
                )
                .item(
                    Action::Edit,
                    "Edit a server",
                    "Address, credentials, timeout, tools",
                )
                .item(
                    Action::Toggle,
                    "Turn servers on and off",
                    "Without losing their settings",
                )
                .item(Action::Remove, "Remove a server", "Drop it from the config")
                .item(Action::Done, "Done", "Leave the manager")
                .interact(),
        )?
        else {
            break;
        };

        if action != Action::Connect && action != Action::Done && servers.is_empty() {
            cliclack::log::warning("Add a server first")?;
            continue;
        }

        match action {
            Action::Connect => changes.extend(add_server().await?),
            Action::Inspect => inspect(&servers).await?,
            Action::Edit => changes.extend(edit(&servers).await?),
            Action::Toggle => changes.extend(toggle(&servers)?),
            Action::Remove => changes.extend(remove(&servers)?),
            Action::Done => break,
        }
    }

    if changes.is_empty() {
        cliclack::outro("Nothing changed")?;
    } else {
        cliclack::outro(format!("{} change(s) saved", style(changes.len()).green()))?;
    }
    Ok(changes)
}

async fn add_server() -> Result<Vec<ExtensionChange>> {
    let Some(source) = cancellable(
        cliclack::select("Where should the server come from?")
            .item(Source::Pgpro, "From PgPro registry", "Ours, ready to use")
            .item(
                Source::Registry,
                "Search the registry",
                "Public catalogue of third-party servers",
            )
            .item(
                Source::Manual,
                "Enter manually",
                "A local command or a remote URL",
            )
            .interact(),
    )?
    else {
        return Ok(Vec::new());
    };

    match source {
        Source::Pgpro => match pick_ours()? {
            Some(prefill) => connect(prefill).await,
            None => Ok(Vec::new()),
        },
        Source::Registry => match search_registry().await? {
            Some(prefill) => connect(prefill).await,
            None => Ok(Vec::new()),
        },
        Source::Manual => connect(Prefill::default()).await,
    }
}

/// Our own catalogue is short enough to show whole: no search box, and no
/// warning about strangers, because these entries are not from strangers.
fn pick_ours() -> Result<Option<Prefill>> {
    let found = mcp_registry::pgpro();
    if found.is_empty() {
        let unset = mcp_registry::pgpro_unset();
        cliclack::log::info(match unset.is_empty() {
            true => "The PgPro catalogue is empty".to_string(),
            false => format!(
                "Nothing set up for this machine yet. Ask for the address and set {}",
                unset.join(", ")
            ),
        })?;
        return Ok(None);
    }

    let items: Vec<(usize, String, String)> = found
        .iter()
        .enumerate()
        .map(|(index, candidate)| (index, candidate.title.clone(), summarise(candidate)))
        .collect();

    let Some(index) = cancellable(select("Pick a server", &items).interact())? else {
        return Ok(None);
    };

    let candidate = &found[index];
    let Some(install) = pick_install(candidate)? else {
        return Ok(None);
    };
    Ok(Some(prefill(candidate, install)))
}

fn prefill(candidate: &Candidate, install: &Install) -> Prefill {
    Prefill {
        target: install.target.clone(),
        name: mcp_registry::short_name(&candidate.name),
        secrets: install.secrets.clone(),
        plain: install.plain.clone(),
    }
}

/// Nothing here is trusted enough to skip the connect form: the entry only
/// fills the fields in, the person still sees and approves every one.
async fn search_registry() -> Result<Option<Prefill>> {
    let Some(query) = cancellable(
        cliclack::input("Search")
            .placeholder("postgres")
            .validate(|input: &String| {
                if input.trim().is_empty() {
                    Err("Enter something to look for")
                } else {
                    Ok(())
                }
            })
            .interact(),
    )?
    else {
        return Ok(None);
    };
    let query: String = query;
    let query = query.trim().to_string();

    let spinner = cliclack::spinner();
    spinner.start("Searching the registry");
    let found = match mcp_registry::search(&query).await {
        Ok(found) => found,
        Err(e) => {
            spinner.error(format!("The registry did not answer: {e}"));
            return Ok(None);
        }
    };

    if found.is_empty() {
        spinner.stop(format!("Nothing found for {}", style(&query).red()));
        return Ok(None);
    }
    spinner.stop(format!(
        "{} {}",
        style(found.len()).green(),
        if found.len() == 1 {
            "server"
        } else {
            "servers"
        }
    ));

    cliclack::log::warning("These servers are published by strangers. Markov does not vet them.")?;

    let items: Vec<(usize, String, String)> = found
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            (
                index,
                format!("{}   {}", candidate.title, style(&candidate.name).dim()),
                summarise(candidate),
            )
        })
        .collect();

    let Some(index) = cancellable(select("Pick a server", &items).interact())? else {
        return Ok(None);
    };

    let candidate = &found[index];
    let Some(install) = pick_install(candidate)? else {
        return Ok(None);
    };
    Ok(Some(prefill(candidate, install)))
}

fn pick_install(candidate: &Candidate) -> Result<Option<&Install>> {
    if candidate.options.len() == 1 {
        return Ok(candidate.options.first());
    }

    let items: Vec<(usize, String, String)> = candidate
        .options
        .iter()
        .enumerate()
        .map(|(index, install)| (index, install.label.clone(), install.target.clone()))
        .collect();

    let Some(index) = cancellable(
        cliclack::select("This server can be run in more than one way")
            .items(&items)
            .interact(),
    )?
    else {
        return Ok(None);
    };

    Ok(candidate.options.get(index))
}

/// The catalogue puts no limit on a description, and a select item that wraps
/// pushes the rest of the list off the screen.
fn summarise(candidate: &Candidate) -> String {
    const WIDTH: usize = 70;

    let text = candidate.description.split_whitespace().collect::<Vec<_>>();
    let mut summary = String::new();
    for word in text {
        if summary.len() + word.len() + 1 > WIDTH {
            summary.push('…');
            break;
        }
        if !summary.is_empty() {
            summary.push(' ');
        }
        summary.push_str(word);
    }
    summary
}

async fn connect(prefill: Prefill) -> Result<Vec<ExtensionChange>> {
    let target = cliclack::input("Command or URL").validate(|input: &String| {
        if input.trim().is_empty() {
            Err("Enter a command to run or a URL to connect to")
        } else {
            Ok(())
        }
    });
    // A placeholder of our own would hide what the entry filled in: cliclack
    // shows the default only while no placeholder is set.
    let mut target = match prefill.target.is_empty() {
        true => target.placeholder("npx -y @scope/server · https://host/mcp"),
        false => target.default_input(&prefill.target),
    };

    let Some(target) = cancellable(target.interact())? else {
        return Ok(Vec::new());
    };
    let target: String = target;
    let target = target.trim().to_string();

    let transport = detect_transport(&target);
    let suggested = match is_usable_key(&name_to_key(&prefill.name)) {
        true => prefill.name.clone(),
        false => derive_name(&target, transport),
    };
    let taken = get_all_extension_names();
    let Some(name) = cancellable(
        cliclack::input("Name")
            .default_input(&suggested)
            .validate(move |input: &String| {
                if input.trim().is_empty() {
                    Err("Enter a name")
                } else if !is_usable_key(&name_to_key(input)) {
                    Err("Use latin letters, digits, - or _")
                } else if taken.contains(&name_to_key(input)) {
                    Err("A server with this name already exists")
                } else {
                    Ok(())
                }
            })
            .interact(),
    )?
    else {
        return Ok(Vec::new());
    };
    let name: String = name;
    let name = name.trim().to_string();

    let Some(settings) = ask_settings(&prefill.plain)? else {
        return Ok(Vec::new());
    };

    let Some(mut credentials) = collect_credentials(&name, transport, &prefill.secrets)? else {
        return Ok(Vec::new());
    };

    let target = match transport {
        Transport::Stdio => with_assignments(&settings, &target),
        Transport::Http => {
            credentials.headers.extend(settings);
            target
        }
    };

    let stored_keys = credentials.env_keys.clone();
    let config = match build_config(&name, &target, transport, credentials) {
        Ok(config) => config,
        Err(e) => {
            forget_secrets(&stored_keys);
            cliclack::log::error(format!("{e}"))?;
            return Ok(Vec::new());
        }
    };

    if !confirm_connection(&config, &stored_keys).await? {
        return Ok(Vec::new());
    }

    set_extension(ExtensionEntry {
        enabled: true,
        config: config.clone(),
    });

    cliclack::log::success(format!("Added {}", style(&name).green()))?;
    Ok(vec![ExtensionChange::Connected(config)])
}

/// Probe the server and let the user decide what a failure means. `false` means
/// the server should not be saved.
async fn confirm_connection(config: &ExtensionConfig, stored_keys: &[String]) -> Result<bool> {
    let name = config.name();
    loop {
        let spinner = cliclack::spinner();
        spinner.start(format!("Connecting to {name}"));

        match probe(config).await {
            ProbeOutcome::Connected(tools) => {
                spinner.stop(format!(
                    "{}: connected, {} {}",
                    style(&name).green(),
                    tools.len(),
                    if tools.len() == 1 { "tool" } else { "tools" }
                ));
                return Ok(true);
            }
            ProbeOutcome::Interrupted => {
                spinner.cancel(format!("{}: gave up waiting", style(&name).red()));
                forget_secrets(stored_keys);
                return Ok(false);
            }
            ProbeOutcome::Failed(e) => {
                spinner.error(format!("{}: {}", style(&name).red(), e));

                let Some(next) = cancellable(
                    cliclack::select("The server did not answer. What now?")
                        .item(
                            FailedProbe::Retry,
                            "Try again",
                            "A first npx or uvx start can outlast the wait",
                        )
                        .item(
                            FailedProbe::SaveAnyway,
                            "Save anyway",
                            "Markov will retry on every session start",
                        )
                        .item(FailedProbe::Discard, "Discard", "Forget this server")
                        .interact(),
                )?
                else {
                    forget_secrets(stored_keys);
                    return Ok(false);
                };

                match next {
                    FailedProbe::Retry => continue,
                    FailedProbe::SaveAnyway => return Ok(true),
                    FailedProbe::Discard => {
                        forget_secrets(stored_keys);
                        return Ok(false);
                    }
                }
            }
        }
    }
}

async fn inspect(servers: &[ExtensionEntry]) -> Result<()> {
    let Some(entry) = pick_server("Which server?", servers)? else {
        return Ok(());
    };

    let mut lines = vec![
        format!("state    {}", if entry.enabled { "on" } else { "off" }),
        format!("kind     {}", transport_label(&entry.config)),
        format!("target   {}", target_of(&entry.config)),
        format!("timeout  {}s", timeout_of(&entry.config)),
    ];

    let secrets = credential_keys(&entry.config);
    if !secrets.is_empty() {
        lines.push(format!("secrets  {}", secrets.join(", ")));
    }
    if let ExtensionConfig::StreamableHttp { headers, .. } = &entry.config {
        for (header, value) in headers {
            lines.push(format!("header   {header}: {value}"));
        }
    }
    let allowed = allowed_tools(&entry.config);
    if !allowed.is_empty() {
        lines.push(format!("tools    limited to {}", allowed.join(", ")));
    }

    let spinner = cliclack::spinner();
    spinner.start("Asking the server for its tools");
    match probe(&entry.config).await {
        ProbeOutcome::Connected(tools) => {
            spinner.stop(format!("{} answered", style(entry.config.name()).green()));
            lines.push(format!(
                "offers   {}",
                if tools.is_empty() {
                    "no tools".to_string()
                } else {
                    tools.join(", ")
                }
            ));
        }
        ProbeOutcome::Interrupted => {
            spinner.cancel("gave up waiting");
        }
        ProbeOutcome::Failed(e) => {
            spinner.error(format!("{}: {}", style(entry.config.name()).red(), e));
        }
    }

    cliclack::note(entry.config.name(), lines.join("\n"))?;
    Ok(())
}

async fn edit(servers: &[ExtensionEntry]) -> Result<Vec<ExtensionChange>> {
    let Some(entry) = pick_server("Which server?", servers)? else {
        return Ok(Vec::new());
    };

    let Some(what) = cancellable(
        cliclack::select("What would you like to change?")
            .item(
                EditTarget::Address,
                "Address or command",
                target_of(&entry.config),
            )
            .item(
                EditTarget::Credentials,
                "Credentials",
                "Replace the tokens this server needs",
            )
            .item(
                EditTarget::Timeout,
                "Timeout",
                format!("{}s", timeout_of(&entry.config)),
            )
            .item(
                EditTarget::Tools,
                "Tools",
                "Choose which of its tools markov may call",
            )
            .item(EditTarget::Back, "Back", "Change nothing")
            .interact(),
    )?
    else {
        return Ok(Vec::new());
    };

    let name = entry.config.name();
    let updated = match what {
        EditTarget::Back => return Ok(Vec::new()),
        EditTarget::Address => {
            let transport = transport_of(&entry.config);
            let Some(target) = cancellable(
                cliclack::input("Command or URL")
                    .default_input(&target_of(&entry.config))
                    .validate(|input: &String| {
                        if input.trim().is_empty() {
                            Err("Enter a command to run or a URL to connect to")
                        } else {
                            Ok(())
                        }
                    })
                    .interact(),
            )?
            else {
                return Ok(Vec::new());
            };
            let target: String = target;
            let target = target.trim().to_string();

            if detect_transport(&target) != transport {
                cliclack::log::warning(
                    "That changes the transport. Remove the server and connect it again.",
                )?;
                return Ok(Vec::new());
            }

            let credentials = Credentials {
                env_keys: credential_keys(&entry.config),
                headers: match &entry.config {
                    ExtensionConfig::StreamableHttp { headers, .. } => headers.clone(),
                    _ => HashMap::new(),
                },
            };
            let mut config = build_config(&name, &target, transport, credentials)?;
            set_allowed_tools(&mut config, allowed_tools(&entry.config));
            set_timeout(&mut config, timeout_of(&entry.config));

            if !confirm_connection(&config, &[]).await? {
                return Ok(Vec::new());
            }
            config
        }
        EditTarget::Credentials => {
            let transport = transport_of(&entry.config);
            // Replacing a token overwrites the stored value under the same name,
            // so the working one has to be kept until the new one is proven.
            let previous = snapshot_secrets(&credential_keys(&entry.config));

            let slots = credential_slots(&entry.config);
            let Some(credentials) = collect_credentials(&name, transport, &slots)? else {
                restore_secrets(&previous);
                return Ok(Vec::new());
            };

            let fresh_keys = credentials.env_keys.clone();
            let mut config =
                build_config(&name, &target_of(&entry.config), transport, credentials)?;
            set_allowed_tools(&mut config, allowed_tools(&entry.config));
            set_timeout(&mut config, timeout_of(&entry.config));

            // The old secrets go only once the new ones are known to work.
            if !confirm_connection(&config, &fresh_keys).await? {
                restore_secrets(&previous);
                return Ok(Vec::new());
            }
            let stale: Vec<String> = credential_keys(&entry.config)
                .into_iter()
                .filter(|key| !fresh_keys.contains(key))
                .collect();
            forget_secrets(&stale);
            config
        }
        EditTarget::Timeout => {
            let Some(seconds) = cancellable(
                cliclack::input("Timeout in seconds")
                    .default_input(&timeout_of(&entry.config).to_string())
                    .validate(|input: &String| match input.trim().parse::<u64>() {
                        Ok(0) => Err("Use at least one second"),
                        Ok(_) => Ok(()),
                        Err(_) => Err("Enter a whole number of seconds"),
                    })
                    .interact(),
            )?
            else {
                return Ok(Vec::new());
            };
            let seconds: String = seconds;

            let mut config = entry.config.clone();
            set_timeout(&mut config, seconds.trim().parse()?);
            config
        }
        EditTarget::Tools => {
            let spinner = cliclack::spinner();
            spinner.start("Asking the server for its tools");
            let offered = match probe(&entry.config).await {
                ProbeOutcome::Connected(tools) => {
                    spinner.stop(format!("{} answered", style(&name).green()));
                    tools
                }
                ProbeOutcome::Interrupted => {
                    spinner.cancel("gave up waiting");
                    return Ok(Vec::new());
                }
                ProbeOutcome::Failed(e) => {
                    spinner.error(format!("{}: {}", style(&name).red(), e));
                    return Ok(Vec::new());
                }
            };

            if offered.is_empty() {
                cliclack::log::warning("This server offers no tools")?;
                return Ok(Vec::new());
            }

            let allowed = allowed_tools(&entry.config);
            let initial = if allowed.is_empty() {
                offered.clone()
            } else {
                allowed
            };
            let items: Vec<(String, String, String)> = offered
                .iter()
                .map(|tool| (tool.clone(), tool.clone(), String::new()))
                .collect();

            let Some(selected) = cancellable(
                multiselect("Which tools may markov call? (space to toggle)", &items)
                    .initial_values(initial)
                    .required(false)
                    .interact(),
            )?
            else {
                return Ok(Vec::new());
            };

            let mut config = entry.config.clone();
            // An empty list means "everything", so a full selection is stored as empty.
            set_allowed_tools(
                &mut config,
                if selected.len() == offered.len() {
                    Vec::new()
                } else {
                    selected
                },
            );
            config
        }
    };

    set_extension(ExtensionEntry {
        enabled: entry.enabled,
        config: updated.clone(),
    });
    cliclack::log::success(format!("Updated {}", style(&name).green()))?;

    Ok(if entry.enabled {
        vec![ExtensionChange::Connected(updated)]
    } else {
        Vec::new()
    })
}

fn toggle(servers: &[ExtensionEntry]) -> Result<Vec<ExtensionChange>> {
    let items = server_items(servers);
    let enabled: Vec<String> = servers
        .iter()
        .filter(|entry| entry.enabled)
        .map(|entry| entry.config.key())
        .collect();

    let Some(selected) = cancellable(
        multiselect(
            "Which servers should be on? (space to toggle, enter to submit)",
            &items,
        )
        .initial_values(enabled)
        .required(false)
        .interact(),
    )?
    else {
        return Ok(Vec::new());
    };

    let mut changes = Vec::new();
    for entry in servers {
        let key = entry.config.key();
        let wanted = selected.contains(&key);
        if wanted == entry.enabled {
            continue;
        }

        set_extension_enabled(&key, wanted);
        changes.push(if wanted {
            ExtensionChange::Enabled(entry.config.clone())
        } else {
            ExtensionChange::Disabled(entry.config.name())
        });
    }

    if changes.is_empty() {
        cliclack::log::info("Nothing changed")?;
    } else {
        cliclack::log::success(format!(
            "Updated {} server(s)",
            style(changes.len()).green()
        ))?;
    }
    Ok(changes)
}

fn remove(servers: &[ExtensionEntry]) -> Result<Vec<ExtensionChange>> {
    let items = server_items(servers);

    let Some(selected) = cancellable(
        multiselect(
            "Remove which servers? (space to toggle, enter to submit)",
            &items,
        )
        .required(false)
        .interact(),
    )?
    else {
        return Ok(Vec::new());
    };

    if selected.is_empty() {
        cliclack::log::info("Nothing removed")?;
        return Ok(Vec::new());
    }

    let Some(confirmed) = cancellable(
        cliclack::confirm(format!(
            "Remove {} server(s) from the config?",
            selected.len()
        ))
        .initial_value(false)
        .interact(),
    )?
    else {
        return Ok(Vec::new());
    };
    if !confirmed {
        return Ok(Vec::new());
    }

    let mut changes = Vec::new();
    for entry in servers {
        let key = entry.config.key();
        if !selected.contains(&key) {
            continue;
        }

        remove_extension(&key);
        PermissionManager::instance().remove_extension(&key);
        forget_secrets(&credential_keys(&entry.config));
        changes.push(ExtensionChange::Removed(entry.config.name()));
    }

    cliclack::log::success(format!(
        "Removed {} server(s)",
        style(changes.len()).green()
    ))?;
    Ok(changes)
}

/// Settings a catalogue entry says the server needs. Nothing secret goes in
/// here, so the values are written to the config as typed.
fn ask_settings(wanted: &[mcp_registry::Variable]) -> Result<Option<Vec<(String, String)>>> {
    let mut settings = Vec::new();

    for variable in wanted {
        let mut question = cliclack::input(&variable.name);
        match &variable.default {
            Some(default) => question = question.default_input(default),
            None if !variable.description.is_empty() => {
                question = question.placeholder(&variable.description)
            }
            None => {}
        }

        let Some(value) = cancellable(question.interact())? else {
            return Ok(None);
        };
        let value: String = value;
        let value = value.trim().to_string();
        if !value.is_empty() {
            settings.push((variable.name.clone(), value));
        }
    }

    Ok(Some(settings))
}

/// A stdio server reads its settings from the environment, and the connect form
/// already understands assignments in front of a command.
fn with_assignments(settings: &[(String, String)], target: &str) -> String {
    let mut parts: Vec<String> = settings
        .iter()
        .map(|(key, value)| match value.contains(char::is_whitespace) {
            true => format!("{key}=\"{value}\""),
            false => format!("{key}={value}"),
        })
        .collect();

    parts.push(target.to_string());
    parts.join(" ")
}

/// Tokens typed here go to the secret store; the config only ever holds their
/// names, which the extension manager resolves when the server starts.
///
/// `known` names the slots a catalogue entry already declared, so the person is
/// asked for values instead of having to look up what the server reads.
fn collect_credentials(
    name: &str,
    transport: Transport,
    known: &[String],
) -> Result<Option<Credentials>> {
    let mut credentials = Credentials::default();

    if known.is_empty() {
        let Some(needed) = cancellable(
            cliclack::confirm("Does this server need a token or an API key?")
                .initial_value(false)
                .interact(),
        )?
        else {
            return Ok(None);
        };
        if !needed {
            return Ok(Some(credentials));
        }
    }

    if !known.is_empty() {
        cliclack::log::info("Leave a value empty to skip that one")?;
    }

    let mut pending = known.to_vec();
    loop {
        let listed = !pending.is_empty();
        let slot = if listed {
            pending.remove(0)
        } else {
            let Some(slot) = ask_credential_slot(transport, credentials.env_keys.len())? else {
                forget_secrets(&credentials.env_keys);
                return Ok(None);
            };
            slot
        };

        let mut ask = cliclack::password(format!("Value for {slot}")).mask('▪');
        if listed {
            ask = ask.allow_empty();
        }

        let Some(value) = cancellable(ask.interact())? else {
            forget_secrets(&credentials.env_keys);
            return Ok(None);
        };
        let value: String = value;

        // An entry lists every slot its server knows, and one person rarely
        // holds them all. Nothing is stored for the ones left blank.
        if !(listed && value.trim().is_empty()) {
            let key = match transport {
                Transport::Stdio => slot.clone(),
                Transport::Http => secret_key_for(name, &slot),
            };
            if !try_store_secret(Config::global(), &key, value)? {
                anyhow::bail!("Failed to store the secret");
            }
            credentials.env_keys.push(key.clone());

            if transport == Transport::Http {
                credentials.headers.insert(
                    slot.clone(),
                    if slot.eq_ignore_ascii_case("Authorization") {
                        format!("Bearer ${{{key}}}")
                    } else {
                        format!("${{{key}}}")
                    },
                );
            }
        }

        if !pending.is_empty() {
            continue;
        }

        let Some(more) = cancellable(
            cliclack::confirm("Add another one?")
                .initial_value(false)
                .interact(),
        )?
        else {
            forget_secrets(&credentials.env_keys);
            return Ok(None);
        };
        if !more {
            break;
        }
    }

    Ok(Some(credentials))
}

/// stdio servers read their secrets from the environment, remote ones from a
/// header, so the question differs but the storage does not.
fn ask_credential_slot(transport: Transport, already: usize) -> Result<Option<String>> {
    let prompt = match transport {
        Transport::Stdio => "Environment variable the server reads",
        Transport::Http => "Header the server expects",
    };
    let mut input = cliclack::input(prompt).validate(|input: &String| {
        if input.trim().is_empty() {
            Err("Enter a name")
        } else {
            Ok(())
        }
    });
    input = match transport {
        Transport::Stdio => input.placeholder("GITHUB_TOKEN"),
        Transport::Http if already == 0 => input.default_input("Authorization"),
        Transport::Http => input.placeholder("X-API-Key"),
    };

    let Some(slot) = cancellable(input.interact())? else {
        return Ok(None);
    };
    let slot: String = slot;
    Ok(Some(slot.trim().to_string()))
}

/// A server the user walked away from must not leave its tokens behind.
fn forget_secrets(keys: &[String]) {
    for key in keys {
        let _ = Config::global().delete_secret(key);
    }
}

fn snapshot_secrets(keys: &[String]) -> Vec<(String, String)> {
    keys.iter()
        .filter_map(|key| {
            Config::global()
                .get_secret::<String>(key)
                .ok()
                .map(|value| (key.clone(), value))
        })
        .collect()
}

fn restore_secrets(saved: &[(String, String)]) {
    for (key, value) in saved {
        let _ = Config::global().set_secret(key, value);
    }
}

#[derive(Default)]
struct Credentials {
    env_keys: Vec<String>,
    headers: HashMap<String, String>,
}

fn build_config(
    name: &str,
    target: &str,
    transport: Transport,
    credentials: Credentials,
) -> Result<ExtensionConfig> {
    let Credentials { env_keys, headers } = credentials;

    Ok(match transport {
        Transport::Http => ExtensionConfig::StreamableHttp {
            name: name.to_string(),
            uri: target.to_string(),
            envs: Envs::new(HashMap::new()),
            env_keys,
            headers,
            description: target.to_string(),
            timeout: Some(DEFAULT_EXTENSION_TIMEOUT),
            socket: None,
            client_id: None,
            client_secret_key: None,
            scopes: Vec::new(),
            bundled: None,
            available_tools: Vec::new(),
        },
        Transport::Stdio => {
            let mut parts = goose::utils::split_command_args(target)?;
            let mut envs = HashMap::new();
            while let Some(part) = parts.first() {
                if !part.contains('=') {
                    break;
                }
                let assignment = parts.remove(0);
                let (key, value) = assignment.split_once('=').unwrap();
                envs.insert(key.to_string(), value.to_string());
            }
            if parts.is_empty() {
                anyhow::bail!("No command to run in {target:?}");
            }

            ExtensionConfig::Stdio {
                name: name.to_string(),
                cmd: parts.remove(0),
                args: parts,
                envs: Envs::new(envs),
                env_keys,
                description: target.to_string(),
                timeout: Some(DEFAULT_EXTENSION_TIMEOUT),
                cwd: None,
                bundled: None,
                available_tools: Vec::new(),
            }
        }
    })
}

/// Start the server in a throwaway manager: a successful add means the MCP
/// handshake completed. Runs the same way with or without a live session.
async fn probe(config: &ExtensionConfig) -> ProbeOutcome {
    let probe_config = with_timeout(config.clone(), PROBE_TIMEOUT.as_secs());

    let outcome = tokio::select! {
        result = tokio::time::timeout(PROBE_TIMEOUT, connect_and_list(probe_config)) => match result {
            Ok(Ok(tools)) => ProbeOutcome::Connected(tools),
            Ok(Err(e)) => ProbeOutcome::Failed(readable_error(&e.to_string())),
            Err(_) => ProbeOutcome::Failed(format!(
                "no answer within {} seconds",
                PROBE_TIMEOUT.as_secs()
            )),
        },
        _ = tokio::signal::ctrl_c() => ProbeOutcome::Interrupted,
    };

    match outcome {
        ProbeOutcome::Failed(error) => {
            ProbeOutcome::Failed(match speaks_legacy_sse(config).await {
                true => LEGACY_SSE_MESSAGE.to_string(),
                false => error,
            })
        }
        other => other,
    }
}

/// Both protocols answer a GET with an event stream, so the status line tells
/// them apart only by what comes first: the old one opens with the `endpoint`
/// event naming the URL to post to, the current one has no such event.
async fn speaks_legacy_sse(config: &ExtensionConfig) -> bool {
    let ExtensionConfig::StreamableHttp {
        uri,
        headers,
        env_keys,
        ..
    } = config
    else {
        return false;
    };

    let Ok(client) = reqwest::Client::builder()
        .timeout(SSE_CHECK_TIMEOUT)
        .build()
    else {
        return false;
    };

    let mut request = client
        .get(resolve_secrets(uri, env_keys))
        .header(reqwest::header::ACCEPT, "text/event-stream");
    for (name, value) in headers {
        request = request.header(name, resolve_secrets(value, env_keys));
    }

    let Ok(Ok(response)) = tokio::time::timeout(SSE_CHECK_TIMEOUT, request.send()).await else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }

    let mut stream = response.bytes_stream();
    let mut seen = String::new();
    let read = tokio::time::timeout(SSE_CHECK_TIMEOUT, async {
        while let Some(Ok(chunk)) = stream.next().await {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("event: endpoint") {
                return true;
            }
            if seen.len() > 4096 {
                break;
            }
        }
        false
    })
    .await;

    read.unwrap_or(false)
}

/// Headers and URLs keep `${KEY}` references so the config stays free of
/// plaintext; a request made here has to resolve them the same way the agent does.
fn resolve_secrets(value: &str, keys: &[String]) -> String {
    if keys.is_empty() {
        return value.to_string();
    }

    let config = Config::global();
    let mut resolved = value.to_string();
    for key in keys {
        if let Ok(secret) = config.get_secret::<String>(key) {
            resolved = resolved.replace(&format!("${{{key}}}"), &secret);
        }
    }
    resolved
}

/// Returns the tool names as the server publishes them, without the extension
/// prefix markov adds, because that is what the allow list is matched against.
async fn connect_and_list(config: ExtensionConfig) -> Result<Vec<String>> {
    let manager = Arc::new(ExtensionManager::new_without_provider(Paths::data_dir()));
    manager
        .add_extension(config.clone(), None, None, None)
        .await?;

    let prefix = format!("{}__", config.key());
    let tools = manager
        .get_prefixed_tools(PROBE_SESSION, Some(config.name()))
        .await
        .map(|tools| {
            tools
                .into_iter()
                .map(|tool| {
                    tool.name
                        .strip_prefix(&prefix)
                        .unwrap_or(&tool.name)
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default();

    let _ = manager.remove_extension(&config.name()).await;
    Ok(tools)
}

/// Transport errors arrive wrapped in the rmcp type that produced them; the part
/// worth reading is what the server actually said.
fn readable_error(error: &str) -> String {
    let message = match error.rsplit_once("] error: ") {
        Some((_, tail)) => tail,
        None => error,
    };
    message
        .trim_end_matches(", when send initialize request")
        .trim()
        .to_string()
}

/// The saved server keeps its own timeout; the check does not, or a server that
/// never answers holds the form for five minutes.
fn with_timeout(config: ExtensionConfig, seconds: u64) -> ExtensionConfig {
    let mut config = config;
    set_timeout(&mut config, seconds);
    config
}

fn set_timeout(config: &mut ExtensionConfig, seconds: u64) {
    match config {
        ExtensionConfig::Stdio { timeout, .. }
        | ExtensionConfig::StreamableHttp { timeout, .. }
        | ExtensionConfig::Builtin { timeout, .. } => *timeout = Some(seconds),
        _ => {}
    }
}

fn timeout_of(config: &ExtensionConfig) -> u64 {
    match config {
        ExtensionConfig::Stdio { timeout, .. }
        | ExtensionConfig::StreamableHttp { timeout, .. }
        | ExtensionConfig::Builtin { timeout, .. } => timeout.unwrap_or(DEFAULT_EXTENSION_TIMEOUT),
        _ => DEFAULT_EXTENSION_TIMEOUT,
    }
}

fn set_allowed_tools(config: &mut ExtensionConfig, tools: Vec<String>) {
    match config {
        ExtensionConfig::Stdio {
            available_tools, ..
        }
        | ExtensionConfig::StreamableHttp {
            available_tools, ..
        }
        | ExtensionConfig::Builtin {
            available_tools, ..
        } => *available_tools = tools,
        _ => {}
    }
}

fn allowed_tools(config: &ExtensionConfig) -> Vec<String> {
    match config {
        ExtensionConfig::Stdio {
            available_tools, ..
        }
        | ExtensionConfig::StreamableHttp {
            available_tools, ..
        }
        | ExtensionConfig::Builtin {
            available_tools, ..
        } => available_tools.clone(),
        _ => Vec::new(),
    }
}

fn credential_keys(config: &ExtensionConfig) -> Vec<String> {
    match config {
        ExtensionConfig::Stdio { env_keys, .. }
        | ExtensionConfig::StreamableHttp { env_keys, .. } => env_keys.clone(),
        _ => Vec::new(),
    }
}

/// What the server calls each secret, which is not always what we call it: a
/// stdio server names its own environment variables, while a remote one is asked
/// for header names and the storage key is derived from them. Asking again with
/// the storage keys would put a derived name in front of a person and then
/// derive a second one from it.
fn credential_slots(config: &ExtensionConfig) -> Vec<String> {
    match config {
        ExtensionConfig::Stdio { env_keys, .. } => env_keys.clone(),
        ExtensionConfig::StreamableHttp { headers, .. } => {
            let mut names: Vec<String> = headers.keys().cloned().collect();
            names.sort();
            names
        }
        _ => Vec::new(),
    }
}

fn pick_server(prompt: &str, servers: &[ExtensionEntry]) -> Result<Option<ExtensionEntry>> {
    let items = server_items(servers);
    let Some(key) = cancellable(select(prompt, &items).interact())? else {
        return Ok(None);
    };

    Ok(servers
        .iter()
        .find(|entry| entry.config.key() == key)
        .cloned())
}

fn server_items(servers: &[ExtensionEntry]) -> Vec<(String, String, String)> {
    servers
        .iter()
        .map(|entry| {
            (
                entry.config.key(),
                entry.config.name(),
                target_of(&entry.config),
            )
        })
        .collect()
}

fn configured_servers() -> Vec<ExtensionEntry> {
    get_all_extensions()
        .into_iter()
        .filter(|entry| entry.config.kind() == ExtensionKind::Mcp)
        .collect()
}

fn describe(entry: &ExtensionEntry) -> String {
    let state = if entry.enabled { "on" } else { "off" };
    format!("{} ({})", entry.config.name(), state)
}

fn transport_label(config: &ExtensionConfig) -> &'static str {
    match config {
        ExtensionConfig::Stdio { .. } => "local command",
        ExtensionConfig::StreamableHttp { .. } => "remote (streamable http)",
        ExtensionConfig::Sse { .. } => "remote (sse, unsupported)",
        _ => "other",
    }
}

fn transport_of(config: &ExtensionConfig) -> Transport {
    match config {
        ExtensionConfig::Stdio { .. } => Transport::Stdio,
        _ => Transport::Http,
    }
}

fn target_of(config: &ExtensionConfig) -> String {
    match config {
        ExtensionConfig::Stdio {
            cmd, args, envs, ..
        } => {
            let mut parts: Vec<String> = envs
                .get_env()
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect();
            parts.sort();
            parts.push(cmd.clone());
            parts.extend(args.iter().cloned());
            parts.join(" ")
        }
        ExtensionConfig::StreamableHttp { uri, .. } => uri.clone(),
        ExtensionConfig::Sse { uri, .. } => uri.clone().unwrap_or_default(),
        other => other.name(),
    }
}

/// `name_to_key` replaces everything outside `[a-z0-9_-]` with an underscore, so
/// a name written in another script collapses to a row of underscores: unusable
/// as a tool prefix and colliding with the next such name of the same length.
fn is_usable_key(key: &str) -> bool {
    key.chars().any(|c| c.is_ascii_alphanumeric())
}

fn secret_key_for(name: &str, slot: &str) -> String {
    let shout = |text: &str| {
        text.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>()
    };

    if slot.eq_ignore_ascii_case("Authorization") {
        format!("MCP_{}_TOKEN", shout(&name_to_key(name)))
    } else {
        format!("MCP_{}_{}", shout(&name_to_key(name)), shout(slot))
    }
}

fn detect_transport(target: &str) -> Transport {
    match target.split_whitespace().next() {
        Some(first) if first.starts_with("http://") || first.starts_with("https://") => {
            Transport::Http
        }
        _ => Transport::Stdio,
    }
}

/// Naming a server after one of these would say nothing: `docker run <image>`,
/// `markov mcp <server>`.
const RUNNER_VERBS: &[&str] = &["run", "exec", "serve", "start", "mcp"];

/// A name people would recognise: the host for a URL, the package for a command.
/// The binary is a poor name because every npx server would be called `npx`.
fn derive_name(target: &str, transport: Transport) -> String {
    match transport {
        Transport::Http => url::Url::parse(target)
            .ok()
            .and_then(|url| {
                let host = url.host_str()?.trim_start_matches("www.").to_string();
                if host.is_empty() {
                    return None;
                }
                // Several servers on one host differ only by port.
                Some(match url.port() {
                    Some(port) => format!("{host}-{port}"),
                    None => host,
                })
            })
            .unwrap_or_else(|| "remote-server".to_string()),
        Transport::Stdio => {
            let parts = goose::utils::split_command_args(target).unwrap_or_default();
            let mut parts = parts.into_iter().skip_while(|part| part.contains('='));

            let Some(cmd) = parts.next() else {
                return "server".to_string();
            };
            let package =
                parts.find(|part| !part.starts_with('-') && !RUNNER_VERBS.contains(&part.as_str()));

            let candidate = package.unwrap_or(cmd);
            let candidate = candidate.rsplit('/').next().unwrap_or(&candidate);
            let candidate = candidate
                .split('@')
                .find(|part| !part.is_empty())
                .unwrap_or(candidate);

            if candidate.is_empty() {
                "server".to_string()
            } else {
                candidate.to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn a_url_is_a_remote_server_and_anything_else_is_a_command() {
        assert_eq!(
            detect_transport("https://mcp.example.com/mcp"),
            Transport::Http
        );
        assert_eq!(
            detect_transport("http://localhost:8000/mcp"),
            Transport::Http
        );
        assert_eq!(detect_transport("npx -y @scope/server"), Transport::Stdio);
        assert_eq!(detect_transport("uvx mcp-server-git"), Transport::Stdio);
    }

    #[test]
    fn a_query_string_does_not_turn_a_url_into_a_command() {
        assert_eq!(
            detect_transport("https://mcp.example.com/mcp?key=value"),
            Transport::Http
        );
    }

    #[test]
    fn an_environment_assignment_in_front_still_reads_as_a_command() {
        assert_eq!(
            detect_transport("HTTP_PROXY=http://proxy node server.js"),
            Transport::Stdio
        );
    }

    #[test]
    fn a_remote_server_is_named_after_its_host() {
        assert_eq!(
            derive_name("https://mcp.example.com/mcp", Transport::Http),
            "mcp.example.com"
        );
        assert_eq!(
            derive_name("https://www.example.com/mcp", Transport::Http),
            "example.com"
        );
    }

    #[test]
    fn a_port_stays_in_the_name_so_two_local_servers_do_not_clash() {
        assert_eq!(
            derive_name("http://127.0.0.1:8931/mcp", Transport::Http),
            "127.0.0.1-8931"
        );
        assert_eq!(
            derive_name("http://localhost:9000/mcp", Transport::Http),
            "localhost-9000"
        );
    }

    #[test]
    fn a_command_is_named_after_its_package_not_its_runner() {
        assert_eq!(
            derive_name(
                "npx -y @modelcontextprotocol/server-github",
                Transport::Stdio
            ),
            "server-github"
        );
        assert_eq!(
            derive_name("uvx mcp-server-git", Transport::Stdio),
            "mcp-server-git"
        );
        assert_eq!(
            derive_name("npx -y mcp-server-git@0.1.2", Transport::Stdio),
            "mcp-server-git"
        );
    }

    #[test]
    fn a_runner_subcommand_is_not_a_name() {
        assert_eq!(
            derive_name("/usr/local/bin/markov mcp tutorial", Transport::Stdio),
            "tutorial"
        );
        assert_eq!(
            derive_name("docker run -i --rm ghcr.io/acme/wiki-mcp", Transport::Stdio),
            "wiki-mcp"
        );
    }

    #[test]
    fn a_command_without_arguments_falls_back_to_the_binary() {
        assert_eq!(
            derive_name("/usr/local/bin/my-mcp", Transport::Stdio),
            "my-mcp"
        );
        assert_eq!(
            derive_name("FOO=bar /usr/local/bin/my-mcp", Transport::Stdio),
            "my-mcp"
        );
    }

    #[test]
    fn a_transport_error_is_reduced_to_what_the_server_said() {
        let raw = "failed to initialize MCP client: Send message error Transport \
                   [rmcp::transport::worker::WorkerTransport<rmcp::transport::streamable_http_client::\
                   StreamableHttpClientWorker<reqwest::async_impl::client::Client>>] error: \
                   unexpected server response: HTTP 401 Unauthorized: {\"error\":\"wrong token\"}, \
                   when send initialize request";

        assert_eq!(
            readable_error(raw),
            "unexpected server response: HTTP 401 Unauthorized: {\"error\":\"wrong token\"}"
        );
    }

    #[test]
    fn an_error_without_transport_noise_is_left_alone() {
        assert_eq!(
            readable_error("process quit before initialization: stderr = mock: dying on purpose"),
            "process quit before initialization: stderr = mock: dying on purpose"
        );
    }

    #[test]
    fn a_name_that_leaves_no_usable_key_is_rejected() {
        assert!(!is_usable_key(&name_to_key("Поиск по вики")));
        assert!(!is_usable_key(&name_to_key("...")));
        assert!(is_usable_key(&name_to_key("wiki-search")));
        assert!(is_usable_key(&name_to_key("Поиск wiki")));
    }

    #[test]
    fn every_header_gets_its_own_secret_key() {
        assert_eq!(secret_key_for("wiki", "Authorization"), "MCP_WIKI_TOKEN");
        assert_eq!(secret_key_for("wiki", "X-API-Key"), "MCP_WIKI_X_API_KEY");
        assert_eq!(
            secret_key_for("mcp.example.com", "Authorization"),
            "MCP_MCP_EXAMPLE_COM_TOKEN"
        );
    }

    #[test]
    fn a_token_becomes_a_named_key_and_never_a_literal_value() {
        let credentials = Credentials {
            env_keys: vec!["MCP_WIKI_TOKEN".to_string()],
            headers: HashMap::from([(
                "Authorization".to_string(),
                "Bearer ${MCP_WIKI_TOKEN}".to_string(),
            )]),
        };

        let config =
            build_config("wiki", "https://wiki/mcp", Transport::Http, credentials).unwrap();

        let ExtensionConfig::StreamableHttp {
            env_keys, headers, ..
        } = &config
        else {
            panic!("expected a streamable http config");
        };
        assert_eq!(env_keys, &["MCP_WIKI_TOKEN"]);
        assert_eq!(headers["Authorization"], "Bearer ${MCP_WIKI_TOKEN}");
    }

    #[test]
    fn a_command_keeps_its_leading_environment_assignments() {
        let config = build_config(
            "local",
            "FOO=bar node server.js --port 1",
            Transport::Stdio,
            Credentials::default(),
        )
        .unwrap();

        let ExtensionConfig::Stdio {
            cmd, args, envs, ..
        } = &config
        else {
            panic!("expected a stdio config");
        };
        assert_eq!(cmd, "node");
        assert_eq!(args, &["server.js", "--port", "1"]);
        assert_eq!(envs.get_env().get("FOO").map(String::as_str), Some("bar"));
    }

    #[test]
    fn a_stdio_target_round_trips_through_editing() {
        let config = build_config(
            "local",
            "FOO=bar node server.js",
            Transport::Stdio,
            Credentials::default(),
        )
        .unwrap();

        assert_eq!(target_of(&config), "FOO=bar node server.js");
    }

    #[test]
    fn a_remote_server_is_asked_for_its_headers_and_a_local_one_for_its_variables() {
        let stdio = ExtensionConfig::Stdio {
            name: "local".to_string(),
            description: String::new(),
            cmd: "node".to_string(),
            args: Vec::new(),
            envs: Envs::default(),
            env_keys: vec!["TOKEN".to_string()],
            timeout: None,
            bundled: None,
            available_tools: Vec::new(),
            cwd: None,
        };
        assert_eq!(credential_slots(&stdio), ["TOKEN"]);

        let remote = ExtensionConfig::StreamableHttp {
            name: "atlas".to_string(),
            description: String::new(),
            uri: "https://example.test/mcp".to_string(),
            envs: Envs::default(),
            env_keys: vec!["mcp_atlas_authorization".to_string()],
            headers: HashMap::from([
                ("X-Tenant".to_string(), "${mcp_atlas_x_tenant}".to_string()),
                (
                    "Authorization".to_string(),
                    "Bearer ${mcp_atlas_authorization}".to_string(),
                ),
            ]),
            timeout: None,
            bundled: None,
            available_tools: Vec::new(),
            socket: None,
            client_id: None,
            client_secret_key: None,
            scopes: Vec::new(),
        };
        // The header names, in a fixed order, and never the derived storage keys.
        assert_eq!(credential_slots(&remote), ["Authorization", "X-Tenant"]);
    }

    #[test]
    fn timeout_and_tool_limits_survive_a_rebuild() {
        let mut config = build_config(
            "local",
            "node server.js",
            Transport::Stdio,
            Credentials::default(),
        )
        .unwrap();

        set_timeout(&mut config, 45);
        set_allowed_tools(&mut config, vec!["search".to_string()]);

        assert_eq!(timeout_of(&config), 45);
        assert_eq!(allowed_tools(&config), vec!["search".to_string()]);
    }

    /// Answers one request with `response` and holds the connection open, the way
    /// a server keeps an event stream running.
    async fn serve_once(response: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("http://{}/sse", listener.local_addr().unwrap());

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            let _ = socket.write_all(response.as_bytes()).await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        });

        uri
    }

    fn remote(uri: &str) -> ExtensionConfig {
        build_config("probe", uri, Transport::Http, Credentials::default()).unwrap()
    }

    #[tokio::test]
    async fn the_endpoint_event_gives_the_old_protocol_away() {
        let uri = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n\
             event: endpoint\ndata: /messages?session=1\n\n",
        )
        .await;

        assert!(speaks_legacy_sse(&remote(&uri)).await);
    }

    #[tokio::test]
    async fn a_server_that_refuses_the_get_is_not_diagnosed() {
        let uri = serve_once("HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n").await;

        assert!(!speaks_legacy_sse(&remote(&uri)).await);
    }

    #[tokio::test]
    async fn a_local_command_is_never_diagnosed_as_sse() {
        let config = build_config(
            "probe",
            "npx -y @scope/server",
            Transport::Stdio,
            Credentials::default(),
        )
        .unwrap();

        assert!(!speaks_legacy_sse(&config).await);
    }
}
