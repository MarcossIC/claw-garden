//! Pure retrieval-quality metrics for the evaluation binary. No async, no
//! database, no embedder: every function here takes an already-ranked list of
//! ids and a set of relevant ids and returns a plain `f64`. Ids are `String`
//! refs rather than a generic type: by the time a search result reaches this
//! module it has already been translated out of the database's autoincrement
//! id space back into the golden set's stable `ref` strings, so there is
//! exactly one id representation to reason about here, and a generic
//! parameter would only let a caller accidentally mix ref-space and
//! id-space without a compile error to catch it.
//!
//! None of these functions resolve or reason about score ties: they consume
//! whatever order they are handed. How equal scores are ordered is a property
//! of the system under test (the store's stable sort), not of the scorer —
//! see the `tie_break_is_pass_through` test below for what that means in
//! practice.

use std::collections::HashSet;

use serde::Serialize;

/// Share of `expected` found within the first `k` ranked results.
///
/// Deliberately NOT hit-or-miss: a case with three expected refs that
/// retrieves one scores `1/3`, so single- and multi-relevant cases sit on the
/// same `0.0..=1.0` scale. `k = 0` and `k` larger than `ranked.len()` both
/// fall out of `Iterator::take`, which is a no-op past the end and empty at
/// zero, so neither needs special-casing. An empty `expected` returns `0.0`
/// rather than dividing by zero; the golden-set schema requires
/// `expected_refs` to be non-empty so this never fires in practice, but a
/// pure function must still be total.
pub fn recall_at_k(ranked: &[String], expected: &HashSet<String>, k: usize) -> f64 {
    if expected.is_empty() {
        return 0.0;
    }
    let hits = ranked.iter().take(k).filter(|r| expected.contains(*r)).count();
    hits as f64 / expected.len() as f64
}

/// The newer row of an `update` case came first: it is in the ranking, and no
/// stale row sits above it. A stale row that was not returned at all cannot
/// outrank it.
pub fn fresh_first(ranked: &[String], expected: &str, stale: &[String]) -> bool {
    let Some(at) = ranked.iter().position(|r| r == expected) else { return false };
    !ranked[..at].iter().any(|r| stale.contains(r))
}

/// Reciprocal of the 1-indexed rank of the first relevant hit, `0.0` if none
/// of `ranked` is in `expected`. The rank runs over the WHOLE list, never
/// capped at a `k` — capping would turn a correct-but-late hit into a miss,
/// which is a different failure mode than not finding it at all.
pub fn mrr_contribution(ranked: &[String], expected: &HashSet<String>) -> f64 {
    ranked
        .iter()
        .position(|r| expected.contains(r))
        .map(|i| 1.0 / (i as f64 + 1.0))
        .unwrap_or(0.0)
}

/// Binary-gain nDCG at a caller-chosen cutoff `k`. Gain is `1` for a relevant
/// ref and `0` otherwise — the golden set carries no partial-relevance
/// grading. The discount is `log2(rank + 1)` over the 1-indexed rank, so the
/// top position (`rank = 1`) is undiscounted (`log2(2) = 1`). Both the gain
/// shape and the discount base are written out because implementations
/// silently disagree here: swap the base or drop the `+ 1` and every score
/// changes without becoming implausible.
///
/// The ideal ranking places every relevant ref first, so `IDCG` depends only
/// on how many refs are expected, capped at `k`. `IDCG == 0` exactly when
/// `expected` is empty or `k == 0`; both return `0.0` rather than dividing by
/// zero.
pub fn ndcg_at_k(ranked: &[String], expected: &HashSet<String>, k: usize) -> f64 {
    let dcg: f64 = ranked
        .iter()
        .take(k)
        .enumerate()
        .filter(|(_, r)| expected.contains(*r))
        .map(|(i, _)| 1.0 / (i as f64 + 2.0).log2())
        .sum();
    let idcg: f64 = (0..expected.len().min(k))
        .map(|i| 1.0 / (i as f64 + 2.0).log2())
        .sum();
    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}

/// Pinned name for the cutoff production actually truncates results to
/// (`k_final = 10`): a thin wrapper so a result-JSON field can be named
/// without its reader needing to know the cutoff value. `ndcg_at_k`
/// underneath is what the edge cases (`k = 0`, `k` past the list, ties) are
/// tested against directly.
pub fn ndcg_at_10(ranked: &[String], expected: &HashSet<String>) -> f64 {
    ndcg_at_k(ranked, expected, 10)
}

/// The seven L2 thresholds the contradiction sweep reports at, around the
/// production detector's `CONTRADICTION_THRESHOLD` (0.42), to show whether
/// that choice sits on a plateau or a cliff.
pub const SWEEP: [f64; 7] = [0.20, 0.25, 0.30, 0.35, 0.40, 0.45, 0.50];

/// One row of the contradiction threshold sweep. `Serialize` is derived here
/// (added when the results-JSON writer was wired up) rather than re-derived
/// on a report-layer copy, so the JSON field names can never drift from the
/// values the sweep actually computed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ThresholdCurveRow {
    pub threshold: f64,
    pub recall: f64,
    pub fpr_no_contradiction: f64,
    pub fpr_identical: f64,
}

/// Sweeps `SWEEP` over three pre-bucketed L2-distance slices, one per pair
/// label, rather than over a `(label, distance)` pair plus a label enum: the
/// label enum belongs to the golden-set loader, a sibling module this one has
/// no dependency on today, and a sweep function has no business acquiring one
/// just to describe three buckets of floats. Each row reports, per bucket,
/// the fraction of distances at or under the threshold: `recall` over pairs
/// labeled `contradiction` (want them caught), `fpr_no_contradiction` and
/// `fpr_identical` over the other two labels (a hit there is a false
/// positive). `fpr_identical` is kept apart from `fpr_no_contradiction`
/// because an identical pair flagged "contradictory" at a low threshold is
/// evidence about the embedding or the label, not the same class of miss as
/// two merely-different, compatible statements.
///
/// Uses `distance <= threshold`. The live production detector
/// (`contradiction.rs:40`) uses strict `distance < CONTRADICTION_THRESHOLD`;
/// the two disagree only on an exact floating-point tie, which is why the
/// choice is written down rather than left implicit — a threshold calibrated
/// off this curve and copied into production must account for the boundary
/// being inclusive here and exclusive there.
///
/// An empty bucket yields `0.0` for its column instead of `NaN`. The golden
/// set is expected to carry at least one pair per label, but the function
/// stays total either way.
pub fn threshold_curve(
    contradiction_distances: &[f64],
    no_contradiction_distances: &[f64],
    identical_distances: &[f64],
) -> Vec<ThresholdCurveRow> {
    let ratio_at_or_under = |distances: &[f64], t: f64| -> f64 {
        if distances.is_empty() {
            return 0.0;
        }
        let hits = distances.iter().filter(|d| **d <= t).count();
        hits as f64 / distances.len() as f64
    };

    SWEEP
        .iter()
        .map(|&t| ThresholdCurveRow {
            threshold: t,
            recall: ratio_at_or_under(contradiction_distances, t),
            fpr_no_contradiction: ratio_at_or_under(no_contradiction_distances, t),
            fpr_identical: ratio_at_or_under(identical_distances, t),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Absolute tolerance for `f64` comparisons below. The values under test
    /// are sums of a handful of exact halves/thirds and `1/log2(n)` terms;
    /// native `f64` rounding error at that scale sits around `1e-15`, so
    /// `1e-9` rejects a formula that is actually wrong (off by a discount
    /// term, a missing `+1`, a swapped numerator) while comfortably tolerating
    /// legitimate floating-point noise.
    const EPSILON: f64 = 1e-9;

    fn approx_eq(a: f64, b: f64) {
        assert!(
            (a - b).abs() < EPSILON,
            "expected {b} within {EPSILON}, got {a} (diff {})",
            (a - b).abs()
        );
    }

    fn refs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // --- recall_at_k -------------------------------------------------------

    #[test]
    fn recall_at_k_partial_pool() {
        // R = [c3, c1, c5, c2, c4], E = {c1, c2}.
        // top_2(R) = [c3, c1] -> intersection with E = {c1} -> 1 / |E| = 1/2 = 0.5
        let ranked = refs(&["c3", "c1", "c5", "c2", "c4"]);
        let expected = set(&["c1", "c2"]);
        approx_eq(recall_at_k(&ranked, &expected, 2), 0.5);
    }

    #[test]
    fn recall_at_k_full_pool_is_perfect() {
        // Same R, E as above but k = 5 covers the whole list:
        // top_5(R) = [c3, c1, c5, c2, c4] -> intersection = {c1, c2} -> 2/2 = 1.0
        let ranked = refs(&["c3", "c1", "c5", "c2", "c4"]);
        let expected = set(&["c1", "c2"]);
        approx_eq(recall_at_k(&ranked, &expected, 5), 1.0);
    }

    #[test]
    fn recall_at_k_perfect_ranking() {
        // R = [c1, c2], E = {c1, c2}, k = 2 -> both found -> 2/2 = 1.0
        let ranked = refs(&["c1", "c2"]);
        let expected = set(&["c1", "c2"]);
        approx_eq(recall_at_k(&ranked, &expected, 2), 1.0);
    }

    #[test]
    fn recall_at_k_worst_case_ranking() {
        // Relevant ref is last of three. top_2([a, b, c1]) = [a, b] -> 0 hits -> 0/1 = 0.0
        // top_3([a, b, c1]) = [a, b, c1] -> 1 hit -> 1/1 = 1.0
        let ranked = refs(&["a", "b", "c1"]);
        let expected = set(&["c1"]);
        approx_eq(recall_at_k(&ranked, &expected, 2), 0.0);
        approx_eq(recall_at_k(&ranked, &expected, 3), 1.0);
    }

    #[test]
    fn recall_at_k_partial_ranking_multi_relevant() {
        // R = [c1, x, y], E = {c1, c2}. c2 never appears anywhere in R.
        // top_3(R) = [c1, x, y] -> intersection {c1} -> 1/2 = 0.5
        let ranked = refs(&["c1", "x", "y"]);
        let expected = set(&["c1", "c2"]);
        approx_eq(recall_at_k(&ranked, &expected, 3), 0.5);
    }

    #[test]
    fn recall_at_k_k_larger_than_ranked_list() {
        // ranked has 2 entries; k = 10 asks for more than exist.
        // take(10) yields both entries -> hits = 1 (c1) -> 1/1 = 1.0
        let ranked = refs(&["c1", "x"]);
        let expected = set(&["c1"]);
        approx_eq(recall_at_k(&ranked, &expected, 10), 1.0);
    }

    #[test]
    fn recall_at_k_empty_relevant_set_is_zero() {
        // expected = {} -> defined as 0.0 (division-by-zero guard), never hit
        // by real golden-set data but must still be a total function.
        let ranked = refs(&["c1", "c2"]);
        let expected: HashSet<String> = HashSet::new();
        approx_eq(recall_at_k(&ranked, &expected, 2), 0.0);
    }

    #[test]
    fn recall_at_k_empty_ranked_list_is_zero() {
        // ranked = [] -> take(k) yields nothing -> 0 hits -> 0/1 = 0.0
        let ranked: Vec<String> = Vec::new();
        let expected = set(&["c1"]);
        approx_eq(recall_at_k(&ranked, &expected, 5), 0.0);
    }

    #[test]
    fn recall_at_k_zero_k_is_zero() {
        // k = 0 -> take(0) yields nothing regardless of ranked's contents -> 0/1 = 0.0
        let ranked = refs(&["c1"]);
        let expected = set(&["c1"]);
        approx_eq(recall_at_k(&ranked, &expected, 0), 0.0);
    }

    #[test]
    fn recall_at_k_no_relevant_hit_is_zero_for_every_k() {
        let ranked = refs(&["a", "b", "c"]);
        let expected = set(&["x"]);
        approx_eq(recall_at_k(&ranked, &expected, 1), 0.0);
        approx_eq(recall_at_k(&ranked, &expected, 3), 0.0);
    }

    // --- mrr_contribution ----------------------------------------------------

    #[test]
    fn mrr_contribution_hand_computed() {
        // R = [c3, c1, c5, c2, c4], E = {c1, c2}. First relevant ref is "c1"
        // at 1-indexed rank 2 -> 1/2 = 0.5
        let ranked = refs(&["c3", "c1", "c5", "c2", "c4"]);
        let expected = set(&["c1", "c2"]);
        approx_eq(mrr_contribution(&ranked, &expected), 0.5);
    }

    #[test]
    fn mrr_contribution_perfect_ranking() {
        // First entry is relevant -> rank 1 -> 1/1 = 1.0
        let ranked = refs(&["c1", "c2"]);
        let expected = set(&["c1", "c2"]);
        approx_eq(mrr_contribution(&ranked, &expected), 1.0);
    }

    #[test]
    fn mrr_contribution_worst_case_ranking() {
        // Relevant ref is last of three -> rank 3 -> 1/3
        let ranked = refs(&["a", "b", "c1"]);
        let expected = set(&["c1"]);
        approx_eq(mrr_contribution(&ranked, &expected), 1.0 / 3.0);
    }

    #[test]
    fn mrr_contribution_no_hit_is_zero() {
        let ranked = refs(&["a", "b", "c"]);
        let expected = set(&["x"]);
        approx_eq(mrr_contribution(&ranked, &expected), 0.0);
    }

    #[test]
    fn mrr_contribution_empty_ranked_list_is_zero() {
        let ranked: Vec<String> = Vec::new();
        let expected = set(&["c1"]);
        approx_eq(mrr_contribution(&ranked, &expected), 0.0);
    }

    #[test]
    fn mrr_contribution_empty_relevant_set_is_zero() {
        let ranked = refs(&["c1", "c2"]);
        let expected: HashSet<String> = HashSet::new();
        approx_eq(mrr_contribution(&ranked, &expected), 0.0);
    }

    // --- ndcg_at_k / ndcg_at_10 ------------------------------------------

    #[test]
    fn ndcg_at_10_hand_computed_rank_two_hit() {
        // R = [c3, c1], E = {c1}.
        // DCG = gain(c3)/log2(2) + gain(c1)/log2(3) = 0/1 + 1/log2(3) ~= 0.6309297535714575
        // IDCG = 1/log2(2) = 1.0 (ideal places the single relevant ref first)
        // nDCG = DCG / IDCG ~= 0.6309297535714575
        let ranked = refs(&["c3", "c1"]);
        let expected = set(&["c1"]);
        approx_eq(ndcg_at_10(&ranked, &expected), 0.6309297535714575);
        // Same case shows nDCG != recall@10: recall@10 sees c1 within the
        // full list and scores 1.0, while nDCG penalizes it for not being
        // first. This is the "normalization is genuinely exercised" case.
        approx_eq(recall_at_k(&ranked, &expected, 10), 1.0);
    }

    #[test]
    fn ndcg_at_k_perfect_ranking_equals_one() {
        // R = [c1, c2], E = {c1, c2}: both relevant refs occupy the first two
        // (i.e. the only two) ranks, matching the ideal ranking exactly.
        // DCG  = 1/log2(2) + 1/log2(3) = 1.0 + 0.6309297535714575 = 1.6309297535714575
        // IDCG = same sum (ideal places both relevant refs first) = 1.6309297535714575
        // nDCG = 1.6309297535714575 / 1.6309297535714575 = 1.0
        let ranked = refs(&["c1", "c2"]);
        let expected = set(&["c1", "c2"]);
        approx_eq(ndcg_at_k(&ranked, &expected, 10), 1.0);
    }

    #[test]
    fn ndcg_at_k_worst_case_ranking() {
        // R = [a, b, c1], E = {c1}. Relevant ref at rank 3.
        // DCG  = 1/log2(4) = 1/2 = 0.5
        // IDCG = 1/log2(2) = 1.0
        // nDCG = 0.5
        let ranked = refs(&["a", "b", "c1"]);
        let expected = set(&["c1"]);
        approx_eq(ndcg_at_k(&ranked, &expected, 10), 0.5);
    }

    #[test]
    fn ndcg_at_k_partial_ranking_multi_relevant() {
        // R = [c1, x, y], E = {c1, c2}. Only c1 is present, at rank 1; c2
        // never appears.
        // DCG  = 1/log2(2) = 1.0
        // IDCG = 1/log2(2) + 1/log2(3) = 1.6309297535714575 (2 relevant refs expected)
        // nDCG = 1.0 / 1.6309297535714575 = 0.6131471927654585
        let ranked = refs(&["c1", "x", "y"]);
        let expected = set(&["c1", "c2"]);
        approx_eq(ndcg_at_k(&ranked, &expected, 10), 0.6131471927654585);
    }

    #[test]
    fn ndcg_at_k_no_relevant_hit_is_zero() {
        let ranked = refs(&["a", "b", "c"]);
        let expected = set(&["x"]);
        approx_eq(ndcg_at_k(&ranked, &expected, 10), 0.0);
    }

    #[test]
    fn ndcg_at_k_empty_relevant_set_is_zero() {
        // expected = {} -> IDCG's range is 0..min(0, k) = empty -> IDCG = 0.0
        // -> defined as 0.0 rather than a division by zero.
        let ranked = refs(&["c1", "c2"]);
        let expected: HashSet<String> = HashSet::new();
        approx_eq(ndcg_at_k(&ranked, &expected, 10), 0.0);
    }

    #[test]
    fn ndcg_at_k_empty_ranked_list_is_zero() {
        // ranked = [] -> DCG's iterator is empty -> DCG = 0.0; IDCG is still
        // 1.0 (one relevant ref expected) -> nDCG = 0.0 / 1.0 = 0.0
        let ranked: Vec<String> = Vec::new();
        let expected = set(&["c1"]);
        approx_eq(ndcg_at_k(&ranked, &expected, 10), 0.0);
    }

    #[test]
    fn ndcg_at_k_zero_k_is_zero() {
        // k = 0 -> both DCG's take(0) and IDCG's 0..min(1, 0) ranges are
        // empty -> DCG = 0.0, IDCG = 0.0 -> the IDCG == 0.0 guard returns 0.0
        // rather than computing 0.0 / 0.0 (NaN).
        let ranked = refs(&["c1"]);
        let expected = set(&["c1"]);
        approx_eq(ndcg_at_k(&ranked, &expected, 0), 0.0);
    }

    // --- tie handling (pass-through, not resolved here) --------------------

    #[test]
    fn tie_break_is_pass_through_not_resolved_here() {
        // Two equally-relevant-looking orderings of the same two refs, as if
        // upstream had resolved a genuine score tie between "c1" and "c2" two
        // different ways. Only "c1" is relevant. metrics.rs does not
        // re-derive or second-guess which order is "right" -- it trusts
        // whatever order it is handed, so the two orderings must score
        // differently:
        //   order A = [c1, c2]: c1 at rank 1 -> recall@1 = 1.0, mrr = 1.0
        //   order B = [c2, c1]: c1 at rank 2 -> recall@1 = 0.0, mrr = 1/2 = 0.5
        let expected = set(&["c1"]);

        let order_a = refs(&["c1", "c2"]);
        approx_eq(recall_at_k(&order_a, &expected, 1), 1.0);
        approx_eq(mrr_contribution(&order_a, &expected), 1.0);

        let order_b = refs(&["c2", "c1"]);
        approx_eq(recall_at_k(&order_b, &expected, 1), 0.0);
        approx_eq(mrr_contribution(&order_b, &expected), 0.5);
    }

    // --- threshold_curve -----------------------------------------------------

    #[test]
    fn threshold_curve_hand_picked_buckets() {
        // Three hand-picked distance buckets, one per label.
        //
        // contradiction  = [0.05, 0.18, 0.30]  (n = 3)
        // no_contradiction = [0.25, 0.45]      (n = 2)
        // identical      = [0.02]              (n = 1)
        //
        // Sweep uses `distance <= threshold`. Per threshold t, each column is
        // (count of bucket values <= t) / (bucket size):
        //
        // t=0.20: contradiction {0.05, 0.18}          -> 2/3
        //         no_contradiction {}                -> 0.0
        //         identical {0.02}                    -> 1.0
        // t=0.25: contradiction {0.05, 0.18}          -> 2/3
        //         no_contradiction {0.25}             -> 1/2 = 0.5
        //         identical {0.02}                    -> 1.0
        // t=0.30: contradiction {0.05, 0.18, 0.30}    -> 3/3 = 1.0
        //         no_contradiction {0.25}             -> 1/2 = 0.5
        //         identical {0.02}                    -> 1.0
        // t=0.35: contradiction all <= t              -> 1.0
        //         no_contradiction {0.25}             -> 0.5
        //         identical {0.02}                    -> 1.0
        // t=0.40: contradiction all <= t              -> 1.0
        //         no_contradiction {0.25}             -> 0.5 (0.48 still over)
        //         identical {0.02}                    -> 1.0
        // t=0.45: same as 0.40
        // t=0.50: contradiction all <= t              -> 1.0
        //         no_contradiction {0.25, 0.48}       -> 1.0
        //         identical {0.02}                    -> 1.0
        let contradiction = vec![0.05, 0.18, 0.30];
        let no_contradiction = vec![0.25, 0.48];
        let identical = vec![0.02];

        let rows = threshold_curve(&contradiction, &no_contradiction, &identical);
        assert_eq!(rows.len(), 7, "one row per SWEEP threshold");

        let expected_rows = [
            (0.20, 2.0 / 3.0, 0.0, 1.0),
            (0.25, 2.0 / 3.0, 0.5, 1.0),
            (0.30, 1.0, 0.5, 1.0),
            (0.35, 1.0, 0.5, 1.0),
            (0.40, 1.0, 0.5, 1.0),
            (0.45, 1.0, 0.5, 1.0),
            (0.50, 1.0, 1.0, 1.0),
        ];

        for (row, (t, recall, fpr_no_c, fpr_identical)) in rows.iter().zip(expected_rows) {
            approx_eq(row.threshold, t);
            approx_eq(row.recall, recall);
            approx_eq(row.fpr_no_contradiction, fpr_no_c);
            approx_eq(row.fpr_identical, fpr_identical);
        }
    }

    #[test]
    fn threshold_curve_empty_buckets_are_zero_not_nan() {
        let rows = threshold_curve(&[], &[], &[]);
        assert_eq!(rows.len(), 7);
        for row in rows {
            approx_eq(row.recall, 0.0);
            approx_eq(row.fpr_no_contradiction, 0.0);
            approx_eq(row.fpr_identical, 0.0);
        }
    }

    #[test]
    fn threshold_curve_uses_sweep_constants_in_order() {
        let rows = threshold_curve(&[], &[], &[]);
        let thresholds: Vec<f64> = rows.iter().map(|r| r.threshold).collect();
        assert_eq!(thresholds, SWEEP.to_vec());
    }

    #[test]
    fn fresh_first_needs_the_expected_row_before_every_stale_one() {
        let r = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let stale = r(&["old"]);
        assert!(fresh_first(&r(&["x", "new", "y", "old"]), "new", &stale));
        assert!(!fresh_first(&r(&["old", "x", "new"]), "new", &stale));
        assert!(fresh_first(&r(&["new"]), "new", &stale), "a stale row not returned cannot outrank");
        assert!(!fresh_first(&r(&["old", "x"]), "new", &stale), "the newer row has to be there");
    }
}
