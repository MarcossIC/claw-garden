//! The pure geometry behind golden-set construction: cosine similarity,
//! nearest-neighbour lookup, and the two selection passes that turn a large
//! pool of candidate documents into a small corpus that actually discriminates.
//!
//! Nothing here loads a model or touches a file. Every function is a
//! deterministic transform over vectors already produced elsewhere, which is
//! what makes the selection strategy testable without paying for a 470 MB
//! model — and testable is the point, because this module decides how hard the
//! evaluation set is, and a set that is accidentally easy reports perfect
//! scores while measuring nothing.

// `cosine` lives in `eval_api` now, not here: binaries cannot share modules
// with each other, only with the lib, and the function that produced this
// module's mined distractor gaps and the function `rag-eval`'s similarity
// block ranks against must be one definition or a comparison between the two
// runs is a comparison between two unverified implementations.
use clawg_core::eval_api::cosine;

/// A candidate document paired with its embedding. Index into the slice is the
/// document's identity throughout this module: refs and ids are the caller's
/// concern, and threading them through here would only add a translation layer
/// with nothing to translate.
#[derive(Debug, Clone)]
pub struct Embedded {
    pub memory_type: String,
    pub lang: String,
    pub vector: Vec<f32>,
}

/// Indices of the `k` documents most similar to `target`, nearest first,
/// excluding `target` itself and anything in `exclude`.
///
/// Ties break toward the lower index so that a given pool always yields the
/// same neighbours: the whole harness rests on being able to re-run a golden
/// set and get the same numbers, and a selection pass that shuffled on ties
/// would break that before the first metric is ever computed.
pub fn nearest(
    pool: &[Embedded],
    target: usize,
    k: usize,
    exclude: &[usize],
    same_type_only: bool,
) -> Vec<usize> {
    let mut scored: Vec<(usize, f64)> = pool
        .iter()
        .enumerate()
        .filter(|(i, e)| {
            *i != target
                && !exclude.contains(i)
                && (!same_type_only || e.memory_type == pool[target].memory_type)
        })
        .map(|(i, e)| (i, cosine(&pool[target].vector, &e.vector)))
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
    scored.into_iter().take(k).map(|(i, _)| i).collect()
}

/// How many documents of each `memory_type` the corpus should contain, and how
/// many language-balanced ones among them.
#[derive(Debug, Clone)]
pub struct Quota {
    pub memory_type: String,
    pub count: usize,
}

/// Picks a corpus by growing dense clusters rather than sampling broadly.
///
/// The naive approach — take N documents spread across the pool — produces a
/// corpus where every query has one obviously-correct answer and everything
/// else is unrelated filler. That corpus reports recall@10 = 1.0 forever and
/// cannot detect a ranking regression, which is exactly the state the existing
/// 13-document golden set is stuck in.
///
/// So each cluster is a seed document plus its nearest same-type neighbours:
/// documents that a good embedder genuinely struggles to separate. The
/// resulting corpus is adversarial by construction, before a single query has
/// been written.
///
/// Seeds are spaced out deliberately (a seed may not be a neighbour of an
/// already-chosen cluster) so the corpus covers several distinct topics
/// instead of collapsing into one tight blob, which would make every query
/// ambiguous rather than merely hard.
pub fn select_corpus(pool: &[Embedded], quotas: &[Quota], cluster_size: usize) -> Vec<usize> {
    let mut chosen: Vec<usize> = Vec::new();

    for quota in quotas {
        let mut taken_for_type = 0usize;
        let type_members: Vec<usize> = pool
            .iter()
            .enumerate()
            .filter(|(_, e)| e.memory_type == quota.memory_type)
            .map(|(i, _)| i)
            .collect();

        // Seeds are visited in an order that chases whichever language is
        // currently behind. The candidate pool runs about 72% Spanish, and
        // taking it in natural order would yield a nearly monolingual corpus —
        // which cannot exercise the one property `multilingual-e5-small` was
        // chosen for. Ties still resolve by index, so the pass stays
        // deterministic.
        let mut remaining = type_members;
        let mut es_taken = 0usize;
        let mut en_taken = 0usize;

        // Two passes. The first spaces seeds apart so the corpus spans several
        // topics; the second refills whatever the spacing rule starved. Without
        // the refill a quota can silently come up short — and a `memory_type`
        // that vanishes from the corpus is a whole category the golden set
        // stops measuring, reported as nothing at all.
        let mut deferred: Vec<usize> = Vec::new();
        let mut enforce_spacing = true;

        loop {
            if remaining.is_empty() {
                if !enforce_spacing || deferred.is_empty() || taken_for_type >= quota.count {
                    break;
                }
                remaining = std::mem::take(&mut deferred);
                enforce_spacing = false;
            }
            if taken_for_type >= quota.count {
                break;
            }

            let wanted = if es_taken <= en_taken { "es" } else { "en" };
            let position = remaining
                .iter()
                .position(|&i| pool[i].lang == wanted)
                .unwrap_or(0);
            let seed = remaining.remove(position);

            if chosen.contains(&seed) {
                continue;
            }
            // A seed adjacent to an already-built cluster of the SAME type
            // would deepen that topic instead of opening a new one. Comparing
            // across types instead would be wrong twice over: the quotas are
            // per-type, and with the real model every document sits in a high
            // similarity band, so a cross-type test rejects seeds for
            // resembling documents they are not competing with.
            if enforce_spacing
                && chosen
                    .iter()
                    .filter(|&&c| pool[c].memory_type == quota.memory_type)
                    .any(|&c| cosine(&pool[c].vector, &pool[seed].vector) > 0.92)
            {
                deferred.push(seed);
                continue;
            }

            chosen.push(seed);
            taken_for_type += 1;
            if pool[seed].lang == "es" {
                es_taken += 1;
            } else {
                en_taken += 1;
            }

            let room = quota.count.saturating_sub(taken_for_type);
            let want = cluster_size.saturating_sub(1).min(room);
            for n in nearest(pool, seed, want, &chosen, true) {
                if pool[n].lang == "es" {
                    es_taken += 1;
                } else {
                    en_taken += 1;
                }
                remaining.retain(|&r| r != n);
                chosen.push(n);
                taken_for_type += 1;
            }
        }
    }

    chosen
}

/// One retrieval case's document set: the expected answer plus the hard
/// negatives it must be ranked above.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinedCase {
    pub expected: usize,
    pub distractors: Vec<usize>,
}

/// Builds one case per corpus document, whose distractors are that document's
/// nearest same-type neighbours *within the corpus*.
///
/// Restricting distractors to the same `memory_type` is what makes them hard:
/// a `user_preference` competing against another `user_preference` about a
/// different topic is a real discrimination test, while the same preference
/// competing against a stack trace is answered by the type signal alone.
///
/// A document with fewer than `min_distractors` same-type neighbours available
/// yields no case at all rather than an easy one. Dropping it is honest; a case
/// with one weak distractor would score near-perfectly and pull the aggregate
/// up while measuring nothing.
pub fn mine_cases(corpus: &[Embedded], distractors: usize, min_distractors: usize) -> Vec<MinedCase> {
    (0..corpus.len())
        .filter_map(|expected| {
            let found = nearest(corpus, expected, distractors, &[], true);
            (found.len() >= min_distractors).then_some(MinedCase { expected, distractors: found })
        })
        .collect()
}

/// Mean cosine gap between each case's expected document and its strongest
/// distractor. Near zero means the corpus is maximally confusable; large means
/// the cases are easy and the set will saturate.
///
/// Reported so that the difficulty of a generated set is a number in the run
/// log rather than a claim in a commit message.
pub fn mean_distractor_gap(corpus: &[Embedded], cases: &[MinedCase]) -> f64 {
    if cases.is_empty() {
        return 0.0;
    }
    let total: f64 = cases
        .iter()
        .map(|c| {
            let best = c
                .distractors
                .iter()
                .map(|&d| cosine(&corpus[c.expected].vector, &corpus[d].vector))
                .fold(f64::NEG_INFINITY, f64::max);
            if best.is_finite() {
                1.0 - best
            } else {
                1.0
            }
        })
        .sum();
    total / cases.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(memory_type: &str, lang: &str, v: &[f32]) -> Embedded {
        Embedded {
            memory_type: memory_type.to_string(),
            lang: lang.to_string(),
            vector: v.to_vec(),
        }
    }

    // --- cosine ---------------------------------------------------------

    #[test]
    fn cosine_of_identical_vectors_is_one() {
        assert!((cosine(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn cosine_of_orthogonal_vectors_is_zero() {
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-12);
    }

    #[test]
    fn cosine_of_opposed_vectors_is_minus_one() {
        assert!((cosine(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-12);
    }

    #[test]
    fn cosine_ignores_magnitude() {
        // The whole point of cosine: a vector and its scaled copy are the same
        // direction. If this ever failed, un-normalized candidates would score
        // by length instead of meaning.
        assert!((cosine(&[1.0, 1.0], &[7.0, 7.0]) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn cosine_of_a_zero_vector_is_zero_not_nan() {
        // A NaN here would propagate into every ranking downstream and produce
        // an arbitrary sort order rather than an error anyone could see.
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn cosine_of_mismatched_lengths_is_zero_not_a_panic() {
        assert_eq!(cosine(&[1.0, 2.0], &[1.0]), 0.0);
    }

    // --- nearest --------------------------------------------------------

    #[test]
    fn nearest_returns_closest_first_and_excludes_the_target() {
        let pool = vec![
            doc("a", "en", &[1.0, 0.0]),
            doc("a", "en", &[0.9, 0.1]),  // closest to 0
            doc("a", "en", &[0.0, 1.0]),  // farthest from 0
            doc("a", "en", &[0.7, 0.3]),  // middle
        ];
        assert_eq!(nearest(&pool, 0, 3, &[], false), vec![1, 3, 2]);
    }

    #[test]
    fn nearest_honours_the_exclusion_list() {
        let pool = vec![
            doc("a", "en", &[1.0, 0.0]),
            doc("a", "en", &[0.9, 0.1]),
            doc("a", "en", &[0.7, 0.3]),
        ];
        assert_eq!(nearest(&pool, 0, 2, &[1], false), vec![2]);
    }

    #[test]
    fn nearest_can_restrict_to_the_targets_own_memory_type() {
        let pool = vec![
            doc("pref", "en", &[1.0, 0.0]),
            doc("error", "en", &[0.99, 0.01]), // nearer, but wrong type
            doc("pref", "en", &[0.6, 0.4]),
        ];
        assert_eq!(nearest(&pool, 0, 5, &[], true), vec![2]);
    }

    #[test]
    fn nearest_breaks_ties_toward_the_lower_index_for_determinism() {
        let pool = vec![
            doc("a", "en", &[1.0, 0.0]),
            doc("a", "en", &[0.5, 0.5]),
            doc("a", "en", &[0.5, 0.5]), // identical to index 1
        ];
        assert_eq!(nearest(&pool, 0, 1, &[], false), vec![1]);
    }

    #[test]
    fn nearest_returns_fewer_than_k_when_the_pool_is_small() {
        let pool = vec![doc("a", "en", &[1.0, 0.0]), doc("a", "en", &[0.5, 0.5])];
        assert_eq!(nearest(&pool, 0, 10, &[], false).len(), 1);
    }

    // --- select_corpus ---------------------------------------------------

    #[test]
    fn select_corpus_respects_the_per_type_quota() {
        let pool: Vec<Embedded> = (0..20)
            .map(|i| {
                let t = if i < 10 { "pref" } else { "error" };
                doc(t, "en", &[i as f32, 1.0])
            })
            .collect();
        let quotas = vec![
            Quota { memory_type: "pref".into(), count: 4 },
            Quota { memory_type: "error".into(), count: 3 },
        ];
        let chosen = select_corpus(&pool, &quotas, 2);
        assert_eq!(chosen.iter().filter(|&&i| pool[i].memory_type == "pref").count(), 4);
        assert_eq!(chosen.iter().filter(|&&i| pool[i].memory_type == "error").count(), 3);
    }

    #[test]
    fn select_corpus_never_repeats_a_document() {
        let pool: Vec<Embedded> = (0..12).map(|i| doc("a", "en", &[i as f32, 1.0])).collect();
        let chosen = select_corpus(&pool, &[Quota { memory_type: "a".into(), count: 8 }], 3);
        let mut sorted = chosen.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), chosen.len(), "a document was selected twice");
    }

    #[test]
    fn select_corpus_pulls_neighbours_in_with_each_seed() {
        // Two tight groups far apart. A cluster-growing pass must take both
        // members of a group together; a broad sample would take one from each.
        let pool = vec![
            doc("a", "en", &[1.0, 0.0]),
            doc("a", "en", &[0.99, 0.01]), // twin of 0
            doc("a", "en", &[0.0, 1.0]),
            doc("a", "en", &[0.01, 0.99]), // twin of 2
        ];
        let chosen = select_corpus(&pool, &[Quota { memory_type: "a".into(), count: 2 }], 2);
        assert!(
            chosen.contains(&0) && chosen.contains(&1),
            "cluster growth must keep a seed and its nearest neighbour together, got {chosen:?}"
        );
    }

    #[test]
    fn select_corpus_stops_at_the_quota_even_when_more_candidates_exist() {
        let pool: Vec<Embedded> = (0..50).map(|i| doc("a", "en", &[i as f32, 1.0])).collect();
        let chosen = select_corpus(&pool, &[Quota { memory_type: "a".into(), count: 5 }], 3);
        assert_eq!(chosen.len(), 5);
    }

    #[test]
    fn one_types_cluster_cannot_starve_another_types_quota() {
        // Every document here points nearly the same way, which is what the
        // real model produces (its similarities live in a high, narrow band).
        // If seed spacing were tested across types, the `error` quota would
        // come back empty because its candidates resemble already-chosen
        // `pref` ones — losing a whole category with no error reported.
        let pool: Vec<Embedded> = (0..20)
            .map(|i| doc(if i < 10 { "pref" } else { "error" }, "en", &[i as f32, 1.0]))
            .collect();
        let quotas = vec![
            Quota { memory_type: "pref".into(), count: 4 },
            Quota { memory_type: "error".into(), count: 3 },
        ];
        let chosen = select_corpus(&pool, &quotas, 2);
        assert_eq!(
            chosen.iter().filter(|&&i| pool[i].memory_type == "error").count(),
            3,
            "the error quota was starved by the pref clusters"
        );
    }

    #[test]
    fn a_quota_is_filled_even_when_every_candidate_is_tightly_packed() {
        // All ten documents are near-duplicates, so seed spacing rejects all
        // but the first. The refill pass must still deliver the quota rather
        // than a corpus of one.
        let pool: Vec<Embedded> = (0..10)
            .map(|i| doc("a", "en", &[1.0, i as f32 * 0.001]))
            .collect();
        let chosen = select_corpus(&pool, &[Quota { memory_type: "a".into(), count: 5 }], 1);
        assert_eq!(chosen.len(), 5);
    }

    #[test]
    fn select_corpus_balances_language_against_a_lopsided_pool() {
        // 12 Spanish documents ahead of 4 English ones, all mutually distant so
        // clustering never forces a pick. Taking the pool in natural order
        // would produce an all-Spanish corpus and leave the multilingual
        // behaviour of the real model untested.
        let mut pool: Vec<Embedded> = (0..12)
            .map(|i| doc("a", "es", &[i as f32, 0.0, 1.0]))
            .collect();
        pool.extend((0..4).map(|i| doc("a", "en", &[0.0, i as f32 + 1.0, 1.0])));

        let chosen = select_corpus(&pool, &[Quota { memory_type: "a".into(), count: 6 }], 1);
        let en = chosen.iter().filter(|&&i| pool[i].lang == "en").count();
        assert!(en >= 2, "expected the pass to reach for English seeds, got {en} of 6");
    }

    #[test]
    fn select_corpus_ignores_types_absent_from_the_pool() {
        let pool = vec![doc("a", "en", &[1.0, 0.0])];
        let chosen = select_corpus(&pool, &[Quota { memory_type: "ghost".into(), count: 3 }], 2);
        assert!(chosen.is_empty());
    }

    // --- mine_cases -------------------------------------------------------

    #[test]
    fn mine_cases_gives_each_document_its_nearest_same_type_rivals() {
        let corpus = vec![
            doc("a", "en", &[1.0, 0.0]),
            doc("a", "en", &[0.9, 0.1]),
            doc("a", "en", &[0.8, 0.2]),
        ];
        let cases = mine_cases(&corpus, 2, 1);
        assert_eq!(cases.len(), 3);
        assert_eq!(cases[0], MinedCase { expected: 0, distractors: vec![1, 2] });
    }

    #[test]
    fn mine_cases_drops_a_document_without_enough_distractors() {
        // The lone `solo` document has no same-type rival, so it cannot form an
        // honest case — and an easy case would inflate the aggregate.
        let corpus = vec![
            doc("a", "en", &[1.0, 0.0]),
            doc("a", "en", &[0.9, 0.1]),
            doc("solo", "en", &[0.0, 1.0]),
        ];
        let cases = mine_cases(&corpus, 3, 1);
        assert_eq!(cases.len(), 2);
        assert!(cases.iter().all(|c| c.expected != 2));
    }

    #[test]
    fn mine_cases_never_lists_the_expected_document_as_its_own_distractor() {
        let corpus: Vec<Embedded> = (0..6).map(|i| doc("a", "en", &[i as f32, 1.0])).collect();
        for case in mine_cases(&corpus, 3, 1) {
            assert!(!case.distractors.contains(&case.expected));
        }
    }

    // --- mean_distractor_gap ------------------------------------------------

    #[test]
    fn gap_is_near_zero_when_distractors_are_near_duplicates() {
        let corpus = vec![doc("a", "en", &[1.0, 0.0]), doc("a", "en", &[1.0, 0.001])];
        let cases = mine_cases(&corpus, 1, 1);
        assert!(mean_distractor_gap(&corpus, &cases) < 0.01);
    }

    #[test]
    fn gap_is_large_when_distractors_are_unrelated() {
        let corpus = vec![doc("a", "en", &[1.0, 0.0]), doc("a", "en", &[0.0, 1.0])];
        let cases = mine_cases(&corpus, 1, 1);
        assert!(mean_distractor_gap(&corpus, &cases) > 0.9);
    }

    #[test]
    fn gap_of_no_cases_is_zero_not_nan() {
        assert_eq!(mean_distractor_gap(&[], &[]), 0.0);
    }
}
