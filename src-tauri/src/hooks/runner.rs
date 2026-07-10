use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

// ── Backpressure ──────────────────────────────────────────────────────────

/// Maximum number of async (fire & forget) hooks that can be in flight
/// simultaneously per `HookRunner` instance (= per agent).
///
/// When the limit is reached, new async hooks are **skipped with a log warning**
/// rather than queued. Queueing would cause visual hooks (animations, zone
/// changes) to play minutes late — worse UX than occasionally missing one.
///
/// 16 per agent is generous: typical setups have 1-3 async hooks per event.
/// With 5 agents, the system-wide ceiling is 80 concurrent processes.
const MAX_CONCURRENT_ASYNC_HOOKS: usize = 16;

// ── Output size limits ────────────────────────────────────────────────────

/// Maximum bytes accepted from a hook's stdout after trimming.
///
/// Hook output is JSON — a well-behaved hook should never approach this.
/// The limit prevents a misbehaving or runaway hook from:
///   - exhausting memory (one hook × N agents × large output)
///   - producing a payload that bloats LLM conversation context
///   - slowing down `parse_hook_output` with a huge JSON parse
///
/// Note: `wait_with_output()` still collects everything into memory before
/// truncation. For collection-time limiting, the hooks would need to use
/// piped streams with a `take()` adapter — a future improvement if needed.
const MAX_HOOK_STDOUT_BYTES: usize = 64 * 1024; // 64 KB

/// Maximum bytes accepted from a hook's stderr.
///
/// Diagnostic messages should be short and actionable. A hook producing more
/// than 16 KB of stderr is almost certainly either logging excessively or has
/// a bug. Anything beyond this limit is noise and would pollute the UI.
const MAX_HOOK_STDERR_BYTES: usize = 16 * 1024; // 16 KB

use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};

use super::compiled::{CompiledAgentHookConfig, CompiledHookCommand};
use super::types::{
    AgentContext, GardenEvent, HookDecision, HookEvent, HookOutput, HookPhase, HookRunResult,
    OnTimeoutPolicy, PermissionDecision, PermissionResult, StderrDestination, StderrEntry,
    StderrLevel,
};

// ── Abort signal ──────────────────────────────────────────────────────────

/// Shared abort signal. Call abort() to cancel all in-flight hooks for an agent.
#[derive(Debug, Clone, Default)]
pub struct HookAbortSignal(Arc<AtomicBool>);

impl HookAbortSignal {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn abort(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_aborted(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

// ── Internal types ────────────────────────────────────────────────────────

enum CommandExecution {
    Finished(std::process::Output), // tokio::process reuses std::process::Output
    TimedOut,
    Cancelled,
}

#[derive(Default)]
struct ParsedHookOutput {
    stderr_output: Option<String>,
    deny: bool,
    additional_context: Option<String>,
    system_message: Option<String>,
    updated_input: Option<serde_json::Value>,
    updated_output: Option<String>,
    garden_event: Option<GardenEvent>,
    permission: Option<PermissionResult>,
}

enum HookCommandOutcome {
    Allow { parsed: ParsedHookOutput },
    /// exit 2 or JSON with decision: block
    Deny { parsed: ParsedHookOutput },
    /// exit 1, 3, 4... — does NOT stop the sync chain
    NonBlockingError { stderr: String, command: String },
    /// IO error launching the process — always non-blocking
    SpawnError { message: String },
    Cancelled { message: String },
    /// policy: Fail → mapped to Deny in aggregation
    /// policy: Continue → mapped to NonBlockingError (non-blocking)
    TimedOut { message: String, policy: OnTimeoutPolicy },
}

// ── HookRunner ────────────────────────────────────────────────────────────

pub struct HookRunner {
    config: CompiledAgentHookConfig,
    /// Channel toward the UI for garden events (animations, zone changes).
    event_tx: Option<mpsc::Sender<GardenEvent>>,
    /// Backpressure for async (fire & forget) hooks.
    /// Limits how many can be in flight simultaneously per agent.
    /// Non-blocking: if at capacity, new hooks are skipped (logged).
    async_semaphore: Arc<Semaphore>,
}

impl HookRunner {
    pub fn new(config: CompiledAgentHookConfig) -> Self {
        Self {
            config,
            event_tx: None,
            async_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_ASYNC_HOOKS)),
        }
    }

    pub fn with_event_sender(mut self, tx: mpsc::Sender<GardenEvent>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    // ── Public API ────────────────────────────────────────────────────────

    pub async fn run_pre_tool_use(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
        agent_ctx: &AgentContext,
    ) -> HookRunResult {
        if self.config.pre_tool_use.is_empty() {
            return HookRunResult::default();
        }
        let input_json = serde_json::to_string(input).unwrap_or_default();
        let payload = serialize_tool_payload(
            HookEvent::PreToolUse.as_str(),
            tool_name,
            input,
            None,
            false,
            None,
            agent_ctx,
        );
        let env_vars = build_tool_env_vars(
            HookEvent::PreToolUse.as_str(),
            tool_name,
            &input_json,
            None,
            false,
            agent_ctx,
        );
        self.run_commands(
            &self.config.pre_tool_use,
            HookEvent::PreToolUse,
            &payload,
            tool_name,
            &input_json,
            input,
            &env_vars,
        )
        .await
    }

    pub async fn run_post_tool_use(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
        output: &str,
        agent_ctx: &AgentContext,
    ) -> HookRunResult {
        if self.config.post_tool_use.is_empty() {
            return HookRunResult::default();
        }
        let input_json = serde_json::to_string(input).unwrap_or_default();
        let payload = serialize_tool_payload(
            HookEvent::PostToolUse.as_str(),
            tool_name,
            input,
            Some(output),
            false,
            None,
            agent_ctx,
        );
        let env_vars = build_tool_env_vars(
            HookEvent::PostToolUse.as_str(),
            tool_name,
            &input_json,
            Some(output),
            false,
            agent_ctx,
        );
        self.run_commands(
            &self.config.post_tool_use,
            HookEvent::PostToolUse,
            &payload,
            tool_name,
            &input_json,
            input,
            &env_vars,
        )
        .await
    }

    pub async fn run_post_tool_use_failure(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
        error: &str,
        agent_ctx: &AgentContext,
    ) -> HookRunResult {
        if self.config.post_tool_use_failure.is_empty() {
            return HookRunResult::default();
        }
        let input_json = serde_json::to_string(input).unwrap_or_default();
        let payload = serialize_tool_payload(
            HookEvent::PostToolUseFailure.as_str(),
            tool_name,
            input,
            Some(error),
            true,
            Some(error),
            agent_ctx,
        );
        let env_vars = build_tool_env_vars(
            HookEvent::PostToolUseFailure.as_str(),
            tool_name,
            &input_json,
            Some(error),
            true,
            agent_ctx,
        );
        self.run_commands(
            &self.config.post_tool_use_failure,
            HookEvent::PostToolUseFailure,
            &payload,
            tool_name,
            &input_json,
            input,
            &env_vars,
        )
        .await
    }

    pub async fn run_user_prompt_submit(
        &self,
        prompt: &str,
        agent_ctx: &AgentContext,
    ) -> HookRunResult {
        if self.config.user_prompt_submit.is_empty() {
            return HookRunResult::default();
        }
        let payload = serde_json::to_string(&serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "agent_id": agent_ctx.agent_id,
            "agent_zone": agent_ctx.agent_zone,
            "prompt": prompt,
        }))
        .unwrap_or_default();
        let env_vars = build_lifecycle_env_vars(HookEvent::UserPromptSubmit.as_str(), agent_ctx);
        self.run_commands(
            &self.config.user_prompt_submit,
            HookEvent::UserPromptSubmit,
            &payload,
            "",
            "",
            &serde_json::Value::Null,
            &env_vars,
        )
        .await
    }

    pub async fn run_session_start(&self, agent_ctx: &AgentContext) -> HookRunResult {
        if self.config.session_start.is_empty() {
            return HookRunResult::default();
        }
        let payload = serialize_lifecycle_payload(HookEvent::SessionStart.as_str(), agent_ctx);
        let env_vars = build_lifecycle_env_vars(HookEvent::SessionStart.as_str(), agent_ctx);
        self.run_commands(
            &self.config.session_start,
            HookEvent::SessionStart,
            &payload,
            "",
            "",
            &serde_json::Value::Null,
            &env_vars,
        )
        .await
    }

    /// Stop is always fire & forget — the agent has already finished responding.
    pub fn run_stop(
        &self,
        stop_reason: &str,
        last_message: Option<&str>,
        agent_ctx: &AgentContext,
    ) {
        if self.config.stop.is_empty() {
            return;
        }
        let payload = serialize_stop_payload(stop_reason, last_message, agent_ctx);
        let env_vars = build_stop_env_vars(stop_reason, agent_ctx);
        let commands = self.config.stop.clone();
        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            for cmd in &commands {
                let outcome = run_command(
                    cmd.command_str().to_string(),
                    HookEvent::Stop,
                    payload.clone(),
                    env_vars.clone(),
                    Duration::from_secs(60),
                    OnTimeoutPolicy::Continue,
                    None,
                )
                .await;
                // async_rewake: if exit 2, send garden event to notify the model.
                if cmd.async_rewake() {
                    if let HookCommandOutcome::Deny { parsed } = outcome {
                        if let (Some(ev), Some(tx)) = (parsed.garden_event, &event_tx) {
                            let _ = tx.send(ev).await;
                        }
                    }
                }
            }
        });
    }

    /// SessionEnd is fire & forget — session is already closing.
    pub fn run_session_end(&self, agent_ctx: &AgentContext) {
        if self.config.session_end.is_empty() {
            return;
        }
        let payload = serialize_lifecycle_payload(HookEvent::SessionEnd.as_str(), agent_ctx);
        let env_vars = build_lifecycle_env_vars(HookEvent::SessionEnd.as_str(), agent_ctx);
        let commands = self.config.session_end.clone();
        tokio::spawn(async move {
            for cmd in &commands {
                let _ = run_command(
                    cmd.command_str().to_string(),
                    HookEvent::SessionEnd,
                    payload.clone(),
                    env_vars.clone(),
                    Duration::from_secs(30),
                    OnTimeoutPolicy::Continue,
                    None,
                )
                .await;
            }
        });
    }

    // ── Core dispatch ─────────────────────────────────────────────────────

    async fn run_commands(
        &self,
        commands: &[CompiledHookCommand],
        event: HookEvent,
        payload: &str,
        tool_name: &str,
        tool_input_json: &str,
        original_input: &serde_json::Value,
        env_vars: &HashMap<String, String>,
    ) -> HookRunResult {
        let mut result = HookRunResult::default();

        // Filter hooks that apply to this tool/input combination.
        let matching: Vec<_> = commands
            .iter()
            .filter(|cmd| cmd.matches_tool(tool_name, tool_input_json))
            .collect();

        if matching.is_empty() {
            return result;
        }

        // Separate into three tracks: async | sync Main | sync Enforce.
        let mut async_cmds = vec![];
        let mut main_sync_cmds = vec![];
        let mut enforce_cmds = vec![];

        for cmd in matching {
            if cmd.is_async() {
                async_cmds.push(cmd);
            } else if cmd.phase() == &HookPhase::Enforce {
                enforce_cmds.push(cmd);
            } else {
                main_sync_cmds.push(cmd);
            }
        }

        // ── TRACK 1: ASYNC — fire & forget ───────────────────────────────
        // Dispatched immediately; never block the tool.
        // Backpressure: each hook must acquire a semaphore permit before spawning.
        // try_acquire_owned() is non-blocking — if at capacity the hook is skipped
        // and logged. For fire & forget visual hooks, skipping beats late playback.
        for cmd in async_cmds {
            let permit: OwnedSemaphorePermit =
                match Arc::clone(&self.async_semaphore).try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        log::warn!(
                            "Async hook '{}' skipped: {} concurrent async hooks already \
                             in flight for this agent",
                            cmd.command_str(),
                            MAX_CONCURRENT_ASYNC_HOOKS,
                        );
                        continue;
                    }
                };

            let payload_owned = payload.to_string();
            let env_vars_owned = env_vars.clone();
            let cmd_owned = cmd.clone();
            let event_tx = self.event_tx.clone();
            tokio::spawn(async move {
                // Permit is held for the lifetime of the task.
                // Dropping it at the end releases the slot for the next hook.
                let _permit = permit;

                let outcome = run_command(
                    cmd_owned.command_str().to_string(),
                    event,
                    payload_owned,
                    env_vars_owned,
                    Duration::from_secs(60),
                    OnTimeoutPolicy::Continue,
                    None,
                )
                .await;
                if cmd_owned.async_rewake() {
                    if let HookCommandOutcome::Deny { parsed } = outcome {
                        if let (Some(ev), Some(tx)) = (parsed.garden_event, event_tx) {
                            let _ = tx.send(ev).await;
                        }
                    }
                }
            });
        }

        // ── TRACK 2: SYNC MAIN — parallel ────────────────────────────────
        // Responsibility: formatters, enrichers, zone tagging.
        // Each hook sees the original input; mutations are merged last-write-wins.
        // Convention: Main hooks MUST NOT overlap fields — use Enforce for composition.
        let timeout = self.config.timeout_for_event(event);

        let main_futures: Vec<_> = main_sync_cmds
            .iter()
            .map(|cmd| {
                let cmd_timeout = cmd
                    .timeout_secs()
                    .map(Duration::from_secs)
                    .unwrap_or(timeout);
                let on_timeout = cmd.on_timeout.clone();
                run_command(
                    cmd.command_str().to_string(),
                    event,
                    payload.to_string(),
                    env_vars.clone(),
                    cmd_timeout,
                    on_timeout,
                    None,
                )
            })
            .collect();

        let main_outcomes = futures::future::join_all(main_futures).await;

        let mut effective_input = original_input.clone();
        let mut any_deny = false;

        for (cmd, outcome) in main_sync_cmds.iter().zip(main_outcomes.iter()) {
            aggregate_outcome(
                &mut result,
                &mut effective_input,
                &mut any_deny,
                cmd,
                outcome,
                event,
            );
        }

        // If any Main hook denied, no point running Enforce.
        if any_deny {
            result.denied = true;
            return result;
        }

        // ── TRACK 3: SYNC ENFORCE — sequential ───────────────────────────
        // Responsibility: security guards, invariant checks, sanitization.
        // Each hook receives the accumulated effective_input in its payload,
        // guaranteeing it sees mutations from all previous hooks (Main + prior Enforce).
        for cmd in &enforce_cmds {
            // Rebuild payload with the effective_input accumulated so far.
            // The enforce hook must see the final state of the input to operate correctly.
            let enforce_payload = rebuild_payload_with_input(payload, &effective_input);
            let cmd_timeout = cmd
                .timeout_secs()
                .map(Duration::from_secs)
                .unwrap_or(timeout);
            let on_timeout_policy = cmd.on_timeout.clone();

            let outcome = run_command(
                cmd.command_str().to_string(),
                event,
                enforce_payload,
                env_vars.clone(),
                cmd_timeout,
                on_timeout_policy,
                None,
            )
            .await;

            match &outcome {
                HookCommandOutcome::Deny { parsed } => {
                    // Enforce deny is immediate and final — do not continue with more Enforce hooks.
                    if let Some(stderr) = &parsed.stderr_output {
                        result.stderr_lines.push(StderrEntry {
                            command: cmd.command_str().to_string(),
                            event,
                            content: stderr.clone(),
                            level: StderrLevel::Error,
                            destination: StderrDestination::Model,
                        });
                    }
                    collect_contexts(&mut result, parsed);
                    result.denied = true;
                    return result;
                }
                HookCommandOutcome::Allow { parsed } => {
                    // Apply mutation IMMEDIATELY — the next Enforce hook will see it.
                    if let Some(upd) = &parsed.updated_input {
                        effective_input = merge_input(&effective_input, upd);
                    }
                    aggregate_allow_channels(&mut result, cmd, parsed);
                }
                HookCommandOutcome::TimedOut { policy, message } => match policy {
                    OnTimeoutPolicy::Fail => {
                        result.timed_out = true;
                        result.denied = true;
                        log::error!(
                            "Enforce hook timed out — blocking: hook={}, {}",
                            cmd.command_str(),
                            message
                        );
                        return result;
                    }
                    OnTimeoutPolicy::Continue => {
                        log::warn!(
                            "Enforce hook timed out — continuing: hook={}, {}",
                            cmd.command_str(),
                            message
                        );
                    }
                },
                HookCommandOutcome::NonBlockingError { stderr, .. } => {
                    result.stderr_lines.push(StderrEntry {
                        command: cmd.command_str().to_string(),
                        event,
                        content: stderr.clone(),
                        level: StderrLevel::Warning,
                        destination: StderrDestination::User,
                    });
                }
                HookCommandOutcome::Cancelled { .. } => {
                    result.cancelled = true;
                    return result;
                }
                HookCommandOutcome::SpawnError { message } => {
                    log::error!(
                        "Enforce hook spawn failed: hook={}, {}",
                        cmd.command_str(),
                        message
                    );
                }
            }
        }

        // Promote effective_input to result only if it diverged from original.
        if effective_input != *original_input {
            result.updated_input = Some(effective_input);
        }

        result
    }
}

// ── Core execution ────────────────────────────────────────────────────────

/// Fully async hook execution — no blocking threads, no polling sleep.
///
/// Uses `tokio::process::Command` so the OS notifies the runtime when the
/// process exits (epoll/kqueue). `tokio::select!` races completion against
/// the timeout and the abort signal. No thread is consumed while waiting.
async fn run_command(
    command: String,
    _event: HookEvent,
    payload: String,
    env_vars: HashMap<String, String>,
    timeout: Duration,
    on_timeout: OnTimeoutPolicy,
    abort_signal: Option<Arc<AtomicBool>>,
) -> HookCommandOutcome {
    let exec = execute_hook_async(&command, &payload, &env_vars, timeout, abort_signal).await;

    match exec {
        Err(e) => HookCommandOutcome::SpawnError {
            message: format!("spawn failed for `{}`: {}", command, e),
        },
        Ok(CommandExecution::Cancelled) => HookCommandOutcome::Cancelled {
            message: "Hook cancelled by abort signal".into(),
        },
        Ok(CommandExecution::TimedOut) => HookCommandOutcome::TimedOut {
            message: format!("Hook `{}` timed out after {}s", command, timeout.as_secs()),
            policy: on_timeout,
        },
        Ok(CommandExecution::Finished(output)) => {
            let stdout = truncate_to_bytes(
                String::from_utf8_lossy(&output.stdout).trim().to_string(),
                MAX_HOOK_STDOUT_BYTES,
            );
            let stderr_raw = truncate_to_bytes(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
                MAX_HOOK_STDERR_BYTES,
            );

            let mut parsed = parse_hook_output(&stdout);
            if !stderr_raw.is_empty() {
                parsed.stderr_output = Some(stderr_raw.clone());
            }

            match output.status.code() {
                Some(0) => {
                    // JSON has priority over exit code:
                    // exit 0 with {"decision": "block"} is still a Deny.
                    if parsed.deny {
                        HookCommandOutcome::Deny { parsed }
                    } else {
                        HookCommandOutcome::Allow { parsed }
                    }
                }
                Some(2) => {
                    parsed.deny = true;
                    HookCommandOutcome::Deny { parsed }
                }
                // exit 1, 3, 4... → NonBlockingError. Never stops the chain.
                _ => HookCommandOutcome::NonBlockingError {
                    stderr: stderr_raw,
                    command,
                },
            }
        }
    }
}

/// Launches the hook process and races it against the timeout and the abort
/// signal. No thread is blocked — all waits are async.
///
/// ## Process group isolation (Unix)
///
/// On Unix the child process is placed in its own process group
/// (`process_group(0)` → PGID = child PID). When the hook is cancelled or
/// times out, the entire process group is killed with `kill(-PGID, SIGKILL)`,
/// not just the shell process. This prevents orphaned grandchildren:
///
/// ```text
/// shell [PID/PGID 1234]     ← kill_on_drop kills this
///   └── python script.py     ← previously orphaned; now killed by group kill
///         └── subprocess     ← previously orphaned; now killed by group kill
/// ```
///
/// ## Design: two-layer cancellation
///
/// `wait_with_output()` takes ownership of `child`, so we cannot call
/// `child.kill()` from a sibling branch in the same `select!`. Solution:
///
///   outer: `tokio::time::timeout` enforces the deadline
///   inner: `wait_or_abort` — `select!{ abort | wait_with_output }`
///
/// The child's PID (= PGID on Unix) is saved BEFORE moving child into the
/// inner future so the timeout path can still issue the group kill.
///
/// ## Windows
///
/// Windows does not have Unix process groups. The equivalent (Job Objects)
/// is not yet implemented. On Windows, only the shell process is killed.
async fn execute_hook_async(
    command: &str,
    payload: &str,
    env_vars: &HashMap<String, String>,
    timeout: Duration,
    abort_signal: Option<Arc<AtomicBool>>,
) -> std::io::Result<CommandExecution> {
    use tokio::io::AsyncWriteExt as _;

    let mut cmd = spawn_hook_process(command);
    cmd.envs(env_vars);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    // Auto-kill the shell process when its handle is dropped (abort / timeout).
    // On Unix this is the safety net; the real kill is the process group kill below.
    cmd.kill_on_drop(true);
    // Unix: place the child in its own process group (PGID = child PID).
    // This is what enables the group kill — all spawned grandchildren share this PGID.
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn()?;

    // Save the PID/PGID BEFORE consuming `child` into `wait_with_output()`.
    // On Unix with process_group(0), PGID == child's PID.
    #[cfg(unix)]
    let pgid = child.id();

    // Write payload to stdin, then close it so the process sees EOF.
    // Errors are intentionally ignored — hooks may choose not to read stdin.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes()).await;
    }

    // Outer layer: deadline enforcement.
    // When timeout fires, `wait_or_abort` is dropped → child dropped → kill_on_drop fires.
    // We also kill the process group so grandchildren don't escape.
    match tokio::time::timeout(timeout, wait_or_abort(child, abort_signal)).await {
        Ok(Ok(output)) => Ok(CommandExecution::Finished(output)),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::Interrupted => {
            Ok(CommandExecution::Cancelled)
        }
        Ok(Err(e)) => Err(e),
        Err(_elapsed) => {
            // kill_on_drop already killed the shell leader.
            // Kill the whole group to reap any orphaned grandchildren.
            #[cfg(unix)]
            if let Some(pid) = pgid {
                kill_process_group(pid);
            }
            Ok(CommandExecution::TimedOut)
        }
    }
}

/// Inner layer: races process completion against the abort signal.
///
/// Saves the PGID before consuming `child` into `wait_with_output()`.
/// When the abort branch fires, kills the process group before dropping `child`.
async fn wait_or_abort(
    child: tokio::process::Child,
    abort_signal: Option<Arc<AtomicBool>>,
) -> std::io::Result<std::process::Output> {
    // Save PGID before child is consumed by wait_with_output().
    #[cfg(unix)]
    let pgid = child.id();

    tokio::select! {
        biased;
        _ = watch_abort_signal(abort_signal) => {
            // Kill the process group before dropping child.
            // kill_on_drop will also fire (redundant but harmless — ESRCH if already dead).
            #[cfg(unix)]
            if let Some(pid) = pgid {
                kill_process_group(pid);
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "hook aborted by external signal",
            ))
        }
        result = child.wait_with_output() => {
            result
        }
    }
}

/// Sends SIGKILL to all processes in the group identified by `pgid`.
///
/// Uses `kill(-pgid, SIGKILL)` — the negative PID targets the process GROUP,
/// not a single process. Processes that already exited are silently ignored
/// by the kernel (ESRCH). The return value is discarded intentionally.
#[cfg(unix)]
fn kill_process_group(pgid: u32) {
    // SAFETY: kill() is a standard POSIX syscall. Sending SIGKILL to a
    // process group is always safe — the kernel ignores already-dead PIDs.
    let _ = unsafe { libc::kill(-(pgid as libc::pid_t), libc::SIGKILL) };
}

/// Resolves when the abort signal is set.
/// If no signal is provided, stays pending forever (select! ignores this branch).
/// Polls every 50 ms using tokio::time::sleep — async, no thread consumed.
async fn watch_abort_signal(signal: Option<Arc<AtomicBool>>) {
    match signal {
        Some(sig) => loop {
            if sig.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        },
        None => std::future::pending().await,
    }
}

/// Returns a tokio-compatible shell command wrapping the hook command string.
#[cfg(windows)]
fn spawn_hook_process(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.args(["/C", command]);
    cmd
}

#[cfg(not(windows))]
fn spawn_hook_process(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.args(["-c", command]);
    cmd
}

// ── Output size limiting ──────────────────────────────────────────────────

/// Truncates `s` to at most `limit` bytes, preserving UTF-8 validity.
///
/// Walks backwards from `limit` to find the last valid char boundary, then
/// appends a truncation marker so the operator knows output was cut.
/// Returns `s` unchanged if it is already within the limit.
fn truncate_to_bytes(s: String, limit: usize) -> String {
    if s.len() <= limit {
        return s;
    }
    let end = (0..=limit)
        .rev()
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(0);
    format!(
        "{}\n[output truncated: {} of {} bytes shown]",
        &s[..end],
        end,
        s.len()
    )
}

// ── Stdout parsing ────────────────────────────────────────────────────────

fn parse_hook_output(stdout: &str) -> ParsedHookOutput {
    let mut parsed = ParsedHookOutput::default();
    if stdout.is_empty() {
        return parsed;
    }
    let Ok(output) = serde_json::from_str::<HookOutput>(stdout) else {
        return parsed;
    };

    // JSON "continue": false or "decision": "block" → deny.
    parsed.deny = output.should_continue == Some(false)
        || matches!(output.decision, Some(HookDecision::Block));

    parsed.additional_context = output.additional_context.clone();
    parsed.system_message = output.system_message.clone();
    parsed.garden_event = output.garden_event.clone();

    if let Some(ref specific) = output.hook_specific_output {
        parsed.updated_input = specific.updated_input.clone();
        parsed.updated_output = specific.updated_output.clone();

        if let Some(perm) = specific.permission_result() {
            if matches!(perm.decision, PermissionDecision::Deny) {
                parsed.deny = true;
            }
            parsed.permission = Some(perm);
        }
    }

    // If denied but no PermissionResult yet (e.g. from `{"decision":"block","reason":"..."}`)
    // build one so the reason flows through to HookRunResult.
    if parsed.deny && parsed.permission.is_none() {
        parsed.permission = Some(PermissionResult {
            decision: PermissionDecision::Deny,
            reason: output.reason.clone(),
        });
    } else if parsed.deny {
        // Already have a PermissionResult but reason may be absent — fill it from HookOutput.reason.
        if let Some(ref mut p) = parsed.permission {
            if p.reason.is_none() {
                p.reason = output.reason.clone();
            }
        }
    }

    parsed
}

// ── Aggregation ───────────────────────────────────────────────────────────

fn aggregate_outcome(
    result: &mut HookRunResult,
    effective_input: &mut serde_json::Value,
    any_deny: &mut bool,
    cmd: &CompiledHookCommand,
    outcome: &HookCommandOutcome,
    event: HookEvent,
) {
    match outcome {
        HookCommandOutcome::Deny { parsed } => {
            *any_deny = true;
            if let Some(stderr) = &parsed.stderr_output {
                result.stderr_lines.push(StderrEntry {
                    command: cmd.command_str().to_string(),
                    event,
                    content: stderr.clone(),
                    level: StderrLevel::Error,
                    destination: StderrDestination::Model,
                });
            }
            collect_contexts(result, parsed);
        }
        HookCommandOutcome::Allow { parsed } => {
            if let Some(stderr) = &parsed.stderr_output {
                result.stderr_lines.push(StderrEntry {
                    command: cmd.command_str().to_string(),
                    event,
                    content: stderr.clone(),
                    level: StderrLevel::Warning,
                    destination: StderrDestination::User,
                });
            }
            if let Some(upd) = &parsed.updated_input {
                *effective_input = merge_input(effective_input, upd);
            }
            if let Some(upd_out) = &parsed.updated_output {
                result.updated_output = Some(upd_out.clone());
            }
            aggregate_allow_channels(result, cmd, parsed);
        }
        HookCommandOutcome::NonBlockingError { stderr, .. } => {
            result.stderr_lines.push(StderrEntry {
                command: cmd.command_str().to_string(),
                event,
                content: stderr.clone(),
                level: StderrLevel::Warning,
                destination: StderrDestination::User,
            });
        }
        HookCommandOutcome::TimedOut { policy, message } => match policy {
            OnTimeoutPolicy::Fail => {
                *any_deny = true;
                result.timed_out = true;
                log::warn!(
                    "Main hook timed out — blocking: hook={}, {}",
                    cmd.command_str(),
                    message
                );
            }
            OnTimeoutPolicy::Continue => {
                log::warn!(
                    "Main hook timed out — continuing: hook={}, {}",
                    cmd.command_str(),
                    message
                );
            }
        },
        HookCommandOutcome::Cancelled { .. } => {
            result.cancelled = true;
        }
        HookCommandOutcome::SpawnError { message } => {
            log::error!(
                "Main hook spawn failed: hook={}, {}",
                cmd.command_str(),
                message
            );
        }
    }
}

/// Accumulates output channels from an Allow outcome (contexts, messages, garden events, permissions).
fn aggregate_allow_channels(
    result: &mut HookRunResult,
    _cmd: &CompiledHookCommand,
    parsed: &ParsedHookOutput,
) {
    if let Some(ctx) = &parsed.additional_context {
        result.additional_contexts.push(ctx.clone());
    }
    if let Some(msg) = &parsed.system_message {
        result.system_messages.push(msg.clone());
    }
    if let Some(ev) = &parsed.garden_event {
        result.garden_events.push(ev.clone());
    }
    match parsed.permission.as_ref().map(|p| &p.decision) {
        Some(PermissionDecision::Ask) => {
            if !result.denied && result.permission.is_none() {
                // First Ask wins — preserve its reason.
                result.permission = parsed.permission.clone();
            }
        }
        Some(PermissionDecision::Allow) => {
            if result.permission.is_none() {
                result.permission = parsed.permission.clone();
            }
        }
        Some(PermissionDecision::Deny) => {
            // Defensive: should have been caught as Deny outcome.
            result.denied = true;
        }
        None => {}
    }
}

/// Collects context channels from a Deny outcome — model and user must receive
/// the information even when the action is blocked.
fn collect_contexts(result: &mut HookRunResult, parsed: &ParsedHookOutput) {
    if let Some(ctx) = &parsed.additional_context {
        result.additional_contexts.push(ctx.clone());
    }
    if let Some(msg) = &parsed.system_message {
        result.system_messages.push(msg.clone());
    }
    if let Some(ev) = &parsed.garden_event {
        result.garden_events.push(ev.clone());
    }
}

// ── Input merge ───────────────────────────────────────────────────────────

/// Shallow partial merge: top-level keys in `update` overwrite the
/// corresponding keys in `base`. Top-level keys absent from `update` are
/// preserved unchanged. Nested objects are replaced in full, NOT merged
/// recursively.
///
/// ## Why shallow
///
/// MCP tool inputs are overwhelmingly flat objects:
/// `{"command": "git push"}`, `{"file_path": "/src/main.ts"}`, etc.
/// Deep merge would add significant complexity (array handling, null semantics,
/// cycle detection) with no practical benefit for the real hook use cases.
///
/// ## Nested-object behavior (intentional)
///
/// ```text
/// base:   {"cmd": "ls", "opts": {"force": false, "tags": true}}
/// update: {"opts": {"force": true}}
/// result: {"cmd": "ls", "opts": {"force": true}}
///                         ^^^^^^^^^^^^^^^^^^^
///                         "tags" is gone — opts was replaced, not merged
/// ```
///
/// If a hook needs to mutate a nested key without losing siblings, it must
/// return the full nested object in `updated_input`. The hook receives the
/// complete `tool_input` in its payload and can read + modify it accordingly.
///
/// If `update` is not a JSON object, it fully replaces `base` (total replace).
pub fn merge_input(
    base: &serde_json::Value,
    update: &serde_json::Value,
) -> serde_json::Value {
    use serde_json::Value;
    match (base, update) {
        (Value::Object(orig), Value::Object(upd)) => {
            let mut merged = orig.clone();
            for (k, v) in upd {
                merged.insert(k.clone(), v.clone());
            }
            Value::Object(merged)
        }
        // Non-object update → total replace (no partial merge possible).
        _ => update.clone(),
    }
}

/// Rebuilds the payload JSON replacing tool_input (and tool_input_json) with
/// the given effective_input. Used to feed each Enforce hook the accumulated state.
fn rebuild_payload_with_input(payload: &str, effective_input: &serde_json::Value) -> String {
    if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(payload) {
        if let Some(obj) = v.as_object_mut() {
            obj.insert("tool_input".to_string(), effective_input.clone());
            if let Ok(json_str) = serde_json::to_string(effective_input) {
                obj.insert(
                    "tool_input_json".to_string(),
                    serde_json::Value::String(json_str),
                );
            }
        }
        serde_json::to_string(&v).unwrap_or_else(|_| payload.to_string())
    } else {
        payload.to_string()
    }
}

// ── Payload serialization ─────────────────────────────────────────────────

fn serialize_tool_payload(
    event_name: &str,
    tool_name: &str,
    input: &serde_json::Value,
    tool_output: Option<&str>,
    tool_result_is_error: bool,
    tool_error: Option<&str>,
    agent_ctx: &AgentContext,
) -> String {
    let input_json = serde_json::to_string(input).unwrap_or_default();
    serde_json::to_string(&serde_json::json!({
        "hook_event_name": event_name,
        "agent_id": agent_ctx.agent_id,
        "agent_zone": agent_ctx.agent_zone,
        "tool_name": tool_name,
        "tool_input": input,
        "tool_input_json": input_json,
        "tool_output": tool_output,
        "tool_result_is_error": tool_result_is_error,
        "tool_error": tool_error,
    }))
    .unwrap_or_default()
}

fn serialize_stop_payload(
    stop_reason: &str,
    last_message: Option<&str>,
    agent_ctx: &AgentContext,
) -> String {
    serde_json::to_string(&serde_json::json!({
        "hook_event_name": "Stop",
        "agent_id": agent_ctx.agent_id,
        "agent_zone": agent_ctx.agent_zone,
        "stop_reason": stop_reason,
        "last_assistant_message": last_message,
    }))
    .unwrap_or_default()
}

fn serialize_lifecycle_payload(event_name: &str, agent_ctx: &AgentContext) -> String {
    serde_json::to_string(&serde_json::json!({
        "hook_event_name": event_name,
        "agent_id": agent_ctx.agent_id,
        "agent_zone": agent_ctx.agent_zone,
    }))
    .unwrap_or_default()
}

// ── Environment variables ─────────────────────────────────────────────────

fn build_tool_env_vars(
    event_name: &str,
    tool_name: &str,
    input_json: &str,
    tool_output: Option<&str>,
    is_error: bool,
    agent_ctx: &AgentContext,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert("HOOK_EVENT".to_string(), event_name.to_string());
    map.insert("HOOK_TOOL_NAME".to_string(), tool_name.to_string());
    map.insert("HOOK_TOOL_INPUT".to_string(), input_json.to_string());
    map.insert(
        "HOOK_TOOL_OUTPUT".to_string(),
        tool_output.unwrap_or("").to_string(),
    );
    map.insert(
        "HOOK_TOOL_IS_ERROR".to_string(),
        if is_error { "1" } else { "0" }.to_string(),
    );
    map.insert("HOOK_AGENT_ID".to_string(), agent_ctx.agent_id.clone());
    if let Some(zone) = &agent_ctx.agent_zone {
        map.insert("HOOK_AGENT_ZONE".to_string(), zone.clone());
    }
    map
}

fn build_stop_env_vars(stop_reason: &str, agent_ctx: &AgentContext) -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert("HOOK_EVENT".to_string(), "Stop".to_string());
    map.insert("HOOK_STOP_REASON".to_string(), stop_reason.to_string());
    map.insert("HOOK_AGENT_ID".to_string(), agent_ctx.agent_id.clone());
    if let Some(zone) = &agent_ctx.agent_zone {
        map.insert("HOOK_AGENT_ZONE".to_string(), zone.clone());
    }
    map
}

fn build_lifecycle_env_vars(
    event_name: &str,
    agent_ctx: &AgentContext,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert("HOOK_EVENT".to_string(), event_name.to_string());
    map.insert("HOOK_AGENT_ID".to_string(), agent_ctx.agent_id.clone());
    if let Some(zone) = &agent_ctx.agent_zone {
        map.insert("HOOK_AGENT_ZONE".to_string(), zone.clone());
    }
    map
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::compiled::{CompiledAgentHookConfig, CompiledHookCommand};
    use crate::hooks::types::{
        AgentContext, HookPhase, OnTimeoutPolicy, PermissionDecision, PermissionResult,
        StderrDestination, StderrLevel,
    };
    use serde_json::json;

    // ── Test helpers ──────────────────────────────────────────────────────

    fn ctx() -> AgentContext {
        AgentContext { agent_id: "test".to_string(), agent_zone: None }
    }

    fn simple_hook(cmd: &str) -> CompiledHookCommand {
        CompiledHookCommand {
            command: cmd.to_string(),
            tools: None,
            compiled_matcher: None,
            compiled_if: None,
            r#async: false,
            async_rewake: false,
            phase: HookPhase::Main,
            timeout_secs: None,
            on_timeout: OnTimeoutPolicy::Fail,
            status_message: None,
        }
    }

    fn enforce_hook(cmd: &str) -> CompiledHookCommand {
        CompiledHookCommand { phase: HookPhase::Enforce, ..simple_hook(cmd) }
    }

    fn async_hook(cmd: &str) -> CompiledHookCommand {
        CompiledHookCommand { r#async: true, ..simple_hook(cmd) }
    }

    fn runner_pre(hooks: Vec<CompiledHookCommand>) -> HookRunner {
        HookRunner::new(CompiledAgentHookConfig {
            pre_tool_use: hooks,
            post_tool_use: vec![],
            post_tool_use_failure: vec![],
            stop: vec![],
            user_prompt_submit: vec![],
            session_start: vec![],
            session_end: vec![],
            subagent_start: vec![],
            subagent_stop: vec![],
            pre_tool_use_timeout: Duration::from_secs(5),
            post_tool_use_timeout: Duration::from_secs(5),
            post_tool_use_failure_timeout: Duration::from_secs(5),
        })
    }

    fn runner_post(hooks: Vec<CompiledHookCommand>) -> HookRunner {
        HookRunner::new(CompiledAgentHookConfig {
            pre_tool_use: vec![],
            post_tool_use: hooks,
            post_tool_use_failure: vec![],
            stop: vec![],
            user_prompt_submit: vec![],
            session_start: vec![],
            session_end: vec![],
            subagent_start: vec![],
            subagent_stop: vec![],
            pre_tool_use_timeout: Duration::from_secs(5),
            post_tool_use_timeout: Duration::from_secs(5),
            post_tool_use_failure_timeout: Duration::from_secs(5),
        })
    }

    // On Unix: outputs JSON to stdout and exits with the given code.
    // The JSON is written via printf to avoid trailing newlines altering parse.
    #[cfg(unix)]
    fn sh_json_exit(json_str: &str, exit_code: i32) -> String {
        let escaped = json_str.replace('\'', r"'\''");
        format!("printf '%s' '{}'; exit {}", escaped, exit_code)
    }

    #[cfg(unix)]
    fn sh_stderr_exit(stderr_msg: &str, exit_code: i32) -> String {
        let escaped = stderr_msg.replace('\'', r"'\''");
        format!("printf '%s' '{}' >&2; exit {}", escaped, exit_code)
    }

    #[cfg(unix)]
    fn sh_json_stderr_exit(json_str: &str, stderr_msg: &str, exit_code: i32) -> String {
        format!(
            "{}; {}",
            sh_json_exit(json_str, 0).trim_end_matches("; exit 0"),
            sh_stderr_exit(stderr_msg, exit_code)
        )
    }

    // ── truncate_to_bytes ─────────────────────────────────────────────────

    #[test]
    fn truncate_within_limit_unchanged() {
        let s = "hello world".to_string();
        assert_eq!(truncate_to_bytes(s.clone(), 100), s);
    }

    #[test]
    fn truncate_over_limit_appends_marker() {
        let s = "abcdefghij".to_string(); // 10 bytes
        let result = truncate_to_bytes(s, 5);
        assert!(result.starts_with("abcde"));
        assert!(result.contains("truncated"));
        assert!(result.contains("10")); // total bytes in marker
    }

    #[test]
    fn truncate_preserves_utf8_validity() {
        // "é" is 2 bytes (0xc3 0xa9). A naive byte-slice at offset 1 would
        // produce invalid UTF-8. truncate_to_bytes must back up to a char boundary.
        let s = "aé".to_string(); // 3 bytes: 'a'(1) + 'é'(2)
        let result = truncate_to_bytes(s, 2); // limit falls inside the 2-byte 'é'
        // The result is valid UTF-8 (Rust &str guarantees this)
        assert!(result.starts_with('a'));
        assert!(result.contains("truncated"));
    }

    // ── merge_input — pure unit tests ─────────────────────────────────────

    #[test]
    fn merge_adds_new_key() {
        let base = json!({"a": 1});
        let update = json!({"b": 2});
        let result = merge_input(&base, &update);
        assert_eq!(result, json!({"a": 1, "b": 2}));
    }

    #[test]
    fn merge_overwrites_existing_key() {
        let base = json!({"a": 1, "b": 99});
        let update = json!({"b": 2});
        let result = merge_input(&base, &update);
        assert_eq!(result, json!({"a": 1, "b": 2}));
    }

    #[test]
    fn merge_preserves_untouched_keys() {
        let base = json!({"command": "original", "cwd": "/home"});
        let update = json!({"cwd": "/new"});
        let result = merge_input(&base, &update);
        assert_eq!(result["command"], "original");
        assert_eq!(result["cwd"], "/new");
    }

    #[test]
    fn merge_non_object_replaces_totally() {
        let base = json!({"a": 1});
        let update = json!("string");
        let result = merge_input(&base, &update);
        assert_eq!(result, json!("string"));
    }

    /// Spec test: documents the intentional shallow (non-recursive) behavior.
    /// A hook that wants to mutate a nested key must return the full nested object.
    #[test]
    fn merge_shallow_replaces_nested_object_entirely() {
        let base = json!({
            "command": "git push",
            "options": {"force": false, "tags": true}
        });
        let update = json!({
            "options": {"force": true}
            // "tags" is absent — the whole "options" object is replaced
        });
        let result = merge_input(&base, &update);
        assert_eq!(result["command"], "git push");       // preserved
        assert_eq!(result["options"]["force"], true);    // updated
        assert!(result["options"]["tags"].is_null());    // GONE — this is intentional
    }

    // ── parse_hook_output — pure unit tests ───────────────────────────────

    #[test]
    fn parse_empty_stdout_allows() {
        let out = parse_hook_output("");
        assert!(!out.deny);
        assert!(out.garden_event.is_none());
        assert!(out.additional_context.is_none());
        assert!(out.system_message.is_none());
    }

    #[test]
    fn parse_continue_false_denies() {
        let out = parse_hook_output(r#"{"continue": false}"#);
        assert!(out.deny);
    }

    #[test]
    fn parse_decision_block_denies() {
        let out = parse_hook_output(r#"{"decision": "block"}"#);
        assert!(out.deny);
    }

    #[test]
    fn parse_decision_approve_allows() {
        let out = parse_hook_output(r#"{"decision": "approve"}"#);
        assert!(!out.deny);
    }

    #[test]
    fn parse_permission_deny_denies() {
        let out = parse_hook_output(
            r#"{"hook_specific_output": {"hook_event_name": "PreToolUse", "permission_decision": "deny"}}"#,
        );
        assert!(out.deny);
        assert!(matches!(
            out.permission,
            Some(PermissionResult { decision: PermissionDecision::Deny, .. })
        ));
    }

    #[test]
    fn parse_permission_ask_sets_result() {
        let out = parse_hook_output(
            r#"{"hook_specific_output": {"hook_event_name": "PreToolUse", "permission_decision": "ask", "permission_decision_reason": "Need approval"}}"#,
        );
        assert!(!out.deny);
        let perm = out.permission.unwrap();
        assert!(matches!(perm.decision, PermissionDecision::Ask));
        assert_eq!(perm.reason.as_deref(), Some("Need approval"));
    }

    #[test]
    fn parse_garden_event() {
        let out = parse_hook_output(
            r#"{"garden_event": {"move_to_zone": "desk-zone", "animation": "typing"}}"#,
        );
        let ev = out.garden_event.unwrap();
        assert_eq!(ev.move_to_zone.as_deref(), Some("desk-zone"));
        assert_eq!(ev.animation.as_deref(), Some("typing"));
    }

    #[test]
    fn parse_system_message() {
        let out = parse_hook_output(r#"{"system_message": "Warning: something happened"}"#);
        assert_eq!(out.system_message.as_deref(), Some("Warning: something happened"));
        assert!(out.additional_context.is_none());
    }

    #[test]
    fn parse_additional_context() {
        let out = parse_hook_output(r#"{"additional_context": "Context for LLM"}"#);
        assert_eq!(out.additional_context.as_deref(), Some("Context for LLM"));
        assert!(out.system_message.is_none());
    }

    #[test]
    fn parse_updated_input() {
        let out = parse_hook_output(
            r#"{"hook_specific_output": {"hook_event_name": "PreToolUse", "updated_input": {"command": "ls -la"}}}"#,
        );
        assert_eq!(out.updated_input, Some(json!({"command": "ls -la"})));
    }

    #[test]
    fn parse_updated_output() {
        let out = parse_hook_output(
            r#"{"hook_specific_output": {"hook_event_name": "PostToolUse", "updated_output": "redacted"}}"#,
        );
        assert_eq!(out.updated_output.as_deref(), Some("redacted"));
    }

    // ── HookRunResult methods ─────────────────────────────────────────────

    #[test]
    fn hook_run_result_ask_requires_confirmation() {
        let mut r = HookRunResult::default();
        r.permission = Some(PermissionResult {
            decision: PermissionDecision::Ask,
            reason: Some("Please confirm".to_string()),
        });
        assert!(r.requires_confirmation());
        assert!(!r.is_denied());
        assert_eq!(r.reason(), Some("Please confirm"));
    }

    #[test]
    fn hook_run_result_deny_overrides_ask() {
        let mut r = HookRunResult::default();
        r.denied = true;
        r.permission = Some(PermissionResult {
            decision: PermissionDecision::Ask,
            reason: None,
        }); // should not matter
        assert!(r.is_denied());
        // requires_confirmation only matters if not denied
        assert!(r.requires_confirmation()); // Ask is still set, but the agent checks is_denied first
    }

    // ── Exit code outcomes — process tests ───────────────────────────────
    //
    // On Unix  : `spawn_hook_process` wraps commands with `sh -c`.
    // On Windows: `spawn_hook_process` wraps commands with `cmd /C`.
    // Both run the same Rust code path — only the shell differs.
    // Unix tests use `exit N` (sh syntax).
    // Windows tests use `exit N` (cmd.exe syntax — same, but no stderr redirect).

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn exit0_no_stderr_allows() {
        let runner = runner_pre(vec![simple_hook("exit 0")]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(!result.denied);
        assert!(result.stderr_lines.is_empty());
    }

    // Windows equivalents — basic exit code coverage.
    // JSON output tests are omitted because quoting double quotes inside
    // cmd.exe is extremely fragile. Use Python hooks for JSON on Windows.
    #[cfg(windows)]
    #[tokio::test(flavor = "current_thread")]
    async fn win_exit0_allows() {
        let runner = runner_pre(vec![simple_hook("exit 0")]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(!result.denied);
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "current_thread")]
    async fn win_exit2_denies() {
        let runner = runner_pre(vec![simple_hook("exit 2")]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(result.denied);
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "current_thread")]
    async fn win_exit1_nonblocking() {
        let runner = runner_pre(vec![simple_hook("exit 1")]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        // exit 1 → NonBlockingError: chain continues
        assert!(!result.denied);
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "current_thread")]
    async fn win_spawn_error_nonblocking() {
        let runner = runner_pre(vec![simple_hook("__nonexistent_cmd_xyz__")]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(!result.denied);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn exit0_with_stderr_allows_warning_to_user() {
        let cmd = sh_stderr_exit("something suspicious", 0);
        let runner = runner_pre(vec![simple_hook(&cmd)]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(!result.denied);
        assert_eq!(result.stderr_lines.len(), 1);
        assert!(matches!(result.stderr_lines[0].level, StderrLevel::Warning));
        assert!(matches!(result.stderr_lines[0].destination, StderrDestination::User));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn exit2_no_stderr_denies() {
        let runner = runner_pre(vec![simple_hook("exit 2")]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(result.denied);
        assert!(result.stderr_lines.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn exit2_with_stderr_denies_error_to_model() {
        let cmd = sh_stderr_exit("blocked because dangerous", 2);
        let runner = runner_pre(vec![simple_hook(&cmd)]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(result.denied);
        assert_eq!(result.stderr_lines.len(), 1);
        assert!(matches!(result.stderr_lines[0].level, StderrLevel::Error));
        assert!(matches!(result.stderr_lines[0].destination, StderrDestination::Model));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn exit1_with_stderr_nonblocking_warning_to_user() {
        let cmd = sh_stderr_exit("linter warning", 1);
        let runner = runner_pre(vec![simple_hook(&cmd)]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        // NonBlockingError: chain continues
        assert!(!result.denied);
        assert_eq!(result.stderr_lines.len(), 1);
        assert!(matches!(result.stderr_lines[0].level, StderrLevel::Warning));
        assert!(matches!(result.stderr_lines[0].destination, StderrDestination::User));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn exit1_no_stderr_nonblocking_empty_lines() {
        let runner = runner_pre(vec![simple_hook("exit 1")]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(!result.denied);
        assert!(result.stderr_lines.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn spawn_error_is_nonblocking() {
        let runner = runner_pre(vec![simple_hook("__nonexistent_command_xyz__")]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        // SpawnError must not block the tool
        assert!(!result.denied);
        assert!(result.stderr_lines.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn json_decision_block_at_exit0_denies() {
        let cmd = sh_json_exit(r#"{"decision":"block","reason":"not allowed"}"#, 0);
        let runner = runner_pre(vec![simple_hook(&cmd)]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        // JSON has priority over exit code
        assert!(result.denied);
    }

    // ── Three tracks ──────────────────────────────────────────────────────

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn two_main_hooks_both_applied() {
        let j1 = sh_json_exit(r#"{"additional_context":"ctx-A"}"#, 0);
        let j2 = sh_json_exit(r#"{"additional_context":"ctx-B"}"#, 0);
        let runner = runner_pre(vec![simple_hook(&j1), simple_hook(&j2)]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(!result.denied);
        assert_eq!(result.additional_contexts.len(), 2);
        assert!(result.additional_contexts.contains(&"ctx-A".to_string()));
        assert!(result.additional_contexts.contains(&"ctx-B".to_string()));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn main_deny_stops_enforce() {
        let deny = sh_json_exit(r#"{"decision":"block"}"#, 0);
        // Enforce hook would set additional_context if it ran
        let enforce = sh_json_exit(r#"{"additional_context":"enforce-ran"}"#, 0);
        let runner = runner_pre(vec![
            simple_hook(&deny),
            enforce_hook(&enforce),
        ]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(result.denied);
        // Enforce must not have run
        assert!(result.additional_contexts.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn two_main_updated_input_distinct_fields() {
        let j1 = sh_json_exit(
            r#"{"hook_specific_output":{"hook_event_name":"PreToolUse","updated_input":{"a":1}}}"#,
            0,
        );
        let j2 = sh_json_exit(
            r#"{"hook_specific_output":{"hook_event_name":"PreToolUse","updated_input":{"b":2}}}"#,
            0,
        );
        let runner = runner_pre(vec![simple_hook(&j1), simple_hook(&j2)]);
        let result = runner
            .run_pre_tool_use("Bash", &json!({"command":"original"}), &ctx())
            .await;
        let upd = result.updated_input.unwrap();
        assert_eq!(upd["a"], 1);
        assert_eq!(upd["b"], 2);
        assert_eq!(upd["command"], "original"); // original preserved
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn two_main_updated_input_same_field_last_write_wins() {
        let j1 = sh_json_exit(
            r#"{"hook_specific_output":{"hook_event_name":"PreToolUse","updated_input":{"command":"from-hook-1"}}}"#,
            0,
        );
        let j2 = sh_json_exit(
            r#"{"hook_specific_output":{"hook_event_name":"PreToolUse","updated_input":{"command":"from-hook-2"}}}"#,
            0,
        );
        let runner = runner_pre(vec![simple_hook(&j1), simple_hook(&j2)]);
        let result = runner
            .run_pre_tool_use("Bash", &json!({"command":"original"}), &ctx())
            .await;
        // Last write wins (hook-2 is last in config order)
        assert_eq!(result.updated_input.unwrap()["command"], "from-hook-2");
    }

    // ── Main / Enforce composability ──────────────────────────────────────

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn enforce_after_main_both_mutations_applied() {
        let main_cmd = sh_json_exit(
            r#"{"hook_specific_output":{"hook_event_name":"PreToolUse","updated_input":{"a":1}}}"#,
            0,
        );
        let enforce_cmd = sh_json_exit(
            r#"{"hook_specific_output":{"hook_event_name":"PreToolUse","updated_input":{"b":2}}}"#,
            0,
        );
        let runner = runner_pre(vec![simple_hook(&main_cmd), enforce_hook(&enforce_cmd)]);
        let result = runner
            .run_pre_tool_use("Bash", &json!({"command":"original"}), &ctx())
            .await;
        let upd = result.updated_input.unwrap();
        assert_eq!(upd["a"], 1);
        assert_eq!(upd["b"], 2);
        assert_eq!(upd["command"], "original");
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn two_enforce_hooks_sequential() {
        // First Enforce adds "a", second adds "b"
        // The final result must have both — proving they compose
        let e1 = sh_json_exit(
            r#"{"hook_specific_output":{"hook_event_name":"PreToolUse","updated_input":{"a":1}}}"#,
            0,
        );
        let e2 = sh_json_exit(
            r#"{"hook_specific_output":{"hook_event_name":"PreToolUse","updated_input":{"b":2}}}"#,
            0,
        );
        let runner = runner_pre(vec![enforce_hook(&e1), enforce_hook(&e2)]);
        let result = runner
            .run_pre_tool_use("Bash", &json!({}), &ctx())
            .await;
        let upd = result.updated_input.unwrap();
        assert_eq!(upd["a"], 1);
        assert_eq!(upd["b"], 2);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn enforce_deny_stops_immediately_second_never_runs() {
        let deny = sh_json_exit(r#"{"decision":"block"}"#, 0);
        // If second Enforce ran, it would add additional_context
        let second = sh_json_exit(r#"{"additional_context":"second-ran"}"#, 0);
        let runner = runner_pre(vec![enforce_hook(&deny), enforce_hook(&second)]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(result.denied);
        assert!(result.additional_contexts.is_empty());
    }

    // ── PermissionDecision::Ask ───────────────────────────────────────────

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn hook_ask_sets_requires_confirmation() {
        let cmd = sh_json_exit(
            r#"{"hook_specific_output":{"hook_event_name":"PreToolUse","permission_decision":"ask","permission_decision_reason":"Need your approval"}}"#,
            0,
        );
        let runner = runner_pre(vec![simple_hook(&cmd)]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(!result.denied);
        assert!(result.requires_confirmation());
        assert_eq!(result.reason(), Some("Need your approval"));
    }

    // ── Message channels ──────────────────────────────────────────────────

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn system_message_goes_to_system_messages_not_additional_contexts() {
        let cmd = sh_json_exit(r#"{"system_message":"warning for user"}"#, 0);
        let runner = runner_pre(vec![simple_hook(&cmd)]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert_eq!(result.system_messages, vec!["warning for user"]);
        assert!(result.additional_contexts.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn additional_context_goes_to_additional_contexts_not_system_messages() {
        let cmd = sh_json_exit(r#"{"additional_context":"context for LLM"}"#, 0);
        let runner = runner_pre(vec![simple_hook(&cmd)]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert_eq!(result.additional_contexts, vec!["context for LLM"]);
        assert!(result.system_messages.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn two_hooks_additional_contexts_accumulated() {
        let c1 = sh_json_exit(r#"{"additional_context":"ctx-1"}"#, 0);
        let c2 = sh_json_exit(r#"{"additional_context":"ctx-2"}"#, 0);
        let runner = runner_pre(vec![simple_hook(&c1), simple_hook(&c2)]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert_eq!(result.additional_contexts.len(), 2);
    }

    // ── GardenEvent ───────────────────────────────────────────────────────

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn garden_event_from_allow_hook_accumulated() {
        let cmd = sh_json_exit(
            r#"{"garden_event":{"animation":"typing","move_to_zone":"desk-zone"}}"#,
            0,
        );
        let runner = runner_pre(vec![simple_hook(&cmd)]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert_eq!(result.garden_events.len(), 1);
        assert_eq!(result.garden_events[0].animation.as_deref(), Some("typing"));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn garden_event_from_deny_hook_still_in_result() {
        let cmd = sh_json_exit(
            r#"{"decision":"block","garden_event":{"animation":"error"}}"#,
            0,
        );
        let runner = runner_pre(vec![simple_hook(&cmd)]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(result.denied);
        assert_eq!(result.garden_events.len(), 1);
        assert_eq!(result.garden_events[0].animation.as_deref(), Some("error"));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn multiple_hooks_garden_events_all_accumulated() {
        let h1 = sh_json_exit(r#"{"garden_event":{"animation":"typing"}}"#, 0);
        let h2 = sh_json_exit(r#"{"garden_event":{"animation":"idle"}}"#, 0);
        let runner = runner_pre(vec![simple_hook(&h1), simple_hook(&h2)]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert_eq!(result.garden_events.len(), 2);
    }

    // ── Updated output (PostToolUse) ──────────────────────────────────────

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn updated_output_overrides_tool_output() {
        let cmd = sh_json_exit(
            r#"{"hook_specific_output":{"hook_event_name":"PostToolUse","updated_output":"redacted output"}}"#,
            0,
        );
        let runner = runner_post(vec![simple_hook(&cmd)]);
        let result = runner
            .run_post_tool_use("Bash", &json!({}), "raw tool output", &ctx())
            .await;
        assert_eq!(result.updated_output.as_deref(), Some("redacted output"));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn no_updated_output_passes_through_original() {
        let runner = runner_post(vec![simple_hook("exit 0")]);
        let result = runner
            .run_post_tool_use("Bash", &json!({}), "original", &ctx())
            .await;
        assert!(result.updated_output.is_none());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn updated_output_empty_string_is_intentional() {
        let cmd = sh_json_exit(
            r#"{"hook_specific_output":{"hook_event_name":"PostToolUse","updated_output":""}}"#,
            0,
        );
        let runner = runner_post(vec![simple_hook(&cmd)]);
        let result = runner
            .run_post_tool_use("Bash", &json!({}), "original", &ctx())
            .await;
        // Empty string is intentional, not absent
        assert_eq!(result.updated_output.as_deref(), Some(""));
    }

    // ── Timeouts ──────────────────────────────────────────────────────────

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn timeout_fail_policy_denies() {
        let mut h = simple_hook("sleep 10");
        h.timeout_secs = Some(0); // immediate timeout
        h.on_timeout = OnTimeoutPolicy::Fail;
        let runner = runner_pre(vec![h]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(result.denied);
        assert!(result.timed_out);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn timeout_continue_policy_allows() {
        let mut h = simple_hook("sleep 10");
        h.timeout_secs = Some(0); // immediate timeout
        h.on_timeout = OnTimeoutPolicy::Continue;
        let runner = runner_pre(vec![h]);
        let result = runner.run_pre_tool_use("Bash", &json!({}), &ctx()).await;
        assert!(!result.denied);
        assert!(!result.timed_out);
    }

    // ── Stop — fire & forget ──────────────────────────────────────────────

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn stop_hook_does_not_block_caller() {
        // Stop spawns via tokio::spawn — caller returns immediately.
        // We just verify run_stop() returns without awaiting anything.
        let mut h = simple_hook("sleep 5");
        h.r#async = true;
        let runner = HookRunner::new(CompiledAgentHookConfig {
            pre_tool_use: vec![],
            post_tool_use: vec![],
            post_tool_use_failure: vec![],
            stop: vec![h],
            user_prompt_submit: vec![],
            session_start: vec![],
            session_end: vec![],
            subagent_start: vec![],
            subagent_stop: vec![],
            pre_tool_use_timeout: Duration::from_secs(5),
            post_tool_use_timeout: Duration::from_secs(5),
            post_tool_use_failure_timeout: Duration::from_secs(5),
        });
        // run_stop is sync (spawns tokio task); if it blocks, this test hangs.
        runner.run_stop("end_turn", Some("last message"), &ctx());
        // If we reach here, it didn't block.
    }
}
