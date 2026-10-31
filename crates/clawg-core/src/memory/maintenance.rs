//! Maintenance job — periodic inactivity-based tier demotion.
//!
//! Wires [`crate::memory::tier::demote_inactive`] (pure, previously unused)
//! into a bulk `UPDATE` against `memory` ONLY, on a timer (`maintenance`
//! module).

use std::sync::Arc;
use std::time::Duration;

use crate::memory::chunker::embedding_text;
use crate::memory::clock::TimestampMs;
use crate::memory::db::{EmbeddingIdentity, MemoryStore};
use crate::memory::dispute::settle_disputed;
use crate::memory::embedder::{EmbedRole, Embedder};
use crate::memory::revival::revive_hidden;
use crate::memory::vector::to_unit;
use crate::memory::MemoryError;
use crate::metrics::EventKind;
use serde::Serialize;

/// Startup delay before the first maintenance pass: long enough to let the
/// initial-ingest burst (embedder model download + `memory_worker` draining
/// the first session) drain off the single-writer `tokio_rusqlite` connection
/// without contending for disk/writer, and short enough (<< a typical
/// multi-hour session) that the first pass reliably fires.
pub const STARTUP_DELAY: Duration = Duration::from_secs(5 * 60);

/// Steady-state period between maintenance passes.
pub const MAINTENANCE_PERIOD: Duration = Duration::from_secs(24 * 60 * 60);

/// SQL for the bulk demotion pass.
///
/// `?1` is `now_ms`, computed in Rust (see [`run_maintenance_once`]) and
/// bound here — NEVER via `strftime`. All producers (`access.rs::flush`,
/// `worker.rs`) store `last_accessed_at` in
/// MILLISECONDS since UNIX epoch, and this pass divides that stamp by the
/// same millisecond day constant the retrieval queries use, so this pass
/// and retrieval agree on the unit.
///
/// The `WHERE tier < 3 AND demote_tier(...) <> tier` guard means only rows
/// that actually change tier get their `last_modified_at` bumped — avoids
/// stamping every tier<3 row (and the WAL churn that would cause) on every
/// pass.
fn demote_sql(table: &str) -> String {
    format!(
        "UPDATE {table} \
         SET tier = demote_tier(tier, (?1 - last_accessed_at) / 86400000.0, importance), \
             last_modified_at = ?1 \
         WHERE tier < 3 \
           AND demote_tier(tier, (?1 - last_accessed_at) / 86400000.0, importance) <> tier"
    )
}

/// Every table whose rows carry a tier that inactivity is allowed to lower.
///
/// All three, and the two beyond `memory` are not symmetry for its own sake.
/// A skill is inserted at tier 1 with importance 8.0 — under the 9.0 pivotal
/// cutoff — so one the user stopped reaching for a year ago kept a tier-1 floor
/// forever and outranked fresh material on every query. That is the reachability
/// bug this pass exists to prevent, and it was live on `procedural_skill`.
///
/// `episodic_trajectory` finds nothing today: the writer hands its insert a
/// hardcoded importance of 5.0, which lands every trajectory on tier 3, and
/// tier 3 is the floor. It is covered anyway because that is one literal, not a
/// structural property — `compute_tier` fast-tracks importance >= 9.0 to tier 1
/// before it looks at the type at all. Leaving the table out would mean the day
/// someone derives trajectory importance from its success score, trajectories
/// start promoting and never decay, and nothing says so. The cost of covering
/// it is one UPDATE per day that matches zero rows.
const DEMOTING_TABLES: [&str; 3] = ["memory", "episodic_trajectory", "procedural_skill"];

/// Runs one bulk inactivity-demotion pass against `memory`.
///
/// `now_ms` is computed HERE, once, in Rust (mirrors `access.rs::flush:121`)
/// and handed to [`run_maintenance_once_at`] — a single consistent "now"
/// per pass.
pub(crate) async fn run_maintenance_once(store: &MemoryStore) {
    let now_ms = TimestampMs::now();

    run_maintenance_once_at(store, now_ms).await;
}

/// Clock-injected core of [`run_maintenance_once`]: `now_ms` is bound as
/// `?1` for both the `days_since_last_access` calculation and the
/// `last_modified_at` stamp. Injectable so boundary tests can seed rows and
/// run the pass against the SAME instant — the just-under-boundary cases
/// (90d/180d minus 1ms) are only deterministic when seed and pass share
/// one "now"; with two wall-clock reads any >1ms scheduling gap crosses
/// the boundary and demotes spuriously.
///
/// Never panics: a recoverable DB error is logged via `log::warn!` and the
/// call returns — the caller (the maintenance loop) keeps ticking.
pub(crate) async fn run_maintenance_once_at(store: &MemoryStore, now_ms: TimestampMs) {
    let started = std::time::Instant::now();
    // One statement per table, and a failure moves to the next rather than
    // aborting: the three tables decay independently, so one table's demotion
    // failing is no reason to leave the other two stale for a whole day. Same
    // per-table isolation `sweep_orphaned_vectors` applies below.
    for table in DEMOTING_TABLES {
        let sql = demote_sql(table);
        if let Err(e) = store
            .conn
            .call(move |c| c.execute(&sql, rusqlite::params![now_ms]))
            .await
        {
            log::warn!("maintenance: bulk tier demotion failed for {table}: {e}");
        }
    }

    // Placed after the demotion pass (which can't orphan a vector — it never
    // touches deleted_at) and before the corpus-size log below, so that log
    // line reports the post-sweep state. Soft-deleting a row prunes its
    // vector inline (`delete_memory_core`), but rows deleted before that
    // code existed are still sitting on an orphaned vector row nothing will
    // ever reference again — this sweep is the backfill for those.
    let swept = sweep_orphaned_vectors(store).await;
    if swept > 0 {
        log::info!("maintenance: dropped {swept} vector rows left behind by soft-deleted rows");
    }

    let released = release_orphaned_disputes(store, now_ms).await;
    if released > 0 {
        log::info!("maintenance: released {released} disputes whose other side was deleted");
    }

    let revived = revive_hidden_revisions(store, now_ms).await;
    if revived > 0 {
        log::info!("maintenance: brought back {revived} rows a delete of the current version of their topic left hidden");
    }

    let dropped = drop_orphaned_pairs(store).await;
    if dropped > 0 {
        log::info!("maintenance: dropped {dropped} conflict pairs that name a row no longer stored");
    }

    let classified = classify_languages(store).await;
    if classified > 0 {
        log::info!("maintenance: recorded the language of {classified} rows saved before it was recorded");
    }
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    store.metrics().event(
        EventKind::Maintenance,
        &[
            ("ms", ms.into()),
            ("swept", swept.into()),
            ("released", released.into()),
            ("revived", revived.into()),
            ("dropped_pairs", dropped.into()),
            ("classified", classified.into()),
        ],
    );

    // Placed after the demotion pass so the counts describe the post-pass
    // state. A query failure here logs and returns like the demotion above —
    // losing one day's corpus-size log line is not worth stopping the loop.
    match count_rows(store).await {
        Ok(counts) => {
            log::info!(
                "corpus live/total memory={}/{} episodic={}/{} procedural={}/{}",
                counts.memory.live,
                counts.memory.total,
                counts.episodic.live,
                counts.episodic.total,
                counts.procedural.live,
                counts.procedural.total,
            );
        }
        Err(e) => log::warn!("maintenance: row count query failed: {e}"),
    }
}

/// Rows [`classify_languages`] reads and writes per transaction, so a save
/// waiting for the single connection waits for one short batch at most.
const LANG_BATCH: i64 = 500;

/// Records the language of every `memory` row that has none, deleted and
/// superseded rows included since they can come back, and returns how many.
/// Only `lang` changes: no trigger reads it, so the text index, the content,
/// its hash and the modification stamps stay as they were. The text is read
/// in batches and classified off the connection's thread. A failure is
/// logged and ends the pass; the next one picks up what is left.
async fn classify_languages(store: &MemoryStore) -> usize {
    let mut classified = 0usize;
    loop {
        let batch = store
            .conn
            .call(|c| {
                let mut stmt = c.prepare_cached("SELECT id, content FROM memory WHERE lang IS NULL ORDER BY id LIMIT ?1")?;
                let rows = stmt.query_map([LANG_BATCH], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await;
        let batch = match batch {
            Ok(batch) if batch.is_empty() => return classified,
            Ok(batch) => batch,
            Err(e) => {
                log::warn!("maintenance: reading rows without a language failed: {e}");
                return classified;
            }
        };
        let langs: Vec<(i64, &'static str)> = batch
            .iter()
            .map(|(id, content)| (*id, crate::memory::language::column_value(crate::memory::language::detect(content))))
            .collect();
        let written = store
            .conn
            .call(move |c| {
                let tx = c.transaction()?;
                let mut written = 0usize;
                {
                    let mut stmt = tx.prepare_cached("UPDATE memory SET lang = ?2 WHERE id = ?1 AND lang IS NULL")?;
                    for (id, lang) in &langs {
                        written += stmt.execute(rusqlite::params![id, lang])?;
                    }
                }
                tx.commit()?;
                Ok::<_, rusqlite::Error>(written)
            })
            .await;
        match written {
            Ok(n) => classified += n,
            Err(e) => {
                log::warn!("maintenance: recording the language of {} rows failed: {e}", batch.len());
                return classified;
            }
        }
    }
}

/// Deletes the vector row of every soft-deleted parent row, across all 3
/// tables, and reports how many it found. Two steps per table — select the
/// orphans, then one `DELETE FROM {vectors} WHERE rowid = ?1` per id — rather
/// than `DELETE ... WHERE rowid IN (subquery)`: a bare `rowid = ?` is the
/// point-lookup plan the vector virtual table supports directly, while the
/// `IN (subquery)` form drops to a full scan plus a per-row update on the
/// whole vector table.
///
/// The selection joins the parent to its vector rather than listing every
/// soft-deleted id, and that join is what makes a repeated pass free. Only
/// `clawg purge` removes a parent row, so "soft-deleted" is a set that keeps
/// growing: selecting on it alone would hand the loop below a longer
/// list every single day and spend a point delete on each one to discover the
/// vector had already been dropped years ago. Joining to the vector table
/// means the list IS the orphans — once they are gone the select returns
/// nothing and the loop runs zero statements, which is the property
/// `maintenance_sweep_idempotent` pins down.
///
/// What the join costs is a walk of the vector table's rowid list, with one
/// primary-key lookup per row (`SCAN v VIRTUAL TABLE INDEX 0:1` + `SEARCH p
/// USING INTEGER PRIMARY KEY`, read off `EXPLAIN QUERY PLAN` against this
/// schema). That walk touches no embedding blob and computes no distance, and
/// it is bounded by how many vectors are live — a number that shrinks as rows
/// are deleted instead of growing with every tombstone the parent table keeps.
///
/// A query failure is logged and this moves to the next table — one table's
/// sweep failing must not block the others or abort the demotion pass already
/// committed above.
async fn sweep_orphaned_vectors(store: &MemoryStore) -> usize {
    const TABLES: [(&str, &str); 3] = [
        ("memory", "memory_vectors"),
        ("episodic_trajectory", "episodic_vectors"),
        ("procedural_skill", "procedural_vectors"),
    ];

    let mut swept = 0usize;
    for (parent, vectors) in TABLES {
        let select_sql = format!(
            "SELECT p.id FROM {parent} p \
             JOIN {vectors} v ON v.rowid = p.id \
             WHERE p.deleted_at IS NOT NULL"
        );
        let delete_sql = format!("DELETE FROM {vectors} WHERE rowid = ?1");
        let result = store
            .conn
            .call(move |c| -> rusqlite::Result<usize> {
                let ids: Vec<i64> = {
                    let mut stmt = c.prepare(&select_sql)?;
                    let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
                    rows.collect::<rusqlite::Result<Vec<i64>>>()?
                };
                let n = ids.len();
                for id in ids {
                    c.execute(&delete_sql, rusqlite::params![id])?;
                }
                Ok(n)
            })
            .await;
        match result {
            Ok(n) => swept += n,
            Err(e) => log::warn!("maintenance: orphaned-vector sweep failed for {parent}: {e}"),
        }
    }
    swept
}

/// Returns every `disputed` row with no live row left to dispute to
/// `active`, and reports how many it changed. A failure is logged and
/// reported as 0, like the vector sweep above.
async fn release_orphaned_disputes(store: &MemoryStore, now_ms: TimestampMs) -> usize {
    match store.conn.call(move |c| settle_disputed(c, now_ms)).await {
        Ok(n) => n,
        Err(e) => {
            log::warn!("maintenance: orphaned-dispute release failed: {e}");
            0
        }
    }
}

/// Brings back the versions a delete that predates revivals left superseded
/// by the deleted current version of their topic (`revival.rs`). A failure is
/// logged and reported as 0, like the passes above.
async fn revive_hidden_revisions(store: &MemoryStore, now_ms: TimestampMs) -> usize {
    match store.conn.call(move |c| revive_hidden(c, now_ms)).await {
        Ok(n) => n,
        Err(e) => {
            log::warn!("maintenance: reviving hidden revisions failed: {e}");
            0
        }
    }
}

/// Deletes the conflict pairs that name a row the store no longer holds,
/// which only a purge by a version that predates the pairs leaves behind. A
/// failure is logged and reported as 0, like the passes above.
async fn drop_orphaned_pairs(store: &MemoryStore) -> usize {
    let sql = "DELETE FROM memory_conflict \
               WHERE NOT EXISTS (SELECT 1 FROM memory WHERE id = new_id) \
                  OR NOT EXISTS (SELECT 1 FROM memory WHERE id = old_id)";
    match store.conn.call(move |c| c.execute(sql, [])).await {
        Ok(n) => n,
        Err(e) => {
            log::warn!("maintenance: orphaned-pair drop failed: {e}");
            0
        }
    }
}

/// Total vs. live (`deleted_at IS NULL`) row count for one table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowCounts {
    pub(crate) total: i64,
    pub(crate) live: i64,
}

/// Row counts for all three memory tables, queried once per maintenance pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TableRowCounts {
    pub(crate) memory: RowCounts,
    pub(crate) episodic: RowCounts,
    pub(crate) procedural: RowCounts,
}

/// Two narrow `SELECT COUNT(*)` statements per table rather than one
/// `SUM(deleted_at IS NULL)`: the bare count rides `idx_memory_last_accessed`
/// (`V1__init.sql`, the only non-partial index on `memory`) as a
/// covering-index scan, and the live count rides the partial
/// `idx_memory_agent_active` (`WHERE deleted_at IS NULL`) exactly. Folding
/// both into one `SUM` expression forces a full table scan instead, because
/// `deleted_at` sits in neither index's payload — verified with `EXPLAIN
/// QUERY PLAN` against this schema: the two-statement form scans via
/// `COVERING INDEX idx_memory_last_accessed` and `INDEX
/// idx_memory_agent_active` respectively, while the folded `SUM` form falls
/// back to a bare `SCAN memory`.
///
/// Reporting only the live count would hide how much of each table is
/// soft-deleted. Those rows stay in the parent table until `clawg purge`
/// removes one from `memory` (nothing removes rows of `episodic_trajectory`
/// or `procedural_skill`), so the gap between `total` and `live` is accumulated
/// tombstone weight. Their vectors no longer sit in that gap: a delete drops
/// the vector row inline and [`sweep_orphaned_vectors`] backfills the ones
/// deleted before that existed, so the gap no longer inflates the vec0 scan.
/// What it still measures is parent-table growth, which is what decides when
/// a real purge becomes necessary.
pub(crate) async fn count_rows(store: &MemoryStore) -> Result<TableRowCounts, MemoryError> {
    store
        .conn
        .call(|c| -> rusqlite::Result<TableRowCounts> {
            let count = |sql: &str| -> rusqlite::Result<i64> { c.query_row(sql, [], |r| r.get(0)) };
            Ok(TableRowCounts {
                memory: RowCounts {
                    total: count("SELECT COUNT(*) FROM memory")?,
                    live: count("SELECT COUNT(*) FROM memory WHERE deleted_at IS NULL")?,
                },
                episodic: RowCounts {
                    total: count("SELECT COUNT(*) FROM episodic_trajectory")?,
                    live: count(
                        "SELECT COUNT(*) FROM episodic_trajectory WHERE deleted_at IS NULL",
                    )?,
                },
                procedural: RowCounts {
                    total: count("SELECT COUNT(*) FROM procedural_skill")?,
                    live: count(
                        "SELECT COUNT(*) FROM procedural_skill WHERE deleted_at IS NULL",
                    )?,
                },
            })
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend("connection closed during count_rows".into())
            }
            other => MemoryError::Backend(other.to_string()),
        })
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
pub async fn run_maintenance_loop(
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


// ---------------------------------------------------------------------------
// reembed_all — operator-invoked E5-prefix backfill
// ---------------------------------------------------------------------------

/// Rows per batch for [`reembed_table`]. Unlike [`sweep_orphaned_vectors`],
/// which loads every candidate id unbounded, this cannot: each row carries
/// full TEXT and pays ONNX inference, so an unbounded load would hold the
/// whole corpus's text in memory at once and never surface partial progress.
/// Keyset-paginated (`id > last_id`, never `OFFSET`) — ids are stable and
/// never rewritten, so the cursor needs no re-scan of rows already passed.
const REEMBED_BATCH: i64 = 200;

/// Which of the three vector-bearing tables [`reembed_table`] is walking.
/// A closed enum rather than a `(&str, &str)` pair like
/// [`sweep_orphaned_vectors`]'s `TABLES`, because `procedural_skill` needs a
/// THIRD column (`skill_name`, alongside `trigger`) the other two tables
/// don't have — a uniform 2-column tuple can't express that without a
/// sentinel value threaded through code that doesn't need it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReembedTable {
    Memory,
    Episodic,
    Procedural,
}

impl ReembedTable {
    fn parent(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Episodic => "episodic_trajectory",
            Self::Procedural => "procedural_skill",
        }
    }

    fn vectors(self) -> &'static str {
        match self {
            Self::Memory => "memory_vectors",
            Self::Episodic => "episodic_vectors",
            Self::Procedural => "procedural_vectors",
        }
    }
}

/// The text a `procedural_skill` row's vector was embedded from. It is the
/// string the skill was first written with, so a re-embed must rebuild it
/// exactly or the new vector lands somewhere the old one never was.
pub(crate) fn skill_embedding_text(trigger: &str, skill_name: &str) -> String {
    format!("{trigger} {skill_name}")
}

/// One row read by [`select_reembed_batch`]: its id, the text a `ReInfer`
/// pass would embed (for `procedural_skill` this is already
/// [`skill_embedding_text`]'s output — never rebuilt
/// by hand here), and the raw bytes currently sitting in the matching
/// vector row (what a `Normalize` pass renormalizes).
struct ReembedRow {
    id: i64,
    text: String,
    existing: Vec<u8>,
}

/// Two modes as types: which operation [`reembed_all`] performs against the
/// store's three vector tables.
///
/// `Normalize` carries no embedder and no identity — structurally, not by
/// convention, it cannot reach the arm in [`reembed_all`] that writes
/// `embedding_contract`. That makes the contract row a receipt readable
/// without opening this file: old prefixes in the row means no
/// re-inference ever ran. It is idempotent AND cheap — [`to_unit`] is
/// proven idempotent (`vector.rs::to_unit_is_idempotent`), so a repeated
/// pass writes byte-identical bytes.
///
/// `ReInfer` pays for real inference on every row. It is idempotent in
/// RESULT — the model is deterministic, so a re-run produces the same
/// bytes — but NOT in cost: a re-run pays for every row's inference again.
/// Because a failed pass leaves `embedding_contract` untouched (see
/// [`reembed_all`]'s failure semantics), re-running IS the recovery path,
/// and it converges: rows already written get the same bytes back, rows
/// that were skipped get done, and the contract flips only once all three
/// tables come back clean.
pub enum ReembedMode {
    Normalize,
    ReInfer(ReInferPlan),
}

/// The embedder driving a [`ReembedMode::ReInfer`] pass, plus the identity
/// that will be written to `embedding_contract` if the pass fully succeeds.
///
/// Both fields are PRIVATE and [`ReInferPlan::new`] is the only
/// constructor: it derives `identity` FROM `embedder` via
/// `EmbeddingIdentity::of`, so there is no way to construct a plan whose
/// `identity` describes a different embedder than the one that will
/// actually produce every vector. A public two-field variant would let a
/// caller pair embedder A with identity B and write a contract for vectors
/// that embedder never made.
pub struct ReInferPlan {
    embedder: Arc<dyn Embedder>,
    identity: EmbeddingIdentity,
}

impl ReInferPlan {
    pub fn new(embedder: Arc<dyn Embedder>) -> Self {
        let identity = EmbeddingIdentity::of(embedder.as_ref());
        Self { embedder, identity }
    }
}

/// Attempted/updated/failed/aborted counts for one table's pass. The unit
/// [`reembed_all`]'s caller reads to learn what actually happened, since
/// [`reembed_all`] itself never returns `Err`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TableOutcome {
    pub(crate) table: &'static str,
    pub(crate) attempted: usize,
    pub(crate) updated: usize,
    pub(crate) failed: usize,
    pub(crate) aborted: bool,
}

/// Outcome of a full [`reembed_all`] pass across all three tables.
#[derive(Debug, Clone, Serialize)]
pub struct ReembedReport {
    pub(crate) per_table: [TableOutcome; 3],
    pub(crate) contract_updated: bool,
}

/// Backfill for the three vector tables. The only automatic caller is the
/// pooling upgrade in [`crate::memory::bootstrap::verify_or_reembed`], a
/// mismatch this binary itself introduced and can name exactly; any other
/// mismatch stays the embedding contract's loud, safe stop (an empty cell,
/// an `error` event), because re-inferring the whole store whenever a config
/// changed would be a CPU-heavy migration nobody asked for.
///
/// Never returns `Err` — fail-soft like [`sweep_orphaned_vectors`]. Each
/// table is walked independently; a failure in one (a batch's `UPDATE`
/// transaction failing, which rolls the whole batch back and aborts the
/// REST of that table) does not stop the other two from being attempted.
/// [`ReembedReport`] is how the caller finds out what actually happened.
///
/// The `embedding_contract` row is written LAST, and only when `mode` is
/// `ReInfer` AND every table came back with `failed == 0` and
/// `aborted == false`. After a partial `ReInfer` failure the store holds
/// vectors from BOTH the old and the new embedding space, and no value
/// written to the contract can describe that truthfully — so the choice is
/// between two lies. Leaving the OLD prefixes in place produces a
/// `Mismatch` the next time the app boots with the new embedder: the cell
/// stays empty, memory is degraded, chat keeps working — the loud, safe
/// stop doing exactly its job. Writing the NEW prefixes would produce a
/// `Match`, and the store would silently serve half its rows out of a
/// space the active embedder never produced them in, forever, with
/// nothing left to detect it. The old value is the only one that stays
/// honest, so leaving the contract untouched on any failure is not an
/// omission — it is the whole point of writing it last.
pub async fn reembed_all(store: &MemoryStore, mode: ReembedMode) -> ReembedReport {
    let memory = reembed_table(store, &mode, ReembedTable::Memory).await;
    let episodic = reembed_table(store, &mode, ReembedTable::Episodic).await;
    let procedural = reembed_table(store, &mode, ReembedTable::Procedural).await;
    let per_table = [memory, episodic, procedural];

    let contract_updated = match &mode {
        ReembedMode::Normalize => false,
        ReembedMode::ReInfer(plan) => {
            let all_clean = per_table.iter().all(|t| t.failed == 0 && !t.aborted);
            if !all_clean {
                false
            } else {
                match write_embedding_contract(store, &plan.identity).await {
                    Ok(()) => true,
                    Err(e) => {
                        log::warn!(
                            "reembed_all: all 3 tables migrated cleanly but the contract write \
                             failed: {e} — embedding_contract still holds the OLD identity, so \
                             the next boot will report Mismatch rather than trust a migration \
                             that never got its receipt written"
                        );
                        false
                    }
                }
            }
        }
    };

    ReembedReport { per_table, contract_updated }
}

/// Walks one table to completion via keyset pagination. Three steps per
/// batch, in this order and no other: (1) SELECT the batch inside
/// `conn.call`, connection released immediately after; (2) OUTSIDE the
/// connection, compute every row's new vector (`embed_with_role` is async
/// and the writer connection's closure is sync, so inference cannot happen
/// inside `conn.call`); (3) one `conn.call` transaction that `UPDATE`s
/// every successfully-computed row and commits — the writer is held only
/// for SQL, and the batch is all-or-nothing.
async fn reembed_table(store: &MemoryStore, mode: &ReembedMode, table: ReembedTable) -> TableOutcome {
    let mut attempted = 0usize;
    let mut updated = 0usize;
    let mut failed = 0usize;
    let mut after_id: i64 = 0;

    loop {
        let batch = match select_reembed_batch(store, table, after_id, REEMBED_BATCH).await {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("reembed_all: batch select failed for {}: {e}", table.parent());
                return TableOutcome { table: table.parent(), attempted, updated, failed, aborted: true };
            }
        };
        if batch.is_empty() {
            break;
        }
        let batch_len = batch.len();
        after_id = batch.last().map(|r| r.id).unwrap_or(after_id);
        attempted += batch_len;

        // Outside the connection: one un-embeddable row must not deny every
        // other row in the batch its migration, so a per-row failure is
        // logged, counted, and left out of this batch's write — its old
        // vector stays exactly as it was.
        let mut writes = Vec::with_capacity(batch_len);
        for row in &batch {
            match compute_new_embedding(mode, row).await {
                Ok(v) => writes.push((row.id, serialize_f32_vec(&v))),
                Err(e) => {
                    log::warn!(
                        "reembed_all: embed failed for {} id={}: {e} — row keeps its old vector",
                        table.parent(),
                        row.id
                    );
                    failed += 1;
                }
            }
        }

        if !writes.is_empty() {
            let n = writes.len();
            match write_reembed_batch(store, table, writes).await {
                Ok(()) => updated += n,
                Err(e) => {
                    // A write failure here is about the STORE refusing a
                    // write, not about any one row's content — burning
                    // inference against more batches would not change
                    // that, so this table stops here while the other two
                    // still get their own independent attempt.
                    log::warn!("reembed_all: batch UPDATE failed for {}: {e}", table.parent());
                    return TableOutcome { table: table.parent(), attempted, updated, failed, aborted: true };
                }
            }
        }

        if batch_len < REEMBED_BATCH as usize {
            break;
        }
    }

    TableOutcome { table: table.parent(), attempted, updated, failed, aborted: false }
}

/// Computes one row's new vector. `ReInfer` re-runs inference — always
/// [`EmbedRole::Passage`], since every reembedded row is something searched
/// THROUGH, never a query — then [`to_unit`]s the result. `Normalize`
/// deserializes the vector already sitting in the store and [`to_unit`]s
/// THAT — no embedder call, which is also why `Normalize` cannot fail on
/// inference, only on a corrupt stored blob.
async fn compute_new_embedding(mode: &ReembedMode, row: &ReembedRow) -> Result<Vec<f32>, String> {
    match mode {
        ReembedMode::Normalize => {
            let existing = deserialize_f32_vec(&row.existing);
            to_unit(&existing).map_err(|e| e.to_string())
        }
        ReembedMode::ReInfer(plan) => {
            let raw = plan
                .embedder
                .embed_with_role(&row.text, EmbedRole::Passage)
                .await
                .map_err(|e| e.to_string())?;
            to_unit(&raw).map_err(|e| e.to_string())
        }
    }
}

/// One batch's SELECT, joined to its vector table so the currently-stored
/// bytes ride along with the text in a single point-lookup pass (same
/// `rowid = p.id` join shape as [`sweep_orphaned_vectors`]) — `Normalize`
/// needs those bytes and `ReInfer` needs the text, and fetching both
/// unconditionally is cheaper than a second round trip to find out which
/// one the caller actually wanted.
///
/// `procedural_skill` is the one table where the "text" isn't a single
/// column: it resolves `trigger`+`skill_name` through
/// [`skill_embedding_text`] instead of concatenating them here by hand.
async fn select_reembed_batch(
    store: &MemoryStore,
    table: ReembedTable,
    after_id: i64,
    limit: i64,
) -> Result<Vec<ReembedRow>, tokio_rusqlite::Error> {
    store
        .conn
        .call(move |c| -> rusqlite::Result<Vec<ReembedRow>> {
            match table {
                ReembedTable::Procedural => {
                    let mut stmt = c.prepare(
                        "SELECT p.id, p.trigger, p.skill_name, v.embedding \
                         FROM procedural_skill p JOIN procedural_vectors v ON v.rowid = p.id \
                         WHERE p.deleted_at IS NULL AND p.id > ?1 ORDER BY p.id LIMIT ?2",
                    )?;
                    let rows = stmt.query_map(rusqlite::params![after_id, limit], |r| {
                        let id: i64 = r.get(0)?;
                        let trigger: String = r.get(1)?;
                        let skill_name: String = r.get(2)?;
                        let existing: Vec<u8> = r.get(3)?;
                        Ok(ReembedRow {
                            id,
                            text: skill_embedding_text(&trigger, &skill_name),
                            existing,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok(rows)
                }
                ReembedTable::Memory => {
                    // A piece of a long save was embedded behind the save's
                    // head; its link keeps that head so the vector rebuilt
                    // here is the one the piece was saved with.
                    let mut stmt = c.prepare(
                        "SELECT p.id, p.content, v.embedding, \
                                (SELECT mp.context FROM memory_piece mp \
                                 WHERE mp.memory_id = p.id AND mp.context <> '' \
                                 ORDER BY mp.save_id LIMIT 1) \
                         FROM memory p JOIN memory_vectors v ON v.rowid = p.id \
                         WHERE p.deleted_at IS NULL AND p.id > ?1 ORDER BY p.id LIMIT ?2",
                    )?;
                    let rows = stmt
                        .query_map(rusqlite::params![after_id, limit], |r| {
                            let id: i64 = r.get(0)?;
                            let content: String = r.get(1)?;
                            let existing: Vec<u8> = r.get(2)?;
                            let context: Option<String> = r.get(3)?;
                            let text = embedding_text(context.as_deref().unwrap_or(""), &content).into_owned();
                            Ok(ReembedRow { id, text, existing })
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok(rows)
                }
                ReembedTable::Episodic => {
                    let mut stmt = c.prepare(
                        "SELECT p.id, p.task, v.embedding FROM episodic_trajectory p \
                         JOIN episodic_vectors v ON v.rowid = p.id \
                         WHERE p.deleted_at IS NULL AND p.id > ?1 ORDER BY p.id LIMIT ?2",
                    )?;
                    let rows = stmt
                        .query_map(rusqlite::params![after_id, limit], |r| {
                            let id: i64 = r.get(0)?;
                            let text: String = r.get(1)?;
                            let existing: Vec<u8> = r.get(2)?;
                            Ok(ReembedRow { id, text, existing })
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok(rows)
                }
            }
        })
        .await
}

/// One batch's write: a single transaction, one `UPDATE ... WHERE rowid = ?`
/// per row, then commit. vec0 in-place `UPDATE` is correct for this
/// schema — verified in vendored `sqlite-vec.c` (`vec0Update` →
/// `vec0Update_Update` → `vec0Update_UpdateVectorColumn`); `V2__vectors.sql`
/// declares only `embedding float[384]` with no partition/auxiliary/metadata
/// columns, so the "not supported yet" rejection those functions guard
/// against is unreachable against this schema. No DELETE+INSERT dance, and
/// no `rowid IN (subquery)` either — a bare `rowid = ?` is the point-lookup
/// plan the vector virtual table supports directly.
async fn write_reembed_batch(
    store: &MemoryStore,
    table: ReembedTable,
    updates: Vec<(i64, Vec<u8>)>,
) -> Result<(), tokio_rusqlite::Error> {
    let vectors = table.vectors();
    let sql = format!("UPDATE {vectors} SET embedding = ?1 WHERE rowid = ?2");
    store
        .conn
        .call(move |c| -> rusqlite::Result<()> {
            let tx = c.transaction()?;
            for (id, bytes) in &updates {
                tx.execute(&sql, rusqlite::params![bytes, id])?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
}

/// Unconditionally writes `identity` to the single-row `embedding_contract`
/// table — an UPSERT, unlike [`MemoryStore::ensure_embedding_contract`]'s
/// write-if-absent/verify-if-present semantics. By the time `reembed_all`
/// reaches this call a row already exists (the app booted with SOME
/// embedder), and the whole point is to REPLACE it with the identity that
/// just finished producing every vector in the store.
async fn write_embedding_contract(
    store: &MemoryStore,
    identity: &EmbeddingIdentity,
) -> Result<(), tokio_rusqlite::Error> {
    let identity = identity.clone();
    store
        .conn
        .call(move |c| -> rusqlite::Result<()> {
            let created_at = crate::memory::clock::TimestampMs::now();
            c.execute(
                "INSERT INTO embedding_contract \
                 (id, model_id, dim, metric, normalized, query_prefix, passage_prefix, created_at) \
                 VALUES (1, ?1, ?2, 'l2_unit', 1, ?3, ?4, ?5) \
                 ON CONFLICT(id) DO UPDATE SET \
                   model_id = excluded.model_id, \
                   dim = excluded.dim, \
                   query_prefix = excluded.query_prefix, \
                   passage_prefix = excluded.passage_prefix, \
                   created_at = excluded.created_at",
                rusqlite::params![
                    identity.model_id,
                    identity.dim as i64,
                    identity.query_prefix,
                    identity.passage_prefix,
                    created_at,
                ],
            )?;
            Ok(())
        })
        .await
}

/// Little-endian `f32` -> bytes, matching every vector column's encoding in
/// this schema (`store_insert.rs`'s producer, `db.rs`'s test-side reader).
fn serialize_f32_vec(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for &f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Inverse of [`serialize_f32_vec`].
fn deserialize_f32_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::memory::clock::TimestampMs;
    use crate::memory::db::MemoryStore;
    use crate::memory::embedder::{EmbedRole, Embedder};
    use std::sync::atomic::{AtomicI64, Ordering};

    // ── Test helpers ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_maintenance_pass_is_an_event_with_what_it_did() {
        let store = MemoryStore::new_for_test().await.unwrap();
        super::run_maintenance_once(&store).await;
        let events = store.metrics().take_events();
        assert_eq!(events.iter().map(|e| e.kind).collect::<Vec<_>>(), [crate::metrics::EventKind::Maintenance]);
        let keys: Vec<&str> = events[0].detail.keys().copied().collect();
        assert_eq!(keys, ["classified", "dropped_pairs", "ms", "released", "revived", "swept"]);
    }

    /// Current wall-clock time in milliseconds since UNIX epoch — same
    /// formula used in production (`run_maintenance_once`, `access.rs::flush`).
    fn now_ms() -> TimestampMs {
        TimestampMs::now()
    }

    /// Monotonic counter to keep `episodic_trajectory.session_id` (UNIQUE)
    /// distinct across seeded rows within a single test.
    static NONCE: AtomicI64 = AtomicI64::new(0);

    /// Seeds one row in `table` ("memory" | "episodic_trajectory" |
    /// "procedural_skill") with the given `tier`/`importance`, and
    /// `last_accessed_at = now + offset_ms` (so a NEGATIVE `offset_ms`
    /// places the row that many milliseconds in the PAST — matching the
    /// boundary offsets, e.g. `-7_776_000_001` for "just over
    /// 90 days ago"). `now` is passed in (not read here) so a test can run
    /// `run_maintenance_once_at` against the SAME instant it seeded with —
    /// the 1ms-under-boundary offsets are racy otherwise.
    /// `last_modified_at`/`created_at` are seeded at `0` so tests can
    /// assert "untouched" by checking they stay `0`.
    async fn seed_row(store: &MemoryStore, table: &str, tier: i64, importance: f64, now: TimestampMs, offset_ms: i64) -> i64 {
        // Adding a millisecond offset to an instant leaves the type, because the
        // instant deliberately exposes no arithmetic — every value enters it through a
        // named constructor so the unit is written at each construction site.
        let last_accessed_at = now.as_millis_i64() + offset_ms;
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

    /// Seeds one row already soft-deleted (`deleted_at = 1`), plus its
    /// vector row, via a raw transaction that bypasses `delete_memory_core`
    /// entirely — the vector is never pruned, exactly like a row that was
    /// soft-deleted before vector pruning existed. Returns the parent row's
    /// id; the vector table's rowid is the same value.
    async fn seed_orphaned_deleted_row(store: &MemoryStore, table: &str) -> i64 {
        let nonce = NONCE.fetch_add(1, Ordering::SeqCst);
        let table = table.to_string();
        let embedding = vec![0u8; 384 * 4];
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                let tx = c.transaction()?;
                let (vectors_table, id) = match table.as_str() {
                    "memory" => {
                        tx.execute(
                            "INSERT INTO memory \
                             (scope, agent_id, content, memory_type, tier, importance, \
                              created_at, last_modified_at, last_accessed_at, \
                              created_by_type, created_by_ref, deleted_at) \
                             VALUES ('agent', 'test-agent', 'seed content', 'chat_message', \
                                     3, 5.0, 0, 0, 0, 'agent', '', 1)",
                            [],
                        )?;
                        ("memory_vectors", tx.last_insert_rowid())
                    }
                    "episodic_trajectory" => {
                        tx.execute(
                            "INSERT INTO episodic_trajectory \
                             (scope, agent_id, session_id, task, tier, importance, \
                              created_at, last_modified_at, last_accessed_at, \
                              created_by_type, created_by_ref, deleted_at) \
                             VALUES ('agent', 'test-agent', ?1, 'seed task', \
                                     3, 5.0, 0, 0, 0, 'agent', '', 1)",
                            rusqlite::params![format!("sess-sweep-{nonce}")],
                        )?;
                        ("episodic_vectors", tx.last_insert_rowid())
                    }
                    "procedural_skill" => {
                        tx.execute(
                            "INSERT INTO procedural_skill \
                             (scope, agent_id, skill_name, trigger, tier, importance, \
                              promoted_at, source_trajectory_ids, \
                              created_at, last_modified_at, last_accessed_at, \
                              created_by_type, created_by_ref, deleted_at) \
                             VALUES ('agent', 'test-agent', ?1, 'seed trigger', \
                                     1, 8.0, 0, '[]', 0, 0, 0, 'agent', '', 1)",
                            rusqlite::params![format!("skill-sweep-{nonce}")],
                        )?;
                        ("procedural_vectors", tx.last_insert_rowid())
                    }
                    other => panic!("seed_orphaned_deleted_row: unknown table '{other}'"),
                };
                tx.execute(
                    &format!("INSERT INTO {vectors_table}(rowid, embedding) VALUES (?, ?)"),
                    rusqlite::params![id, embedding],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .expect("seed_orphaned_deleted_row failed")
    }

    async fn vector_row_count(store: &MemoryStore, vectors_table: &'static str, id: i64) -> i64 {
        let sql = format!("SELECT COUNT(*) FROM {vectors_table} WHERE rowid = ?1");
        store
            .conn
            .call(move |c| c.query_row(&sql, [id], |r| r.get(0)))
            .await
            .unwrap()
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

    /// Polls `tier_of(store, "memory", id)` until it equals `want`, bounded by
    /// `budget` of REAL wall-clock time (`std::time::Instant`, never
    /// `tokio::time`). These loop tests run under a PAUSED tokio clock
    /// (`#[tokio::test(start_paused = true)]`); `tokio::time::sleep` auto-advances
    /// that virtual clock while paused, which would fire additional scheduled
    /// `interval` ticks the test is deliberately trying to hold still — so this
    /// polls with `tokio::task::yield_now()` (never touches the clock) and times
    /// the budget against the OS clock instead. `run_maintenance_loop` drives its
    /// pass through `store.conn` (`tokio_rusqlite`), which dispatches to a real
    /// background OS thread — a fixed `yield_now` count is not a synchronization
    /// primitive for that hop, so this bounds the wait instead of guessing a
    /// count.
    async fn await_tier(
        store: &MemoryStore,
        id: i64,
        want: i64,
        budget: std::time::Duration,
    ) -> Result<(), String> {
        let deadline = std::time::Instant::now() + budget;
        loop {
            let got = tier_of(store, "memory", id).await;
            if got == want {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!(
                    "tier stayed at {got}, expected {want}, after {budget:?} of real time"
                ));
            }
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn tier1_low_importance_over_90_days_demotes_to_2() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let now = now_ms();
        let id = seed_row(&store, "memory", 1, 5.0, now, -7_776_000_001).await;

        super::run_maintenance_once_at(&store, now).await;

        assert_eq!(tier_of(&store, "memory", id).await, 2);
    }

    #[tokio::test]
    async fn tier1_low_importance_under_90_days_unchanged_last_modified_intact() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let now = now_ms();
        // 90 days minus exactly 1ms — deterministic ONLY because the pass
        // below runs against the same `now` the row was seeded with.
        let id = seed_row(&store, "memory", 1, 5.0, now, -7_775_999_999).await;

        super::run_maintenance_once_at(&store, now).await;

        assert_eq!(tier_of(&store, "memory", id).await, 1, "must NOT demote before 90 days");
        assert_eq!(
            last_modified_at_of(&store, "memory", id).await,
            0,
            "last_modified_at must stay untouched (0) when the guard skips the row"
        );
    }

    #[tokio::test]
    async fn tier2_over_180_days_demotes_to_3() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let now = now_ms();
        let id = seed_row(&store, "memory", 2, 5.0, now, -15_552_000_001).await;

        super::run_maintenance_once_at(&store, now).await;

        assert_eq!(tier_of(&store, "memory", id).await, 3);
    }

    // ── tier2, <180 days → unchanged ─────────────────────────────────────────

    #[tokio::test]
    async fn tier2_under_180_days_unchanged() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let now = now_ms();
        // 180 days minus exactly 1ms — same shared-now discipline as the
        // 90-day under-boundary test above.
        let id = seed_row(&store, "memory", 2, 5.0, now, -15_551_999_999).await;

        super::run_maintenance_once_at(&store, now).await;

        assert_eq!(tier_of(&store, "memory", id).await, 2, "must NOT demote before 180 days");
    }

    #[tokio::test]
    async fn tier1_pivotal_importance_never_demotes() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let now = now_ms();
        let id = seed_row(&store, "memory", 1, 9.5, now, -31_536_000_000).await;

        super::run_maintenance_once_at(&store, now).await;

        assert_eq!(tier_of(&store, "memory", id).await, 1, "importance >= 9.0 must never demote");
    }

    #[tokio::test]
    async fn tier3_row_untouched() {
        let store = MemoryStore::new_for_test().await.unwrap();
        // Same offset as the tier2->3 case — if the WHERE tier<3 guard were
        // missing this would still be a no-op (demote_inactive(3, ..) = 3
        // unconditionally), so this test specifically pins tier staying 3
        // AND last_modified_at staying untouched (proving the row was never
        // part of the UPDATE's matched set).
        let id = seed_row(&store, "memory", 3, 5.0, now_ms(), -15_552_000_001).await;

        super::run_maintenance_once(&store).await;

        assert_eq!(tier_of(&store, "memory", id).await, 3);
        assert_eq!(last_modified_at_of(&store, "memory", id).await, 0);
    }

    #[tokio::test]
    async fn inactivity_demotion_covers_memory_and_procedural_skills() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let now = now_ms();
        let mem_id = seed_row(&store, "memory", 1, 5.0, now, -7_776_000_001).await;
        let proc_id = seed_row(&store, "procedural_skill", 1, 5.0, now, -7_776_000_001).await;

        super::run_maintenance_once_at(&store, now).await;

        assert_eq!(tier_of(&store, "memory", mem_id).await, 2, "memory row must demote");
        assert_eq!(
            tier_of(&store, "procedural_skill", proc_id).await,
            2,
            "a skill nobody has reached for in 90 days must lose its tier-1 floor \
             like any other stale row"
        );
    }

    /// A skill as the desktop app wrote it lands at tier 1 with importance 8.0 — under
    /// the 9.0 pivotal cutoff — so the pass reaches the DEFAULT skill, not just
    /// a hand-seeded low-importance one. Without this, the test above could pass
    /// on a row shape production never writes.
    #[tokio::test]
    async fn default_importance_skill_still_demotes_after_90_days() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let now = now_ms();
        let id = seed_row(&store, "procedural_skill", 1, 8.0, now, -7_776_000_001).await;

        super::run_maintenance_once_at(&store, now).await;

        assert_eq!(tier_of(&store, "procedural_skill", id).await, 2);
    }

    #[tokio::test]
    async fn procedural_skill_under_90_days_is_not_demoted() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let now = now_ms();
        let id = seed_row(&store, "procedural_skill", 1, 5.0, now, -7_775_999_999).await;

        super::run_maintenance_once_at(&store, now).await;

        assert_eq!(tier_of(&store, "procedural_skill", id).await, 1, "must NOT demote before 90 days");
        assert_eq!(
            last_modified_at_of(&store, "procedural_skill", id).await,
            0,
            "last_modified_at must stay untouched (0) when the guard skips the row"
        );
    }

    #[tokio::test]
    async fn stale_episodic_trajectory_is_demoted() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let now = now_ms();
        let id = seed_row(&store, "episodic_trajectory", 1, 5.0, now, -7_776_000_001).await;

        super::run_maintenance_once_at(&store, now).await;

        assert_eq!(tier_of(&store, "episodic_trajectory", id).await, 2);
    }

    /// Why the episodic arm of the pass finds nothing on today's corpus, and
    /// why it is still there.
    ///
    /// `compute_tier` fast-tracks anything at importance >= 9.0 to tier 1 before
    /// it ever looks at the memory type, so a trajectory is NOT structurally
    /// pinned to tier 3 — it sits there only because the writer hands the insert
    /// a hardcoded 5.0. That literal is one edit away from putting trajectories
    /// on a tier that decays, and this test is what makes such an edit announce
    /// itself instead of silently creating a row that promotes and never falls
    /// back.
    #[test]
    fn trajectory_importance_is_the_only_thing_keeping_it_off_tier_1() {
        use crate::memory::tier::compute_tier;

        assert_eq!(
            compute_tier(3, 5.0, 0, "episodic_trajectory"),
            3,
            "the importance the writer actually passes must keep trajectories at tier 3"
        );
        assert_eq!(
            compute_tier(3, 9.0, 0, "episodic_trajectory"),
            1,
            "raising that importance past the pivotal cutoff WOULD put a trajectory \
             on tier 1 — which is why the demotion pass covers this table"
        );
    }

    use std::sync::Arc;
    use std::time::Duration;

    // interval_at's first tick fires at now+startup_delay, NOT at t=0:
    // asserts NO change before any clock advance (anti false-green), then
    // asserts the first pass DOES fire once startup_delay elapses.
    #[tokio::test(start_paused = true)]
    async fn loop_does_not_fire_before_startup_delay_then_fires_first_pass_after_delay() {
        let store = MemoryStore::new_for_test().await.unwrap();
        // The loop reads the real wall clock per pass, so the seed uses a
        // fresh now_ms(): the over-boundary offset only grows staler as
        // real time advances — safe direction, no shared-now needed.
        let id = seed_row(&store, "memory", 1, 5.0, now_ms(), -7_776_000_001).await;
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
        await_tier(&store, id, 2, Duration::from_secs(5))
            .await
            .expect("first pass must fire once startup_delay elapses");

        handle.abort();
    }

    // A second pass fires ONLY after a full `period` has elapsed since the
    // first tick, not before.
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
        let id = seed_row(&store, "memory", 1, 5.0, now_ms(), -15_552_000_001).await;
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
        // Polling until tier==2 here is still sound as an anti-false-green
        // check for tick 2: the virtual clock only sits at t=32s (advance()
        // moved it, yield_now never does), so tick 2 — scheduled at t=61s —
        // cannot structurally have fired yet, regardless of how long this
        // poll takes in real time.
        tokio::time::advance(Duration::from_secs(32)).await;
        await_tier(&store, id, 2, Duration::from_secs(5))
            .await
            .expect("only tick 1 must have fired by t=32s (tier 1→2), tick 2 (t=61s) must NOT fire yet");

        // Complete the period (total elapsed since spawn now > 61s) → tick 2
        // fires → tier 2→3.
        tokio::time::advance(Duration::from_secs(35)).await;
        await_tier(&store, id, 3, Duration::from_secs(5))
            .await
            .expect("second pass must fire once the full period elapses");

        handle.abort();
    }


    // ── count_rows ────────────────────────────────────────────────────────
    //
    // Asserts the struct directly, matching the pure-formatter convention
    // used elsewhere in this codebase (e.g. `format_tier_distribution`) — no
    // log capture needed.

    #[tokio::test]
    async fn count_rows_reports_live_and_total_separately() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let now = now_ms();

        // 2 memory rows, one soft-deleted -> total 2, live 1.
        let live_id = seed_row(&store, "memory", 3, 5.0, now, 0).await;
        let deleted_id = seed_row(&store, "memory", 3, 5.0, now, 0).await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE memory SET deleted_at = 1 WHERE id = ?1",
                    rusqlite::params![deleted_id],
                )
            })
            .await
            .unwrap();
        let _ = live_id;

        // 1 episodic row, not deleted -> total 1, live 1.
        seed_row(&store, "episodic_trajectory", 3, 5.0, now, 0).await;

        // 0 procedural rows -> total 0, live 0.

        let counts = super::count_rows(&store).await.unwrap();

        assert_eq!((counts.memory.total, counts.memory.live), (2, 1));
        assert_eq!((counts.episodic.total, counts.episodic.live), (1, 1));
        assert_eq!((counts.procedural.total, counts.procedural.live), (0, 0));
    }

    // ── orphaned-vector sweep ─────────────────────────────────────────────

    #[tokio::test]
    async fn maintenance_sweeps_orphan_vectors() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let mem_id = seed_orphaned_deleted_row(&store, "memory").await;
        let epi_id = seed_orphaned_deleted_row(&store, "episodic_trajectory").await;
        let proc_id = seed_orphaned_deleted_row(&store, "procedural_skill").await;

        assert_eq!(vector_row_count(&store, "memory_vectors", mem_id).await, 1, "test setup: orphan must exist before the sweep");

        super::run_maintenance_once_at(&store, now_ms()).await;

        assert_eq!(vector_row_count(&store, "memory_vectors", mem_id).await, 0, "memory vector orphan must be swept");
        assert_eq!(vector_row_count(&store, "episodic_vectors", epi_id).await, 0, "episodic vector orphan must be swept");
        assert_eq!(vector_row_count(&store, "procedural_vectors", proc_id).await, 0, "procedural vector orphan must be swept");
    }

    #[tokio::test]
    async fn maintenance_sweep_idempotent() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_orphaned_deleted_row(&store, "memory").await;

        let first = super::sweep_orphaned_vectors(&store).await;
        assert_eq!(first, 1, "first pass must find and drop the one orphan");
        assert_eq!(vector_row_count(&store, "memory_vectors", id).await, 0, "first pass must sweep the orphan");

        // The parent row is still there and still soft-deleted — nothing in
        // this codebase hard-deletes it. So "no work left" cannot mean "no
        // soft-deleted rows left"; it has to mean the second pass finds no
        // vector to drop and issues no statement at all. Asserting the count
        // stayed 0 would not distinguish that from re-issuing a delete for
        // every tombstone the table has ever accumulated, which is the same
        // observable state at a cost that grows forever.
        let second = super::sweep_orphaned_vectors(&store).await;
        assert_eq!(second, 0, "an already-swept corpus must give the second pass nothing to do");
        assert_eq!(vector_row_count(&store, "memory_vectors", id).await, 0, "second pass must stay a no-op, not error or re-delete");
    }

    // ── orphaned-dispute release ──────────────────────────────────────────

    /// A row as the detector writes it: `disputed`, naming `against`.
    async fn seed_dispute(store: &MemoryStore, content: &str, status: &str, against: Option<i64>) -> i64 {
        let (content, status) = (content.to_string(), status.to_string());
        let against = against.map(|id| format!("[{id}]"));
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source, created_at, \
                     created_by_type, created_by_ref, last_modified_at, last_accessed_at, importance, \
                     tier, status, disputed_against, last_disputed_at) \
                     VALUES ('agent', 'agent-m', ?1, 'semantic_fact', 'explicit', 0, 'user', 'agent-m', \
                     0, 0, 5.0, 3, ?2, ?3, CASE WHEN ?3 IS NULL THEN NULL ELSE 0 END)",
                    rusqlite::params![content, status, against],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    async fn status_of(store: &MemoryStore, id: i64) -> (String, i64) {
        store
            .conn
            .call(move |c| {
                c.query_row("SELECT status, last_modified_at FROM memory WHERE id = ?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn maintenance_releases_disputes_left_by_an_earlier_delete() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let gone = seed_dispute(&store, "gone", "active", None).await;
        let orphan = seed_dispute(&store, "orphan", "disputed", Some(gone)).await;
        let live = seed_dispute(&store, "live", "active", None).await;
        let open = seed_dispute(&store, "open", "disputed", Some(live)).await;
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET deleted_at = 1 WHERE id = ?1", [gone]))
            .await
            .unwrap();

        super::run_maintenance_once_at(&store, now_ms()).await;

        assert_eq!(status_of(&store, orphan).await.0, "active");
        assert_eq!(status_of(&store, open).await, ("disputed".to_string(), 0), "an open dispute is not touched");
    }

    #[tokio::test]
    async fn maintenance_release_is_idempotent() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let gone = seed_dispute(&store, "gone", "active", None).await;
        seed_dispute(&store, "orphan", "disputed", Some(gone)).await;
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET deleted_at = 1 WHERE id = ?1", [gone]))
            .await
            .unwrap();

        assert_eq!(super::release_orphaned_disputes(&store, now_ms()).await, 1);
        assert_eq!(super::release_orphaned_disputes(&store, now_ms()).await, 0);
    }

    #[tokio::test]
    async fn maintenance_revives_what_an_earlier_delete_left_hidden() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v1 = seed_dispute(&store, "storage engine is sqlite", "active", None).await;
        let v2 = seed_dispute(&store, "storage engine is postgres", "active", None).await;
        let loser = seed_dispute(&store, "the api listens on 8080", "active", None).await;
        let winner = seed_dispute(&store, "the api listens on 9090", "active", None).await;
        // What a delete by the previous version left: the current version of
        // a topic deleted, the one it replaced still superseded by it.
        store
            .conn
            .call(move |c| {
                c.execute("UPDATE memory SET topic_key = 'storage/engine' WHERE id IN (?1, ?2)", [v1, v2])?;
                c.execute(
                    "UPDATE memory SET status = 'superseded', superseded_by = ?2, supersede_cause = 'revision' WHERE id = ?1",
                    [v1, v2],
                )?;
                c.execute(
                    "UPDATE memory SET status = 'superseded', superseded_by = ?2, supersede_cause = 'verdict' WHERE id = ?1",
                    [loser, winner],
                )?;
                c.execute("UPDATE memory SET deleted_at = 1 WHERE id IN (?1, ?2)", [v2, winner])
            })
            .await
            .unwrap();

        assert_eq!(super::revive_hidden_revisions(&store, now_ms()).await, 1);
        assert_eq!(super::revive_hidden_revisions(&store, now_ms()).await, 0, "idempotent");

        let revived: (String, Option<i64>) = store
            .conn
            .call(move |c| c.query_row("SELECT status, revived_from FROM memory WHERE id = ?1", [v1], |r| Ok((r.get(0)?, r.get(1)?))))
            .await
            .unwrap();
        assert_eq!(revived, ("active".to_string(), Some(v2)));
        assert_eq!(status_of(&store, loser).await.0, "superseded", "a verdict stands");
    }

    #[tokio::test]
    async fn maintenance_drops_pairs_of_missing_rows() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let old = seed_dispute(&store, "old", "active", None).await;
        let new = seed_dispute(&store, "new", "active", None).await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory_conflict (new_id, old_id, distance, state, created_at) \
                     VALUES (?1, ?2, 0.3, 'pending', 0), (?3, ?1, 0.3, 'pending', 0), (?1, 999999, 0.3, 'pending', 0)",
                    [new, old, new + 1000],
                )
            })
            .await
            .unwrap();

        super::run_maintenance_once_at(&store, now_ms()).await;

        let left: Vec<(i64, i64)> = store
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT new_id, old_id FROM memory_conflict")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect();
                rows
            })
            .await
            .unwrap();
        assert_eq!(left, vec![(new, old)]);
        assert_eq!(super::drop_orphaned_pairs(&store).await, 0, "a second pass finds nothing");
    }

    // ── reembed_all ──────────────────────────────────────────────────────

    async fn seed_reembed_memory(store: &MemoryStore, content: &str, raw_vector: Vec<f32>) -> i64 {
        let bytes = super::serialize_f32_vec(&raw_vector);
        let content = content.to_string();
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, tier, importance, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', 'test-agent', ?1, 'chat_message', 3, 5.0, 0, 0, 0, 'agent', '')",
                    rusqlite::params![content],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                    rusqlite::params![id, bytes],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .expect("seed_reembed_memory failed")
    }

    async fn seed_reembed_episodic(store: &MemoryStore, task: &str, raw_vector: Vec<f32>) -> i64 {
        let nonce = NONCE.fetch_add(1, Ordering::SeqCst);
        let bytes = super::serialize_f32_vec(&raw_vector);
        let task = task.to_string();
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO episodic_trajectory \
                     (scope, agent_id, session_id, task, tier, importance, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', 'test-agent', ?1, ?2, 3, 5.0, 0, 0, 0, 'agent', '')",
                    rusqlite::params![format!("sess-reembed-{nonce}"), task],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO episodic_vectors(rowid, embedding) VALUES (?, ?)",
                    rusqlite::params![id, bytes],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .expect("seed_reembed_episodic failed")
    }

    async fn seed_reembed_procedural(
        store: &MemoryStore,
        trigger: &str,
        skill_name: &str,
        raw_vector: Vec<f32>,
    ) -> i64 {
        let nonce = NONCE.fetch_add(1, Ordering::SeqCst);
        let bytes = super::serialize_f32_vec(&raw_vector);
        let trigger = trigger.to_string();
        let skill_name = skill_name.to_string();
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO procedural_skill \
                     (scope, agent_id, skill_name, trigger, tier, importance, \
                      promoted_at, source_trajectory_ids, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', 'test-agent', ?1, ?2, 3, 5.0, 0, '[]', 0, 0, 0, 'agent', ?3)",
                    rusqlite::params![skill_name, trigger, format!("t-{nonce}")],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO procedural_vectors(rowid, embedding) VALUES (?, ?)",
                    rusqlite::params![id, bytes],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .expect("seed_reembed_procedural failed")
    }

    async fn read_vector_bytes(store: &MemoryStore, vectors_table: &'static str, id: i64) -> Vec<u8> {
        let sql = format!("SELECT embedding FROM {vectors_table} WHERE rowid = ?1");
        store.conn.call(move |c| c.query_row(&sql, [id], |r| r.get(0))).await.unwrap()
    }

    async fn read_contract_row(store: &MemoryStore) -> Option<(String, i64, String, String)> {
        use rusqlite::OptionalExtension;
        store
            .conn
            .call(|c| -> rusqlite::Result<Option<(String, i64, String, String)>> {
                c.query_row(
                    "SELECT model_id, dim, query_prefix, passage_prefix FROM embedding_contract WHERE id = 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()
            })
            .await
            .unwrap()
    }

    /// Records `(role, text)` for every call. Used to pin which role `reembed_all` uses per table, and to
    /// pin that `procedural_skill` embeds `skill_embedding_text(trigger,
    /// skill_name)` rather than a hand-rebuilt string.
    struct RecordingEmbedder {
        calls: std::sync::Mutex<Vec<(EmbedRole, String)>>,
    }

    impl RecordingEmbedder {
        fn new() -> Self {
            Self { calls: std::sync::Mutex::new(Vec::new()) }
        }
    }

    #[async_trait::async_trait]
    impl Embedder for RecordingEmbedder {
        async fn embed_with_role(
            &self,
            text: &str,
            role: EmbedRole,
        ) -> Result<Vec<f32>, crate::memory::embedder::EmbedError> {
            self.calls.lock().unwrap().push((role, text.to_string()));
            Ok(vec![1.0f32; 384])
        }
        fn dimensions(&self) -> usize {
            384
        }
        fn model_id(&self) -> &str {
            "recording-embedder"
        }
        fn query_prefix(&self) -> &str {
            ""
        }
        fn passage_prefix(&self) -> &str {
            ""
        }
    }

    /// Fails on exactly the Nth call (1-indexed), succeeds deterministically
    /// on every other call — the seam for the "one row's embed fails, batch
    /// continues" failure-semantics test.
    struct FlakyEmbedder {
        fail_on_call: usize,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Embedder for FlakyEmbedder {
        async fn embed_with_role(
            &self,
            text: &str,
            _role: EmbedRole,
        ) -> Result<Vec<f32>, crate::memory::embedder::EmbedError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if n == self.fail_on_call {
                return Err(crate::memory::embedder::EmbedError::InferenceFailed(
                    "simulated failure".into(),
                ));
            }
            Ok(vec![(text.len() as f32) + 1.0; 384])
        }
        fn dimensions(&self) -> usize {
            384
        }
        fn model_id(&self) -> &str {
            "flaky-embedder"
        }
        fn query_prefix(&self) -> &str {
            ""
        }
        fn passage_prefix(&self) -> &str {
            ""
        }
    }

    #[tokio::test]
    async fn reembed_normalize_leaves_contract_row_byte_identical() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .ensure_embedding_contract(crate::memory::db::EmbeddingIdentity {
                model_id: "multilingual-e5-small".into(),
                dim: 384,
                query_prefix: "query: ".into(),
                passage_prefix: "passage: ".into(),
            })
            .await
            .unwrap();
        let before = read_contract_row(&store).await;

        let report = super::reembed_all(&store, super::ReembedMode::Normalize).await;

        let after = read_contract_row(&store).await;
        assert_eq!(before, after, "Normalize must never touch the embedding_contract row");
        assert!(!report.contract_updated, "Normalize's report must say contract_updated=false");
    }

    #[tokio::test]
    async fn reembed_normalize_renormalizes_and_is_idempotent() {
        let store = MemoryStore::new_for_test().await.unwrap();
        // Deliberately non-unit: ||v|| = sqrt(384 * 4) ≈ 39.2.
        let raw: Vec<f32> = vec![2.0; 384];
        let id = seed_reembed_memory(&store, "needs renormalizing", raw).await;

        super::reembed_all(&store, super::ReembedMode::Normalize).await;
        let once = read_vector_bytes(&store, "memory_vectors", id).await;
        let once_f = super::deserialize_f32_vec(&once);
        let norm: f32 = once_f.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "Normalize must renormalize to unit length, got norm={norm}");

        super::reembed_all(&store, super::ReembedMode::Normalize).await;
        let twice = read_vector_bytes(&store, "memory_vectors", id).await;
        assert_eq!(once, twice, "a second Normalize pass must write byte-identical bytes (idempotent)");
    }

    #[tokio::test]
    async fn reembed_rebuilds_a_pieces_context() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let first = seed_reembed_memory(&store, "Notes on clawg-ledger. The first piece", vec![0.3; 384]).await;
        let second = seed_reembed_memory(&store, "the second piece", vec![0.3; 384]).await;
        let absorbed = seed_reembed_memory(&store, "a row an earlier save held", vec![0.3; 384]).await;
        store
            .link_pieces(vec![
                crate::memory::pieces::StoredPiece { memory_id: first, context: String::new() },
                crate::memory::pieces::StoredPiece { memory_id: second, context: "Notes on clawg-ledger.".into() },
                crate::memory::pieces::StoredPiece { memory_id: absorbed, context: String::new() },
            ])
            .await
            .unwrap();

        let recorder = std::sync::Arc::new(RecordingEmbedder::new());
        let embedder: std::sync::Arc<dyn Embedder> = recorder.clone();
        super::reembed_all(&store, super::ReembedMode::ReInfer(super::ReInferPlan::new(embedder))).await;

        let calls = recorder.calls.lock().unwrap();
        let texts: Vec<&str> = calls.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "Notes on clawg-ledger. The first piece",
                "Notes on clawg-ledger.\n…\nthe second piece",
                "a row an earlier save held",
            ]
        );
    }

    #[tokio::test]
    async fn reembed_reinfer_uses_passage_role_and_shared_skill_embedding_text() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_reembed_memory(&store, "hello world", vec![0.3; 384]).await;
        seed_reembed_episodic(&store, "fix the bug", vec![0.3; 384]).await;
        seed_reembed_procedural(&store, "when login fails", "Fix login flow", vec![0.3; 384]).await;

        let recorder = std::sync::Arc::new(RecordingEmbedder::new());
        let embedder: std::sync::Arc<dyn Embedder> = recorder.clone();
        let plan = super::ReInferPlan::new(embedder);

        super::reembed_all(&store, super::ReembedMode::ReInfer(plan)).await;

        let calls = recorder.calls.lock().unwrap();
        assert_eq!(calls.len(), 3, "expected exactly one embed call per seeded row");
        for (role, _) in calls.iter() {
            assert_eq!(*role, EmbedRole::Passage, "reembed_all must always embed with role Passage");
        }
        let texts: std::collections::HashSet<&str> = calls.iter().map(|(_, t)| t.as_str()).collect();
        assert!(texts.contains("hello world"), "memory.content must reach the embedder verbatim");
        assert!(texts.contains("fix the bug"), "episodic_trajectory.task must reach the embedder verbatim");
        assert!(
            texts.contains("when login fails Fix login flow"),
            "procedural_skill must embed skill_embedding_text(trigger, skill_name), not a hand-rebuilt string"
        );
    }

    #[tokio::test]
    async fn reembed_reinfer_skips_failed_row_keeps_old_vector_and_continues_batch() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let _id1 = seed_reembed_memory(&store, "row one", vec![1.0f32; 384]).await;
        let id2 = seed_reembed_memory(&store, "row two", vec![1.0f32; 384]).await;
        let _id3 = seed_reembed_memory(&store, "row three", vec![1.0f32; 384]).await;
        let before2 = read_vector_bytes(&store, "memory_vectors", id2).await;

        let embedder: std::sync::Arc<dyn Embedder> = std::sync::Arc::new(FlakyEmbedder {
            fail_on_call: 2,
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let plan = super::ReInferPlan::new(embedder);
        let report = super::reembed_all(&store, super::ReembedMode::ReInfer(plan)).await;

        let memory_outcome = &report.per_table[0];
        assert_eq!(memory_outcome.attempted, 3);
        assert_eq!(memory_outcome.updated, 2);
        assert_eq!(memory_outcome.failed, 1);
        assert!(!memory_outcome.aborted, "a per-row embed failure must not abort the table");

        let after2 = read_vector_bytes(&store, "memory_vectors", id2).await;
        assert_eq!(before2, after2, "the failed row must keep its old vector, byte-for-byte");
        assert!(!report.contract_updated, "a partial failure must never write the contract");
    }

    #[tokio::test]
    async fn reembed_reinfer_update_failure_aborts_only_that_table() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_reembed_memory(&store, "will not survive the write", vec![1.0; 384]).await;

        // Force every write to fail (reads still work) so the failure lands
        // in the batch's UPDATE transaction, not the SELECT — a failure
        // mode distinct from a per-row embed failure.
        store.conn.call(|c| c.execute("PRAGMA query_only = ON", [])).await.unwrap();

        let embedder: std::sync::Arc<dyn Embedder> =
            std::sync::Arc::new(crate::memory::embedder::FakeEmbedder::default());
        let plan = super::ReInferPlan::new(embedder);
        let report = super::reembed_all(&store, super::ReembedMode::ReInfer(plan)).await;

        let memory_outcome = &report.per_table[0];
        assert!(memory_outcome.aborted, "a failing UPDATE transaction must abort the table");
        assert_eq!(memory_outcome.updated, 0, "no row can have been written if the UPDATE tx failed");
        assert!(!report.per_table[1].aborted, "an empty table with no batch attempted must not be marked aborted");
        assert!(!report.per_table[2].aborted, "an empty table with no batch attempted must not be marked aborted");
        assert!(!report.contract_updated);
    }

    #[tokio::test]
    async fn reembed_reinfer_writes_contract_only_after_total_success() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_reembed_memory(&store, "one row is enough", vec![1.0; 384]).await;

        let embedder: std::sync::Arc<dyn Embedder> =
            std::sync::Arc::new(crate::memory::embedder::FakeEmbedder::default());
        let plan = super::ReInferPlan::new(embedder);
        let report = super::reembed_all(&store, super::ReembedMode::ReInfer(plan)).await;

        assert!(report.per_table.iter().all(|t| t.failed == 0 && !t.aborted));
        assert!(report.contract_updated, "a fully clean ReInfer pass must write the contract");

        let (model_id, dim, query_prefix, passage_prefix) =
            read_contract_row(&store).await.expect("contract row must exist after a clean ReInfer pass");
        assert_eq!(model_id, "fake-embedder");
        assert_eq!(dim, 384);
        assert_eq!(query_prefix, "");
        assert_eq!(passage_prefix, "");
    }

    #[tokio::test]
    async fn reembed_reinfer_partial_failure_leaves_old_contract_row_untouched() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .ensure_embedding_contract(crate::memory::db::EmbeddingIdentity {
                model_id: "old-model".into(),
                dim: 384,
                query_prefix: "".into(),
                passage_prefix: "".into(),
            })
            .await
            .unwrap();

        seed_reembed_memory(&store, "row A", vec![1.0; 384]).await;
        seed_reembed_memory(&store, "row B", vec![1.0; 384]).await;

        let embedder: std::sync::Arc<dyn Embedder> = std::sync::Arc::new(FlakyEmbedder {
            fail_on_call: 1,
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let plan = super::ReInferPlan::new(embedder);
        let report = super::reembed_all(&store, super::ReembedMode::ReInfer(plan)).await;

        assert!(!report.contract_updated, "any failed row must block the contract write entirely");
        let (model_id, ..) = read_contract_row(&store).await.unwrap();
        assert_eq!(
            model_id, "old-model",
            "after a partial failure the store still holds the OLD identity — a mismatch on next \
             boot is the honest signal; a rewritten contract would silently claim the whole store \
             was migrated when it was not"
        );
    }

    #[tokio::test]
    async fn reembed_keyset_pagination_processes_every_row_past_one_batch_boundary() {
        let store = MemoryStore::new_for_test().await.unwrap();
        // REEMBED_BATCH is 200; seed comfortably past two full batches so an
        // off-by-one on the `id > last_id` cursor (`>=` would drop the
        // boundary row; a cursor that never advances would spin forever) is
        // forced to show up as a wrong `attempted` count.
        for i in 0..450 {
            seed_reembed_memory(&store, &format!("row {i}"), vec![1.0; 384]).await;
        }

        let report = super::reembed_all(&store, super::ReembedMode::Normalize).await;

        let memory_outcome = &report.per_table[0];
        assert_eq!(memory_outcome.attempted, 450);
        assert_eq!(memory_outcome.updated, 450);
        assert_eq!(memory_outcome.failed, 0);
        assert!(!memory_outcome.aborted);
    }

    #[tokio::test]
    async fn reembed_reinfer_is_idempotent_in_result_across_two_runs() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_reembed_memory(&store, "stable content", vec![1.0; 384]).await;

        let embedder1: std::sync::Arc<dyn Embedder> =
            std::sync::Arc::new(crate::memory::embedder::FakeEmbedder::default());
        super::reembed_all(&store, super::ReembedMode::ReInfer(super::ReInferPlan::new(embedder1))).await;
        let first = read_vector_bytes(&store, "memory_vectors", id).await;

        let embedder2: std::sync::Arc<dyn Embedder> =
            std::sync::Arc::new(crate::memory::embedder::FakeEmbedder::default());
        super::reembed_all(&store, super::ReembedMode::ReInfer(super::ReInferPlan::new(embedder2))).await;
        let second = read_vector_bytes(&store, "memory_vectors", id).await;

        assert_eq!(
            first, second,
            "re-running ReInfer over unchanged text must produce byte-identical vectors — \
             same bytes, but NOT free: it pays for inference again"
        );
    }

    // ── classifying the language of older rows ────────────────────────────

    /// `n` rows without a language, as a store from before the column has
    /// them: every third one Spanish, every third English, the rest code; the
    /// last one deleted.
    async fn seed_unclassified(store: &MemoryStore, n: i64) {
        store
            .conn
            .call(move |c| {
                c.execute(
                    "WITH RECURSIVE k(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM k WHERE i < ?1) \
                     INSERT INTO memory (scope, agent_id, content, memory_type, source, created_at, created_by_type, \
                      created_by_ref, last_modified_at, last_accessed_at, importance, tier, status, content_hash, \
                      update_count, deleted_at) \
                     SELECT 'agent', 'a', CASE i % 3 \
                       WHEN 0 THEN 'La fila ' || i || ' se guardó en español para que la búsqueda la encuentre.' \
                       WHEN 1 THEN 'Row ' || i || ' was saved in English so that the search finds it.' \
                       ELSE '`fn row_' || i || '()`' END, \
                      'semantic_fact', 'explicit', i, 'agent', 'a', 7, 7, 5.0, 3, 'active', 'h' || i, 2, \
                      CASE WHEN i = ?1 THEN 9 END \
                     FROM k",
                    [n],
                )
            })
            .await
            .unwrap();
    }

    type Untouched = (Vec<(i64, String, String, i64, i64)>, i64);

    /// What classifying must leave as it was: each row's text, hash,
    /// modification and update count, and the text index.
    async fn untouched(store: &MemoryStore) -> Untouched {
        store
            .conn
            .call(|c| {
                let mut stmt =
                    c.prepare("SELECT id, content, content_hash, last_modified_at, update_count FROM memory ORDER BY id")?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                let indexed = c.query_row("SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'search'", [], |r| r.get(0))?;
                Ok::<_, rusqlite::Error>((rows, indexed))
            })
            .await
            .unwrap()
    }

    async fn langs(store: &MemoryStore) -> Vec<(i64, Option<String>)> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT id, lang FROM memory ORDER BY id")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
                Ok::<_, rusqlite::Error>(rows)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn the_maintenance_pass_classifies_rows_without_a_language() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_unclassified(&store, 1203).await;

        super::run_maintenance_once_at(&store, TimestampMs::now()).await;

        let langs = langs(&store).await;
        assert_eq!(langs.len(), 1203);
        assert!(langs.iter().all(|(_, l)| l.is_some()), "every row is classified, past one batch");
        let expected = |id: i64| match id % 3 {
            0 => "es",
            1 => "en",
            _ => "und",
        };
        for (id, lang) in &langs {
            assert_eq!(lang.as_deref(), Some(expected(*id)), "row {id}");
        }
        assert_eq!(langs.last().unwrap().1.as_deref(), Some("es"), "a deleted row is classified too");
    }

    #[tokio::test]
    async fn classifying_leaves_content_and_modification_alone() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_unclassified(&store, 6).await;
        store.conn.call(|c| c.execute("UPDATE memory SET lang = 'en' WHERE id = 3", [])).await.unwrap();
        let before = untouched(&store).await;
        assert!(before.1 > 0, "the English rows are indexed");

        super::run_maintenance_once_at(&store, TimestampMs::now()).await;

        assert_eq!(untouched(&store).await, before);
        let langs = langs(&store).await;
        assert_eq!(langs[2], (3, Some("en".to_string())), "a classified row is never classified again");
    }
}
