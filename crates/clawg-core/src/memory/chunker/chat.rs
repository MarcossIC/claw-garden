//! Turn-based chat chunker.
//!
//! Speaker boundary is the outer hard rule, deliberately: each call here is
//! ONE turn, and letting a chunk cross turns would mix two speakers' voices
//! into a single vector. Inside a turn, content is capped at
//! `CHUNK_MAX_TOKENS` (headroom under the embedder's 512-token model limit)
//! with `CHUNK_OVERLAP_TOKENS` of repeated content between adjacent chunks,
//! so a fact spanning a cut point still lands whole in at least one chunk.
//! No Markdown awareness: splitting by heading is the documentation index's
//! job (`memory::project_docs::chunk`), not a chat turn's.

use std::path::Path;
use std::sync::Arc;

use once_cell::sync::OnceCell;

use super::token_splitter::TokenSplitter;
use super::{Chunk, CHUNK_MAX_TOKENS, PIECE_MAX_TOKENS, PIECE_OVERLAP_TOKENS, SAVE_HEAD_TOKENS};

/// How the worker stores one event: its pieces in order and, for a text that
/// did not fit in one, the head each piece after the first is embedded with.
#[derive(Debug, Clone)]
pub struct SaveSplit {
    pub pieces: Vec<Chunk>,
    pub head: Option<String>,
}

/// Turn-based chat chunker. One call per `MemoryEvent` from the worker.
pub struct ChatChunker {
    source: Source,
}

enum Source {
    Fixed(TokenSplitter),
    /// Read on every call: the worker's chunker is built before a new
    /// install's first model download lands, and a splitter fixed at that
    /// moment would cut by bytes for the rest of the process.
    Cell(Arc<OnceCell<Arc<tokenizers::Tokenizer>>>),
}

// Compile-time Send + Sync proof.
const _: fn() = || {
    fn _send<T: Send>() {}
    fn _sync<T: Sync>() {}
    _send::<ChatChunker>();
    _sync::<ChatChunker>();
};

impl ChatChunker {
    /// Build a chunker from an already-resolved tokenizer — `Some` uses it,
    /// `None` falls back to the byte-length splitter. Pulled out of
    /// `new_from_base_dir` so a caller that already loaded (or shares) a
    /// tokenizer via `model_download::load_tokenizer` never pays for a
    /// second parse of the same ~17 MB file.
    pub fn from_tokenizer(tokenizer: Option<Arc<tokenizers::Tokenizer>>) -> Self {
        match tokenizer {
            Some(tok) => Self { source: Source::Fixed(TokenSplitter::with_tokenizer(tok)) },
            None => Self { source: Source::Fixed(TokenSplitter::fallback()) },
        }
    }

    /// A chunker that splits by the tokenizer in `cell` once it is filled, and
    /// by byte length until then.
    pub fn from_cell(cell: Arc<OnceCell<Arc<tokenizers::Tokenizer>>>) -> Self {
        Self { source: Source::Cell(cell) }
    }

    fn splitter(&self) -> TokenSplitter {
        match &self.source {
            Source::Fixed(splitter) => splitter.clone(),
            Source::Cell(cell) => match cell.get() {
                Some(tok) => TokenSplitter::with_tokenizer(tok.clone()),
                None => TokenSplitter::fallback(),
            },
        }
    }

    /// Build a chunker by loading the tokenizer from the model download cache.
    /// Falls back to the byte-length splitter if the file is absent — the
    /// tokenizer download can fail or still be in flight, and chunking must
    /// keep working (with a coarser cap estimate) rather than block ingestion.
    pub fn new_from_base_dir(base_dir: &Path) -> Self {
        Self::from_tokenizer(crate::memory::model_download::load_tokenizer(base_dir))
    }

    /// Test-only — pre-wired tokenizer.
    #[cfg(test)]
    pub fn with_tokenizer(tok: Arc<tokenizers::Tokenizer>) -> Self {
        Self { source: Source::Fixed(TokenSplitter::with_tokenizer(tok)) }
    }

    /// Test-only — force fallback path.
    #[cfg(test)]
    pub fn with_fallback() -> Self {
        Self { source: Source::Fixed(TokenSplitter::fallback()) }
    }

    /// Chunk a single turn's content. The speaker boundary is implicit:
    /// the caller (worker) invokes this once per `MemoryEvent`. Adjacent
    /// chunks share `CHUNK_OVERLAP_TOKENS` tokens so a fact that would
    /// otherwise straddle a hard cut stays whole in at least one of them.
    pub fn chunk(&self, content: &str, max_tokens: usize) -> Vec<Chunk> {
        chunk_with(&self.splitter(), content, max_tokens)
    }

    /// Splits one event's text for storing. A text that fits in one piece of
    /// [`CHUNK_MAX_TOKENS`] comes back exactly as [`ChatChunker::chunk`] cuts
    /// it, with no head. A longer one is cut into pieces of
    /// [`PIECE_MAX_TOKENS`] sharing [`PIECE_OVERLAP_TOKENS`], each later one
    /// embedded behind the first [`SAVE_HEAD_TOKENS`] of the text.
    ///
    /// One splitter cuts all three, so a tokenizer that loads in between
    /// cannot leave a head cut by bytes over pieces cut by tokens.
    pub fn split_save(&self, content: &str) -> SaveSplit {
        let splitter = self.splitter();
        let whole = chunk_with(&splitter, content, CHUNK_MAX_TOKENS);
        if whole.len() <= 1 {
            return SaveSplit { pieces: whole, head: None };
        }
        let head = splitter.split(content, SAVE_HEAD_TOKENS, 0).into_iter().next().map(|c| c.text);
        SaveSplit { pieces: splitter.split(content, PIECE_MAX_TOKENS, PIECE_OVERLAP_TOKENS), head }
    }

    /// Test-only: see `TokenSplitter::tokenizer_ptr`.
    #[cfg(test)]
    pub(crate) fn tokenizer_ptr(&self) -> Option<*const tokenizers::Tokenizer> {
        self.splitter().tokenizer_ptr()
    }
}

fn chunk_with(splitter: &TokenSplitter, content: &str, max_tokens: usize) -> Vec<Chunk> {
    if content.is_empty() {
        return vec![];
    }
    splitter.split(content, max_tokens, super::CHUNK_OVERLAP_TOKENS)
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
    fn adjacent_chunks_share_exactly_overlap() {
        let chunker = ChatChunker::with_fallback();
        let content: String = "word ".repeat(1200); // 6000 bytes
        let chunks = chunker.chunk(&content, 480);
        // 6000 bytes at cap 1920 / overlap 256 (stride 1664) is
        // ceil((6000 - 256) / 1664) = 4 windows. Pinning the count and not
        // just `>= 2` is what makes a stride change visible here.
        assert_eq!(chunks.len(), 4, "expected 4 chunks, got {}", chunks.len());
        for pair in chunks.windows(2) {
            let tail = &pair[0].text[pair[0].text.len() - 256..];
            let head = &pair[1].text[..256];
            assert_eq!(tail, head, "adjacent chunks must share exactly 256 overlap bytes");
        }
    }

    #[test]
    fn chunks_reconstruct_input_when_overlap_removed() {
        let chunker = ChatChunker::with_fallback();
        let content: String = "word ".repeat(1200);
        let chunks = chunker.chunk(&content, 480);
        let mut reconstructed = chunks[0].text.clone();
        for c in &chunks[1..] {
            reconstructed.push_str(&c.text[256..]);
        }
        assert_eq!(
            reconstructed, content,
            "removing each chunk's shared overlap must reconstruct the original input exactly (no gaps, no data loss)"
        );
    }

    #[test]
    fn a_text_that_fits_one_piece_is_split_as_before() {
        let chunker = ChatChunker::with_fallback();
        for content in ["hello world".to_string(), "x".repeat(1920)] {
            let split = chunker.split_save(&content);
            assert_eq!(split.pieces.len(), 1);
            assert_eq!(split.pieces[0].text, chunker.chunk(&content, 480)[0].text);
            assert_eq!(split.head, None);
        }
    }

    #[test]
    fn a_long_text_is_split_into_small_pieces() {
        let chunker = ChatChunker::with_fallback();
        let content = "word ".repeat(1200);
        let split = chunker.split_save(&content);
        // 6000 bytes at 96 tokens (384 bytes) with 96 bytes of overlap is
        // ceil((6000 - 96) / 288) = 21 pieces, where the 480 cap gives 4.
        assert_eq!(split.pieces.len(), 21);
        assert!(split.pieces.iter().all(|p| p.text.len() <= 384), "a piece passed the cap");
    }

    #[test]
    fn small_pieces_share_their_own_overlap() {
        let chunker = ChatChunker::with_fallback();
        let content = "word ".repeat(1200);
        let split = chunker.split_save(&content);
        let rebuilt = split.pieces[1..].iter().fold(split.pieces[0].text.clone(), |acc, p| acc + &p.text[96..]);
        assert_eq!(rebuilt, content, "neighbours share 24 tokens, not the 64 of a chunk");
    }

    #[test]
    fn the_head_is_the_first_tokens_of_the_text() {
        let chunker = ChatChunker::with_fallback();
        let content = format!("Notes on clawg-ledger, the crate that settles balances. {}", "word ".repeat(1200));
        let head = chunker.split_save(&content).head.expect("a long text has a head");
        assert_eq!(head, content[..48 * 4]);
        assert!(head.contains("clawg-ledger"));
    }

    #[test]
    fn the_head_ends_at_a_word_boundary() {
        use crate::memory::chunker::token_splitter::tests::{subword_tokenizer, subword_words};
        // Words of two and three tokens in turn: 480 of them are 1200 tokens,
        // and token 48 is the second of the twentieth word, which opens at 47.
        let words = subword_words(480);
        let content = words.join(" ");
        let tok = subword_tokenizer();
        let offsets = tok.encode(content.as_str(), false).expect("the fixture encodes").get_offsets().to_vec();
        assert_eq!(offsets.len(), 1200);
        assert_eq!((offsets[47].0, offsets[48].0), (content.find(&words[19]).unwrap(), offsets[47].1), "token 48 is inside a word");

        let head = ChatChunker::with_tokenizer(tok.clone()).split_save(&content).head.expect("a long text has a head");
        assert_eq!(head, words[..19].join(" "), "the head stops before the word it would cut");
        assert!(TokenSplitter::with_tokenizer(tok).count(&head) <= SAVE_HEAD_TOKENS);
    }

    #[test]
    fn no_piece_of_a_long_save_starts_inside_a_word() {
        use crate::memory::chunker::token_splitter::tests::{assert_cut_at_word_boundaries, subword_tokenizer, subword_words};
        let content = subword_words(480).join(" ");
        let tok = subword_tokenizer();
        let split = ChatChunker::with_tokenizer(tok.clone()).split_save(&content);
        let pieces: Vec<&str> = split.pieces.iter().map(|c| c.text.as_str()).collect();
        assert!(pieces.len() > 10, "a long save, {} pieces", pieces.len());
        assert_cut_at_word_boundaries(&content, &pieces);
        let splitter = TokenSplitter::with_tokenizer(tok);
        assert!(pieces.iter().all(|p| splitter.count(p) <= PIECE_MAX_TOKENS), "a piece passed the cap");
    }

    #[test]
    fn empty_content_returns_empty_vec() {
        let chunker = ChatChunker::with_fallback();
        let chunks = chunker.chunk("", 480);
        assert!(chunks.is_empty(), "empty input must produce empty vec");
    }

    #[test]
    fn tokenizer_absent_falls_back_to_byte_heuristic() {
        // from_tokenizer(None) is what new_from_base_dir reduces to once the
        // tokenizer file cannot be loaded — tested directly here rather than
        // through a nonexistent path, so this test exercises the exact same
        // fallback branch a caller with a pre-resolved `None` from
        // `model_download::load_tokenizer` takes.
        let chunker = ChatChunker::from_tokenizer(None);
        let chunks = chunker.chunk("hello", 480);
        assert!(!chunks.is_empty(), "fallback chunker should produce at least 1 chunk");
    }

    #[test]
    fn new_from_base_dir_still_falls_back_when_the_file_is_absent() {
        // Regression coverage for the thin wrapper itself: new_from_base_dir
        // must still degrade gracefully end to end, not just its delegate.
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
