use clawg_core::models::config::{
    AnthropicSettings, AppConfig, OllamaSettings, OllamaStatus, OpenRouterSettings, ProviderType,
};
use clawg_core::models::AppError;
use clawg_core::providers::openrouter::OpenRouterModel;
use clawg_core::services::auth::{self, ANTHROPIC_KEY_ENV, OPENROUTER_KEY_ENV};
use clawg_core::services::config_store;
use crate::state::{self, AppState};
use serde::Serialize;

#[tauri::command]
pub async fn set_project_path(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    use std::path::PathBuf;

    let path_buf = PathBuf::from(&path);
    state.mcp_pool.ensure_connected(&path_buf).await?;

    if let Ok(mut config) = state.config.write() {
        config.project_path = Some(path);
        let snapshot = config.clone();
        drop(config);
        if let Err(e) = config_store::save_config(&snapshot) {
            log::error!("Failed to persist config: {}", e);
        }
    }

    Ok(())
}

#[tauri::command]
pub fn set_aux_llm_flags(
    disable_extractor: bool,
    disable_resolver: bool,
    disable_miner: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    if let Ok(mut config) = state.config.write() {
        config.disable_extractor = disable_extractor;
        config.disable_resolver = disable_resolver;
        config.disable_miner = disable_miner;
        let snapshot = config.clone();
        drop(config);
        if let Err(e) = config_store::save_config(&snapshot) {
            log::error!("Failed to persist config: {}", e);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_project_path(state: tauri::State<'_, AppState>) -> Result<Option<String>, AppError> {
    let config = state
        .config
        .read()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(config.project_path.clone())
}

#[tauri::command]
pub fn get_config(state: tauri::State<'_, AppState>) -> Result<AppConfig, AppError> {
    let config = state
        .config
        .read()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(config.clone())
}

#[tauri::command]
pub fn set_provider(
    provider_type: ProviderType,
    ollama: Option<OllamaSettings>,
    anthropic: Option<AnthropicSettings>,
    openrouter: Option<OpenRouterSettings>,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    state.switch_provider(provider_type, ollama, anthropic, openrouter)
}

#[tauri::command]
pub async fn check_ollama() -> Result<OllamaStatus, AppError> {
    Ok(state::check_ollama_status().await)
}

#[tauri::command]
pub async fn openrouter_list_models(api_key: String) -> Result<Vec<OpenRouterModel>, AppError> {
    use clawg_core::providers::openrouter::{OpenRouterConfig, OpenRouterProvider};

    let provider = OpenRouterProvider::new(OpenRouterConfig {
        api_key,
        model: String::new(),
        base_url: clawg_core::services::auth::DEFAULT_OPENROUTER_URL.to_string(),
    });
    provider.list_models().await
}

#[derive(Debug, Serialize)]
pub struct AuthStatus {
    pub anthropic_source: String,
    pub openrouter_source: String,
}

#[tauri::command]
pub fn get_auth_status(state: tauri::State<'_, AppState>) -> AuthStatus {
    let config = state.config.read().ok();
    let (anthropic_val, openrouter_val) = match &config {
        Some(c) => (c.anthropic.api_key.as_str(), c.openrouter.api_key.as_str()),
        None => ("", ""),
    };
    AuthStatus {
        anthropic_source: auth::auth_source(ANTHROPIC_KEY_ENV, anthropic_val),
        openrouter_source: auth::auth_source(OPENROUTER_KEY_ENV, openrouter_val),
    }
}
