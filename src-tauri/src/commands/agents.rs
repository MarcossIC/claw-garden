use clawg_core::models::{Agent, AppError};
use crate::state::AppState;

#[tauri::command]
pub fn get_agents(state: tauri::State<'_, AppState>) -> Result<Vec<Agent>, AppError> {
    let agents = state
        .agents
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(agents.clone())
}
