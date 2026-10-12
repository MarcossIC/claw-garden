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
const MULTI_REF_TOPICS: usize = 12;
const HIGH_IMPORTANCE_ROWS: usize = 24;
/// Rows in a high-importance case's pool: the expected one and its successors.
const HIGH_IMPORTANCE_POOL: usize = 4;

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

    let mut names = Codenames::default();
    let (rows, cases) = multi_ref_topics(base, &mut names, &mut rng);
    out.corpus.extend(rows);
    out.retrieval_cases.extend(cases);
    let (rows, cases) = high_importance_rules(base, &mut names, &mut rng);
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

/// Families of eight rows, each with a question that names the identifier of
/// one member. The seven siblings read like the question and carry identifiers
/// a few digits apart from the queried one; the member that holds the queried
/// identifier mentions it in passing, so only an exact-token match can single
/// it out.
fn lexical_families(base: &Golden, rng: &mut SplitMix64) -> (Vec<Row>, Vec<Case>) {
    let mut used: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    let mut cases = Vec::new();

    for family in 0..LEXICAL_FAMILIES {
        let kind = if family % 2 == 0 { IdentKind::Function } else { IdentKind::ErrorCode };
        let spanish = (family / 2) % 2 == 1;
        let frame = rng.pick(templates::lexical_frames(kind, spanish));
        let mention = *rng.pick(templates::mention_frames(kind, spanish));
        let stem = match kind {
            IdentKind::Function => format!(
                "{}{}",
                rng.pick(&templates::FUNCTION_VERBS),
                rng.pick(&templates::FUNCTION_NOUNS)
            ),
            IdentKind::ErrorCode => format!("E_{}_", rng.pick(&templates::ERROR_AREAS)),
        };

        let taken = |candidate: &str| {
            used.iter().any(|u| u.contains(candidate) || candidate.contains(u.as_str()))
                || base.corpus.iter().any(|r| r.content.contains(candidate))
        };
        let (seed_ident, near) = loop {
            let digits = (1000 + rng.below(9000)).to_string();
            let seed_ident = format!("{stem}{digits}");
            if taken(&seed_ident) {
                continue;
            }
            let near = near_identifiers(&stem, &digits, SIBLINGS_PER_FAMILY - 1, rng, &taken);
            if near.len() == SIBLINGS_PER_FAMILY - 1 {
                break (seed_ident, near);
            }
        };
        let expected = rng.below(SIBLINGS_PER_FAMILY);
        let mut idents = near;
        idents.insert(expected, seed_ident);
        used.extend(idents.iter().cloned());

        let mut refs = Vec::with_capacity(SIBLINGS_PER_FAMILY);
        for (sibling, ident) in idents.iter().enumerate() {
            let ref_ = format!("s-lex-{family:02}-{sibling}");
            let content = if sibling == expected { mention } else { frame.content };
            rows.push(Row {
                ref_: ref_.clone(),
                content: templates::fill(content, &[("{id}", ident)]),
                memory_type: frame.memory_type.to_string(),
                importance: 5.0,
                lang: Some(if spanish { "es" } else { "en" }.to_string()),
            });
            refs.push(ref_);
        }

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

/// Identifiers that look like `{stem}{digits}` without being it: the same four
/// digits in another order, or the last digit one step away. At most `n` come
/// back, fewer when the digits have too few neighbours; `taken` rules out any
/// candidate the corpus already holds.
fn near_identifiers(
    stem: &str,
    digits: &str,
    n: usize,
    rng: &mut SplitMix64,
    taken: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    let original: Vec<u8> = digits.bytes().collect();
    let mut candidates: Vec<Vec<u8>> = Vec::new();
    permutations(&mut original.clone(), 0, &mut candidates);
    let last = original.len() - 1;
    for step in [1i8, -1] {
        let moved = original[last] as i16 - b'0' as i16 + step as i16;
        if (0..=9).contains(&moved) {
            let mut next = original.clone();
            next[last] = b'0' + moved as u8;
            candidates.push(next);
        }
    }
    candidates.retain(|c| *c != original);
    candidates.sort();
    candidates.dedup();

    for i in (1..candidates.len()).rev() {
        candidates.swap(i, rng.below(i + 1));
    }
    candidates
        .into_iter()
        .map(|c| format!("{stem}{}", String::from_utf8_lossy(&c)))
        .filter(|id| !taken(id))
        .take(n)
        .collect()
}

fn permutations(items: &mut Vec<u8>, from: usize, out: &mut Vec<Vec<u8>>) {
    if from == items.len() {
        out.push(items.clone());
        return;
    }
    for i in from..items.len() {
        items.swap(from, i);
        permutations(items, from + 1, out);
        items.swap(from, i);
    }
}

/// Two-word names that tag a topic, handed out once each and never one that
/// the base already mentions, so a question about a name has a known set of
/// rows as its answer.
#[derive(Default)]
struct Codenames {
    used: HashSet<String>,
}

impl Codenames {
    fn next(&mut self, base: &Golden, rng: &mut SplitMix64) -> String {
        loop {
            let name = format!(
                "{}-{}",
                rng.pick(&templates::CODENAME_ADJECTIVES),
                rng.pick(&templates::CODENAME_NOUNS)
            );
            if !self.used.contains(&name) && !base.corpus.iter().any(|r| r.content.contains(&name)) {
                self.used.insert(name.clone());
                return name;
            }
        }
    }
}

/// Topics split over two or three rows, with one question whose answer is all
/// of them: the case that gives nDCG something to rank.
fn multi_ref_topics(base: &Golden, names: &mut Codenames, rng: &mut SplitMix64) -> (Vec<Row>, Vec<Case>) {
    let mut rows = Vec::new();
    let mut cases = Vec::new();
    let mut groups: Vec<Vec<String>> = Vec::new();

    for topic in 0..MULTI_REF_TOPICS {
        let key = names.next(base, rng);
        let spanish = topic % 2 == 1;
        let aspects = templates::topic_aspects(spanish);
        let take = if topic % 2 == 0 { 3 } else { 2 };

        let refs: Vec<String> = (0..take).map(|k| format!("s-mr-{topic:02}-{k}")).collect();
        for (ref_, aspect) in refs.iter().zip(aspects) {
            rows.push(Row {
                ref_: ref_.clone(),
                content: templates::fill(aspect, &[("{key}", &key)]),
                memory_type: "semantic_fact".to_string(),
                importance: 5.0,
                lang: Some(if spanish { "es" } else { "en" }.to_string()),
            });
        }
        cases.push(Case {
            query: templates::fill(templates::topic_query(spanish), &[("{key}", &key)]),
            corpus_refs: refs.clone(),
            expected_refs: refs.clone(),
            status: Status::Active,
            blocked_by: None,
            history: Vec::new(),
            subset: Subset::MultiRef,
        });
        groups.push(refs);
    }

    // A neighbouring topic's first row joins each pool so the pool is not
    // made only of answers.
    for (i, case) in cases.iter_mut().enumerate() {
        case.corpus_refs.push(groups[(i + 1) % groups.len()][0].clone());
    }
    (rows, cases)
}

/// User rules of importance 8 to 10, a range the hand-written set never
/// reaches. Importance cycles 8, 9, 10, so two thirds sit at 9 or above.
fn high_importance_rules(base: &Golden, names: &mut Codenames, rng: &mut SplitMix64) -> (Vec<Row>, Vec<Case>) {
    let mut rows = Vec::new();
    let mut queries = Vec::new();

    for i in 0..HIGH_IMPORTANCE_ROWS {
        let key = names.next(base, rng);
        let spanish = i % 2 == 1;
        let (content, query) = templates::rule_frames(spanish)[(i / 2) % 2];
        rows.push(Row {
            ref_: format!("s-hi-{i:02}"),
            content: templates::fill(content, &[("{key}", &key)]),
            memory_type: "user_preference".to_string(),
            importance: 8.0 + (i % 3) as f64,
            lang: Some(if spanish { "es" } else { "en" }.to_string()),
        });
        queries.push(templates::fill(query, &[("{key}", &key)]));
    }

    let cases = queries
        .into_iter()
        .enumerate()
        .map(|(i, query)| Case {
            query,
            corpus_refs: (0..HIGH_IMPORTANCE_POOL)
                .map(|k| rows[(i + k) % rows.len()].ref_.clone())
                .collect(),
            expected_refs: vec![rows[i].ref_.clone()],
            status: Status::Active,
            blocked_by: None,
            history: Vec::new(),
            subset: Subset::HighImportance,
        })
        .collect();
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
    fn the_expected_row_uses_a_mention_frame_and_its_siblings_the_queried_frame() {
        let out = generate(&grouped_base(), 7, &NONE).unwrap();
        let shape = |row: &Row| row.content.replace(ident_of(row), "{id}");
        for case in lexical_cases(&out) {
            let expected = row_of(&out, &case.expected_refs[0]);
            let spanish = expected.lang.as_deref() == Some("es");
            let kinds = [IdentKind::Function, IdentKind::ErrorCode];
            let in_mentions = |row: &Row| kinds.iter().any(|k| templates::mention_frames(*k, spanish).contains(&shape(row).as_str()));
            let lexical_of = |row: &Row| -> Option<&'static templates::LexicalFrame> {
                kinds.iter().flat_map(|k| templates::lexical_frames(*k, spanish)).find(|f| f.content == shape(row))
            };
            assert!(in_mentions(expected), "{}", expected.content);

            let queried = lexical_of(row_of(&out, case.corpus_refs.iter().find(|r| **r != case.expected_refs[0]).unwrap()))
                .expect("siblings use a lexical frame");
            assert_eq!(case.query, templates::fill(queried.query, &[("{id}", ident_of(expected))]));
            for r in case.corpus_refs.iter().filter(|r| **r != case.expected_refs[0]) {
                let sibling = row_of(&out, r);
                assert_eq!(shape(sibling), queried.content);
                assert_eq!(sibling.memory_type, expected.memory_type);
                assert_eq!(sibling.lang, expected.lang);
            }
        }
    }

    #[test]
    fn siblings_carry_identifiers_near_the_queried_one() {
        let out = generate(&grouped_base(), 7, &NONE).unwrap();
        for case in lexical_cases(&out) {
            let queried = ident_of(row_of(&out, &case.expected_refs[0]));
            let split = queried.len() - 4;
            let (stem, digits) = queried.split_at(split);
            for r in case.corpus_refs.iter().filter(|r| **r != case.expected_refs[0]) {
                let sibling = ident_of(row_of(&out, r));
                assert_ne!(sibling, queried);
                let tail = sibling.strip_prefix(stem).unwrap_or_else(|| panic!("{sibling} vs {stem}"));
                assert!(digits_are_near(digits, tail), "{sibling} vs {queried}");
            }
        }
    }

    #[test]
    fn the_base_rows_stay_first_and_lexical_refs_use_their_own_prefix() {
        let base = grouped_base();
        let out = generate(&base, 7, &NONE).unwrap();
        assert_eq!(out.corpus[..base.corpus.len()], base.corpus[..]);
        assert!(out.corpus.iter().any(|r| r.ref_.starts_with("s-lex-")));
    }

    fn digits_are_near(original: &str, candidate: &str) -> bool {
        let mut a: Vec<char> = original.chars().collect();
        let mut b: Vec<char> = candidate.chars().collect();
        let last_step = (original.as_bytes()[3] as i32 - candidate.as_bytes()[3] as i32).abs() == 1
            && original[..3] == candidate[..3];
        a.sort_unstable();
        b.sort_unstable();
        a == b || last_step
    }

    #[test]
    fn near_identifiers_share_the_prefix_and_keep_the_digits_close() {
        let mut rng = SplitMix64::new(7);
        let ids = near_identifiers("hydrateJournal", "6105", 7, &mut rng, &|_| false);
        assert_eq!(ids.len(), 7);
        let unique: HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), 7);
        for id in &ids {
            assert_ne!(id, "hydrateJournal6105");
            let digits = id.strip_prefix("hydrateJournal").expect("same prefix");
            assert_eq!(digits.len(), 4);
            assert!(digits_are_near("6105", digits), "{id}");
        }
    }

    #[test]
    fn near_identifiers_are_deterministic_for_a_seed() {
        let a = near_identifiers("E_CACHE_", "3698", 7, &mut SplitMix64::new(3), &|_| false);
        let b = near_identifiers("E_CACHE_", "3698", 7, &mut SplitMix64::new(3), &|_| false);
        let c = near_identifiers("E_CACHE_", "3698", 7, &mut SplitMix64::new(4), &|_| false);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn a_taken_identifier_is_skipped_for_the_next_candidate() {
        let mut rng = SplitMix64::new(7);
        let first = near_identifiers("E_CACHE_", "3698", 7, &mut rng, &|_| false);
        let banned = first[0].clone();
        let again = near_identifiers("E_CACHE_", "3698", 7, &mut SplitMix64::new(7), &|c| c == banned);
        assert_eq!(again.len(), 7);
        assert!(!again.contains(&banned));
    }

    #[test]
    fn digits_with_few_neighbours_give_fewer_identifiers_than_asked() {
        let ids = near_identifiers("E_CACHE_", "1111", 7, &mut SplitMix64::new(7), &|_| false);
        let set: HashSet<&str> = ids.iter().map(String::as_str).collect();
        assert_eq!(set, HashSet::from(["E_CACHE_1110", "E_CACHE_1112"]));
    }

    fn cases_of(out: &Golden, subset: Subset) -> Vec<&Case> {
        out.retrieval_cases.iter().filter(|c| c.subset == subset).collect()
    }

    fn synthetic_rows<'a>(out: &'a Golden, base: &Golden) -> &'a [Row] {
        &out.corpus[base.corpus.len()..]
    }

    #[test]
    fn there_are_at_least_ten_multi_reference_cases_at_any_size() {
        let base = grouped_base();
        for distractors in [0, 10, 500] {
            let out = generate(&base, 7, &Sizes { distractors }).unwrap();
            let multi = cases_of(&out, Subset::MultiRef);
            assert!(multi.len() >= 10, "distractors={distractors}");
            assert!(multi.iter().all(|c| c.expected_refs.len() > 1));
        }
    }

    #[test]
    fn every_multi_reference_expectation_is_in_the_pool_and_the_corpus() {
        let out = generate(&grouped_base(), 7, &NONE).unwrap();
        for case in cases_of(&out, Subset::MultiRef) {
            for r in &case.expected_refs {
                assert!(case.corpus_refs.contains(r), "{}", case.query);
                row_of(&out, r);
            }
        }
    }

    #[test]
    fn a_multi_reference_topic_key_lives_in_exactly_the_expected_rows() {
        let out = generate(&grouped_base(), 7, &Sizes { distractors: 100 }).unwrap();
        for case in cases_of(&out, Subset::MultiRef) {
            let key = ident_of(row_of(&out, &case.expected_refs[0]));
            assert!(case.query.contains(key), "{} lacks {key}", case.query);
            let holders = out.corpus.iter().filter(|r| r.content.contains(key)).count();
            assert_eq!(holders, case.expected_refs.len(), "{key}");
        }
    }

    #[test]
    fn there_are_enough_high_importance_rows_at_any_size() {
        let base = grouped_base();
        for distractors in [0, 10, 500] {
            let out = generate(&base, 7, &Sizes { distractors }).unwrap();
            let rows = synthetic_rows(&out, &base);
            let high = rows.iter().filter(|r| (8.0..=10.0).contains(&r.importance)).count();
            let tier_one = rows.iter().filter(|r| (9.0..=10.0).contains(&r.importance)).count();
            assert!(high >= 20, "distractors={distractors}: {high}");
            assert!(tier_one >= 10, "distractors={distractors}: {tier_one}");
        }
    }

    #[test]
    fn each_high_importance_case_expects_a_row_of_importance_eight_or_more() {
        let out = generate(&grouped_base(), 7, &NONE).unwrap();
        let cases = cases_of(&out, Subset::HighImportance);
        assert!(cases.len() >= 20);
        for case in cases {
            assert_eq!(case.expected_refs.len(), 1);
            let expected = row_of(&out, &case.expected_refs[0]);
            assert!(expected.importance >= 8.0, "{}", case.query);
            assert!(case.corpus_refs.contains(&case.expected_refs[0]));
            let key = ident_of(expected);
            assert!(case.query.contains(key));
            assert_eq!(out.corpus.iter().filter(|r| r.content.contains(key)).count(), 1);
        }
    }

    #[test]
    fn the_synthetic_refs_keep_their_prefixes() {
        let base = grouped_base();
        let out = generate(&base, 7, &Sizes { distractors: 5 }).unwrap();
        for prefix in ["s-dis-", "s-lex-", "s-mr-", "s-hi-"] {
            assert!(synthetic_rows(&out, &base).iter().any(|r| r.ref_.starts_with(prefix)), "{prefix}");
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
