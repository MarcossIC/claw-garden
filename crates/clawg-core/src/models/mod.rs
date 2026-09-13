pub mod chat;
pub mod config;
mod error;
mod session_error;
pub mod prompt;
pub mod provider_config;
pub mod session;

pub use chat::{
    ChatMessage, ChatRequest, ChatStreamResult, Role, StreamEvent, ToolCall, ToolDefinition,
    ToolFunction, ToolFunctionDef,
};
pub use error::{AppError, LlmError, McpError};
pub use session_error::SessionError;
pub use prompt::{compose_system_prompt, AgentRole};
pub use session::SessionMeta;

pub use crate::hooks::AgentHookConfig;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum AgentState {
    Idle,
    Walking { target_x: f32, target_y: f32 },
    Thinking { task_summary: String },
    Coding { file: Option<String> },
    Reading { source: String },
    Searching { query: String },
    Error { message: String },
}

impl AgentState {
    pub fn idle() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub role: AgentRole,
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_instructions: Option<String>,
    #[serde(skip_serializing, default)]
    pub system_prompt: String,
    #[serde(default)]
    pub efficient_mode: bool,
    #[serde(default = "AgentState::idle")]
    pub state: AgentState,
    pub position: Position,
    /// Hook configuration for this agent. Empty by default.
    #[serde(default)]
    pub hooks: AgentHookConfig,
}

impl Agent {
    /// Recomposes the system prompt from current name, role, efficient_mode, and project_path.
    /// Call this after changing any of those fields.
    pub fn recompose_prompt(&mut self) {
        self.system_prompt = compose_system_prompt(
            &self.name,
            &self.role,
            self.efficient_mode,
            self.project_path.as_deref(),
        );
    }

    /// Resolve this agent's final effective config by merging all three config levels:
    ///
    /// `BaseConfig` (global + workspace) → Agent fields → `AgentConfigOverride` (workspace-persisted)
    ///
    /// Returns `None` when no matching provider can be found in `providers`.
    /// The caller should fall back to the global provider in that case.
    pub fn resolve_config(
        &self,
        base: &crate::config::BaseConfig,
        providers: &[provider_config::ProviderConfig],
        workspace_override: Option<&crate::config::AgentConfigOverride>,
    ) -> Option<crate::config::ResolvedConfig> {
        // Priority: workspace_override.provider_id > agent.provider_id > base.provider_id
        let provider_id = workspace_override
            .and_then(|o| o.provider_id.as_deref())
            .unwrap_or(self.provider_id.as_str());

        let provider_config = providers
            .iter()
            .find(|p| p.id == provider_id)
            .or_else(|| {
                base.provider_id
                    .as_deref()
                    .and_then(|id| providers.iter().find(|p| p.id == id))
            })
            .cloned()?;

        // Priority: workspace_override.project_path > agent.project_path > base.project_path
        let project_path = workspace_override
            .and_then(|o| o.project_path.clone())
            .or_else(|| self.project_path.as_ref().map(PathBuf::from))
            .or_else(|| base.project_path.clone());

        Some(crate::config::ResolvedConfig {
            provider_config,
            project_path,
        })
    }
}
