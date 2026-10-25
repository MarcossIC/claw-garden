//! Taking an import back without losing what was saved after it.
//!
//! The import journaled what it wrote (V26). Undoing it removes for good the
//! rows it inserted that are still as it left them, with their vectors, piece
//! links, derivations and pairs, and gives back what it changed in rows that
//! were here: a row a newer version of its note replaced, a link filled where
//! it was empty, and the pairs, derivations, pieces, roots and remotes it
//! added. A row someone edited, deleted, restored or judged since, or one a
//! row that stays names, stays, and is named with why.
//!
//! Removed for good rather than marked deleted: a deleted row still counts as
//! here, so the same file could not be imported again, and a delete brings back
//! what the row replaced. The `pre-undo` snapshot is the way back.
//!
//! The decision is [`decide`], pure, over what [`load`] read of the store in
//! short calls; the writing goes in bounded transactions, each reading again
//! the columns the decision read of its rows, so an edit that came in between
//! keeps its row.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::memory::backup::Reason;
use crate::memory::{MemoryError, MemoryStore};

/// Lookups per call while the store is read, each a seek in an index.
const LOAD_BATCH: usize = 256;
/// Rows removed per transaction.
const WRITE_BATCH: usize = 256;
/// Journal entries read per call.
const PAGE: usize = 2048;

/// Why a row the import inserted stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepReason {
    /// Its `update_count` moved: an edit.
    Edited,
    Deleted,
    Restored,
    /// A verdict replaced it, or a pair of it is not as the import wrote it.
    Judged,
    /// A row that stays names it: a newer version, a conclusion, a dispute.
    Referenced,
    /// It is a piece of a save with a row that stays.
    Tied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptRow {
    pub id: i64,
    pub reason: KeepReason,
}

/// What an undo did, or would do on a dry run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoReport {
    pub import_id: i64,
    pub dry_run: bool,
    /// Rows the import inserted, removed for good.
    pub removed: Vec<i64>,
    /// Rows the import had replaced, given back the state they had.
    pub restored: Vec<i64>,
    /// Rows the import inserted that stay, with why.
    pub kept: Vec<KeptRow>,
    /// Rows here before the import whose link it filled, emptied again.
    pub unfilled: Vec<i64>,
    /// Pairs, derivations and piece links it added to rows that stay, removed.
    pub links: usize,
    /// Roots it recorded, removed because nothing uses them.
    pub projects: Vec<String>,
    /// Remotes it recorded, removed because nothing uses them.
    pub remotes: Vec<String>,
    /// The `pre-undo` snapshot taken before writing.
    pub snapshot: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum UndoError {
    #[error("no import {0}")]
    UnknownImport(i64),
    #[error("import {0} is still being written; undo it once it ends")]
    Running(i64),
    #[error(transparent)]
    Store(#[from] MemoryError),
}

impl From<rusqlite::Error> for UndoError {
    fn from(e: rusqlite::Error) -> Self {
        UndoError::Store(MemoryError::Sqlite(e))
    }
}

/// A row the import inserted, as it left it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Logged {
    pub id: i64,
    pub update_count: i64,
    pub deleted_at: Option<i64>,
    pub cause: Option<String>,
}

/// A row the import replaced, with the state it had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Replaced {
    pub row: i64,
    pub by: i64,
    pub status: String,
    pub superseded_by: Option<i64>,
    pub cause: Option<String>,
}

/// A link column the import fills where it is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Column {
    SupersededBy,
    RevivedFrom,
    DisputedAgainst,
}

impl Column {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "superseded_by" => Some(Column::SupersededBy),
            "revived_from" => Some(Column::RevivedFrom),
            "disputed_against" => Some(Column::DisputedAgainst),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Column::SupersededBy => "superseded_by",
            Column::RevivedFrom => "revived_from",
            Column::DisputedAgainst => "disputed_against",
        }
    }
}

/// What the import added that hangs from rows it did not insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Extra {
    Fill { row: i64, column: Column, value: String },
    Pair { new_id: i64, old_id: i64, state: String, verdict: Option<String> },
    Derivation { memory: i64, source: i64 },
    Piece { save: i64, piece: i64 },
    Project(String),
    Remote(String),
}

impl Extra {
    /// The rows a link the import wrote in a row it did not insert names,
    /// which name no row it inserted as far as keeping them goes.
    fn names(&self) -> Vec<(i64, i64)> {
        match self {
            Extra::Fill { row, column: Column::DisputedAgainst, value } => {
                serde_json::from_str::<Vec<i64>>(value).unwrap_or_default().into_iter().map(|to| (*row, to)).collect()
            }
            Extra::Fill { row, value, .. } => value.parse().map(|to| vec![(*row, to)]).unwrap_or_default(),
            Extra::Derivation { memory, source } => vec![(*memory, *source)],
            _ => Vec::new(),
        }
    }
}

/// A row as the store holds it now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Now {
    pub update_count: i64,
    pub deleted_at: Option<i64>,
    pub cause: Option<String>,
    pub project: Option<String>,
    pub status: String,
    pub superseded_by: Option<i64>,
    pub revived_from: Option<i64>,
    pub disputed_against: Option<String>,
}

impl Now {
    fn column(&self, column: Column) -> Option<String> {
        match column {
            Column::SupersededBy => self.superseded_by.map(|v| v.to_string()),
            Column::RevivedFrom => self.revived_from.map(|v| v.to_string()),
            Column::DisputedAgainst => self.disputed_against.clone(),
        }
    }

    /// Why a row whose journal says `logged` stays on its own columns.
    fn changed_from(&self, logged: &Logged) -> Option<KeepReason> {
        if self.deleted_at != logged.deleted_at {
            return Some(match (logged.deleted_at, self.deleted_at) {
                (Some(_), None) => KeepReason::Restored,
                _ => KeepReason::Deleted,
            });
        }
        if self.update_count != logged.update_count {
            return Some(KeepReason::Edited);
        }
        let verdict = |c: &Option<String>| c.as_deref() == Some("verdict");
        (verdict(&self.cause) && !verdict(&logged.cause)).then_some(KeepReason::Judged)
    }
}

/// A root the import recorded, as it is now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ProjectNow {
    pub index_docs: bool,
    pub docs: bool,
    /// Rows of every kind under its id.
    pub rows: i64,
    /// Remotes that send their clones to it.
    pub remotes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct RemoteNow {
    pub project_id: String,
    /// Rows of every kind under its project.
    pub rows: i64,
    /// Whether a root has its project's id.
    pub root: bool,
}

/// Everything [`decide`] reads: the journal of one import and what the store
/// holds now of the rows it names.
#[derive(Debug, Clone, Default)]
pub(crate) struct Facts {
    pub rows: Vec<Logged>,
    pub replaced: Vec<Replaced>,
    pub extras: Vec<Extra>,
    /// Each row the journal names that is still here.
    pub now: HashMap<i64, Now>,
    /// Rows naming a row the import inserted, `(from, to)`, by
    /// `superseded_by`, `revived_from`, `disputed_against` or as the source of
    /// a derivation.
    pub references: Vec<(i64, i64)>,
    /// Every pair that touches a row the import inserted or that it added,
    /// with its state and verdict now.
    pub pairs: HashMap<(i64, i64), (String, Option<String>)>,
    /// The derivations it added that are still here.
    pub derivations: HashSet<(i64, i64)>,
    /// The pieces of every save with a row it inserted or a piece it added,
    /// `(piece, memory)` by save.
    pub saves: HashMap<i64, Vec<(i64, i64)>>,
    pub projects: HashMap<String, ProjectNow>,
    pub remotes: HashMap<String, RemoteNow>,
}

/// What an undo writes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    /// Rows to remove for good, by id.
    pub retire: BTreeSet<i64>,
    pub kept: BTreeMap<i64, KeepReason>,
    /// Rows of the journal no longer here (purged), whose entries go.
    pub gone: Vec<i64>,
    pub restore: Vec<Replaced>,
    pub unfill: Vec<Extra>,
    /// Pairs, derivations and pieces to remove.
    pub links: Vec<Extra>,
    pub projects: Vec<String>,
    pub remotes: Vec<String>,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.retire.is_empty()
            && self.restore.is_empty()
            && self.unfill.is_empty()
            && self.links.is_empty()
            && self.projects.is_empty()
            && self.remotes.is_empty()
    }
}

/// Decides what an undo takes back. A row the import inserted is removed
/// unless it changed on its own, a pair of it was judged, a row that stays
/// names it, or a save with a row that stays holds it; each kept row can keep
/// others, so it runs until nothing more is kept. What the import changed in
/// other rows comes back only where it is still as the import left it.
pub(crate) fn decide(facts: &Facts) -> Plan {
    let mut plan = Plan::default();
    let mut candidates = BTreeSet::new();
    for logged in &facts.rows {
        match facts.now.get(&logged.id) {
            None => plan.gone.push(logged.id),
            Some(now) => {
                candidates.insert(logged.id);
                if let Some(reason) = now.changed_from(logged) {
                    plan.kept.insert(logged.id, reason);
                }
            }
        }
    }
    let written: HashMap<(i64, i64), (&str, Option<&str>)> = facts
        .extras
        .iter()
        .filter_map(|e| match e {
            Extra::Pair { new_id, old_id, state, verdict } => Some(((*new_id, *old_id), (state.as_str(), verdict.as_deref()))),
            _ => None,
        })
        .collect();
    let judged = |key: &(i64, i64), (state, verdict): &(String, Option<String>)| match written.get(key) {
        Some(w) => *w != (state.as_str(), verdict.as_deref()),
        None => verdict.is_some(),
    };
    for (key, now) in &facts.pairs {
        if judged(key, now) {
            for end in [key.0, key.1] {
                if candidates.contains(&end) {
                    plan.kept.entry(end).or_insert(KeepReason::Judged);
                }
            }
        }
    }

    // What the import wrote in rows it did not insert, still as it left it,
    // names its rows only because of the import.
    let replaced_now = |r: &&Replaced| facts.now.get(&r.row).is_some_and(|n| n.status == "superseded" && n.superseded_by == Some(r.by));
    let mut own: HashSet<(i64, i64)> = facts.extras.iter().filter(|e| intact(facts, e)).flat_map(Extra::names).collect();
    own.extend(facts.replaced.iter().filter(replaced_now).map(|r| (r.row, r.by)));
    let added_pieces: HashSet<(i64, i64)> = facts
        .extras
        .iter()
        .filter_map(|e| match e {
            Extra::Piece { save, piece } => Some((*save, *piece)),
            _ => None,
        })
        .collect();
    let gone: HashSet<i64> = plan.gone.iter().copied().collect();
    loop {
        let retire = |id: &i64, kept: &BTreeMap<i64, KeepReason>| candidates.contains(id) && !kept.contains_key(id);
        let mut more = Vec::new();
        for (from, to) in &facts.references {
            if retire(to, &plan.kept) && !retire(from, &plan.kept) && !own.contains(&(*from, *to)) {
                more.push((*to, KeepReason::Referenced));
            }
        }
        for (save, pieces) in &facts.saves {
            // A piece link the import added to a row it did not insert goes
            // with the undo, and the link a purge left to its row holds
            // nothing either.
            let stays = pieces.iter().any(|(piece, memory)| {
                !retire(memory, &plan.kept) && !gone.contains(memory) && !added_pieces.contains(&(*save, *piece))
            });
            if stays {
                more.extend(pieces.iter().filter(|(_, m)| retire(m, &plan.kept)).map(|(_, m)| (*m, KeepReason::Tied)));
            }
        }
        if more.is_empty() {
            break;
        }
        for (id, reason) in more {
            plan.kept.entry(id).or_insert(reason);
        }
    }
    plan.retire = candidates.iter().copied().filter(|id| !plan.kept.contains_key(id)).collect();

    let kept = |id: &i64| plan.kept.contains_key(id);
    plan.restore = facts.replaced.iter().filter(replaced_now).filter(|r| !kept(&r.by)).cloned().collect();
    let after = |id: &i64| !plan.retire.contains(id) && !gone.contains(id);
    let mut rows_left: HashMap<&str, i64> = HashMap::new();
    for id in &plan.retire {
        if let Some(project) = facts.now[id].project.as_deref() {
            *rows_left.entry(project).or_default() -= 1;
        }
    }
    let unused = |project: &str, rows: i64| rows + rows_left.get(project).copied().unwrap_or_default() <= 0;
    let removable_remote = |remote: &str| facts.remotes.get(remote).is_some_and(|r| unused(&r.project_id, r.rows));
    let logged_remotes: HashSet<&str> =
        facts.extras.iter().filter_map(|e| if let Extra::Remote(r) = e { Some(r.as_str()) } else { None }).collect();
    for extra in &facts.extras {
        if !intact(facts, extra) {
            continue;
        }
        match extra {
            Extra::Fill { .. } => plan.unfill.push(extra.clone()),
            Extra::Pair { new_id, old_id, .. } if after(new_id) && after(old_id) && !kept(new_id) && !kept(old_id) => {
                plan.links.push(extra.clone());
            }
            Extra::Derivation { memory, source } if after(memory) && after(source) && !kept(source) => plan.links.push(extra.clone()),
            Extra::Piece { save, .. } => {
                let pieces = facts.saves.get(save).map(Vec::as_slice).unwrap_or_default();
                if !pieces.iter().any(|(_, m)| kept(m)) {
                    plan.links.push(extra.clone());
                }
            }
            Extra::Project(root) => {
                let p = &facts.projects[root];
                let remotes_go = p.remotes.iter().all(|r| logged_remotes.contains(r.as_str()) && removable_remote(r));
                if !p.index_docs && !p.docs && unused(root, p.rows) && remotes_go {
                    plan.projects.push(root.clone());
                }
            }
            _ => {}
        }
    }
    // A save the undo takes whole loses the link a purge left to a piece.
    for (save, pieces) in &facts.saves {
        if !pieces.iter().any(|(_, m)| kept(m)) {
            plan.links.extend(pieces.iter().filter(|(_, m)| gone.contains(m)).map(|(piece, _)| Extra::Piece { save: *save, piece: *piece }));
        }
    }
    for extra in &facts.extras {
        if let Extra::Remote(remote) = extra {
            let Some(r) = facts.remotes.get(remote) else { continue };
            if removable_remote(remote) && (!r.root || plan.projects.contains(&r.project_id)) {
                plan.remotes.push(remote.clone());
            }
        }
    }
    plan
}

/// Whether `extra` is still as the import wrote it.
fn intact(facts: &Facts, extra: &Extra) -> bool {
    match extra {
        Extra::Fill { row, column, value } => facts.now.get(row).is_some_and(|n| n.column(*column).as_deref() == Some(value)),
        Extra::Pair { new_id, old_id, state, verdict } => {
            facts.pairs.get(&(*new_id, *old_id)).is_some_and(|(s, v)| s == state && v == verdict)
        }
        Extra::Derivation { memory, source } => facts.derivations.contains(&(*memory, *source)),
        Extra::Piece { save, piece } => facts.saves.get(save).is_some_and(|p| p.iter().any(|(k, _)| k == piece)),
        Extra::Project(root) => facts.projects.contains_key(root),
        Extra::Remote(remote) => facts.remotes.contains_key(remote),
    }
}

/// Runs `f` once per key, [`LOAD_BATCH`] keys to a call.
async fn each<K, T, F>(store: &MemoryStore, keys: Vec<K>, f: F) -> Result<Vec<T>, MemoryError>
where
    K: Send + 'static,
    T: Send + 'static,
    F: Fn(&Connection, &K) -> rusqlite::Result<T> + Send + Copy + 'static,
{
    let mut out = Vec::with_capacity(keys.len());
    let mut keys = keys.into_iter().peekable();
    while keys.peek().is_some() {
        let batch: Vec<K> = keys.by_ref().take(LOAD_BATCH).collect();
        let found = store
            .conn
            .call(move |c| batch.iter().map(|k| f(c, k)).collect::<rusqlite::Result<Vec<T>>>())
            .await
            .map_err(MemoryError::Connection)?;
        out.extend(found);
    }
    Ok(out)
}

/// Reads `sql`, which takes the import and the last key read and returns
/// the key first, in its order, [`PAGE`] entries to a call.
async fn paged<T, F>(store: &MemoryStore, import_id: i64, sql: &'static str, read: F) -> Result<Vec<T>, MemoryError>
where
    T: Send + 'static,
    F: Fn(&rusqlite::Row<'_>) -> rusqlite::Result<T> + Send + Copy + 'static,
{
    let mut out = Vec::new();
    let mut after = 0i64;
    loop {
        let page = store
            .conn
            .call(move |c| -> rusqlite::Result<Vec<(i64, T)>> {
                let mut stmt = c.prepare_cached(sql)?;
                let rows = stmt.query_map(rusqlite::params![import_id, after, PAGE as i64], |r| Ok((r.get(0)?, read(r)?)))?;
                rows.collect()
            })
            .await
            .map_err(MemoryError::Connection)?;
        let full = page.len() == PAGE;
        for (key, item) in page {
            after = key;
            out.push(item);
        }
        if !full {
            return Ok(out);
        }
    }
}

/// By row, which its index orders after the import.
const ROWS_SQL: &str = "SELECT row_id, row_id, update_count, deleted_at, supersede_cause FROM memory_import_row \
     WHERE import_id = ?1 AND row_id > ?2 ORDER BY row_id LIMIT ?3";
const REPLACED_SQL: &str = "SELECT rowid, row_id, superseded_by, from_status, from_superseded_by, from_cause \
     FROM memory_import_supersede WHERE import_id = ?1 AND rowid > ?2 ORDER BY rowid LIMIT ?3";
const EXTRAS_SQL: &str = "SELECT rowid, kind, a, b, value FROM memory_import_extra \
     WHERE import_id = ?1 AND rowid > ?2 ORDER BY rowid LIMIT ?3";

/// The rows naming a row by `superseded_by`, by `revived_from` and as a
/// source of a conclusion.
const NAMED_BY_SQL: [&str; 3] = [
    "SELECT id FROM memory WHERE superseded_by = ?1",
    "SELECT id FROM memory WHERE revived_from = ?1",
    "SELECT memory_id FROM memory_derivation WHERE source_id = ?1",
];
/// `disputed_against` names a row only while its row is disputed, and an
/// index holds those.
const DISPUTES_SQL: &str = "SELECT m.id, j.value FROM memory m, \
     json_each(CASE WHEN json_valid(m.disputed_against) THEN m.disputed_against END) j \
     WHERE m.status = 'disputed' AND j.type = 'integer'";
const PAIRS_OF_SQL: &str = "SELECT new_id, old_id, state, verdict FROM memory_conflict WHERE new_id = ?1 \
     UNION ALL SELECT new_id, old_id, state, verdict FROM memory_conflict WHERE old_id = ?1";
const SAVES_OF_SQL: &str = "SELECT save_id FROM memory_piece WHERE memory_id = ?1";
const PIECES_SQL: &str = "SELECT piece, memory_id FROM memory_piece WHERE save_id = ?1 ORDER BY piece";

const NOW_SQL: &str = "SELECT update_count, deleted_at, supersede_cause, project_id, status, superseded_by, revived_from, \
     disputed_against FROM memory WHERE id = ?1";

/// Rows of every kind under a project id.
const PROJECT_ROWS_SQL: &str = "SELECT (SELECT COUNT(*) FROM memory WHERE project_id = ?1) \
     + (SELECT COUNT(*) FROM episodic_trajectory WHERE project_id = ?1) \
     + (SELECT COUNT(*) FROM procedural_skill WHERE project_id = ?1)";

/// No row of any kind is under the project id `$id`.
macro_rules! no_rows {
    ($id:literal) => {
        concat!(
            "NOT EXISTS (SELECT 1 FROM memory WHERE project_id = ", $id, ") \
             AND NOT EXISTS (SELECT 1 FROM episodic_trajectory WHERE project_id = ", $id, ") \
             AND NOT EXISTS (SELECT 1 FROM procedural_skill WHERE project_id = ", $id, ")"
        )
    };
}

fn now_of(c: &Connection, id: &i64) -> rusqlite::Result<Option<(i64, Now)>> {
    let now = c
        .prepare_cached(NOW_SQL)?
        .query_row([id], |r| {
            Ok(Now {
                update_count: r.get(0)?,
                deleted_at: r.get(1)?,
                cause: r.get(2)?,
                project: r.get(3)?,
                status: r.get(4)?,
                superseded_by: r.get(5)?,
                revived_from: r.get(6)?,
                disputed_against: r.get(7)?,
            })
        })
        .optional()?;
    Ok(now.map(|n| (*id, n)))
}

/// Reads what [`decide`] needs of import `import_id`.
async fn load(store: &MemoryStore, import_id: i64) -> Result<Facts, UndoError> {
    let known: bool = store
        .conn
        .call(move |c| c.query_row("SELECT EXISTS (SELECT 1 FROM memory_import WHERE import_id = ?1)", [import_id], |r| r.get(0)))
        .await
        .map_err(MemoryError::Connection)?;
    if !known {
        return Err(UndoError::UnknownImport(import_id));
    }
    let rows = paged(store, import_id, ROWS_SQL, |r| {
        Ok(Logged { id: r.get(1)?, update_count: r.get(2)?, deleted_at: r.get(3)?, cause: r.get(4)? })
    })
    .await?;
    let replaced = paged(store, import_id, REPLACED_SQL, |r| {
        Ok(Replaced { row: r.get(1)?, by: r.get(2)?, status: r.get(3)?, superseded_by: r.get(4)?, cause: r.get(5)? })
    })
    .await?;
    let extras: Vec<Extra> = paged(store, import_id, EXTRAS_SQL, |r| {
        let (kind, a, b, value): (String, Option<i64>, Option<String>, Option<String>) = (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
        let (a, b) = (a.unwrap_or_default(), b.unwrap_or_default());
        let number = || b.parse::<i64>().unwrap_or_default();
        Ok(match kind.as_str() {
            "fill" => Column::parse(&b).map(|column| Extra::Fill { row: a, column, value: value.unwrap_or_default() }),
            "pair" => {
                let (state, verdict): (String, Option<String>) = serde_json::from_str(value.as_deref().unwrap_or("[]")).unwrap_or_default();
                Some(Extra::Pair { new_id: a, old_id: number(), state, verdict })
            }
            "derivation" => Some(Extra::Derivation { memory: a, source: number() }),
            "piece" => Some(Extra::Piece { save: a, piece: number() }),
            "project" => Some(Extra::Project(b)),
            "remote" => Some(Extra::Remote(b)),
            _ => None,
        })
    })
    .await?
    .into_iter()
    .flatten()
    .collect();

    let inserted: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let mut ids: BTreeSet<i64> = inserted.iter().copied().collect();
    ids.extend(replaced.iter().map(|r| r.row));
    ids.extend(extras.iter().filter_map(|e| if let Extra::Fill { row, .. } = e { Some(*row) } else { None }));
    let now: HashMap<i64, Now> = each(store, ids.into_iter().collect(), now_of).await?.into_iter().flatten().collect();
    let present: Vec<i64> = inserted.iter().copied().filter(|id| now.contains_key(id)).collect();

    let mut references: Vec<(i64, i64)> = each(store, present.clone(), |c, id| -> rusqlite::Result<Vec<(i64, i64)>> {
        let mut found = Vec::new();
        for sql in NAMED_BY_SQL {
            let mut stmt = c.prepare_cached(sql)?;
            for from in stmt.query_map([id], |r| r.get::<_, i64>(0))? {
                found.push((from?, *id));
            }
        }
        Ok(found)
    })
    .await?
    .into_iter()
    .flatten()
    .collect();
    let present_set: HashSet<i64> = present.iter().copied().collect();
    let disputes = store
        .conn
        .call(|c| -> rusqlite::Result<Vec<(i64, i64)>> {
            let mut stmt = c.prepare(DISPUTES_SQL)?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .await
        .map_err(MemoryError::Connection)?;
    references.extend(disputes.into_iter().filter(|(_, to)| present_set.contains(to)));

    let mut pairs: HashMap<(i64, i64), (String, Option<String>)> = each(store, present.clone(), |c, id| {
        let mut stmt = c.prepare_cached(PAIRS_OF_SQL)?;
        let rows = stmt.query_map([id], |r| Ok(((r.get(0)?, r.get(1)?), (r.get(2)?, r.get(3)?))))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
    .await?
    .into_iter()
    .flatten()
    .collect();
    let logged_pairs: Vec<(i64, i64)> =
        extras.iter().filter_map(|e| if let Extra::Pair { new_id, old_id, .. } = e { Some((*new_id, *old_id)) } else { None }).collect();
    pairs.extend(
        each(store, logged_pairs, |c, key: &(i64, i64)| {
            c.prepare_cached("SELECT state, verdict FROM memory_conflict WHERE new_id = ?1 AND old_id = ?2")?
                .query_row([key.0, key.1], |r| Ok((*key, (r.get(0)?, r.get(1)?))))
                .optional()
        })
        .await?
        .into_iter()
        .flatten(),
    );

    let logged_derivations: Vec<(i64, i64)> =
        extras.iter().filter_map(|e| if let Extra::Derivation { memory, source } = e { Some((*memory, *source)) } else { None }).collect();
    let derivations: HashSet<(i64, i64)> = each(store, logged_derivations, |c, key: &(i64, i64)| {
        let here: bool = c
            .prepare_cached("SELECT EXISTS (SELECT 1 FROM memory_derivation WHERE memory_id = ?1 AND source_id = ?2)")?
            .query_row([key.0, key.1], |r| r.get(0))?;
        Ok(here.then_some(*key))
    })
    .await?
    .into_iter()
    .flatten()
    .collect();

    let mut save_ids: BTreeSet<i64> = each(store, present, |c, id| {
        let mut stmt = c.prepare_cached(SAVES_OF_SQL)?;
        let rows = stmt.query_map([id], |r| r.get::<_, i64>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
    .await?
    .into_iter()
    .flatten()
    .collect();
    save_ids.extend(extras.iter().filter_map(|e| if let Extra::Piece { save, .. } = e { Some(*save) } else { None }));
    let saves: HashMap<i64, Vec<(i64, i64)>> = each(store, save_ids.into_iter().collect(), |c, save| {
        let mut stmt = c.prepare_cached(PIECES_SQL)?;
        let rows = stmt.query_map([save], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok((*save, rows.collect::<rusqlite::Result<Vec<_>>>()?))
    })
    .await?
    .into_iter()
    .filter(|(_, pieces)| !pieces.is_empty())
    .collect();

    let roots: Vec<String> = extras.iter().filter_map(|e| if let Extra::Project(r) = e { Some(r.clone()) } else { None }).collect();
    let projects: HashMap<String, ProjectNow> = each(store, roots, |c, root: &String| {
        let index_docs: Option<bool> =
            c.prepare_cached("SELECT index_docs FROM project_path WHERE id = ?1")?.query_row([root], |r| r.get(0)).optional()?;
        let Some(index_docs) = index_docs else { return Ok(None) };
        let docs = c
            .prepare_cached("SELECT EXISTS (SELECT 1 FROM project_doc_file WHERE project_id = ?1)")?
            .query_row([root], |r| r.get(0))?;
        let rows = c.prepare_cached(PROJECT_ROWS_SQL)?.query_row([root], |r| r.get(0))?;
        let mut stmt = c.prepare_cached("SELECT remote FROM project_remote WHERE project_id = ?1")?;
        let remotes = stmt.query_map([root], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(Some((root.clone(), ProjectNow { index_docs, docs, rows, remotes })))
    })
    .await?
    .into_iter()
    .flatten()
    .collect();
    let keys: Vec<String> = extras.iter().filter_map(|e| if let Extra::Remote(r) = e { Some(r.clone()) } else { None }).collect();
    let remotes: HashMap<String, RemoteNow> = each(store, keys, |c, remote: &String| {
        let project_id: Option<String> =
            c.prepare_cached("SELECT project_id FROM project_remote WHERE remote = ?1")?.query_row([remote], |r| r.get(0)).optional()?;
        let Some(project_id) = project_id else { return Ok(None) };
        let rows = c.prepare_cached(PROJECT_ROWS_SQL)?.query_row([&project_id], |r| r.get(0))?;
        let root = c
            .prepare_cached("SELECT EXISTS (SELECT 1 FROM project_path WHERE id = ?1)")?
            .query_row([&project_id], |r| r.get(0))?;
        Ok(Some((remote.clone(), RemoteNow { project_id, rows, root })))
    })
    .await?
    .into_iter()
    .flatten()
    .collect();

    Ok(Facts { rows, replaced, extras, now, references, pairs, derivations, saves, projects, remotes })
}

const REMOVE_SQL: [&str; 7] = [
    "DELETE FROM memory_vectors WHERE rowid = ?1",
    "DELETE FROM memory_piece WHERE memory_id = ?1",
    "DELETE FROM memory_derivation WHERE memory_id = ?1",
    "DELETE FROM memory_derivation WHERE source_id = ?1",
    "DELETE FROM memory_conflict WHERE new_id = ?1",
    "DELETE FROM memory_conflict WHERE old_id = ?1",
    "DELETE FROM memory WHERE id = ?1",
];

/// Removes row `id` for good: the row, its vector, its piece links, its
/// derivations either way and its pairs. The text index trigger of V8 takes
/// out a row that was live, and leaves one already taken out.
fn remove_in(c: &Connection, id: i64) -> rusqlite::Result<()> {
    for sql in REMOVE_SQL {
        c.prepare_cached(sql)?.execute([id])?;
    }
    Ok(())
}

fn restore_in(c: &Connection, import_id: i64, r: &Replaced) -> rusqlite::Result<bool> {
    let restored = c.execute(
        "UPDATE memory SET status = ?2, superseded_by = ?3, supersede_cause = ?4 \
         WHERE id = ?1 AND status = 'superseded' AND superseded_by = ?5",
        rusqlite::params![r.row, r.status, r.superseded_by, r.cause, r.by],
    )? == 1;
    c.execute("DELETE FROM memory_import_supersede WHERE import_id = ?1 AND row_id = ?2 AND superseded_by = ?3", [import_id, r.row, r.by])?;
    Ok(restored)
}

/// Takes back `extra` where it is still as the import wrote it, and drops
/// its journal entry. Whether anything changed.
fn take_back_in(c: &Connection, import_id: i64, extra: &Extra) -> rusqlite::Result<bool> {
    let (changed, kind, a, b) = match extra {
        Extra::Fill { row, column, value } => {
            let sql = format!("UPDATE memory SET {0} = NULL WHERE id = ?1 AND CAST({0} AS TEXT) = ?2", column.name());
            (c.execute(&sql, rusqlite::params![row, value])?, "fill", Some(*row), column.name().to_owned())
        }
        Extra::Pair { new_id, old_id, state, verdict } => (
            c.execute(
                "DELETE FROM memory_conflict WHERE new_id = ?1 AND old_id = ?2 AND state = ?3 AND verdict IS ?4",
                rusqlite::params![new_id, old_id, state, verdict],
            )?,
            "pair",
            Some(*new_id),
            old_id.to_string(),
        ),
        Extra::Derivation { memory, source } => (
            c.execute("DELETE FROM memory_derivation WHERE memory_id = ?1 AND source_id = ?2", [memory, source])?,
            "derivation",
            Some(*memory),
            source.to_string(),
        ),
        Extra::Piece { save, piece } => (
            c.execute("DELETE FROM memory_piece WHERE save_id = ?1 AND piece = ?2", [save, piece])?,
            "piece",
            Some(*save),
            piece.to_string(),
        ),
        Extra::Project(root) => (
            c.execute(
                concat!(
                    "DELETE FROM project_path WHERE id = ?1 AND index_docs = 0 AND ",
                    no_rows!("?1"),
                    " AND NOT EXISTS (SELECT 1 FROM project_doc_file WHERE project_id = ?1) \
                     AND NOT EXISTS (SELECT 1 FROM project_remote WHERE project_id = ?1)"
                ),
                [root],
            )?,
            "project",
            None,
            root.clone(),
        ),
        Extra::Remote(remote) => (
            c.execute(
                concat!("DELETE FROM project_remote WHERE remote = ?1 AND ", no_rows!("project_remote.project_id")),
                [remote],
            )?,
            "remote",
            None,
            remote.clone(),
        ),
    };
    c.execute(
        "DELETE FROM memory_import_extra WHERE import_id = ?1 AND kind = ?2 AND a IS ?3 AND b = ?4",
        rusqlite::params![import_id, kind, a, b],
    )?;
    Ok(changed > 0)
}

/// Takes import `import_id` back: removes for good the rows it inserted that
/// are still as it left them and gives back what it changed in the others,
/// after a `pre-undo` snapshot into `backups`. A dry run writes nothing.
/// Undoing an import twice changes nothing the second time; what stayed is
/// still in the journal, and a later undo weighs it again.
pub async fn undo_import(store: &MemoryStore, backups: &Path, import_id: i64, dry_run: bool) -> Result<UndoReport, UndoError> {
    if store.importing.lock().unwrap_or_else(|e| e.into_inner()).contains(&import_id) {
        return Err(UndoError::Running(import_id));
    }
    let facts = load(store, import_id).await?;
    let (facts, plan) = tokio::task::spawn_blocking(move || {
        let plan = decide(&facts);
        (facts, plan)
    })
    .await
    .map_err(|e| MemoryError::Internal(format!("could not decide the undo: {e}")))?;
    let mut report = UndoReport { import_id, dry_run, ..Default::default() };
    if dry_run || plan.is_empty() {
        report.removed = plan.retire.iter().copied().collect();
        report.kept = plan.kept.iter().map(|(id, reason)| KeptRow { id: *id, reason: *reason }).collect();
        if dry_run {
            report.restored = plan.restore.iter().map(|r| r.row).collect();
            report.unfilled = rows_of(&plan.unfill);
            report.links = plan.links.len();
            report.projects = plan.projects.clone();
            report.remotes = plan.remotes.clone();
        }
        return Ok(report);
    }
    report.snapshot = Some(store.snapshot(backups.to_path_buf(), Reason::PreUndo).await?.display().to_string());

    let logged: HashMap<i64, Logged> = facts.rows.iter().map(|r| (r.id, r.clone())).collect();
    let mut kept = plan.kept.clone();
    let retire: Vec<i64> = plan.retire.iter().copied().collect();
    // What a removed row replaced comes back in the transaction that removes
    // it; the rest, replaced by a row that was here or is gone, at the end.
    let mut restore_by: HashMap<i64, Vec<Replaced>> = HashMap::new();
    let mut rest = Vec::new();
    for r in &plan.restore {
        match plan.retire.contains(&r.by) {
            true => restore_by.entry(r.by).or_default().push(r.clone()),
            false => rest.push(r.clone()),
        }
    }
    // A row that changed after the decision stays; what it would have kept
    // with it is weighed again by the next undo.
    for batch in retire.chunks(WRITE_BATCH) {
        let batch: Vec<(i64, Logged, Vec<Replaced>)> =
            batch.iter().map(|id| (*id, logged[id].clone(), restore_by.get(id).cloned().unwrap_or_default())).collect();
        let done = store
            .conn
            .call(move |c| -> rusqlite::Result<Vec<(i64, Result<Vec<i64>, KeepReason>)>> {
                let tx = c.transaction()?;
                let mut done = Vec::with_capacity(batch.len());
                for (id, logged, replaced) in &batch {
                    let changed = now_of(&tx, id)?.and_then(|(_, now)| now.changed_from(logged));
                    if let Some(reason) = changed {
                        done.push((*id, Err(reason)));
                        continue;
                    }
                    remove_in(&tx, *id)?;
                    tx.execute("DELETE FROM memory_import_row WHERE import_id = ?1 AND row_id = ?2", [import_id, *id])?;
                    let mut restored = Vec::new();
                    // In the transaction that removes the row that replaced
                    // them, so no cut leaves two live versions of a note.
                    for r in replaced {
                        if restore_in(&tx, import_id, r)? {
                            restored.push(r.row);
                        }
                    }
                    done.push((*id, Ok(restored)));
                }
                tx.commit()?;
                Ok(done)
            })
            .await
            .map_err(MemoryError::Connection)?;
        for (id, outcome) in done {
            match outcome {
                Ok(restored) => {
                    report.removed.push(id);
                    report.restored.extend(restored);
                }
                Err(reason) => {
                    kept.insert(id, reason);
                }
            }
        }
    }

    let (unfill, links, projects, remotes, gone) =
        (plan.unfill.clone(), plan.links.clone(), plan.projects.clone(), plan.remotes.clone(), plan.gone.clone());
    let (restored, unfilled, removed_links, removed_projects, removed_remotes) = store
        .conn
        .call(move |c| -> rusqlite::Result<(Vec<i64>, Vec<i64>, usize, Vec<String>, Vec<String>)> {
            let tx = c.transaction()?;
            let mut restored = Vec::new();
            for r in &rest {
                if restore_in(&tx, import_id, r)? {
                    restored.push(r.row);
                }
            }
            let mut emptied = Vec::new();
            for extra in &unfill {
                if take_back_in(&tx, import_id, extra)? {
                    emptied.push(extra.clone());
                }
            }
            let unfilled = rows_of(&emptied);
            let mut links_removed = 0;
            for extra in &links {
                links_removed += usize::from(take_back_in(&tx, import_id, extra)?);
            }
            let mut removed_remotes = Vec::new();
            for remote in &remotes {
                if take_back_in(&tx, import_id, &Extra::Remote(remote.clone()))? {
                    removed_remotes.push(remote.clone());
                }
            }
            let mut removed_projects = Vec::new();
            for root in &projects {
                if take_back_in(&tx, import_id, &Extra::Project(root.clone()))? {
                    removed_projects.push(root.clone());
                }
            }
            for id in &gone {
                tx.execute("DELETE FROM memory_import_row WHERE import_id = ?1 AND row_id = ?2", [import_id, *id])?;
            }
            tx.commit()?;
            Ok((restored, unfilled, links_removed, removed_projects, removed_remotes))
        })
        .await
        .map_err(MemoryError::Connection)?;
    report.restored.extend(restored);
    report.restored.sort_unstable();
    report.unfilled = unfilled;
    report.links = removed_links;
    report.projects = removed_projects;
    report.remotes = removed_remotes;
    report.kept = kept.into_iter().map(|(id, reason)| KeptRow { id, reason }).collect();
    Ok(report)
}

fn rows_of(fills: &[Extra]) -> Vec<i64> {
    let rows: BTreeSet<i64> = fills.iter().filter_map(|e| if let Extra::Fill { row, .. } = e { Some(*row) } else { None }).collect();
    rows.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::admin::{delete_memory_core, restore_memory_core, MemoryKind};
    use crate::memory::embedder::FakeEmbedder;
    use crate::memory::portability::foreign::{ForeignNote, When};
    use crate::memory::portability::import::tests::{
        destination, dump, insert_row, relation, run, run_export, run_foreign, sample_notes, seeded_file, EVERYTHING,
    };
    use crate::memory::portability::import::ImportOptions;
    use crate::memory::portability::translate::tests::{export as foreign_export, note as foreign_note, T0};
    use crate::memory::relocate::{move_memories_core, MoveFilter};

    fn logged(id: i64) -> Logged {
        Logged { id, update_count: 0, deleted_at: None, cause: None }
    }

    fn now(project: Option<&str>) -> Now {
        Now { status: "active".into(), project: project.map(str::to_owned), ..Default::default() }
    }

    #[test]
    fn decide_keeps_what_changed_and_what_is_named() {
        let mut facts = Facts { rows: (1..=13).map(logged).collect(), ..Default::default() };
        facts.rows[3].deleted_at = Some(5);
        for id in (1..=13).filter(|id| *id != 12) {
            facts.now.insert(id, now(Some("r")));
        }
        facts.now.get_mut(&2).unwrap().update_count = 1;
        facts.now.get_mut(&3).unwrap().deleted_at = Some(9);
        facts.now.get_mut(&5).unwrap().cause = Some("verdict".into());
        facts.extras.push(Extra::Pair { new_id: 100, old_id: 6, state: "pending".into(), verdict: None });
        facts.pairs.insert((100, 6), ("judged".into(), Some("compatible".into())));
        facts.pairs.insert((9, 1), ("pending".into(), None));
        facts.extras.push(Extra::Pair { new_id: 9, old_id: 1, state: "pending".into(), verdict: None });
        // 7 is a source of a conclusion saved since; 8 the newer version of
        // 2, which stays; 9 is named by 1, which goes; 11 and 1 by rows only
        // through what the import wrote in them.
        facts.references.extend([(200, 7), (2, 8), (1, 9), (300, 11), (400, 1)]);
        facts.saves.insert(40, vec![(1, 10), (2, 2)]);
        // 12 was purged, and its save goes with 13.
        facts.saves.insert(41, vec![(1, 12), (2, 13)]);
        // 300 was here, and the import filled its link to 11.
        facts.now.insert(300, Now { superseded_by: Some(11), ..now(None) });
        facts.extras.push(Extra::Fill { row: 300, column: Column::SupersededBy, value: "11".into() });
        facts.replaced.push(Replaced { row: 400, by: 1, status: "active".into(), superseded_by: None, cause: None });
        facts.replaced.push(Replaced { row: 401, by: 2, status: "active".into(), superseded_by: None, cause: None });
        facts.replaced.push(Replaced { row: 402, by: 13, status: "active".into(), superseded_by: None, cause: None });
        for (row, by) in [(400, 1), (401, 2)] {
            facts.now.insert(row, Now { status: "superseded".into(), superseded_by: Some(by), ..now(None) });
        }
        facts.now.insert(402, now(None));
        facts.extras.push(Extra::Project("r".into()));
        facts.projects.insert("r".into(), ProjectNow { rows: 12, ..Default::default() });

        let plan = decide(&facts);
        assert_eq!(
            plan.kept,
            BTreeMap::from([
                (2, KeepReason::Edited),
                (3, KeepReason::Deleted),
                (4, KeepReason::Restored),
                (5, KeepReason::Judged),
                (6, KeepReason::Judged),
                (7, KeepReason::Referenced),
                (8, KeepReason::Referenced),
                (10, KeepReason::Tied),
            ])
        );
        assert_eq!(plan.retire, BTreeSet::from([1, 9, 11, 13]), "a link the import filled keeps nothing");
        assert_eq!(plan.gone, [12]);
        assert_eq!(plan.restore.iter().map(|r| r.row).collect::<Vec<_>>(), [400], "not under a row that stays, nor one replaced since");
        assert_eq!(plan.unfill, [Extra::Fill { row: 300, column: Column::SupersededBy, value: "11".into() }]);
        assert_eq!(plan.links, [Extra::Piece { save: 41, piece: 1 }], "the pair goes with its rows, and the link to a purged piece with its save");
        assert!(plan.projects.is_empty(), "kept rows still use the root");

        let mut empty = facts.clone();
        empty.rows.retain(|r| r.id == 1);
        empty.projects.insert("r".into(), ProjectNow { rows: 1, ..Default::default() });
        let plan = decide(&empty);
        assert_eq!((plan.retire, plan.projects), (BTreeSet::from([1]), vec!["r".to_owned()]), "a root nothing uses any more goes");
        empty.projects.insert("r".into(), ProjectNow { rows: 1, index_docs: true, ..Default::default() });
        assert!(decide(&empty).projects.is_empty(), "a root whose files are read stays");
    }

    /// Every table an import writes, row by row, as text.
    async fn state(store: &MemoryStore) -> Vec<String> {
        store
            .conn
            .call(|c| {
                let mut out = Vec::new();
                for sql in [
                    "SELECT * FROM memory ORDER BY id",
                    "SELECT rowid, embedding FROM memory_vectors ORDER BY rowid",
                    "SELECT id FROM memory_fts_docsize ORDER BY id",
                    "SELECT * FROM memory_piece ORDER BY save_id, piece",
                    "SELECT * FROM memory_derivation ORDER BY memory_id, source_id",
                    "SELECT * FROM memory_conflict ORDER BY new_id, old_id",
                    "SELECT * FROM project_path ORDER BY id",
                    "SELECT * FROM project_remote ORDER BY remote",
                ] {
                    let mut stmt = c.prepare(sql)?;
                    let width = stmt.column_count();
                    let rows = stmt.query_map([], |r| {
                        (0..width).map(|k| r.get::<_, rusqlite::types::Value>(k).map(|v| format!("{v:?}"))).collect::<rusqlite::Result<Vec<_>>>()
                    })?;
                    for row in rows {
                        out.push(format!("{sql}: {}", row?.join(", ")));
                    }
                }
                Ok::<_, rusqlite::Error>(out)
            })
            .await
            .unwrap()
    }

    async fn undo(store: &MemoryStore, id: i64) -> (UndoReport, tempfile::TempDir) {
        let backups = tempfile::tempdir().unwrap();
        (undo_import(store, backups.path(), id, false).await.unwrap(), backups)
    }

    async fn id_of(store: &MemoryStore, content: &'static str) -> i64 {
        store.conn.call(move |c| c.query_row("SELECT id FROM memory WHERE content = ?1", [content], |r| r.get(0))).await.unwrap()
    }

    async fn sql(store: &MemoryStore, sql: String) {
        store.conn.call(move |c| c.execute_batch(&sql)).await.unwrap();
    }

    /// Each lookup and removal of a row seeks an index, so neither the load
    /// nor the writing of an undo grows with the store.
    #[tokio::test]
    async fn the_undo_lookups_seek_an_index() {
        let store = MemoryStore::new_for_test().await.unwrap();
        crate::memory::portability::fixtures::seed(&store).await;
        let mut cases: Vec<(String, &'static str, &'static str)> = vec![
            ("ROWS_SQL".into(), ROWS_SQL, "idx_memory_import_row"),
            ("REPLACED_SQL".into(), REPLACED_SQL, "idx_memory_import_supersede"),
            ("EXTRAS_SQL".into(), EXTRAS_SQL, "idx_memory_import_extra"),
            ("NOW_SQL".into(), NOW_SQL, "INTEGER PRIMARY KEY"),
            ("DISPUTES_SQL".into(), DISPUTES_SQL, "idx_memory_disputed"),
            ("PAIRS_OF_SQL".into(), PAIRS_OF_SQL, "idx_memory_conflict_old"),
            ("SAVES_OF_SQL".into(), SAVES_OF_SQL, "idx_memory_piece_memory"),
            ("PIECES_SQL".into(), PIECES_SQL, "PRIMARY KEY"),
        ];
        for (sql, index) in NAMED_BY_SQL.into_iter().zip(["idx_memory_superseded", "idx_memory_revived", "idx_memory_derivation_source"]) {
            cases.push((sql.into(), sql, index));
        }
        for (sql, index) in REMOVE_SQL.into_iter().zip(["", "idx_memory_piece_memory", "PRIMARY KEY", "idx_memory_derivation_source", "PRIMARY KEY", "idx_memory_conflict_old", "INTEGER PRIMARY KEY"]) {
            // A vec0 table is searched by its own module.
            if !index.is_empty() {
                cases.push((sql.into(), sql, index));
            }
        }
        let mut wrong = Vec::new();
        for (name, sql, index) in cases {
            let plan: Vec<String> = store
                .conn
                .call(move |c| {
                    let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
                    let binds = vec![1i64; stmt.parameter_count()];
                    let plan = stmt.query_map(rusqlite::params_from_iter(&binds), |r| r.get(3))?.collect::<rusqlite::Result<_>>();
                    plan
                })
                .await
                .unwrap();
            let seeks = plan.iter().any(|l| l.starts_with("SEARCH") && l.contains(index));
            let scans = plan.iter().any(|l| l.starts_with("SCAN") && !l.starts_with("SCAN j") && !l.contains("VIRTUAL TABLE"));
            if !seeks || scans {
                wrong.push(format!("{name} should seek `{index}`: {plan:?}"));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[tokio::test]
    async fn undoing_a_clawg_import_gives_the_store_back() {
        let file = seeded_file().await;
        let dst = destination().await;
        let before = state(&dst).await;
        let (imported, _) = run(&dst, &file, ImportOptions::default()).await;
        let (report, backups) = undo(&dst, imported.import_id.unwrap()).await;

        assert_eq!(state(&dst).await, before);
        assert_eq!((report.removed.len(), report.kept.len(), report.projects.len()), (17, 0, 2), "{report:#?}");
        let snapshot = report.snapshot.clone().unwrap();
        assert!(snapshot.ends_with("-pre-undo.db") && std::fs::read_dir(backups.path()).unwrap().count() == 1, "{snapshot}");

        let (again, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!((again.inserted, again.present, again.import_id), (17, 0, Some(2)), "{again:#?}");
    }

    #[tokio::test]
    async fn undoing_a_foreign_import_gives_the_store_back() {
        let dst = destination().await;
        let before = state(&dst).await;
        let mut export = foreign_export(sample_notes());
        export.relations = vec![relation("old", "new", "supersedes"), relation("b", "loose", "related")];
        let (imported, _) = run_export(&dst, export, ImportOptions::default()).await;
        let pairs = dump(&dst, EVERYTHING).await.conflicts.len();
        assert!(pairs >= 1 && !dump(&dst, EVERYTHING).await.pieces.is_empty(), "{imported:#?}");

        let (report, _) = undo(&dst, imported.import_id.unwrap()).await;
        assert_eq!(state(&dst).await, before, "{report:#?}");
        assert_eq!(report.removed.len(), imported.inserted);
    }

    #[tokio::test]
    async fn undoing_a_rewrite_brings_the_stored_version_back() {
        let dst = destination().await;
        let before = state(&dst).await;
        let (first, _) = run_foreign(&dst, vec![foreign_note("engram:port", "Port", "the daemon listens on 27380")], ImportOptions::default()).await;
        let stored = state(&dst).await;
        let newer = ForeignNote { written: When::At(T0 + 1000), ..foreign_note("engram:port", "Port", "the daemon listens on 27390") };
        let (second, _) = run_foreign(&dst, vec![newer], ImportOptions::default()).await;
        assert_eq!(second.foreign.as_ref().unwrap().revised, ["engram:port"], "{second:#?}");

        let (report, _) = undo(&dst, second.import_id.unwrap()).await;
        assert_eq!(report.restored.len(), 1, "{report:#?}");
        assert_eq!(state(&dst).await, stored, "the stored version is live again, as it was");
        undo(&dst, first.import_id.unwrap()).await;
        assert_eq!(state(&dst).await, before);
    }

    #[tokio::test]
    async fn undoing_fills_empties_the_links_it_filled() {
        let file = seeded_file().await;
        let dst = destination().await;
        run(&dst, &file, ImportOptions::default()).await;
        sql(
            &dst,
            "UPDATE memory SET superseded_by = NULL, revived_from = NULL, disputed_against = NULL;
             DELETE FROM memory_piece; DELETE FROM memory_derivation; DELETE FROM memory_conflict;"
                .into(),
        )
        .await;
        let cut = state(&dst).await;
        let (again, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!((again.inserted, again.import_id), (0, Some(2)), "filling links is a write: {again:#?}");

        let (report, _) = undo(&dst, 2).await;
        assert!(report.removed.is_empty() && !report.unfilled.is_empty() && report.links > 0, "{report:#?}");
        assert_eq!(state(&dst).await, cut);

        run(&dst, &file, ImportOptions::default()).await;
        let revived = id_of(&dst, "Cache policy: none").await;
        sql(&dst, format!("UPDATE memory SET revived_from = 1 WHERE id = {revived}")).await;
        undo(&dst, 3).await;
        let (by, from, against): (i64, Vec<i64>, i64) = dst
            .conn
            .call(|c| {
                let by = c.query_row("SELECT COUNT(*) FROM memory WHERE superseded_by IS NOT NULL", [], |r| r.get(0))?;
                let mut stmt = c.prepare("SELECT revived_from FROM memory WHERE revived_from IS NOT NULL")?;
                let from = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                let against = c.query_row("SELECT COUNT(*) FROM memory WHERE disputed_against IS NOT NULL", [], |r| r.get(0))?;
                Ok::<_, rusqlite::Error>((by, from, against))
            })
            .await
            .unwrap();
        assert_eq!((by, from, against), (0, vec![1], 0), "a link changed since stays as it is");
    }

    #[tokio::test]
    async fn what_was_saved_after_stays() {
        let file = seeded_file().await;
        let dst = destination().await;
        let (imported, _) = run(&dst, &file, ImportOptions::default()).await;
        let later = insert_row(&dst, "claude-code", None, "A row saved after the import", None, 9).await;
        let fact = id_of(&dst, "hello there").await;
        sql(
            &dst,
            format!(
                "INSERT INTO memory_conflict (new_id, old_id, state, created_at) VALUES ({later}, 1, 'pending', 9), ({later}, {fact}, 'pending', 9)"
            ),
        )
        .await;
        undo(&dst, imported.import_id.unwrap()).await;
        let (rows, pairs): (Vec<i64>, Vec<(i64, i64)>) = dst
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT id FROM memory ORDER BY id")?;
                let rows = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                let mut stmt = c.prepare("SELECT new_id, old_id FROM memory_conflict")?;
                let pairs = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
                Ok::<_, rusqlite::Error>((rows, pairs))
            })
            .await
            .unwrap();
        assert_eq!(rows, [1, later]);
        assert_eq!(pairs, [(later, 1)], "a pair the detector left pending judges nothing");
    }

    #[tokio::test]
    async fn the_rows_someone_changed_stay() {
        let file = seeded_file().await;
        let dst = destination().await;
        let (imported, _) = run(&dst, &file, ImportOptions::default()).await;
        let id = |text| id_of(&dst, text);
        let (edited, deleted, restored, judged, judged_old) = (
            id("Codex keeps notes in English").await,
            id("hello there").await,
            id("Daemon port: 27390").await,
            id("Indent Makefiles with spaces").await,
            id("Prefer tabs over spaces in Makefiles").await,
        );
        let (source, piece, tied) =
            (id("Session: wired the exporter").await, id("fn main() { first half of a long save }").await, id("second half of a long save }").await);
        let (moved, accessed, demoted, revised) =
            (id("Cache policy: none").await, id("cargo build failed: linker not found").await, id("Every crate inherits the workspace version").await, id("Storage engine: SQLite with WAL").await);

        sql(&dst, format!("UPDATE memory SET update_count = update_count + 1 WHERE id IN ({edited}, {piece})")).await;
        delete_memory_core(&dst, deleted, MemoryKind::Memory, None).await.unwrap();
        restore_memory_core(&dst, &FakeEmbedder::default(), MemoryKind::Memory, restored, None).await.unwrap();
        sql(&dst, format!("UPDATE memory_conflict SET state = 'judged', verdict = 'compatible' WHERE new_id = {judged}")).await;
        let conclusion = insert_row(&dst, "claude-code", None, "A conclusion drawn since", None, 9).await;
        sql(&dst, format!("INSERT INTO memory_derivation (memory_id, source_id, created_at) VALUES ({conclusion}, {source}, 9)")).await;
        sql(
            &dst,
            "INSERT INTO project_path (id, root_path, created_at) VALUES ('elsewhere', '/work/elsewhere', 0);
             INSERT INTO project (id, name, born, created_at) VALUES ('elsewhere', 'elsewhere', 'root', 0);"
                .into(),
        )
        .await;
        let backups = tempfile::tempdir().unwrap();
        let filter = MoveFilter { ids: vec![moved], ..Default::default() };
        move_memories_core(&dst, backups.path(), &filter, Some("elsewhere".into()), false).await.unwrap();
        sql(&dst, format!("UPDATE memory SET access_count = access_count + 1, last_accessed_at = 9 WHERE id = {accessed}")).await;
        sql(&dst, format!("UPDATE memory SET tier = 3, last_modified_at = 9 WHERE id = {demoted}")).await;
        let newer = insert_row(&dst, "claude-code", None, "Storage engine: SQLite, saved since", Some("storage/engine"), 9).await;
        sql(&dst, format!("UPDATE memory SET status = 'superseded', superseded_by = {newer}, supersede_cause = 'revision' WHERE id = {revised}")).await;

        let (report, _) = undo(&dst, imported.import_id.unwrap()).await;
        let kept: BTreeMap<i64, KeepReason> = report.kept.iter().map(|k| (k.id, k.reason)).collect();
        assert_eq!(
            kept,
            BTreeMap::from([
                (edited, KeepReason::Edited),
                (deleted, KeepReason::Deleted),
                (restored, KeepReason::Restored),
                (judged_old, KeepReason::Judged),
                (judged, KeepReason::Judged),
                (source, KeepReason::Referenced),
                (piece, KeepReason::Edited),
                (tied, KeepReason::Tied),
            ]),
            "{report:#?}"
        );
        for gone in [moved, accessed, demoted, revised] {
            assert!(report.removed.contains(&gone), "{gone}: {report:#?}");
        }
        assert_eq!(report.removed.len() + report.kept.len(), 17);
        let left: i64 = dst.conn.call(|c| c.query_row("SELECT COUNT(*) FROM memory", [], |r| r.get(0))).await.unwrap();
        assert_eq!(left, 1 + 8 + 2, "the row from before, the kept ones and the two saved since");
        assert_eq!(report.projects, Vec::<String>::new(), "kept rows still use both roots");

        let (again, _) = undo(&dst, imported.import_id.unwrap()).await;
        assert_eq!(again.kept, report.kept, "what stayed is weighed again");
    }

    #[tokio::test]
    async fn a_purged_row_does_not_stop_the_undo() {
        let file = seeded_file().await;
        let dst = destination().await;
        let (imported, _) = run(&dst, &file, ImportOptions::default()).await;
        // A purge leaves a piece's link, which says which piece is missing.
        for text in ["hello there", "fn main() { first half of a long save }"] {
            let purged = id_of(&dst, text).await;
            sql(&dst, format!("DELETE FROM memory_vectors WHERE rowid = {purged}; DELETE FROM memory WHERE id = {purged};")).await;
        }
        let (report, _) = undo(&dst, imported.import_id.unwrap()).await;
        assert_eq!((report.removed.len(), report.kept.len()), (15, 0), "{report:#?}");
        let left = dst
            .conn
            .call(|c| {
                let count = |table: &str| c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get::<_, i64>(0));
                Ok::<_, rusqlite::Error>((count("memory_import_row")?, count("memory_piece")?))
            })
            .await
            .unwrap();
        assert_eq!(left, (0, 0), "the purged rows leave the journal, and their save goes whole");
    }

    #[tokio::test]
    async fn an_undo_twice_changes_nothing_the_second_time() {
        let file = seeded_file().await;
        let dst = destination().await;
        let (imported, _) = run(&dst, &file, ImportOptions::default()).await;
        undo(&dst, imported.import_id.unwrap()).await;
        let after = state(&dst).await;
        let (again, backups) = undo(&dst, imported.import_id.unwrap()).await;
        assert_eq!(again, UndoReport { import_id: 1, ..Default::default() });
        assert_eq!(state(&dst).await, after);
        assert_eq!(std::fs::read_dir(backups.path()).map(|d| d.count()).unwrap_or_default(), 0, "no snapshot when nothing changes");
    }

    #[tokio::test]
    async fn an_unknown_or_running_import_is_refused() {
        let dst = destination().await;
        let backups = tempfile::tempdir().unwrap();
        let err = undo_import(&dst, backups.path(), 99, false).await.unwrap_err();
        assert!(matches!(err, UndoError::UnknownImport(99)), "{err}");
        sql(&dst, "INSERT INTO memory_import (import_id, started_at, source) VALUES (5, 0, 'clawg')".into()).await;
        dst.importing.lock().unwrap().insert(5);
        let err = undo_import(&dst, backups.path(), 5, false).await.unwrap_err();
        assert!(matches!(err, UndoError::Running(5)), "{err}");
        dst.importing.lock().unwrap().clear();
        assert!(undo_import(&dst, backups.path(), 5, false).await.is_ok(), "an import cut short is undone as any");
    }

    #[tokio::test]
    async fn a_dry_run_undo_reports_without_writing() {
        let file = seeded_file().await;
        let dst = destination().await;
        let (imported, _) = run(&dst, &file, ImportOptions::default()).await;
        let id = imported.import_id.unwrap();
        let kept = id_of(&dst, "Codex keeps notes in English").await;
        sql(&dst, format!("UPDATE memory SET update_count = update_count + 1 WHERE id = {kept}")).await;
        let before = state(&dst).await;
        let backups = tempfile::tempdir().unwrap();
        let dry = undo_import(&dst, backups.path(), id, true).await.unwrap();
        assert_eq!(state(&dst).await, before);
        assert_eq!(std::fs::read_dir(backups.path()).unwrap().count(), 0);
        let (real, _) = undo(&dst, id).await;
        assert_eq!(UndoReport { dry_run: false, snapshot: real.snapshot.clone(), ..dry }, real);
        assert_eq!(real.kept, [KeptRow { id: kept, reason: KeepReason::Edited }]);
    }
}
