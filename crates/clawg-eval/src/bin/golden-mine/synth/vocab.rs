//! Vocabulary pulled from the base rows at generation time, so synthetic
//! distractors share identifiers and terms with the real ones and compete with
//! them in both the dense and the lexical channel. Nothing from the base is
//! stored in the source tree.

use std::collections::BTreeMap;

use super::golden::Row;

/// Short es/en list of long words that carry no topic.
const STOPWORDS: &[&str] = &[
    "about", "after", "again", "their", "there", "these", "those", "which", "would", "could",
    "should", "where", "while", "being", "other", "using", "because", "before", "between",
    "through", "para", "como", "pero", "desde", "entre", "sobre", "hasta", "donde", "cuando",
    "porque", "tiene", "tienen", "estos", "estas", "tambien", "también", "puede", "pueden",
    "cada", "todos", "todas", "otros", "otras", "aunque", "mientras", "durante", "luego",
];

const MAX_IDENT_CHARS: usize = 60;
const MIN_WORD_CHARS: usize = 5;

/// What one base row contributes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Terms {
    pub idents: Vec<String>,
    pub words: Vec<String>,
}

/// Rows sharing a `(memory_type, lang)`, with the importance range they span.
#[derive(Debug)]
pub struct Group {
    pub memory_type: String,
    pub lang: Option<String>,
    pub rows: Vec<Terms>,
    pub importance_lo: f64,
    pub importance_hi: f64,
}

#[derive(Debug)]
pub struct Vocab {
    pub groups: Vec<Group>,
    /// Every base row as `(group, row within group)`, in base order, so a row
    /// can be drawn uniformly.
    pub rows: Vec<(usize, usize)>,
}

impl Vocab {
    pub fn from_rows(rows: &[Row]) -> Self {
        let mut index: BTreeMap<(&str, Option<&str>), usize> = BTreeMap::new();
        let mut groups: Vec<Group> = Vec::new();
        let mut flat = Vec::with_capacity(rows.len());

        for row in rows {
            let key = (row.memory_type.as_str(), row.lang.as_deref());
            let g = *index.entry(key).or_insert_with(|| {
                groups.push(Group {
                    memory_type: row.memory_type.clone(),
                    lang: row.lang.clone(),
                    rows: Vec::new(),
                    importance_lo: row.importance,
                    importance_hi: row.importance,
                });
                groups.len() - 1
            });
            let group = &mut groups[g];
            group.importance_lo = group.importance_lo.min(row.importance);
            group.importance_hi = group.importance_hi.max(row.importance);
            group.rows.push(terms_of(&row.content));
            flat.push((g, group.rows.len() - 1));
        }
        Self { groups, rows: flat }
    }
}

fn terms_of(content: &str) -> Terms {
    Terms { idents: backtick_spans(content), words: long_words(content) }
}

/// Spans between a pair of backticks. An unmatched trailing backtick opens
/// nothing.
fn backtick_spans(content: &str) -> Vec<String> {
    let parts: Vec<&str> = content.split('`').collect();
    let mut out: Vec<String> = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let closed = i % 2 == 1 && i + 1 < parts.len();
        let span = part.trim();
        if closed
            && !span.is_empty()
            && span.chars().count() <= MAX_IDENT_CHARS
            && !span.contains('\n')
            && !out.iter().any(|s| s == span)
        {
            out.push(span.to_string());
        }
    }
    out
}

fn long_words(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in content.split(|c: char| !c.is_alphanumeric()) {
        let word = raw.to_lowercase();
        if word.chars().count() >= MIN_WORD_CHARS
            && !STOPWORDS.contains(&word.as_str())
            && !word.chars().all(|c| c.is_ascii_digit())
            && !out.contains(&word)
        {
            out.push(word);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(memory_type: &str, lang: Option<&str>, importance: f64, content: &str) -> Row {
        Row {
            ref_: "r".into(),
            content: content.into(),
            memory_type: memory_type.into(),
            importance,
            lang: lang.map(str::to_string),
        }
    }

    #[test]
    fn backtick_spans_are_extracted_once_each() {
        assert_eq!(backtick_spans("use `foo` then `bar` and `foo` again"), vec!["foo", "bar"]);
    }

    #[test]
    fn an_unmatched_backtick_opens_nothing() {
        assert_eq!(backtick_spans("closed `a` and a dangling `b"), vec!["a"]);
    }

    #[test]
    fn an_overlong_or_multiline_span_is_not_an_identifier() {
        let long = "x".repeat(MAX_IDENT_CHARS + 1);
        assert!(backtick_spans(&format!("`{long}`")).is_empty());
        assert!(backtick_spans("`a\nb`").is_empty());
    }

    #[test]
    fn words_need_five_letters_and_must_not_be_stopwords() {
        let words = long_words("The pricing rules about caching, para el Gateway 12345");
        assert_eq!(words, vec!["pricing", "rules", "caching", "gateway"]);
    }

    #[test]
    fn rows_are_grouped_by_type_and_language_with_their_importance_range() {
        let vocab = Vocab::from_rows(&[
            row("tool_error", Some("es"), 4.0, "uno `a`"),
            row("tool_error", Some("en"), 9.0, "two `b`"),
            row("tool_error", Some("es"), 6.0, "tres `c`"),
        ]);
        assert_eq!(vocab.groups.len(), 2);
        assert_eq!(vocab.groups[0].lang.as_deref(), Some("es"));
        assert_eq!(vocab.groups[0].rows.len(), 2);
        assert_eq!((vocab.groups[0].importance_lo, vocab.groups[0].importance_hi), (4.0, 6.0));
        assert_eq!(vocab.rows.len(), 3);
    }

    #[test]
    fn a_row_without_a_language_is_its_own_group() {
        let vocab = Vocab::from_rows(&[row("tool_error", None, 4.0, "x"), row("tool_error", Some("en"), 4.0, "y")]);
        assert_eq!(vocab.groups.len(), 2);
    }
}
