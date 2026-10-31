//! Splitting source code into passages along its syntax tree (cAST): the tree
//! is split from the top down, and small neighbouring nodes are joined until
//! a passage would pass the cap, so a definition is cut only when it alone
//! does not fit.

use super::chunk::{breadcrumb_of, emit, Passage};
use crate::memory::chunker::token_splitter::TokenSplitter;

/// Largest passage, in characters that are not whitespace, so the cap does
/// not depend on how the code is formatted. Below the cAST paper's 2,000:
/// e5-small reads 512 tokens and code cuts into short tokens. Every passage
/// still goes through the token cap afterwards.
pub const CODE_CHUNK_MAX_CHARS: usize = 1_200;

/// Longest label a container adds to the breadcrumb, in bytes.
const MAX_LABEL_BYTES: usize = 120;

/// Nodes that are the body of the container above them: descending into one
/// adds no label, the container already named it.
const BODIES: &[&str] = &[
    "block",
    "declaration_list",
    "field_declaration_list",
    "enum_variant_list",
    "class_body",
    "statement_block",
    "interface_body",
    "object_type",
    "enum_body",
    "decorated_definition",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeLang {
    Rust,
    Python,
    TypeScript,
    Tsx,
}

impl CodeLang {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "rs" => Some(Self::Rust),
            "py" => Some(Self::Python),
            "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            _ => None,
        }
    }

    fn grammar(self) -> tree_sitter::Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }
}

/// The passages of the source file at `path`.
pub(crate) fn chunk_code(path: &str, text: &str, lang: CodeLang, splitter: &TokenSplitter) -> Vec<Passage> {
    let mut out = Vec::new();
    let Some(tree) = parse(text, lang) else {
        emit(&mut out, &breadcrumb_of(path, &[]), text, splitter);
        return out;
    };
    let mut packer = Packer::default();
    split(&children(tree.root_node()), text, &mut Vec::new(), None, &mut packer);
    packer.flush();
    for piece in packer.pieces {
        let body = &text[piece.start..piece.end];
        if body.chars().any(char::is_alphanumeric) {
            emit(&mut out, &breadcrumb_of(path, &piece.labels), body, splitter);
        }
    }
    out
}

fn parse(text: &str, lang: CodeLang) -> Option<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang.grammar()).ok()?;
    parser.parse(text, None)
}

/// A passage in the making, by byte range. Its labels are those of its
/// largest node, the one most of its text belongs to.
struct Piece {
    start: usize,
    end: usize,
    labels: Vec<String>,
}

#[derive(Default)]
struct Packer {
    pieces: Vec<Piece>,
    run: Option<Piece>,
    size: usize,
    largest: usize,
}

impl Packer {
    fn add(&mut self, start: usize, end: usize, size: usize, labels: &[String]) {
        if self.run.is_some() && self.size + size > CODE_CHUNK_MAX_CHARS {
            self.flush();
        }
        let run = self.run.get_or_insert_with(|| Piece { start, end, labels: labels.to_vec() });
        run.end = end;
        if size > self.largest {
            run.labels = labels.to_vec();
            self.largest = size;
        }
        self.size += size;
    }

    fn flush(&mut self) {
        if let Some(piece) = self.run.take() {
            self.pieces.push(piece);
        }
        self.size = 0;
        self.largest = 0;
    }
}

pub(crate) fn non_whitespace_len(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}

fn children(node: tree_sitter::Node<'_>) -> Vec<tree_sitter::Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

/// Siblings that fit are packed in order. A node that does not fit is split
/// by its own children, and whatever small nodes came right before it (a doc
/// comment, a `class Foo:` line) open its first passage; its last passage
/// ends with it, so no passage straddles the end of a large container.
/// `labelled_row` is the line of the last container that added a label, so
/// a node starting on that same line (`export` around `class`, an `impl`'s
/// body) does not repeat it.
fn split(
    nodes: &[tree_sitter::Node<'_>],
    text: &str,
    labels: &mut Vec<String>,
    labelled_row: Option<usize>,
    packer: &mut Packer,
) {
    for node in nodes {
        let (start, end) = (node.start_byte(), node.end_byte());
        let size = non_whitespace_len(&text[start..end]);
        if size <= CODE_CHUNK_MAX_CHARS {
            packer.add(start, end, size, labels);
            continue;
        }
        let inner = children(*node);
        if inner.is_empty() {
            packer.flush();
            packer.add(start, end, size, labels);
            packer.flush();
            continue;
        }
        let row = node.start_position().row;
        let label = (!BODIES.contains(&node.kind()) && labelled_row != Some(row))
            .then(|| header(&text[start..end]))
            .flatten();
        match label {
            Some(label) => {
                labels.push(label);
                split(&inner, text, labels, Some(row), packer);
                labels.pop();
            }
            None => split(&inner, text, labels, labelled_row, packer),
        }
        packer.flush();
    }
}

/// A container's first line without its body: what opens at `{`, or the `:`
/// that ends a Python header.
fn header(node_text: &str) -> Option<String> {
    let line = node_text.lines().next()?;
    let line = line.split('{').next().unwrap_or(line).trim();
    let line = line.strip_suffix(':').unwrap_or(line).trim_end();
    if line.is_empty() {
        return None;
    }
    let mut cut = line.len().min(MAX_LABEL_BYTES);
    while !line.is_char_boundary(cut) {
        cut -= 1;
    }
    Some(line[..cut].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::chunker::CHUNK_MAX_TOKENS;

    fn chunks(path: &str, text: &str, lang: CodeLang) -> Vec<Passage> {
        chunk_code(path, text, lang, &TokenSplitter::fallback())
    }

    /// A body of `n` statements, one per line.
    fn statements(n: usize, indent: &str, line: impl Fn(usize) -> String) -> String {
        (0..n).map(|i| format!("{indent}{}\n", line(i))).collect()
    }

    #[test]
    fn non_whitespace_size_ignores_formatting() {
        assert_eq!(non_whitespace_len("fn a ( ) {\n\t}\n"), non_whitespace_len("fn a(){}"));
        assert_eq!(non_whitespace_len("fn a(){}"), 7);
    }

    #[test]
    fn small_siblings_merge_and_keep_their_doc_comments() {
        let text = "use std::fmt;\n\n/// Adds one.\nfn add_one(x: i32) -> i32 {\n    x + 1\n}\n\nconst LIMIT: usize = 3;\n";

        let passages = chunks("src/lib.rs", text, CodeLang::Rust);

        assert_eq!(passages.len(), 1, "{passages:#?}");
        assert_eq!(passages[0].breadcrumb, "src/lib.rs");
        assert_eq!(passages[0].body, text.trim());
    }

    #[test]
    fn a_node_over_the_cap_is_split_by_its_children_under_its_header() {
        let body = |name: &str| {
            format!("    fn {name}(&self) {{\n{}    }}\n", statements(60, "        ", |i| format!("let value_{i} = {i};")))
        };
        let text = format!("/// The store.\nimpl Store {{\n{}\n{}}}\n", body("first"), body("second"));

        let passages = chunks("src/lib.rs", &text, CodeLang::Rust);

        assert_eq!(passages.len(), 2, "{passages:#?}");
        for p in &passages {
            assert_eq!(p.breadcrumb, "src/lib.rs > impl Store");
        }
        assert!(passages[0].body.starts_with("/// The store.\nimpl Store {\n    fn first(&self)"), "{}", passages[0].body);
        assert!(passages[0].body.ends_with("let value_59 = 59;\n    }"), "the first method ends whole in its passage");
        assert!(passages[1].body.starts_with("fn second(&self)"), "{}", passages[1].body);
        assert!(passages[1].body.ends_with("    }\n}"));
    }

    #[test]
    fn python_and_typescript_classes_split_by_method() {
        let py_method = |name: &str| {
            format!("    def {name}(self):\n{}", statements(70, "        ", |i| format!("value_{i} = {i}")))
        };
        let py = format!("class Greeter(Base):\n{}\n{}", py_method("one"), py_method("two"));
        let ts_method = |name: &str| {
            format!("  {name}(): void {{\n{}  }}\n", statements(70, "    ", |i| format!("const v{i} = {i};")))
        };
        let ts = format!("export class Greeter {{\n{}\n{}}}\n", ts_method("one"), ts_method("two"));

        let py_passages = chunks("app/greet.py", &py, CodeLang::Python);
        let ts_passages = chunks("web/greet.ts", &ts, CodeLang::TypeScript);

        let crumbs = |ps: &[Passage]| ps.iter().map(|p| p.breadcrumb.clone()).collect::<Vec<_>>();
        assert_eq!(crumbs(&py_passages), ["app/greet.py > class Greeter(Base)"; 2]);
        assert!(py_passages[0].body.starts_with("class Greeter(Base):\n    def one(self):"));
        assert!(py_passages[1].body.starts_with("def two(self):"));
        assert_eq!(crumbs(&ts_passages), ["web/greet.ts > export class Greeter"; 2], "`export` and `class` share a line and one label");
        assert!(ts_passages[1].body.starts_with("two(): void"));
    }

    #[test]
    fn a_leaf_over_the_cap_falls_back_to_the_token_splitter() {
        let text = format!("# {}\n", "word ".repeat(1_000));

        let passages = chunks("notes.py", &text, CodeLang::Python);

        assert!(passages.len() > 1, "{}", passages.len());
        assert!(passages.iter().all(|p| p.breadcrumb == "notes.py"));
    }

    #[test]
    fn every_passage_fits_the_token_cap() {
        let text = include_str!("mod.rs");
        let splitter = TokenSplitter::fallback();

        let passages = chunk_code("project_docs/mod.rs", text, CodeLang::Rust, &splitter);

        assert!(passages.len() > 5, "{}", passages.len());
        for p in &passages {
            assert!(splitter.count(&p.indexed_text()) <= CHUNK_MAX_TOKENS, "{}", p.breadcrumb);
        }
        // A definition over the cap is split by the splitter, whose cut may
        // fall inside its signature; each piece still sits under its header.
        for name in ["fn plan_refresh", "fn needs_refresh", "fn fuse", "pub async fn search"] {
            assert!(passages.iter().any(|p| p.body.contains(name) || p.breadcrumb.contains(name)), "{name} is in no passage");
        }
    }

    #[test]
    fn an_empty_file_has_no_passages() {
        assert!(chunks("a.rs", "", CodeLang::Rust).is_empty());
        assert!(chunks("a.py", "\n\n  \n", CodeLang::Python).is_empty());
    }
}
