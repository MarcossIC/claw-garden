use crate::recovery::tool_failure::ToolFailureKind;

/// A single step in a recovery recipe.
///
/// `Clone`: pushed to `completed_steps` (Vec) and `remaining` (slice-to-vec).
/// `PartialEq`: assert_eq! over `PartialRecovery { completed, remaining }` in tests.
/// `Debug`: error messages in tests.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolRecoveryStep {
    /// Sleep for `delay_ms` (base, with jitter applied by `compute_delay`) then re-execute the tool.
    ///
    /// This step IS the execution — `attempt_tool_recovery` calls `retry_fn` here.
    RetryToolCall { delay_ms: u64 },

    /// Cancel the existing MCP client, respawn the process, and perform a new handshake.
    ///
    /// `server_name` field removed — `McpPool` uses `project_path` as key.
    ReconnectMcp,

    /// Refresh the OAuth token before the next retry.
    RefreshOAuth,

    /// Escalate to the LLM immediately — tell it what failed and why.
    /// Used for permanent failures (`ToolNotFound`, `ToolInputInvalid`) where retrying is futile.
    EscalateToLlm,

    /// Abort the current session with a reason string.
    /// Reserved for catastrophic, unrecoverable failures. No current recipe uses this.
    AbortSession { reason: String },
}

impl ToolRecoveryStep {
    /// True if this step interacts with `McpPool` (documentation only).
    pub fn needs_mcp_lock(&self) -> bool {
        matches!(self, Self::ReconnectMcp)
    }
}

/// What to do once all recovery steps are exhausted.
///
/// `Clone`: stored in `ToolRecoveryEvent::Escalated`.
/// `PartialEq + Debug`: comparisons and logging in tests.
#[derive(Debug, Clone, PartialEq)]
pub enum EscalationPolicy {
    InformLlm,
    LogAndContinue,
    AbortSession,
}

/// A full recovery recipe for a specific `ToolFailureKind`.
pub struct ToolRecoveryRecipe {
    pub kind: ToolFailureKind,
    pub steps: Vec<ToolRecoveryStep>,
    pub max_attempts: u32,
    pub escalation_policy: EscalationPolicy,
}

/// Returns the recovery recipe for the given failure kind.
///
/// Recipe table:
///
/// | Kind                 | Steps                                 | max_attempts | Escalation |
/// |----------------------|---------------------------------------|:------------:|------------|
/// | McpConnectionLost    | [ReconnectMcp, RetryToolCall{500ms}]  | 1            | InformLlm  |
/// | McpHandshakeTimeout  | [ReconnectMcp, RetryToolCall{1000ms}] | 1            | InformLlm  |
/// | ToolNotFound         | [EscalateToLlm]                       | 1            | InformLlm  |
/// | ToolInputInvalid     | [EscalateToLlm]                       | 1            | InformLlm  |
/// | ToolPermissionDenied | [RefreshOAuth, RetryToolCall{200ms}]  | 1            | InformLlm  |
/// | ToolTimeout          | [RetryToolCall{2000ms}]               | 2            | InformLlm  |
/// | ToolTransientError   | [RetryToolCall{500ms}]                | 2            | InformLlm  |
pub fn recipe_for(kind: &ToolFailureKind) -> ToolRecoveryRecipe {
    use ToolFailureKind::*;
    use ToolRecoveryStep::*;

    match kind {
        McpConnectionLost => ToolRecoveryRecipe {
            kind: kind.clone(),
            steps: vec![ReconnectMcp, RetryToolCall { delay_ms: 500 }],
            max_attempts: 1,
            escalation_policy: EscalationPolicy::InformLlm,
        },
        McpHandshakeTimeout => ToolRecoveryRecipe {
            kind: kind.clone(),
            steps: vec![ReconnectMcp, RetryToolCall { delay_ms: 1000 }],
            max_attempts: 1,
            escalation_policy: EscalationPolicy::InformLlm,
        },
        ToolNotFound => ToolRecoveryRecipe {
            kind: kind.clone(),
            steps: vec![EscalateToLlm],
            max_attempts: 1,
            escalation_policy: EscalationPolicy::InformLlm,
        },
        ToolInputInvalid => ToolRecoveryRecipe {
            kind: kind.clone(),
            steps: vec![EscalateToLlm],
            max_attempts: 1,
            escalation_policy: EscalationPolicy::InformLlm,
        },
        ToolPermissionDenied => ToolRecoveryRecipe {
            kind: kind.clone(),
            steps: vec![RefreshOAuth, RetryToolCall { delay_ms: 200 }],
            max_attempts: 1,
            escalation_policy: EscalationPolicy::InformLlm,
        },
        ToolTimeout => ToolRecoveryRecipe {
            kind: kind.clone(),
            steps: vec![RetryToolCall { delay_ms: 2000 }],
            max_attempts: 2,
            escalation_policy: EscalationPolicy::InformLlm,
        },
        ToolTransientError => ToolRecoveryRecipe {
            kind: kind.clone(),
            steps: vec![RetryToolCall { delay_ms: 500 }],
            max_attempts: 2,
            escalation_policy: EscalationPolicy::InformLlm,
        },
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ToolFailureKind::*;

    #[test]
    fn mcp_connection_lost_recipe_has_reconnect_and_retry() {
        let r = recipe_for(&McpConnectionLost);
        assert!(!r.steps.is_empty());
        assert_eq!(r.max_attempts, 1);
        assert_eq!(r.escalation_policy, EscalationPolicy::InformLlm);
        assert!(r.steps.contains(&ToolRecoveryStep::ReconnectMcp));
    }

    #[test]
    fn mcp_handshake_timeout_recipe_has_reconnect_and_retry() {
        let r = recipe_for(&McpHandshakeTimeout);
        assert_eq!(r.max_attempts, 1);
        assert!(r.steps.contains(&ToolRecoveryStep::ReconnectMcp));
    }

    #[test]
    fn tool_not_found_recipe_has_escalate_to_llm_only() {
        let r = recipe_for(&ToolNotFound);
        assert_eq!(r.steps, vec![ToolRecoveryStep::EscalateToLlm]);
        assert_eq!(r.max_attempts, 1);
    }

    #[test]
    fn tool_input_invalid_recipe_has_escalate_to_llm_only() {
        let r = recipe_for(&ToolInputInvalid);
        assert_eq!(r.steps, vec![ToolRecoveryStep::EscalateToLlm]);
    }

    #[test]
    fn tool_permission_denied_recipe_has_oauth_refresh_then_retry() {
        let r = recipe_for(&ToolPermissionDenied);
        assert_eq!(r.steps[0], ToolRecoveryStep::RefreshOAuth);
        assert!(matches!(r.steps[1], ToolRecoveryStep::RetryToolCall { .. }));
        assert_eq!(r.escalation_policy, EscalationPolicy::InformLlm);
    }

    #[test]
    fn tool_timeout_recipe_has_two_attempts() {
        let r = recipe_for(&ToolTimeout);
        assert_eq!(r.max_attempts, 2);
        assert!(matches!(r.steps[0], ToolRecoveryStep::RetryToolCall { delay_ms: 2000 }));
    }

    #[test]
    fn tool_transient_recipe_has_two_attempts() {
        let r = recipe_for(&ToolTransientError);
        assert_eq!(r.max_attempts, 2);
        assert!(matches!(r.steps[0], ToolRecoveryStep::RetryToolCall { delay_ms: 500 }));
    }

    #[test]
    fn all_recipes_have_non_empty_steps() {
        let kinds = [
            McpConnectionLost, McpHandshakeTimeout, ToolNotFound, ToolInputInvalid,
            ToolPermissionDenied, ToolTimeout, ToolTransientError,
        ];
        for kind in &kinds {
            let r = recipe_for(kind);
            assert!(!r.steps.is_empty(), "recipe for {kind:?} has no steps");
        }
    }
}
