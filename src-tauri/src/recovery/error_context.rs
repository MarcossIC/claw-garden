use serde::Serialize;

use clawg_core::models::{AppError, McpError};
use crate::recovery::tool_failure::ToolFailureKind;

/// Structured error context sent to the LLM when recovery fails.
///
/// Invariant: the LLM receives ONE final error — not the history of intermediate attempts.
/// `ToolRecoveryContext` tracks the steps; only this final structure reaches the message history.
#[derive(Debug, Serialize)]
struct ErrorContext<'a> {
    tool_name: &'a str,
    error_kind: &'static str,
    message: String,
    recovery_attempted: bool,
    recovery_outcome: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<String>,
}

/// Build the structured error JSON that gets sent to the LLM as the tool result.
///
/// `recovery_outcome` is a short string describing what happened:
/// - `"escalated_to_llm"` — permanent failure, no retry possible
/// - `"max_attempts_exhausted"` — all retries failed
/// - `"partial-recovery"` — preparation step failed mid-recovery
/// - `"recovery-step-timeout"` — a preparation step timed out
/// - `"oauth-refresh-failed"` / `"oauth-refresh-timeout"` — token refresh problem
pub fn build_error_context(
    tool_name: &str,
    error: &AppError,
    recovery_outcome: &str,
) -> String {
    let kind = ToolFailureKind::from_error(error);
    let recovery_attempted = !matches!(recovery_outcome, "escalated_to_llm");

    let ctx = ErrorContext {
        tool_name,
        error_kind: kind_to_str(&kind),
        message: error.to_string(),
        recovery_attempted,
        recovery_outcome,
        suggestion: suggestion_for(&kind, tool_name),
    };

    serde_json::to_string(&ctx).unwrap_or_else(|_| {
        format!(
            r#"{{"tool_name":"{tool_name}","error_kind":"unknown","message":"{}","recovery_outcome":"{recovery_outcome}"}}"#,
            error
        )
    })
}

fn kind_to_str(kind: &ToolFailureKind) -> &'static str {
    match kind {
        ToolFailureKind::McpConnectionLost => "mcp_connection_lost",
        ToolFailureKind::McpHandshakeTimeout => "mcp_handshake_timeout",
        ToolFailureKind::ToolNotFound => "tool_not_found",
        ToolFailureKind::ToolInputInvalid => "tool_input_invalid",
        ToolFailureKind::ToolPermissionDenied => "tool_permission_denied",
        ToolFailureKind::ToolTimeout => "tool_timeout",
        ToolFailureKind::ToolTransientError => "tool_transient_error",
    }
}

fn suggestion_for(kind: &ToolFailureKind, tool_name: &str) -> Option<String> {
    match kind {
        ToolFailureKind::ToolPermissionDenied => Some(format!(
            "OAuth token refresh failed for tool '{tool_name}'. The user may need to re-authenticate. \
             You can continue working on tasks that don't require this tool and ask the user to \
             re-authenticate when convenient."
        )),
        ToolFailureKind::McpConnectionLost => Some(
            "The MCP server was restarted during the operation. A reconnection was attempted. \
             You may retry the tool call."
                .into(),
        ),
        ToolFailureKind::ToolNotFound => Some(
            "This tool does not exist on the connected MCP server. \
             Check the available tools and use an alternative if one exists."
                .into(),
        ),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn transient_err() -> AppError {
        AppError::Mcp(McpError::Cancelled { reason: None })
    }
    fn not_found_err() -> AppError {
        AppError::Mcp(McpError::ToolNotFound { message: "no such tool".into() })
    }

    #[test]
    fn output_is_valid_json() {
        let json = build_error_context("bash", &transient_err(), "max_attempts_exhausted");
        assert!(serde_json::from_str::<serde_json::Value>(&json).is_ok());
    }

    #[test]
    fn output_includes_required_fields() {
        let json = build_error_context("bash", &transient_err(), "max_attempts_exhausted");
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(val.get("tool_name").is_some());
        assert!(val.get("error_kind").is_some());
        assert!(val.get("message").is_some());
        assert!(val.get("recovery_outcome").is_some());
    }

    #[test]
    fn tool_not_found_has_suggestion() {
        let json = build_error_context("bash", &not_found_err(), "escalated_to_llm");
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(val.get("suggestion").is_some());
    }

    #[test]
    fn transient_error_has_no_suggestion() {
        let json = build_error_context("bash", &transient_err(), "max_attempts_exhausted");
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(val.get("suggestion").is_none());
    }

    #[test]
    fn escalated_to_llm_marks_recovery_not_attempted() {
        let json = build_error_context("bash", &not_found_err(), "escalated_to_llm");
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["recovery_attempted"], false);
    }

    #[test]
    fn max_attempts_exhausted_marks_recovery_attempted() {
        let json = build_error_context("bash", &transient_err(), "max_attempts_exhausted");
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["recovery_attempted"], true);
    }
}
