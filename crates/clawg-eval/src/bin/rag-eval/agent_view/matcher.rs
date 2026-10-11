//! Decides which golden-set rows an agent-visible text carries.
//!
//! The agent-view run stores rows through the real ingest pipeline, which
//! chunks them and assigns ids the golden set knows nothing about. Identity is
//! therefore recovered from the text itself: a visible fragment belongs to a
//! corpus row when it appears verbatim inside that row's content.

use std::collections::BTreeSet;

/// Shortest visible fragment accepted as evidence of a row. Anything shorter
/// is common enough phrasing to occur inside an unrelated row, so matching it
/// would credit a retrieval that returned the wrong memory.
pub const MIN_SEGMENT_CHARS: usize = 40;

/// What the search tool shows in place of content it cut.
const ELLIPSIS: char = '…';

/// Collapses every whitespace run to one space and trims the ends. Both sides
/// of a comparison go through this: the search renderer flattens newlines and
/// the chunker trims chunk edges, so raw equality would miss rows that are
/// plainly the same text.
pub fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One fragment of content as an agent sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visible {
    /// Normalized, with the truncation marker removed.
    pub text: String,
    /// The tool cut this content short, so the agent never saw the rest.
    pub truncated: bool,
}

/// Normalizes `raw` and drops a trailing truncation marker, remembering that
/// it was there.
pub fn visible_segment(raw: &str) -> Visible {
    let normalized = normalize(raw);
    match normalized.strip_suffix(ELLIPSIS) {
        Some(rest) => Visible { text: rest.trim_end().to_string(), truncated: true },
        None => Visible { text: normalized, truncated: false },
    }
}

/// True when `segment` is long enough to count as evidence and appears inside
/// `row_content`. Both must already be normalized.
pub fn is_match(segment: &str, row_content: &str) -> bool {
    segment.chars().count() >= MIN_SEGMENT_CHARS && row_content.contains(segment)
}

/// The corpus as the matcher sees it: each row's ref beside its normalized
/// content.
#[derive(Debug, Clone)]
pub struct Corpus {
    rows: Vec<(String, String)>,
}

impl Corpus {
    pub fn new<'a>(rows: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        Self { rows: rows.into_iter().map(|(r, c)| (r.to_string(), normalize(c))).collect() }
    }

    /// Refs of every row whose content contains `segment`, in corpus order.
    pub fn refs_containing(&self, segment: &str) -> Vec<&str> {
        self.rows
            .iter()
            .filter(|(_, content)| is_match(segment, content))
            .map(|(r, _)| r.as_str())
            .collect()
    }

    /// Refs sharing their normalized content with at least one other row.
    /// A segment that matches one of them matches all of them, so a hit on
    /// such a ref cannot be attributed to it alone.
    pub fn ambiguous_refs(&self) -> BTreeSet<String> {
        self.rows
            .iter()
            .filter(|(r, content)| self.rows.iter().any(|(other, c)| other != r && c == content))
            .map(|(r, _)| r.clone())
            .collect()
    }

    /// Refs whose whole content is shorter than [`MIN_SEGMENT_CHARS`]: no
    /// visible fragment of them can ever qualify, so they can never be found.
    pub fn unmatchable_refs(&self) -> Vec<&str> {
        self.rows
            .iter()
            .filter(|(_, content)| content.chars().count() < MIN_SEGMENT_CHARS)
            .map(|(r, _)| r.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: &str = "the retrieval decay term compares milliseconds against seconds";

    #[test]
    fn normalize_collapses_whitespace_runs_and_trims() {
        assert_eq!(normalize("  a \n\t b\r\n  c  "), "a b c");
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn visible_segment_strips_a_trailing_ellipsis_and_flags_the_cut() {
        let v = visible_segment("some cut content …");
        assert_eq!(v, Visible { text: "some cut content".to_string(), truncated: true });
    }

    #[test]
    fn visible_segment_without_an_ellipsis_is_only_normalized() {
        let v = visible_segment("plain   content\nhere");
        assert_eq!(v, Visible { text: "plain content here".to_string(), truncated: false });
    }

    #[test]
    fn visible_segment_keeps_an_ellipsis_that_is_not_trailing() {
        assert!(!visible_segment("a … b").truncated);
    }

    #[test]
    fn a_segment_that_is_a_chunk_of_the_row_matches() {
        let row = normalize(&format!("{LONG}. And then a second sentence follows it."));
        assert!(is_match(LONG, &row));
    }

    #[test]
    fn a_500_char_preview_matches_the_longer_row_it_was_cut_from() {
        let row: String = "word ".repeat(300);
        let flat = normalize(&row);
        let preview: String = flat.chars().take(500).collect();
        let shown = visible_segment(&format!("{preview}…"));
        assert!(shown.truncated);
        assert!(is_match(&shown.text, &flat));
    }

    #[test]
    fn a_segment_shorter_than_the_minimum_never_matches() {
        let short = "a".repeat(MIN_SEGMENT_CHARS - 1);
        assert!(!is_match(&short, &short), "even the row's own full text is too short to be evidence");
        let exact = "a".repeat(MIN_SEGMENT_CHARS);
        assert!(is_match(&exact, &exact));
    }

    #[test]
    fn minimum_length_counts_characters_not_bytes() {
        let accented = "é".repeat(MIN_SEGMENT_CHARS);
        assert!(is_match(&accented, &accented));
    }

    #[test]
    fn a_segment_from_another_row_does_not_match() {
        assert!(!is_match(LONG, "an unrelated third row that no query targets directly at all"));
    }

    #[test]
    fn refs_containing_lists_every_row_that_holds_the_segment() {
        let corpus = Corpus::new([
            ("c1", LONG),
            ("c2", "el indice knn mide distancia l2 cruda y no similitud coseno"),
            ("c3", "prefix then the retrieval decay term compares milliseconds against seconds suffix"),
        ]);
        assert_eq!(corpus.refs_containing(LONG), vec!["c1", "c3"]);
    }

    #[test]
    fn corpus_content_is_normalized_before_matching() {
        let corpus = Corpus::new([("c1", "the retrieval decay term\ncompares milliseconds   against seconds")]);
        assert_eq!(corpus.refs_containing(LONG), vec!["c1"]);
    }

    #[test]
    fn ambiguous_refs_are_the_rows_sharing_normalized_content() {
        let corpus = Corpus::new([("c1", LONG), ("c2", "different   content that is long enough to matter here"), ("c3", "the retrieval decay term compares milliseconds   against seconds")]);
        let ambiguous = corpus.ambiguous_refs();
        assert_eq!(ambiguous.into_iter().collect::<Vec<_>>(), vec!["c1".to_string(), "c3".to_string()]);
    }

    #[test]
    fn a_unique_row_is_not_ambiguous() {
        assert!(Corpus::new([("c1", LONG)]).ambiguous_refs().is_empty());
    }

    #[test]
    fn unmatchable_refs_are_the_rows_too_short_to_ever_qualify() {
        let corpus = Corpus::new([("c1", LONG), ("c2", "too short")]);
        assert_eq!(corpus.unmatchable_refs(), vec!["c2"]);
    }
}
