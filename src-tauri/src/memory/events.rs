//! Memory ingest event types (Phase 3 — REQ-EVT-001, REQ-EVT-002, REQ-EVT-003).

use serde::{Deserialize, Serialize};

use crate::models::chat::{ChatMessage, Role};

/// One persisted chat message → one event on the ingest channel.
///
/// Built inside `push_and_persist` AFTER the on-disk session append succeeds,
/// so a lost event cannot leave a memory row without its conversation peer.
/// Owned values only — provably `Send + 'static` (see const block below).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MemoryEvent {
    pub agent_id: String,
    pub project_id: Option<String>,
    pub role: Role,
    pub content: String,
    /// Only `Some(_)` for `Role::Tool` messages. Drives `memory_type` selection.
    pub is_error: Option<bool>,
    pub source: MemorySource,
    pub timestamp_ms: u64,
}

/// Where the ingest event originated (which producer).
///
/// `source` describes the producer — NOT the role. Phase 3 only fires
/// `MemorySource::Chat`; the other variants are reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemorySource {
    /// Chat turn — user/assistant/tool message from `push_and_persist`.
    Chat,
    /// Compaction summary (reserved — Phase 7 may emit; not produced in Phase 3).
    Compaction,
    /// File-watcher-driven code ingest (reserved — Phase 4+).
    CodeFile,
}

impl MemoryEvent {
    /// Build an event from a persisted `ChatMessage`.
    ///
    /// Returns `None` for messages we deliberately do NOT ingest:
    ///   - `Role::System` (LOCKED D8 — never reaches push_and_persist anyway, defensive)
    ///   - `Role::Assistant` with empty content AND `tool_calls = Some(_)`
    ///     (the "I'm about to call a tool" announcement — no content to embed)
    pub fn from_message(
        agent_id: String,
        project_id: Option<String>,
        msg: &ChatMessage,
        source: MemorySource,
    ) -> Option<Self> {
        match msg.role {
            Role::System => None,
            Role::Assistant if msg.content.is_empty() && msg.tool_calls.is_some() => None,
            _ => Some(Self {
                agent_id,
                project_id,
                role: msg.role.clone(),
                content: msg.content.clone(),
                is_error: msg.is_error,
                source,
                timestamp_ms: msg.timestamp_ms,
            }),
        }
    }
}

// Compile-time Send + 'static proof (Phase 2 carry-forward pattern — REQ-STATE-003).
const _: fn() = || {
    fn _send_static<T: Send + 'static>() {}
    fn _sync<T: Sync>() {}
    _send_static::<MemoryEvent>();
    _sync::<MemoryEvent>();
    _send_static::<tokio::sync::mpsc::UnboundedSender<MemoryEvent>>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::chat::Role;

    #[test]
    fn memory_event_is_send() {
        fn _assert_send<T: Send>() {}
        _assert_send::<MemoryEvent>();
    }

    #[test]
    fn memory_event_fields_round_trip() {
        let evt = MemoryEvent {
            role: Role::Tool,
            is_error: Some(true),
            content: "failed".into(),
            agent_id: "a".into(),
            project_id: None,
            source: MemorySource::Chat,
            timestamp_ms: 123,
        };
        assert!(matches!(evt.role, Role::Tool));
        assert_eq!(evt.is_error, Some(true));
    }

    #[test]
    fn memory_source_is_copy_and_clone() {
        let s = MemorySource::Chat;
        let _a = s;
        let _b = s;
    }

    #[test]
    fn memory_source_exhaustive_match() {
        let s = MemorySource::Chat;
        let _ = match s {
            MemorySource::Chat => "chat",
            MemorySource::Compaction => "compaction",
            MemorySource::CodeFile => "code_file",
        };
    }

    #[test]
    fn sender_dropped_does_not_panic() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<MemoryEvent>();
        drop(rx);
        let result = tx.send(MemoryEvent {
            agent_id: "a".into(),
            project_id: None,
            role: Role::User,
            content: "hello".into(),
            is_error: None,
            source: MemorySource::Chat,
            timestamp_ms: 0,
        });
        // Result is Err when receiver is dropped — must not panic
        let _ = result;
    }
}
