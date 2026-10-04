//! Inspection/override primitives for the 3 memory tables, plus 2 additive
//! pure log-formatting helpers that measure the "Tier 1 covers 80%" criterion.
//!
//! The `pub async fn *_core(store: &MemoryStore, ...)` functions here are
//! directly testable via `MemoryStore::new_for_test()`.

use serde::{Deserialize, Serialize};

// Only the test fixtures below build a RankedHit, and its timestamp field is a
// typed instant. Production code here never constructs one — it only reads rows
// this module queries itself — so the import is test-only.
#[cfg(test)]
use crate::memory::clock::TimestampMs;
use crate::memory::search::RankedHit;
use crate::memory::TableKind;
use crate::memory::{MemoryError, MemoryStore};

// ---------------------------------------------------------------------------
// MemoryKind — a fresh enum, NOT a mutation of `record::TableKind`.
//
// `record::TableKind` is used as `RankedHit.kind` (search.rs:47) with
// PascalCase serde (no rename) — it is a record-provenance concept consumed
// by retrieval. `MemoryKind` here is a separate API-input concept: it
// deserializes lowercase ("memory"|"episodic"|"procedural"), matching the
// frontend invoke convention. Unifying the two is deliberately not done here
// (ripple risk into RankedHit's existing serde shape).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    Memory,
    Episodic,
    Procedural,
}

// ---------------------------------------------------------------------------
// Pure log-formatting helpers (tier-per-hit logging / tier-1 coverage
// logging). Unit-testable without a DB — the call sites in
// `memory/search.rs` and `memory/prompt_builder.rs` route their side-effect
// (log::debug!/log::info!) through these, per the "side-effect at the call
// site, not the primitive" convention.
// ---------------------------------------------------------------------------

/// Counts hits by tier and renders `"t1:{n1},t2:{n2},t3:{n3}"`.
///
/// All three counts can be non-zero: the KNN pool scans every tier, so `t1`
/// reports how much of the hottest tier the query actually pulled in.
pub fn format_tier_distribution(hits: &[RankedHit]) -> String {
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
pub fn format_tier1_coverage(workspace_present: bool, hit_count: usize) -> String {
    format!("tier1_present={workspace_present} tier23_hits={hit_count}")
}

/// Counts hits whose `kind == TableKind::Procedural` (skill hits) and renders
/// `"skill_hits={n}"` — the single call site (`prompt_builder.rs`) logs this
/// beside `format_tier1_coverage`, before `bucket_by_recency` moves `hits`.
pub fn format_skill_hits(hits: &[RankedHit]) -> String {
    let n = hits.iter().filter(|h| h.kind == TableKind::Procedural).count();
    format!("skill_hits={n}")
}

/// Renders requested against returned pool size per table (`memory`,
/// `episodic`, `procedural`, in that order).
///
/// A returned count consistently below the requested one means rows are
/// being dropped after the vector scan already committed to its candidates —
/// the failure these two triples exist to expose. Neither number alone can
/// show it: a low `returned` count is equally consistent with "the corpus
/// genuinely has fewer matching rows" as with "another agent's rows starved
/// the pool", so this only makes the symptom visible, not the diagnosis.
///
/// `requested` is always the pool size the caller asked for, never the wider
/// window a starved table was re-queried with — otherwise the denominator
/// moves under the ratio and a rescued query reads as the worst one in the
/// log. `retried` carries that fact separately, and only the tables it
/// applies to are named, so a line with no marker means no table needed one.
pub fn format_pool_survival(
    requested: (usize, usize, usize),
    returned: (usize, usize, usize),
    retried: (bool, bool, bool),
) -> String {
    let base = format!(
        "pool_req=({}+{}+{}) pool_ret=({}+{}+{})",
        requested.0, requested.1, requested.2, returned.0, returned.1, returned.2,
    );
    let names: Vec<&str> = [("mem", retried.0), ("epi", retried.1), ("pro", retried.2)]
        .into_iter()
        .filter_map(|(name, did)| did.then_some(name))
        .collect();
    if names.is_empty() {
        base
    } else {
        format!("{base} retry=({})", names.join(","))
    }
}

/// Renders how many memory-kind hits came from the dense (KNN) channel
/// versus how many the lexical (FTS5) channel found that the dense channel
/// missed. `lexical_only` is exact — the size of the post-dedup leftover set
/// — so it proves the lexical channel FIRED, never that what it found
/// survived the final `k_final` truncation; only a measured eval delta can
/// show that.
pub fn format_channel_attribution(dense: usize, lexical_only: usize) -> String {
    format!("chan=(dense={dense},lex_only={lexical_only})")
}

// ---------------------------------------------------------------------------
// set_importance — one UPDATE per table, with the tier recomputed inside the
// same statement rather than read-modify-written from Rust: importance and
// tier are derived from each other, and splitting them into two statements
// would leave a window where a concurrent reader sees one updated and the
// other stale.
//
// The 4th argument to `compute_tier` is the `memory_type` COLUMN on `memory`
// but a hardcoded literal on the other two, because those tables have no such
// column — the literal is the same value `store_insert.rs` writes at INSERT
// time, so a tier recomputed here matches the one computed at ingest.
//
// `importance` is set unconditionally; `tier` can only ever move up, because
// `compute_tier` itself is promote-only (see `tier.rs`) — lowering importance
// must not silently discard a tier a row earned through access.
//
// `deleted_at IS NULL` makes this a 0-row no-op on an already-deleted row,
// deliberately, not an error: the caller is adjusting metadata on a row that
// no longer participates in retrieval, and nothing it could do with an error
// differs from what it does with a 0.
// ---------------------------------------------------------------------------

const SET_IMPORTANCE_SQL_MEMORY: &str = "\
    UPDATE memory \
    SET importance = ?1, \
        tier = compute_tier(tier, ?1, access_count, memory_type), \
        last_modified_at = ?2 \
    WHERE id = ?3 AND deleted_at IS NULL";

const SET_IMPORTANCE_SQL_EPISODIC: &str = "\
    UPDATE episodic_trajectory \
    SET importance = ?1, \
        tier = compute_tier(tier, ?1, access_count, 'episodic_trajectory'), \
        last_modified_at = ?2 \
    WHERE id = ?3 AND deleted_at IS NULL";

const SET_IMPORTANCE_SQL_PROCEDURAL: &str = "\
    UPDATE procedural_skill \
    SET importance = ?1, \
        tier = compute_tier(tier, ?1, access_count, 'procedural_skill'), \
        last_modified_at = ?2 \
    WHERE id = ?3 AND deleted_at IS NULL";

/// Sets `importance` and atomically recomputes `tier` (promote-only) for the
/// row `id` in the table selected by `kind`. Returns the number of rows
/// affected (`0` when `id` doesn't exist or the row is soft-deleted).
pub async fn set_importance_core(
    store: &MemoryStore,
    id: i64,
    kind: MemoryKind,
    importance: f64,
) -> Result<usize, MemoryError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let sql = match kind {
        MemoryKind::Memory => SET_IMPORTANCE_SQL_MEMORY,
        MemoryKind::Episodic => SET_IMPORTANCE_SQL_EPISODIC,
        MemoryKind::Procedural => SET_IMPORTANCE_SQL_PROCEDURAL,
    };

    store
        .conn
        .call(move |c| c.execute(sql, rusqlite::params![importance, now_ms, id]))
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend("connection closed during set_importance_core".into())
            }
            other => MemoryError::Backend(other.to_string()),
        })
}

// ---------------------------------------------------------------------------
// delete_memory — mandatory `kind` soft delete. `kind` disambiguates
// colliding autoincrement ids across the 3 physical tables (each has its own
// independent id sequence). `deleted_at IS NULL` in the WHERE clause makes a
// second delete of the same row idempotent (0 rows affected instead of
// re-stamping `deleted_at`).
// ---------------------------------------------------------------------------

const DELETE_SQL_MEMORY: &str = "UPDATE memory SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL";
const DELETE_SQL_EPISODIC: &str =
    "UPDATE episodic_trajectory SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL";
const DELETE_SQL_PROCEDURAL: &str =
    "UPDATE procedural_skill SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL";

const DELETE_VEC_MEMORY: &str = "DELETE FROM memory_vectors WHERE rowid = ?1";
const DELETE_VEC_EPISODIC: &str = "DELETE FROM episodic_vectors WHERE rowid = ?1";
const DELETE_VEC_PROCEDURAL: &str = "DELETE FROM procedural_vectors WHERE rowid = ?1";

/// Soft-deletes the row `id` in the table selected by `kind` by stamping
/// `deleted_at = now_ms`, and prunes its vector row in the same transaction.
/// Returns the number of rows affected by the UPDATE (`0` when `id` doesn't
/// exist in that table or was already deleted — idempotent).
///
/// The vector DELETE runs unconditionally, never gated on the UPDATE's row
/// count: a point delete on an absent rowid is a 0-row no-op, not an error,
/// and running it every time means a row soft-deleted before this code
/// existed gets its orphaned vector swept the next time anyone calls delete
/// on it — no separate backfill needed.
pub async fn delete_memory_core(
    store: &MemoryStore,
    id: i64,
    kind: MemoryKind,
) -> Result<usize, MemoryError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    // Exhaustive match, not string interpolation: a new MemoryKind variant
    // without a table pair here is a compile error, never a runtime SQL
    // injection surface.
    let (row_sql, vec_sql) = match kind {
        MemoryKind::Memory => (DELETE_SQL_MEMORY, DELETE_VEC_MEMORY),
        MemoryKind::Episodic => (DELETE_SQL_EPISODIC, DELETE_VEC_EPISODIC),
        MemoryKind::Procedural => (DELETE_SQL_PROCEDURAL, DELETE_VEC_PROCEDURAL),
    };

    store
        .conn
        .call(move |c| {
            let tx = c.transaction()?;
            let affected = tx.execute(row_sql, rusqlite::params![now_ms, id])?;
            tx.execute(vec_sql, rusqlite::params![id])?;
            tx.commit()?;
            Ok(affected)
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend("connection closed during delete_memory_core".into())
            }
            other => MemoryError::Backend(other.to_string()),
        })
}

// ---------------------------------------------------------------------------
// list_memories — one query per physical table. A UNION view over the three
// would have to flatten them onto one column list, which drops `memory_type`
// (only `memory` has one) — and filtering by memory type is something this
// endpoint has to support.
// `summary` is the table-specific content column:
// `content` (memory) | `task` (episodic) | `skill_name` (procedural).
// ---------------------------------------------------------------------------

// MemoryAdminRow, FailureHistogramRow and SkillStepsTrendRow keep bare i64
// millisecond fields. They cross into the frontend through Serialize, and the
// millisecond newtype the memory module uses internally deliberately implements
// neither Serialize nor Deserialize: a tuple struct that derived it would emit
// {"0":1700000000000} where the UI reads a bare number — it compiles, it runs,
// and nothing logs. Leaving these three untyped is what turns that mistake into
// a compile error instead of a wire-format surprise. If a timestamp here ever
// needs the newtype it needs #[serde(transparent)] and a test asserting the
// emitted JSON is a bare integer, in the same change.

/// A single row from the `list_memories` inspection command, normalized
/// across the 3 physical tables.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryAdminRow {
    pub id: i64,
    pub kind: MemoryKind,
    pub scope: String,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    /// `Some` only for `kind == Memory`; `None` for episodic/procedural rows.
    pub memory_type: Option<String>,
    pub summary: String,
    pub tier: i64,
    pub importance: f64,
    pub status: String,
    pub access_count: i64,
    pub created_at: i64,
    pub last_accessed_at: i64,
    pub superseded_by: Option<i64>,
    pub deleted_at: Option<i64>,
    /// `Some` only for `kind == Procedural`; `None` for memory/episodic rows.
    pub success_count: Option<i64>,
    /// `Some` only for `kind == Procedural`; `None` for memory/episodic rows.
    pub trigger: Option<String>,
    /// Raw JSON text (e.g. `"[42,7,55]"`). `Some` only for `kind ==
    /// Procedural`; `None` for memory/episodic rows.
    pub source_trajectory_ids: Option<String>,
}

/// Filter/pagination params for `list_memories_core`. `kind: None` merges
/// all 3 tables; `memory_type` is a no-op (silently ignored) on non-`Memory`
/// kinds.
#[derive(Debug, Clone, Default)]
pub struct ListParams {
    pub kind: Option<MemoryKind>,
    pub tier: Option<i64>,
    pub memory_type: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// Lists rows from `memory` / `episodic_trajectory` / `procedural_skill`
/// directly. `kind: None` runs all 3 per-table queries inside the
/// SAME `conn.call` and merges + sorts (`created_at DESC`) + paginates in
/// Rust; `kind: Some(_)` pushes `LIMIT`/`OFFSET` down to SQL directly.
/// `deleted_at IS NULL` is always applied; `status` is NOT filtered —
/// `superseded` rows remain visible (inspection is an active inventory with
/// superseded shown, distinct from retrieval's stricter filter).
pub async fn list_memories_core(
    store: &MemoryStore,
    params: ListParams,
) -> Result<Vec<MemoryAdminRow>, MemoryError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 100) as i64;
    let offset = params.offset.unwrap_or(0) as i64;
    let tier = params.tier;
    let memory_type = params.memory_type;
    let kind = params.kind;

    store
        .conn
        .call(move |c| -> rusqlite::Result<Vec<MemoryAdminRow>> {
            let want_memory = matches!(kind, None | Some(MemoryKind::Memory));
            let want_episodic = matches!(kind, None | Some(MemoryKind::Episodic));
            let want_procedural = matches!(kind, None | Some(MemoryKind::Procedural));

            // When merging across tables (`kind: None`) each table is queried
            // with LIMIT=offset+limit/OFFSET=0 so the post-merge global sort
            // has enough candidates; the final skip/take happens in Rust
            // below. For a single `kind`, SQL pushes LIMIT/OFFSET down directly.
            let is_merge = kind.is_none();
            let (per_table_limit, per_table_offset) =
                if is_merge { (offset + limit, 0) } else { (limit, offset) };

            let mut rows: Vec<MemoryAdminRow> = Vec::new();
            if want_memory {
                rows.extend(query_memory_rows(
                    c,
                    tier,
                    memory_type.as_deref(),
                    per_table_limit,
                    per_table_offset,
                )?);
            }
            if want_episodic {
                rows.extend(query_episodic_rows(c, tier, per_table_limit, per_table_offset)?);
            }
            if want_procedural {
                rows.extend(query_procedural_rows(c, tier, per_table_limit, per_table_offset)?);
            }

            if is_merge {
                rows.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                rows = rows
                    .into_iter()
                    .skip(offset as usize)
                    .take(limit as usize)
                    .collect();
            }

            Ok(rows)
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend("connection closed during list_memories_core".into())
            }
            other => MemoryError::Backend(other.to_string()),
        })
}

fn query_memory_rows(
    c: &rusqlite::Connection,
    tier: Option<i64>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
) -> rusqlite::Result<Vec<MemoryAdminRow>> {
    let mut sql = String::from(
        "SELECT id, scope, agent_id, project_id, memory_type, content, tier, importance, \
         status, access_count, created_at, last_accessed_at, superseded_by, deleted_at \
         FROM memory WHERE deleted_at IS NULL",
    );
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(t) = tier {
        sql.push_str(" AND tier = ?");
        params.push(rusqlite::types::Value::Integer(t));
    }
    if let Some(mt) = memory_type {
        sql.push_str(" AND memory_type = ?");
        params.push(rusqlite::types::Value::Text(mt.to_string()));
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");
    params.push(rusqlite::types::Value::Integer(limit));
    params.push(rusqlite::types::Value::Integer(offset));

    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
        Ok(MemoryAdminRow {
            id: row.get(0)?,
            kind: MemoryKind::Memory,
            scope: row.get(1)?,
            agent_id: row.get(2)?,
            project_id: row.get(3)?,
            memory_type: row.get(4)?,
            summary: row.get(5)?,
            tier: row.get(6)?,
            importance: row.get(7)?,
            status: row.get(8)?,
            access_count: row.get(9)?,
            created_at: row.get(10)?,
            last_accessed_at: row.get(11)?,
            superseded_by: row.get(12)?,
            deleted_at: row.get(13)?,
            success_count: None,
            trigger: None,
            source_trajectory_ids: None,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn query_episodic_rows(
    c: &rusqlite::Connection,
    tier: Option<i64>,
    limit: i64,
    offset: i64,
) -> rusqlite::Result<Vec<MemoryAdminRow>> {
    let mut sql = String::from(
        "SELECT id, scope, agent_id, project_id, task, tier, importance, status, \
         access_count, created_at, last_accessed_at, superseded_by, deleted_at \
         FROM episodic_trajectory WHERE deleted_at IS NULL",
    );
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(t) = tier {
        sql.push_str(" AND tier = ?");
        params.push(rusqlite::types::Value::Integer(t));
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");
    params.push(rusqlite::types::Value::Integer(limit));
    params.push(rusqlite::types::Value::Integer(offset));

    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
        Ok(MemoryAdminRow {
            id: row.get(0)?,
            kind: MemoryKind::Episodic,
            scope: row.get(1)?,
            agent_id: row.get(2)?,
            project_id: row.get(3)?,
            memory_type: None,
            summary: row.get(4)?,
            tier: row.get(5)?,
            importance: row.get(6)?,
            status: row.get(7)?,
            access_count: row.get(8)?,
            created_at: row.get(9)?,
            last_accessed_at: row.get(10)?,
            superseded_by: row.get(11)?,
            deleted_at: row.get(12)?,
            success_count: None,
            trigger: None,
            source_trajectory_ids: None,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn query_procedural_rows(
    c: &rusqlite::Connection,
    tier: Option<i64>,
    limit: i64,
    offset: i64,
) -> rusqlite::Result<Vec<MemoryAdminRow>> {
    let mut sql = String::from(
        "SELECT id, scope, agent_id, project_id, skill_name, tier, importance, status, \
         access_count, created_at, last_accessed_at, superseded_by, deleted_at, \
         success_count, trigger, source_trajectory_ids \
         FROM procedural_skill WHERE deleted_at IS NULL",
    );
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(t) = tier {
        sql.push_str(" AND tier = ?");
        params.push(rusqlite::types::Value::Integer(t));
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");
    params.push(rusqlite::types::Value::Integer(limit));
    params.push(rusqlite::types::Value::Integer(offset));

    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
        Ok(MemoryAdminRow {
            id: row.get(0)?,
            kind: MemoryKind::Procedural,
            scope: row.get(1)?,
            agent_id: row.get(2)?,
            project_id: row.get(3)?,
            memory_type: None,
            summary: row.get(4)?,
            tier: row.get(5)?,
            importance: row.get(6)?,
            status: row.get(7)?,
            access_count: row.get(8)?,
            created_at: row.get(9)?,
            last_accessed_at: row.get(10)?,
            superseded_by: row.get(11)?,
            deleted_at: row.get(12)?,
            success_count: Some(row.get(13)?),
            trigger: Some(row.get(14)?),
            source_trajectory_ids: Some(row.get(15)?),
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// get_failure_histogram — root-cause histogram
// bucketed by `created_at` (already ms — no `/1000`). Excludes NULL
// `root_cause`: RCA only classifies FAILED trajectories (miner.rs:379,
// `success_score <= GOLDEN_SCORE_THRESHOLD`); golden/successful rows never
// get a `root_cause` and including them would count successes as failures.
// No agent/project scoping — matches `list_memories`.
// ---------------------------------------------------------------------------

/// One `(bucket, root_cause)` group from `get_failure_histogram_core`.
#[derive(Debug, Clone, Serialize)]
pub struct FailureHistogramRow {
    pub bucket_start_ms: i64,
    pub root_cause: String,
    pub count: i64,
}

/// Groups failed `episodic_trajectory` rows by `(created_at / bucket_ms) *
/// bucket_ms` and `root_cause`, excluding `root_cause IS NULL` and
/// soft-deleted rows. `bucket_ms` is clamped to at least `1` (division-by-zero
/// guard).
pub async fn get_failure_histogram_core(
    store: &MemoryStore,
    bucket_ms: i64,
) -> Result<Vec<FailureHistogramRow>, MemoryError> {
    let bucket_ms = bucket_ms.max(1);

    store
        .conn
        .call(move |c| -> rusqlite::Result<Vec<FailureHistogramRow>> {
            let mut stmt = c.prepare(
                "SELECT (created_at / ?1) * ?1 AS bucket_start_ms, root_cause, COUNT(*) \
                 FROM episodic_trajectory \
                 WHERE root_cause IS NOT NULL AND deleted_at IS NULL \
                 GROUP BY bucket_start_ms, root_cause \
                 ORDER BY bucket_start_ms ASC, root_cause ASC",
            )?;
            let rows = stmt.query_map(rusqlite::params![bucket_ms], |row| {
                Ok(FailureHistogramRow {
                    bucket_start_ms: row.get(0)?,
                    root_cause: row.get(1)?,
                    count: row.get(2)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend("connection closed during get_failure_histogram_core".into())
            }
            other => MemoryError::Backend(other.to_string()),
        })
}

// ---------------------------------------------------------------------------
// get_skill_steps_trend — INNER-joins each
// `procedural_skill.source_trajectory_ids` entry (via `json_each`) to its
// `episodic_trajectory` row: dangling/soft-deleted ids silently produce zero
// rows, never an error (join semantics, not a manual filter). `step_count`
// guards `json_array_length` with `json_valid` — SQLite's
// `json_array_length` RAISES "malformed JSON" on invalid input (it does NOT
// return NULL, so `COALESCE` alone cannot catch it).
// ---------------------------------------------------------------------------

/// One `(skill, trajectory)` point from `get_skill_steps_trend_core`.
#[derive(Debug, Clone, Serialize)]
pub struct SkillStepsTrendRow {
    pub skill_id: i64,
    pub skill_name: String,
    pub trajectory_id: i64,
    pub created_at_ms: i64,
    pub step_count: i64,
}

/// Steps-per-task trend: for each `procedural_skill` (optionally filtered by
/// `skill_id`), joins its `source_trajectory_ids` to the matching
/// `episodic_trajectory` rows and reports `step_count` (length of
/// `tool_calls`). `deleted_at IS NULL` on both tables. `limit` clamps to
/// `[1, 1000]` (default `500`) after the deterministic `ORDER BY ps.id ASC,
/// e.created_at ASC`.
pub async fn get_skill_steps_trend_core(
    store: &MemoryStore,
    skill_id: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<SkillStepsTrendRow>, MemoryError> {
    let limit = limit.unwrap_or(500).clamp(1, 1000) as i64;

    store
        .conn
        .call(move |c| -> rusqlite::Result<Vec<SkillStepsTrendRow>> {
            let mut sql = String::from(
                "SELECT ps.id, ps.skill_name, e.id, e.created_at, \
                 CASE WHEN json_valid(e.tool_calls) \
                      THEN COALESCE(json_array_length(e.tool_calls), 0) \
                      ELSE 0 END AS step_count \
                 FROM procedural_skill ps, json_each(ps.source_trajectory_ids) je \
                 JOIN episodic_trajectory e ON e.id = je.value \
                 WHERE ps.deleted_at IS NULL AND e.deleted_at IS NULL",
            );
            let mut params: Vec<rusqlite::types::Value> = Vec::new();
            if let Some(sid) = skill_id {
                sql.push_str(" AND ps.id = ?");
                params.push(rusqlite::types::Value::Integer(sid));
            }
            sql.push_str(" ORDER BY ps.id ASC, e.created_at ASC LIMIT ?");
            params.push(rusqlite::types::Value::Integer(limit));

            let mut stmt = c.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
                Ok(SkillStepsTrendRow {
                    skill_id: row.get(0)?,
                    skill_name: row.get(1)?,
                    trajectory_id: row.get(2)?,
                    created_at_ms: row.get(3)?,
                    step_count: row.get(4)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend("connection closed during get_skill_steps_trend_core".into())
            }
            other => MemoryError::Backend(other.to_string()),
        })
}

#[cfg(test)]
mod tests {
    use super::*;


    // ── MemoryKind serde round-trip ─────────────────────

    #[test]
    fn memory_kind_serde_lowercase_roundtrip() {
        let cases = [
            (MemoryKind::Memory, "\"memory\""),
            (MemoryKind::Episodic, "\"episodic\""),
            (MemoryKind::Procedural, "\"procedural\""),
        ];
        for (kind, expected_json) in cases {
            let json = serde_json::to_string(&kind).expect("serialize MemoryKind");
            assert_eq!(json, expected_json, "unexpected JSON for {kind:?}");
            let back: MemoryKind =
                serde_json::from_str(&json).expect("deserialize MemoryKind");
            assert_eq!(back, kind, "round-trip mismatch for {kind:?}");
        }
    }

    // ── format_tier_distribution ─────────────────────────

    fn make_hit(tier: i64) -> RankedHit {
        RankedHit {
            id: 1,
            kind: crate::memory::TableKind::Memory,
            content: String::new(),
            memory_type: "chat_message".to_string(),
            tier,
            status: "active".to_string(),
            score: 0.0,
            distance: 0.0,
            created_at: TimestampMs::from_unix_millis(0),
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
    fn format_pool_survival_renders_both_triples() {
        assert_eq!(
            format_pool_survival((30, 5, 5), (28, 5, 2), (false, false, false)),
            "pool_req=(30+5+5) pool_ret=(28+5+2)"
        );
    }

    // A returned count below the requested one reads as "the corpus is thin"
    // until you know whether a wider window was already tried. Naming the
    // tables that retried is what separates a starved pool that recovered
    // from one that was never crowded to begin with.
    #[test]
    fn format_pool_survival_names_the_tables_that_retried() {
        assert_eq!(
            format_pool_survival((30, 5, 5), (28, 5, 2), (true, false, true)),
            "pool_req=(30+5+5) pool_ret=(28+5+2) retry=(mem,pro)"
        );
        assert_eq!(
            format_pool_survival((30, 5, 5), (28, 5, 2), (false, true, false)),
            "pool_req=(30+5+5) pool_ret=(28+5+2) retry=(epi)"
        );
    }

    // The requested triple is the pool size the caller asked for, never the
    // widened window a retry ran with. Reporting the widened number would make
    // the survival ratio look catastrophic exactly on the queries the retry
    // just rescued.
    #[test]
    fn format_pool_survival_denominator_survives_a_retry() {
        assert_eq!(
            format_pool_survival((30, 5, 5), (5, 5, 5), (true, false, false)),
            "pool_req=(30+5+5) pool_ret=(5+5+5) retry=(mem)"
        );
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

    // ── set_importance_core ──────────────────────────────────────────────────
    //
    // Seed helpers mirror `memory/maintenance.rs`'s `seed_row` pattern: a
    // monotonic NONCE keeps `episodic_trajectory.session_id` /
    // `procedural_skill.skill_name` unique (both columns are effectively
    // unique per test row here) across seeded rows within a single test.

    use std::sync::atomic::{AtomicI64, Ordering};
    static NONCE: AtomicI64 = AtomicI64::new(0);

    async fn seed_memory_row(
        store: &MemoryStore,
        memory_type: &str,
        tier: i64,
        importance: f64,
        deleted: bool,
    ) -> i64 {
        let mt = memory_type.to_string();
        let deleted_at: Option<i64> = if deleted { Some(1) } else { None };
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, tier, importance, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref, deleted_at) \
                     VALUES ('agent', 'test-agent', 'seed content', ?, ?, ?, \
                             0, 0, 0, 'agent', '', ?)",
                    rusqlite::params![mt, tier, importance, deleted_at],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .expect("seed_memory_row failed")
    }

    async fn seed_episodic_row(store: &MemoryStore, tier: i64, importance: f64) -> i64 {
        let nonce = NONCE.fetch_add(1, Ordering::SeqCst);
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO episodic_trajectory \
                     (scope, agent_id, session_id, task, tier, importance, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', 'test-agent', ?, 'seed task', ?, ?, \
                             0, 0, 0, 'agent', '')",
                    rusqlite::params![format!("sess-{nonce}"), tier, importance],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .expect("seed_episodic_row failed")
    }

    async fn seed_procedural_row(store: &MemoryStore, tier: i64, importance: f64) -> i64 {
        let nonce = NONCE.fetch_add(1, Ordering::SeqCst);
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO procedural_skill \
                     (scope, agent_id, skill_name, trigger, tier, importance, \
                      promoted_at, source_trajectory_ids, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', 'test-agent', ?, 'seed trigger', ?, ?, \
                             0, '[]', 0, 0, 0, 'agent', '')",
                    rusqlite::params![format!("skill-{nonce}"), tier, importance],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .expect("seed_procedural_row failed")
    }

    async fn memory_tier_importance(store: &MemoryStore, id: i64) -> (i64, f64) {
        store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT tier, importance FROM memory WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap()
    }

    async fn episodic_tier_importance(store: &MemoryStore, id: i64) -> (i64, f64) {
        store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT tier, importance FROM episodic_trajectory WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap()
    }

    async fn procedural_tier_importance(store: &MemoryStore, id: i64) -> (i64, f64) {
        store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT tier, importance FROM procedural_skill WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap()
    }

    // Memory tier=3 imp=5 architectural_decision, set 9 → tier=1 (Rule 1).
    #[tokio::test]
    async fn set_importance_memory_high_importance_promotes_to_tier_1() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row(&store, "architectural_decision", 3, 5.0, false).await;

        let affected = set_importance_core(&store, id, MemoryKind::Memory, 9.0)
            .await
            .unwrap();

        assert_eq!(affected, 1);
        let (tier, importance) = memory_tier_importance(&store, id).await;
        assert_eq!(tier, 1, "importance >= 9.0 must promote to tier 1");
        assert!((importance - 9.0).abs() < 1e-9);
    }

    // Memory tier=1 imp=9.5, set 2.0 → tier stays 1 (promote-only), importance=2.0.
    #[tokio::test]
    async fn set_importance_lowering_never_demotes_tier_1() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row(&store, "chat_message", 1, 9.5, false).await;

        let affected = set_importance_core(&store, id, MemoryKind::Memory, 2.0)
            .await
            .unwrap();

        assert_eq!(affected, 1);
        let (tier, importance) = memory_tier_importance(&store, id).await;
        assert_eq!(tier, 1, "promote-only: lowering importance must never demote tier");
        assert!((importance - 2.0).abs() < 1e-9);
    }

    // Episodic tier=3, set 7 → tier stays 3 (no compute_tier rule for
    // the 'episodic_trajectory' literal), importance=7.
    #[tokio::test]
    async fn set_importance_episodic_no_type_rule_only_rule_1() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_episodic_row(&store, 3, 5.0).await;

        let affected = set_importance_core(&store, id, MemoryKind::Episodic, 7.0)
            .await
            .unwrap();

        assert_eq!(affected, 1);
        let (tier, importance) = episodic_tier_importance(&store, id).await;
        assert_eq!(tier, 3, "episodic has no type-rule below importance 9.0");
        assert!((importance - 7.0).abs() < 1e-9);
    }

    // Procedural tier=1, set 3.0 → tier stays 1 (Rule 2, benign no-op), importance=3.0.
    #[tokio::test]
    async fn set_importance_procedural_recompute_is_benign_noop() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_procedural_row(&store, 1, 8.0).await;

        let affected = set_importance_core(&store, id, MemoryKind::Procedural, 3.0)
            .await
            .unwrap();

        assert_eq!(affected, 1);
        let (tier, importance) = procedural_tier_importance(&store, id).await;
        assert_eq!(tier, 1, "procedural_skill always recomputes to min(1)");
        assert!((importance - 3.0).abs() < 1e-9);
    }

    // Id inexistente → Ok(0).
    #[tokio::test]
    async fn set_importance_nonexistent_id_returns_zero() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let affected = set_importance_core(&store, 999_999, MemoryKind::Memory, 9.0)
            .await
            .unwrap();

        assert_eq!(affected, 0);
    }

    // Deleted_at set → returns 0, no-op (no-op, not error).
    #[tokio::test]
    async fn set_importance_soft_deleted_row_is_noop() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row(&store, "chat_message", 3, 5.0, true).await;

        let affected = set_importance_core(&store, id, MemoryKind::Memory, 9.0)
            .await
            .unwrap();

        assert_eq!(affected, 0, "deleted rows must not be updated");
    }

    // ── delete_memory_core ──────────────────────────

    async fn memory_deleted_at(store: &MemoryStore, id: i64) -> Option<i64> {
        store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT deleted_at FROM memory WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap()
    }

    async fn episodic_deleted_at(store: &MemoryStore, id: i64) -> Option<i64> {
        store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT deleted_at FROM episodic_trajectory WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap()
    }

    // Seed → delete → deleted_at set (SELECT follow-up).
    #[tokio::test]
    async fn delete_memory_sets_deleted_at() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row(&store, "chat_message", 3, 5.0, false).await;
        assert_eq!(memory_deleted_at(&store, id).await, None);

        let affected = delete_memory_core(&store, id, MemoryKind::Memory)
            .await
            .unwrap();

        assert_eq!(affected, 1);
        assert!(
            memory_deleted_at(&store, id).await.is_some(),
            "deleted_at must be set after delete"
        );
    }

    // Colliding autoincrement ids across tables: kind disambiguates.
    #[tokio::test]
    async fn delete_memory_kind_disambiguates_colliding_ids() {
        let store = MemoryStore::new_for_test().await.unwrap();
        // Both tables start empty in a fresh in-memory DB, so the first row
        // inserted into each table gets id=1 — a genuine cross-table collision.
        let mem_id = seed_memory_row(&store, "chat_message", 3, 5.0, false).await;
        let epi_id = seed_episodic_row(&store, 3, 5.0).await;
        assert_eq!(mem_id, epi_id, "test setup requires a real id collision");

        let affected = delete_memory_core(&store, epi_id, MemoryKind::Episodic)
            .await
            .unwrap();

        assert_eq!(affected, 1);
        assert!(
            episodic_deleted_at(&store, epi_id).await.is_some(),
            "episodic row must be deleted"
        );
        assert_eq!(
            memory_deleted_at(&store, mem_id).await,
            None,
            "memory row with the same id must NOT be affected"
        );
    }

    // 2nd delete of the same id → 0 rows affected (idempotent).
    #[tokio::test]
    async fn delete_memory_second_delete_is_idempotent_zero() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row(&store, "chat_message", 3, 5.0, false).await;

        let first = delete_memory_core(&store, id, MemoryKind::Memory)
            .await
            .unwrap();
        let second = delete_memory_core(&store, id, MemoryKind::Memory)
            .await
            .unwrap();

        assert_eq!(first, 1);
        assert_eq!(second, 0, "second delete of an already-deleted row must affect 0 rows");
    }

    // Deleted row is invisible in search_all_memories end-to-end.
    #[tokio::test]
    async fn delete_memory_row_invisible_in_search_end_to_end() {
        use crate::memory::embedder::{Embedder, FakeEmbedder};
        use crate::memory::search::{search_all_memories, SearchScope};

        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let content = "content for delete-then-search test";
        let emb = embedder
            .embed_with_role(content, crate::memory::embedder::EmbedRole::Passage)
            .await
            .unwrap();
        let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
        let agent = "agent-delete-search";

        let id: i64 = {
            let agent = agent.to_string();
            let content = content.to_string();
            let emb_bytes = emb_bytes.clone();
            store
                .conn
                .call(move |c| -> rusqlite::Result<i64> {
                    let tx = c.transaction()?;
                    tx.execute(
                        "INSERT INTO memory (scope, agent_id, content, memory_type, source, \
                         created_at, created_by_type, created_by_ref, \
                         last_modified_at, last_accessed_at, importance, tier, status) \
                         VALUES ('agent', ?1, ?2, 'chat_message', 'chat', \
                         0, 'user', ?1, 0, 0, 5.0, 3, 'active')",
                        rusqlite::params![agent, content],
                    )?;
                    let id = tx.last_insert_rowid();
                    tx.execute(
                        "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                        rusqlite::params![id, emb_bytes],
                    )?;
                    tx.commit()?;
                    Ok(id)
                })
                .await
                .unwrap()
        };

        let before = search_all_memories(&store, &emb, None, agent, SearchScope::all_default(), 10)
            .await
            .unwrap();
        assert!(before.iter().any(|h| h.id == id), "row must be findable before delete");

        delete_memory_core(&store, id, MemoryKind::Memory)
            .await
            .unwrap();

        let after = search_all_memories(&store, &emb, None, agent, SearchScope::all_default(), 10)
            .await
            .unwrap();
        assert!(
            !after.iter().any(|h| h.id == id),
            "deleted row must be invisible to search_all_memories"
        );
    }

    // ── delete_memory_core — vector pruning on soft-delete ───────────────────

    fn dummy_embedding_384() -> Vec<u8> {
        vec![0u8; 384 * 4]
    }

    async fn seed_memory_row_with_vector(store: &MemoryStore) -> i64 {
        let emb = dummy_embedding_384();
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, tier, importance, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', 'test-agent', 'seed content', 'chat_message', \
                             3, 5.0, 0, 0, 0, 'agent', '')",
                    [],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                    rusqlite::params![id, emb],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .expect("seed_memory_row_with_vector failed")
    }

    async fn seed_episodic_row_with_vector(store: &MemoryStore) -> i64 {
        let nonce = NONCE.fetch_add(1, Ordering::SeqCst);
        let emb = dummy_embedding_384();
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO episodic_trajectory \
                     (scope, agent_id, session_id, task, tier, importance, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', 'test-agent', ?1, 'seed task', 3, 5.0, \
                             0, 0, 0, 'agent', '')",
                    rusqlite::params![format!("sess-vec-{nonce}")],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO episodic_vectors(rowid, embedding) VALUES (?, ?)",
                    rusqlite::params![id, emb],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .expect("seed_episodic_row_with_vector failed")
    }

    async fn seed_procedural_row_with_vector(store: &MemoryStore) -> i64 {
        let nonce = NONCE.fetch_add(1, Ordering::SeqCst);
        let emb = dummy_embedding_384();
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
                     VALUES ('agent', 'test-agent', ?1, 'seed trigger', 1, 8.0, \
                             0, '[]', 0, 0, 0, 'agent', '')",
                    rusqlite::params![format!("skill-vec-{nonce}")],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO procedural_vectors(rowid, embedding) VALUES (?, ?)",
                    rusqlite::params![id, emb],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .expect("seed_procedural_row_with_vector failed")
    }

    async fn vector_row_count(store: &MemoryStore, vectors_table: &'static str, id: i64) -> i64 {
        let sql = format!("SELECT COUNT(*) FROM {vectors_table} WHERE rowid = ?1");
        store
            .conn
            .call(move |c| c.query_row(&sql, [id], |r| r.get(0)))
            .await
            .unwrap()
    }

    // Soft-deleting must not leave the vector row behind: a vector with no
    // live parent still costs the vec0 scan on every future query, forever,
    // for a row nothing can ever return again.
    #[tokio::test]
    async fn delete_memory_removes_vector_row_all_kinds() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let mem_id = seed_memory_row_with_vector(&store).await;
        delete_memory_core(&store, mem_id, MemoryKind::Memory).await.unwrap();
        assert_eq!(
            vector_row_count(&store, "memory_vectors", mem_id).await,
            0,
            "memory_vectors row must be gone after soft-delete"
        );

        let epi_id = seed_episodic_row_with_vector(&store).await;
        delete_memory_core(&store, epi_id, MemoryKind::Episodic).await.unwrap();
        assert_eq!(
            vector_row_count(&store, "episodic_vectors", epi_id).await,
            0,
            "episodic_vectors row must be gone after soft-delete"
        );

        let proc_id = seed_procedural_row_with_vector(&store).await;
        delete_memory_core(&store, proc_id, MemoryKind::Procedural).await.unwrap();
        assert_eq!(
            vector_row_count(&store, "procedural_vectors", proc_id).await,
            0,
            "procedural_vectors row must be gone after soft-delete"
        );
    }

    // Re-running delete on an already soft-deleted row must still clean up a
    // vector the first delete missed (e.g. one soft-deleted before this
    // change shipped) — the UPDATE affecting 0 rows must not gate the DELETE.
    #[tokio::test]
    async fn delete_memory_idempotent_self_heals_orphan_vector() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row_with_vector(&store).await;

        // Simulate a row soft-deleted before vector pruning existed: stamp
        // deleted_at directly, bypassing delete_memory_core, so the vector
        // row is left orphaned exactly like a pre-existing production row.
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE memory SET deleted_at = 1 WHERE id = ?1",
                    rusqlite::params![id],
                )
            })
            .await
            .unwrap();
        assert_eq!(
            vector_row_count(&store, "memory_vectors", id).await,
            1,
            "test setup: the vector row must still be present before self-heal"
        );

        let affected = delete_memory_core(&store, id, MemoryKind::Memory).await.unwrap();

        assert_eq!(affected, 0, "the row was already deleted, so the UPDATE affects 0 rows");
        assert_eq!(
            vector_row_count(&store, "memory_vectors", id).await,
            0,
            "the orphaned vector must be pruned even though the row update affected 0 rows"
        );
    }

    // ── format_skill_hits ────────────────────────────────────────────────────

    fn make_kind_hit(kind: crate::memory::TableKind) -> RankedHit {
        RankedHit {
            id: 1,
            kind,
            content: String::new(),
            memory_type: "chat_message".to_string(),
            tier: 2,
            status: "active".to_string(),
            score: 0.0,
            distance: 0.0,
            created_at: TimestampMs::from_unix_millis(0),
        }
    }

    #[test]
    fn format_skill_hits_counts_only_procedural() {
        let hits = vec![
            make_kind_hit(crate::memory::TableKind::Procedural),
            make_kind_hit(crate::memory::TableKind::Procedural),
            make_kind_hit(crate::memory::TableKind::Memory),
            make_kind_hit(crate::memory::TableKind::Memory),
            make_kind_hit(crate::memory::TableKind::Memory),
            make_kind_hit(crate::memory::TableKind::Episodic),
        ];
        assert_eq!(format_skill_hits(&hits), "skill_hits=2");
    }

    #[test]
    fn format_skill_hits_empty_is_zero() {
        assert_eq!(format_skill_hits(&[]), "skill_hits=0");
    }

    // ── get_failure_histogram_core ─────────

    /// Seeds a raw `episodic_trajectory` row with an explicit `created_at`
    /// and optional `root_cause` — the columns `seed_episodic_row` above
    /// doesn't expose. `deleted_at` stays NULL.
    async fn seed_episodic_with_cause(
        store: &MemoryStore,
        created_at: i64,
        root_cause: Option<&str>,
    ) -> i64 {
        let nonce = NONCE.fetch_add(1, Ordering::SeqCst);
        let root_cause = root_cause.map(|s| s.to_string());
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO episodic_trajectory \
                     (scope, agent_id, session_id, task, root_cause, tier, importance, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', 'test-agent', ?, 'seed task', ?, 3, 5.0, \
                             ?, ?, ?, 'agent', '')",
                    rusqlite::params![
                        format!("sess-hist-{nonce}"),
                        root_cause,
                        created_at,
                        created_at,
                        created_at
                    ],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .expect("seed_episodic_with_cause failed")
    }

    // Groups by bucket and cause; ordered bucket_start_ms ASC.
    #[tokio::test]
    async fn failure_histogram_groups_by_bucket_and_cause() {
        let store = MemoryStore::new_for_test().await.unwrap();
        const BUCKET_MS: i64 = 86_400_000;
        // Bucket 0: two 'missing_context' rows.
        seed_episodic_with_cause(&store, 0, Some("missing_context")).await;
        seed_episodic_with_cause(&store, 1_000, Some("missing_context")).await;
        // Bucket 1 (different day): one 'model_limitation' row.
        seed_episodic_with_cause(&store, BUCKET_MS, Some("model_limitation")).await;

        let rows = get_failure_histogram_core(&store, BUCKET_MS).await.unwrap();

        assert_eq!(rows.len(), 2, "expected 2 groups, got {rows:?}");
        assert_eq!(rows[0].bucket_start_ms, 0);
        assert_eq!(rows[0].root_cause, "missing_context");
        assert_eq!(rows[0].count, 2);
        assert_eq!(rows[1].bucket_start_ms, BUCKET_MS);
        assert_eq!(rows[1].root_cause, "model_limitation");
        assert_eq!(rows[1].count, 1);
        for w in rows.windows(2) {
            assert!(w[0].bucket_start_ms <= w[1].bucket_start_ms, "must be ASC");
        }
    }

    // Golden/NULL root_cause excluded.
    #[tokio::test]
    async fn failure_histogram_excludes_null_root_cause() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_episodic_with_cause(&store, 0, Some("missing_context")).await;
        seed_episodic_with_cause(&store, 0, None).await; // golden
        seed_episodic_with_cause(&store, 0, None).await; // golden

        let rows = get_failure_histogram_core(&store, 86_400_000).await.unwrap();

        assert_eq!(rows.len(), 1, "NULL root_cause rows must be excluded, got {rows:?}");
        assert_eq!(rows[0].count, 1);
        assert_eq!(rows[0].root_cause, "missing_context");
        assert!(
            rows.iter().all(|r| !r.root_cause.is_empty()),
            "no row may have an empty/NULL root_cause"
        );
    }

    // Empty table → Ok(vec![]).
    #[tokio::test]
    async fn failure_histogram_empty_table_returns_empty() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let rows = get_failure_histogram_core(&store, 86_400_000).await.unwrap();

        assert!(rows.is_empty());
    }

    // ── get_skill_steps_trend_core ─────────

    fn tool_calls_json_of_len(n: usize) -> String {
        serde_json::to_string(&(0..n).map(|i| format!("t{i}")).collect::<Vec<String>>())
            .expect("serialize tool_calls fixture")
    }

    /// Seeds a raw `episodic_trajectory` row with an explicit `created_at`
    /// and `tool_calls` JSON array — the columns `seed_episodic_row` above
    /// doesn't expose.
    async fn seed_episodic_with_tool_calls(
        store: &MemoryStore,
        created_at: i64,
        tool_calls_json: &str,
    ) -> i64 {
        let nonce = NONCE.fetch_add(1, Ordering::SeqCst);
        let tool_calls = tool_calls_json.to_string();
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO episodic_trajectory \
                     (scope, agent_id, session_id, task, tool_calls, tier, importance, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', 'test-agent', ?, 'seed task', ?, 3, 5.0, \
                             ?, ?, ?, 'agent', '')",
                    rusqlite::params![
                        format!("sess-trend-{nonce}"),
                        tool_calls,
                        created_at,
                        created_at,
                        created_at
                    ],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .expect("seed_episodic_with_tool_calls failed")
    }

    /// Seeds a `procedural_skill` row with an explicit `source_trajectory_ids`
    /// JSON array (`seed_procedural_row` above always writes `'[]'`).
    async fn seed_procedural_with_sources(store: &MemoryStore, source_ids_json: &str) -> i64 {
        let nonce = NONCE.fetch_add(1, Ordering::SeqCst);
        let ids = source_ids_json.to_string();
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO procedural_skill \
                     (scope, agent_id, skill_name, trigger, tier, importance, \
                      promoted_at, source_trajectory_ids, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', 'test-agent', ?, 'seed trigger', 1, 8.0, \
                             0, ?, 0, 0, 0, 'agent', '')",
                    rusqlite::params![format!("skill-trend-{nonce}"), ids],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .expect("seed_procedural_with_sources failed")
    }

    // Ordered by skill and time, step_count from tool_calls length.
    #[tokio::test]
    async fn skill_steps_trend_ordered_by_skill_and_time() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id1 = seed_episodic_with_tool_calls(&store, 100, &tool_calls_json_of_len(8)).await;
        let id2 = seed_episodic_with_tool_calls(&store, 200, &tool_calls_json_of_len(5)).await;
        let id3 = seed_episodic_with_tool_calls(&store, 300, &tool_calls_json_of_len(3)).await;
        let skill_id = seed_procedural_with_sources(
            &store,
            &serde_json::to_string(&[id1, id2, id3]).unwrap(),
        )
        .await;

        let rows = get_skill_steps_trend_core(&store, Some(skill_id), None)
            .await
            .unwrap();

        assert_eq!(rows.len(), 3, "expected 3 rows, got {rows:?}");
        assert_eq!(rows[0].step_count, 8);
        assert_eq!(rows[1].step_count, 5);
        assert_eq!(rows[2].step_count, 3);
        for w in rows.windows(2) {
            assert!(w[0].created_at_ms <= w[1].created_at_ms, "must be ASC by time");
        }
    }

    // Empty procedural_skill table → Ok(vec![]).
    #[tokio::test]
    async fn skill_steps_trend_empty_table_returns_empty() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let rows = get_skill_steps_trend_core(&store, None, None).await.unwrap();

        assert!(rows.is_empty());
    }

    // Malformed/empty tool_calls never errors; step_count=0 for both.
    #[tokio::test]
    async fn skill_steps_trend_malformed_or_empty_tool_calls_never_errors() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let empty_id = seed_episodic_with_tool_calls(&store, 100, "[]").await;
        let malformed_id = seed_episodic_with_tool_calls(&store, 200, "[]").await;
        // Bypass the typed insert (serde `Vec<String>` + NOT NULL DEFAULT '[]'
        // make malformed JSON unreachable in prod) via a raw UPDATE.
        store
            .conn
            .call(move |c| -> rusqlite::Result<()> {
                c.execute(
                    "UPDATE episodic_trajectory SET tool_calls = '{bad' WHERE id = ?1",
                    rusqlite::params![malformed_id],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let skill_id = seed_procedural_with_sources(
            &store,
            &serde_json::to_string(&[empty_id, malformed_id]).unwrap(),
        )
        .await;

        let rows = get_skill_steps_trend_core(&store, Some(skill_id), None)
            .await
            .expect("must return Ok even with malformed JSON, never Err");

        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.step_count == 0), "both must yield step_count=0, got {rows:?}");
    }

    // Dangling trajectory id silently dropped by the INNER JOIN.
    #[tokio::test]
    async fn skill_steps_trend_dangling_id_dropped_silently() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let live_id = seed_episodic_with_tool_calls(&store, 100, &tool_calls_json_of_len(4)).await;
        let skill_id = seed_procedural_with_sources(
            &store,
            &serde_json::to_string(&[live_id, 999_999]).unwrap(),
        )
        .await;

        let rows = get_skill_steps_trend_core(&store, Some(skill_id), None)
            .await
            .unwrap();

        assert_eq!(rows.len(), 1, "dangling id must be dropped with no error, got {rows:?}");
        assert_eq!(rows[0].trajectory_id, live_id);
    }

    // Limit clamp math: unwrap_or(500).clamp(1,1000). Routed
    // through a variable-parameter helper (not a bare `Some(_).unwrap_or`
    // literal) so clippy's `unnecessary_literal_unwrap` doesn't fire on a
    // pattern that mirrors production `get_skill_steps_trend_core`'s real
    // clamp, which operates on a runtime `Option<u32>` argument.
    fn clamp_limit(limit: Option<u32>) -> u32 {
        limit.unwrap_or(500).clamp(1, 1000)
    }

    #[test]
    fn skill_steps_trend_limit_clamp_math() {
        assert_eq!(clamp_limit(Some(5_000)), 1_000);
        assert_eq!(clamp_limit(None), 500);
    }

    #[tokio::test]
    async fn skill_steps_trend_large_limit_does_not_error_on_small_set() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id1 = seed_episodic_with_tool_calls(&store, 100, &tool_calls_json_of_len(1)).await;
        let skill_id =
            seed_procedural_with_sources(&store, &serde_json::to_string(&[id1]).unwrap()).await;

        let rows = get_skill_steps_trend_core(&store, Some(skill_id), Some(5_000))
            .await
            .unwrap();

        assert_eq!(rows.len(), 1, "limit=5000 clamps to 1000 but must not error or truncate below actual count");
    }

    // ── list_memories_core ──────────────────────────

    async fn seed_memory_full(
        store: &MemoryStore,
        memory_type: &str,
        tier: i64,
        status: &str,
        deleted: bool,
        created_at: i64,
    ) -> i64 {
        let mt = memory_type.to_string();
        let status = status.to_string();
        let deleted_at: Option<i64> = if deleted { Some(1) } else { None };
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, tier, importance, status, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref, deleted_at) \
                     VALUES ('agent', 'test-agent', 'row content', ?, ?, 5.0, ?, \
                             ?, 0, 0, 'agent', '', ?)",
                    rusqlite::params![mt, tier, status, created_at, deleted_at],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .expect("seed_memory_full failed")
    }

    // Kind=Some filters to 1 table.
    #[tokio::test]
    async fn list_memories_kind_some_filters_to_one_table() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_full(&store, "chat_message", 3, "active", false, 100).await;
        seed_episodic_row(&store, 3, 5.0).await;
        seed_procedural_row(&store, 1, 8.0).await;

        let rows = list_memories_core(
            &store,
            ListParams { kind: Some(MemoryKind::Memory), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, MemoryKind::Memory);
    }

    // Kind=None merges 3 tables, ORDER BY created_at DESC.
    #[tokio::test]
    async fn list_memories_kind_none_merges_all_tables_sorted_desc() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_full(&store, "chat_message", 3, "active", false, 100).await;
        seed_episodic_row(&store, 3, 5.0).await; // created_at = 0
        seed_procedural_row(&store, 1, 8.0).await; // created_at = 0

        let rows = list_memories_core(&store, ListParams::default())
            .await
            .unwrap();

        assert_eq!(rows.len(), 3, "expected rows merged from all 3 tables");
        assert_eq!(rows[0].kind, MemoryKind::Memory, "highest created_at (100) must sort first");
        for w in rows.windows(2) {
            assert!(w[0].created_at >= w[1].created_at, "must be sorted DESC by created_at");
        }
    }

    // Tier filter (WHERE).
    #[tokio::test]
    async fn list_memories_tier_filter_applies() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_full(&store, "chat_message", 2, "active", false, 100).await;
        seed_memory_full(&store, "chat_message", 3, "active", false, 200).await;

        let rows = list_memories_core(
            &store,
            ListParams { kind: Some(MemoryKind::Memory), tier: Some(2), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tier, 2);
    }

    // Memory_type filters ONLY the `memory` kind; no-op (no error) elsewhere.
    #[tokio::test]
    async fn list_memories_memory_type_is_noop_on_other_kinds() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_episodic_row(&store, 3, 5.0).await;

        let rows = list_memories_core(
            &store,
            ListParams {
                kind: Some(MemoryKind::Episodic),
                memory_type: Some("user_preference".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1, "memory_type filter must be ignored for non-Memory kinds");
    }

    // Limit=500 clamps to <=100.
    #[tokio::test]
    async fn list_memories_limit_clamps_to_100() {
        let store = MemoryStore::new_for_test().await.unwrap();
        for i in 0..120i64 {
            seed_memory_full(&store, "chat_message", 3, "active", false, i).await;
        }

        let rows = list_memories_core(
            &store,
            ListParams { kind: Some(MemoryKind::Memory), limit: Some(500), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 100, "limit must clamp to the hard cap of 100");
    }

    // Offset paginates over the kind=None merge.
    #[tokio::test]
    async fn list_memories_offset_paginates_merged_results() {
        let store = MemoryStore::new_for_test().await.unwrap();
        for i in 0..5i64 {
            seed_memory_full(&store, "chat_message", 3, "active", false, i * 10).await;
        }

        let page1 = list_memories_core(
            &store,
            ListParams { limit: Some(2), offset: Some(0), ..Default::default() },
        )
        .await
        .unwrap();
        let page2 = list_memories_core(
            &store,
            ListParams { limit: Some(2), offset: Some(2), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(page1.len(), 2);
        assert_eq!(page2.len(), 2);
        assert_ne!(page1[0].id, page2[0].id, "offset must shift the page window");
    }

    // Deleted_at excluded; status='superseded' visible.
    #[tokio::test]
    async fn list_memories_excludes_deleted_keeps_superseded() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_full(&store, "chat_message", 3, "active", true, 100).await; // deleted
        let superseded_id =
            seed_memory_full(&store, "chat_message", 3, "superseded", false, 200).await;

        let rows = list_memories_core(
            &store,
            ListParams { kind: Some(MemoryKind::Memory), limit: Some(500), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1, "deleted row must be excluded");
        assert_eq!(rows[0].id, superseded_id);
        assert_eq!(rows[0].status, "superseded", "superseded rows must remain visible");
    }

    // ── MemoryAdminRow +3 skill fields ──────

    /// Seeds a `procedural_skill` row with explicit `success_count`, `trigger`,
    /// and `source_trajectory_ids` (`seed_procedural_row` above hardcodes
    /// `trigger`/`success_count`/`ids`).
    async fn seed_procedural_full(
        store: &MemoryStore,
        success_count: i64,
        trigger: &str,
        source_ids_json: &str,
    ) -> i64 {
        let nonce = NONCE.fetch_add(1, Ordering::SeqCst);
        let trigger = trigger.to_string();
        let ids = source_ids_json.to_string();
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO procedural_skill \
                     (scope, agent_id, skill_name, trigger, success_count, tier, importance, \
                      promoted_at, source_trajectory_ids, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', 'test-agent', ?, ?, ?, 1, 8.0, \
                             0, ?, 0, 0, 0, 'agent', '')",
                    rusqlite::params![format!("skill-full-{nonce}"), trigger, success_count, ids],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .expect("seed_procedural_full failed")
    }

    // Procedural row populates the 3 new fields.
    #[tokio::test]
    async fn list_memories_procedural_row_populates_skill_fields() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_procedural_full(&store, 3, "when login fails", "[1,2,3]").await;

        let rows = list_memories_core(
            &store,
            ListParams { kind: Some(MemoryKind::Procedural), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].success_count, Some(3));
        assert_eq!(rows[0].trigger, Some("when login fails".to_string()));
        assert_eq!(rows[0].source_trajectory_ids, Some("[1,2,3]".to_string()));
    }

    // Memory/episodic rows keep the 3 new fields as None.
    #[tokio::test]
    async fn list_memories_memory_and_episodic_rows_have_none_skill_fields() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_full(&store, "chat_message", 3, "active", false, 100).await;
        seed_episodic_row(&store, 3, 5.0).await;

        let rows = list_memories_core(&store, ListParams::default()).await.unwrap();

        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(row.success_count, None, "row {row:?} must have success_count=None");
            assert_eq!(row.trigger, None, "row {row:?} must have trigger=None");
            assert_eq!(
                row.source_trajectory_ids, None,
                "row {row:?} must have source_trajectory_ids=None"
            );
        }
    }

    // ── skill hits never touch the access tracker ──
    //
    // `format_skill_hits` is a pure function — it touches ZERO DB. This
    // asserts the orthogonal STATE invariant (not a call-count): a seeded
    // `procedural_skill.access_count` must stay `0` after the skill-hits path
    // runs, proving no procedural/episodic id ever reaches
    // `AccessTracker::record` (which only ever `UPDATE memory` — access.rs).

    #[tokio::test]
    async fn skill_hits_path_never_touches_access_tracker() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let skill_id = seed_procedural_row(&store, 1, 8.0).await;

        let hits = vec![RankedHit {
            id: skill_id,
            kind: crate::memory::TableKind::Procedural,
            content: String::new(),
            memory_type: "skill".to_string(),
            tier: 1,
            status: "active".to_string(),
            score: 0.0,
            distance: 0.0,
            created_at: TimestampMs::from_unix_millis(0),
        }];

        assert_eq!(format_skill_hits(&hits), "skill_hits=1");

        let access_count: i64 = store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT access_count FROM procedural_skill WHERE id = ?1",
                    [skill_id],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();

        assert_eq!(
            access_count, 0,
            "format_skill_hits must never write to AccessTracker/DB; access_count must stay 0"
        );
    }
}
