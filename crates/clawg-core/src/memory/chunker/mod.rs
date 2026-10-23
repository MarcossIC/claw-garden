//! Chunker selector + shared types.

pub(crate) mod chat;
pub(crate) mod token_splitter;

pub use chat::{ChatChunker, SaveSplit};

use std::borrow::Cow;

/// A tokenizer with no vocabulary but `[UNK]`, over a whitespace
/// pre-tokenizer, so each word is one token: what a test puts where the
/// model's tokenizer goes, to split by tokens without the model.
#[cfg(any(test, feature = "testing"))]
pub fn word_tokenizer() -> std::sync::Arc<tokenizers::Tokenizer> {
    let json = r#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,
        "pre_tokenizer":{"type":"Whitespace"},"post_processor":null,"decoder":null,
        "model":{"type":"WordLevel","vocab":{"[UNK]":0},"unk_token":"[UNK]"}}"#;
    std::sync::Arc::new(json.parse().expect("the word tokenizer parses"))
}

use crate::models::chat::Role;

/// Upper bound on a chunk's token count. Stays under the embedder's 512-token
/// model limit with a safety margin (the embedder truncates silently past that,
/// and a truncated chunk's stored text would no longer match its own vector).
pub(crate) const CHUNK_MAX_TOKENS: usize = 480;

/// How many trailing tokens of one chunk are repeated as the leading tokens
/// of the next. A hard cut loses any fact that straddles the boundary — no
/// single vector represents it, so it is unretrievable. 64 (~13% of the cap)
/// buys back that coverage at a proportional embedding-cost increase.
pub(crate) const CHUNK_OVERLAP_TOKENS: usize = 64;

/// Leading tokens of a save that go into the embedder with each piece after
/// the first. A piece cut from the middle of a long text loses whatever the
/// text named at its start (the crate, the decision, the bug), and so does
/// its vector; the head puts that topic back. 48 tokens is a sentence or two:
/// enough to carry a subject line, small enough to leave most of the window
/// to the piece's own text.
pub(crate) const SAVE_HEAD_TOKENS: usize = 48;

/// Room for the separator, and for the tokens a tokenizer can add where two
/// texts are joined: token counts are not additive across the seam.
const HEAD_JOIN_RESERVE: usize = 8;

/// Cap of each piece of a text that does not fit in one. Far below what the
/// embedder takes: a vector averages its whole text, and a fact that is one
/// sentence among 424 tokens loses to short memories on the same topic, while
/// the same sentence in 96 tokens behind the head ranks first.
pub(crate) const PIECE_MAX_TOKENS: usize = 96;

/// Tokens two neighbouring pieces share. A quarter of the piece, so a sentence
/// cut at the boundary is whole in one of them; the 64 of a chunk would make
/// every piece two thirds repeat.
pub(crate) const PIECE_OVERLAP_TOKENS: usize = 24;

const HEAD_SEPARATOR: &str = "\n…\n";

const _: () = assert!(PIECE_OVERLAP_TOKENS < PIECE_MAX_TOKENS);
const _: () = assert!(SAVE_HEAD_TOKENS + HEAD_JOIN_RESERVE + PIECE_MAX_TOKENS <= CHUNK_MAX_TOKENS);

/// The text a piece is embedded with: the piece alone, or the save's head,
/// the separator and the piece when it was stored with a context. The worker
/// and the re-embed both build it here, so a re-embedded piece gets the same
/// vector it was saved with.
pub(crate) fn embedding_text<'a>(context: &str, piece: &'a str) -> Cow<'a, str> {
    if context.is_empty() {
        Cow::Borrowed(piece)
    } else {
        Cow::Owned(format!("{context}{HEAD_SEPARATOR}{piece}"))
    }
}

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
    /// Worker drops the event without inserting.
    Skip,
}

/// One unit of content that becomes ONE row in `memory` + one in `memory_vectors`.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub text: String,
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
    fn a_piece_without_context_is_embedded_as_stored() {
        assert_eq!(embedding_text("", "the piece"), "the piece");
    }

    #[test]
    fn a_piece_with_context_is_embedded_after_the_head() {
        assert_eq!(embedding_text("Notes on clawg-ledger", "the piece"), "Notes on clawg-ledger\n…\nthe piece");
    }

    #[test]
    fn chunk_cap_and_overlap_constants_hold_the_contract() {
        assert_eq!(CHUNK_MAX_TOKENS, 480);
        assert_eq!(CHUNK_OVERLAP_TOKENS, 64);
        assert!(CHUNK_OVERLAP_TOKENS < CHUNK_MAX_TOKENS, "overlap must stay below max or stride collapses to 0");
    }
}
