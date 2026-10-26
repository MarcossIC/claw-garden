//! Secrets already in the store: finding the rows that hold one, and purging
//! a row for good.
//!
//! New saves are masked on the way in (see [`crate::memory::redact`]); this
//! is for what was stored before, which nothing rewrites on its own.

use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;

use crate::memory::backup::{
    known_migrations, open_existing, open_for_probe, parse_snapshot_name, read_history, schema_plan,
    BackupError, SchemaPlan,
};
use crate::memory::clock::TimestampMs;
use crate::memory::dispute::release_challengers;
use crate::memory::redact::{redact, SecretKind};
use crate::memory::revival::{revive_after_delete_of_piece, Revived};
use crate::memory::store_lock::{StoreHolder, StoreLock};
use crate::memory::project_delete::Counts;
use crate::memory::project_docs::drop_root_index_in;
use crate::memory::workspace::{project_workspaces, workspaces_holding_any, Holding};
use crate::memory::MemoryError;

/// A `memory` row whose content holds at least one secret. Carries what the
/// row is and what it holds, never the content itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flagged {
    pub id: i64,
    pub kinds: Vec<SecretKind>,
    pub deleted: bool,
}

/// A pair of `memory_conflict` whose verdict reason holds a secret, by the
/// ids of its two rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlaggedPair {
    pub new_id: i64,
    pub old_id: i64,
    pub kinds: Vec<SecretKind>,
}

/// What [`scan`] found, each list in id order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scanned {
    pub memories: Vec<Flagged>,
    pub pairs: Vec<FlaggedPair>,
}

/// Every row of `memory`, live or deleted, and every verdict reason that
/// holds a secret.
///
/// Reads without the lock and without writing, so it works beside a running
/// daemon: WAL lets a reader in while the daemon writes.
pub fn scan(base_dir: &Path) -> Result<Scanned, MemoryError> {
    let path = base_dir.join("memory.db");
    if !path.exists() {
        return Err(BackupError::NoStore { path }.into());
    }
    crate::memory::db::ensure_vec0_registered();
    let c = open_for_probe(&path)?;
    let mut stmt = c.prepare("SELECT id, content, deleted_at IS NOT NULL FROM memory ORDER BY id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?)))?;
    let mut scanned = Scanned::default();
    for row in rows {
        let (id, content, deleted) = row?;
        let kinds = redact(&content).kinds;
        if !kinds.is_empty() {
            scanned.memories.push(Flagged { id, kinds, deleted });
        }
    }
    scanned.pairs = reasons_with_a_secret(&c)?.into_iter().map(|(pair, _)| pair).collect();
    Ok(scanned)
}

/// The verdict reasons that hold a secret, with each reason as stored. A
/// store from before the pairs has none.
fn reasons_with_a_secret(c: &rusqlite::Connection) -> rusqlite::Result<Vec<(FlaggedPair, String)>> {
    if !has_table(c, "memory_conflict")? {
        return Ok(Vec::new());
    }
    let mut stmt =
        c.prepare("SELECT new_id, old_id, reason FROM memory_conflict WHERE reason IS NOT NULL ORDER BY new_id, old_id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?;
    let mut flagged = Vec::new();
    for row in rows {
        let (new_id, old_id, reason) = row?;
        let kinds = redact(&reason).kinds;
        if !kinds.is_empty() {
            flagged.push((FlaggedPair { new_id, old_id, kinds }, reason));
        }
    }
    Ok(flagged)
}

fn has_table(c: &rusqlite::Connection, name: &str) -> rusqlite::Result<bool> {
    c.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", [name], |r| {
        r.get::<_, i64>(0)
    })
    .map(|n| n > 0)
}

/// Why a purge did not run.
#[derive(Debug, thiserror::Error)]
pub enum PurgeError {
    #[error("there is no memory #{0}")]
    NotFound(i64),
    #[error("there is no delete #{0}")]
    UnknownDelete(i64),
    #[error("delete #{0} was undone: there is nothing of it to purge")]
    DeleteUndone(i64),
    #[error("delete #{0} was already purged")]
    DeletePurged(i64),
    #[error("memory.db has migrations pending (V{from} to V{to}); start the daemon once so it \
             migrates them, stop it, and run this again")]
    Pending { from: i32, to: i32 },
    #[error("memory.db stayed busy after the rewrite, so its WAL could not be emptied; \
             make sure no other clawg is running and run this again")]
    Busy,
}

/// What [`purge`] did.
#[derive(Debug, PartialEq, Eq)]
pub struct Purged {
    /// The rows the purged one had replaced as the current version of its
    /// topic, current again, as a delete of it would have left them.
    pub revived: Vec<i64>,
    /// The rows a verdict replaced with the purged one, which stay replaced.
    pub hidden: Vec<i64>,
    /// The other rows of the long save the row was a piece of, in id order.
    /// Neighbours share the tokens at their edges, so each holds a little of
    /// the purged text.
    pub pieces: Vec<i64>,
    /// The outbox rows removed with the row's text: saves accepted and not
    /// stored, or failed after they were accepted.
    pub outbox: Vec<OutboxRemoved>,
    /// Snapshots in `<data>/backups` that still hold the row, by name. None
    /// is changed: deleting them is the user's call.
    pub backups: Vec<PathBuf>,
    /// The `workspace.md` files that still hold the row's text, in path
    /// order. None is changed: the AUTO section is rewritten by the agent's
    /// next context there, and the rest is the user's.
    pub workspaces: Vec<WorkspaceHolding>,
}

/// A `workspace.md` that holds a purged text, and in which section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceHolding {
    pub path: PathBuf,
    pub holding: Holding,
}

/// An outbox row a purge removed, by the save it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxRemoved {
    pub operation_id: String,
    pub agent_id: String,
    pub state: String,
}

/// Which outbox rows hold a purged row's text. The save of a row with no
/// `memory_piece` link was that row alone, so its outbox row holds exactly
/// the row's text; a piece of a long save is somewhere in the whole save.
/// Either way only the save of the row's own agent: another agent's save that
/// says the same is that agent's, and accepted. A row with no agent is
/// matched in every agent's saves.
struct OutboxMatch<'a> {
    agent: Option<&'a str>,
    content: &'a str,
    piece: bool,
}

const OUTBOX_MATCH: &str = "(?1 IS NULL OR agent_id = ?1) \
     AND CASE WHEN ?3 THEN instr(content, ?2) > 0 ELSE content = ?2 END";

impl OutboxMatch<'_> {
    fn params(&self) -> impl rusqlite::Params + '_ {
        (self.agent, self.content, self.piece)
    }
}

/// Removes `memory` row `id` for good: the row, its vector, its terms in the
/// full-text index and the outbox rows holding its text, so that text is in
/// neither `memory.db` nor its WAL.
///
/// Only on a stopped store that is up to date: the lock is taken first, and
/// nothing is written before every check passes. No snapshot is taken first,
/// on purpose: it would be one more copy of what is being removed.
pub fn purge(base_dir: &Path, id: i64) -> Result<Purged, MemoryError> {
    let Some((_lock, mut c)) = open_to_rewrite(base_dir)? else {
        return Err(PurgeError::NotFound(id).into());
    };
    let (content, live, agent, piece): (String, bool, Option<String>, bool) = c
        .query_row(
            "SELECT content, deleted_at IS NULL, agent_id, \
                    EXISTS (SELECT 1 FROM memory_piece WHERE memory_id = ?1) \
             FROM memory WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or(PurgeError::NotFound(id))?;
    let held = OutboxMatch { agent: agent.as_deref(), content: &content, piece };

    let tx = c.transaction()?;
    // Deleted first, so the revival a delete runs sees it as one: the rows it
    // replaced as its topic's current version come back. Purging must not
    // hide more than the row.
    let revived = if live {
        let now = TimestampMs::now();
        tx.execute("UPDATE memory SET deleted_at = ?2 WHERE id = ?1", rusqlite::params![id, now])?;
        revive_after_delete_of_piece(&tx, id, now)?
    } else {
        Revived::default()
    };
    let (pieces, outbox) = remove_in(&tx, id, &held)?;
    // After the delete, so the row no longer counts as live for them.
    if live {
        release_challengers(&tx, id, TimestampMs::now())?;
    }
    tx.commit()?;
    // FTS5 records a removal as a new entry that carries the row's terms, and
    // keeps the old segments until a merge: both still hold the text. A full
    // merge rewrites the index without them.
    c.execute("INSERT INTO memory_fts(memory_fts) VALUES ('optimize')", [])?;
    leave_no_old_copy(c)?;
    let backups = backups_holding(&crate::paths::backups_dir(base_dir), id, &held);
    Ok(Purged {
        revived: revived.revived.into_iter().map(|r| r.id).collect(),
        hidden: revived.hidden,
        pieces,
        outbox,
        backups,
        workspaces: workspaces_holding_any(&base_dir.join("workspaces"), &[&content])
            .into_iter()
            .map(|(path, holding)| WorkspaceHolding { path, holding })
            .collect(),
    })
}

/// Removes the deleted `memory` row `id` for good inside the caller's
/// transaction: its vector, its piece heads, its links as a conclusion, its
/// pairs and the outbox rows `held` matches. Answers the other pieces of its
/// save and the outbox rows removed.
fn remove_in(tx: &rusqlite::Connection, id: i64, held: &OutboxMatch) -> rusqlite::Result<(Vec<i64>, Vec<OutboxRemoved>)> {
    let pieces: Vec<i64> = {
        let mut stmt = tx.prepare(
            "SELECT DISTINCT q.memory_id FROM memory_piece p \
             JOIN memory_piece q ON q.save_id = p.save_id JOIN memory m ON m.id = q.memory_id \
             WHERE p.memory_id = ?1 AND q.memory_id <> ?1 ORDER BY q.memory_id",
        )?;
        let rows = stmt.query_map([id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    // Every later piece of a save was embedded behind its opening, which the
    // links keep as text. The link of the row itself stays: it holds an id,
    // and says which piece is missing.
    tx.execute(
        "UPDATE memory_piece SET context = '' \
         WHERE save_id IN (SELECT save_id FROM memory_piece WHERE memory_id = ?1 AND piece = 1)",
        [id],
    )?;
    tx.execute("DELETE FROM memory_vectors WHERE rowid = ?1", [id])?;
    tx.execute("DELETE FROM memory WHERE id = ?1", [id])?;
    // A purged conclusion rests on nothing. A link that names the row as a
    // source stays: it holds an id, never text, and it is the only way the
    // conclusion learns it lost a source. `AUTOINCREMENT` never hands the id
    // to a later row.
    tx.execute("DELETE FROM memory_derivation WHERE memory_id = ?1", [id])?;
    tx.execute("DELETE FROM memory_conflict WHERE new_id = ?1 OR old_id = ?1", [id])?;
    // A save not stored yet, or that failed after it was accepted, holds the
    // text in the outbox.
    let outbox: Vec<OutboxRemoved> = {
        let mut stmt = tx.prepare(&format!(
            "SELECT operation_id, agent_id, state FROM ingest_outbox WHERE {OUTBOX_MATCH} ORDER BY id"
        ))?;
        let rows = stmt.query_map(held.params(), |r| {
            Ok(OutboxRemoved { operation_id: r.get(0)?, agent_id: r.get(1)?, state: r.get(2)? })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    tx.execute(&format!("DELETE FROM ingest_outbox WHERE {OUTBOX_MATCH}"), held.params())?;
    Ok((pieces, outbox))
}

/// A row of a delete that is live again, and so stays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRow {
    /// `memory`, `episodic_trajectory` or `procedural_skill`.
    pub table: String,
    pub id: i64,
}

/// What [`purge_delete`] did.
#[derive(Debug, PartialEq, Eq)]
pub struct PurgedDelete {
    pub project_id: String,
    /// The rows removed for good, by table.
    pub removed: Counts,
    /// The rows of the delete that are live again, which stay.
    pub live: Vec<LiveRow>,
    /// The outbox rows removed with the texts of the rows.
    pub outbox: Vec<OutboxRemoved>,
    /// Snapshots in `<data>/backups` that still hold a row of the delete, by
    /// name, the `pre-delete` one among them. None is changed.
    pub backups: Vec<PathBuf>,
    /// The project's own `workspace.md` files, whatever they hold. None is
    /// changed.
    pub project_workspaces: Vec<PathBuf>,
    /// The other `workspace.md` files that hold a text of the rows. None is
    /// changed.
    pub workspaces: Vec<WorkspaceHolding>,
}

/// Removes for good every row of delete `delete_id` that is still deleted,
/// wherever it is now, in the three tables: the row, its vector and, for a
/// row of `memory`, what [`purge`] removes of a deleted row. The docs index
/// under the folders the delete turned off is emptied again, since a refresh
/// in flight may have written after it, and the delete is marked purged. The
/// project stays as it is.
///
/// On a stopped store that is up to date, as [`purge`], with no snapshot first
/// and nothing written before every check passes. The text index is merged,
/// the file rebuilt and the WAL emptied once, at the end.
pub fn purge_delete(base_dir: &Path, delete_id: i64) -> Result<PurgedDelete, MemoryError> {
    let Some((_lock, mut c)) = open_to_rewrite(base_dir)? else {
        return Err(PurgeError::UnknownDelete(delete_id).into());
    };
    let (project_id, undone, purged): (String, bool, bool) = c
        .query_row(
            "SELECT project_id, undone_at IS NOT NULL, purged_at IS NOT NULL FROM project_delete WHERE delete_id = ?1",
            [delete_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or(PurgeError::UnknownDelete(delete_id))?;
    if undone {
        return Err(PurgeError::DeleteUndone(delete_id).into());
    }
    if purged {
        return Err(PurgeError::DeletePurged(delete_id).into());
    }

    struct Gone {
        id: i64,
        content: String,
        agent: Option<String>,
        piece: bool,
    }
    let tx = c.transaction()?;
    let journaled: Vec<(String, i64)> = {
        let mut stmt = tx.prepare("SELECT tbl, row_id FROM project_delete_row WHERE delete_id = ?1 ORDER BY tbl, row_id")?;
        let rows = stmt.query_map([delete_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut removed = Counts::default();
    let mut live = Vec::new();
    let mut gone: Vec<Gone> = Vec::new();
    for (table, id) in journaled {
        let vectors = match table.as_str() {
            "memory" => "memory_vectors",
            "episodic_trajectory" => "episodic_vectors",
            "procedural_skill" => "procedural_vectors",
            other => {
                log::warn!("delete {delete_id} journaled a row of {other}, which no purge knows");
                continue;
            }
        };
        let state: Option<bool> =
            tx.query_row(&format!("SELECT deleted_at IS NULL FROM {table} WHERE id = ?1"), [id], |r| r.get(0)).optional()?;
        match state {
            None => continue,
            Some(true) => {
                live.push(LiveRow { table, id });
                continue;
            }
            Some(false) => {}
        }
        if table == "memory" {
            let (content, agent, piece): (String, Option<String>, bool) = tx.query_row(
                "SELECT content, agent_id, EXISTS (SELECT 1 FROM memory_piece WHERE memory_id = ?1) FROM memory WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            gone.push(Gone { id, content, agent, piece });
            removed.memories += 1;
        } else {
            tx.execute(&format!("DELETE FROM {vectors} WHERE rowid = ?1"), [id])?;
            tx.execute(&format!("DELETE FROM {table} WHERE id = ?1"), [id])?;
            if table == "episodic_trajectory" {
                removed.trajectories += 1;
            } else {
                removed.skills += 1;
            }
        }
    }
    let mut outbox = Vec::new();
    for g in &gone {
        let held = OutboxMatch { agent: g.agent.as_deref(), content: &g.content, piece: g.piece };
        outbox.extend(remove_in(&tx, g.id, &held)?.1);
    }
    let roots: Vec<String> = {
        let mut stmt = tx.prepare(
            "SELECT f.root_id FROM project_delete_folder f JOIN project_path p ON p.id = f.root_id \
             WHERE f.delete_id = ?1 AND p.index_docs = 0 ORDER BY f.root_id",
        )?;
        let rows = stmt.query_map([delete_id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for root in roots {
        drop_root_index_in(&tx, &root)?;
    }
    tx.execute("UPDATE project_delete SET purged_at = ?2 WHERE delete_id = ?1", rusqlite::params![delete_id, TimestampMs::now()])?;
    tx.commit()?;
    c.execute("INSERT INTO memory_fts(memory_fts) VALUES ('optimize')", [])?;
    // The docs index keeps its own terms until a merge too.
    c.execute("INSERT INTO project_doc_fts(project_doc_fts) VALUES ('optimize')", [])?;
    leave_no_old_copy(c)?;

    let held: Vec<OutboxMatch> =
        gone.iter().map(|g| OutboxMatch { agent: g.agent.as_deref(), content: &g.content, piece: g.piece }).collect();
    let rows: Vec<(i64, &OutboxMatch)> = gone.iter().map(|g| g.id).zip(held.iter()).collect();
    let backups = backups_holding_any(&crate::paths::backups_dir(base_dir), &rows);
    let workspaces_root = base_dir.join("workspaces");
    let project_workspaces = project_workspaces(&workspaces_root, &project_id);
    let texts: Vec<&str> = gone.iter().map(|g| g.content.as_str()).collect();
    let workspaces = workspaces_holding_any(&workspaces_root, &texts)
        .into_iter()
        .filter(|(path, _)| !project_workspaces.contains(path))
        .map(|(path, holding)| WorkspaceHolding { path, holding })
        .collect();
    Ok(PurgedDelete { project_id, removed, live, outbox, backups, project_workspaces, workspaces })
}

/// What [`mask_reasons`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MaskedReasons {
    /// The pairs whose reason was masked, with the kinds it held.
    pub pairs: Vec<FlaggedPair>,
    /// Snapshots in `<data>/backups` that still hold one of those reasons as
    /// it was, by name. None is changed.
    pub backups: Vec<PathBuf>,
}

/// Replaces every verdict reason that holds a secret with its masked text, so
/// the secret is in neither `memory.db` nor its WAL. Every other reason, and
/// everything else in the pair, stays as it is.
///
/// On a stopped store that is up to date, as [`purge`], and with no snapshot
/// first for the same reason.
pub fn mask_reasons(base_dir: &Path) -> Result<MaskedReasons, MemoryError> {
    let Some((_lock, mut c)) = open_to_rewrite(base_dir)? else {
        return Ok(MaskedReasons::default());
    };
    let found = reasons_with_a_secret(&c)?;
    if found.is_empty() {
        return Ok(MaskedReasons::default());
    }
    let tx = c.transaction()?;
    for (pair, reason) in &found {
        tx.execute(
            "UPDATE memory_conflict SET reason = ?3 WHERE new_id = ?1 AND old_id = ?2",
            rusqlite::params![pair.new_id, pair.old_id, redact(reason).text],
        )?;
    }
    tx.commit()?;
    leave_no_old_copy(c)?;
    let backups = backups_holding_reasons(&crate::paths::backups_dir(base_dir), &found);
    Ok(MaskedReasons { pairs: found.into_iter().map(|(pair, _)| pair).collect(), backups })
}

/// The store, locked against every other clawg and at a schema this binary
/// writes, or `None` when it has never been migrated and so holds nothing.
/// Nothing is written before every check passes.
fn open_to_rewrite(base_dir: &Path) -> Result<Option<(StoreLock, rusqlite::Connection)>, MemoryError> {
    let path = base_dir.join("memory.db");
    if !path.exists() {
        return Err(BackupError::NoStore { path }.into());
    }
    let lock = StoreLock::acquire(base_dir, StoreHolder::Maintenance)?;
    let c = open_existing(base_dir)?;
    match schema_plan(&read_history(&c)?, &known_migrations()) {
        SchemaPlan::UpToDate => Ok(Some((lock, c))),
        SchemaPlan::Fresh => Ok(None),
        SchemaPlan::Pending { from, to } => Err(PurgeError::Pending { from, to }.into()),
        SchemaPlan::Newer { db, binary } => Err(MemoryError::SchemaNewer { db, binary }),
        SchemaPlan::Diverged { version } => Err(MemoryError::SchemaDiverged { version }),
    }
}

/// Rebuilds the file and empties the WAL after a rewrite, then closes it.
fn leave_no_old_copy(c: rusqlite::Connection) -> Result<(), MemoryError> {
    // Pages freed long before this rewrite (every merge of the text index
    // frees some) still hold whatever they held, and `secure_delete` only
    // zeroes pages freed from now on. Rebuilding the file leaves no free
    // page. It keeps the rowids vec0 addresses its chunks by: a table with an
    // index is copied with its rowids, and every vec0 shadow table has one.
    c.execute_batch("VACUUM")?;
    // Older frames of the WAL can still hold the pages as they were.
    let busy: i64 = c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
    if busy != 0 {
        return Err(PurgeError::Busy.into());
    }
    Ok(())
}

/// The snapshots in `dir` that hold any of `found` with its reason as it was,
/// by name. One that cannot be read is listed too.
fn backups_holding_reasons(dir: &Path, found: &[(FlaggedPair, String)]) -> Vec<PathBuf> {
    let mut holding: Vec<PathBuf> = snapshots_in(dir)
        .into_iter()
        .filter(|p| {
            let Ok(c) = open_for_probe(p) else { return true };
            if !has_table(&c, "memory_conflict").unwrap_or(true) {
                return false;
            }
            found.iter().any(|(pair, reason)| {
                c.query_row(
                    "SELECT count(*) FROM memory_conflict WHERE new_id = ?1 AND old_id = ?2 AND reason = ?3",
                    rusqlite::params![pair.new_id, pair.old_id, reason],
                    |r| r.get::<_, i64>(0),
                )
                .map_or(true, |n| n > 0)
            })
        })
        .collect();
    holding.sort();
    holding
}

/// The files in `dir` named as snapshots.
fn snapshots_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).and_then(parse_snapshot_name).is_some())
        .collect()
}

/// The snapshots in `dir` with a row `id` whose content is the purged one, or
/// an outbox row holding it, by name. One that cannot be read is listed too:
/// it may hold the row.
fn backups_holding(dir: &Path, id: i64, held: &OutboxMatch) -> Vec<PathBuf> {
    backups_holding_any(dir, &[(id, held)])
}

/// [`backups_holding`] for several rows at once: each snapshot is opened one
/// time and searched for any of them.
fn backups_holding_any(dir: &Path, rows: &[(i64, &OutboxMatch)]) -> Vec<PathBuf> {
    let mut holding: Vec<PathBuf> = snapshots_in(dir)
        .into_iter()
        .filter(|p| {
            let Ok(c) = open_for_probe(p) else { return true };
            rows.iter().any(|(id, held)| {
                let in_memory = c
                    .query_row(
                        "SELECT count(*) FROM memory WHERE id = ?1 AND content = ?2",
                        rusqlite::params![id, held.content],
                        |r| r.get::<_, i64>(0),
                    )
                    .map_or(true, |n| n > 0);
                in_memory || outbox_holds(&c, held)
            })
        })
        .collect();
    holding.sort();
    holding
}

/// Whether a snapshot's outbox has a row `held` matches. A snapshot from
/// before the outbox has no such table, and so cannot hold one.
fn outbox_holds(c: &rusqlite::Connection, held: &OutboxMatch) -> bool {
    has_table(c, "ingest_outbox").unwrap_or(true)
        && c.query_row(&format!("SELECT count(*) FROM ingest_outbox WHERE {OUTBOX_MATCH}"), held.params(), |r| {
            r.get::<_, i64>(0)
        })
        .map_or(true, |n| n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryStore;

    const INSERT_ROW: &str = "INSERT INTO memory \
        (scope, agent_id, content, memory_type, source, \
         created_at, created_by_type, created_by_ref, \
         last_modified_at, last_accessed_at, importance, tier, status) \
        VALUES ('agent', 'agent-s', ?1, 'semantic_fact', 'explicit', \
         0, 'user', 'agent-s', 0, 0, 5.0, 3, 'active')";

    // Built at run time so no literal in the repository looks like a live
    // credential to a secret scanner.
    fn fake(prefix: &str, unit: &str, len: usize) -> String {
        format!("{prefix}{}", unit.chars().cycle().take(len).collect::<String>())
    }

    async fn insert(store: &MemoryStore, content: String) -> i64 {
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(INSERT_ROW, [content])?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    async fn soft_delete(store: &MemoryStore, id: i64) {
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET deleted_at = 1 WHERE id = ?1", [id]))
            .await
            .unwrap();
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|rd| rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    #[tokio::test]
    async fn the_scan_finds_live_and_deleted_rows_without_their_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let aws = insert(&store, format!("staging uses {}", fake("AKIA", "Q7ZX", 16))).await;
        insert(&store, "nothing secret here".to_string()).await;
        let jwt = insert(
            &store,
            format!("session {}", fake("eyJ", "hbGc", 20) + "." + &fake("eyJ", "zdWI", 20) + ".sig"),
        )
        .await;
        soft_delete(&store, jwt).await;
        store.conn.clone().close().await.unwrap();
        drop(store);
        let before = files_in(dir.path());

        assert_eq!(
            scan(dir.path()).unwrap().memories,
            vec![
                Flagged { id: aws, kinds: vec![SecretKind::AwsAccessKey], deleted: false },
                Flagged { id: jwt, kinds: vec![SecretKind::Jwt], deleted: true },
            ]
        );
        assert_eq!(files_in(dir.path()), before, "the scan wrote nothing beside the store");
    }

    #[tokio::test]
    async fn the_scan_flags_rows_saved_before_a_new_rule() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let password = insert(&store, "db password=hunter2".to_string()).await;
        let marked = insert(&store, "card <private>4111 1111</private>".to_string()).await;
        store.conn.clone().close().await.unwrap();
        drop(store);

        assert_eq!(
            scan(dir.path()).unwrap().memories,
            vec![
                Flagged { id: password, kinds: vec![SecretKind::Password], deleted: false },
                Flagged { id: marked, kinds: vec![SecretKind::Private], deleted: false },
            ]
        );
    }

    async fn add_pair(store: &MemoryStore, new: i64, old: i64, reason: Option<String>) {
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory_conflict (new_id, old_id, distance, state, verdict, reason, created_at) \
                     VALUES (?1, ?2, 0.3, 'judged', 'scoped', ?3, 0)",
                    rusqlite::params![new, old, reason],
                )
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn the_scan_finds_reasons_that_hold_a_secret() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let a = insert(&store, "the deploy runs on fridays".to_string()).await;
        let b = insert(&store, "the deploy runs on mondays".to_string()).await;
        let c = insert(&store, "the deploy runs on sundays".to_string()).await;
        add_pair(&store, b, a, Some(format!("rotated {}", fake("ghp_", "a1B2", 36)))).await;
        add_pair(&store, c, a, Some("one is the dev profile".to_string())).await;
        add_pair(&store, c, b, None).await;
        store.conn.clone().close().await.unwrap();
        drop(store);

        let found = scan(dir.path()).unwrap();

        assert!(found.memories.is_empty(), "{:?}", found.memories);
        assert_eq!(found.pairs, vec![FlaggedPair { new_id: b, old_id: a, kinds: vec![SecretKind::GithubToken] }]);
    }

    fn reasons(dir: &Path) -> Vec<Option<String>> {
        let c = rusqlite::Connection::open(dir.join("memory.db")).unwrap();
        let mut stmt = c.prepare("SELECT reason FROM memory_conflict ORDER BY new_id, old_id").unwrap();
        let reasons = stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        reasons
    }

    #[tokio::test]
    async fn masking_reasons_rewrites_only_the_ones_with_a_secret() {
        use crate::memory::backup::backup_offline;
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let a = insert(&store, "the deploy runs on fridays".to_string()).await;
        let b = insert(&store, "the deploy runs on mondays".to_string()).await;
        let c = insert(&store, "the deploy runs on sundays".to_string()).await;
        let token = fake("ghp_", "q8Zk", 36);
        add_pair(&store, b, a, Some(format!("rotated {token}"))).await;
        add_pair(&store, c, a, Some("one is the dev profile".to_string())).await;
        add_pair(&store, c, b, None).await;
        store.conn.clone().close().await.unwrap();
        drop(store);
        let held = backup_offline(dir.path()).unwrap();
        assert!(bytes_mention(dir.path(), b"q8Zkq8Zk"), "the fixture holds the token");

        let masked = mask_reasons(dir.path()).unwrap();

        assert_eq!(
            masked,
            MaskedReasons {
                pairs: vec![FlaggedPair { new_id: b, old_id: a, kinds: vec![SecretKind::GithubToken] }],
                backups: vec![held],
            }
        );
        assert_eq!(
            reasons(dir.path()),
            [Some("rotated [REDACTED:github_token]".to_string()), Some("one is the dev profile".to_string()), None]
        );
        assert!(!bytes_mention(dir.path(), b"q8Zkq8Zk"), "the token's bytes are still on disk");
        assert_eq!(mask_reasons(dir.path()).unwrap(), MaskedReasons::default(), "a second run finds nothing");
        StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("the lock was released");
    }

    #[tokio::test]
    async fn masking_reasons_refuses_while_the_daemon_holds_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let a = insert(&store, "the deploy runs on fridays".to_string()).await;
        let b = insert(&store, "the deploy runs on mondays".to_string()).await;
        add_pair(&store, b, a, Some(format!("rotated {}", fake("ghp_", "a1B2", 36)))).await;
        store.conn.clone().close().await.unwrap();
        drop(store);
        let before = std::fs::read(dir.path().join("memory.db")).unwrap();
        let _daemon = StoreLock::acquire(dir.path(), StoreHolder::Daemon).unwrap();

        let err = mask_reasons(dir.path()).unwrap_err();

        assert!(matches!(err, MemoryError::StoreInUse { .. }), "{err}");
        assert_eq!(std::fs::read(dir.path().join("memory.db")).unwrap(), before);
    }

    #[tokio::test]
    async fn the_scan_of_a_store_from_before_the_pairs_lists_rows_only() {
        let dir = tempfile::tempdir().unwrap();
        crate::memory::db::seed_on_disk(dir.path(), Some(14)).await;
        rusqlite::Connection::open(dir.path().join("memory.db"))
            .unwrap()
            .execute(INSERT_ROW, [format!("ci token {}", fake("ghp_", "a1B2", 36))])
            .unwrap();

        let found = scan(dir.path()).unwrap();

        assert_eq!(found.memories.len(), 1);
        assert!(found.pairs.is_empty());
    }

    #[tokio::test]
    async fn the_scan_reads_while_another_process_holds_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let id = insert(&store, format!("ci token {}", fake("ghp_", "a1B2", 36))).await;
        assert!(matches!(
            StoreLock::acquire(dir.path(), StoreHolder::Maintenance),
            Err(MemoryError::StoreInUse { .. })
        ));

        let found = scan(dir.path()).unwrap();

        assert_eq!(found.memories, vec![Flagged { id, kinds: vec![SecretKind::GithubToken], deleted: false }]);
        drop(store);
    }

    const SECRET: &str = "the vault passphrase is zebra-quokka-7741";

    /// A closed WAL-mode store with 300 ordinary rows and the secret one,
    /// which has a vector. Returns the secret row's id.
    async fn store_with_a_secret(dir: &Path, deleted: bool) -> i64 {
        let store = MemoryStore::open(dir, StoreHolder::Eval).await.unwrap();
        for i in 0..300 {
            let row = insert(&store, format!("ordinary row {i} about trains and buses")).await;
            add_vector(&store, row, i).await;
        }
        let id = insert(&store, SECRET.to_string()).await;
        add_vector(&store, id, 300).await;
        if deleted {
            soft_delete(&store, id).await;
        }
        store.conn.clone().close().await.unwrap();
        drop(store);
        id
    }

    /// A unit vector along axis `axis`, so a KNN probe along the same axis
    /// finds exactly this row.
    fn axis(axis: usize) -> Vec<u8> {
        let mut v = vec![0f32; 384];
        v[axis] = 1.0;
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    async fn add_vector(store: &MemoryStore, id: i64, n: usize) {
        let blob = axis(n % 384);
        store
            .conn
            .call(move |c| {
                c.execute("INSERT INTO memory_vectors (rowid, embedding) VALUES (?1, ?2)", rusqlite::params![id, blob])
            })
            .await
            .unwrap();
    }

    fn nearest(dir: &Path, n: usize) -> i64 {
        crate::memory::db::ensure_vec0_registered();
        let c = rusqlite::Connection::open(dir.join("memory.db")).unwrap();
        c.query_row("SELECT rowid FROM memory_vectors WHERE embedding MATCH ?1 AND k = 1", [axis(n)], |r| r.get(0))
            .unwrap()
    }

    fn bytes_mention(dir: &Path, needle: &[u8]) -> bool {
        ["memory.db", "memory.db-wal"].iter().any(|name| {
            std::fs::read(dir.join(name))
                .map(|bytes| bytes.windows(needle.len()).any(|w| w == needle))
                .unwrap_or(false)
        })
    }

    fn count(dir: &Path, sql: &str, id: i64) -> i64 {
        crate::memory::db::ensure_vec0_registered();
        let c = rusqlite::Connection::open(dir.join("memory.db")).unwrap();
        c.query_row(sql, [id], |r| r.get(0)).unwrap()
    }

    fn assert_no_trace(dir: &Path, id: i64) {
        assert_eq!(count(dir, "SELECT count(*) FROM memory WHERE id = ?1", id), 0);
        assert_eq!(count(dir, "SELECT count(*) FROM memory_vectors WHERE rowid = ?1", id), 0);
        assert_eq!(
            count(dir, "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'quokka' AND rowid >= ?1", 0),
            0
        );
        assert!(!bytes_mention(dir, b"quokka"), "the secret's bytes are still on disk");
    }

    #[tokio::test]
    async fn purging_a_live_row_leaves_no_trace_of_its_text() {
        let dir = tempfile::tempdir().unwrap();
        let id = store_with_a_secret(dir.path(), false).await;
        assert!(bytes_mention(dir.path(), b"quokka"), "the fixture holds the secret");

        purge(dir.path(), id).unwrap();

        assert_no_trace(dir.path(), id);
        assert_eq!(count(dir.path(), "SELECT count(*) FROM memory WHERE id <> ?1", id), 300);
        assert_eq!(nearest(dir.path(), 150), id - 150, "the other rows keep their vectors");
        StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("the lock was released");
    }

    #[tokio::test]
    async fn purging_a_deleted_row_leaves_no_trace_of_its_text() {
        let dir = tempfile::tempdir().unwrap();
        let id = store_with_a_secret(dir.path(), true).await;
        assert!(bytes_mention(dir.path(), b"quokka"), "a soft delete keeps the text");

        purge(dir.path(), id).unwrap();

        assert_no_trace(dir.path(), id);
    }

    #[tokio::test]
    async fn purging_releases_the_rows_that_disputed_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let disputed = insert(&store, "the deploy runs on fridays".to_string()).await;
        let challenger = insert(&store, "the deploy runs on mondays".to_string()).await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE memory SET status = 'disputed', disputed_against = ?2 WHERE id = ?1",
                    rusqlite::params![challenger, format!("[{disputed}]")],
                )
            })
            .await
            .unwrap();
        store.conn.clone().close().await.unwrap();
        drop(store);

        purge(dir.path(), disputed).unwrap();

        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        let status: String =
            c.query_row("SELECT status FROM memory WHERE id = ?1", [challenger], |r| r.get(0)).unwrap();
        assert_eq!(status, "active");
    }

    #[tokio::test]
    async fn purging_drops_the_pairs_of_the_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let old = insert(&store, "the deploy runs on fridays".to_string()).await;
        let purged = insert(&store, "the deploy runs on mondays".to_string()).await;
        let newer = insert(&store, "the deploy runs on sundays".to_string()).await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory_conflict (new_id, old_id, distance, state, created_at) \
                     VALUES (?2, ?1, 0.3, 'pending', 0), (?3, ?2, 0.3, 'judged', 0), (?3, ?1, 0.3, 'pending', 0)",
                    [old, purged, newer],
                )
            })
            .await
            .unwrap();
        store.conn.clone().close().await.unwrap();
        drop(store);

        purge(dir.path(), purged).unwrap();

        let sql = "SELECT count(*) FROM memory_conflict WHERE new_id = ?1 OR old_id = ?1";
        assert_eq!(count(dir.path(), sql, purged), 0);
        assert_eq!(count(dir.path(), sql, old), 1, "the pair of two other rows stays");
    }

    #[tokio::test]
    async fn purging_keeps_the_links_that_name_the_row_as_a_source() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let source = insert(&store, "the go service uses tabs".to_string()).await;
        let purged = insert(&store, "the user indents with tabs".to_string()).await;
        let other = insert(&store, "the rust crate uses tabs".to_string()).await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory_derivation (memory_id, source_id, created_at) \
                     VALUES (?1, ?2, 0), (?3, ?1, 0)",
                    [purged, source, other],
                )
            })
            .await
            .unwrap();
        store.conn.clone().close().await.unwrap();
        drop(store);

        purge(dir.path(), purged).unwrap();

        let drawn = "SELECT count(*) FROM memory_derivation WHERE memory_id = ?1";
        assert_eq!(count(dir.path(), drawn, purged), 0, "a purged conclusion rests on nothing");
        let named = "SELECT count(*) FROM memory_derivation WHERE source_id = ?1";
        assert_eq!(count(dir.path(), named, purged), 1, "the conclusion drawn from it keeps the trace of its loss");
    }

    #[tokio::test]
    async fn purge_refuses_while_the_daemon_holds_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let id = store_with_a_secret(dir.path(), false).await;
        let before = std::fs::read(dir.path().join("memory.db")).unwrap();
        let _daemon = StoreLock::acquire(dir.path(), StoreHolder::Daemon).unwrap();

        let err = purge(dir.path(), id).unwrap_err();

        match err {
            MemoryError::StoreInUse { owner: Some(o) } => assert_eq!(o.holder, StoreHolder::Daemon),
            other => panic!("expected StoreInUse, got {other}"),
        }
        assert_eq!(std::fs::read(dir.path().join("memory.db")).unwrap(), before);
    }

    #[tokio::test]
    async fn purge_refuses_an_unknown_id_untouched() {
        let dir = tempfile::tempdir().unwrap();
        store_with_a_secret(dir.path(), false).await;
        let before = std::fs::read(dir.path().join("memory.db")).unwrap();

        let err = purge(dir.path(), 999_999).unwrap_err();

        assert!(matches!(err, MemoryError::Purge(PurgeError::NotFound(999_999))), "{err}");
        assert_eq!(err.to_string(), "there is no memory #999999");
        assert_eq!(std::fs::read(dir.path().join("memory.db")).unwrap(), before);
        StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("the lock was released");
    }

    #[tokio::test]
    async fn purge_refuses_a_store_with_pending_migrations_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let older = crate::memory::backup::latest_known() - 1;
        crate::memory::db::seed_on_disk(dir.path(), Some(older)).await;
        let before = std::fs::read(dir.path().join("memory.db")).unwrap();

        let err = purge(dir.path(), 1).unwrap_err();

        assert!(matches!(err, MemoryError::Purge(PurgeError::Pending { .. })), "{err}");
        assert!(err.to_string().contains("start the daemon once"), "{err}");
        assert_eq!(std::fs::read(dir.path().join("memory.db")).unwrap(), before);
    }

    #[tokio::test]
    async fn purge_names_the_backups_that_still_hold_the_row() {
        use crate::memory::backup::backup_offline;
        let pause = || std::thread::sleep(std::time::Duration::from_millis(5));
        let dir = tempfile::tempdir().unwrap();
        crate::memory::db::seed_on_disk(dir.path(), None).await;
        let without = backup_offline(dir.path()).unwrap();
        pause();
        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        c.execute(INSERT_ROW, [SECRET]).unwrap();
        let id = c.last_insert_rowid();
        drop(c);
        let with = backup_offline(dir.path()).unwrap();
        pause();
        rusqlite::Connection::open(dir.path().join("memory.db"))
            .unwrap()
            .execute("UPDATE memory SET deleted_at = 1 WHERE id = ?1", [id])
            .unwrap();
        let after_delete = backup_offline(dir.path()).unwrap();
        let kept: Vec<Vec<u8>> =
            [&without, &with, &after_delete].iter().map(|p| std::fs::read(p).unwrap()).collect();

        let purged = purge(dir.path(), id).unwrap();

        assert_eq!(purged.backups, vec![with.clone(), after_delete.clone()]);
        let now: Vec<Vec<u8>> =
            [&without, &with, &after_delete].iter().map(|p| std::fs::read(p).unwrap()).collect();
        assert_eq!(now, kept, "no backup was changed");
    }

    fn add_outbox_row(c: &rusqlite::Connection, operation_id: &str, content: &str) {
        add_outbox_row_of(c, "agent-s", operation_id, content);
    }

    fn add_outbox_row_of(c: &rusqlite::Connection, agent: &str, operation_id: &str, content: &str) {
        c.execute(
            "INSERT INTO ingest_outbox (operation_id, agent_id, content, event, state, created_at, updated_at) \
             VALUES (?1, ?2, ?3, '{}', 'failed', 0, 0)",
            [operation_id, agent, content],
        )
        .unwrap();
    }

    fn outbox_ids(dir: &Path) -> Vec<String> {
        let c = rusqlite::Connection::open(dir.join("memory.db")).unwrap();
        let mut stmt = c.prepare("SELECT operation_id FROM ingest_outbox ORDER BY id").unwrap();
        let ids = stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        ids
    }

    #[tokio::test]
    async fn purging_the_current_revision_brings_the_previous_one_back() {
        let dir = tempfile::tempdir().unwrap();
        crate::memory::db::seed_on_disk(dir.path(), None).await;
        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        let mut ids = Vec::new();
        for text in ["deploy to fly", "deploy to render", "a rule a verdict replaced", SECRET] {
            c.execute(INSERT_ROW, [text]).unwrap();
            ids.push(c.last_insert_rowid());
        }
        let [first, middle, judged, current] = ids[..] else { unreachable!() };
        let supersede = "UPDATE memory SET status = 'superseded', superseded_by = ?2, supersede_cause = ?3, \
                         topic_key = ?4 WHERE id = ?1";
        let key = Some("deploy/target");
        c.execute(supersede, rusqlite::params![first, middle, "revision", key]).unwrap();
        c.execute(supersede, rusqlite::params![middle, current, "revision", key]).unwrap();
        c.execute(supersede, rusqlite::params![judged, current, "verdict", None::<&str>]).unwrap();
        c.execute("UPDATE memory SET topic_key = 'deploy/target' WHERE id = ?1", [current]).unwrap();
        drop(c);

        let purged = purge(dir.path(), current).unwrap();

        assert_eq!((purged.revived.as_slice(), purged.hidden.as_slice()), ([middle].as_slice(), [judged].as_slice()));
        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        let state = |id: i64| -> (String, Option<i64>) {
            c.query_row("SELECT status, superseded_by FROM memory WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
        };
        assert_eq!(state(middle), ("active".to_string(), None));
        assert_eq!(state(first), ("superseded".to_string(), Some(middle)));
        assert_eq!(state(judged), ("superseded".to_string(), Some(current)), "a verdict stays");
        drop(c);
        assert_no_trace(dir.path(), current);
    }

    #[tokio::test]
    async fn purge_names_the_workspaces_that_still_hold_the_text() {
        use crate::memory::workspace::{AUTO_END, AUTO_START, USER_END, USER_START};
        let dir = tempfile::tempdir().unwrap();
        let id = store_with_a_secret(dir.path(), false).await;
        let root = dir.path().join("workspaces");
        let write = |agent: &str, project: &str, auto: &str, user: &str| {
            let path = root.join(agent).join(project).join("workspace.md");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let text = format!("{AUTO_START}\n{auto}\n{AUTO_END}\n{USER_START}\n{user}\n{USER_END}\n");
            std::fs::write(&path, &text).unwrap();
            (path, text)
        };
        let auto = write("agent-s", "repo-1a2b", &format!("- {SECRET}"), "");
        let user = write("agent-o", "default", "", &format!("copied: {SECRET}"));
        let other = write("agent-s", "default", "- nothing to see", "");

        let purged = purge(dir.path(), id).unwrap();

        assert_eq!(
            purged.workspaces,
            vec![
                WorkspaceHolding { path: user.0.clone(), holding: Holding { auto: false, user: true } },
                WorkspaceHolding { path: auto.0.clone(), holding: Holding { auto: true, user: false } },
            ]
        );
        for (path, text) in [auto, user, other] {
            assert_eq!(std::fs::read_to_string(path).unwrap(), text, "no workspace was changed");
        }
    }

    #[tokio::test]
    async fn purging_the_first_piece_empties_the_head_of_its_save() {
        let dir = tempfile::tempdir().unwrap();
        crate::memory::db::seed_on_disk(dir.path(), None).await;
        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        let mut ids = Vec::new();
        for text in [SECRET, "and the second piece of it", "and the third piece of it"] {
            c.execute(INSERT_ROW, [text]).unwrap();
            ids.push(c.last_insert_rowid());
        }
        for (piece, (id, context)) in ids.iter().zip(["", SECRET, SECRET]).enumerate() {
            c.execute(
                "INSERT INTO memory_piece (save_id, piece, memory_id, context) VALUES (1, ?1, ?2, ?3)",
                rusqlite::params![piece as i64 + 1, id, context],
            )
            .unwrap();
        }
        let whole = format!("{SECRET} and the second piece of it and the third piece of it");
        add_outbox_row(&c, "op-long", &whole);
        drop(c);

        let purged = purge(dir.path(), ids[0]).unwrap();

        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        let links: Vec<(i64, String)> = c
            .prepare("SELECT memory_id, context FROM memory_piece ORDER BY piece")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(c);
        assert_eq!(links, ids.iter().map(|&id| (id, String::new())).collect::<Vec<_>>());
        assert!(outbox_ids(dir.path()).is_empty());
        assert_eq!(purged.pieces, ids[1..]);
        assert!(!bytes_mention(dir.path(), b"quokka"), "the secret's bytes are still on disk");
    }

    #[tokio::test]
    async fn purge_keeps_other_agents_and_longer_saves_in_the_outbox() {
        let dir = tempfile::tempdir().unwrap();
        crate::memory::db::seed_on_disk(dir.path(), None).await;
        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        c.execute(INSERT_ROW, ["use pnpm"]).unwrap();
        let id = c.last_insert_rowid();
        add_outbox_row_of(&c, "agent-o", "op-other", "use pnpm");
        add_outbox_row_of(&c, "agent-s", "op-same", "use pnpm");
        add_outbox_row_of(&c, "agent-s", "op-longer", "use pnpm in the monorepo");
        drop(c);

        let purged = purge(dir.path(), id).unwrap();

        assert_eq!(outbox_ids(dir.path()), ["op-other", "op-longer"]);
        assert_eq!(
            purged.outbox,
            vec![OutboxRemoved { operation_id: "op-same".into(), agent_id: "agent-s".into(), state: "failed".into() }]
        );
    }

    #[tokio::test]
    async fn purge_removes_the_text_from_the_outbox() {
        use crate::memory::backup::backup_offline;
        let dir = tempfile::tempdir().unwrap();
        crate::memory::db::seed_on_disk(dir.path(), None).await;
        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        add_outbox_row(&c, "op-1", SECRET);
        add_outbox_row(&c, "op-2", "nothing to do with it");
        drop(c);
        let only_in_the_outbox = backup_offline(dir.path()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        c.execute(INSERT_ROW, [SECRET]).unwrap();
        let id = c.last_insert_rowid();
        drop(c);

        let purged = purge(dir.path(), id).unwrap();

        assert_eq!(outbox_ids(dir.path()), ["op-2"]);
        assert!(!bytes_mention(dir.path(), b"quokka"), "the secret's bytes are still on disk");
        assert_eq!(purged.backups, vec![only_in_the_outbox]);
    }

    #[tokio::test]
    async fn a_backup_from_before_the_outbox_is_not_named_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let backups = crate::paths::backups_dir(dir.path());
        std::fs::create_dir_all(&backups).unwrap();
        let old = backups.join(crate::memory::backup::snapshot_name(0, crate::memory::backup::Reason::Manual));
        crate::memory::db::seed_on_disk(dir.path(), Some(crate::memory::backup::latest_known() - 1)).await;
        std::fs::copy(dir.path().join("memory.db"), &old).unwrap();

        let held = OutboxMatch { agent: Some("agent-s"), content: SECRET, piece: false };
        assert!(backups_holding(&backups, 1, &held).is_empty());
    }

    #[test]
    fn the_scan_of_a_missing_store_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let err = scan(dir.path()).unwrap_err();
        assert!(matches!(err, MemoryError::Backup(BackupError::NoStore { .. })), "{err}");
        assert!(files_in(dir.path()).is_empty(), "{:?}", files_in(dir.path()));
    }

    // ── purging a project delete ────────────────────────────────────────

    use crate::memory::project_delete::{delete_core, Counts};
    use crate::memory::workspace::WorkspaceManager;

    const GONE: &str = "gone-aaaa";
    const WEB: &str = "web-bbbb";

    /// A project with a secret row, another live row, a row deleted before,
    /// a trajectory, a docs index and a pending save, deleted with its
    /// memories; and another project that links to it. The store is closed.
    struct DeletedProject {
        dir: tempfile::TempDir,
        delete_id: i64,
        secret: i64,
        other: i64,
        old: i64,
        web: i64,
    }

    async fn insert_in(store: &MemoryStore, content: &str, project: &str) -> i64 {
        let (content, project) = (content.to_string(), project.to_string());
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(INSERT_ROW, [content])?;
                let id = c.last_insert_rowid();
                c.execute("UPDATE memory SET project_id = ?2 WHERE id = ?1", rusqlite::params![id, project])?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    async fn deleted_project() -> DeletedProject {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        store
            .conn
            .call(|c| {
                c.execute_batch(&format!(
                    "INSERT INTO project (id, name, born, reviewed, created_at) VALUES \
                       ('{GONE}', 'gone', 'root', 1, 1), ('{WEB}', 'web', 'root', 1, 1);
                     INSERT INTO project_path (id, root_path, created_at, index_docs, project_id) \
                       VALUES ('{GONE}', '/work/gone', 1, 1, '{GONE}');
                     INSERT INTO project_doc_file (project_id, path, content_hash, model_id, chunks, indexed_at) \
                       VALUES ('{GONE}', 'README.md', 'h', 'm', 1, 1);"
                ))
            })
            .await
            .unwrap();
        let secret = insert_in(&store, SECRET, GONE).await;
        let other = insert_in(&store, "another note of the gone project", GONE).await;
        let old = insert_in(&store, "an older note of the gone project", GONE).await;
        let web = insert_in(&store, "a note of the web project", WEB).await;
        soft_delete(&store, old).await;
        for (n, id) in [secret, other, web].into_iter().enumerate() {
            add_vector(&store, id, n).await;
        }
        store
            .conn
            .call(move |c| {
                c.execute_batch(&format!(
                    "INSERT INTO episodic_trajectory (id, scope, agent_id, project_id, session_id, task, created_at, last_modified_at, last_accessed_at) \
                       VALUES (50, 'agent', 'agent-s', '{GONE}', 's1', 'a trajectory of the gone project', 1, 1, 1);
                     INSERT INTO episodic_vectors (rowid, embedding) VALUES (50, zeroblob(1536));
                     INSERT INTO memory_conflict (new_id, old_id, distance, state, created_at) VALUES ({web}, {secret}, 0.1, 'pending', 1);
                     INSERT INTO memory_derivation (memory_id, source_id, created_at) VALUES ({web}, {secret}, 1), ({secret}, {other}, 1);
                     INSERT INTO ingest_outbox (operation_id, agent_id, content, event, state, created_at, updated_at) \
                       VALUES ('op-secret', 'agent-s', '{SECRET}', '{{}}', 'failed', 0, 0);"
                ))
            })
            .await
            .unwrap();
        let workspaces = WorkspaceManager::new(dir.path().join("workspaces"));
        let report = delete_core(&store, &workspaces, &crate::paths::backups_dir(dir.path()), GONE, true, None).await.unwrap();
        let done =
            delete_core(&store, &workspaces, &crate::paths::backups_dir(dir.path()), GONE, true, report.code.as_deref()).await.unwrap();
        let delete_id = done.delete_id.expect("deleted");
        store.conn.clone().close().await.unwrap();
        drop(store);
        DeletedProject { dir, delete_id, secret, other, old, web }
    }

    fn sql(dir: &Path, sql: &str) {
        crate::memory::db::ensure_vec0_registered();
        rusqlite::Connection::open(dir.join("memory.db")).unwrap().execute_batch(sql).unwrap();
    }

    fn total(dir: &Path, sql: &str) -> i64 {
        crate::memory::db::ensure_vec0_registered();
        let c = rusqlite::Connection::open(dir.join("memory.db")).unwrap();
        c.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[tokio::test]
    async fn purging_a_delete_leaves_no_trace_of_its_rows() {
        let d = deleted_project().await;
        // A refresh in flight wrote to the docs index after the delete.
        sql(
            d.dir.path(),
            &format!(
                "INSERT INTO project_doc_file (project_id, path, content_hash, model_id, chunks, indexed_at) VALUES ('{GONE}', 'a.md', 'h', 'm', 1, 1);
                 INSERT INTO project_doc_chunk (id, project_id, path, ordinal, breadcrumb, body) VALUES (900, '{GONE}', 'a.md', 0, 'a.md', 'quokka in the docs');
                 INSERT INTO project_doc_fts (rowid, text, project_id) VALUES (900, 'quokka in the docs', '{GONE}');"
            ),
        );
        assert!(bytes_mention(d.dir.path(), b"quokka"), "the fixture holds the secret");

        let purged = purge_delete(d.dir.path(), d.delete_id).unwrap();

        assert_eq!(purged.project_id, GONE);
        assert_eq!(purged.removed, Counts { memories: 3, trajectories: 1, skills: 0 });
        assert!(purged.live.is_empty());
        assert_eq!(
            purged.outbox,
            vec![OutboxRemoved { operation_id: "op-secret".into(), agent_id: "agent-s".into(), state: "failed".into() }]
        );
        let dir = d.dir.path();
        for table in ["memory", "episodic_trajectory"] {
            assert_eq!(total(dir, &format!("SELECT count(*) FROM {table} WHERE project_id = '{GONE}'")), 0, "{table}");
        }
        assert_eq!(total(dir, "SELECT count(*) FROM memory_vectors WHERE rowid IN (1, 2, 3)"), 0, "the vectors went");
        assert_eq!(total(dir, "SELECT count(*) FROM episodic_vectors WHERE rowid = 50"), 0);
        assert_eq!(total(dir, "SELECT count(*) FROM memory_conflict"), 0, "its pairs went");
        assert_eq!(
            total(dir, &format!("SELECT count(*) FROM memory_derivation WHERE memory_id = {}", d.secret)),
            0,
            "a purged conclusion rests on nothing"
        );
        assert_eq!(
            total(dir, &format!("SELECT count(*) FROM memory_derivation WHERE source_id = {}", d.secret)),
            1,
            "a conclusion elsewhere keeps the link that tells it lost a source"
        );
        assert_eq!(total(dir, "SELECT count(*) FROM ingest_outbox"), 0);
        assert_eq!(total(dir, "SELECT count(*) FROM project_doc_file WHERE project_id = 'gone-aaaa'"), 0, "the docs index went again");
        assert_eq!(total(dir, "SELECT count(*) FROM project_doc_chunk"), 0);
        assert_eq!(total(dir, "SELECT count(*) FROM memory WHERE project_id = 'web-bbbb'"), 1, "other projects keep their rows");
        assert_eq!(total(dir, &format!("SELECT count(*) FROM memory_vectors WHERE rowid = {}", d.web)), 1);
        assert!(!bytes_mention(dir, b"quokka"), "the secret's bytes are still on disk");
        assert_eq!(total(dir, "SELECT count(*) FROM project_delete WHERE purged_at IS NOT NULL"), 1);
        assert_eq!(total(dir, "SELECT count(*) FROM project WHERE id = 'gone-aaaa' AND deleted_at IS NOT NULL"), 1, "the project stays as it is");
        StoreLock::acquire(dir, StoreHolder::Daemon).expect("the lock was released");
    }

    #[tokio::test]
    async fn purging_a_delete_keeps_a_row_restored_since() {
        let d = deleted_project().await;
        sql(d.dir.path(), &format!("UPDATE memory SET deleted_at = NULL WHERE id = {}", d.other));

        let purged = purge_delete(d.dir.path(), d.delete_id).unwrap();

        assert_eq!(purged.live, vec![LiveRow { table: "memory".into(), id: d.other }]);
        assert_eq!(purged.removed, Counts { memories: 2, trajectories: 1, skills: 0 });
        let dir = d.dir.path();
        assert_eq!(total(dir, &format!("SELECT count(*) FROM memory WHERE id = {}", d.other)), 1);
        assert_eq!(total(dir, &format!("SELECT count(*) FROM memory WHERE id IN ({}, {})", d.secret, d.old)), 0);
        assert_eq!(total(dir, "SELECT count(*) FROM project_delete WHERE purged_at IS NOT NULL"), 1);
    }

    #[tokio::test]
    async fn a_purge_of_a_delete_undone_or_purged_is_refused_untouched() {
        let d = deleted_project().await;
        let dir = d.dir.path();
        let before = std::fs::read(dir.join("memory.db")).unwrap();

        let unknown = purge_delete(dir, 99).unwrap_err();
        assert!(matches!(unknown, MemoryError::Purge(PurgeError::UnknownDelete(99))), "{unknown}");
        assert_eq!(unknown.to_string(), "there is no delete #99");
        assert_eq!(std::fs::read(dir.join("memory.db")).unwrap(), before);

        {
            let _daemon = StoreLock::acquire(dir, StoreHolder::Daemon).unwrap();
            let held = purge_delete(dir, d.delete_id).unwrap_err();
            assert!(matches!(held, MemoryError::StoreInUse { .. }), "{held}");
        }

        sql(dir, &format!("UPDATE project_delete SET undone_at = 5 WHERE delete_id = {}", d.delete_id));
        let before = std::fs::read(dir.join("memory.db")).unwrap();
        let undone = purge_delete(dir, d.delete_id).unwrap_err();
        assert!(matches!(&undone, MemoryError::Purge(PurgeError::DeleteUndone(n)) if *n == d.delete_id), "{undone}");
        assert_eq!(std::fs::read(dir.join("memory.db")).unwrap(), before, "nothing was written");
        assert_eq!(total(dir, "SELECT count(*) FROM memory WHERE project_id = 'gone-aaaa'"), 3);

        sql(dir, &format!("UPDATE project_delete SET undone_at = NULL WHERE delete_id = {}", d.delete_id));
        purge_delete(dir, d.delete_id).unwrap();
        let twice = purge_delete(dir, d.delete_id).unwrap_err();
        assert!(matches!(&twice, MemoryError::Purge(PurgeError::DeletePurged(n)) if *n == d.delete_id), "{twice}");
        StoreLock::acquire(dir, StoreHolder::Daemon).expect("the lock was released");
    }

    #[tokio::test]
    async fn purging_a_delete_names_the_backups_and_workspaces_that_hold_its_text() {
        use crate::memory::backup::backup_offline;
        use crate::memory::workspace::{AUTO_END, AUTO_START, USER_END, USER_START};
        let d = deleted_project().await;
        let dir = d.dir.path();
        let write = |agent: &str, project: &str, auto: &str, user: &str| {
            let path = dir.join("workspaces").join(agent).join(project).join("workspace.md");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let text = format!("{AUTO_START}\n{auto}\n{AUTO_END}\n{USER_START}\n{user}\n{USER_END}\n");
            std::fs::write(&path, &text).unwrap();
            (path, text)
        };
        let own = write("agent-s", GONE, "- nothing of it is written here", "");
        let copy = write("agent-o", "default", "", &format!("copied: {SECRET}"));
        let other = write("agent-s", WEB, "- a note of the web project", "");
        let backups = crate::paths::backups_dir(dir);
        let pre_delete = std::fs::read_dir(&backups)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().contains("pre-delete"))
            .expect("the snapshot the delete took");
        std::thread::sleep(std::time::Duration::from_millis(5));
        let manual = backup_offline(dir).unwrap();
        let kept: Vec<Vec<u8>> = [&pre_delete, &manual].iter().map(|p| std::fs::read(p).unwrap()).collect();

        let purged = purge_delete(dir, d.delete_id).unwrap();

        let mut named = purged.backups.clone();
        named.sort();
        let mut want = vec![pre_delete.clone(), manual.clone()];
        want.sort();
        assert_eq!(named, want, "both snapshots hold the rows");
        assert_eq!(purged.project_workspaces, vec![own.0.clone()], "the project's own file is named whatever it holds");
        assert_eq!(
            purged.workspaces,
            vec![WorkspaceHolding { path: copy.0.clone(), holding: Holding { auto: false, user: true } }],
            "and the others that hold a text of it"
        );
        let now: Vec<Vec<u8>> = [&pre_delete, &manual].iter().map(|p| std::fs::read(p).unwrap()).collect();
        assert_eq!(now, kept, "no backup was changed");
        for (path, text) in [own, copy, other] {
            assert_eq!(std::fs::read_to_string(path).unwrap(), text, "no workspace was changed");
        }
    }
}
