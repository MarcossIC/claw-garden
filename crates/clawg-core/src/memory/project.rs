//! Registers a project's canonical root, gives it an id, and joins into its
//! project the rows saved under the ids it had before.
//!
//! Each root keeps one id in `project_path` (its root id), which the docs
//! index uses. Its memories live under its project id, a row of `project`:
//! the id the root's remote was first registered with (`project_remote`,
//! V24), so the clones of one remote share them, or the root id without a
//! remote. Rows of `memory` join through the move engine, which logs them in
//! `memory_move` and leaves one live row per key and per text (see
//! [`merge_in`]); the other tables move row by row with `project_rekey`,
//! whose undo SQL is in the migration that creates it.
//! `memory_fts` needs nothing: it reads `project_id` from `memory`.
//!
//! A path is not an identity: another repository can be cloned where one
//! was. So a recorded folder serves a root only while its fingerprints, the
//! remote and the first commit it recorded, do not contradict the checkout
//! there now (see [`fit`]); a root that contradicts every folder of its path
//! gets a folder and a project of its own, and its remote is bound there.
//! A checkout on a new path keeps its project when its first commit was
//! recorded in another path whose checkout is gone (see [`moved`]); when
//! that cannot be told, the root stays apart and the two projects are left
//! as a suggestion in `project_suggestion`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension};

use crate::memory::backup::Reason;
use crate::memory::clock::TimestampMs;
use crate::memory::identity::{candidate_id, derive_project_id, Checkout, Ground, RootCommit, CANDIDATES};
use crate::memory::registry::{name_key, project_name};
use crate::memory::merge::canonical_in;
use crate::memory::relocate::{merge_in, Span};
use crate::memory::subfolder::declared_in;
use crate::memory::workspace::WorkspaceManager;
use crate::memory::{MemoryError, MemoryStore};

/// The tables besides `memory` whose rows carry a `project_id`. A literal
/// list rather than interpolated input, so no caller value ever reaches a
/// table name.
const OTHER_PROJECT_TABLES: [&str; 2] = ["episodic_trajectory", "procedural_skill"];

/// What a caller asks of [`register_project`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration<'a> {
    pub root: &'a Path,
    /// What is at the root now: the key of its remote and the stamp of its
    /// git directory ([`Ground::checkout`]). A caller that derives its id is
    /// never bound to the key; it only settles the root's folder.
    pub checkout: Checkout,
    /// Ids the same project was saved under before.
    pub legacy_ids: &'a [String],
    /// Whether the daemon may read the root's files.
    pub index_docs: bool,
    /// The caller uses the ids the answer gives. One that does not derives
    /// the id from the root itself, so it keeps the rule it was written for.
    pub assigned: bool,
    /// Where the caller works, measured from the top of the root's checkout
    /// ([`Ground::subpath`]): a declared subfolder there leads to another
    /// project. `None` or empty for the root itself.
    pub subpath: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterOutcome {
    /// Where the root's memories are read and written.
    pub project_id: String,
    /// The root's own id, which its docs index uses.
    pub root_id: String,
    /// Rows moved onto `project_id` by this call, across every table.
    pub rekeyed: usize,
    /// Whether the daemon may read the root's files, as stored after this call.
    pub index_docs: bool,
    /// The move in `memory_move` that joined the rows of `memory`, which
    /// `clawg projects move --undo` takes.
    pub move_id: Option<i64>,
    /// The `pre-rekey` snapshot taken before rows moved.
    pub snapshot: Option<PathBuf>,
    /// The project a declared subfolder leads the caller's path to, read
    /// after the root settled in `project_id`, where its legacy ids joined.
    pub subproject: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    #[error("{0:?} is not a project id")]
    InvalidLegacyId(String),
    /// A caller that derives its own id, for a root whose id belongs to
    /// another root. Moving rows into it would merge two projects, so nothing
    /// moves and the caller keeps the id it had.
    #[error("project id {id} already belongs to {existing}")]
    RootCollision { id: String, existing: String },
    #[error("every id {root} may take belongs to another root", root = .0.display())]
    NoFreeId(PathBuf),
    #[error(transparent)]
    Store(#[from] MemoryError),
}

/// Whether `s` has the shape [`derive_project_id`] produces:
/// a non-empty name, a dash and four lowercase hex digits.
pub fn is_project_id(s: &str) -> bool {
    match s.rsplit_once('-') {
        Some((name, hash)) => {
            !name.is_empty()
                && hash.len() == 4
                && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        None => false,
    }
}

/// The ids a registration settles on, before anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Plan {
    root_id: String,
    project_id: String,
    /// Whether `project_path` has no row for the root yet.
    new_root: bool,
    /// Ids whose rows join the project.
    legacy: Vec<String>,
    /// Whether any of them has a row.
    moves: bool,
    /// The root's folder as the registration leaves it.
    folder: Folder,
    /// Whether that is not how `project_path` holds it: a registration
    /// that changes nothing writes nothing, since the hooks register on
    /// every event.
    folder_changed: bool,
    /// The paths the root's checkout was recorded in before it moved.
    moved_from: Vec<PathBuf>,
    /// The projects `project_id` may be the same as, each with why.
    suggestions: Vec<(String, &'static str)>,
}

/// A folder of a path, as `project_path` records it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Folder {
    id: String,
    project_id: Option<String>,
    remote: Option<String>,
    /// The remotes `project_remote` binds to its id.
    bound: Vec<String>,
    root_commit: Option<String>,
    git_stamp: Option<String>,
}

impl Folder {
    /// The remotes the folder goes by: its own, or for one from before V27
    /// without one, those bound to its id. A folder whose project was fused
    /// from two remotes has both, and no way to tell which is its own.
    fn remotes(&self) -> &[String] {
        match &self.remote {
            Some(remote) => std::slice::from_ref(remote),
            None => &self.bound,
        }
    }
}

/// How a folder of a path compares with the checkout there now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fit {
    Same,
    Contradicted,
    /// No fingerprint is known on both sides.
    Unknown,
}

/// Compares `folder` with a checkout whose remote has the key `key` and whose
/// first commit is `commit`: by remote when both sides know one, else by
/// first commit when both know it. The remote decides first: an orphan
/// branch or a rewritten history changes the first commit of one
/// repository, and a fork shares the first commit of another.
fn fit(folder: &Folder, key: Option<&str>, commit: Option<&str>) -> Fit {
    let remotes = folder.remotes();
    if let (false, Some(key)) = (remotes.is_empty(), key) {
        return if remotes.iter().any(|r| r == key) { Fit::Same } else { Fit::Contradicted };
    }
    match (folder.root_commit.as_deref(), commit) {
        (Some(recorded), Some(now)) if recorded == now => Fit::Same,
        (Some(_), Some(_)) => Fit::Contradicted,
        _ => Fit::Unknown,
    }
}

/// Which of the folders `project_path` records for a path is the repository
/// there now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Settled {
    /// The folder at this index, oldest first.
    Row(usize),
    /// The path has no folder.
    Fresh,
    /// Every folder of the path is another repository: a new one took the
    /// path, and it gets a folder of its own.
    Contradicted,
}

/// Settles a path on one of its folders (oldest first): the first that is
/// the checkout there now (see [`fit`]), else the oldest nothing contradicts.
fn settle(folders: &[Folder], key: Option<&str>, commit: Option<&str>) -> Settled {
    if folders.is_empty() {
        return Settled::Fresh;
    }
    let fits: Vec<Fit> = folders.iter().map(|folder| fit(folder, key, commit)).collect();
    fits.iter()
        .position(|f| *f == Fit::Same)
        .or_else(|| fits.iter().position(|f| *f == Fit::Unknown))
        .map_or(Settled::Contradicted, Settled::Row)
}

/// The first commit of a checkout whose git directory is stamped `stamp`, as
/// a folder of its path recorded it with that stamp, or `None` when none did
/// and it is to be read. A root without a git directory has none to read.
fn recorded_commit(folders: &[Folder], stamp: Option<&str>) -> Option<RootCommit> {
    let Some(stamp) = stamp else {
        return Some(RootCommit::Later);
    };
    let folder = folders.iter().find(|f| f.git_stamp.as_deref() == Some(stamp))?;
    Some(folder.root_commit.clone().map_or(RootCommit::Unknowable, RootCommit::Known))
}

/// The columns of `project_path AS p` that [`folder_from`] reads, in its order.
const FOLDER_COLUMNS: &str = "p.id, p.project_id, p.remote, p.root_commit, p.git_stamp, \
     (SELECT json_group_array(remote) FROM project_remote WHERE project_id = p.id)";

fn folder_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<Folder> {
    let bound: String = r.get(5)?;
    Ok(Folder {
        id: r.get(0)?,
        project_id: r.get(1)?,
        remote: r.get(2)?,
        bound: serde_json::from_str(&bound).unwrap_or_default(),
        root_commit: r.get(3)?,
        git_stamp: r.get(4)?,
    })
}

/// The folders `project_path` records for `root`, oldest first.
fn path_folders_in(c: &Connection, root: &str) -> rusqlite::Result<Vec<Folder>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {FOLDER_COLUMNS} FROM project_path AS p WHERE p.root_path = ?1 ORDER BY p.created_at, p.id"
    ))?;
    let rows = stmt.query_map([root], folder_from)?;
    rows.collect()
}

/// The root `project_path` records for `id`, if any.
fn root_of_in(c: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    c.query_row("SELECT root_path FROM project_path WHERE id = ?1", [id], |r| r.get(0)).optional()
}

/// The project the clones of `remote` share, if one registered it.
fn remote_project_in(c: &Connection, remote: &str) -> rusqlite::Result<Option<String>> {
    c.query_row("SELECT project_id FROM project_remote WHERE remote = ?1", [remote], |r| r.get(0)).optional()
}

/// The project `folder` leads to: the one a registration recorded in it,
/// else, for a folder none recorded one in since V27, the one the clones of
/// its remote share, else its own id.
fn folder_project_in(c: &Connection, folder: &Folder) -> rusqlite::Result<String> {
    if let Some(project) = &folder.project_id {
        return Ok(project.clone());
    }
    let shared = match &folder.remote {
        Some(remote) => remote_project_in(c, remote)?,
        None => None,
    };
    Ok(shared.unwrap_or_else(|| folder.id.clone()))
}

/// A checkout recorded at a path, by the stamp of its git directory.
type Recorded = (PathBuf, Option<String>);

/// A folder of another path recorded with the first commit of a root on a
/// new path, which the root's remote does not contradict: the checkout
/// recorded there may be the root's, moved.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sighting {
    path: PathBuf,
    stamp: Option<String>,
    /// The project the folder leads to.
    project: String,
}

impl Sighting {
    fn recorded(&self) -> Recorded {
        (self.path.clone(), self.stamp.clone())
    }
}

/// What a root on a new path is, from where its first commit was recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Moved {
    /// The checkout recorded for this project, moved.
    To(String),
    /// A root of its own, which may be the same as any of these projects.
    Apart(Vec<String>),
}

/// Decides [`Moved`] from the `sightings` of a root's first commit, of which
/// `gone` are the ones whose recorded checkout is no longer there: the root
/// is that checkout, moved, only when every sighting leads to one project
/// and none is there anymore. A checkout still in its place makes the root a
/// copy or a second checkout, and two projects leave no way to choose, so
/// the root stays apart: a join can be undone, a fusion cannot be split.
fn moved(sightings: &[Sighting], gone: &[Recorded]) -> Moved {
    let mut projects: Vec<String> = Vec::new();
    for sighting in sightings {
        if !projects.contains(&sighting.project) {
            projects.push(sighting.project.clone());
        }
    }
    let all_gone = sightings.iter().all(|s| gone.contains(&s.recorded()));
    match projects.as_slice() {
        [project] if all_gone => Moved::To(project.clone()),
        _ => Moved::Apart(projects),
    }
}

/// The [`Sighting`]s of `root`, whose checkout has the first commit `commit`,
/// among the folders of other paths and those `pending` would write, oldest
/// first. None unless the first commit is known and `looks`: the root's path
/// has no folder and no project shares its remote, which would name its
/// project already.
fn sightings_in(
    c: &Connection,
    root: &Path,
    checkout: &Checkout,
    commit: &RootCommit,
    looks: bool,
    pending: &Pending,
) -> rusqlite::Result<Vec<Sighting>> {
    let (RootCommit::Known(commit), true) = (commit, looks) else {
        return Ok(Vec::new());
    };
    let mut stmt = c.prepare(&format!(
        "SELECT {FOLDER_COLUMNS}, p.root_path FROM project_path AS p \
         WHERE p.root_commit = ?1 ORDER BY p.created_at, p.id"
    ))?;
    let rows = stmt.query_map([commit], |r| Ok((PathBuf::from(r.get::<_, String>(6)?), folder_from(r)?)))?;
    let mut found = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    for (path, folder) in &pending.folders {
        match found.iter_mut().find(|(_, f)| f.id == folder.id) {
            Some(recorded) => *recorded = (path.clone(), folder.clone()),
            None => found.push((path.clone(), folder.clone())),
        }
    }
    let key = checkout.key.as_deref();
    let mut sightings = Vec::new();
    for (path, folder) in found {
        let same = folder.root_commit.as_deref() == Some(commit.as_str()) && fit(&folder, key, Some(commit)) == Fit::Same;
        if path != root && same {
            sightings.push(Sighting { project: folder_project_in(c, &folder)?, path, stamp: folder.git_stamp });
        }
    }
    Ok(sightings)
}

/// The live projects, other than `own`, named as the folder `root` is (by
/// [`name_key`]), each with the checkouts its folders recorded; none unless
/// `own`, the project the root would go to, is a new one. A project without
/// a folder has nothing that moved, so it is left out.
fn named_after_in(c: &Connection, root: &Path, own: &str) -> rusqlite::Result<Vec<(String, Vec<Recorded>)>> {
    let key = root.file_name().map(|n| name_key(&n.to_string_lossy())).unwrap_or_default();
    let exists: bool = c.query_row("SELECT EXISTS (SELECT 1 FROM project WHERE id = ?1)", [own], |r| r.get(0))?;
    if key.is_empty() || exists {
        return Ok(Vec::new());
    }
    let mut stmt = c.prepare("SELECT id, name FROM project WHERE merged_into IS NULL AND deleted_at IS NULL AND id <> ?1")?;
    let projects = stmt.query_map([own], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut folders = c.prepare(
        "SELECT root_path, git_stamp FROM project_path WHERE project_id = ?1 OR (project_id IS NULL AND id = ?1) ORDER BY created_at, id",
    )?;
    let mut named = Vec::new();
    for (id, _) in projects.into_iter().filter(|(_, name)| name_key(name) == key) {
        let recorded =
            folders.query_map([&id], |r| Ok((PathBuf::from(r.get::<_, String>(0)?), r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if !recorded.is_empty() {
            named.push((id, recorded));
        }
    }
    Ok(named)
}

/// What a registration records in the folder it settles on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Record {
    /// The root's key, when the folder goes by no remote: it is its first.
    remote: Option<String>,
    /// The first commit (none when it is not to be known) and the stamp of
    /// the git directory it was read in; none when it is to be read again.
    prints: Option<(Option<String>, String)>,
}

/// The folder a root settles on, as it is before the registration writes.
#[derive(Debug, Clone)]
struct Settlement {
    /// The folder, or a new one with only its id.
    folder: Folder,
    /// Whether `project_path` has no row with its id yet.
    new_row: bool,
    /// Whether the ids the path had before roots were resolved are this
    /// root's: they are the path's, so only its first repository, the one in
    /// its oldest folder, takes them.
    owns_legacy: bool,
    record: Record,
}

impl Settlement {
    /// Whether every folder of the path was another repository.
    fn contradicted(&self) -> bool {
        self.new_row && !self.owns_legacy
    }

    /// Whether the path had no folder.
    fn fresh(&self) -> bool {
        self.new_row && self.owns_legacy
    }

    /// The folder as the registration leaves it, leading to `project_id`
    /// when it names one.
    fn written(&self, project_id: Option<&str>) -> Folder {
        let mut folder = self.folder.clone();
        if let Some(project_id) = project_id {
            folder.project_id = Some(project_id.to_string());
        }
        if let Some(remote) = &self.record.remote {
            folder.remote = Some(remote.clone());
        }
        if let Some((commit, stamp)) = &self.record.prints {
            (folder.root_commit, folder.git_stamp) = (commit.clone(), Some(stamp.clone()));
        }
        folder
    }
}

/// The project a root settled on `settled` goes to: the one the clones of
/// its remote share (`shared`), else the one its folder leads to, else one of
/// the folder's own id.
fn project_of(shared: Option<String>, settled: &Settlement) -> String {
    shared.or_else(|| settled.folder.project_id.clone()).unwrap_or_else(|| settled.folder.id.clone())
}

/// What a batch would have written before the root it settles now.
#[derive(Debug, Default)]
struct Pending {
    /// Folders as the batch would leave them, by path; new ones in the order
    /// they would be written.
    folders: Vec<(PathBuf, Folder)>,
    /// Remote to project.
    remotes: HashMap<String, String>,
    /// Every root id given so far.
    claimed: Vec<String>,
}

impl Pending {
    fn write(&mut self, root: &Path, folder: Folder) {
        match self.folders.iter_mut().find(|(_, f)| f.id == folder.id) {
            Some((_, written)) => *written = folder,
            None => self.folders.push((root.to_path_buf(), folder)),
        }
    }
}

/// The folders of `root` (see [`path_folders_in`]) as `pending` would leave them.
fn folders_in(c: &Connection, root: &Path, pending: &Pending) -> rusqlite::Result<Vec<Folder>> {
    let mut folders = path_folders_in(c, &root.to_string_lossy())?;
    for (_, folder) in pending.folders.iter().filter(|(path, _)| path == root) {
        match folders.iter_mut().find(|f| f.id == folder.id) {
            Some(recorded) => *recorded = folder.clone(),
            None => folders.push(folder.clone()),
        }
    }
    for (remote, project) in &pending.remotes {
        for folder in folders.iter_mut().filter(|f| f.id == *project && !f.bound.contains(remote)) {
            folder.bound.push(remote.clone());
        }
    }
    Ok(folders)
}

/// The folder of `root`, among its path's `folders`, that a checkout with
/// the first commit `commit` settles on (see [`settle`]), or a new one with
/// the first [`candidate_id`] no folder has and no earlier root of the batch
/// took.
fn settle_in(
    c: &Connection,
    root: &Path,
    folders: &[Folder],
    checkout: &Checkout,
    commit: &RootCommit,
    pending: &Pending,
) -> Result<Settlement, RegisterError> {
    let sql = |e: rusqlite::Error| RegisterError::Store(MemoryError::Sqlite(e));
    let key = checkout.key.as_deref();
    let known = match commit {
        RootCommit::Known(commit) => Some(commit.as_str()),
        RootCommit::Unknowable | RootCommit::Later => None,
    };
    let prints = checkout.stamp.clone().and_then(|stamp| match commit {
        RootCommit::Known(commit) => Some((Some(commit.clone()), stamp)),
        RootCommit::Unknowable => Some((None, stamp)),
        RootCommit::Later => None,
    });
    let record = |folder: &Folder| Record {
        remote: key.filter(|_| folder.remotes().is_empty()).map(str::to_string),
        prints: prints.clone(),
    };
    let owns_legacy = match settle(folders, key, known) {
        Settled::Row(i) => {
            let folder = folders[i].clone();
            return Ok(Settlement { record: record(&folder), folder, new_row: false, owns_legacy: i == 0 });
        }
        Settled::Fresh => true,
        Settled::Contradicted => false,
    };
    for n in 0..CANDIDATES {
        let id = candidate_id(root, n);
        if !pending.claimed.contains(&id) && root_of_in(c, &id).map_err(sql)?.is_none() {
            let folder = Folder { id, ..Folder::default() };
            return Ok(Settlement { record: record(&folder), folder, new_row: true, owns_legacy });
        }
    }
    Err(RegisterError::NoFreeId(root.to_path_buf()))
}

/// The project id `root` reads and writes under, as the store says now and
/// without writing: the one a registration with assigned ids would give it,
/// from what `ground` finds at the root, or the project a declared subfolder
/// leads `path` to inside it.
pub fn project_id_in(c: &Connection, root: &Path, path: Option<&Path>, ground: &dyn Ground) -> rusqlite::Result<String> {
    let project = plan_ids_in(c, &[root.to_path_buf()], ground)?.remove(0).project_id;
    let subpath = path.and_then(|p| ground.subpath(root, p));
    Ok(match subpath {
        Some(subpath) => declared_in(c, &project, &subpath)?.unwrap_or(project),
        None => project,
    })
}

/// What a registration with assigned ids would give a root ([`plan_ids_in`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RootPlan {
    pub(crate) root_id: String,
    pub(crate) project_id: String,
    /// The key of the root's remote, which is bound to `project_id` and is
    /// the remote of a new folder.
    pub(crate) remote: Option<String>,
    /// The first commit and the git stamp the root's folder records.
    pub(crate) root_commit: Option<String>,
    pub(crate) git_stamp: Option<String>,
}

/// What each of `roots` would get, as [`register_project`] gives it, from
/// what `ground` finds at each, read without writing: a root later in the
/// list never takes an id or a remote an earlier one was given, and settles
/// as if the earlier ones were written, fingerprints included.
pub(crate) fn plan_ids_in(c: &Connection, roots: &[PathBuf], ground: &dyn Ground) -> rusqlite::Result<Vec<RootPlan>> {
    let mut pending = Pending::default();
    let mut out = Vec::new();
    for root in roots {
        let checkout = ground.checkout(root);
        let folders = folders_in(c, root, &pending)?;
        let commit = recorded_commit(&folders, checkout.stamp.as_deref()).unwrap_or_else(|| ground.root_commit(root));
        let settled = match settle_in(c, root, &folders, &checkout, &commit, &pending) {
            Ok(settled) => Some(settled),
            Err(RegisterError::Store(MemoryError::Sqlite(e))) => return Err(e),
            Err(_) => None,
        };
        let root_id = settled.as_ref().map_or_else(|| derive_project_id(root), |s| s.folder.id.clone());
        pending.claimed.push(root_id.clone());
        let shared = match checkout.key.as_deref() {
            Some(key) => match pending.remotes.get(key) {
                Some(project) => Some(project.clone()),
                None => remote_project_in(c, key)?,
            },
            None => None,
        };
        let project_id = match &settled {
            Some(settled) => {
                let looks = shared.is_none() && settled.fresh();
                let sightings = sightings_in(c, root, &checkout, &commit, looks, &pending)?;
                let gone: Vec<Recorded> = sightings
                    .iter()
                    .filter(|s| !ground.recorded_checkout_present(&s.path, s.stamp.as_deref()))
                    .map(Sighting::recorded)
                    .collect();
                match moved(&sightings, &gone) {
                    Moved::To(project) => project,
                    Moved::Apart(_) => project_of(shared, settled),
                }
            }
            None => shared.unwrap_or_else(|| root_id.clone()),
        };
        let project_id = canonical_in(c, &project_id)?;
        if let Some(key) = &checkout.key {
            pending.remotes.entry(key.clone()).or_insert_with(|| project_id.clone());
        }
        let written = settled.map(|settled| settled.written(Some(&project_id)));
        let (root_commit, git_stamp) = written.as_ref().map_or((None, None), |f| (f.root_commit.clone(), f.git_stamp.clone()));
        if let Some(folder) = written {
            pending.write(root, folder);
        }
        out.push(RootPlan { root_id, project_id, remote: checkout.key, root_commit, git_stamp });
    }
    Ok(out)
}

/// What the passes of [`register_project`] read from the disk so far.
#[derive(Debug, Clone, Default)]
struct FromDisk {
    /// The first commit of the root.
    commit: Option<RootCommit>,
    /// Of the recorded checkouts looked at, the ones no longer there. One
    /// that was not looked at counts as there, which keeps the root apart.
    gone: Option<Vec<Recorded>>,
}

/// What a pass of [`plan_in`] settles on, or what it needs from the disk first.
enum Planned {
    Plan(Plan),
    /// The first commit of the root: no folder of its path recorded its git directory.
    Commit,
    /// Whether these checkouts are still where they were recorded: the root
    /// may be one of them, moved.
    Presence(Vec<Recorded>),
}

/// Settles the ids of a registration inside the caller's transaction from
/// what `disk` read, or stops for what it still needs (see [`Planned`]) when
/// `may_read` or `may_look` let it. A pass that may not stop settles without:
/// a first commit not read contradicts nothing, and a checkout not looked at
/// is there.
fn plan_in(
    c: &Connection,
    reg: &Registration<'_>,
    disk: &FromDisk,
    may_read: bool,
    may_look: bool,
) -> Result<Planned, RegisterError> {
    let root_path = reg.root.to_string_lossy().into_owned();
    let sql = |e: rusqlite::Error| RegisterError::Store(MemoryError::Sqlite(e));
    let folders = folders_in(c, reg.root, &Pending::default()).map_err(sql)?;
    let commit = match (recorded_commit(&folders, reg.checkout.stamp.as_deref()), &disk.commit) {
        (Some(recorded), _) => recorded,
        (None, Some(read)) => read.clone(),
        (None, None) if may_read => return Ok(Planned::Commit),
        (None, None) => RootCommit::Later,
    };
    let settled = settle_in(c, reg.root, &folders, &reg.checkout, &commit, &Pending::default());
    let key = reg.checkout.key.as_deref();
    let (mut moved_from, mut suggestions) = (Vec::new(), Vec::new());
    let (settled, project_id, folder) = if reg.assigned {
        let settled = settled?;
        if settled.contradicted() {
            let first = match &commit {
                RootCommit::Known(commit) => commit.as_str(),
                RootCommit::Unknowable | RootCommit::Later => "unknown",
            };
            log::warn!(
                "{} holds a repository that is none of the ones recorded for that path (remote {}, first commit {first}); \
                 it gets a project of its own, {}",
                reg.root.display(),
                key.unwrap_or("none"),
                settled.folder.id
            );
        }
        let shared = key.map(|r| remote_project_in(c, r)).transpose().map_err(sql)?.flatten();
        let looks = shared.is_none() && settled.fresh();
        let sightings = sightings_in(c, reg.root, &reg.checkout, &commit, looks, &Pending::default()).map_err(sql)?;
        let named = if looks { named_after_in(c, reg.root, &project_of(shared.clone(), &settled)).map_err(sql)? } else { Vec::new() };
        let gone = match &disk.gone {
            Some(gone) => gone.as_slice(),
            None if may_look && (!sightings.is_empty() || !named.is_empty()) => {
                let looked = sightings.iter().map(Sighting::recorded).chain(named.iter().flat_map(|(_, r)| r.iter().cloned()));
                return Ok(Planned::Presence(looked.collect()));
            }
            None => &[],
        };
        // Every project named here is the live one a merge left it in.
        let live = |id: &str| canonical_in(c, id).map_err(sql);
        let project_id = match moved(&sightings, gone) {
            Moved::To(project) => {
                moved_from = sightings.into_iter().map(|s| s.path).collect();
                live(&project)?
            }
            Moved::Apart(others) => {
                let project_id = live(&project_of(shared, &settled))?;
                for other in others {
                    let other = live(&other)?;
                    if other != project_id && !suggestions.iter().any(|(o, _)| *o == other) {
                        suggestions.push((other, "root_commit"));
                    }
                }
                // A folder that moved with nothing to tell it by but its name:
                // the project of that name whose every folder is gone.
                for (other, recorded) in &named {
                    let all_gone = recorded.iter().all(|r| gone.contains(r));
                    if all_gone && *other != project_id && !suggestions.iter().any(|(o, _)| o == other) {
                        suggestions.push((other.clone(), "folder_name"));
                    }
                }
                project_id
            }
        };
        // A remote that changed in place, a rename or a new organization,
        // keeps the first commit the path recorded: told apart by the remote,
        // and maybe the same project.
        if let (true, RootCommit::Known(commit)) = (settled.contradicted(), &commit) {
            for folder in folders.iter().filter(|f| f.root_commit.as_ref() == Some(commit)) {
                let other = live(&folder_project_in(c, folder).map_err(sql)?)?;
                if other != project_id && !suggestions.iter().any(|(o, _)| *o == other) {
                    suggestions.push((other, "remote_changed"));
                }
            }
        }
        let folder = settled.written(Some(&project_id));
        (settled, project_id, folder)
    } else {
        let id = derive_project_id(reg.root);
        let owner = root_of_in(c, &id).map_err(sql)?;
        if let Some(other) = owner.as_ref().filter(|o| **o != root_path) {
            return Err(RegisterError::RootCollision { id, existing: other.clone() });
        }
        // The root settles on another folder, or the repository there now is
        // none of the ones its path recorded: either way the id it derives is
        // not its project.
        let settled = match settled {
            Ok(settled) if settled.folder.id == id && !settled.contradicted() => settled,
            Ok(_) | Err(RegisterError::NoFreeId(_)) => {
                return Err(RegisterError::RootCollision { id, existing: root_path });
            }
            Err(e) => return Err(e),
        };
        // It records what it found in its own folder, but never replaces the
        // project a registration with assigned ids recorded there.
        let folder = settled.written(settled.new_row.then_some(id.as_str()));
        (settled, id, folder)
    };
    let root_id = settled.folder.id.clone();
    let mut legacy = Vec::new();
    let legacy_ids: &[String] = if settled.owns_legacy { reg.legacy_ids } else { &[] };
    for id in legacy_ids.iter().chain(std::iter::once(&root_id)) {
        if *id == project_id || legacy.contains(id) {
            continue;
        }
        // A legacy id that is another root's own id would take that
        // project's rows, even when both roots are one path.
        if *id != root_id && root_of_in(c, id).map_err(sql)?.is_some() {
            log::warn!("{id} belongs to another registered project; its rows were not moved to {project_id}");
            continue;
        }
        legacy.push(id.clone());
    }
    let ids = serde_json::to_string(&legacy).unwrap_or_default();
    let moves: bool = c
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM memory WHERE project_id IN (SELECT value FROM json_each(?1))) \
             OR EXISTS (SELECT 1 FROM episodic_trajectory WHERE project_id IN (SELECT value FROM json_each(?1))) \
             OR EXISTS (SELECT 1 FROM procedural_skill WHERE project_id IN (SELECT value FROM json_each(?1)))",
            [ids],
            |r| r.get(0),
        )
        .map_err(sql)?;
    let folder_changed = folder != settled.folder;
    Ok(Planned::Plan(Plan {
        root_id,
        project_id,
        new_root: settled.new_row,
        legacy,
        moves,
        folder,
        folder_changed,
        moved_from,
        suggestions,
    }))
}

/// Writes `plan` inside the caller's transaction and returns how many rows
/// moved, the move that joined the rows of `memory`, and the mark
/// `index_docs` keeps.
fn write_in(
    c: &Connection,
    reg: &Registration<'_>,
    plan: &Plan,
    now: TimestampMs,
) -> rusqlite::Result<(usize, Option<i64>, bool)> {
    let root_path = reg.root.to_string_lossy().into_owned();
    let folder = &plan.folder;
    if plan.new_root {
        c.execute(
            "INSERT INTO project_path (id, root_path, created_at, index_docs, project_id, remote, root_commit, git_stamp) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                plan.root_id,
                root_path,
                now,
                reg.index_docs,
                folder.project_id,
                folder.remote,
                folder.root_commit,
                folder.git_stamp
            ],
        )?;
    } else if plan.folder_changed {
        c.execute(
            "UPDATE project_path SET project_id = ?2, remote = ?3, root_commit = ?4, git_stamp = ?5 WHERE id = ?1",
            rusqlite::params![plan.root_id, folder.project_id, folder.remote, folder.root_commit, folder.git_stamp],
        )?;
    }
    // A root nothing was saved in yet is a project too: a move may target it
    // before its first save.
    c.execute(
        "INSERT OR IGNORE INTO project (id, name, born, reviewed, created_at) VALUES (?1, ?2, 'root', 0, ?3)",
        rusqlite::params![plan.project_id, project_name(&plan.project_id), now],
    )?;
    if !plan.moved_from.is_empty() {
        let paths: Vec<String> = plan.moved_from.iter().map(|p| p.display().to_string()).collect();
        log::info!(
            "{root_path} holds the checkout recorded at {} before it moved; it keeps project {}",
            paths.join(", "),
            plan.project_id
        );
    }
    // A pair is suggested once whatever its order (the unique index), so an
    // answer keeps it from coming back.
    for (other, reason) in &plan.suggestions {
        let made = c.execute(
            "INSERT OR IGNORE INTO project_suggestion (project_id, other_id, reason, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![plan.project_id, other, reason, now],
        )?;
        if made > 0 {
            log::info!(
                "project {} of {root_path} may be the same as {other} ({reason}); `clawg projects list` lists the suggestion",
                plan.project_id
            );
        }
    }
    let indexed: bool = c.query_row("SELECT index_docs FROM project_path WHERE id = ?1", [&plan.root_id], |r| r.get(0))?;
    if reg.index_docs && !indexed {
        c.execute("UPDATE project_path SET index_docs = 1 WHERE id = ?1", [&plan.root_id])?;
    }
    if let (true, Some(remote)) = (reg.assigned, reg.checkout.key.as_deref()) {
        c.execute(
            "INSERT OR IGNORE INTO project_remote (remote, project_id, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![remote, plan.project_id, now],
        )?;
    }
    let (mut moved, mut move_id) = (0, None);
    if !plan.legacy.is_empty() {
        let merged = merge_in(c, &plan.legacy, &plan.project_id, Span::Keep, now)?;
        (moved, move_id) = (merged.moved.len(), merged.move_id);
        for legacy_id in &plan.legacy {
            for table in OTHER_PROJECT_TABLES {
                c.execute(
                    &format!(
                        "INSERT INTO project_rekey (tbl, row_id, legacy_id, project_id, rekeyed_at) \
                         SELECT '{table}', id, project_id, ?2, ?3 FROM {table} WHERE project_id = ?1"
                    ),
                    rusqlite::params![legacy_id, plan.project_id, now],
                )?;
                moved += c.execute(
                    &format!("UPDATE {table} SET project_id = ?2 WHERE project_id = ?1"),
                    rusqlite::params![legacy_id, plan.project_id],
                )?;
            }
        }
    }
    Ok((moved, move_id, indexed || reg.index_docs))
}

/// The outcome of one pass of [`register_project`].
enum Pass {
    Done(RegisterOutcome, Vec<String>),
    /// The first commit of the root is needed: the caller reads it and runs
    /// the pass again.
    WouldRead,
    /// The root may be one of these recorded checkouts, moved: the caller
    /// looks whether each is still there and runs the pass again.
    WouldLook(Vec<Recorded>),
    /// Rows would move: the caller snapshots and runs the pass again.
    WouldMove,
}

/// Records `reg.root` as a project and joins into its project every row of
/// each legacy id, soft-deleted rows included, then hands each legacy
/// `workspace.md` to the project (see [`WorkspaceManager::adopt_legacy`]).
/// When no folder of the root's path recorded its git directory, it first
/// reads the first commit from `ground` on a blocking thread, and when the
/// root may be a checkout moved from another path it looks there the same
/// way; when rows are to move it first snapshots the store as `pre-rekey`
/// into `backups`. Each time it decides again after.
///
/// `index_docs` only ever raises the stored mark: any tool that names a path
/// registers its root, and such a call must not take an index away from a
/// root the user accepted.
///
/// Idempotent: once a legacy id has no rows left, a repeat moves nothing.
pub async fn register_project(
    store: &MemoryStore,
    workspaces: &WorkspaceManager,
    backups: &Path,
    reg: Registration<'_>,
    ground: Arc<dyn Ground>,
) -> Result<RegisterOutcome, RegisterError> {
    if let Some(bad) = reg.legacy_ids.iter().find(|id| !is_project_id(id)) {
        return Err(RegisterError::InvalidLegacyId(bad.clone()));
    }
    let owned = (reg.root.to_path_buf(), reg.checkout.clone(), reg.legacy_ids.to_vec(), reg.subpath.map(str::to_string));
    let (index_docs, assigned) = (reg.index_docs, reg.assigned);
    let pass = move |snapshot: Option<PathBuf>, disk: FromDisk, may_read: bool, may_look: bool| {
        let (root, checkout, legacy_ids, subpath) = owned.clone();
        move |c: &mut Connection| -> Result<Pass, RegisterError> {
            let reg = Registration {
                root: &root,
                checkout,
                legacy_ids: &legacy_ids,
                index_docs,
                assigned,
                subpath: subpath.as_deref(),
            };
            let tx = c.transaction().map_err(MemoryError::Sqlite)?;
            let plan = match plan_in(&tx, &reg, &disk, may_read, may_look)? {
                Planned::Plan(plan) => plan,
                Planned::Commit => return Ok(Pass::WouldRead),
                Planned::Presence(recorded) => return Ok(Pass::WouldLook(recorded)),
            };
            if plan.moves && snapshot.is_none() {
                return Ok(Pass::WouldMove);
            }
            let (rekeyed, move_id, index_docs) =
                write_in(&tx, &reg, &plan, TimestampMs::now()).map_err(MemoryError::Sqlite)?;
            // Only once the root settled and its ids joined there: a plan made
            // for the subproject would join the repository's rows into it.
            let subproject = match reg.subpath {
                Some(subpath) if reg.assigned => declared_in(&tx, &plan.project_id, subpath).map_err(MemoryError::Sqlite)?,
                _ => None,
            };
            tx.commit().map_err(MemoryError::Sqlite)?;
            let outcome = RegisterOutcome {
                project_id: plan.project_id,
                root_id: plan.root_id,
                rekeyed,
                index_docs,
                move_id,
                snapshot,
                subproject,
            };
            Ok(Pass::Done(outcome, plan.legacy))
        }
    };
    let mut disk = FromDisk::default();
    let mut outcome = store.conn.call(pass(None, disk.clone(), true, true)).await.map_err(connection_error)?;
    if let Pass::WouldRead = outcome {
        // `git` may take up to ROOT_COMMIT_WAIT, which the connection's
        // thread must not spend.
        let (root, ground) = (reg.root.to_path_buf(), ground.clone());
        let commit = tokio::task::spawn_blocking(move || ground.root_commit(&root)).await.unwrap_or(RootCommit::Later);
        disk.commit = Some(commit);
        outcome = store.conn.call(pass(None, disk.clone(), false, true)).await.map_err(connection_error)?;
    }
    if let Pass::WouldLook(recorded) = outcome {
        // A look that fails finds nothing gone, which keeps the root apart.
        let gone = tokio::task::spawn_blocking(move || {
            recorded.into_iter().filter(|(path, stamp)| !ground.recorded_checkout_present(path, stamp.as_deref())).collect()
        });
        disk.gone = Some(gone.await.unwrap_or_default());
        outcome = store.conn.call(pass(None, disk.clone(), false, false)).await.map_err(connection_error)?;
    }
    if let Pass::WouldMove = outcome {
        let snapshot = store.snapshot(backups.to_path_buf(), Reason::PreRekey).await?;
        outcome = store.conn.call(pass(Some(snapshot), disk, false, false)).await.map_err(connection_error)?;
    }
    let Pass::Done(outcome, legacy) = outcome else {
        unreachable!("a pass stops only for what no pass before it asked, and a pass after the snapshot always writes");
    };

    // After the commit and outside it: a file copy that fails must not undo
    // rows already moved, and the legacy file is still there for the next try.
    for legacy_id in &legacy {
        if let Err(e) = workspaces.adopt_legacy(legacy_id, &outcome.project_id, true).await {
            log::warn!("could not carry the workspace of {legacy_id} over to {}: {e}", outcome.project_id);
        }
    }
    Ok(outcome)
}

fn connection_error(e: tokio_rusqlite::Error<RegisterError>) -> RegisterError {
    match e {
        tokio_rusqlite::Error::Error(e) => e,
        other => RegisterError::Store(MemoryError::Backend(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::identity::{candidate_id, FakeGround};
    use crate::memory::relocate::undo_move_core;

    #[test]
    fn project_ids_are_a_name_a_dash_and_four_hex_digits() {
        for ok in ["repo-1a2b", "claw-garden-00ff", "project-0000"] {
            assert!(is_project_id(ok), "{ok}");
        }
        for bad in ["repo", "-1a2b", "repo-1A2B", "repo-1a2", "repo-1a2bc", "repo-zzzz", "", "repo 1a2b"] {
            assert!(!is_project_id(bad), "{bad}");
        }
    }

    struct Fixture {
        store: MemoryStore,
        ws: WorkspaceManager,
        _dir: tempfile::TempDir,
        backups: PathBuf,
        ground: Arc<FakeGround>,
    }

    impl Fixture {
        async fn new() -> Self {
            let store = MemoryStore::new_for_test().await.unwrap();
            let dir = tempfile::tempdir().unwrap();
            let ws = WorkspaceManager::new(dir.path().join("workspaces"));
            let backups = dir.path().join("backups");
            Fixture { store, ws, _dir: dir, backups, ground: Arc::new(FakeGround::default()) }
        }

        async fn seeded() -> Self {
            let f = Self::new().await;
            for (project, content, deleted) in
                [("src-1a2b", "zanzibar parser decision", None), ("src-1a2b", "second fact", Some(5_i64)), ("other-9f9f", "unrelated", None)]
            {
                f.memory(project, content, deleted, 0).await;
            }
            f.store
                .conn
                .call(|c| {
                    c.execute(
                        "INSERT INTO episodic_trajectory \
                         (scope, agent_id, project_id, session_id, task, created_at, \
                          last_modified_at, last_accessed_at) \
                         VALUES ('agent', 'agent-a', 'src-1a2b', 's', 't', 0, 0, 0)",
                        [],
                    )?;
                    c.execute(
                        "INSERT INTO procedural_skill \
                         (scope, agent_id, project_id, skill_name, trigger, promoted_at, \
                          source_trajectory_ids, created_at, last_modified_at, last_accessed_at) \
                         VALUES ('agent', 'agent-a', 'src-1a2b', 'k', 't', 0, '[]', 0, 0, 0)",
                        [],
                    )?;
                    Ok::<_, rusqlite::Error>(())
                })
                .await
                .unwrap();
            f
        }

        async fn memory(&self, project: &str, content: &str, deleted: Option<i64>, created_at: i64) -> i64 {
            let (project, content) = (project.to_string(), content.to_string());
            self.store
                .conn
                .call(move |c| {
                    c.execute(
                        "INSERT INTO memory \
                         (scope, agent_id, project_id, content, memory_type, source, \
                          created_at, created_by_type, created_by_ref, \
                          last_modified_at, last_accessed_at, importance, tier, status, deleted_at, content_hash) \
                         VALUES ('agent', 'agent-a', ?1, ?2, 'observation', 'agent', \
                          ?4, 'agent', 'agent-a', 0, 0, 5.0, 3, 'active', ?3, 'h:' || ?2)",
                        rusqlite::params![project, content, deleted, created_at],
                    )?;
                    Ok::<_, rusqlite::Error>(c.last_insert_rowid())
                })
                .await
                .unwrap()
        }

        async fn root(&self, id: &str, root: &str) {
            let (id, root) = (id.to_string(), root.to_string());
            self.store
                .conn
                .call(move |c| {
                    c.execute(
                        "INSERT INTO project_path (id, root_path, created_at, index_docs) VALUES (?1, ?2, 0, 0)",
                        rusqlite::params![id, root],
                    )
                })
                .await
                .unwrap();
        }

        async fn register(&self, reg: Registration<'_>) -> Result<RegisterOutcome, RegisterError> {
            register_project(&self.store, &self.ws, &self.backups, reg, self.ground.clone()).await
        }

        /// Puts at `root` a git directory stamped `stamp` whose first commit is
        /// `commit`, and the registration the daemon would make of it.
        fn found<'a>(&self, root: &'a Path, key: Option<&str>, stamp: &str, commit: RootCommit) -> Registration<'a> {
            self.ground.put(root, key, Some(stamp), commit);
            Registration { checkout: self.ground.checkout(root), ..reg(root, &[]) }
        }

        /// Each folder of `root`: its id, project, remote, first commit and stamp.
        async fn folders(&self, root: &str) -> Vec<FolderRow> {
            let root = root.to_string();
            self.store
                .conn
                .call(move |c| {
                    let mut stmt = c.prepare(
                        "SELECT id, project_id, remote, root_commit, git_stamp FROM project_path \
                         WHERE root_path = ?1 ORDER BY created_at, id",
                    )?;
                    let rows = stmt.query_map([root], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
                    rows.collect::<rusqlite::Result<_>>()
                })
                .await
                .unwrap()
        }

        async fn count(&self, sql: &'static str) -> i64 {
            self.store.conn.call(move |c| c.query_row(sql, [], |r| r.get(0))).await.unwrap()
        }

        /// Each suggestion: the project made apart, the other, the reason and the answer.
        async fn suggestions(&self) -> Vec<SuggestionRow> {
            self.store
                .conn
                .call(|c| {
                    let mut stmt = c.prepare(
                        "SELECT project_id, other_id, reason, answer FROM project_suggestion ORDER BY project_id, other_id",
                    )?;
                    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
                    rows.collect::<rusqlite::Result<_>>()
                })
                .await
                .unwrap()
        }

        async fn snapshots(&self) -> usize {
            std::fs::read_dir(&self.backups).map(|d| d.count()).unwrap_or(0)
        }
    }

    type FolderRow = (String, Option<String>, Option<String>, Option<String>, Option<String>);
    type SuggestionRow = (String, String, String, Option<String>);

    fn reg<'a>(root: &'a Path, legacy: &'a [String]) -> Registration<'a> {
        Registration { root, checkout: Checkout::default(), legacy_ids: legacy, index_docs: true, assigned: true, subpath: None }
    }

    fn keyed(remote: &str) -> Checkout {
        Checkout { key: Some(remote.to_string()), stamp: None }
    }

    fn cloned<'a>(root: &'a Path, remote: &str) -> Registration<'a> {
        Registration { checkout: keyed(remote), ..reg(root, &[]) }
    }

    fn known(commit: &str) -> RootCommit {
        RootCommit::Known(commit.to_string())
    }

    fn some(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    async fn project_ids(store: &MemoryStore) -> Vec<(String, i64, Option<String>)> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT 'memory', id, project_id FROM memory \
                     UNION ALL SELECT 'episodic_trajectory', id, project_id FROM episodic_trajectory \
                     UNION ALL SELECT 'procedural_skill', id, project_id FROM procedural_skill \
                     ORDER BY 1, 2",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect();
                rows
            })
            .await
            .unwrap()
    }

    async fn index_docs_of(store: &MemoryStore, id: &str) -> Vec<i64> {
        let id = id.to_string();
        store
            .conn
            .call(move |c| {
                let mut stmt = c.prepare("SELECT index_docs FROM project_path WHERE id = ?1")?;
                let marks = stmt.query_map([id], |r| r.get(0))?.collect();
                marks
            })
            .await
            .unwrap()
    }

    /// A move may target a root before anything is saved in it; a clone
    /// joins the project of its remote and makes none.
    #[tokio::test]
    async fn a_registered_root_is_a_project_before_its_first_save() {
        let f = Fixture::new().await;
        let first = f.register(cloned(Path::new("/work/app"), "github.com/acme/app")).await.unwrap();
        let clone = f.register(cloned(Path::new("/review/app"), "github.com/acme/app")).await.unwrap();

        assert_eq!(clone.project_id, first.project_id);
        let projects: Vec<(String, String, String, i64)> = f
            .store
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT id, name, born, reviewed FROM project")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
                rows.collect::<rusqlite::Result<_>>()
            })
            .await
            .unwrap();
        assert_eq!(projects, vec![(first.project_id, "app".to_string(), "root".to_string(), 0)]);
    }

    #[tokio::test]
    async fn rows_of_a_legacy_id_move_to_the_resolved_root() {
        let f = Fixture::seeded().await;
        let root = Path::new("/work/repo");
        let id = derive_project_id(root);
        let legacy = ["src-1a2b".to_string()];

        let outcome = f.register(reg(root, &legacy)).await.unwrap();

        assert_eq!((outcome.project_id.as_str(), outcome.root_id.as_str(), outcome.rekeyed, outcome.index_docs), (id.as_str(), id.as_str(), 4, true));
        let ids = project_ids(&f.store).await;
        let moved = ids.iter().filter(|(_, _, p)| p.as_deref() == Some(id.as_str())).count();
        assert_eq!(moved, 4, "{ids:?}");
        assert!(ids.iter().any(|(_, _, p)| p.as_deref() == Some("other-9f9f")), "{ids:?}");
        assert_eq!(f.count("SELECT COUNT(*) FROM memory_move").await, 2, "memory rows are logged as a move");
        assert_eq!(f.count("SELECT COUNT(*) FROM project_rekey").await, 2, "the other tables in project_rekey");
        assert_eq!(f.count("SELECT COUNT(*) FROM project_path").await, 1);
    }

    #[tokio::test]
    async fn registering_again_moves_nothing() {
        let f = Fixture::seeded().await;
        let root = Path::new("/work/repo");
        let legacy = ["src-1a2b".to_string()];
        f.register(reg(root, &legacy)).await.unwrap();

        let again = f.register(reg(root, &legacy)).await.unwrap();

        assert_eq!((again.rekeyed, again.snapshot), (0, None));
        assert_eq!(f.count("SELECT COUNT(*) FROM memory_move").await, 2);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_rekey").await, 2);
        assert_eq!(f.snapshots().await, 1, "only the registration that moved rows took one");
    }

    #[tokio::test]
    async fn a_registration_that_moves_rows_snapshots_first() {
        let f = Fixture::seeded().await;
        let nothing = f.register(reg(Path::new("/work/empty"), &[])).await.unwrap();
        assert_eq!((nothing.snapshot, f.snapshots().await), (None, 0));

        let legacy = ["src-1a2b".to_string()];
        let moved = f.register(reg(Path::new("/work/repo"), &legacy)).await.unwrap();

        let snapshot = moved.snapshot.expect("a snapshot");
        let name = snapshot.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with("-pre-rekey.db"), "{name}");
        let held: i64 = rusqlite::Connection::open(&snapshot)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM memory WHERE project_id = 'src-1a2b'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(held, 2, "the snapshot is of the store before the move");
    }

    #[tokio::test]
    async fn the_logs_restore_every_row_a_registration_moved() {
        let f = Fixture::seeded().await;
        let before = project_ids(&f.store).await;
        let legacy = ["src-1a2b".to_string()];
        let moved = f.register(reg(Path::new("/work/repo"), &legacy)).await.unwrap();
        let again = f.register(reg(Path::new("/work/repo"), &legacy)).await.unwrap();
        assert_eq!((moved.move_id, again.move_id), (Some(1), None), "a registration that moves nothing makes no move");

        undo_move_core(&f.store, &f.backups, 1, false).await.unwrap();
        f.store
            .conn
            .call(|c| {
                for table in OTHER_PROJECT_TABLES {
                    c.execute(
                        &format!(
                            "UPDATE {table} SET project_id = r.legacy_id FROM project_rekey AS r \
                             WHERE r.tbl = '{table}' AND r.row_id = {table}.id \
                               AND r.project_id = {table}.project_id"
                        ),
                        [],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .unwrap();

        assert_eq!(project_ids(&f.store).await, before);
    }

    #[tokio::test]
    async fn a_moved_row_is_found_lexically_under_its_new_project() {
        let f = Fixture::seeded().await;
        let legacy = ["src-1a2b".to_string()];
        let outcome = f.register(reg(Path::new("/work/repo"), &legacy)).await.unwrap();

        let id = outcome.project_id;
        let found: i64 = f
            .store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT COUNT(*) FROM memory_fts WHERE memory_fts MATCH 'zanzibar' AND project_id = ?1",
                    [id],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(found, 1);
    }

    #[tokio::test]
    async fn the_root_of_always_keeps_its_id() {
        let f = Fixture::new().await;
        let root = Path::new("/work/repo");
        let id = derive_project_id(root);
        let row = f.memory(&id, "saved before ids were assigned", None, 0).await;

        let outcome = f.register(cloned(root, "github.com/acme/repo")).await.unwrap();

        assert_eq!((outcome.project_id.as_str(), outcome.root_id.as_str(), outcome.rekeyed), (id.as_str(), id.as_str(), 0));
        assert_eq!(project_ids(&f.store).await, vec![("memory".into(), row, Some(id))]);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_remote").await, 1);
    }

    #[tokio::test]
    async fn a_root_whose_id_is_taken_gets_the_next_free_one() {
        let f = Fixture::new().await;
        let root = Path::new("/work/repo");
        let taken = derive_project_id(root);
        f.root(&taken, "/elsewhere/repo").await;
        let theirs = f.memory(&taken, "the other project's note", None, 0).await;

        let outcome = f.register(reg(root, &[])).await.unwrap();

        assert_eq!(outcome.root_id, candidate_id(root, 1));
        assert_eq!(outcome.project_id, outcome.root_id);
        assert_eq!(outcome.rekeyed, 0, "the other root's id is never a legacy one");
        assert_eq!(project_ids(&f.store).await, vec![("memory".into(), theirs, Some(taken.clone()))]);
        assert_eq!(index_docs_of(&f.store, &taken).await, vec![0], "the other root's mark is left alone");
        assert_eq!(index_docs_of(&f.store, &outcome.root_id).await, vec![1]);
    }

    #[tokio::test]
    async fn a_root_keeps_the_id_it_was_given() {
        let f = Fixture::new().await;
        let root = Path::new("/work/repo");
        f.root(&derive_project_id(root), "/elsewhere/repo").await;
        let first = f.register(reg(root, &[])).await.unwrap();

        let again = f.register(reg(root, &[])).await.unwrap();

        assert_eq!(again.root_id, first.root_id);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_path").await, 2);
    }

    #[tokio::test]
    async fn every_id_taken_is_an_error_that_writes_nothing() {
        let f = Fixture::new().await;
        let root = Path::new("/work/repo");
        for n in 0..CANDIDATES {
            f.root(&candidate_id(root, n), &format!("/elsewhere/{n}")).await;
        }

        let err = f.register(reg(root, &[])).await.unwrap_err();

        assert!(matches!(err, RegisterError::NoFreeId(_)), "{err:?}");
        assert_eq!(f.count("SELECT COUNT(*) FROM project_path").await, i64::from(CANDIDATES));
    }

    #[tokio::test]
    async fn a_second_clone_joins_the_project_of_the_first() {
        let f = Fixture::new().await;
        let (work, review) = (Path::new("/work/app"), Path::new("/review/app"));
        let ours = f.memory(&derive_project_id(work), "tests run with nextest", None, 1).await;
        let theirs = f.memory(&derive_project_id(review), "the socket is clawg.sock", None, 2).await;
        let first = f.register(cloned(work, "github.com/acme/app")).await.unwrap();

        let second = f.register(cloned(review, "github.com/acme/app")).await.unwrap();

        assert_eq!(second.project_id, first.project_id);
        assert_eq!(second.root_id, derive_project_id(review), "the clone keeps its own root id");
        assert_eq!(second.rekeyed, 1);
        assert!(second.snapshot.is_some());
        let p = Some(first.project_id.clone());
        assert_eq!(project_ids(&f.store).await, vec![("memory".into(), ours, p.clone()), ("memory".into(), theirs, p)]);
        assert_eq!(f.count("SELECT COUNT(*) FROM memory_move WHERE row_id = 2").await, 1);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_path").await, 2);
    }

    #[tokio::test]
    async fn a_clone_joins_with_one_live_row_per_text() {
        let f = Fixture::new().await;
        let (work, review) = (Path::new("/work/app"), Path::new("/review/app"));
        let older = f.memory(&derive_project_id(work), "the socket is clawg.sock", None, 1).await;
        let newer = f.memory(&derive_project_id(review), "the socket is clawg.sock", None, 2).await;
        f.register(cloned(work, "github.com/acme/app")).await.unwrap();

        f.register(cloned(review, "github.com/acme/app")).await.unwrap();

        let live = f.count("SELECT COUNT(*) FROM memory WHERE status = 'active'").await;
        assert_eq!(live, 1);
        let by: i64 = f
            .store
            .conn
            .call(move |c| c.query_row("SELECT superseded_by FROM memory WHERE id = ?1", [newer], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(by, older);
    }

    /// A folder from before V27, known by the remotes bound to its id.
    fn bound(remotes: &[&str]) -> Folder {
        Folder { bound: remotes.iter().map(|r| r.to_string()).collect(), ..Folder::default() }
    }

    #[test]
    fn a_path_settles_on_the_row_its_remote_does_not_contradict() {
        let settle = |folders: &[Folder], remote: Option<&str>| settle(folders, remote, None);
        let none = bound(&[]);
        let a = bound(&["github.com/acme/a"]);
        let b = bound(&["github.com/acme/b"]);

        assert_eq!(settle(&[], Some("github.com/acme/a")), Settled::Fresh);
        assert_eq!(settle(&[], None), Settled::Fresh);
        assert_eq!(settle(&[a.clone()], None), Settled::Row(0), "an unknown remote contradicts nothing");
        assert_eq!(settle(&[a.clone()], Some("github.com/acme/a")), Settled::Row(0));
        assert_eq!(settle(&[none.clone()], Some("github.com/acme/a")), Settled::Row(0), "a first remote");
        assert_eq!(settle(&[a.clone()], Some("github.com/acme/b")), Settled::Contradicted);
        assert_eq!(settle(&[a.clone(), b.clone()], Some("github.com/acme/b")), Settled::Row(1));
        assert_eq!(settle(&[a.clone(), b.clone()], Some("github.com/acme/c")), Settled::Contradicted);
        assert_eq!(settle(&[a.clone(), none, b], Some("github.com/acme/c")), Settled::Row(1));
    }

    #[test]
    fn a_checkout_settles_on_the_folder_its_fingerprints_do_not_contradict() {
        let folder = |remote: Option<&str>, commit: Option<&str>| Folder {
            remote: remote.map(str::to_string),
            root_commit: commit.map(str::to_string),
            ..Folder::default()
        };
        let (a, b) = (Some("github.com/acme/a"), Some("github.com/acme/b"));
        let (c1, c2) = (Some("c1"), Some("c2"));

        assert_eq!(fit(&folder(a, c1), a, c2), Fit::Same, "the remote decides before the first commit");
        assert_eq!(fit(&folder(a, c1), b, c1), Fit::Contradicted, "a fork shares the first commit");
        assert_eq!(fit(&folder(None, c1), a, c1), Fit::Same, "a first remote, and the same history");
        assert_eq!(fit(&folder(None, c1), a, c2), Fit::Contradicted);
        assert_eq!(fit(&folder(a, c1), None, c2), Fit::Contradicted, "a remote known on one side only");
        assert_eq!(fit(&folder(None, c1), None, c1), Fit::Same);
        assert_eq!(fit(&folder(None, None), a, c1), Fit::Unknown);
        assert_eq!(fit(&folder(a, c1), None, None), Fit::Unknown);
        assert_eq!(fit(&bound(&["github.com/acme/a", "github.com/acme/b"]), b, None), Fit::Same, "a fused folder");
        assert_eq!(fit(&Folder { remote: a.map(str::to_string), ..bound(&["github.com/acme/b"]) }, b, None), Fit::Contradicted, "its own remote first");

        assert_eq!(settle(&[folder(None, None), folder(None, c1)], None, c1), Settled::Row(1), "the same before an older unknown");
        assert_eq!(settle(&[folder(a, None), folder(None, None), folder(None, None)], b, None), Settled::Row(1), "the oldest unknown");
        assert_eq!(settle(&[folder(None, c1), folder(a, None)], b, c2), Settled::Contradicted);
    }

    #[tokio::test]
    async fn a_repository_in_the_path_of_another_gets_a_project_of_its_own() {
        let f = Fixture::new().await;
        let path = Path::new("/work/app");
        let a = f.register(cloned(path, "github.com/acme/a")).await.unwrap();
        let theirs = f.memory(&a.project_id, "the parser of a", None, 1).await;

        let b = f.register(cloned(path, "github.com/acme/b")).await.unwrap();

        assert_ne!(b.project_id, a.project_id);
        assert_ne!(b.root_id, a.root_id);
        assert_eq!((b.project_id.as_str(), b.root_id.as_str()), (candidate_id(path, 1).as_str(), candidate_id(path, 1).as_str()));
        assert_eq!((b.rekeyed, b.snapshot), (0, None));
        assert_eq!(project_ids(&f.store).await, vec![("memory".into(), theirs, Some(a.project_id.clone()))]);
        let bound: String = f
            .store
            .conn
            .call(|c| c.query_row("SELECT project_id FROM project_remote WHERE remote = 'github.com/acme/b'", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(bound, b.project_id, "the newcomer's remote is bound to its own project");

        let clone = f.register(cloned(Path::new("/review/b"), "github.com/acme/b")).await.unwrap();
        assert_eq!(clone.project_id, b.project_id, "a later clone of its remote joins it");
    }

    #[tokio::test]
    async fn a_repository_in_the_path_of_another_takes_none_of_its_rows() {
        let f = Fixture::new().await;
        let path = Path::new("/work/app");
        let sub = derive_project_id(Path::new("/work/app/sub"));
        let a = f.register(cloned(path, "github.com/acme/a")).await.unwrap();
        let older = f.memory(&sub, "saved under the subfolder before roots were resolved", None, 1).await;
        let mine = f.memory(&a.project_id, "the parser of a", None, 2).await;
        f.register(cloned(Path::new("/else/w"), "github.com/acme/w")).await.unwrap();
        let legacy = [sub.clone()];

        let w = f
            .register(Registration { checkout: keyed("github.com/acme/w"), ..reg(path, &legacy) })
            .await
            .unwrap();

        assert_ne!(w.root_id, a.root_id);
        assert_eq!((w.rekeyed, w.snapshot), (0, None));
        assert_eq!(
            project_ids(&f.store).await,
            vec![("memory".into(), older, Some(sub)), ("memory".into(), mine, Some(a.project_id))]
        );
    }

    #[tokio::test]
    async fn each_repository_in_one_path_returns_to_its_own_project() {
        let f = Fixture::new().await;
        let path = Path::new("/work/app");
        let a = f.register(cloned(path, "github.com/acme/a")).await.unwrap();
        let b = f.register(cloned(path, "github.com/acme/b")).await.unwrap();

        let a_again = f.register(cloned(path, "github.com/acme/a")).await.unwrap();
        let b_again = f.register(cloned(path, "github.com/acme/b")).await.unwrap();

        assert_eq!((a_again.project_id, a_again.root_id), (a.project_id, a.root_id));
        assert_eq!((b_again.project_id, b_again.root_id), (b.project_id, b.root_id));
        assert_eq!(f.count("SELECT COUNT(*) FROM project_path").await, 2, "no third row");
        assert_eq!(f.count("SELECT COUNT(*) FROM project_remote").await, 2);
    }

    #[tokio::test]
    async fn the_project_of_a_repository_in_another_s_path_is_read_alike_without_writing() {
        let f = Fixture::new().await;
        let path = Path::new("/work/app");
        let a = f.register(cloned(path, "github.com/acme/a")).await.unwrap();
        f.ground.put(path, Some("github.com/acme/b"), None, RootCommit::Later);
        let ground = f.ground.clone();

        let (read, planned) = f
            .store
            .conn
            .call(move |c| {
                let read = project_id_in(c, path, None, &*ground)?;
                let planned = plan_ids_in(c, &[path.to_path_buf(), path.to_path_buf()], &*ground)?;
                Ok::<_, rusqlite::Error>((read, planned.into_iter().map(|p| (p.root_id, p.project_id)).collect::<Vec<_>>()))
            })
            .await
            .unwrap();
        assert_eq!(f.count("SELECT COUNT(*) FROM project_path").await, 1, "reading writes nothing");
        let b = f.register(cloned(path, "github.com/acme/b")).await.unwrap();

        assert_ne!(read, a.project_id);
        assert_eq!(read, b.project_id);
        let ids = (b.root_id.clone(), b.project_id.clone());
        assert_eq!(planned, vec![ids.clone(), ids], "one path and remote twice in a batch is one project");
    }

    #[tokio::test]
    async fn a_first_remote_keeps_the_project_of_its_root() {
        let f = Fixture::new().await;
        let root = Path::new("/work/app");
        let local = f.register(reg(root, &[])).await.unwrap();

        let pushed = f.register(cloned(root, "github.com/acme/app")).await.unwrap();

        assert_eq!((pushed.project_id, pushed.root_id), (local.project_id.clone(), local.root_id));
        assert_eq!(f.count("SELECT COUNT(*) FROM project_path").await, 1);
        let unknown = f.register(reg(root, &[])).await.unwrap();
        assert_eq!(unknown.project_id, local.project_id, "a root whose remote cannot be read keeps its project");
    }

    #[tokio::test]
    async fn a_different_remote_in_a_registered_path_is_another_project() {
        let f = Fixture::new().await;
        let root = Path::new("/work/app");
        let first = f.register(cloned(root, "github.com/acme/app")).await.unwrap();

        let moved = f.register(cloned(root, "gitlab.com/acme/app")).await.unwrap();

        assert_ne!(moved.project_id, first.project_id);
        assert_eq!(moved.rekeyed, 0);
    }

    #[tokio::test]
    async fn a_caller_that_derives_its_id_is_refused_another_repository_s_project() {
        let f = Fixture::new().await;
        let path = Path::new("/work/app");
        let a = f.register(cloned(path, "github.com/acme/a")).await.unwrap();
        f.memory(&a.project_id, "the parser of a", None, 1).await;
        let before = project_ids(&f.store).await;

        let err = f
            .register(Registration { assigned: false, ..cloned(path, "github.com/acme/b") })
            .await
            .unwrap_err();

        assert!(matches!(err, RegisterError::RootCollision { ref id, .. } if *id == a.root_id), "{err:?}");
        assert_eq!(project_ids(&f.store).await, before);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_path").await, 1);
        let same = f.register(Registration { assigned: false, ..cloned(path, "github.com/acme/a") }).await.unwrap();
        assert_eq!(same.project_id, a.project_id, "its own repository is still served");
    }

    #[tokio::test]
    async fn two_remotes_are_two_projects() {
        let f = Fixture::new().await;
        let ours = f.register(cloned(Path::new("/work/app"), "github.com/acme/app")).await.unwrap();
        let fork = f.register(cloned(Path::new("/fork/app"), "github.com/me/app")).await.unwrap();

        assert_ne!(ours.project_id, fork.project_id);
    }

    #[tokio::test]
    async fn a_caller_that_derives_its_id_keeps_the_conflict() {
        let f = Fixture::seeded().await;
        let root = Path::new("/work/repo");
        let id = derive_project_id(root);
        f.root(&id, "/elsewhere/repo").await;
        let before = project_ids(&f.store).await;
        let legacy = ["src-1a2b".to_string()];

        let err = f.register(Registration { assigned: false, ..reg(root, &legacy) }).await.unwrap_err();

        assert!(matches!(err, RegisterError::RootCollision { ref existing, .. } if existing == "/elsewhere/repo"), "{err:?}");
        assert_eq!(project_ids(&f.store).await, before);
        assert_eq!(index_docs_of(&f.store, &id).await, vec![0], "the other root's mark is left alone");
        assert_eq!(f.count("SELECT COUNT(*) FROM project_rekey").await, 0);

        // Given another id since, the root is still refused to such a caller.
        let given = f.register(reg(root, &[])).await.unwrap();
        assert_ne!(given.root_id, id);
        let err = f.register(Registration { assigned: false, ..reg(root, &[]) }).await.unwrap_err();
        assert!(matches!(err, RegisterError::RootCollision { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn a_caller_that_derives_its_id_joins_no_remote() {
        let f = Fixture::new().await;
        let first = f.register(cloned(Path::new("/work/app"), "github.com/acme/app")).await.unwrap();
        let review = Path::new("/review/app");

        let old = f.register(Registration { assigned: false, ..cloned(review, "github.com/acme/app") }).await.unwrap();

        assert_eq!(old.project_id, derive_project_id(review));
        assert_ne!(old.project_id, first.project_id);
    }

    #[tokio::test]
    async fn registering_to_index_raises_the_mark_and_never_lowers_it() {
        let f = Fixture::seeded().await;
        let root = Path::new("/work/repo");
        let id = derive_project_id(root);

        let first = f.register(Registration { index_docs: false, ..reg(root, &[]) }).await.unwrap();
        assert!(!first.index_docs);
        assert_eq!(index_docs_of(&f.store, &id).await, vec![0]);

        let raised = f.register(reg(root, &[])).await.unwrap();
        assert!(raised.index_docs);
        assert_eq!(index_docs_of(&f.store, &id).await, vec![1]);

        let kept = f.register(Registration { index_docs: false, ..reg(root, &[]) }).await.unwrap();
        assert!(kept.index_docs, "a later registration that does not index keeps the mark");
        assert_eq!(index_docs_of(&f.store, &id).await, vec![1]);
    }

    #[tokio::test]
    async fn a_malformed_legacy_id_is_refused_before_anything_moves() {
        let f = Fixture::seeded().await;
        let before = project_ids(&f.store).await;
        let legacy = ["src-1a2b".to_string(), "x' OR 1".to_string()];

        let err = f.register(reg(Path::new("/work/repo"), &legacy)).await.unwrap_err();

        assert!(matches!(err, RegisterError::InvalidLegacyId(_)), "{err:?}");
        assert_eq!(project_ids(&f.store).await, before);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_path").await, 0);
    }

    #[tokio::test]
    async fn a_legacy_id_of_another_registered_project_keeps_its_rows_and_workspace() {
        let f = Fixture::seeded().await;
        f.root("src-1a2b", "/work/other").await;
        let theirs = f.ws.workspace_path("agent-a", Some("src-1a2b"));
        std::fs::create_dir_all(theirs.parent().unwrap()).unwrap();
        std::fs::write(&theirs, "notes of the other project").unwrap();
        let before = project_ids(&f.store).await;
        let legacy = ["src-1a2b".to_string()];

        let outcome = f.register(reg(Path::new("/work/repo"), &legacy)).await.unwrap();

        assert_eq!(outcome.rekeyed, 0);
        assert_eq!(project_ids(&f.store).await, before);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_rekey").await, 0);
        assert!(!f.ws.workspace_path("agent-a", Some(&outcome.project_id)).exists());
    }

    #[tokio::test]
    async fn registering_carries_the_legacy_workspace_over() {
        let f = Fixture::seeded().await;
        let legacy_file = f.ws.workspace_path("agent-a", Some("src-1a2b"));
        std::fs::create_dir_all(legacy_file.parent().unwrap()).unwrap();
        std::fs::write(&legacy_file, "notes").unwrap();
        let legacy = ["src-1a2b".to_string()];

        let outcome = f.register(reg(Path::new("/work/repo"), &legacy)).await.unwrap();

        let adopted = f.ws.workspace_path("agent-a", Some(&outcome.project_id));
        assert_eq!(std::fs::read_to_string(adopted).unwrap(), "notes");
        assert!(legacy_file.exists());
    }

    #[tokio::test]
    async fn a_clone_hands_its_workspace_to_the_shared_project() {
        let f = Fixture::new().await;
        let (work, review) = (Path::new("/work/app"), Path::new("/review/app"));
        let first = f.register(cloned(work, "github.com/acme/app")).await.unwrap();
        let mine = f.ws.workspace_path("agent-a", Some(&derive_project_id(review)));
        std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
        std::fs::write(&mine, "review notes").unwrap();

        f.register(cloned(review, "github.com/acme/app")).await.unwrap();

        let shared = f.ws.workspace_path("agent-a", Some(&first.project_id));
        assert_eq!(std::fs::read_to_string(shared).unwrap(), "review notes");
    }

    #[tokio::test]
    async fn the_project_of_a_root_is_read_without_writing() {
        let f = Fixture::new().await;
        let (work, review, other) = (Path::new("/work/app"), Path::new("/review/app"), Path::new("/work/other"));
        let first = f.register(cloned(work, "github.com/acme/app")).await.unwrap();
        f.root(&derive_project_id(other), "/somewhere/else").await;
        let ground = f.ground.clone();

        let ids = f
            .store
            .conn
            .call(move |c| {
                ground.put(review, Some("github.com/acme/app"), None, RootCommit::Later);
                let shared = project_id_in(c, review, None, &*ground)?;
                ground.put(review, None, None, RootCommit::Later);
                Ok::<_, rusqlite::Error>((shared, project_id_in(c, review, None, &*ground)?, project_id_in(c, other, None, &*ground)?))
            })
            .await
            .unwrap();

        assert_eq!(ids, (first.project_id, derive_project_id(review), candidate_id(other, 1)));
        assert_eq!(f.count("SELECT COUNT(*) FROM project_path").await, 2, "reading writes nothing");
    }

    #[tokio::test]
    async fn a_recorded_checkout_does_not_read_its_first_commit_again() {
        let f = Fixture::new().await;
        let root = Path::new("/work/app");
        let first = f.register(f.found(root, None, "s1", known("c1"))).await.unwrap();
        assert_eq!(f.ground.commit_reads(), 1);
        let written = f.count("SELECT total_changes()").await;

        let again = f.register(f.found(root, None, "s1", known("c1"))).await.unwrap();
        assert_eq!(f.ground.commit_reads(), 1, "the git directory it recorded");
        assert_eq!(f.count("SELECT total_changes()").await, written, "nor writes anything");
        let recloned = f.register(f.found(root, None, "s2", known("c1"))).await.unwrap();
        assert_eq!(f.ground.commit_reads(), 2, "another git directory in the same path");

        assert_eq!((again.project_id.as_str(), recloned.project_id.as_str()), (first.project_id.as_str(), first.project_id.as_str()));
        let id = first.root_id.clone();
        assert_eq!(f.folders("/work/app").await, vec![(id.clone(), Some(id), None, some("c1"), some("s2"))]);

        let shallow = Path::new("/work/shallow");
        f.register(f.found(shallow, None, "t1", RootCommit::Unknowable)).await.unwrap();
        f.register(f.found(shallow, None, "t1", RootCommit::Unknowable)).await.unwrap();
        assert_eq!(f.ground.commit_reads(), 3, "a first commit not to be known is recorded as such");
        let folder = f.folders("/work/shallow").await.remove(0);
        assert_eq!((folder.3, folder.4), (None, some("t1")));

        let empty = Path::new("/work/empty");
        f.register(f.found(empty, None, "u1", RootCommit::Later)).await.unwrap();
        f.register(f.found(empty, None, "u1", RootCommit::Later)).await.unwrap();
        assert_eq!(f.ground.commit_reads(), 5, "one to read later is read again");
        let folder = f.folders("/work/empty").await.remove(0);
        assert_eq!((folder.3, folder.4), (None, None), "nothing recorded");
    }

    #[tokio::test]
    async fn a_repository_without_origin_in_the_path_of_another_gets_a_project_of_its_own() {
        let f = Fixture::new().await;
        let path = Path::new("/work/app");
        let a = f.register(f.found(path, None, "s1", known("c1"))).await.unwrap();
        let theirs = f.memory(&a.project_id, "the parser of a", None, 1).await;

        let b = f.register(f.found(path, Some("github.com/acme/b"), "s2", known("c2"))).await.unwrap();

        let fresh = candidate_id(path, 1);
        assert_eq!((b.root_id.as_str(), b.project_id.as_str()), (fresh.as_str(), fresh.as_str()));
        assert_eq!((b.rekeyed, b.snapshot), (0, None));
        assert_eq!(project_ids(&f.store).await, vec![("memory".into(), theirs, Some(a.project_id.clone()))]);
        let bound: String = f
            .store
            .conn
            .call(|c| c.query_row("SELECT project_id FROM project_remote WHERE remote = 'github.com/acme/b'", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(bound, b.project_id);
        let mut expected = vec![
            (a.root_id.clone(), Some(a.project_id), None, some("c1"), some("s1")),
            (fresh.clone(), Some(fresh), some("github.com/acme/b"), some("c2"), some("s2")),
        ];
        expected.sort();
        let mut folders = f.folders("/work/app").await;
        folders.sort();
        assert_eq!(folders, expected);
    }

    #[tokio::test]
    async fn two_repositories_without_origin_in_one_path_are_two_projects() {
        let f = Fixture::new().await;
        let path = Path::new("/work/app");
        let a = f.register(f.found(path, None, "s1", known("c1"))).await.unwrap();

        let b = f.register(f.found(path, None, "s2", known("c2"))).await.unwrap();

        assert_ne!(b.project_id, a.project_id);
        assert_eq!(b.root_id, candidate_id(path, 1));
        assert_eq!(f.count("SELECT COUNT(*) FROM project_remote").await, 0);
        let a_again = f.register(f.found(path, None, "s3", known("c1"))).await.unwrap();
        let b_again = f.register(f.found(path, None, "s2", known("c2"))).await.unwrap();
        assert_eq!((a_again.project_id, b_again.project_id), (a.project_id, b.project_id), "each returns to its own");
        assert_eq!(f.count("SELECT COUNT(*) FROM project_path").await, 2);
    }

    #[tokio::test]
    async fn a_remote_decides_before_the_first_commit() {
        let f = Fixture::new().await;
        let path = Path::new("/work/app");
        let first = f.register(f.found(path, Some("github.com/acme/app"), "s1", known("c1"))).await.unwrap();

        let rewritten = f.register(f.found(path, Some("github.com/acme/app"), "s2", known("c9"))).await.unwrap();

        assert_eq!((rewritten.project_id, rewritten.root_id.clone()), (first.project_id.clone(), first.root_id));
        assert_eq!(
            f.folders("/work/app").await,
            vec![(rewritten.root_id, Some(first.project_id), some("github.com/acme/app"), some("c9"), some("s2"))]
        );
    }

    #[tokio::test]
    async fn a_clone_folder_records_its_own_remote() {
        let f = Fixture::new().await;
        let first = f.register(cloned(Path::new("/work/app"), "github.com/acme/app")).await.unwrap();
        let review = Path::new("/review/app");

        let clone = f.register(cloned(review, "github.com/acme/app")).await.unwrap();

        assert_eq!(clone.project_id, first.project_id);
        let mine = (clone.root_id.clone(), Some(first.project_id.clone()), some("github.com/acme/app"), None, None);
        assert_eq!(f.folders("/review/app").await, vec![mine]);
        let other = f.register(cloned(review, "github.com/acme/other")).await.unwrap();
        assert_ne!(other.root_id, clone.root_id, "its own remote contradicts another repository in its path");
        assert_ne!(other.project_id, first.project_id);
    }

    #[tokio::test]
    async fn a_caller_that_derives_its_id_is_refused_a_repository_without_origin_in_its_path() {
        let f = Fixture::new().await;
        let path = Path::new("/work/app");
        let a = f.register(f.found(path, None, "s1", known("c1"))).await.unwrap();
        f.memory(&a.project_id, "the parser of a", None, 1).await;
        let before = project_ids(&f.store).await;

        let err = f.register(Registration { assigned: false, ..f.found(path, None, "s2", known("c2")) }).await.unwrap_err();

        assert!(matches!(err, RegisterError::RootCollision { ref id, .. } if *id == a.root_id), "{err:?}");
        assert_eq!(project_ids(&f.store).await, before);
        let recorded = (a.root_id.clone(), Some(a.project_id.clone()), None, some("c1"), some("s1"));
        assert_eq!(f.folders("/work/app").await, vec![recorded], "nothing written");
        let same = f.register(Registration { assigned: false, ..f.found(path, None, "s1", known("c1")) }).await.unwrap();
        assert_eq!(same.project_id, a.project_id, "its own repository is still served");
    }

    #[tokio::test]
    async fn a_checkout_moved_without_origin_keeps_its_project() {
        let f = Fixture::new().await;
        let (old, new) = (Path::new("/work/app"), Path::new("/work/app2"));
        let a = f.register(f.found(old, None, "s1", known("c1"))).await.unwrap();
        let mine = f.memory(&a.project_id, "the parser of app", None, 1).await;
        let stray = f.memory(&derive_project_id(new), "saved in app2 before it was registered", None, 2).await;
        f.ground.remove(old);

        let moved = f.register(f.found(new, None, "s1", known("c1"))).await.unwrap();

        assert_eq!(moved.project_id, a.project_id);
        assert_eq!(moved.root_id, derive_project_id(new), "the new path has a folder of its own");
        assert_eq!(moved.rekeyed, 1, "the id of that folder joins the project");
        assert!(moved.snapshot.is_some());
        let p = Some(a.project_id.clone());
        assert_eq!(project_ids(&f.store).await, vec![("memory".into(), mine, p.clone()), ("memory".into(), stray, p.clone())]);
        assert_eq!(f.folders("/work/app2").await, vec![(moved.root_id.clone(), p.clone(), None, some("c1"), some("s1"))]);
        let left = (a.root_id.clone(), p, None, some("c1"), some("s1"));
        assert_eq!(f.folders("/work/app").await, vec![left], "the old folder stays as it was");
        assert!(f.suggestions().await.is_empty());

        f.ground.remove(new);
        let back = f.register(f.found(old, None, "s1", known("c1"))).await.unwrap();
        assert_eq!((back.project_id, back.root_id), (a.project_id.clone(), a.root_id), "back in its old path, it uses its old folder");

        let lib = Path::new("/work/lib");
        let l = f.register(f.found(lib, None, "t1", known("c2"))).await.unwrap();
        f.ground.put(lib, None, Some("t2"), known("c3"));
        let relocated = f.register(f.found(Path::new("/work/lib2"), None, "t1", known("c2"))).await.unwrap();
        assert_eq!(relocated.project_id, l.project_id, "another repository where it was recorded is not it");
    }

    /// A folder no registration since V27 recorded a project in leads to its
    /// own id, and that id was merged since: the root gets the live project,
    /// read, planned and registered alike.
    #[tokio::test]
    async fn a_root_whose_project_was_merged_registers_into_its_target() {
        let f = Fixture::new().await;
        let app = Path::new("/work/app");
        let id = derive_project_id(app);
        f.root(&id, "/work/app").await;
        let tombstone = id.clone();
        f.store
            .conn
            .call(move |c| {
                c.execute_batch(&format!(
                    "INSERT INTO project (id, name, born, created_at) VALUES ('web-bbbb', 'web', 'root', 0);
                     INSERT INTO project (id, name, born, created_at, merged_into, merged_at) \
                       VALUES ('{tombstone}', 'app', 'root', 0, 'web-bbbb', 1);"
                ))
            })
            .await
            .unwrap();
        let ground = f.ground.clone();
        let (read, planned) = f
            .store
            .conn
            .call(move |c| {
                let read = project_id_in(c, Path::new("/work/app"), None, &*ground)?;
                let planned = plan_ids_in(c, &[PathBuf::from("/work/app")], &*ground)?.remove(0).project_id;
                Ok::<_, rusqlite::Error>((read, planned))
            })
            .await
            .unwrap();
        assert_eq!((read.as_str(), planned.as_str()), ("web-bbbb", "web-bbbb"));

        let registered = f.register(reg(app, &[])).await.unwrap();

        assert_eq!((registered.project_id.as_str(), registered.root_id.as_str()), ("web-bbbb", id.as_str()));
        assert_eq!(f.folders("/work/app").await[0].1.as_deref(), Some("web-bbbb"), "the folder records the live project");
    }

    #[tokio::test]
    async fn a_copy_of_a_checkout_is_a_project_of_its_own_with_a_suggestion() {
        let f = Fixture::new().await;
        let (app, copy) = (Path::new("/work/app"), Path::new("/work/copia"));
        let a = f.register(f.found(app, None, "s1", known("c1"))).await.unwrap();
        let row = f.memory(&a.project_id, "the parser of app", None, 1).await;

        let c = f.register(f.found(copy, None, "s2", known("c1"))).await.unwrap();

        let own = derive_project_id(copy);
        assert_eq!((c.project_id.as_str(), c.root_id.as_str(), c.rekeyed), (own.as_str(), own.as_str(), 0));
        let pending = (own, a.project_id.clone(), "root_commit".to_string(), None);
        assert_eq!(f.suggestions().await, vec![pending.clone()]);
        assert_eq!(project_ids(&f.store).await, vec![("memory".into(), row, Some(a.project_id.clone()))], "nothing moves");

        f.register(f.found(copy, None, "s2", known("c1"))).await.unwrap();
        assert_eq!(f.suggestions().await, vec![pending], "registering it again adds none");
    }

    #[tokio::test]
    async fn a_pair_kept_apart_is_not_suggested_again() {
        let f = Fixture::new().await;
        let (app, copy) = (Path::new("/work/app"), Path::new("/work/copia"));
        let a = f.register(f.found(app, None, "s1", known("c1"))).await.unwrap();
        let c = f.register(f.found(copy, None, "s2", known("c1"))).await.unwrap();

        crate::memory::registry::keep_apart(&f.store, &c.project_id, &a.project_id).await.unwrap();
        f.register(f.found(copy, None, "s2", known("c1"))).await.unwrap();

        let answered = (c.project_id.clone(), a.project_id.clone(), "root_commit".to_string(), Some("apart".to_string()));
        assert_eq!(f.suggestions().await, vec![answered], "registering it again leaves the answer");
    }

    #[tokio::test]
    async fn a_checkout_two_projects_could_be_is_a_project_of_its_own() {
        let f = Fixture::new().await;
        let (one, two, three) = (Path::new("/work/one"), Path::new("/work/two"), Path::new("/work/three"));
        let p1 = f.register(f.found(one, None, "s1", known("c1"))).await.unwrap();
        let p2 = f.register(f.found(two, None, "s2", known("c1"))).await.unwrap();
        f.ground.remove(one);
        f.ground.remove(two);

        let p3 = f.register(f.found(three, None, "s1", known("c1"))).await.unwrap();

        assert_eq!(p3.project_id, derive_project_id(three));
        let mut suggested: Vec<_> =
            f.suggestions().await.into_iter().filter(|s| s.0 == p3.project_id).map(|s| (s.1, s.2)).collect();
        suggested.sort();
        let mut expected = vec![(p1.project_id, "root_commit".to_string()), (p2.project_id, "root_commit".to_string())];
        expected.sort();
        assert_eq!(suggested, expected);
    }

    /// Puts at `root` a folder without a git directory, and the registration
    /// the daemon would make of it.
    fn plain<'a>(f: &Fixture, root: &'a Path) -> Registration<'a> {
        f.ground.put(root, None, None, RootCommit::Later);
        reg(root, &[])
    }

    #[tokio::test]
    async fn a_folder_named_as_a_project_whose_folders_are_gone_is_suggested() {
        let f = Fixture::new().await;
        let (old, moved) = (Path::new("/work/notas"), Path::new("/archivo/Notas"));
        let a = f.register(plain(&f, old)).await.unwrap();
        f.memory(&a.project_id, "the notes of the week", None, 1).await;
        f.ground.remove(old);

        let b = f.register(plain(&f, moved)).await.unwrap();

        assert_ne!(b.project_id, a.project_id, "a name is weak evidence: it stays apart");
        assert_eq!(f.suggestions().await, vec![(b.project_id.clone(), a.project_id.clone(), "folder_name".to_string(), None)]);
        f.register(plain(&f, moved)).await.unwrap();
        assert_eq!(f.suggestions().await.len(), 1, "once per pair");
    }

    #[tokio::test]
    async fn a_folder_named_as_a_project_still_there_is_not_suggested() {
        let f = Fixture::new().await;
        let (one, two) = (Path::new("/work/api"), Path::new("/other/api"));
        f.register(plain(&f, one)).await.unwrap();

        f.register(plain(&f, two)).await.unwrap();
        assert!(f.suggestions().await.is_empty(), "two folders `api`, both there, are two projects");

        // Nor one without a folder, nor a deleted one, nor the folder's own.
        f.store
            .conn
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('ideas-1a2b', 'Ideas', 'agent', 0, 1);
                     INSERT INTO project (id, name, born, reviewed, created_at, deleted_at) VALUES ('web-3c4d', 'web', 'root', 1, 1, 2);
                     INSERT INTO project_path (id, root_path, created_at, project_id) VALUES ('web-3c4d', '/gone/web', 1, 'web-3c4d');",
                )
            })
            .await
            .unwrap();
        f.register(plain(&f, Path::new("/work/ideas"))).await.unwrap();
        f.register(plain(&f, Path::new("/work/web"))).await.unwrap();
        assert!(f.suggestions().await.is_empty(), "{:?}", f.suggestions().await);
    }

    #[tokio::test]
    async fn a_renamed_remote_in_its_path_is_another_project_with_a_suggestion() {
        let f = Fixture::new().await;
        let path = Path::new("/work/app");
        let a = f.register(f.found(path, Some("github.com/acme/app"), "s1", known("c1"))).await.unwrap();

        let renamed = f.register(f.found(path, Some("github.com/acme-org/app"), "s1", known("c1"))).await.unwrap();

        assert_ne!(renamed.project_id, a.project_id, "the remote tells them apart");
        assert_eq!(f.ground.commit_reads(), 1, "the git directory is the one recorded");
        let made = (renamed.project_id.clone(), a.project_id.clone(), "remote_changed".to_string(), None);
        assert_eq!(f.suggestions().await, vec![made]);

        let other = Path::new("/work/other");
        f.register(f.found(other, Some("github.com/acme/x"), "t1", known("c5"))).await.unwrap();
        f.register(f.found(other, Some("github.com/acme/y"), "t2", known("c6"))).await.unwrap();
        assert_eq!(f.suggestions().await.len(), 1, "without the same first commit there is none");
    }

    #[tokio::test]
    async fn a_suggestion_is_made_once_per_pair() {
        let f = Fixture::new().await;
        let (app, twin, copy) = (Path::new("/work/app"), Path::new("/work/twin"), Path::new("/work/copia"));
        let a = f.register(f.found(app, None, "s1", known("c1"))).await.unwrap();
        let t = f.register(f.found(twin, None, "s2", known("c1"))).await.unwrap();
        let answered = (a.project_id.clone(), derive_project_id(copy), "remote_changed".to_string(), some("apart"));
        let row = answered.clone();
        f.store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO project_suggestion (project_id, other_id, reason, created_at, answer, answered_at) \
                     VALUES (?1, ?2, ?3, 0, ?4, 0)",
                    rusqlite::params![row.0, row.1, row.2, row.3],
                )
            })
            .await
            .unwrap();

        let c = f.register(f.found(copy, None, "s3", known("c1"))).await.unwrap();
        f.register(f.found(copy, None, "s3", known("c1"))).await.unwrap();

        assert_eq!(c.project_id, answered.1);
        let mut expected = vec![
            (t.project_id.clone(), a.project_id.clone(), "root_commit".to_string(), None),
            answered,
            (c.project_id.clone(), t.project_id.clone(), "root_commit".to_string(), None),
        ];
        expected.sort();
        assert_eq!(f.suggestions().await, expected, "a pair already made, in either order and answered or not, is not made again");
    }

    #[tokio::test]
    async fn a_moved_checkout_is_read_and_planned_alike_without_writing() {
        let f = Fixture::new().await;
        let (old, new, copy) = (Path::new("/work/app"), Path::new("/work/app2"), Path::new("/work/copia"));
        let a = f.register(f.found(old, None, "s1", known("c1"))).await.unwrap();
        f.ground.remove(old);
        f.ground.put(new, None, Some("s1"), known("c1"));
        f.ground.put(copy, None, Some("s2"), known("c1"));
        let ground = f.ground.clone();
        let written = f.count("SELECT total_changes()").await;

        let (read, planned) = f
            .store
            .conn
            .call(move |c| {
                let read = project_id_in(c, new, None, &*ground)?;
                let planned = plan_ids_in(c, &[new.to_path_buf(), copy.to_path_buf()], &*ground)?;
                Ok::<_, rusqlite::Error>((read, planned.into_iter().map(|p| (p.root_id, p.project_id)).collect::<Vec<_>>()))
            })
            .await
            .unwrap();

        assert_eq!(f.count("SELECT total_changes()").await, written, "reading writes nothing");
        let moved = f.register(f.found(new, None, "s1", known("c1"))).await.unwrap();
        let copied = f.register(f.found(copy, None, "s2", known("c1"))).await.unwrap();
        assert_eq!(read, a.project_id);
        assert_eq!(moved.project_id, a.project_id);
        assert_eq!(
            planned,
            vec![(moved.root_id, moved.project_id), (copied.root_id, copied.project_id)],
            "a copy sees the moved checkout an earlier root of the batch would record"
        );
    }

    #[tokio::test]
    async fn a_caller_that_derives_its_id_never_follows_a_move() {
        let f = Fixture::new().await;
        let (old, new) = (Path::new("/work/app"), Path::new("/work/app2"));
        let a = f.register(f.found(old, None, "s1", known("c1"))).await.unwrap();
        f.ground.remove(old);

        let derived = f.register(Registration { assigned: false, ..f.found(new, None, "s1", known("c1")) }).await.unwrap();

        assert_eq!(derived.project_id, derive_project_id(new));
        assert_ne!(derived.project_id, a.project_id);
        assert!(f.suggestions().await.is_empty());
        let own = (derived.root_id.clone(), Some(derived.project_id), None, some("c1"), some("s1"));
        assert_eq!(f.folders("/work/app2").await, vec![own], "it records what it found in its own folder");
    }

    impl Fixture {
        /// A project with no folder, as `projects create` leaves it.
        async fn project(&self, id: &str) {
            let id = id.to_string();
            self.store
                .conn
                .call(move |c| {
                    c.execute("INSERT INTO project (id, name, born, reviewed, created_at) VALUES (?1, ?1, 'user', 1, 0)", [id])
                })
                .await
                .unwrap();
        }

        /// Declares `subpath` of the repository of `parent` as `project`.
        async fn declare(&self, parent: &str, subpath: &str, project: &str, created_at: i64) {
            let (parent, subpath, project) = (parent.to_string(), subpath.to_string(), project.to_string());
            self.store
                .conn
                .call(move |c| {
                    c.execute(
                        "INSERT INTO project_subfolder (parent_id, subpath, project_id, created_at) VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![parent, subpath, project, created_at],
                    )
                })
                .await
                .unwrap();
        }

        /// The project a registration of `root` from `subpath` answers.
        async fn reached(&self, reg: Registration<'_>, subpath: &str) -> String {
            let outcome = self.register(Registration { subpath: Some(subpath), ..reg }).await.unwrap();
            outcome.subproject.unwrap_or(outcome.project_id)
        }
    }

    #[tokio::test]
    async fn a_declared_subfolder_takes_what_is_saved_inside_it() {
        let f = Fixture::new().await;
        let (mono, other) = (Path::new("/work/monorepo"), Path::new("/other/monorepo"));
        let m = f.register(cloned(mono, "github.com/acme/monorepo")).await.unwrap().project_id;
        f.project("web").await;
        f.declare(&m, "apps/web", "web", 1).await;

        assert_eq!(f.reached(cloned(mono, "github.com/acme/monorepo"), "apps/web/src").await, "web");
        assert_eq!(f.reached(cloned(mono, "github.com/acme/monorepo"), "apps/web").await, "web");
        assert_eq!(f.reached(cloned(mono, "github.com/acme/monorepo"), "apps/webapp").await, m);
        assert_eq!(f.reached(cloned(mono, "github.com/acme/monorepo"), "").await, m);
        let root = f.register(cloned(mono, "github.com/acme/monorepo")).await.unwrap();
        assert_eq!((root.project_id.as_str(), root.subproject), (m.as_str(), None), "without a path, the repository");
        assert_eq!(f.reached(cloned(other, "github.com/acme/monorepo"), "apps/web/src").await, "web", "a clone reaches it");
        assert_eq!(f.reached(cloned(other, "github.com/acme/monorepo"), "").await, m);
        let derived = f.register(Registration { assigned: false, subpath: Some("apps/web"), ..cloned(mono, "github.com/acme/monorepo") });
        assert_eq!(derived.await.unwrap().subproject, None, "a caller that derives its id keeps the rule of before");
    }

    #[tokio::test]
    async fn a_declaration_follows_a_moved_checkout() {
        let f = Fixture::new().await;
        let (old, new) = (Path::new("/work/mono"), Path::new("/work/mono2"));
        let m = f.register(f.found(old, None, "s1", known("c1"))).await.unwrap().project_id;
        f.project("web").await;
        f.declare(&m, "apps/web", "web", 1).await;
        f.ground.remove(old);

        let moved = f.register(Registration { subpath: Some("apps/web"), ..f.found(new, None, "s1", known("c1")) }).await.unwrap();

        assert_eq!(moved.project_id, m, "the root keeps its project");
        assert_eq!(moved.subproject.as_deref(), Some("web"));
    }

    #[tokio::test]
    async fn a_declaration_is_read_through_merges() {
        let f = Fixture::new().await;
        let mono = Path::new("/work/monorepo");
        let m = f.register(cloned(mono, "github.com/acme/monorepo")).await.unwrap().project_id;
        for id in ["web", "zeta"] {
            f.project(id).await;
        }
        f.declare(&m, "apps/web", "web", 1).await;
        let reached = |subpath: &'static str| f.reached(cloned(mono, "github.com/acme/monorepo"), subpath);

        crate::memory::merge::merge_core(&f.store, &f.ws, &f.backups, &m, "zeta", false).await.unwrap();
        assert_eq!(reached("").await, "zeta", "the root leads to the project its own was merged into");
        assert_eq!(reached("apps/web").await, "web", "and its declaration with it");

        let merged = crate::memory::merge::merge_core(&f.store, &f.ws, &f.backups, "web", "zeta", false).await.unwrap();
        assert_eq!(reached("apps/web").await, "zeta", "a declaration that leads to the repository answers it");
        crate::memory::merge::undo_merge_core(&f.store, &f.ws, &f.backups, merged.merge_id.unwrap(), false).await.unwrap();
        assert_eq!(reached("apps/web").await, "web", "undoing the merge gives it back");
    }

    #[tokio::test]
    async fn resolving_a_subfolder_joins_nothing() {
        let f = Fixture::new().await;
        let (first, mono) = (Path::new("/work/first"), Path::new("/work/monorepo"));
        let p = f.register(cloned(first, "github.com/acme/monorepo")).await.unwrap().project_id;
        f.project("web").await;
        f.declare(&p, "apps/web", "web", 1).await;
        let legacy = vec![derive_project_id(&mono.join("apps").join("web"))];
        f.memory(&legacy[0], "saved before roots were resolved", None, 1).await;
        f.memory(&derive_project_id(mono), "saved under the folder's own id", None, 2).await;

        let outcome = f
            .register(Registration { subpath: Some("apps/web"), ..Registration { checkout: keyed("github.com/acme/monorepo"), ..reg(mono, &legacy) } })
            .await
            .unwrap();

        assert_eq!(outcome.project_id, p, "the legacy and the folder's ids joined the repository's project");
        assert_eq!(outcome.subproject.as_deref(), Some("web"));
        assert_eq!(outcome.root_id, derive_project_id(mono), "the folder keeps its own id for its docs");
        let projects: Vec<Option<String>> = project_ids(&f.store).await.into_iter().map(|(_, _, p)| p).collect();
        assert_eq!(projects, vec![Some(p.clone()), Some(p)], "nothing went to the subproject");
    }
}
