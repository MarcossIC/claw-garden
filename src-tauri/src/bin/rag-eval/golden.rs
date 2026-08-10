//! Golden-set schema, loader, and validator for the retrieval-evaluation
//! harness. Consumes `docs/eval/golden.json` (or an override path), parses it
//! strictly, and validates it end to end BEFORE any row is seeded into a
//! store — a malformed or structurally impossible golden file must surface as
//! a load-time abort, not as a retrieval number nobody can later trust.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use agent_office_lib::eval_api::knn_reachable;

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
    /// Kept as the raw on-disk string rather than parsed into an enum: the
    /// tier gate below hands this same string straight to `knn_reachable`,
    /// the exact call the production insert path makes, so there is only one
    /// string in play and it can never drift from what actually gets seeded.
    pub memory_type: String,
    pub importance: f64,
    #[serde(default)]
    pub lang: Option<String>,
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
    #[allow(dead_code)]
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

    #[error(
        "E_TIER1_UNMARKED: retrieval_case '{query}' expects '{corpus_ref}' (memory_type={memory_type}, importance={importance}) which seeds at tier 1 and is invisible to search_all_memories; mark {{\"status\":\"known_red\",\"blocked_by\":\"<change>\"}} or change the expectation"
    )]
    Tier1Unmarked {
        query: String,
        corpus_ref: String,
        memory_type: String,
        importance: f64,
    },

    #[error("E_MISSING_BLOCKED_BY: retrieval_case '{0}' is marked known_red but has no blocked_by")]
    MissingBlockedBy(String),
}

/// Reads and parses `path`, then validates it. Returns the validated set plus
/// any non-fatal warnings (currently just stale `known_red` markers) — no
/// database is opened and no row is seeded here.
pub fn load(path: &Path) -> Result<(GoldenSet, Vec<String>), GoldenError> {
    let raw = std::fs::read_to_string(path).map_err(|source| GoldenError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let golden: GoldenSet = serde_json::from_str(&raw).map_err(|source| GoldenError::Parse {
        path: path.display().to_string(),
        source,
    })?;
    let warnings = validate(&golden)?;
    Ok((golden, warnings))
}

/// Validates a parsed golden set, aborting on the first structural problem
/// found. Order: duplicate refs, unknown memory types, per-case reference
/// resolution (`corpus_refs ⊆ corpus`, `expected_refs ⊆ corpus_refs`), the
/// `known_red`/`blocked_by` pairing, then the tier-reachability gate. Returns
/// any stale-marker warnings collected along the way.
pub fn validate(golden: &GoldenSet) -> Result<Vec<String>, GoldenError> {
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

    let corpus_by_ref: HashMap<&str, &CorpusRow> =
        golden.corpus.iter().map(|r| (r.ref_.as_str(), r)).collect();

    let mut warnings = Vec::new();

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

        if case.status == CaseStatus::KnownRed && case.blocked_by.is_none() {
            return Err(GoldenError::MissingBlockedBy(case.query.clone()));
        }

        // Every `expected_ref` here is already proven present in
        // `corpus_by_ref` by the resolution checks above (expected_refs is a
        // subset of corpus_refs, and corpus_refs was just proven a subset of
        // corpus), so the direct index below cannot panic.
        let mut any_tier1 = false;
        for expected_ref in &case.expected_refs {
            let row = corpus_by_ref[expected_ref.as_str()];
            if !knn_reachable(row.importance, &row.memory_type) {
                any_tier1 = true;
                if case.status != CaseStatus::KnownRed {
                    return Err(GoldenError::Tier1Unmarked {
                        query: case.query.clone(),
                        corpus_ref: expected_ref.clone(),
                        memory_type: row.memory_type.clone(),
                        importance: row.importance,
                    });
                }
            }
        }

        if case.status == CaseStatus::KnownRed && !any_tier1 {
            warnings.push(format!(
                "W_STALE_KNOWN_RED: '{}' is marked known_red but no expected ref seeds at tier 1 — remove the marker",
                case.query
            ));
        }
    }

    Ok(warnings)
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

    // --- valid file ------------------------------------------------------

    #[test]
    fn valid_minimal_file_validates_with_no_warnings() {
        let golden = parse_only(VALID_MINIMAL);
        let warnings = validate(&golden).expect("minimal golden set must validate");
        assert!(warnings.is_empty(), "expected no warnings, got {warnings:?}");
        assert_eq!(golden.corpus.len(), 1);
        assert_eq!(golden.retrieval_cases.len(), 1);
        assert_eq!(golden.contradiction_pairs.len(), 1);
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

    // --- tier gate: unmarked user_preference --------------------------------

    #[test]
    fn unmarked_tier1_user_preference_expectation_aborts() {
        let raw = r#"{
            "corpus": [
                {"ref": "c1", "content": "prefers dark mode", "memory_type": "user_preference", "importance": 5.0}
            ],
            "retrieval_cases": [
                {"query": "q1", "corpus_refs": ["c1"], "expected_refs": ["c1"]}
            ],
            "contradiction_pairs": []
        }"#;
        let golden = parse_only(raw);
        let err = validate(&golden).expect_err("unmarked tier-1 expectation must abort");
        assert_eq!(
            err.to_string(),
            "E_TIER1_UNMARKED: retrieval_case 'q1' expects 'c1' (memory_type=user_preference, importance=5) which seeds at tier 1 and is invisible to search_all_memories; mark {\"status\":\"known_red\",\"blocked_by\":\"<change>\"} or change the expectation"
        );
    }

    // --- tier gate: unmarked high-importance non-preference type ------------

    #[test]
    fn unmarked_tier1_high_importance_non_preference_type_also_aborts() {
        let raw = r#"{
            "corpus": [
                {"ref": "c2", "content": "critical fact", "memory_type": "chat_message", "importance": 9.5}
            ],
            "retrieval_cases": [
                {"query": "q2", "corpus_refs": ["c2"], "expected_refs": ["c2"]}
            ],
            "contradiction_pairs": []
        }"#;
        let golden = parse_only(raw);
        let err = validate(&golden).expect_err("importance-forced tier-1 must also abort");
        match err {
            GoldenError::Tier1Unmarked { memory_type, .. } => {
                assert_eq!(memory_type, "chat_message");
            }
            other => panic!("expected Tier1Unmarked, got {other:?}"),
        }
    }

    // --- tier gate: marked known_red is accepted ---------------------------

    #[test]
    fn marked_known_red_tier1_case_is_accepted_with_no_warnings() {
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
                    "blocked_by": "rag-tier-reachability"
                }
            ],
            "contradiction_pairs": []
        }"#;
        let golden = parse_only(raw);
        let warnings = validate(&golden).expect("marked known_red tier-1 case must validate");
        assert!(warnings.is_empty(), "expected no warnings, got {warnings:?}");
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

    // --- stale known_red marker warns, does not abort -----------------------

    #[test]
    fn stale_known_red_marker_warns_but_does_not_abort() {
        let raw = r#"{
            "corpus": [
                {"ref": "c1", "content": "a", "memory_type": "chat_message", "importance": 5.0}
            ],
            "retrieval_cases": [
                {
                    "query": "q1",
                    "corpus_refs": ["c1"],
                    "expected_refs": ["c1"],
                    "status": "known_red",
                    "blocked_by": "rag-tier-reachability"
                }
            ],
            "contradiction_pairs": []
        }"#;
        let golden = parse_only(raw);
        let warnings = validate(&golden).expect("stale marker must not abort the load");
        assert_eq!(warnings.len(), 1, "expected exactly one warning, got {warnings:?}");
        assert_eq!(
            warnings[0],
            "W_STALE_KNOWN_RED: 'q1' is marked known_red but no expected ref seeds at tier 1 — remove the marker"
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
