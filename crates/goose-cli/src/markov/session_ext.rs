//! What a live session grew that upstream's own does not have: the model and
//! extension managers reachable mid-session, the context gauge, and what an
//! interrupted turn leaves behind.
//!
//! An inherent impl only has to share a crate with its type, and privacy reaches
//! every descendant of the module that declared it — so declaring this file as a
//! child of `session` keeps the fields and the private render modules in view
//! while the methods themselves stay out of the file upstream keeps rewriting.

use super::*;

impl CliSession {
    pub(super) async fn handle_mcp(&mut self) {
        match hooks().mcp_dialog().await {
            Ok(changes) => self.apply_extension_changes(&changes).await,
            Err(e) => output::render_error(&e.to_string()),
        }
    }

    pub(super) async fn handle_extensions(&mut self) {
        match hooks().extensions_dialog() {
            Ok(changes) => self.apply_extension_changes(&changes).await,
            Err(e) => output::render_error(&e.to_string()),
        }
    }

    /// The dialog has already written the config; here we only mirror what it
    /// changed into the running agent so the tools are usable straight away.
    /// Nothing below depends on which kind of extension it was, so both managers
    /// end here and a switch means the same thing wherever it was thrown.
    pub(super) async fn apply_extension_changes(&mut self, changes: &[ExtensionChange]) {
        if changes.is_empty() {
            return;
        }

        for change in changes {
            let (name, result) = match change {
                ExtensionChange::Connected(config) | ExtensionChange::Enabled(config) => (
                    config.name(),
                    self.agent
                        .add_extension(config.clone(), &self.session_id)
                        .await
                        .map_err(anyhow::Error::from),
                ),
                ExtensionChange::Disabled(name) | ExtensionChange::Removed(name) => (
                    name.clone(),
                    self.agent.remove_extension(name, &self.session_id).await,
                ),
            };

            if let Err(e) = result {
                output::render_extension_error(&name, &e.to_string());
            }
        }

        self.invalidate_completion_cache().await;
    }

    /// The id above is only worth spelling out as a command when it will still
    /// be there tomorrow: an ephemeral session is never offered back, and one
    /// that was opened and closed has nothing to return to.
    pub(super) async fn resume_command(&self) -> Option<String> {
        if self.messages.user_visible_messages().is_empty() {
            return None;
        }

        let session = self.get_session().await.ok()?;
        matches!(session.session_type, SessionType::User)
            .then(|| format!("markov resume {}", self.session_id))
    }

    /// The manager reads the same three layers the session was built from, so it
    /// only needs to be told what is answering right now.
    pub(super) async fn handle_model_dialog(&mut self) -> Result<()> {
        let provider = self.agent.provider().await?;
        let model_config = self
            .agent
            .model_config_for_session(&self.session_id)
            .await?;

        let choice = hooks()
            .models_dialog(Current {
                provider: provider.get_name().to_string(),
                model: model_config.model_name.clone(),
                in_session: true,
            })
            .await;

        match choice {
            Ok(Some(choice)) => {
                self.apply_model_switch(&choice.provider, &choice.model)
                    .await
            }
            Ok(None) => Ok(()),
            Err(e) => {
                output::render_error(&e.to_string());
                Ok(())
            }
        }
    }

    /// Every route to another model ends here — the written command, the manager,
    /// and whatever comes after — so the rules about which switches a live
    /// session survives are stated once.
    pub(super) async fn apply_model_switch(
        &mut self,
        target_provider_name: &str,
        target_model_name: &str,
    ) -> Result<()> {
        let provider = self.agent.provider().await?;
        let current_provider_name = provider.get_name().to_string();
        let current_model_config = self
            .agent
            .model_config_for_session(&self.session_id)
            .await?;
        let current_model_name = current_model_config.model_name.clone();

        let Ok(target_entry) = goose::providers::get_from_registry(target_provider_name).await
        else {
            output::render_error(&format!("Unknown provider '{target_provider_name}'."));
            return Ok(());
        };

        // Leaving is the direction that cannot be made to work: the turns so far
        // live on the provider's side, and goose holds only what was streamed
        // back, so there is nothing to hand to whoever comes next.
        if provider.manages_own_context() {
            output::render_error(&format!(
                "Session model or provider switching is not supported for provider '{}' because it manages its own conversation context.",
                current_provider_name
            ));
            return Ok(());
        }

        let new_model_config = build_switched_model_config(
            Config::global(),
            target_provider_name,
            target_model_name,
            &current_model_config,
        )?;

        // Both configs already carry whatever the file said when they were built,
        // so compare them as they are. Falling back to today's setting on the
        // current side would hide exactly the case the manager creates: the
        // effort changed under a session that started without one.
        let new_effort = new_model_config.thinking_effort();
        let current_effort = current_model_config.thinking_effort();
        let model_unchanged = new_model_config.model_name == current_model_config.model_name;
        let provider_unchanged = target_provider_name == current_provider_name;
        if provider_unchanged && model_unchanged && new_effort == current_effort {
            output::goose_mode_message(&format!(
                "Session already using model '{}' for provider '{}'",
                current_model_name, current_provider_name
            ));
            return Ok(());
        }

        if let Some(model_info) = target_entry
            .metadata()
            .known_models
            .iter()
            .find(|m| m.name == target_model_name)
        {
            if model_info.context_limit < current_model_config.context_limit.unwrap_or(0) {
                eprintln!(
                    "{}",
                    console::style(format!(
                        "Warning: '{}' has a smaller context window ({} tokens) than the current session ({} tokens). \
                        You may need to use /compact.",
                        target_model_name,
                        model_info.context_limit,
                        current_model_config.context_limit.unwrap_or(0)
                    ))
                    .yellow()
                );
            }
        }

        let extensions = self.agent.get_extension_configs().await;
        let new_provider = match goose::providers::create(target_provider_name, extensions).await {
            Ok(p) => p,
            Err(e) => {
                output::render_error(&format!(
                    "Cannot switch to provider '{}': {}\n\
                         Set credentials via `goose configure` or the appropriate environment variable.\n\
                         Session continues with current provider '{}'.",
                    target_provider_name, e, current_provider_name
                ));
                return Ok(());
            }
        };

        // Arriving is fine as long as the conversation arrives too. A provider
        // that keeps its own history and takes none of ours would answer the
        // next question as if the session had just begun.
        if new_provider.manages_own_context() && !new_provider.accepts_conversation_handoff() {
            output::render_error(&format!(
                "Session provider switching is not supported for '{}' because it manages its own conversation context and cannot take over this one.",
                target_provider_name
            ));
            return Ok(());
        }

        // Only the provider itself knows this, and it is about to be handed over.
        let brings_own_tools = new_provider.manages_own_context();

        self.agent
            .update_provider(new_provider, new_model_config, &self.session_id)
            .await?;

        let mode = self.agent.goose_mode().await;
        self.agent.update_goose_mode(mode, &self.session_id).await?;

        self.update_completion_cache().await?;

        if provider_unchanged && model_unchanged {
            output::goose_mode_message(&format!(
                "Session reloaded '{}' with thinking effort {}",
                current_model_name,
                new_effort.map_or_else(|| "off".to_string(), |effort| effort.to_string())
            ));
        } else if provider_unchanged {
            output::goose_mode_message(&format!(
                "Session model switched from '{}' to '{}' for provider '{}'",
                current_model_name, target_model_name, current_provider_name
            ));
        } else {
            output::goose_mode_message(&format!(
                "Session switched from provider '{}' / model '{}' to provider '{}' / model '{}'",
                current_provider_name, current_model_name, target_provider_name, target_model_name
            ));
        }

        // The set of tools changes under the session, and nothing else says so:
        // only stdio and streamable_http servers are passed on, everything
        // compiled into markov is not, and the adapter answers with its own
        // configuration on top. Reachable only on a switch, since a provider
        // like this cannot be switched away from.
        if brings_own_tools {
            output::render_note(&format!(
                "{target_provider_name} runs its own tools, so markov's builtin and platform \
                 extensions stay behind. MCP servers travel; its own configuration may add \
                 more that /mcp does not list."
            ));
        }
        Ok(())
    }

    /// Skills are read from disk on every turn, so the manager needs no help
    /// from the live agent: whatever it wrote is picked up by itself.
    pub(super) async fn handle_skills(&mut self) {
        if let Err(e) = crate::markov::hooks::hooks().skills_dialog().await {
            output::render_error(&e.to_string());
        }
    }

    /// Leaves the conversation exactly as the agent stored it: the abandoned turn
    /// is marked as such on the next reply, so nothing has to be patched here.
    pub(super) fn mark_interrupted(&mut self) {
        {
            let mut cache = self.completion_cache.write().unwrap();
            cache.hint_status = HintStatus::Interrupted;
        }
        output::render_interrupted();
    }

    /// A gateway is the only place that knows which models it serves, so ask the
    /// provider of this session for its list. Detached on purpose: the prompt
    /// must not wait on the network, and completion keeps answering from the
    /// cache until the list arrives.
    pub(super) fn refresh_provider_models(
        &self,
        provider: Arc<dyn Provider>,
        provider_name: String,
    ) {
        let cache = Arc::clone(&self.completion_cache);
        tokio::spawn(async move {
            let models = match provider.fetch_supported_models().await {
                Ok(models) if !models.is_empty() => models,
                Ok(_) => return,
                Err(err) => {
                    tracing::debug!("could not fetch models of {provider_name}: {err}");
                    return;
                }
            };

            let mut cache = cache.write().unwrap();
            let known = cache.provider_models.entry(provider_name).or_default();
            merge_fetched_models(known, models);
        });
    }

    /// Display enhanced context usage with session totals
    /// Refresh the numbers behind the context gauge. They are handed to the
    /// completer rather than printed: on stdout the gauge would leave a stale
    /// copy of itself above every past answer.
    pub async fn update_context_gauge(&self) -> Result<()> {
        let provider = self.agent.provider().await?;
        let model_config = self
            .agent
            .model_config_for_session(&self.session_id)
            .await?;
        let context_limit = provider
            .get_context_limit(&model_config)
            .await
            .unwrap_or_else(|_| model_config.context_limit());

        let metadata = self.get_session().await.ok();
        let total_tokens = metadata
            .as_ref()
            .and_then(|metadata| metadata.usage.total_tokens)
            .unwrap_or(0) as usize;

        if let Ok(mut cache) = self.completion_cache.write() {
            cache.context_tokens = total_tokens;
            cache.context_limit = context_limit;
        }

        let config = Config::global();
        let show_cost = config
            .get_param::<bool>("GOOSE_CLI_SHOW_COST")
            .unwrap_or(false);

        if let (true, Some(metadata)) = (show_cost, metadata) {
            let provider_name = config
                .get_goose_provider()
                .unwrap_or_else(|_| "unknown".to_string());
            output::display_cost_usage(&provider_name, &model_config.model_name, &metadata.usage);
        }

        Ok(())
    }
}
