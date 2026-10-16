//! Paired per-case diff between two result JSONs. Neither block's per-case
//! metrics move for the same reason twice — `retrieval` moves when the
//! store's ranked search changes, `similarity` moves only when the
//! embedding space itself changes — so a diff that folded both into one
//! number would hide which of the two actually moved. Cases are joined on
//! `query`, matching how the golden set itself identifies a case: there is
//! no other stable key shared by two independently-generated results files.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::agent_view::CaseView;
use crate::golden::CaseStatus;
use crate::report::CaseResult;
use crate::stats::{paired_bootstrap_ci, Interval, BOOTSTRAP_RESAMPLES, BOOTSTRAP_SEED};

/// One metric's movement between two runs, counted over every case the join
/// could match. `net` is `better - worse`, not `better + worse`: a metric
/// that improved for five cases and regressed for five is a net-zero wash,
/// not a ten-case swing, and the sweep gate this feeds reads `net`, not the
/// raw counts.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MetricDiff {
    pub better: usize,
    pub worse: usize,
    pub net: i64,
    /// Mean per-case difference (second minus first) and its bootstrap
    /// interval. Only on continuous metrics, where how far a case moved
    /// matters as well as which way.
    pub ci: Option<Interval>,
}

impl MetricDiff {
    /// Exact paired p-value of this movement: how likely a split at least
    /// this lopsided is if neither run is better.
    pub fn p_value(&self) -> f64 {
        crate::stats::mcnemar_exact(self.better, self.worse)
    }
}

/// A primary metric got worse by more than chance explains. The informative
/// metrics never count: testing every row would make a false alarm likely.
pub fn regressed(diff: &RunDiff) -> bool {
    let worse = |m: &MetricDiff| m.worse > m.better && m.p_value() < crate::stats::ALPHA;
    worse(&diff.retrieval.recall_5) || diff.agent_view.is_some_and(|av| worse(&av.search_found))
}

/// How a primary metric's movement reads at `stats::ALPHA`. A movement that
/// does not reach it says how many one-way cases would have, and never that
/// the runs are the same: with a few dozen cases, "not distinguishable" is all
/// the data can say.
pub fn verdict(name: &str, m: &MetricDiff) -> String {
    let p = m.p_value();
    let moved = m.better + m.worse;
    if p < crate::stats::ALPHA {
        let direction = if m.better > m.worse { "better" } else { "worse" };
        format!("{name}: distinguishable, {direction} (p = {p:.4}, {} better / {} worse)", m.better, m.worse)
    } else {
        format!(
            "{name}: not distinguishable with {moved} case(s) moving (p = {p:.4}); \
             a conclusion needs at least {} cases moving one way",
            crate::stats::min_one_way_significant()
        )
    }
}

/// The five per-case metrics' movement for one block (`retrieval` or
/// `similarity`), each scored independently — a case can improve on `mrr`
/// while regressing on `recall@1` in the same run, and collapsing the five
/// into one number would erase exactly that kind of disagreement.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BlockDiff {
    pub recall_1: MetricDiff,
    pub recall_5: MetricDiff,
    pub recall_10: MetricDiff,
    pub mrr: MetricDiff,
    pub ndcg_10: MetricDiff,
}

/// What the agent received, case by case, from two `--agent-view` runs.
/// Fewer tokens counts as better; the interval of a token row is still the
/// raw change, second run minus first.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AgentViewDiff {
    pub search_found: MetricDiff,
    pub context_found: MetricDiff,
    pub search_tokens: MetricDiff,
    pub context_tokens: MetricDiff,
    /// Over `unanswerable` cases only, which stay out of the two `found` rows.
    pub search_abstained: MetricDiff,
}

/// The whole paired diff: `retrieval` and `similarity`, computed the same
/// way but never combined — see the module doc comment for why.
/// `agent_view` is there only when both files measured it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RunDiff {
    pub retrieval: BlockDiff,
    pub similarity: BlockDiff,
    pub agent_view: Option<AgentViewDiff>,
    /// `retrieval.unanswerable[].abstained`, when both files have such cases.
    pub abstention: Option<MetricDiff>,
    /// `fresh_first` of the `update` cases both files scored.
    pub freshness: Option<MetricDiff>,
}

#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    #[error("failed to read '{}': {source}", path.display())]
    Read {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("'{}' is not valid JSON: {source}", path.display())]
    Parse {
        path: std::path::PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("'{block}' in '{}' is not a valid case array: {source}", path.display())]
    Cases {
        path: std::path::PathBuf,
        block: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

/// Reads and parses two results files, then diffs them. The only thing this
/// function adds over [`diff_values`] is the file I/O — every comparison
/// rule lives in the pure function below, so a test never has to write a
/// results file to disk just to exercise the join.
pub fn diff_files(a_path: &Path, b_path: &Path) -> Result<RunDiff, DiffError> {
    let a = read_json(a_path)?;
    let b = read_json(b_path)?;
    diff_values(&a, a_path, &b, b_path)
}

fn read_json(path: &Path) -> Result<Value, DiffError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| DiffError::Read { path: path.to_path_buf(), source: e })?;
    serde_json::from_str(&text).map_err(|e| DiffError::Parse { path: path.to_path_buf(), source: e })
}

/// Extracts the case array at `block` (`retrieval.per_case`, …) as `Vec<CaseResult>`. Absent
/// (`value[block]` missing, e.g. `similarity` under a `--fake` run) is not an
/// error — it degrades to an empty case list, the same way an empty slice
/// degrades every other aggregator in this harness (see
/// `similarity::mean_expected_distractor_gap`). A present-but-malformed
/// block IS an error: that is a results file this tool cannot honestly
/// compare, not a block that was never computed.
fn per_case(value: &Value, path: &Path, block: &'static str) -> Result<Vec<CaseResult>, DiffError> {
    Ok(cases_at(value, path, block)?.unwrap_or_default())
}

/// The case array at `block` (a dotted path such as `retrieval.per_case`), or
/// `None` when the file has no such key.
fn cases_at<T: serde::de::DeserializeOwned>(
    value: &Value,
    path: &Path,
    block: &'static str,
) -> Result<Option<Vec<T>>, DiffError> {
    match value.pointer(&format!("/{}", block.replace('.', "/"))) {
        None => Ok(None),
        Some(cases) => serde_json::from_value(cases.clone())
            .map(Some)
            .map_err(|e| DiffError::Cases { path: path.to_path_buf(), block, source: e }),
    }
}

/// `value.agent_view.per_case`, or `None` when the run did not measure what
/// the agent receives. Same rule as [`per_case`] otherwise: present but
/// malformed is an error.
fn agent_view_cases(value: &Value, path: &Path) -> Result<Option<Vec<CaseView>>, DiffError> {
    cases_at(value, path, "agent_view.per_case")
}

/// The pure join-and-diff: two already-parsed results values, diffed on
/// `retrieval.per_case` and `similarity.per_case` independently. `a_path`/
/// `b_path` are used only to label a [`DiffError::Cases`] with which file was
/// malformed; they carry no meaning for the comparison itself.
pub fn diff_values(a: &Value, a_path: &Path, b: &Value, b_path: &Path) -> Result<RunDiff, DiffError> {
    let a_retrieval = per_case(a, a_path, "retrieval.per_case")?;
    let b_retrieval = per_case(b, b_path, "retrieval.per_case")?;
    let a_similarity = per_case(a, a_path, "similarity.per_case")?;
    let b_similarity = per_case(b, b_path, "similarity.per_case")?;

    let abstention = match (unanswerable_cases(a, a_path)?, unanswerable_cases(b, b_path)?) {
        (Some(a_cases), Some(b_cases)) => Some(diff_abstention(&a_cases, &b_cases)),
        _ => None,
    };
    let agent_view = match (agent_view_cases(a, a_path)?, agent_view_cases(b, b_path)?) {
        (Some(a_cases), Some(b_cases)) => Some(diff_agent_view(&a_cases, &b_cases)),
        _ => None,
    };

    Ok(RunDiff {
        retrieval: diff_block(&a_retrieval, &b_retrieval),
        similarity: diff_block(&a_similarity, &b_similarity),
        freshness: diff_freshness(&a_retrieval, &b_retrieval),
        agent_view,
        abstention,
    })
}

fn diff_freshness(a: &[CaseResult], b: &[CaseResult]) -> Option<MetricDiff> {
    let b_by_query: HashMap<&str, &CaseResult> = b.iter().map(|c| (c.query.as_str(), c)).collect();
    let mut m = MetricDiff::default();
    let mut paired = 0;
    for a_case in a {
        let Some(b_case) = b_by_query.get(a_case.query.as_str()) else { continue };
        if let (Some(a_fresh), Some(b_fresh)) = (a_case.fresh_first, b_case.fresh_first) {
            accumulate(&mut m, flag(a_fresh), flag(b_fresh));
            paired += 1;
        }
    }
    (paired > 0).then_some(m)
}

/// `value.retrieval.unanswerable`, or `None` when the set had no such case.
fn unanswerable_cases(value: &Value, path: &Path) -> Result<Option<Vec<CaseResult>>, DiffError> {
    cases_at(value, path, "retrieval.unanswerable")
}

fn flag(f: bool) -> f64 {
    if f {
        1.0
    } else {
        0.0
    }
}

fn diff_abstention(a: &[CaseResult], b: &[CaseResult]) -> MetricDiff {
    let b_by_query: HashMap<&str, &CaseResult> = b.iter().map(|c| (c.query.as_str(), c)).collect();
    let mut m = MetricDiff::default();
    for a_case in a {
        let Some(b_case) = b_by_query.get(a_case.query.as_str()) else { continue };
        accumulate(&mut m, flag(a_case.abstained == Some(true)), flag(b_case.abstained == Some(true)));
    }
    m
}

/// Joins the active cases of two agent-view runs on `query`. `known_red`
/// cases are left out for the same reason the summaries leave them out: they
/// expect rows the search structurally cannot return.
fn diff_agent_view(a: &[CaseView], b: &[CaseView]) -> AgentViewDiff {
    let b_by_query: HashMap<&str, &CaseView> =
        b.iter().filter(|c| c.status == CaseStatus::Active).map(|c| (c.query.as_str(), c)).collect();
    let mut diff = AgentViewDiff::default();
    let mut search_tokens = Vec::new();
    let mut context_tokens = Vec::new();
    for a_case in a.iter().filter(|c| c.status == CaseStatus::Active) {
        let Some(b_case) = b_by_query.get(a_case.query.as_str()) else { continue };
        if a_case.abstained.is_some() || b_case.abstained.is_some() {
            let abstained = |c: &CaseView| flag(c.abstained == Some(true));
            accumulate(&mut diff.search_abstained, abstained(a_case), abstained(b_case));
        } else {
            accumulate(&mut diff.search_found, flag(a_case.search.found), flag(b_case.search.found));
            accumulate(&mut diff.context_found, flag(a_case.context.found), flag(b_case.context.found));
        }
        let (a_s, b_s) = (a_case.search.tokens as f64, b_case.search.tokens as f64);
        let (a_c, b_c) = (a_case.context.tokens as f64, b_case.context.tokens as f64);
        accumulate(&mut diff.search_tokens, -a_s, -b_s);
        accumulate(&mut diff.context_tokens, -a_c, -b_c);
        search_tokens.push(b_s - a_s);
        context_tokens.push(b_c - a_c);
    }
    diff.search_tokens.ci = Some(bootstrap(&search_tokens));
    diff.context_tokens.ci = Some(bootstrap(&context_tokens));
    diff
}

/// Joins `a` and `b` on `query` and diffs each of the five metrics over the
/// matched pairs. A query present in only one side contributes to neither
/// side's counts — there is nothing to compare it against, and counting it
/// as an unqualified "better" or "worse" would score movement that was
/// really just the golden set changing shape between the two runs.
fn diff_block(a: &[CaseResult], b: &[CaseResult]) -> BlockDiff {
    let b_by_query: HashMap<&str, &CaseResult> = b.iter().map(|c| (c.query.as_str(), c)).collect();

    let mut block = BlockDiff::default();
    let mut mrr_deltas = Vec::new();
    let mut ndcg_deltas = Vec::new();
    for a_case in a {
        let Some(b_case) = b_by_query.get(a_case.query.as_str()) else { continue };
        accumulate(&mut block.recall_1, a_case.recall_1, b_case.recall_1);
        accumulate(&mut block.recall_5, a_case.recall_5, b_case.recall_5);
        accumulate(&mut block.recall_10, a_case.recall_10, b_case.recall_10);
        accumulate(&mut block.mrr, a_case.mrr, b_case.mrr);
        accumulate(&mut block.ndcg_10, a_case.ndcg_10, b_case.ndcg_10);
        mrr_deltas.push(b_case.mrr - a_case.mrr);
        ndcg_deltas.push(b_case.ndcg_10 - a_case.ndcg_10);
    }
    block.mrr.ci = Some(bootstrap(&mrr_deltas));
    block.ndcg_10.ci = Some(bootstrap(&ndcg_deltas));
    block
}

fn bootstrap(deltas: &[f64]) -> Interval {
    paired_bootstrap_ci(deltas, BOOTSTRAP_RESAMPLES, BOOTSTRAP_SEED)
}

/// `b_val` vs `a_val` for one matched pair, folded into the running
/// `better`/`worse`/`net` tally. `partial_cmp` rather than `==`/`<`/`>`
/// directly: `f64` has no total order, and a `None` (a NaN on either side,
/// which a well-formed results file never produces) counts as neither better
/// nor worse rather than panicking a comparison this tool has no stake in
/// resolving.
fn accumulate(m: &mut MetricDiff, a_val: f64, b_val: f64) {
    match b_val.partial_cmp(&a_val) {
        Some(Ordering::Greater) => m.better += 1,
        Some(Ordering::Less) => m.worse += 1,
        _ => {}
    }
    m.net = m.better as i64 - m.worse as i64;
}

/// Human-readable report printed to stdout for `--diff`. Mirrors
/// `report::print_stdout`'s posture: this is for a person reading the
/// terminal, not a machine-readable artifact — there is no JSON writer for a
/// diff, because a diff is derived entirely from two files that are already
/// on disk.
pub fn print_stdout(diff: &RunDiff) {
    println!("rag-eval diff");
    print_block("retrieval", &diff.retrieval);
    print_block("similarity", &diff.similarity);
    if let Some(av) = &diff.agent_view {
        println!("  agent_view (fewer tokens is better; the mean is second run minus first):");
        println!("    metric          better  worse     net       p");
        print_metric("search.found", &av.search_found);
        print_metric("context.found", &av.context_found);
        print_metric("search.tokens", &av.search_tokens);
        print_metric("context.tokens", &av.context_tokens);
        if av.search_abstained.better + av.search_abstained.worse > 0 {
            print_metric("search.abstain", &av.search_abstained);
        }
    }
    if let Some(m) = &diff.freshness {
        println!("  update (the newer row first):");
        print_metric("fresh_first", m);
    }
    if let Some(m) = &diff.abstention {
        println!("  unanswerable (the search returned nothing):");
        print_metric("abstained", m);
    }
    println!("  primary:");
    println!("    {}", verdict("retrieval recall@5", &diff.retrieval.recall_5));
    if let Some(av) = &diff.agent_view {
        println!("    {}", verdict("agent_view search.found", &av.search_found));
    }
}

fn print_block(name: &str, block: &BlockDiff) {
    println!("  {name}:");
    println!("    metric          better  worse     net       p");
    print_metric("recall@1", &block.recall_1);
    print_metric("recall@5", &block.recall_5);
    print_metric("recall@10", &block.recall_10);
    print_metric("mrr", &block.mrr);
    print_metric("ndcg@10", &block.ndcg_10);
}

fn print_metric(name: &str, m: &MetricDiff) {
    let ci = m
        .ci
        .map(|c| format!("  mean {:+.4} [{:+.4}, {:+.4}]", c.mean, c.lo, c.hi))
        .unwrap_or_default();
    println!("    {:<14}  {:>6}  {:>5}  {:>6}  {:>6.4}{ci}", name, m.better, m.worse, m.net, m.p_value());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A results file small enough to read at a glance: two cases in each
    /// block, the first one recalled at rank 1. Inline rather than read from
    /// `docs/eval/results/`, which git ignores, so the test runs on any machine.
    fn reference_value() -> Value {
        let case = |query: &str, expected: &str, ranked: [&str; 2], hit: f64| {
            serde_json::json!({
                "query": query, "expected_refs": [expected], "ranked_refs": ranked,
                "recall@1": hit, "recall@5": 1.0, "recall@10": 1.0, "mrr": if hit == 1.0 { 1.0 } else { 0.5 },
                "ndcg@10": if hit == 1.0 { 1.0 } else { 0.6309 }, "status": "active", "lang": "en"
            })
        };
        let cases = serde_json::json!([
            case("where is the cache", "c1", ["c1", "c2"], 1.0),
            case("which port does it use", "c2", ["c1", "c2"], 0.0),
        ]);
        serde_json::json!({
            "retrieval": { "per_case": cases.clone() },
            "similarity": { "per_case": cases },
        })
    }

    fn reference_results_path() -> std::path::PathBuf {
        std::path::PathBuf::from("reference.json")
    }

    fn assert_block_is_a_no_op(block: &BlockDiff, label: &str) {
        for (metric_name, m) in [
            ("recall@1", block.recall_1),
            ("recall@5", block.recall_5),
            ("recall@10", block.recall_10),
            ("mrr", block.mrr),
            ("ndcg@10", block.ndcg_10),
        ] {
            assert_eq!(m.better, 0, "{label}.{metric_name}.better must be 0 for a self-diff, got {m:?}");
            assert_eq!(m.worse, 0, "{label}.{metric_name}.worse must be 0 for a self-diff, got {m:?}");
            assert_eq!(m.net, 0, "{label}.{metric_name}.net must be 0 for a self-diff, got {m:?}");
            assert_eq!(m.p_value(), 1.0, "{label}.{metric_name} must have p = 1 for a self-diff");
        }
    }

    fn moved(better: usize, worse: usize) -> MetricDiff {
        MetricDiff { better, worse, net: better as i64 - worse as i64, ci: None }
    }

    #[test]
    fn six_one_way_improvements_are_significant() {
        let m = moved(6, 0);
        assert!((m.p_value() - 0.03125).abs() < 1e-9, "p = {}", m.p_value());
        let line = verdict("retrieval recall@5", &m);
        assert!(line.contains("distinguishable, better"), "{line}");
        assert!(!line.contains("not distinguishable"), "{line}");
    }

    #[test]
    fn one_improvement_is_not_distinguishable_and_says_how_many_are_needed() {
        let m = moved(1, 0);
        assert_eq!(m.p_value(), 1.0);
        let line = verdict("retrieval recall@5", &m);
        assert!(line.contains("not distinguishable with 1 case(s)"), "{line}");
        assert!(line.contains("at least 6 cases"), "{line}");
    }

    #[test]
    fn a_significant_primary_regression_fails() {
        let mut diff = RunDiff::default();
        diff.retrieval.recall_5 = moved(0, 6);
        assert!(regressed(&diff));

        let mut diff = RunDiff::default();
        diff.agent_view = Some(AgentViewDiff { search_found: moved(0, 6), ..AgentViewDiff::default() });
        assert!(regressed(&diff));
    }

    #[test]
    fn five_lost_cases_do_not_fail() {
        let mut diff = RunDiff::default();
        diff.retrieval.recall_5 = moved(0, 5);
        assert!(!regressed(&diff));
        diff.retrieval.recall_5 = moved(6, 0);
        assert!(!regressed(&diff), "an improvement never fails");
    }

    #[test]
    fn an_informative_regression_never_fails() {
        let mut diff = RunDiff::default();
        diff.retrieval.mrr = moved(0, 20);
        diff.retrieval.recall_1 = moved(0, 20);
        diff.similarity.recall_5 = moved(0, 20);
        diff.agent_view = Some(AgentViewDiff { context_found: moved(0, 20), ..AgentViewDiff::default() });
        assert!(!regressed(&diff));
    }

    #[test]
    fn the_verdict_never_says_there_is_no_difference() {
        for (b, w) in [(0, 0), (1, 0), (3, 3), (0, 6), (9, 1)] {
            let line = verdict("m", &moved(b, w)).to_lowercase();
            for phrase in ["no difference", "same", "equal", "unchanged"] {
                assert!(!line.contains(phrase), "{line}");
            }
        }
    }

    #[test]
    fn a_continuous_metric_carries_a_bootstrap_interval() {
        let a = reference_value();
        let mut b = a.clone();
        b["retrieval"]["per_case"][1]["mrr"] = serde_json::json!(1.0);
        let path = reference_results_path();

        let diff = diff_values(&a, &path, &b, &path).expect("diffing must succeed");

        let ci = diff.retrieval.mrr.ci.expect("mrr carries an interval");
        assert!((ci.mean - 0.25).abs() < 1e-9, "{ci:?}");
        assert!(ci.lo <= ci.mean && ci.mean <= ci.hi, "{ci:?}");
        assert_eq!(diff.retrieval.recall_5.ci, None, "a recall carries no interval");
    }

    fn view(query: &str, status: &str, search_found: bool, search_tokens: usize, context_found: bool) -> Value {
        serde_json::json!({
            "query": query, "expected_refs": ["c1"], "status": status, "ambiguous_refs": [],
            "search": { "found": search_found, "found_refs": [], "rank": null, "tokens": search_tokens,
                        "latency_ms": 1.0, "truncated": false },
            "context": { "found": context_found, "found_refs": [], "tokens": 300, "latency_ms": 1.0 },
        })
    }

    fn agent_view_file(cases: Vec<Value>) -> Value {
        serde_json::json!({ "retrieval": { "per_case": [] }, "agent_view": { "per_case": cases } })
    }

    #[test]
    fn a_found_flip_in_agent_view_shows_up() {
        let a = agent_view_file(vec![view("q1", "active", false, 100, true), view("q2", "active", true, 100, true)]);
        let b = agent_view_file(vec![view("q1", "active", true, 100, true), view("q2", "active", true, 100, true)]);
        let path = reference_results_path();

        let diff = diff_values(&a, &path, &b, &path).expect("diffing must succeed");

        let av = diff.agent_view.expect("both files measured the agent view");
        assert_eq!((av.search_found.better, av.search_found.worse), (1, 0));
        assert_eq!((av.context_found.better, av.context_found.worse), (0, 0));
        assert_block_is_a_no_op(&diff.retrieval, "retrieval");
    }

    #[test]
    fn fewer_tokens_count_as_better() {
        let a = agent_view_file(vec![view("q1", "active", true, 100, true)]);
        let b = agent_view_file(vec![view("q1", "active", true, 80, true)]);
        let path = reference_results_path();

        let av = diff_values(&a, &path, &b, &path).expect("diffing must succeed").agent_view.expect("block");

        assert_eq!((av.search_tokens.better, av.search_tokens.worse), (1, 0));
        let ci = av.search_tokens.ci.expect("tokens carry an interval");
        assert_eq!(ci.mean, -20.0, "the interval is the raw change: {ci:?}");
    }

    #[test]
    fn agent_view_joins_on_query_and_skips_known_red() {
        let a = agent_view_file(vec![view("q1", "known_red", false, 100, false), view("only a", "active", false, 1, false)]);
        let b = agent_view_file(vec![view("q1", "known_red", true, 100, true), view("only b", "active", true, 1, true)]);
        let path = reference_results_path();

        let av = diff_values(&a, &path, &b, &path).expect("diffing must succeed").agent_view.expect("block");

        assert_eq!(av.search_found, MetricDiff { ci: None, ..MetricDiff::default() });
        assert_eq!((av.context_found.better, av.context_found.worse), (0, 0));
    }

    #[test]
    fn a_file_without_agent_view_diffs_without_the_block() {
        let a = reference_value();
        let b = agent_view_file(vec![view("q1", "active", true, 100, true)]);
        let path = reference_results_path();

        let diff = diff_values(&a, &path, &b, &path).expect("an older file must still diff");
        assert_eq!(diff.agent_view, None);
        assert_eq!(diff_values(&a, &path, &a, &path).expect("self-diff").agent_view, None);
    }

    #[test]
    fn abstention_is_diffed_when_both_files_have_it() {
        let unanswerable = |query: &str, abstained: bool| {
            serde_json::json!({
                "query": query, "expected_refs": [], "ranked_refs": [], "recall@1": 0.0, "recall@5": 0.0,
                "recall@10": 0.0, "mrr": 0.0, "ndcg@10": 0.0, "status": "active", "lang": null,
                "subset": "unanswerable", "abstained": abstained
            })
        };
        let file = |abstained: bool| {
            serde_json::json!({ "retrieval": { "per_case": [], "unanswerable": [unanswerable("u1", abstained), unanswerable("u2", false)] } })
        };
        let path = reference_results_path();

        let diff = diff_values(&file(false), &path, &file(true), &path).expect("diffing must succeed");
        let m = diff.abstention.expect("both files have unanswerable cases");
        assert_eq!((m.better, m.worse), (1, 0));
        assert_eq!(diff_values(&reference_value(), &path, &file(true), &path).unwrap().abstention, None);

        let mut a_view = view("u1", "active", false, 50, false);
        a_view["abstained"] = serde_json::json!(false);
        let mut b_view = a_view.clone();
        b_view["abstained"] = serde_json::json!(true);
        let av = diff_values(&agent_view_file(vec![a_view]), &path, &agent_view_file(vec![b_view]), &path)
            .unwrap()
            .agent_view
            .unwrap();
        assert_eq!((av.search_abstained.better, av.search_abstained.worse), (1, 0));
        assert_eq!(av.search_found.better + av.search_found.worse, 0, "an unanswerable case has no found to move");
    }

    #[test]
    fn freshness_is_diffed_when_both_files_have_update_cases() {
        let a = reference_value();
        let mut b = a.clone();
        let path = reference_results_path();
        assert_eq!(diff_values(&a, &path, &b, &path).unwrap().freshness, None);

        let mut a = a;
        a["retrieval"]["per_case"][0]["fresh_first"] = serde_json::json!(false);
        b["retrieval"]["per_case"][0]["fresh_first"] = serde_json::json!(true);
        let m = diff_values(&a, &path, &b, &path).unwrap().freshness.expect("both scored the update case");
        assert_eq!((m.better, m.worse), (1, 0));
    }

    #[test]
    fn self_diff_is_a_no_op() {
        let a = reference_value();
        let b = a.clone();
        let path = reference_results_path();

        let diff = diff_values(&a, &path, &b, &path).expect("diffing a well-formed file must succeed");

        assert_block_is_a_no_op(&diff.retrieval, "retrieval");
        assert_block_is_a_no_op(&diff.similarity, "similarity");
    }

    #[test]
    fn diff_isolates_similarity_from_ranked_movement() {
        let a = reference_value();
        let mut b = a.clone();
        let path = reference_results_path();

        // Only `retrieval.per_case[0]` moves — a self-matching query that
        // recalled at rank 1 now does not. `similarity` is computed straight
        // from raw cosine distances, never from the scored/ranked pipeline,
        // so it must stay byte-identical here even though `retrieval` moved.
        // That is what makes a diff between two result files trustworthy as
        // a control check: if `similarity` ever moved too, it would mean the
        // underlying embeddings or corpus changed, not just the ranking
        // logic being compared.
        assert_eq!(
            a["retrieval"]["per_case"][0]["recall@1"], 1.0,
            "test fixture assumption: the first case must start at a perfect recall@1"
        );
        b["retrieval"]["per_case"][0]["recall@1"] = serde_json::json!(0.0);

        let diff = diff_values(&a, &path, &b, &path).expect("diffing must succeed");

        assert_eq!(
            diff.retrieval.recall_1,
            MetricDiff { better: 0, worse: 1, net: -1, ci: None },
            "the one mutated case must show up as a retrieval regression"
        );
        assert_block_is_a_no_op(&diff.similarity, "similarity");
    }

    #[test]
    fn a_query_present_in_only_one_side_is_not_counted_either_way() {
        let a_json = serde_json::json!({
            "retrieval": { "per_case": [
                { "query": "only in a", "expected_refs": ["c1"], "ranked_refs": ["c1"],
                  "recall@1": 1.0, "recall@5": 1.0, "recall@10": 1.0, "mrr": 1.0, "ndcg@10": 1.0,
                  "status": "active", "lang": null }
            ]},
        });
        let b_json = serde_json::json!({
            "retrieval": { "per_case": [
                { "query": "only in b", "expected_refs": ["c2"], "ranked_refs": ["c2"],
                  "recall@1": 0.0, "recall@5": 0.0, "recall@10": 0.0, "mrr": 0.0, "ndcg@10": 0.0,
                  "status": "active", "lang": null }
            ]},
        });
        let path = std::path::PathBuf::from("unused");

        let diff = diff_values(&a_json, &path, &b_json, &path).expect("diffing must succeed");
        assert_block_is_a_no_op(&diff.retrieval, "retrieval");
    }

    #[test]
    fn missing_similarity_block_degrades_to_an_empty_no_op_diff() {
        // A `--fake`-mode results file has no `similarity` key at all — this
        // must not be an error, matching every other aggregator's empty-slice
        // rule in this harness.
        let a_json = serde_json::json!({ "retrieval": { "per_case": [] } });
        let b_json = a_json.clone();
        let path = std::path::PathBuf::from("unused");

        let diff = diff_values(&a_json, &path, &b_json, &path).expect("a missing block must not error");
        assert_block_is_a_no_op(&diff.similarity, "similarity");
    }

    #[test]
    fn diff_files_reads_both_paths_and_produces_the_same_result_as_diff_values() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("reference.json");
        std::fs::write(&path, reference_value().to_string()).expect("write the fixture");
        let diff = diff_files(&path, &path).expect("diffing the reference file against itself must succeed");
        assert_block_is_a_no_op(&diff.retrieval, "retrieval");
        assert_block_is_a_no_op(&diff.similarity, "similarity");
    }

    #[test]
    fn diff_files_reports_which_path_failed_to_read() {
        let missing = std::path::PathBuf::from("this-file-does-not-exist-anywhere.json");
        let err = diff_files(&missing, &reference_results_path()).expect_err("a missing file must be an error");
        match err {
            DiffError::Read { path: p, .. } => assert_eq!(p, missing),
            other => panic!("expected DiffError::Read, got {other:?}"),
        }
    }
}
