//! Merging one project into another. The project merged leaves a tombstone in
//! `project` (`merged_into`), since open bridges keep its id for their whole
//! session and old exports carry it, and every entry that takes a project
//! goes through [`canonical`] to reach the live one.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::memory::backup::Reason;
use crate::memory::clock::TimestampMs;
use crate::memory::project_delete::deleted_in;
use crate::memory::relocate::{merge_in, undo_move_in, MoveError, Span, Supersede};
use crate::memory::workspace::{Adopted, Dropped, WorkspaceManager};
use crate::memory::{MemoryError, MemoryStore};

/// How far [`canonical_in`] follows `merged_into`. A merge points every
/// tombstone at a live project, so a store it wrote needs one hop; the bound
/// is for one edited by hand or restored broken.
pub const MAX_MERGE_HOPS: usize = 8;

/// The live project `id` leads to: the one a tombstone was merged into, or
/// `id` itself for a live project or an id `project` does not know. A cycle,
/// a chain longer than [`MAX_MERGE_HOPS`] or one that ends in no project
/// answers `id` itself, logged, and `doctor` names it.
pub(crate) fn canonical_in(c: &Connection, id: &str) -> rusqlite::Result<String> {
    let mut stmt = c.prepare_cached("SELECT merged_into FROM project WHERE id = ?1")?;
    let mut at = id.to_string();
    let mut seen = HashSet::from([at.clone()]);
    for hops in 0..=MAX_MERGE_HOPS {
        let next = match stmt.query_row([&at], |r| r.get::<_, Option<String>>(0)).optional()? {
            Some(None) => return Ok(at),
            None if hops == 0 => return Ok(at),
            None => {
                log::warn!("project {id} was merged into {at}, which is no project");
                return Ok(id.to_string());
            }
            Some(Some(next)) => next,
        };
        if !seen.insert(next.clone()) {
            log::warn!("the merges of project {id} go round in a cycle through {next}");
            return Ok(id.to_string());
        }
        at = next;
    }
    log::warn!("project {id} is more than {MAX_MERGE_HOPS} merges away from a live project");
    Ok(id.to_string())
}

/// [`canonical_in`] on the store's connection.
pub async fn canonical(store: &MemoryStore, id: &str) -> Result<String, MemoryError> {
    let id = id.to_string();
    store.conn.call(move |c| canonical_in(c, &id)).await.map_err(MemoryError::Connection)
}

/// How long a merge waits for the saves of its two projects still in the
/// outbox: the worker reads a save's project more than once before it stores
/// it, and a merge in between would split what it decides.
pub const SAVES_WAIT: Duration = Duration::from_secs(5);

const SAVES_POLL: Duration = Duration::from_millis(100);

/// Why a merge was refused. Nothing is written.
#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    #[error("no project {0}")]
    UnknownProject(String),
    #[error("project {0} cannot be merged into itself")]
    SameProject(String),
    #[error("project {id} was merged into {into}")]
    Merged { id: String, into: String },
    /// `delete_id` is the open delete that took it.
    #[error("project {id} is deleted")]
    Deleted { id: String, delete_id: Option<i64> },
    #[error("{0} saves of these projects are still being stored; try again in a moment")]
    SavesPending(i64),
    #[error("no merge {0}")]
    UnknownMerge(i64),
    #[error("merge {0} was already undone")]
    Undone(i64),
    #[error("project {target} was merged since, in merge {later}: undo that one first")]
    MergedLater { target: String, later: i64 },
    #[error(transparent)]
    Store(#[from] MemoryError),
}

impl From<rusqlite::Error> for MergeError {
    fn from(e: rusqlite::Error) -> Self {
        MergeError::Store(MemoryError::Sqlite(e))
    }
}

impl From<MoveError> for MergeError {
    fn from(e: MoveError) -> Self {
        match e {
            MoveError::Store(e) => MergeError::Store(e),
            other => MergeError::Store(MemoryError::Internal(other.to_string())),
        }
    }
}

/// The `workspace.md` files of a merge, by path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCopies {
    /// The copies made in the target, or that a dry run would make.
    pub copied: Vec<String>,
    /// Each file of the source left beside the target's own, which differs.
    pub kept_beside: Vec<[String; 2]>,
    /// Why copying stopped, when it did; the merge stands.
    pub failed: Vec<String>,
    /// The copies an undo removed, or would remove.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    /// The copies an undo leaves because the user edited them since.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kept: Vec<String>,
}

impl From<Adopted> for WorkspaceCopies {
    fn from(adopted: Adopted) -> Self {
        WorkspaceCopies {
            copied: adopted.copied.iter().map(|(_, path)| path.display().to_string()).collect(),
            kept_beside: adopted
                .kept_beside
                .iter()
                .map(|(from, to)| [from.display().to_string(), to.display().to_string()])
                .collect(),
            ..WorkspaceCopies::default()
        }
    }
}

impl From<Dropped> for WorkspaceCopies {
    fn from(dropped: Dropped) -> Self {
        let paths = |paths: Vec<std::path::PathBuf>| paths.iter().map(|p| p.display().to_string()).collect();
        WorkspaceCopies { removed: paths(dropped.removed), kept: paths(dropped.kept), ..WorkspaceCopies::default() }
    }
}

/// What a merge does, or would do on a dry run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeReport {
    pub dry_run: bool,
    /// The merge's number in `project_merge`; `None` when nothing was written.
    pub merge_id: Option<i64>,
    pub source: String,
    pub target: String,
    /// The move of its rows of `memory` in `memory_move`.
    pub move_id: Option<i64>,
    /// How many rows of `memory` went to the target.
    pub memories: usize,
    /// What the target keeps live in place of a row of the same key or text.
    pub superseded: Vec<Supersede>,
    /// The first row of each unit whose rows in another project stayed there.
    pub split: Vec<i64>,
    /// How many live preferences and decisions went to the target.
    pub protected: usize,
    pub trajectories: usize,
    pub skills: usize,
    /// How many reviews of `synthesize` went to the target.
    pub reviews: usize,
    /// The folders that lead to the target now, by id.
    pub folders: Vec<String>,
    pub remotes: Vec<String>,
    /// The projects merged before into the source, which lead to the target now.
    pub tombstones: Vec<String>,
    /// How many suggestions it answered `merged`.
    pub suggestions: usize,
    pub workspaces: WorkspaceCopies,
    /// The `pre-merge` snapshot taken before writing, or `pre-undo` for an undo.
    pub snapshot: Option<String>,
    /// The rows of a merge its undo leaves where they are: moved elsewhere
    /// since, or refused by the rules of a move.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kept: Vec<i64>,
}

/// One row of `project_merge_change`: something a merge changed, with what
/// it was before.
struct Change {
    kind: &'static str,
    a: String,
    b: Option<String>,
}

fn change(kind: &'static str, a: impl Into<String>, b: Option<String>) -> Change {
    Change { kind, a: a.into(), b }
}

fn texts_in(c: &Connection, sql: &str, id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = c.prepare(sql)?;
    let rows = stmt.query_map([id], |r| r.get(0))?.collect();
    rows
}

/// Points the rows of `table` whose `column` is `from` at `to`, and returns
/// their `key`s.
fn repoint_in(c: &Connection, table: &str, key: &str, column: &str, from: &str, to: &str) -> rusqlite::Result<Vec<String>> {
    let keys = texts_in(c, &format!("SELECT {key} FROM {table} WHERE {column} = ?1 ORDER BY {key}"), from)?;
    c.execute(&format!("UPDATE {table} SET {column} = ?2 WHERE {column} = ?1"), [from, to])?;
    Ok(keys)
}

/// Merges `source` into `target` inside the caller's transaction and
/// journals it with `snapshot`. A source already merged into `target` is
/// repaired: what is still under it moves, and it stays the tombstone it was.
/// A report without `merge_id` changed nothing.
fn merge_pass(
    c: &Connection,
    source: &str,
    target: &str,
    snapshot: Option<&str>,
    now: TimestampMs,
) -> Result<MergeReport, MergeError> {
    if source == target {
        return Err(MergeError::SameProject(source.to_string()));
    }
    let merged_into = |id: &str| -> Result<Option<String>, MergeError> {
        c.query_row("SELECT merged_into FROM project WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| MergeError::UnknownProject(id.to_string()))
    };
    let source_into = merged_into(source)?;
    if let Some(into) = merged_into(target)? {
        return Err(MergeError::Merged { id: target.to_string(), into });
    }
    let repair = match source_into {
        None => false,
        Some(into) if into == target => true,
        Some(into) => return Err(MergeError::Merged { id: source.to_string(), into }),
    };
    for id in [source, target] {
        if let Some(deleted) = deleted_in(c, id)? {
            return Err(MergeError::Deleted { id: id.to_string(), delete_id: deleted.delete_id });
        }
    }
    let pending: i64 = c.query_row(
        "SELECT COUNT(*) FROM ingest_outbox \
         WHERE state = 'pending' AND json_extract(event, '$.project_id') IN (?1, ?2)",
        [source, target],
        |r| r.get(0),
    )?;
    if pending > 0 {
        return Err(MergeError::SavesPending(pending));
    }

    let moved = merge_in(c, &[source.to_string()], target, Span::Carry, now)?;
    let mut report = MergeReport {
        source: source.to_string(),
        target: target.to_string(),
        move_id: moved.move_id,
        memories: moved.moved.len(),
        superseded: moved.superseded,
        split: moved.split,
        protected: moved.protected,
        ..MergeReport::default()
    };
    let mut changes = Vec::new();
    for (table, kind) in [("episodic_trajectory", "trajectory"), ("procedural_skill", "skill")] {
        let rows = texts_in(c, &format!("SELECT CAST(id AS TEXT) FROM {table} WHERE project_id = ?1 ORDER BY id"), source)?;
        c.execute(&format!("UPDATE {table} SET project_id = ?2 WHERE project_id = ?1"), [source, target])?;
        match kind {
            "trajectory" => report.trajectories = rows.len(),
            _ => report.skills = rows.len(),
        }
        changes.extend(rows.into_iter().map(|id| change(kind, id, None)));
    }

    let reviews: Vec<(String, i64)> = {
        let mut stmt = c.prepare("SELECT agent_id, reviewed_at FROM synthesis_review WHERE scope_key = ?1 ORDER BY agent_id")?;
        let rows = stmt.query_map([source], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (agent, at) in &reviews {
        let had: Option<i64> = c
            .query_row(
                "SELECT reviewed_at FROM synthesis_review WHERE agent_id = ?1 AND scope_key = ?2",
                [agent, target],
                |r| r.get(0),
            )
            .optional()?;
        // The older review: one memory of either side left uncounted would
        // never remind anyone to synthesize it.
        c.execute(
            "INSERT INTO synthesis_review (agent_id, scope_key, reviewed_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT (agent_id, scope_key) DO UPDATE SET reviewed_at = MIN(reviewed_at, excluded.reviewed_at)",
            rusqlite::params![agent, target, at],
        )?;
        changes.push(change("review", agent.as_str(), Some(at.to_string())));
        changes.push(change("review_target", agent.as_str(), had.map(|h| h.to_string())));
    }
    c.execute("DELETE FROM synthesis_review WHERE scope_key = ?1", [source])?;
    report.reviews = reviews.len();

    report.folders = repoint_in(c, "project_path", "id", "project_id", source, target)?;
    report.remotes = repoint_in(c, "project_remote", "remote", "project_id", source, target)?;
    report.tombstones = repoint_in(c, "project", "id", "merged_into", source, target)?;
    changes.extend(report.folders.iter().map(|id| change("folder", id.as_str(), None)));
    changes.extend(report.remotes.iter().map(|remote| change("remote", remote.as_str(), None)));
    changes.extend(report.tombstones.iter().map(|id| change("tombstone", id.as_str(), None)));

    if !repair {
        c.execute(
            "UPDATE project SET merged_into = ?2, merged_at = ?3 WHERE id = ?1",
            rusqlite::params![source, target, now],
        )?;
        changes.push(change("tombstoned", source, None));
    }

    let open: Vec<(i64, String, String)> = {
        let mut stmt =
            c.prepare("SELECT rowid, project_id, other_id FROM project_suggestion WHERE answer IS NULL ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (rowid, a, b) in open {
        if canonical_in(c, &a)? == canonical_in(c, &b)? {
            c.execute(
                "UPDATE project_suggestion SET answer = 'merged', answered_at = ?2 WHERE rowid = ?1",
                rusqlite::params![rowid, now],
            )?;
            changes.push(change("suggestion", rowid.to_string(), None));
            report.suggestions += 1;
        }
    }

    if report.move_id.is_none() && changes.is_empty() {
        return Ok(report);
    }
    let reviewed: bool = c.query_row("SELECT reviewed FROM project WHERE id = ?1", [target], |r| r.get(0))?;
    if !reviewed {
        c.execute("UPDATE project SET reviewed = 1 WHERE id = ?1", [target])?;
        changes.push(change("reviewed", target, Some("0".to_string())));
    }
    c.execute(
        "INSERT INTO project_merge (source, target, move_id, snapshot, merged_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![source, target, report.move_id, snapshot, now],
    )?;
    let merge_id = c.last_insert_rowid();
    let mut log = c.prepare_cached("INSERT INTO project_merge_change (merge_id, kind, a, b) VALUES (?1, ?2, ?3, ?4)")?;
    for ch in &changes {
        log.execute(rusqlite::params![merge_id, ch.kind, ch.a, ch.b])?;
    }
    report.merge_id = Some(merge_id);
    Ok(report)
}

/// What [`retrying`] needs of the error of a pass: to tell the wait it
/// repeats on from the rest, and to take a failure of the connection.
pub(crate) trait Retried: From<rusqlite::Error> + Send + 'static {
    fn saves_pending(&self) -> bool;
    fn connection(e: tokio_rusqlite::Error<Self>) -> Self;
}

impl Retried for MergeError {
    fn saves_pending(&self) -> bool {
        matches!(self, MergeError::SavesPending(_))
    }

    fn connection(e: tokio_rusqlite::Error<MergeError>) -> MergeError {
        match e {
            tokio_rusqlite::Error::Error(e) => e,
            other => MergeError::Store(MemoryError::Backend(other.to_string())),
        }
    }
}

/// Runs `run` in a transaction it commits only when `write`, and again while
/// saves of the projects it touches are pending, for up to [`SAVES_WAIT`].
pub(crate) async fn retrying<T, E, F>(store: &MemoryStore, write: bool, run: F) -> Result<T, E>
where
    T: Send + 'static,
    E: Retried,
    F: Fn(&Connection) -> Result<T, E> + Clone + Send + 'static,
{
    let deadline = tokio::time::Instant::now() + SAVES_WAIT;
    loop {
        let run = run.clone();
        let result = store
            .conn
            .call(move |c| -> Result<T, E> {
                let tx = c.transaction()?;
                let out = run(&tx)?;
                if write {
                    tx.commit()?;
                }
                Ok(out)
            })
            .await
            .map_err(E::connection);
        match result {
            Err(e) if e.saves_pending() && tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(SAVES_POLL).await;
            }
            other => return other,
        }
    }
}

async fn pass(
    store: &MemoryStore,
    source: &str,
    target: &str,
    snapshot: Option<String>,
    write: bool,
) -> Result<MergeReport, MergeError> {
    let (source, target) = (source.to_string(), target.to_string());
    retrying(store, write, move |c| merge_pass(c, &source, &target, snapshot.as_deref(), TimestampMs::now())).await
}

/// Takes merge `merge_id` back inside the caller's transaction: its source
/// is live again, and what it moved and still points at its target goes
/// back, rows of `memory` as a move of their own. Returns the report and the
/// agent folders whose `workspace.md` it copied.
fn undo_pass(c: &Connection, merge_id: i64, now: TimestampMs) -> Result<(MergeReport, Vec<String>), MergeError> {
    let (source, target, move_id, undone_at): (String, String, Option<i64>, Option<i64>) = c
        .query_row(
            "SELECT source, target, move_id, undone_at FROM project_merge WHERE merge_id = ?1",
            [merge_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or(MergeError::UnknownMerge(merge_id))?;
    if undone_at.is_some() {
        return Err(MergeError::Undone(merge_id));
    }
    let target_into: Option<String> = c
        .query_row("SELECT merged_into FROM project WHERE id = ?1", [&target], |r| r.get(0))
        .optional()?
        .flatten();
    if let Some(into) = target_into {
        let later: Option<i64> = c
            .query_row(
                "SELECT merge_id FROM project_merge WHERE source = ?1 AND undone_at IS NULL ORDER BY merge_id DESC LIMIT 1",
                [&target],
                |r| r.get(0),
            )
            .optional()?;
        return Err(match later {
            Some(later) => MergeError::MergedLater { target, later },
            None => MergeError::Merged { id: target, into },
        });
    }
    // Its rows would go back to the source while the delete journaled them in the target.
    if let Some(deleted) = deleted_in(c, &target)? {
        return Err(MergeError::Deleted { id: target, delete_id: deleted.delete_id });
    }
    let pending: i64 = c.query_row(
        "SELECT COUNT(*) FROM ingest_outbox \
         WHERE state = 'pending' AND json_extract(event, '$.project_id') IN (?1, ?2)",
        [&source, &target],
        |r| r.get(0),
    )?;
    if pending > 0 {
        return Err(MergeError::SavesPending(pending));
    }
    let changes: Vec<(String, String, Option<String>)> = {
        let mut stmt = c.prepare("SELECT kind, a, b FROM project_merge_change WHERE merge_id = ?1 ORDER BY rowid")?;
        let rows = stmt.query_map([merge_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut report = MergeReport {
        merge_id: Some(merge_id),
        source: source.clone(),
        target: target.clone(),
        ..MergeReport::default()
    };

    // The source is live again first, so its rows go back to a project.
    if changes.iter().any(|(kind, _, _)| kind == "tombstoned") {
        c.execute(
            "UPDATE project SET merged_into = NULL, merged_at = NULL WHERE id = ?1 AND merged_into = ?2",
            [&source, &target],
        )?;
    }
    if let Some(move_id) = move_id {
        let elsewhere: Vec<i64> = {
            let mut stmt = c.prepare(
                "SELECT m.row_id FROM memory_move m JOIN memory r ON r.id = m.row_id \
                 WHERE m.move_id = ?1 AND r.project_id IS NOT m.to_project ORDER BY m.row_id",
            )?;
            let rows = stmt.query_map([move_id], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let back = undo_move_in(c, move_id, true)?;
        report.memories = back.moved.len();
        report.move_id = back.move_id;
        report.kept = elsewhere;
        report.kept.extend(back.skipped.iter().map(|s| s.id));
        report.kept.sort_unstable();
    }

    let reviewed_at = |b: &Option<String>| b.as_deref().and_then(|b| b.parse::<i64>().ok());
    let mut agents = Vec::new();
    for (kind, a, b) in &changes {
        let back = |sql: &str| c.execute(sql, [a.as_str(), source.as_str(), target.as_str()]);
        match kind.as_str() {
            "trajectory" => {
                report.trajectories +=
                    back("UPDATE episodic_trajectory SET project_id = ?2 WHERE id = ?1 AND project_id = ?3")?;
            }
            "skill" => {
                report.skills += back("UPDATE procedural_skill SET project_id = ?2 WHERE id = ?1 AND project_id = ?3")?;
            }
            "folder" => {
                if back("UPDATE project_path SET project_id = ?2 WHERE id = ?1 AND project_id = ?3")? > 0 {
                    report.folders.push(a.clone());
                }
            }
            "remote" => {
                if back("UPDATE project_remote SET project_id = ?2 WHERE remote = ?1 AND project_id = ?3")? > 0 {
                    report.remotes.push(a.clone());
                }
            }
            "tombstone" => {
                if back("UPDATE project SET merged_into = ?2 WHERE id = ?1 AND merged_into = ?3")? > 0 {
                    report.tombstones.push(a.clone());
                }
            }
            "review" => {
                report.reviews += c.execute(
                    "INSERT OR IGNORE INTO synthesis_review (agent_id, scope_key, reviewed_at) VALUES (?1, ?2, ?3)",
                    rusqlite::params![a, source, reviewed_at(b)],
                )?;
            }
            "review_target" => {
                let theirs = changes
                    .iter()
                    .find(|(k, agent, _)| k == "review" && agent == a)
                    .and_then(|(_, _, at)| reviewed_at(at));
                let before = reviewed_at(b);
                let written = match (before, theirs) {
                    (Some(before), Some(theirs)) => Some(before.min(theirs)),
                    (before, theirs) => before.or(theirs),
                };
                let now_at: Option<i64> = c
                    .query_row(
                        "SELECT reviewed_at FROM synthesis_review WHERE agent_id = ?1 AND scope_key = ?2",
                        [a, &target],
                        |r| r.get(0),
                    )
                    .optional()?;
                // A synthesis of the target since the merge keeps its review.
                if now_at.is_some() && now_at == written {
                    match before {
                        Some(before) => c.execute(
                            "UPDATE synthesis_review SET reviewed_at = ?3 WHERE agent_id = ?1 AND scope_key = ?2",
                            rusqlite::params![a, target, before],
                        )?,
                        None => c.execute(
                            "DELETE FROM synthesis_review WHERE agent_id = ?1 AND scope_key = ?2",
                            [a, &target],
                        )?,
                    };
                }
            }
            "suggestion" => {
                report.suggestions += c.execute(
                    "UPDATE project_suggestion SET answer = NULL, answered_at = NULL WHERE rowid = ?1 AND answer = 'merged'",
                    [a],
                )?;
            }
            "reviewed" => {
                c.execute("UPDATE project SET reviewed = ?2 WHERE id = ?1", rusqlite::params![a, b])?;
            }
            "workspace" => agents.push(a.clone()),
            "tombstoned" => {}
            other => log::warn!("merge {merge_id} journaled a change of kind {other}, which no undo knows"),
        }
    }
    c.execute(
        "UPDATE project_merge SET undone_at = ?2, undo_move_id = ?3 WHERE merge_id = ?1",
        rusqlite::params![merge_id, now, report.move_id],
    )?;
    Ok((report, agents))
}

/// Takes merge `merge_id` back, after a `pre-undo` snapshot into `backups`:
/// its source is a project again with what the merge moved and still points
/// at its target, and each `workspace.md` copy it made is removed unless the
/// user edited it since. What was saved in the target after the merge stays.
/// A merge whose target was merged since waits for that merge to be undone.
/// A dry run writes nothing and touches no file.
pub async fn undo_merge_core(
    store: &MemoryStore,
    workspaces: &WorkspaceManager,
    backups: &Path,
    merge_id: i64,
    dry_run: bool,
) -> Result<MergeReport, MergeError> {
    let run = move |c: &Connection| undo_pass(c, merge_id, TimestampMs::now());
    let (found, agents) = retrying(store, false, run).await?;
    let drop = |write| workspaces.drop_adopted(&found.source, &found.target, &agents, write);
    if dry_run {
        let workspaces = match drop(false).await {
            Ok(dropped) => dropped.into(),
            Err(e) => WorkspaceCopies { failed: vec![e.to_string()], ..WorkspaceCopies::default() },
        };
        return Ok(MergeReport { dry_run, move_id: None, workspaces, ..found });
    }
    let snapshot = store.snapshot(backups.to_path_buf(), Reason::PreUndo).await?.display().to_string();
    let (mut report, agents) = retrying(store, true, run).await?;
    report.snapshot = Some(snapshot);
    match workspaces.drop_adopted(&report.source, &report.target, &agents, true).await {
        Ok(dropped) => report.workspaces = dropped.into(),
        Err(e) => {
            log::warn!("undoing merge {merge_id} could not take back its workspace copies: {e}");
            report.workspaces.failed.push(e.to_string());
        }
    }
    Ok(report)
}

async fn record_copies(store: &MemoryStore, merge_id: i64, agents: Vec<String>) -> Result<(), MemoryError> {
    store
        .conn
        .call(move |c| {
            let tx = c.transaction()?;
            for agent in &agents {
                tx.execute(
                    "INSERT INTO project_merge_change (merge_id, kind, a, b) VALUES (?1, 'workspace', ?2, NULL)",
                    rusqlite::params![merge_id, agent],
                )?;
            }
            tx.commit()
        })
        .await
        .map_err(MemoryError::Connection)
}

/// Merges `source` into `target`: everything under `source` moves there and
/// `source` is left a tombstone, after a `pre-merge` snapshot into `backups`,
/// in one transaction that decides again what a first pass, which writes
/// nothing, found. Then each agent's `workspace.md` of `source` is copied
/// where `target` has none, and the copies are journaled. A dry run stops
/// after the first pass and touches no file.
pub async fn merge_core(
    store: &MemoryStore,
    workspaces: &WorkspaceManager,
    backups: &Path,
    source: &str,
    target: &str,
    dry_run: bool,
) -> Result<MergeReport, MergeError> {
    let found = pass(store, source, target, None, false).await?;
    if found.merge_id.is_none() {
        return Ok(MergeReport { dry_run, ..found });
    }
    if dry_run {
        let workspaces = match workspaces.adopt_legacy(source, target, false).await {
            Ok(adopted) => adopted.into(),
            Err(e) => WorkspaceCopies { failed: vec![e.to_string()], ..WorkspaceCopies::default() },
        };
        return Ok(MergeReport { dry_run, merge_id: None, move_id: None, workspaces, ..found });
    }
    let snapshot = store.snapshot(backups.to_path_buf(), Reason::PreMerge).await?.display().to_string();
    let mut report = pass(store, source, target, Some(snapshot.clone()), true).await?;
    report.snapshot = Some(snapshot);
    let Some(merge_id) = report.merge_id else {
        return Ok(report);
    };
    match workspaces.adopt_legacy(source, target, true).await {
        Ok(adopted) => {
            let agents: Vec<String> = adopted.copied.iter().map(|(agent, _)| agent.clone()).collect();
            report.workspaces = adopted.into();
            if let Err(e) = record_copies(store, merge_id, agents).await {
                log::warn!("merge {merge_id} copied workspaces it could not journal: {e}");
            }
        }
        Err(e) => {
            log::warn!("merge {merge_id} could not copy the workspaces of {source}: {e}");
            report.workspaces.failed.push(e.to_string());
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryStore;

    async fn store_with(projects: &[(&'static str, Option<&'static str>)]) -> MemoryStore {
        let store = MemoryStore::new_for_test().await.unwrap();
        let projects: Vec<_> = projects.to_vec();
        store
            .conn
            .call(move |c| {
                for (id, merged_into) in &projects {
                    c.execute(
                        "INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) \
                         VALUES (?1, ?1, 'root', 0, 1, ?2, CASE WHEN ?2 IS NULL THEN NULL ELSE 2 END)",
                        rusqlite::params![id, merged_into],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .unwrap();
        store
    }

    #[tokio::test]
    async fn canonical_follows_a_merge_and_leaves_any_other_id_as_it_is() {
        let store = store_with(&[("app-1a2b", Some("web-3c4d")), ("web-3c4d", None)]).await;
        assert_eq!(canonical(&store, "app-1a2b").await.unwrap(), "web-3c4d", "a tombstone leads to its project");
        assert_eq!(canonical(&store, "web-3c4d").await.unwrap(), "web-3c4d", "a live project is itself");
        assert_eq!(canonical(&store, "nowhere-9f9f").await.unwrap(), "nowhere-9f9f", "an unknown id is itself");
    }

    #[tokio::test]
    async fn a_cycle_or_a_chain_too_long_answers_the_id_itself() {
        let store = store_with(&[("a", Some("b")), ("b", Some("a")), ("lost", Some("gone"))]).await;
        assert_eq!(canonical(&store, "a").await.unwrap(), "a", "a cycle");
        assert_eq!(canonical(&store, "lost").await.unwrap(), "lost", "a chain that ends in no project");

        let names: Vec<String> = (0..=MAX_MERGE_HOPS + 1).map(|n| format!("p{n}")).collect();
        let starts: Vec<String> = names[..2].to_vec();
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .conn
            .call(move |c| {
                for (n, id) in names.iter().enumerate() {
                    c.execute(
                        "INSERT INTO project (id, name, born, reviewed, created_at, merged_into) VALUES (?1, ?1, 'root', 0, 1, ?2)",
                        rusqlite::params![id, names.get(n + 1)],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .unwrap();
        assert_eq!(
            canonical(&store, &starts[1]).await.unwrap(),
            format!("p{}", MAX_MERGE_HOPS + 1),
            "a chain of MAX_MERGE_HOPS merges still ends in its project"
        );
        assert_eq!(canonical(&store, &starts[0]).await.unwrap(), "p0", "one merge more is too long");
    }

    // ── merging ─────────────────────────────────────────────────────────

    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use crate::memory::events::{MemoryEvent, MemorySource};
    use crate::memory::relocate::Supersede;
    use crate::memory::workspace::WorkspaceManager;
    use crate::memory::worker::save_for_test;
    use crate::models::chat::Role;

    const A: &str = "app-aaaa";
    const B: &str = "web-bbbb";
    const C: &str = "api-cccc";

    struct Fixture {
        store: Arc<MemoryStore>,
        workspaces: WorkspaceManager,
        backups: PathBuf,
        _dirs: (tempfile::TempDir, tempfile::TempDir),
    }

    impl Fixture {
        async fn new(projects: &[&str]) -> Self {
            let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
            let (workspaces, backups) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            let fixture = Fixture {
                store,
                workspaces: WorkspaceManager::new(workspaces.path().to_path_buf()),
                backups: backups.path().join("backups"),
                _dirs: (workspaces, backups),
            };
            for id in projects {
                fixture.sql(&format!(
                    "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('{id}', '{id}', 'root', 0, 1);
                     INSERT INTO project_path (id, root_path, created_at, index_docs, project_id) \
                       VALUES ('{id}', '/work/{id}', 1, 0, '{id}');"
                ))
                .await;
            }
            fixture
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

        async fn save(&self, content: &str, project: &str, key: Option<&str>, at: u64) -> i64 {
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
                topic_key: key.map(str::to_string),
                user_stated: false,
                session_id: None,
            };
            let report = save_for_test(&self.store, &evt).await;
            assert_eq!(report.failure, None);
            report.memories[0].id
        }

        async fn merge(&self, source: &str, target: &str, dry_run: bool) -> Result<MergeReport, MergeError> {
            merge_core(&self.store, &self.workspaces, &self.backups, source, target, dry_run).await
        }

        fn workspace(&self, agent: &str, project: &str) -> PathBuf {
            self.workspaces.workspace_path(agent, Some(project))
        }

        fn write_workspace(&self, agent: &str, project: &str, user: &str) -> String {
            let text = format!("<!-- AUTO-START -->\n<!-- AUTO-END -->\n<!-- USER-START -->\n{user}\n<!-- USER-END -->\n");
            let path = self.workspace(agent, project);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &text).unwrap();
            text
        }

        /// Every row a merge may touch, as text, to compare before and after.
        async fn everything(&self) -> Vec<String> {
            let mut out = Vec::new();
            for sql in [
                "SELECT json_array(id, project_id, scope, status, superseded_by, supersede_cause) FROM memory",
                "SELECT json_array('trajectory', id, project_id) FROM episodic_trajectory",
                "SELECT json_array('skill', id, project_id) FROM procedural_skill",
                "SELECT json_array(id, name, reviewed, merged_into, merged_at) FROM project",
                "SELECT json_array(id, project_id) FROM project_path",
                "SELECT json_array(remote, project_id) FROM project_remote",
                "SELECT json_array(project_id, other_id, answer) FROM project_suggestion",
                "SELECT json_array(agent_id, scope_key, reviewed_at) FROM synthesis_review",
                "SELECT json_array('merge', merge_id) FROM project_merge",
                "SELECT json_array('move', move_id, row_id) FROM memory_move",
            ] {
                out.extend(self.rows(&format!("{sql} ORDER BY 1"), |r| r.get::<_, String>(0)).await);
            }
            out
        }
    }

    fn ids(rows: &[(i64, Option<String>)]) -> Vec<i64> {
        rows.iter().map(|r| r.0).collect()
    }

    #[tokio::test]
    async fn merging_moves_every_row_and_locator_into_the_target() {
        let f = Fixture::new(&[A, B]).await;
        let note = f.save("the daemon listens on clawg.sock", A, None, 1).await;
        f.sql(&format!(
            "INSERT INTO episodic_trajectory \
               (scope, agent_id, project_id, session_id, task, created_at, last_modified_at, last_accessed_at) \
               VALUES ('agent', 'agent-a', '{A}', 's', 't', 1, 1, 1);
             INSERT INTO procedural_skill \
               (scope, agent_id, project_id, skill_name, trigger, promoted_at, \
                source_trajectory_ids, created_at, last_modified_at, last_accessed_at) \
               VALUES ('agent', 'agent-a', '{A}', 'k', 't', 1, '[]', 1, 1, 1);
             INSERT INTO synthesis_review (agent_id, scope_key, reviewed_at) VALUES ('', '{A}', 10), ('', '{B}', 20);
             INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/app', '{A}', 1);
             INSERT INTO project_suggestion (project_id, other_id, reason, created_at) VALUES ('{B}', '{A}', 'root_commit', 1);"
        ))
        .await;
        let workspace = f.write_workspace("agent-a", A, "a note of the user");

        let report = f.merge(A, B, false).await.unwrap();

        assert_eq!((report.source.as_str(), report.target.as_str(), report.dry_run), (A, B, false));
        assert_eq!(report.merge_id, Some(1));
        assert!(report.move_id.is_some());
        assert_eq!(
            (report.memories, report.trajectories, report.skills, report.reviews, report.suggestions),
            (1, 1, 1, 1, 1)
        );
        assert_eq!((report.folders.clone(), report.remotes.clone()), (vec![A.to_string()], vec!["github.com/acme/app".to_string()]));
        let snapshot = report.snapshot.clone().expect("a snapshot before writing");
        assert!(snapshot.contains("pre-merge") && Path::new(&snapshot).exists(), "{snapshot}");

        let rows = f.rows("SELECT id, project_id FROM memory WHERE id > 0 ORDER BY id", |r| Ok((r.get(0)?, r.get(1)?))).await;
        assert_eq!(ids(&rows), vec![note]);
        assert!(rows.iter().all(|r| r.1.as_deref() == Some(B)), "{rows:?}");
        let other: Vec<String> = f
            .rows(
                "SELECT project_id FROM episodic_trajectory UNION ALL SELECT project_id FROM procedural_skill \
                 UNION ALL SELECT project_id FROM project_path WHERE id = 'app-aaaa' \
                 UNION ALL SELECT project_id FROM project_remote",
                |r| r.get(0),
            )
            .await;
        assert_eq!(other, vec![B; 4]);
        let reviews: Vec<(String, i64)> =
            f.rows("SELECT scope_key, reviewed_at FROM synthesis_review", |r| Ok((r.get(0)?, r.get(1)?))).await;
        assert_eq!(reviews, vec![(B.to_string(), 10)], "the older review of the two");
        let projects: Vec<(String, i64, Option<String>, bool)> = f
            .rows("SELECT id, reviewed, merged_into, merged_at IS NOT NULL FROM project ORDER BY id", |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .await;
        assert_eq!(projects, vec![(A.to_string(), 0, Some(B.to_string()), true), (B.to_string(), 1, None, false)]);
        let answers: Vec<Option<String>> = f.rows("SELECT answer FROM project_suggestion", |r| r.get(0)).await;
        assert_eq!(answers, vec![Some("merged".to_string())]);
        assert_eq!(canonical(&f.store, A).await.unwrap(), B);

        assert_eq!(std::fs::read_to_string(f.workspace("agent-a", A)).unwrap(), workspace, "the source's file is never changed");
        assert_eq!(std::fs::read_to_string(f.workspace("agent-a", B)).unwrap(), workspace, "the target had none");
        assert_eq!(report.workspaces.copied, vec![f.workspace("agent-a", B).display().to_string()]);
    }

    #[tokio::test]
    async fn a_merge_keeps_the_newest_save_of_a_key_and_the_oldest_of_a_text() {
        let f = Fixture::new(&[A, B]).await;
        let older = f.save("Storage engine is SQLite", A, Some("storage/engine"), 10).await;
        let newer = f.save("Storage engine is SQLite with WAL", B, Some("storage/engine"), 20).await;
        let first = f.save("the socket is clawg.sock", A, None, 5).await;
        let second = f.save("the socket is clawg.sock", B, None, 9).await;

        let report = f.merge(A, B, false).await.unwrap();

        assert_eq!(report.superseded, vec![Supersede { row: older, by: newer }, Supersede { row: second, by: first }]);
        let live: Vec<i64> = f
            .rows("SELECT id FROM memory WHERE status = 'active' AND id > 0 ORDER BY id", |r| r.get(0))
            .await;
        assert_eq!(live, vec![newer, first]);
    }

    #[tokio::test]
    async fn a_merge_moves_the_part_of_a_split_unit_it_holds() {
        let f = Fixture::new(&[A, B, C]).await;
        let old = f.save("Logs go to serve.log", A, Some("logging/target"), 10).await;
        let new = f.save("Logs go to serve.log, rotated", A, Some("logging/target"), 20).await;
        // Split before units moved whole: the older version went to C alone.
        f.sql(&format!("UPDATE memory SET project_id = '{C}' WHERE id = {old}")).await;

        let report = f.merge(A, B, false).await.unwrap();

        assert_eq!(report.split, vec![new]);
        let places: Vec<(i64, Option<String>)> =
            f.rows("SELECT id, project_id FROM memory WHERE id > 0 ORDER BY id", |r| Ok((r.get(0)?, r.get(1)?))).await;
        assert_eq!(places, vec![(old, Some(C.to_string())), (new, Some(B.to_string()))], "nothing is left under the tombstone");
    }

    #[tokio::test]
    async fn merging_a_target_repoints_its_tombstones() {
        let f = Fixture::new(&[A, B, C]).await;
        f.merge(A, B, false).await.unwrap();

        let report = f.merge(B, C, false).await.unwrap();

        assert_eq!(report.tombstones, vec![A.to_string()]);
        let merged: Vec<(String, Option<String>)> =
            f.rows("SELECT id, merged_into FROM project ORDER BY id", |r| Ok((r.get(0)?, r.get(1)?))).await;
        assert_eq!(merged, vec![(C.to_string(), None), (A.to_string(), Some(C.to_string())), (B.to_string(), Some(C.to_string()))]);
        assert_eq!(canonical(&f.store, A).await.unwrap(), C);
    }

    #[tokio::test]
    async fn a_dry_run_merge_changes_nothing() {
        let f = Fixture::new(&[A, B]).await;
        f.save("the daemon listens on clawg.sock", A, None, 1).await;
        f.sql(&format!("INSERT INTO synthesis_review (agent_id, scope_key, reviewed_at) VALUES ('', '{A}', 10);")).await;
        f.write_workspace("agent-a", A, "a note of the user");
        let before = f.everything().await;

        let report = f.merge(A, B, true).await.unwrap();

        assert!(report.dry_run);
        assert_eq!((report.merge_id, report.move_id, report.snapshot.clone()), (None, None, None));
        assert_eq!((report.memories, report.reviews, report.folders.clone()), (1, 1, vec![A.to_string()]));
        assert_eq!(report.workspaces.copied, vec![f.workspace("agent-a", B).display().to_string()]);
        assert_eq!(f.everything().await, before);
        assert!(!f.workspace("agent-a", B).exists());
        assert!(!f.backups.exists(), "no snapshot");
    }

    #[tokio::test]
    async fn a_merge_copies_a_workspace_only_where_the_target_has_none() {
        let f = Fixture::new(&[A, B]).await;
        let only_source = f.write_workspace("agent-a", A, "kept by agent a");
        let source = f.write_workspace("agent-b", A, "the source's notes");
        let target = f.write_workspace("agent-b", B, "the target's notes");

        let report = f.merge(A, B, false).await.unwrap();

        assert_eq!(report.workspaces.copied, vec![f.workspace("agent-a", B).display().to_string()]);
        assert_eq!(
            report.workspaces.kept_beside,
            vec![[f.workspace("agent-b", A).display().to_string(), f.workspace("agent-b", B).display().to_string()]]
        );
        assert_eq!(std::fs::read_to_string(f.workspace("agent-a", B)).unwrap(), only_source);
        assert_eq!(std::fs::read_to_string(f.workspace("agent-b", A)).unwrap(), source);
        assert_eq!(std::fs::read_to_string(f.workspace("agent-b", B)).unwrap(), target);
        let journal: Vec<(String, String)> = f
            .rows("SELECT kind, a FROM project_merge_change WHERE kind = 'workspace'", |r| Ok((r.get(0)?, r.get(1)?)))
            .await;
        assert_eq!(journal, vec![("workspace".to_string(), "agent-a".to_string())], "only the copy made is journaled");
    }

    #[tokio::test(start_paused = true)]
    async fn a_merge_is_refused_while_a_save_of_its_projects_is_pending() {
        let f = Fixture::new(&[A, B]).await;
        for (operation, project) in [("op-a", A), ("op-b", B)] {
            let event = format!(r#"{{"agent_id":"agent-a","project_id":"{project}","content":""}}"#);
            f.sql(&format!(
                "INSERT INTO ingest_outbox (operation_id, agent_id, content, event, created_at, updated_at) \
                 VALUES ('{operation}', 'agent-a', 'a save', '{event}', 1, 1);"
            ))
            .await;
        }
        let before = f.everything().await;

        let refused = f.merge(A, B, false).await;

        assert!(matches!(refused, Err(MergeError::SavesPending(2))), "{refused:?}");
        assert_eq!(f.everything().await, before);
        f.sql("UPDATE ingest_outbox SET state = 'failed'").await;
        assert!(f.merge(A, B, false).await.is_ok(), "a failed save is not in flight");
    }

    #[tokio::test]
    async fn a_merge_of_a_tombstone_into_its_target_repairs_it() {
        let f = Fixture::new(&[A, B]).await;
        f.merge(A, B, false).await.unwrap();
        let merged_at: Vec<i64> = f.rows(&format!("SELECT merged_at FROM project WHERE id = '{A}'"), |r| r.get(0)).await;
        // A row left under the tombstone, as a store edited by hand could hold.
        let left = f.save("a note left behind", B, None, 1).await;
        f.sql(&format!("UPDATE memory SET project_id = '{A}' WHERE id = {left}")).await;

        let report = f.merge(A, B, false).await.unwrap();

        assert_eq!((report.merge_id, report.memories), (Some(2), 1));
        let after: Vec<(Option<String>, i64)> =
            f.rows(&format!("SELECT merged_into, merged_at FROM project WHERE id = '{A}'"), |r| Ok((r.get(0)?, r.get(1)?))).await;
        assert_eq!(after, vec![(Some(B.to_string()), merged_at[0])], "the tombstone stays the one it was");
        let nothing = f.merge(A, B, false).await.unwrap();
        assert_eq!((nothing.merge_id, nothing.snapshot), (None, None), "a repair with nothing to move writes nothing");
    }

    // ── undoing ─────────────────────────────────────────────────────────

    impl Fixture {
        async fn undo(&self, merge_id: i64, dry_run: bool) -> Result<MergeReport, MergeError> {
            undo_merge_core(&self.store, &self.workspaces, &self.backups, merge_id, dry_run).await
        }

        /// [`Fixture::everything`] without the journals, which an undo adds to.
        async fn state(&self) -> Vec<String> {
            let journals = ["[\"merge\"", "[\"move\""];
            self.everything().await.into_iter().filter(|r| !journals.iter().any(|j| r.starts_with(j))).collect()
        }

        /// Two projects with something of each kind a merge moves, and a
        /// third merged into A before.
        async fn two_projects(&self) {
            self.save("Storage engine is SQLite", A, Some("storage/engine"), 10).await;
            self.save("Storage engine is SQLite with WAL", B, Some("storage/engine"), 20).await;
            self.save("the socket is clawg.sock", A, None, 5).await;
            self.save("the socket is clawg.sock", B, None, 9).await;
            let deleted = self.save("an old note", A, None, 3).await;
            self.sql(&format!(
                "UPDATE memory SET deleted_at = 4 WHERE id = {deleted};
                 INSERT INTO episodic_trajectory \
                   (scope, agent_id, project_id, session_id, task, created_at, last_modified_at, last_accessed_at) \
                   VALUES ('agent', 'agent-a', '{A}', 's', 't', 1, 1, 1);
                 INSERT INTO procedural_skill \
                   (scope, agent_id, project_id, skill_name, trigger, promoted_at, \
                    source_trajectory_ids, created_at, last_modified_at, last_accessed_at) \
                   VALUES ('agent', 'agent-a', '{A}', 'k', 't', 1, '[]', 1, 1, 1);
                 INSERT INTO synthesis_review (agent_id, scope_key, reviewed_at) VALUES ('', '{A}', 10), ('', '{B}', 20);
                 INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/app', '{A}', 1);
                 INSERT INTO project_suggestion (project_id, other_id, reason, created_at) VALUES ('{B}', '{A}', 'root_commit', 1);
                 UPDATE project SET merged_into = '{A}', merged_at = 1 WHERE id = '{C}';"
            ))
            .await;
        }
    }

    #[tokio::test]
    async fn undoing_a_merge_leaves_the_store_as_it_was() {
        let f = Fixture::new(&[A, B, C]).await;
        f.two_projects().await;
        let source = f.write_workspace("agent-a", A, "a note of the user");
        let before = f.state().await;
        let merged = f.merge(A, B, false).await.unwrap();
        assert_ne!(f.state().await, before);
        let merged_state = f.everything().await;

        let dry = f.undo(merged.merge_id.unwrap(), true).await.unwrap();
        assert!(dry.dry_run && dry.snapshot.is_none());
        assert_eq!(dry.workspaces.removed, vec![f.workspace("agent-a", B).display().to_string()]);
        assert_eq!(f.everything().await, merged_state, "a dry run changes nothing");
        assert!(f.workspace("agent-a", B).exists());

        let undone = f.undo(merged.merge_id.unwrap(), false).await.unwrap();

        assert_eq!(f.state().await, before);
        assert_eq!((undone.memories, undone.trajectories, undone.skills, undone.reviews), (merged.memories, 1, 1, 1));
        assert_eq!((undone.folders.clone(), undone.remotes.clone(), undone.tombstones.clone()), (merged.folders, merged.remotes, merged.tombstones));
        assert!(undone.snapshot.as_deref().is_some_and(|s| s.contains("pre-undo")), "{:?}", undone.snapshot);
        assert!(undone.move_id.is_some(), "the rows went back as a move of their own");
        assert!(!f.workspace("agent-a", B).exists(), "the copy the merge made is gone");
        assert!(!f.workspace("agent-a", B).parent().unwrap().exists(), "and the folder it made for it");
        assert_eq!(std::fs::read_to_string(f.workspace("agent-a", A)).unwrap(), source);
        assert_eq!(canonical(&f.store, A).await.unwrap(), A);
        assert_eq!(canonical(&f.store, C).await.unwrap(), A);
        let again = f.undo(merged.merge_id.unwrap(), false).await;
        assert!(matches!(again, Err(MergeError::Undone(1))), "{again:?}");
        assert!(matches!(f.undo(9, false).await, Err(MergeError::UnknownMerge(9))));
    }

    #[tokio::test]
    async fn undoing_a_merge_keeps_what_was_saved_after_it() {
        let f = Fixture::new(&[A, B, C]).await;
        let gone = f.save("tests run with cargo nextest", A, None, 1).await;
        let stays = f.save("the daemon listens on clawg.sock", A, None, 2).await;
        f.sql(&format!("INSERT INTO synthesis_review (agent_id, scope_key, reviewed_at) VALUES ('', '{A}', 10), ('', '{B}', 20);"))
            .await;
        let merged = f.merge(A, B, false).await.unwrap();
        let later = f.save("a note saved after the merge", B, None, 30).await;
        f.sql(&format!(
            "UPDATE memory SET project_id = '{C}' WHERE id = {gone};
             UPDATE synthesis_review SET reviewed_at = 40 WHERE scope_key = '{B}';"
        ))
        .await;

        let undone = f.undo(merged.merge_id.unwrap(), false).await.unwrap();

        assert_eq!(undone.kept, vec![gone], "a row moved elsewhere since stays where it is");
        let places: Vec<(i64, Option<String>)> =
            f.rows("SELECT id, project_id FROM memory WHERE id > 0 ORDER BY id", |r| Ok((r.get(0)?, r.get(1)?))).await;
        assert_eq!(
            places,
            vec![(gone, Some(C.to_string())), (stays, Some(A.to_string())), (later, Some(B.to_string()))]
        );
        let reviews: Vec<(String, i64)> = f
            .rows("SELECT scope_key, reviewed_at FROM synthesis_review ORDER BY scope_key", |r| Ok((r.get(0)?, r.get(1)?)))
            .await;
        assert_eq!(reviews, vec![(A.to_string(), 10), (B.to_string(), 40)], "a review written since stays");
    }

    #[tokio::test]
    async fn a_merge_whose_target_was_merged_since_is_undone_after_that_one() {
        let f = Fixture::new(&[A, B, C]).await;
        let first = f.merge(A, B, false).await.unwrap().merge_id.unwrap();
        let second = f.merge(B, C, false).await.unwrap().merge_id.unwrap();

        let refused = f.undo(first, false).await;

        assert!(matches!(&refused, Err(MergeError::MergedLater { target, later }) if target == B && *later == second), "{refused:?}");
        f.undo(second, false).await.unwrap();
        assert_eq!(canonical(&f.store, A).await.unwrap(), B, "undoing the second leaves the first");
        f.undo(first, false).await.unwrap();
        for id in [A, B, C] {
            assert_eq!(canonical(&f.store, id).await.unwrap(), id);
        }
    }

    #[tokio::test]
    async fn a_workspace_edited_since_the_merge_is_kept() {
        let f = Fixture::new(&[A, B]).await;
        f.write_workspace("agent-a", A, "agent a's notes");
        f.write_workspace("agent-b", A, "agent b's notes");
        let merged = f.merge(A, B, false).await.unwrap();
        assert_eq!(merged.workspaces.copied.len(), 2);
        let edited = f.write_workspace("agent-b", B, "agent b's notes, and one more line");

        let undone = f.undo(merged.merge_id.unwrap(), false).await.unwrap();

        assert_eq!(undone.workspaces.removed, vec![f.workspace("agent-a", B).display().to_string()]);
        assert_eq!(undone.workspaces.kept, vec![f.workspace("agent-b", B).display().to_string()]);
        assert!(!f.workspace("agent-a", B).exists());
        assert_eq!(std::fs::read_to_string(f.workspace("agent-b", B)).unwrap(), edited);
    }

    #[tokio::test]
    async fn a_merge_is_refused_for_an_unknown_the_same_or_a_merged_project() {
        let f = Fixture::new(&[A, B, C]).await;
        assert!(matches!(f.merge("nowhere-9f9f", B, false).await, Err(MergeError::UnknownProject(id)) if id == "nowhere-9f9f"));
        assert!(matches!(f.merge(A, A, false).await, Err(MergeError::SameProject(id)) if id == A));
        f.merge(A, B, false).await.unwrap();
        let into_tombstone = f.merge(C, A, false).await;
        assert!(matches!(&into_tombstone, Err(MergeError::Merged { id, into }) if id == A && into == B), "{into_tombstone:?}");
        let elsewhere = f.merge(A, C, false).await;
        assert!(matches!(&elsewhere, Err(MergeError::Merged { id, into }) if id == A && into == B), "{elsewhere:?}");
    }

    #[tokio::test]
    async fn a_deleted_project_is_refused_by_what_manages_projects() {
        let f = Fixture::new(&[A, B, C, "doc-dddd"]).await;
        let mark = |id: &'static str, delete_id: i64| {
            let f = &f;
            async move {
                f.sql(&format!(
                    "UPDATE project SET deleted_at = 5 WHERE id = '{id}';
                     INSERT INTO project_delete (delete_id, project_id, with_memories, deleted_at) VALUES ({delete_id}, '{id}', 0, 5);"
                ))
                .await
            }
        };
        let merged = f.merge("doc-dddd", C, false).await.unwrap().merge_id.unwrap();
        mark(B, 3).await;
        mark(C, 4).await;
        let before = f.everything().await;

        for (source, target, deleted) in [(A, B, B), (B, A, B), (A, C, C)] {
            let refused = f.merge(source, target, false).await;
            assert!(matches!(&refused, Err(MergeError::Deleted { id, .. }) if id == deleted), "{source} into {target}: {refused:?}");
        }
        let undo = f.undo(merged, false).await;
        assert!(matches!(&undo, Err(MergeError::Deleted { id, delete_id }) if id == C && *delete_id == Some(4)), "{undo:?}");
        assert_eq!(f.everything().await, before, "nothing is written");
    }
}
