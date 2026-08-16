//! Pure cosine-based measurement over already-embedded vectors — no store, no
//! vec0, no decay, no importance. Exists so the retrieval quality a run's
//! ranked search reports can be checked against the plain geometry of the
//! embedding space it ran in: if the two disagree by a lot, the gap is
//! telling you something about the store's scoring path, not the model.

use std::cmp::Ordering;

use agent_office_lib::eval_api::cosine;

/// Ranks every `(ref, vector)` pair in `corpus` by cosine similarity to
/// `query`, descending, truncated to `k`. Ties are left in `corpus`'s own
/// order: `sort_by` is stable, so two refs with identical similarity keep
/// whichever order they arrived in rather than being reshuffled by the sort.
pub fn rank_by_cosine(query: &[f32], corpus: &[(String, Vec<f32>)], k: usize) -> Vec<String> {
    let mut scored: Vec<(&str, f64)> =
        corpus.iter().map(|(r, v)| (r.as_str(), cosine(query, v))).collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
    scored.into_iter().take(k).map(|(r, _)| r.to_string()).collect()
}

/// One case's data for the query-anchored gap, already resolved to vectors —
/// this function has no notion of a ref, only of vectors.
pub struct ExpectedGapCase<'a> {
    pub query: &'a [f32],
    /// The case's expected refs' vectors.
    pub expected: Vec<&'a [f32]>,
    /// Every OTHER corpus vector — the whole seeded corpus minus the
    /// expected refs, not just this case's declared pool: production search
    /// runs against the whole store, so restricting rivals to the case's own
    /// pool would score an easier problem than the one being measured.
    pub rivals: Vec<&'a [f32]>,
}

/// The query-anchored gap for one case: its weakest expected match's cosine
/// to the query, minus its strongest rival's. Negative means the top rival
/// beats a required row. `min` over expected on purpose — recall needs EVERY
/// expected ref ranked above the rivals, so a case's real margin is set by
/// its weakest required row, not its strongest.
fn expected_gap(case: &ExpectedGapCase) -> f64 {
    let min_expected =
        case.expected.iter().map(|e| cosine(case.query, e)).fold(f64::INFINITY, f64::min);
    let max_rival =
        case.rivals.iter().map(|r| cosine(case.query, r)).fold(f64::NEG_INFINITY, f64::max);
    min_expected - max_rival
}

/// Mean of [`expected_gap`] across `cases`. `0.0` for an empty slice rather
/// than a division by zero — this can only happen when a run has zero active
/// cases, which is a golden-set problem this function is not the one to flag.
pub fn mean_expected_distractor_gap(cases: &[ExpectedGapCase]) -> f64 {
    if cases.is_empty() {
        return 0.0;
    }
    cases.iter().map(expected_gap).sum::<f64>() / cases.len() as f64
}

/// One case's data for the doc-anchored gap: its expected refs' vectors
/// paired with the vectors of the distractors they must be pulled apart
/// from — which are the case's OWN declared pool minus its expected refs,
/// matching the mined golden set's distractor structure exactly (unlike
/// [`ExpectedGapCase::rivals`], this is deliberately scoped to the case, not
/// the whole corpus).
pub struct CorpusGapCase<'a> {
    pub expected: Vec<&'a [f32]>,
    pub distractors: Vec<&'a [f32]>,
}

/// The doc-anchored gap for one case: one minus its strongest cosine against
/// a distractor, per expected ref, averaged over the case's OWN expected
/// set — a case with three expected refs counts once in the overall mean,
/// not three times, so it cannot outweigh a case with a single expected ref.
/// Same `cosine`, same `1 - max` shape as the golden-set miner's own
/// distractor-gap statistic per expected ref, so a result here is comparable
/// to the value recorded when the set was mined — a query-anchored gap (see
/// [`expected_gap`]) is not, because it measures a different pair of things.
fn corpus_gap(case: &CorpusGapCase) -> f64 {
    if case.expected.is_empty() {
        return 0.0;
    }
    let per_expected: f64 = case
        .expected
        .iter()
        .map(|e| {
            let max_distractor =
                case.distractors.iter().map(|d| cosine(e, d)).fold(f64::NEG_INFINITY, f64::max);
            if max_distractor.is_finite() {
                1.0 - max_distractor
            } else {
                1.0
            }
        })
        .sum();
    per_expected / case.expected.len() as f64
}

/// Mean of [`corpus_gap`] across `cases`, one case one weight regardless of
/// how many expected refs it declares.
pub fn mean_corpus_distractor_gap(cases: &[CorpusGapCase]) -> f64 {
    if cases.is_empty() {
        return 0.0;
    }
    cases.iter().map(corpus_gap).sum::<f64>() / cases.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- rank_by_cosine ----------------------------------------------------

    #[test]
    fn rank_by_cosine_orders_descending_ties_break_toward_corpus_order() {
        let corpus = vec![
            ("a".to_string(), vec![1.0, 0.0]),
            ("b".to_string(), vec![0.9, 0.1]),  // closest to the query
            ("c".to_string(), vec![0.0, 1.0]),  // farthest
            ("d".to_string(), vec![0.9, 0.1]),  // identical similarity to "b" — a genuine tie
        ];
        let query = [1.0, 0.0];
        // "b" and "d" are an exact tie; "b" precedes "d" in `corpus`, so a
        // stable sort must keep that relative order.
        assert_eq!(rank_by_cosine(&query, &corpus, 10), vec!["a", "b", "d", "c"]);
    }

    #[test]
    fn rank_by_cosine_truncates_to_k() {
        let corpus = vec![
            ("a".to_string(), vec![1.0, 0.0]),
            ("b".to_string(), vec![0.9, 0.1]),
            ("c".to_string(), vec![0.0, 1.0]),
        ];
        let query = [1.0, 0.0];
        assert_eq!(rank_by_cosine(&query, &corpus, 2), vec!["a", "b"]);
    }

    // --- mean_expected_distractor_gap --------------------------------------

    #[test]
    fn expected_gap_uses_the_weakest_expected_match_not_the_strongest() {
        let query = [1.0, 0.0];
        // Two expected vectors: one a near-perfect match (cos ~1.0), one a
        // weak match (cos ~0.6). One rival sits in between (cos ~0.8).
        // A correct `min`-over-expected implementation reports a NEGATIVE
        // gap (0.6 - 0.8), because the weak expected row sits below the
        // rival. A bug that used `max` instead would report a positive gap
        // (1.0 - 0.8) and call the case comfortable when it is not.
        let strong_expected = [1.0, 0.0];
        let weak_expected = [0.6, 0.8];
        let rival = [0.8, 0.6];
        let case = ExpectedGapCase {
            query: &query,
            expected: vec![&strong_expected, &weak_expected],
            rivals: vec![&rival],
        };
        let gap = mean_expected_distractor_gap(std::slice::from_ref(&case));
        let expected_value = cosine(&query, &weak_expected) - cosine(&query, &rival);
        assert!(expected_value < 0.0, "test setup must produce a negative gap, got {expected_value}");
        assert!((gap - expected_value).abs() < 1e-9, "gap was {gap}, expected {expected_value}");
    }

    #[test]
    fn mean_expected_distractor_gap_of_no_cases_is_zero_not_nan() {
        assert_eq!(mean_expected_distractor_gap(&[]), 0.0);
    }

    // --- mean_corpus_distractor_gap -----------------------------------------

    #[test]
    fn corpus_gap_is_one_minus_the_strongest_distractor_cosine() {
        let expected = [1.0, 0.0];
        let near_distractor = [0.99, 0.01]; // cos ~0.99978 — the strongest of the two
        let far_distractor = [0.0, 1.0];    // cos = 0.0
        let case =
            CorpusGapCase { expected: vec![&expected], distractors: vec![&near_distractor, &far_distractor] };
        let gap = mean_corpus_distractor_gap(std::slice::from_ref(&case));
        let expected_value = 1.0 - cosine(&expected, &near_distractor);
        assert!((gap - expected_value).abs() < 1e-9, "gap was {gap}, expected {expected_value}");
        assert!(gap < 0.01, "the near-duplicate distractor must dominate the max, got gap={gap}");
    }

    #[test]
    fn mean_corpus_distractor_gap_of_no_cases_is_zero_not_nan() {
        assert_eq!(mean_corpus_distractor_gap(&[]), 0.0);
    }

    #[test]
    fn a_multi_expected_case_averages_over_its_own_expected_refs_before_averaging_across_cases() {
        // Case A has two expected refs against one shared distractor pool.
        // Case B has a single expected ref against a DIFFERENT pool. If the
        // implementation flattened every expected ref into one global list
        // instead of averaging within a case first, case A would silently
        // count twice as much as case B in the final mean.
        let a_expected_1 = [1.0, 0.0]; // strong: distractor cos ~0.1 -> gap ~0.9
        let a_expected_2 = [0.1, 0.995]; // weak: distractor cos ~0.995 -> gap ~0.005
        let a_distractor = [0.1, 0.995];
        let case_a =
            CorpusGapCase { expected: vec![&a_expected_1, &a_expected_2], distractors: vec![&a_distractor] };

        let b_expected = [0.0, 1.0];
        let b_distractor = [1.0, 0.0]; // orthogonal -> gap = 1.0
        let case_b = CorpusGapCase { expected: vec![&b_expected], distractors: vec![&b_distractor] };

        let gap = mean_corpus_distractor_gap(&[case_a, case_b]);

        let a_gap_1 = 1.0 - cosine(&a_expected_1, &a_distractor);
        let a_gap_2 = 1.0 - cosine(&a_expected_2, &a_distractor);
        let case_a_mean = (a_gap_1 + a_gap_2) / 2.0;
        let case_b_mean = 1.0 - cosine(&b_expected, &b_distractor);
        let expected_value = (case_a_mean + case_b_mean) / 2.0;

        assert!(
            (gap - expected_value).abs() < 1e-9,
            "gap was {gap}, expected {expected_value} (equal per-case weight, not per-expected-ref weight)"
        );
    }
}
