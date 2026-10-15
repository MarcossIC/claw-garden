//! Inspection/override primitives for the 3 memory tables, plus 2 additive
//! pure log-formatting helpers that measure the "Tier 1 covers 80%" criterion.
//!
//! The `pub async fn *_core(store: &MemoryStore, ...)` functions here are
//! directly testable via `MemoryStore::new_for_test()`.

use serde::{Deserialize, Serialize};

use crate::memory::clock::TimestampMs;
use crate::memory::chunker::embedding_text;
use crate::memory::dispute::{release_challengers, settle_row};
use crate::memory::embedder::{EmbedError, EmbedRole, Embedder};
use crate::memory::search::{viewer_match_expression, RankedHit};
use crate::memory::store_insert::MemoryType;
use crate::memory::tier::retier;
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
// update_memory — a correction of importance and/or type. The row is read and
// written inside one transaction on the store's single connection: the new
// tier depends on the old values (`tier::retier`), and no reader can see the
// two statements apart.
//
// Outside `memory` the type is the table literal `store_insert.rs` writes at
// INSERT time, so a tier recomputed here matches the one computed at ingest.
//
// Only a live row is changed: a deleted one no longer participates in
// retrieval, and "missing", "deleted" and "another agent's" are one answer.
// ---------------------------------------------------------------------------

/// Why a requested change was refused before touching the store.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PatchError {
    #[error("nothing to change: pass importance, memory_type or both")]
    Empty,
    #[error("importance must be a number from 0 to 10, got {0}")]
    ImportanceOutOfScale(f64),
    #[error("unknown memory_type {0:?}; expected one of semantic_fact, user_preference, architectural_decision, session_summary, code_block, chat_message, tool_error")]
    UnknownType(String),
    #[error("memory_type can only be changed on memory rows, not on {0:?} rows")]
    TypeOnlyOnMemory(MemoryKind),
}

/// A valid change to one row: at least one field, importance on the store's
/// scale, and a type only where the table has one.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryPatch {
    importance: Option<f64>,
    memory_type: Option<MemoryType>,
}

impl MemoryPatch {
    /// Unlike a save, which clamps the estimate an agent makes in passing,
    /// an out-of-scale importance here is refused: a correction is explicit,
    /// and clamping it would report a value nobody asked for as done.
    pub fn new(kind: MemoryKind, importance: Option<f64>, memory_type: Option<&str>) -> Result<Self, PatchError> {
        if importance.is_none() && memory_type.is_none() {
            return Err(PatchError::Empty);
        }
        if let Some(v) = importance {
            if !(0.0..=10.0).contains(&v) {
                return Err(PatchError::ImportanceOutOfScale(v));
            }
        }
        let memory_type = match memory_type {
            None => None,
            Some(_) if kind != MemoryKind::Memory => return Err(PatchError::TypeOnlyOnMemory(kind)),
            Some(raw) => Some(MemoryType::from_db_str(raw).ok_or_else(|| PatchError::UnknownType(raw.to_string()))?),
        };
        Ok(Self { importance, memory_type })
    }
}

/// How a changed row ended up.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UpdatedRow {
    pub id: i64,
    pub kind: MemoryKind,
    pub importance: f64,
    /// `Some` only for `kind == Memory`.
    pub memory_type: Option<String>,
    pub tier: i64,
}

const UPDATE_READ_MEMORY: &str = "SELECT tier, importance, memory_type, access_count FROM memory \
     WHERE id = ?1 AND deleted_at IS NULL AND (?2 IS NULL OR agent_id = ?2)";
const UPDATE_READ_EPISODIC: &str = "SELECT tier, importance, 'episodic_trajectory', access_count \
     FROM episodic_trajectory WHERE id = ?1 AND deleted_at IS NULL AND (?2 IS NULL OR agent_id = ?2)";
const UPDATE_READ_PROCEDURAL: &str = "SELECT tier, importance, 'procedural_skill', access_count \
     FROM procedural_skill WHERE id = ?1 AND deleted_at IS NULL AND (?2 IS NULL OR agent_id = ?2)";

const UPDATE_WRITE_MEMORY: &str = "UPDATE memory \
     SET importance = ?1, memory_type = ?2, tier = ?3, last_modified_at = ?4, update_count = update_count + 1 \
     WHERE id = ?5";
const UPDATE_WRITE_EPISODIC: &str = "UPDATE episodic_trajectory \
     SET importance = ?1, tier = ?3, last_modified_at = ?4, update_count = update_count + 1 \
     WHERE id = ?5";
const UPDATE_WRITE_PROCEDURAL: &str = "UPDATE procedural_skill \
     SET importance = ?1, tier = ?3, last_modified_at = ?4, update_count = update_count + 1 \
     WHERE id = ?5";

/// Applies `patch` to the live row `id` of `kind` and returns how it ended
/// up, or `None` when there is no such live row (or, with `owner`, it is
/// another agent's) — in which case nothing is written.
pub async fn update_memory_core(
    store: &MemoryStore,
    id: i64,
    kind: MemoryKind,
    patch: &MemoryPatch,
    owner: Option<&str>,
) -> Result<Option<UpdatedRow>, MemoryError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let (read_sql, write_sql) = match kind {
        MemoryKind::Memory => (UPDATE_READ_MEMORY, UPDATE_WRITE_MEMORY),
        MemoryKind::Episodic => (UPDATE_READ_EPISODIC, UPDATE_WRITE_EPISODIC),
        MemoryKind::Procedural => (UPDATE_READ_PROCEDURAL, UPDATE_WRITE_PROCEDURAL),
    };
    let patch = patch.clone();
    let owner = owner.map(str::to_owned);

    store
        .conn
        .call(move |c| {
            use rusqlite::OptionalExtension;
            let tx = c.transaction()?;
            let current: Option<(i64, f64, String, i64)> = tx
                .query_row(read_sql, rusqlite::params![id, owner], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .optional()?;
            let Some((tier, importance, memory_type, access_count)) = current else {
                return Ok(None);
            };
            let new_importance = patch.importance.unwrap_or(importance);
            let new_type = patch.memory_type.map_or(memory_type.clone(), |t| t.as_str().to_string());
            let new_tier = retier(tier, access_count, (importance, &memory_type), (new_importance, &new_type));
            tx.execute(write_sql, rusqlite::params![new_importance, new_type, new_tier, now_ms, id])?;
            tx.commit()?;
            Ok(Some(UpdatedRow {
                id,
                kind,
                importance: new_importance,
                memory_type: (kind == MemoryKind::Memory).then_some(new_type),
                tier: new_tier,
            }))
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend("connection closed during update_memory_core".into())
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

const OWNED_ELSEWHERE_MEMORY: &str = "SELECT 1 FROM memory WHERE id = ?1 AND agent_id IS NOT ?2";
const OWNED_ELSEWHERE_EPISODIC: &str = "SELECT 1 FROM episodic_trajectory WHERE id = ?1 AND agent_id IS NOT ?2";
const OWNED_ELSEWHERE_PROCEDURAL: &str = "SELECT 1 FROM procedural_skill WHERE id = ?1 AND agent_id IS NOT ?2";

const DELETE_VEC_MEMORY: &str = "DELETE FROM memory_vectors WHERE rowid = ?1";
const DELETE_VEC_EPISODIC: &str = "DELETE FROM episodic_vectors WHERE rowid = ?1";
const DELETE_VEC_PROCEDURAL: &str = "DELETE FROM procedural_vectors WHERE rowid = ?1";

/// Soft-deletes the row `id` in the table selected by `kind` by stamping
/// `deleted_at = now_ms`, and prunes its vector row in the same transaction.
/// Rows of `memory` that disputed it are released in that transaction too.
/// Returns the number of rows affected by the UPDATE (`0` when `id` doesn't
/// exist in that table or was already deleted — idempotent).
///
/// With `owner`, a row that belongs to another agent is answered as missing
/// and the transaction writes nothing, sweeps included: they would otherwise
/// drop that agent's vector.
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
    owner: Option<&str>,
) -> Result<usize, MemoryError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    // Exhaustive match, not string interpolation: a new MemoryKind variant
    // without a table pair here is a compile error, never a runtime SQL
    // injection surface.
    let (row_sql, vec_sql, elsewhere_sql) = match kind {
        MemoryKind::Memory => (DELETE_SQL_MEMORY, DELETE_VEC_MEMORY, OWNED_ELSEWHERE_MEMORY),
        MemoryKind::Episodic => (DELETE_SQL_EPISODIC, DELETE_VEC_EPISODIC, OWNED_ELSEWHERE_EPISODIC),
        MemoryKind::Procedural => (DELETE_SQL_PROCEDURAL, DELETE_VEC_PROCEDURAL, OWNED_ELSEWHERE_PROCEDURAL),
    };
    let owner = owner.map(str::to_owned);

    store
        .conn
        .call(move |c| {
            let tx = c.transaction()?;
            if let Some(owner) = &owner {
                if tx.prepare(elsewhere_sql)?.exists(rusqlite::params![id, owner])? {
                    return Ok(0);
                }
            }
            let affected = tx.execute(row_sql, rusqlite::params![now_ms, id])?;
            tx.execute(vec_sql, rusqlite::params![id])?;
            // Unconditional for the same reason as the vector delete: a row
            // deleted before deletes released its challengers gets them
            // released the next time anyone deletes it again.
            if kind == MemoryKind::Memory {
                release_challengers(&tx, id, TimestampMs::from_unix_millis(now_ms))?;
            }
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
// restore_memory — the way back from `delete_memory`, for `memory` rows: the
// only table production writes, and the only one with a text index, pieces
// and a dedup to keep whole.
// ---------------------------------------------------------------------------

/// Why a row was not restored. Nothing is written in any of these cases.
#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    #[error("only memory rows can be restored, not {0:?} rows")]
    OnlyMemory(MemoryKind),
    /// Missing, live, purged or, with an owner, another agent's: a scoped
    /// caller learns nothing about ids it does not own.
    #[error("no deleted memory row with id {0}")]
    NotFound(i64),
    #[error("memory #{id} was saved again as #{twin}, which is live; #{id} stays deleted so the text is not stored twice")]
    LiveTwin { id: i64, twin: i64 },
    #[error(
        "memory #{id} holds a secret ({}), so it stays deleted: restored, it would reach searches and other agents. \
         `clawg purge {id} --yes` removes it for good",
        .kinds.join(", ")
    )]
    HoldsSecret { id: i64, kinds: Vec<&'static str> },
    #[error("could not embed memory #{0}: {1}")]
    Embed(i64, #[source] EmbedError),
    #[error(transparent)]
    Store(#[from] MemoryError),
}

/// How a restored row ended up. A `superseded` row is live again but still
/// replaced: searches leave it out until the dispute is resolved otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RestoredRow {
    pub id: i64,
    pub status: String,
    pub superseded_by: Option<i64>,
}

/// The deleted row, scoped like the update read, with the head its vector was
/// embedded behind when it is a piece of a long save (as `select_reembed_batch`
/// reads it), so the restored vector is the one the row was saved with.
const RESTORE_READ: &str = "SELECT m.content, \
     (SELECT mp.context FROM memory_piece mp WHERE mp.memory_id = m.id AND mp.context <> '' \
      ORDER BY mp.save_id LIMIT 1) \
     FROM memory m WHERE m.id = ?1 AND m.deleted_at IS NOT NULL AND (?2 IS NULL OR m.agent_id = ?2)";

/// The live row a save of the same text would reinforce instead of storing
/// it again (`REINFORCE_LOOKUP_SQL` in `store_insert.rs`).
const RESTORE_TWIN: &str = "SELECT t.id FROM memory m JOIN memory t \
     ON t.agent_id IS m.agent_id AND COALESCE(t.project_id, '') = COALESCE(m.project_id, '') \
     AND t.content_hash = m.content_hash \
     WHERE m.id = ?1 AND m.content_hash IS NOT NULL AND t.id <> m.id \
     AND t.deleted_at IS NULL AND t.status IN ('active', 'disputed') \
     ORDER BY t.created_at ASC LIMIT 1";

const RESTORE_ROW: &str = "UPDATE memory SET deleted_at = NULL, last_modified_at = ?2, \
     update_count = update_count + 1 WHERE id = ?1 AND deleted_at IS NOT NULL";

/// The row's text index entry, added back in the shape of the insert trigger
/// of V8. V8 has no trigger for this transition because, when it was written,
/// nothing undeleted a row; this statement is that missing half, and it runs
/// only right after `RESTORE_ROW` turned a deleted row live, so every live row
/// stays indexed exactly once.
const RESTORE_TEXT_INDEX: &str = "INSERT INTO memory_fts(rowid, content, agent_id, project_id, memory_type) \
     SELECT id, content, agent_id, project_id, memory_type FROM memory WHERE id = ?1";

/// Brings back the deleted `memory` row `id` with its vector and text index
/// entry. With `owner`, another agent's row is answered as missing.
///
/// The vector is computed before the transaction, from the current embedder;
/// inside it the row is read again, so a restore or a save that landed in
/// between is seen. The row's own dispute is settled against what is live
/// now; the disputes its deletion released stay released.
pub async fn restore_memory_core(
    store: &MemoryStore,
    embedder: &dyn Embedder,
    kind: MemoryKind,
    id: i64,
    owner: Option<&str>,
) -> Result<RestoredRow, RestoreError> {
    if kind != MemoryKind::Memory {
        return Err(RestoreError::OnlyMemory(kind));
    }
    let owner = owner.map(str::to_owned);
    let read_owner = owner.clone();
    let found = store
        .conn
        .call(move |c| {
            use rusqlite::OptionalExtension;
            c.query_row(RESTORE_READ, rusqlite::params![id, read_owner], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .optional()
        })
        .await
        .map_err(backend("restore_memory_core"))?;
    let Some((content, context)) = found else {
        return Err(RestoreError::NotFound(id));
    };

    let kinds: Vec<&'static str> = crate::memory::redact::redact(&content).kinds.iter().map(|k| k.as_str()).collect();
    if !kinds.is_empty() {
        return Err(RestoreError::HoldsSecret { id, kinds });
    }

    let text = embedding_text(context.as_deref().unwrap_or(""), &content);
    let vector = embedder
        .embed_with_role(&text, EmbedRole::Passage)
        .await
        .map_err(|e| RestoreError::Embed(id, e))?;
    let unit = crate::memory::vector::to_unit(&vector)
        .map_err(|e| RestoreError::Embed(id, EmbedError::InferenceFailed(e.to_string())))?;
    let bytes: Vec<u8> = unit.iter().flat_map(|f| f.to_le_bytes()).collect();
    let now = TimestampMs::now();

    enum Outcome {
        Restored(RestoredRow),
        Gone,
        Twin(i64),
    }
    let outcome = store
        .conn
        .call(move |c| {
            use rusqlite::OptionalExtension;
            let tx = c.transaction()?;
            let still_deleted = tx
                .query_row(RESTORE_READ, rusqlite::params![id, owner], |_| Ok(()))
                .optional()?
                .is_some();
            if !still_deleted {
                return Ok(Outcome::Gone);
            }
            if let Some(twin) = tx.query_row(RESTORE_TWIN, [id], |r| r.get(0)).optional()? {
                return Ok(Outcome::Twin(twin));
            }
            tx.execute(RESTORE_ROW, rusqlite::params![id, now])?;
            tx.execute(DELETE_VEC_MEMORY, [id])?;
            tx.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?1, ?2)", rusqlite::params![id, bytes])?;
            tx.execute(RESTORE_TEXT_INDEX, [id])?;
            settle_row(&tx, id, now)?;
            let row = tx.query_row("SELECT status, superseded_by FROM memory WHERE id = ?1", [id], |r| {
                Ok(RestoredRow { id, status: r.get(0)?, superseded_by: r.get(1)? })
            })?;
            tx.commit()?;
            Ok(Outcome::Restored(row))
        })
        .await
        .map_err(backend("restore_memory_core"))?;
    match outcome {
        Outcome::Restored(row) => Ok(row),
        Outcome::Gone => Err(RestoreError::NotFound(id)),
        Outcome::Twin(twin) => Err(RestoreError::LiveTwin { id, twin }),
    }
}

fn backend(context: &'static str) -> impl Fn(tokio_rusqlite::Error) -> MemoryError {
    move |e| match e {
        tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
        tokio_rusqlite::Error::ConnectionClosed => MemoryError::Backend(format!("connection closed during {context}")),
        other => MemoryError::Backend(other.to_string()),
    }
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
    // Provenance. Only `memory` rows carry it: the viewer shows where a stored
    // fact came from, which the other two tables derive rather than record.
    pub source: Option<String>,
    pub source_session_id: Option<String>,
    pub created_by_type: Option<String>,
    pub created_by_ref: Option<String>,
    pub last_modified_at: Option<i64>,
    pub update_count: Option<i64>,
    pub dispute_count: Option<i64>,
    /// Raw JSON text of the ids this row disputes.
    pub disputed_against: Option<String>,
    pub last_disputed_at: Option<i64>,
    /// The `memory` rows this one was drawn from, ascending. Only a
    /// conclusion has them; every other row omits the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<Vec<i64>>,
}

/// Filter/pagination params for `list_memories_core`. `kind: None` merges
/// all 3 tables; `memory_type` and `query` are no-ops (silently ignored) on
/// non-`Memory` kinds, the only table with a text index. `agent_id`,
/// `project_id` and `status` are exact matches applied to every table.
///
/// Unlike retrieval, `agent_id` is not a visibility rule: this is the owner's
/// inspection tool, so it filters instead of scoping.
#[derive(Debug, Clone, Default)]
pub struct ListParams {
    pub kind: Option<MemoryKind>,
    pub tier: Option<i64>,
    pub memory_type: Option<String>,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub status: Option<String>,
    /// Free text over `memory.content`: every token must match, each as a
    /// prefix, ignoring case and accents. A query with no token filters nothing.
    pub query: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// Whether `query` holds anything `ListParams::query` can search for. A query
/// that does not (only punctuation, only spaces) filters nothing, so a caller
/// that cares can refuse it instead of showing an unfiltered list as a result.
pub fn query_has_tokens(query: &str) -> bool {
    viewer_match_expression(query).is_some()
}

/// `ListParams` after validation and MATCH-expression building, shaped for
/// the SQL builders. Every value reaches SQL as a bound parameter.
#[derive(Default)]
struct RowFilter {
    tier: Option<i64>,
    memory_type: Option<String>,
    agent_id: Option<String>,
    project_id: Option<String>,
    status: Option<String>,
    match_expr: Option<String>,
}

/// Appends the filters every table understands, then the `memory`-only ones
/// when `memory_table` is set. Pure: no I/O, so the SQL shape is testable
/// without a connection.
fn push_filters(
    sql: &mut String,
    params: &mut Vec<rusqlite::types::Value>,
    f: &RowFilter,
    memory_table: bool,
) {
    use rusqlite::types::Value;
    if let Some(t) = f.tier {
        sql.push_str(" AND tier = ?");
        params.push(Value::Integer(t));
    }
    if let Some(a) = &f.agent_id {
        sql.push_str(" AND agent_id = ?");
        params.push(Value::Text(a.clone()));
    }
    if let Some(p) = &f.project_id {
        sql.push_str(" AND project_id = ?");
        params.push(Value::Text(p.clone()));
    }
    if let Some(s) = &f.status {
        sql.push_str(" AND status = ?");
        params.push(Value::Text(s.clone()));
    }
    if !memory_table {
        return;
    }
    if let Some(mt) = &f.memory_type {
        sql.push_str(" AND memory_type = ?");
        params.push(Value::Text(mt.clone()));
    }
    if let Some(expr) = &f.match_expr {
        // No LIMIT inside: a cap here would drop matches before the live,
        // filtered rows are chosen and make the page depend on index order.
        sql.push_str(" AND id IN (SELECT rowid FROM memory_fts WHERE memory_fts MATCH ?)");
        params.push(Value::Text(expr.clone()));
    }
}

/// Lists rows from `memory` / `episodic_trajectory` / `procedural_skill`
/// directly. `kind: None` runs all 3 per-table queries inside the
/// SAME `conn.call` and merges + sorts (`created_at DESC`) + paginates in
/// Rust; `kind: Some(_)` pushes `LIMIT`/`OFFSET` down to SQL directly.
/// `deleted_at IS NULL` is always applied; `status` is only filtered when
/// asked — `superseded` rows stay visible by default (inspection is an active
/// inventory with superseded shown, distinct from retrieval's stricter filter).
pub async fn list_memories_core(
    store: &MemoryStore,
    params: ListParams,
) -> Result<Vec<MemoryAdminRow>, MemoryError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 100) as i64;
    let offset = params.offset.unwrap_or(0) as i64;
    let kind = params.kind;
    let filter = RowFilter {
        tier: params.tier,
        memory_type: params.memory_type,
        agent_id: params.agent_id,
        project_id: params.project_id,
        status: params.status,
        match_expr: params.query.as_deref().and_then(viewer_match_expression),
    };

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
                rows.extend(query_memory_rows(c, &filter, per_table_limit, per_table_offset)?);
            }
            if want_episodic {
                rows.extend(query_episodic_rows(c, &filter, per_table_limit, per_table_offset)?);
            }
            if want_procedural {
                rows.extend(query_procedural_rows(c, &filter, per_table_limit, per_table_offset)?);
            }

            if is_merge {
                rows.sort_by(|a, b| {
                    b.created_at.cmp(&a.created_at).then_with(|| b.id.cmp(&a.id))
                });
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

// Column order here is the contract with the `map_*_row` functions below.
pub(crate) const SELECT_MEMORY: &str = "SELECT id, scope, agent_id, project_id, memory_type, content, \
     tier, importance, status, access_count, created_at, last_accessed_at, superseded_by, \
     deleted_at, source, source_session_id, created_by_type, created_by_ref, \
     last_modified_at, update_count, dispute_count, disputed_against, last_disputed_at, \
     (SELECT json_group_array(source_id) FROM memory_derivation WHERE memory_id = memory.id) \
     FROM memory";
const SELECT_EPISODIC: &str = "SELECT id, scope, agent_id, project_id, task, tier, importance, \
     status, access_count, created_at, last_accessed_at, superseded_by, deleted_at \
     FROM episodic_trajectory";
const SELECT_PROCEDURAL: &str = "SELECT id, scope, agent_id, project_id, skill_name, tier, \
     importance, status, access_count, created_at, last_accessed_at, superseded_by, \
     deleted_at, success_count, trigger, source_trajectory_ids \
     FROM procedural_skill";

/// Reads one row by `(kind, id)` for inspection. Unlike the list it also
/// returns soft-deleted rows, reporting them through `deleted_at`: someone
/// holding an id from an earlier listing should learn the row was deleted,
/// not that it never existed. Read-only on purpose — it records no access, so
/// looking at a row never promotes it.
pub async fn get_memory_core(
    store: &MemoryStore,
    kind: MemoryKind,
    id: i64,
) -> Result<Option<MemoryAdminRow>, MemoryError> {
    store
        .conn
        .call(move |c| -> rusqlite::Result<Option<MemoryAdminRow>> {
            use rusqlite::OptionalExtension;
            match kind {
                MemoryKind::Memory => c
                    .query_row(&format!("{SELECT_MEMORY} WHERE id = ?1"), [id], map_memory_row)
                    .optional(),
                MemoryKind::Episodic => c
                    .query_row(
                        &format!("{SELECT_EPISODIC} WHERE id = ?1"),
                        [id],
                        map_episodic_row,
                    )
                    .optional(),
                MemoryKind::Procedural => c
                    .query_row(
                        &format!("{SELECT_PROCEDURAL} WHERE id = ?1"),
                        [id],
                        map_procedural_row,
                    )
                    .optional(),
            }
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend("connection closed during get_memory_core".into())
            }
            other => MemoryError::Backend(other.to_string()),
        })
}

fn query_memory_rows(
    c: &rusqlite::Connection,
    filter: &RowFilter,
    limit: i64,
    offset: i64,
) -> rusqlite::Result<Vec<MemoryAdminRow>> {
    let mut sql = format!("{SELECT_MEMORY} WHERE deleted_at IS NULL");
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    push_filters(&mut sql, &mut params, filter, true);
    sql.push_str(" ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?");
    params.push(rusqlite::types::Value::Integer(limit));
    params.push(rusqlite::types::Value::Integer(offset));

    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), map_memory_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub(crate) fn map_memory_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryAdminRow> {
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
        source: Some(row.get(14)?),
        source_session_id: row.get(15)?,
        created_by_type: Some(row.get(16)?),
        created_by_ref: Some(row.get(17)?),
        last_modified_at: Some(row.get(18)?),
        update_count: Some(row.get(19)?),
        dispute_count: Some(row.get(20)?),
        disputed_against: row.get(21)?,
        last_disputed_at: row.get(22)?,
        derived_from: sources_from_json(&row.get::<_, String>(23)?),
    })
}

/// Reads the `json_group_array` of a row's sources: `None` for a row with
/// none, so only a conclusion carries the field. Sorted here because an
/// aggregate's order is not something SQLite promises.
fn sources_from_json(json: &str) -> Option<Vec<i64>> {
    let mut ids: Vec<i64> = serde_json::from_str(json).unwrap_or_default();
    ids.sort_unstable();
    Some(ids).filter(|ids| !ids.is_empty())
}

fn query_episodic_rows(
    c: &rusqlite::Connection,
    filter: &RowFilter,
    limit: i64,
    offset: i64,
) -> rusqlite::Result<Vec<MemoryAdminRow>> {
    let mut sql = format!("{SELECT_EPISODIC} WHERE deleted_at IS NULL");
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    push_filters(&mut sql, &mut params, filter, false);
    sql.push_str(" ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?");
    params.push(rusqlite::types::Value::Integer(limit));
    params.push(rusqlite::types::Value::Integer(offset));

    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), map_episodic_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn map_episodic_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryAdminRow> {
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
        source: None,
        source_session_id: None,
        created_by_type: None,
        created_by_ref: None,
        last_modified_at: None,
        update_count: None,
        dispute_count: None,
        disputed_against: None,
        last_disputed_at: None,
        derived_from: None,
    })
}

fn query_procedural_rows(
    c: &rusqlite::Connection,
    filter: &RowFilter,
    limit: i64,
    offset: i64,
) -> rusqlite::Result<Vec<MemoryAdminRow>> {
    let mut sql = format!("{SELECT_PROCEDURAL} WHERE deleted_at IS NULL");
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    push_filters(&mut sql, &mut params, filter, false);
    sql.push_str(" ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?");
    params.push(rusqlite::types::Value::Integer(limit));
    params.push(rusqlite::types::Value::Integer(offset));

    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), map_procedural_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn map_procedural_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryAdminRow> {
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
        source: None,
        source_session_id: None,
        created_by_type: None,
        created_by_ref: None,
        last_modified_at: None,
        update_count: None,
        dispute_count: None,
        disputed_against: None,
        last_disputed_at: None,
        derived_from: None,
    })
}

// ---------------------------------------------------------------------------
// memory_stats — what is stored, at a glance. Live rows only, with the
// soft-deleted ones reported apart so a total is never silently inflated by
// rows nothing can return any more.
// ---------------------------------------------------------------------------

/// Optional narrowing applied to every figure in [`MemoryStats`].
#[derive(Debug, Clone, Default)]
pub struct StatsParams {
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
}

/// One bucket of a grouping. `key: None` is the NULL bucket (rows with no
/// project, for instance), kept distinct from any real value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Count {
    pub key: Option<String>,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatsTotals {
    pub memory: u64,
    pub episodic: u64,
    pub procedural: u64,
    /// Soft-deleted rows across the three tables; not part of the live counts.
    pub deleted: u64,
}

/// Snapshot of the stores. The groupings cover `memory` only: type and the
/// dispute lifecycle are not columns of the other two tables.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryStats {
    pub totals: StatsTotals,
    pub by_type: Vec<Count>,
    pub by_tier: Vec<Count>,
    pub by_status: Vec<Count>,
    pub by_agent: Vec<Count>,
    pub by_project: Vec<Count>,
    /// Newest `created_at` (ms) among live rows of any table.
    pub newest_created_at: Option<i64>,
}

/// The `memory` column a grouping is computed over. An enum with an exhaustive
/// match, so the SQL fragment is always one of these literals and never text
/// that came from a caller.
#[derive(Clone, Copy)]
enum GroupBy {
    Type,
    Tier,
    Status,
    Agent,
    Project,
}

impl GroupBy {
    fn column(self) -> &'static str {
        match self {
            GroupBy::Type => "memory_type",
            GroupBy::Tier => "CAST(tier AS TEXT)",
            GroupBy::Status => "status",
            GroupBy::Agent => "agent_id",
            GroupBy::Project => "project_id",
        }
    }
}

const STATS_TABLES: [&str; 3] = ["memory", "episodic_trajectory", "procedural_skill"];

fn stats_where(deleted: bool, filter: &RowFilter) -> (String, Vec<rusqlite::types::Value>) {
    let mut sql = String::from(if deleted {
        " WHERE deleted_at IS NOT NULL"
    } else {
        " WHERE deleted_at IS NULL"
    });
    let mut params = Vec::new();
    push_filters(&mut sql, &mut params, filter, false);
    (sql, params)
}

fn count_rows(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    deleted: bool,
    filter: &RowFilter,
) -> rusqlite::Result<u64> {
    let (clause, params) = stats_where(deleted, filter);
    tx.query_row(
        &format!("SELECT COUNT(*) FROM {table}{clause}"),
        rusqlite::params_from_iter(params),
        |r| r.get(0),
    )
}

fn group_memory(
    tx: &rusqlite::Transaction<'_>,
    by: GroupBy,
    filter: &RowFilter,
) -> rusqlite::Result<Vec<Count>> {
    let (clause, params) = stats_where(false, filter);
    let col = by.column();
    let mut stmt = tx.prepare(&format!(
        "SELECT {col}, COUNT(*) FROM memory{clause} GROUP BY 1 ORDER BY 2 DESC, 1 ASC"
    ))?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), |r| {
        Ok(Count { key: r.get(0)?, count: r.get(1)? })
    })?;
    rows.collect()
}

/// Computes [`MemoryStats`] inside a single transaction so the totals and the
/// groupings describe the same instant even while the ingest worker writes.
pub async fn memory_stats_core(
    store: &MemoryStore,
    params: StatsParams,
) -> Result<MemoryStats, MemoryError> {
    let filter = RowFilter {
        agent_id: params.agent_id,
        project_id: params.project_id,
        ..Default::default()
    };

    store
        .conn
        .call(move |c| -> rusqlite::Result<MemoryStats> {
            let tx = c.transaction()?;
            let mut live = [0u64; 3];
            let mut deleted = 0;
            let mut newest: Option<i64> = None;
            for (slot, table) in live.iter_mut().zip(STATS_TABLES) {
                *slot = count_rows(&tx, table, false, &filter)?;
                deleted += count_rows(&tx, table, true, &filter)?;
                let (clause, params) = stats_where(false, &filter);
                let max: Option<i64> = tx.query_row(
                    &format!("SELECT MAX(created_at) FROM {table}{clause}"),
                    rusqlite::params_from_iter(params),
                    |r| r.get(0),
                )?;
                newest = newest.max(max);
            }
            let stats = MemoryStats {
                totals: StatsTotals {
                    memory: live[0],
                    episodic: live[1],
                    procedural: live[2],
                    deleted,
                },
                by_type: group_memory(&tx, GroupBy::Type, &filter)?,
                by_tier: group_memory(&tx, GroupBy::Tier, &filter)?,
                by_status: group_memory(&tx, GroupBy::Status, &filter)?,
                by_agent: group_memory(&tx, GroupBy::Agent, &filter)?,
                by_project: group_memory(&tx, GroupBy::Project, &filter)?,
                newest_created_at: newest,
            };
            tx.commit()?;
            Ok(stats)
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend("connection closed during memory_stats_core".into())
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
            agent_id: None,
            piece: None,
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

    // ── update_memory_core ───────────────────────────────────────────────────
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

    fn importance(v: f64) -> MemoryPatch {
        MemoryPatch::new(MemoryKind::Memory, Some(v), None).unwrap()
    }

    async fn memory_columns(store: &MemoryStore, id: i64) -> (String, i64, i64, String, i64, i64) {
        store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT memory_type, update_count, last_modified_at, status, access_count, last_accessed_at \
                     FROM memory WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
            })
            .await
            .unwrap()
    }

    #[test]
    fn a_change_needs_at_least_one_field() {
        assert_eq!(MemoryPatch::new(MemoryKind::Memory, None, None), Err(PatchError::Empty));
    }

    #[test]
    fn importance_outside_the_scale_is_refused() {
        for v in [-0.5, 10.5, 50.0, f64::NAN, f64::INFINITY] {
            let err = MemoryPatch::new(MemoryKind::Memory, Some(v), None).unwrap_err();
            assert!(matches!(err, PatchError::ImportanceOutOfScale(_)), "{v}: {err:?}");
            assert!(err.to_string().contains("importance"), "{err}");
        }
        for v in [0.0, 10.0, 7.5] {
            assert!(MemoryPatch::new(MemoryKind::Memory, Some(v), None).is_ok(), "{v}");
        }
    }

    #[test]
    fn an_unknown_memory_type_is_refused() {
        let err = MemoryPatch::new(MemoryKind::Memory, None, Some("preference")).unwrap_err();
        assert_eq!(err, PatchError::UnknownType("preference".into()));
        assert!(err.to_string().contains("memory_type"), "{err}");
        assert!(MemoryPatch::new(MemoryKind::Memory, None, Some("user_preference")).is_ok());
    }

    #[test]
    fn only_memory_rows_have_a_type_to_change() {
        for kind in [MemoryKind::Episodic, MemoryKind::Procedural] {
            let err = MemoryPatch::new(kind, Some(5.0), Some("user_preference")).unwrap_err();
            assert_eq!(err, PatchError::TypeOnlyOnMemory(kind));
            assert!(err.to_string().contains("memory_type"), "{err}");
            assert!(MemoryPatch::new(kind, Some(5.0), None).is_ok());
        }
    }

    #[tokio::test]
    async fn update_with_high_importance_promotes_to_tier_1() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row(&store, "architectural_decision", 3, 5.0, false).await;

        let row = update_memory_core(&store, id, MemoryKind::Memory, &importance(9.0), None)
            .await
            .unwrap()
            .expect("updated");

        assert_eq!(row.tier, 1, "importance >= 9.0 must promote to tier 1");
        assert_eq!(memory_tier_importance(&store, id).await, (1, 9.0));
    }

    #[tokio::test]
    async fn lowering_importance_takes_away_the_tier_it_gave() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row(&store, "chat_message", 1, 9.5, false).await;

        let row = update_memory_core(&store, id, MemoryKind::Memory, &importance(2.0), None)
            .await
            .unwrap()
            .expect("updated");

        assert_eq!(row.tier, 3, "the importance was all that held tier 1");
        assert_eq!(memory_tier_importance(&store, id).await, (3, 2.0));
    }

    #[tokio::test]
    async fn an_update_writes_what_was_asked_and_counts_itself() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row(&store, "semantic_fact", 3, 5.0, false).await;
        let patch = MemoryPatch::new(MemoryKind::Memory, Some(8.0), Some("user_preference")).unwrap();

        let row = update_memory_core(&store, id, MemoryKind::Memory, &patch, None).await.unwrap();

        assert_eq!(
            row,
            Some(UpdatedRow {
                id,
                kind: MemoryKind::Memory,
                importance: 8.0,
                memory_type: Some("user_preference".into()),
                tier: 1,
            })
        );
        let (memory_type, update_count, last_modified_at, ..) = memory_columns(&store, id).await;
        assert_eq!((memory_type.as_str(), update_count), ("user_preference", 1));
        assert!(last_modified_at > 0, "the seed wrote 0");

        let type_only = MemoryPatch::new(MemoryKind::Memory, None, Some("semantic_fact")).unwrap();
        let row = update_memory_core(&store, id, MemoryKind::Memory, &type_only, None).await.unwrap().unwrap();
        assert_eq!(row.importance, 8.0, "a field not asked for stays as it was");
        assert_eq!(memory_columns(&store, id).await.1, 2);
    }

    #[tokio::test]
    async fn an_update_leaves_status_accesses_and_disputes_alone() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let old = seed_memory_scoped(&store, "old", "a", None, "active").await;
        let id = seed_disputing(&store, &[old]).await;
        let before = memory_columns(&store, id).await;
        async fn disputed_against(store: &MemoryStore, id: i64) -> Option<String> {
            store
                .conn
                .call(move |c| {
                    c.query_row("SELECT disputed_against FROM memory WHERE id = ?1", [id], |r| r.get(0))
                })
                .await
                .unwrap()
        }
        let disputes_before = disputed_against(&store, id).await;

        update_memory_core(&store, id, MemoryKind::Memory, &importance(7.0), None).await.unwrap().unwrap();

        let after = memory_columns(&store, id).await;
        assert_eq!(after.3, before.3, "status");
        assert_eq!((after.4, after.5), (before.4, before.5), "accesses");
        assert!(disputes_before.is_some(), "the seed disputes a row");
        assert_eq!(disputed_against(&store, id).await, disputes_before);
    }

    #[tokio::test]
    async fn update_on_episodic_has_no_type_rule_below_importance_9() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_episodic_row(&store, 3, 5.0).await;
        let patch = MemoryPatch::new(MemoryKind::Episodic, Some(7.0), None).unwrap();

        let row = update_memory_core(&store, id, MemoryKind::Episodic, &patch, None).await.unwrap().unwrap();

        assert_eq!((row.tier, row.memory_type), (3, None));
        assert_eq!(episodic_tier_importance(&store, id).await, (3, 7.0));
    }

    #[tokio::test]
    async fn update_on_procedural_keeps_the_tier_its_table_gives() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_procedural_row(&store, 1, 8.0).await;
        let patch = MemoryPatch::new(MemoryKind::Procedural, Some(3.0), None).unwrap();

        update_memory_core(&store, id, MemoryKind::Procedural, &patch, None).await.unwrap().unwrap();

        assert_eq!(procedural_tier_importance(&store, id).await, (1, 3.0));
    }

    #[tokio::test]
    async fn update_on_a_missing_id_changes_nothing() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let row = update_memory_core(&store, 999_999, MemoryKind::Memory, &importance(9.0), None).await.unwrap();

        assert_eq!(row, None);
    }

    #[tokio::test]
    async fn update_on_a_soft_deleted_row_changes_nothing() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row(&store, "chat_message", 3, 5.0, true).await;

        let row = update_memory_core(&store, id, MemoryKind::Memory, &importance(9.0), None).await.unwrap();

        assert_eq!(row, None, "deleted rows must not be updated");
        assert_eq!(memory_tier_importance(&store, id).await, (3, 5.0));
        assert_eq!(memory_columns(&store, id).await.1, 0, "update_count");
    }

    // Rows as the current version writes them, and as the desktop app left
    // them: tiers no rule of today awards, which a change must not take.
    #[tokio::test]
    async fn rows_from_the_previous_version_can_be_updated() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let earned = seed_memory_row(&store, "semantic_fact", 1, 9.5, false).await;
        let foreign = seed_memory_row(&store, "semantic_fact", 1, 5.0, false).await;
        let epi = seed_episodic_row(&store, 1, 5.0).await;
        let pro = seed_procedural_row(&store, 2, 5.0).await;

        let lower = |kind| MemoryPatch::new(kind, Some(2.0), None).unwrap();
        let tier_of = |row: Option<UpdatedRow>| row.expect("updated").tier;
        assert_eq!(tier_of(update_memory_core(&store, earned, MemoryKind::Memory, &lower(MemoryKind::Memory), None).await.unwrap()), 3);
        assert_eq!(tier_of(update_memory_core(&store, foreign, MemoryKind::Memory, &lower(MemoryKind::Memory), None).await.unwrap()), 1);
        assert_eq!(tier_of(update_memory_core(&store, epi, MemoryKind::Episodic, &lower(MemoryKind::Episodic), None).await.unwrap()), 1);
        assert_eq!(tier_of(update_memory_core(&store, pro, MemoryKind::Procedural, &lower(MemoryKind::Procedural), None).await.unwrap()), 1);

        assert_eq!(memory_tier_importance(&store, foreign).await, (1, 2.0));
        assert_eq!(episodic_tier_importance(&store, epi).await, (1, 2.0));
        assert_eq!(procedural_tier_importance(&store, pro).await, (1, 2.0));
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

        let affected = delete_memory_core(&store, id, MemoryKind::Memory, None)
            .await
            .unwrap();

        assert_eq!(affected, 1);
        assert!(
            memory_deleted_at(&store, id).await.is_some(),
            "deleted_at must be set after delete"
        );
    }

    async fn seed_disputing(store: &MemoryStore, disputes: &[i64]) -> i64 {
        let id = seed_memory_scoped(store, "challenger", "a", None, "disputed").await;
        let disputes = serde_json::to_string(disputes).unwrap();
        store
            .conn
            .call(move |c| {
                c.execute("UPDATE memory SET disputed_against = ?1 WHERE id = ?2", rusqlite::params![disputes, id])
            })
            .await
            .unwrap();
        id
    }

    async fn memory_status(store: &MemoryStore, id: i64) -> String {
        store
            .conn
            .call(move |c| c.query_row("SELECT status FROM memory WHERE id = ?1", [id], |r| r.get(0)))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn deleting_the_disputed_row_releases_its_challenger() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let old = seed_memory_scoped(&store, "old", "a", None, "active").await;
        let new = seed_disputing(&store, &[old]).await;

        delete_memory_core(&store, old, MemoryKind::Memory, None).await.unwrap();

        assert_eq!(memory_status(&store, new).await, "active");
    }

    #[tokio::test]
    async fn deleting_one_of_two_keeps_the_dispute() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let first = seed_memory_scoped(&store, "first", "a", None, "active").await;
        let second = seed_memory_scoped(&store, "second", "a", None, "active").await;
        let new = seed_disputing(&store, &[first, second]).await;

        delete_memory_core(&store, first, MemoryKind::Memory, None).await.unwrap();

        assert_eq!(memory_status(&store, new).await, "disputed");
    }

    /// Another agent's row is answered as missing, and nothing about it moves:
    /// not the row, not its vector, not the disputes that name it.
    #[tokio::test]
    async fn an_owner_mismatch_deletes_nothing() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mem = seed_memory_row_with_vector(&store).await;
        let epi = seed_episodic_row_with_vector(&store).await;
        let pro = seed_procedural_row_with_vector(&store).await;
        let challenger = seed_disputing(&store, &[mem]).await;

        for (id, kind) in [(mem, MemoryKind::Memory), (epi, MemoryKind::Episodic), (pro, MemoryKind::Procedural)] {
            assert_eq!(delete_memory_core(&store, id, kind, Some("other-agent")).await.unwrap(), 0);
        }

        assert_eq!(memory_deleted_at(&store, mem).await, None);
        assert_eq!(episodic_deleted_at(&store, epi).await, None);
        assert_eq!(vector_row_count(&store, "memory_vectors", mem).await, 1);
        assert_eq!(vector_row_count(&store, "episodic_vectors", epi).await, 1);
        assert_eq!(vector_row_count(&store, "procedural_vectors", pro).await, 1);
        assert_eq!(memory_status(&store, challenger).await, "disputed");

        assert_eq!(delete_memory_core(&store, mem, MemoryKind::Memory, Some("test-agent")).await.unwrap(), 1);
        assert_eq!(vector_row_count(&store, "memory_vectors", mem).await, 0);
        assert_eq!(memory_status(&store, challenger).await, "active");
    }

    #[tokio::test]
    async fn an_owner_mismatch_changes_no_importance() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mem = seed_memory_row(&store, "chat_message", 3, 5.0, false).await;
        let epi = seed_episodic_row(&store, 3, 5.0).await;
        let pro = seed_procedural_row(&store, 3, 5.0).await;

        for (id, kind) in [(mem, MemoryKind::Memory), (epi, MemoryKind::Episodic), (pro, MemoryKind::Procedural)] {
            let patch = MemoryPatch::new(kind, Some(9.5), None).unwrap();
            assert_eq!(update_memory_core(&store, id, kind, &patch, Some("other-agent")).await.unwrap(), None);
        }

        assert_eq!(memory_tier_importance(&store, mem).await, (3, 5.0));
        assert_eq!(episodic_tier_importance(&store, epi).await, (3, 5.0));
        assert_eq!(procedural_tier_importance(&store, pro).await, (3, 5.0));
        assert_eq!(memory_columns(&store, mem).await.1, 0, "update_count");

        let row = update_memory_core(&store, mem, MemoryKind::Memory, &importance(9.5), Some("test-agent")).await.unwrap();
        assert_eq!(row.map(|r| r.importance), Some(9.5));
    }

    #[tokio::test]
    async fn deleting_an_episodic_row_leaves_memory_disputes_alone() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let old = seed_memory_scoped(&store, "old", "a", None, "active").await;
        let new = seed_disputing(&store, &[old]).await;

        delete_memory_core(&store, old, MemoryKind::Episodic, None).await.unwrap();

        assert_eq!(memory_status(&store, new).await, "disputed", "ids are per table");
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

        let affected = delete_memory_core(&store, epi_id, MemoryKind::Episodic, None)
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

        let first = delete_memory_core(&store, id, MemoryKind::Memory, None)
            .await
            .unwrap();
        let second = delete_memory_core(&store, id, MemoryKind::Memory, None)
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

        delete_memory_core(&store, id, MemoryKind::Memory, None)
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
        delete_memory_core(&store, mem_id, MemoryKind::Memory, None).await.unwrap();
        assert_eq!(
            vector_row_count(&store, "memory_vectors", mem_id).await,
            0,
            "memory_vectors row must be gone after soft-delete"
        );

        let epi_id = seed_episodic_row_with_vector(&store).await;
        delete_memory_core(&store, epi_id, MemoryKind::Episodic, None).await.unwrap();
        assert_eq!(
            vector_row_count(&store, "episodic_vectors", epi_id).await,
            0,
            "episodic_vectors row must be gone after soft-delete"
        );

        let proc_id = seed_procedural_row_with_vector(&store).await;
        delete_memory_core(&store, proc_id, MemoryKind::Procedural, None).await.unwrap();
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

        let affected = delete_memory_core(&store, id, MemoryKind::Memory, None).await.unwrap();

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
            agent_id: None,
            piece: None,
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

    async fn link(store: &MemoryStore, memory_id: i64, source_id: i64) {
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory_derivation (memory_id, source_id, created_at) VALUES (?1, ?2, 0)",
                    [memory_id, source_id],
                )
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn the_listing_names_the_sources_of_a_conclusion() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = seed_memory_full(&store, "semantic_fact", 3, "active", false, 1).await;
        let b = seed_memory_full(&store, "semantic_fact", 3, "active", false, 2).await;
        let c = seed_memory_full(&store, "user_preference", 1, "active", false, 3).await;
        link(&store, c, b).await;
        link(&store, c, a).await;

        let rows = list_memories_core(&store, ListParams::default()).await.unwrap();
        let sources = |id: i64| rows.iter().find(|r| r.id == id).unwrap().derived_from.clone();
        assert_eq!(sources(c), Some(vec![a, b]));
        assert_eq!(sources(a), None);

        let one = get_memory_core(&store, MemoryKind::Memory, c).await.unwrap().unwrap();
        assert_eq!(one.derived_from, Some(vec![a, b]));
        let json = serde_json::to_value(&rows).unwrap();
        let plain = json.as_array().unwrap().iter().find(|r| r["id"] == a).unwrap();
        assert!(plain.get("derived_from").is_none(), "a row with no sources omits the field: {plain}");
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

    // Fragments of one save share `created_at`; without a tiebreak the page
    // boundaries between equal timestamps are undefined.
    #[tokio::test]
    async fn list_memories_ties_on_created_at_order_by_id_desc() {
        for kind in [Some(MemoryKind::Memory), None] {
            let store = MemoryStore::new_for_test().await.unwrap();
            let mut ids = Vec::new();
            for _ in 0..5 {
                ids.push(seed_memory_full(&store, "chat_message", 3, "active", false, 100).await);
            }
            let expected: Vec<i64> = ids.iter().rev().copied().collect();

            let all = list_memories_core(&store, ListParams { kind, ..Default::default() })
                .await
                .unwrap();
            assert_eq!(all.iter().map(|r| r.id).collect::<Vec<_>>(), expected, "kind={kind:?}");

            let mut paged = Vec::new();
            for offset in [0u32, 2, 4] {
                let page = list_memories_core(
                    &store,
                    ListParams { kind, limit: Some(2), offset: Some(offset), ..Default::default() },
                )
                .await
                .unwrap();
                paged.extend(page.iter().map(|r| r.id));
            }
            assert_eq!(paged, expected, "pages must neither overlap nor skip, kind={kind:?}");
        }
    }

    #[tokio::test]
    async fn list_memories_memory_row_carries_provenance() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .conn
            .call(|c| {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, source, source_session_id, \
                      created_at, created_by_type, created_by_ref, last_modified_at, \
                      update_count, last_accessed_at, dispute_count, disputed_against, \
                      last_disputed_at, status) \
                     VALUES ('agent', 'a1', 'body', 'observation', 'explicit', 'sess-9', \
                             10, 'user', 'marco', 20, 3, 30, 2, '[7,8]', 40, 'disputed')",
                    [],
                )
            })
            .await
            .unwrap();

        let rows = list_memories_core(&store, ListParams::default()).await.unwrap();

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.source.as_deref(), Some("explicit"));
        assert_eq!(row.source_session_id.as_deref(), Some("sess-9"));
        assert_eq!(row.created_by_type.as_deref(), Some("user"));
        assert_eq!(row.created_by_ref.as_deref(), Some("marco"));
        assert_eq!(row.last_modified_at, Some(20));
        assert_eq!(row.update_count, Some(3));
        assert_eq!(row.dispute_count, Some(2));
        assert_eq!(row.disputed_against.as_deref(), Some("[7,8]"));
        assert_eq!(row.last_disputed_at, Some(40));
    }

    #[tokio::test]
    async fn episodic_row_has_no_source() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_episodic_row(&store, 3, 5.0).await;
        seed_procedural_row(&store, 1, 8.0).await;

        let rows = list_memories_core(&store, ListParams::default()).await.unwrap();

        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(row.source, None, "{row:?}");
            assert_eq!(row.source_session_id, None, "{row:?}");
            assert_eq!(row.created_by_type, None, "{row:?}");
            assert_eq!(row.created_by_ref, None, "{row:?}");
            assert_eq!(row.last_modified_at, None, "{row:?}");
            assert_eq!(row.update_count, None, "{row:?}");
            assert_eq!(row.dispute_count, None, "{row:?}");
            assert_eq!(row.disputed_against, None, "{row:?}");
            assert_eq!(row.last_disputed_at, None, "{row:?}");
        }
    }

    // ── list_memories_core filters ──────────────────

    async fn seed_memory_scoped(
        store: &MemoryStore,
        content: &str,
        agent: &str,
        project: Option<&str>,
        status: &str,
    ) -> i64 {
        let (content, agent, project, status) = (
            content.to_string(),
            agent.to_string(),
            project.map(str::to_string),
            status.to_string(),
        );
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, project_id, content, memory_type, status, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', ?, ?, ?, 'observation', ?, 0, 0, 0, 'agent', '')",
                    rusqlite::params![agent, project, content, status],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .expect("seed_memory_scoped failed")
    }

    async fn seed_episodic_scoped(store: &MemoryStore, agent: &str, task: &str) {
        let nonce = NONCE.fetch_add(1, Ordering::SeqCst);
        let (agent, task) = (agent.to_string(), task.to_string());
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO episodic_trajectory \
                     (scope, agent_id, session_id, task, created_at, last_modified_at, \
                      last_accessed_at, created_by_type, created_by_ref) \
                     VALUES ('agent', ?, ?, ?, 0, 0, 0, 'agent', '')",
                    rusqlite::params![agent, format!("sess-scoped-{nonce}"), task],
                )
            })
            .await
            .expect("seed_episodic_scoped failed");
    }

    async fn seed_procedural_scoped(store: &MemoryStore, agent: &str) {
        let nonce = NONCE.fetch_add(1, Ordering::SeqCst);
        let agent = agent.to_string();
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO procedural_skill \
                     (scope, agent_id, skill_name, trigger, promoted_at, \
                      source_trajectory_ids, created_at, last_modified_at, \
                      last_accessed_at, created_by_type, created_by_ref) \
                     VALUES ('agent', ?, ?, 'trig', 0, '[]', 0, 0, 0, 'agent', '')",
                    rusqlite::params![agent, format!("skill-scoped-{nonce}")],
                )
            })
            .await
            .expect("seed_procedural_scoped failed");
    }

    fn ids_of(rows: &[MemoryAdminRow]) -> Vec<i64> {
        let mut ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        ids.sort_unstable();
        ids
    }

    #[tokio::test]
    async fn list_filters_by_agent() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = seed_memory_scoped(&store, "one", "agent-a", None, "active").await;
        seed_memory_scoped(&store, "two", "agent-b", None, "active").await;

        let rows = list_memories_core(
            &store,
            ListParams { agent_id: Some("agent-a".into()), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(ids_of(&rows), vec![a]);
    }

    #[tokio::test]
    async fn list_filters_by_project() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let exact = seed_memory_scoped(&store, "one", "a", Some("proj"), "active").await;
        seed_memory_scoped(&store, "two", "a", Some("proj-2"), "active").await;
        seed_memory_scoped(&store, "three", "a", None, "active").await;

        let rows = list_memories_core(
            &store,
            ListParams { project_id: Some("proj".into()), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(ids_of(&rows), vec![exact], "project filter is equality, not prefix");
    }

    #[tokio::test]
    async fn list_filters_by_status() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_scoped(&store, "one", "a", None, "active").await;
        let disputed = seed_memory_scoped(&store, "two", "a", None, "disputed").await;
        seed_memory_scoped(&store, "three", "a", None, "superseded").await;

        let rows = list_memories_core(
            &store,
            ListParams { status: Some("disputed".into()), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(ids_of(&rows), vec![disputed]);
    }

    #[tokio::test]
    async fn agent_filter_applies_to_every_kind() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_scoped(&store, "m1", "a1", None, "active").await;
        seed_memory_scoped(&store, "m2", "a2", None, "active").await;
        seed_episodic_scoped(&store, "a1", "t1").await;
        seed_episodic_scoped(&store, "a2", "t2").await;
        seed_procedural_scoped(&store, "a1").await;
        seed_procedural_scoped(&store, "a2").await;

        let rows = list_memories_core(
            &store,
            ListParams { agent_id: Some("a1".into()), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 3, "{rows:?}");
        assert!(rows.iter().all(|r| r.agent_id.as_deref() == Some("a1")));
        let mut kinds: Vec<_> = rows.iter().map(|r| format!("{:?}", r.kind)).collect();
        kinds.sort();
        assert_eq!(kinds, ["Episodic", "Memory", "Procedural"]);
    }

    #[tokio::test]
    async fn viewer_match_is_prefix_and_folds_accents() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_scoped(&store, "la configuración del núcleo", "a", None, "active").await;
        seed_memory_scoped(&store, "otra cosa distinta", "a", None, "active").await;

        for query in ["configuracion", "config", "CONFIGURACIÓN nucleo"] {
            let rows = list_memories_core(
                &store,
                ListParams { query: Some(query.into()), ..Default::default() },
            )
            .await
            .unwrap();
            assert_eq!(ids_of(&rows), vec![id], "query {query:?}");
        }
    }

    #[tokio::test]
    async fn text_query_requires_every_token_across_rows() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let both = seed_memory_scoped(&store, "alpha and beta together", "a", None, "active").await;
        seed_memory_scoped(&store, "only alpha here", "a", None, "active").await;

        let rows = list_memories_core(
            &store,
            ListParams { query: Some("alpha beta".into()), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(ids_of(&rows), vec![both]);
    }

    #[tokio::test]
    async fn query_skips_tables_without_text_index() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_episodic_scoped(&store, "a", "unrelated task").await;
        seed_procedural_scoped(&store, "a").await;

        let rows = list_memories_core(
            &store,
            ListParams { query: Some("needle".into()), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 2, "episodic and procedural have no text index: {rows:?}");
    }

    // ── get_memory_core ─────────────────────────────

    #[tokio::test]
    async fn get_memory_by_kind_and_id() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mem = seed_memory_scoped(&store, "full body text", "a1", Some("p1"), "active").await;
        let proc_id = seed_procedural_full(&store, 4, "on failure", "[1]").await;

        let row = get_memory_core(&store, MemoryKind::Memory, mem).await.unwrap().unwrap();
        assert_eq!(row.kind, MemoryKind::Memory);
        assert_eq!(row.summary, "full body text");
        assert_eq!(row.agent_id.as_deref(), Some("a1"));
        assert_eq!(row.project_id.as_deref(), Some("p1"));
        assert_eq!(row.source.as_deref(), Some("agent"));

        let skill = get_memory_core(&store, MemoryKind::Procedural, proc_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(skill.kind, MemoryKind::Procedural);
        assert_eq!(skill.trigger.as_deref(), Some("on failure"));
        assert_eq!(skill.success_count, Some(4));
    }

    #[tokio::test]
    async fn get_kind_disambiguates_colliding_ids() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mem_id = seed_memory_scoped(&store, "a memory", "a", None, "active").await;
        seed_episodic_scoped(&store, "a", "an episode").await;
        let epi_id = mem_id; // fresh tables: both first rows get id 1
        assert_eq!(
            get_memory_core(&store, MemoryKind::Episodic, epi_id)
                .await
                .unwrap()
                .unwrap()
                .summary,
            "an episode"
        );
        assert_eq!(
            get_memory_core(&store, MemoryKind::Memory, mem_id).await.unwrap().unwrap().summary,
            "a memory"
        );
    }

    #[tokio::test]
    async fn get_missing_is_none() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_scoped(&store, "x", "a", None, "active").await;

        assert!(get_memory_core(&store, MemoryKind::Memory, 999).await.unwrap().is_none());
        assert!(get_memory_core(&store, MemoryKind::Procedural, 1).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn get_returns_soft_deleted() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_scoped(&store, "gone", "a", None, "active").await;
        delete_memory_core(&store, id, MemoryKind::Memory, None).await.unwrap();

        let row = get_memory_core(&store, MemoryKind::Memory, id).await.unwrap().unwrap();

        assert!(row.deleted_at.is_some());
        assert_eq!(row.summary, "gone");
    }

    #[tokio::test]
    async fn get_does_not_touch_access_count() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_scoped(&store, "x", "a", None, "active").await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE memory SET access_count = 7, last_accessed_at = 55 WHERE id = ?1",
                    [id],
                )
            })
            .await
            .unwrap();

        for _ in 0..2 {
            let row = get_memory_core(&store, MemoryKind::Memory, id).await.unwrap().unwrap();
            assert_eq!((row.access_count, row.last_accessed_at), (7, 55));
        }
    }

    // ── memory_stats_core ───────────────────────────

    async fn seed_stat_row(
        store: &MemoryStore,
        (memory_type, tier, status): (&str, i64, &str),
        (agent, project): (&str, Option<&str>),
        created_at: i64,
        deleted: bool,
    ) {
        let (memory_type, status, agent, project) = (
            memory_type.to_string(),
            status.to_string(),
            agent.to_string(),
            project.map(str::to_string),
        );
        let deleted_at: Option<i64> = deleted.then_some(1);
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, project_id, content, memory_type, tier, status, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref, deleted_at) \
                     VALUES ('agent', ?, ?, 'c', ?, ?, ?, ?, 0, 0, 'agent', '', ?)",
                    rusqlite::params![agent, project, memory_type, tier, status, created_at, deleted_at],
                )
            })
            .await
            .expect("seed_stat_row failed");
    }

    fn count(key: Option<&str>, n: u64) -> Count {
        Count { key: key.map(str::to_string), count: n }
    }

    #[tokio::test]
    async fn stats_counts_live_per_kind_and_deleted_apart() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_full(&store, "observation", 3, "active", false, 10).await;
        seed_memory_full(&store, "observation", 3, "active", false, 20).await;
        seed_memory_full(&store, "observation", 3, "active", true, 30).await;
        seed_episodic_row(&store, 3, 5.0).await;
        seed_procedural_row(&store, 1, 8.0).await;
        let gone = seed_procedural_row(&store, 1, 8.0).await;
        delete_memory_core(&store, gone, MemoryKind::Procedural, None).await.unwrap();

        let stats = memory_stats_core(&store, StatsParams::default()).await.unwrap();

        assert_eq!(
            stats.totals,
            StatsTotals { memory: 2, episodic: 1, procedural: 1, deleted: 2 }
        );
        assert_eq!(stats.newest_created_at, Some(20), "newest live row, deleted ones ignored");
    }

    #[tokio::test]
    async fn stats_groups_by_type_tier_status_agent_project() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_stat_row(&store, ("fact", 1, "active"), ("a2", Some("p1")), 1, false).await;
        seed_stat_row(&store, ("fact", 3, "active"), ("a1", Some("p1")), 2, false).await;
        seed_stat_row(&store, ("note", 3, "disputed"), ("a1", Some("p2")), 3, false).await;
        // Deleted rows must not reach any grouping.
        seed_stat_row(&store, ("ghost", 2, "superseded"), ("a9", Some("p9")), 4, true).await;

        let stats = memory_stats_core(&store, StatsParams::default()).await.unwrap();

        assert_eq!(stats.by_type, vec![count(Some("fact"), 2), count(Some("note"), 1)]);
        assert_eq!(stats.by_tier, vec![count(Some("3"), 2), count(Some("1"), 1)]);
        assert_eq!(stats.by_status, vec![count(Some("active"), 2), count(Some("disputed"), 1)]);
        assert_eq!(stats.by_agent, vec![count(Some("a1"), 2), count(Some("a2"), 1)]);
        assert_eq!(stats.by_project, vec![count(Some("p1"), 2), count(Some("p2"), 1)]);
    }

    #[tokio::test]
    async fn stats_filters_by_agent_and_project() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_stat_row(&store, ("fact", 3, "active"), ("a1", Some("p1")), 1, false).await;
        seed_stat_row(&store, ("fact", 3, "active"), ("a1", Some("p2")), 2, false).await;
        seed_stat_row(&store, ("fact", 3, "active"), ("a2", Some("p1")), 3, false).await;
        seed_stat_row(&store, ("fact", 3, "active"), ("a1", Some("p1")), 4, true).await;
        seed_episodic_scoped(&store, "a1", "t").await;
        seed_episodic_scoped(&store, "a2", "t").await;

        let stats = memory_stats_core(
            &store,
            StatsParams { agent_id: Some("a1".into()), project_id: Some("p1".into()) },
        )
        .await
        .unwrap();

        assert_eq!(stats.totals.memory, 1);
        assert_eq!(stats.totals.deleted, 1);
        assert_eq!(stats.by_agent, vec![count(Some("a1"), 1)]);
        assert_eq!(stats.by_project, vec![count(Some("p1"), 1)]);

        let by_agent = memory_stats_core(
            &store,
            StatsParams { agent_id: Some("a1".into()), project_id: None },
        )
        .await
        .unwrap();
        assert_eq!((by_agent.totals.memory, by_agent.totals.episodic), (2, 1));
    }

    #[tokio::test]
    async fn stats_empty_store_is_zero() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let stats = memory_stats_core(&store, StatsParams::default()).await.unwrap();

        assert_eq!(stats.totals, StatsTotals { memory: 0, episodic: 0, procedural: 0, deleted: 0 });
        assert!(stats.by_type.is_empty() && stats.by_tier.is_empty());
        assert!(stats.by_status.is_empty() && stats.by_agent.is_empty());
        assert!(stats.by_project.is_empty());
        assert_eq!(stats.newest_created_at, None);
    }

    #[tokio::test]
    async fn stats_null_project_is_own_bucket() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_stat_row(&store, ("fact", 3, "active"), ("a", None), 1, false).await;
        seed_stat_row(&store, ("fact", 3, "active"), ("a", None), 2, false).await;
        seed_stat_row(&store, ("fact", 3, "active"), ("a", Some("p")), 3, false).await;

        let stats = memory_stats_core(&store, StatsParams::default()).await.unwrap();

        assert_eq!(stats.by_project, vec![count(None, 2), count(Some("p"), 1)]);
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
            agent_id: None,
            piece: None,
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

    // ── restore_memory_core ──────────────────────────────────────────────────

    use crate::memory::embedder::{EmbedError, EmbedRole, Embedder, FakeEmbedder};

    struct BrokenEmbedder;

    #[async_trait::async_trait]
    impl Embedder for BrokenEmbedder {
        async fn embed_with_role(&self, _: &str, _: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            Err(EmbedError::NotReady)
        }
        fn dimensions(&self) -> usize {
            384
        }
        fn model_id(&self) -> &str {
            "broken"
        }
        fn query_prefix(&self) -> &str {
            ""
        }
        fn passage_prefix(&self) -> &str {
            ""
        }
    }

    async fn passage(text: &str) -> Vec<f32> {
        FakeEmbedder::default().embed_with_role(text, EmbedRole::Passage).await.unwrap()
    }

    /// A live row as a save leaves it: hash, vector and text index entry.
    async fn save(store: &MemoryStore, agent: &str, content: &str) -> i64 {
        let unit = crate::memory::vector::to_unit(&passage(content).await).unwrap();
        let bytes: Vec<u8> = unit.iter().flat_map(|f| f.to_le_bytes()).collect();
        let hash = crate::memory::dedup::content_hash(content);
        let (agent, content) = (agent.to_owned(), content.to_owned());
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory (scope, agent_id, project_id, content, content_hash, memory_type, \
                     source, created_at, created_by_type, created_by_ref, last_modified_at, \
                     last_accessed_at, importance, tier, status) \
                     VALUES ('agent', ?1, 'p', ?2, ?3, 'semantic_fact', 'explicit', 1, 'user', ?1, 1, 1, \
                     5.0, 3, 'active')",
                    rusqlite::params![agent, content, hash],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?1, ?2)", rusqlite::params![id, bytes])?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    async fn restore(store: &MemoryStore, id: i64, owner: Option<&str>) -> Result<RestoredRow, RestoreError> {
        restore_memory_core(store, &FakeEmbedder::default(), MemoryKind::Memory, id, owner).await
    }

    async fn stored_vector(store: &MemoryStore, id: i64) -> Option<Vec<f32>> {
        store
            .conn
            .call(move |c| {
                use rusqlite::OptionalExtension;
                c.query_row("SELECT embedding FROM memory_vectors WHERE rowid = ?1", [id], |r| r.get::<_, Vec<u8>>(0))
                    .optional()
            })
            .await
            .unwrap()
            .map(|b| b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    }

    async fn text_matches(store: &MemoryStore, word: &str) -> Vec<i64> {
        let word = word.to_owned();
        store
            .conn
            .call(move |c| -> rusqlite::Result<Vec<i64>> {
                let mut stmt = c.prepare("SELECT rowid FROM memory_fts WHERE memory_fts MATCH ?1 ORDER BY rowid")?;
                let rows = stmt.query_map([word], |r| r.get(0))?;
                rows.collect()
            })
            .await
            .unwrap()
    }

    /// Whether the text index holds exactly the postings of the live rows: a
    /// live row the index lacks, or a subtraction of postings that were not
    /// there, shows up as a difference against an index rebuilt from scratch.
    /// FTS5's own check against the content table cannot say this, because
    /// that table keeps its deleted rows.
    async fn text_index_is_whole(store: &MemoryStore) -> bool {
        store
            .conn
            .call(|c| -> rusqlite::Result<bool> {
                c.execute_batch(
                    "DROP TABLE IF EXISTS temp.expected_fts; DROP TABLE IF EXISTS temp.got_terms; \
                     DROP TABLE IF EXISTS temp.expected_terms; \
                     CREATE VIRTUAL TABLE temp.expected_fts USING fts5(content, tokenize='unicode61 remove_diacritics 2'); \
                     INSERT INTO temp.expected_fts(rowid, content) SELECT id, content FROM memory WHERE deleted_at IS NULL; \
                     CREATE VIRTUAL TABLE temp.got_terms USING fts5vocab(main, memory_fts, instance); \
                     CREATE VIRTUAL TABLE temp.expected_terms USING fts5vocab(temp, expected_fts, instance);",
                )?;
                c.execute("INSERT INTO memory_fts(memory_fts) VALUES('integrity-check')", [])?;
                let differences: i64 = c.query_row(
                    "WITH got AS (SELECT term, doc, offset, COUNT(*) n FROM temp.got_terms \
                                  WHERE col = 'content' GROUP BY term, doc, offset), \
                          expected AS (SELECT term, doc, offset, COUNT(*) n FROM temp.expected_terms \
                                       GROUP BY term, doc, offset) \
                     SELECT (SELECT COUNT(*) FROM (SELECT * FROM got EXCEPT SELECT * FROM expected)) \
                          + (SELECT COUNT(*) FROM (SELECT * FROM expected EXCEPT SELECT * FROM got))",
                    [],
                    |r| r.get(0),
                )?;
                Ok(differences == 0)
            })
            .await
            .unwrap()
    }

    async fn columns(store: &MemoryStore, id: i64) -> (Option<i64>, String, i64, i64, i64, i64) {
        store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT deleted_at, status, update_count, access_count, last_accessed_at, tier \
                     FROM memory WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
            })
            .await
            .unwrap()
    }

    async fn delete(store: &MemoryStore, id: i64) {
        assert_eq!(delete_memory_core(store, id, MemoryKind::Memory, None).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_restored_row_is_found_again() {
        use crate::memory::search::{search_all_memories, SearchScope};
        let store = MemoryStore::new_for_test().await.unwrap();
        let content = "the build cache lives on the zircon volume";
        let id = save(&store, "a", content).await;
        delete(&store, id).await;

        let row = restore(&store, id, Some("a")).await.unwrap();

        assert_eq!(row, RestoredRow { id, status: "active".into(), superseded_by: None });
        let query = passage(content).await;
        let hits = search_all_memories(&store, &query, None, "a", SearchScope::all_default(), 10).await.unwrap();
        assert!(hits.iter().any(|h| h.id == id), "the vector search finds it again");
        assert_eq!(text_matches(&store, "zircon").await, vec![id], "the text index finds it again");
        let (deleted_at, _, update_count, ..) = columns(&store, id).await;
        assert_eq!((deleted_at, update_count), (None, 1));
    }

    #[tokio::test]
    async fn delete_restore_delete_keeps_the_text_index_whole() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = save(&store, "a", "zircon holds the cache").await;
        let other = save(&store, "a", "zircon also names a mineral").await;

        delete(&store, id).await;
        restore(&store, id, None).await.unwrap();
        assert!(text_index_is_whole(&store).await);
        delete(&store, id).await;

        assert!(text_index_is_whole(&store).await);
        assert_eq!(text_matches(&store, "zircon").await, vec![other]);
        assert_eq!(text_matches(&store, "cache").await, Vec::<i64>::new());
    }

    #[tokio::test]
    async fn restoring_needs_a_deleted_row_of_the_owner() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let live = save(&store, "a", "a live row").await;
        let theirs = save(&store, "b", "another agent's row").await;
        delete(&store, theirs).await;
        let before = columns(&store, theirs).await;

        for (id, owner) in [(live, Some("a")), (theirs, Some("a")), (999_999, None), (live, None)] {
            let err = restore(&store, id, owner).await.unwrap_err();
            assert!(matches!(err, RestoreError::NotFound(n) if n == id), "{id}: {err:?}");
        }
        assert_eq!(columns(&store, theirs).await, before, "another agent's row is untouched");
        assert_eq!(stored_vector(&store, theirs).await, None);
        assert!(text_index_is_whole(&store).await);
    }

    #[tokio::test]
    async fn only_memory_rows_are_restored() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let epi = seed_episodic_row_with_vector(&store).await;
        delete_memory_core(&store, epi, MemoryKind::Episodic, None).await.unwrap();

        let err = restore_memory_core(&store, &FakeEmbedder::default(), MemoryKind::Episodic, epi, None)
            .await
            .unwrap_err();

        assert!(matches!(err, RestoreError::OnlyMemory(MemoryKind::Episodic)), "{err:?}");
        assert!(episodic_deleted_at(&store, epi).await.is_some());
    }

    #[tokio::test]
    async fn a_live_twin_blocks_the_restore() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let first = save(&store, "a", "deploys go out on fridays").await;
        delete(&store, first).await;
        let twin = save(&store, "a", "deploys go out on fridays").await;

        let err = restore(&store, first, Some("a")).await.unwrap_err();

        assert!(matches!(err, RestoreError::LiveTwin { id, twin: t } if id == first && t == twin), "{err:?}");
        assert!(err.to_string().contains(&format!("#{twin}")), "{err}");
        assert!(columns(&store, first).await.0.is_some(), "the row stays deleted");
        assert_eq!(stored_vector(&store, first).await, None);
    }

    #[tokio::test]
    async fn a_row_holding_a_secret_stays_deleted() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = save(&store, "a", "the key is sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123").await;
        delete(&store, id).await;

        let err = restore(&store, id, Some("a")).await.unwrap_err();

        assert!(matches!(&err, RestoreError::HoldsSecret { kinds, .. } if kinds == &["anthropic_key"]), "{err:?}");
        let message = err.to_string();
        assert!(message.contains("anthropic_key") && message.contains("clawg purge"), "{message}");
        assert!(!message.contains("sk-ant"), "the message never carries the secret: {message}");
        assert!(columns(&store, id).await.0.is_some());
    }

    #[tokio::test]
    async fn a_failed_embedding_restores_nothing() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = save(&store, "a", "a fact the embedder cannot reach").await;
        delete(&store, id).await;
        let before = columns(&store, id).await;

        let err = restore_memory_core(&store, &BrokenEmbedder, MemoryKind::Memory, id, None).await.unwrap_err();

        assert!(matches!(err, RestoreError::Embed(_, EmbedError::NotReady)), "{err:?}");
        assert_eq!(columns(&store, id).await, before);
        assert_eq!(stored_vector(&store, id).await, None);
    }

    #[tokio::test]
    async fn a_stale_vector_does_not_stop_a_restore() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let content = "a row deleted before deletes pruned vectors";
        let id = save(&store, "a", content).await;
        // Marked by hand, as the delete of an older version did: the vector stays.
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET deleted_at = 1 WHERE id = ?1", [id]))
            .await
            .unwrap();

        restore(&store, id, None).await.unwrap();

        let expected = crate::memory::vector::to_unit(&passage(content).await).unwrap();
        assert_eq!(stored_vector(&store, id).await, Some(expected));
    }

    #[tokio::test]
    async fn a_restored_row_settles_its_own_dispute() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let old = save(&store, "a", "the api listens on 8080").await;
        let other = save(&store, "a", "the api listens on 9090").await;
        let new = save(&store, "a", "the api listens on 7070").await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE memory SET status = 'disputed', disputed_against = ?2 WHERE id = ?1",
                    rusqlite::params![new, format!("[{old}]")],
                )?;
                c.execute(
                    "UPDATE memory SET status = 'disputed', disputed_against = ?2 WHERE id = ?1",
                    rusqlite::params![other, format!("[{old},{new}]")],
                )
            })
            .await
            .unwrap();
        delete(&store, new).await;
        delete(&store, old).await;
        assert_eq!(memory_status(&store, other).await, "active", "both deletes released it");

        let row = restore(&store, new, None).await.unwrap();

        assert_eq!(row.status, "active", "the row it disputed is still deleted");
        assert_eq!(memory_status(&store, other).await, "active", "released disputes stay released");
    }

    #[tokio::test]
    async fn restoring_leaves_the_rest_of_the_row_alone() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let keep = save(&store, "a", "the newer claim").await;
        let id = save(&store, "a", "an older claim").await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE memory SET status = 'superseded', superseded_by = ?2, tier = 2, \
                     importance = 7.5, access_count = 4, last_accessed_at = 33 WHERE id = ?1",
                    [id, keep],
                )
            })
            .await
            .unwrap();
        delete(&store, id).await;

        let row = restore(&store, id, None).await.unwrap();

        assert_eq!(row, RestoredRow { id, status: "superseded".into(), superseded_by: Some(keep) });
        let (deleted_at, status, update_count, access_count, last_accessed_at, tier) = columns(&store, id).await;
        assert_eq!(
            (deleted_at, status.as_str(), update_count, access_count, last_accessed_at, tier),
            (None, "superseded", 1, 4, 33, 2)
        );
        assert_eq!(memory_tier_importance(&store, id).await, (2, 7.5));
        assert_eq!(columns(&store, keep).await.2, 0, "no other row is written");
    }

    async fn link_save(store: &MemoryStore, pieces: Vec<(i64, &str)>) {
        use crate::memory::pieces::StoredPiece;
        let pieces = pieces
            .into_iter()
            .map(|(memory_id, context)| StoredPiece { memory_id, context: context.to_owned() })
            .collect();
        store.link_pieces(pieces).await.unwrap();
    }

    #[tokio::test]
    async fn a_restored_piece_is_embedded_behind_its_head_again() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let first = save(&store, "a", "Notes on clawg-ledger. The first piece of a long save").await;
        let second = save(&store, "a", "the second piece of a long save").await;
        let third = save(&store, "a", "the third piece of a long save").await;
        link_save(&store, vec![(first, ""), (second, "Notes on clawg-ledger."), (third, "Notes on clawg-ledger.")]).await;
        delete(&store, second).await;
        let gap = crate::memory::read_save_core(&store, MemoryKind::Memory, first, None).await.unwrap().unwrap();
        assert_eq!(gap.missing, vec![2]);

        restore(&store, second, Some("a")).await.unwrap();

        let behind = crate::memory::chunker::embedding_text("Notes on clawg-ledger.", "the second piece of a long save");
        let expected = crate::memory::vector::to_unit(&passage(&behind).await).unwrap();
        assert_eq!(stored_vector(&store, second).await, Some(expected));
        let whole = crate::memory::read_save_core(&store, MemoryKind::Memory, first, None).await.unwrap().unwrap();
        assert!(whole.missing.is_empty(), "{whole:?}");
        assert_eq!(whole.ids, vec![Some(first), Some(second), Some(third)]);
    }

    /// Seeds a row the way the previous version leaves a deleted one: the
    /// insert indexes it, the delete's trigger subtracts it again, and the
    /// delete prunes its vector. Raw SQL only, so nothing of this version's
    /// code shapes it.
    async fn seed_deleted_by_previous_version(store: &MemoryStore, content: &str, vector: &[f32]) -> i64 {
        let bytes: Vec<u8> = vector.iter().flat_map(|f| f.to_le_bytes()).collect();
        let hash = crate::memory::dedup::content_hash(content);
        let content = content.to_owned();
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory (scope, agent_id, project_id, content, content_hash, memory_type, \
                     source, created_at, created_by_type, created_by_ref, last_modified_at, \
                     last_accessed_at, importance, tier, status) \
                     VALUES ('agent', 'a', 'p', ?1, ?2, 'semantic_fact', 'explicit', 1, 'user', 'a', 1, 1, \
                     5.0, 3, 'active')",
                    rusqlite::params![content, hash],
                )?;
                let id = c.last_insert_rowid();
                c.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?1, ?2)", rusqlite::params![id, bytes])?;
                c.execute("UPDATE memory SET deleted_at = 5 WHERE id = ?1", [id])?;
                c.execute("DELETE FROM memory_vectors WHERE rowid = ?1", [id])?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn rows_deleted_by_the_previous_version_can_be_restored() {
        use crate::memory::search::{search_all_memories, SearchScope};
        let store = MemoryStore::new_for_test().await.unwrap();
        let alone = "the release notes live in the obsidian vault";
        let head = "Notes on clawg-ledger. The first piece";
        let piece = "the second piece mentions the quartz bucket";
        let alone_id = seed_deleted_by_previous_version(&store, alone, &passage(alone).await).await;
        let head_id = save(&store, "a", head).await;
        let behind = crate::memory::chunker::embedding_text("Notes on clawg-ledger.", piece).into_owned();
        let piece_id = seed_deleted_by_previous_version(&store, piece, &passage(&behind).await).await;
        link_save(&store, vec![(head_id, ""), (piece_id, "Notes on clawg-ledger.")]).await;
        assert!(text_matches(&store, "obsidian").await.is_empty());

        restore(&store, alone_id, Some("a")).await.unwrap();
        restore(&store, piece_id, Some("a")).await.unwrap();

        assert_eq!(text_matches(&store, "obsidian").await, vec![alone_id]);
        assert_eq!(text_matches(&store, "quartz").await, vec![piece_id]);
        assert!(text_index_is_whole(&store).await);
        for (id, text) in [(alone_id, alone.to_owned()), (piece_id, behind)] {
            let hits = search_all_memories(&store, &passage(&text).await, None, "a", SearchScope::all_default(), 10)
                .await
                .unwrap();
            assert!(hits.iter().any(|h| h.id == id), "{id} is reachable by its vector");
        }
    }
}
