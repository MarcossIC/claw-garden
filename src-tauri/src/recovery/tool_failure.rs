use crate::models::{AppError, LlmError, McpError};

/// Semantic classification of a tool failure.
///
/// `Hash + Eq`: used as key in `HashMap<(String, ToolFailureKind), AttemptRecord>`.
/// `Clone`: stored in `ToolRecoveryEvent`.
/// `PartialEq + Debug`: required by tests.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ToolFailureKind {
    /// MCP server process died or transport was closed.
    McpConnectionLost,
    /// MCP server did not respond within the configured timeout.
    McpHandshakeTimeout,
    /// Tool does not exist on the server (permanent — cannot retry).
    ToolNotFound,
    /// Tool parameters are invalid (permanent — cannot retry meaningfully).
    ToolInputInvalid,
    /// Auth / permissions failure — may recover with OAuth refresh.
    ToolPermissionDenied,
    /// Tool executed but took too long.
    ToolTimeout,
    /// Generic transient error with a chance of recovery.
    ToolTransientError,
}

impl ToolFailureKind {
    /// Classify an `AppError` into the corresponding kind.
    ///
    /// Pure pattern matching over `McpError` variants — no string parsing.
    /// Never panics: unknown errors fall back to `ToolTransientError`.
    pub fn from_error(err: &AppError) -> Self {
        match err {
            AppError::Mcp(McpError::ConnectionLost) => Self::McpConnectionLost,
            AppError::Mcp(McpError::InitializationFailed { .. }) => Self::McpConnectionLost,
            AppError::Mcp(McpError::Timeout { .. }) => Self::McpHandshakeTimeout,
            AppError::Mcp(McpError::ToolNotFound { .. }) => Self::ToolNotFound,
            AppError::Mcp(McpError::InvalidParams { .. }) => Self::ToolInputInvalid,
            AppError::Mcp(McpError::Cancelled { .. }) => Self::ToolTransientError,
            AppError::Mcp(McpError::Protocol { .. }) => Self::ToolTransientError,
            AppError::Llm(LlmError::Authentication(_)) => Self::ToolPermissionDenied,
            AppError::AuthExpired(_) => Self::ToolPermissionDenied,
            _ => Self::ToolTransientError,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn mcp_connection_lost() -> AppError {
        AppError::Mcp(McpError::ConnectionLost)
    }
    fn mcp_timeout() -> AppError {
        AppError::Mcp(McpError::Timeout { elapsed: Duration::from_secs(5) })
    }
    fn mcp_tool_not_found() -> AppError {
        AppError::Mcp(McpError::ToolNotFound { message: "no such tool".into() })
    }
    fn mcp_invalid_params() -> AppError {
        AppError::Mcp(McpError::InvalidParams { message: "bad params".into() })
    }
    fn mcp_protocol() -> AppError {
        AppError::Mcp(McpError::Protocol { code: -32000, message: "internal".into() })
    }
    fn auth_expired() -> AppError {
        AppError::AuthExpired("token expired".into())
    }
    fn unknown_error() -> AppError {
        AppError::Internal("some internal failure".into())
    }

    #[test]
    fn connection_lost_classified_as_mcp_connection_lost() {
        assert_eq!(ToolFailureKind::from_error(&mcp_connection_lost()), ToolFailureKind::McpConnectionLost);
    }

    #[test]
    fn initialization_failed_classified_as_mcp_connection_lost() {
        let err = AppError::Mcp(McpError::InitializationFailed { message: "spawn failed".into() });
        assert_eq!(ToolFailureKind::from_error(&err), ToolFailureKind::McpConnectionLost);
    }

    #[test]
    fn timeout_classified_as_mcp_handshake_timeout() {
        assert_eq!(ToolFailureKind::from_error(&mcp_timeout()), ToolFailureKind::McpHandshakeTimeout);
    }

    #[test]
    fn tool_not_found_classified_as_tool_not_found() {
        assert_eq!(ToolFailureKind::from_error(&mcp_tool_not_found()), ToolFailureKind::ToolNotFound);
    }

    #[test]
    fn invalid_params_classified_as_tool_input_invalid() {
        assert_eq!(ToolFailureKind::from_error(&mcp_invalid_params()), ToolFailureKind::ToolInputInvalid);
    }

    #[test]
    fn protocol_error_classified_as_transient() {
        assert_eq!(ToolFailureKind::from_error(&mcp_protocol()), ToolFailureKind::ToolTransientError);
    }

    #[test]
    fn auth_expired_classified_as_permission_denied() {
        assert_eq!(ToolFailureKind::from_error(&auth_expired()), ToolFailureKind::ToolPermissionDenied);
    }

    #[test]
    fn unknown_app_error_falls_back_to_transient() {
        assert_eq!(ToolFailureKind::from_error(&unknown_error()), ToolFailureKind::ToolTransientError);
    }

    #[test]
    fn cancelled_classified_as_transient() {
        let err = AppError::Mcp(McpError::Cancelled { reason: Some("stop".into()) });
        assert_eq!(ToolFailureKind::from_error(&err), ToolFailureKind::ToolTransientError);
    }
}
