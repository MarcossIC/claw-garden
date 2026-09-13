use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::time::Duration;

use clawg_core::models::{AppError, McpError, StreamEvent};
use crate::recovery::recipes::{EscalationPolicy, ToolRecoveryStep, recipe_for};
use crate::recovery::tool_failure::ToolFailureKind;
use clawg_core::services::auth::ensure_oauth_fresh;
use clawg_core::services::retry::{RetryConfig, compute_delay};
use crate::state::AppState;

/// Decay window for attempt counters.
///
/// Entries older than this are reset on the next `record_attempt()`.
/// Allows recovery after a server has been down for a while and recovered,
/// without accumulating stale counts across unrelated failure bursts.
///
/// Uses `tokio::time::Instant` (not `std::time`) so tests can use
/// `#[tokio::test(start_paused = true)]` + `tokio::time::advance()`.
const ATTEMPT_DECAY_WINDOW: Duration = Duration::from_secs(300); // 5 minutes

/// Per-(tool, kind) attempt record with decay support.
struct AttemptRecord {
    count: u32,
    last_attempt_at: tokio::time::Instant,
}

/// Events emitted by the recovery system for auditing.
#[derive(Debug)]
pub enum ToolRecoveryEvent {
    RecoveryAttempted { tool: String, kind: ToolFailureKind, result: ToolRecoveryResult },
    RecoverySucceeded { tool: String },
    RecoveryFailed { tool: String },
    Escalated { tool: String, policy: EscalationPolicy },
}

/// The outcome returned by `attempt_tool_recovery`.
///
/// `PartialEq + Debug`: required by tests.
/// `ToolRecoveryStep: PartialEq` is transitively required by `PartialRecovery`.
#[derive(Debug, PartialEq)]
pub enum ToolRecoveryResult {
    /// Recovery succeeded. `content` is the tool output — `chat.rs` does NOT re-execute.
    Recovered { content: String },
    /// A preparation step failed mid-way. `completed` = what ran, `remaining` = what didn't.
    PartialRecovery { completed: Vec<ToolRecoveryStep>, remaining: Vec<ToolRecoveryStep> },
    /// All attempts exhausted, or a permanent failure (ToolNotFound, etc.).
    EscalationRequired { reason: String },
}

/// Error returned by preparation steps.
#[derive(Debug)]
pub enum StepError {
    Mcp(McpError),
    Auth(AppError),
    /// The step did not complete within its time budget.
    /// Escalates immediately — a timeout means the server is not responding.
    Timeout { step_name: &'static str, budget: Duration },
}

/// Tracks recovery attempts per `(tool_name, ToolFailureKind)` across tool calls.
///
/// Lives for the duration of an agentic session — tracking is cumulative.
/// Never persisted across crashes (see docs/recovery-error-handling-plan.md — "Fuera de Scope").
pub struct ToolRecoveryContext {
    attempts: HashMap<(String, ToolFailureKind), AttemptRecord>,
    events: Vec<ToolRecoveryEvent>,
    /// Simulation knob for tests: `Some(n)` → fail at step index `n` without real I/O.
    pub fail_at_step: Option<usize>,
}

impl ToolRecoveryContext {
    pub fn new() -> Self {
        Self {
            attempts: HashMap::new(),
            events: Vec::new(),
            fail_at_step: None,
        }
    }

    /// Record an attempt for `(tool, kind)`.
    ///
    /// If the last attempt was more than `ATTEMPT_DECAY_WINDOW` ago, resets the counter
    /// first (the server likely recovered). Returns the count AFTER the increment.
    pub fn record_attempt(&mut self, tool: &str, kind: &ToolFailureKind) -> u32 {
        let key = (tool.to_string(), kind.clone());
        let now = tokio::time::Instant::now();
        let record = self.attempts.entry(key).or_insert(AttemptRecord {
            count: 0,
            last_attempt_at: now,
        });

        if record.last_attempt_at.elapsed() > ATTEMPT_DECAY_WINDOW {
            record.count = 0;
        }

        record.count += 1;
        record.last_attempt_at = now;
        record.count
    }

    /// Reset the attempt counter for `(tool, kind)` immediately.
    ///
    /// Called on success — confirms the server is healthy, so stale burst counts
    /// should not affect the next failure.
    pub fn reset_attempts(&mut self, tool: &str, kind: &ToolFailureKind) {
        self.attempts.remove(&(tool.to_string(), kind.clone()));
    }

    pub fn push_event(&mut self, event: ToolRecoveryEvent) {
        self.events.push(event);
    }

    pub fn events(&self) -> &[ToolRecoveryEvent] {
        &self.events
    }
}

impl Default for ToolRecoveryContext {
    fn default() -> Self {
        Self::new()
    }
}

// ── Time budget per preparation step ─────────────────────────────────────────

/// Maximum time allowed for a preparation step.
///
/// `RetryToolCall`, `EscalateToLlm`, and `AbortSession` are terminal — not preparation steps.
fn step_budget(step: &ToolRecoveryStep) -> Duration {
    match step {
        // Full rebuild: disconnect + respawn npx process + MCP handshake
        ToolRecoveryStep::ReconnectMcp => Duration::from_secs(15),
        // HTTP token refresh request
        ToolRecoveryStep::RefreshOAuth => Duration::from_secs(5),
        _ => unreachable!("step_budget called on a non-preparation step"),
    }
}

fn step_name_str(step: &ToolRecoveryStep) -> &'static str {
    match step {
        ToolRecoveryStep::ReconnectMcp => "ReconnectMcp",
        ToolRecoveryStep::RefreshOAuth => "RefreshOAuth",
        _ => unreachable!(),
    }
}

/// Execute a preparation step that interacts with McpPool, with a timeout.
///
/// Does NOT hold any pool lock — `McpPool` manages its own per-server `RwLock` internally.
/// On timeout: returns `StepError::Timeout` → caller escalates immediately.
async fn execute_mcp_step(
    step: &ToolRecoveryStep,
    state: &AppState,
    project_path: &Path,
) -> Result<(), StepError> {
    let budget = step_budget(step);
    let step_name = step_name_str(step);

    let fut = async {
        match step {
            ToolRecoveryStep::ReconnectMcp => {
                state
                    .mcp_pool
                    .reconnect(project_path)
                    .await
                    .map_err(StepError::Mcp)
            }
            _ => unreachable!("non-MCP step passed to execute_mcp_step"),
        }
    };

    tokio::time::timeout(budget, fut)
        .await
        .unwrap_or(Err(StepError::Timeout { step_name, budget }))
}

// ── attempt_tool_recovery ─────────────────────────────────────────────────────

/// Attempt to recover from a tool failure, executing the recipe for `kind`.
///
/// The full retry loop lives here — `chat.rs` calls once and receives the final result.
///
/// - `retry_fn`: the closure that executes the actual tool call. Called once per attempt.
/// - `emit`: callback invoked on each recovery step/result event. In production this wraps
///   `Channel::send`; in tests it can be a plain closure, avoiding the Tauri/WebView2 linkage.
/// - `state`: `AppState` reference for MCP pool and OAuth lock access.
/// - `project_path`: used as the key into `McpPool`.
///
/// The MCP pool lock MUST NOT be held when calling this function.
pub async fn attempt_tool_recovery<F, Fut, E>(
    ctx: &mut ToolRecoveryContext,
    tool_name: &str,
    kind: ToolFailureKind,
    state: &AppState,
    project_path: &Path,
    emit: &E,
    retry_fn: F,
) -> ToolRecoveryResult
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<String, AppError>>,
    E: Fn(StreamEvent),
{
    let recipe = recipe_for(&kind);

    for attempt in 1..=recipe.max_attempts {
        ctx.record_attempt(tool_name, &kind);
        emit(StreamEvent::RecoveryAttempt {
            tool: tool_name.into(),
            step: format!("attempt {}/{}", attempt, recipe.max_attempts),
            attempt,
        });

        let mut completed_steps: Vec<ToolRecoveryStep> = vec![];

        for (step_idx, step) in recipe.steps.iter().enumerate() {
            // Test simulation knob — fail at this step without real I/O
            if ctx.fail_at_step == Some(step_idx) {
                return ToolRecoveryResult::PartialRecovery {
                    completed: completed_steps,
                    remaining: recipe.steps[step_idx..].to_vec(),
                };
            }

            match step {
                ToolRecoveryStep::RetryToolCall { delay_ms } => {
                    // This step IS the actual tool execution.
                    //
                    // Build RetryConfig with the recipe's base delay. compute_delay is
                    // 0-based (attempt=0 → base delay) so we pass (attempt - 1) to keep
                    // the first retry at ~delay_ms and subsequent ones exponentially higher.
                    let retry_config = RetryConfig {
                        initial_delay_ms: *delay_ms,
                        ..RetryConfig::default()
                    };
                    let delay = compute_delay(&retry_config, attempt - 1);
                    tokio::time::sleep(delay).await;

                    emit(StreamEvent::RecoveryAttempt {
                            tool: tool_name.into(),
                            step: "Retrying tool call...".into(),
                            attempt,
                        });

                    match retry_fn().await {
                        Ok(content) => {
                            ctx.reset_attempts(tool_name, &kind);
                            ctx.push_event(ToolRecoveryEvent::RecoverySucceeded {
                                tool: tool_name.into(),
                            });
                            emit(StreamEvent::RecoveryResult {
                                    tool: tool_name.into(),
                                    succeeded: true,
                                    outcome: "recovered".into(),
                                });
                            return ToolRecoveryResult::Recovered { content };
                        }
                        Err(_) => {
                            // Tool still failing — break the inner step loop, try next attempt
                            break;
                        }
                    }
                }

                ToolRecoveryStep::EscalateToLlm => {
                    emit(StreamEvent::RecoveryResult {
                            tool: tool_name.into(),
                            succeeded: false,
                            outcome: "escalated_to_llm".into(),
                        });
                    ctx.push_event(ToolRecoveryEvent::Escalated {
                        tool: tool_name.into(),
                        policy: recipe.escalation_policy.clone(),
                    });
                    return ToolRecoveryResult::EscalationRequired {
                        reason: "escalated_to_llm".into(),
                    };
                }

                ToolRecoveryStep::AbortSession { reason } => {
                    ctx.push_event(ToolRecoveryEvent::Escalated {
                        tool: tool_name.into(),
                        policy: EscalationPolicy::AbortSession,
                    });
                    return ToolRecoveryResult::EscalationRequired {
                        reason: reason.clone(),
                    };
                }

                ToolRecoveryStep::RefreshOAuth => {
                    let budget = step_budget(step);
                    match tokio::time::timeout(
                        budget,
                        ensure_oauth_fresh(&state.oauth_refresh_lock),
                    )
                    .await
                    {
                        Ok(Ok(_)) => {
                            completed_steps.push(step.clone());
                            // continue to next step
                        }
                        Ok(Err(_e)) => {
                            emit(StreamEvent::RecoveryResult {
                                    tool: tool_name.into(),
                                    succeeded: false,
                                    outcome: "oauth-refresh-failed".into(),
                                });
                            return ToolRecoveryResult::EscalationRequired {
                                reason: "oauth-refresh-failed".into(),
                            };
                        }
                        Err(_elapsed) => {
                            emit(StreamEvent::RecoveryResult {
                                    tool: tool_name.into(),
                                    succeeded: false,
                                    outcome: format!(
                                        "oauth-refresh-timeout-{}s",
                                        budget.as_secs()
                                    ),
                                });
                            return ToolRecoveryResult::EscalationRequired {
                                reason: "oauth-refresh-timeout".into(),
                            };
                        }
                    }
                }

                ToolRecoveryStep::ReconnectMcp => {
                    // RecoveryGate: serializes recoveries per server, enables fast-path skip
                    let _permit = state.mcp_recovery_gate.acquire(project_path).await;

                    // Fast-path: another agent may have already restored the connection
                    if state.mcp_pool.is_connected_for_path(project_path).await {
                        completed_steps.push(step.clone());
                        continue;
                    }

                    match execute_mcp_step(step, state, project_path).await {
                        Ok(()) => {
                            completed_steps.push(step.clone());
                            // permit drops here — next step continues
                        }
                        Err(step_err) => {
                            let outcome = match &step_err {
                                StepError::Timeout { step_name, budget } => {
                                    format!("{step_name}-timeout-{}s", budget.as_secs())
                                }
                                StepError::Mcp(_) | StepError::Auth(_) => "step-failed".into(),
                            };
                            emit(StreamEvent::RecoveryResult {
                                    tool: tool_name.into(),
                                    succeeded: false,
                                    outcome,
                                });
                            return match step_err {
                                StepError::Timeout { .. } => {
                                    ToolRecoveryResult::EscalationRequired {
                                        reason: "recovery-step-timeout".into(),
                                    }
                                }
                                _ => ToolRecoveryResult::PartialRecovery {
                                    completed: completed_steps,
                                    remaining: recipe.steps[step_idx..].to_vec(),
                                },
                            };
                        }
                    }
                }
            }
        }
        // Inner step loop completed without returning Recovered or Escalation.
        // This happens when RetryToolCall broke (tool still failing).
        // The outer loop tries the next attempt.
    }

    // All attempts exhausted
    ctx.push_event(ToolRecoveryEvent::Escalated {
        tool: tool_name.into(),
        policy: recipe.escalation_policy.clone(),
    });
    emit(StreamEvent::RecoveryResult {
        tool: tool_name.into(),
        succeeded: false,
        outcome: "max_attempts_exhausted".into(),
    });
    ToolRecoveryResult::EscalationRequired {
        reason: "max_attempts_exhausted".into(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── ToolRecoveryContext: attempt tracking and decay ───────────────────────

    #[tokio::test(start_paused = true)]
    async fn fresh_attempt_starts_at_one() {
        let mut ctx = ToolRecoveryContext::new();
        let count = ctx.record_attempt("bash", &ToolFailureKind::McpConnectionLost);
        assert_eq!(count, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn second_attempt_within_window_accumulates() {
        let mut ctx = ToolRecoveryContext::new();
        let c1 = ctx.record_attempt("bash", &ToolFailureKind::McpConnectionLost);
        let c2 = ctx.record_attempt("bash", &ToolFailureKind::McpConnectionLost);
        assert_eq!(c1, 1);
        assert_eq!(c2, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn attempt_count_resets_after_decay_window() {
        let mut ctx = ToolRecoveryContext::new();
        ctx.record_attempt("bash", &ToolFailureKind::McpConnectionLost);

        tokio::time::advance(Duration::from_secs(301)).await;

        let count = ctx.record_attempt("bash", &ToolFailureKind::McpConnectionLost);
        assert_eq!(count, 1, "should reset after decay window");
    }

    #[tokio::test(start_paused = true)]
    async fn attempt_count_does_not_reset_before_decay_window() {
        let mut ctx = ToolRecoveryContext::new();
        ctx.record_attempt("bash", &ToolFailureKind::McpConnectionLost);

        tokio::time::advance(Duration::from_secs(299)).await;

        let count = ctx.record_attempt("bash", &ToolFailureKind::McpConnectionLost);
        assert_eq!(count, 2, "should NOT reset before decay window");
    }

    #[tokio::test(start_paused = true)]
    async fn reset_on_success_clears_count_immediately() {
        let mut ctx = ToolRecoveryContext::new();
        ctx.record_attempt("bash", &ToolFailureKind::McpConnectionLost);
        ctx.record_attempt("bash", &ToolFailureKind::McpConnectionLost);
        ctx.reset_attempts("bash", &ToolFailureKind::McpConnectionLost);

        let count = ctx.record_attempt("bash", &ToolFailureKind::McpConnectionLost);
        assert_eq!(count, 1, "should start fresh after explicit reset");
    }

    #[tokio::test(start_paused = true)]
    async fn decay_is_per_kind_independent() {
        let mut ctx = ToolRecoveryContext::new();
        ctx.record_attempt("bash", &ToolFailureKind::McpConnectionLost);

        tokio::time::advance(Duration::from_secs(301)).await;

        // McpConnectionLost is stale → will reset
        let c1 = ctx.record_attempt("bash", &ToolFailureKind::McpConnectionLost);
        // ToolTransientError is fresh (never recorded before)
        let c2 = ctx.record_attempt("bash", &ToolFailureKind::ToolTransientError);

        assert_eq!(c1, 1, "stale McpConnectionLost should reset");
        assert_eq!(c2, 1, "fresh ToolTransientError starts at 1");
    }

    // ── attempt_tool_recovery: simulation-based tests ────────────────────────

    // Helper: build an emit callback backed by a std Mutex<Vec> for tests.
    // No Tauri runtime needed — just a plain Rust closure.
    fn make_emit() -> (
        impl Fn(StreamEvent) + use<>,
        std::sync::Arc<std::sync::Mutex<Vec<StreamEvent>>>,
    ) {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
        let events_clone = events.clone();
        let emit = move |ev: StreamEvent| {
            events_clone.lock().unwrap().push(ev);
        };
        (emit, events)
    }

    // Minimal fake AppState for unit tests — only oauth_refresh_lock and mcp_recovery_gate needed
    async fn make_fake_state() -> AppState {
        let (state, _rx) = AppState::new_for_test_async().await;
        state
    }

    #[tokio::test(start_paused = true)]
    async fn successful_recovery_on_first_attempt_returns_recovered_with_content() {
        let (emit, _events) = make_emit();
        let mut ctx = ToolRecoveryContext::new();
        let state = make_fake_state().await;
        let path = std::path::PathBuf::from("/test");

        let result = attempt_tool_recovery(
            &mut ctx,
            "bash",
            ToolFailureKind::ToolTransientError,
            &state,
            &path,
            &emit,
            || async { Ok("tool output".to_string()) },
        )
        .await;

        assert_eq!(result, ToolRecoveryResult::Recovered { content: "tool output".into() });
    }

    #[tokio::test(start_paused = true)]
    async fn escalation_after_max_attempts_exceeded() {
        let (emit, _events) = make_emit();
        let mut ctx = ToolRecoveryContext::new();
        let state = make_fake_state().await;
        let path = std::path::PathBuf::from("/test");

        let result = attempt_tool_recovery(
            &mut ctx,
            "bash",
            ToolFailureKind::ToolTransientError,
            &state,
            &path,
            &emit,
            || async { Err(AppError::Internal("still failing".into())) },
        )
        .await;

        assert!(matches!(result, ToolRecoveryResult::EscalationRequired { .. }));
    }

    #[tokio::test(start_paused = true)]
    async fn max_attempts_2_fires_retry_fn_exactly_twice() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let (emit, _events) = make_emit();
        let mut ctx = ToolRecoveryContext::new();
        let state = make_fake_state().await;
        let path = std::path::PathBuf::from("/test");
        let call_count = Arc::new(AtomicUsize::new(0));

        let counter = call_count.clone();
        attempt_tool_recovery(
            &mut ctx,
            "bash",
            ToolFailureKind::ToolTransientError,
            &state,
            &path,
            &emit,
            move || {
                let c = counter.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err(AppError::Internal("fail".into()))
                }
            },
        )
        .await;

        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn tool_not_found_escalates_without_calling_retry_fn() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let (emit, _events) = make_emit();
        let mut ctx = ToolRecoveryContext::new();
        let state = make_fake_state().await;
        let path = std::path::PathBuf::from("/test");
        let call_count = Arc::new(AtomicUsize::new(0));

        let counter = call_count.clone();
        let result = attempt_tool_recovery(
            &mut ctx,
            "bash",
            ToolFailureKind::ToolNotFound,
            &state,
            &path,
            &emit,
            move || {
                let c = counter.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok("should not be called".into())
                }
            },
        )
        .await;

        assert_eq!(call_count.load(Ordering::SeqCst), 0, "retry_fn must not be called for ToolNotFound");
        assert_eq!(
            result,
            ToolRecoveryResult::EscalationRequired { reason: "escalated_to_llm".into() }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tool_input_invalid_escalates_without_calling_retry_fn() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let (emit, _events) = make_emit();
        let mut ctx = ToolRecoveryContext::new();
        let state = make_fake_state().await;
        let path = std::path::PathBuf::from("/test");
        let call_count = Arc::new(AtomicUsize::new(0));

        let counter = call_count.clone();
        let result = attempt_tool_recovery(
            &mut ctx,
            "bash",
            ToolFailureKind::ToolInputInvalid,
            &state,
            &path,
            &emit,
            move || {
                let c = counter.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok("nope".into())
                }
            },
        )
        .await;

        assert_eq!(call_count.load(Ordering::SeqCst), 0);
        assert_eq!(
            result,
            ToolRecoveryResult::EscalationRequired { reason: "escalated_to_llm".into() }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn partial_recovery_when_prepare_step_fails_at_step_0() {
        let (emit, _events) = make_emit();
        let mut ctx = ToolRecoveryContext::new();
        ctx.fail_at_step = Some(0);
        let state = make_fake_state().await;
        let path = std::path::PathBuf::from("/test");

        let result = attempt_tool_recovery(
            &mut ctx,
            "bash",
            ToolFailureKind::McpConnectionLost,
            &state,
            &path,
            &emit,
            || async { Ok("should not reach".into()) },
        )
        .await;

        assert!(matches!(
            result,
            ToolRecoveryResult::PartialRecovery { completed, remaining }
            if completed.is_empty() && !remaining.is_empty()
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn recovered_content_matches_retry_fn_output() {
        let (emit, _events) = make_emit();
        let mut ctx = ToolRecoveryContext::new();
        let state = make_fake_state().await;
        let path = std::path::PathBuf::from("/test");

        let result = attempt_tool_recovery(
            &mut ctx,
            "bash",
            ToolFailureKind::ToolTransientError,
            &state,
            &path,
            &emit,
            || async { Ok("exact content here".to_string()) },
        )
        .await;

        assert_eq!(
            result,
            ToolRecoveryResult::Recovered { content: "exact content here".into() }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn escalate_to_llm_step_emits_recovery_result_stream_event() {
        let (emit, events) = make_emit();
        let mut ctx = ToolRecoveryContext::new();
        let state = make_fake_state().await;
        let path = std::path::PathBuf::from("/test");

        attempt_tool_recovery(
            &mut ctx,
            "bash",
            ToolFailureKind::ToolNotFound,
            &state,
            &path,
            &emit,
            || async { Ok("nope".into()) },
        )
        .await;

        let captured = events.lock().unwrap();
        let found = captured.iter().any(|ev| {
            matches!(ev, StreamEvent::RecoveryResult { succeeded: false, outcome, .. } if outcome == "escalated_to_llm")
        });
        assert!(found, "expected RecoveryResult {{ succeeded: false, outcome: 'escalated_to_llm' }}");
    }
}
