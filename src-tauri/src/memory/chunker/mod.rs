//! Chunker selector + shared types (Phase 3 — REQ-CHK-001, REQ-CHK-002, REQ-CHK-006).

pub(crate) mod chat;
pub(crate) mod code;
pub(crate) mod token_splitter;

pub(crate) use chat::ChatChunker;
pub(crate) use code::CodeChunker;

use crate::models::chat::Role;

/// Which chunker the worker dispatches to for this event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChunkerKind {
    Chat,
    /// Reserved for Phase 4 file-watcher code ingest. `select_chunker` does not
    /// return this variant in Phase 3 (all events come from `push_and_persist`),
    /// but the worker match arm is present so Phase 4 can wire it without
    /// touching the worker.
    #[allow(dead_code)]
    Code,
    /// Worker drops the event without inserting.
    Skip,
}

/// One unit of content that becomes ONE row in `memory` + one in `memory_vectors`.
#[derive(Debug, Clone)]
pub(crate) struct Chunk {
    pub text: String,
    /// Approximate token count (480 cap target). Set by the tokenizer path;
    /// 0 in byte-fallback mode. Read by telemetry / Phase 4 retrieval re-ranking.
    #[allow(dead_code)]
    pub token_count: usize,
    /// Optional header describing context (e.g. "fn foo (rust)") — currently
    /// populated by code.rs. None for chat chunks.
    pub header: Option<String>,
}

/// Error type for chunking operations.
///
/// Currently unused at runtime — reserved for Phase 4 where code-chunk parse
/// failures are surfaced through the command layer.
#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub(crate) enum ChunkerError {
    #[error("{0}")]
    ParseFailed(String),
    #[error("tokenizer init failed: {0}")]
    TokenizerInit(String),
}

/// Route an event to its chunker. Pure function — no I/O.
pub(crate) fn select_chunker(
    role: Role,
    is_error: Option<bool>,
    content: &str,
) -> ChunkerKind {
    if content.is_empty() {
        return ChunkerKind::Skip;
    }
    match role {
        Role::User | Role::Assistant | Role::Tool => {
            // is_error only changes memory_type at insert time, not chunker selection.
            let _ = is_error;
            ChunkerKind::Chat
        }
        Role::System => ChunkerKind::Skip, // LOCKED D8 — defensive
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::chat::Role;

    #[test]
    fn select_chunker_user_routes_to_chat() {
        assert_eq!(select_chunker(Role::User, None, "hello"), ChunkerKind::Chat);
    }

    #[test]
    fn select_chunker_assistant_with_content_routes_to_chat() {
        assert_eq!(select_chunker(Role::Assistant, None, "some text"), ChunkerKind::Chat);
    }

    #[test]
    fn select_chunker_assistant_empty_content_routes_to_skip() {
        assert_eq!(select_chunker(Role::Assistant, None, ""), ChunkerKind::Skip);
    }

    #[test]
    fn select_chunker_tool_routes_to_chat() {
        assert_eq!(select_chunker(Role::Tool, Some(true), "error msg"), ChunkerKind::Chat);
    }

    #[test]
    fn select_chunker_system_routes_to_skip() {
        assert_eq!(select_chunker(Role::System, None, "system prompt"), ChunkerKind::Skip);
    }

    #[test]
    fn chat_chunk_has_no_header() {
        let c = Chunk { text: "hi".into(), token_count: 1, header: None };
        assert_eq!(c.header, None);
    }

    #[test]
    fn code_chunk_has_header() {
        let c = Chunk { text: "fn foo() {}".into(), token_count: 5, header: Some("fn foo".into()) };
        assert!(c.header.is_some());
    }

    #[test]
    fn chunker_error_implements_error_trait() {
        let _: Box<dyn std::error::Error> = Box::new(ChunkerError::ParseFailed("x".into()));
    }

    #[test]
    fn chunker_error_parse_failed_displays_message() {
        let e = ChunkerError::ParseFailed("syntax error".into());
        let s = format!("{e}");
        assert!(s.contains("syntax error"), "got: {s}");
    }

    // T07-test: TokenSplitter tests (included here per tasks.md)
    #[test]
    fn token_split_respects_cap() {
        use crate::memory::chunker::token_splitter::TokenSplitter;
        let splitter = TokenSplitter::fallback();
        let long_text = "a".repeat(10_000);
        let max_bytes = 480 * 4; // 1920
        let segments = splitter.split(&long_text, 480);
        assert!(segments.len() >= 2, "expected >= 2 segments for 10k chars");
        for seg in &segments {
            assert!(
                seg.text.len() <= max_bytes,
                "segment byte length {} > max_bytes {}",
                seg.text.len(),
                max_bytes
            );
        }
    }

    #[test]
    fn token_split_short_input_single_segment() {
        use crate::memory::chunker::token_splitter::TokenSplitter;
        let splitter = TokenSplitter::fallback();
        let segments = splitter.split("hello world", 480);
        assert_eq!(segments.len(), 1, "short input should produce exactly 1 segment");
    }
}
