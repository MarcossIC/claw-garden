use crate::models::chat::current_time_millis;
use crate::models::prompt::AgentRole;
use crate::models::{Agent, AgentState, AppError, Position};
use crate::state::AppState;

#[tauri::command]
pub fn create_agent(
    name: String,
    role: AgentRole,
    provider_id: String,
    project_path: Option<String>,
    custom_instructions: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Agent, AppError> {
    // Validate provider exists
    {
        let providers = state
            .providers
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if !providers.iter().any(|p| p.id == provider_id) {
            return Err(AppError::Internal(format!(
                "Provider not found: {}",
                provider_id
            )));
        }
    }

    let agent_id = format!("agent-{}", current_time_millis());

    // Find a free spawn position (offset from existing agents)
    let spawn_pos = {
        let agents = state
            .agents
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let base_x = 8.0 + (agents.len() as f32 * 4.0);
        Position {
            x: base_x,
            y: 6.0,
        }
    };

    let system_prompt = crate::models::compose_system_prompt(
        &name,
        &role,
        false,
        project_path.as_deref(),
    );

    let agent = Agent {
        id: agent_id,
        name,
        role,
        provider_id,
        project_path,
        custom_instructions,
        system_prompt,
        efficient_mode: false,
        state: AgentState::Idle,
        position: spawn_pos,
        hooks: Default::default(),
    };

    let mut agents = state
        .agents
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    agents.push(agent.clone());
    state.save_agents(&agents);

    Ok(agent)
}

#[tauri::command]
pub fn update_agent(
    id: String,
    name: Option<String>,
    role: Option<AgentRole>,
    provider_id: Option<String>,
    project_path: Option<Option<String>>,
    custom_instructions: Option<Option<String>>,
    efficient_mode: Option<bool>,
    state: tauri::State<'_, AppState>,
) -> Result<Agent, AppError> {
    // Validate provider if changing
    if let Some(ref pid) = provider_id {
        let providers = state
            .providers
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if !providers.iter().any(|p| p.id == *pid) {
            return Err(AppError::Internal(format!("Provider not found: {}", pid)));
        }
    }

    let mut agents = state
        .agents
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let agent = agents
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or_else(|| AppError::AgentNotFound(id))?;

    if let Some(n) = name {
        agent.name = n;
    }
    if let Some(r) = role {
        agent.role = r;
    }
    if let Some(pid) = provider_id {
        agent.provider_id = pid;
    }
    if let Some(pp) = project_path {
        agent.project_path = pp;
    }
    if let Some(ci) = custom_instructions {
        agent.custom_instructions = ci;
    }
    if let Some(em) = efficient_mode {
        agent.efficient_mode = em;
    }

    agent.recompose_prompt();
    let result = agent.clone();
    state.save_agents(&agents);

    Ok(result)
}

#[tauri::command]
pub fn delete_agent(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    let mut agents = state
        .agents
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    agents.retain(|a| a.id != id);
    state.save_agents(&agents);

    // Clean up conversation
    let mut conversations = state
        .conversations
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    conversations.remove(&id);

    Ok(())
}
