//! Memory ingest event types (Phase 3 — REQ-EVT-001, REQ-EVT-002, REQ-EVT-003).

use serde::{Deserialize, Serialize};

use crate::models::chat::{ChatMessage, Role};

/// One persisted chat message → one event on the ingest channel.
///
/// Built inside `push_and_persist` AFTER the on-disk session append succeeds,
/// so a lost event cannot leave a memory row without its conversation peer.
/// Owned values only — provably `Send + 'static` (see const block below).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEvent {
    pub agent_id: String,
    pub project_id: Option<String>,
    pub role: Role,
    pub content: String,
    /// Only `Some(_)` for `Role::Tool` messages. Drives `memory_type` selection.
    pub is_error: Option<bool>,
    pub source: MemorySource,
    pub timestamp_ms: u64,
    /// Client-declared memory type, honoured only for [`MemorySource::Explicit`].
    ///
    /// A chat turn's type is *derived* — the role and `is_error` already say
    /// what the message is, and letting a producer override that would let the
    /// ingest path disagree with the conversation it came from. A deliberately
    /// saved memory has no such derivation available: only the client knows
    /// whether it just recorded a preference, a decision or a fact.
    ///
    /// `#[serde(default)]` so every payload written before this field existed
    /// still parses, and so an MCP client may simply omit it.
    #[serde(default)]
    pub memory_type: Option<String>,
    /// Client-declared importance on the store's 0–10 scale, honoured only for
    /// [`MemorySource::Explicit`] and clamped on the way in. Same rationale and
    /// same `#[serde(default)]` contract as [`MemoryEvent::memory_type`].
    #[serde(default)]
    pub importance: Option<f64>,
}

/// Where the ingest event originated (which producer).
///
/// `source` describes the producer — NOT the role. Phase 3 only fires
/// `MemorySource::Chat`; the other variants are reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    /// Chat turn — user/assistant/tool message from `push_and_persist`.
    Chat,
    /// Compaction summary (reserved — Phase 7 may emit; not produced in Phase 3).
    Compaction,
    /// File-watcher-driven code ingest (reserved — Phase 4+).
    CodeFile,
    /// A memory a client saved deliberately (the MCP `save_memory` tool), not
    /// derived from a conversation.
    ///
    /// Kept distinct from [`MemorySource::Chat`] because the provenance is
    /// genuinely different: a chat memory is something the pipeline *noticed*,
    /// an explicit one is something a user *asserted*. That difference is what
    /// justifies honouring the client's `memory_type`/`importance` overrides
    /// here and nowhere else, and it keeps "what did I save on purpose?"
    /// answerable with a `WHERE source = 'explicit'`.
    Explicit,
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
                // A chat turn derives both from role/is_error — see the field
                // docs for why a producer is not allowed to override them.
                memory_type: None,
                importance: None,
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
    use crate::models::chat::{ChatMessage, Role};

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
            memory_type: None,
            importance: None,
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
            MemorySource::Explicit => "explicit",
        };
    }

    // ── Explicit source + client overrides ───────────────────────────────

    /// The MCP `save_memory` tool serializes an event over the wire, so the
    /// snake_case discriminant is a protocol constant, not an internal name.
    #[test]
    fn explicit_source_round_trips_through_serde_as_snake_case() {
        let json = serde_json::to_string(&MemorySource::Explicit).expect("serialize");
        assert_eq!(json, "\"explicit\"");
        let back: MemorySource = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, MemorySource::Explicit);
    }

    #[test]
    fn explicit_event_round_trips_with_its_overrides() {
        let evt = MemoryEvent {
            agent_id: "a".into(),
            project_id: Some("p".into()),
            role: Role::User,
            content: "the team uses conventional commits".into(),
            is_error: None,
            source: MemorySource::Explicit,
            timestamp_ms: 7,
            memory_type: Some("user_preference".into()),
            importance: Some(9.5),
        };
        let json = serde_json::to_string(&evt).expect("serialize");
        let back: MemoryEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.source, MemorySource::Explicit);
        assert_eq!(back.memory_type.as_deref(), Some("user_preference"));
        assert_eq!(back.importance, Some(9.5));
    }

    /// `#[serde(default)]` is what lets an MCP client omit both overrides —
    /// and what keeps every already-serialized chat event parseable.
    #[test]
    fn overrides_default_to_none_when_absent_from_the_payload() {
        let json = r#"{
            "agent_id": "a",
            "project_id": null,
            "role": "user",
            "content": "hello",
            "is_error": null,
            "source": "chat",
            "timestamp_ms": 0
        }"#;
        let evt: MemoryEvent = serde_json::from_str(json).expect("legacy payload must still parse");
        assert_eq!(evt.memory_type, None);
        assert_eq!(evt.importance, None);
    }

    /// Chat ingest never carries overrides: they exist for deliberately saved
    /// memories, and a chat turn has no client to declare them.
    #[test]
    fn from_message_leaves_both_overrides_unset() {
        let msg = ChatMessage {
            id: "msg-1".into(),
            role: Role::User,
            content: "hello".into(),
            tool_calls: None,
            timestamp_ms: 3,
            is_error: None,
        };
        let evt = MemoryEvent::from_message("a".into(), None, &msg, MemorySource::Chat)
            .expect("user message is ingested");
        assert_eq!(evt.memory_type, None);
        assert_eq!(evt.importance, None);
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
            memory_type: None,
            importance: None,
        });
        // Result is Err when receiver is dropped — must not panic
        let _ = result;
    }
}
