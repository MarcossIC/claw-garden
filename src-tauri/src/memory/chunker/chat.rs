//! Turn-based chat chunker (Phase 3 — REQ-CHK-003).
//!
//! Phase 3 contract (LOCKED D6, D7):
//!   - Speaker boundary is the OUTER hard rule (each call here is ONE turn).
//!   - 480-token cap (under the 512-token model limit, safety margin).
//!   - NO overlap.
//!   - NO markdown-fence / AST enrichment (deferred to Phase 4).

use std::path::Path;
use std::sync::Arc;

use super::token_splitter::TokenSplitter;
use super::Chunk;

/// Turn-based chat chunker. One call per `MemoryEvent` from the worker.
pub(crate) struct ChatChunker {
    splitter: TokenSplitter,
}

// Compile-time Send + Sync proof.
const _: fn() = || {
    fn _send<T: Send>() {}
    fn _sync<T: Sync>() {}
    _send::<ChatChunker>();
    _sync::<ChatChunker>();
};

impl ChatChunker {
    /// Build a chunker by loading the tokenizer from the Phase 2 cache.
    /// Falls back to byte-length splitter if the file is absent (R-NEW-2 mitigation).
    pub fn new_from_base_dir(base_dir: &Path) -> Self {
        let path = crate::memory::model_download::tokenizer_path(base_dir);
        match tokenizers::Tokenizer::from_file(&path) {
            Ok(tok) => Self {
                splitter: TokenSplitter::with_tokenizer(Arc::new(tok)),
            },
            Err(e) => {
                log::warn!(
                    "ChatChunker: tokenizer not loadable at {} ({e}); \
                     falling back to byte-length splitter",
                    path.display()
                );
                Self { splitter: TokenSplitter::fallback() }
            }
        }
    }

    /// Test-only — pre-wired tokenizer.
    #[cfg(test)]
    pub fn with_tokenizer(tok: Arc<tokenizers::Tokenizer>) -> Self {
        Self { splitter: TokenSplitter::with_tokenizer(tok) }
    }

    /// Test-only — force fallback path.
    #[cfg(test)]
    pub fn with_fallback() -> Self {
        Self { splitter: TokenSplitter::fallback() }
    }

    /// Chunk a single turn's content. The speaker boundary is implicit:
    /// the caller (worker) invokes this once per `MemoryEvent`.
    pub fn chunk(&self, content: &str, max_tokens: usize) -> Vec<Chunk> {
        if content.is_empty() {
            return vec![];
        }
        self.splitter.split(content, max_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaker_boundary_respected_single_turn() {
        let chunker = ChatChunker::with_fallback();
        let chunks = chunker.chunk("hello world", 480);
        assert_eq!(chunks.len(), 1, "short input should produce exactly 1 chunk");
        // byte heuristic: 480*4 = 1920 bytes cap; "hello world" is 11 bytes
        assert!(chunks[0].text.len() <= 1920);
    }

    #[test]
    fn token_cap_480_produces_multiple_chunks() {
        let chunker = ChatChunker::with_fallback();
        // ~6000 bytes >> 1920 (480 * 4) → must split into >= 2
        let content = "word ".repeat(1200); // 6000 bytes
        let chunks = chunker.chunk(&content, 480);
        assert!(chunks.len() >= 2, "expected >= 2 chunks, got {}", chunks.len());
        for chunk in &chunks {
            assert!(
                chunk.text.len() <= 1920,
                "chunk exceeds max_bytes: {} > 1920",
                chunk.text.len()
            );
        }
    }

    #[test]
    fn no_overlap_adjacent_chunks() {
        let chunker = ChatChunker::with_fallback();
        let content: String = "word ".repeat(1200);
        let chunks = chunker.chunk(&content, 480);
        let concatenated: String = chunks.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            concatenated, content,
            "concatenated chunks must equal original input (no gaps, no overlaps)"
        );
    }

    #[test]
    fn empty_content_returns_empty_vec() {
        let chunker = ChatChunker::with_fallback();
        let chunks = chunker.chunk("", 480);
        assert!(chunks.is_empty(), "empty input must produce empty vec");
    }

    #[test]
    fn tokenizer_absent_falls_back_to_byte_heuristic() {
        // /nonexistent/path cannot contain tokenizer.json
        let chunker = ChatChunker::new_from_base_dir(std::path::Path::new("/nonexistent/path"));
        let chunks = chunker.chunk("hello", 480);
        assert!(!chunks.is_empty(), "fallback chunker should produce at least 1 chunk");
    }

    #[test]
    fn exactly_480_tokens_is_single_chunk() {
        let chunker = ChatChunker::with_fallback();
        // In byte fallback: 480 * 4 = 1920 bytes = single chunk
        let content = "x".repeat(1920);
        let chunks = chunker.chunk(&content, 480);
        assert_eq!(chunks.len(), 1, "exactly-cap-size input should be 1 chunk: got {}", chunks.len());
    }
}
