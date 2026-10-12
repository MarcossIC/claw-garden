//! The relative relevance cut shared by the daemon's search and the MCP
//! bridge's rendering of it.
//!
//! It lives outside `clawg-core` on purpose: the store's ranking, and every
//! metric `rag-eval` reads from it, stays the full ranked list. The cut only
//! decides how much of that list a caller is handed.

/// Share of the best hit's score another hit needs to be shown.
///
/// Relative, not absolute: measured on the golden set, the score of the
/// expected memory and of the rest overlap (0.15–0.51 against up to 0.47), so
/// no fixed floor separates them, while a clear winner can still shorten the
/// list for free.
pub const RELEVANCE_RATIO: f64 = 0.6;

/// Hits shown regardless of score.
pub const MIN_SHOWN_HITS: usize = 2;

/// How many of `scores`, best first, survive a cut at `ratio` of the best one.
pub fn relevant_len(scores: &[f64], ratio: f64) -> usize {
    let Some(&best) = scores.first() else {
        return 0;
    };
    scores
        .iter()
        .enumerate()
        .take_while(|&(i, &s)| i < MIN_SHOWN_HITS || s >= ratio * best)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_far_below_the_best_match_are_not_shown() {
        assert_eq!(relevant_len(&[0.50, 0.40, 0.31, 0.29, 0.10], RELEVANCE_RATIO), 3);
    }

    #[test]
    fn a_hit_exactly_at_the_ratio_is_shown() {
        assert_eq!(relevant_len(&[0.50, 0.40, 0.30], RELEVANCE_RATIO), 3);
    }

    /// When every score is low the best one is no clear winner, and the runner
    /// up is as likely to be the answer: measured on the golden set, a query
    /// whose answer scored 0.04 under a best of 0.10.
    #[test]
    fn the_two_best_are_shown_whatever_they_score() {
        assert_eq!(relevant_len(&[0.10, 0.04, 0.01], RELEVANCE_RATIO), 2);
    }

    #[test]
    fn no_scores_keep_nothing() {
        assert_eq!(relevant_len(&[], RELEVANCE_RATIO), 0);
    }
}
