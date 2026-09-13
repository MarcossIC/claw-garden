//! `clawg mcp` — the stdio bridge between an MCP host and a running daemon.
//!
//! # stdout is the transport
//!
//! Every byte this process writes to stdout is an MCP frame. A stray `println!`
//! — a progress line, a warning, a panic message — corrupts the stream and the
//! host disconnects with an error that points nowhere near the cause. So
//! logging is initialised against stderr in `main`, and nothing in this module
//! or in [`server`] ever writes to stdout directly.

pub mod server;
pub mod tools;

use clawg_core::paths::{data_dir, Profile};
use rmcp::transport::stdio;
use rmcp::ServiceExt;

use crate::client::{ensure_running, ClientError, ProcessSpawner};
use server::ClawgMcp;

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("the MCP stdio server failed: {0}")]
    Serve(String),
}

/// Connects to (or starts) a daemon, then serves MCP over stdio until the host
/// closes the stream.
pub async fn run(profile: Profile, autostart: bool) -> Result<(), McpError> {
    let base_dir = data_dir(profile);
    let spawner = ProcessSpawner { base_dir };
    let client = ensure_running(profile, autostart, &spawner).await?;

    log::info!(
        "clawg mcp {} bridging to {}",
        profile.as_str(),
        client.base_url()
    );

    let service = ClawgMcp::new(client, profile)
        .serve(stdio())
        .await
        .map_err(|e| McpError::Serve(e.to_string()))?;

    // Returns when the host disconnects; the daemon keeps running, which is the
    // point of the split — closing an editor must not discard the memory index.
    service
        .waiting()
        .await
        .map_err(|e| McpError::Serve(e.to_string()))?;
    Ok(())
}
