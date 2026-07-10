use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::models::provider_config::ProviderConfig;

use super::WorkspaceConfig;

/// Identifies where a config value originated, in ascending precedence order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    AppGlobal,
    Workspace,
    Agent,
}

/// App-global settings stored at `~/.claw-garden/settings.json`.
///
/// Provides baseline provider and model defaults for all agents.
/// Overridden by [`WorkspaceConfig`] and then [`AgentConfigOverride`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

/// Merged result of AppGlobal + Workspace configs, before any per-agent override.
///
/// Produced by [`ConfigLoader::load`]. Passed to [`Agent::resolve_config`] together
/// with the providers list and an optional [`AgentConfigOverride`].
#[derive(Debug, Clone, Default)]
pub struct BaseConfig {
    pub provider_id: Option<String>,
    pub model: Option<String>,
    /// Set to the workspace path when a workspace config was loaded.
    pub project_path: Option<PathBuf>,
}

/// Per-agent config overrides stored at `<project>/.claw-garden/agents/<agent_id>.json`.
///
/// Takes the highest precedence, overriding both workspace and global settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentConfigOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Fully resolved config for a specific agent — the product of merging all three levels.
///
/// `provider_config` is guaranteed to be a valid [`ProviderConfig`] from the providers list.
/// `project_path` is `None` when no workspace or per-agent path has been set.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub provider_config: ProviderConfig,
    pub project_path: Option<PathBuf>,
}

/// Loads and deep-merges config from the 3-level hierarchy.
///
/// Precedence (highest last): `AppGlobal` → `Workspace` → `Agent`
///
/// # File layout
///
/// | Level     | Location                                          |
/// |-----------|---------------------------------------------------|
/// | AppGlobal | `~/.claw-garden/settings.json`                    |
/// | Workspace | `<project>/.claw-garden/config.json`              |
/// | Agent     | `<project>/.claw-garden/agents/<agent_id>.json`   |
pub struct ConfigLoader {
    pub app_home: PathBuf,
    pub workspace_path: Option<PathBuf>,
}

impl ConfigLoader {
    pub fn new(app_home: PathBuf, workspace_path: Option<PathBuf>) -> Self {
        Self {
            app_home,
            workspace_path,
        }
    }

    /// Returns the config files that currently exist on disk, ordered from lowest
    /// to highest precedence.
    pub fn discover(&self) -> Vec<(ConfigSource, PathBuf)> {
        let mut sources = Vec::new();

        let global = self.app_home.join("settings.json");
        if global.exists() {
            sources.push((ConfigSource::AppGlobal, global));
        }

        if let Some(ws) = &self.workspace_path {
            let ws_config = ws.join(".claw-garden").join("config.json");
            if ws_config.exists() {
                sources.push((ConfigSource::Workspace, ws_config));
            }
        }

        sources
    }

    /// Load and merge AppGlobal + Workspace into a [`BaseConfig`].
    ///
    /// Missing files are silently skipped; the merge is always additive
    /// (higher-precedence values win, `None` fields are left unchanged).
    pub fn load(&self) -> BaseConfig {
        let mut base = BaseConfig::default();

        // App-global settings (lowest precedence)
        let global_path = self.app_home.join("settings.json");
        if let Some(global) = read_json::<GlobalSettings>(&global_path) {
            base.provider_id = global.provider_id;
            base.model = global.default_model;
        }

        // Workspace config overrides global where present
        if let Some(ws_path) = &self.workspace_path {
            let ws_config_path = ws_path.join(".claw-garden").join("config.json");
            if let Some(ws) = read_json::<WorkspaceConfig>(&ws_config_path) {
                if ws.provider_id.is_some() {
                    base.provider_id = ws.provider_id;
                }
                if ws.model.is_some() {
                    base.model = ws.model;
                }
            }
            base.project_path = Some(ws_path.clone());
        }

        base
    }

    // ── Read helpers ────────────────────────────────────────────────────���───

    /// Read the [`WorkspaceConfig`] for `project_path`.
    ///
    /// Returns `WorkspaceConfig::default()` when the file is missing or unparseable.
    pub fn load_workspace_config_for(project_path: &Path) -> WorkspaceConfig {
        let path = project_path.join(".claw-garden").join("config.json");
        read_json::<WorkspaceConfig>(&path).unwrap_or_default()
    }

    // ── Persistence helpers ──────────────────────────────────────────────────

    /// Write workspace-level config to `<project>/.claw-garden/config.json`.
    pub fn save_workspace_config(
        project_path: &Path,
        config: &WorkspaceConfig,
    ) -> Result<(), std::io::Error> {
        let dir = project_path.join(".claw-garden");
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(dir.join("config.json"), json)?;
        Ok(())
    }

    /// Write per-agent config to `<project>/.claw-garden/agents/<agent_id>.json`.
    pub fn save_agent_override(
        project_path: &Path,
        agent_id: &str,
        override_cfg: &AgentConfigOverride,
    ) -> Result<(), std::io::Error> {
        let dir = project_path.join(".claw-garden").join("agents");
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(override_cfg)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(dir.join(format!("{agent_id}.json")), json)?;
        Ok(())
    }

    /// Read per-agent config from `<project>/.claw-garden/agents/<agent_id>.json`.
    /// Returns `None` when the file does not exist or cannot be parsed.
    pub fn load_agent_override(project_path: &Path, agent_id: &str) -> Option<AgentConfigOverride> {
        let path = project_path
            .join(".claw-garden")
            .join("agents")
            .join(format!("{agent_id}.json"));
        read_json::<AgentConfigOverride>(&path)
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}
