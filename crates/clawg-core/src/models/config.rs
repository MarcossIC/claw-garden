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
    /// Skip the session-close memory extractor.
    ///
    /// Named for the disabled state, not the enabled one: `#[serde(default)]`
    /// fills a missing field with `false`, so a config file written before
    /// these fields existed keeps every job running. Spelled `*_enabled` the
    /// same missing field would read as `false` and silently disable the job
    /// for every existing install.
    #[serde(default)]
    pub disable_extractor: bool,
    #[serde(default)]
    pub disable_resolver: bool,
    #[serde(default)]
    pub disable_miner: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            active_provider: ProviderType::Ollama,
            ollama: OllamaSettings::default(),
            anthropic: AnthropicSettings::default(),
            openrouter: OpenRouterSettings::default(),
            project_path: None,
            disable_extractor: false,
            disable_resolver: false,
            disable_miner: false,
        }
    }
}

/// One of the auxiliary LLM jobs spawned at session close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuxJob {
    Extractor,
    Resolver,
    Miner,
}

/// Whether `job` should run under `config`.
pub fn aux_job_enabled(config: &AppConfig, job: AuxJob) -> bool {
    match job {
        AuxJob::Extractor => !config.disable_extractor,
        AuxJob::Resolver => !config.disable_resolver,
        AuxJob::Miner => !config.disable_miner,
    }
}

/// Response from check_ollama command
#[derive(Debug, Serialize)]
pub struct OllamaStatus {
    pub running: bool,
    pub models: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `config_store::load_config` is `serde_json::from_str(...).ok().unwrap_or_default()`
    /// — deserialization is all-or-nothing per file. A JSON literal written
    /// before the aux-LLM flags existed must still parse, come back with all
    /// three flags `false`, AND preserve `api_key`/`project_path` — a missing
    /// `#[serde(default)]` would make the whole file fail to parse and reset
    /// every field, including these two.
    #[test]
    fn config_without_aux_flags_deserializes_and_preserves_existing_fields() {
        let json = r#"{
            "active_provider": "anthropic",
            "ollama": {"host": "localhost", "port": 11434, "model": "llama3.1"},
            "anthropic": {"api_key": "sk-test-123", "model": "claude-sonnet-4-5-20250514"},
            "openrouter": {"api_key": "", "model": "openrouter/free"},
            "project_path": "/home/user/project"
        }"#;

        let config: AppConfig = serde_json::from_str(json).expect("legacy config must still parse");

        assert!(!config.disable_extractor, "missing field must default to false (job runs)");
        assert!(!config.disable_resolver, "missing field must default to false (job runs)");
        assert!(!config.disable_miner, "missing field must default to false (job runs)");
        assert_eq!(config.anthropic.api_key, "sk-test-123", "api_key must survive, not reset");
        assert_eq!(
            config.project_path,
            Some("/home/user/project".to_string()),
            "project_path must survive, not reset"
        );
    }

    #[test]
    fn aux_job_enabled_true_by_default() {
        let config = AppConfig::default();
        assert!(aux_job_enabled(&config, AuxJob::Extractor));
        assert!(aux_job_enabled(&config, AuxJob::Resolver));
        assert!(aux_job_enabled(&config, AuxJob::Miner));
    }

    #[test]
    fn aux_job_enabled_false_when_extractor_disabled() {
        let mut config = AppConfig::default();
        config.disable_extractor = true;
        assert!(!aux_job_enabled(&config, AuxJob::Extractor));
        assert!(aux_job_enabled(&config, AuxJob::Resolver));
        assert!(aux_job_enabled(&config, AuxJob::Miner));
    }

    #[test]
    fn aux_job_enabled_false_when_resolver_disabled() {
        let mut config = AppConfig::default();
        config.disable_resolver = true;
        assert!(aux_job_enabled(&config, AuxJob::Extractor));
        assert!(!aux_job_enabled(&config, AuxJob::Resolver));
        assert!(aux_job_enabled(&config, AuxJob::Miner));
    }

    #[test]
    fn aux_job_enabled_false_when_miner_disabled() {
        let mut config = AppConfig::default();
        config.disable_miner = true;
        assert!(aux_job_enabled(&config, AuxJob::Extractor));
        assert!(aux_job_enabled(&config, AuxJob::Resolver));
        assert!(!aux_job_enabled(&config, AuxJob::Miner));
    }

    #[test]
    fn aux_flags_round_trip_through_json() {
        let mut config = AppConfig::default();
        config.disable_extractor = true;
        config.disable_miner = true;

        let json = serde_json::to_string(&config).expect("AppConfig serializes infallibly");
        let round_tripped: AppConfig = serde_json::from_str(&json).expect("must round trip");

        assert!(round_tripped.disable_extractor);
        assert!(!round_tripped.disable_resolver);
        assert!(round_tripped.disable_miner);
    }
}
