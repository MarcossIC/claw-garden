//! Splitting a doc into passages: one per Markdown section, each named by its
//! breadcrumb, and a section too long for one vector split with overlap.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::memory::chunker::token_splitter::TokenSplitter;
use crate::memory::chunker::{CHUNK_MAX_TOKENS, CHUNK_OVERLAP_TOKENS};

/// Longest breadcrumb kept, in bytes. Past it the front is cut, so a deeply
/// nested section keeps the headings nearest to it and its body keeps room.
pub const MAX_BREADCRUMB_BYTES: usize = 256;

/// Tokens reserved for the blank line between the breadcrumb and the body.
const SEPARATOR_TOKENS: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    /// Position within its doc, from 0.
    pub ordinal: usize,
    pub breadcrumb: String,
    pub body: String,
}

impl Passage {
    /// What is embedded and indexed: the body under its breadcrumb, so a
    /// heading's words find the section and the vector knows where it sits.
    pub fn indexed_text(&self) -> String {
        format!("{}\n\n{}", self.breadcrumb, self.body)
    }
}

static HEADING: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^ {0,3}(#{1,6})(?:[ \t]+(.*?))?[ \t]*$").expect("heading regex"));
static CLOSING_HASHES: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:^|[ \t]+)#+$").expect("closing hashes regex"));
static FENCE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^ {0,3}(`{3,}|~{3,})").expect("fence regex"));

/// The passages of the Markdown doc at `path`.
pub(crate) fn chunk_markdown(path: &str, text: &str, splitter: &TokenSplitter) -> Vec<Passage> {
    let mut out = Vec::new();
    let mut headings: Vec<(usize, String)> = Vec::new();
    let mut body = String::new();
    let mut fence: Option<(char, usize)> = None;

    for line in text.lines() {
        if let Some((ch, len)) = fence {
            if closes_fence(line, ch, len) {
                fence = None;
            }
            body.push_str(line);
            body.push('\n');
            continue;
        }
        if let Some(open) = FENCE.captures(line).map(|c| c[1].to_string()) {
            let ch = open.chars().next().expect("a fence has a character");
            fence = Some((ch, open.chars().count()));
            body.push_str(line);
            body.push('\n');
            continue;
        }
        if let Some((level, title)) = heading(line) {
            emit(&mut out, &breadcrumb(path, &headings), &body, splitter);
            body.clear();
            while headings.last().is_some_and(|(l, _)| *l >= level) {
                headings.pop();
            }
            headings.push((level, title));
            continue;
        }
        body.push_str(line);
        body.push('\n');
    }
    emit(&mut out, &breadcrumb(path, &headings), &body, splitter);
    out
}

/// The passages of the manifest at `path`: the whole file under its path.
pub(crate) fn chunk_manifest(path: &str, text: &str, splitter: &TokenSplitter) -> Vec<Passage> {
    let mut out = Vec::new();
    emit(&mut out, &cap_breadcrumb(path.to_string()), text, splitter);
    out
}

fn heading(line: &str) -> Option<(usize, String)> {
    let caps = HEADING.captures(line)?;
    let level = caps[1].len();
    let raw = caps.get(2).map_or("", |m| m.as_str());
    let title = CLOSING_HASHES.replace(raw, "").trim().to_string();
    Some((level, title))
}

fn closes_fence(line: &str, ch: char, len: usize) -> bool {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return false;
    }
    let run = trimmed.chars().take_while(|c| *c == ch).count();
    run >= len && trimmed[run * ch.len_utf8()..].trim().is_empty()
}

fn breadcrumb(path: &str, headings: &[(usize, String)]) -> String {
    let titles: Vec<String> = headings.iter().map(|(_, t)| t.clone()).collect();
    breadcrumb_of(path, &titles)
}

/// `path` followed by the non-empty `labels`, joined with ` > ` and capped.
pub(super) fn breadcrumb_of(path: &str, labels: &[String]) -> String {
    let mut parts = vec![path];
    parts.extend(labels.iter().map(String::as_str).filter(|t| !t.is_empty()));
    cap_breadcrumb(parts.join(" > "))
}

fn cap_breadcrumb(crumb: String) -> String {
    if crumb.len() <= MAX_BREADCRUMB_BYTES {
        return crumb;
    }
    let ellipsis = "…";
    let mut start = crumb.len() - (MAX_BREADCRUMB_BYTES - ellipsis.len());
    while !crumb.is_char_boundary(start) {
        start += 1;
    }
    format!("{ellipsis}{}", &crumb[start..])
}

pub(super) fn emit(out: &mut Vec<Passage>, breadcrumb: &str, body: &str, splitter: &TokenSplitter) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }
    let budget = CHUNK_MAX_TOKENS.saturating_sub(splitter.count(breadcrumb) + SEPARATOR_TOKENS);
    let pieces = if splitter.count(body) <= budget {
        vec![body.to_string()]
    } else {
        splitter
            .split(body, budget, CHUNK_OVERLAP_TOKENS)
            .into_iter()
            .map(|c| c.text)
            .collect()
    };
    for piece in pieces {
        out.push(Passage { ordinal: out.len(), breadcrumb: breadcrumb.to_string(), body: piece });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(path: &str, text: &str) -> Vec<(String, String)> {
        chunk_markdown(path, text, &TokenSplitter::fallback())
            .into_iter()
            .map(|p| (p.breadcrumb, p.body))
            .collect()
    }

    fn pair(crumb: &str, body: &str) -> (String, String) {
        (crumb.to_string(), body.to_string())
    }

    #[test]
    fn sections_carry_their_breadcrumb() {
        let text = "intro\n# Guía\ntexto A\n## Instalación\n### macOS\ntexto B\n## Uso\ntexto C\n";
        assert_eq!(
            md("docs/guia.md", text),
            [
                pair("docs/guia.md", "intro"),
                pair("docs/guia.md > Guía", "texto A"),
                pair("docs/guia.md > Guía > Instalación > macOS", "texto B"),
                pair("docs/guia.md > Guía > Uso", "texto C"),
            ]
        );
    }

    #[test]
    fn a_heading_inside_a_fence_does_not_split() {
        let text = "# Setup\nRun:\n```sh\n# no es encabezado\nmake\n```\nDone.\n";
        assert_eq!(
            md("a.md", text),
            [pair("a.md > Setup", "Run:\n```sh\n# no es encabezado\nmake\n```\nDone.")]
        );
    }

    #[test]
    fn a_fence_closes_only_with_its_own_character_and_length() {
        let text = "# A\n````\n```\n# inside\n~~~\n````\n# B\nb\n";
        assert_eq!(
            md("a.md", text),
            [pair("a.md > A", "````\n```\n# inside\n~~~\n````"), pair("a.md > B", "b")]
        );
    }

    #[test]
    fn an_empty_section_gives_no_passage_but_names_its_children() {
        let text = "# Top\n\n## Child\nbody\n";
        assert_eq!(md("a.md", text), [pair("a.md > Top > Child", "body")]);
    }

    #[test]
    fn closing_hashes_are_not_part_of_the_heading() {
        let text = "## Uso ##\nx\n# C# notes\ny\n#hashtag\nz\n";
        assert_eq!(
            md("a.md", text),
            [pair("a.md > Uso", "x"), pair("a.md > C# notes", "y\n#hashtag\nz")]
        );
    }

    #[test]
    fn a_heading_indented_four_spaces_is_text() {
        assert_eq!(md("a.md", "    # code\n"), [pair("a.md", "# code")]);
    }

    #[test]
    fn a_long_section_is_split_with_overlap_under_the_cap() {
        let words: Vec<String> = (0..1600).map(|i| format!("w{i:04}")).collect();
        let text = format!("# Long\n{}\n", words.join(" "));
        let splitter = TokenSplitter::fallback();

        let passages = chunk_markdown("a.md", &text, &splitter);

        assert!(passages.len() > 1, "got {}", passages.len());
        for p in &passages {
            assert_eq!(p.breadcrumb, "a.md > Long");
            assert!(splitter.count(&p.indexed_text()) <= CHUNK_MAX_TOKENS, "{}", splitter.count(&p.indexed_text()));
        }
        for pair in passages.windows(2) {
            let tail = &pair[0].body[pair[0].body.len() - 40..];
            assert!(pair[1].body.contains(tail), "the end of one piece must open the next");
        }
        let ordinals: Vec<usize> = passages.iter().map(|p| p.ordinal).collect();
        assert_eq!(ordinals, (0..passages.len()).collect::<Vec<_>>());
        assert!(passages.last().unwrap().body.ends_with("w1599"));
    }

    #[test]
    fn a_long_breadcrumb_keeps_its_nearest_headings() {
        let deep = "título ".repeat(60);
        let text = format!("# {deep}\n## Cerca\nbody\n");

        let passages = md("docs/a.md", &text);

        let crumb = &passages[0].0;
        assert!(crumb.len() <= MAX_BREADCRUMB_BYTES, "{}", crumb.len());
        assert!(crumb.starts_with('…'));
        assert!(crumb.ends_with(" > Cerca"));
    }

    #[test]
    fn a_manifest_is_one_passage_named_by_its_path() {
        let text = "[package]\nname = \"clawg\"\n\n[dependencies]\nignore = \"0.4\"\n";
        let passages = chunk_manifest("crates/a/Cargo.toml", text, &TokenSplitter::fallback());
        assert_eq!(passages.len(), 1);
        assert_eq!(passages[0].breadcrumb, "crates/a/Cargo.toml");
        assert_eq!(passages[0].body, text.trim());
        assert_eq!(passages[0].indexed_text(), format!("crates/a/Cargo.toml\n\n{}", text.trim()));
    }

    #[test]
    fn an_empty_doc_has_no_passages() {
        assert!(md("a.md", "").is_empty());
        assert!(md("a.md", "# Only a heading\n\n").is_empty());
        assert!(chunk_manifest("go.mod", "  \n", &TokenSplitter::fallback()).is_empty());
    }
}
