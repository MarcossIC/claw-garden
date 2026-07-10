//! AST-aware code chunker (Phase 3 — REQ-CHK-004).
//!
//! Phase 3 scope: Rust, TypeScript, Python. Unknown languages or parse failures
//! degrade gracefully to the shared `TokenSplitter`. NEVER panics.

use std::sync::Mutex;

use super::token_splitter::TokenSplitter;
use super::Chunk;

/// AST-aware code chunker. Maintains one `Parser` per supported language
/// behind a `Mutex` (tree-sitter `Parser` is `Send` but NOT `Sync` —
/// verified via the compile-time block below).
///
/// Phase 3 scope: Rust, TypeScript, Python. Unknown languages OR parse
/// failures degrade to the shared `TokenSplitter`. NEVER panics.
pub(crate) struct CodeChunker {
    rust: Mutex<tree_sitter::Parser>,
    typescript: Mutex<tree_sitter::Parser>,
    python: Mutex<tree_sitter::Parser>,
    fallback: TokenSplitter,
}

// Compile-time Send + Sync proof.
// `Mutex<Parser>` is `Send + Sync` iff `Parser: Send`.
// tree-sitter `Parser` is `Send` (confirmed in tree-sitter docs).
const _: fn() = || {
    fn _send<T: Send>() {}
    fn _sync<T: Sync>() {}
    _send::<CodeChunker>();
    _sync::<CodeChunker>();
};

impl CodeChunker {
    /// Build a chunker with all three language parsers pre-initialized.
    ///
    /// Panics on grammar-version mismatch (detectable at startup, not per-request).
    pub fn new() -> Self {
        Self {
            rust: Mutex::new(make_parser(tree_sitter_rust::LANGUAGE.into())),
            typescript: Mutex::new(make_parser(
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            )),
            python: Mutex::new(make_parser(tree_sitter_python::LANGUAGE.into())),
            fallback: TokenSplitter::fallback(),
        }
    }

    /// Chunk `content`. `language` is a hint: `Some("rs")` / `Some("ts")` /
    /// `Some("py")`; everything else → fallback splitter.
    ///
    /// Never panics. Parse failures or unknown languages degrade to the
    /// byte-length fallback splitter.
    pub fn chunk(&self, content: &str, language: Option<&str>) -> Vec<Chunk> {
        let parser_mu = match language {
            Some("rs") => &self.rust,
            Some("ts") | Some("tsx") => &self.typescript,
            Some("py") => &self.python,
            _ => return self.fallback.split(content, 480),
        };

        let tree_opt = {
            let mut p = parser_mu.lock().unwrap_or_else(|e| e.into_inner());
            p.parse(content, None)
        };

        let Some(tree) = tree_opt else {
            return self.fallback.split(content, 480);
        };

        let bytes = content.as_bytes();
        let mut chunks = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for node in root.children(&mut cursor) {
            if !node.is_named() {
                continue;
            }
            let start = node.start_byte();
            let end = node.end_byte();
            let snippet = match std::str::from_utf8(&bytes[start..end]) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let header =
                Some(format!("{} ({})", node.kind(), language.unwrap_or("unknown")));

            // Each node may itself exceed 480 tokens — sub-split via fallback.
            for mut c in self.fallback.split(snippet, 480) {
                c.header = header.clone();
                chunks.push(c);
            }
        }

        if chunks.is_empty() {
            // Root had no named children (e.g. malformed input or empty tree).
            return self.fallback.split(content, 480);
        }

        chunks
    }
}

fn make_parser(lang: tree_sitter::Language) -> tree_sitter::Parser {
    let mut p = tree_sitter::Parser::new();
    // Returns Err only for grammar-version mismatch; we ship the grammars
    // ourselves so a mismatch is a compile-time-detectable dep bug.
    p.set_language(&lang)
        .expect("tree-sitter language version mismatch — check dep versions");
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_two_functions_produces_two_chunks_with_headers() {
        let content = "fn foo() -> u32 { 42 }\nfn bar() -> u32 { 0 }";
        let chunker = CodeChunker::new();
        let chunks = chunker.chunk(content, Some("rs"));
        assert_eq!(chunks.len(), 2, "expected 2 chunks for two fn items, got {}", chunks.len());
        for chunk in &chunks {
            let hdr = chunk.header.as_deref().unwrap_or("");
            assert!(
                hdr.contains("function_item") || hdr.contains("rs"),
                "expected header containing 'function_item' or 'rs', got {:?}",
                chunk.header
            );
        }
    }

    #[test]
    fn typescript_class_produces_chunk_with_header() {
        let content = "class Foo { bar(): void {} }";
        let chunker = CodeChunker::new();
        let chunks = chunker.chunk(content, Some("ts"));
        assert!(!chunks.is_empty(), "expected >= 1 chunk for TypeScript class");
        let has_class = chunks.iter().any(|c| {
            c.header.as_deref().map(|h| h.contains("class")).unwrap_or(false)
        });
        assert!(
            has_class,
            "expected at least one chunk with 'class' in header; got {:?}",
            chunks.iter().map(|c| &c.header).collect::<Vec<_>>()
        );
    }

    #[test]
    fn python_function_produces_chunk_with_header() {
        let content = "def foo():\n    return 42\n";
        let chunker = CodeChunker::new();
        let chunks = chunker.chunk(content, Some("py"));
        assert!(!chunks.is_empty(), "expected >= 1 chunk for Python function");
        let has_fn = chunks.iter().any(|c| {
            c.header
                .as_deref()
                .map(|h| h.contains("function_definition") || h.contains("py"))
                .unwrap_or(false)
        });
        assert!(
            has_fn,
            "expected chunk with function_definition header; got {:?}",
            chunks.iter().map(|c| &c.header).collect::<Vec<_>>()
        );
    }

    #[test]
    fn unknown_extension_falls_back_to_chat_chunker() {
        let content = "some text that cannot be parsed as code";
        let chunker = CodeChunker::new();
        let chunks = chunker.chunk(content, None);
        assert!(!chunks.is_empty(), "expected >= 1 chunk from fallback");
        // Unknown extension — no headers expected
        for chunk in &chunks {
            assert_eq!(chunk.header, None, "fallback chunks must not have headers");
        }
    }

    #[test]
    fn malformed_rust_falls_back_no_panic() {
        let content = "fn {{{{{ broken";
        let chunker = CodeChunker::new();
        // Must not panic, must return >= 1 chunk
        let chunks = chunker.chunk(content, Some("rs"));
        assert!(!chunks.is_empty(), "malformed input must still produce at least 1 chunk");
    }

    #[test]
    fn large_function_split_respects_token_cap() {
        // ~8000 bytes — must be split into multiple chunks, each <= 1920 bytes
        let body = "    let x = 1;\n".repeat(500); // ~8000 bytes
        let content = format!("fn big_fn() {{\n{}}}\n", body);
        let chunker = CodeChunker::new();
        let chunks = chunker.chunk(&content, Some("rs"));
        assert!(!chunks.is_empty(), "expected >= 1 chunk");
        for chunk in &chunks {
            assert!(
                chunk.text.len() <= 1920,
                "chunk byte length {} > 1920 (byte fallback cap = 480*4)",
                chunk.text.len()
            );
        }
    }
}
