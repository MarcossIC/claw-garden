use std::collections::HashMap;
use std::sync::Arc;

use crate::models::config::ProviderType;
use crate::models::provider_config::ProviderConfig;
use crate::providers::anthropic::{AnthropicConfig, AnthropicProvider};
use crate::providers::ollama::{OllamaConfig, OllamaProvider};
use crate::providers::openrouter::{OpenRouterConfig, OpenRouterProvider};
use crate::providers::AiProvider;
use crate::services::auth::{
    resolve_anthropic_auth, resolve_api_key, resolve_base_url, ANTHROPIC_BASE_URL_ENV,
    DEFAULT_ANTHROPIC_URL, DEFAULT_OPENROUTER_URL, OPENROUTER_BASE_URL_ENV, OPENROUTER_KEY_ENV,
};
use crate::services::model_meta;

/// Cache of live provider instances keyed by [`ProviderConfig::id`].
///
/// Avoids rebuilding identical provider connections on every chat iteration.
/// Entries must be invalidated whenever the underlying [`ProviderConfig`] changes
/// or when OAuth credentials are refreshed.
///
/// # Thread safety
///
/// Designed to live behind a `std::sync::Mutex` — all operations are synchronous
/// because provider construction is CPU-bound, not I/O-bound.
pub struct ProviderPool {
    cache: HashMap<String, Arc<dyn AiProvider>>,
}

impl ProviderPool {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Return a cached provider or build and cache a new one.
    pub fn get_or_build(&mut self, config: &ProviderConfig) -> Arc<dyn AiProvider> {
        if let Some(provider) = self.cache.get(&config.id) {
            return provider.clone();
        }
        let provider = Self::build_provider(config);
        self.cache.insert(config.id.clone(), provider.clone());
        provider
    }

    /// Remove the cached entry for `provider_id`, forcing a rebuild on next access.
    ///
    /// Must be called after:
    /// - A [`ProviderConfig`] is updated or deleted.
    /// - OAuth credentials are refreshed (so the new token is picked up).
    pub fn invalidate(&mut self, provider_id: &str) {
        self.cache.remove(provider_id);
    }

    /// Drop all cached providers.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Instantiate a provider from a [`ProviderConfig`] without caching.
    ///
    /// Exposed so that `AppState::build_provider_from_config` can delegate here,
    /// keeping the construction logic in a single place.
    pub fn build_provider(config: &ProviderConfig) -> Arc<dyn AiProvider> {
        match config.provider_type {
            ProviderType::Ollama => {
                let s = config.ollama.as_ref().expect("Ollama settings missing");
                Arc::new(OllamaProvider::new(OllamaConfig {
                    host: s.host.clone(),
                    port: s.port,
                    model: model_meta::resolve_alias(&s.model).to_string(),
                }))
            }
            ProviderType::Anthropic => {
                let s = config
                    .anthropic
                    .as_ref()
                    .expect("Anthropic settings missing");
                Arc::new(AnthropicProvider::new(AnthropicConfig {
                    auth: resolve_anthropic_auth(&s.api_key),
                    model: model_meta::resolve_alias(&s.model).to_string(),
                    base_url: resolve_base_url(
                        ANTHROPIC_BASE_URL_ENV,
                        &s.base_url,
                        DEFAULT_ANTHROPIC_URL,
                    ),
                }))
            }
            ProviderType::OpenRouter => {
                let s = config
                    .openrouter
                    .as_ref()
                    .expect("OpenRouter settings missing");
                Arc::new(OpenRouterProvider::new(OpenRouterConfig {
                    api_key: resolve_api_key(OPENROUTER_KEY_ENV, &s.api_key),
                    model: model_meta::resolve_alias(&s.model).to_string(),
                    base_url: resolve_base_url(
                        OPENROUTER_BASE_URL_ENV,
                        &s.base_url,
                        DEFAULT_OPENROUTER_URL,
                    ),
                }))
            }
        }
    }
}

impl Default for ProviderPool {
    fn default() -> Self {
        Self::new()
    }
}
