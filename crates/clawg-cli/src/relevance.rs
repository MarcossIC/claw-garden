//! The relative relevance cut shared by the daemon's search and the MCP
//! bridge's rendering of it.
//!
//! It lives outside `clawg-core` on purpose: the store's ranking, and every
//! metric `rag-eval` reads from it, stays the full ranked list. The cut only
//! decides how much of that list a caller is handed.

/// Share of the best hit's score another hit needs to be shown.
///
/// Relative, not absolute: measured on the golden set, the score of the
/// expected memory and of the rest overlap (0.03–0.41 against up to 0.38), so
/// no fixed floor separates them, while a clear winner can still shorten the
/// list for free.
///
/// Measured with `rag-eval --agent-view` on mean-pooled vectors, whose best
/// score has a median of 0.19: 0.35 shows the expected memory in 36 of 46
/// queries at 764 tokens on average, where 0.6 showed 30 at 455 and 0.2
/// shows 37 at 894.
pub const RELEVANCE_RATIO: f64 = 0.35;

/// Hits without an identifier match shown regardless of score.
pub const MIN_SHOWN_HITS: usize = 2;

/// Which of `hits`, best first as `(score, exact)`, survive a cut at `ratio`.
///
/// An exact hit, one that holds every identifier of the query, is always
/// shown and never sets the bar: its lexical bonus puts it a whole point above
/// any cosine, and measured against it no row about the same topic survived.
/// The others are cut against the best of them, so a query with an identifier
/// shows its matches above the list a query without one would show.
pub fn shown_hits(hits: &[(f64, bool)], ratio: f64) -> Vec<bool> {
    let best = hits.iter().find(|(_, exact)| !exact).map_or(0.0, |&(score, _)| score);
    let mut inexact_seen = 0;
    hits.iter()
        .map(|&(score, exact)| {
            if exact {
                return true;
            }
            inexact_seen += 1;
            inexact_seen <= MIN_SHOWN_HITS || score >= ratio * best
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inexact(scores: &[f64]) -> Vec<(f64, bool)> {
        scores.iter().map(|&s| (s, false)).collect()
    }

    fn kept(hits: &[(f64, bool)]) -> Vec<f64> {
        hits.iter().zip(shown_hits(hits, RELEVANCE_RATIO)).filter(|(_, k)| *k).map(|(h, _)| h.0).collect()
    }

    #[test]
    fn relevance_ratio_pins_the_shipped_value() {
        assert_eq!(RELEVANCE_RATIO, 0.35);
    }

    #[test]
    fn hits_far_below_the_best_match_are_not_shown() {
        assert_eq!(kept(&inexact(&[0.20, 0.15, 0.08, 0.06, 0.02])), [0.20, 0.15, 0.08]);
    }

    #[test]
    fn a_hit_exactly_at_the_ratio_is_shown() {
        let best = 0.40;
        assert_eq!(kept(&inexact(&[best, 0.30, RELEVANCE_RATIO * best])).len(), 3);
    }

    /// When every score is low the best one is no clear winner, and the runner
    /// up is as likely to be the answer: measured on the golden set, a query
    /// whose answer scored 0.04 under a best of 0.10.
    #[test]
    fn the_two_best_are_shown_whatever_they_score() {
        assert_eq!(kept(&inexact(&[0.10, 0.02, 0.01])), [0.10, 0.02]);
    }

    #[test]
    fn no_scores_keep_nothing() {
        assert!(shown_hits(&[], RELEVANCE_RATIO).is_empty());
    }

    /// The bonus puts an identifier match a whole point above any cosine, so
    /// measuring the others against it hid every row about the same topic.
    #[test]
    fn exact_hits_do_not_set_the_bar_for_the_others() {
        let hits = [(1.6, true), (0.30, false), (0.25, false), (0.20, false), (0.05, false)];
        assert_eq!(kept(&hits), [1.6, 0.30, 0.25, 0.20]);
    }

    #[test]
    fn an_exact_hit_is_shown_behind_one_the_cut_drops() {
        let hits = [(0.40, false), (0.30, false), (0.05, false), (0.02, true)];
        assert_eq!(kept(&hits), [0.40, 0.30, 0.02]);
    }

    #[test]
    fn the_two_best_inexact_hits_are_shown_whatever_they_score() {
        let hits = [(1.6, true), (1.5, true), (0.10, false), (0.02, false), (0.01, false)];
        assert_eq!(kept(&hits), [1.6, 1.5, 0.10, 0.02]);
    }
}
