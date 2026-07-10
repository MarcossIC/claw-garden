use serde::{Deserialize, Serialize};

use super::config::{AnthropicSettings, OllamaSettings, OpenRouterSettings, ProviderType};

/// A saved provider configuration — reusable across agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub provider_type: ProviderType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ollama: Option<OllamaSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic: Option<AnthropicSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openrouter: Option<OpenRouterSettings>,
}

impl ProviderConfig {
    pub fn generate_id() -> String {
        format!(
            "provider-{}",
            crate::models::chat::current_time_millis()
        )
    }
}
