//! Chunker selector + shared types.

pub(crate) mod chat;
pub(crate) mod code;
pub(crate) mod token_splitter;

pub use chat::ChatChunker;
pub use code::CodeChunker;

use crate::models::chat::Role;

/// Upper bound on a chunk's token count. Stays under the embedder's 512-token
/// model limit with a safety margin (fastembed truncates silently past that,
/// and a truncated chunk's stored text would no longer match its own vector).
pub(crate) const CHUNK_MAX_TOKENS: usize = 480;

/// How many trailing tokens of one chunk are repeated as the leading tokens
/// of the next. A hard cut loses any fact that straddles the boundary — no
/// single vector represents it, so it is unretrievable. 64 (~13% of the cap)
/// buys back that coverage at a proportional embedding-cost increase.
pub(crate) const CHUNK_OVERLAP_TOKENS: usize = 64;

// overlap >= max collapses the stride to zero and the split loop never
// advances — catch that at compile time for the constants above (callers
// passing a different overlap are still runtime-clamped inside `split`).
const _: () = assert!(CHUNK_OVERLAP_TOKENS < CHUNK_MAX_TOKENS);

// A chunk plus the embedder's own special-token reserve must still fit under
// the window the embedder actually truncates to — otherwise the model's
// silent right-truncation would eat real chunk content, and the stored text
// would no longer match the vector inserted alongside it (the exact failure
// mode `CHUNK_MAX_TOKENS`'s own doc comment above describes). Catching the
// mismatch at compile time means changing either constant on its own trips
// this assert immediately, instead of surfacing later as a quietly
// truncated embedding.
const _: () = assert!(
    CHUNK_MAX_TOKENS + crate::memory::embedder::SPECIAL_TOKEN_RESERVE
        <= crate::memory::embedder::EMBED_MAX_TOKENS
);

/// Which chunker the worker dispatches to for this event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChunkerKind {
    Chat,
    /// Never returned by `select_chunker` today: every event reaches the worker
    /// through `push_and_persist`, which carries chat turns, and a chat turn is
    /// never routed here. Reaching this variant needs a producer that ingests
    /// files — a watcher emitting `MemorySource::CodeFile` — which does not
    /// exist. The worker's match arm is kept so that producer can be added
    /// without reopening the worker.
    #[allow(dead_code)]
    Code,
    /// Worker drops the event without inserting.
    Skip,
}

/// One unit of content that becomes ONE row in `memory` + one in `memory_vectors`.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub text: String,
    /// Approximate token count (target: `CHUNK_MAX_TOKENS`). Set by the tokenizer path;
    /// 0 in byte-fallback mode. Nothing reads it yet — it is carried so a
    /// re-ranking or telemetry consumer does not have to re-tokenize text the
    /// splitter already counted.
    #[allow(dead_code)]
    pub token_count: usize,
    /// Optional header describing context (e.g. "fn foo (rust)") — currently
    /// populated by code.rs. None for chat chunks.
    pub header: Option<String>,
}

/// Error type for chunking operations.
///
/// Unused at runtime: the only chunker the worker reaches is the chat one,
/// which cannot fail — it splits on speaker turns, never parses. These
/// variants describe failures of the code path (tree-sitter parse, tokenizer
/// init), which no producer exercises yet.
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
        // A system prompt is instructions the agent was given, not something
        // that happened — storing it would put the harness's own configuration
        // into the corpus and retrieve it back as if it were a memory.
        Role::System => ChunkerKind::Skip,
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

    // TokenSplitter's own behavior is covered in token_splitter.rs; these two
    // stay here as a cap-respecting smoke test wired through the real constants.
    #[test]
    fn token_split_respects_cap() {
        use crate::memory::chunker::token_splitter::TokenSplitter;
        let splitter = TokenSplitter::fallback();
        let long_text = "a".repeat(10_000);
        let max_bytes = CHUNK_MAX_TOKENS * 4; // 1920
        let segments = splitter.split(&long_text, CHUNK_MAX_TOKENS, CHUNK_OVERLAP_TOKENS);
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
        let segments = splitter.split("hello world", CHUNK_MAX_TOKENS, CHUNK_OVERLAP_TOKENS);
        assert_eq!(segments.len(), 1, "short input should produce exactly 1 segment");
    }

    #[test]
    fn chunk_cap_and_overlap_constants_hold_the_contract() {
        assert_eq!(CHUNK_MAX_TOKENS, 480);
        assert_eq!(CHUNK_OVERLAP_TOKENS, 64);
        assert!(CHUNK_OVERLAP_TOKENS < CHUNK_MAX_TOKENS, "overlap must stay below max or stride collapses to 0");
    }
}
