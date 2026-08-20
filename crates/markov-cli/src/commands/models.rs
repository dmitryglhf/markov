//! Interactive choice of provider and model.
//!
//! The dialog picks, it does not apply. A live session has to route the pick
//! through its agent, and `markov model` has nowhere to put it but the config
//! file, so both callers finish the job themselves.
//!
//! What the form spends most of its screen on is not the list but the question
//! underneath it: three layers decide which model answers — the environment, the
//! session, and `config.yaml` — and until now nothing said which one was
//! winning.

use anyhow::Result;
use console::style;
use goose::config::{get_default_provider, get_provider_entry, set_active_provider, Config};
use goose::providers::base::{ModelInfo, ProviderMetadata};
use goose::providers::inventory::{ProviderInventoryEntry, ProviderInventoryService};
use goose::providers::{create, providers};
use goose::session::SessionManager;
use goose_providers::thinking::ThinkingEffort;

use crate::ui::{cancellable, is_cancel, require_terminal, select};
use goose_cli::commands::configure::{ensure_credentials, fetch_and_select_model};
use goose_cli::markov::types::{Choice, Current};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Model,
    Provider,
    Effort,
    Default,
    Details,
    Done,
}

const EFFORTS: [(ThinkingEffort, &str); 5] = [
    (ThinkingEffort::Off, "Off - No extended thinking"),
    (
        ThinkingEffort::Low,
        "Low - Better latency, lighter reasoning",
    ),
    (ThinkingEffort::Medium, "Medium - Moderate thinking"),
    (ThinkingEffort::High, "High - Deep reasoning"),
    (
        ThinkingEffort::Max,
        "Max - No constraints on thinking depth",
    ),
];

pub async fn models_dialog(current: Current) -> Result<Option<Choice>> {
    require_terminal("markov model")?;
    cliclack::intro(style(" markov-model ").on_cyan().black())?;

    let mut catalogue = Catalogue::load().await;
    let mut picked = Choice {
        provider: current.provider.clone(),
        model: current.model.clone(),
    };
    let mut changed = false;

    loop {
        cliclack::log::info(header(&picked, current.in_session, &Layers::read()))?;

        let mut menu = cliclack::select("What would you like to do?")
            .item(
                Action::Model,
                "Switch model",
                match picked.provider.is_empty() {
                    true => "choose a provider first".to_string(),
                    false => format!("within {}", picked.provider),
                },
            )
            .item(Action::Provider, "Switch provider", catalogue.tally())
            .item(Action::Effort, "Thinking effort", effort_hint());
        if current.in_session {
            menu = menu.item(
                Action::Default,
                "Make this the default",
                format!(
                    "write {} / {} to config.yaml",
                    picked.provider, picked.model
                ),
            );
        }
        menu = menu
            .item(
                Action::Details,
                "Details",
                "Context window, price, and which layer decides",
            )
            .item(Action::Done, "Done", "Leave the manager");

        let Some(action) = cancellable(menu.interact())? else {
            break;
        };

        // Reachable on a machine that has never been configured: without a
        // provider there is no model list to open and nothing to describe.
        if picked.provider.is_empty() && !matches!(action, Action::Provider | Action::Done) {
            cliclack::log::warning("Choose a provider first")?;
            continue;
        }

        match action {
            Action::Model => {
                changed |= switch_model(&mut picked, &catalogue, current.in_session).await?
            }
            Action::Provider => {
                changed |= switch_provider(&mut picked, &mut catalogue, current.in_session).await?
            }
            Action::Effort => changed |= choose_effort(&picked)?,
            Action::Default => set_default(&picked)?,
            Action::Details => details(&picked, current.in_session, &catalogue)?,
            Action::Done => break,
        }
    }

    // Outside a session there is nowhere else for a pick to go, so Done writes
    // it; inside one the file is only touched by the menu item that says so.
    if !current.in_session && picked != Choice::from(&current) && !picked.provider.is_empty() {
        set_default(&picked)?;
    }

    match changed {
        true => cliclack::outro(format!(
            "{} / {}",
            style(&picked.provider).green(),
            style(&picked.model).green()
        ))?,
        false => cliclack::outro("Nothing changed")?,
    }
    Ok(changed.then_some(picked))
}

/// `markov model` outside a session: the same form, and the config file is the
/// only place its answer can land.
pub async fn handle_model_command() -> Result<()> {
    let config = Config::global();
    models_dialog(Current {
        provider: goose::config::get_active_provider(config).unwrap_or_default(),
        model: goose::config::get_active_model(config).unwrap_or_default(),
        in_session: false,
    })
    .await
    .map(|_| ())
}

/// Everything the form knows about providers before it talks to any of them.
/// The inventory is the only place that gets "is this one usable" right for all
/// of OAuth, declarative and plain key-based providers.
struct Catalogue {
    entries: Vec<ProviderInventoryEntry>,
    metadata: Vec<ProviderMetadata>,
}

impl Catalogue {
    async fn load() -> Self {
        let metadata: Vec<ProviderMetadata> = providers()
            .await
            .into_iter()
            .map(|(meta, _)| meta)
            .collect();
        let ids: Vec<String> = metadata.iter().map(|meta| meta.name.clone()).collect();
        let inventory = ProviderInventoryService::new(SessionManager::instance().storage().clone());
        let entries = inventory.entries(&ids).await.unwrap_or_default();
        Catalogue { entries, metadata }
    }

    fn entry(&self, name: &str) -> Option<&ProviderInventoryEntry> {
        self.entries.iter().find(|entry| entry.provider_id == name)
    }

    fn meta(&self, name: &str) -> Option<&ProviderMetadata> {
        self.metadata.iter().find(|meta| meta.name == name)
    }

    fn configured(&self, name: &str) -> bool {
        self.entry(name).is_some_and(|entry| entry.configured)
    }

    fn tally(&self) -> String {
        let configured = self.entries.iter().filter(|entry| entry.configured).count();
        format!("{configured} configured, {} available", self.entries.len())
    }

    fn model_info(&self, provider: &str, model: &str) -> Option<&ModelInfo> {
        self.meta(provider)?
            .known_models
            .iter()
            .find(|known| known.name == model)
    }

    fn context_limit(&self, provider: &str, model: &str) -> Option<usize> {
        self.entry(provider)
            .and_then(|entry| entry.models.iter().find(|known| known.id == model))
            .and_then(|known| known.context_limit)
            .or_else(|| {
                self.model_info(provider, model)
                    .map(|info| info.context_limit)
            })
    }
}

struct Layers {
    env_provider: Option<String>,
    env_model: Option<String>,
    default_provider: Option<String>,
    default_model: Option<String>,
}

impl Layers {
    fn read() -> Self {
        let config = Config::global();
        let default_provider = get_default_provider(config);
        let default_model = default_provider
            .as_deref()
            .and_then(|name| get_provider_entry(config, name))
            .map(|entry| entry.model)
            .filter(|model| !model.is_empty());
        Layers {
            env_provider: env_value("GOOSE_PROVIDER"),
            env_model: env_value("GOOSE_MODEL"),
            default_provider,
            default_model,
        }
    }

    fn default_pair(&self) -> Option<(&str, &str)> {
        Some((
            self.default_provider.as_deref()?,
            self.default_model.as_deref()?,
        ))
    }

    /// The variables that are actually set, named so a reader can go and unset
    /// the right one.
    fn env_names(&self) -> Option<String> {
        let names: Vec<&str> = [
            self.env_provider.as_ref().map(|_| "GOOSE_PROVIDER"),
            self.env_model.as_ref().map(|_| "GOOSE_MODEL"),
        ]
        .into_iter()
        .flatten()
        .collect();
        match names.is_empty() {
            true => None,
            false => Some(names.join(" and ")),
        }
    }
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// Which of the three layers the value on screen actually came from. A session
/// was handed its model at start-up and owns it from then on; outside one the
/// environment is read ahead of the file.
fn deciding_layer(in_session: bool, layers: &Layers) -> &'static str {
    match (in_session, layers.env_names().is_some()) {
        (true, _) => "this session",
        (false, true) => "the environment",
        (false, false) => "config.yaml",
    }
}

fn header(picked: &Choice, in_session: bool, layers: &Layers) -> String {
    let mut lines = match picked.provider.is_empty() {
        true => vec!["no provider chosen yet".to_string()],
        false => vec![format!("{} / {}", picked.provider, picked.model)],
    };

    let env = layers.env_names();
    lines.push(match layers.default_pair() {
        None => "nothing saved in config.yaml yet".to_string(),
        Some((provider, model)) => {
            let same = (provider, model) == (&*picked.provider, &*picked.model);
            match (in_session, env.is_some(), same) {
                (true, _, true) => "this session, matching config.yaml".to_string(),
                (true, _, false) => {
                    format!("this session only, config.yaml says {provider} / {model}")
                }
                (false, true, _) => {
                    format!("from the environment, config.yaml says {provider} / {model}")
                }
                (false, false, true) => "saved in config.yaml".to_string(),
                (false, false, false) => {
                    format!("not saved yet, config.yaml says {provider} / {model}")
                }
            }
        }
    });

    if let Some(names) = env {
        lines.push(format!("{names} is what set it, ahead of the file"));
    }

    lines.join("\n")
}

fn effort_hint() -> String {
    match Config::global().get_goose_thinking_effort() {
        Some(effort) => format!("{effort}, and it applies to every provider"),
        None => "not set, and it applies to every provider".to_string(),
    }
}

/// A provider that picks its own model has nothing for us to list, and saying so
/// is more use than an empty prompt.
fn selection_hint(catalogue: &Catalogue, provider: &str) -> Option<String> {
    catalogue
        .entry(provider)
        .and_then(|entry| entry.model_selection_hint.clone())
}

async fn switch_model(
    picked: &mut Choice,
    catalogue: &Catalogue,
    in_session: bool,
) -> Result<bool> {
    if let Some(hint) = selection_hint(catalogue, &picked.provider) {
        cliclack::log::info(hint)?;
        return Ok(false);
    }

    let Some(meta) = catalogue.meta(&picked.provider) else {
        cliclack::log::warning(format!("Unknown provider {}", picked.provider))?;
        return Ok(false);
    };

    let provider = match create(&picked.provider, Vec::new()).await {
        Ok(provider) => provider,
        Err(e) => {
            cliclack::log::error(format!("Cannot reach {}: {e}", picked.provider))?;
            return Ok(false);
        }
    };

    // Only the built provider knows this, and it is the other half of what
    // `apply_model_switch` refuses.
    if in_session && provider.manages_own_context() && !provider.accepts_conversation_handoff() {
        cliclack::log::warning(format!(
            "{} keeps its own conversation context and cannot take this one over, \
             so the session would carry on as if nothing had been said. \
             Run `markov model` outside a session to make it the default.",
            picked.provider
        ))?;
        return Ok(false);
    }

    let model = match fetch_and_select_model(&provider, meta).await {
        Ok(model) => model,
        Err(e) if is_cancel(&e) => return Ok(false),
        Err(e) => {
            cliclack::log::error(e.to_string())?;
            return Ok(false);
        }
    };

    if model == picked.model {
        return Ok(false);
    }
    picked.model = model;
    Ok(true)
}

async fn switch_provider(
    picked: &mut Choice,
    catalogue: &mut Catalogue,
    in_session: bool,
) -> Result<bool> {
    let items = provider_items(catalogue, &picked.provider);
    let Some(chosen) = cancellable(select("Which provider?", &items).interact())? else {
        return Ok(false);
    };

    if !catalogue.configured(&chosen) {
        if !ensure_credentials(&chosen).await? {
            return Ok(false);
        }
        // The key just written is what makes the provider usable, and the answer
        // is cached, so ask again rather than offer the same key twice.
        *catalogue = Catalogue::load().await;
    }

    let previous = std::mem::replace(
        picked,
        Choice {
            provider: chosen,
            model: String::new(),
        },
    );
    picked.model = match selection_hint(catalogue, &picked.provider) {
        Some(hint) => {
            cliclack::log::info(hint)?;
            catalogue
                .entry(&picked.provider)
                .map(|entry| entry.default_model.clone())
                .unwrap_or_default()
        }
        None => String::new(),
    };

    if picked.model.is_empty() && !switch_model(picked, catalogue, in_session).await? {
        *picked = previous;
        return Ok(false);
    }
    Ok(*picked != previous)
}

/// Copies of a provider name mean nothing here, so the label carries what does:
/// whether it can be used at all, and which one is in hand.
fn provider_items(catalogue: &Catalogue, current: &str) -> Vec<(String, String, String)> {
    let mut entries: Vec<&ProviderInventoryEntry> = catalogue.entries.iter().collect();
    entries.sort_by_key(|entry| (!entry.configured, entry.provider_name.to_lowercase()));
    entries
        .into_iter()
        .map(|entry| {
            (
                entry.provider_id.clone(),
                provider_label(entry, current),
                entry.description.clone(),
            )
        })
        .collect()
}

fn provider_label(entry: &ProviderInventoryEntry, current: &str) -> String {
    let note = match (entry.provider_id == current, entry.configured) {
        (true, _) => " (current)",
        (false, true) => "",
        (false, false) => " (needs a key)",
    };
    format!("{}{note}", entry.provider_name)
}

fn choose_effort(picked: &Choice) -> Result<bool> {
    let config = Config::global();
    let before = config.get_goose_thinking_effort();
    let items: Vec<(ThinkingEffort, &str, &str)> = EFFORTS
        .iter()
        .map(|(effort, label)| (*effort, *label, ""))
        .collect();

    let mut list = cliclack::select(format!("Thinking effort for {}", picked.model)).items(&items);
    if let Some(effort) = before {
        list = list.initial_value(effort);
    }
    let Some(effort) = cancellable(list.interact())? else {
        return Ok(false);
    };

    if Some(effort) == before {
        return Ok(false);
    }
    config.set_goose_thinking_effort(effort)?;
    cliclack::log::success(format!("Thinking effort set to {effort}"))?;
    Ok(true)
}

fn set_default(picked: &Choice) -> Result<()> {
    let config = Config::global();
    set_active_provider(config, &picked.provider, &picked.model)?;
    cliclack::log::success(format!(
        "{} / {} saved to {}",
        picked.provider,
        picked.model,
        config.path()
    ))?;
    Ok(())
}

fn row(label: &str, value: impl std::fmt::Display) -> String {
    format!("{label:<10} {value}")
}

/// Context windows run to seven digits, and an unbroken run of zeroes is read
/// by counting them.
fn grouped(number: usize) -> String {
    number
        .to_string()
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn details(picked: &Choice, in_session: bool, catalogue: &Catalogue) -> Result<()> {
    let layers = Layers::read();
    let mut rows = vec![
        row("Provider", display_name(catalogue, &picked.provider)),
        row("Model", &picked.model),
    ];

    if let Some(limit) = catalogue.context_limit(&picked.provider, &picked.model) {
        rows.push(row("Context", format!("{} tokens", grouped(limit))));
    }
    if let Some(price) = price(catalogue.model_info(&picked.provider, &picked.model)) {
        rows.push(row("Price", price));
    }
    if let Some(effort) = Config::global().get_goose_thinking_effort() {
        rows.push(row("Thinking", effort));
    }

    rows.push(row("In effect", deciding_layer(in_session, &layers)));
    match layers.default_pair() {
        Some((provider, model)) => rows.push(row("Default", format!("{provider} / {model}"))),
        None => rows.push(row("Default", "not set")),
    }
    for (name, value) in [
        ("GOOSE_PROVIDER", &layers.env_provider),
        ("GOOSE_MODEL", &layers.env_model),
    ] {
        if let Some(value) = value {
            rows.push(row("Env", format!("{name}={value}")));
        }
    }

    cliclack::note(&picked.model, rows.join("\n"))?;
    Ok(())
}

fn display_name(catalogue: &Catalogue, provider: &str) -> String {
    catalogue
        .meta(provider)
        .map(|meta| format!("{} ({provider})", meta.display_name))
        .unwrap_or_else(|| provider.to_string())
}

fn price(info: Option<&ModelInfo>) -> Option<String> {
    let info = info?;
    let currency = info.currency.as_deref().unwrap_or("$");
    let per_million = |cost: f64| format!("{currency}{:.2}", cost * 1_000_000.0);
    match (info.input_token_cost, info.output_token_cost) {
        (Some(input), Some(output)) => Some(format!(
            "{} in, {} out, per million tokens",
            per_million(input),
            per_million(output)
        )),
        (Some(input), None) => Some(format!("{} in, per million tokens", per_million(input))),
        (None, Some(output)) => Some(format!("{} out, per million tokens", per_million(output))),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use goose::providers::base::ProviderType;
    use goose::providers::catalog::ProviderSetupCategory;
    use goose::providers::inventory::InventoryModel;

    fn choice(provider: &str, model: &str) -> Choice {
        Choice {
            provider: provider.to_string(),
            model: model.to_string(),
        }
    }

    fn layers(default: Option<(&str, &str)>, env: Option<&str>) -> Layers {
        Layers {
            env_provider: None,
            env_model: env.map(str::to_string),
            default_provider: default.map(|(provider, _)| provider.to_string()),
            default_model: default.map(|(_, model)| model.to_string()),
        }
    }

    fn entry(id: &str, name: &str, configured: bool) -> ProviderInventoryEntry {
        ProviderInventoryEntry {
            provider_id: id.to_string(),
            provider_name: name.to_string(),
            description: String::new(),
            default_model: String::new(),
            configured,
            provider_type: ProviderType::Builtin,
            category: ProviderSetupCategory::Model,
            config_keys: Vec::new(),
            setup_steps: Vec::new(),
            supports_refresh: false,
            refreshing: false,
            models: Vec::new(),
            last_updated_at: None,
            last_refresh_attempt_at: None,
            last_refresh_error: None,
            model_selection_hint: None,
        }
    }

    #[test]
    fn a_pick_that_matches_the_file_says_so() {
        let text = header(
            &choice("anthropic", "sonnet"),
            false,
            &layers(Some(("anthropic", "sonnet")), None),
        );
        assert!(text.contains("saved in config.yaml"), "{text}");
    }

    #[test]
    fn the_environment_is_named_as_the_layer_that_decided() {
        let text = header(
            &choice("anthropic", "haiku"),
            false,
            &layers(Some(("anthropic", "sonnet")), Some("haiku")),
        );
        assert!(text.contains("from the environment"), "{text}");
        assert_eq!(
            deciding_layer(false, &layers(None, Some("haiku"))),
            "the environment"
        );
        assert_eq!(
            deciding_layer(true, &layers(None, Some("haiku"))),
            "this session"
        );
        assert_eq!(deciding_layer(false, &layers(None, None)), "config.yaml");
    }

    #[test]
    fn a_session_pick_names_what_the_file_still_says() {
        let text = header(
            &choice("anthropic", "sonnet"),
            true,
            &layers(Some(("openai", "gpt-5")), None),
        );
        assert!(text.contains("this session only"), "{text}");
        assert!(text.contains("openai / gpt-5"), "{text}");
    }

    #[test]
    fn an_empty_file_is_said_out_loud() {
        let text = header(&choice("anthropic", "sonnet"), false, &layers(None, None));
        assert!(text.contains("nothing saved in config.yaml yet"), "{text}");
    }

    #[test]
    fn the_variable_that_overrides_the_file_is_named() {
        let text = header(
            &choice("anthropic", "sonnet"),
            true,
            &layers(Some(("anthropic", "sonnet")), Some("sonnet")),
        );
        assert!(text.contains("GOOSE_MODEL"), "{text}");
        assert!(!text.contains("GOOSE_PROVIDER"), "{text}");
    }

    #[test]
    fn a_provider_without_a_key_is_marked_in_the_list() {
        assert_eq!(
            provider_label(&entry("openai", "OpenAI", false), "anthropic"),
            "OpenAI (needs a key)"
        );
        assert_eq!(
            provider_label(&entry("anthropic", "Anthropic", true), "anthropic"),
            "Anthropic (current)"
        );
    }

    #[test]
    fn usable_providers_are_offered_before_the_rest() {
        let catalogue = Catalogue {
            entries: vec![
                entry("azure", "Azure", false),
                entry("openai", "OpenAI", true),
            ],
            metadata: Vec::new(),
        };
        let labels: Vec<String> = provider_items(&catalogue, "openai")
            .into_iter()
            .map(|(_, label, _)| label)
            .collect();
        assert_eq!(labels, ["OpenAI (current)", "Azure (needs a key)"]);
    }

    #[test]
    fn the_context_window_falls_back_to_the_static_metadata() {
        let catalogue = Catalogue {
            entries: vec![ProviderInventoryEntry {
                models: vec![InventoryModel {
                    id: "fetched".to_string(),
                    name: "fetched".to_string(),
                    family: None,
                    context_limit: Some(1234),
                    reasoning: None,
                    recommended: false,
                }],
                ..entry("openai", "OpenAI", true)
            }],
            metadata: vec![ProviderMetadata::with_models(
                "openai",
                "OpenAI",
                "",
                "known",
                vec![ModelInfo::new("known", 4321)],
                "",
                Vec::new(),
            )],
        };
        assert_eq!(catalogue.context_limit("openai", "fetched"), Some(1234));
        assert_eq!(catalogue.context_limit("openai", "known"), Some(4321));
        assert_eq!(catalogue.context_limit("openai", "neither"), None);
    }

    #[test]
    fn a_context_window_is_grouped_into_threes() {
        assert_eq!(grouped(1_000_000), "1 000 000");
        assert_eq!(grouped(128_000), "128 000");
        assert_eq!(grouped(999), "999");
    }

    #[test]
    fn a_price_is_quoted_per_million_tokens() {
        let mut info = ModelInfo::new("m", 1);
        info.input_token_cost = Some(0.000003);
        info.output_token_cost = Some(0.000015);
        assert_eq!(
            price(Some(&info)).unwrap(),
            "$3.00 in, $15.00 out, per million tokens"
        );
        assert!(price(Some(&ModelInfo::new("m", 1))).is_none());
    }
}
