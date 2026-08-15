//! Result structures, human-readable stdout report, and the committed
//! results-JSON writer for the retrieval-evaluation harness.
//!
//! The JSON this module writes is the only artifact anyone downstream reads:
//! nobody re-runs the harness to double-check a number quoted from
//! `docs/eval/results/*.json`, so every field that makes a run comparable and
//! honestly interpretable lives here — including the defects that were live
//! when the run was taken and whether the golden set itself is still a
//! placeholder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::golden::CaseStatus;
use crate::metrics::ThresholdCurveRow;

/// Which embedder produced a run. A thin, report-local mirror of
/// [`crate::cli::Mode`] rather than a re-export: the CLI's mode selects an
/// embedder, this one labels a finished result, and the two are allowed to
/// diverge in shape without one module's refactor forcing the other's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Fake,
    Real,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct KPool {
    pub memory: usize,
    pub episodic: usize,
    pub procedural: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct Aggregate {
    #[serde(rename = "recall@1")]
    pub recall_1: f64,
    #[serde(rename = "recall@5")]
    pub recall_5: f64,
    #[serde(rename = "recall@10")]
    pub recall_10: f64,
    pub mrr: f64,
    #[serde(rename = "ndcg@10")]
    pub ndcg_10: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaseResult {
    pub query: String,
    pub expected_refs: Vec<String>,
    pub ranked_refs: Vec<String>,
    #[serde(rename = "recall@1")]
    pub recall_1: f64,
    #[serde(rename = "recall@5")]
    pub recall_5: f64,
    #[serde(rename = "recall@10")]
    pub recall_10: f64,
    pub mrr: f64,
    #[serde(rename = "ndcg@10")]
    pub ndcg_10: f64,
    pub status: CaseStatus,
    /// The language this case is written in, when its expected corpus rows
    /// agree on one. `None` for a case whose expected rows declare no `lang`
    /// or disagree about it — such a case still counts toward the headline
    /// aggregate, it just cannot be attributed to a single language.
    pub lang: Option<String>,
}

/// Retrieval quality restricted to the cases of one language. The embedding
/// model is multilingual and the golden set is deliberately bilingual, so a
/// single headline average can hide one language retrieving markedly worse
/// than the other — that asymmetry is a real, actionable defect and it is
/// invisible until the numbers are split.
#[derive(Debug, Clone, Serialize)]
pub struct LangAggregate {
    pub lang: String,
    pub n_cases: usize,
    pub aggregate: Aggregate,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetrievalResults {
    pub aggregate: Aggregate,
    /// Per-language split of `aggregate`, over the same active cases. Cases
    /// with no unanimous language are absent here but present in `aggregate`,
    /// so these sub-counts need not sum to `per_case.len()`.
    pub by_lang: Vec<LangAggregate>,
    pub per_case: Vec<CaseResult>,
    /// `known_red` cases: scored and recorded, but never folded into
    /// `aggregate` — a structural impossibility (a row seeded at a tier the
    /// KNN pool never scans) must not be misread as a ranking regression.
    pub blocked: Vec<CaseResult>,
}

/// The threshold sweep restricted to the pairs of one language. A distance
/// threshold that separates contradictions cleanly in one language and not in
/// the other is a calibration problem no combined curve would reveal, and the
/// production detector applies a single threshold to both.
#[derive(Debug, Clone, Serialize)]
pub struct LangCurve {
    pub lang: String,
    pub n_pairs: usize,
    pub curve: Vec<ThresholdCurveRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContradictionResults {
    pub curve: Vec<ThresholdCurveRow>,
    pub by_lang: Vec<LangCurve>,
}

/// Whether the embeddings this run seeded with are close to unit-norm, and
/// how far off they sit. Only `--real` embeddings are expected to satisfy
/// this; under `--fake` the numbers are still computed and reported; a
/// reader who does not already know `FakeEmbedder` is unnormalized would
/// otherwise have no way to tell a plumbing artifact from a model problem.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct NormCheck {
    pub asserted: bool,
    pub mean_norm: f64,
    pub max_deviation: f64,
}

/// Marks whether the golden set backing this run is the small template sample
/// or a domain set large enough to measure against. A run against a fuller
/// golden set moves every metric, and that movement must never be read as a
/// retrieval improvement or regression — it is a change in what was measured,
/// not in how well it was measured.
#[derive(Debug, Clone, Serialize)]
pub struct GoldenSetProvenance {
    pub provisional: bool,
    pub note: String,
}

/// Case and pair counts below which a golden set cannot carry a claim. Taken
/// from the sizes the improvement plan specifies for the domain set (~30–50
/// retrieval cases, ~20–30 contradiction pairs) rather than picked here: a set
/// that reaches them is the set that plan asked for.
const DOMAIN_SET_MIN_CASES: usize = 30;
const DOMAIN_SET_MIN_PAIRS: usize = 20;

impl GoldenSetProvenance {
    /// Derives provenance from what the loaded golden set actually contains.
    ///
    /// This was a hardcoded `provisional: true` and stayed true after it
    /// stopped being so — the same failure mode a `known_bugs_live` entry hit
    /// earlier, and worse here because the flag is serialized into every
    /// results file. A stale `true` teaches every future reader to discount
    /// numbers that deserve to be trusted, and nothing in a passing test suite
    /// would ever contradict it. Computing it from the counts means the claim
    /// cannot outlive the condition it describes.
    pub fn derive(n_cases: usize, n_pairs: usize) -> Self {
        let short_on_cases = n_cases < DOMAIN_SET_MIN_CASES;
        let short_on_pairs = n_pairs < DOMAIN_SET_MIN_PAIRS;
        let lacking = match (short_on_cases, short_on_pairs) {
            // The one arm that is not a shortfall, kept inside the match so
            // the compiler proves every combination is handled.
            (false, false) => {
                return Self {
                    provisional: false,
                    note: format!(
                        "domain golden set: {n_cases} retrieval cases, \
                         {n_pairs} contradiction pairs"
                    ),
                }
            }
            (true, true) => format!(
                "only {n_cases} retrieval cases (needs {DOMAIN_SET_MIN_CASES}) and \
                 {n_pairs} contradiction pairs (needs {DOMAIN_SET_MIN_PAIRS})"
            ),
            (true, false) => format!(
                "only {n_cases} retrieval cases (needs {DOMAIN_SET_MIN_CASES})"
            ),
            (false, true) => format!(
                "only {n_pairs} contradiction pairs (needs {DOMAIN_SET_MIN_PAIRS})"
            ),
        };
        Self {
            provisional: true,
            note: format!(
                "{lacking}; a later run against a fuller golden set will move every metric \
                 for reasons that have nothing to do with retrieval quality"
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResults {
    pub date: String,
    pub mode: Mode,
    pub model_id: String,
    pub git_sha: String,
    pub k_pool: KPool,
    pub k_final: usize,
    pub corpus_size: usize,
    pub n_retrieval_cases: usize,
    pub n_contradiction_pairs: usize,
    /// Defects known to be live in the codebase at run time. A baseline taken
    /// on broken code is what later makes the repair measurable, so a reader
    /// needs to know which numbers are standing on a bug rather than a
    /// finished retrieval path.
    pub known_bugs_live: Vec<String>,
    pub golden_set: GoldenSetProvenance,
    pub retrieval: RetrievalResults,
    pub contradiction: ContradictionResults,
    /// Present only when the run was asked to audit its own golden set.
    /// Absent — rather than a zeroed report — when it was not, so a reader can
    /// tell "this set was never audited" from "this set was audited and passed".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit: Option<crate::audit::AuditReport>,
    pub norm_check: NormCheck,
}

/// Averages the five per-case metrics across `cases`. Callers decide which
/// cases belong in the average by choosing what to pass in — this function
/// has no notion of `known_red`/`active` at all, so a caller that
/// accidentally includes a blocked case gets exactly what it asked for. The
/// routing decision (blocked cases never reach this function) lives in
/// `main.rs`, and the test below exercises that contract from this side:
/// aggregating only the active slice must ignore a blocked case's numbers
/// even though both are `CaseResult`s of the same shape.
///
/// An empty slice averages to all-zero rather than dividing by zero; this
/// can only happen if a golden set has zero active retrieval cases, which
/// `golden::validate` does not forbid today.
pub fn aggregate(cases: &[CaseResult]) -> Aggregate {
    let n = cases.len();
    if n == 0 {
        return Aggregate::default();
    }
    let n = n as f64;
    let sum_of = |f: fn(&CaseResult) -> f64| cases.iter().map(f).sum::<f64>();
    Aggregate {
        recall_1: sum_of(|c| c.recall_1) / n,
        recall_5: sum_of(|c| c.recall_5) / n,
        recall_10: sum_of(|c| c.recall_10) / n,
        mrr: sum_of(|c| c.mrr) / n,
        ndcg_10: sum_of(|c| c.ndcg_10) / n,
    }
}

/// Splits `cases` by `CaseResult::lang` and aggregates each group on its own.
/// Cases with no language are dropped rather than pooled into an "unknown"
/// bucket: this breakdown answers "does the model do worse in Spanish than in
/// English", and a bucket that means "we do not know" cannot answer it.
///
/// Output is ordered by language code so two runs of the same golden set
/// produce byte-comparable JSON.
pub fn aggregate_by_lang(cases: &[CaseResult]) -> Vec<LangAggregate> {
    let mut grouped: BTreeMap<&str, Vec<CaseResult>> = BTreeMap::new();
    for case in cases {
        if let Some(lang) = case.lang.as_deref() {
            grouped.entry(lang).or_default().push(case.clone());
        }
    }
    grouped
        .into_iter()
        .map(|(lang, group)| LangAggregate {
            lang: lang.to_string(),
            n_cases: group.len(),
            aggregate: aggregate(&group),
        })
        .collect()
}

/// The on-disk filename for a run's results: `{date}-{mode}.json`, matching
/// the spec's `docs/eval/results/{fecha}-{config}.json` pattern with `mode`
/// standing in for `config` (the only axis this harness varies today).
pub fn results_filename(date: &str, mode: Mode) -> String {
    let config = match mode {
        Mode::Fake => "fake",
        Mode::Real => "real",
    };
    format!("{date}-{config}.json")
}

#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("results file '{}' already exists; rerun with --force to overwrite", path.display())]
    AlreadyExists { path: PathBuf },
    #[error("failed to create results directory '{}': {source}", path.display())]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize results to JSON: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("failed to write results file '{}': {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Writes `results` to `{out_dir}/{date}-{mode}.json`, refusing to clobber an
/// existing file for the same date+config unless `force` is set. The
/// production RAG path is deliberately fail-soft (a broken contradiction
/// check must never abort an ingest); this writer is the opposite on
/// purpose. A measurement tool's only output is a number someone will quote,
/// and a silently-overwritten or partially-written result is worse than one
/// that never appeared, because nothing downstream can tell the difference
/// between "this run improved retrieval" and "this run silently replaced the
/// baseline it was supposed to be compared against".
pub fn write_json(results: &RunResults, out_dir: &Path, force: bool) -> Result<PathBuf, ReportError> {
    std::fs::create_dir_all(out_dir)
        .map_err(|e| ReportError::CreateDir { path: out_dir.to_path_buf(), source: e })?;
    let path = out_dir.join(results_filename(&results.date, results.mode));
    if path.exists() && !force {
        return Err(ReportError::AlreadyExists { path });
    }
    let json = serde_json::to_string_pretty(results)?;
    std::fs::write(&path, json).map_err(|e| ReportError::Write { path: path.clone(), source: e })?;
    Ok(path)
}

/// Best-effort short commit sha for the working tree at run time. Degrades
/// to `"unknown"` rather than failing the run: a missing `git` binary or a
/// checkout without history must not stop a measurement from being taken,
/// it just means that measurement is harder to place on a timeline later.
pub fn git_sha() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Human-readable report printed to stdout alongside the JSON write. Not a
/// substitute for the JSON — this is for a person watching the run finish,
/// the JSON is for whatever reads it later.
pub fn print_stdout(results: &RunResults) {
    println!("rag-eval run — {} ({:?})", results.date, results.mode);
    println!("  model:          {}", results.model_id);
    println!("  git sha:        {}", results.git_sha);
    println!(
        "  corpus:         {} rows, {} retrieval cases, {} contradiction pairs",
        results.corpus_size, results.n_retrieval_cases, results.n_contradiction_pairs
    );
    if results.golden_set.provisional {
        println!("  ⚠ golden set is PROVISIONAL: {}", results.golden_set.note);
    }
    println!("  retrieval (active cases only, n={}):", results.retrieval.per_case.len());
    println!("    recall@1  = {:.4}", results.retrieval.aggregate.recall_1);
    println!("    recall@5  = {:.4}", results.retrieval.aggregate.recall_5);
    println!("    recall@10 = {:.4}", results.retrieval.aggregate.recall_10);
    println!("    mrr       = {:.4}", results.retrieval.aggregate.mrr);
    println!("    ndcg@10   = {:.4}", results.retrieval.aggregate.ndcg_10);
    if !results.retrieval.by_lang.is_empty() {
        println!("  retrieval by language:");
        println!("    lang    n   recall@1  recall@5  recall@10     mrr   ndcg@10");
        for split in &results.retrieval.by_lang {
            println!(
                "    {:<4} {:>3}  {:>9.4} {:>9.4} {:>10.4} {:>7.4} {:>9.4}",
                split.lang,
                split.n_cases,
                split.aggregate.recall_1,
                split.aggregate.recall_5,
                split.aggregate.recall_10,
                split.aggregate.mrr,
                split.aggregate.ndcg_10
            );
        }
    }
    if !results.retrieval.blocked.is_empty() {
        println!(
            "  blocked (known_red, excluded from aggregate, n={}):",
            results.retrieval.blocked.len()
        );
        for case in &results.retrieval.blocked {
            println!("    - '{}' recall@10={:.4}", case.query, case.recall_10);
        }
    }
    println!("  contradiction threshold sweep:");
    println!("    threshold  recall  fpr_no_contradiction  fpr_identical");
    for row in &results.contradiction.curve {
        println!(
            "    {:>9.2}  {:>6.3}  {:>21.3}  {:>13.3}",
            row.threshold, row.recall, row.fpr_no_contradiction, row.fpr_identical
        );
    }
    for split in &results.contradiction.by_lang {
        println!("  contradiction threshold sweep — {} ({} pairs):", split.lang, split.n_pairs);
        println!("    threshold  recall  fpr_no_contradiction  fpr_identical");
        for row in &split.curve {
            println!(
                "    {:>9.2}  {:>6.3}  {:>21.3}  {:>13.3}",
                row.threshold, row.recall, row.fpr_no_contradiction, row.fpr_identical
            );
        }
    }
    println!(
        "  norm check: asserted={} mean_norm={:.4} max_deviation={:.4}",
        results.norm_check.asserted, results.norm_check.mean_norm, results.norm_check.max_deviation
    );
    if !results.known_bugs_live.is_empty() {
        println!("  known bugs live at run time:");
        for bug in &results.known_bugs_live {
            println!("    - {bug}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- golden-set provenance ------------------------------------------

    #[test]
    fn a_full_domain_set_is_not_provisional() {
        let p = GoldenSetProvenance::derive(46, 28);
        assert!(!p.provisional);
        assert!(p.note.contains("46 retrieval cases"), "note was: {}", p.note);
    }

    #[test]
    fn a_set_exactly_at_both_thresholds_is_not_provisional() {
        // The boundary is the one place an off-by-one would flip the meaning
        // of every results file written that day.
        assert!(!GoldenSetProvenance::derive(DOMAIN_SET_MIN_CASES, DOMAIN_SET_MIN_PAIRS).provisional);
    }

    #[test]
    fn too_few_cases_stays_provisional_and_says_which_count_is_short() {
        let p = GoldenSetProvenance::derive(13, 28);
        assert!(p.provisional);
        assert!(p.note.contains("13 retrieval cases"), "note was: {}", p.note);
        assert!(!p.note.contains("contradiction pairs (needs"), "note was: {}", p.note);
    }

    #[test]
    fn too_few_pairs_stays_provisional_even_with_plenty_of_cases() {
        let p = GoldenSetProvenance::derive(46, 9);
        assert!(p.provisional);
        assert!(p.note.contains("9 contradiction pairs"), "note was: {}", p.note);
    }

    #[test]
    fn short_on_both_counts_reports_both() {
        let p = GoldenSetProvenance::derive(13, 9);
        assert!(p.provisional);
        assert!(p.note.contains("13 retrieval cases"), "note was: {}", p.note);
        assert!(p.note.contains("9 contradiction pairs"), "note was: {}", p.note);
    }

    #[test]
    fn an_empty_golden_set_is_provisional() {
        assert!(GoldenSetProvenance::derive(0, 0).provisional);
    }

    fn case(query: &str, recall_10: f64, status: CaseStatus) -> CaseResult {
        CaseResult {
            query: query.to_string(),
            expected_refs: vec!["c1".to_string()],
            ranked_refs: vec!["c1".to_string()],
            recall_1: recall_10,
            recall_5: recall_10,
            recall_10,
            mrr: recall_10,
            ndcg_10: recall_10,
            status,
            lang: None,
        }
    }

    fn case_in(query: &str, recall_10: f64, lang: Option<&str>) -> CaseResult {
        CaseResult { lang: lang.map(str::to_string), ..case(query, recall_10, CaseStatus::Active) }
    }

    #[test]
    fn aggregate_by_lang_scores_each_language_on_its_own_cases_only() {
        let cases = vec![
            case_in("q-es-1", 1.0, Some("es")),
            case_in("q-es-2", 0.0, Some("es")),
            case_in("q-en-1", 1.0, Some("en")),
        ];
        let split = aggregate_by_lang(&cases);

        // Ordered by language code, not by insertion order, so two runs of the
        // same golden set produce comparable JSON.
        assert_eq!(split.iter().map(|s| s.lang.as_str()).collect::<Vec<_>>(), vec!["en", "es"]);
        assert_eq!(split[0].n_cases, 1);
        assert_eq!(split[0].aggregate.recall_10, 1.0);
        assert_eq!(split[1].n_cases, 2);
        assert_eq!(split[1].aggregate.recall_10, 0.5);

        // The headline number is unchanged by the split: 2/3 across all three.
        let overall = aggregate(&cases);
        assert!((overall.recall_10 - 2.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn aggregate_by_lang_drops_unattributable_cases_without_dropping_them_from_the_headline() {
        let cases = vec![case_in("q-es", 1.0, Some("es")), case_in("q-mixed", 0.0, None)];
        let split = aggregate_by_lang(&cases);

        assert_eq!(split.len(), 1, "a case with no unanimous language gets no bucket of its own");
        assert_eq!(split[0].lang, "es");
        assert_eq!(split[0].n_cases, 1);
        assert_eq!(
            aggregate(&cases).recall_10,
            0.5,
            "but it still counts toward the headline aggregate"
        );
    }

    #[test]
    fn aggregate_by_lang_of_cases_without_any_language_is_empty_not_a_bucket_named_unknown() {
        let cases = vec![case_in("q1", 1.0, None), case_in("q2", 0.0, None)];
        assert!(aggregate_by_lang(&cases).is_empty());
    }

    #[test]
    fn aggregate_averages_the_five_metrics_across_active_cases() {
        let active = vec![case("q1", 1.0, CaseStatus::Active), case("q2", 0.0, CaseStatus::Active)];
        let agg = aggregate(&active);
        assert_eq!(agg.recall_1, 0.5);
        assert_eq!(agg.recall_10, 0.5);
        assert_eq!(agg.mrr, 0.5);
        assert_eq!(agg.ndcg_10, 0.5);
    }

    #[test]
    fn aggregate_of_active_slice_is_unaffected_by_a_blocked_cases_numbers() {
        // Simulates the routing contract: main.rs must hand `aggregate` only
        // the active-status cases. A blocked case with a wildly different
        // score (here a perfect 1.0, simulating "KNOWN_RED CASE NOW PASSING")
        // must not move the aggregate computed over the active slice alone.
        let active = vec![case("q1", 0.5, CaseStatus::Active)];
        let blocked = vec![case("q2", 1.0, CaseStatus::KnownRed)];

        let agg_active_only = aggregate(&active);
        assert_eq!(agg_active_only.recall_10, 0.5, "blocked case must not be part of this average");

        // Sanity: had blocked been included, the average would differ.
        let combined: Vec<CaseResult> = active.into_iter().chain(blocked).collect();
        let agg_combined = aggregate(&combined);
        assert_ne!(
            agg_active_only.recall_10, agg_combined.recall_10,
            "the two aggregates must differ, proving inclusion of the blocked case would have changed the number"
        );
    }

    #[test]
    fn aggregate_of_empty_slice_is_all_zero_not_nan() {
        let agg = aggregate(&[]);
        assert_eq!(agg.recall_1, 0.0);
        assert_eq!(agg.recall_10, 0.0);
        assert_eq!(agg.mrr, 0.0);
        assert_eq!(agg.ndcg_10, 0.0);
    }

    fn minimal_results(date: &str, mode: Mode) -> RunResults {
        RunResults {
            date: date.to_string(),
            mode,
            model_id: "fake-embedder".to_string(),
            git_sha: "deadbee".to_string(),
            k_pool: KPool { memory: 30, episodic: 5, procedural: 5 },
            k_final: 10,
            corpus_size: 1,
            n_retrieval_cases: 1,
            n_contradiction_pairs: 0,
            known_bugs_live: vec!["fake-embedder-unnormalized-contradiction-sweep-degenerate".to_string()],
            golden_set: GoldenSetProvenance::derive(1, 0),
            retrieval: RetrievalResults {
                aggregate: Aggregate::default(),
                by_lang: vec![],
                per_case: vec![],
                blocked: vec![],
            },
            contradiction: ContradictionResults { curve: vec![], by_lang: vec![] },
            norm_check: NormCheck { asserted: false, mean_norm: 11.3, max_deviation: 10.3 },
            audit: None,
        }
    }

    #[test]
    fn an_unaudited_run_omits_the_audit_key_entirely() {
        // Serializing it as `null` would read as "audited, found nothing",
        // which is a different claim from "never audited".
        let out_dir = std::env::temp_dir().join(format!("rag-eval-report-test-{}", uuid::Uuid::new_v4()));
        let results = minimal_results("2026-07-27", Mode::Fake);
        let path = write_json(&results, &out_dir, false).expect("write must succeed");
        let written = std::fs::read_to_string(&path).expect("must read back");
        let json: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
        assert!(json.get("audit").is_none(), "audit key must be absent, got {:?}", json.get("audit"));
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[test]
    fn write_json_creates_the_out_dir_and_writes_the_expected_filename() {
        let out_dir = std::env::temp_dir().join(format!("rag-eval-report-test-{}", uuid::Uuid::new_v4()));
        let results = minimal_results("2026-07-27", Mode::Fake);
        let path = write_json(&results, &out_dir, false).expect("first write must succeed");
        assert_eq!(path, out_dir.join("2026-07-27-fake.json"));
        assert!(path.exists());
        let written = std::fs::read_to_string(&path).expect("must be able to read back the file");
        assert!(written.contains("\"recall@1\""));
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[test]
    fn write_json_refuses_to_overwrite_without_force() {
        let out_dir = std::env::temp_dir().join(format!("rag-eval-report-test-{}", uuid::Uuid::new_v4()));
        let results = minimal_results("2026-07-27", Mode::Real);
        write_json(&results, &out_dir, false).expect("first write must succeed");

        let err = write_json(&results, &out_dir, false).expect_err("second write without --force must fail");
        match err {
            ReportError::AlreadyExists { path } => {
                assert_eq!(path, out_dir.join("2026-07-27-real.json"));
            }
            other => panic!("expected AlreadyExists, got {other:?}"),
        }
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[test]
    fn write_json_overwrites_when_force_is_set() {
        let out_dir = std::env::temp_dir().join(format!("rag-eval-report-test-{}", uuid::Uuid::new_v4()));
        let mut results = minimal_results("2026-07-27", Mode::Fake);
        write_json(&results, &out_dir, false).expect("first write must succeed");

        results.corpus_size = 99;
        write_json(&results, &out_dir, true).expect("write with --force must succeed");
        let written = std::fs::read_to_string(out_dir.join("2026-07-27-fake.json")).unwrap();
        assert!(written.contains("\"corpus_size\": 99"));
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[test]
    fn results_filename_uses_mode_as_the_config_slug() {
        assert_eq!(results_filename("2026-07-27", Mode::Fake), "2026-07-27-fake.json");
        assert_eq!(results_filename("2026-07-27", Mode::Real), "2026-07-27-real.json");
    }
}
