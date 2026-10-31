//! The projects of the store. `project` is the reference every table keys
//! its rows by, whether a folder of this machine leads to it
//! (`project_path`), a remote does (`project_remote`), or nothing does.
//!
//! A row of `memory` written under an id `project` does not know adopts it in
//! the same write, through the two triggers of V27, which name the project as
//! [`project_name`] does.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::memory::clock::TimestampMs;
use crate::memory::identity::Ground;
use crate::memory::merge::{canonical_in, MAX_MERGE_HOPS};
use crate::memory::project::is_project_id;
use crate::memory::project_delete::{deleted_in, deleted_message};
use crate::memory::{MemoryError, MemoryStore};

/// The name a project has until someone gives it one: its id without the
/// `-` and four hex digits `derive_project_id` appends, or the whole id when
/// it does not have that shape. V27's backfill and triggers say the same in
/// SQL.
pub(crate) fn project_name(id: &str) -> &str {
    match id.rsplit_once('-') {
        Some((name, _)) if is_project_id(id) => name,
        _ => id,
    }
}

/// The longest name a project may be given, in characters.
pub const MAX_NAME_CHARS: usize = 80;

/// A project as `project` holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// How it started: `migrated`, `root`, `adopted` or `import`.
    pub born: String,
    /// Whether the user has looked at it: named it, even with the name it had.
    pub reviewed: bool,
    pub created_at: i64,
}

fn project_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project { id: r.get(0)?, name: r.get(1)?, born: r.get(2)?, reviewed: r.get(3)?, created_at: r.get(4)? })
}

/// A folder of this machine that leads to a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedFolder {
    pub root_id: String,
    pub root_path: String,
    pub remote: Option<String>,
    pub index_docs: bool,
    /// Whether the checkout recorded there is still there.
    pub present: bool,
}

/// A suggestion nobody answered yet that a project may be the same as another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedSuggestion {
    pub other_id: String,
    /// The other's name, when it is still a project.
    pub other_name: Option<String>,
    pub reason: String,
    pub created_at: i64,
}

/// A project merged into a listed one: its tombstone, which is not listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedMerged {
    pub id: String,
    pub name: String,
    /// The merge that made it a tombstone, which undoes it.
    pub merge_id: Option<i64>,
    pub merged_at: Option<i64>,
}

/// A delete nobody undid or purged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedDelete {
    pub delete_id: i64,
    pub project_id: String,
    pub name: String,
    pub with_memories: bool,
    pub deleted_at: i64,
    /// The rows of its set that are still deleted: what its purge would remove.
    pub rows: i64,
    /// Whether the project is live again, brought back by a row saved in it.
    pub back: bool,
}

/// A folder of this machine a declared subfolder is in: a folder of its
/// repository and the subpath.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedSubfolderPath {
    pub path: String,
    /// Whether that directory is there.
    pub present: bool,
}

/// A subfolder of a repository declared as a project, listed under both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedSubfolder {
    /// The live project of the repository.
    pub repository_id: String,
    pub subpath: String,
    /// The live project it leads to.
    pub project_id: String,
    pub paths: Vec<ListedSubfolderPath>,
}

/// A project with what leads to it and what it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedProject {
    #[serde(flatten)]
    pub project: Project,
    pub folders: Vec<ListedFolder>,
    /// The remotes whose clones share it.
    pub remotes: Vec<String>,
    /// Its rows of `memory` that are not deleted.
    pub memories: i64,
    pub suggestions: Vec<ListedSuggestion>,
    /// The projects merged into it, which a daemon from before merges does not send.
    #[serde(default)]
    pub merged: Vec<ListedMerged>,
    /// The subfolders declared as it and those of its repository, which a
    /// daemon from before declarations does not send.
    #[serde(default)]
    pub subfolders: Vec<ListedSubfolder>,
}

/// A folder as `project_path` records it.
struct StoredFolder {
    id: String,
    root_path: String,
    project_id: Option<String>,
    remote: Option<String>,
    index_docs: bool,
    git_stamp: Option<String>,
}

/// What [`list_projects`] reads of the store.
struct Stored {
    /// Tombstones and deleted projects too.
    projects: Vec<Project>,
    deleted: HashSet<String>,
    /// The project each tombstone was merged into, and when.
    merged_into: HashMap<String, (String, Option<i64>)>,
    /// The merge that made each tombstone, by its id.
    merges: HashMap<String, i64>,
    /// Oldest first.
    folders: Vec<StoredFolder>,
    /// Remote and project, by remote.
    remotes: Vec<(String, String)>,
    memories: HashMap<String, i64>,
    /// Project, other, reason and date of each suggestion not answered, oldest first.
    suggestions: Vec<(String, String, String, i64)>,
    /// Repository, subpath and project of each declaration, oldest first.
    subfolders: Vec<(String, String, String)>,
}

fn stored_in(c: &Connection) -> rusqlite::Result<Stored> {
    let mut stmt =
        c.prepare("SELECT id, name, born, reviewed, created_at, merged_into, merged_at, deleted_at IS NOT NULL FROM project")?;
    let rows = stmt
        .query_map([], |r| Ok((project_from(r)?, r.get::<_, Option<String>>(5)?, r.get::<_, Option<i64>>(6)?, r.get::<_, bool>(7)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let deleted = rows.iter().filter(|(_, _, _, deleted)| *deleted).map(|(p, ..)| p.id.clone()).collect();
    let merged_into = rows.iter().filter_map(|(p, into, at, _)| Some((p.id.clone(), (into.clone()?, *at)))).collect();
    let projects = rows.into_iter().map(|(p, ..)| p).collect();
    let mut stmt = c.prepare(
        "SELECT m.source, MIN(m.merge_id) FROM project_merge m \
         JOIN project_merge_change ch ON ch.merge_id = m.merge_id AND ch.kind = 'tombstoned' \
         WHERE m.undone_at IS NULL GROUP BY m.source",
    )?;
    let merges = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut stmt =
        c.prepare("SELECT id, root_path, project_id, remote, index_docs, git_stamp FROM project_path ORDER BY created_at, id")?;
    let folders = stmt
        .query_map([], |r| {
            Ok(StoredFolder {
                id: r.get(0)?,
                root_path: r.get(1)?,
                project_id: r.get(2)?,
                remote: r.get(3)?,
                index_docs: r.get(4)?,
                git_stamp: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut stmt = c.prepare("SELECT remote, project_id FROM project_remote ORDER BY remote")?;
    let remotes = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut stmt = c.prepare(
        "SELECT project_id, COUNT(*) FROM memory WHERE deleted_at IS NULL AND project_id IS NOT NULL GROUP BY project_id",
    )?;
    let memories = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut stmt = c.prepare(
        "SELECT project_id, other_id, reason, created_at FROM project_suggestion WHERE answer IS NULL ORDER BY created_at, rowid",
    )?;
    let suggestions = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut stmt = c.prepare("SELECT parent_id, subpath, project_id FROM project_subfolder ORDER BY created_at, parent_id, subpath")?;
    let subfolders = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(Stored { projects, deleted, merged_into, merges, folders, remotes, memories, suggestions, subfolders })
}

/// The live project `id` leads to through `merged_into`, as
/// [`canonical_in`] follows it in the store.
fn follow(merged_into: &HashMap<String, (String, Option<i64>)>, id: &str) -> String {
    let mut at = id;
    let mut seen = vec![id];
    while let Some((into, _)) = merged_into.get(at) {
        if seen.contains(&into.as_str()) || seen.len() > MAX_MERGE_HOPS {
            return id.to_string();
        }
        seen.push(into);
        at = into;
    }
    at.to_string()
}

/// What the disk says of a folder: whether its checkout is still there, and
/// the key of its remote when no registration recorded its project.
struct Found {
    present: bool,
    key: Option<String>,
}

/// Every project, with its folders, its remotes, how many memories it holds,
/// its declared subfolders and the suggestions about it nobody answered, by
/// name. Whether each recorded checkout is still there, the remote of a folder
/// no registration recorded a project in, and whether each path of a declared
/// subfolder is there are read from the disk on a blocking thread.
pub async fn list_projects(store: &MemoryStore, ground: Arc<dyn Ground>) -> Result<Vec<ListedProject>, MemoryError> {
    let stored = store.conn.call(|c| stored_in(c)).await.map_err(MemoryError::Connection)?;
    tokio::task::spawn_blocking(move || {
        let found: Vec<Found> = stored
            .folders
            .iter()
            .map(|f| {
                let path = PathBuf::from(&f.root_path);
                Found {
                    present: ground.recorded_checkout_present(&path, f.git_stamp.as_deref()),
                    key: if f.project_id.is_none() { ground.checkout(&path).key } else { None },
                }
            })
            .collect();
        listed(&stored, &found, &|path| path.is_dir())
    })
    .await
    .map_err(|e| MemoryError::Internal(format!("could not look at the project folders: {e}")))
}

/// The listing of `stored`, with `found` for each of its folders in order and
/// `is_dir` for the paths of its declared subfolders.
fn listed(stored: &Stored, found: &[Found], is_dir: &dyn Fn(&std::path::Path) -> bool) -> Vec<ListedProject> {
    let live = |id: &str| follow(&stored.merged_into, id);
    let mut listed: Vec<ListedProject> = stored
        .projects
        .iter()
        .filter(|p| !stored.merged_into.contains_key(&p.id) && !stored.deleted.contains(&p.id))
        .map(|p| ListedProject {
            project: p.clone(),
            folders: Vec::new(),
            remotes: Vec::new(),
            memories: stored.memories.get(&p.id).copied().unwrap_or(0),
            suggestions: Vec::new(),
            merged: Vec::new(),
            subfolders: Vec::new(),
        })
        .collect();
    let at: HashMap<String, usize> = listed.iter().enumerate().map(|(i, l)| (l.project.id.clone(), i)).collect();
    let names: HashMap<&str, &str> = stored.projects.iter().map(|p| (p.id.as_str(), p.name.as_str())).collect();
    let bound: HashMap<&str, &str> = stored.remotes.iter().map(|(r, p)| (r.as_str(), p.as_str())).collect();
    for (folder, found) in stored.folders.iter().zip(found) {
        let remote = folder.remote.as_deref().or(found.key.as_deref());
        // The one a registration recorded, else the one the clones of its
        // remote share, else its own id, past any merge. A folder that leads
        // to no project (nothing was saved in it before V27) waits for a
        // registration to make it one.
        let project = folder.project_id.as_deref().or_else(|| remote.and_then(|r| bound.get(r).copied())).unwrap_or(&folder.id);
        if let Some(&i) = at.get(&live(project)) {
            listed[i].folders.push(ListedFolder {
                root_id: folder.id.clone(),
                root_path: folder.root_path.clone(),
                remote: remote.map(str::to_string),
                index_docs: folder.index_docs,
                present: found.present,
            });
        }
    }
    for (remote, project) in &stored.remotes {
        if let Some(&i) = at.get(&live(project)) {
            listed[i].remotes.push(remote.clone());
        }
    }
    for (a, b, reason, created_at) in pending_pairs(&stored.suggestions, &stored.merged_into, &stored.deleted) {
        for (this, other) in [(&a, &b), (&b, &a)] {
            if let Some(&i) = at.get(this) {
                let other_name = at.get(other).map(|&o| listed[o].project.name.clone());
                listed[i].suggestions.push(ListedSuggestion {
                    other_id: other.clone(),
                    other_name,
                    reason: reason.clone(),
                    created_at,
                });
            }
        }
    }
    for (id, (_, merged_at)) in &stored.merged_into {
        if let Some(&i) = at.get(&live(id)) {
            listed[i].merged.push(ListedMerged {
                id: id.clone(),
                name: names.get(id.as_str()).map(|n| n.to_string()).unwrap_or_default(),
                merge_id: stored.merges.get(id).copied(),
                merged_at: *merged_at,
            });
        }
    }
    for (parent, subpath, project) in &stored.subfolders {
        let (repository, project) = (live(parent), live(project));
        let Some(&r) = at.get(&repository) else { continue };
        let paths = listed[r]
            .folders
            .iter()
            .map(|folder| {
                let path = subpath.split('/').fold(PathBuf::from(&folder.root_path), |path, part| path.join(part));
                ListedSubfolderPath { present: is_dir(&path), path: path.display().to_string() }
            })
            .collect();
        let entry = ListedSubfolder { repository_id: repository.clone(), subpath: subpath.clone(), project_id: project.clone(), paths };
        let sides = if project == repository { vec![r] } else { std::iter::once(r).chain(at.get(&project).copied()).collect() };
        for i in sides {
            if !listed[i].subfolders.iter().any(|s| s.repository_id == entry.repository_id && s.subpath == entry.subpath) {
                listed[i].subfolders.push(entry.clone());
            }
        }
    }
    for l in &mut listed {
        l.merged.sort_by(|a, b| (a.merged_at, &a.id).cmp(&(b.merged_at, &b.id)));
    }
    listed.sort_by_cached_key(|l| (l.project.name.to_lowercase(), l.project.id.clone()));
    listed
}

/// The pending `suggestions` (project, other, reason, date, oldest first) as
/// someone is asked them: each side the live project it leads to, one per
/// pair, none whose sides became one project, and none with a deleted side
/// until that side comes back.
fn pending_pairs(
    suggestions: &[(String, String, String, i64)],
    merged_into: &HashMap<String, (String, Option<i64>)>,
    deleted: &HashSet<String>,
) -> Vec<(String, String, String, i64)> {
    let mut pairs: Vec<(String, String, String, i64)> = Vec::new();
    for (a, b, reason, created_at) in suggestions {
        let (a, b) = (follow(merged_into, a), follow(merged_into, b));
        if a == b || deleted.contains(&a) || deleted.contains(&b) {
            continue;
        }
        if !pairs.iter().any(|(x, y, ..)| (*x == a && *y == b) || (*x == b && *y == a)) {
            pairs.push((a, b, reason.clone(), *created_at));
        }
    }
    pairs
}

/// The suggestions nobody answered about project `id`, read as
/// [`list_projects`] lists them, each with the other project.
pub async fn pending_suggestions(store: &MemoryStore, id: &str) -> Result<Vec<ListedSuggestion>, MemoryError> {
    let id = id.to_string();
    store.conn.call(move |c| pending_suggestions_in(c, &id)).await.map_err(MemoryError::Connection)
}

fn pending_suggestions_in(c: &Connection, id: &str) -> rusqlite::Result<Vec<ListedSuggestion>> {
    let mut stmt = c.prepare("SELECT id, name, merged_into, merged_at, deleted_at IS NOT NULL FROM project")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<i64>>(3)?, r.get(4)?))
        })?
        .collect::<rusqlite::Result<Vec<(String, String, Option<String>, Option<i64>, bool)>>>()?;
    let names: HashMap<&str, &str> = rows.iter().map(|(id, name, ..)| (id.as_str(), name.as_str())).collect();
    let merged_into = rows.iter().filter_map(|(id, _, into, at, _)| Some((id.clone(), (into.clone()?, *at)))).collect();
    let deleted = rows.iter().filter(|(.., deleted)| *deleted).map(|(id, ..)| id.clone()).collect();
    let mut stmt = c.prepare(
        "SELECT project_id, other_id, reason, created_at FROM project_suggestion WHERE answer IS NULL ORDER BY created_at, rowid",
    )?;
    let suggestions = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut found = Vec::new();
    for (a, b, reason, created_at) in pending_pairs(&suggestions, &merged_into, &deleted) {
        let other = if a == id { b } else if b == id { a } else { continue };
        let other_name = names.get(other.as_str()).map(|n| n.to_string());
        found.push(ListedSuggestion { other_id: other, other_name, reason, created_at });
    }
    Ok(found)
}

/// The suggestion nobody answered between projects `a` and `b`, read through
/// the merges since it was made, after checking both as [`keep_apart`] does:
/// each must be a live project, not a tombstone and not deleted.
pub async fn suggestion_between(store: &MemoryStore, a: &str, b: &str) -> Result<Option<ListedSuggestion>, KeepApartError> {
    let (a, b) = (a.to_string(), b.to_string());
    store
        .conn
        .call(move |c| {
            let sql = |e: rusqlite::Error| KeepApartError::Store(MemoryError::Sqlite(e));
            for id in [&a, &b] {
                let known: bool =
                    c.query_row("SELECT EXISTS (SELECT 1 FROM project WHERE id = ?1)", [id], |r| r.get(0)).map_err(sql)?;
                if !known {
                    return Err(KeepApartError::NotFound(id.clone()));
                }
                let into = canonical_in(c, id).map_err(sql)?;
                if into != *id {
                    return Err(KeepApartError::Merged { id: id.clone(), into });
                }
                if let Some(deleted) = deleted_in(c, id).map_err(sql)? {
                    return Err(KeepApartError::Deleted { id: id.clone(), delete_id: deleted.delete_id });
                }
            }
            if a == b {
                return Err(KeepApartError::Same(a));
            }
            Ok(pending_suggestions_in(c, &a).map_err(sql)?.into_iter().find(|s| s.other_id == b))
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(e) => e,
            other => KeepApartError::Store(MemoryError::Backend(other.to_string())),
        })
}

/// Every delete nobody undid or purged, oldest first, with the project it
/// took and whether a row saved since brought the project back.
pub async fn list_deletes(store: &MemoryStore) -> Result<Vec<ListedDelete>, MemoryError> {
    store
        .conn
        .call(|c| {
            let mut stmt = c.prepare(
                "SELECT d.delete_id, d.project_id, COALESCE(p.name, d.project_id), d.with_memories, d.deleted_at, \
                   (SELECT COUNT(*) FROM project_delete_row r JOIN memory m ON r.tbl = 'memory' AND m.id = r.row_id \
                     WHERE r.delete_id = d.delete_id AND m.deleted_at IS NOT NULL) \
                 + (SELECT COUNT(*) FROM project_delete_row r JOIN episodic_trajectory m ON r.tbl = 'episodic_trajectory' AND m.id = r.row_id \
                     WHERE r.delete_id = d.delete_id AND m.deleted_at IS NOT NULL) \
                 + (SELECT COUNT(*) FROM project_delete_row r JOIN procedural_skill m ON r.tbl = 'procedural_skill' AND m.id = r.row_id \
                     WHERE r.delete_id = d.delete_id AND m.deleted_at IS NOT NULL), \
                   p.deleted_at IS NULL \
                 FROM project_delete d LEFT JOIN project p ON p.id = d.project_id \
                 WHERE d.undone_at IS NULL AND d.purged_at IS NULL ORDER BY d.delete_id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(ListedDelete {
                    delete_id: r.get(0)?,
                    project_id: r.get(1)?,
                    name: r.get(2)?,
                    with_memories: r.get(3)?,
                    deleted_at: r.get(4)?,
                    rows: r.get(5)?,
                    back: r.get(6)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .await
        .map_err(MemoryError::Connection)
}

/// Why a name may not be a project's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("a project name must not be empty")]
    Empty,
    #[error("a project name has at most {MAX_NAME_CHARS} characters")]
    TooLong,
    #[error("a project name must not hold control characters")]
    Control,
}

#[derive(Debug, thiserror::Error)]
pub enum RenameError {
    #[error("no project has the id {0}")]
    NotFound(String),
    #[error(transparent)]
    Name(#[from] NameError),
    #[error("{name:?} is already the name or the id of project {other}")]
    Taken { name: String, other: String },
    #[error("project {id} was merged into {into}: rename {into} instead")]
    Merged { id: String, into: String },
    /// `delete_id` is the open delete that took it.
    #[error("{}", deleted_message(.id, .delete_id))]
    Deleted { id: String, delete_id: Option<i64> },
    #[error(transparent)]
    Store(#[from] MemoryError),
}

/// `name` without the spaces around it, when it may name a project.
fn checked_name(name: &str) -> Result<&str, NameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(NameError::TooLong);
    }
    if name.chars().any(char::is_control) {
        return Err(NameError::Control);
    }
    Ok(name)
}

/// Gives project `id` the name `name`, without the spaces around it, and
/// marks it reviewed, even when the name is the one it had. Nothing else
/// changes, its id least of all, so what was saved under it stays there.
/// Two projects may share a name a registration gave them, but not one the
/// user chose: the name or the id of another project, in any case, is
/// refused.
pub async fn rename_project(store: &MemoryStore, id: &str, name: &str) -> Result<Project, RenameError> {
    let (id, name) = (id.to_string(), checked_name(name)?.to_string());
    store
        .conn
        .call(move |c| {
            let sql = |e: rusqlite::Error| RenameError::Store(MemoryError::Sqlite(e));
            let tx = c.transaction().map_err(sql)?;
            let project = tx
                .query_row("SELECT id, name, born, reviewed, created_at FROM project WHERE id = ?1", [&id], project_from)
                .optional()
                .map_err(sql)?
                .ok_or_else(|| RenameError::NotFound(id.clone()))?;
            // Renaming the project it went into by an old id would surprise.
            let into = canonical_in(&tx, &id).map_err(sql)?;
            if into != id {
                return Err(RenameError::Merged { id, into });
            }
            if let Some(deleted) = deleted_in(&tx, &id).map_err(sql)? {
                return Err(RenameError::Deleted { id, delete_id: deleted.delete_id });
            }
            // The name of a tombstone or of a deleted project is free: nobody
            // chooses it any more. Its id is not: it leads to the project it
            // went into, or to the one that comes back.
            let others: Vec<(String, Option<String>)> = {
                let mut stmt = tx
                    .prepare("SELECT id, CASE WHEN merged_into IS NULL AND deleted_at IS NULL THEN name END FROM project WHERE id <> ?1")
                    .map_err(sql)?;
                let rows = stmt.query_map([&id], |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)?;
                rows.collect::<rusqlite::Result<_>>().map_err(sql)?
            };
            // In Rust, since SQLite's lower() folds ASCII alone.
            let wanted = name.to_lowercase();
            let holds = |(o, n): &(String, Option<String>)| o.to_lowercase() == wanted || n.as_ref().is_some_and(|n| n.to_lowercase() == wanted);
            if let Some((other, _)) = others.into_iter().find(holds) {
                return Err(RenameError::Taken { name, other });
            }
            tx.execute("UPDATE project SET name = ?2, reviewed = 1 WHERE id = ?1", rusqlite::params![id, name]).map_err(sql)?;
            tx.commit().map_err(sql)?;
            Ok(Project { name, reviewed: true, ..project })
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(e) => e,
            other => RenameError::Store(MemoryError::Backend(other.to_string())),
        })
}

/// What [`keep_apart`] answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptApart {
    pub project_id: String,
    pub other_id: String,
    /// How many suggestions pending between the two it answered.
    pub answered: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum KeepApartError {
    #[error("no project has the id {0}")]
    NotFound(String),
    #[error("project {0} cannot be kept apart from itself")]
    Same(String),
    #[error("project {id} was merged into {into}")]
    Merged { id: String, into: String },
    #[error("{}", deleted_message(.id, .delete_id))]
    Deleted { id: String, delete_id: Option<i64> },
    #[error(transparent)]
    Store(#[from] MemoryError),
}

/// Answers `apart` every suggestion pending between projects `a` and `b`,
/// read through the merges since it was made, and marks both reviewed. With
/// none pending it records the pair answered, with reason `user`, which keeps
/// a registration from suggesting it later: a suggestion is made once per
/// pair.
pub async fn keep_apart(store: &MemoryStore, a: &str, b: &str) -> Result<KeptApart, KeepApartError> {
    let (a, b) = (a.to_string(), b.to_string());
    store
        .conn
        .call(move |c| {
            let sql = |e: rusqlite::Error| KeepApartError::Store(MemoryError::Sqlite(e));
            let tx = c.transaction().map_err(sql)?;
            for id in [&a, &b] {
                let known: bool =
                    tx.query_row("SELECT EXISTS (SELECT 1 FROM project WHERE id = ?1)", [id], |r| r.get(0)).map_err(sql)?;
                if !known {
                    return Err(KeepApartError::NotFound(id.clone()));
                }
                let into = canonical_in(&tx, id).map_err(sql)?;
                if into != *id {
                    return Err(KeepApartError::Merged { id: id.clone(), into });
                }
                if let Some(deleted) = deleted_in(&tx, id).map_err(sql)? {
                    return Err(KeepApartError::Deleted { id: id.clone(), delete_id: deleted.delete_id });
                }
            }
            if a == b {
                return Err(KeepApartError::Same(a));
            }
            let now = TimestampMs::now();
            let open: Vec<(i64, String, String)> = {
                let mut stmt = tx
                    .prepare("SELECT rowid, project_id, other_id FROM project_suggestion WHERE answer IS NULL")
                    .map_err(sql)?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(sql)?;
                rows.collect::<rusqlite::Result<_>>().map_err(sql)?
            };
            let mut answered = 0;
            for (rowid, one, other) in open {
                let sides = (canonical_in(&tx, &one).map_err(sql)?, canonical_in(&tx, &other).map_err(sql)?);
                if (sides.0 == a && sides.1 == b) || (sides.0 == b && sides.1 == a) {
                    tx.execute(
                        "UPDATE project_suggestion SET answer = 'apart', answered_at = ?2 WHERE rowid = ?1",
                        rusqlite::params![rowid, now],
                    )
                    .map_err(sql)?;
                    answered += 1;
                }
            }
            if answered == 0 {
                tx.execute(
                    "INSERT OR IGNORE INTO project_suggestion (project_id, other_id, reason, created_at, answer, answered_at) \
                     VALUES (?1, ?2, 'user', ?3, 'apart', ?3)",
                    rusqlite::params![a, b, now],
                )
                .map_err(sql)?;
            }
            tx.execute("UPDATE project SET reviewed = 1 WHERE id IN (?1, ?2)", [&a, &b]).map_err(sql)?;
            tx.commit().map_err(sql)?;
            Ok(KeptApart { project_id: a, other_id: b, answered })
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(e) => e,
            other => KeepApartError::Store(MemoryError::Backend(other.to_string())),
        })
}

/// What tells two names apart for a person: the name in Unicode lower case,
/// without anything that is not a letter or a digit. `Mi App`, `mi-app` and
/// `mi_app` have one key.
pub fn name_key(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The longest slug of a project id made from a name.
const MAX_SLUG_CHARS: usize = 40;

/// How many ids [`resolve_project`] tries for a new project before giving up.
const NEW_ID_TRIES: u32 = 64;

/// The `n`-th id a project named `name`, made at `now_ms`, may take: the
/// shape of [`derive_project_id`](crate::memory::identity::derive_project_id),
/// a slug of ASCII letters, digits and `-`, then four hex digits of a hash.
/// The time is in the hash so a new project never takes the id of a deleted
/// one of the same name, whose name is free and whose id is not.
pub(crate) fn new_project_id(name: &str, now_ms: i64, n: u32) -> String {
    use sha2::{Digest, Sha256};
    let mut slug = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(MAX_SLUG_CHARS).collect();
    let slug = match slug.trim_end_matches('-') {
        "" => "project",
        slug => slug,
    };
    let digest = Sha256::digest(format!("{name}\0{now_ms}\0{n}").as_bytes());
    format!("{slug}-{:02x}{:02x}", digest[0], digest[1])
}

/// Who asks for a project that does not exist yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Born {
    /// An agent saving under a name: the user has not seen it yet.
    #[default]
    Agent,
    /// The user, through the CLI: it is reviewed.
    User,
}

impl Born {
    fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::User => "user",
        }
    }
}

/// How [`resolve_project`] treats a name no project holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResolveOptions {
    /// Create the project when nothing holds or resembles the name.
    pub create: bool,
    /// With `create`, create it even when other projects resemble it.
    pub new_project: bool,
    pub born: Born,
}

/// Why a reference resolved to no project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// Two or more live projects have the name.
    Ambiguous,
    /// No project has it, and the candidates resemble it.
    Similar,
    /// Nothing has or resembles it, and nothing was to be created.
    Unknown,
}

/// A folder of a resolved project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedFolder {
    pub root_id: String,
    pub index_docs: bool,
}

/// A project a reference may mean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub deleted: bool,
    /// The open delete of a deleted one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_id: Option<i64>,
}

/// What a reference to a project leads to: a project, or why none and the
/// projects it may mean.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub project_id: Option<String>,
    pub name: Option<String>,
    /// Whether this call created it.
    #[serde(default)]
    pub created: bool,
    /// Whether it is deleted: a save under it brings it back.
    #[serde(default)]
    pub deleted: bool,
    /// Its folders, oldest first.
    #[serde(default)]
    pub folders: Vec<ResolvedFolder>,
    #[serde(default)]
    pub refused: Option<Refusal>,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error(transparent)]
    Name(#[from] NameError),
    #[error("every id a project named {0:?} may take is taken")]
    NoFreeId(String),
    #[error(transparent)]
    Store(#[from] MemoryError),
}

/// `(id, name, deleted)` of every project that is not a tombstone.
fn untombed_in(c: &Connection) -> rusqlite::Result<Vec<(String, String, bool)>> {
    let mut stmt = c.prepare("SELECT id, name, deleted_at IS NOT NULL FROM project WHERE merged_into IS NULL ORDER BY created_at, id")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    rows.collect()
}

/// The ids of the live projects named `name`, in any case, oldest first.
fn named_in(all: &[(String, String, bool)], name: &str) -> Vec<String> {
    // In Rust, since SQLite's lower() folds ASCII alone.
    let wanted = name.to_lowercase();
    all.iter().filter(|(_, n, deleted)| !deleted && n.to_lowercase() == wanted).map(|(id, ..)| id.clone()).collect()
}

/// The folders that lead to `project`, oldest first: those a registration
/// recorded it in, and one from before V27 with its id. A project with none
/// of its own that a declared subfolder leads to has the folders of the
/// repositories it is declared in, whose docs index holds its files.
fn folders_of_in(c: &Connection, project: &str) -> rusqlite::Result<Vec<ResolvedFolder>> {
    let own = own_folders_in(c, project)?;
    if !own.is_empty() {
        return Ok(own);
    }
    let mut repositories: Vec<String> = Vec::new();
    for declared in crate::memory::subfolder::leading_to_in(c, project)? {
        if declared.repository_id != project && !repositories.contains(&declared.repository_id) {
            repositories.push(declared.repository_id);
        }
    }
    let mut folders: Vec<(i64, ResolvedFolder)> = Vec::new();
    for repository in repositories {
        folders.extend(own_folders_dated_in(c, &repository)?);
    }
    folders.sort_by(|a, b| (a.0, &a.1.root_id).cmp(&(b.0, &b.1.root_id)));
    folders.dedup_by(|a, b| a.1 == b.1);
    Ok(folders.into_iter().map(|(_, f)| f).collect())
}

fn own_folders_in(c: &Connection, project: &str) -> rusqlite::Result<Vec<ResolvedFolder>> {
    Ok(own_folders_dated_in(c, project)?.into_iter().map(|(_, f)| f).collect())
}

fn own_folders_dated_in(c: &Connection, project: &str) -> rusqlite::Result<Vec<(i64, ResolvedFolder)>> {
    let mut stmt = c.prepare(
        "SELECT created_at, id, index_docs FROM project_path \
         WHERE project_id = ?1 OR (project_id IS NULL AND id = ?1) ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map([project], |r| Ok((r.get(0)?, ResolvedFolder { root_id: r.get(1)?, index_docs: r.get(2)? })))?;
    rows.collect()
}

/// What `reference` (an id or a name) leads to, as an agent or the user
/// names a project: the project with that id, past any merge (a deleted one
/// too, which a save brings back); else the one live project with that name
/// in any case; else nothing, with the projects that resemble it (see
/// [`name_key`]), deleted ones among them, as candidates. With
/// `options.create` a name nothing holds or resembles, or with
/// `options.new_project` one that only resembles others, becomes a project
/// without a folder, in the same transaction that looked. Nothing else is
/// written.
pub async fn resolve_project(store: &MemoryStore, reference: &str, options: ResolveOptions) -> Result<Resolution, ResolveError> {
    let reference = reference.trim().to_string();
    store
        .conn
        .call(move |c| {
            let sql = |e: rusqlite::Error| ResolveError::Store(MemoryError::Sqlite(e));
            let tx = c.transaction().map_err(sql)?;
            let resolved = |id: String, created: bool| -> Result<Resolution, ResolveError> {
                let (name, deleted): (String, bool) = tx
                    .query_row("SELECT name, deleted_at IS NOT NULL FROM project WHERE id = ?1", [&id], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(sql)?;
                let folders = folders_of_in(&tx, &id).map_err(sql)?;
                Ok(Resolution { project_id: Some(id), name: Some(name), created, deleted, folders, ..Resolution::default() })
            };
            let known: bool =
                tx.query_row("SELECT EXISTS (SELECT 1 FROM project WHERE id = ?1)", [&reference], |r| r.get(0)).map_err(sql)?;
            if known {
                let live = canonical_in(&tx, &reference).map_err(sql)?;
                return resolved(live, false);
            }
            // Only a name is bounded: an id keeps the whole name of its folder.
            let reference = checked_name(&reference)?;
            let all = untombed_in(&tx).map_err(sql)?;
            let candidate = |id: &str, name: &str, deleted: bool| -> Result<Candidate, ResolveError> {
                let delete_id = if deleted { deleted_in(&tx, id).map_err(sql)?.and_then(|d| d.delete_id) } else { None };
                Ok(Candidate { id: id.to_string(), name: name.to_string(), deleted, delete_id })
            };
            match named_in(&all, &reference).as_slice() {
                [one] => return resolved(one.clone(), false),
                [] => {}
                many => {
                    let candidates = all
                        .iter()
                        .filter(|(id, ..)| many.contains(id))
                        .map(|(id, name, deleted)| candidate(id, name, *deleted))
                        .collect::<Result<_, _>>()?;
                    return Ok(Resolution { refused: Some(Refusal::Ambiguous), candidates, ..Resolution::default() });
                }
            }
            let key = name_key(&reference);
            let similar = all
                .iter()
                .filter(|(_, name, _)| !key.is_empty() && name_key(name) == key)
                .map(|(id, name, deleted)| candidate(id, name, *deleted))
                .collect::<Result<Vec<_>, _>>()?;
            let refused = match (options.create, options.new_project, similar.is_empty()) {
                (true, true, _) | (true, false, true) => None,
                (_, _, false) => Some(Refusal::Similar),
                (false, _, true) => Some(Refusal::Unknown),
            };
            if let Some(refused) = refused {
                return Ok(Resolution { refused: Some(refused), candidates: similar, ..Resolution::default() });
            }
            let now = TimestampMs::now();
            let mut id = None;
            for n in 0..NEW_ID_TRIES {
                let tried = new_project_id(&reference, now.as_millis_i64(), n);
                let taken: bool = tx
                    .query_row(
                        "SELECT EXISTS (SELECT 1 FROM project WHERE id = ?1) OR EXISTS (SELECT 1 FROM project_path WHERE id = ?1)",
                        [&tried],
                        |r| r.get(0),
                    )
                    .map_err(sql)?;
                if !taken {
                    id = Some(tried);
                    break;
                }
            }
            let id = id.ok_or_else(|| ResolveError::NoFreeId(reference.to_string()))?;
            tx.execute(
                "INSERT INTO project (id, name, born, reviewed, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![id, reference, options.born.as_str(), options.born == Born::User, now],
            )
            .map_err(sql)?;
            log::info!("created project {id} ({reference:?}) for the {}", options.born.as_str());
            let resolution = resolved(id, true)?;
            tx.commit().map_err(sql)?;
            Ok(resolution)
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(e) => e,
            other => ResolveError::Store(MemoryError::Backend(other.to_string())),
        })
}

/// What a reference names where a project must already exist, as a move's
/// target or a folder's project: the project with that id as it is, a
/// tombstone or a deleted one too (what takes it judges them), else the one
/// live project with that name in any case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Named {
    Project(String),
    /// The live projects that share the name, oldest first.
    Ambiguous(Vec<String>),
    Unknown,
}

/// See [`Named`]. Neither creates nor looks for projects that resemble it.
pub async fn find_project(store: &MemoryStore, reference: &str) -> Result<Named, MemoryError> {
    let reference = reference.trim().to_string();
    store
        .conn
        .call(move |c| {
            let known: bool = c.query_row("SELECT EXISTS (SELECT 1 FROM project WHERE id = ?1)", [&reference], |r| r.get(0))?;
            if known {
                return Ok(Named::Project(reference));
            }
            Ok(match named_in(&untombed_in(c)?, &reference).as_slice() {
                [] => Named::Unknown,
                [one] => Named::Project(one.clone()),
                many => Named::Ambiguous(many.to_vec()),
            })
        })
        .await
        .map_err(MemoryError::Connection)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use super::*;
    use crate::memory::events::{MemoryEvent, MemorySource};
    use crate::memory::identity::{FakeGround, RootCommit};
    use crate::memory::relocate::{move_memories_core, undo_move_core, MoveFilter};
    use crate::memory::worker::save_for_test;
    use crate::memory::MemoryStore;
    use crate::models::chat::Role;

    /// `(name, born, reviewed, created_at)` of project `id`.
    type Project = (String, String, i64, i64);

    async fn project(store: &MemoryStore, id: &str) -> Option<Project> {
        use rusqlite::OptionalExtension;
        let id = id.to_string();
        store
            .conn
            .call(move |c| {
                c.query_row("SELECT name, born, reviewed, created_at FROM project WHERE id = ?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .optional()
            })
            .await
            .unwrap()
    }

    async fn row(store: &MemoryStore, id: i64) -> (Option<String>, i64) {
        store
            .conn
            .call(move |c| c.query_row("SELECT project_id, created_at FROM memory WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))))
            .await
            .unwrap()
    }

    fn fact(content: &str, project: Option<&str>) -> MemoryEvent {
        MemoryEvent {
            agent_id: "agent-a".into(),
            project_id: project.map(str::to_string),
            role: Role::User,
            content: content.into(),
            is_error: None,
            source: MemorySource::Explicit,
            timestamp_ms: 1,
            memory_type: Some("semantic_fact".into()),
            importance: None,
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
            session_id: None,
        }
    }

    async fn save(store: &Arc<MemoryStore>, evt: MemoryEvent) -> i64 {
        let report = save_for_test(store, &evt).await;
        assert_eq!(report.failure, None, "a row under an unknown project is not refused");
        assert_eq!(report.memories.len(), 1);
        report.memories[0].id
    }

    #[tokio::test]
    async fn a_row_saved_under_an_unknown_project_adopts_it() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());

        let id = save(&store, fact("a note from a bridge whose registration failed", Some("app-1a2b"))).await;

        let (project_id, created_at) = row(&store, id).await;
        assert_eq!(project_id.as_deref(), Some("app-1a2b"));
        assert_eq!(project(&store, "app-1a2b").await, Some(("app".into(), "adopted".into(), 0, created_at)));
    }

    #[tokio::test]
    async fn a_row_moved_to_an_unknown_project_adopts_it() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        save(&store, fact("a note that makes the target a project", Some("target-2b3c"))).await;
        let id = save(&store, fact("a note of an id from before projects", Some("old-1a2b"))).await;
        let backups = crate::test_dir::test_dir();
        let filter = MoveFilter { ids: vec![id], ..MoveFilter::default() };
        let moved = move_memories_core(&store, backups.path(), &filter, Some("target-2b3c".into()), false).await.unwrap();
        // As an id a store from before V27 only names in its history.
        store.conn.call(|c| c.execute("DELETE FROM project WHERE id = 'old-1a2b'", [])).await.unwrap();

        undo_move_core(&store, backups.path(), moved.move_id.expect("a move"), false).await.unwrap();

        let (project_id, created_at) = row(&store, id).await;
        assert_eq!(project_id.as_deref(), Some("old-1a2b"));
        assert_eq!(project(&store, "old-1a2b").await, Some(("old".into(), "adopted".into(), 0, created_at)));
    }

    #[tokio::test]
    async fn the_name_of_an_adopted_id_is_the_one_rust_gives() {
        assert_eq!(project_name("claw-garden-00ff"), "claw-garden");
        assert_eq!(project_name("release-2024"), "release");
        assert_eq!(project_name("repo-1A2B"), "repo-1A2B");
        assert_eq!(project_name("p"), "p");
        let ids = [
            "repo-1a2b", "claw-garden-00ff", "release-2024", "a--1a2b", "ñandú-0f0f", "repo", "p", "-1a2b", "repo-1A2B",
            "repo-1a2", "repo-1a2bc", "repo-zzzz", "repo 1a2b", "repo-1a2b-",
        ];
        let store = MemoryStore::new_for_test().await.unwrap();
        let named: Vec<(String, String)> = store
            .conn
            .call(move |c| {
                // Every other id arrives by a change of project, the other trigger.
                for (n, id) in ids.iter().enumerate() {
                    let inserted = if n % 2 == 0 { Some(*id) } else { None };
                    c.execute(
                        "INSERT INTO memory \
                         (scope, agent_id, project_id, content, memory_type, source, \
                          created_at, created_by_type, created_by_ref, \
                          last_modified_at, last_accessed_at, importance, tier, status) \
                         VALUES ('agent', 'a', ?1, 'note ' || ?2, 'semantic_fact', 'explicit', 1, 'agent', 'a', \
                          1, 1, 5.0, 3, 'active')",
                        rusqlite::params![inserted, n],
                    )?;
                    if inserted.is_none() {
                        c.execute("UPDATE memory SET project_id = ?1 WHERE id = ?2", rusqlite::params![id, c.last_insert_rowid()])?;
                    }
                }
                let mut stmt = c.prepare("SELECT id, name FROM project")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<rusqlite::Result<_>>()
            })
            .await
            .unwrap();

        assert_eq!(named.len(), ids.len());
        for (id, name) in named {
            assert_eq!(name, project_name(&id), "{id}");
        }
    }

    /// Inserts each `(id, name, reviewed)` into `project`.
    async fn projects(store: &MemoryStore, rows: &'static [(&'static str, &'static str, i64)]) {
        store
            .conn
            .call(move |c| {
                for (id, name, reviewed) in rows {
                    c.execute(
                        "INSERT INTO project (id, name, born, reviewed, created_at) VALUES (?1, ?2, 'root', ?3, 7)",
                        rusqlite::params![id, name, reviewed],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .unwrap();
    }

    async fn changes(store: &MemoryStore) -> i64 {
        store.conn.call(|c| c.query_row("SELECT total_changes()", [], |r| r.get(0))).await.unwrap()
    }

    #[tokio::test]
    async fn renaming_keeps_the_id_and_marks_the_project_reviewed() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let id = save(&store, fact("a note of the web app", Some("app-1a2b"))).await;
        let (_, created_at) = row(&store, id).await;
        let before = changes(&store).await;

        let renamed = rename_project(&store, "app-1a2b", "App web").await.unwrap();

        assert_eq!(changes(&store).await - before, 1, "only the project's row changes");
        assert_eq!((renamed.id.as_str(), renamed.name.as_str(), renamed.reviewed), ("app-1a2b", "App web", true));
        assert_eq!(project(&store, "app-1a2b").await, Some(("App web".into(), "adopted".into(), 1, created_at)));
        assert_eq!(row(&store, id).await.0.as_deref(), Some("app-1a2b"), "what was saved under it stays");

        projects(&store, &[("web-3c4d", "web", 0)]).await;
        rename_project(&store, "web-3c4d", "web").await.expect("the same name only marks it reviewed");
        assert_eq!(project(&store, "web-3c4d").await, Some(("web".into(), "root".into(), 1, 7)));
    }

    #[tokio::test]
    async fn a_name_another_project_holds_is_refused() {
        let store = MemoryStore::new_for_test().await.unwrap();
        projects(&store, &[("api-1a2b", "api", 0), ("web-3c4d", "web", 0), ("ñandú-0f0f", "Ñandú", 0)]).await;

        for (id, name) in [("web-3c4d", "API"), ("web-3c4d", "Api-1A2B"), ("api-1a2b", "ÑANDÚ")] {
            let err = rename_project(&store, id, name).await.unwrap_err();
            assert!(matches!(&err, RenameError::Taken { .. }), "{name}: {err}");
        }
        assert_eq!(project(&store, "web-3c4d").await, Some(("web".into(), "root".into(), 0, 7)), "a refusal changes nothing");
        rename_project(&store, "api-1a2b", "Api").await.expect("its own name in another case");
        rename_project(&store, "api-1a2b", "API-1A2B").await.expect("its own id");
        let err = rename_project(&store, "nobody-0000", "anything").await.unwrap_err();
        assert!(matches!(err, RenameError::NotFound(ref id) if id == "nobody-0000"), "{err}");
    }

    #[tokio::test]
    async fn a_name_is_trimmed_and_bounded() {
        let store = MemoryStore::new_for_test().await.unwrap();
        projects(&store, &[("app-1a2b", "app", 0)]).await;

        assert_eq!(rename_project(&store, "app-1a2b", "  App web\t ").await.unwrap().name, "App web");
        let longest = "ñ".repeat(MAX_NAME_CHARS);
        assert_eq!(rename_project(&store, "app-1a2b", &longest).await.unwrap().name, longest, "counted in characters");
        let too_long = "x".repeat(MAX_NAME_CHARS + 1);
        let refused = [("", "empty"), ("   ", "blank"), ("a\nb", "a line break"), ("a\u{7f}b", "a control"), (too_long.as_str(), "too long")];
        for (name, why) in refused {
            let err = rename_project(&store, "app-1a2b", name).await.unwrap_err();
            assert!(matches!(err, RenameError::Name(_)), "{why}: {err}");
        }
        assert_eq!(project(&store, "app-1a2b").await.unwrap().0, longest, "a refused name changes nothing");
    }

    #[tokio::test]
    async fn a_folder_no_registration_recorded_is_listed_in_the_project_of_its_remote() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .conn
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('app-1a2b', 'app', 'migrated', 1, 1);
                     INSERT INTO project_path (id, root_path, created_at, project_id, remote) VALUES ('app-1a2b', '/work/app', 1, 'app-1a2b', 'github.com/acme/app');
                     INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/app', 'app-1a2b', 1);
                     INSERT INTO project_path (id, root_path, created_at) VALUES ('clone-9f9f', '/work/clone', 2);",
                )
            })
            .await
            .unwrap();
        let ground = Arc::new(FakeGround::default());
        ground.put(Path::new("/work/clone"), Some("github.com/acme/app"), Some("s1"), RootCommit::Later);

        let listed = list_projects(&store, ground).await.unwrap();

        assert_eq!(listed.len(), 1, "a folder's id alone is no project: {listed:?}");
        let folders: Vec<(&str, Option<&str>, bool)> =
            listed[0].folders.iter().map(|f| (f.root_path.as_str(), f.remote.as_deref(), f.present)).collect();
        assert_eq!(
            folders,
            vec![("/work/app", Some("github.com/acme/app"), false), ("/work/clone", Some("github.com/acme/app"), true)],
            "the clone's folder goes by the remote its checkout has"
        );
        assert_eq!(listed[0].remotes, vec!["github.com/acme/app".to_string()]);
    }

    #[tokio::test]
    async fn a_declaration_is_listed_on_both_sides_with_its_paths() {
        let dir = crate::test_dir::test_dir();
        let (mono, other) = (dir.path().join("mono"), dir.path().join("other"));
        std::fs::create_dir_all(mono.join("apps").join("web")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let store = MemoryStore::new_for_test().await.unwrap();
        let (m, o) = (mono.display().to_string(), other.display().to_string());
        store
            .conn
            .call(move |c| {
                c.execute_batch(
                    "INSERT INTO project (id, name, born, reviewed, created_at) VALUES \
                       ('mono-1a2b', 'mono', 'root', 1, 1), ('web', 'web', 'user', 1, 2);
                     INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) \
                       VALUES ('site', 'site', 'user', 1, 3, 'web', 4);
                     INSERT INTO project_subfolder (parent_id, subpath, project_id, created_at) VALUES \
                       ('mono-1a2b', 'apps/web', 'site', 5), ('mono-1a2b', 'apps/home', 'mono-1a2b', 6);",
                )?;
                c.execute(
                    "INSERT INTO project_path (id, root_path, created_at, project_id) VALUES ('mono-1a2b', ?1, 1, 'mono-1a2b'), ('mono-0002', ?2, 2, 'mono-1a2b')",
                    [m, o],
                )
            })
            .await
            .unwrap();

        let listed = list_projects(&store, Arc::new(FakeGround::default())).await.unwrap();

        let ids: Vec<&str> = listed.iter().map(|l| l.project.id.as_str()).collect();
        assert_eq!(ids, vec!["mono-1a2b", "web"]);
        let web_paths = vec![
            ListedSubfolderPath { path: mono.join("apps").join("web").display().to_string(), present: true },
            ListedSubfolderPath { path: other.join("apps").join("web").display().to_string(), present: false },
        ];
        let declared = ListedSubfolder {
            repository_id: "mono-1a2b".into(),
            subpath: "apps/web".into(),
            project_id: "web".into(),
            paths: web_paths,
        };
        assert_eq!(listed[1].subfolders, vec![declared.clone()], "under the project, through the tombstone it names");
        assert!(listed[1].folders.is_empty());
        let repository: Vec<(&str, &str)> = listed[0].subfolders.iter().map(|s| (s.subpath.as_str(), s.project_id.as_str())).collect();
        assert_eq!(repository, vec![("apps/web", "web"), ("apps/home", "mono-1a2b")], "and under its repository");

        let json = serde_json::to_value(&listed[1]).unwrap();
        let mut old = json.clone();
        old.as_object_mut().unwrap().remove("subfolders");
        let read: ListedProject = serde_json::from_value(old).unwrap();
        assert!(read.subfolders.is_empty(), "a daemon from before declarations sends none");
    }

    #[tokio::test]
    async fn a_subproject_resolves_to_the_folders_of_its_repository() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .conn
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO project (id, name, born, reviewed, created_at) VALUES \
                       ('mono-1a2b', 'mono', 'root', 1, 1), ('web', 'web', 'user', 1, 2), ('api', 'api', 'user', 1, 3);
                     INSERT INTO project_path (id, root_path, created_at, index_docs, project_id) VALUES \
                       ('mono-1a2b', '/work/mono', 2, 1, 'mono-1a2b'), ('mono-0002', '/other/mono', 1, 0, 'mono-1a2b'), \
                       ('api-9f9f', '/work/api', 1, 1, 'api');
                     INSERT INTO project_subfolder (parent_id, subpath, project_id, created_at) VALUES \
                       ('mono-1a2b', 'apps/web', 'web', 5), ('mono-1a2b', 'apps/api', 'api', 5);",
                )
            })
            .await
            .unwrap();

        let web = resolve_project(&store, "web", AGENT).await.unwrap();
        let folders: Vec<(&str, bool)> = web.folders.iter().map(|f| (f.root_id.as_str(), f.index_docs)).collect();
        assert_eq!(folders, vec![("mono-0002", false), ("mono-1a2b", true)], "its repository's, oldest first");
        let api = resolve_project(&store, "api", AGENT).await.unwrap();
        let folders: Vec<&str> = api.folders.iter().map(|f| f.root_id.as_str()).collect();
        assert_eq!(folders, vec!["api-9f9f"], "a folder of its own goes first and alone");
    }

    #[tokio::test]
    async fn a_merged_project_is_listed_under_its_target_and_frees_its_name() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .conn
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO project (id, name, born, reviewed, created_at) VALUES \
                       ('web-3c4d', 'web', 'root', 1, 1), ('api-5e6f', 'api', 'root', 1, 2);
                     INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) \
                       VALUES ('app-1a2b', 'app', 'root', 0, 1, 'web-3c4d', 9);
                     INSERT INTO project_merge (source, target, merged_at) VALUES ('app-1a2b', 'web-3c4d', 9);
                     INSERT INTO project_merge_change (merge_id, kind, a) VALUES (1, 'tombstoned', 'app-1a2b');
                     INSERT INTO project_path (id, root_path, created_at) VALUES ('app-1a2b', '/work/app', 1);
                     INSERT INTO project_suggestion (project_id, other_id, reason, created_at) VALUES \
                       ('app-1a2b', 'api-5e6f', 'root_commit', 3), ('api-5e6f', 'web-3c4d', 'root_commit', 4), \
                       ('app-1a2b', 'web-3c4d', 'root_commit', 5);",
                )
            })
            .await
            .unwrap();

        let listed = list_projects(&store, Arc::new(FakeGround::default())).await.unwrap();

        let ids: Vec<&str> = listed.iter().map(|l| l.project.id.as_str()).collect();
        assert_eq!(ids, vec!["api-5e6f", "web-3c4d"], "a tombstone is not listed");
        let web = &listed[1];
        let merged = ListedMerged { id: "app-1a2b".into(), name: "app".into(), merge_id: Some(1), merged_at: Some(9) };
        assert_eq!(web.merged, vec![merged]);
        let folders: Vec<&str> = web.folders.iter().map(|f| f.root_id.as_str()).collect();
        assert_eq!(folders, vec!["app-1a2b"], "a folder that led to the tombstone leads to its target");
        // (app, api) is (web, api) now, the pair (api, web) already is; (app, web) is one project.
        let others = |l: &ListedProject| l.suggestions.iter().map(|s| s.other_id.clone()).collect::<Vec<_>>();
        assert_eq!((others(&listed[0]), others(web)), (vec!["web-3c4d".to_string()], vec!["api-5e6f".to_string()]));

        rename_project(&store, "api-5e6f", "APP").await.expect("a tombstone's name is free");
        let taken = rename_project(&store, "api-5e6f", "app-1a2b").await;
        assert!(matches!(taken, Err(RenameError::Taken { .. })), "its id is not: {taken:?}");
        let merged = rename_project(&store, "app-1a2b", "x").await;
        assert!(matches!(&merged, Err(RenameError::Merged { into, .. }) if into == "web-3c4d"), "{merged:?}");
    }

    #[tokio::test]
    async fn keeping_two_projects_apart_answers_their_suggestion_for_good() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .conn
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO project (id, name, born, reviewed, created_at) VALUES \
                       ('web-3c4d', 'web', 'root', 0, 1), ('api-5e6f', 'api', 'root', 0, 2), ('lib-7a8b', 'lib', 'root', 1, 3);
                     INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) \
                       VALUES ('app-1a2b', 'app', 'root', 0, 1, 'web-3c4d', 9);
                     INSERT INTO project_suggestion (project_id, other_id, reason, created_at) VALUES \
                       ('app-1a2b', 'api-5e6f', 'root_commit', 3), ('api-5e6f', 'web-3c4d', 'remote_changed', 4);",
                )
            })
            .await
            .unwrap();

        let kept = keep_apart(&store, "api-5e6f", "web-3c4d").await.unwrap();

        assert_eq!(kept, KeptApart { project_id: "api-5e6f".into(), other_id: "web-3c4d".into(), answered: 2 });
        let rows = |sql: &'static str| {
            let store = &store;
            async move {
                store
                    .conn
                    .call(move |c| {
                        let mut stmt = c.prepare(sql)?;
                        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>();
                        rows
                    })
                    .await
                    .unwrap()
            }
        };
        assert_eq!(rows("SELECT answer FROM project_suggestion ORDER BY rowid").await, vec!["apart", "apart"]);
        assert_eq!(project(&store, "api-5e6f").await.unwrap().2, 1, "both are reviewed");
        assert_eq!(project(&store, "web-3c4d").await.unwrap().2, 1);
        let listed = list_projects(&store, Arc::new(FakeGround::default())).await.unwrap();
        assert!(listed.iter().all(|l| l.suggestions.is_empty()), "{listed:?}");

        let without = keep_apart(&store, "lib-7a8b", "web-3c4d").await.unwrap();
        assert_eq!(without.answered, 0);
        assert_eq!(
            rows("SELECT project_id || ' ' || other_id || ' ' || reason || ' ' || answer FROM project_suggestion WHERE rowid = 3").await,
            vec!["lib-7a8b web-3c4d user apart"],
            "a pair kept apart before any suggestion is recorded, so none is made later"
        );

        assert!(matches!(keep_apart(&store, "web-3c4d", "web-3c4d").await, Err(KeepApartError::Same(_))));
        assert!(matches!(keep_apart(&store, "web-3c4d", "nobody-0000").await, Err(KeepApartError::NotFound(id)) if id == "nobody-0000"));
        let merged = keep_apart(&store, "app-1a2b", "api-5e6f").await;
        assert!(matches!(&merged, Err(KeepApartError::Merged { into, .. }) if into == "web-3c4d"), "{merged:?}");
    }

    // ── deleted projects ────────────────────────────────────────────────

    /// Marks project `id` as delete `delete_id` leaves it.
    async fn mark_deleted(store: &MemoryStore, id: &str, delete_id: i64) {
        let batch = format!(
            "UPDATE project SET deleted_at = 5 WHERE id = '{id}';
             INSERT INTO project_delete (delete_id, project_id, with_memories, deleted_at) VALUES ({delete_id}, '{id}', 0, 5);"
        );
        store.conn.call(move |c| c.execute_batch(&batch)).await.unwrap();
    }

    #[tokio::test]
    async fn a_deleted_project_is_refused_by_what_manages_projects() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .conn
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO project (id, name, born, reviewed, created_at) VALUES \
                       ('web-3c4d', 'web', 'root', 0, 1), ('api-5e6f', 'api', 'root', 0, 2);",
                )
            })
            .await
            .unwrap();
        mark_deleted(&store, "api-5e6f", 7).await;

        let renamed = rename_project(&store, "api-5e6f", "interfaces").await;
        assert!(matches!(&renamed, Err(RenameError::Deleted { id, delete_id }) if id == "api-5e6f" && *delete_id == Some(7)), "{renamed:?}");
        for (a, b) in [("api-5e6f", "web-3c4d"), ("web-3c4d", "api-5e6f")] {
            let apart = keep_apart(&store, a, b).await;
            assert!(matches!(&apart, Err(KeepApartError::Deleted { id, delete_id }) if id == "api-5e6f" && *delete_id == Some(7)), "{apart:?}");
        }
        assert_eq!(project(&store, "api-5e6f").await.unwrap().2, 0, "nothing was marked reviewed");
        assert_eq!(project(&store, "web-3c4d").await.unwrap().2, 0);
        let suggestions: i64 = store
            .conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM project_suggestion", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(suggestions, 0, "and no pair was recorded");
    }

    #[tokio::test]
    async fn a_deleted_project_is_listed_only_in_deletes_and_frees_its_name() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        store
            .conn
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO project (id, name, born, reviewed, created_at) VALUES \
                       ('app-1a2b', 'app', 'root', 1, 1), ('web-3c4d', 'web', 'root', 1, 2);
                     INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) \
                       VALUES ('old-9f9f', 'old', 'root', 0, 1, 'app-1a2b', 9);
                     INSERT INTO project_path (id, root_path, created_at, index_docs, project_id) \
                       VALUES ('app-1a2b', '/work/app', 1, 0, 'app-1a2b'), ('web-3c4d', '/work/web', 1, 0, 'web-3c4d');
                     INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/app', 'app-1a2b', 1);
                     INSERT INTO project_suggestion (project_id, other_id, reason, created_at) VALUES ('app-1a2b', 'web-3c4d', 'root_commit', 3);",
                )
            })
            .await
            .unwrap();
        let kept = save(&store, fact("a note that stays deleted", Some("app-1a2b"))).await;
        let other = save(&store, fact("another note that stays deleted", Some("app-1a2b"))).await;
        store
            .conn
            .call(move |c| {
                c.execute_batch(&format!(
                    "UPDATE memory SET deleted_at = 4 WHERE id IN ({kept}, {other});
                     UPDATE project SET deleted_at = 5 WHERE id = 'app-1a2b';
                     INSERT INTO project_delete (delete_id, project_id, with_memories, deleted_at) VALUES (1, 'app-1a2b', 1, 5);
                     INSERT INTO project_delete_row (delete_id, tbl, row_id, deleted_at) VALUES (1, 'memory', {kept}, NULL), (1, 'memory', {other}, 3);"
                ))
            })
            .await
            .unwrap();

        let listed = list_projects(&store, Arc::new(FakeGround::default())).await.unwrap();

        let ids: Vec<&str> = listed.iter().map(|l| l.project.id.as_str()).collect();
        assert_eq!(ids, vec!["web-3c4d"], "a deleted project is not listed");
        let web = &listed[0];
        assert!(web.suggestions.is_empty(), "nor a suggestion with a deleted side: {:?}", web.suggestions);
        assert!(web.folders.iter().all(|f| f.root_id != "app-1a2b") && web.merged.is_empty() && web.remotes.is_empty());
        let deletes = list_deletes(&store).await.unwrap();
        assert_eq!(
            deletes,
            vec![ListedDelete {
                delete_id: 1,
                project_id: "app-1a2b".into(),
                name: "app".into(),
                with_memories: true,
                deleted_at: 5,
                rows: 2,
                back: false
            }]
        );

        rename_project(&store, "web-3c4d", "APP").await.expect("the name of a deleted project is free");
        let taken = rename_project(&store, "web-3c4d", "app-1a2b").await;
        assert!(matches!(taken, Err(RenameError::Taken { .. })), "its id is not: {taken:?}");

        // A save brings it back, and the delete is still open.
        save(&store, fact("a note saved after the delete", Some("app-1a2b"))).await;
        let listed = list_projects(&store, Arc::new(FakeGround::default())).await.unwrap();
        let app = listed.iter().find(|l| l.project.id == "app-1a2b").expect("listed again");
        assert_eq!((app.project.reviewed, app.memories, app.folders.len(), app.remotes.len(), app.merged.len()), (false, 1, 1, 1, 1));
        let deletes = list_deletes(&store).await.unwrap();
        assert_eq!((deletes.len(), deletes[0].back, deletes[0].rows), (1, true, 2), "the rows it deleted stay deleted");
    }

    // ── references by id or name ────────────────────────────────────────

    #[test]
    fn name_keys_join_what_people_write_alike() {
        for (a, b) in [("Mi App", "mi-app"), ("mi_app", "MiApp"), ("Ñandú web", "ñandú-WEB"), ("app 2", "App2")] {
            assert_eq!(name_key(a), name_key(b), "{a} {b}");
        }
        for (a, b) in [("api", "app"), ("web", "wiki"), ("app", "app2")] {
            assert_ne!(name_key(a), name_key(b), "{a} {b}");
        }
        assert_eq!(name_key("Mi App"), "miapp");
    }

    #[test]
    fn a_new_project_gets_a_slug_and_four_hex_digits() {
        let cases = [
            ("ideas de producto", "ideas-de-producto-"),
            ("  Mi App!! ", "mi-app-"),
            ("Ñandú", "and-"),
            ("проект", "project-"),
            ("---", "project-"),
        ];
        for (name, prefix) in cases {
            let id = new_project_id(name, 1_700_000_000_000, 0);
            assert!(is_project_id(&id), "{name}: {id}");
            assert!(id.starts_with(prefix) && id.len() == prefix.len() + 4, "{name}: {id}");
        }
        let long = new_project_id(&"x".repeat(200), 1, 0);
        assert_eq!(long.len(), 40 + 5, "{long}");
        assert_ne!(new_project_id("app", 1, 0), new_project_id("app", 1, 1), "the counter tries another");
        assert_ne!(new_project_id("app", 1, 0), new_project_id("app", 2, 0), "and so does the time");
    }

    const AGENT: ResolveOptions = ResolveOptions { create: false, new_project: false, born: Born::Agent };

    fn creating(new_project: bool) -> ResolveOptions {
        ResolveOptions { create: true, new_project, ..AGENT }
    }

    fn candidates(resolution: &Resolution) -> Vec<(&str, bool)> {
        resolution.candidates.iter().map(|c| (c.id.as_str(), c.deleted)).collect()
    }

    async fn count_projects(store: &MemoryStore) -> i64 {
        store.conn.call(|c| c.query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))).await.unwrap()
    }

    #[tokio::test]
    async fn a_reference_is_an_id_before_a_name() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .conn
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO project (id, name, born, reviewed, created_at) VALUES \
                       ('release-2024', 'release', 'root', 1, 1), ('web-3c4d', 'release-2024', 'root', 1, 2), \
                       ('api-5e6f', 'Interfaces', 'root', 1, 3);
                     INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) \
                       VALUES ('old-9f9f', 'old', 'root', 0, 1, 'api-5e6f', 9);
                     INSERT INTO project_path (id, root_path, created_at, index_docs, project_id) VALUES \
                       ('api-5e6f', '/work/api', 2, 0, 'api-5e6f'), ('api-0001', '/work/api2', 1, 1, 'api-5e6f');",
                )
            })
            .await
            .unwrap();

        let by_id = resolve_project(&store, "release-2024", AGENT).await.unwrap();
        assert_eq!((by_id.project_id.as_deref(), by_id.refused), (Some("release-2024"), None), "the id wins over a name");
        let by_name = resolve_project(&store, " interfaces ", AGENT).await.unwrap();
        assert_eq!(by_name.project_id.as_deref(), Some("api-5e6f"), "a name in any case, trimmed");
        assert_eq!((by_name.name.as_deref(), by_name.created, by_name.deleted), (Some("Interfaces"), false, false));
        let folders: Vec<(&str, bool)> = by_name.folders.iter().map(|f| (f.root_id.as_str(), f.index_docs)).collect();
        assert_eq!(folders, vec![("api-0001", true), ("api-5e6f", false)], "its folders, oldest first");
        let tombstone = resolve_project(&store, "old-9f9f", AGENT).await.unwrap();
        assert_eq!(tombstone.project_id.as_deref(), Some("api-5e6f"), "a tombstone's id leads to its project");
        let unknown = resolve_project(&store, "nothing like it", AGENT).await.unwrap();
        assert_eq!((unknown.project_id, unknown.refused), (None, Some(Refusal::Unknown)));
        let old = resolve_project(&store, "old", AGENT).await.unwrap();
        assert_eq!(old.refused, Some(Refusal::Unknown), "a tombstone's name is free and no candidate");
    }

    #[tokio::test]
    async fn a_name_two_projects_hold_is_ambiguous() {
        let store = MemoryStore::new_for_test().await.unwrap();
        projects(&store, &[("api-1a2b", "api", 0), ("api-3c4d", "API", 0)]).await;
        let before = count_projects(&store).await;

        let resolved = resolve_project(&store, "api", creating(true)).await.unwrap();

        assert_eq!((resolved.project_id.as_deref(), resolved.refused), (None, Some(Refusal::Ambiguous)));
        assert_eq!(candidates(&resolved), vec![("api-1a2b", false), ("api-3c4d", false)]);
        assert_eq!(count_projects(&store).await, before, "nothing is created, new_project or not");
    }

    #[tokio::test]
    async fn a_similar_name_creates_nothing_unless_asked() {
        let store = MemoryStore::new_for_test().await.unwrap();
        projects(&store, &[("mi-app-1a2b", "mi-app", 1)]).await;

        let similar = resolve_project(&store, "Mi App", creating(false)).await.unwrap();
        assert_eq!((similar.project_id.as_deref(), similar.refused), (None, Some(Refusal::Similar)));
        assert_eq!(candidates(&similar), vec![("mi-app-1a2b", false)]);
        assert_eq!(count_projects(&store).await, 1, "nothing is created");

        let apart = resolve_project(&store, "Mi App", creating(true)).await.unwrap();
        let id = apart.project_id.clone().expect("created");
        assert!(apart.created && apart.refused.is_none() && apart.folders.is_empty(), "{apart:?}");
        assert!(id.starts_with("mi-app-") && is_project_id(&id) && id != "mi-app-1a2b", "{id}");
        assert_eq!(project(&store, &id).await.map(|p| (p.0, p.1, p.2)), Some(("Mi App".into(), "agent".into(), 0)));

        let user = resolve_project(&store, "Ideas", ResolveOptions { born: Born::User, ..creating(false) }).await.unwrap();
        let ideas = user.project_id.expect("created");
        assert_eq!(project(&store, &ideas).await.map(|p| (p.1, p.2)), Some(("user".into(), 1)), "the user's own is reviewed");
        let exact = resolve_project(&store, "IDEAS", creating(true)).await.unwrap();
        assert_eq!((exact.project_id.as_deref(), exact.created), (Some(ideas.as_str()), false), "a live name never gets a twin");
    }

    #[tokio::test]
    async fn a_deleted_project_is_a_candidate_by_name_and_itself_by_id() {
        let store = MemoryStore::new_for_test().await.unwrap();
        projects(&store, &[("notas-1a2b", "notas", 1)]).await;
        mark_deleted(&store, "notas-1a2b", 4).await;

        let by_id = resolve_project(&store, "notas-1a2b", AGENT).await.unwrap();
        assert_eq!((by_id.project_id.as_deref(), by_id.deleted), (Some("notas-1a2b"), true));
        let by_name = resolve_project(&store, "Notas", creating(false)).await.unwrap();
        assert_eq!((by_name.refused, candidates(&by_name)), (Some(Refusal::Similar), vec![("notas-1a2b", true)]));
        assert_eq!(by_name.candidates[0].delete_id, Some(4));
        let fresh = resolve_project(&store, "Notas", creating(true)).await.unwrap();
        let id = fresh.project_id.expect("its name is free");
        assert!(fresh.created && id != "notas-1a2b", "{id}");
    }

    #[tokio::test]
    async fn a_reference_is_bounded() {
        let store = MemoryStore::new_for_test().await.unwrap();
        for bad in ["", "   ", "a\nb", &"x".repeat(MAX_NAME_CHARS + 1)] {
            let err = resolve_project(&store, bad, creating(true)).await.unwrap_err();
            assert!(matches!(err, ResolveError::Name(_)), "{bad:?}: {err}");
        }
        assert_eq!(count_projects(&store).await, 0);
    }

    #[tokio::test]
    async fn an_id_longer_than_a_name_still_resolves() {
        let store = MemoryStore::new_for_test().await.unwrap();
        // derive_project_id keeps the whole name of its folder.
        let long = format!("{}-1a2b", "x".repeat(MAX_NAME_CHARS));
        let id = long.clone();
        store
            .conn
            .call(move |c| c.execute("INSERT INTO project (id, name, born, reviewed, created_at) VALUES (?1, 'x', 'root', 1, 7)", [id]))
            .await
            .unwrap();

        let by_id = resolve_project(&store, &format!(" {long} "), creating(true)).await.unwrap();

        assert_eq!((by_id.project_id.as_deref(), by_id.created), (Some(long.as_str()), false));
        let err = resolve_project(&store, &format!("{long}0"), creating(true)).await.unwrap_err();
        assert!(matches!(err, ResolveError::Name(NameError::TooLong)), "a name is bounded: {err}");
        assert_eq!(count_projects(&store).await, 1);
    }

    #[tokio::test]
    async fn a_project_is_created_once_by_name() {
        let store = MemoryStore::new_for_test().await.unwrap();
        // An id a folder holds is taken too.
        store
            .conn
            .call(|c| c.execute("INSERT INTO project_path (id, root_path, created_at) VALUES ('x-0000', '/x', 1)", []))
            .await
            .unwrap();

        let first = resolve_project(&store, "ideas de producto", creating(false)).await.unwrap();
        let again = resolve_project(&store, "ideas de producto", creating(false)).await.unwrap();

        let id = first.project_id.expect("created");
        assert!(first.created && id.starts_with("ideas-de-producto-"), "{id}");
        assert_eq!((again.project_id.as_deref(), again.created), (Some(id.as_str()), false));
        assert_eq!(count_projects(&store).await, 1);
    }

    /// `app` merged into `web`; `api`, `lib` and the deleted `gone` live; the
    /// suggestions (app, api), (api, web), (web, lib) answered, (gone, web).
    async fn suggested(store: &MemoryStore) {
        store
            .conn
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO project (id, name, born, reviewed, created_at) VALUES \
                       ('web-3c4d', 'web', 'root', 0, 1), ('api-5e6f', 'api', 'root', 0, 2), ('lib-7a8b', 'lib', 'root', 0, 3);
                     INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) \
                       VALUES ('app-1a2b', 'app', 'root', 0, 1, 'web-3c4d', 9);
                     INSERT INTO project (id, name, born, reviewed, created_at, deleted_at) VALUES ('gone-9f9f', 'gone', 'root', 0, 1, 5);
                     INSERT INTO project_suggestion (project_id, other_id, reason, created_at) VALUES \
                       ('app-1a2b', 'api-5e6f', 'root_commit', 3), ('api-5e6f', 'web-3c4d', 'folder_name', 4), \
                       ('gone-9f9f', 'web-3c4d', 'root_commit', 5);
                     INSERT INTO project_suggestion (project_id, other_id, reason, created_at, answer, answered_at) \
                       VALUES ('web-3c4d', 'lib-7a8b', 'remote_changed', 6, 'apart', 7);",
                )
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn the_pending_suggestions_of_a_project_are_read_through_merges() {
        let store = MemoryStore::new_for_test().await.unwrap();
        suggested(&store).await;

        let of = |id: &'static str| {
            let store = &store;
            async move {
                pending_suggestions(store, id).await.unwrap().into_iter().map(|s| (s.other_id, s.other_name, s.reason)).collect::<Vec<_>>()
            }
        };

        let api = ("api-5e6f".to_string(), Some("api".to_string()), "root_commit".to_string());
        assert_eq!(of("web-3c4d").await, vec![api], "(app, api) is (web, api), once; a deleted side and an answer are none");
        assert_eq!(of("api-5e6f").await, vec![("web-3c4d".to_string(), Some("web".to_string()), "root_commit".to_string())]);
        assert!(of("lib-7a8b").await.is_empty() && of("gone-9f9f").await.is_empty());
        let listed = list_projects(&store, Arc::new(FakeGround::default())).await.unwrap();
        let web = listed.iter().find(|l| l.project.id == "web-3c4d").unwrap();
        assert_eq!(web.suggestions, pending_suggestions(&store, "web-3c4d").await.unwrap(), "as the listing reads them");
    }

    #[tokio::test]
    async fn a_suggestion_is_answered_only_when_pending() {
        let store = MemoryStore::new_for_test().await.unwrap();
        suggested(&store).await;

        let found = suggestion_between(&store, "api-5e6f", "web-3c4d").await.unwrap();
        assert_eq!(found.map(|s| s.other_id), Some("web-3c4d".to_string()));
        assert_eq!(suggestion_between(&store, "web-3c4d", "lib-7a8b").await.unwrap(), None, "an answered pair is not pending");
        let merged = suggestion_between(&store, "app-1a2b", "api-5e6f").await;
        assert!(matches!(&merged, Err(KeepApartError::Merged { into, .. }) if into == "web-3c4d"), "{merged:?}");
        let deleted = suggestion_between(&store, "web-3c4d", "gone-9f9f").await;
        assert!(matches!(&deleted, Err(KeepApartError::Deleted { .. })), "{deleted:?}");
        assert!(matches!(suggestion_between(&store, "web-3c4d", "nobody-0000").await, Err(KeepApartError::NotFound(_))));
    }

    #[tokio::test]
    async fn a_target_is_found_by_its_id_or_its_one_live_name() {
        let store = MemoryStore::new_for_test().await.unwrap();
        projects(&store, &[("api-1a2b", "api", 0), ("api-3c4d", "Api", 0), ("web-5e6f", "Web", 0), ("gone-7a8b", "gone", 0)]).await;
        mark_deleted(&store, "gone-7a8b", 1).await;

        assert_eq!(find_project(&store, "api-3c4d").await.unwrap(), Named::Project("api-3c4d".into()));
        assert_eq!(find_project(&store, "web").await.unwrap(), Named::Project("web-5e6f".into()));
        assert_eq!(find_project(&store, "API").await.unwrap(), Named::Ambiguous(vec!["api-1a2b".into(), "api-3c4d".into()]));
        assert_eq!(find_project(&store, "gone").await.unwrap(), Named::Unknown, "a deleted project's name is free");
        assert_eq!(find_project(&store, "gone-7a8b").await.unwrap(), Named::Project("gone-7a8b".into()), "its id is not");
        assert_eq!(find_project(&store, "nobody").await.unwrap(), Named::Unknown);
    }
}
