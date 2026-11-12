//! Override primitives for `memory`: updating, deleting and restoring a row.
//! Reading the rows is `listing.rs`, whose public items this module shows too.
//!
//! The `pub async fn *_core(store: &MemoryStore, ...)` functions here are
//! directly testable via `MemoryStore::new_for_test()`.

use serde::Serialize;

use crate::memory::error::{ClosedAs, failed_call};
use crate::memory::clock::TimestampMs;
use crate::memory::chunker::embedding_text;
use crate::memory::contradiction::{detect_contradiction, excerpt, Candidate};
use crate::memory::dispute::{add_dispute_in, release_challengers, settle_challengers, settle_row, SUPERSEDE_SQL};
use crate::memory::relocate::{apply_one_in, plan_one_in, MoveError, MoveReport};
use crate::memory::revival::{revive_after_delete, revive_after_delete_of_piece, RESTORE_TEXT_INDEX, RESTORE_TOPIC_HOLDER, RESTORE_TWIN};
use crate::memory::embedder::{EmbedError, EmbedRole, Embedder};
use crate::memory::search::{reader_bind, reader_predicate};
use crate::memory::sharing::{Reach, Reader};
use crate::memory::record::{MemoryType, UnknownMemoryType};
use crate::memory::tier::retier;
use crate::memory::topic::Replaced;
use crate::memory::{MemoryError, MemoryStore};

pub use crate::memory::listing::{
    get_memory_core, list_memories_core, memory_stats_core, query_has_tokens, Count, ListParams, MemoryAdminRow, MemoryStats, StatsParams, StatsTotals,
};

// ---------------------------------------------------------------------------
// update_memory — a correction of importance, type and/or project. The row is
// read and written inside one transaction on the store's single connection:
// the new tier depends on the old values (`tier::retier`), a move is decided
// on what is stored (`relocate.rs`), and no reader can see the statements
// apart.
//
// Only a live row is changed: a deleted one no longer participates in
// retrieval, and "missing", "deleted" and "another agent's" are one answer.
// ---------------------------------------------------------------------------

/// Why a requested change was refused before touching the store.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PatchError {
    #[error("nothing to change: pass importance, memory_type, a project to move to, or none")]
    Empty,
    #[error("importance must be a number from 0 to 10, got {0}")]
    ImportanceOutOfScale(f64),
    #[error(transparent)]
    UnknownType(#[from] UnknownMemoryType),
}

/// A valid change to one row: at least one field, and importance on the
/// store's scale.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryPatch {
    importance: Option<f64>,
    memory_type: Option<MemoryType>,
    /// The project to move the row's save to, `Some(None)` for none; `None`
    /// leaves it where it is.
    project: Option<Option<String>>,
    user_answered: bool,
}

impl MemoryPatch {
    /// Unlike a save, which clamps the estimate an agent makes in passing,
    /// an out-of-scale importance here is refused: a correction is explicit,
    /// and clamping it would report a value nobody asked for as done.
    pub fn new(
        importance: Option<f64>,
        memory_type: Option<&str>,
        project: Option<Option<String>>,
    ) -> Result<Self, PatchError> {
        if importance.is_none() && memory_type.is_none() && project.is_none() {
            return Err(PatchError::Empty);
        }
        if let Some(v) = importance {
            if !(0.0..=10.0).contains(&v) {
                return Err(PatchError::ImportanceOutOfScale(v));
            }
        }
        let memory_type = memory_type.map(str::parse::<MemoryType>).transpose()?;
        Ok(Self { importance, memory_type, project, user_answered: false })
    }

    /// The caller declares the user answered, which taking a preference or a
    /// decision out of its type, or out of its project, needs. Nothing records
    /// it: the row keeps no reason for a change.
    pub fn answered(mut self, user_answered: bool) -> Self {
        self.user_answered = user_answered;
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error(
        "memory {id} is a {from}, so it loses that type only on the user's word: ask the user, \
         then call again with `user_answered`"
    )]
    NeedsUser { id: i64, from: String },
    #[error(
        "memory {id} is a {from}, so it changes project only on the user's word: ask the user, \
         then call again with `user_answered`"
    )]
    MoveNeedsUser { id: i64, from: String },
    #[error(transparent)]
    Move(#[from] MoveError),
    #[error(transparent)]
    Store(#[from] MemoryError),
}

/// How a changed row ended up.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UpdatedRow {
    pub id: i64,
    pub importance: f64,
    pub memory_type: String,
    pub tier: i64,
    /// The project the row is in after the change, `None` without one.
    pub project_id: Option<String>,
    /// The move that changed its project, when one did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_id: Option<i64>,
    /// The other rows of its save or topic history that moved with it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pulled_in: Vec<i64>,
}

const UPDATE_READ: &str = concat!(
    "SELECT tier, importance, memory_type, access_count, project_id FROM memory \
     WHERE id = ?1 AND deleted_at IS NULL AND ",
    reader_predicate!("agent_id", "?2")
);

const UPDATE_WRITE: &str = "UPDATE memory \
     SET importance = ?1, memory_type = ?2, tier = ?3, last_modified_at = ?4, update_count = update_count + 1 \
     WHERE id = ?5";

/// Applies `patch` to the live row `id` and returns how it ended
/// up, or `None` when there is no such live row within `reader`'s reach — in
/// which case nothing is written.
///
/// A new project moves the row's whole unit (`relocate.rs`), clashes judged
/// within `reader`'s reach. Any refusal, of the type or of the move, writes
/// nothing, and a move alone leaves `last_modified_at` and `update_count` as
/// they were: it changes where a row is, not what it says.
pub async fn update_memory_core(
    store: &MemoryStore,
    id: i64,
    patch: &MemoryPatch,
    reader: Option<&Reader>,
) -> Result<Option<UpdatedRow>, UpdateError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let patch = patch.clone();
    let reach = reader_bind(reader);

    store
        .conn
        .call(move |c| -> Result<Option<UpdatedRow>, UpdateError> {
            use rusqlite::OptionalExtension;
            let tx = c.transaction().map_err(MemoryError::from)?;
            let current: Option<(i64, f64, String, i64, Option<String>)> = tx
                .query_row(UPDATE_READ, rusqlite::params![id, reach], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })
                .optional()
                .map_err(MemoryError::from)?;
            let Some((tier, importance, memory_type, access_count, project_id)) = current else {
                return Ok(None);
            };
            let demoted = patch.memory_type.is_some_and(|t| !t.is_protected());
            if demoted && MemoryType::is_protected_name(&memory_type) && !patch.user_answered {
                return Err(UpdateError::NeedsUser { id, from: memory_type });
            }
            let mut moved: Option<MoveReport> = match &patch.project {
                Some(target) => Some(plan_one_in(&tx, id, target.as_deref(), reach.as_deref())?),
                None => None,
            };
            if let Some(report) = &moved {
                if let Some(skipped) = report.skipped.first() {
                    return Err(MoveError::Refused(skipped.clone()).into());
                }
                let protected = report.moved.iter().find_map(|m| Some((m.id, m.protected.clone()?)));
                if let (Some((row, from)), false) = (protected, patch.user_answered) {
                    return Err(UpdateError::MoveNeedsUser { id: row, from });
                }
            }
            let new_importance = patch.importance.unwrap_or(importance);
            let new_type = patch.memory_type.map_or(memory_type.clone(), |t| t.as_str().to_string());
            let new_tier = retier(tier, access_count, (importance, &memory_type), (new_importance, &new_type));
            if patch.importance.is_some() || patch.memory_type.is_some() {
                tx.execute(UPDATE_WRITE, rusqlite::params![new_importance, new_type, new_tier, now_ms, id])
                    .map_err(MemoryError::from)?;
            }
            if let Some(report) = &mut moved {
                apply_one_in(&tx, report, TimestampMs::from_unix_millis(now_ms)).map_err(MemoryError::from)?;
            }
            tx.commit().map_err(MemoryError::from)?;
            // Moved or already there, the row is in the target now.
            let project_id = patch.project.clone().unwrap_or(project_id);
            Ok(Some(UpdatedRow {
                id,
                importance: new_importance,
                memory_type: new_type,
                tier: new_tier,
                project_id,
                move_id: moved.as_ref().and_then(|r| r.move_id),
                pulled_in: moved.map(|r| r.pulled_in).unwrap_or_default(),
            }))
        })
        .await
        .map_err(failed_call(ClosedAs::During("update_memory_core")))
}

// ---------------------------------------------------------------------------
// delete_memory — soft delete. `deleted_at IS NULL` in the WHERE clause makes
// a second delete of the same row idempotent (0 rows affected instead of
// re-stamping `deleted_at`).
// ---------------------------------------------------------------------------

const DELETE_SQL: &str = "UPDATE memory SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL";

const OUT_OF_REACH: &str = concat!("SELECT 1 FROM memory WHERE id = ?1 AND NOT ", reader_predicate!("agent_id", "?2"));

pub(crate) const DELETE_VEC_MEMORY: &str = "DELETE FROM memory_vectors WHERE rowid = ?1";

/// What a delete did. Deleting the current version of a topic brings back the
/// version its revision replaced (`revival.rs`); a row a verdict replaced
/// stays superseded by the deleted one and is named in `hidden`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Deleted {
    pub deleted: usize,
    pub revived: Vec<Replaced>,
    pub hidden: Vec<i64>,
}

/// Soft-deletes the row `id` by stamping `deleted_at = now_ms`, and prunes its
/// vector row in the same transaction.
/// Rows of `memory` that disputed it are released in that transaction too.
/// Returns the number of rows affected by the UPDATE (`0` when `id` doesn't
/// exist or was already deleted — idempotent), with what its
/// delete brought back.
///
/// A row out of `reader`'s reach is answered as missing and the transaction
/// writes nothing, sweeps included: they would otherwise drop the vector of a
/// row the reader cannot see.
///
/// The vector DELETE runs unconditionally, never gated on the UPDATE's row
/// count: a point delete on an absent rowid is a 0-row no-op, not an error,
/// and running it every time means a row soft-deleted before this code
/// existed gets its orphaned vector swept the next time anyone calls delete
/// on it — no separate backfill needed.
pub async fn delete_memory_core(
    store: &MemoryStore,
    id: i64,
    reader: Option<&Reader>,
) -> Result<Deleted, MemoryError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let reach = reader_bind(reader);

    store
        .conn
        .call(move |c| {
            let tx = c.transaction()?;
            if reach.is_some() && tx.prepare(OUT_OF_REACH)?.exists(rusqlite::params![id, reach])? {
                return Ok(Deleted::default());
            }
            let affected = tx.execute(DELETE_SQL, rusqlite::params![now_ms, id])?;
            tx.execute(DELETE_VEC_MEMORY, rusqlite::params![id])?;
            let mut deleted = Deleted { deleted: affected, ..Deleted::default() };
            // Unconditional for the same reason as the vector delete: a row
            // deleted before deletes released its challengers gets them
            // released the next time anyone deletes it again.
            let now = TimestampMs::from_unix_millis(now_ms);
            release_challengers(&tx, id, now)?;
            if affected == 1 {
                let revived = revive_after_delete_of_piece(&tx, id, now)?;
                deleted.revived = revived.revived;
                deleted.hidden = revived.hidden;
            }
            tx.commit()?;
            Ok(deleted)
        })
        .await
        .map_err(failed_call::<rusqlite::Error, _>(ClosedAs::During("delete_memory_core")))
}

// ---------------------------------------------------------------------------
// restore_memory — the way back from `delete_memory`, with the text index,
// the pieces and the dedup kept whole.
// ---------------------------------------------------------------------------

/// Why a row was not restored. Nothing is written in any of these cases.
#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    /// Missing, live, purged or out of the reader's reach: a reader learns
    /// nothing about ids the policy hides from it.
    #[error("no deleted memory row with id {0}")]
    NotFound(i64),
    #[error("memory #{id} was saved again as #{twin}, which is live; #{id} stays deleted so the text is not stored twice")]
    LiveTwin { id: i64, twin: i64 },
    #[error("memory #{id} was saved under topic {key}, which #{live} holds now; #{id} stays deleted so the topic has one current memory")]
    TopicTaken { id: i64, key: String, live: i64 },
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
/// replaced: searches leave it out until a `resolve_conflict` verdict keeps
/// it, or, when the row that replaced it is deleted too
/// (`superseded_by_deleted`), until that row is restored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RestoredRow {
    pub id: i64,
    pub status: String,
    pub superseded_by: Option<i64>,
    pub topic_key: Option<String>,
    /// The rows its delete had brought back, superseded by it again.
    pub replaced: Vec<Replaced>,
    pub superseded_by_deleted: bool,
    /// The rows saved while it was deleted that the detector found close to
    /// it, each now a pair for the agent to judge, the newer row first.
    pub related: Vec<Candidate>,
}

struct DeletedRow {
    content: String,
    context: Option<String>,
    project_id: Option<String>,
    memory_type: String,
}

/// The deleted row, scoped like the update read, with the head its vector was
/// embedded behind when it is a piece of a long save (as `select_reembed_batch`
/// reads it), so the restored vector is the one the row was saved with, and
/// what the detector needs to compare it with what was saved meanwhile.
const RESTORE_READ: &str = concat!(
    "SELECT m.content, \
     (SELECT mp.context FROM memory_piece mp WHERE mp.memory_id = m.id AND mp.context <> '' \
      ORDER BY mp.save_id LIMIT 1), \
     m.agent_id, m.project_id, m.memory_type, m.deleted_at \
     FROM memory m WHERE m.id = ?1 AND m.deleted_at IS NOT NULL AND ",
    reader_predicate!("m.agent_id", "?2")
);

const RESTORE_ROW: &str = "UPDATE memory SET deleted_at = NULL, last_modified_at = ?2, \
     update_count = update_count + 1 WHERE id = ?1 AND deleted_at IS NOT NULL";

/// The restored row as it stands, with whether the row that replaces it is
/// deleted.
const RESTORED_SQL: &str = "SELECT m.status, m.superseded_by, m.topic_key, \
     m.status = 'superseded' AND EXISTS (SELECT 1 FROM memory b WHERE b.id = m.superseded_by AND b.deleted_at IS NOT NULL) \
     FROM memory m WHERE m.id = ?1";

/// The rows the detector finds close to the deleted row `id`, vectored as
/// `unit`, before the restore's transaction: the row is not live yet, so it
/// cannot find itself. The rows it already forms a pair with and the ones it
/// will take back from a revival are left out and widen the pool, so they do
/// not take the places of rows saved while it was deleted. The restoring
/// reader is the saver; the user's own restore compares with every row.
async fn restore_candidates(
    store: &MemoryStore,
    id: i64,
    unit: &[f32],
    content: &str,
    reader: Option<&Reader>,
    project_id: Option<String>,
    memory_type: &str,
) -> Result<Vec<Candidate>, MemoryError> {
    let Some(memory_type) = MemoryType::from_db_str(memory_type) else {
        return Ok(Vec::new());
    };
    let saver = reader.cloned().unwrap_or_else(|| Reader::new("", Reach::AllBut(Vec::new())));
    let siblings: Vec<i64> = store
        .conn
        .call(move |c| -> rusqlite::Result<Vec<i64>> {
            let mut stmt = c.prepare(
                "SELECT old_id FROM memory_conflict WHERE new_id = ?1 \
                 UNION SELECT new_id FROM memory_conflict WHERE old_id = ?1 \
                 UNION SELECT id FROM memory WHERE revived_from = ?1",
            )?;
            let rows = stmt.query_map([id], |r| r.get(0))?;
            rows.collect()
        })
        .await
        .map_err(failed_call::<_, MemoryError>(ClosedAs::During("restore_memory_core")))?;
    Ok(detect_contradiction(store, unit, content, &saver, project_id.as_deref(), memory_type, &siblings)
        .await
        .into_iter()
        .map(|(_, candidate)| candidate)
        .collect())
}

/// Writes a pair `(candidate, id)` for each candidate saved after the
/// restored row's `deleted_at` that is still live, inside the restore's
/// transaction. An escalated pair names the restored row in the newer row's
/// `disputed_against`; settling the restored row's challengers afterwards
/// makes it `disputed`. A pair already there, with its verdict, is kept.
fn note_restored_pairs(
    c: &rusqlite::Connection,
    id: i64,
    deleted_at: i64,
    candidates: &[Candidate],
    now: TimestampMs,
) -> rusqlite::Result<Vec<Candidate>> {
    let mut noted = Vec::new();
    for candidate in candidates {
        let in_window: bool = c.query_row(
            "SELECT EXISTS (SELECT 1 FROM memory WHERE id = ?1 AND created_at > ?2 \
             AND deleted_at IS NULL AND status IN ('active', 'disputed'))",
            rusqlite::params![candidate.id, deleted_at],
            |r| r.get(0),
        )?;
        if !in_window {
            continue;
        }
        let state = if candidate.escalated { "escalated" } else { "pending" };
        let written = c.execute(
            "INSERT OR IGNORE INTO memory_conflict (new_id, old_id, distance, state, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![candidate.id, id, candidate.distance, state, now],
        )?;
        if written == 0 {
            continue;
        }
        if candidate.escalated {
            add_dispute_in(c, candidate.id, id, now)?;
        }
        noted.push(candidate.clone());
    }
    Ok(noted)
}

/// Supersedes again, by the restored row `id`, the live rows its delete had
/// brought back, as the revision they were first replaced by.
fn take_back_revived(c: &rusqlite::Connection, id: i64, now: TimestampMs) -> rusqlite::Result<Vec<Replaced>> {
    let revived: Vec<(i64, String)> = {
        let mut stmt = c.prepare(
            "SELECT id, content FROM memory WHERE revived_from = ?1 \
             AND deleted_at IS NULL AND status IN ('active', 'disputed') ORDER BY id",
        )?;
        let rows = stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut replaced = Vec::with_capacity(revived.len());
    for (row, content) in revived {
        c.execute(SUPERSEDE_SQL, rusqlite::params![row, id, now, "revision"])?;
        release_challengers(c, row, now)?;
        replaced.push(Replaced { id: row, excerpt: excerpt(&content) });
    }
    Ok(replaced)
}

/// Brings back the deleted `memory` row `id` with its vector and text index
/// entry. A row out of `reader`'s reach is answered as missing.
///
/// The vector is computed before the transaction, from the current embedder;
/// inside it the row is read again, so a restore or a save that landed in
/// between is seen. The row's own dispute is settled against what is live
/// now, and the escalated pairs its deletion released are put back.
pub async fn restore_memory_core(
    store: &MemoryStore,
    embedder: &dyn Embedder,
    id: i64,
    reader: Option<&Reader>,
) -> Result<RestoredRow, RestoreError> {
    let reach = reader_bind(reader);
    let read_reach = reach.clone();
    let found = store
        .conn
        .call(move |c| {
            use rusqlite::OptionalExtension;
            c.query_row(RESTORE_READ, rusqlite::params![id, read_reach], |r| {
                Ok(DeletedRow {
                    content: r.get(0)?,
                    context: r.get(1)?,
                    project_id: r.get(3)?,
                    memory_type: r.get(4)?,
                })
            })
            .optional()
        })
        .await
        .map_err(failed_call::<_, MemoryError>(ClosedAs::During("restore_memory_core")))?;
    let Some(DeletedRow { content, context, project_id, memory_type }) = found else {
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
    let candidates = restore_candidates(store, id, &unit, &content, reader, project_id, &memory_type).await?;
    let now = TimestampMs::now();

    enum Outcome {
        Restored(RestoredRow),
        Gone,
        Twin(i64),
        TopicTaken(String, i64),
    }
    let outcome = store
        .conn
        .call(move |c| {
            use rusqlite::OptionalExtension;
            let tx = c.transaction()?;
            let Some(deleted_at) = tx
                .query_row(RESTORE_READ, rusqlite::params![id, reach], |r| r.get::<_, i64>(5))
                .optional()?
            else {
                return Ok(Outcome::Gone);
            };
            if let Some(twin) = tx.query_row(RESTORE_TWIN, rusqlite::params![id, reach], |r| r.get(0)).optional()? {
                return Ok(Outcome::Twin(twin));
            }
            if let Some((key, live)) = tx
                .query_row(RESTORE_TOPIC_HOLDER, rusqlite::params![id, reach], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?
            {
                return Ok(Outcome::TopicTaken(key, live));
            }
            tx.execute(RESTORE_ROW, rusqlite::params![id, now])?;
            tx.execute(DELETE_VEC_MEMORY, [id])?;
            tx.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?1, ?2)", rusqlite::params![id, bytes])?;
            tx.execute(RESTORE_TEXT_INDEX, [id])?;
            let (status, superseded_by): (String, Option<i64>) =
                tx.query_row("SELECT status, superseded_by FROM memory WHERE id = ?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            let (replaced, related) = match superseded_by.filter(|_| status == "superseded") {
                None => (take_back_revived(&tx, id, now)?, note_restored_pairs(&tx, id, deleted_at, &candidates, now)?),
                Some(by) => {
                    revive_after_delete(&tx, by, now)?;
                    (Vec::new(), Vec::new())
                }
            };
            settle_row(&tx, id, now)?;
            settle_challengers(&tx, id, now)?;
            let row = tx.query_row(RESTORED_SQL, [id], |r| {
                Ok(RestoredRow {
                    id,
                    status: r.get(0)?,
                    superseded_by: r.get(1)?,
                    topic_key: r.get(2)?,
                    replaced,
                    superseded_by_deleted: r.get(3)?,
                    related,
                })
            })?;
            tx.commit()?;
            Ok(Outcome::Restored(row))
        })
        .await
        .map_err(failed_call::<rusqlite::Error, MemoryError>(ClosedAs::During("restore_memory_core")))?;
    match outcome {
        Outcome::Restored(row) => Ok(row),
        Outcome::Gone => Err(RestoreError::NotFound(id)),
        Outcome::Twin(twin) => Err(RestoreError::LiveTwin { id, twin }),
        Outcome::TopicTaken(key, live) => Err(RestoreError::TopicTaken { id, key, live }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::sharing::Reach;

    /// `agent` as a policy that hides nothing lets it act.
    fn shared(agent: &str) -> Reader {
        Reader::new(agent, Reach::AllBut(Vec::new()))
    }

    /// `agent` with `author`'s rows hidden from it.
    fn hiding(agent: &str, author: &str) -> Reader {
        Reader::new(agent, Reach::AllBut(vec![author.to_string()]))
    }


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

    // ── update_memory_core ───────────────────────────────────────────────────

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

    fn importance(v: f64) -> MemoryPatch {
        MemoryPatch::new(Some(v), None, None).unwrap()
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
        assert_eq!(MemoryPatch::new(None, None, None), Err(PatchError::Empty));
    }

    #[test]
    fn a_patch_can_move_a_row_alone() {
        assert!(MemoryPatch::new(None, None, Some(Some("q-1a2b".into()))).is_ok());
        assert!(MemoryPatch::new(None, None, Some(None)).is_ok(), "to no project");
    }

    async fn register_project_row(store: &MemoryStore, id: &str) {
        let id = id.to_string();
        store
            .conn
            .call(move |c| {
                c.execute("INSERT INTO project_path (id, root_path, created_at) VALUES (?1, '/work/' || ?1, 0)", [&id])?;
                c.execute("INSERT INTO project (id, name, born, created_at) VALUES (?1, ?1, 'root', 0)", [&id])
            })
            .await
            .unwrap();
    }

    fn move_to(project: Option<&str>) -> MemoryPatch {
        MemoryPatch::new(None, None, Some(project.map(str::to_string))).unwrap()
    }

    async fn project_of(store: &MemoryStore, id: i64) -> Option<String> {
        store
            .conn
            .call(move |c| c.query_row("SELECT project_id FROM memory WHERE id = ?1", [id], |r| r.get(0)))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_move_alone_changes_the_project_and_nothing_it_says() {
        let store = MemoryStore::new_for_test().await.unwrap();
        register_project_row(&store, "q-1a2b").await;
        let id = save(&store, "agent-a", "the parser is hand written").await;
        let before = memory_columns(&store, id).await;

        let row = update_memory_core(&store, id, &move_to(Some("q-1a2b")), None)
            .await
            .unwrap()
            .expect("updated");

        assert_eq!((row.project_id.as_deref(), row.move_id), (Some("q-1a2b"), Some(1)));
        assert_eq!(project_of(&store, id).await.as_deref(), Some("q-1a2b"));
        assert_eq!(memory_columns(&store, id).await, before, "no update_count, no last_modified_at");
    }

    #[tokio::test]
    async fn a_move_and_a_change_of_importance_are_one_write() {
        let store = MemoryStore::new_for_test().await.unwrap();
        register_project_row(&store, "q-1a2b").await;
        let id = save(&store, "agent-a", "the parser is hand written").await;
        let both = MemoryPatch::new(Some(8.0), None, Some(Some("q-1a2b".into()))).unwrap();

        let row = update_memory_core(&store, id, &both, None).await.unwrap().expect("updated");
        assert_eq!((row.importance, row.project_id.as_deref()), (8.0, Some("q-1a2b")));

        let other = save(&store, "agent-a", "the lexer is generated").await;
        let twin = save(&store, "agent-a", "the lexer is generated").await;
        assert_ne!(other, twin, "a twin in another project");
        store.conn.call(move |c| c.execute("UPDATE memory SET project_id = 'q-1a2b' WHERE id = ?1", [twin])).await.unwrap();
        let clash = MemoryPatch::new(Some(1.0), None, Some(Some("q-1a2b".into()))).unwrap();

        let err = update_memory_core(&store, other, &clash, None).await.unwrap_err();

        assert!(err.to_string().contains(&format!("#{twin}")), "{err}");
        assert_eq!(memory_tier_importance(&store, other).await.1, 5.0, "the importance waits for the move");
        assert_eq!(project_of(&store, other).await.as_deref(), Some("p"));
    }

    #[tokio::test]
    async fn an_unregistered_project_is_not_a_place_to_move_to() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = save(&store, "agent-a", "the parser is hand written").await;

        let err = update_memory_core(&store, id, &move_to(Some("typo-0000")), None).await.unwrap_err();

        assert!(matches!(err, UpdateError::Move(MoveError::UnknownProject(_))), "{err:?}");
        let row = update_memory_core(&store, id, &move_to(None), None).await.unwrap().unwrap();
        assert_eq!(row.project_id, None, "no project is always a place");
    }

    #[tokio::test]
    async fn a_protected_row_leaves_its_project_only_on_the_users_word() {
        let store = MemoryStore::new_for_test().await.unwrap();
        register_project_row(&store, "q-1a2b").await;
        let id = save(&store, "agent-a", "the store is SQLite").await;
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET memory_type = 'architectural_decision' WHERE id = ?1", [id]))
            .await
            .unwrap();

        let err = update_memory_core(&store, id, &move_to(Some("q-1a2b")), Some(&shared("agent-b")))
            .await
            .unwrap_err();
        assert!(matches!(err, UpdateError::MoveNeedsUser { id: row, .. } if row == id), "{err:?}");
        assert_eq!(project_of(&store, id).await.as_deref(), Some("p"));

        let answered = move_to(Some("q-1a2b")).answered(true);
        update_memory_core(&store, id, &answered, Some(&shared("agent-b"))).await.unwrap().unwrap();
        assert_eq!(project_of(&store, id).await.as_deref(), Some("q-1a2b"));
    }

    #[tokio::test]
    async fn a_move_takes_the_rest_of_the_save_along() {
        let store = MemoryStore::new_for_test().await.unwrap();
        register_project_row(&store, "q-1a2b").await;
        let head = save(&store, "agent-a", "the opening of a long save").await;
        let tail = save(&store, "agent-a", "and the rest of it").await;
        store
            .conn
            .call(move |c| c.execute("INSERT INTO memory_piece VALUES (1, 1, ?1, ''), (1, 2, ?2, 'the opening')", [head, tail]))
            .await
            .unwrap();

        let row = update_memory_core(&store, tail, &move_to(Some("q-1a2b")), None).await.unwrap().unwrap();

        assert_eq!(row.pulled_in, vec![head]);
        assert_eq!(project_of(&store, head).await.as_deref(), Some("q-1a2b"));
    }

    #[test]
    fn importance_outside_the_scale_is_refused() {
        for v in [-0.5, 10.5, 50.0, f64::NAN, f64::INFINITY] {
            let err = MemoryPatch::new(Some(v), None, None).unwrap_err();
            assert!(matches!(err, PatchError::ImportanceOutOfScale(_)), "{v}: {err:?}");
            assert!(err.to_string().contains("importance"), "{err}");
        }
        for v in [0.0, 10.0, 7.5] {
            assert!(MemoryPatch::new(Some(v), None, None).is_ok(), "{v}");
        }
    }

    #[test]
    fn an_unknown_memory_type_is_refused() {
        let err = MemoryPatch::new(None, Some("preference"), None).unwrap_err();
        assert_eq!(err, PatchError::UnknownType(UnknownMemoryType("preference".into())));
        assert!(err.to_string().contains("memory_type"), "{err}");
        assert!(MemoryPatch::new(None, Some("user_preference"), None).is_ok());
    }

    #[tokio::test]
    async fn update_with_high_importance_promotes_to_tier_1() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row(&store, "architectural_decision", 3, 5.0, false).await;

        let row = update_memory_core(&store, id, &importance(9.0), None)
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

        let row = update_memory_core(&store, id, &importance(2.0), None)
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
        let patch = MemoryPatch::new(Some(8.0), Some("user_preference"), None).unwrap();

        let row = update_memory_core(&store, id, &patch, None).await.unwrap();

        assert_eq!(
            row,
            Some(UpdatedRow {
                id,
                importance: 8.0,
                memory_type: "user_preference".into(),
                tier: 1,
                project_id: None,
                move_id: None,
                pulled_in: Vec::new(),
            })
        );
        let (memory_type, update_count, last_modified_at, ..) = memory_columns(&store, id).await;
        assert_eq!((memory_type.as_str(), update_count), ("user_preference", 1));
        assert!(last_modified_at > 0, "the seed wrote 0");

        let type_only = MemoryPatch::new(None, Some("architectural_decision"), None).unwrap();
        let row = update_memory_core(&store, id, &type_only, None).await.unwrap().unwrap();
        assert_eq!(row.importance, 8.0, "a field not asked for stays as it was");
        assert_eq!(memory_columns(&store, id).await.1, 2);
    }

    #[tokio::test]
    async fn demoting_a_protected_row_needs_the_answer() {
        let store = MemoryStore::new_for_test().await.unwrap();
        for protected in ["user_preference", "architectural_decision"] {
            let id = seed_memory_row(&store, protected, 3, 5.0, false).await;
            let demote = MemoryPatch::new(Some(9.0), Some("semantic_fact"), None).unwrap();

            let refused = update_memory_core(&store, id, &demote, None).await;

            assert!(
                matches!(&refused, Err(UpdateError::NeedsUser { id: refused_id, from }) if *refused_id == id && from == protected),
                "{refused:?}"
            );
            let (memory_type, update_count, ..) = memory_columns(&store, id).await;
            assert_eq!((memory_type.as_str(), update_count), (protected, 0), "a refused change writes nothing");

            let row = update_memory_core(&store, id, &demote.answered(true), None)
                .await
                .unwrap()
                .unwrap();
            assert_eq!((row.memory_type.as_str(), row.importance), ("semantic_fact", 9.0));
        }
    }

    #[tokio::test]
    async fn promoting_or_changing_importance_needs_no_answer() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let fact = seed_memory_row(&store, "semantic_fact", 3, 5.0, false).await;
        let promote = MemoryPatch::new(None, Some("user_preference"), None).unwrap();
        assert!(update_memory_core(&store, fact, &promote, None).await.unwrap().is_some());

        let preference = seed_memory_row(&store, "user_preference", 3, 5.0, false).await;
        assert!(update_memory_core(&store, preference, &importance(2.0), None).await.unwrap().is_some());
        let between = MemoryPatch::new(None, Some("architectural_decision"), None).unwrap();
        assert!(update_memory_core(&store, preference, &between, None).await.unwrap().is_some());
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

        update_memory_core(&store, id, &importance(7.0), None).await.unwrap().unwrap();

        let after = memory_columns(&store, id).await;
        assert_eq!(after.3, before.3, "status");
        assert_eq!((after.4, after.5), (before.4, before.5), "accesses");
        assert!(disputes_before.is_some(), "the seed disputes a row");
        assert_eq!(disputed_against(&store, id).await, disputes_before);
    }

    #[tokio::test]
    async fn update_on_a_missing_id_changes_nothing() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let row = update_memory_core(&store, 999_999, &importance(9.0), None).await.unwrap();

        assert_eq!(row, None);
    }

    #[tokio::test]
    async fn update_on_a_soft_deleted_row_changes_nothing() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row(&store, "chat_message", 3, 5.0, true).await;

        let row = update_memory_core(&store, id, &importance(9.0), None).await.unwrap();

        assert_eq!(row, None, "deleted rows must not be updated");
        assert_eq!(memory_tier_importance(&store, id).await, (3, 5.0));
        assert_eq!(memory_columns(&store, id).await.1, 0, "update_count");
    }

    // A tier no rule of today awards, as an older version or a moved row
    // left it, is not taken by a change.
    #[tokio::test]
    async fn a_tier_no_rule_awards_survives_an_update() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let earned = seed_memory_row(&store, "semantic_fact", 1, 9.5, false).await;
        let foreign = seed_memory_row(&store, "semantic_fact", 1, 5.0, false).await;

        let lower = || MemoryPatch::new(Some(2.0), None, None).unwrap();
        let tier_of = |row: Option<UpdatedRow>| row.expect("updated").tier;
        assert_eq!(tier_of(update_memory_core(&store, earned, &lower(), None).await.unwrap()), 3);
        assert_eq!(tier_of(update_memory_core(&store, foreign, &lower(), None).await.unwrap()), 1);

        assert_eq!(memory_tier_importance(&store, foreign).await, (1, 2.0));
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

    // Seed → delete → deleted_at set (SELECT follow-up).
    #[tokio::test]
    async fn delete_memory_sets_deleted_at() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row(&store, "chat_message", 3, 5.0, false).await;
        assert_eq!(memory_deleted_at(&store, id).await, None);

        let affected = delete_memory_core(&store, id, None)
            .await
            .unwrap();

        assert_eq!(affected.deleted, 1);
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

        delete_memory_core(&store, old, None).await.unwrap();

        assert_eq!(memory_status(&store, new).await, "active");
    }

    #[tokio::test]
    async fn deleting_one_of_two_keeps_the_dispute() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let first = seed_memory_scoped(&store, "first", "a", None, "active").await;
        let second = seed_memory_scoped(&store, "second", "a", None, "active").await;
        let new = seed_disputing(&store, &[first, second]).await;

        delete_memory_core(&store, first, None).await.unwrap();

        assert_eq!(memory_status(&store, new).await, "disputed");
    }

    /// A row the policy hides from the reader is answered as missing, and
    /// nothing about it moves: not the row, not its vector, not the disputes
    /// that name it. Another agent that reaches it deletes it.
    #[tokio::test]
    async fn a_hidden_authors_row_is_not_found_and_stays() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mem = seed_memory_row_with_vector(&store).await;
        let challenger = seed_disputing(&store, &[mem]).await;

        assert_eq!(delete_memory_core(&store, mem, Some(&hiding("other-agent", "test-agent"))).await.unwrap().deleted, 0);

        assert_eq!(memory_deleted_at(&store, mem).await, None);
        assert_eq!(vector_row_count(&store, mem).await, 1);
        assert_eq!(memory_status(&store, challenger).await, "disputed");

        assert_eq!(delete_memory_core(&store, mem, Some(&shared("other-agent"))).await.unwrap().deleted, 1);
        assert_eq!(vector_row_count(&store, mem).await, 0);
        assert_eq!(memory_status(&store, challenger).await, "active");
    }

    #[tokio::test]
    async fn another_agent_updates_a_row_it_reaches_but_not_a_hidden_one() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mem = seed_memory_row(&store, "chat_message", 3, 5.0, false).await;

        let hidden = hiding("other-agent", "test-agent");
        assert_eq!(update_memory_core(&store, mem, &importance(9.5), Some(&hidden)).await.unwrap(), None);

        assert_eq!(memory_tier_importance(&store, mem).await, (3, 5.0));
        assert_eq!(memory_columns(&store, mem).await.1, 0, "update_count");

        let row = update_memory_core(&store, mem, &importance(9.5), Some(&shared("other-agent")))
            .await
            .unwrap();
        assert_eq!(row.map(|r| r.importance), Some(9.5), "another agent changes what it reaches");
    }

    // 2nd delete of the same id → 0 rows affected (idempotent).
    #[tokio::test]
    async fn delete_memory_second_delete_is_idempotent_zero() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_row(&store, "chat_message", 3, 5.0, false).await;

        let first = delete_memory_core(&store, id, None)
            .await
            .unwrap();
        let second = delete_memory_core(&store, id, None)
            .await
            .unwrap();

        assert_eq!(first.deleted, 1);
        assert_eq!(second.deleted, 0, "second delete of an already-deleted row must affect 0 rows");
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

        delete_memory_core(&store, id, None)
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

    async fn vector_row_count(store: &MemoryStore, id: i64) -> i64 {
        store
            .conn
            .call(move |c| c.query_row("SELECT COUNT(*) FROM memory_vectors WHERE rowid = ?1", [id], |r| r.get(0)))
            .await
            .unwrap()
    }

    // Soft-deleting must not leave the vector row behind: a vector with no
    // live parent still costs the vec0 scan on every future query, forever,
    // for a row nothing can ever return again.
    #[tokio::test]
    async fn delete_memory_removes_the_vector_row() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let mem_id = seed_memory_row_with_vector(&store).await;
        delete_memory_core(&store, mem_id, None).await.unwrap();
        assert_eq!(vector_row_count(&store, mem_id).await, 0, "memory_vectors row must be gone after soft-delete");
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
            vector_row_count(&store, id).await,
            1,
            "test setup: the vector row must still be present before self-heal"
        );

        let affected = delete_memory_core(&store, id, None).await.unwrap();

        assert_eq!(affected.deleted, 0, "the row was already deleted, so the UPDATE affects 0 rows");
        assert_eq!(
            vector_row_count(&store, id).await,
            0,
            "the orphaned vector must be pruned even though the row update affected 0 rows"
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

    async fn restore(store: &MemoryStore, id: i64, reader: Option<&Reader>) -> Result<RestoredRow, RestoreError> {
        restore_memory_core(store, &FakeEmbedder::default(), id, reader).await
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
            .map(|b| b.as_chunks::<4>().0.iter().copied().map(f32::from_le_bytes).collect())
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
        assert_eq!(delete_memory_core(store, id, None).await.unwrap().deleted, 1);
    }

    #[tokio::test]
    async fn a_restored_row_is_found_again() {
        use crate::memory::search::{search_all_memories, SearchScope};
        let store = MemoryStore::new_for_test().await.unwrap();
        let content = "the build cache lives on the zircon volume";
        let id = save(&store, "a", content).await;
        delete(&store, id).await;

        let row = restore(&store, id, Some(&shared("a"))).await.unwrap();

        assert_eq!(row, RestoredRow { id, status: "active".into(), ..RestoredRow::default() });
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
    async fn restoring_needs_a_deleted_row_in_reach() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let live = save(&store, "a", "a live row").await;
        let theirs = save(&store, "b", "another agent's row").await;
        delete(&store, theirs).await;
        let before = columns(&store, theirs).await;

        let hidden = hiding("a", "b");
        for (id, reader) in [(live, Some(&shared("a"))), (theirs, Some(&hidden)), (999_999, None), (live, None)] {
            let err = restore(&store, id, reader).await.unwrap_err();
            assert!(matches!(err, RestoreError::NotFound(n) if n == id), "{id}: {err:?}");
        }
        assert_eq!(columns(&store, theirs).await, before, "a hidden author's row is untouched");
        assert_eq!(stored_vector(&store, theirs).await, None);
        assert!(text_index_is_whole(&store).await);

        let row = restore(&store, theirs, Some(&shared("a"))).await.unwrap();
        assert_eq!(row.status, "active", "another agent restores what it reaches");
        assert!(stored_vector(&store, theirs).await.is_some());
    }

    #[tokio::test]
    async fn a_live_twin_blocks_the_restore() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let first = save(&store, "a", "deploys go out on fridays").await;
        delete(&store, first).await;
        let twin = save(&store, "a", "deploys go out on fridays").await;

        let err = restore(&store, first, Some(&shared("a"))).await.unwrap_err();

        assert!(matches!(err, RestoreError::LiveTwin { id, twin: t } if id == first && t == twin), "{err:?}");
        assert!(err.to_string().contains(&format!("#{twin}")), "{err}");
        assert!(columns(&store, first).await.0.is_some(), "the row stays deleted");
        assert_eq!(stored_vector(&store, first).await, None);
    }

    async fn set_key(store: &MemoryStore, id: i64, key: &str) {
        let key = key.to_owned();
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET topic_key = ?2 WHERE id = ?1", rusqlite::params![id, key]))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_row_whose_key_is_live_again_is_not_restored() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let first = save(&store, "a", "storage engine is sqlite").await;
        set_key(&store, first, "storage/engine").await;
        delete(&store, first).await;
        let current = save(&store, "a", "storage engine moved to postgres").await;
        set_key(&store, current, "storage/engine").await;

        let err = restore(&store, first, Some(&shared("a"))).await.unwrap_err();

        assert!(
            matches!(err, RestoreError::TopicTaken { id, live, ref key } if id == first && live == current && key == "storage/engine"),
            "{err:?}"
        );
        assert!(err.to_string().contains(&format!("#{current}")), "{err}");
        assert!(columns(&store, first).await.0.is_some(), "the row stays deleted");
        assert_eq!(stored_vector(&store, first).await, None);

        delete(&store, current).await;
        assert_eq!(restore(&store, first, Some(&shared("a"))).await.unwrap().status, "active");
    }

    #[tokio::test]
    async fn a_row_whose_key_another_agent_holds_is_not_restored() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let first = save(&store, "a", "storage engine is sqlite").await;
        set_key(&store, first, "storage/engine").await;
        delete(&store, first).await;
        let theirs = save(&store, "b", "storage engine is duckdb").await;
        set_key(&store, theirs, "storage/engine").await;

        let err = restore(&store, first, Some(&shared("a"))).await.unwrap_err();
        assert!(matches!(err, RestoreError::TopicTaken { live, .. } if live == theirs), "{err:?}");

        let row = restore(&store, first, Some(&hiding("a", "b"))).await.unwrap();
        assert_eq!(row.status, "active", "a holder the reader cannot see does not hold the key for it");
    }

    #[tokio::test]
    async fn a_restore_finds_another_agents_twin() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let first = save(&store, "a", "the deploy runs on fridays").await;
        delete(&store, first).await;
        let twin = save(&store, "b", "the deploy runs on fridays").await;

        let err = restore(&store, first, Some(&shared("a"))).await.unwrap_err();

        assert!(matches!(err, RestoreError::LiveTwin { id, twin: t } if id == first && t == twin), "{err:?}");
    }

    #[tokio::test]
    async fn a_row_holding_a_secret_stays_deleted() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = save(&store, "a", "the key is sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123").await;
        delete(&store, id).await;

        let err = restore(&store, id, Some(&shared("a"))).await.unwrap_err();

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

        let err = restore_memory_core(&store, &BrokenEmbedder, id, None).await.unwrap_err();

        assert!(matches!(err, RestoreError::Embed(_, EmbedError::NotReady)), "{err:?}");
        assert_eq!(columns(&store, id).await, before);
        assert_eq!(stored_vector(&store, id).await, None);
    }

    #[tokio::test]
    async fn a_stale_vector_does_not_stop_a_restore() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let content = "a row deleted before deletes pruned vectors";
        let id = save(&store, "a", content).await;
        // Marked by hand, as the delete of an older version did: the vector
        // stays, and it is one an earlier model wrote.
        let earlier_model: Vec<u8> = crate::memory::vector::to_unit(&[0.3; 384])
            .unwrap()
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        store
            .conn
            .call(move |c| {
                c.execute("UPDATE memory_vectors SET embedding = ?2 WHERE rowid = ?1", rusqlite::params![id, earlier_model])?;
                c.execute("UPDATE memory SET deleted_at = 1 WHERE id = ?1", [id])
            })
            .await
            .unwrap();

        restore(&store, id, None).await.unwrap();

        let expected = crate::memory::vector::to_unit(&passage(content).await).unwrap();
        assert_eq!(stored_vector(&store, id).await, Some(expected), "embedded by the current model, never kept");
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
        assert_eq!(memory_status(&store, other).await, "disputed", "its dispute with the restored row is back");
    }

    #[tokio::test]
    async fn a_restored_row_brings_its_escalated_pair_back() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let old = save(&store, "a", "the api listens on 8080").await;
        let new = save(&store, "a", "the api listens on 9090").await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE memory SET status = 'disputed', disputed_against = ?2 WHERE id = ?1",
                    rusqlite::params![new, format!("[{old}]")],
                )?;
                c.execute(
                    "INSERT INTO memory_conflict (new_id, old_id, distance, state, created_at) \
                     VALUES (?1, ?2, 0.3, 'escalated', 0)",
                    [new, old],
                )
            })
            .await
            .unwrap();
        delete(&store, old).await;
        assert_eq!(memory_status(&store, new).await, "active", "the delete released it");

        restore(&store, old, None).await.unwrap();

        assert_eq!(memory_status(&store, new).await, "disputed", "the pair is back with the row");
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

        assert_eq!(row, RestoredRow { id, status: "superseded".into(), superseded_by: Some(keep), ..RestoredRow::default() });
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
        let gap = crate::memory::read_save_core(&store, first, None).await.unwrap().unwrap();
        assert_eq!(gap.missing, vec![2]);

        restore(&store, second, Some(&shared("a"))).await.unwrap();

        let behind = crate::memory::chunker::embedding_text("Notes on clawg-ledger.", "the second piece of a long save");
        let expected = crate::memory::vector::to_unit(&passage(&behind).await).unwrap();
        assert_eq!(stored_vector(&store, second).await, Some(expected));
        let whole = crate::memory::read_save_core(&store, first, None).await.unwrap().unwrap();
        assert!(whole.missing.is_empty(), "{whole:?}");
        assert_eq!(whole.ids, vec![Some(first), Some(second), Some(third)]);
    }

    async fn keyed_save(store: &MemoryStore, key: &str) -> (i64, i64, i64) {
        let first = save(store, "a", "Notes on clawg-ledger. The first piece of a long save").await;
        let second = save(store, "a", "the second piece of a long save").await;
        let third = save(store, "a", "the third piece of a long save").await;
        link_save(store, vec![(first, ""), (second, "Notes on clawg-ledger."), (third, "Notes on clawg-ledger.")]).await;
        for id in [first, second, third] {
            set_key(store, id, key).await;
        }
        (first, second, third)
    }

    #[tokio::test]
    async fn a_piece_of_a_keyed_save_is_restored_beside_its_live_siblings() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let (first, second, third) = keyed_save(&store, "ledger/notes").await;
        delete(&store, second).await;

        let row = restore(&store, second, Some(&shared("a"))).await.unwrap();

        assert_eq!(
            row,
            RestoredRow {
                id: second,
                status: "active".into(),
                topic_key: Some("ledger/notes".into()),
                ..RestoredRow::default()
            }
        );
        assert!(columns(&store, second).await.0.is_none(), "the piece is live");
        assert!(stored_vector(&store, second).await.is_some());
        assert_eq!(text_matches(&store, "second").await, vec![second]);
        assert!(columns(&store, first).await.0.is_none() && columns(&store, third).await.0.is_none());
    }

    #[tokio::test]
    async fn a_whole_keyed_save_is_restored_piece_by_piece() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let (first, second, third) = keyed_save(&store, "ledger/notes").await;
        for id in [first, second, third] {
            delete(&store, id).await;
        }

        for id in [first, second, third] {
            assert_eq!(restore(&store, id, Some(&shared("a"))).await.unwrap().status, "active", "piece #{id}");
        }
    }

    #[tokio::test]
    async fn a_live_row_of_another_save_blocks_the_restore_of_a_piece() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let (_, second, _) = keyed_save(&store, "ledger/notes").await;
        delete(&store, second).await;
        let other = save(&store, "a", "ledger notes were rewritten").await;
        set_key(&store, other, "ledger/notes").await;

        let err = restore(&store, second, Some(&shared("a"))).await.unwrap_err();

        assert!(matches!(err, RestoreError::TopicTaken { id, live, .. } if id == second && live == other), "{err:?}");
        assert!(columns(&store, second).await.0.is_some(), "the piece stays deleted");
    }

    async fn supersede(store: &MemoryStore, id: i64, by: i64, cause: &str) {
        let cause = cause.to_owned();
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE memory SET status = 'superseded', superseded_by = ?2, supersede_cause = ?3 WHERE id = ?1",
                    rusqlite::params![id, by, cause],
                )
            })
            .await
            .unwrap();
    }

    /// `status`, `superseded_by`, `supersede_cause` and `revived_from`.
    async fn lineage(store: &MemoryStore, id: i64) -> (String, Option<i64>, Option<String>, Option<i64>) {
        store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT status, superseded_by, supersede_cause, revived_from FROM memory WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
            })
            .await
            .unwrap()
    }

    fn current() -> (String, Option<i64>, Option<String>, Option<i64>) {
        ("active".into(), None, None, None)
    }

    fn replaced_by(by: i64, cause: &str) -> (String, Option<i64>, Option<String>, Option<i64>) {
        ("superseded".into(), Some(by), Some(cause.into()), None)
    }

    fn revived_by(deleted: i64) -> (String, Option<i64>, Option<String>, Option<i64>) {
        ("active".into(), None, None, Some(deleted))
    }

    async fn keyed(store: &MemoryStore, agent: &str, content: &str, key: &str) -> i64 {
        let id = save(store, agent, content).await;
        set_key(store, id, key).await;
        id
    }

    async fn delete_reporting(store: &MemoryStore, id: i64) -> Deleted {
        delete_memory_core(store, id, Some(&shared("a"))).await.unwrap()
    }

    #[tokio::test]
    async fn deleting_the_current_revision_revives_the_one_it_replaced() {
        use crate::memory::search::{search_all_memories, SearchScope};
        let store = MemoryStore::new_for_test().await.unwrap();
        let content = "Storage engine is SQLite\nchosen for the basalt file";
        let v1 = keyed(&store, "a", content, "storage/engine").await;
        let v2 = keyed(&store, "a", "Storage engine moved to Postgres", "storage/engine").await;
        supersede(&store, v1, v2, "revision").await;

        let outcome = delete_reporting(&store, v2).await;

        assert_eq!(
            outcome,
            Deleted {
                deleted: 1,
                revived: vec![Replaced { id: v1, excerpt: "Storage engine is SQLite".into() }],
                hidden: vec![]
            }
        );
        assert_eq!(lineage(&store, v1).await, revived_by(v2));
        let hits = search_all_memories(&store, &passage(content).await, None, "a", SearchScope::all_default(), 10)
            .await
            .unwrap();
        assert!(hits.iter().any(|h| h.id == v1), "the vector search finds it again");
        assert_eq!(text_matches(&store, "basalt").await, vec![v1]);
        assert_eq!(delete_reporting(&store, v2).await, Deleted::default(), "a second delete brings nothing back");
    }

    #[tokio::test]
    async fn deleting_the_head_of_a_chain_revives_only_the_one_before() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v1 = keyed(&store, "a", "api port is 8080", "api/port").await;
        let v2 = keyed(&store, "a", "api port is 9090", "api/port").await;
        let v3 = keyed(&store, "a", "api port is 7070", "api/port").await;
        supersede(&store, v1, v2, "revision").await;
        supersede(&store, v2, v3, "revision").await;

        let outcome = delete_reporting(&store, v3).await;

        assert_eq!(outcome.revived.iter().map(|r| r.id).collect::<Vec<_>>(), vec![v2]);
        assert_eq!(lineage(&store, v2).await, revived_by(v3));
        assert_eq!(lineage(&store, v1).await, replaced_by(v2, "revision"));
    }

    #[tokio::test]
    async fn deleting_a_replaced_revision_revives_nothing() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v1 = keyed(&store, "a", "api port is 8080", "api/port").await;
        let v2 = keyed(&store, "a", "api port is 9090", "api/port").await;
        let v3 = keyed(&store, "a", "api port is 7070", "api/port").await;
        supersede(&store, v1, v2, "revision").await;
        supersede(&store, v2, v3, "revision").await;

        let outcome = delete_reporting(&store, v2).await;

        assert_eq!(outcome, Deleted { deleted: 1, ..Deleted::default() });
        assert_eq!(lineage(&store, v1).await, replaced_by(v2, "revision"));
        assert_eq!(lineage(&store, v3).await, current());
    }

    #[tokio::test]
    async fn deleting_a_verdict_winner_leaves_the_loser_hidden() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let loser = save(&store, "a", "the api listens on 8080").await;
        let winner = save(&store, "a", "the api listens on 9090").await;
        supersede(&store, loser, winner, "verdict").await;

        let outcome = delete_reporting(&store, winner).await;

        assert_eq!(outcome, Deleted { deleted: 1, revived: vec![], hidden: vec![loser] });
        assert_eq!(lineage(&store, loser).await, replaced_by(winner, "verdict"));
    }

    #[tokio::test]
    async fn deleting_an_undone_revision_revives_nothing() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v1 = keyed(&store, "a", "api port is 8080", "api/port").await;
        let v2 = keyed(&store, "a", "api port is 9090", "api/port").await;
        // The revision to v2 was undone with a `supersedes` verdict keeping v1.
        supersede(&store, v2, v1, "verdict").await;

        let outcome = delete_reporting(&store, v1).await;

        assert_eq!(outcome, Deleted { deleted: 1, revived: vec![], hidden: vec![v2] });
        assert_eq!(lineage(&store, v2).await, replaced_by(v1, "verdict"));
    }

    #[tokio::test]
    async fn a_revived_save_comes_back_whole_and_the_rest_repoint() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let older = keyed(&store, "a", "ledger notes, the oldest version", "ledger/notes").await;
        let (first, second, third) = keyed_save(&store, "ledger/notes").await;
        let current_row = keyed(&store, "a", "ledger notes were rewritten", "ledger/notes").await;
        for id in [older, first, second, third] {
            supersede(&store, id, current_row, "revision").await;
        }

        let outcome = delete_reporting(&store, current_row).await;

        assert_eq!(outcome.revived.iter().map(|r| r.id).collect::<Vec<_>>(), vec![first, second, third]);
        for id in [first, second, third] {
            assert_eq!(lineage(&store, id).await, revived_by(current_row), "piece #{id}");
        }
        assert_eq!(lineage(&store, older).await, replaced_by(first, "revision"));
    }

    #[tokio::test]
    async fn deleting_every_piece_of_a_long_revision_revives_the_one_before() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let older = keyed(&store, "a", "ledger notes, the older version", "ledger/notes").await;
        let (first, second, third) = keyed_save(&store, "ledger/notes").await;
        supersede(&store, older, first, "revision").await;

        assert!(delete_reporting(&store, first).await.revived.is_empty(), "its other pieces still hold the key");
        assert!(delete_reporting(&store, second).await.revived.is_empty());
        let last = delete_reporting(&store, third).await;

        assert_eq!(last.revived.iter().map(|r| r.id).collect::<Vec<_>>(), vec![older]);
        assert_eq!(lineage(&store, older).await, revived_by(first));
        restore(&store, first, Some(&shared("a"))).await.unwrap();
        assert_eq!(lineage(&store, older).await, replaced_by(first, "revision"), "restoring the head takes it back");
    }

    #[tokio::test]
    async fn deleting_another_agents_revision_revives_the_earlier_one() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let theirs = keyed(&store, "b", "api port is 8080", "api/port").await;
        let mine = keyed(&store, "a", "api port is 9090", "api/port").await;
        supersede(&store, theirs, mine, "revision").await;

        let outcome = delete_reporting(&store, mine).await;

        assert_eq!(outcome.revived.iter().map(|r| r.id).collect::<Vec<_>>(), vec![theirs], "{outcome:?}");
        assert_eq!(lineage(&store, theirs).await, revived_by(mine));
    }

    #[tokio::test]
    async fn a_revival_stays_in_its_project() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let elsewhere = keyed(&store, "a", "api port is 8080", "api/port").await;
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET project_id = 'q' WHERE id = ?1", [elsewhere]))
            .await
            .unwrap();
        let mine = keyed(&store, "a", "api port is 9090", "api/port").await;
        supersede(&store, elsewhere, mine, "revision").await;

        let outcome = delete_reporting(&store, mine).await;

        assert!(outcome.revived.is_empty(), "{outcome:?}");
        assert_eq!(lineage(&store, elsewhere).await, replaced_by(mine, "revision"));
    }

    #[tokio::test]
    async fn a_revived_row_is_disputed_again() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v1 = keyed(&store, "a", "api port is 8080", "api/port").await;
        let challenger = save(&store, "a", "the api listens on 6060").await;
        let v2 = keyed(&store, "a", "api port is 9090", "api/port").await;
        store
            .conn
            .call(move |c| {
                c.execute("UPDATE memory SET disputed_against = ?2 WHERE id = ?1", rusqlite::params![challenger, format!("[{v1}]")])
            })
            .await
            .unwrap();
        supersede(&store, v1, v2, "revision").await;

        delete_reporting(&store, v2).await;

        assert_eq!(memory_status(&store, challenger).await, "disputed");
    }

    #[tokio::test]
    async fn restoring_the_deleted_revision_replaces_the_revived_one_again() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v1 = keyed(&store, "a", "Storage engine is SQLite", "storage/engine").await;
        let v2 = keyed(&store, "a", "Storage engine moved to Postgres", "storage/engine").await;
        supersede(&store, v1, v2, "revision").await;
        delete_reporting(&store, v2).await;

        let row = restore(&store, v2, Some(&shared("a"))).await.unwrap();

        assert_eq!(
            row,
            RestoredRow {
                id: v2,
                status: "active".into(),
                topic_key: Some("storage/engine".into()),
                replaced: vec![Replaced { id: v1, excerpt: "Storage engine is SQLite".into() }],
                ..RestoredRow::default()
            }
        );
        assert_eq!(lineage(&store, v1).await, replaced_by(v2, "revision"));
        assert_eq!(lineage(&store, v2).await, current());
    }

    #[tokio::test]
    async fn a_newer_revision_still_blocks_the_restore() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v1 = keyed(&store, "a", "Storage engine is SQLite", "storage/engine").await;
        let v2 = keyed(&store, "a", "Storage engine moved to Postgres", "storage/engine").await;
        supersede(&store, v1, v2, "revision").await;
        delete_reporting(&store, v2).await;
        let v3 = keyed(&store, "a", "Storage engine moved to DuckDB", "storage/engine").await;
        supersede(&store, v1, v3, "revision").await;

        let err = restore(&store, v2, Some(&shared("a"))).await.unwrap_err();

        assert!(matches!(err, RestoreError::TopicTaken { id, live, .. } if id == v2 && live == v3), "{err:?}");
        assert_eq!(lineage(&store, v3).await, current());
    }

    #[tokio::test]
    async fn a_row_replaced_by_a_deleted_revision_comes_back_current() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v1 = keyed(&store, "a", "Storage engine is SQLite", "storage/engine").await;
        let v2 = keyed(&store, "a", "Storage engine moved to Postgres", "storage/engine").await;
        supersede(&store, v1, v2, "revision").await;
        delete_reporting(&store, v1).await;
        delete_reporting(&store, v2).await;

        let row = restore(&store, v1, Some(&shared("a"))).await.unwrap();

        assert_eq!((row.status.as_str(), row.superseded_by, row.superseded_by_deleted), ("active", None, false));
        assert_eq!(lineage(&store, v1).await, revived_by(v2));

        restore(&store, v2, Some(&shared("a"))).await.unwrap();
        assert_eq!(lineage(&store, v1).await, replaced_by(v2, "revision"), "restoring v2 takes it back");
    }

    #[tokio::test]
    async fn a_row_replaced_by_a_deleted_verdict_winner_says_so() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let loser = save(&store, "a", "the api listens on 8080").await;
        let winner = save(&store, "a", "the api listens on 9090").await;
        supersede(&store, loser, winner, "verdict").await;
        delete_reporting(&store, loser).await;
        delete_reporting(&store, winner).await;

        let row = restore(&store, loser, Some(&shared("a"))).await.unwrap();

        assert_eq!(
            row,
            RestoredRow {
                id: loser,
                status: "superseded".into(),
                superseded_by: Some(winner),
                superseded_by_deleted: true,
                ..RestoredRow::default()
            }
        );
    }

    /// Answers every text with one vector, so a test places the restored row.
    struct PlacedEmbedder(Vec<f32>);

    #[async_trait::async_trait]
    impl Embedder for PlacedEmbedder {
        async fn embed_with_role(&self, _: &str, _: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            Ok(self.0.clone())
        }
        fn dimensions(&self) -> usize {
            384
        }
        fn model_id(&self) -> &str {
            "placed"
        }
        fn query_prefix(&self) -> &str {
            ""
        }
        fn passage_prefix(&self) -> &str {
            ""
        }
    }

    /// The first axis turned toward axis `toward` far enough that the two sit
    /// at L2 distance `distance`.
    fn placed(toward: usize, distance: f32) -> Vec<f32> {
        let angle = 2.0 * (distance / 2.0).asin();
        let mut v = vec![0.0f32; 384];
        v[0] = angle.cos();
        v[toward] = angle.sin();
        v
    }

    /// A live row of agent `a` in project `p` with the vector `vector`,
    /// created at `created_at`.
    async fn save_placed(store: &MemoryStore, content: &str, memory_type: &str, vector: Vec<f32>, created_at: i64) -> i64 {
        let bytes: Vec<u8> = vector.iter().flat_map(|f| f.to_le_bytes()).collect();
        let hash = crate::memory::dedup::content_hash(content);
        let (content, memory_type) = (content.to_owned(), memory_type.to_owned());
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory (scope, agent_id, project_id, content, content_hash, memory_type, \
                     source, created_at, created_by_type, created_by_ref, last_modified_at, \
                     last_accessed_at, importance, tier, status) \
                     VALUES ('agent', 'a', 'p', ?1, ?2, ?3, 'explicit', ?4, 'agent', 'a', ?4, ?4, 5.0, 3, 'active')",
                    rusqlite::params![content, hash, memory_type, created_at],
                )?;
                let id = c.last_insert_rowid();
                c.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?1, ?2)", rusqlite::params![id, bytes])?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    async fn deleted_at(store: &MemoryStore, id: i64) -> i64 {
        columns(store, id).await.0.expect("deleted")
    }

    type Pair = (i64, i64, String, Option<String>);

    async fn pairs(store: &MemoryStore) -> Vec<Pair> {
        store
            .conn
            .call(|c| -> rusqlite::Result<Vec<Pair>> {
                let mut stmt = c.prepare("SELECT new_id, old_id, state, verdict FROM memory_conflict ORDER BY new_id, old_id")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
                rows.collect()
            })
            .await
            .unwrap()
    }

    async fn restore_placed(store: &MemoryStore, id: i64) -> RestoredRow {
        restore_memory_core(store, &PlacedEmbedder(placed(1, 0.0)), id, Some(&shared("a"))).await.unwrap()
    }

    #[tokio::test]
    async fn a_restored_row_is_paired_with_what_was_saved_while_deleted() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let restored = save_placed(&store, "the api listens on 8080", "semantic_fact", placed(1, 0.0), 1).await;
        delete(&store, restored).await;
        let after = deleted_at(&store, restored).await + 1;
        let newer = save_placed(&store, "the api listens on 9090\nsince the proxy moved", "semantic_fact", placed(2, 0.3), after).await;

        let row = restore_placed(&store, restored).await;

        assert_eq!(row.related.iter().map(|c| (c.id, c.escalated)).collect::<Vec<_>>(), vec![(newer, false)]);
        assert_eq!(row.related[0].excerpt, "the api listens on 9090");
        assert_eq!(pairs(&store).await, vec![(newer, restored, "pending".into(), None)]);
        assert_eq!(memory_status(&store, newer).await, "active");
        assert_eq!(row.status, "active");
    }

    #[tokio::test]
    async fn an_escalated_pair_from_a_restore_disputes_the_newer_row() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let restored = save_placed(&store, "storage engine is sqlite", "architectural_decision", placed(1, 0.0), 1).await;
        delete(&store, restored).await;
        let after = deleted_at(&store, restored).await + 1;
        let newer = save_placed(&store, "storage engine is postgres", "architectural_decision", placed(2, 0.3), after).await;

        let row = restore_placed(&store, restored).await;

        assert_eq!(row.related.iter().map(|c| (c.id, c.escalated)).collect::<Vec<_>>(), vec![(newer, true)]);
        assert_eq!(pairs(&store).await, vec![(newer, restored, "escalated".into(), None)]);
        assert_eq!(memory_status(&store, newer).await, "disputed");
        assert_eq!(row.status, "active", "the newer row carries the dispute");
        let named: Option<String> = store
            .conn
            .call(move |c| c.query_row("SELECT disputed_against FROM memory WHERE id = ?1", [newer], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(named, Some(format!("[{restored}]")));
    }

    #[tokio::test]
    async fn rows_saved_before_the_delete_or_already_paired_are_not_paired_again() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let restored = save_placed(&store, "the api listens on 8080", "semantic_fact", placed(1, 0.0), 1).await;
        let before = save_placed(&store, "the api listens on 9090", "semantic_fact", placed(1, 0.3), 2).await;
        delete(&store, restored).await;
        let after = deleted_at(&store, restored).await + 1;
        let judged = save_placed(&store, "the api listens on 7070", "semantic_fact", placed(2, 0.3), after).await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory_conflict (new_id, old_id, distance, state, verdict, created_at, judged_at) \
                     VALUES (?1, ?2, 0.3, 'judged', 'compatible', ?3, ?3)",
                    [judged, restored, after],
                )
            })
            .await
            .unwrap();

        let row = restore_placed(&store, restored).await;

        assert!(row.related.is_empty(), "{:?}", row.related);
        assert_eq!(pairs(&store).await, vec![(judged, restored, "judged".into(), Some("compatible".into()))]);
        assert_eq!(memory_status(&store, before).await, "active");
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

        restore(&store, alone_id, Some(&shared("a"))).await.unwrap();
        restore(&store, piece_id, Some(&shared("a"))).await.unwrap();

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
