use std::env;

use crate::models::AppError;
use crate::services::{credentials, oauth};

/// Standard env var names per provider.
pub const ANTHROPIC_KEY_ENV: &str = "ANTHROPIC_API_KEY";
pub const OPENROUTER_KEY_ENV: &str = "OPENROUTER_API_KEY";

pub const ANTHROPIC_BASE_URL_ENV: &str = "ANTHROPIC_BASE_URL";
pub const OPENROUTER_BASE_URL_ENV: &str = "OPENROUTER_BASE_URL";

pub const DEFAULT_ANTHROPIC_URL: &str = "https://api.anthropic.com";
pub const DEFAULT_OPENROUTER_URL: &str = "https://openrouter.ai/api/v1";

/// Resolves API key for a provider.
/// Priority: env var > config value.
pub fn resolve_api_key(env_var: &str, config_value: &str) -> String {
    env::var(env_var)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| config_value.to_string())
}

/// Resolves base URL for a provider.
/// Priority: env var > config value > default.
pub fn resolve_base_url(env_var: &str, config_value: &Option<String>, default: &str) -> String {
    env::var(env_var)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| config_value.clone().filter(|v| !v.trim().is_empty()))
        .unwrap_or_else(|| default.to_string())
}

/// Pre-flight check: if the OAuth token is expired and refreshable, refresh it now.
/// Call this BEFORE creating the provider for a request so that
/// `resolve_anthropic_auth` picks up the freshly saved token.
///
/// The `refresh_lock` serializes concurrent refresh attempts across agents.
/// Uses double-checked locking: a fast lock-free check first, then a locked
/// re-check to avoid duplicate HTTP refresh requests.
///
/// Returns `Ok(true)` when a refresh was performed (caller should rebuild the provider),
/// `Ok(false)` when no action was needed, or an error if the token is expired and
/// cannot be refreshed.
pub async fn ensure_oauth_fresh(
    refresh_lock: &tokio::sync::Mutex<()>,
) -> Result<bool, AppError> {
    // Fast path (no lock): skip if no credentials or token still valid
    let token_set = match credentials::load_oauth_credentials() {
        Some(ts) => ts,
        None => return Ok(false),
    };
    if !oauth::is_token_expired(&token_set) {
        return Ok(false);
    }

    // Token appears expired — acquire lock to serialize refresh attempts
    let _guard = refresh_lock.lock().await;

    // Re-read from disk: another caller may have refreshed while we waited
    let token_set = match credentials::load_oauth_credentials() {
        Some(ts) => ts,
        None => return Ok(false),
    };
    if !oauth::is_token_expired(&token_set) {
        return Ok(false); // another agent already refreshed — we're good
    }

    // Still expired and we hold the lock — we're the one who refreshes
    if token_set.refresh_token.is_none() {
        log::warn!("OAuth token expired and no refresh token available");
        return Err(AppError::AuthExpired(
            "OAuth token expired. Please login again.".to_string(),
        ));
    }

    log::info!("OAuth token expired — refreshing proactively");
    let refreshed = oauth::refresh_token(&token_set)
        .await
        .map_err(|e| AppError::AuthExpired(format!("Token refresh failed: {e}")))?;

    credentials::save_oauth_credentials(&refreshed)
        .map_err(|e| AppError::Internal(format!("Failed to save refreshed token: {e}")))?;

    log::info!("OAuth token refreshed successfully");
    Ok(true)
}

/// Resolves Anthropic auth with priority: env API key > OAuth token > config API key.
pub fn resolve_anthropic_auth(config_api_key: &str) -> crate::providers::anthropic::AnthropicAuth {
    use crate::providers::anthropic::AnthropicAuth;

    // 1. Env var API key — highest priority
    if let Ok(key) = env::var(ANTHROPIC_KEY_ENV) {
        if !key.trim().is_empty() {
            return AnthropicAuth::ApiKey(key);
        }
    }

    // 2. OAuth token from credentials.json
    // Note: callers should run `ensure_oauth_fresh()` before calling this to guarantee
    // a valid token is on disk. If the token is still expired here, it means refresh
    // failed and we fall back to API key.
    if let Some(token_set) = credentials::load_oauth_credentials() {
        if !oauth::is_token_expired(&token_set) {
            return AnthropicAuth::Bearer(token_set.access_token);
        }
        log::warn!("OAuth token still expired after pre-flight check — falling back to API key");
    }

    // 3. Config API key
    AnthropicAuth::ApiKey(config_api_key.to_string())
}

/// Returns the source of authentication for a provider.
/// For Anthropic, also checks OAuth credentials.
pub fn auth_source(env_var: &str, config_val: &str) -> String {
    if env::var(env_var)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
    {
        "env".to_string()
    } else if env_var == ANTHROPIC_KEY_ENV {
        // For Anthropic, check OAuth as a source
        if credentials::load_oauth_credentials().is_some() {
            "oauth".to_string()
        } else if !config_val.is_empty() {
            "config".to_string()
        } else {
            "none".to_string()
        }
    } else if !config_val.is_empty() {
        "config".to_string()
    } else {
        "none".to_string()
    }
}
