use std::time::Duration;

use serde::Serialize;

/// Typed error for MCP protocol failures.
/// Mapped from rmcp::service::ServiceError and rmcp::service::ClientInitializeError in mcp/client.rs.
/// AppError::Mcp now contains this instead of a catch-all String.
#[derive(Debug, thiserror::Error, Clone)]
pub enum McpError {
    /// Transport closed — the MCP server died or disconnected.
    /// Origin: ServiceError::TransportClosed | ServiceError::TransportSend(_) | ClientInitializeError::ConnectionClosed
    #[error("MCP connection lost")]
    ConnectionLost,

    /// Operation timed out.
    /// Origin: ServiceError::Timeout { timeout }
    #[error("MCP operation timed out after {elapsed:?}")]
    Timeout { elapsed: Duration },

    /// The MCP server rejected the tool with JSON-RPC -32601 (method not found).
    /// Origin: ErrorData { code: -32601 }
    #[error("tool not found: {message}")]
    ToolNotFound { message: String },

    /// Invalid parameters — JSON-RPC -32602 (invalid params).
    /// Origin: ErrorData { code: -32602 }
    #[error("invalid tool parameters: {message}")]
    InvalidParams { message: String },

    /// Operation cancelled.
    /// Origin: ServiceError::Cancelled { reason }
    #[error("MCP operation cancelled: {reason:?}")]
    Cancelled { reason: Option<String> },

    /// Failed to initialize the connection with the server.
    /// Origin: ClientInitializeError variants, io::Error from spawn, invalid path
    #[error("MCP initialization failed: {message}")]
    InitializationFailed { message: String },

    /// Unclassified protocol error — includes ErrorData with other JSON-RPC codes.
    /// Fallback for non-exhaustive ServiceError and RmcpError variants.
    #[error("MCP protocol error (code {code}): {message}")]
    Protocol { code: i32, message: String },
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("Authentication failed: {0}")]
    Authentication(String),

    #[error("Rate limit exceeded. Retry after {retry_after_secs}s")]
    RateLimit { retry_after_secs: u64 },

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Provider unavailable: {0}")]
    ProviderUnavailable(String),

    #[error("Safety refusal: {0}")]
    Refusal(String),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Streaming error: {0}")]
    Streaming(String),

    #[error(transparent)]
    Mcp(#[from] McpError),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Session error: {0}")]
    Session(String),

    #[error("Authentication expired: {0}")]
    AuthExpired(String),

    #[error(transparent)]
    Llm(#[from] LlmError),

    #[error(transparent)]
    Memory(#[from] crate::memory::MemoryError),
}

impl From<crate::session::SessionError> for AppError {
    fn from(e: crate::session::SessionError) -> Self {
        AppError::Session(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryError;

    /// B.3-test — SPEC-8 Sc 8.1, 8.2 — From<MemoryError> for AppError works.
    #[test]
    fn from_memory_error_to_app_error() {
        let mem_err = MemoryError::VecExtensionRegistration("x".into());
        let app_err: AppError = mem_err.into();
        assert!(
            matches!(app_err, AppError::Memory(_)),
            "Expected AppError::Memory, got {:?}",
            app_err
        );
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
