use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::mcp::McpManager;
use clawg_core::models::{AppError, McpError, ToolDefinition};

/// Pool of MCP connections, one per project path.
///
/// Uses two levels of locking for maximum concurrency:
///
/// - Outer `RwLock`: protects the map structure (rare writes — add/remove servers).
/// - Inner `Arc<RwLock<McpManager>>`: one lock per server, independent of all others.
///
/// Result: Agent A on Server A and Agent B on Server B never block each other.
/// Multiple concurrent `call_tool` calls to the same server are also possible
/// because `McpManager::call_tool` takes `&self`.
pub struct McpPool {
    connections: RwLock<HashMap<PathBuf, Arc<RwLock<McpManager>>>>,
}

impl McpPool {
    pub fn new() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
        }
    }

    /// Ensure a connected [`McpManager`] exists for `project_path`.
    /// No-op when a connection is already active. Uses double-checked locking.
    pub async fn ensure_connected(&self, project_path: &Path) -> Result<(), AppError> {
        let path_str = project_path
            .to_str()
            .ok_or_else(|| McpError::InitializationFailed {
                message: "project path is not valid UTF-8".into(),
            })?;

        // Fast path: read lock — check if we already have an Arc for this path
        let manager_arc = {
            let connections = self.connections.read().await;
            connections.get(project_path).cloned()
        };

        let manager_arc = match manager_arc {
            Some(arc) => arc,
            None => {
                // Slow path: insert a new McpManager — write lock on the map
                let mut connections = self.connections.write().await;
                // Double-check: another agent may have inserted between read and write
                connections
                    .entry(project_path.to_path_buf())
                    .or_insert_with(|| Arc::new(RwLock::new(McpManager::new())))
                    .clone()
            }
        };

        // Connect if not already connected — write lock only on this manager
        {
            let manager = manager_arc.read().await;
            if manager.is_connected() {
                return Ok(());
            }
        }
        let mut manager = manager_arc.write().await;
        // Double-check after acquiring write lock
        if !manager.is_connected() {
            manager.connect(path_str).await?;
        }
        Ok(())
    }

    /// Returns `true` when an active connection exists for `project_path`.
    pub async fn is_connected_for_path(&self, project_path: &Path) -> bool {
        let connections = self.connections.read().await;
        match connections.get(project_path) {
            Some(arc) => {
                let manager = arc.read().await;
                manager.is_connected()
            }
            None => false,
        }
    }

    /// Retrieve tool definitions for `project_path`.
    /// Returns `None` when no connection exists or the MCP call fails.
    pub async fn get_tools(&self, project_path: &Path) -> Option<Vec<ToolDefinition>> {
        let manager_arc = {
            let connections = self.connections.read().await;
            connections.get(project_path).cloned()?
        };
        let manager = manager_arc.read().await;
        if manager.is_connected() {
            manager.get_tools_for_ollama().await.ok()
        } else {
            None
        }
    }

    /// Call a tool on the MCP server connected for `project_path`.
    ///
    /// Acquires a brief read lock on the map to clone the server's Arc, then
    /// a read lock on the manager to execute the tool call — concurrent with
    /// other `call_tool` calls on the same server.
    pub async fn call_tool(
        &self,
        project_path: &Path,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<String, AppError> {
        let manager_arc = {
            let connections = self.connections.read().await;
            connections.get(project_path).cloned()
        }
        .ok_or(McpError::ConnectionLost)?;

        let manager = manager_arc.read().await;
        manager.call_tool(name, arguments).await.map_err(AppError::from)
    }

    /// Reconnect the MCP server for `project_path`.
    ///
    /// Acquires a write lock only on the specific server's manager, leaving
    /// all other servers unaffected. Used by the recovery system.
    pub async fn reconnect(&self, project_path: &Path) -> Result<(), McpError> {
        let manager_arc = {
            let connections = self.connections.read().await;
            connections.get(project_path).cloned()
        }
        .ok_or(McpError::ConnectionLost)?;

        let mut manager = manager_arc.write().await;

        // Explicit disconnect first — defense in depth.
        // McpManager::connect() already does client.take() internally, but making
        // the intent explicit avoids relying on that implementation detail.
        if manager.is_connected() {
            manager.disconnect().await;
        }

        let path_str = match manager.project_path() {
            Some(p) => p.to_string(),
            None => return Err(McpError::InitializationFailed {
                message: "manager has no project_path — cannot reconnect".into(),
            }),
        };

        manager.connect(&path_str).await
    }

    /// Remove a server permanently from the pool.
    ///
    /// Call ONLY when the user removes a server from their configuration.
    /// Do NOT call during reconnect/rebuild — those are temporary disconnections.
    pub async fn remove_server(&self, project_path: &Path) {
        let mut connections = self.connections.write().await;
        if let Some(manager_arc) = connections.remove(project_path) {
            // Best-effort disconnect — may fail if recovery is in flight
            if let Ok(mut manager) = manager_arc.try_write() {
                manager.disconnect().await;
            }
            // If try_write() fails: the Arc drops when the in-flight write lock is released
        }
    }

    /// Disconnect all active connections and clear the pool.
    pub async fn disconnect_all(&mut self) {
        let mut connections = self.connections.write().await;
        for (_, manager_arc) in connections.drain() {
            let mut manager = manager_arc.write().await;
            manager.disconnect().await;
        }
    }
}

impl Default for McpPool {
    fn default() -> Self {
        Self::new()
    }
}
