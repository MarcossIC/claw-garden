//! Maintenance job — periodic inactivity-based tier demotion
//! (memoria-3-capas-fase-3).
//!
//! Wires [`crate::memory::tier::demote_inactive`] (pure, previously unused)
//! into a bulk `UPDATE` against `memory` ONLY, on a timer (`maintenance`
//! module, Lote 3). 5th instance of the wrapper/core convention
//! (worker → resolver → extractor → episodic_writer → maintenance).

use std::sync::Arc;
use std::time::Duration;

use crate::memory::db::MemoryStore;

/// Startup delay before the first maintenance pass: long enough to let the
/// initial-ingest burst (embedder model download + `memory_worker` draining
/// the first session) drain off the single-writer `tokio_rusqlite` connection
/// without contending for disk/writer, and short enough (<< a typical
/// multi-hour desktop session) that the first pass reliably fires.
pub(crate) const STARTUP_DELAY: Duration = Duration::from_secs(5 * 60);

/// Steady-state period between maintenance passes.
pub(crate) const MAINTENANCE_PERIOD: Duration = Duration::from_secs(24 * 60 * 60);

/// SQL for the bulk demotion pass.
///
/// `?1` is `now_ms`, computed in Rust (see [`run_maintenance_once`]) and
/// bound here — NEVER via `strftime`. All producers (`access.rs::flush`,
/// `worker.rs`, `episodic_writer.rs`) store `last_accessed_at` in
/// MILLISECONDS since UNIX epoch. `search.rs:83` treats it as SECONDS
/// (`strftime('%s','now') - m.last_accessed_at`) — a PRE-EXISTING unit
/// mismatch bug, out of scope here and tracked by the future
/// `fix-decay-unit-mismatch` change. Do NOT copy that pattern.
///
/// The `WHERE tier < 3 AND demote_tier(...) <> tier` guard means only rows
/// that actually change tier get their `last_modified_at` bumped — avoids
/// stamping every tier<3 row (and the WAL churn that would cause) on every
/// pass, and scopes the statement to `memory` ONLY (R3 — `episodic_trajectory`
/// / `procedural_skill` are never touched by this job).
const DEMOTE_SQL: &str = "\
    UPDATE memory \
    SET tier = demote_tier(tier, (?1 - last_accessed_at) / 86400000.0, importance), \
        last_modified_at = ?1 \
    WHERE tier < 3 \
      AND demote_tier(tier, (?1 - last_accessed_at) / 86400000.0, importance) <> tier";

/// Runs one bulk inactivity-demotion pass against `memory`.
///
/// `now_ms` is computed HERE, once, in Rust (mirrors `access.rs::flush:121`)
/// and bound as `?1` for both the `days_since_last_access` calculation and
/// the `last_modified_at` stamp — a single consistent "now" per pass.
///
/// Never panics: a recoverable DB error is logged via `log::warn!` and the
/// call returns — the caller (the maintenance loop, Lote 3) keeps ticking.
pub(crate) async fn run_maintenance_once(store: &MemoryStore) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    if let Err(e) = store
        .conn
        .call(move |c| c.execute(DEMOTE_SQL, rusqlite::params![now_ms]))
        .await
    {
        log::warn!("maintenance: bulk tier demotion failed: {e}");
    }
}

/// Drives [`run_maintenance_once`] on a timer: fires the first pass
/// `startup_delay` after being spawned (NOT at t=0 — `interval_at`'s first
/// tick is at `Instant::now() + startup_delay`), then every `period`
/// thereafter. `startup_delay`/`period` are injectable so tests can drive
/// this deterministically with a paused mock clock (`#[tokio::test(start_paused
/// = true)]` + `tokio::time::advance`), mirroring the `AccessTracker` flush
/// worker's `interval_at` discipline (`access.rs`).
///
/// Runs for the app lifetime — never returns. `Arc<MemoryStore>` derefs to
/// `&MemoryStore` for [`run_maintenance_once`] (no explicit deref needed).
pub(crate) async fn run_maintenance_loop(
    store: Arc<MemoryStore>,
    startup_delay: Duration,
    period: Duration,
) {
    let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + startup_delay, period);
    loop {
        interval.tick().await;
        run_maintenance_once(&store).await;
    }
}

/// Spawn entry point — called from `lib.rs::run()` setup hook.
///
/// AppHandle-thin wrapper: resolves `Arc<MemoryStore>` from `AppState` and
/// hands off to the testable core [`run_maintenance_loop`]. Mirrors
/// `worker::start_memory_worker` — 5th instance of the wrapper/core
/// convention (worker → resolver → extractor → episodic_writer →
/// maintenance). Not directly testable (no `tauri` "test" feature in this
/// codebase); see the compile-time `Send + 'static` proof below instead.
pub(crate) fn start_maintenance_worker(app_handle: tauri::AppHandle) {
    use tauri::Manager;
    let state = app_handle.state::<crate::state::AppState>();
    let store = state.memory_store.clone();

    tauri::async_runtime::spawn(run_maintenance_loop(store, STARTUP_DELAY, MAINTENANCE_PERIOD));
}

// Compile-time Send proof — every captured value MUST be Send + 'static so
// tauri::async_runtime::spawn accepts the future (mirrors worker.rs:165 /
// episodic_writer.rs:307 — 5th instance of this guard).
const _: fn() = || {
    fn s<T: Send + 'static>() {}
    s::<tauri::AppHandle>();
    s::<Arc<MemoryStore>>();
    s::<Duration>();
};

#[cfg(test)]
mod tests {
    use crate::memory::db::MemoryStore;
    use std::sync::atomic::{AtomicI64, Ordering};

    // ── Test helpers ──────────────────────────────────────────────────────

    /// Current wall-clock time in milliseconds since UNIX epoch — same
    /// formula used in production (`run_maintenance_once`, `access.rs::flush`).
    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    }

    /// Monotonic counter to keep `episodic_trajectory.session_id` (UNIQUE)
    /// distinct across seeded rows within a single test.
    static NONCE: AtomicI64 = AtomicI64::new(0);

    /// Seeds one row in `table` ("memory" | "episodic_trajectory" |
    /// "procedural_skill") with the given `tier`/`importance`, and
    /// `last_accessed_at = now_ms() + offset_ms` (so a NEGATIVE `offset_ms`
    /// places the row that many milliseconds in the PAST — matching the
    /// spec's worked-example offsets, e.g. `-7_776_000_001` for "just over
    /// 90 days ago"). `last_modified_at`/`created_at` are seeded at `0` so
    /// tests can assert "untouched" by checking they stay `0`.
    async fn seed_row(store: &MemoryStore, table: &str, tier: i64, importance: f64, offset_ms: i64) -> i64 {
        let last_accessed_at = now_ms() + offset_ms;
        let nonce = NONCE.fetch_add(1, Ordering::SeqCst);
        let table = table.to_string();
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                match table.as_str() {
                    "memory" => {
                        c.execute(
                            "INSERT INTO memory \
                             (scope, agent_id, content, memory_type, tier, importance, \
                              created_at, last_modified_at, last_accessed_at, \
                              created_by_type, created_by_ref) \
                             VALUES ('agent', 'test-agent', 'seed content', 'chat_message', \
                                     ?1, ?2, 0, 0, ?3, 'agent', '')",
                            rusqlite::params![tier, importance, last_accessed_at],
                        )?;
                    }
                    "episodic_trajectory" => {
                        c.execute(
                            "INSERT INTO episodic_trajectory \
                             (scope, agent_id, session_id, task, tier, importance, \
                              created_at, last_modified_at, last_accessed_at, \
                              created_by_type, created_by_ref) \
                             VALUES ('agent', 'test-agent', ?4, 'seed task', \
                                     ?1, ?2, 0, 0, ?3, 'agent', '')",
                            rusqlite::params![
                                tier,
                                importance,
                                last_accessed_at,
                                format!("sess-{nonce}")
                            ],
                        )?;
                    }
                    "procedural_skill" => {
                        c.execute(
                            "INSERT INTO procedural_skill \
                             (scope, agent_id, skill_name, trigger, tier, importance, \
                              promoted_at, source_trajectory_ids, \
                              created_at, last_modified_at, last_accessed_at, \
                              created_by_type, created_by_ref) \
                             VALUES ('agent', 'test-agent', ?4, 'seed trigger', \
                                     ?1, ?2, 0, '[]', 0, 0, ?3, 'agent', '')",
                            rusqlite::params![
                                tier,
                                importance,
                                last_accessed_at,
                                format!("skill-{nonce}")
                            ],
                        )?;
                    }
                    other => panic!("seed_row: unknown table '{other}'"),
                }
                Ok(c.last_insert_rowid())
            })
            .await
            .expect("seed_row failed")
    }

    async fn tier_of(store: &MemoryStore, table: &str, id: i64) -> i64 {
        let sql = format!("SELECT tier FROM {table} WHERE id = ?1");
        store
            .conn
            .call(move |c| c.query_row(&sql, [id], |r| r.get(0)))
            .await
            .unwrap()
    }

    async fn last_modified_at_of(store: &MemoryStore, table: &str, id: i64) -> i64 {
        let sql = format!("SELECT last_modified_at FROM {table} WHERE id = ?1");
        store
            .conn
            .call(move |c| c.query_row(&sql, [id], |r| r.get(0)))
            .await
            .unwrap()
    }

    // ── Task 2.2 — tier1, importance<9, >90 days → tier 2 ───────────────────

    #[tokio::test]
    async fn tier1_low_importance_over_90_days_demotes_to_2() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_row(&store, "memory", 1, 5.0, -7_776_000_001).await;

        super::run_maintenance_once(&store).await;

        assert_eq!(tier_of(&store, "memory", id).await, 2);
    }

    // ── Task 2.3 — tier1, importance<9, <90 days → unchanged, last_modified_at intact ──

    #[tokio::test]
    async fn tier1_low_importance_under_90_days_unchanged_last_modified_intact() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_row(&store, "memory", 1, 5.0, -7_775_999_999).await;

        super::run_maintenance_once(&store).await;

        assert_eq!(tier_of(&store, "memory", id).await, 1, "must NOT demote before 90 days");
        assert_eq!(
            last_modified_at_of(&store, "memory", id).await,
            0,
            "last_modified_at must stay untouched (0) when the guard skips the row"
        );
    }

    // ── Task 2.4 — tier2, >180 days → tier 3 ─────────────────────────────────

    #[tokio::test]
    async fn tier2_over_180_days_demotes_to_3() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_row(&store, "memory", 2, 5.0, -15_552_000_001).await;

        super::run_maintenance_once(&store).await;

        assert_eq!(tier_of(&store, "memory", id).await, 3);
    }

    // ── Extra triangulation — tier2, <180 days → unchanged ───────────────────

    #[tokio::test]
    async fn tier2_under_180_days_unchanged() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_row(&store, "memory", 2, 5.0, -15_551_999_999).await;

        super::run_maintenance_once(&store).await;

        assert_eq!(tier_of(&store, "memory", id).await, 2, "must NOT demote before 180 days");
    }

    // ── Task 2.5 — tier1, importance>=9.0 (pivotal), 365 days → NEVER demotes ──

    #[tokio::test]
    async fn tier1_pivotal_importance_never_demotes() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_row(&store, "memory", 1, 9.5, -31_536_000_000).await;

        super::run_maintenance_once(&store).await;

        assert_eq!(tier_of(&store, "memory", id).await, 1, "importance >= 9.0 must never demote");
    }

    // ── Task 2.6 — tier3 rows are never touched (WHERE tier < 3 guard) ──────

    #[tokio::test]
    async fn tier3_row_untouched() {
        let store = MemoryStore::new_for_test().await.unwrap();
        // Same offset as the tier2->3 case — if the WHERE tier<3 guard were
        // missing this would still be a no-op (demote_inactive(3, ..) = 3
        // unconditionally), so this test specifically pins tier staying 3
        // AND last_modified_at staying untouched (proving the row was never
        // part of the UPDATE's matched set).
        let id = seed_row(&store, "memory", 3, 5.0, -15_552_000_001).await;

        super::run_maintenance_once(&store).await;

        assert_eq!(tier_of(&store, "memory", id).await, 3);
        assert_eq!(last_modified_at_of(&store, "memory", id).await, 0);
    }

    // ── Task 2.7 (R3) — only `memory` is scoped; episodic_trajectory / ──────
    // ── procedural_skill stay untouched even with identical demotable rows ──

    #[tokio::test]
    async fn only_memory_table_is_demoted_other_tables_untouched() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let mem_id = seed_row(&store, "memory", 1, 5.0, -7_776_000_001).await;
        let epi_id = seed_row(&store, "episodic_trajectory", 1, 5.0, -7_776_000_001).await;
        let proc_id = seed_row(&store, "procedural_skill", 1, 5.0, -7_776_000_001).await;

        super::run_maintenance_once(&store).await;

        assert_eq!(tier_of(&store, "memory", mem_id).await, 2, "memory row must demote");
        assert_eq!(
            tier_of(&store, "episodic_trajectory", epi_id).await,
            1,
            "episodic_trajectory must NOT be touched by run_maintenance_once (R3)"
        );
        assert_eq!(
            tier_of(&store, "procedural_skill", proc_id).await,
            1,
            "procedural_skill must NOT be touched by run_maintenance_once (R3)"
        );
    }

    // ── Lote 3 — timer/loop ──────────────────────────────────────────────────

    use std::sync::Arc;
    use std::time::Duration;

    // Task 3.1 + 3.2 — interval_at's first tick fires at now+startup_delay,
    // NOT at t=0: asserts NO change before any clock advance (anti
    // false-green), then asserts the first pass DOES fire once startup_delay
    // elapses.
    #[tokio::test(start_paused = true)]
    async fn loop_does_not_fire_before_startup_delay_then_fires_first_pass_after_delay() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_row(&store, "memory", 1, 5.0, -7_776_000_001).await;
        let store = Arc::new(store);

        let handle = tokio::spawn(super::run_maintenance_loop(
            Arc::clone(&store),
            Duration::from_secs(1),
            Duration::from_secs(60),
        ));

        // Anti false-green: BEFORE any clock advance, nothing must have fired.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            tier_of(&store, "memory", id).await,
            1,
            "must NOT fire before startup_delay elapses (t=0 must be a no-op)"
        );

        // Advance past startup_delay=1s.
        tokio::time::advance(Duration::from_secs(2)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            tier_of(&store, "memory", id).await,
            2,
            "first pass must fire once startup_delay elapses"
        );

        handle.abort();
    }

    // Task 3.3 — a second pass fires ONLY after a full `period` has elapsed
    // since the first tick, not before.
    //
    // Strategy: ONE row, seeded ONCE before spawning (no `conn.call` is ever
    // interleaved between `advance()` calls while the loop task is
    // concurrently parked on its own timer — doing so was observed to make
    // Tokio's paused clock auto-advance PAST the intended checkpoint to
    // avoid a runtime stall, since a real cross-thread channel await and a
    // pending timer look "stalled" to the executor). The row is seeded
    // >180 days stale (`-15_552_000_001`) so `demote_inactive` fires TWICE
    // for the SAME row against the SAME (real-clock) "now": tier 1→2 at
    // tick 1, then 2→3 at tick 2 — `demote_inactive` only steps down ONE
    // tier per call (see `tier.rs`), so this needs exactly two passes.
    #[tokio::test(start_paused = true)]
    async fn loop_second_pass_fires_only_after_full_period_not_before() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_row(&store, "memory", 1, 5.0, -15_552_000_001).await;
        let store = Arc::new(store);

        let handle = tokio::spawn(super::run_maintenance_loop(
            Arc::clone(&store),
            Duration::from_secs(1),
            Duration::from_secs(60),
        ));

        // Anti false-green: before ANY clock advance, nothing must have fired.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(tier_of(&store, "memory", id).await, 1, "must NOT fire before startup_delay elapses");

        // ONE jump past startup_delay (1s) but well below the next deadline
        // (startup_delay + period = 61s) — tick 1 fires, tick 2 must NOT.
        tokio::time::advance(Duration::from_secs(32)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            tier_of(&store, "memory", id).await,
            2,
            "only tick 1 must have fired by t=32s (tier 1→2), tick 2 (t=61s) must NOT fire yet"
        );

        // Complete the period (total elapsed since spawn now > 61s) → tick 2
        // fires → tier 2→3.
        tokio::time::advance(Duration::from_secs(35)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            tier_of(&store, "memory", id).await,
            3,
            "second pass must fire once the full period elapses"
        );

        handle.abort();
    }

    // Task 4.3 — compile-time Send + 'static proof for the value(s) captured
    // by the future passed to
    // tauri::async_runtime::spawn(run_maintenance_loop(store, delay, period)).
    #[test]
    fn start_maintenance_worker_captured_types_are_send_static() {
        fn s<T: Send + 'static>() {}
        s::<tauri::AppHandle>();
        s::<Arc<MemoryStore>>();
        s::<Duration>();
    }
}
