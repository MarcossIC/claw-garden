use serde::{Deserialize, Serialize};

/// Machine-readable error kind. Lets the model reason structurally.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    PermissionDenied,
    NotFound,
    InvalidInput,
    RateLimitExceeded,
    CircuitOpen,
    HookBlocked,
    Timeout,
    FileLocked,
    Io,
    Execution,
}

/// Semantic error message. Serialized as JSON into tool_result content.
/// is_retryable is ALWAYS false when the model sees this — retries are exhausted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolErrorMessage {
    pub error_kind: ErrorKind,
    pub is_retryable: bool,
    pub detail: String,
    pub suggested_action: String,
    pub context: Option<serde_json::Value>,
}

impl ToolErrorMessage {
    pub fn permission_denied(tool: &str, required: &str, current: &str) -> Self {
        Self {
            error_kind: ErrorKind::PermissionDenied,
            is_retryable: false,
            detail: format!(
                "{tool} requires {required}. Your current role is {current}."
            ),
            suggested_action: "Report your findings to the Orchestrator and request a permission upgrade. Do not retry — it will fail with the same error.".to_string(),
            context: Some(serde_json::json!({
                "required_role": required,
                "current_role": current,
            })),
        }
    }

    pub fn invalid_input(tool: &str, detail: &str) -> Self {
        Self {
            error_kind: ErrorKind::InvalidInput,
            is_retryable: false,
            detail: format!("Invalid input for {tool}: {detail}"),
            suggested_action: "Fix the input fields and retry.".to_string(),
            context: None,
        }
    }

    pub fn not_found(path: &str) -> Self {
        Self {
            error_kind: ErrorKind::NotFound,
            is_retryable: false,
            detail: format!("Path not found: {path}"),
            suggested_action: "Check the path and try again. Do not retry with the same path.".to_string(),
            context: None,
        }
    }

    pub fn rate_limit_exceeded(tool: &str, retry_after_secs: u64, calls_used: u32, limit_per_min: u32) -> Self {
        Self {
            error_kind: ErrorKind::RateLimitExceeded,
            is_retryable: false,
            detail: format!("{tool} called {calls_used}/{limit_per_min} times within the window. Max wait exceeded."),
            suggested_action: format!("Stop calling {tool}. Wait {retry_after_secs} seconds or proceed with information already gathered."),
            context: Some(serde_json::json!({
                "retry_after_secs": retry_after_secs,
                "calls_used": calls_used,
                "limit_per_min": limit_per_min,
            })),
        }
    }

    pub fn hook_blocked(hook: &str, blocked_path: &str, reason: Option<&str>) -> Self {
        Self {
            error_kind: ErrorKind::HookBlocked,
            is_retryable: false,
            detail: format!(
                "Pre-tool hook blocked the operation on '{}': {}",
                blocked_path,
                reason.unwrap_or("security policy")
            ),
            suggested_action: "This operation is protected by a security hook. Report to the Orchestrator — do not attempt to bypass.".to_string(),
            context: Some(serde_json::json!({
                "hook": hook,
                "blocked_path": blocked_path,
            })),
        }
    }

    pub fn timeout(tool: &str, timeout_secs: u64, attempts: u32) -> Self {
        Self {
            error_kind: ErrorKind::Timeout,
            is_retryable: false,
            detail: format!("{tool} timed out after {timeout_secs}s. Retried {attempts} time(s) — all attempts exhausted."),
            suggested_action: "The operation is taking too long. Try a simpler request or report to the user.".to_string(),
            context: Some(serde_json::json!({
                "timeout_secs": timeout_secs,
                "attempts": attempts,
            })),
        }
    }

    pub fn circuit_open(server: &str, retry_after_secs: u64, alternatives: &[&str]) -> Self {
        Self {
            error_kind: ErrorKind::CircuitOpen,
            is_retryable: false,
            detail: format!("MCP server '{server}' is temporarily unavailable (circuit open)."),
            suggested_action: format!(
                "Use native alternatives: {}. Or wait {retry_after_secs}s and try again.",
                alternatives.join(", ")
            ),
            context: Some(serde_json::json!({
                "server": server,
                "retry_after_secs": retry_after_secs,
                "native_alternatives": alternatives,
            })),
        }
    }

    pub fn io_error(tool: &str, detail: &str) -> Self {
        Self {
            error_kind: ErrorKind::Io,
            is_retryable: false,
            detail: format!("{tool} I/O error: {detail}"),
            suggested_action: "Check that the path exists and you have the required permissions.".to_string(),
            context: None,
        }
    }

    pub fn execution_error(tool: &str, detail: &str, exit_code: Option<i32>) -> Self {
        Self {
            error_kind: ErrorKind::Execution,
            is_retryable: false,
            detail: format!("{tool} failed: {detail}"),
            suggested_action: "Check the command and its arguments.".to_string(),
            context: exit_code.map(|c| serde_json::json!({ "exit_code": c })),
        }
    }

    /// Serialize to JSON string for embedding in tool_result content.
    pub fn to_json_string(&self) -> String {
        serde_json::to_string_pretty(self)
            .unwrap_or_else(|_| self.detail.clone())
    }

    /// True if the error is likely transient and a retry might succeed.
    ///
    /// Used by the executor's retry loop to decide whether to retry.
    /// Only `Timeout` and `Io` (network I/O) are transient.
    ///
    /// NOTE: when this error reaches the model, it's ALWAYS non-retryable
    /// (all internal retries were exhausted). `is_transient` is only read
    /// by the executor, never serialized to the model.
    pub fn is_transient(&self) -> bool {
        matches!(self.error_kind, ErrorKind::Timeout | ErrorKind::Io)
    }
}
