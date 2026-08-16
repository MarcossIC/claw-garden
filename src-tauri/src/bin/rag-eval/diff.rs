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

use crate::report::CaseResult;

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

/// The whole paired diff: `retrieval` and `similarity`, computed the same
/// way but never combined — see the module doc comment for why.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RunDiff {
    pub retrieval: BlockDiff,
    pub similarity: BlockDiff,
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
    #[error("'{block}.per_case' in '{}' is not a valid case array: {source}", path.display())]
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

/// Extracts `value[block]["per_case"]` as `Vec<CaseResult>`. Absent
/// (`value[block]` missing, e.g. `similarity` under a `--fake` run) is not an
/// error — it degrades to an empty case list, the same way an empty slice
/// degrades every other aggregator in this harness (see
/// `similarity::mean_expected_distractor_gap`). A present-but-malformed
/// block IS an error: that is a results file this tool cannot honestly
/// compare, not a block that was never computed.
fn per_case(value: &Value, path: &Path, block: &'static str) -> Result<Vec<CaseResult>, DiffError> {
    match value.pointer(&format!("/{block}/per_case")) {
        None => Ok(Vec::new()),
        Some(cases) => serde_json::from_value(cases.clone())
            .map_err(|e| DiffError::Cases { path: path.to_path_buf(), block, source: e }),
    }
}

/// The pure join-and-diff: two already-parsed results values, diffed on
/// `retrieval.per_case` and `similarity.per_case` independently. `a_path`/
/// `b_path` are used only to label a [`DiffError::Cases`] with which file was
/// malformed; they carry no meaning for the comparison itself.
pub fn diff_values(a: &Value, a_path: &Path, b: &Value, b_path: &Path) -> Result<RunDiff, DiffError> {
    let a_retrieval = per_case(a, a_path, "retrieval")?;
    let b_retrieval = per_case(b, b_path, "retrieval")?;
    let a_similarity = per_case(a, a_path, "similarity")?;
    let b_similarity = per_case(b, b_path, "similarity")?;

    Ok(RunDiff {
        retrieval: diff_block(&a_retrieval, &b_retrieval),
        similarity: diff_block(&a_similarity, &b_similarity),
    })
}

/// Joins `a` and `b` on `query` and diffs each of the five metrics over the
/// matched pairs. A query present in only one side contributes to neither
/// side's counts — there is nothing to compare it against, and counting it
/// as an unqualified "better" or "worse" would score movement that was
/// really just the golden set changing shape between the two runs.
fn diff_block(a: &[CaseResult], b: &[CaseResult]) -> BlockDiff {
    let b_by_query: HashMap<&str, &CaseResult> = b.iter().map(|c| (c.query.as_str(), c)).collect();

    let mut block = BlockDiff::default();
    for a_case in a {
        let Some(b_case) = b_by_query.get(a_case.query.as_str()) else { continue };
        accumulate(&mut block.recall_1, a_case.recall_1, b_case.recall_1);
        accumulate(&mut block.recall_5, a_case.recall_5, b_case.recall_5);
        accumulate(&mut block.recall_10, a_case.recall_10, b_case.recall_10);
        accumulate(&mut block.mrr, a_case.mrr, b_case.mrr);
        accumulate(&mut block.ndcg_10, a_case.ndcg_10, b_case.ndcg_10);
    }
    block
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
}

fn print_block(name: &str, block: &BlockDiff) {
    println!("  {name}:");
    println!("    metric      better  worse     net");
    print_metric("recall@1", &block.recall_1);
    print_metric("recall@5", &block.recall_5);
    print_metric("recall@10", &block.recall_10);
    print_metric("mrr", &block.mrr);
    print_metric("ndcg@10", &block.ndcg_10);
}

fn print_metric(name: &str, m: &MetricDiff) {
    println!("    {:<10}  {:>6}  {:>5}  {:>6}", name, m.better, m.worse, m.net);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference results file this stage's tests measure against —
    /// resolved the same CWD-independent way as `cli::default_golden_path`,
    /// since `cargo test` may run from a different working directory than
    /// this crate's manifest.
    fn reference_results_path() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("docs")
            .join("eval")
            .join("results")
            .join("2026-08-16-real-symmetric.json")
    }

    fn reference_value() -> Value {
        let text = std::fs::read_to_string(reference_results_path())
            .expect("reference results file must exist");
        serde_json::from_str(&text).expect("reference results file must be valid JSON")
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
        }
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
            MetricDiff { better: 0, worse: 1, net: -1 },
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
        let path = reference_results_path();
        let diff = diff_files(&path, &path).expect("diffing the reference file against itself must succeed");
        assert_block_is_a_no_op(&diff.retrieval, "retrieval");
        assert_block_is_a_no_op(&diff.similarity, "similarity");
    }

    #[test]
    fn diff_files_reports_which_path_failed_to_read() {
        let missing = std::path::PathBuf::from("this-file-does-not-exist-anywhere.json");
        let path = reference_results_path();
        let err = diff_files(&missing, &path).expect_err("a missing file must be an error");
        match err {
            DiffError::Read { path: p, .. } => assert_eq!(p, missing),
            other => panic!("expected DiffError::Read, got {other:?}"),
        }
    }
}
