//! Deterministic generator that grows a golden set with synthetic rows and
//! cases. Pure: it takes a parsed base and a seed and returns the expanded
//! set, so tests need no filesystem; reading and writing live in `main`.

use std::collections::HashSet;

pub mod golden;

use golden::Golden;

pub const DEFAULT_DISTRACTORS: usize = 500;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum SynthError {
    #[error("ref '{0}' appears more than once")]
    DuplicateRef(String),
    #[error("query '{0}' appears more than once; `rag-eval --diff` joins cases on the query text")]
    DuplicateQuery(String),
}

/// Expands `base` into a new golden set: every base row, case and pair comes
/// through unchanged, followed by whatever the generator adds.
pub fn generate(base: &Golden, _seed: u64, _distractors: usize) -> Result<Golden, SynthError> {
    let out = base.clone();
    check_unique(&out)?;
    Ok(out)
}

fn check_unique(golden: &Golden) -> Result<(), SynthError> {
    let mut refs = HashSet::new();
    for row in &golden.corpus {
        if !refs.insert(row.ref_.as_str()) {
            return Err(SynthError::DuplicateRef(row.ref_.clone()));
        }
    }
    let mut queries = HashSet::new();
    for case in &golden.retrieval_cases {
        if !queries.insert(case.query.as_str()) {
            return Err(SynthError::DuplicateQuery(case.query.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::golden::*;
    use super::*;

    fn row(r: &str) -> Row {
        Row {
            ref_: r.into(),
            content: format!("content of {r}"),
            memory_type: "semantic_fact".into(),
            importance: 5.0,
            lang: Some("en".into()),
        }
    }

    fn case(query: &str, expected: &str) -> Case {
        Case {
            query: query.into(),
            corpus_refs: vec![expected.into()],
            expected_refs: vec![expected.into()],
            status: Status::Active,
            blocked_by: None,
            history: vec![],
            subset: Subset::Natural,
        }
    }

    fn base() -> Golden {
        Golden {
            corpus: vec![row("c01"), row("c02")],
            retrieval_cases: vec![case("first question", "c01"), case("second question", "c02")],
            contradiction_pairs: vec![Pair {
                a: "x".into(),
                b: "y".into(),
                label: PairLabel::Contradiction,
                lang: "en".into(),
            }],
        }
    }

    #[test]
    fn the_base_rows_cases_and_pairs_come_through_untouched() {
        let base = base();
        let out = generate(&base, 7, 0).unwrap();
        assert_eq!(out.corpus[..base.corpus.len()], base.corpus[..]);
        assert_eq!(out.retrieval_cases[..base.retrieval_cases.len()], base.retrieval_cases[..]);
        assert_eq!(out.contradiction_pairs, base.contradiction_pairs);
    }

    #[test]
    fn a_duplicate_ref_is_an_error() {
        let mut base = base();
        base.corpus.push(row("c01"));
        assert_eq!(generate(&base, 7, 0), Err(SynthError::DuplicateRef("c01".into())));
    }

    #[test]
    fn a_duplicate_query_is_an_error() {
        let mut base = base();
        base.retrieval_cases.push(case("first question", "c02"));
        assert_eq!(generate(&base, 7, 0), Err(SynthError::DuplicateQuery("first question".into())));
    }
}
