use clawg_core::models::provider_config::ProviderConfig;
use clawg_core::models::AppError;
use crate::state::AppState;

#[tauri::command]
pub fn list_providers(state: tauri::State<'_, AppState>) -> Result<Vec<ProviderConfig>, AppError> {
    let providers = state
        .providers
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(providers.clone())
}

#[tauri::command]
pub fn add_provider(
    name: String,
    provider_type: clawg_core::models::config::ProviderType,
    ollama: Option<clawg_core::models::config::OllamaSettings>,
    anthropic: Option<clawg_core::models::config::AnthropicSettings>,
    openrouter: Option<clawg_core::models::config::OpenRouterSettings>,
    state: tauri::State<'_, AppState>,
) -> Result<ProviderConfig, AppError> {
    let config = ProviderConfig {
        id: ProviderConfig::generate_id(),
        name,
        provider_type,
        ollama,
        anthropic,
        openrouter,
    };

    let mut providers = state
        .providers
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    providers.push(config.clone());
    state.save_providers(&providers);

    Ok(config)
}

#[tauri::command]
pub fn update_provider(
    id: String,
    name: String,
    provider_type: clawg_core::models::config::ProviderType,
    ollama: Option<clawg_core::models::config::OllamaSettings>,
    anthropic: Option<clawg_core::models::config::AnthropicSettings>,
    openrouter: Option<clawg_core::models::config::OpenRouterSettings>,
    state: tauri::State<'_, AppState>,
) -> Result<ProviderConfig, AppError> {
    let mut providers = state
        .providers
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let provider = providers
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| AppError::Internal(format!("Provider not found: {}", id)))?;

    provider.name = name;
    provider.provider_type = provider_type;
    provider.ollama = ollama;
    provider.anthropic = anthropic;
    provider.openrouter = openrouter;

    let result = provider.clone();
    state.save_providers(&providers);
    drop(providers);

    // Evict the stale cached instance so the next chat picks up updated settings.
    state.invalidate_agent_provider(&id);

    Ok(result)
}

#[tauri::command]
pub fn delete_provider(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    // Check no agents reference this provider
    let agents = state
        .agents
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let in_use = agents.iter().any(|a| a.provider_id == id);
    if in_use {
        return Err(AppError::Internal(
            "Cannot delete provider: it is assigned to an agent".to_string(),
        ));
    }
    drop(agents);

    let mut providers = state
        .providers
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    providers.retain(|p| p.id != id);
    state.save_providers(&providers);
    drop(providers);

    // Evict the deleted provider from the cache.
    state.invalidate_agent_provider(&id);

    Ok(())
}
