//! Tauri commands for inspection/override of the 3 memory tables
//! (memoria-3-capas-fase-4), plus 2 additive pure log-formatting helpers that
//! measure doc-7's "Tier 1 covers 80%" criterion.
//!
//! Wrapper/core split (6th instance of the convention — see
//! `commands/embedder.rs` / `commands/session.rs` / `memory/maintenance.rs`):
//! each `#[tauri::command]` wrapper resolves `state.memory_store` and delegates
//! to a `pub(crate) async fn *_core(store: &MemoryStore, ...)` that is directly
//! testable via `MemoryStore::new_for_test()` without a Tauri runtime.

use serde::{Deserialize, Serialize};

use crate::memory::search::RankedHit;
use crate::memory::{MemoryError, MemoryStore};
use crate::models::AppError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// MemoryKind — D-A: fresh enum, NOT a mutation of `record::TableKind`.
//
// `record::TableKind` is used as `RankedHit.kind` (search.rs:47) with
// PascalCase serde (no rename) — it is a record-provenance concept consumed
// by retrieval. `MemoryKind` here is a separate API-input concept: it
// deserializes lowercase ("memory"|"episodic"|"procedural"), matching the
// `all_memories` view's `kind` discriminator and the frontend invoke
// convention. Unifying the two is an open question deferred to a future
// change (ripple risk into RankedHit's existing serde shape).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MemoryKind {
    Memory,
    Episodic,
    Procedural,
}

// ---------------------------------------------------------------------------
// Pure log-formatting helpers (Requirement: Tier-per-hit logging /
// Tier-1 coverage logging). Unit-testable without a DB — the call sites in
// `memory/search.rs` and `memory/prompt_builder.rs` route their side-effect
// (log::debug!/log::info!) through these, per the "side-effect at the call
// site, not the primitive" convention.
// ---------------------------------------------------------------------------

/// Counts hits by tier and renders `"t1:{n1},t2:{n2},t3:{n3}"`.
///
/// `t1` is always `0` when called from `search_all_memories` — the
/// pre-existing `tier > 1` filter in `SEARCH_SQL_*` means tier-1 rows never
/// reach KNN results. That is expected, not a bug.
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

/// Renders whether tier-1 workspace content was present alongside the tier
/// 2/3 retrieval hit count — the real measure of doc-7's "Tier 1 covers 80%"
/// criterion (neither signal alone can answer it).
pub(crate) fn format_tier1_coverage(workspace_present: bool, hit_count: usize) -> String {
    format!("tier1_present={workspace_present} tier23_hits={hit_count}")
}

// ---------------------------------------------------------------------------
// set_importance — D-E: single-statement UPDATE, atomic recompute via
// compute_tier (arity-4 UDF, promote-only — tier.rs:104). D-B: the 4th arg
// to compute_tier is the `memory_type` COLUMN for `memory`, and the literal
// 'episodic_trajectory' / 'procedural_skill' for the other two tables
// (mirrors store_insert.rs:241's INSERT-time literal). `importance` is set
// unconditionally; `tier` stays promote-only via compute_tier itself.
// `deleted_at IS NULL` makes this a no-op (0 rows) on soft-deleted rows —
// locked design decision (open question in design.md), not an error.
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
pub(crate) async fn set_importance_core(
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

/// Soft-deletes the row `id` in the table selected by `kind` by stamping
/// `deleted_at = now_ms`. Returns the number of rows affected (`0` when
/// `id` doesn't exist in that table or was already deleted — idempotent).
pub(crate) async fn delete_memory_core(
    store: &MemoryStore,
    id: i64,
    kind: MemoryKind,
) -> Result<usize, MemoryError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let sql = match kind {
        MemoryKind::Memory => DELETE_SQL_MEMORY,
        MemoryKind::Episodic => DELETE_SQL_EPISODIC,
        MemoryKind::Procedural => DELETE_SQL_PROCEDURAL,
    };

    store
        .conn
        .call(move |c| c.execute(sql, rusqlite::params![now_ms, id]))
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
// list_memories — DTO + params (Task 4.1). Per-table query, not the
// `all_memories` view (the view lacks `memory_type` and can't filter it —
// requirement in spec.md). `summary` is the table-specific content column:
// `content` (memory) | `task` (episodic) | `skill_name` (procedural).
// ---------------------------------------------------------------------------

/// A single row from the `list_memories` inspection command, normalized
/// across the 3 physical tables.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct MemoryAdminRow {
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
}

/// Filter/pagination params for `list_memories_core`. `kind: None` merges
/// all 3 tables; `memory_type` is a no-op (silently ignored) on non-`Memory`
/// kinds.
#[derive(Debug, Clone, Default)]
pub(crate) struct ListParams {
    pub kind: Option<MemoryKind>,
    pub tier: Option<i64>,
    pub memory_type: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// Lists rows from `memory` / `episodic_trajectory` / `procedural_skill`
/// directly (never the `all_memories` view — it lacks `memory_type` and
/// can't filter on it). `kind: None` runs all 3 per-table queries inside the
/// SAME `conn.call` and merges + sorts (`created_at DESC`) + paginates in
/// Rust; `kind: Some(_)` pushes `LIMIT`/`OFFSET` down to SQL directly.
/// `deleted_at IS NULL` is always applied; `status` is NOT filtered —
/// `superseded` rows remain visible (D-C: inspection is an active inventory
/// with superseded shown, distinct from retrieval's stricter filter).
pub(crate) async fn list_memories_core(
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
         access_count, created_at, last_accessed_at, superseded_by, deleted_at \
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
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tauri command wrappers — thin, resolve `state.memory_store`, delegate to
// the `*_core` functions above, `?`-propagate via `AppError`'s
// `#[from] MemoryError` (models/error.rs:97). Not directly unit-testable
// (`State` cannot be constructed without a Tauri "test" feature this
// codebase doesn't enable) — verified by compilation + `lib.rs` registration,
// mirroring `commands/session.rs` / `commands/embedder.rs`.
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_memories(
    state: tauri::State<'_, AppState>,
    kind: Option<MemoryKind>,
    tier: Option<i64>,
    memory_type: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<MemoryAdminRow>, AppError> {
    let params = ListParams { kind, tier, memory_type, limit, offset };
    list_memories_core(&state.memory_store, params)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_importance(
    state: tauri::State<'_, AppState>,
    id: i64,
    kind: MemoryKind,
    importance: f64,
) -> Result<usize, AppError> {
    set_importance_core(&state.memory_store, id, kind, importance)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_memory(
    state: tauri::State<'_, AppState>,
    id: i64,
    kind: MemoryKind,
) -> Result<usize, AppError> {
    delete_memory_core(&state.memory_store, id, kind)
        .await
        .map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Lote 1 — MemoryKind serde round-trip (Task 1.1) ─────────────────────

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

    // ── Lote 1 — format_tier_distribution (Task 1.3) ─────────────────────────

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
            created_at: 0,
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

    // ── Lote 1 — format_tier1_coverage (Task 1.5) ────────────────────────────

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

    // ── Lote 2 — set_importance_core (Tasks 2.1–2.6) ─────────────────────────
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

    // Task 2.1 — memory tier=3 imp=5 architectural_decision, set 9 → tier=1 (Rule 1).
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

    // Task 2.2 — memory tier=1 imp=9.5, set 2.0 → tier stays 1 (promote-only), importance=2.0.
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

    // Task 2.3 — episodic tier=3, set 7 → tier stays 3 (no compute_tier rule for
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

    // Task 2.4 — procedural tier=1, set 3.0 → tier stays 1 (Rule 2, benign no-op), importance=3.0.
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

    // Task 2.5 — id inexistente → Ok(0).
    #[tokio::test]
    async fn set_importance_nonexistent_id_returns_zero() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let affected = set_importance_core(&store, 999_999, MemoryKind::Memory, 9.0)
            .await
            .unwrap();

        assert_eq!(affected, 0);
    }

    // Task 2.6 — deleted_at set → returns 0, no-op (open question locked: no-op, not error).
    #[tokio::test]
    async fn set_importance_soft_deleted_row_is_noop() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row(&store, "chat_message", 3, 5.0, true).await;

        let affected = set_importance_core(&store, id, MemoryKind::Memory, 9.0)
            .await
            .unwrap();

        assert_eq!(affected, 0, "deleted rows must not be updated");
    }

    // ── Lote 3 — delete_memory_core (Tasks 3.1–3.4) ──────────────────────────

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

    // Task 3.1 — seed → delete → deleted_at set (SELECT follow-up).
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

    // Task 3.2 — colliding autoincrement ids across tables: kind disambiguates.
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

    // Task 3.3 — 2nd delete of the same id → 0 rows affected (idempotent).
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

    // Task 3.4 — deleted row is invisible in search_all_memories end-to-end (D-C).
    #[tokio::test]
    async fn delete_memory_row_invisible_in_search_end_to_end() {
        use crate::memory::embedder::{Embedder, FakeEmbedder};
        use crate::memory::search::{search_all_memories, SearchScope};

        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let content = "content for delete-then-search test";
        let emb = embedder.embed(content).await.unwrap();
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

        let before = search_all_memories(&store, &emb, agent, SearchScope::all_default(), 10)
            .await
            .unwrap();
        assert!(before.iter().any(|h| h.id == id), "row must be findable before delete");

        delete_memory_core(&store, id, MemoryKind::Memory)
            .await
            .unwrap();

        let after = search_all_memories(&store, &emb, agent, SearchScope::all_default(), 10)
            .await
            .unwrap();
        assert!(
            !after.iter().any(|h| h.id == id),
            "deleted row must be invisible to search_all_memories"
        );
    }

    // ── Lote 4 — list_memories_core (Tasks 4.2–4.8) ──────────────────────────

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

    // Task 4.2 — kind=Some filters to 1 table.
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

    // Task 4.3 — kind=None merges 3 tables, ORDER BY created_at DESC.
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

    // Task 4.4 — tier filter (WHERE).
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

    // Task 4.5 — memory_type filters ONLY the `memory` kind; no-op (no error) elsewhere.
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

    // Task 4.6 — limit=500 clamps to <=100.
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

    // Task 4.7 — offset paginates over the kind=None merge.
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

    // Task 4.8 — deleted_at excluded; status='superseded' visible.
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
}
