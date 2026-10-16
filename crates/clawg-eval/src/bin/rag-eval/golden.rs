//! Golden-set schema, loader, and validator for the retrieval-evaluation
//! harness. Consumes `docs/eval/golden.json` (or an override path), parses it
//! strictly, and validates it end to end BEFORE any row is seeded into a
//! store — a malformed or structurally impossible golden file must surface as
//! a load-time abort, not as a retrieval number nobody can later trust.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The `MemoryType` serde names a corpus row may declare, copied from
/// `store_insert.rs`'s `#[serde(rename_all = "snake_case")]` enum rather than
/// imported. `procedural_skill` is deliberately excluded: it lives in a
/// separate table not reachable through the insert path this harness uses, so
/// a golden-set row can never legitimately claim it.
const VALID_MEMORY_TYPES: [&str; 7] = [
    "chat_message",
    "tool_error",
    "code_block",
    "session_summary",
    "user_preference",
    "architectural_decision",
    "semantic_fact",
];

/// The parsed, not-yet-validated shape of `docs/eval/golden.json`.
/// `deny_unknown_fields` throughout: a typo'd key must be an error, never a
/// silently ignored expectation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenSet {
    pub corpus: Vec<CorpusRow>,
    pub retrieval_cases: Vec<RetrievalCase>,
    pub contradiction_pairs: Vec<ContradictionPair>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorpusRow {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub content: String,
    /// Kept as the raw on-disk string rather than parsed into an enum: it is
    /// handed straight to the production insert path, so there is only one
    /// string in play and it can never drift from what actually gets seeded.
    pub memory_type: String,
    pub importance: f64,
    #[serde(default)]
    pub lang: Option<String>,
    /// How many days before the run the row was saved. Relative, so a set
    /// written today does not age: a fixed date would let decay move the
    /// numbers from one month to the next.
    #[serde(default)]
    pub age_days: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetrievalCase {
    pub query: String,
    pub corpus_refs: Vec<String>,
    pub expected_refs: Vec<String>,
    #[serde(default)]
    pub status: CaseStatus,
    #[serde(default)]
    pub blocked_by: Option<String>,
    /// Prior turns for the `--follow-up` synthetic mode, chronological
    /// (oldest-first), as a human would write them. `#[serde(default)]` so
    /// every case in the current `golden.json` — which has no `history`
    /// key at all — still parses, with `history == []`; `deny_unknown_fields`
    /// only rejects keys present IN the file, so adding a known optional
    /// field here only widens what parses, it never narrows it.
    #[serde(default)]
    pub history: Vec<String>,
    /// Which slice of the set this case belongs to, so the report can split
    /// metrics by it. Absent means `natural`, which keeps the hand-written
    /// `golden.json` parsing unchanged.
    #[serde(default)]
    pub subset: CaseSubset,
    /// The fact the case asks for, exactly as its expected row states it. Only
    /// cases about long memories carry one: a fragment of such a row can be
    /// visible without the part that answers.
    #[serde(default)]
    pub answer: Option<String>,
    /// Only in an `update` case: older rows stating the same fact the
    /// expected row restates.
    #[serde(default)]
    pub stale_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseSubset {
    #[default]
    Natural,
    Lexical,
    MultiRef,
    HighImportance,
    LowImportance,
    Long,
    /// A newer row restates a fact older rows also state; it should come first.
    Update,
    /// No row answers the query: what is measured is whether the search
    /// returns nothing, so the case stays out of every ranked metric.
    Unanswerable,
}

impl CaseSubset {
    /// The name this subset serializes as, used to order and label report
    /// buckets without a round trip through serde.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Natural => "natural",
            Self::Lexical => "lexical",
            Self::MultiRef => "multi_ref",
            Self::HighImportance => "high_importance",
            Self::LowImportance => "low_importance",
            Self::Long => "long",
            Self::Update => "update",
            Self::Unanswerable => "unanswerable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    #[default]
    Active,
    KnownRed,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContradictionPair {
    pub a: String,
    pub b: String,
    pub label: PairLabel,
    pub lang: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PairLabel {
    Contradiction,
    NoContradiction,
    Identical,
}

/// Bidirectional map between a golden-set row's stable `ref` string and the
/// autoincrement id SQLite assigns it once seeded. Ids only exist after
/// seeding into a fresh-per-run database, so every score and error message
/// downstream stays in ref-space: a search hit's id is translated back to its
/// `ref` through this map rather than letting a bare integer leak into the
/// metrics or reporting layers.
#[derive(Debug, Default)]
pub struct RefMap {
    to_id: HashMap<String, i64>,
    to_ref: HashMap<i64, String>,
}

impl RefMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that `r` was seeded as `id`. Called once per corpus row during
    /// seeding — never during validation, which runs before any row exists in
    /// a database.
    pub fn insert(&mut self, r: impl Into<String>, id: i64) {
        let r = r.into();
        self.to_ref.insert(id, r.clone());
        self.to_id.insert(r, id);
    }

    /// Exists for the seeding tests, which use it to prove every corpus row
    /// came back with its own distinct id — the property the whole ref-space
    /// translation rests on, and the one thing `ref_of` alone cannot check
    /// (it can only answer questions about ids you already hold). Scoring
    /// itself only ever travels id → ref, so this direction is unused outside
    /// tests by design rather than by oversight.
    #[cfg(test)]
    pub fn id_of(&self, r: &str) -> Option<i64> {
        self.to_id.get(r).copied()
    }

    pub fn ref_of(&self, id: i64) -> Option<&str> {
        self.to_ref.get(&id).map(String::as_str)
    }
}

/// Everything that can make a golden set unusable, each carrying the exact
/// message an author needs in order to fix the file — printed verbatim, never
/// wrapped, so the code and the text a human sees are the same string.
#[derive(Debug, thiserror::Error)]
pub enum GoldenError {
    #[error("failed to read golden set file '{path}': {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse golden set JSON in '{path}': {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("E_DUPLICATE_REF: corpus ref '{0}' appears more than once")]
    DuplicateRef(String),

    #[error(
        "E_UNKNOWN_MEMORY_TYPE: corpus ref '{ref_}' has memory_type '{memory_type}', expected one of chat_message|tool_error|code_block|session_summary|user_preference|architectural_decision|semantic_fact"
    )]
    UnknownMemoryType { ref_: String, memory_type: String },

    #[error(
        "E_UNRESOLVED_REF: retrieval_case '{query}' references corpus ref '{corpus_ref}' which does not exist in corpus"
    )]
    UnresolvedRef { query: String, corpus_ref: String },

    #[error(
        "E_EXPECTED_NOT_IN_POOL: retrieval_case '{query}' expects '{expected_ref}' which is not listed in corpus_refs"
    )]
    ExpectedNotInPool { query: String, expected_ref: String },

    #[error("E_MISSING_BLOCKED_BY: retrieval_case '{0}' is marked known_red but has no blocked_by")]
    MissingBlockedBy(String),

    #[error("E_NO_EXPECTED: retrieval_case '{0}' expects no row; only an unanswerable case may")]
    NoExpected(String),

    #[error("E_UNANSWERABLE_EXPECTS: retrieval_case '{0}' is unanswerable but lists expected_refs")]
    UnanswerableExpects(String),

    #[error("E_NEGATIVE_AGE: corpus ref '{0}' has a negative age_days")]
    NegativeAge(String),

    #[error("E_STALE_OUTSIDE_UPDATE: retrieval_case '{0}' lists stale_refs but is not an update case")]
    StaleOutsideUpdate(String),

    #[error(
        "E_UPDATE_SHAPE: update case '{0}' needs exactly one expected_ref and at least one stale_ref, \
         all in corpus_refs, the expected one not among the stale"
    )]
    UpdateShape(String),

    #[error("E_UPDATE_NOT_NEWER: update case '{query}' expects a row no newer than its stale row '{stale_ref}'")]
    UpdateNotNewer { query: String, stale_ref: String },
}

/// A validated golden set paired with the SHA-256 of the exact bytes it was
/// parsed from. The hash is computed over the same read, never a re-read by
/// path (which could see a different file) or over the re-serialized struct
/// (which would hide a whitespace-only edit) — it is the one honest answer to
/// "what set did this run actually score against".
#[derive(Debug)]
pub struct LoadedGolden {
    pub set: GoldenSet,
    pub content_sha256: String,
}

/// Reads, parses and validates `path` — no database is opened and no row is
/// seeded here.
pub fn load(path: &Path) -> Result<LoadedGolden, GoldenError> {
    let raw = std::fs::read_to_string(path).map_err(|source| GoldenError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let golden: GoldenSet = serde_json::from_str(&raw).map_err(|source| GoldenError::Parse {
        path: path.display().to_string(),
        source,
    })?;
    validate(&golden)?;
    let content_sha256 = clawg_core::eval_api::sha256_hex(raw.as_bytes());
    Ok(LoadedGolden { set: golden, content_sha256 })
}

/// Validates a parsed golden set, aborting on the first structural problem
/// found. Order: duplicate refs, unknown memory types, per-case reference
/// resolution (`corpus_refs ⊆ corpus`, `expected_refs ⊆ corpus_refs`), then
/// the `known_red`/`blocked_by` pairing.
///
/// Whether a `known_red` marker is still deserved is NOT decided here. It was,
/// while retrieval had one structural exclusion this file could predict from a
/// row's declared type and importance; that exclusion is gone, and guessing at
/// the replacement would put a second model of retrieval in the loader, free
/// to disagree with the real one. The run itself answers it instead: a blocked
/// case that scores is reported as `KNOWN_RED CASE NOW PASSING`, which is
/// evidence rather than prediction.
pub fn validate(golden: &GoldenSet) -> Result<(), GoldenError> {
    let mut seen_refs = HashSet::new();
    for row in &golden.corpus {
        if !seen_refs.insert(row.ref_.as_str()) {
            return Err(GoldenError::DuplicateRef(row.ref_.clone()));
        }
    }

    for row in &golden.corpus {
        if !VALID_MEMORY_TYPES.contains(&row.memory_type.as_str()) {
            return Err(GoldenError::UnknownMemoryType {
                ref_: row.ref_.clone(),
                memory_type: row.memory_type.clone(),
            });
        }
    }

    for row in &golden.corpus {
        if row.age_days < 0.0 {
            return Err(GoldenError::NegativeAge(row.ref_.clone()));
        }
    }

    let corpus_by_ref: HashMap<&str, &CorpusRow> =
        golden.corpus.iter().map(|r| (r.ref_.as_str(), r)).collect();

    for case in &golden.retrieval_cases {
        for corpus_ref in &case.corpus_refs {
            if !corpus_by_ref.contains_key(corpus_ref.as_str()) {
                return Err(GoldenError::UnresolvedRef {
                    query: case.query.clone(),
                    corpus_ref: corpus_ref.clone(),
                });
            }
        }

        let pool: HashSet<&str> = case.corpus_refs.iter().map(String::as_str).collect();
        for expected_ref in &case.expected_refs {
            if !pool.contains(expected_ref.as_str()) {
                return Err(GoldenError::ExpectedNotInPool {
                    query: case.query.clone(),
                    expected_ref: expected_ref.clone(),
                });
            }
        }

        // `recall_at_k` scores an empty expectation as a total miss, so a case
        // without one would drag every aggregate down; only the class that
        // stays out of them may have it.
        match (case.subset == CaseSubset::Unanswerable, case.expected_refs.is_empty()) {
            (true, false) => return Err(GoldenError::UnanswerableExpects(case.query.clone())),
            (false, true) => return Err(GoldenError::NoExpected(case.query.clone())),
            _ => {}
        }

        validate_update(case, &pool, &corpus_by_ref)?;

        if case.status == CaseStatus::KnownRed && case.blocked_by.is_none() {
            return Err(GoldenError::MissingBlockedBy(case.query.clone()));
        }
    }

    Ok(())
}

/// An `update` case names one newer row and the older rows it restates; no
/// other case names stale rows.
fn validate_update(
    case: &RetrievalCase,
    pool: &HashSet<&str>,
    corpus_by_ref: &HashMap<&str, &CorpusRow>,
) -> Result<(), GoldenError> {
    if case.subset != CaseSubset::Update {
        if !case.stale_refs.is_empty() {
            return Err(GoldenError::StaleOutsideUpdate(case.query.clone()));
        }
        return Ok(());
    }
    let shape_ok = case.expected_refs.len() == 1
        && !case.stale_refs.is_empty()
        && case.stale_refs.iter().all(|s| pool.contains(s.as_str()) && *s != case.expected_refs[0]);
    if !shape_ok {
        return Err(GoldenError::UpdateShape(case.query.clone()));
    }
    let age = |r: &str| corpus_by_ref[r].age_days;
    let expected_age = age(&case.expected_refs[0]);
    match case.stale_refs.iter().find(|s| age(s) <= expected_age) {
        Some(stale) => Err(GoldenError::UpdateNotNewer { query: case.query.clone(), stale_ref: stale.clone() }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- fixtures ------------------------------------------------------

    const VALID_MINIMAL: &str = r#"{
        "corpus": [
            {"ref": "c1", "content": "hello world", "memory_type": "chat_message", "importance": 5.0}
        ],
        "retrieval_cases": [
            {"query": "q1", "corpus_refs": ["c1"], "expected_refs": ["c1"]}
        ],
        "contradiction_pairs": [
            {"a": "the sky is blue", "b": "the sky is not blue", "label": "contradiction", "lang": "en"}
        ]
    }"#;

    fn parse_only(raw: &str) -> GoldenSet {
        serde_json::from_str(raw).expect("fixture must parse")
    }

    // --- unanswerable cases ----------------------------------------------

    fn with_case(case: &str) -> GoldenSet {
        parse_only(&VALID_MINIMAL.replace(
            r#"{"query": "q1", "corpus_refs": ["c1"], "expected_refs": ["c1"]}"#,
            case,
        ))
    }

    #[test]
    fn an_unanswerable_case_must_expect_nothing() {
        let ok = with_case(r#"{"query": "q1", "corpus_refs": ["c1"], "expected_refs": [], "subset": "unanswerable"}"#);
        assert!(validate(&ok).is_ok());
        let bad = with_case(r#"{"query": "q1", "corpus_refs": ["c1"], "expected_refs": ["c1"], "subset": "unanswerable"}"#);
        assert!(matches!(validate(&bad), Err(GoldenError::UnanswerableExpects(q)) if q == "q1"));
    }

    #[test]
    fn only_an_unanswerable_case_may_expect_nothing() {
        let bad = with_case(r#"{"query": "q1", "corpus_refs": ["c1"], "expected_refs": []}"#);
        assert!(matches!(validate(&bad), Err(GoldenError::NoExpected(q)) if q == "q1"));
    }

    // --- update cases ----------------------------------------------------

    const UPDATE: &str = r#"{
        "corpus": [
            {"ref": "new", "content": "the cache lives in redis now", "memory_type": "semantic_fact", "importance": 5.0, "age_days": 1},
            {"ref": "old", "content": "the cache lives in memcached", "memory_type": "semantic_fact", "importance": 5.0, "age_days": 30}
        ],
        "retrieval_cases": [
            {"query": "where does the cache live", "corpus_refs": ["new", "old"], "expected_refs": ["new"], "stale_refs": ["old"], "subset": "update"}
        ],
        "contradiction_pairs": []
    }"#;

    #[test]
    fn an_update_case_needs_one_expected_and_older_stale_rows() {
        assert!(validate(&parse_only(UPDATE)).is_ok());
        let reversed = UPDATE.replace(r#""age_days": 1}"#, r#""age_days": 60}"#);
        assert!(matches!(
            validate(&parse_only(&reversed)),
            Err(GoldenError::UpdateNotNewer { stale_ref, .. }) if stale_ref == "old"
        ));
        let no_stale = UPDATE.replace(r#""stale_refs": ["old"], "#, "");
        assert!(matches!(validate(&parse_only(&no_stale)), Err(GoldenError::UpdateShape(_))));
        let stale_is_expected = UPDATE.replace(r#""stale_refs": ["old"]"#, r#""stale_refs": ["new"]"#);
        assert!(matches!(validate(&parse_only(&stale_is_expected)), Err(GoldenError::UpdateShape(_))));
    }

    #[test]
    fn stale_refs_outside_an_update_case_fail() {
        let natural = UPDATE.replace(r#", "subset": "update""#, "");
        assert!(matches!(validate(&parse_only(&natural)), Err(GoldenError::StaleOutsideUpdate(_))));
    }

    #[test]
    fn a_negative_age_fails() {
        let negative = UPDATE.replace(r#""age_days": 30}"#, r#""age_days": -2}"#);
        assert!(matches!(validate(&parse_only(&negative)), Err(GoldenError::NegativeAge(r)) if r == "old"));
    }

    // --- valid file ------------------------------------------------------

    #[test]
    fn valid_minimal_file_validates() {
        let golden = parse_only(VALID_MINIMAL);
        validate(&golden).expect("minimal golden set must validate");
        assert_eq!(golden.corpus.len(), 1);
        assert_eq!(golden.retrieval_cases.len(), 1);
        assert_eq!(golden.contradiction_pairs.len(), 1);
    }

    // --- subset label ------------------------------------------------------

    fn case_with(extra: &str) -> String {
        format!(
            r#"{{
                "corpus": [{{"ref": "c1", "content": "x", "memory_type": "chat_message", "importance": 5.0}}],
                "retrieval_cases": [{{"query": "q", "corpus_refs": ["c1"], "expected_refs": ["c1"]{extra}}}],
                "contradiction_pairs": []
            }}"#
        )
    }

    #[test]
    fn a_case_without_a_subset_is_natural() {
        let golden = parse_only(VALID_MINIMAL);
        assert_eq!(golden.retrieval_cases[0].subset, CaseSubset::Natural);
    }

    #[test]
    fn an_explicit_subset_is_parsed() {
        let golden = parse_only(&case_with(r#", "subset": "lexical""#));
        assert_eq!(golden.retrieval_cases[0].subset, CaseSubset::Lexical);
    }

    #[test]
    fn every_subset_name_parses_in_snake_case() {
        for (name, want) in [
            ("natural", CaseSubset::Natural),
            ("lexical", CaseSubset::Lexical),
            ("multi_ref", CaseSubset::MultiRef),
            ("high_importance", CaseSubset::HighImportance),
            ("low_importance", CaseSubset::LowImportance),
            ("long", CaseSubset::Long),
        ] {
            let golden = parse_only(&case_with(&format!(r#", "subset": "{name}""#)));
            assert_eq!(golden.retrieval_cases[0].subset, want, "{name}");
        }
    }

    #[test]
    fn an_answer_and_the_long_subset_parse() {
        let golden = parse_only(&case_with(r#", "subset": "long", "answer": "capped at 40 records""#));
        assert_eq!(golden.retrieval_cases[0].subset, CaseSubset::Long);
        assert_eq!(golden.retrieval_cases[0].answer.as_deref(), Some("capped at 40 records"));
        assert_eq!(parse_only(VALID_MINIMAL).retrieval_cases[0].answer, None);
    }

    #[test]
    fn a_misspelled_subset_key_fails_to_parse() {
        let raw = case_with(r#", "subsett": "lexical""#);
        assert!(serde_json::from_str::<GoldenSet>(&raw).is_err());
    }

    #[test]
    fn an_unknown_subset_value_fails_to_parse() {
        let raw = case_with(r#", "subset": "lexcial""#);
        assert!(serde_json::from_str::<GoldenSet>(&raw).is_err());
    }

    // --- unknown memory_type ---------------------------------------------

    #[test]
    fn unknown_memory_type_rejected() {
        let raw = r#"{
            "corpus": [
                {"ref": "c1", "content": "x", "memory_type": "skill", "importance": 5.0}
            ],
            "retrieval_cases": [],
            "contradiction_pairs": []
        }"#;
        let golden = parse_only(raw);
        let err = validate(&golden).expect_err("unknown memory_type must abort");
        assert_eq!(
            err.to_string(),
            "E_UNKNOWN_MEMORY_TYPE: corpus ref 'c1' has memory_type 'skill', expected one of chat_message|tool_error|code_block|session_summary|user_preference|architectural_decision|semantic_fact"
        );
    }

    // --- duplicate ref -----------------------------------------------------

    #[test]
    fn duplicate_ref_rejected() {
        let raw = r#"{
            "corpus": [
                {"ref": "c1", "content": "a", "memory_type": "chat_message", "importance": 1.0},
                {"ref": "c1", "content": "b", "memory_type": "chat_message", "importance": 2.0}
            ],
            "retrieval_cases": [],
            "contradiction_pairs": []
        }"#;
        let golden = parse_only(raw);
        let err = validate(&golden).expect_err("duplicate ref must abort");
        assert_eq!(err.to_string(), "E_DUPLICATE_REF: corpus ref 'c1' appears more than once");
    }

    // --- dangling corpus_refs reference (unresolved) ------------------------

    #[test]
    fn unresolved_corpus_ref_rejected() {
        let raw = r#"{
            "corpus": [
                {"ref": "c1", "content": "a", "memory_type": "chat_message", "importance": 1.0}
            ],
            "retrieval_cases": [
                {"query": "q1", "corpus_refs": ["c9"], "expected_refs": []}
            ],
            "contradiction_pairs": []
        }"#;
        let golden = parse_only(raw);
        let err = validate(&golden).expect_err("dangling corpus_ref must abort");
        assert_eq!(
            err.to_string(),
            "E_UNRESOLVED_REF: retrieval_case 'q1' references corpus ref 'c9' which does not exist in corpus"
        );
    }

    // --- expected_refs not subset of corpus_refs ---------------------------

    #[test]
    fn expected_ref_not_in_pool_rejected() {
        let raw = r#"{
            "corpus": [
                {"ref": "c1", "content": "a", "memory_type": "chat_message", "importance": 1.0},
                {"ref": "c2", "content": "b", "memory_type": "chat_message", "importance": 1.0}
            ],
            "retrieval_cases": [
                {"query": "q1", "corpus_refs": ["c1"], "expected_refs": ["c1", "c2"]}
            ],
            "contradiction_pairs": []
        }"#;
        let golden = parse_only(raw);
        let err = validate(&golden).expect_err("expected_ref outside corpus_refs must abort");
        assert_eq!(
            err.to_string(),
            "E_EXPECTED_NOT_IN_POOL: retrieval_case 'q1' expects 'c2' which is not listed in corpus_refs"
        );
    }

    // --- a tier-1 expectation is an ordinary expectation ---------------------
    //
    // These used to abort the load: a row seeding at tier 1 was structurally
    // unreachable, so expecting it could only ever measure a bug. The pool
    // scans every tier now, which makes both of these plain cases that the run
    // is free to pass or fail on its own merits.

    #[test]
    fn a_tier1_user_preference_expectation_is_valid() {
        let raw = r#"{
            "corpus": [
                {"ref": "c1", "content": "prefers dark mode", "memory_type": "user_preference", "importance": 5.0}
            ],
            "retrieval_cases": [
                {"query": "q1", "corpus_refs": ["c1"], "expected_refs": ["c1"]}
            ],
            "contradiction_pairs": []
        }"#;
        validate(&parse_only(raw)).expect("a tier-1 expectation must validate");
    }

    #[test]
    fn a_high_importance_tier1_expectation_is_valid() {
        let raw = r#"{
            "corpus": [
                {"ref": "c2", "content": "critical fact", "memory_type": "chat_message", "importance": 9.5}
            ],
            "retrieval_cases": [
                {"query": "q2", "corpus_refs": ["c2"], "expected_refs": ["c2"]}
            ],
            "contradiction_pairs": []
        }"#;
        validate(&parse_only(raw)).expect("an importance-forced tier-1 expectation must validate");
    }

    // A `known_red` marker still parses and still requires its `blocked_by`;
    // only the automatic guess at whether it is deserved is gone.
    #[test]
    fn marked_known_red_case_is_accepted() {
        let raw = r#"{
            "corpus": [
                {"ref": "c1", "content": "prefers dark mode", "memory_type": "user_preference", "importance": 5.0}
            ],
            "retrieval_cases": [
                {
                    "query": "q1",
                    "corpus_refs": ["c1"],
                    "expected_refs": ["c1"],
                    "status": "known_red",
                    "blocked_by": "some-future-change"
                }
            ],
            "contradiction_pairs": []
        }"#;
        validate(&parse_only(raw)).expect("marked known_red case must validate");
    }

    // --- known_red without blocked_by aborts --------------------------------

    #[test]
    fn known_red_without_blocked_by_aborts() {
        let raw = r#"{
            "corpus": [
                {"ref": "c1", "content": "prefers dark mode", "memory_type": "user_preference", "importance": 5.0}
            ],
            "retrieval_cases": [
                {"query": "q1", "corpus_refs": ["c1"], "expected_refs": ["c1"], "status": "known_red"}
            ],
            "contradiction_pairs": []
        }"#;
        let golden = parse_only(raw);
        let err = validate(&golden).expect_err("known_red without blocked_by must abort");
        assert_eq!(
            err.to_string(),
            "E_MISSING_BLOCKED_BY: retrieval_case 'q1' is marked known_red but has no blocked_by"
        );
    }

    // --- malformed JSON ------------------------------------------------------

    #[test]
    fn malformed_json_is_a_parse_error() {
        let path = std::env::temp_dir().join(format!(
            "rag-eval-golden-test-malformed-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, "{ this is not valid json").expect("write temp fixture");
        let result = load(&path);
        std::fs::remove_file(&path).ok();
        match result {
            Err(GoldenError::Parse { .. }) => {}
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    // --- missing file ----------------------------------------------------

    #[test]
    fn missing_file_is_an_io_error() {
        let path = std::env::temp_dir().join(format!(
            "rag-eval-golden-test-missing-{}-does-not-exist.json",
            std::process::id()
        ));
        assert!(!path.exists(), "fixture path must not exist for this test");
        let result = load(&path);
        match result {
            Err(GoldenError::Io { .. }) => {}
            other => panic!("expected Io error, got {other:?}"),
        }
    }

    // --- content hash ------------------------------------------------------

    #[test]
    fn loaded_golden_carries_the_sha256_of_its_own_raw_bytes() {
        let path = std::env::temp_dir().join(format!(
            "rag-eval-golden-test-hash-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, VALID_MINIMAL).expect("write temp fixture");

        let loaded = load(&path).expect("valid fixture must load");
        std::fs::remove_file(&path).ok();

        assert_eq!(
            loaded.content_sha256,
            clawg_core::eval_api::sha256_hex(VALID_MINIMAL.as_bytes()),
            "the hash must describe the exact bytes on disk, not a re-serialization"
        );
        assert_eq!(loaded.set.corpus.len(), 1);
    }

    // --- history field: hash-safety and default-parse -----------------------

    /// `content_sha256` hashes the file's raw bytes (`golden::load`'s own doc
    /// comment says so), never the re-serialized struct. Adding an optional
    /// `history` field to `RetrievalCase` widens what the struct can hold,
    /// but does not touch a single byte on disk — so hashing the SAME
    /// `VALID_MINIMAL` bytes before and after this field existed must produce
    /// the identical digest. What actually proves this (the struct change
    /// already landed by the time this test runs) is that the hash comes
    /// from `sha256_hex(raw.as_bytes())`, never from `serde_json::to_string`
    /// on the parsed `GoldenSet` — a re-serialization hash would silently
    /// start including the new field's default value and drift.
    #[test]
    fn an_added_optional_field_does_not_change_content_sha256() {
        let path = std::env::temp_dir().join(format!(
            "rag-eval-golden-test-history-hash-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, VALID_MINIMAL).expect("write temp fixture");

        let loaded = load(&path).expect("valid fixture must load");
        std::fs::remove_file(&path).ok();

        assert_eq!(
            loaded.content_sha256,
            clawg_core::eval_api::sha256_hex(VALID_MINIMAL.as_bytes()),
            "the hash must describe the exact bytes on disk — an added optional field must not move it"
        );
    }

    /// The current `golden.json` shape has no `history` key at all. Every
    /// case must still parse, defaulting `history` to an empty vec.
    #[test]
    fn a_case_with_no_history_key_defaults_to_an_empty_vec() {
        let golden = parse_only(VALID_MINIMAL);
        assert_eq!(golden.retrieval_cases.len(), 1);
        assert_eq!(
            golden.retrieval_cases[0].history,
            Vec::<String>::new(),
            "a case written before `history` existed must default to an empty vec, not fail to parse"
        );
    }

    /// A case that DOES declare `history` must carry it through untouched,
    /// oldest-first, exactly as written.
    #[test]
    fn a_case_with_an_explicit_history_parses_it_verbatim() {
        let raw = r#"{
            "corpus": [
                {"ref": "c1", "content": "hello world", "memory_type": "chat_message", "importance": 5.0}
            ],
            "retrieval_cases": [
                {
                    "query": "q1",
                    "corpus_refs": ["c1"],
                    "expected_refs": ["c1"],
                    "history": ["oldest turn", "newest turn"]
                }
            ],
            "contradiction_pairs": []
        }"#;
        let golden = parse_only(raw);
        assert_eq!(
            golden.retrieval_cases[0].history,
            vec!["oldest turn".to_string(), "newest turn".to_string()]
        );
    }

    // --- RefMap: trivial bidirectional lookups ------------------------------

    #[test]
    fn ref_map_resolves_both_directions() {
        let mut map = RefMap::new();
        map.insert("c1", 42);
        assert_eq!(map.id_of("c1"), Some(42));
        assert_eq!(map.ref_of(42), Some("c1"));
        assert_eq!(map.id_of("c9"), None);
        assert_eq!(map.ref_of(99), None);
    }
}
