/// Tier promotion and demotion logic for the RAG access-tracker (Phase 5).
///
/// [`compute_tier`] is **promote-only** — it never returns a tier number
/// greater than `current` (lower numbers = higher tier).  Use
/// [`demote_inactive`] for inactivity-based decay; it is the *only* demoting
/// function in Phase 5.

use crate::memory::MemoryError;

// ---------------------------------------------------------------------------
// Pure-Rust functions (no SQL, fully unit-testable)
// ---------------------------------------------------------------------------

/// Computes the promoted tier for a memory entry.
///
/// Promote-only: the returned value is never numerically greater than
/// `current`.  If no promotion rule fires the function returns `current`
/// unchanged.  Use [`demote_inactive`] for decay.
pub(crate) fn compute_tier(
    current: i64,
    importance: f64,
    access_count: i64,
    memory_type: &str,
) -> i64 {
    // Rule 1: pivotal fast-track — high-importance memories always land at tier 1.
    if importance >= 9.0 {
        return current.min(1);
    }

    match memory_type {
        // Rule 2: type-based tier-1 types (initial and always).
        "user_preference" | "procedural_skill" => current.min(1),

        // Rule 3: architectural_decision / session_summary
        //   – access_count >= 3: promote to tier 1.
        //   – otherwise:         promote to tier 2 (INSERT initial-tier rule).
        //   The min() call preserves the promote-only invariant when current < target.
        //
        //   A freshly inserted row's access_count starts at 1, not 0 — its own
        //   creation counts as the first occurrence of its content (see
        //   `insert_memory_with_vector` in store_insert.rs) — and the write-border
        //   dedup gate adds one more per reinforced resend. So this rule's
        //   access_count >= 3 threshold is crossed by the SECOND reinforcement of
        //   an inserted row, not the third. Pinned in store_insert.rs so a change
        //   to either number is caught instead of silently shifting this boundary.
        "architectural_decision" | "session_summary" => {
            if access_count >= 3 {
                current.min(1)
            } else {
                current.min(2)
            }
        }

        // No rule for chat_message, episodic_trajectory, or unknown types.
        _ => current,
    }
}

/// Applies inactivity-based demotion to a memory tier.
///
/// Runs in production today: the maintenance pass wires this in through the
/// demote_tier UDF and sweeps the memory table on a timer.
///
/// Takes a day count as f64, never a timestamp — the subtraction that produces
/// it happens inside the demotion SQL, one expression for the whole table.
/// Handing this function a millisecond-typed instant instead would drag that
/// arithmetic back into Rust and turn a single bulk UPDATE into a per-row round
/// trip. The type discipline is worth a great deal; it is not worth that.
///
/// Demote-only: the returned value is never numerically less than
/// `current_tier` (inactivity can only lower the tier, not raise it).
pub(crate) fn demote_inactive(
    current_tier: i64,
    days_since_last_access: f64,
    importance: f64,
) -> i64 {
    match current_tier {
        1 => {
            // DEMOTE-2: absolutely pivotal memories (importance >= 9.0) NEVER demote.
            if importance >= 9.0 {
                1
            // DEMOTE-4: low-importance tier-1 demotes to tier 2 after 90 days.
            } else if days_since_last_access > 90.0 {
                2
            } else {
                1
            }
        }
        2 => {
            // DEMOTE-3: tier-2 demotes to tier 3 after 180 days.
            if days_since_last_access > 180.0 {
                3
            } else {
                2
            }
        }
        // DEMOTE-5: no condition met — return current unchanged.
        _ => current_tier,
    }
}

// ---------------------------------------------------------------------------
// UDF registration
// ---------------------------------------------------------------------------

/// Registers the `compute_tier` scalar UDF on the connection.
///
/// Arity 4, `SQLITE_UTF8 | SQLITE_DETERMINISTIC`.  Delegates argument
/// extraction to the closure and computation to the pure-Rust [`compute_tier`]
/// function.  Called from `MemoryStore::open()` and `MemoryStore::new_for_test()`.
pub(crate) async fn register_tier_udf(
    conn: &tokio_rusqlite::Connection,
) -> Result<(), MemoryError> {
    conn.call(|c| {
        use rusqlite::functions::FunctionFlags;
        let det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;

        c.create_scalar_function("compute_tier", 4, det, |ctx| {
            let current: i64 = ctx.get(0)?;
            let importance: f64 = ctx.get(1)?;
            let access_count: i64 = ctx.get(2)?;
            let memory_type: String = ctx.get(3)?;
            Ok(compute_tier(current, importance, access_count, &memory_type))
        })?;

        Ok(())
    })
    .await
    .map_err(|e| match e {
        tokio_rusqlite::Error::Error(rusqlite_err) => MemoryError::Sqlite(rusqlite_err),
        tokio_rusqlite::Error::ConnectionClosed => {
            MemoryError::Backend("connection closed during tier UDF registration".into())
        }
        other => MemoryError::Backend(other.to_string()),
    })
}

/// Registers the `demote_tier` scalar UDF on the connection.
///
/// Arity 3, `SQLITE_UTF8 | SQLITE_DETERMINISTIC`.  Delegates argument
/// extraction to the closure and computation to the pure-Rust
/// [`demote_inactive`] function. Mirrors [`register_tier_udf`] above.
/// Called alongside it in `MemoryStore::open()` and
/// `MemoryStore::new_for_test()` (memoria-3-capas-fase-3, maintenance job).
pub(crate) async fn register_demote_udf(
    conn: &tokio_rusqlite::Connection,
) -> Result<(), MemoryError> {
    conn.call(|c| {
        use rusqlite::functions::FunctionFlags;
        let det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;

        c.create_scalar_function("demote_tier", 3, det, |ctx| {
            let current_tier: i64 = ctx.get(0)?;
            let days_since_last_access: f64 = ctx.get(1)?;
            let importance: f64 = ctx.get(2)?;
            Ok(demote_inactive(current_tier, days_since_last_access, importance))
        })?;

        Ok(())
    })
    .await
    .map_err(|e| match e {
        tokio_rusqlite::Error::Error(rusqlite_err) => MemoryError::Sqlite(rusqlite_err),
        tokio_rusqlite::Error::ConnectionClosed => {
            MemoryError::Backend("connection closed during demote UDF registration".into())
        }
        other => MemoryError::Backend(other.to_string()),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── T-01-test: compute_tier truth table ─────────────────────────────────

    // TIER-4: INSERT tier-1 types (compute_tier(3, importance, 0, type) → 1)

    #[test]
    fn compute_tier_user_preference_inserts_at_tier_1() {
        assert_eq!(compute_tier(3, 5.0, 0, "user_preference"), 1);
    }

    #[test]
    fn compute_tier_procedural_skill_inserts_at_tier_1() {
        assert_eq!(compute_tier(3, 5.0, 0, "procedural_skill"), 1);
    }

    // TIER-5: INSERT tier-2 types (compute_tier(3, importance, 0, type) → 2)

    #[test]
    fn compute_tier_architectural_decision_inserts_at_tier_2() {
        assert_eq!(compute_tier(3, 5.0, 0, "architectural_decision"), 2);
    }

    #[test]
    fn compute_tier_session_summary_inserts_at_tier_2() {
        assert_eq!(compute_tier(3, 5.0, 0, "session_summary"), 2);
    }

    // TIER-6: INSERT tier-3 types (compute_tier(3, importance, 0, type) → 3)

    #[test]
    fn compute_tier_chat_message_inserts_at_tier_3() {
        assert_eq!(compute_tier(3, 5.0, 0, "chat_message"), 3);
    }

    #[test]
    fn compute_tier_episodic_trajectory_inserts_at_tier_3() {
        assert_eq!(compute_tier(3, 5.0, 0, "episodic_trajectory"), 3);
    }

    // TIER-2: promote-only invariant — result MUST NOT exceed current.

    #[test]
    fn compute_tier_promote_only_chat_message_at_tier_1_stays_1() {
        // No rule fires for chat_message, returns current = 1.
        assert_eq!(compute_tier(1, 1.0, 0, "chat_message"), 1);
    }

    // TIER-7: no rule fires → returns current unchanged.

    #[test]
    fn compute_tier_no_rule_fires_returns_current_unchanged() {
        // chat_message at tier 2 with access_count=1 — no rule fires.
        assert_eq!(compute_tier(2, 2.0, 1, "chat_message"), 2);
    }

    // Extra: importance >= 9.0 fast-track (from tasks.md impl description).

    #[test]
    fn compute_tier_high_importance_promotes_any_type_to_tier_1() {
        assert_eq!(compute_tier(3, 9.0, 0, "chat_message"), 1);
        assert_eq!(compute_tier(2, 9.5, 1, "session_summary"), 1);
    }

    // Extra: promote-only invariant on architectural_decision below threshold.

    #[test]
    fn compute_tier_arch_decision_below_threshold_capped_at_current_if_lower() {
        // current=1, access_count=0 → min(2, 1) = 1, never demotes.
        assert_eq!(compute_tier(1, 5.0, 0, "architectural_decision"), 1);
    }

    // ── T-01: flush re-eval rule (TIER-1 scenario, pure Rust)

    #[test]
    fn compute_tier_architectural_decision_access_count_3_promotes_to_tier_1() {
        // current=2, access_count=3 → flush re-eval fires → 1.
        assert_eq!(compute_tier(2, 7.0, 3, "architectural_decision"), 1);
    }

    #[test]
    fn compute_tier_session_summary_access_count_3_promotes_to_tier_1() {
        assert_eq!(compute_tier(2, 5.0, 3, "session_summary"), 1);
    }

    // ── T-02-test: demote_inactive truth table ──────────────────────────────

    // DEMOTE-4: tier 1 → 2 when days > 90 AND importance < 9.0.

    #[test]
    fn demote_inactive_tier1_low_importance_over_90_days_demotes_to_2() {
        assert_eq!(demote_inactive(1, 91.0, 5.0), 2);
    }

    // DEMOTE-2: importance >= 9.0 → tier 1 NEVER demotes.

    #[test]
    fn demote_inactive_tier1_high_importance_never_demotes() {
        assert_eq!(demote_inactive(1, 91.0, 9.0), 1);
        // Even with extreme days, pivotal memories stay at tier 1.
        assert_eq!(demote_inactive(1, 365.0, 9.0), 1);
    }

    // DEMOTE-3: tier 2 → 3 when days > 180.

    #[test]
    fn demote_inactive_tier2_over_180_days_demotes_to_3() {
        assert_eq!(demote_inactive(2, 181.0, 5.0), 3);
    }

    // DEMOTE-5: no condition met → returns current unchanged.

    #[test]
    fn demote_inactive_tier2_not_inactive_enough_returns_current() {
        // days = 100, tier 2 threshold is > 180 — not met.
        assert_eq!(demote_inactive(2, 100.0, 5.0), 2);
    }

    #[test]
    fn demote_inactive_tier1_under_90_days_returns_current() {
        // days = 89, under the 90-day threshold for tier 1.
        assert_eq!(demote_inactive(1, 89.0, 5.0), 1);
    }

    // ── T-03-test: register_tier_udf — SQL callable ─────────────────────────

    #[tokio::test]
    async fn register_tier_udf_makes_compute_tier_callable_via_sql() {
        // Uses new_for_test() which calls register_tier_udf internally (T-04-impl).
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let v: i64 = store
            .conn
            .call(|c| {
                c.query_row(
                    "SELECT compute_tier(2, 5.0, 3, 'architectural_decision')",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(v, 1, "expected tier 1 (flush re-eval), got {v}");
    }

    #[tokio::test]
    async fn register_tier_udf_initial_insert_user_preference_returns_1() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let v: i64 = store
            .conn
            .call(|c| {
                c.query_row(
                    "SELECT compute_tier(3, 5.0, 0, 'user_preference')",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(v, 1, "expected tier 1 (user_preference INSERT), got {v}");
    }

    // ── T-05-test: register_demote_udf — SQL callable (memoria-3-capas-fase-3) ──

    #[tokio::test]
    async fn register_demote_udf_makes_demote_tier_callable_via_sql() {
        // Uses new_for_test() which calls register_demote_udf internally.
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let v: i64 = store
            .conn
            .call(|c| {
                c.query_row("SELECT demote_tier(1, 91.0, 5.0)", [], |r| r.get(0))
            })
            .await
            .unwrap();
        assert_eq!(v, 2, "expected tier 2 (DEMOTE-4, >90 days), got {v}");
    }
}
