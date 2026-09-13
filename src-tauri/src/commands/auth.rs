use serde::Serialize;

use clawg_core::models::AppError;
use clawg_core::services::{credentials, oauth};

#[derive(Debug, Serialize)]
pub struct OAuthStatusResponse {
    pub logged_in: bool,
    pub expired: bool,
    pub has_refresh_token: bool,
}

#[tauri::command]
pub fn oauth_status() -> OAuthStatusResponse {
    match credentials::load_oauth_credentials() {
        Some(token_set) => OAuthStatusResponse {
            logged_in: true,
            expired: oauth::is_token_expired(&token_set),
            has_refresh_token: token_set.refresh_token.is_some(),
        },
        None => OAuthStatusResponse {
            logged_in: false,
            expired: false,
            has_refresh_token: false,
        },
    }
}

#[derive(Debug, Serialize)]
pub struct OAuthLoginResult {
    pub success: bool,
    pub message: String,
}

/// Starts the full OAuth PKCE flow:
/// 1. Generate PKCE challenge + state
/// 2. Open browser with authorization URL
/// 3. Wait for callback with auth code
/// 4. Exchange code for tokens
/// 5. Save tokens to credentials.json
#[tauri::command]
pub async fn oauth_login(app: tauri::AppHandle) -> Result<OAuthLoginResult, AppError> {
    let pkce = oauth::generate_pkce_pair();
    let state = oauth::generate_random_token(32);
    let port = oauth::DEFAULT_CALLBACK_PORT;

    let authorize_url = oauth::build_authorize_url(&pkce, &state, port);

    // Open browser — using tauri-plugin-opener
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_url(&authorize_url, None::<&str>)
        .map_err(|e| AppError::Internal(format!("Failed to open browser: {e}")))?;

    // Wait for callback in a blocking thread (TCP listener)
    let verifier = pkce.verifier.clone();
    let expected_state = state.clone();

    let callback = tokio::task::spawn_blocking(move || {
        oauth::wait_for_callback(port, 300) // 5 minute timeout
    })
    .await
    .map_err(|e| AppError::Internal(format!("Callback task failed: {e}")))?
    .map_err(|e| AppError::Internal(format!("Callback error: {e}")))?;

    // Verify state (CSRF protection)
    if callback.state != expected_state {
        return Err(AppError::Internal(
            "OAuth state mismatch — possible CSRF attack".to_string(),
        ));
    }

    // Exchange code for tokens
    let token_set = oauth::exchange_code(&callback.code, &verifier, port).await?;

    // Save credentials
    credentials::save_oauth_credentials(&token_set)
        .map_err(|e| AppError::Internal(format!("Failed to save credentials: {e}")))?;

    Ok(OAuthLoginResult {
        success: true,
        message: "Successfully logged in with Anthropic".to_string(),
    })
}

#[tauri::command]
pub fn oauth_logout() -> Result<OAuthLoginResult, AppError> {
    credentials::clear_oauth_credentials()
        .map_err(|e| AppError::Internal(format!("Failed to clear credentials: {e}")))?;

    Ok(OAuthLoginResult {
        success: true,
        message: "Logged out successfully".to_string(),
    })
}

/// Manually refresh the OAuth token.
#[tauri::command]
pub async fn oauth_refresh() -> Result<OAuthLoginResult, AppError> {
    let token_set = credentials::load_oauth_credentials()
        .ok_or_else(|| AppError::Internal("No OAuth credentials found".to_string()))?;

    let refreshed = oauth::refresh_token(&token_set).await?;

    credentials::save_oauth_credentials(&refreshed)
        .map_err(|e| AppError::Internal(format!("Failed to save refreshed credentials: {e}")))?;

    Ok(OAuthLoginResult {
        success: true,
        message: "Token refreshed successfully".to_string(),
    })
}
