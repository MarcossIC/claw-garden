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
use vocab::{Group, Terms, Vocab};

pub const DEFAULT_DISTRACTORS: usize = 500;

const MAX_CHAINED_FRAMES: usize = 3;
/// Words in a row that a distractor must not repeat from a base row.
const COPY_RUN_WORDS: usize = 6;
const MAX_COPY_RETRIES: usize = 50;
const LEXICAL_FAMILIES: usize = 30;
const SIBLINGS_PER_FAMILY: usize = 8;
const MULTI_REF_TOPICS: usize = 12;
/// Cases whose answer is the identifier's row and the rows on its topic.
const RELATED_FAMILIES: usize = 30;
/// Rows in a related family that carry a near identifier and another topic.
const RELATED_SIBLINGS: usize = 4;
/// Cases per direction of the importance split.
const CONTESTED_CASES: usize = 12;
/// Rows that name the same entity in a contested case: the answer and five rivals.
const CONTESTED_POOL: usize = 6;
/// Cases with one long memory each, half of them in Spanish.
const LONG_CASES: usize = 12;
/// Pieces a long row reaches without a tokenizer. One more than the three the
/// set promises, so the fact placed two thirds in stays past the first piece
/// even where the real tokenizer packs more text into a piece.
const LONG_MIN_PIECES: usize = 4;
/// The piece cap the long rows are sized against: the one production split a
/// long save at when the subset was defined. Production's cap can move; the
/// rows that measure it must not move with it.
const LONG_PIECE_TOKENS: usize = 424;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum SynthError {
    #[error("ref '{0}' appears more than once")]
    DuplicateRef(String),
    #[error("query '{0}' appears more than once; `rag-eval --diff` joins cases on the query text")]
    DuplicateQuery(String),
    #[error("could not build distractor '{0}' without repeating a run of a base row")]
    DistractorCopiesBase(String),
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
    out.corpus.extend(distractors(base, &vocab, &mut rng, sizes.distractors)?);
    let (rows, cases) = lexical_families(base, &mut rng);
    out.corpus.extend(rows);
    out.retrieval_cases.extend(cases);

    let mut names = Codenames::default();
    let (rows, cases) = multi_ref_topics(base, &mut names, &mut rng);
    out.corpus.extend(rows);
    out.retrieval_cases.extend(cases);
    let (rows, cases) = contested_importance(base, &mut names, &mut rng);
    out.corpus.extend(rows);
    out.retrieval_cases.extend(cases);
    // Last, so the draws above, and everything they generate, stay as they
    // were before long cases existed.
    let (rows, cases) = long_saves(base, &mut names, &mut rng);
    out.corpus.extend(rows);
    out.retrieval_cases.extend(cases);
    // After long cases for the same reason.
    let (rows, cases) = lexical_related(base, &out.corpus[base.corpus.len()..], &mut names, &mut rng);
    out.corpus.extend(rows);
    out.retrieval_cases.extend(cases);

    check_unique(&out)?;
    Ok(out)
}

/// Rows that sit near real ones without answering anything: each mixes the
/// identifiers and terms of two base rows of the same type and language, and
/// runs as long as the rows it imitates so length does not give it away.
fn distractors(base: &Golden, vocab: &Vocab, rng: &mut SplitMix64, count: usize) -> Result<Vec<Row>, SynthError> {
    let base_windows: HashSet<String> = base.corpus.iter().flat_map(|r| word_windows(&r.content)).collect();
    let mut out = Vec::with_capacity(count);
    for n in 0..count {
        let ref_ = format!("s-dis-{:04}", n + 1);
        let content = (0..MAX_COPY_RETRIES)
            .find_map(|_| {
                let (g, first) = *rng.pick(&vocab.rows);
                let group = &vocab.groups[g];
                let second = rng.below(group.rows.len());
                let (content, _) = compose_distractor(rng, group, &group.rows[first], &group.rows[second]);
                (!shares_word_run(&content, &base_windows)).then_some((g, content))
            })
            .ok_or_else(|| SynthError::DistractorCopiesBase(ref_.clone()))?;
        let group = &vocab.groups[content.0];
        out.push(Row {
            ref_,
            content: content.1,
            memory_type: group.memory_type.clone(),
            importance: importance_in(rng, group.importance_lo, group.importance_hi),
            lang: group.lang.clone(),
        });
    }
    Ok(out)
}

/// Chains frames over the same pair of rows until the text is as long as the
/// group's median row or three frames are in; returns the text and how many
/// frames it took.
fn compose_distractor(rng: &mut SplitMix64, group: &Group, r: &Terms, r2: &Terms) -> (String, usize) {
    let spanish = group.lang.as_deref() == Some("es");
    let fallback = templates::fallback_word(spanish);
    let frames = templates::distractor_frames(&group.memory_type, spanish);
    let start = rng.below(frames.len());
    let mut content = String::new();
    let mut used = 0;
    while used < MAX_CHAINED_FRAMES {
        let a = pick_term(rng, &r.idents, &r.words, fallback);
        let b = pick_term(rng, &r2.idents, &r2.words, fallback);
        let w1 = pick_term(rng, &r.words, &r2.words, fallback);
        let w2 = pick_term(rng, &r2.words, &r.words, fallback);
        let w3 = pick_term(rng, &r.words, &r2.words, fallback);
        if used > 0 {
            content.push(' ');
        }
        content.push_str(&templates::fill(
            frames[(start + used) % frames.len()],
            &[("{a}", a), ("{b}", b), ("{w1}", w1), ("{w2}", w2), ("{w3}", w3)],
        ));
        used += 1;
        if content.chars().count() >= group.median_len {
            break;
        }
    }
    (content, used)
}

/// Every run of `COPY_RUN_WORDS` consecutive words in `text`, lowercased and
/// stripped of punctuation.
fn word_windows(text: &str) -> Vec<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    words.windows(COPY_RUN_WORDS).map(|w| w.join(" ")).collect()
}

fn shares_word_run(text: &str, base_windows: &HashSet<String>) -> bool {
    word_windows(text).iter().any(|w| base_windows.contains(w))
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
        let taken = |candidate: &str| {
            used.iter().any(|u| u.contains(candidate) || candidate.contains(u.as_str()))
                || base.corpus.iter().any(|r| r.content.contains(candidate))
        };
        let (seed_ident, near) = identifier_family(kind, SIBLINGS_PER_FAMILY - 1, rng, &taken);
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
            answer: None,
            query_lang: None,
        });
    }
    (rows, cases)
}

/// A `{stem}{digits}` identifier of `kind` that is not `taken`, and `near`
/// others a few digits away from it, none of them `taken` either.
fn identifier_family(
    kind: IdentKind,
    near: usize,
    rng: &mut SplitMix64,
    taken: &dyn Fn(&str) -> bool,
) -> (String, Vec<String>) {
    let stem = match kind {
        IdentKind::Function => format!(
            "{}{}",
            rng.pick(&templates::FUNCTION_VERBS),
            rng.pick(&templates::FUNCTION_NOUNS)
        ),
        IdentKind::ErrorCode => format!("E_{}_", rng.pick(&templates::ERROR_AREAS)),
    };
    loop {
        let digits = (1000 + rng.below(9000)).to_string();
        let seed_ident = format!("{stem}{digits}");
        if taken(&seed_ident) {
            continue;
        }
        let others = near_identifiers(&stem, &digits, near, rng, taken);
        if others.len() == near {
            return (seed_ident, others);
        }
    }
}

/// Families of eight rows asked about one identifier, where the answer is
/// more than the row that holds it: three rows on the same topic, named by a
/// codename, say the rest without naming the identifier, and four rows carry
/// identifiers a few digits away and talk about something else. What it
/// measures is whether the rows about the topic still reach the agent once
/// the identifier's row takes the lexical bonus.
fn lexical_related(
    base: &Golden,
    generated: &[Row],
    names: &mut Codenames,
    rng: &mut SplitMix64,
) -> (Vec<Row>, Vec<Case>) {
    let mut used: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    let mut cases = Vec::new();

    for family in 0..RELATED_FAMILIES {
        let kind = if family % 2 == 0 { IdentKind::Function } else { IdentKind::ErrorCode };
        let spanish = (family / 2) % 2 == 1;
        let lang = if spanish { "es" } else { "en" };
        let topic = &templates::related_topics(kind, spanish)[(family / 4) % 2];
        let sibling = rng.pick(templates::lexical_frames(kind, spanish));
        let key = names.next(base, rng);
        let taken = |candidate: &str| {
            used.iter().any(|u| u.contains(candidate) || candidate.contains(u.as_str()))
                || base.corpus.iter().chain(generated).any(|r| r.content.contains(candidate))
        };
        let (ident, near) = identifier_family(kind, RELATED_SIBLINGS, rng, &taken);
        used.push(ident.clone());
        used.extend(near.iter().cloned());

        let values = [("{id}", ident.as_str()), ("{key}", key.as_str())];
        let mut texts: Vec<(String, &str)> = vec![(templates::fill(topic.exact, &values), topic.exact_type)];
        texts.extend(topic.related.iter().map(|r| (templates::fill(r, &values), "semantic_fact")));
        texts.extend(near.iter().map(|id| (templates::fill(sibling.content, &[("{id}", id)]), sibling.memory_type)));

        let refs: Vec<String> = (0..texts.len()).map(|j| format!("s-lr-{family:02}-{j}")).collect();
        for (ref_, (content, memory_type)) in refs.iter().zip(texts) {
            rows.push(Row {
                ref_: ref_.clone(),
                content,
                memory_type: memory_type.to_string(),
                importance: 5.0,
                lang: Some(lang.to_string()),
            });
        }
        cases.push(Case {
            query: templates::fill(topic.query, &values),
            expected_refs: refs[..1 + topic.related.len()].to_vec(),
            corpus_refs: refs,
            status: Status::Active,
            blocked_by: None,
            history: Vec::new(),
            subset: Subset::LexicalRelated,
            answer: None,
            query_lang: Some(lang.to_string()),
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

/// Entities that many rows talk about: each key gets one row per aspect
/// (retries, timeouts, deployment, ...), and the question asks for two or
/// three of them. The other rows name the same key, so they compete with the
/// answers and are wrong only because they cover a different aspect.
fn multi_ref_topics(base: &Golden, names: &mut Codenames, rng: &mut SplitMix64) -> (Vec<Row>, Vec<Case>) {
    let mut rows = Vec::new();
    let mut cases = Vec::new();

    for topic in 0..MULTI_REF_TOPICS {
        let key = names.next(base, rng);
        let spanish = topic % 2 == 1;
        let catalog = templates::aspect_catalog(spanish);
        let queries = templates::multi_aspect_queries(spanish);
        let (asked, query) = queries[topic % queries.len()];

        let refs: Vec<String> = (0..catalog.len()).map(|k| format!("s-mr-{topic:02}-{k}")).collect();
        for (ref_, (content, _)) in refs.iter().zip(catalog) {
            rows.push(Row {
                ref_: ref_.clone(),
                content: templates::fill(content, &[("{key}", &key)]),
                memory_type: "semantic_fact".to_string(),
                importance: 5.0,
                lang: Some(if spanish { "es" } else { "en" }.to_string()),
            });
        }
        cases.push(Case {
            query: templates::fill(query, &[("{key}", &key)]),
            expected_refs: asked.iter().map(|&k| refs[k].clone()).collect(),
            corpus_refs: refs,
            status: Status::Active,
            blocked_by: None,
            history: Vec::new(),
            subset: Subset::MultiRef,
            answer: None,
            query_lang: None,
        });
    }
    (rows, cases)
}

/// Which side of the importance split a contested case defends.
#[derive(Clone, Copy)]
enum Defends {
    High,
    Low,
}

/// Entities with six rows each, one per aspect, asked about one aspect. The
/// row that answers sits at one end of the importance scale and its five
/// rivals at the other, so which one ranks first shows whether importance
/// moves the order, in both directions.
fn contested_importance(base: &Golden, names: &mut Codenames, rng: &mut SplitMix64) -> (Vec<Row>, Vec<Case>) {
    let mut rows = Vec::new();
    let mut cases = Vec::new();

    for (defends, prefix, subset) in [
        (Defends::High, "s-hi", Subset::HighImportance),
        (Defends::Low, "s-lo", Subset::LowImportance),
    ] {
        for n in 0..CONTESTED_CASES {
            let key = names.next(base, rng);
            let spanish = n % 2 == 1;
            let catalog = templates::aspect_catalog(spanish);
            let target = (n * 3) % catalog.len();
            let refs: Vec<String> = (0..CONTESTED_POOL).map(|j| format!("{prefix}-{n:02}-{j}")).collect();

            for (j, ref_) in refs.iter().enumerate() {
                let answers = j == 0;
                let importance = match (defends, answers) {
                    (Defends::High, true) => 8.0 + (n % 3) as f64,
                    (Defends::High, false) => 3.0 + (j % 3) as f64,
                    (Defends::Low, true) => 3.0 + (n % 3) as f64,
                    (Defends::Low, false) => 8.0 + (j % 3) as f64,
                };
                rows.push(Row {
                    ref_: ref_.clone(),
                    content: templates::fill(catalog[(target + j) % catalog.len()].0, &[("{key}", &key)]),
                    memory_type: "user_preference".to_string(),
                    importance,
                    lang: Some(if spanish { "es" } else { "en" }.to_string()),
                });
            }
            cases.push(Case {
                query: templates::fill(catalog[target].1, &[("{key}", &key)]),
                expected_refs: vec![refs[0].clone()],
                corpus_refs: refs,
                status: Status::Active,
                blocked_by: None,
                history: Vec::new(),
                subset,
                answer: None,
                query_lang: None,
            });
        }
    }
    (rows, cases)
}

/// Memories long enough that production cuts them into several pieces, each
/// asked about a fact placed well past the first piece while only the opening
/// sentence names the key. Whether the agent gets the answer then depends on
/// how the later pieces are found and put back together.
fn long_saves(base: &Golden, names: &mut Codenames, rng: &mut SplitMix64) -> (Vec<Row>, Vec<Case>) {
    let mut rows = Vec::new();
    let mut cases = Vec::new();

    for n in 0..LONG_CASES {
        let key = names.next(base, rng);
        let spanish = n % 2 == 1;
        let facts = templates::long_facts(spanish);
        let (sentence, answer, query) = facts[(n / 2) % facts.len()];
        let number = (10 + rng.below(90)).to_string();
        let opening = templates::fill(rng.pick(templates::long_openings(spanish)), &[("{key}", &key)]);

        let mut sentences = vec![opening];
        while clawg_core::eval_api::fallback_pieces(&sentences.join(" "), LONG_PIECE_TOKENS).len() < LONG_MIN_PIECES {
            let frame = *rng.pick(templates::long_filler(spanish));
            let count = (2 + rng.below(98)).to_string();
            let topic = *rng.pick(templates::long_topics(spanish));
            sentences.push(templates::fill(frame, &[("{n}", &count), ("{w}", topic)]));
        }
        let at = (sentences.len() * 2).div_ceil(3);
        sentences.insert(at, templates::fill(sentence, &[("{n}", &number)]));

        let ref_ = format!("s-long-{n:02}");
        rows.push(Row {
            ref_: ref_.clone(),
            content: sentences.join(" "),
            memory_type: "semantic_fact".to_string(),
            importance: 5.0,
            lang: Some(if spanish { "es" } else { "en" }.to_string()),
        });
        cases.push(Case {
            query: templates::fill(query, &[("{key}", &key)]),
            corpus_refs: vec![ref_.clone()],
            expected_refs: vec![ref_],
            status: Status::Active,
            blocked_by: None,
            history: Vec::new(),
            subset: Subset::Long,
            answer: Some(templates::fill(answer, &[("{n}", &number)])),
            query_lang: None,
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
            answer: None,
            query_lang: None,
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

    fn long_group_base() -> Golden {
        let long = "The OrderService owns pricing rules and delegates persistence to `OrderRepository`;             the gateway keeps sessions in `SessionStore` and refreshes them every few minutes.";
        Golden {
            corpus: vec![
                rich_row("c01", "architectural_decision", "en", 6.0, long),
                rich_row("c02", "architectural_decision", "en", 7.0, long),
                rich_row("c03", "tool_error", "en", 4.0, "Falló `x`."),
            ],
            retrieval_cases: vec![],
            contradiction_pairs: vec![],
        }
    }

    #[test]
    fn a_distractor_reaches_its_groups_median_length_or_chains_three_frames() {
        let base = long_group_base();
        let vocab = Vocab::from_rows(&base.corpus);
        let mut rng = SplitMix64::new(7);
        for group in &vocab.groups {
            for _ in 0..50 {
                let (content, frames) = compose_distractor(&mut rng, group, &group.rows[0], &group.rows[group.rows.len() - 1]);
                assert!((1..=3).contains(&frames));
                assert!(content.chars().count() >= group.median_len || frames == 3, "{content}");
            }
        }
    }

    #[test]
    fn a_short_group_needs_one_frame_and_a_long_one_chains_more() {
        let base = long_group_base();
        let vocab = Vocab::from_rows(&base.corpus);
        let mut rng = SplitMix64::new(7);
        let frames_of = |group: &vocab::Group, rng: &mut SplitMix64| {
            compose_distractor(rng, group, &group.rows[0], &group.rows[0]).1
        };
        let long = vocab.groups.iter().find(|g| g.memory_type == "architectural_decision").unwrap();
        let short = vocab.groups.iter().find(|g| g.memory_type == "tool_error").unwrap();
        assert!((0..20).all(|_| frames_of(short, &mut rng) == 1));
        assert!((0..20).any(|_| frames_of(long, &mut rng) >= 2));
    }

    #[test]
    fn a_run_of_six_words_is_detected_case_and_punctuation_insensitively() {
        let windows = word_windows("The quick, brown fox jumps over the lazy dog");
        let base: HashSet<String> = windows.into_iter().collect();
        assert!(shares_word_run("Yesterday the QUICK brown fox jumps over it", &base));
        assert!(!shares_word_run("the quick brown fox leaps over the lazy dog", &base));
        assert!(!shares_word_run("too short", &base));
    }

    #[test]
    fn no_distractor_shares_six_words_in_a_row_with_a_base_row() {
        let base = grouped_base();
        let out = generate(&base, 7, &Sizes { distractors: 300 }).unwrap();
        let windows: HashSet<String> = base.corpus.iter().flat_map(|r| word_windows(&r.content)).collect();
        for d in distractors(&out, &base) {
            assert!(!shares_word_run(&d.content, &windows), "{}", d.content);
        }
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
            assert!((2..=3).contains(&case.expected_refs.len()), "{}", case.query);
            for r in &case.expected_refs {
                assert!(case.corpus_refs.contains(r), "{}", case.query);
                row_of(&out, r);
            }
        }
    }

    #[test]
    fn a_multi_reference_key_names_eight_rows_and_only_some_of_them_answer() {
        let base = grouped_base();
        let out = generate(&base, 7, &Sizes { distractors: 100 }).unwrap();
        let mut keys = HashSet::new();
        for case in cases_of(&out, Subset::MultiRef) {
            let key = ident_of(row_of(&out, &case.expected_refs[0]));
            assert!(case.query.contains(key), "{} lacks {key}", case.query);
            assert!(keys.insert(key.to_string()), "{key} is reused");
            assert!(!base.corpus.iter().any(|r| r.content.contains(key)));

            let holders: Vec<&str> =
                out.corpus.iter().filter(|r| r.content.contains(key)).map(|r| r.ref_.as_str()).collect();
            assert_eq!(holders.len(), 8, "{key}");
            assert_eq!(case.corpus_refs.len(), 8, "{key}");
            assert!(holders.iter().all(|h| case.corpus_refs.iter().any(|r| r == h)), "{key}");
            assert!(case.expected_refs.len() < holders.len());
        }
    }

    #[test]
    fn the_rows_of_a_multi_reference_key_cover_different_aspects() {
        let out = generate(&grouped_base(), 7, &NONE).unwrap();
        for case in cases_of(&out, Subset::MultiRef) {
            let contents: HashSet<&str> =
                case.corpus_refs.iter().map(|r| row_of(&out, r).content.as_str()).collect();
            assert_eq!(contents.len(), 8, "{}", case.query);
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

    /// The catalog row and question a contested case is about, found from the
    /// expected row's content.
    fn aspect_of(row: &Row, key: &str) -> (usize, bool) {
        let spanish = row.lang.as_deref() == Some("es");
        let index = templates::aspect_catalog(spanish)
            .iter()
            .position(|(content, _)| templates::fill(content, &[("{key}", key)]) == row.content)
            .unwrap_or_else(|| panic!("not a catalog row: {}", row.content));
        (index, spanish)
    }

    fn check_contested(out: &Golden, subset: Subset, expected_range: (f64, f64), others_range: (f64, f64)) {
        let cases = cases_of(out, subset);
        assert_eq!(cases.len(), 12, "{subset:?}");
        for case in cases {
            assert_eq!(case.expected_refs.len(), 1);
            let expected = row_of(out, &case.expected_refs[0]);
            let key = ident_of(expected);
            assert!((expected_range.0..=expected_range.1).contains(&expected.importance), "{}", case.query);

            let (aspect, spanish) = aspect_of(expected, key);
            let question = templates::aspect_catalog(spanish)[aspect].1;
            assert_eq!(case.query, templates::fill(question, &[("{key}", key)]));

            let holders: Vec<&Row> = out.corpus.iter().filter(|r| r.content.contains(key)).collect();
            assert_eq!(holders.len(), 6, "{key}");
            assert_eq!(case.corpus_refs.len(), 6);
            assert!(holders.iter().all(|h| case.corpus_refs.contains(&h.ref_)));
            let contents: HashSet<&str> = holders.iter().map(|h| h.content.as_str()).collect();
            assert_eq!(contents.len(), 6, "{key} repeats an aspect");
            for other in holders.iter().filter(|h| h.ref_ != expected.ref_) {
                assert!((others_range.0..=others_range.1).contains(&other.importance), "{key}: {other:?}");
            }
        }
    }

    #[test]
    fn each_high_importance_case_expects_a_high_row_among_low_ones_on_the_same_entity() {
        let out = generate(&grouped_base(), 7, &Sizes { distractors: 50 }).unwrap();
        check_contested(&out, Subset::HighImportance, (8.0, 10.0), (3.0, 5.0));
    }

    #[test]
    fn each_low_importance_case_expects_a_low_row_among_high_ones_on_the_same_entity() {
        let out = generate(&grouped_base(), 7, &Sizes { distractors: 50 }).unwrap();
        check_contested(&out, Subset::LowImportance, (3.0, 5.0), (8.0, 10.0));
    }

    #[test]
    fn the_importance_cases_use_keys_the_other_subsets_do_not() {
        let out = generate(&grouped_base(), 7, &NONE).unwrap();
        let mut keys = HashSet::new();
        for subset in [Subset::MultiRef, Subset::HighImportance, Subset::LowImportance] {
            for case in cases_of(&out, subset) {
                let key = ident_of(row_of(&out, &case.expected_refs[0])).to_string();
                assert!(keys.insert(key.clone()), "{key} is shared between cases");
            }
        }
    }

    #[test]
    fn the_synthetic_refs_keep_their_prefixes() {
        let base = grouped_base();
        let out = generate(&base, 7, &Sizes { distractors: 5 }).unwrap();
        for prefix in ["s-dis-", "s-lex-", "s-mr-", "s-hi-", "s-lo-", "s-long-"] {
            assert!(synthetic_rows(&out, &base).iter().any(|r| r.ref_.starts_with(prefix)), "{prefix}");
        }
    }

    fn long_cases(out: &Golden) -> Vec<(&Case, &Row)> {
        cases_of(out, Subset::Long).into_iter().map(|c| (c, row_of(out, &c.expected_refs[0]))).collect()
    }

    fn key_of(row: &Row) -> &str {
        row.content.split('`').nth(1).expect("a long row names its key in backticks")
    }

    #[test]
    fn there_are_twelve_long_cases_at_any_size() {
        for distractors in [0, 40] {
            let out = generate(&grouped_base(), 7, &Sizes { distractors }).unwrap();
            let long = long_cases(&out);
            assert_eq!(long.len(), 12);
            assert_eq!(long.iter().filter(|(_, r)| r.lang.as_deref() == Some("es")).count(), 6);
            for (case, row) in long {
                assert_eq!(case.expected_refs, vec![row.ref_.clone()]);
                assert!(case.answer.is_some(), "{case:?}");
            }
        }
    }

    #[test]
    fn a_long_row_names_its_key_only_in_its_first_sentence() {
        let out = generate(&grouped_base(), 7, &NONE).unwrap();
        for (case, row) in long_cases(&out) {
            let key = key_of(row);
            assert!(case.query.contains(key), "{case:?}");
            let first_end = row.content.find(". ").expect("a long row has more than one sentence") + 1;
            assert_eq!(row.content.matches(key).count(), 1, "{key}");
            assert!(row.content[..first_end].contains(key), "{key}");
            assert_eq!(out.corpus.iter().filter(|r| r.content.contains(key)).count(), 1, "{key} is shared");
        }
    }

    #[test]
    fn the_answer_appears_once_after_the_first_third() {
        let out = generate(&grouped_base(), 7, &NONE).unwrap();
        let mut answers = HashSet::new();
        for (case, row) in long_cases(&out) {
            let answer = case.answer.as_deref().unwrap();
            assert_eq!(row.content.matches(answer).count(), 1, "{answer}");
            let at = row.content.find(answer).unwrap();
            assert!(at > row.content.len() / 3, "{answer} sits at byte {at} of {}", row.content.len());
            let first = &clawg_core::eval_api::fallback_pieces(&row.content, LONG_PIECE_TOKENS)[0];
            assert!(!first.contains(answer), "{answer} is already in the first piece");
            assert_eq!(out.corpus.iter().filter(|r| r.content.contains(answer)).count(), 1, "{answer}");
            assert!(answers.insert(answer.to_string()), "{answer} repeats");
        }
    }

    #[test]
    fn a_long_row_splits_into_at_least_three_pieces() {
        let out = generate(&grouped_base(), 7, &NONE).unwrap();
        let mut pieces = HashSet::new();
        for (_, row) in long_cases(&out) {
            let split = clawg_core::eval_api::fallback_pieces(&row.content, LONG_PIECE_TOKENS);
            assert!(split.len() >= 3, "{} has {} pieces", row.ref_, split.len());
            for piece in split {
                assert!(pieces.insert(piece), "{} shares a piece with another long row", row.ref_);
            }
        }
        let base = grouped_base();
        let windows: HashSet<String> = base.corpus.iter().flat_map(|r| word_windows(&r.content)).collect();
        for (_, row) in long_cases(&out) {
            assert!(!shares_word_run(&row.content, &windows), "{}", row.ref_);
        }
    }

    /// The digest of the set seed 7 gave before long cases existed. Long cases
    /// draw from the generator after everything else, so removing them must
    /// give back that exact set, byte for byte.
    #[test]
    fn adding_long_cases_leaves_the_rest_of_the_set_unchanged() {
        let mut out = generate(&grouped_base(), 7, &Sizes { distractors: 40 }).unwrap();
        out.corpus.retain(|r| !r.ref_.starts_with("s-long-") && !r.ref_.starts_with("s-lr-"));
        out.retrieval_cases.retain(|c| c.subset != Subset::Long && c.subset != Subset::LexicalRelated);
        let digest = clawg_core::eval_api::sha256_hex(serde_json::to_string(&out).unwrap().as_bytes());
        assert_eq!(digest, "f05a66281b5db6008277aee3fec78691b881f58f02b14102e37d401f5ebc1d27");
    }

    /// The long rows seed 7 gives, pinned so that moving the cap production
    /// splits a save at does not move the set that measures it.
    #[test]
    fn long_rows_do_not_depend_on_the_piece_cap() {
        let out = generate(&grouped_base(), 7, &NONE).unwrap();
        let long: Vec<&Row> = out.corpus.iter().filter(|r| r.ref_.starts_with("s-long-")).collect();
        let digest = clawg_core::eval_api::sha256_hex(serde_json::to_string(&long).unwrap().as_bytes());
        assert_eq!(digest, "ea1ac4ed4ce564b3632e6b1593ba7ef9d32b099b2e98b8329325f6fbf85beff9");
    }

    /// The digest of the set seed 7 gave before related cases existed. They
    /// draw from the generator after everything else, so removing them must
    /// give back that exact set, byte for byte.
    #[test]
    fn the_other_subsets_do_not_change_when_lexical_related_is_added() {
        let mut out = generate(&grouped_base(), 7, &Sizes { distractors: 40 }).unwrap();
        out.corpus.retain(|r| !r.ref_.starts_with("s-lr-"));
        out.retrieval_cases.retain(|c| c.subset != Subset::LexicalRelated);
        let digest = clawg_core::eval_api::sha256_hex(serde_json::to_string(&out).unwrap().as_bytes());
        assert_eq!(digest, "d3f2cdd3047ae3ed010199302d935d8911305885090412ff29f8c69c1718c2c4");
    }

    #[test]
    fn there_are_thirty_lexical_related_cases() {
        for distractors in [0, 500] {
            let out = generate(&grouped_base(), 7, &Sizes { distractors }).unwrap();
            assert_eq!(cases_of(&out, Subset::LexicalRelated).len(), 30, "distractors={distractors}");
        }
    }

    /// The identifier of a related case: the token of its query that its
    /// first expected row holds in backticks.
    fn related_ident<'a>(out: &'a Golden, case: &Case) -> &'a str {
        ident_of(row_of(out, &case.expected_refs[0]))
    }

    #[test]
    fn a_lexical_related_case_expects_the_identifier_row_and_three_related_rows() {
        let out = generate(&grouped_base(), 7, &NONE).unwrap();
        for case in cases_of(&out, Subset::LexicalRelated) {
            assert_eq!((case.corpus_refs.len(), case.expected_refs.len()), (8, 4), "{}", case.query);
            assert!(case.expected_refs.iter().all(|r| case.corpus_refs.contains(r)));
            let ident = related_ident(&out, case);
            assert!(case.query.contains(ident), "{} lacks {ident}", case.query);
            let lang = row_of(&out, &case.expected_refs[0]).lang.clone();
            assert_eq!(case.query_lang, lang);
            let key = case.query.split_whitespace().find(|w| w.contains('-')).expect("a codename");
            for r in &case.corpus_refs {
                let row = row_of(&out, r);
                assert_eq!(row.lang, lang);
                assert_eq!(row.content.contains(key), case.expected_refs.contains(r), "{key} in {}", row.content);
            }
        }
    }

    #[test]
    fn only_the_identifier_row_of_a_related_case_holds_its_identifier() {
        let out = generate(&grouped_base(), 7, &Sizes { distractors: 200 }).unwrap();
        for case in cases_of(&out, Subset::LexicalRelated) {
            let ident = related_ident(&out, case);
            let holders: Vec<&str> =
                out.corpus.iter().filter(|r| r.content.contains(ident)).map(|r| r.ref_.as_str()).collect();
            assert_eq!(holders, [case.expected_refs[0].as_str()]);
            let split = ident.len() - 4;
            let (stem, digits) = ident.split_at(split);
            for r in &case.corpus_refs[4..] {
                let sibling = ident_of(row_of(&out, r));
                let tail = sibling.strip_prefix(stem).unwrap_or_else(|| panic!("{sibling} vs {stem}"));
                assert!(digits_are_near(digits, tail), "{sibling} vs {ident}");
            }
        }
    }

    #[test]
    fn lexical_related_identifiers_are_unique_and_new() {
        let out = generate(&grouped_base(), 7, &NONE).unwrap();
        let lexical: Vec<&str> = out.corpus.iter().filter(|r| r.ref_.starts_with("s-lex-")).map(ident_of).collect();
        let related: Vec<&str> = out.corpus.iter().filter(|r| r.ref_.starts_with("s-lr-")).filter_map(|r| r.content.split('`').nth(1)).filter(|i| !i.contains('-')).collect();
        assert_eq!(related.len(), 30 * 5, "one identifier per related row and one per sibling");
        let unique: HashSet<&str> = related.iter().copied().collect();
        assert_eq!(unique.len(), related.len());
        for ident in &related {
            assert!(lexical.iter().all(|l| !l.contains(ident) && !ident.contains(l)), "{ident}");
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
