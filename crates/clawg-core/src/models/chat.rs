use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    #[serde(default = "generate_message_id")]
    pub id: String,
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default = "current_time_millis")]
    pub timestamp_ms: u64,
    /// Phase 3 (LOCKED D1) — only `Some(true)` on `Role::Tool` messages whose
    /// underlying tool execution surfaced an error. Default `None` keeps old
    /// persisted session JSON forward-compatible (R1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

pub fn generate_message_id() -> String {
    format!("msg-{}", uuid::Uuid::new_v4().as_simple())
}

pub fn current_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique ID for matching tool_use → tool_result (important for Anthropic API)
    #[serde(default = "generate_tool_id")]
    pub id: String,
    pub function: ToolFunction,
}

fn generate_tool_id() -> String {
    format!("toolu_{}", uuid::Uuid::new_v4().as_simple())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ToolFunctionDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub system_prompt: Option<String>,
    /// Per-agent custom instructions — sent as a SEPARATE system block so the
    /// base system prompt prefix can be shared across agents with the same role.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    /// Provider-specific parameters (e.g. `thinking_effort` for Anthropic).
    /// Each provider reads only the keys it understands and ignores the rest.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extensions: HashMap<String, serde_json::Value>,
}

/// Result of a chat stream — either text content or tool call requests.
#[derive(Debug, Clone)]
pub enum ChatStreamResult {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

/// Events sent through the desktop app's streaming channel.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
pub enum StreamEvent {
    #[serde(rename = "delta")]
    Delta { delta: String },

    #[serde(rename = "state")]
    State { state: String },

    #[serde(rename = "done")]
    Done { full_response: String },

    #[serde(rename = "error")]
    Error { error: String },

    #[serde(rename = "tool_use")]
    ToolUse {
        tool_name: String,
        arguments: String,
    },

    #[serde(rename = "compaction")]
    Compaction {
        removed_count: usize,
        estimated_tokens_freed: usize,
    },

    #[serde(rename = "usage")]
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        cache_creation_tokens: u32,
        cache_read_tokens: u32,
    },

    /// Warning message from a hook — shown to the user, NOT injected into the LLM.
    #[serde(rename = "hook_warning")]
    HookWarning { message: String },

    /// Garden event emitted by a hook — drives UI animations and zone changes.
    #[serde(rename = "hook_garden_event")]
    HookGardenEvent {
        move_to_zone: Option<String>,
        animation: Option<String>,
        notify: Option<String>,
    },

    /// A recovery step is about to be executed for a failing tool.
    /// Gives the user real-time feedback instead of silent hanging.
    #[serde(rename = "recovery_attempt")]
    RecoveryAttempt {
        tool: String,
        /// Human-readable description of the step being attempted.
        step: String,
        /// 1-based attempt number within `max_attempts`.
        attempt: u32,
    },

    /// The outcome of a recovery sequence for a tool.
    #[serde(rename = "recovery_result")]
    RecoveryResult {
        tool: String,
        succeeded: bool,
        /// Short outcome code: "recovered", "escalated", "max_attempts_exhausted",
        /// "escalated_to_llm", "recovery-step-timeout", "oauth-refresh-failed", etc.
        outcome: String,
    },

    /// Incremental progress from a long-running tool (bash stdout chunks, web_fetch bytes, etc.).
    /// Emitted BEFORE the final tool_result so the UI can animate the agent in real time
    /// instead of showing a frozen state until the tool finishes.
    ///
    /// The fields mirror `tools::progress::ToolProgress`:
    ///   - `stdout_chunk` / `stderr_chunk` are filled by bash line-by-line streaming
    ///   - `bytes_read` is filled by web_fetch during body download
    ///   - `message` is a free-form description for Generic events
    ///
    /// All structured fields are optional; consumers switch on which is present.
    #[serde(rename = "tool_progress")]
    ToolProgress {
        tool: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        stdout_chunk: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stderr_chunk: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        bytes_read: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Human-in-the-loop gate: the agent is about to invoke a tool that
    /// requires explicit user approval (destructive native tool, or hook
    /// `Ask` decision). The chat loop is paused on a oneshot until the UI
    /// resolves it via `resolve_tool_confirmation(call_id, decision)`.
    #[serde(rename = "confirmation_required")]
    ConfirmationRequired {
        /// Stable identifier for matching the response. Distinct from the
        /// LLM-issued tool_use id so hook-only Asks (no tool_call) also fit.
        call_id: String,
        tool_name: String,
        /// Effective input the tool will receive if approved (post-hook mutation).
        arguments: serde_json::Value,
        /// Why we are asking — populated for hook Asks; None for is_destructive gate.
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// What triggered the prompt: "destructive_tool" | "hook_ask".
        source: String,
    },

    /// Echoed back after the user resolves a confirmation, for UI bookkeeping
    /// (closing the modal, logging in transcript). The actual gate result is
    /// delivered via the oneshot inside the chat loop.
    #[serde(rename = "confirmation_resolved")]
    ConfirmationResolved {
        call_id: String,
        /// "allow" | "deny"
        decision: String,
    },

    /// The agent invoked the `report_anomaly` self-reporting tool (12-B).
    /// This is an ADVISORY signal — the chat loop continues without pause.
    /// The UI surfaces it as a system message so the user can audit the
    /// agent's self-assessment in real time.
    #[serde(rename = "anomaly_reported")]
    AnomalyReported {
        /// Stable id for this report (so the UI can dedupe if the loop replays).
        report_id: String,
        /// One of: unintended_action | reasoning_mismatch |
        /// prompt_injection_suspicion | irreversible_action.
        anomaly_type: String,
        description: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        intended_action: Option<String>,
    },

    /// Embedder model bootstrap progress (Phase 2 RAG — REQ-EVT-001).
    ///
    /// Emitted app-wide via `app.emit("embedder-download", payload)` (a NEW
    /// pattern in this codebase — see `docs/sdd/rag-embedder/design.md` §5.6).
    /// Lifecycle on success:  `start` → `downloading` (1..N, rate-limited) →
    /// `verifying` → `done`. On error: any prefix, then exactly one terminal
    /// `error` event.
    ///
    /// The Angular union in `src/app/models/chat.model.ts` mirrors this shape.
    #[serde(rename = "embedder_download")]
    EmbedderDownload {
        /// One of: `"start" | "downloading" | "verifying" | "done" | "error"`.
        phase: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        bytes_downloaded: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        bytes_total: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

// ============================================================================
// Phase 2 RAG — StreamEvent::EmbedderDownload serde shape (REQ-EVT-001 / T060)
// Phase 3 RAG — ChatMessage.is_error field (REQ-INT-001)
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    // ---------- T03-test — REQ-INT-001: ChatMessage.is_error ----------

    #[test]
    fn is_error_field_defaults_to_none_on_missing_key() {
        let json = r#"{"role":"user","content":"hi","timestamp_ms":0}"#;
        let msg: ChatMessage = serde_json::from_str(json).expect("deserialize");
        assert_eq!(msg.is_error, None);
    }

    #[test]
    fn is_error_field_serializes_false_when_some_false() {
        let msg = ChatMessage {
            id: "x".into(),
            role: Role::Tool,
            content: "ok".into(),
            tool_calls: None,
            timestamp_ms: 0,
            is_error: Some(false),
        };
        let s = serde_json::to_string(&msg).expect("serialize");
        assert!(s.contains(r#""is_error":false"#), "got: {s}");
    }

    #[test]
    fn is_error_field_omitted_when_none() {
        let msg = ChatMessage {
            id: "x".into(),
            role: Role::User,
            content: "hi".into(),
            tool_calls: None,
            timestamp_ms: 0,
            is_error: None,
        };
        let s = serde_json::to_string(&msg).expect("serialize");
        assert!(!s.contains(r#""is_error""#), "is_error must be omitted when None: {s}");
    }

    #[test]
    fn is_error_field_some_true_serializes_correctly() {
        let msg = ChatMessage {
            id: "x".into(),
            role: Role::Tool,
            content: "fail".into(),
            tool_calls: None,
            timestamp_ms: 0,
            is_error: Some(true),
        };
        let s = serde_json::to_string(&msg).expect("serialize");
        assert!(s.contains(r#""is_error":true"#), "got: {s}");
    }

    #[test]
    fn embedder_download_serializes_skipping_none_fields() {
        let ev = StreamEvent::EmbedderDownload {
            phase: "downloading".into(),
            bytes_downloaded: Some(1024),
            bytes_total: Some(2048),
            error: None,
        };
        let s = serde_json::to_string(&ev).expect("serialize");
        assert!(s.contains(r#""kind":"embedder_download""#), "missing kind: {s}");
        assert!(s.contains(r#""phase":"downloading""#), "missing phase: {s}");
        assert!(s.contains(r#""bytes_downloaded":1024"#), "missing bytes_downloaded: {s}");
        assert!(s.contains(r#""bytes_total":2048"#), "missing bytes_total: {s}");
        assert!(!s.contains(r#""error""#), "error key must be omitted when None: {s}");
    }

    #[test]
    fn embedder_download_skips_all_optional_when_none() {
        let ev = StreamEvent::EmbedderDownload {
            phase: "verifying".into(),
            bytes_downloaded: None,
            bytes_total: None,
            error: None,
        };
        let s = serde_json::to_string(&ev).expect("serialize");
        // Must contain only kind + phase.
        assert!(s.contains(r#""kind":"embedder_download""#), "missing kind: {s}");
        assert!(s.contains(r#""phase":"verifying""#), "missing phase: {s}");
        assert!(!s.contains(r#""bytes_downloaded""#), "bytes_downloaded must be omitted: {s}");
        assert!(!s.contains(r#""bytes_total""#), "bytes_total must be omitted: {s}");
        assert!(!s.contains(r#""error""#), "error must be omitted: {s}");
    }

    #[test]
    fn embedder_download_roundtrip() {
        let ev = StreamEvent::EmbedderDownload {
            phase: "error".into(),
            bytes_downloaded: None,
            bytes_total: None,
            error: Some("boom".into()),
        };
        let s = serde_json::to_string(&ev).expect("serialize");
        let back: StreamEvent = serde_json::from_str(&s).expect("deserialize");
        match back {
            StreamEvent::EmbedderDownload {
                phase,
                bytes_downloaded,
                bytes_total,
                error,
            } => {
                assert_eq!(phase, "error");
                assert_eq!(bytes_downloaded, None);
                assert_eq!(bytes_total, None);
                assert_eq!(error, Some("boom".into()));
            }
            other => panic!("expected EmbedderDownload, got {other:?}"),
        }
    }
}
