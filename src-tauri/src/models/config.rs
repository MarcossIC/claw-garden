use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    Ollama,
    Anthropic,
    OpenRouter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaSettings {
    pub host: String,
    pub port: u16,
    pub model: String,
}

impl Default for OllamaSettings {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 11434,
            model: "llama3.1".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicSettings {
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
}

impl Default for AnthropicSettings {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "claude-sonnet-4-5-20250514".to_string(),
            base_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterSettings {
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
}

impl Default for OpenRouterSettings {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "openrouter/free".to_string(),
            base_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub active_provider: ProviderType,
    pub ollama: OllamaSettings,
    pub anthropic: AnthropicSettings,
    pub openrouter: OpenRouterSettings,
    pub project_path: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            active_provider: ProviderType::Ollama,
            ollama: OllamaSettings::default(),
            anthropic: AnthropicSettings::default(),
            openrouter: OpenRouterSettings::default(),
            project_path: None,
        }
    }
}

/// Response from check_ollama command
#[derive(Debug, Serialize)]
pub struct OllamaStatus {
    pub running: bool,
    pub models: Vec<String>,
}
