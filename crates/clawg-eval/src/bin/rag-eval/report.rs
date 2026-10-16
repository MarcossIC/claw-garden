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

use clawg_core::eval_api::EvalScoringConfig;
use serde::{Deserialize, Serialize};

use crate::golden::{CaseStatus, CaseSubset};
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
    /// The lexical-channel candidate budget this run actually searched with
    /// — `None` means the channel was off. Recorded so a reader of the
    /// results JSON can tell which channel config produced these numbers
    /// without having to re-derive it from `scoring_config` (which does not
    /// carry it — the lexical budget is a pool-size knob, not a scoring one).
    pub lexical: Option<usize>,
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

/// `Deserialize` is added here for `diff.rs`'s benefit, not production's: a
/// results file is written once by [`write_json`] and read back only by the
/// paired-diff tool, which needs to turn `retrieval.per_case`/
/// `similarity.per_case` JSON arrays back into this type to join them on
/// `query`.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// `#[serde(default)]` so a results file written before the label existed
    /// still reads back in `--diff`.
    #[serde(default)]
    pub subset: CaseSubset,
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
    pub hits: Hits,
}

/// Retrieval quality restricted to the cases of one subset. The language split
/// cannot tell an identifier lookup from a paraphrase, and averaging the two
/// is what hides a lexical weakness behind natural-language wins.
#[derive(Debug, Clone, Serialize)]
pub struct SubsetAggregate {
    pub subset: CaseSubset,
    pub n_cases: usize,
    pub aggregate: Aggregate,
    pub hits: Hits,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetrievalResults {
    pub aggregate: Aggregate,
    /// How many cases found every expected row (strict) and at least one
    /// (partial) in the top 1, 5 and 10, with their Wilson intervals: the
    /// fractional recall in `aggregate` cannot say how many cases it is made of.
    pub hits: Hits,
    /// Per-language split of `aggregate`, over the same active cases. Cases
    /// with no unanimous language are absent here but present in `aggregate`,
    /// so these sub-counts need not sum to `per_case.len()`.
    pub by_lang: Vec<LangAggregate>,
    /// Per-subset split of `aggregate`, over the same active cases. Only the
    /// subsets present appear; the counts sum to the number of active cases.
    pub by_subset: Vec<SubsetAggregate>,
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
    /// SHA-256 of the exact golden-set bytes this run scored against
    /// (`golden::LoadedGolden::content_sha256`). Makes "were these two runs
    /// scored against the same set" a machine-checkable equality instead of
    /// an eyeballed guess: two results files with different hashes here were
    /// never comparable, whether or not anything else about them looks alike.
    pub content_sha256: String,
    /// Fewest cases that must move one way, none the other, for a paired
    /// comparison of two runs on this set to reach p < 0.05.
    pub min_one_way_cases: usize,
}

/// Case and pair counts below which a golden set cannot carry a claim. Sized
/// for a domain set (~30–50 retrieval cases, ~20–30 contradiction pairs)
/// rather than picked arbitrarily: a set that reaches them is large enough to
/// support a comparison.
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
    pub fn derive(n_cases: usize, n_pairs: usize, content_sha256: String) -> Self {
        let min_one_way_cases = crate::stats::min_one_way_significant();
        let power = if n_cases == 0 {
            format!("a paired comparison needs at least {min_one_way_cases} cases moving one way")
        } else {
            format!(
                "a paired comparison needs at least {min_one_way_cases} cases moving one way \
                 ({:.1} points of {n_cases})",
                100.0 * min_one_way_cases as f64 / n_cases as f64
            )
        };
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
                         {n_pairs} contradiction pairs; {power}"
                    ),
                    content_sha256,
                    min_one_way_cases,
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
                 for reasons that have nothing to do with retrieval quality; {power}"
            ),
            content_sha256,
            min_one_way_cases,
        }
    }
}

/// The pure-cosine measurement block: retrieval quality scored by ranking
/// raw embeddings directly, bypassing the store's own scoring path entirely.
/// `aggregate`/`per_case` are built through the exact same `metrics.rs`
/// functions the ranked path uses — there is no second metric
/// implementation that a `sim` number could disagree with a `ranked` number
/// by being wrong in its own, different way.
#[derive(Debug, Clone, Serialize)]
pub struct SimilarityResults {
    pub aggregate: Aggregate,
    pub per_case: Vec<CaseResult>,
    /// Query-anchored: each active case's weakest expected match minus its
    /// strongest rival, meaned across cases. Not comparable to a mined
    /// distractor-gap statistic — see `mean_corpus_distractor_gap` for the
    /// field that is.
    pub mean_expected_distractor_gap: f64,
    /// Doc-anchored, same `cosine`/`1 - max` shape as the golden-set miner's
    /// own statistic: comparable to it directly, pre- vs post-change, on the
    /// frozen set's unchanged distractor structure.
    pub mean_corpus_distractor_gap: f64,
    /// Seven-number summary of every (query, corpus-doc) cosine value
    /// gathered while ranking `per_case`/`aggregate` — the empirical cosine
    /// distribution of the corpus in this run's actual embedding space, not
    /// an assumed one. This is the measured floor a later re-scaling step
    /// reads from; it must never be replaced with a hand-picked constant,
    /// because a prefixed or re-trained embedder can shift it without
    /// warning.
    pub cosine_min: f64,
    pub cosine_p5: f64,
    pub cosine_p25: f64,
    pub cosine_p50: f64,
    pub cosine_p75: f64,
    pub cosine_p95: f64,
    pub cosine_max: f64,
    /// The prefix strings actually applied to queries/passages during this
    /// run, copied from the live embedder. An empty string here is not a
    /// placeholder — it is the true value for a run that predates prefixing,
    /// and a non-empty value on a run meant to be a baseline is self-refuting
    /// evidence that the wrong code path ran.
    pub query_prefix: String,
    pub passage_prefix: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResults {
    pub date: String,
    pub mode: Mode,
    /// Operator-supplied slug distinguishing this run from another of the
    /// same date and mode — e.g. `pre-prefix` vs `e5-prefix` for the two
    /// gate runs either side of a re-embedding change. Absent by default:
    /// most runs need no tag, and folding it into `mode` would force every
    /// caller to invent one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub model_id: String,
    pub git_sha: String,
    /// The FTS5 tokenizer `memory_fts` was carrying when these numbers were
    /// produced. Sits beside `model_id` rather than inside `k_pool` because
    /// it answers the same kind of question — which instrument produced this
    /// reading — while `k_pool` carries pool SIZES, and a tokenizer is not
    /// one. Always the effective value, never "no flag was passed": a
    /// tokenizer comparison whose files do not each name their own tokenizer
    /// is two numbers and a memory of which terminal produced them.
    pub fts_tokenizer: String,
    pub k_pool: KPool,
    pub k_final: usize,
    pub corpus_size: usize,
    pub n_retrieval_cases: usize,
    pub n_contradiction_pairs: usize,
    /// The scoring config every hit in this run was ranked with. Always
    /// serialized, never `skip_serializing_if`: every run scores with SOME
    /// config, so absence in an old file means only "written by a build that
    /// predates this field", not "not applicable to this run" — unlike
    /// `audit`/`similarity`, which really can be legitimately absent.
    pub scoring_config: EvalScoringConfig,
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
    /// Present only when the run computed the pure-cosine similarity block —
    /// never under `--fake`, since `FakeEmbedder` has no learned geometry for
    /// a cosine measurement to describe. Absent, not null, for the same
    /// reason `audit` is: a reader must be able to tell "not computed" from
    /// "computed and empty".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<SimilarityResults>,
    /// Present only for `--agent-view` runs, which score the text the MCP
    /// tools return rather than the store's ranking. Absent otherwise, so every
    /// earlier results file reads as a run that never measured it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_view: Option<crate::agent_view::AgentViewResults>,
}

impl RetrievalResults {
    /// Scores the active cases and keeps the blocked ones beside them, every
    /// split computed from the same `per_case`.
    pub fn from_cases(per_case: Vec<CaseResult>, blocked: Vec<CaseResult>) -> Self {
        Self {
            aggregate: aggregate(&per_case),
            hits: hits(&per_case),
            by_lang: aggregate_by_lang(&per_case),
            by_subset: aggregate_by_subset(&per_case),
            per_case,
            blocked,
        }
    }
}

/// A count of cases out of `n_cases` and its 95 % Wilson interval.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Rate {
    pub hits: usize,
    pub n_cases: usize,
    pub rate: f64,
    pub lo: f64,
    pub hi: f64,
}

impl Rate {
    pub fn of(hits: usize, n_cases: usize) -> Self {
        let (lo, hi) = crate::stats::wilson(hits, n_cases);
        let rate = if n_cases == 0 { 0.0 } else { hits as f64 / n_cases as f64 };
        Self { hits, n_cases, rate, lo, hi }
    }
}

/// Hits at one cutoff. With a single expected row the two are the same; with
/// several, `partial` is the share that found any of them and `strict` the
/// share that found all.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Hit {
    pub strict: Rate,
    pub partial: Rate,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Hits {
    pub at_1: Hit,
    pub at_5: Hit,
    pub at_10: Hit,
}

/// Counts hits from each case's `recall@k`: the share of its expected rows in
/// the top k, so 1 means all of them and anything above 0 at least one.
pub fn hits(cases: &[CaseResult]) -> Hits {
    let n = cases.len();
    let at = |recall: fn(&CaseResult) -> f64| Hit {
        strict: Rate::of(cases.iter().filter(|c| recall(c) >= 1.0).count(), n),
        partial: Rate::of(cases.iter().filter(|c| recall(c) > 0.0).count(), n),
    };
    Hits { at_1: at(|c| c.recall_1), at_5: at(|c| c.recall_5), at_10: at(|c| c.recall_10) }
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
            hits: hits(&group),
        })
        .collect()
}

/// Splits `cases` by `CaseResult::subset` and aggregates each group on its
/// own. Ordered by the serialized subset name so two runs of the same golden
/// set produce byte-comparable JSON.
pub fn aggregate_by_subset(cases: &[CaseResult]) -> Vec<SubsetAggregate> {
    let mut grouped: BTreeMap<&str, (CaseSubset, Vec<CaseResult>)> = BTreeMap::new();
    for case in cases {
        grouped
            .entry(case.subset.as_str())
            .or_insert_with(|| (case.subset, Vec::new()))
            .1
            .push(case.clone());
    }
    grouped
        .into_values()
        .map(|(subset, group)| SubsetAggregate {
            subset,
            n_cases: group.len(),
            aggregate: aggregate(&group),
            hits: hits(&group),
        })
        .collect()
}

fn format_hit(name: &str, hit: &Hit) -> String {
    let rate = |r: &Rate| format!("{}/{} = {:.4} [{:.3}, {:.3}]", r.hits, r.n_cases, r.rate, r.lo, r.hi);
    format!("{name} strict {}  partial {}", rate(&hit.strict), rate(&hit.partial))
}

fn format_subset_table(split: &[SubsetAggregate]) -> String {
    let mut out = String::from("    subset            n   recall@1  recall@5  recall@10     mrr   ndcg@10
");
    for s in split {
        out.push_str(&format!(
            "    {:<15} {:>3}  {:>9.4} {:>9.4} {:>10.4} {:>7.4} {:>9.4}
",
            s.subset.as_str(),
            s.n_cases,
            s.aggregate.recall_1,
            s.aggregate.recall_5,
            s.aggregate.recall_10,
            s.aggregate.mrr,
            s.aggregate.ndcg_10
        ));
    }
    out
}

/// The on-disk filename for a run's results: `{date}-{mode}.json`, matching
/// the `docs/eval/results/{fecha}-{config}.json` pattern with `mode`
/// standing in for `config` (the only axis this harness varies today) —
/// unless `tag` is given, in which case it becomes `{date}-{mode}-{tag}.json`.
/// The tag exists so two real runs on the same day (a baseline and its
/// follow-up) can each get their own file without either one reaching for
/// `--force`, which would let a rerun silently replace the very baseline it
/// was meant to be compared against.
pub fn results_filename(date: &str, mode: Mode, tag: Option<&str>) -> String {
    let config = match mode {
        Mode::Fake => "fake",
        Mode::Real => "real",
    };
    match tag {
        Some(tag) => format!("{date}-{config}-{tag}.json"),
        None => format!("{date}-{config}.json"),
    }
}

/// One grid cell of a sweep: the config that produced it, where its own
/// full `RunResults` file landed, and the two audit numbers the sweep gate reads
/// (`importance_rank_dominance`/`modal_rank_agreement`) — computed for every
/// point regardless of whether `--audit` was passed, since the sweep gate
/// needs them unconditionally, not only when a full `AuditReport` was asked
/// for.
#[derive(Debug, Clone, Serialize)]
pub struct SweepPoint {
    pub slug: String,
    pub results_file: String,
    pub scoring_config: EvalScoringConfig,
    pub aggregate: Aggregate,
    pub importance_rank_dominance: f64,
    pub modal_rank_agreement: f64,
}

/// The curve index a sweep run writes alongside its N `RunResults` files —
/// one row per grid cell, so a reader can see every point's headline numbers
/// without opening each file individually. Never the results themselves:
/// see `RunResults` in the file-changes rationale — every existing consumer
/// (the paired diff tool, `write_json`'s no-clobber rule, human diffing)
/// takes a whole `RunResults` as its unit, and this index is additive to
/// that, not a replacement for it.
#[derive(Debug, Clone, Serialize)]
pub struct SweepIndex {
    pub date: String,
    pub mode: Mode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub model_id: String,
    pub git_sha: String,
    pub points: Vec<SweepPoint>,
}

/// The importance factor's value at its own `clamp_lo` — i.e. the swept
/// `min` a point's config was built with. Recovered from the stored config
/// rather than carried as a separate field: `EvalScoringConfig` already
/// determines it exactly (`base + clamp_lo/divisor`), and a second field
/// holding the same number could drift from the config that actually ran.
fn imp_min_of(cfg: &EvalScoringConfig) -> f64 {
    cfg.importance_base + cfg.importance_clamp_lo / cfg.importance_divisor
}

/// Sorts sweep points by `(imp_min, cos_floor)` ascending so two sweeps over
/// the same grid produce byte-comparable JSON. `None` (no cosine re-scale) sorts before every
/// `Some` floor: it is the "off" position of that axis, logically less than
/// any active floor value rather than an arbitrary tie-break.
pub fn sort_sweep_points(points: &mut [SweepPoint]) {
    points.sort_by(|a, b| {
        let a_imp = imp_min_of(&a.scoring_config);
        let b_imp = imp_min_of(&b.scoring_config);
        let a_floor = a.scoring_config.cosine_floor.unwrap_or(f64::NEG_INFINITY);
        let b_floor = b.scoring_config.cosine_floor.unwrap_or(f64::NEG_INFINITY);
        a_imp.total_cmp(&b_imp).then_with(|| a_floor.total_cmp(&b_floor))
    });
}

/// Writes a sweep's curve index to `{out_dir}/{date}-{mode}[-{tag}]-sweep.json`
/// — the same no-clobber-unless-`--force` discipline [`write_json`] applies,
/// for the same reason: this file is also a measurement artifact someone
/// will read later, not a scratch file.
pub fn write_sweep_index(index: &SweepIndex, out_dir: &Path, force: bool) -> Result<PathBuf, ReportError> {
    std::fs::create_dir_all(out_dir)
        .map_err(|e| ReportError::CreateDir { path: out_dir.to_path_buf(), source: e })?;
    let config = match index.mode {
        Mode::Fake => "fake",
        Mode::Real => "real",
    };
    let filename = match &index.tag {
        Some(tag) => format!("{}-{config}-{tag}-sweep.json", index.date),
        None => format!("{}-{config}-sweep.json", index.date),
    };
    let path = out_dir.join(filename);
    if path.exists() && !force {
        return Err(ReportError::AlreadyExists { path });
    }
    let json = serde_json::to_string_pretty(index)?;
    std::fs::write(&path, json).map_err(|e| ReportError::Write { path: path.clone(), source: e })?;
    Ok(path)
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
    let path = out_dir.join(results_filename(&results.date, results.mode, results.tag.as_deref()));
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
    println!("    {}", format_hit("hits@5", &results.retrieval.hits.at_5));
    println!("  {}", results.golden_set.note);
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
    if !results.retrieval.by_subset.is_empty() {
        println!("  retrieval by subset:");
        print!("{}", format_subset_table(&results.retrieval.by_subset));
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
        let p = GoldenSetProvenance::derive(46, 28, "deadbeef".to_string());
        assert!(!p.provisional);
        assert!(p.note.contains("46 retrieval cases"), "note was: {}", p.note);
    }

    #[test]
    fn provenance_carries_the_content_hash_it_was_given() {
        // Not re-derived here: the hash describes the exact bytes `golden::load`
        // read, and `derive` has no file to re-hash — it must simply carry the
        // value through, so two results files can be compared without either
        // one being reopened.
        let p = GoldenSetProvenance::derive(46, 28, "abc123".to_string());
        assert_eq!(p.content_sha256, "abc123");
    }

    #[test]
    fn a_set_exactly_at_both_thresholds_is_not_provisional() {
        // The boundary is the one place an off-by-one would flip the meaning
        // of every results file written that day.
        assert!(
            !GoldenSetProvenance::derive(DOMAIN_SET_MIN_CASES, DOMAIN_SET_MIN_PAIRS, "h".to_string())
                .provisional
        );
    }

    #[test]
    fn too_few_cases_stays_provisional_and_says_which_count_is_short() {
        let p = GoldenSetProvenance::derive(13, 28, "h".to_string());
        assert!(p.provisional);
        assert!(p.note.contains("13 retrieval cases"), "note was: {}", p.note);
        assert!(!p.note.contains("contradiction pairs (needs"), "note was: {}", p.note);
    }

    #[test]
    fn too_few_pairs_stays_provisional_even_with_plenty_of_cases() {
        let p = GoldenSetProvenance::derive(46, 9, "h".to_string());
        assert!(p.provisional);
        assert!(p.note.contains("9 contradiction pairs"), "note was: {}", p.note);
    }

    #[test]
    fn short_on_both_counts_reports_both() {
        let p = GoldenSetProvenance::derive(13, 9, "h".to_string());
        assert!(p.provisional);
        assert!(p.note.contains("13 retrieval cases"), "note was: {}", p.note);
        assert!(p.note.contains("9 contradiction pairs"), "note was: {}", p.note);
    }

    #[test]
    fn an_empty_golden_set_is_provisional() {
        assert!(GoldenSetProvenance::derive(0, 0, "h".to_string()).provisional);
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
            subset: CaseSubset::Natural,
        }
    }

    fn case_in(query: &str, recall_10: f64, lang: Option<&str>) -> CaseResult {
        CaseResult { lang: lang.map(str::to_string), ..case(query, recall_10, CaseStatus::Active) }
    }

    fn multi(query: &str, recall_5: f64) -> CaseResult {
        let mut c = case(query, recall_5, CaseStatus::Active);
        c.recall_5 = recall_5;
        c.expected_refs = vec!["a".to_string(), "b".to_string()];
        c
    }

    #[test]
    fn a_case_with_one_of_two_expected_rows_is_a_partial_but_not_a_strict_hit() {
        let h = hits(&[multi("q", 0.5)]);
        assert_eq!((h.at_5.strict.hits, h.at_5.partial.hits), (0, 1));
        assert_eq!(aggregate(&[multi("q", 0.5)]).recall_5, 0.5, "the fractional recall is unchanged");
    }

    #[test]
    fn hits_carry_the_wilson_interval_of_their_count() {
        let cases: Vec<CaseResult> =
            (0..46).map(|i| multi(&format!("q{i}"), if i < 36 { 1.0 } else { 0.0 })).collect();
        let r = hits(&cases).at_5.strict;
        assert_eq!((r.hits, r.n_cases), (36, 46));
        assert!((r.lo - 0.644).abs() < 0.001 && (r.hi - 0.877).abs() < 0.001, "{r:?}");
        let lang = &RetrievalResults::from_cases(cases.clone(), vec![]).by_subset[0];
        assert_eq!(lang.hits.at_5.strict, r, "every split block carries its own hits");
    }

    #[test]
    fn the_note_says_how_many_one_way_cases_a_conclusion_needs() {
        let p = GoldenSetProvenance::derive(46, 28, "h".to_string());
        assert_eq!(p.min_one_way_cases, 6);
        assert!(p.note.contains("at least 6 cases moving one way (13.0 points of 46)"), "{}", p.note);
        assert!(!p.provisional, "the shape check does not change");
        assert!(GoldenSetProvenance::derive(0, 0, "h".to_string()).note.contains("at least 6 cases"));
    }

    #[test]
    fn a_case_result_without_a_subset_reads_back_as_natural() {
        let mut json = serde_json::to_value(case("q", 1.0, CaseStatus::Active)).unwrap();
        json.as_object_mut().unwrap().remove("subset");
        let back: CaseResult = serde_json::from_value(json).unwrap();
        assert_eq!(back.subset, CaseSubset::Natural);
    }

    #[test]
    fn a_lexical_case_result_round_trips() {
        let lexical = CaseResult { subset: CaseSubset::Lexical, ..case("q", 1.0, CaseStatus::Active) };
        let json = serde_json::to_value(&lexical).unwrap();
        assert_eq!(json["subset"], "lexical");
        let back: CaseResult = serde_json::from_value(json).unwrap();
        assert_eq!(back.subset, CaseSubset::Lexical);
    }

    fn case_of(query: &str, recall_5: f64, subset: CaseSubset) -> CaseResult {
        CaseResult { recall_5, subset, ..case(query, recall_5, CaseStatus::Active) }
    }

    #[test]
    fn aggregate_by_subset_scores_each_subset_on_its_own_cases_only() {
        let cases = vec![
            case_of("n1", 1.0, CaseSubset::Natural),
            case_of("l1", 0.0, CaseSubset::Lexical),
            case_of("l2", 1.0, CaseSubset::Lexical),
        ];
        let split = aggregate_by_subset(&cases);

        // Ordered by the serialized name so two runs produce comparable JSON.
        assert_eq!(
            split.iter().map(|s| s.subset).collect::<Vec<_>>(),
            vec![CaseSubset::Lexical, CaseSubset::Natural]
        );
        assert_eq!(split[0].n_cases, 2);
        assert_eq!(split[0].aggregate.recall_5, 0.5);
        assert_eq!(split[1].n_cases, 1);
        assert_eq!(split[1].aggregate.recall_5, 1.0);
    }

    #[test]
    fn aggregate_by_subset_only_lists_the_subsets_present() {
        let cases = vec![case_of("m1", 1.0, CaseSubset::MultiRef)];
        let split = aggregate_by_subset(&cases);
        assert_eq!(split.len(), 1);
        assert_eq!(split[0].subset, CaseSubset::MultiRef);
    }

    #[test]
    fn with_only_natural_cases_the_single_bucket_equals_the_global_aggregate() {
        let cases = vec![case_of("a", 1.0, CaseSubset::Natural), case_of("b", 0.0, CaseSubset::Natural)];
        let split = aggregate_by_subset(&cases);
        let global = aggregate(&cases);
        assert_eq!(split.len(), 1);
        assert_eq!(split[0].subset, CaseSubset::Natural);
        assert_eq!(split[0].n_cases, 2);
        assert_eq!(split[0].aggregate.recall_5, global.recall_5);
        assert_eq!(split[0].aggregate.mrr, global.mrr);
    }

    #[test]
    fn an_empty_case_list_has_no_subset_buckets() {
        assert!(aggregate_by_subset(&[]).is_empty());
    }

    #[test]
    fn the_stdout_table_names_every_subset() {
        let split = aggregate_by_subset(&[
            case_of("n1", 1.0, CaseSubset::Natural),
            case_of("l1", 0.0, CaseSubset::Lexical),
        ]);
        let table = format_subset_table(&split);
        assert!(table.contains("lexical"), "{table}");
        assert!(table.contains("natural"), "{table}");
    }

    #[test]
    fn a_results_file_without_by_subset_still_reads_and_diffs() {
        let mut old = serde_json::to_value(minimal_results("2026-07-27", Mode::Fake)).unwrap();
        old["retrieval"].as_object_mut().unwrap().remove("by_subset");
        let mut with_subset = minimal_results("2026-07-27", Mode::Fake);
        with_subset.retrieval.per_case = vec![case_of("q", 1.0, CaseSubset::Lexical)];
        with_subset.retrieval.by_subset = aggregate_by_subset(&with_subset.retrieval.per_case);
        let new = serde_json::to_value(&with_subset).unwrap();
        assert!(new["retrieval"]["by_subset"].is_array());

        let label = std::path::Path::new("results.json");
        crate::diff::diff_values(&old, label, &new, label).expect("a missing by_subset must not break the diff");
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
            tag: None,
            model_id: "fake-embedder".to_string(),
            git_sha: "deadbee".to_string(),
            fts_tokenizer: "unicode61 remove_diacritics 2".to_string(),
            k_pool: KPool { memory: 30, episodic: 5, procedural: 5, lexical: None },
            k_final: 10,
            corpus_size: 1,
            n_retrieval_cases: 1,
            n_contradiction_pairs: 0,
            scoring_config: EvalScoringConfig::default(),
            known_bugs_live: vec!["fake-embedder-unnormalized-contradiction-sweep-degenerate".to_string()],
            golden_set: GoldenSetProvenance::derive(1, 0, "h".to_string()),
            retrieval: RetrievalResults::from_cases(vec![], vec![]),
            contradiction: ContradictionResults { curve: vec![], by_lang: vec![] },
            norm_check: NormCheck { asserted: false, mean_norm: 11.3, max_deviation: 10.3 },
            audit: None,
            similarity: None,
            agent_view: None,
        }
    }

    fn minimal_agent_view_results() -> crate::agent_view::AgentViewResults {
        crate::agent_view::AgentViewResults {
            corpus_size: 3,
            stored_rows: 4,
            min_segment_chars: 40,
            ambiguous_refs: vec![],
            unmatchable_refs: vec!["c3".to_string()],
            search: Default::default(),
            context: Default::default(),
            by_subset: Default::default(),
            per_case: vec![],
        }
    }

    fn minimal_similarity_results() -> SimilarityResults {
        SimilarityResults {
            aggregate: Aggregate::default(),
            per_case: vec![],
            mean_expected_distractor_gap: 0.1,
            mean_corpus_distractor_gap: 0.2,
            cosine_min: 0.3,
            cosine_p5: 0.4,
            cosine_p25: 0.5,
            cosine_p50: 0.6,
            cosine_p75: 0.7,
            cosine_p95: 0.8,
            cosine_max: 0.9,
            query_prefix: "".to_string(),
            passage_prefix: "".to_string(),
        }
    }

    #[test]
    fn an_unmeasured_run_omits_the_similarity_key_entirely() {
        // Absent, not null: a reader must be able to tell "this run predates
        // the similarity instrument" from "it was computed and found nothing".
        let out_dir = std::env::temp_dir().join(format!("rag-eval-report-test-{}", uuid::Uuid::new_v4()));
        let results = minimal_results("2026-07-27", Mode::Fake);
        let path = write_json(&results, &out_dir, false).expect("write must succeed");
        let written = std::fs::read_to_string(&path).expect("must read back");
        let json: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
        assert!(json.get("similarity").is_none(), "similarity key must be absent, got {:?}", json.get("similarity"));
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[test]
    fn an_ordinary_run_omits_the_agent_view_key_entirely() {
        let json = serde_json::to_value(minimal_results("2026-07-27", Mode::Fake)).unwrap();
        assert!(json.get("agent_view").is_none(), "got {:?}", json.get("agent_view"));
    }

    #[test]
    fn an_agent_view_run_serializes_the_block_and_it_reads_back() {
        let mut results = minimal_results("2026-07-27", Mode::Fake);
        results.agent_view = Some(minimal_agent_view_results());
        let json = serde_json::to_value(&results).unwrap();
        let back: crate::agent_view::AgentViewResults =
            serde_json::from_value(json["agent_view"].clone()).expect("the block must read back");
        assert_eq!(back, minimal_agent_view_results());
    }

    #[test]
    fn a_results_file_without_the_block_still_diffs_against_one_with_it() {
        let case = |recall_5: f64| {
            serde_json::json!({
                "query": "how is the cache invalidated",
                "expected_refs": ["c1"],
                "ranked_refs": ["c1"],
                "recall@1": 1.0,
                "recall@5": recall_5,
                "recall@10": 1.0,
                "mrr": 1.0,
                "ndcg@10": 1.0,
                "status": "active",
                "lang": "en",
            })
        };
        let mut old = serde_json::to_value(minimal_results("2026-07-27", Mode::Fake)).unwrap();
        old["retrieval"]["per_case"] = serde_json::json!([case(0.0)]);
        assert!(old.get("agent_view").is_none(), "the old shape has no block");

        let mut with_block = minimal_results("2026-07-27", Mode::Fake);
        with_block.agent_view = Some(minimal_agent_view_results());
        let mut new = serde_json::to_value(&with_block).unwrap();
        new["retrieval"]["per_case"] = serde_json::json!([case(1.0)]);

        let label = std::path::Path::new("results.json");
        let diff = crate::diff::diff_values(&old, label, &new, label)
            .expect("a block it ignores must not break the diff");
        assert_eq!(diff.retrieval.recall_5.better, 1, "the one case must have been paired");
    }

    #[test]
    fn a_measured_run_serializes_the_similarity_block_with_both_prefixes() {
        let out_dir = std::env::temp_dir().join(format!("rag-eval-report-test-{}", uuid::Uuid::new_v4()));
        let mut results = minimal_results("2026-07-27", Mode::Real);
        results.similarity = Some(minimal_similarity_results());
        let path = write_json(&results, &out_dir, false).expect("write must succeed");
        let written = std::fs::read_to_string(&path).expect("must read back");
        let json: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
        assert!(json.get("similarity").is_some(), "similarity key must be present when computed");
        assert_eq!(json["similarity"]["query_prefix"], "");
        assert_eq!(json["similarity"]["passage_prefix"], "");
        assert_eq!(json["similarity"]["mean_expected_distractor_gap"], 0.1);
        assert_eq!(json["similarity"]["mean_corpus_distractor_gap"], 0.2);
        assert_eq!(json["similarity"]["cosine_min"], 0.3);
        assert_eq!(json["similarity"]["cosine_p5"], 0.4);
        assert_eq!(json["similarity"]["cosine_p25"], 0.5);
        assert_eq!(json["similarity"]["cosine_p50"], 0.6);
        assert_eq!(json["similarity"]["cosine_p75"], 0.7);
        assert_eq!(json["similarity"]["cosine_p95"], 0.8);
        assert_eq!(json["similarity"]["cosine_max"], 0.9);
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[test]
    fn scoring_config_is_always_serialized_and_round_trips_every_field() {
        let out_dir = std::env::temp_dir().join(format!("rag-eval-report-test-{}", uuid::Uuid::new_v4()));
        let mut results = minimal_results("2026-07-27", Mode::Real);
        results.scoring_config = EvalScoringConfig {
            importance_base: 0.7,
            importance_divisor: 15.0,
            importance_clamp_lo: 1.0,
            importance_clamp_hi: 10.0,
            cosine_floor: Some(0.8625),
        };
        let path = write_json(&results, &out_dir, false).expect("write must succeed");
        let written = std::fs::read_to_string(&path).expect("must read back");
        let json: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");

        // Present unconditionally — no skip_serializing_if, unlike audit/similarity.
        assert!(json.get("scoring_config").is_some(), "scoring_config key must always be present");
        assert_eq!(json["scoring_config"]["importance_base"], 0.7);
        assert_eq!(json["scoring_config"]["importance_divisor"], 15.0);
        assert_eq!(json["scoring_config"]["importance_clamp_lo"], 1.0);
        assert_eq!(json["scoring_config"]["importance_clamp_hi"], 10.0);
        assert_eq!(json["scoring_config"]["cosine_floor"], 0.8625);
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[test]
    fn scoring_config_is_present_even_at_default_values() {
        // The zero point of a sweep is not "nothing to report" — it is the
        // one config every non-sweep run has always scored with.
        let out_dir = std::env::temp_dir().join(format!("rag-eval-report-test-{}", uuid::Uuid::new_v4()));
        let results = minimal_results("2026-07-27", Mode::Fake);
        let path = write_json(&results, &out_dir, false).expect("write must succeed");
        let written = std::fs::read_to_string(&path).expect("must read back");
        let json: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
        assert!(json.get("scoring_config").is_some());
        // Derived from EvalScoringConfig::default() rather than a bare
        // literal: this test's subject is "the default's own floor got
        // serialized", not "the floor happens to be this specific number" —
        // it must keep testing the former even if the latter changes again.
        let expected_floor = match EvalScoringConfig::default().cosine_floor {
            Some(f) => serde_json::json!(f),
            None => serde_json::Value::Null,
        };
        assert_eq!(json["scoring_config"]["cosine_floor"], expected_floor);
        std::fs::remove_dir_all(&out_dir).ok();
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

    // --- SweepIndex ---------------------------------------------------------

    fn sweep_point(imp_min: f64, cos_floor: Option<f64>) -> SweepPoint {
        // Sets `cosine_floor` directly to whatever the caller asked for
        // (including `None`) rather than through `with_cosine_floor`, which
        // can only ever set a `Some` — leaving a `None` request untouched
        // would silently inherit the default's own floor instead of the
        // test's intended "off" position on this axis.
        let mut scoring_config = EvalScoringConfig::default().with_importance_span(imp_min, 1.0);
        scoring_config.cosine_floor = cos_floor;
        SweepPoint {
            slug: format!("imp{imp_min:.2}"),
            results_file: format!("2026-08-16-real-imp{imp_min:.2}.json"),
            scoring_config,
            aggregate: Aggregate::default(),
            importance_rank_dominance: 0.5,
            modal_rank_agreement: 0.2,
        }
    }

    #[test]
    fn sweep_index_serializes_the_full_shape() {
        let index = SweepIndex {
            date: "2026-08-16".to_string(),
            mode: Mode::Real,
            tag: Some("floor-sweep".to_string()),
            model_id: "fake-embedder".to_string(),
            git_sha: "deadbee".to_string(),
            points: vec![sweep_point(0.55, Some(0.8625))],
        };
        let json = serde_json::to_value(&index).expect("must serialize");
        assert_eq!(json["date"], "2026-08-16");
        assert_eq!(json["mode"], "real");
        assert_eq!(json["tag"], "floor-sweep");
        assert_eq!(json["model_id"], "fake-embedder");
        assert_eq!(json["git_sha"], "deadbee");
        let points = json["points"].as_array().expect("points must be an array");
        assert_eq!(points.len(), 1);
        let p = &points[0];
        assert_eq!(p["slug"], "imp0.55");
        assert_eq!(p["results_file"], "2026-08-16-real-imp0.55.json");
        assert!(p.get("scoring_config").is_some());
        assert!(p.get("aggregate").is_some());
        assert_eq!(p["importance_rank_dominance"], 0.5);
        assert_eq!(p["modal_rank_agreement"], 0.2);
    }

    #[test]
    fn sort_sweep_points_orders_by_imp_min_then_cos_floor() {
        let mut points = vec![
            sweep_point(0.7, Some(0.90)),
            sweep_point(0.55, None),
            sweep_point(0.7, Some(0.86)),
            sweep_point(0.55, Some(0.86)),
        ];
        sort_sweep_points(&mut points);
        let slugs: Vec<&str> = points.iter().map(|p| p.slug.as_str()).collect();
        // imp_min ascending first (0.55 before 0.70); within a tied imp_min,
        // cos_floor ascending with None (no re-scale) sorting before any Some.
        assert_eq!(
            points.iter().map(|p| (imp_min_of(&p.scoring_config), p.scoring_config.cosine_floor)).collect::<Vec<_>>(),
            vec![(0.55, None), (0.55, Some(0.86)), (0.70, Some(0.86)), (0.70, Some(0.90))],
            "slugs in this order: {slugs:?}"
        );
    }

    #[test]
    fn write_sweep_index_writes_the_expected_filename() {
        let out_dir = std::env::temp_dir().join(format!("rag-eval-sweep-test-{}", uuid::Uuid::new_v4()));
        let index = SweepIndex {
            date: "2026-08-16".to_string(),
            mode: Mode::Real,
            tag: Some("floor-sweep".to_string()),
            model_id: "fake-embedder".to_string(),
            git_sha: "deadbee".to_string(),
            points: vec![sweep_point(0.55, Some(0.8625))],
        };
        let path = write_sweep_index(&index, &out_dir, false).expect("write must succeed");
        assert_eq!(path, out_dir.join("2026-08-16-real-floor-sweep-sweep.json"));
        assert!(path.exists());
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[test]
    fn results_filename_uses_mode_as_the_config_slug() {
        assert_eq!(results_filename("2026-07-27", Mode::Fake, None), "2026-07-27-fake.json");
        assert_eq!(results_filename("2026-07-27", Mode::Real, None), "2026-07-27-real.json");
    }

    #[test]
    fn results_filename_appends_the_tag_when_one_is_given() {
        assert_eq!(
            results_filename("2026-07-27", Mode::Real, Some("pre-prefix")),
            "2026-07-27-real-pre-prefix.json"
        );
        assert_eq!(
            results_filename("2026-07-27", Mode::Fake, Some("e5-prefix")),
            "2026-07-27-fake-e5-prefix.json"
        );
    }
}
