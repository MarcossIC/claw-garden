use std::path::PathBuf;

use clawg_core::config::{ConfigLoader, WorkspaceConfig};
use clawg_core::models::AppError;

/// Return the workspace-level config for the given project path.
///
/// Reads `<project_path>/.clawg/config.json` (see
/// [`clawg_core::paths::PROJECT_CONFIG_DIR`]).
/// Returns an empty [`WorkspaceConfig`] when no file exists yet.
#[tauri::command]
pub fn get_workspace_config(project_path: String) -> Result<WorkspaceConfig, AppError> {
    let path = PathBuf::from(&project_path);
    Ok(ConfigLoader::load_workspace_config_for(&path))
}

/// Persist workspace-level config for the given project path.
///
/// Writes to `<project_path>/.clawg/config.json` (see [`clawg_core::paths::PROJECT_CONFIG_DIR`]),
/// creating the directory if necessary. Passing `None` (or an empty string)
/// for a field clears that override so the global default applies instead.
#[tauri::command]
pub fn set_workspace_config(
    project_path: String,
    provider_id: Option<String>,
    model: Option<String>,
) -> Result<(), AppError> {
    let path = PathBuf::from(&project_path);
    let config = WorkspaceConfig {
        // Treat empty strings the same as None — no override.
        provider_id: provider_id.filter(|s| !s.is_empty()),
        model: model.filter(|s| !s.is_empty()),
    };
    ConfigLoader::save_workspace_config(&path, &config)
        .map_err(|e| AppError::Internal(e.to_string()))
}
