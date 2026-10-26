//! Deleting a project. The project stays in `project` with `deleted_at`: its
//! deleted rows stay under its id, `undelete` restores into it, and undoing
//! the delete needs its name and its origin. A live row written under a
//! deleted project's id brings it back unreviewed, by the triggers of V29.
//!
//! A project without live rows is deleted at once. One with them is only
//! reported, with the code of the set the report read; deleting it with its
//! rows takes that code back, and a code that is no longer the set's gets the
//! new report instead of a delete.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::memory::backup::Reason;
use crate::memory::clock::TimestampMs;
use crate::memory::admin::{RESTORE_TEXT_INDEX, RESTORE_TOPIC_HOLDER, RESTORE_TWIN};
use crate::memory::chunker::embedding_text;
use crate::memory::dispute::{release_challengers, settle_challengers, settle_row};
use crate::memory::embedder::{EmbedError, EmbedRole, Embedder};
use crate::memory::inspect::{answered_pairs, duplicates, folder_prints, RegistryRow};
use crate::memory::merge::{canonical_in, retrying, Retried};
use crate::memory::maintenance::skill_embedding_text;
use crate::memory::project_docs::drop_root_index_in;
use crate::memory::redact::redact;
use crate::memory::vector::to_unit;
use crate::memory::workspace::WorkspaceManager;
use crate::memory::{MemoryError, MemoryStore};

/// Why a delete was refused. Nothing is written.
#[derive(Debug, thiserror::Error)]
pub enum DeleteError {
    #[error("no project {0}")]
    UnknownProject(String),
    #[error("project {id} was merged into {into}")]
    Merged { id: String, into: String },
    /// `delete_id` is the open delete that took it, when the journal has one.
    #[error("project {id} is already deleted")]
    Deleted { id: String, delete_id: Option<i64> },
    #[error("{0} saves of this project are still being stored; try again in a moment")]
    SavesPending(i64),
    #[error("no delete {0}")]
    UnknownDelete(i64),
    #[error("delete {0} was already undone")]
    Undone(i64),
    #[error("delete {0} was purged")]
    Purged(i64),
    #[error("project {project_id} was deleted again since, in delete {later}: undo that one first")]
    DeletedLater { project_id: String, later: i64 },
    #[error("the rows to restore need the embedding model, which is not ready")]
    NeedsModel,
    #[error(transparent)]
    Embed(#[from] EmbedError),
    #[error(transparent)]
    Store(#[from] MemoryError),
}

impl From<rusqlite::Error> for DeleteError {
    fn from(e: rusqlite::Error) -> Self {
        DeleteError::Store(MemoryError::Sqlite(e))
    }
}

impl Retried for DeleteError {
    fn saves_pending(&self) -> bool {
        matches!(self, DeleteError::SavesPending(_))
    }

    fn connection(e: tokio_rusqlite::Error<DeleteError>) -> DeleteError {
        match e {
            tokio_rusqlite::Error::Error(e) => e,
            other => DeleteError::Store(MemoryError::Backend(other.to_string())),
        }
    }
}

/// Why a report is not a delete that went through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// The project has live rows and was asked to be deleted without them.
    NotEmpty,
    /// The code sent is not the one of the set as it is now.
    CodeChanged,
}

/// Why a row an undo would restore stays deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepReason {
    /// A live row of the project saved since has its text.
    LiveTwin,
    /// A live row of the project, of another save, has its key.
    TopicTaken,
    /// Its text holds a secret by today's rules.
    HoldsSecret,
    /// It is not as the delete left it: restored, deleted again or moved.
    Changed,
    /// It is a piece of a save with a piece that stays deleted.
    Tied,
}

/// A row an undo leaves deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kept {
    /// `memory`, `episodic_trajectory` or `procedural_skill`.
    pub table: String,
    pub id: i64,
    pub reason: KeepReason,
}

/// Rows by table.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub memories: usize,
    pub trajectories: usize,
    pub skills: usize,
}

/// Live rows of `memory` one agent saved; `agent_id` is `None` for the rows
/// no agent saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorRows {
    pub agent_id: Option<String>,
    pub rows: usize,
}

/// A project that may be the one to empty a project into.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub project_id: String,
    pub name: String,
    /// `same_name`, `same_first_commit`, or the reason of a pending suggestion.
    pub reason: String,
}

/// A save that gave up, which retrying would bring back into the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedSave {
    pub operation_id: String,
    pub agent_id: String,
}

/// What a project holds and what deleting it does, or did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteReport {
    pub project_id: String,
    pub name: String,
    pub with_memories: bool,
    pub refused: Option<Refusal>,
    /// The code of the set the report read; only for a delete with its rows.
    pub code: Option<String>,
    /// The delete's number in `project_delete`; `None` when nothing was written.
    pub delete_id: Option<i64>,
    pub snapshot: Option<String>,
    pub live: Counts,
    pub deleted: Counts,
    /// The live rows of `memory`, by type, author and status.
    pub by_type: BTreeMap<String, usize>,
    pub by_author: Vec<AuthorRows>,
    pub by_status: BTreeMap<String, usize>,
    /// Its live `user_preference` and `architectural_decision` rows.
    pub protected: Vec<i64>,
    /// How many pairs of `memory_conflict` join a row of the project with one
    /// of another project or of none.
    pub split_pairs: usize,
    /// The live conclusions of another project, or of none, drawn from a live
    /// row of this one.
    pub conclusions: Vec<i64>,
    /// Its `workspace.md` files, which a delete leaves alone.
    pub workspaces: Vec<String>,
    /// The folders whose docs index goes, by root id.
    pub folders: Vec<String>,
    /// Its remotes, which stay linked.
    pub remotes: Vec<String>,
    /// How many saves of the project, or of a project merged into it, wait.
    pub pending: usize,
    pub failed: Vec<FailedSave>,
    pub candidates: Vec<Candidate>,
    /// For an undo: nothing was written, this is what it would do.
    #[serde(default)]
    pub dry_run: bool,
    /// For an undo: the rows that came back, by table.
    #[serde(default)]
    pub restored: Counts,
    /// For an undo: the rows that stay deleted, with why.
    #[serde(default)]
    pub kept: Vec<Kept>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SetRow {
    id: i64,
    status: String,
    deleted_at: Option<i64>,
    update_count: i64,
    agent_id: Option<String>,
    memory_type: Option<String>,
}

impl SetRow {
    fn is_live(&self) -> bool {
        self.deleted_at.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Folder {
    root_id: String,
    index_docs: bool,
}

/// What deleting a project takes in: the project, its folders and remotes and
/// each row it holds, live or deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Set {
    project_id: String,
    name: String,
    folders: Vec<Folder>,
    remotes: Vec<String>,
    memories: Vec<SetRow>,
    trajectories: Vec<SetRow>,
    skills: Vec<SetRow>,
}

impl Set {
    fn live(&self) -> Counts {
        let live = |rows: &[SetRow]| rows.iter().filter(|r| r.is_live()).count();
        Counts { memories: live(&self.memories), trajectories: live(&self.trajectories), skills: live(&self.skills) }
    }

    fn deleted(&self) -> Counts {
        let live = self.live();
        Counts {
            memories: self.memories.len() - live.memories,
            trajectories: self.trajectories.len() - live.trajectories,
            skills: self.skills.len() - live.skills,
        }
    }

    fn is_empty(&self) -> bool {
        self.live() == Counts::default()
    }
}

/// The first 8 hex digits of the SHA-256 of the set in a canonical form: one
/// line per fact, sorted, so the order a store returns rows in does not
/// matter. A text is not part of it: nothing edits one after it is saved, and
/// every change that matters moves a field that is.
fn code_of(set: &Set) -> String {
    let mut lines = vec![format!("project {}", set.project_id)];
    let mut folders: Vec<&str> = set.folders.iter().map(|f| f.root_id.as_str()).collect();
    folders.sort_unstable();
    lines.extend(folders.into_iter().map(|id| format!("folder {id}")));
    let mut remotes: Vec<&str> = set.remotes.iter().map(String::as_str).collect();
    remotes.sort_unstable();
    lines.extend(remotes.into_iter().map(|remote| format!("remote {remote}")));
    for (table, rows) in [("memory", &set.memories), ("episodic_trajectory", &set.trajectories), ("procedural_skill", &set.skills)] {
        let mut rows: Vec<&SetRow> = rows.iter().collect();
        rows.sort_unstable_by_key(|r| r.id);
        for r in rows {
            let deleted_at = r.deleted_at.map_or("-".to_string(), |d| d.to_string());
            lines.push(format!("{table} {} {} {deleted_at} {}", r.id, r.status, r.update_count));
        }
    }
    let digest = Sha256::digest(lines.join("\n").as_bytes());
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}

fn rows_in(c: &Connection, table: &str, project_id: &str) -> rusqlite::Result<Vec<SetRow>> {
    let kind = if table == "memory" { "memory_type" } else { "NULL" };
    let mut stmt = c.prepare(&format!(
        "SELECT id, status, deleted_at, update_count, agent_id, {kind} FROM {table} WHERE project_id = ?1 ORDER BY id"
    ))?;
    let rows = stmt.query_map([project_id], |r| {
        Ok(SetRow {
            id: r.get(0)?,
            status: r.get(1)?,
            deleted_at: r.get(2)?,
            update_count: r.get(3)?,
            agent_id: r.get(4)?,
            memory_type: r.get(5)?,
        })
    })?;
    rows.collect()
}

/// Reads the set of a project, in the caller's transaction.
fn set_in(c: &Connection, project_id: &str, name: &str) -> rusqlite::Result<Set> {
    let folders = {
        let mut stmt = c.prepare("SELECT id, index_docs FROM project_path WHERE project_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map([project_id], |r| Ok(Folder { root_id: r.get(0)?, index_docs: r.get(1)? }))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let remotes = {
        let mut stmt = c.prepare("SELECT remote FROM project_remote WHERE project_id = ?1 ORDER BY remote")?;
        let rows = stmt.query_map([project_id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    Ok(Set {
        project_id: project_id.to_string(),
        name: name.to_string(),
        folders,
        remotes,
        memories: rows_in(c, "memory", project_id)?,
        trajectories: rows_in(c, "episodic_trajectory", project_id)?,
        skills: rows_in(c, "procedural_skill", project_id)?,
    })
}

/// The project and the tombstones that lead to it: the ids a save of the
/// project may still carry.
fn ids_of_in(c: &Connection, project_id: &str) -> Result<Vec<String>, DeleteError> {
    let mut ids = vec![project_id.to_string()];
    let tombstones: Vec<String> = {
        let mut stmt = c.prepare("SELECT id FROM project WHERE merged_into IS NOT NULL ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for id in tombstones {
        if canonical_in(c, &id)? == project_id {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// How many saves wait in the outbox for the project, and each one that gave up.
fn outbox_in(c: &Connection, ids: &[String]) -> rusqlite::Result<(usize, Vec<FailedSave>)> {
    let mut pending = 0;
    let mut failed = Vec::new();
    let mut count = c.prepare_cached(
        "SELECT COUNT(*) FROM ingest_outbox WHERE state = 'pending' AND json_extract(event, '$.project_id') = ?1",
    )?;
    let mut gave_up = c.prepare_cached(
        "SELECT operation_id, agent_id FROM ingest_outbox \
         WHERE state = 'failed' AND json_extract(event, '$.project_id') = ?1 ORDER BY id",
    )?;
    for id in ids {
        pending += count.query_row([id], |r| r.get::<_, usize>(0))?;
        let rows = gave_up.query_map([id], |r| Ok(FailedSave { operation_id: r.get(0)?, agent_id: r.get(1)? }))?;
        for row in rows {
            failed.push(row?);
        }
    }
    Ok((pending, failed))
}

/// The projects to empty `project_id` into: the duplicates `doctor` names
/// that include it, and the other side of its pending suggestions. A deleted
/// project and a pair someone answered are neither.
fn candidates_in(c: &Connection, project_id: &str) -> Result<Vec<Candidate>, DeleteError> {
    // A deleted project is left out of the registry, so a tombstone that leads
    // to one leads nowhere and gives no pair.
    let registry: Vec<RegistryRow> = {
        let mut stmt =
            c.prepare("SELECT id, name, created_at, merged_into FROM project WHERE deleted_at IS NULL ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok(RegistryRow { id: r.get(0)?, name: r.get(1)?, created_at: r.get(2)?, merged_into: r.get(3)? })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let answered = answered_pairs(c)?;
    let mut found: Vec<Candidate> = Vec::new();
    for pair in duplicates(&registry, &folder_prints(c)?, &answered) {
        let other = match (pair.older.id == project_id, pair.newer.id == project_id) {
            (true, _) => &pair.newer,
            (_, true) => &pair.older,
            _ => continue,
        };
        found.push(Candidate { project_id: other.id.clone(), name: other.name.clone(), reason: pair.reason.to_string() });
    }

    let suggestions: Vec<(String, String, String)> = {
        let mut stmt =
            c.prepare("SELECT project_id, other_id, reason FROM project_suggestion WHERE answer IS NULL ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (a, b, reason) in suggestions {
        let (a, b) = (canonical_in(c, &a)?, canonical_in(c, &b)?);
        let other = match (a == project_id, b == project_id) {
            (true, false) => b,
            (false, true) => a,
            _ => continue,
        };
        let (lesser, greater) = if project_id < other.as_str() { (project_id, other.as_str()) } else { (other.as_str(), project_id) };
        if answered.iter().any(|(x, y)| x == lesser && y == greater) {
            continue;
        }
        if let Some(live) = registry.iter().find(|p| p.id == other && p.merged_into.is_none()) {
            found.push(Candidate { project_id: live.id.clone(), name: live.name.clone(), reason });
        }
    }
    found.sort_by(|a, b| (&a.project_id, &a.reason).cmp(&(&b.project_id, &b.reason)));
    found.dedup();
    Ok(found)
}

/// Pairs of `memory_conflict` with one side in the project and the other out of it.
fn split_pairs_in(c: &Connection, project_id: &str) -> rusqlite::Result<usize> {
    c.query_row(
        "SELECT COUNT(*) FROM memory_conflict k \
         JOIN memory n ON n.id = k.new_id JOIN memory o ON o.id = k.old_id \
         WHERE (n.project_id IS ?1) <> (o.project_id IS ?1)",
        [project_id],
        |r| r.get(0),
    )
}

/// The live conclusions of another project, or of none, with a live source here.
fn conclusions_in(c: &Connection, project_id: &str) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = c.prepare(
        "SELECT DISTINCT k.id FROM memory_derivation d \
         JOIN memory s ON s.id = d.source_id AND s.project_id = ?1 AND s.deleted_at IS NULL \
         JOIN memory k ON k.id = d.memory_id \
         WHERE k.deleted_at IS NULL AND k.project_id IS NOT ?1 ORDER BY k.id",
    )?;
    let rows = stmt.query_map([project_id], |r| r.get(0))?;
    rows.collect()
}

/// What a report says about the set it read.
fn report_of(c: &Connection, set: &Set, with_memories: bool) -> Result<DeleteReport, DeleteError> {
    let (pending, failed) = outbox_in(c, &ids_of_in(c, &set.project_id)?)?;
    let mut report = DeleteReport {
        project_id: set.project_id.clone(),
        name: set.name.clone(),
        with_memories,
        live: set.live(),
        deleted: set.deleted(),
        split_pairs: split_pairs_in(c, &set.project_id)?,
        conclusions: conclusions_in(c, &set.project_id)?,
        folders: set.folders.iter().filter(|f| f.index_docs).map(|f| f.root_id.clone()).collect(),
        remotes: set.remotes.clone(),
        pending,
        failed,
        candidates: candidates_in(c, &set.project_id)?,
        ..DeleteReport::default()
    };
    let mut by_author: BTreeMap<Option<String>, usize> = BTreeMap::new();
    for row in set.memories.iter().filter(|r| r.is_live()) {
        let kind = row.memory_type.clone().unwrap_or_default();
        if kind == "user_preference" || kind == "architectural_decision" {
            report.protected.push(row.id);
        }
        *report.by_type.entry(kind).or_default() += 1;
        *report.by_status.entry(row.status.clone()).or_default() += 1;
        *by_author.entry(row.agent_id.clone()).or_default() += 1;
    }
    report.by_author = by_author.into_iter().map(|(agent_id, rows)| AuthorRows { agent_id, rows }).collect();
    Ok(report)
}

/// The vector table of each table of rows.
const VECTORS: [(&str, &str); 3] = [
    ("memory", "memory_vectors"),
    ("episodic_trajectory", "episodic_vectors"),
    ("procedural_skill", "procedural_vectors"),
];

/// Deletes a project inside the caller's transaction and returns the delete's
/// number. With `taken`, the code and the snapshot of a delete with its rows,
/// every live row of the set is marked deleted at once and loses its vector,
/// and each row of `memory` releases the rows that disputed it. Marking them
/// together is what keeps the older version of a key from coming back when the
/// newer one goes, which deleting row by row would do. Then the set is
/// journaled with the `deleted_at` each row had (none for the ones deleted
/// here), the docs index of its folders goes, and the project is marked.
fn write_delete(c: &Connection, set: &Set, taken: Option<(&str, &str)>, now: TimestampMs) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO project_delete (project_id, with_memories, code, snapshot, deleted_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![set.project_id, taken.is_some(), taken.map(|t| t.0), taken.map(|t| t.1), now],
    )?;
    let delete_id = c.last_insert_rowid();
    let mut journal =
        c.prepare_cached("INSERT INTO project_delete_row (delete_id, tbl, row_id, deleted_at) VALUES (?1, ?2, ?3, ?4)")?;
    for (table, rows) in [("memory", &set.memories), ("episodic_trajectory", &set.trajectories), ("procedural_skill", &set.skills)] {
        for row in rows {
            journal.execute(rusqlite::params![delete_id, table, row.id, row.deleted_at])?;
        }
    }
    if taken.is_some() {
        for ((table, vectors), rows) in VECTORS.iter().zip([&set.memories, &set.trajectories, &set.skills]) {
            c.execute(
                &format!("UPDATE {table} SET deleted_at = ?2 WHERE project_id = ?1 AND deleted_at IS NULL"),
                rusqlite::params![set.project_id, now],
            )?;
            for row in rows.iter().filter(|r| r.is_live()) {
                c.execute(&format!("DELETE FROM {vectors} WHERE rowid = ?1"), [row.id])?;
            }
        }
        for row in set.memories.iter().filter(|r| r.is_live()) {
            release_challengers(c, row.id, now)?;
        }
    }
    for folder in set.folders.iter().filter(|f| f.index_docs) {
        c.execute("UPDATE project_path SET index_docs = 0 WHERE id = ?1", [&folder.root_id])?;
        c.execute(
            "INSERT INTO project_delete_folder (delete_id, root_id) VALUES (?1, ?2)",
            rusqlite::params![delete_id, folder.root_id],
        )?;
        drop_root_index_in(c, &folder.root_id)?;
    }
    c.execute("UPDATE project SET deleted_at = ?2 WHERE id = ?1", rusqlite::params![set.project_id, now])?;
    Ok(delete_id)
}

/// What a pass does with the set it reads.
#[derive(Clone)]
enum Step {
    /// Delete the project if it is empty, else report `not_empty`.
    Empty,
    /// Report the set and its code; waits for nothing and writes nothing.
    Report,
    /// Compare `code` with the set's. Without a `snapshot` (the first pass)
    /// a match writes nothing; with one (the second) it deletes the set.
    Confirm { code: String, snapshot: Option<String> },
}

/// One pass over `project_id` inside the caller's transaction. It reads the
/// set and answers the report, and deletes what its step says to.
fn delete_pass(c: &Connection, project_id: &str, step: &Step, now: TimestampMs) -> Result<DeleteReport, DeleteError> {
    let (name, merged_into, deleted_at): (String, Option<String>, Option<i64>) = c
        .query_row("SELECT name, merged_into, deleted_at FROM project WHERE id = ?1", [project_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?
        .ok_or_else(|| DeleteError::UnknownProject(project_id.to_string()))?;
    if let Some(into) = merged_into {
        return Err(DeleteError::Merged { id: project_id.to_string(), into });
    }
    if deleted_at.is_some() {
        let delete_id = c
            .query_row(
                "SELECT delete_id FROM project_delete \
                 WHERE project_id = ?1 AND undone_at IS NULL AND purged_at IS NULL ORDER BY delete_id DESC LIMIT 1",
                [project_id],
                |r| r.get(0),
            )
            .optional()?;
        return Err(DeleteError::Deleted { id: project_id.to_string(), delete_id });
    }
    if !matches!(step, Step::Report) {
        let (pending, _) = outbox_in(c, &ids_of_in(c, project_id)?)?;
        if pending > 0 {
            return Err(DeleteError::SavesPending(pending as i64));
        }
    }
    let set = set_in(c, project_id, &name)?;
    let mut report = report_of(c, &set, !matches!(step, Step::Empty))?;
    match step {
        Step::Report => report.code = Some(code_of(&set)),
        Step::Empty if !set.is_empty() => report.refused = Some(Refusal::NotEmpty),
        Step::Empty => report.delete_id = Some(write_delete(c, &set, None, now)?),
        Step::Confirm { code, snapshot } => {
            let current = code_of(&set);
            report.code = Some(current.clone());
            if !code.trim().eq_ignore_ascii_case(&current) {
                report.refused = Some(Refusal::CodeChanged);
            } else if let Some(snapshot) = snapshot {
                report.delete_id = Some(write_delete(c, &set, Some((&current, snapshot)), now)?);
                report.snapshot = Some(snapshot.clone());
            }
        }
    }
    Ok(report)
}

/// Deletes `project_id`, or reports what deleting it would take. Without
/// `with_memories` a project with no live rows is deleted in one transaction,
/// after waiting up to `SAVES_WAIT` for its saves in the outbox, and one with
/// live rows is refused as `not_empty`; a `confirm` there is ignored. With
/// `with_memories` and no `confirm`, the answer is the report of its set and
/// the code of that set, and nothing is written. With both, the code is
/// compared with the set's in a first pass that writes nothing, a `pre-delete`
/// snapshot goes into `backups`, and a second pass compares it again and
/// deletes the project with its rows in one transaction. A code that is not
/// the set's answers the report of the set as it is, as `code_changed`, and
/// writes nothing. A confirmed delete waits for the saves of the project as
/// the one of an empty project does.
pub async fn delete_core(
    store: &MemoryStore,
    workspaces: &WorkspaceManager,
    backups: &Path,
    project_id: &str,
    with_memories: bool,
    confirm: Option<&str>,
) -> Result<DeleteReport, DeleteError> {
    let pass = |step: Step, write: bool| {
        let id = project_id.to_string();
        async move { retrying(store, write, move |c| delete_pass(c, &id, &step, TimestampMs::now())).await }
    };
    let mut report = match (with_memories, confirm) {
        (false, _) => pass(Step::Empty, true).await?,
        (true, None) => pass(Step::Report, false).await?,
        (true, Some(code)) => {
            let code = code.to_string();
            let found = pass(Step::Confirm { code: code.clone(), snapshot: None }, false).await?;
            if found.refused.is_some() {
                found
            } else {
                let snapshot = store.snapshot(backups.to_path_buf(), Reason::PreDelete).await?.display().to_string();
                pass(Step::Confirm { code, snapshot: Some(snapshot) }, true).await?
            }
        }
    };
    match workspaces.project_files(project_id).await {
        Ok(files) => report.workspaces = files.iter().map(|f| f.display().to_string()).collect(),
        Err(e) if report.delete_id.is_some() => {
            log::warn!("project {project_id} was deleted but its workspace files could not be listed: {e}");
        }
        Err(e) => return Err(e.into()),
    }
    Ok(report)
}

/// Texts embedded per call.
const EMBED_BATCH: usize = 32;

/// The tables a delete takes rows from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Table {
    Memory,
    Trajectory,
    Skill,
}

impl Table {
    fn of(name: &str) -> Option<Table> {
        match name {
            "memory" => Some(Table::Memory),
            "episodic_trajectory" => Some(Table::Trajectory),
            "procedural_skill" => Some(Table::Skill),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Table::Memory => "memory",
            Table::Trajectory => "episodic_trajectory",
            Table::Skill => "procedural_skill",
        }
    }

    fn vectors(self) -> &'static str {
        match self {
            Table::Memory => "memory_vectors",
            Table::Trajectory => "episodic_vectors",
            Table::Skill => "procedural_vectors",
        }
    }
}

/// What an undo decided: the rows that come back and the ones that stay
/// deleted, with why.
struct Plan {
    project_id: String,
    name: String,
    with_memories: bool,
    /// The `deleted_at` the delete gave the project and its rows.
    at: i64,
    restore: Vec<(Table, i64)>,
    kept: Vec<Kept>,
}

impl Plan {
    fn restored(&self) -> Counts {
        let of = |table| self.restore.iter().filter(|(t, _)| *t == table).count();
        Counts { memories: of(Table::Memory), trajectories: of(Table::Trajectory), skills: of(Table::Skill) }
    }
}

/// The rows among `restoring` that stay deleted because a piece of their
/// save does: a row comes back only with every piece of its save that is
/// deleted. Dropping one can tie another, so it goes on until none is left.
/// `siblings` are the other pieces of each row's saves, and `deleted` the
/// ones among them that are deleted.
fn tied(restoring: &[i64], siblings: &BTreeMap<i64, Vec<i64>>, deleted: &HashSet<i64>) -> BTreeSet<i64> {
    let mut staying: BTreeSet<i64> = restoring.iter().copied().collect();
    let mut tied = BTreeSet::new();
    loop {
        let next = staying
            .iter()
            .copied()
            .find(|id| siblings.get(id).is_some_and(|s| s.iter().any(|p| deleted.contains(p) && !staying.contains(p))));
        match next {
            Some(id) => {
                staying.remove(&id);
                tied.insert(id);
            }
            None => return tied,
        }
    }
}

/// Decides delete `delete_id`'s undo in the caller's transaction.
fn undo_plan_in(c: &Connection, delete_id: i64) -> Result<Plan, DeleteError> {
    let (project_id, with_memories, at, undone, purged): (String, bool, i64, Option<i64>, Option<i64>) = c
        .query_row(
            "SELECT project_id, with_memories, deleted_at, undone_at, purged_at FROM project_delete WHERE delete_id = ?1",
            [delete_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?
        .ok_or(DeleteError::UnknownDelete(delete_id))?;
    if undone.is_some() {
        return Err(DeleteError::Undone(delete_id));
    }
    if purged.is_some() {
        return Err(DeleteError::Purged(delete_id));
    }
    let later: Option<i64> = c
        .query_row(
            "SELECT delete_id FROM project_delete \
             WHERE project_id = ?1 AND delete_id > ?2 AND undone_at IS NULL AND purged_at IS NULL ORDER BY delete_id LIMIT 1",
            rusqlite::params![project_id, delete_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(later) = later {
        return Err(DeleteError::DeletedLater { project_id, later });
    }
    let name: String = c
        .query_row("SELECT name FROM project WHERE id = ?1", [&project_id], |r| r.get(0))
        .optional()?
        .ok_or_else(|| DeleteError::UnknownProject(project_id.clone()))?;
    let (pending, _) = outbox_in(c, &ids_of_in(c, &project_id)?)?;
    if pending > 0 {
        return Err(DeleteError::SavesPending(pending as i64));
    }

    let taken: Vec<(Table, i64)> = {
        let mut stmt = c.prepare(
            "SELECT tbl, row_id FROM project_delete_row WHERE delete_id = ?1 AND deleted_at IS NULL ORDER BY tbl, row_id",
        )?;
        let rows = stmt.query_map([delete_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut taken = Vec::new();
        for row in rows {
            let (tbl, id) = row?;
            match Table::of(&tbl) {
                Some(table) => taken.push((table, id)),
                None => log::warn!("delete {delete_id} journaled a row of {tbl}, which no undo knows"),
            }
        }
        taken.sort();
        taken
    };
    let mut kept = Vec::new();
    let mut restore = Vec::new();
    for (table, id) in taken {
        let kept_as = |reason| Kept { table: table.name().to_string(), id, reason };
        let now: Option<(Option<String>, Option<i64>)> = c
            .query_row(&format!("SELECT project_id, deleted_at FROM {} WHERE id = ?1", table.name()), [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        if now != Some((Some(project_id.clone()), Some(at))) {
            kept.push(kept_as(KeepReason::Changed));
            continue;
        }
        if table == Table::Memory {
            let content: String = c.query_row("SELECT content FROM memory WHERE id = ?1", [id], |r| r.get(0))?;
            let twin = c.query_row(RESTORE_TWIN, rusqlite::params![id, None::<String>], |r| r.get::<_, i64>(0)).optional()?;
            let holder = c
                .query_row(RESTORE_TOPIC_HOLDER, rusqlite::params![id, None::<String>], |r| r.get::<_, String>(0))
                .optional()?;
            let reason = if !redact(&content).kinds.is_empty() {
                Some(KeepReason::HoldsSecret)
            } else if twin.is_some() {
                Some(KeepReason::LiveTwin)
            } else if holder.is_some() {
                Some(KeepReason::TopicTaken)
            } else {
                None
            };
            if let Some(reason) = reason {
                kept.push(kept_as(reason));
                continue;
            }
        }
        restore.push((table, id));
    }

    let memories: Vec<i64> = restore.iter().filter(|(t, _)| *t == Table::Memory).map(|(_, id)| *id).collect();
    let mut siblings: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    let mut deleted: HashSet<i64> = HashSet::new();
    for id in &memories {
        let mut stmt = c.prepare_cached(
            "SELECT DISTINCT b.memory_id, m.deleted_at IS NOT NULL FROM memory_piece a \
             JOIN memory_piece b ON b.save_id = a.save_id AND b.memory_id <> a.memory_id \
             JOIN memory m ON m.id = b.memory_id WHERE a.memory_id = ?1",
        )?;
        let rows = stmt.query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, bool>(1)?)))?;
        for row in rows {
            let (piece, is_deleted) = row?;
            siblings.entry(*id).or_default().push(piece);
            if is_deleted {
                deleted.insert(piece);
            }
        }
    }
    let ties = tied(&memories, &siblings, &deleted);
    restore.retain(|(table, id)| !(*table == Table::Memory && ties.contains(id)));
    kept.extend(ties.into_iter().map(|id| Kept { table: Table::Memory.name().to_string(), id, reason: KeepReason::Tied }));
    kept.sort_by(|a, b| (Table::of(&a.table), a.id).cmp(&(Table::of(&b.table), b.id)));
    Ok(Plan { project_id, name, with_memories, at, restore, kept })
}

fn plan_report(delete_id: i64, plan: &Plan) -> DeleteReport {
    DeleteReport {
        project_id: plan.project_id.clone(),
        name: plan.name.clone(),
        with_memories: plan.with_memories,
        delete_id: Some(delete_id),
        restored: plan.restored(),
        kept: plan.kept.clone(),
        ..DeleteReport::default()
    }
}

/// The text each row of `restore` was embedded from.
fn embedding_texts_in(c: &Connection, restore: &[(Table, i64)]) -> rusqlite::Result<Vec<(Table, i64, String)>> {
    let mut texts = Vec::with_capacity(restore.len());
    for (table, id) in restore {
        let text = match table {
            Table::Memory => {
                // The head the row's save was embedded behind, as a restore reads it.
                let (content, context): (String, Option<String>) = c.query_row(
                    "SELECT content, (SELECT mp.context FROM memory_piece mp WHERE mp.memory_id = memory.id \
                     AND mp.context <> '' ORDER BY mp.save_id LIMIT 1) FROM memory WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                embedding_text(context.as_deref().unwrap_or(""), &content).into_owned()
            }
            Table::Trajectory => c.query_row("SELECT task FROM episodic_trajectory WHERE id = ?1", [id], |r| r.get(0))?,
            Table::Skill => {
                let (trigger, name): (String, String) =
                    c.query_row("SELECT trigger, skill_name FROM procedural_skill WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
                skill_embedding_text(&trigger, &name)
            }
        };
        texts.push((*table, *id, text));
    }
    Ok(texts)
}

/// Takes delete `delete_id` back inside the caller's transaction: decides
/// again what comes back, and puts it back with the vectors computed before.
/// A row without one stays deleted, as one that changed since the first pass.
fn undo_write_in(
    c: &Connection,
    delete_id: i64,
    vectors: &HashMap<(Table, i64), Vec<u8>>,
    now: TimestampMs,
) -> Result<DeleteReport, DeleteError> {
    let plan = undo_plan_in(c, delete_id)?;
    let mut report = plan_report(delete_id, &plan);
    report.restored = Counts::default();
    // The project is live first, so the rows that come back find it so and
    // leave it as it was.
    c.execute("UPDATE project SET deleted_at = NULL WHERE id = ?1 AND deleted_at = ?2", rusqlite::params![plan.project_id, plan.at])?;
    let roots: Vec<String> = {
        let mut stmt = c.prepare("SELECT root_id FROM project_delete_folder WHERE delete_id = ?1 ORDER BY root_id")?;
        let rows = stmt.query_map([delete_id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for root in roots {
        if c.execute("UPDATE project_path SET index_docs = 1 WHERE id = ?1 AND index_docs = 0", [&root])? > 0 {
            report.folders.push(root);
        }
    }
    let mut memories = Vec::new();
    for (table, id) in &plan.restore {
        let Some(vector) = vectors.get(&(*table, *id)) else {
            report.kept.push(Kept { table: table.name().to_string(), id: *id, reason: KeepReason::Changed });
            continue;
        };
        c.execute(
            &format!("UPDATE {} SET deleted_at = NULL WHERE id = ?1 AND project_id = ?2 AND deleted_at = ?3", table.name()),
            rusqlite::params![id, plan.project_id, plan.at],
        )?;
        c.execute(&format!("DELETE FROM {} WHERE rowid = ?1", table.vectors()), [id])?;
        c.execute(&format!("INSERT INTO {}(rowid, embedding) VALUES (?1, ?2)", table.vectors()), rusqlite::params![id, vector])?;
        match table {
            Table::Memory => {
                c.execute(RESTORE_TEXT_INDEX, [id])?;
                memories.push(*id);
                report.restored.memories += 1;
            }
            Table::Trajectory => report.restored.trajectories += 1,
            Table::Skill => report.restored.skills += 1,
        }
    }
    // The rows that disputed or were disputed come back to what is live now.
    for id in memories {
        settle_row(c, id, now)?;
        settle_challengers(c, id, now)?;
    }
    report.kept.sort_by(|a, b| (Table::of(&a.table), a.id).cmp(&(Table::of(&b.table), b.id)));
    c.execute("UPDATE project_delete SET undone_at = ?2 WHERE delete_id = ?1", rusqlite::params![delete_id, now])?;
    Ok(report)
}

/// Takes delete `delete_id` back: the project is live again with its docs
/// index turned on, and each row the delete deleted comes back as it was,
/// with its vector and its entry in the text index, unless it stays deleted:
/// a live row saved since has its text or its key (the refusals of
/// `undelete`), it holds a secret by today's rules, it is not as the delete
/// left it, or it is a piece of a save with a piece that stays. A row deleted
/// before the delete never comes back. The vectors are computed with
/// `embedder` before the transaction, so an undo with rows to restore and no
/// embedder is `NeedsModel`; a dry run and an undo with nothing to restore
/// need none. A `pre-undo` snapshot goes into `backups` before writing.
pub async fn undo_delete_core(
    store: &MemoryStore,
    embedder: Option<&dyn Embedder>,
    backups: &Path,
    delete_id: i64,
    dry_run: bool,
) -> Result<DeleteReport, DeleteError> {
    let plan = retrying(store, false, move |c| undo_plan_in(c, delete_id)).await?;
    if dry_run {
        return Ok(DeleteReport { dry_run: true, ..plan_report(delete_id, &plan) });
    }
    let mut vectors = HashMap::with_capacity(plan.restore.len());
    if !plan.restore.is_empty() {
        let embedder = embedder.ok_or(DeleteError::NeedsModel)?;
        let restore = plan.restore.clone();
        let texts = store.conn.call(move |c| embedding_texts_in(c, &restore)).await.map_err(MemoryError::Connection)?;
        for batch in texts.chunks(EMBED_BATCH) {
            let inputs: Vec<String> = batch.iter().map(|(_, _, text)| text.clone()).collect();
            let embedded = embedder.embed_batch(&inputs, EmbedRole::Passage).await?;
            for ((table, id, _), vector) in batch.iter().zip(embedded) {
                let unit = to_unit(&vector).map_err(|e| EmbedError::InferenceFailed(e.to_string()))?;
                vectors.insert((*table, *id), unit.iter().flat_map(|f| f.to_le_bytes()).collect());
            }
        }
    }
    let snapshot = store.snapshot(backups.to_path_buf(), Reason::PreUndo).await?.display().to_string();
    let mut report = retrying(store, true, move |c| undo_write_in(c, delete_id, &vectors, TimestampMs::now())).await?;
    report.snapshot = Some(snapshot);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryStore;

    const GONE: &str = "app-aaaa";

    async fn store_with_a_deleted_project() -> MemoryStore {
        let store = MemoryStore::new_for_test().await.unwrap();
        run(
            &store,
            &format!(
                "INSERT INTO project (id, name, born, reviewed, created_at, deleted_at) \
                   VALUES ('{GONE}', 'app', 'root', 1, 1, 5), ('other-bbbb', 'other', 'root', 1, 1, NULL);"
            ),
        )
        .await;
        store
    }

    async fn run(store: &MemoryStore, batch: &str) {
        let batch = batch.to_string();
        store.conn.call(move |c| c.execute_batch(&batch)).await.unwrap();
    }

    fn memory(id: i64, project: &str, deleted_at: &str) -> String {
        format!(
            "INSERT INTO memory \
             (id, scope, agent_id, project_id, content, memory_type, source, created_at, created_by_type, \
              created_by_ref, last_modified_at, last_accessed_at, importance, tier, status, deleted_at) \
             VALUES ({id}, 'agent', 'a', '{project}', 'note {id}', 'semantic_fact', 'explicit', {id}, 'agent', \
              'x', {id}, {id}, 5.0, 3, 'active', {deleted_at});"
        )
    }

    /// `(deleted_at, reviewed)` of a project.
    async fn state(store: &MemoryStore, id: &str) -> (Option<i64>, i64) {
        let id = id.to_string();
        store
            .conn
            .call(move |c| {
                c.query_row("SELECT deleted_at, reviewed FROM project WHERE id = ?1", [&id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_live_row_brings_a_deleted_project_back_unreviewed() {
        // A save, an import or a worker replay: a live row inserted under the id.
        let store = store_with_a_deleted_project().await;
        run(&store, &memory(1, GONE, "NULL")).await;
        assert_eq!(state(&store, GONE).await, (None, 0), "an insert");

        // A move into the project: a live row changes project.
        let store = store_with_a_deleted_project().await;
        run(&store, &memory(2, "other-bbbb", "NULL")).await;
        assert_eq!(state(&store, GONE).await, (Some(5), 1), "nothing wrote under it yet");
        run(&store, &format!("UPDATE memory SET project_id = '{GONE}' WHERE id = 2;")).await;
        assert_eq!(state(&store, GONE).await, (None, 0), "a change of project");

        // A restore: a deleted row of the project becomes live.
        let store = store_with_a_deleted_project().await;
        run(&store, &memory(3, GONE, "9")).await;
        assert_eq!(state(&store, GONE).await, (Some(5), 1), "a deleted row insert leaves it");
        run(&store, "UPDATE memory SET deleted_at = NULL WHERE id = 3;").await;
        assert_eq!(state(&store, GONE).await, (None, 0), "a restore");
    }

    #[tokio::test]
    async fn a_deleted_row_or_a_registration_leaves_it_deleted() {
        let store = store_with_a_deleted_project().await;
        run(&store, &memory(1, GONE, "9")).await;
        run(&store, &memory(2, "other-bbbb", "9")).await;
        run(&store, &format!("UPDATE memory SET project_id = '{GONE}' WHERE id = 2;")).await;
        assert_eq!(state(&store, GONE).await, (Some(5), 1), "deleted rows");

        run(
            &store,
            &format!(
                "INSERT OR IGNORE INTO project (id, name, born, reviewed, created_at) VALUES ('{GONE}', 'app', 'root', 0, 7);
                 INSERT INTO project_path (id, root_path, created_at, index_docs, project_id) \
                   VALUES ('{GONE}', '/work/app', 7, 0, '{GONE}');"
            ),
        )
        .await;
        assert_eq!(state(&store, GONE).await, (Some(5), 1), "a registration");

        run(&store, &format!("UPDATE memory SET importance = 6.0 WHERE id = 1; {}", memory(3, "other-bbbb", "NULL"))).await;
        assert_eq!(state(&store, GONE).await, (Some(5), 1), "another project's live row");
        assert_eq!(state(&store, "other-bbbb").await, (None, 1), "a live project keeps its review");
    }

    // ── deleting ────────────────────────────────────────────────────────

    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::memory::events::{MemoryEvent, MemorySource};
    use crate::memory::worker::save_for_test;
    use crate::memory::workspace::WorkspaceManager;
    use crate::models::chat::Role;

    const A: &str = "app-aaaa";
    const B: &str = "web-bbbb";
    const C: &str = "api-cccc";
    const D: &str = "doc-dddd";

    struct Fixture {
        store: Arc<MemoryStore>,
        workspaces: WorkspaceManager,
        root: tempfile::TempDir,
    }

    impl Fixture {
        /// Projects named after their ids without the suffix, each with a
        /// folder of its own.
        async fn new(projects: &[&str]) -> Self {
            let root = tempfile::tempdir().unwrap();
            let f = Fixture {
                store: Arc::new(MemoryStore::new_for_test().await.unwrap()),
                workspaces: WorkspaceManager::new(root.path().join("workspaces")),
                root,
            };
            for id in projects {
                let name = &id[..id.len() - 5];
                f.sql(&format!(
                    "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('{id}', '{name}', 'root', 0, 1);
                     INSERT INTO project_path (id, root_path, created_at, index_docs, project_id) \
                       VALUES ('{id}', '/work/{id}', 1, 0, '{id}');"
                ))
                .await;
            }
            f
        }

        async fn sql(&self, batch: &str) {
            let batch = batch.to_string();
            self.store.conn.call(move |c| c.execute_batch(&batch)).await.unwrap();
        }

        async fn rows<T: Send + 'static>(&self, sql: &str, row: fn(&rusqlite::Row<'_>) -> rusqlite::Result<T>) -> Vec<T> {
            let sql = sql.to_string();
            self.store
                .conn
                .call(move |c| {
                    let mut stmt = c.prepare(&sql)?;
                    let rows = stmt.query_map([], row)?.collect::<rusqlite::Result<Vec<T>>>();
                    rows
                })
                .await
                .unwrap()
        }

        async fn count(&self, sql: &str) -> i64 {
            self.rows(sql, |r| r.get(0)).await[0]
        }

        /// A row of `memory` with every field a report reads.
        async fn row(&self, id: i64, project: &str, agent: Option<&str>, kind: &str, status: &str, deleted_at: Option<i64>) {
            self.row_in(id, project, agent, kind, status, deleted_at, &format!("note {id}")).await;
        }

        async fn row_text(&self, id: i64, project: &str, content: &str) {
            self.row_in(id, project, Some("agent-a"), "semantic_fact", "active", None, content).await;
        }

        async fn row_in(&self, id: i64, project: &str, agent: Option<&str>, kind: &str, status: &str, deleted_at: Option<i64>, content: &str) {
            let agent = agent.map_or("NULL".to_string(), |a| format!("'{a}'"));
            let scope = if agent == "NULL" { "project" } else { "agent" };
            let deleted_at = deleted_at.map_or("NULL".to_string(), |d| d.to_string());
            self.sql(&format!(
                "INSERT INTO memory \
                 (id, scope, agent_id, project_id, content, memory_type, source, created_at, created_by_type, \
                  created_by_ref, last_modified_at, last_accessed_at, importance, tier, status, deleted_at) \
                 VALUES ({id}, '{scope}', {agent}, '{project}', '{content}', '{kind}', 'explicit', {id}, 'agent', \
                  'x', {id}, {id}, 5.0, 3, '{status}', {deleted_at});"
            ))
            .await;
        }

        async fn live(&self, id: i64, project: &str) {
            self.row(id, project, Some("agent-a"), "semantic_fact", "active", None).await;
        }

        async fn save(&self, content: &str, project: &str, at: u64) -> i64 {
            let evt = MemoryEvent {
                agent_id: "agent-a".into(),
                project_id: Some(project.into()),
                role: Role::User,
                content: content.into(),
                is_error: None,
                source: MemorySource::Explicit,
                timestamp_ms: at,
                memory_type: Some("semantic_fact".into()),
                importance: None,
                derived_from: Vec::new(),
                topic_key: None,
                user_stated: false,
                session_id: None,
            };
            let report = save_for_test(&self.store, &evt).await;
            assert_eq!(report.failure, None);
            report.memories[0].id
        }

        async fn outbox(&self, operation: &str, project: &str, state: &str) {
            let event = format!(r#"{{"agent_id":"agent-a","project_id":"{project}","content":""}}"#);
            self.sql(&format!(
                "INSERT INTO ingest_outbox (operation_id, agent_id, content, event, state, created_at, updated_at) \
                 VALUES ('{operation}', 'agent-a', 'a save', '{event}', '{state}', 1, 1);"
            ))
            .await;
        }

        async fn docs(&self, root_id: &str, chunk: i64) {
            self.sql(&format!(
                "INSERT INTO project_doc_file (project_id, path, content_hash, model_id, chunks, indexed_at) \
                   VALUES ('{root_id}', 'README.md', 'h', 'm', 1, 1);
                 INSERT INTO project_doc_chunk (id, project_id, path, ordinal, breadcrumb, body) \
                   VALUES ({chunk}, '{root_id}', 'README.md', 0, 'README.md', 'the docs of the project');
                 INSERT INTO project_doc_fts (rowid, text, project_id) VALUES ({chunk}, 'the docs of the project', '{root_id}');"
            ))
            .await;
        }

        async fn delete(&self, id: &str, with_memories: bool) -> Result<DeleteReport, DeleteError> {
            delete_core(&self.store, &self.workspaces, &self.backups(), id, with_memories, None).await
        }

        fn workspace(&self, agent: &str, project: &str) -> PathBuf {
            self.workspaces.workspace_path(agent, Some(project))
        }

        fn write_workspace(&self, agent: &str, project: &str) {
            let path = self.workspace(agent, project);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "<!-- AUTO-START -->\n<!-- AUTO-END -->\n").unwrap();
        }

        /// Everything a delete may touch, as text, to compare before and after.
        async fn everything(&self) -> Vec<String> {
            let mut out = Vec::new();
            for sql in [
                "SELECT json_array(id, project_id, status, deleted_at, update_count, last_modified_at) FROM memory",
                "SELECT json_array('trajectory', id, project_id, status, deleted_at) FROM episodic_trajectory",
                "SELECT json_array('skill', id, project_id, status, deleted_at) FROM procedural_skill",
                "SELECT json_array(id, name, reviewed, merged_into, deleted_at) FROM project",
                "SELECT json_array(id, project_id, index_docs) FROM project_path",
                "SELECT json_array(remote, project_id) FROM project_remote",
                "SELECT json_array(project_id, other_id, answer) FROM project_suggestion",
                "SELECT json_array(agent_id, scope_key, reviewed_at) FROM synthesis_review",
                "SELECT json_array(new_id, old_id, state) FROM memory_conflict",
                "SELECT json_array(delete_id, project_id) FROM project_delete",
                "SELECT json_array(delete_id, tbl, row_id) FROM project_delete_row",
                "SELECT json_array(delete_id, root_id) FROM project_delete_folder",
                "SELECT json_array(project_id, path) FROM project_doc_file",
                "SELECT json_array(id) FROM project_doc_chunk",
                "SELECT json_array(rowid) FROM project_doc_fts",
            ] {
                out.extend(self.rows(&format!("{sql} ORDER BY 1"), |r| r.get::<_, String>(0)).await);
            }
            out
        }
    }

    #[tokio::test]
    async fn an_empty_project_is_deleted_and_its_docs_index_goes() {
        let f = Fixture::new(&[A, B]).await;
        let old = "app-oldd";
        f.sql(&format!(
            "UPDATE project_path SET index_docs = 1 WHERE id = '{A}';
             INSERT INTO project_path (id, root_path, created_at, index_docs, project_id) VALUES ('{old}', '/old/app', 1, 0, '{A}');
             INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/app', '{A}', 1);
             INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) VALUES ('gone-1111', 'gone', 'root', 0, 1, '{A}', 2);
             INSERT INTO project_suggestion (project_id, other_id, reason, created_at) VALUES ('{B}', '{A}', 'root_commit', 1);
             INSERT INTO synthesis_review (agent_id, scope_key, reviewed_at) VALUES ('', '{A}', 10);"
        ))
        .await;
        f.row(1, A, Some("agent-a"), "semantic_fact", "active", Some(4)).await;
        f.row(2, A, Some("agent-a"), "semantic_fact", "superseded", Some(6)).await;
        f.docs(A, 900).await;
        f.docs(B, 901).await;
        f.write_workspace("agent-a", A);

        let report = f.delete(A, false).await.unwrap();

        assert_eq!((report.refused, report.with_memories, report.delete_id), (None, false, Some(1)));
        assert_eq!((report.snapshot.clone(), report.code.clone()), (None, None), "no snapshot, and no code in the first stage");
        assert_eq!((report.live, report.deleted), (Counts::default(), Counts { memories: 2, ..Counts::default() }));
        assert_eq!(report.folders, vec![A.to_string()], "only the folder that had its docs indexed");

        let state: Vec<Option<i64>> = f.rows(&format!("SELECT deleted_at FROM project WHERE id = '{A}'"), |r| r.get(0)).await;
        assert!(state[0].is_some(), "A is deleted");
        let journal: Vec<(i64, String, i64, i64, Option<String>)> = f
            .rows(
                "SELECT delete_id, project_id, with_memories, deleted_at IS NOT NULL, snapshot FROM project_delete",
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .await;
        assert_eq!(journal, vec![(1, A.to_string(), 0, 1, None)]);
        let rows: Vec<(String, i64, Option<i64>)> = f
            .rows("SELECT tbl, row_id, deleted_at FROM project_delete_row ORDER BY row_id", |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .await;
        assert_eq!(rows, vec![("memory".to_string(), 1, Some(4)), ("memory".to_string(), 2, Some(6))]);
        let folders: Vec<String> = f.rows("SELECT root_id FROM project_delete_folder", |r| r.get(0)).await;
        assert_eq!(folders, vec![A.to_string()]);
        let index: Vec<(String, i64)> =
            f.rows("SELECT id, index_docs FROM project_path ORDER BY id", |r| Ok((r.get(0)?, r.get(1)?))).await;
        assert_eq!(index, vec![(A.to_string(), 0), (old.to_string(), 0), (B.to_string(), 0)]);
        assert_eq!(f.count(&format!("SELECT COUNT(*) FROM project_doc_file WHERE project_id = '{A}'")).await, 0);
        assert_eq!(f.count(&format!("SELECT COUNT(*) FROM project_doc_chunk WHERE project_id = '{A}'")).await, 0);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_doc_fts").await, 1, "the other root keeps its passage");
        assert_eq!(f.count(&format!("SELECT COUNT(*) FROM project_doc_file WHERE project_id = '{B}'")).await, 1);

        assert_eq!(f.count("SELECT COUNT(*) FROM project_path").await, 3, "the folders stay");
        assert_eq!(f.count(&format!("SELECT COUNT(*) FROM project_remote WHERE project_id = '{A}'")).await, 1);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_suggestion WHERE answer IS NULL").await, 1);
        assert_eq!(f.count(&format!("SELECT COUNT(*) FROM project WHERE merged_into = '{A}'")).await, 1);
        assert_eq!(f.count("SELECT COUNT(*) FROM synthesis_review").await, 1);
        assert_eq!(f.count("SELECT COUNT(*) FROM memory WHERE deleted_at IS NOT NULL").await, 2);
        assert_eq!(report.workspaces, vec![f.workspace("agent-a", A).display().to_string()]);
        assert!(f.workspace("agent-a", A).exists());
    }

    #[tokio::test]
    async fn a_project_with_live_rows_is_refused_with_its_candidates() {
        let f = Fixture::new(&[A, B, C, D, "app-eeee", "app-ffff"]).await;
        f.sql(&format!(
            "UPDATE project SET name = 'App' WHERE id IN ('{B}', 'app-eeee', 'app-ffff');
             UPDATE project SET name = 'app' WHERE id = '{A}';
             UPDATE project SET deleted_at = 3 WHERE id = 'app-ffff';
             UPDATE project_path SET root_commit = 'c0ffee' WHERE id IN ('{A}', '{C}');
             INSERT INTO project_suggestion (project_id, other_id, reason, created_at) VALUES ('{D}', '{A}', 'root_commit', 1);
             INSERT INTO project_suggestion (project_id, other_id, reason, created_at, answer, answered_at) \
               VALUES ('app-eeee', '{A}', 'same_name', 1, 'apart', 2);"
        ))
        .await;
        f.live(1, A).await;
        f.live(2, A).await;
        f.row(3, A, Some("agent-a"), "semantic_fact", "superseded", None).await;
        f.row(4, A, Some("agent-a"), "semantic_fact", "active", Some(9)).await;
        let before = f.everything().await;

        let report = f.delete(A, false).await.unwrap();

        assert_eq!(report.refused, Some(Refusal::NotEmpty));
        assert_eq!((report.live.memories, report.deleted.memories, report.delete_id), (3, 1, None));
        let candidates: Vec<(&str, &str)> = report.candidates.iter().map(|c| (c.project_id.as_str(), c.reason.as_str())).collect();
        assert_eq!(
            candidates,
            vec![(C, "same_first_commit"), (D, "root_commit"), (B, "same_name")],
            "an answered pair and a deleted project are no candidates"
        );
        assert_eq!(report.candidates[2].name, "App");
        assert_eq!(f.everything().await, before, "nothing is written");
    }

    #[tokio::test]
    async fn a_delete_is_refused_for_an_unknown_a_merged_or_a_deleted_project() {
        let f = Fixture::new(&[A, B, C]).await;
        f.sql(&format!("UPDATE project SET merged_into = '{B}', merged_at = 2 WHERE id = '{A}';")).await;
        for with_memories in [false, true] {
            let unknown = f.delete("nowhere-9f9f", with_memories).await;
            assert!(matches!(&unknown, Err(DeleteError::UnknownProject(id)) if id == "nowhere-9f9f"), "{unknown:?}");
            let merged = f.delete(A, with_memories).await;
            assert!(matches!(&merged, Err(DeleteError::Merged { id, into }) if id == A && into == B), "{merged:?}");
        }
        let first = f.delete(C, false).await.unwrap().delete_id;
        assert!(first.is_some());
        let before = f.everything().await;
        for with_memories in [false, true] {
            let deleted = f.delete(C, with_memories).await;
            assert!(matches!(&deleted, Err(DeleteError::Deleted { id, delete_id }) if id == C && *delete_id == first), "{deleted:?}");
        }
        assert_eq!(f.everything().await, before);
    }

    #[tokio::test(start_paused = true)]
    async fn a_delete_waits_for_the_saves_of_its_project() {
        let f = Fixture::new(&[A, B]).await;
        f.sql(&format!(
            "INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) VALUES ('old-1111', 'old', 'root', 0, 1, '{A}', 2);"
        ))
        .await;
        f.outbox("op-a", A, "pending").await;
        f.outbox("op-t", "old-1111", "pending").await;
        f.outbox("op-b", B, "pending").await;
        f.outbox("op-f", A, "failed").await;
        let before = f.everything().await;

        let refused = f.delete(A, false).await;

        assert!(matches!(refused, Err(DeleteError::SavesPending(2))), "{refused:?}: its own and its tombstone's, not B's");
        assert_eq!(f.everything().await, before);

        // The report does not wait: it counts what is pending and names what failed.
        f.live(1, A).await;
        let report = f.delete(A, true).await.unwrap();
        assert_eq!((report.pending, report.refused), (2, None));
        assert_eq!(report.failed, vec![FailedSave { operation_id: "op-f".into(), agent_id: "agent-a".into() }]);

        f.sql("DELETE FROM ingest_outbox WHERE state = 'pending'").await;
        f.sql("UPDATE memory SET deleted_at = 5").await;
        assert!(f.delete(A, false).await.unwrap().delete_id.is_some(), "a failed save is not in flight");
    }

    #[tokio::test]
    async fn the_report_counts_by_type_author_and_status_and_writes_nothing() {
        let f = Fixture::new(&[A, B]).await;
        f.row(1, A, Some("agent-a"), "semantic_fact", "active", None).await;
        f.row(2, A, Some("agent-a"), "user_preference", "active", None).await;
        f.row(3, A, Some("agent-b"), "architectural_decision", "disputed", None).await;
        f.row(4, A, None, "semantic_fact", "superseded", None).await;
        f.row(5, A, Some("agent-b"), "semantic_fact", "active", Some(8)).await;
        f.row(6, B, Some("agent-a"), "semantic_fact", "active", None).await;
        f.row(7, B, Some("agent-a"), "semantic_fact", "active", None).await;
        f.row(8, B, Some("agent-a"), "semantic_fact", "active", Some(3)).await;
        f.sql(&format!(
            "INSERT INTO episodic_trajectory (scope, agent_id, project_id, session_id, task, created_at, last_modified_at, last_accessed_at) \
               VALUES ('agent', 'agent-a', '{A}', 's1', 't', 1, 1, 1);
             INSERT INTO episodic_trajectory (scope, agent_id, project_id, session_id, task, created_at, last_modified_at, last_accessed_at, deleted_at) \
               VALUES ('agent', 'agent-a', '{A}', 's2', 't', 1, 1, 1, 5);
             INSERT INTO procedural_skill (scope, agent_id, project_id, skill_name, trigger, promoted_at, source_trajectory_ids, created_at, last_modified_at, last_accessed_at) \
               VALUES ('agent', 'agent-a', '{A}', 'k', 't', 1, '[]', 1, 1, 1);
             INSERT INTO memory_conflict (new_id, old_id, distance, state, created_at) VALUES (6, 1, 0.1, 'pending', 1), (2, 1, 0.1, 'pending', 1), (3, 8, 0.1, 'pending', 1);
             INSERT INTO memory_derivation (memory_id, source_id, created_at) VALUES (7, 1, 1), (2, 1, 1), (8, 2, 1);
             INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/app', '{A}', 1);
             UPDATE project_path SET index_docs = 1 WHERE id = '{A}';"
        ))
        .await;
        f.outbox("op-p", A, "pending").await;
        f.outbox("op-f", A, "failed").await;
        f.docs(A, 900).await;
        f.write_workspace("agent-a", A);
        f.write_workspace("agent-b", A);
        f.write_workspace("agent-a", B);
        let before = f.everything().await;

        let report = f.delete(A, true).await.unwrap();

        assert_eq!((report.project_id.as_str(), report.name.as_str(), report.with_memories, report.refused), (A, "app", true, None));
        assert!(report.code.as_deref().is_some_and(|c| c.len() == 8 && c.chars().all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch))));
        assert_eq!((report.delete_id, report.snapshot.clone()), (None, None));
        assert_eq!(report.live, Counts { memories: 4, trajectories: 1, skills: 1 });
        assert_eq!(report.deleted, Counts { memories: 1, trajectories: 1, skills: 0 });
        assert_eq!(
            report.by_type,
            BTreeMap::from([
                ("architectural_decision".to_string(), 1),
                ("semantic_fact".to_string(), 2),
                ("user_preference".to_string(), 1)
            ])
        );
        assert_eq!(
            report.by_author,
            vec![
                AuthorRows { agent_id: None, rows: 1 },
                AuthorRows { agent_id: Some("agent-a".into()), rows: 2 },
                AuthorRows { agent_id: Some("agent-b".into()), rows: 1 },
            ]
        );
        assert_eq!(
            report.by_status,
            BTreeMap::from([("active".to_string(), 2), ("disputed".to_string(), 1), ("superseded".to_string(), 1)])
        );
        assert_eq!(report.protected, vec![2, 3]);
        assert_eq!(report.split_pairs, 2, "A with B's row and with B's deleted one; the pair inside A stays whole");
        assert_eq!(report.conclusions, vec![7], "B's live conclusion from a row of A; not A's own, not a deleted one");
        assert_eq!(
            report.workspaces,
            vec![f.workspace("agent-a", A).display().to_string(), f.workspace("agent-b", A).display().to_string()]
        );
        assert_eq!((report.folders.clone(), report.remotes.clone()), (vec![A.to_string()], vec!["github.com/acme/app".to_string()]));
        assert_eq!(report.pending, 1);
        assert_eq!(report.failed, vec![FailedSave { operation_id: "op-f".into(), agent_id: "agent-a".into() }]);
        assert_eq!(f.everything().await, before, "the store and the docs index are as they were");
        assert!(!f.root.path().join("backups").exists(), "no snapshot");

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["refused"], serde_json::Value::Null);
        assert_eq!(json["by_author"][0]["agent_id"], serde_json::Value::Null);
        let back: DeleteReport = serde_json::from_value(json).unwrap();
        assert_eq!(back, report);
    }

    #[tokio::test]
    async fn the_code_changes_with_a_save_an_edit_a_restore_or_a_locator() {
        let f = Fixture::new(&[A]).await;
        f.live(1, A).await;
        f.row(2, A, Some("agent-a"), "semantic_fact", "active", Some(7)).await;
        async fn code(f: &Fixture) -> String {
            f.delete(A, true).await.unwrap().code.unwrap()
        }
        let base = code(&f).await;
        assert_eq!(code(&f).await, base, "the same set, the same code");

        f.sql("UPDATE memory SET last_modified_at = 99 WHERE id = 1").await;
        assert_eq!(code(&f).await, base, "a date that no field of the set moves with is no change");

        let mut seen = vec![base];
        f.save("a save that arrives between the two steps", A, 20).await;
        let now = code(&f).await;
        assert!(!seen.contains(&now), "a save left the code as it was");
        seen.push(now);
        f.sql("UPDATE memory SET update_count = update_count + 1 WHERE id = 1").await;
        let now = code(&f).await;
        assert!(!seen.contains(&now), "an edit left the code as it was");
        seen.push(now);
        f.sql("UPDATE memory SET status = 'disputed' WHERE id = 1").await;
        let now = code(&f).await;
        assert!(!seen.contains(&now), "a verdict left the code as it was");
        seen.push(now);
        f.sql("UPDATE memory SET deleted_at = NULL, update_count = update_count + 1 WHERE id = 2").await;
        let now = code(&f).await;
        assert!(!seen.contains(&now), "a restore left the code as it was");
        seen.push(now);
        f.sql(&format!(
            "INSERT INTO project_path (id, root_path, created_at, index_docs, project_id) VALUES ('app-oldd', '/old/app', 1, 0, '{A}');"
        ))
        .await;
        let now = code(&f).await;
        assert!(!seen.contains(&now), "a folder left the code as it was");
        seen.push(now);
        f.sql(&format!("INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/app', '{A}', 1);")).await;
        let now = code(&f).await;
        assert!(!seen.contains(&now), "a remote left the code as it was");
    }

    #[test]
    fn the_code_of_a_set_does_not_depend_on_its_order() {
        let row = |id: i64, status: &str, deleted_at: Option<i64>| SetRow {
            id,
            status: status.to_string(),
            deleted_at,
            update_count: id,
            agent_id: None,
            memory_type: None,
        };
        let set = Set {
            project_id: A.to_string(),
            name: "app".into(),
            folders: vec![Folder { root_id: "a".into(), index_docs: true }, Folder { root_id: "b".into(), index_docs: false }],
            remotes: vec!["github.com/x/y".into(), "github.com/x/z".into()],
            memories: vec![row(1, "active", None), row(2, "superseded", Some(5)), row(3, "active", None)],
            trajectories: vec![row(1, "active", None), row(4, "active", None)],
            skills: vec![row(9, "active", None)],
        };
        let shuffled = Set {
            folders: set.folders.iter().rev().cloned().collect(),
            remotes: set.remotes.iter().rev().cloned().collect(),
            memories: vec![set.memories[2].clone(), set.memories[0].clone(), set.memories[1].clone()],
            trajectories: set.trajectories.iter().rev().cloned().collect(),
            ..set.clone()
        };
        assert_eq!(code_of(&set), code_of(&shuffled));
        assert_eq!(code_of(&set).len(), 8);

        // The same rows in another table are another set.
        let moved = Set { memories: vec![], trajectories: set.memories.clone(), ..set.clone() };
        assert_ne!(code_of(&set), code_of(&moved));
        let other = Set { project_id: B.to_string(), ..set.clone() };
        assert_ne!(code_of(&set), code_of(&other));
    }

    // ── deleting with the rows ──────────────────────────────────────────

    impl Fixture {
        async fn confirm(&self, id: &str, code: &str) -> Result<DeleteReport, DeleteError> {
            delete_core(&self.store, &self.workspaces, &self.backups(), id, true, Some(code)).await
        }

        async fn code(&self, id: &str) -> String {
            self.delete(id, true).await.unwrap().code.unwrap()
        }

        async fn vector(&self, table: &str, id: i64) {
            let sql = format!("INSERT INTO {table}(rowid, embedding) VALUES (?1, ?2)");
            self.store
                .conn
                .call(move |c| c.execute(&sql, rusqlite::params![id, vec![0u8; 384 * 4]]))
                .await
                .unwrap();
        }

        /// What a delete must leave in a row: everything but `deleted_at`.
        async fn untouched(&self) -> Vec<String> {
            let mut out = Vec::new();
            for sql in [
                "SELECT json_array(id, status, update_count, last_modified_at, content, superseded_by, disputed_against, topic_key) FROM memory",
                "SELECT json_array('t', id, status, update_count, last_modified_at) FROM episodic_trajectory",
                "SELECT json_array('s', id, status, update_count, last_modified_at) FROM procedural_skill",
                "SELECT json_array(new_id, old_id, state) FROM memory_conflict",
                "SELECT json_array(memory_id, source_id) FROM memory_derivation",
                "SELECT json_array(id, project_id) FROM project_path",
                "SELECT json_array(remote, project_id) FROM project_remote",
                "SELECT json_array(project_id, other_id, answer) FROM project_suggestion",
                "SELECT json_array(agent_id, scope_key, reviewed_at) FROM synthesis_review",
                "SELECT json_array(operation_id, state) FROM ingest_outbox",
            ] {
                out.extend(self.rows(&format!("{sql} ORDER BY 1"), |r| r.get::<_, String>(0)).await);
            }
            out
        }
    }

    impl Fixture {
        fn backups(&self) -> PathBuf {
            self.root.path().join("backups")
        }
    }

    /// A with a key of two versions, a deleted row, a trajectory and a skill,
    /// each with a vector, a folder with a docs index and a `workspace.md`;
    /// B with a row a verdict of A's replaced and a row that disputed A's.
    async fn project_with_its_rows(f: &Fixture) {
        f.sql(&format!(
            "UPDATE project_path SET index_docs = 1 WHERE id = '{A}';
             INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/app', '{A}', 1);
             INSERT INTO project_suggestion (project_id, other_id, reason, created_at) VALUES ('{B}', '{A}', 'root_commit', 1);
             INSERT INTO synthesis_review (agent_id, scope_key, reviewed_at) VALUES ('', '{A}', 10);"
        ))
        .await;
        f.row(1, A, Some("agent-a"), "semantic_fact", "superseded", None).await;
        f.row(2, A, Some("agent-a"), "semantic_fact", "active", None).await;
        f.row(3, A, Some("agent-a"), "semantic_fact", "active", Some(4)).await;
        f.row(10, B, Some("agent-a"), "semantic_fact", "superseded", None).await;
        f.row(11, B, Some("agent-a"), "semantic_fact", "disputed", None).await;
        f.sql(
            "UPDATE memory SET topic_key = 'storage/engine', superseded_by = 2, supersede_cause = 'revision', update_count = 2 WHERE id = 1;
             UPDATE memory SET topic_key = 'storage/engine', update_count = 3, last_modified_at = 77 WHERE id = 2;
             UPDATE memory SET superseded_by = 2, supersede_cause = 'verdict' WHERE id = 10;
             UPDATE memory SET disputed_against = '[2]' WHERE id = 11;
             INSERT INTO memory_conflict (new_id, old_id, distance, state, created_at) VALUES (11, 2, 0.1, 'escalated', 1), (10, 2, 0.1, 'judged', 1);
             INSERT INTO memory_derivation (memory_id, source_id, created_at) VALUES (11, 2, 1);",
        )
        .await;
        f.sql(&format!(
            "INSERT INTO episodic_trajectory (id, scope, agent_id, project_id, session_id, task, created_at, last_modified_at, last_accessed_at) \
               VALUES (50, 'agent', 'agent-a', '{A}', 's1', 't', 1, 1, 1);
             INSERT INTO procedural_skill (id, scope, agent_id, project_id, skill_name, trigger, promoted_at, source_trajectory_ids, created_at, last_modified_at, last_accessed_at) \
               VALUES (60, 'agent', 'agent-a', '{A}', 'k', 't', 1, '[]', 1, 1, 1);"
        ))
        .await;
        f.vector("memory_vectors", 2).await;
        f.vector("memory_vectors", 1).await;
        f.vector("episodic_vectors", 50).await;
        f.vector("procedural_vectors", 60).await;
        f.docs(A, 900).await;
        f.write_workspace("agent-a", A);
    }

    #[tokio::test]
    async fn deleting_with_memories_marks_every_row_and_revives_none() {
        let f = Fixture::new(&[A, B]).await;
        project_with_its_rows(&f).await;
        let code = f.code(A).await;
        let before = f.untouched().await;
        let workspace = std::fs::read_to_string(f.workspace("agent-a", A)).unwrap();

        let report = f.confirm(A, &code).await.unwrap();

        assert_eq!((report.refused, report.code.as_deref(), report.delete_id), (None, Some(code.as_str()), Some(1)));
        assert_eq!((report.live, report.deleted), (Counts { memories: 2, trajectories: 1, skills: 1 }, Counts { memories: 1, ..Counts::default() }));
        let snapshot = report.snapshot.clone().expect("a snapshot before writing");
        assert!(snapshot.contains("pre-delete") && std::path::Path::new(&snapshot).exists(), "{snapshot}");

        let marks: Vec<(i64, Option<i64>)> = f
            .rows("SELECT id, deleted_at FROM memory WHERE project_id = 'app-aaaa' ORDER BY id", |r| Ok((r.get(0)?, r.get(1)?)))
            .await;
        assert_eq!(marks[2], (3, Some(4)), "a row deleted before keeps its date");
        assert!(marks[0].1.is_some() && marks[0].1 == marks[1].1, "every live row is marked at once: {marks:?}");
        let at = marks[0].1;
        for table in ["episodic_trajectory", "procedural_skill"] {
            let marked: Vec<Option<i64>> = f.rows(&format!("SELECT deleted_at FROM {table}"), |r| r.get(0)).await;
            assert_eq!(marked, vec![at], "{table}");
        }
        let status: Vec<String> = f.rows("SELECT status FROM memory WHERE id = 1", |r| r.get(0)).await;
        assert_eq!(status, vec!["superseded".to_string()], "the older version did not come back");
        for table in ["memory_vectors", "episodic_vectors", "procedural_vectors"] {
            assert_eq!(f.count(&format!("SELECT COUNT(*) FROM {table}")).await, 0, "{table}");
        }
        let indexed: Vec<i64> = f.rows("SELECT rowid FROM memory_fts WHERE memory_fts MATCH 'note' ORDER BY rowid", |r| r.get(0)).await;
        assert_eq!(indexed, vec![3, 10, 11], "the rows deleted here left the text index; row 3 was inserted deleted, so it never did");

        let journal: Vec<(i64, String, i64, i64, String, String)> = f
            .rows(
                "SELECT delete_id, project_id, with_memories, deleted_at IS NOT NULL, code, snapshot FROM project_delete",
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .await;
        assert_eq!(journal, vec![(1, A.to_string(), 1, 1, code.clone(), snapshot)]);
        let rows: Vec<(String, i64, Option<i64>)> = f
            .rows("SELECT tbl, row_id, deleted_at FROM project_delete_row ORDER BY tbl, row_id", |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .await;
        assert_eq!(
            rows,
            vec![
                ("episodic_trajectory".to_string(), 50, None),
                ("memory".to_string(), 1, None),
                ("memory".to_string(), 2, None),
                ("memory".to_string(), 3, Some(4)),
                ("procedural_skill".to_string(), 60, None),
            ]
        );
        let state: Vec<Option<i64>> = f.rows(&format!("SELECT deleted_at FROM project WHERE id = '{A}'"), |r| r.get(0)).await;
        assert!(state[0].is_some());
        let folders: Vec<String> = f.rows("SELECT root_id FROM project_delete_folder", |r| r.get(0)).await;
        assert_eq!(folders, vec![A.to_string()]);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_path WHERE index_docs = 1").await, 0);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_doc_file").await, 0);

        let after = f.untouched().await;
        let strip = |rows: &[String]| -> Vec<String> { rows.iter().filter(|r| !r.starts_with("[11,")).cloned().collect() };
        assert_eq!(strip(&after), strip(&before), "no text, status, count, date, pair, link, locator or save changed");
        assert_eq!(std::fs::read_to_string(f.workspace("agent-a", A)).unwrap(), workspace);
    }

    #[tokio::test]
    async fn a_delete_keeps_outside_rows_replaced_and_releases_their_disputes() {
        let f = Fixture::new(&[A, B]).await;
        project_with_its_rows(&f).await;
        let code = f.code(A).await;

        f.confirm(A, &code).await.unwrap();

        let outside: Vec<(i64, String, Option<i64>, Option<i64>)> = f
            .rows("SELECT id, status, superseded_by, deleted_at FROM memory WHERE project_id = 'web-bbbb' ORDER BY id", |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .await;
        assert_eq!(
            outside,
            vec![(10, "superseded".to_string(), Some(2), None), (11, "active".to_string(), None, None)],
            "a verdict's replacement stays replaced, and the challenger rests"
        );
        let pairs: Vec<String> = f.rows("SELECT state FROM memory_conflict ORDER BY new_id", |r| r.get(0)).await;
        assert_eq!(pairs, vec!["judged".to_string(), "escalated".to_string()], "the pairs stay as they were");
    }

    #[tokio::test]
    async fn a_stale_code_is_refused_with_the_new_report() {
        let f = Fixture::new(&[A, B]).await;
        project_with_its_rows(&f).await;
        let shown = f.code(A).await;
        f.save("a save that arrives between the two steps", A, 20).await;
        let before = f.everything().await;

        let refused = f.confirm(A, &shown).await.unwrap();

        assert_eq!(refused.refused, Some(Refusal::CodeChanged));
        assert_eq!((refused.delete_id, refused.snapshot.clone()), (None, None));
        let new = refused.code.clone().unwrap();
        assert_ne!(new, shown);
        assert_eq!(refused.live.memories, 3, "the report counts the save");
        assert_eq!(f.everything().await, before, "nothing is written");
        assert!(!f.backups().exists(), "and no snapshot");

        let garbage = f.confirm(A, "nope").await.unwrap();
        assert_eq!((garbage.refused, garbage.code), (Some(Refusal::CodeChanged), Some(new.clone())));
        let done = f.confirm(A, &new.to_uppercase()).await.unwrap();
        assert!(done.delete_id.is_some() && done.refused.is_none(), "the new code deletes, typed in any case");
    }

    #[tokio::test(start_paused = true)]
    async fn a_delete_with_memories_waits_for_the_saves_of_its_project() {
        let f = Fixture::new(&[A, B]).await;
        f.live(1, A).await;
        let code = f.code(A).await;
        f.outbox("op-a", A, "pending").await;
        let before = f.everything().await;

        let refused = f.confirm(A, &code).await;

        assert!(matches!(refused, Err(DeleteError::SavesPending(1))), "{refused:?}");
        assert_eq!(f.everything().await, before);
        assert!(!f.backups().exists(), "no snapshot while a save is in flight");

        f.sql("UPDATE ingest_outbox SET state = 'failed'").await;
        let report = f.confirm(A, &code).await.unwrap();
        assert_eq!((report.refused, report.delete_id), (None, Some(1)));
        assert_eq!(report.failed.len(), 1, "a save that gave up is in the report");
    }

    // ── undoing ─────────────────────────────────────────────────────────

    use crate::memory::embedder::{Embedder, FakeEmbedder};

    impl Fixture {
        async fn undo(&self, delete_id: i64, dry_run: bool, embedder: Option<&dyn Embedder>) -> Result<DeleteReport, DeleteError> {
            undo_delete_core(&self.store, embedder, &self.backups(), delete_id, dry_run).await
        }

        /// What an undo must give back: rows, vectors, text index, project and
        /// folders, without the journal.
        async fn restored_state(&self) -> Vec<String> {
            let mut out = Vec::new();
            for sql in [
                "SELECT json_array('m', id, project_id, status, deleted_at, update_count, last_modified_at, superseded_by, disputed_against) FROM memory",
                "SELECT json_array('t', id, project_id, status, deleted_at, update_count, last_modified_at) FROM episodic_trajectory",
                "SELECT json_array('s', id, project_id, status, deleted_at, update_count, last_modified_at) FROM procedural_skill",
                "SELECT json_array('mv', rowid) FROM memory_vectors",
                "SELECT json_array('ev', rowid) FROM episodic_vectors",
                "SELECT json_array('pv', rowid) FROM procedural_vectors",
                "SELECT json_array('fts', rowid) FROM memory_fts WHERE memory_fts MATCH 'note'",
                "SELECT json_array('p', id, deleted_at, reviewed) FROM project",
                "SELECT json_array('f', id, index_docs) FROM project_path",
                "SELECT json_array('c', new_id, old_id, state) FROM memory_conflict",
            ] {
                out.extend(self.rows(&format!("{sql} ORDER BY 1"), |r| r.get::<_, String>(0)).await);
            }
            out
        }

        fn pre_undo_snapshots(&self) -> usize {
            std::fs::read_dir(self.backups())
                .map(|d| d.filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().contains("pre-undo")).count())
                .unwrap_or(0)
        }
    }

    #[tokio::test]
    async fn undoing_a_delete_leaves_the_store_as_it_was() {
        let f = Fixture::new(&[A, B]).await;
        project_with_its_rows(&f).await;
        f.sql(&format!("UPDATE project SET reviewed = 1 WHERE id = '{A}';")).await;
        let before = f.restored_state().await;
        let code = f.code(A).await;
        let n = f.confirm(A, &code).await.unwrap().delete_id.unwrap();
        let deleted = f.restored_state().await;
        assert_ne!(deleted, before);

        let needs_a_model = f.undo(n, false, None).await;
        assert!(matches!(needs_a_model, Err(DeleteError::NeedsModel)), "{needs_a_model:?}");
        assert_eq!((f.restored_state().await, f.pre_undo_snapshots()), (deleted.clone(), 0), "nothing is written, no snapshot");

        let dry = f.undo(n, true, None).await.unwrap();
        assert!(dry.dry_run && dry.snapshot.is_none(), "a dry run needs no model and takes no snapshot");
        assert_eq!(dry.restored, Counts { memories: 2, trajectories: 1, skills: 1 });
        assert!(dry.kept.is_empty());
        assert_eq!((f.restored_state().await, f.pre_undo_snapshots()), (deleted, 0));

        let embedder = FakeEmbedder::default();
        let undone = f.undo(n, false, Some(&embedder)).await.unwrap();

        assert_eq!((undone.delete_id, undone.restored, undone.folders.clone()), (Some(n), dry.restored, vec![A.to_string()]));
        assert!(undone.kept.is_empty() && !undone.dry_run);
        let snapshot = undone.snapshot.clone().expect("a snapshot before writing");
        assert!(snapshot.contains("pre-undo") && std::path::Path::new(&snapshot).exists(), "{snapshot}");
        // The challenger outside the project was released by the delete and settled by the
        // undo, which stamps its date; its status is what is compared.
        let strip = |rows: Vec<String>| -> Vec<String> { rows.into_iter().filter(|r| !r.starts_with("[\"m\",11,")).collect() };
        assert_eq!(
            strip(f.restored_state().await),
            strip(before),
            "rows, statuses, dates, vectors, text index, disputes, project and folders"
        );
        let challenger: Vec<String> = f.rows("SELECT status FROM memory WHERE id = 11", |r| r.get(0)).await;
        assert_eq!(challenger, vec!["disputed".to_string()], "the dispute is back");
        assert_eq!(f.count("SELECT COUNT(*) FROM project_doc_file").await, 0, "the next refresh rebuilds the docs index");
        let journal: Vec<bool> = f.rows("SELECT undone_at IS NOT NULL FROM project_delete", |r| r.get(0)).await;
        assert_eq!(journal, vec![true]);
        let again = f.undo(n, false, Some(&embedder)).await;
        assert!(matches!(again, Err(DeleteError::Undone(id)) if id == n), "{again:?}");
    }

    #[tokio::test]
    async fn undoing_a_delete_keeps_what_was_saved_since() {
        let f = Fixture::new(&[A, B]).await;
        for id in [1, 2, 3, 5, 6, 8, 9] {
            f.live(id, A).await;
        }
        f.row(4, A, Some("agent-a"), "semantic_fact", "active", Some(4)).await;
        f.row_text(7, A, "connect to postgres://admin:hunter2pass@db.local/app").await;
        f.sql(
            "UPDATE memory SET content_hash = 'h1' WHERE id = 1;
             UPDATE memory SET content_hash = 'h6' WHERE id = 6;
             UPDATE memory SET topic_key = 'k' WHERE id = 2;
             INSERT INTO memory_piece (save_id, piece, memory_id, context) VALUES (100, 0, 5, ''), (100, 1, 6, 'ctx');",
        )
        .await;
        let code = f.code(A).await;
        let n = f.confirm(A, &code).await.unwrap().delete_id.unwrap();

        // Saved since: the text of row 1, the key of row 2, the text of row 6.
        f.live(20, A).await;
        f.live(21, A).await;
        f.live(22, A).await;
        f.sql(
            "UPDATE memory SET content_hash = 'h1' WHERE id = 20;
             UPDATE memory SET topic_key = 'k' WHERE id = 21;
             UPDATE memory SET content_hash = 'h6' WHERE id = 22;
             UPDATE memory SET deleted_at = NULL WHERE id = 8;
             UPDATE memory SET project_id = 'web-bbbb' WHERE id = 9;",
        )
        .await;
        let embedder = FakeEmbedder::default();

        let undone = f.undo(n, false, Some(&embedder)).await.unwrap();

        assert_eq!(undone.restored, Counts { memories: 1, ..Counts::default() });
        let kept: Vec<(i64, KeepReason)> = undone.kept.iter().map(|k| (k.id, k.reason)).collect();
        assert_eq!(
            kept,
            vec![
                (1, KeepReason::LiveTwin),
                (2, KeepReason::TopicTaken),
                (5, KeepReason::Tied),
                (6, KeepReason::LiveTwin),
                (7, KeepReason::HoldsSecret),
                (8, KeepReason::Changed),
                (9, KeepReason::Changed),
            ]
        );
        assert!(undone.kept.iter().all(|k| k.table == "memory"));
        let live: Vec<i64> = f
            .rows("SELECT id FROM memory WHERE project_id = 'app-aaaa' AND deleted_at IS NULL ORDER BY id", |r| r.get(0))
            .await;
        assert_eq!(live, vec![3, 8, 20, 21, 22], "the saves since stay, one old row comes back, the rest stay deleted");
        let deleted_before: Vec<Option<i64>> = f.rows("SELECT deleted_at FROM memory WHERE id = 4", |r| r.get(0)).await;
        assert_eq!(deleted_before, vec![Some(4)], "a row deleted before the delete never comes back");
        assert_eq!(f.count("SELECT COUNT(*) FROM memory_vectors WHERE rowid = 3").await, 1);
        assert_eq!(f.count("SELECT COUNT(*) FROM memory_vectors WHERE rowid IN (1, 2, 5, 6, 7)").await, 0);
        let project: Vec<(Option<i64>, i64)> =
            f.rows(&format!("SELECT deleted_at, reviewed FROM project WHERE id = '{A}'"), |r| Ok((r.get(0)?, r.get(1)?))).await;
        assert_eq!(project, vec![(None, 0)], "a save brought it back unreviewed, and the undo leaves it so");
    }

    #[tokio::test]
    async fn a_delete_followed_by_another_is_undone_after_it() {
        let f = Fixture::new(&[A, B, C]).await;
        f.live(1, A).await;
        let first = f.confirm(A, &f.code(A).await).await.unwrap().delete_id.unwrap();
        f.live(2, A).await;
        let second = f.confirm(A, &f.code(A).await).await.unwrap().delete_id.unwrap();
        assert_eq!((first, second), (1, 2));
        let embedder = FakeEmbedder::default();
        let before = f.restored_state().await;

        let refused = f.undo(first, false, Some(&embedder)).await;

        assert!(matches!(&refused, Err(DeleteError::DeletedLater { project_id, later }) if project_id == A && *later == second), "{refused:?}");
        assert_eq!(f.restored_state().await, before);

        let undone = f.undo(second, false, Some(&embedder)).await.unwrap();
        assert_eq!(undone.restored, Counts { memories: 1, ..Counts::default() });
        assert_eq!(f.count("SELECT COUNT(*) FROM memory WHERE id = 2 AND deleted_at IS NULL").await, 1);
        assert_eq!(f.count("SELECT COUNT(*) FROM memory WHERE id = 1 AND deleted_at IS NULL").await, 0, "the earlier delete is still its own");
        let undone = f.undo(first, false, Some(&embedder)).await.unwrap();
        assert_eq!(undone.restored, Counts { memories: 1, ..Counts::default() });
        assert_eq!(f.count("SELECT COUNT(*) FROM memory WHERE project_id = 'app-aaaa' AND deleted_at IS NULL").await, 2);

        let unknown = f.undo(99, false, Some(&embedder)).await;
        assert!(matches!(unknown, Err(DeleteError::UnknownDelete(99))), "{unknown:?}");
        f.live(3, C).await;
        let purged = f.confirm(C, &f.code(C).await).await.unwrap().delete_id.unwrap();
        f.sql(&format!("UPDATE project_delete SET purged_at = 5 WHERE delete_id = {purged}")).await;
        let gone = f.undo(purged, false, Some(&embedder)).await;
        assert!(matches!(gone, Err(DeleteError::Purged(id)) if id == purged), "{gone:?}");
    }

    #[tokio::test]
    async fn undoing_an_empty_delete_needs_no_embedder() {
        let f = Fixture::new(&[A, B]).await;
        f.sql(&format!("UPDATE project_path SET index_docs = 1 WHERE id = '{A}';")).await;
        f.row(1, A, Some("agent-a"), "semantic_fact", "active", Some(4)).await;
        f.docs(A, 900).await;
        let n = f.delete(A, false).await.unwrap().delete_id.unwrap();
        assert_eq!(f.count("SELECT COUNT(*) FROM project_path WHERE index_docs = 1").await, 0);

        let dry = f.undo(n, true, None).await.unwrap();
        assert!(dry.dry_run && dry.snapshot.is_none() && dry.restored == Counts::default());
        let deleted: Vec<Option<i64>> = f.rows(&format!("SELECT deleted_at FROM project WHERE id = '{A}'"), |r| r.get(0)).await;
        assert!(deleted[0].is_some(), "a dry run writes nothing");

        let undone = f.undo(n, false, None).await.unwrap();

        assert_eq!((undone.restored, undone.folders.clone()), (Counts::default(), vec![A.to_string()]));
        assert!(undone.snapshot.as_deref().is_some_and(|s| s.contains("pre-undo")));
        let state: Vec<Option<i64>> = f.rows(&format!("SELECT deleted_at FROM project WHERE id = '{A}'"), |r| r.get(0)).await;
        assert_eq!(state, vec![None]);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_path WHERE index_docs = 1").await, 1);
        let row: Vec<Option<i64>> = f.rows("SELECT deleted_at FROM memory WHERE id = 1", |r| r.get(0)).await;
        assert_eq!(row, vec![Some(4)], "a row deleted before stays deleted");
    }
}
