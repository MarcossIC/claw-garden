//! The log lines of a search: pure formatting, testable without a store.
//! `search.rs` and `prompt_builder.rs` log what these return, so the side
//! effect stays at the call site.

use crate::memory::search::RankedHit;

/// Counts hits by tier and renders `"t1:{n1},t2:{n2},t3:{n3}"`.
///
/// All three counts can be non-zero: the KNN pool scans every tier, so `t1`
/// reports how much of the hottest tier the query actually pulled in.
pub(crate) fn format_tier_distribution(hits: &[RankedHit]) -> String {
    let (mut t1, mut t2, mut t3) = (0u32, 0u32, 0u32);
    for h in hits {
        match h.tier {
            1 => t1 += 1,
            2 => t2 += 1,
            3 => t3 += 1,
            _ => {}
        }
    }
    format!("t1:{t1},t2:{t2},t3:{t3}")
}

/// Renders whether tier-1 workspace content was present alongside the
/// retrieval hit count — the real measure of the "Tier 1 covers 80%"
/// criterion (neither signal alone can answer it).
pub(crate) fn format_tier1_coverage(workspace_present: bool, hit_count: usize) -> String {
    format!("tier1_present={workspace_present} tier23_hits={hit_count}")
}

/// Renders the pool size the KNN arm was asked for against the one it
/// returned.
///
/// A returned count consistently below the requested one means rows are
/// being dropped after the vector scan already committed to its candidates —
/// the failure this pair exists to expose. Neither number alone can show it:
/// a low `returned` count is equally consistent with "the corpus genuinely
/// has fewer matching rows" as with "another agent's rows starved the pool",
/// so this only makes the symptom visible, not the diagnosis.
///
/// `requested` is always the pool size the caller asked for, never the wider
/// window a starved pool was re-queried with — otherwise the denominator
/// moves under the ratio and a rescued query reads as the worst one in the
/// log. `retried` carries that fact separately, so a line with no marker
/// means the pool needed none.
pub(crate) fn format_pool_survival(requested: usize, returned: usize, retried: bool) -> String {
    let base = format!("pool_req={requested} pool_ret={returned}");
    if retried {
        format!("{base} retry")
    } else {
        base
    }
}

/// Renders how many hits came from the dense (KNN) channel
/// versus how many the lexical (FTS5) channel found that the dense channel
/// missed. `lexical_only` is exact — the size of the post-dedup leftover set
/// — so it proves the lexical channel FIRED, never that what it found
/// survived the final `k_final` truncation; only a measured eval delta can
/// show that.
pub(crate) fn format_channel_attribution(dense: usize, lexical_only: usize) -> String {
    format!("chan=(dense={dense},lex_only={lexical_only})")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::clock::TimestampMs;

    // ── format_tier_distribution ─────────────────────────

    fn make_hit(tier: i64) -> RankedHit {
        RankedHit {
            id: 1,
            content: String::new(),
            memory_type: "chat_message".to_string(),
            tier,
            status: "active".to_string(),
            score: 0.0,
            distance: 0.0,
            created_at: TimestampMs::from_unix_millis(0),
            agent_id: None,
            piece: None,
            exact: false,
            topic_key: None,
            project_id: None,
            origin: None,
        }
    }

    #[test]
    fn format_tier_distribution_counts_by_tier() {
        let hits = vec![make_hit(2), make_hit(2), make_hit(3)];
        assert_eq!(format_tier_distribution(&hits), "t1:0,t2:2,t3:1");
    }

    #[test]
    fn format_tier_distribution_empty_is_all_zero() {
        assert_eq!(format_tier_distribution(&[]), "t1:0,t2:0,t3:0");
    }

    // ── format_tier1_coverage ────────────────────────────

    #[test]
    fn format_tier1_coverage_present_true_with_hits() {
        assert_eq!(
            format_tier1_coverage(true, 3),
            "tier1_present=true tier23_hits=3"
        );
    }

    #[test]
    fn format_tier1_coverage_absent_false_no_hits() {
        assert_eq!(
            format_tier1_coverage(false, 0),
            "tier1_present=false tier23_hits=0"
        );
    }

    // ── format_pool_survival ──────────────────────────────────────────────

    #[test]
    fn format_pool_survival_renders_requested_and_returned() {
        assert_eq!(format_pool_survival(30, 28, false), "pool_req=30 pool_ret=28");
    }

    // A returned count below the requested one reads as "the corpus is thin"
    // until you know whether a wider window was already tried. The marker is
    // what separates a starved pool that recovered from one that was never
    // crowded to begin with. The requested side is the pool size the caller
    // asked for, never the widened window a retry ran with: reporting the
    // widened number would make the survival ratio look catastrophic exactly
    // on the queries the retry just rescued.
    #[test]
    fn format_pool_survival_marks_a_retry_against_the_pool_asked_for() {
        assert_eq!(format_pool_survival(30, 5, true), "pool_req=30 pool_ret=5 retry");
    }

    // ── format_channel_attribution ──────────────────────────────────────────

    #[test]
    fn format_channel_attribution_renders_dense_and_lexical_only_counts() {
        assert_eq!(format_channel_attribution(3, 2), "chan=(dense=3,lex_only=2)");
    }

    #[test]
    fn format_channel_attribution_renders_zero_lexical_only() {
        assert_eq!(format_channel_attribution(30, 0), "chan=(dense=30,lex_only=0)");
    }
}
