//! Deterministic generator that grows a golden set with synthetic rows and
//! cases. Pure: it takes a parsed base and a seed and returns the expanded
//! set, so tests need no filesystem; reading and writing live in `main`.

use std::collections::HashSet;

pub mod golden;
mod rng;
mod templates;
mod vocab;

use golden::{Case, Golden, Row, Status, Subset};
use templates::IdentKind;
use rng::SplitMix64;
use vocab::Vocab;

pub const DEFAULT_DISTRACTORS: usize = 500;

const LEXICAL_FAMILIES: usize = 30;
const SIBLINGS_PER_FAMILY: usize = 8;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum SynthError {
    #[error("ref '{0}' appears more than once")]
    DuplicateRef(String),
    #[error("query '{0}' appears more than once; `rag-eval --diff` joins cases on the query text")]
    DuplicateQuery(String),
    #[error("the base has no rows to model distractors on")]
    EmptyBase,
}

/// How much to add. Only the distractor count scales: the case and row
/// minimums of the other subsets hold at every size.
#[derive(Debug, Clone, Copy)]
pub struct Sizes {
    pub distractors: usize,
}

impl Default for Sizes {
    fn default() -> Self {
        Self { distractors: DEFAULT_DISTRACTORS }
    }
}

/// Expands `base` into a new golden set: every base row, case and pair comes
/// through unchanged, followed by whatever the generator adds. The same base,
/// seed and sizes always give the same set.
pub fn generate(base: &Golden, seed: u64, sizes: &Sizes) -> Result<Golden, SynthError> {
    check_unique(base)?;
    if sizes.distractors > 0 && base.corpus.is_empty() {
        return Err(SynthError::EmptyBase);
    }

    let vocab = Vocab::from_rows(&base.corpus);
    let mut rng = SplitMix64::new(seed);
    let mut out = base.clone();
    out.corpus.extend(distractors(&vocab, &mut rng, sizes.distractors));
    let (rows, cases) = lexical_families(base, &mut rng);
    out.corpus.extend(rows);
    out.retrieval_cases.extend(cases);

    check_unique(&out)?;
    Ok(out)
}

/// Rows that sit near real ones without answering anything: each mixes the
/// identifiers and terms of two base rows of the same type and language.
fn distractors(vocab: &Vocab, rng: &mut SplitMix64, count: usize) -> Vec<Row> {
    (0..count)
        .map(|n| {
            let (g, first) = *rng.pick(&vocab.rows);
            let group = &vocab.groups[g];
            let second = rng.below(group.rows.len());
            let (r, r2) = (&group.rows[first], &group.rows[second]);

            let spanish = group.lang.as_deref() == Some("es");
            let fallback = templates::fallback_word(spanish);
            let a = pick_term(rng, &r.idents, &r.words, fallback);
            let b = pick_term(rng, &r2.idents, &r2.words, fallback);
            let w1 = pick_term(rng, &r.words, &r2.words, fallback);
            let w2 = pick_term(rng, &r2.words, &r.words, fallback);
            let w3 = pick_term(rng, &r.words, &r2.words, fallback);

            let frame = *rng.pick(templates::distractor_frames(&group.memory_type, spanish));
            let content = templates::fill(
                frame,
                &[("{a}", a), ("{b}", b), ("{w1}", w1), ("{w2}", w2), ("{w3}", w3)],
            );

            Row {
                ref_: format!("s-dis-{:04}", n + 1),
                content,
                memory_type: group.memory_type.clone(),
                importance: importance_in(rng, group.importance_lo, group.importance_hi),
                lang: group.lang.clone(),
            }
        })
        .collect()
}

/// Families of rows that differ only in one identifier, each with a question
/// that names the identifier of one member. With eight siblings a random
/// ranking would still miss the expected row from the top five often enough
/// for recall to separate a lexical match from a dense guess.
fn lexical_families(base: &Golden, rng: &mut SplitMix64) -> (Vec<Row>, Vec<Case>) {
    let mut used: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    let mut cases = Vec::new();

    for family in 0..LEXICAL_FAMILIES {
        let kind = if family % 2 == 0 { IdentKind::Function } else { IdentKind::ErrorCode };
        let spanish = (family / 2) % 2 == 1;
        let frame = rng.pick(templates::lexical_frames(kind, spanish));
        let stem = match kind {
            IdentKind::Function => format!(
                "{}{}",
                rng.pick(&templates::FUNCTION_VERBS),
                rng.pick(&templates::FUNCTION_NOUNS)
            ),
            IdentKind::ErrorCode => format!("E_{}_", rng.pick(&templates::ERROR_AREAS)),
        };

        let mut refs = Vec::with_capacity(SIBLINGS_PER_FAMILY);
        let mut idents = Vec::with_capacity(SIBLINGS_PER_FAMILY);
        for sibling in 0..SIBLINGS_PER_FAMILY {
            let ident = loop {
                let candidate = format!("{stem}{}", 1000 + rng.below(9000));
                let taken = used.iter().any(|u| u.contains(&candidate) || candidate.contains(u.as_str()))
                    || base.corpus.iter().any(|r| r.content.contains(&candidate));
                if !taken {
                    break candidate;
                }
            };
            used.push(ident.clone());

            let ref_ = format!("s-lex-{family:02}-{sibling}");
            rows.push(Row {
                ref_: ref_.clone(),
                content: templates::fill(frame.content, &[("{id}", &ident)]),
                memory_type: frame.memory_type.to_string(),
                importance: 5.0,
                lang: Some(if spanish { "es" } else { "en" }.to_string()),
            });
            refs.push(ref_);
            idents.push(ident);
        }

        let expected = rng.below(SIBLINGS_PER_FAMILY);
        cases.push(Case {
            query: templates::fill(frame.query, &[("{id}", &idents[expected])]),
            corpus_refs: refs.clone(),
            expected_refs: vec![refs[expected].clone()],
            status: Status::Active,
            blocked_by: None,
            history: Vec::new(),
            subset: Subset::Lexical,
        });
    }
    (rows, cases)
}

/// A term from `primary`, else from `secondary`, else `fallback`.
fn pick_term<'a>(
    rng: &mut SplitMix64,
    primary: &'a [String],
    secondary: &'a [String],
    fallback: &'a str,
) -> &'a str {
    if !primary.is_empty() {
        rng.pick(primary).as_str()
    } else if !secondary.is_empty() {
        rng.pick(secondary).as_str()
    } else {
        fallback
    }
}

/// A whole-number importance inside `lo..=hi`.
fn importance_in(rng: &mut SplitMix64, lo: f64, hi: f64) -> f64 {
    let span = (hi - lo).round().max(0.0) as usize;
    lo + rng.below(span + 1) as f64
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

    const NONE: Sizes = Sizes { distractors: 0 };

    fn rich_row(r: &str, memory_type: &str, lang: &str, importance: f64, content: &str) -> Row {
        Row {
            ref_: r.into(),
            content: content.into(),
            memory_type: memory_type.into(),
            importance,
            lang: Some(lang.into()),
        }
    }

    /// A base with two groups, `(architectural_decision, en)` and
    /// `(tool_error, es)`, each carrying backticked identifiers.
    fn grouped_base() -> Golden {
        Golden {
            corpus: vec![
                rich_row("c01", "architectural_decision", "en", 6.0,
                    "The `OrderService` owns pricing rules and delegates persistence to `OrderRepository`."),
                rich_row("c02", "architectural_decision", "en", 7.0,
                    "Gateway authentication moved into `AuthMiddleware`, replacing the legacy session handling."),
                rich_row("c03", "tool_error", "es", 4.0,
                    "Falló la migración `add_index_users` porque la tabla ya tenía bloqueos pendientes."),
                rich_row("c04", "tool_error", "es", 5.0,
                    "El comando `cargo_build` terminó con errores de enlace durante la compilación incremental."),
            ],
            retrieval_cases: vec![case("how is pricing split", "c01")],
            contradiction_pairs: vec![],
        }
    }

    fn distractors(out: &Golden, base: &Golden) -> Vec<Row> {
        out.corpus[base.corpus.len()..].iter().filter(|r| r.ref_.starts_with("s-dis-")).cloned().collect()
    }

    #[test]
    fn the_same_seed_gives_the_same_set_and_another_seed_a_different_one() {
        let base = grouped_base();
        let sizes = Sizes { distractors: 40 };
        let a = serde_json::to_string(&generate(&base, 7, &sizes).unwrap()).unwrap();
        let b = serde_json::to_string(&generate(&base, 7, &sizes).unwrap()).unwrap();
        let c = serde_json::to_string(&generate(&base, 8, &sizes).unwrap()).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn the_default_size_adds_at_least_five_hundred_distractors() {
        let base = grouped_base();
        let out = generate(&base, 7, &Sizes::default()).unwrap();
        assert!(distractors(&out, &base).len() >= 500);
    }

    #[test]
    fn the_size_only_changes_how_many_distractors_are_added() {
        let base = grouped_base();
        let out = generate(&base, 7, &Sizes { distractors: 12 }).unwrap();
        assert_eq!(distractors(&out, &base).len(), 12);
    }

    #[test]
    fn distractor_refs_are_unique_and_do_not_reuse_a_base_ref() {
        let base = grouped_base();
        let out = generate(&base, 7, &Sizes { distractors: 200 }).unwrap();
        let refs: HashSet<&str> = out.corpus.iter().map(|r| r.ref_.as_str()).collect();
        assert_eq!(refs.len(), out.corpus.len());
    }

    #[test]
    fn a_distractor_inherits_a_type_and_language_the_base_has() {
        let base = grouped_base();
        let out = generate(&base, 7, &Sizes { distractors: 100 }).unwrap();
        let groups: HashSet<(&str, Option<&str>)> =
            base.corpus.iter().map(|r| (r.memory_type.as_str(), r.lang.as_deref())).collect();
        for d in distractors(&out, &base) {
            assert!(groups.contains(&(d.memory_type.as_str(), d.lang.as_deref())), "{d:?}");
        }
    }

    #[test]
    fn a_distractor_stays_inside_its_groups_importance_range() {
        let base = grouped_base();
        let out = generate(&base, 7, &Sizes { distractors: 100 }).unwrap();
        for d in distractors(&out, &base) {
            let (lo, hi) = if d.memory_type == "tool_error" { (4.0, 5.0) } else { (6.0, 7.0) };
            assert!((lo..=hi).contains(&d.importance), "{d:?}");
        }
    }

    #[test]
    fn a_distractor_reuses_identifiers_from_the_base_rows() {
        let base = grouped_base();
        let out = generate(&base, 7, &Sizes { distractors: 50 }).unwrap();
        let idents = ["OrderService", "OrderRepository", "AuthMiddleware", "add_index_users", "cargo_build"];
        for d in distractors(&out, &base) {
            assert!(idents.iter().any(|i| d.content.contains(&format!("`{i}`"))), "{d:?}");
        }
    }

    #[test]
    fn distractors_need_at_least_one_base_row() {
        let empty = Golden { corpus: vec![], retrieval_cases: vec![], contradiction_pairs: vec![] };
        assert_eq!(generate(&empty, 7, &Sizes { distractors: 5 }), Err(SynthError::EmptyBase));
    }

    #[test]
    fn a_base_ref_that_collides_with_a_synthetic_one_is_an_error() {
        let mut base = grouped_base();
        base.corpus.push(row("s-dis-0001"));
        assert_eq!(
            generate(&base, 7, &Sizes { distractors: 3 }),
            Err(SynthError::DuplicateRef("s-dis-0001".into()))
        );
    }

    fn lexical_cases(out: &Golden) -> Vec<&Case> {
        out.retrieval_cases.iter().filter(|c| c.subset == Subset::Lexical).collect()
    }

    fn row_of<'a>(out: &'a Golden, r: &str) -> &'a Row {
        out.corpus.iter().find(|row| row.ref_ == r).unwrap_or_else(|| panic!("no row {r}"))
    }

    /// The identifier a lexical row is about: its first backticked span.
    fn ident_of(row: &Row) -> &str {
        row.content.split('`').nth(1).expect("a lexical row names its identifier")
    }

    #[test]
    fn there_are_at_least_thirty_lexical_cases_at_any_size() {
        let base = grouped_base();
        for distractors in [0, 10, 500] {
            let out = generate(&base, 7, &Sizes { distractors }).unwrap();
            assert!(lexical_cases(&out).len() >= 30, "distractors={distractors}");
        }
    }

    #[test]
    fn a_lexical_family_has_eight_siblings_and_the_expected_one_is_among_them() {
        let out = generate(&grouped_base(), 7, &NONE).unwrap();
        for case in lexical_cases(&out) {
            assert_eq!(case.corpus_refs.len(), 8, "{}", case.query);
            assert_eq!(case.expected_refs.len(), 1);
            assert!(case.corpus_refs.contains(&case.expected_refs[0]));
            for r in &case.corpus_refs {
                row_of(&out, r);
            }
        }
    }

    #[test]
    fn the_queried_identifier_appears_in_the_expected_row_and_nowhere_else() {
        let out = generate(&grouped_base(), 7, &Sizes { distractors: 200 }).unwrap();
        for case in lexical_cases(&out) {
            let ident = ident_of(row_of(&out, &case.expected_refs[0]));
            assert!(case.query.contains(ident), "{} lacks {ident}", case.query);
            let holders = out.corpus.iter().filter(|r| r.content.contains(ident)).count();
            assert_eq!(holders, 1, "{ident} must live in exactly one row");
        }
    }

    #[test]
    fn siblings_differ_from_the_expected_row_only_in_the_identifier() {
        let out = generate(&grouped_base(), 7, &NONE).unwrap();
        for case in lexical_cases(&out) {
            let expected = row_of(&out, &case.expected_refs[0]);
            let shape = expected.content.replace(ident_of(expected), "ID");
            for r in &case.corpus_refs {
                let sibling = row_of(&out, r);
                assert_eq!(sibling.content.replace(ident_of(sibling), "ID"), shape);
                assert_eq!(sibling.memory_type, expected.memory_type);
                assert_eq!(sibling.lang, expected.lang);
            }
        }
    }

    #[test]
    fn lexical_refs_use_their_own_prefix_and_the_base_stays_first() {
        let base = grouped_base();
        let out = generate(&base, 7, &NONE).unwrap();
        assert!(out.corpus[base.corpus.len()..].iter().all(|r| r.ref_.starts_with("s-lex-")));
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
        let out = generate(&base, 7, &NONE).unwrap();
        assert_eq!(out.corpus[..base.corpus.len()], base.corpus[..]);
        assert_eq!(out.retrieval_cases[..base.retrieval_cases.len()], base.retrieval_cases[..]);
        assert_eq!(out.contradiction_pairs, base.contradiction_pairs);
    }

    #[test]
    fn a_duplicate_ref_is_an_error() {
        let mut base = base();
        base.corpus.push(row("c01"));
        assert_eq!(generate(&base, 7, &NONE), Err(SynthError::DuplicateRef("c01".into())));
    }

    #[test]
    fn a_duplicate_query_is_an_error() {
        let mut base = base();
        base.retrieval_cases.push(case("first question", "c02"));
        assert_eq!(generate(&base, 7, &NONE), Err(SynthError::DuplicateQuery("first question".into())));
    }
}
