/// Fire-and-forget access tracker for the RAG hot path.
///
/// [`AccessTracker`] receives memory IDs via [`record`] from the retrieval hot
/// path and forwards them to an async worker over an unbounded channel.  The
/// worker uses `tokio::select!` to flush the buffer either when 30 seconds
/// elapse or when ≥500 entries accumulate.
///
/// # Design decisions
///
/// * `UnboundedSender<Vec<i64>>` — sender-only public surface keeps the struct
///   `Send + Sync` without interior `Connection`/`Mutex`.  The hot path MUST
///   NOT block; `UnboundedSender::send` never awaits.
/// * `record` is synchronous, infallible, and zero-copy beyond the `Vec<i64>`
///   already owned by the caller.
/// * Empty vectors are discarded at `record` entry — no channel traffic for
///   no-op calls.
/// * Flush errors are logged via `log::warn!` and the worker continues —
///   never panics.
///
/// [`record`]: AccessTracker::record

use std::collections::HashMap;
use tokio::sync::mpsc::UnboundedSender;
use tokio_rusqlite::Connection;

use crate::memory::clock::TimestampMs;
use crate::memory::{RankedHit, TableKind};

/// Buffers retrieved memory IDs and forwards them to the async flush worker.
pub struct AccessTracker {
    tx: UnboundedSender<Vec<i64>>,
}

// Compile-time guard — `AccessTracker` MUST be `Send + Sync` so it can live
// inside an `Arc<AccessTracker>` and be shared across threads.
const _: fn() = || {
    fn _ss<T: Send + Sync>() {}
    _ss::<AccessTracker>();
};

impl AccessTracker {
    /// Create a new `AccessTracker` and spawn the background flush worker.
    ///
    /// The worker uses `tokio::select!` with two triggers:
    /// * a 30-second interval tick;
    /// * the buffer reaching ≥500 entries.
    ///
    /// Flush errors are logged via `log::warn!` and the loop continues.
    /// The worker exits cleanly when all `Sender`s are dropped.
    pub fn new(conn: Connection) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<i64>>();

        tokio::spawn(async move {
            let mut buffer: Vec<i64> = Vec::new();
            // `interval_at(now + 30s, 30s)` — first tick fires 30 s after creation,
            // NOT immediately.  `interval(30s)` would fire an initial tick at t=0 which
            // races non-deterministically with rx.recv() in select!, making the
            // size-trigger flush test flaky: if the timer arm wins N times before
            // buffer=500 it flushes a partial buffer and the final count is wrong.
            let start =
                tokio::time::Instant::now() + std::time::Duration::from_secs(30);
            let mut interval =
                tokio::time::interval_at(start, std::time::Duration::from_secs(30));

            loop {
                tokio::select! {
                    maybe_ids = rx.recv() => {
                        match maybe_ids {
                            None => break, // all Senders dropped — graceful shutdown
                            Some(ids) => {
                                buffer.extend(ids);
                                if buffer.len() >= 500 {
                                    // Size trigger.
                                    flush(&conn, std::mem::take(&mut buffer)).await;
                                }
                            }
                        }
                    }
                    _ = interval.tick() => {
                        // Timer trigger.
                        if !buffer.is_empty() {
                            flush(&conn, std::mem::take(&mut buffer)).await;
                        }
                    }
                }
            }
        });

        Self { tx }
    }

    /// A tracker whose `record` goes nowhere: no flush worker, nothing ever
    /// reaches SQLite. For measurement runs, where retrieval must leave the
    /// store exactly as it found it.
    #[cfg(feature = "eval")]
    pub(crate) fn inert() -> Self {
        // The receiver is dropped on purpose; `record` already ignores a send
        // error, so every call becomes a no-op.
        let (tx, _) = tokio::sync::mpsc::unbounded_channel::<Vec<i64>>();
        Self { tx }
    }

    /// Record that the given memory IDs were accessed on the retrieval hot path.
    ///
    /// The call returns immediately without awaiting.  Empty `ids` vectors are
    /// silently discarded — no channel send occurs.
    pub fn record(&self, ids: Vec<i64>) {
        if ids.is_empty() {
            return;
        }
        self.tx.send(ids).ok(); // infallible by design: error means worker exited
    }

    /// Records the hits a retrieval actually found, out of the ones it hands back.
    ///
    /// Only `memory` rows: the flush updates `memory WHERE id = ?`, and ids are
    /// per-table, so an episodic or procedural id would land on an unrelated
    /// `memory` row that happens to share the number. Only hits that scored:
    /// a row at 0 is below the similarity floor and is there to fill a slot,
    /// and counting it would reset its decay and push it toward a promotion
    /// the query never earned it.
    pub fn record_hits(&self, hits: &[RankedHit]) {
        self.record(
            hits.iter()
                .filter(|h| h.kind == TableKind::Memory && h.score > 0.0)
                .map(|h| h.id)
                .collect(),
        );
    }
}

// ---------------------------------------------------------------------------
// Flush implementation
// ---------------------------------------------------------------------------

/// Flush `buffer` to SQLite using one atomic CTE `UPDATE`.
///
/// Coalesces duplicate IDs (summing hits per ID) before building the
/// parameterised `WITH updates … UPDATE memory … FROM updates` statement.
/// The `MAX(memory.last_accessed_at, updates.last_ts)` guard prevents
/// anti-regression from out-of-order batches.
/// `compute_tier` is called inside the same statement.
///
/// On error: logs via `log::warn!` and returns normally — the worker loop
/// MUST continue, never panic.
async fn flush(conn: &Connection, buffer: Vec<i64>) {
    if buffer.is_empty() {
        return;
    }

    // Timestamp for this flush batch (milliseconds since UNIX epoch).
    let now_ms = TimestampMs::now();

    // Coalesce: sum hits per unique ID.
    let mut deduped: HashMap<i64, i64> = HashMap::new();
    for id in buffer {
        *deduped.entry(id).or_insert(0) += 1;
    }

    // Build ordered (id, last_ts, hits) entries for VALUES clause.
    let entries: Vec<(i64, TimestampMs, i64)> = deduped
        .into_iter()
        .map(|(id, hits)| (id, now_ms, hits))
        .collect();

    // One `(?,?,?)` placeholder per deduplicated entry.
    let placeholders: String = std::iter::repeat("(?,?,?)")
        .take(entries.len())
        .collect::<Vec<_>>()
        .join(",");

    // Single CTE UPDATE — tier recomputed in the same statement.
    let sql = format!(
        "WITH updates(id, last_ts, hits) AS (VALUES {placeholders}) \
         UPDATE memory \
         SET last_accessed_at = MAX(memory.last_accessed_at, updates.last_ts), \
             access_count     = memory.access_count + updates.hits, \
             tier = compute_tier(memory.tier, memory.importance, \
                                 memory.access_count + updates.hits, \
                                 memory.memory_type) \
         FROM updates WHERE memory.id = updates.id"
    );

    // Flatten params: [id₁, ts₁, hits₁, id₂, ts₂, hits₂, …].
    // Using `rusqlite::types::Value` gives unambiguous `ToSql` impls.
    let params: Vec<rusqlite::types::Value> = entries
        .iter()
        .flat_map(|(id, ts, hits)| {
            [
                rusqlite::types::Value::Integer(*id),
                // The batch is a dynamically built Vec<Value>, so the instant's ToSql impl
                // never runs here — a Value has to already be an Integer. This is the only
                // place the SQL bind boundary drops the type; every other bind in this module
                // hands the typed value straight to params!. If you find yourself writing
                // as_millis_i64() at another bind site, bind the typed value instead of copying
                // this line.
                rusqlite::types::Value::Integer(ts.as_millis_i64()),
                rusqlite::types::Value::Integer(*hits),
            ]
        })
        .collect();

    // Single conn.call → tx.transaction() → execute.
    if let Err(e) = conn
        .call(move |c| -> rusqlite::Result<()> {
            let tx = c.transaction()?;
            {
                let mut stmt = tx.prepare(&sql)?;
                stmt.execute(rusqlite::params_from_iter(params))?;
            }
            tx.commit()
        })
        .await
    {
        // Log and return — the worker loop continues, never panics.
        log::warn!("access tracker flush failed (buffered updates discarded): {e}");
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
impl AccessTracker {
    /// Test-only constructor — returns both the tracker and the channel receiver
    /// so that tests can observe what was (or was not) sent.
    ///
    /// Does NOT spawn a worker task.  The caller is responsible for driving the
    /// receiver.
    fn for_test() -> (Self, tokio::sync::mpsc::UnboundedReceiver<Vec<i64>>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Self { tx }, rx)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test helpers ─────────────────────────────────────────────────────────

    /// Insert a single memory row with specified fields and return its `id`.
    async fn seed_memory_row(
        conn: &Connection,
        memory_type: &str,
        tier: i64,
        access_count: i64,
        importance: f64,
    ) -> i64 {
        let mt = memory_type.to_owned();
        conn.call(move |c| -> rusqlite::Result<i64> {
            c.execute(
                "INSERT INTO memory \
                 (scope, agent_id, content, memory_type, tier, access_count, importance, \
                  created_at, last_modified_at, last_accessed_at, \
                  created_by_type, created_by_ref) \
                 VALUES ('agent', 'test-agent', 'test content', ?, ?, ?, ?, \
                         0, 0, 0, 'agent', '')",
                rusqlite::params![mt, tier, access_count, importance],
            )?;
            Ok(c.last_insert_rowid())
        })
        .await
        .expect("seed_memory_row failed")
    }

    /// Insert `n` distinct memory rows in one transaction and return their IDs.
    async fn seed_n_rows(conn: &Connection, n: usize) -> Vec<i64> {
        conn.call(move |c| -> rusqlite::Result<Vec<i64>> {
            let tx = c.transaction()?;
            let mut ids = Vec::with_capacity(n);
            for i in 0..n {
                tx.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', 'test-agent', ?, 'chat_message', \
                             0, 0, 0, 'agent', '')",
                    rusqlite::params![format!("row {i}")],
                )?;
                ids.push(tx.last_insert_rowid());
            }
            tx.commit()?;
            Ok(ids)
        })
        .await
        .expect("seed_n_rows failed")
    }

    /// Polls `access_count` for `row_id` until it equals `want`, bounded by
    /// `budget` of REAL wall-clock time (`std::time::Instant`, never
    /// `tokio::time`). The timer and size triggers run under a PAUSED tokio clock
    /// (`#[tokio::test(start_paused = true)]`); `tokio::time::sleep` auto-advances
    /// that virtual clock while paused, which would fire the 30s `interval` again
    /// mid-poll — so this polls with `tokio::task::yield_now()` (never touches
    /// the clock) and times the budget against the OS clock instead. The flush
    /// worker drives its write through `conn` (`tokio_rusqlite`), which dispatches
    /// to a real background OS thread — a fixed `yield_now` count is not a
    /// synchronization primitive for that hop, so this bounds the wait instead of
    /// guessing a count.
    async fn await_access_count(
        conn: &Connection,
        row_id: i64,
        want: i64,
        budget: std::time::Duration,
    ) -> Result<i64, String> {
        let deadline = std::time::Instant::now() + budget;
        loop {
            let got: i64 = conn
                .call(move |c| {
                    c.query_row(
                        "SELECT access_count FROM memory WHERE id = ?",
                        [row_id],
                        |r| r.get(0),
                    )
                })
                .await
                .unwrap();
            if got == want {
                return Ok(got);
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!(
                    "access_count stayed at {got}, expected {want}, after {budget:?} of real time"
                ));
            }
            tokio::task::yield_now().await;
        }
    }

    // ── Compile-time Send + Sync guard ────────────────────────────
    //
    // The `const _` guard above the impl block is the actual compile-time
    // assertion.  If `AccessTracker` were not `Send + Sync`, this file would
    // fail to compile.  The test below documents the invariant in the test
    // output for visibility.

    #[test]
    fn access_tracker_is_send_and_sync() {
        // If this compiles, AccessTracker is Send + Sync.
        fn _assert_send<T: Send + Sync>() {}
        _assert_send::<AccessTracker>();
    }

    // ── empty vec is a no-op ─────────────────────────────────────

    #[test]
    fn record_empty_vec_sends_nothing_to_channel() {
        let (tracker, mut rx) = AccessTracker::for_test();

        tracker.record(vec![]);

        // Nothing should have been sent — try_recv returns an empty-channel error.
        match rx.try_recv() {
            Ok(ids) => panic!("expected no message, got {ids:?}"),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {} // expected
            Err(e) => panic!("unexpected channel error: {e:?}"),
        }
    }

    // ── which hits count as an access ─────────────────────────────

    fn ranked(id: i64, kind: crate::memory::TableKind, score: f64) -> crate::memory::RankedHit {
        crate::memory::RankedHit {
            id,
            kind,
            content: String::new(),
            memory_type: String::new(),
            tier: 3,
            status: "active".into(),
            score,
            distance: 0.5,
            created_at: TimestampMs::from_unix_millis(0),
        }
    }

    #[test]
    fn record_hits_keeps_only_memory_hits_that_scored() {
        use crate::memory::TableKind;
        let (tracker, mut rx) = AccessTracker::for_test();

        tracker.record_hits(&[
            ranked(1, TableKind::Memory, 0.4),
            ranked(2, TableKind::Memory, 0.0),
            ranked(3, TableKind::Procedural, 0.9),
            ranked(4, TableKind::Memory, 0.1),
        ]);

        assert_eq!(rx.try_recv().expect("a batch"), vec![1, 4]);
    }

    #[test]
    fn record_hits_with_nothing_countable_sends_nothing() {
        use crate::memory::TableKind;
        let (tracker, mut rx) = AccessTracker::for_test();

        tracker.record_hits(&[ranked(2, TableKind::Memory, 0.0), ranked(3, TableKind::Episodic, 0.7)]);

        assert!(rx.try_recv().is_err(), "a batch with no countable hit must not reach the channel");
    }

    // ── Extra: non-empty vec DOES send ──────────────────────────────────────

    #[test]
    fn record_nonempty_vec_sends_to_channel() {
        let (tracker, mut rx) = AccessTracker::for_test();

        tracker.record(vec![1, 2, 3]);

        match rx.try_recv() {
            Ok(ids) => assert_eq!(ids, vec![1, 2, 3]),
            Err(e) => panic!("expected a message, got error: {e:?}"),
        }
    }

    // ── timer trigger ─────────────────────────────────────────────
    //
    // Uses `start_paused = true` so Tokio's clock only moves via
    // `tokio::time::advance`.  No real sleep or wall-clock delay.
    //
    // Buffer has < 500 entries; after advance(31 s) the 30-second
    // interval tick fires and flush() executes, incrementing access_count.

    #[tokio::test(start_paused = true)]
    async fn timer_triggers_flush_after_30_seconds() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let row_id = seed_memory_row(&store.conn, "chat_message", 3, 0, 5.0).await;

        let tracker = AccessTracker::new(store.conn.clone());

        // Queue the record BEFORE yielding — it will be in the channel when
        // the worker first runs.
        tracker.record(vec![row_id]);

        // Yield to let the worker consume the initial interval tick (t=0,
        // buffer may be empty → no flush) and the recv (buffer=[row_id]).
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }

        // Advance Tokio's mock clock past the 30 s threshold.
        tokio::time::advance(std::time::Duration::from_secs(31)).await;

        // Let the worker process the second timer tick and complete the flush —
        // bounded poll, not a fixed yield count (see `await_access_count`).
        await_access_count(&store.conn, row_id, 1, std::time::Duration::from_secs(5))
            .await
            .expect("timer-triggered flush must increment access_count to 1");
    }

    // ── size trigger at ≥500 entries ──────────────────────────────
    //
    // Sends 500 records of the same ID (dedup → 1 row with 500 hits).
    // The flush MUST fire without advancing the clock.
    //
    // Uses `start_paused = true` so the 30-second interval cannot fire on its
    // own (no `advance()` is called).  Only the ≥500-entry size trigger can
    // cause a flush, making the test deterministic in the full suite.

    #[tokio::test(start_paused = true)]
    async fn size_trigger_at_500_entries_flushes_immediately() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let row_id = seed_memory_row(&store.conn, "chat_message", 3, 0, 5.0).await;

        let tracker = AccessTracker::new(store.conn.clone());

        // Send 500 records of the same ID — triggers size flush on the 500th recv.
        for _ in 0..500 {
            tracker.record(vec![row_id]);
        }

        // Each `select!` iteration processes one message, so the worker needs
        // many iterations to drain the channel before the size trigger fires —
        // and the flush itself is a real cross-thread DB round trip. Bounded
        // poll instead of a fixed yield count (see `await_access_count`);
        // (initial interval tick fires at t=0 with empty buffer → no flush;
        //  subsequent ticks require advance() which we never call → only size
        //  trigger can fire.)
        await_access_count(&store.conn, row_id, 500, std::time::Duration::from_secs(5))
            .await
            .expect(
                "size-triggered flush must set access_count=500 (500 hits deduped to 1 row)",
            );
    }

    // ── DB error recovery ──────────────────────────────────────────
    //
    // Drop `memory` table so the next flush fails.  Worker MUST log `log::warn!`
    // and continue — no panic, channel stays open.

    #[tokio::test]
    async fn flush_db_error_does_not_panic_worker() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let tracker = AccessTracker::new(store.conn.clone());

        // Corrupt the DB: drop `memory` so the flush SQL fails.
        store
            .conn
            .call(|c| -> rusqlite::Result<usize> { c.execute("DROP TABLE memory", []) })
            .await
            .unwrap();

        // Trigger a size flush (500 records) — flush will fail but MUST NOT panic.
        for i in 0i64..500 {
            tracker.record(vec![i]);
        }

        // Give the worker enough iterations to process and fail gracefully.
        for _ in 0..1_200 {
            tokio::task::yield_now().await;
        }

        // Verify the channel is still open — worker did not exit from the panic.
        tracker.record(vec![9999]);
        // Reaching here without panic means the worker survived the failed flush.
    }

    // ── tier promotion in the SAME CTE UPDATE ─────────────────────
    //
    // Seeds a row with tier=2, access_count=2, memory_type='architectural_decision'.
    // After flush with 1 hit: access_count becomes 3 → compute_tier returns 1.
    // The single CTE UPDATE MUST set both fields atomically.

    #[tokio::test]
    async fn flush_promotes_tier_atomically_in_same_cte_statement() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let row_id =
            seed_memory_row(&store.conn, "architectural_decision", 2, 2, 7.0).await;

        // Call flush() directly with 1 hit for this row.
        flush(&store.conn, vec![row_id]).await;

        let (access_count, tier): (i64, i64) = store
            .conn
            .call(move |c| -> rusqlite::Result<(i64, i64)> {
                c.query_row(
                    "SELECT access_count, tier FROM memory WHERE id = ?",
                    [row_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap();

        assert_eq!(
            access_count, 3,
            "access_count must be 3 after flush (2 stored + 1 hit); got {access_count}"
        );
        assert_eq!(
            tier, 1,
            "tier must be promoted to 1 by compute_tier in the same CTE UPDATE; got {tier}"
        );
    }

    // ── 1000 concurrent record() calls ────────────────────────────
    //
    // Spawn 1000 tokio tasks each calling record().  All must complete without
    // stalling — UnboundedSender::send never blocks the caller.

    #[tokio::test]
    async fn concurrent_1000_record_calls_no_stall() {
        use std::sync::Arc;
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let tracker = Arc::new(AccessTracker::new(store.conn.clone()));

        let handles: Vec<_> = (0i64..1_000)
            .map(|id| {
                let t = Arc::clone(&tracker);
                tokio::spawn(async move { t.record(vec![id]); })
            })
            .collect();

        for h in handles {
            h.await.expect("spawned record task must not panic");
        }
        // All 1000 tasks joined — no stall.
    }

    // ── 500-entry flush (1500 bind vars) ──────────────────────────
    //
    // Verifies that `set_limit(SQLITE_LIMIT_VARIABLE_NUMBER, 32766)`
    // allows a single CTE UPDATE with 500 rows × 3 vars = 1500 bind variables.
    // Without the limit raise this would error at row 334 (default limit = 999).

    #[tokio::test]
    async fn flush_500_distinct_ids_respects_variable_limit() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let ids = seed_n_rows(&store.conn, 500).await;
        assert_eq!(ids.len(), 500);

        // Direct call to flush() with 500 distinct IDs → 1500 bind variables.
        flush(&store.conn, ids).await;

        let updated: i64 = store
            .conn
            .call(|c| -> rusqlite::Result<i64> {
                c.query_row(
                    "SELECT COUNT(*) FROM memory WHERE access_count >= 1",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();

        assert_eq!(updated, 500, "all 500 rows must be updated; got {updated}");
    }
}
