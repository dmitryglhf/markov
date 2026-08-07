// fork: copy of litellm.rs, kept separate so upstream merges never touch it.
// Differences: name and defaults, 1200s timeout, PGPRO_CONTEXT_LIMITS.
use anyhow::Result;
use async_trait::async_trait;
use futures::future::BoxFuture;
use goose_providers::conversation::token_usage::ProviderUsage;
use goose_providers::errors::ProviderError;
use goose_providers::images::ImageFormat;
use serde_json::{json, Value};
use std::collections::HashMap;

use super::api_client::{ApiClient, AuthMethod};
use super::base::{ConfigKey, MessageStream, ModelInfo, Provider, ProviderDef, ProviderMetadata};
use super::openai_compatible::handle_response_openai_compat;
use super::retry::ProviderRetry;
use super::utils::get_model;
use crate::conversation::message::Message;
use goose_providers::model::ModelConfig;
use goose_providers::request_log::{start_log, LoggerHandleExt};
use rmcp::model::Tool;

const PGPRO_PROVIDER_NAME: &str = "pgpro";
pub const PGPRO_DEFAULT_MODEL: &str = "zai-org/GLM-5-FP8";
pub const PGPRO_DOC_URL: &str = "https://docs.litellm.ai/docs/";
const PGPRO_DEFAULT_HOST: &str = "http://10.7.5.88:4000";
const PGPRO_DEFAULT_TIMEOUT_SECS: u64 = 1200;

// measured against the gateway, PGPRO_CONTEXT_LIMITS overrides per model
const PGPRO_DEFAULT_CONTEXT_LIMITS: &[(&str, usize)] = &[
    ("zai-org/GLM-5-FP8", 202_000),
    ("deepseek-ai/DeepSeek-V4-Flash", 210_000),
    ("Qwen/Qwen3.6-27B-FP8", 262_144),
    ("google/gemma-4-31B-it-qat-w4a16-ct", 256_534),
];

#[derive(Debug, serde::Serialize)]
pub struct PgproProvider {
    #[serde(skip)]
    api_client: ApiClient,
    base_path: String,
    #[serde(skip)]
    name: String,
    #[serde(skip)]
    cached_model_info: tokio::sync::OnceCell<Vec<ModelInfo>>,
}

impl PgproProvider {
    pub async fn from_env(
        tls_config: Option<crate::providers::api_client::TlsConfig>,
    ) -> Result<Self> {
        let config = crate::config::Config::global();
        let secrets = config
            .get_secrets("PGPRO_API_KEY", &["PGPRO_CUSTOM_HEADERS"])
            .unwrap_or_default();
        let api_key = secrets.get("PGPRO_API_KEY").cloned().unwrap_or_default();
        let host: String = config
            .get_param("PGPRO_HOST")
            .unwrap_or_else(|_| PGPRO_DEFAULT_HOST.to_string());
        let base_path: String = config
            .get_param("PGPRO_BASE_PATH")
            .unwrap_or_else(|_| "v1/chat/completions".to_string());
        let custom_headers: Option<HashMap<String, String>> = secrets
            .get("PGPRO_CUSTOM_HEADERS")
            .cloned()
            .map(parse_custom_headers);
        let timeout_secs: u64 = config
            .get_param("PGPRO_TIMEOUT")
            .unwrap_or(PGPRO_DEFAULT_TIMEOUT_SECS);

        let auth = if api_key.is_empty() {
            AuthMethod::NoAuth
        } else {
            AuthMethod::BearerToken(api_key)
        };

        let mut api_client = ApiClient::with_timeout_and_tls(
            host,
            auth,
            std::time::Duration::from_secs(timeout_secs),
            tls_config,
        )?
        .with_request_builder(crate::session_context::session_id_request_builder());

        if let Some(headers) = custom_headers {
            let mut header_map = reqwest::header::HeaderMap::new();
            for (key, value) in headers {
                let header_name = reqwest::header::HeaderName::from_bytes(key.as_bytes())?;
                let header_value = reqwest::header::HeaderValue::from_str(&value)?;
                header_map.insert(header_name, header_value);
            }
            api_client = api_client.with_headers(header_map)?;
        }

        Ok(Self {
            api_client,
            base_path,
            name: PGPRO_PROVIDER_NAME.to_string(),
            cached_model_info: tokio::sync::OnceCell::new(),
        })
    }

    async fn get_or_fetch_models(&self) -> Result<&[ModelInfo], ProviderError> {
        self.cached_model_info
            .get_or_try_init(|| self.fetch_models_from_api())
            .await
            .map(|v| v.as_slice())
    }

    async fn fetch_models_from_api(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let response = self.api_client.request("model/info").response_get().await?;

        // the gateway grants /model/info to privileged keys only; an ordinary
        // virtual key gets 403 and has to settle for the plain model list
        if !response.status().is_success() {
            return self.fetch_models_from_list().await;
        }

        let response_json: Value = response.json().await.map_err(|e| {
            ProviderError::RequestFailed(format!("Failed to parse models response: {}", e))
        })?;

        parse_model_info(&response_json)
    }

    async fn fetch_models_from_list(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let response = self.api_client.request("v1/models").response_get().await?;

        if !response.status().is_success() {
            return Err(ProviderError::RequestFailed(format!(
                "Models endpoint returned status: {}",
                response.status()
            )));
        }

        let response_json: Value = response.json().await.map_err(|e| {
            ProviderError::RequestFailed(format!("Failed to parse models response: {}", e))
        })?;

        parse_model_list(&response_json)
    }

    async fn post(
        &self,
        model_config: &ModelConfig,
        payload: &Value,
    ) -> Result<Value, ProviderError> {
        let response = self
            .api_client
            .request(&self.base_path)
            .model_headers(model_config)?
            .response_post(payload)
            .await?;
        handle_response_openai_compat(response).await
    }

    async fn supports_cache_control(&self, model: &ModelConfig) -> bool {
        if let Ok(models) = self.get_or_fetch_models().await {
            if let Some(model_info) = models.iter().find(|m| m.name == model.model_name) {
                return model_info.supports_cache_control.unwrap_or(false);
            }
        }

        model.model_name.to_lowercase().contains("claude")
    }
}

impl goose_providers::base::ProviderDescriptor for PgproProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            PGPRO_PROVIDER_NAME,
            "PgPro",
            "Postgres Professional model gateway",
            PGPRO_DEFAULT_MODEL,
            vec![],
            PGPRO_DOC_URL,
            vec![
                ConfigKey::new("PGPRO_API_KEY", true, true, None, true),
                ConfigKey::new("PGPRO_HOST", true, false, Some(PGPRO_DEFAULT_HOST), true),
                ConfigKey::new(
                    "PGPRO_BASE_PATH",
                    true,
                    false,
                    Some("v1/chat/completions"),
                    false,
                ),
                ConfigKey::new("PGPRO_CUSTOM_HEADERS", false, true, None, false),
                ConfigKey::new("PGPRO_TIMEOUT", false, false, Some("1200"), false),
            ],
        )
    }
}

impl ProviderDef for PgproProvider {
    type Provider = Self;

    fn from_env(
        _extensions: Vec<crate::config::ExtensionConfig>,
        tls_config: Option<crate::providers::api_client::TlsConfig>,
    ) -> BoxFuture<'static, Result<Self::Provider>> {
        Box::pin(Self::from_env(tls_config))
    }
}

#[async_trait]
impl Provider for PgproProvider {
    fn get_name(&self) -> &str {
        &self.name
    }

    async fn get_context_limit(&self, model_config: &ModelConfig) -> Result<usize, ProviderError> {
        if let Some(limit) = model_config.context_limit {
            return Ok(limit);
        }

        // gateway reports no context window, so PGPRO_CONTEXT_LIMITS may supply one
        if let Some(limit) = configured_context_limit(&model_config.model_name) {
            return Ok(limit);
        }

        // The cache is populated lazily by the first stream() call (via
        // supports_cache_control). On turn 1 this will be None and we fall
        // back to DEFAULT_CONTEXT_LIMIT, which is fine — the conversation is
        // too small to trigger compaction. From turn 2 onward the real limit
        // from /model/info is used.
        if let Some(models) = self.cached_model_info.get() {
            if let Some(info) = models.iter().find(|m| m.name == model_config.model_name) {
                if info.context_limit > 0 {
                    return Ok(info.context_limit);
                }
            }
        }

        Ok(model_config.context_limit())
    }

    async fn stream(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let mut payload = goose_providers::formats::openai::create_request(
            model_config,
            system,
            messages,
            tools,
            &ImageFormat::OpenAi,
            false,
        )?;

        if self.supports_cache_control(model_config).await {
            payload = update_request_for_cache_control(&payload);
        }

        let response = self
            .with_retry(|| async {
                let payload_clone = payload.clone();
                self.post(model_config, &payload_clone).await
            })
            .await?;

        let message = goose_providers::formats::openai::response_to_message(&response)?;
        let usage = goose_providers::formats::openai::get_usage(&response);
        let response_model = get_model(&response);
        let mut log = start_log(model_config, &payload)?;
        log.write(&response, Some(&usage))?;
        let provider_usage = ProviderUsage::new(response_model, usage);
        Ok(super::base::stream_from_single_message(
            message,
            provider_usage,
        ))
    }

    fn skip_canonical_filtering(&self) -> bool {
        true
    }

    async fn fetch_supported_models(&self) -> Result<Vec<String>, ProviderError> {
        let models = self.get_or_fetch_models().await?;
        Ok(models.iter().map(|m| m.name.clone()).collect())
    }
}

/// Updates the request payload to include cache control headers for automatic prompt caching
/// Adds ephemeral cache control to the last 2 user messages, system message, and last tool
pub fn update_request_for_cache_control(original_payload: &Value) -> Value {
    let mut payload = original_payload.clone();

    if let Some(messages_spec) = payload
        .as_object_mut()
        .and_then(|obj| obj.get_mut("messages"))
        .and_then(|messages| messages.as_array_mut())
    {
        let mut user_count = 0;
        for message in messages_spec.iter_mut().rev() {
            if message.get("role") == Some(&json!("user")) {
                if let Some(content) = message.get_mut("content") {
                    if let Some(content_str) = content.as_str() {
                        *content = json!([{
                            "type": "text",
                            "text": content_str,
                            "cache_control": { "type": "ephemeral" }
                        }]);
                    }
                }
                user_count += 1;
                if user_count >= 2 {
                    break;
                }
            }
        }

        if let Some(system_message) = messages_spec
            .iter_mut()
            .find(|msg| msg.get("role") == Some(&json!("system")))
        {
            if let Some(content) = system_message.get_mut("content") {
                if let Some(content_str) = content.as_str() {
                    *system_message = json!({
                        "role": "system",
                        "content": [{
                            "type": "text",
                            "text": content_str,
                            "cache_control": { "type": "ephemeral" }
                        }]
                    });
                }
            }
        }
    }

    if let Some(tools_spec) = payload
        .as_object_mut()
        .and_then(|obj| obj.get_mut("tools"))
        .and_then(|tools| tools.as_array_mut())
    {
        if let Some(last_tool) = tools_spec.last_mut() {
            if let Some(function) = last_tool.get_mut("function") {
                function
                    .as_object_mut()
                    .unwrap()
                    .insert("cache_control".to_string(), json!({ "type": "ephemeral" }));
            }
        }
    }
    payload
}

/// PGPRO_CONTEXT_LIMITS maps model name to context window, as a mapping or a JSON string.
fn parse_model_info(response: &Value) -> Result<Vec<ModelInfo>, ProviderError> {
    let models_data = response["data"].as_array().ok_or_else(|| {
        ProviderError::RequestFailed("Missing data field in models response".to_string())
    })?;

    let mut models = Vec::new();
    for model_data in models_data {
        if let Some(model_name) = model_data["model_name"].as_str() {
            if model_name.contains("/*") {
                continue;
            }

            let model_info = &model_data["model_info"];
            let context_length = model_info["max_input_tokens"].as_u64().unwrap_or(128000) as usize;
            let supports_cache_control = model_info["supports_prompt_caching"].as_bool();

            let mut model_info_obj = ModelInfo::new(model_name, context_length);
            model_info_obj.supports_cache_control = supports_cache_control;
            models.push(model_info_obj);
        }
    }

    Ok(models)
}

/// The plain /v1/models list carries names and nothing else, so the context
/// window comes from PGPRO_CONTEXT_LIMITS and the measured defaults.
fn parse_model_list(response: &Value) -> Result<Vec<ModelInfo>, ProviderError> {
    let models_data = response["data"].as_array().ok_or_else(|| {
        ProviderError::RequestFailed("Missing data field in models response".to_string())
    })?;

    Ok(models_data
        .iter()
        .filter_map(|model| model["id"].as_str())
        .filter(|name| !name.contains("/*"))
        .map(|name| ModelInfo::new(name, configured_context_limit(name).unwrap_or(128000)))
        .collect())
}

fn configured_context_limit(model_name: &str) -> Option<usize> {
    let config = crate::config::Config::global();
    let from_config = config
        .get_param::<HashMap<String, usize>>("PGPRO_CONTEXT_LIMITS")
        .ok()
        .or_else(|| {
            let raw: String = config.get_param("PGPRO_CONTEXT_LIMITS").ok()?;
            serde_json::from_str(&raw).ok()
        })
        .and_then(|map| map.get(model_name).copied());

    from_config.or_else(|| {
        PGPRO_DEFAULT_CONTEXT_LIMITS
            .iter()
            .find(|(name, _)| *name == model_name)
            .map(|(_, limit)| *limit)
    })
}

fn parse_custom_headers(headers_str: String) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    for line in headers_str.lines() {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restricted_key_still_gets_the_model_names() {
        let response = serde_json::json!({
            "data": [
                {"id": "zai-org/GLM-5-FP8", "object": "model"},
                {"id": "team/*", "object": "model"},
                {"id": "unknown-model", "object": "model"},
            ]
        });
        let models = parse_model_list(&response).unwrap();
        let names: Vec<_> = models.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["zai-org/GLM-5-FP8", "unknown-model"]);
    }

    #[test]
    fn a_known_model_from_the_list_keeps_its_measured_context_window() {
        let response = serde_json::json!({"data": [{"id": "zai-org/GLM-5-FP8"}]});
        let models = parse_model_list(&response).unwrap();
        assert_eq!(models[0].context_limit, 202_000);
    }

    #[test]
    fn a_model_the_gateway_never_measured_gets_the_default_window() {
        let response = serde_json::json!({"data": [{"id": "unknown-model"}]});
        let models = parse_model_list(&response).unwrap();
        assert_eq!(models[0].context_limit, 128_000);
    }

    #[test]
    fn a_list_without_data_is_an_error() {
        assert!(parse_model_list(&serde_json::json!({"object": "list"})).is_err());
    }
}
