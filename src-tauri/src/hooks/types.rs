use serde::{Deserialize, Serialize};

// ── HookEvent ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    Stop,
    UserPromptSubmit,
    SessionStart,
    SessionEnd,
    SubagentStart,
    SubagentStop,
}

impl HookEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::PostToolUse => "PostToolUse",
            HookEvent::PostToolUseFailure => "PostToolUseFailure",
            HookEvent::Stop => "Stop",
            HookEvent::UserPromptSubmit => "UserPromptSubmit",
            HookEvent::SessionStart => "SessionStart",
            HookEvent::SessionEnd => "SessionEnd",
            HookEvent::SubagentStart => "SubagentStart",
            HookEvent::SubagentStop => "SubagentStop",
        }
    }
}

// ── AgentHookConfig ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentHookConfig {
    #[serde(rename = "PreToolUse", default)]
    pub pre_tool_use: Vec<HookCommand>,

    #[serde(rename = "PostToolUse", default)]
    pub post_tool_use: Vec<HookCommand>,

    #[serde(rename = "PostToolUseFailure", default)]
    pub post_tool_use_failure: Vec<HookCommand>,

    #[serde(rename = "Stop", default)]
    pub stop: Vec<HookCommand>,

    #[serde(rename = "UserPromptSubmit", default)]
    pub user_prompt_submit: Vec<HookCommand>,

    #[serde(rename = "SessionStart", default)]
    pub session_start: Vec<HookCommand>,

    #[serde(rename = "SessionEnd", default)]
    pub session_end: Vec<HookCommand>,

    #[serde(rename = "SubagentStart", default)]
    pub subagent_start: Vec<HookCommand>,

    #[serde(rename = "SubagentStop", default)]
    pub subagent_stop: Vec<HookCommand>,

    #[serde(default = "default_pre_timeout_secs")]
    pub pre_tool_use_timeout_secs: u64,

    #[serde(default = "default_post_timeout_secs")]
    pub post_tool_use_timeout_secs: u64,

    #[serde(default = "default_post_timeout_secs")]
    pub post_tool_use_failure_timeout_secs: u64,
}

fn default_pre_timeout_secs() -> u64 {
    10
}
fn default_post_timeout_secs() -> u64 {
    30
}

// ── HookCommand ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HookCommand {
    /// Short form: just the command string.
    Simple(String),

    WithOptions {
        command: String,

        // ── Filters ──────────────────────────────────────────────────
        /// Exact list of tool names this hook applies to.
        tools: Option<Vec<String>>,

        /// Pattern to match against tool name.
        ///   - Regex:     "/^Bash$/"
        ///   - Wildcard:  "Bash*"
        ///   - Substring: "edit"
        ///   - Absent:    always applies
        matcher: Option<String>,

        /// Filter by tool name + input content.
        /// Syntax: "ToolName(pattern)" where pattern is a glob over the JSON input.
        ///   "Bash(git *)"  → only when Bash runs a git command
        ///   "Read(*.ts)"   → only when Read reads TypeScript files
        #[serde(rename = "if")]
        if_condition: Option<String>,

        // ── Execution mode ────────────────────────────────────────────
        /// true → fire & forget, never blocks the tool.
        /// false → sync, result affects whether the tool executes (default).
        #[serde(default)]
        r#async: bool,

        /// Like async, but if the hook exits with code 2, notifies the model
        /// via event_tx. Ignored when async: false.
        #[serde(default)]
        async_rewake: bool,

        // ── Timing ────────────────────────────────────────────────────
        /// Override the global event timeout for this specific hook.
        timeout_secs: Option<u64>,

        /// What to do when this hook times out in PreToolUse.
        /// "fail"     → cuts the chain (default)
        /// "continue" → tool executes even if the hook didn't respond
        #[serde(default)]
        on_timeout: OnTimeoutPolicy,

        // ── Execution phase ───────────────────────────────────────────
        /// "main"    → runs in parallel with other Main hooks (default).
        ///             Sees the original input. Mutations are merged with
        ///             last-write-wins. Use for: formatters, enrichers, tagging.
        ///
        /// "enforce" → runs sequentially AFTER all Main hooks.
        ///             Sees the accumulated effective_input.
        ///             Use for: security guards, invariant checks, sanitization.
        ///
        /// Async hooks ignore this field — they are always fire-and-forget.
        #[serde(default)]
        phase: HookPhase,

        // ── UI ────────────────────────────────────────────────────────
        /// Message shown in the spinner while the hook runs.
        status_message: Option<String>,
    },
}

// ── HookPhase ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookPhase {
    #[default]
    Main,
    Enforce,
}

// ── OnTimeoutPolicy ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnTimeoutPolicy {
    #[default]
    Fail,
    Continue,
}

// ── HookOutput — what the hook returns on stdout ──────────────────────────

#[derive(Debug, Clone, Deserialize, Default)]
pub struct HookOutput {
    /// false → hook wants to block (equivalent to exit 2 in JSON).
    #[serde(rename = "continue")]
    pub should_continue: Option<bool>,

    /// "approve" | "block" — explicit permission control from JSON.
    pub decision: Option<HookDecision>,

    /// Reason visible to the user when the hook blocks.
    pub reason: Option<String>,

    /// Channel 1: warning to the USER (warning banner in the UI).
    /// NOT sent to the model/LLM.
    pub system_message: Option<String>,

    /// Channel 2: context for the MODEL (added to conversation context).
    /// NOT shown directly to the user.
    pub additional_context: Option<String>,

    /// Suppress stdout from the session transcript.
    #[serde(default)]
    pub suppress_output: bool,

    /// Event-specific output fields.
    pub hook_specific_output: Option<HookSpecificOutput>,

    /// Garden extension: UI events emitted by this hook.
    pub garden_event: Option<GardenEvent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookDecision {
    Approve,
    Block,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HookSpecificOutput {
    pub hook_event_name: String,
    /// Partial input update for the tool (partial merge, not total replace).
    /// Only applies in PreToolUse and PermissionRequest with decision: approve.
    pub updated_input: Option<serde_json::Value>,
    /// Override the tool output as seen by the LLM (PostToolUse).
    /// The real tool output does not change — only what gets injected into LLM context.
    pub updated_output: Option<String>,
    /// Three-state permission decision.
    pub permission_decision: Option<PermissionDecision>,
    pub permission_decision_reason: Option<String>,
}

impl HookSpecificOutput {
    /// Combines the two flat permission fields into a single `PermissionResult`.
    /// Called by `parse_hook_output` — centralizes the construction so no
    /// caller has to remember to pair both fields manually.
    pub fn permission_result(&self) -> Option<PermissionResult> {
        self.permission_decision.as_ref().map(|d| PermissionResult {
            decision: d.clone(),
            reason: self.permission_decision_reason.clone(),
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask,
}

/// Unified permission result: decision + reason in one place.
///
/// Replaces the previously scattered `permission_decision` + `permission_decision_reason`
/// pattern. Single source of truth for both the dialog reason (Ask) and the
/// block reason (Deny).
#[derive(Debug, Clone)]
pub struct PermissionResult {
    pub decision: PermissionDecision,
    /// Shown in the confirmation dialog when Ask, or as the block reason when Deny.
    pub reason: Option<String>,
}

// ── FinalDecision ─────────────────────────────────────────────────────────

/// The single, authoritative outcome after running all PreToolUse hooks.
///
/// Collapses the four decision sources into one unambiguous value:
///   - exit code (`0` / `2` / `1+`)
///   - `decision` field (`"approve"` / `"block"`)
///   - `permission_decision` field (`"allow"` / `"deny"` / `"ask"`)
///   - `should_continue` field (`false`)
///
/// Prefer `HookRunResult::final_decision()` over calling `is_denied()` +
/// `requires_confirmation()` separately — those two boolean checks can produce
/// inconsistent states (e.g. `denied=true` AND `permission=Ask` coexisting).
///
/// Priority invariant: **Deny > Ask > Allow**.
/// A Deny outcome can never be overridden by a later Ask from a different hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalDecision {
    /// All hooks approved — tool should execute.
    Allow,
    /// A hook blocked execution — tool must NOT run.
    Deny {
        /// Human-readable reason (for the user or the LLM). May be absent.
        reason: Option<String>,
    },
    /// A hook requires human confirmation before the tool executes.
    /// The caller must show a dialog and await the user's decision.
    Ask {
        /// Text to display in the confirmation dialog.
        reason: Option<String>,
    },
}

// ── GardenEvent ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GardenEvent {
    pub move_to_zone: Option<String>,
    pub animation: Option<String>,
    pub notify: Option<String>,
}

// ── Stderr capture ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StderrEntry {
    pub command: String,
    pub event: HookEvent,
    pub content: String,
    pub level: StderrLevel,
    pub destination: StderrDestination,
}

#[derive(Debug, Clone)]
pub enum StderrLevel {
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub enum StderrDestination {
    /// Inject into LLM context so the model understands why it was blocked.
    Model,
    /// Show as warning in the UI — does not contaminate the LLM.
    User,
    /// Internal operator logs only.
    Logs,
}

// ── HookRunResult ─────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct HookRunResult {
    // Final decision
    pub denied: bool,
    pub timed_out: bool,
    pub cancelled: bool,

    // Model channel: injected into conversation context
    pub additional_contexts: Vec<String>,

    // User channel: shown as warnings in the UI
    pub system_messages: Vec<String>,

    // Diagnostic channel: stderr segregated by destination
    pub stderr_lines: Vec<StderrEntry>,

    // Effective input for the tool (result of all PreToolUse hook merges)
    pub updated_input: Option<serde_json::Value>,

    // Effective output for the LLM (last-write-wins from PostToolUse hooks)
    pub updated_output: Option<String>,

    /// Unified permission state from all hooks.
    /// None  → no hook expressed a permission preference.
    /// Some  → decision (Allow/Deny/Ask) + optional reason, in one place.
    pub permission: Option<PermissionResult>,

    // Accumulated garden events from all hooks
    pub garden_events: Vec<GardenEvent>,
}

impl HookRunResult {
    pub fn is_denied(&self) -> bool {
        self.denied
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// true if any sync hook returned PermissionDecision::Ask.
    /// Requires showing a confirmation dialog before executing the tool.
    pub fn requires_confirmation(&self) -> bool {
        matches!(
            self.permission,
            Some(PermissionResult { decision: PermissionDecision::Ask, .. })
        )
    }

    pub fn reason(&self) -> Option<&str> {
        self.permission.as_ref().and_then(|p| p.reason.as_deref())
    }

    /// Returns the single, authoritative decision for this hook run.
    ///
    /// Replaces the pattern `if is_denied() { … } else if requires_confirmation() { … }`:
    /// those two separate boolean checks can coexist in an inconsistent state
    /// (`denied=true` AND `permission=Ask` simultaneously). `final_decision()` always
    /// applies the **Deny > Ask > Allow** invariant, producing one unambiguous value.
    pub fn final_decision(&self) -> FinalDecision {
        if self.denied {
            FinalDecision::Deny {
                reason: self.reason().map(str::to_string),
            }
        } else if self.requires_confirmation() {
            FinalDecision::Ask {
                reason: self.reason().map(str::to_string),
            }
        } else {
            FinalDecision::Allow
        }
    }
}

// ── AgentContext ──────────────────────────────────────────────────────────

/// Context passed to hooks about the running agent.
#[derive(Debug, Clone)]
pub struct AgentContext {
    pub agent_id: String,
    pub agent_zone: Option<String>,
}
