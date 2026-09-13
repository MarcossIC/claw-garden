pub mod loader;
pub mod provider_pool;

pub use loader::{AgentConfigOverride, BaseConfig, ConfigLoader, ConfigSource, ResolvedConfig};
pub use provider_pool::ProviderPool;

use serde::{Deserialize, Serialize};

/// Workspace-level config stored at `<project>/.clawg/config.json` (see
/// [`crate::paths::PROJECT_CONFIG_DIR`]).
///
/// Overrides app-global settings for every agent working in this workspace.
/// Is itself overridden by per-agent [`AgentConfigOverride`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Override the default provider for all agents in this workspace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    /// Override the default model for all agents in this workspace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}
