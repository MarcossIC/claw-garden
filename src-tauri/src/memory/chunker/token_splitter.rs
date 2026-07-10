//! Token-cap splitting helper (Phase 3 — REQ-CHK-005).
//!
//! Knows NOTHING about chat speakers OR AST. Both `ChatChunker` (after speaker
//! boundary) and `CodeChunker` (degraded fallback for unknown languages or parse
//! failures) feed text through here. Decouples the cap policy from upstream
//! semantic split (R6 mitigation in proposal).

use std::sync::Arc;

use super::Chunk;

/// Pure "split this text by token cap, no overlap" helper.
pub(crate) struct TokenSplitter {
    tokenizer: Option<Arc<tokenizers::Tokenizer>>,
}

impl TokenSplitter {
    /// Real tokenizer-backed splitter (preferred, post-bootstrap).
    pub fn with_tokenizer(tokenizer: Arc<tokenizers::Tokenizer>) -> Self {
        Self { tokenizer: Some(tokenizer) }
    }

    /// Byte-length fallback when `tokenizer.json` is not yet on disk.
    /// 1 token ≈ 4 bytes for the languages we support.
    pub fn fallback() -> Self {
        Self { tokenizer: None }
    }

    /// Split `text` so each output `Chunk` has `<= max_tokens` tokens.
    /// No overlap. Preserves UTF-8 boundaries.
    pub fn split(&self, text: &str, max_tokens: usize) -> Vec<Chunk> {
        match &self.tokenizer {
            Some(tok) => self.split_by_tokens(text, max_tokens, tok),
            None => self.split_by_bytes(text, max_tokens.saturating_mul(4)),
        }
    }

    fn split_by_tokens(&self, text: &str, max: usize, tok: &tokenizers::Tokenizer) -> Vec<Chunk> {
        let enc = match tok.encode(text, false) {
            Ok(e) => e,
            Err(_) => return self.split_by_bytes(text, max.saturating_mul(4)),
        };
        let offsets = enc.get_offsets();
        if offsets.is_empty() {
            return vec![];
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i < offsets.len() {
            let end_idx = (i + max).min(offsets.len()) - 1;
            let start_byte = offsets[i].0;
            let end_byte = offsets[end_idx].1;
            // Safe: tokenizers guarantees offsets land on char boundaries.
            let slice = &text[start_byte..end_byte];
            out.push(Chunk {
                text: slice.to_string(),
                token_count: end_idx - i + 1,
                header: None,
            });
            i = end_idx + 1;
        }
        out
    }

    fn split_by_bytes(&self, text: &str, max_bytes: usize) -> Vec<Chunk> {
        if max_bytes == 0 {
            return vec![];
        }
        let mut out = Vec::new();
        let mut acc = String::new();
        for ch in text.chars() {
            if acc.len() + ch.len_utf8() > max_bytes && !acc.is_empty() {
                out.push(Chunk {
                    text: std::mem::take(&mut acc),
                    token_count: 0, // unknown in fallback mode
                    header: None,
                });
            }
            acc.push(ch);
        }
        if !acc.is_empty() {
            out.push(Chunk { text: acc, token_count: 0, header: None });
        }
        out
    }
}
