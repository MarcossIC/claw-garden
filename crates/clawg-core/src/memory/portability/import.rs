//! Writes an export into a store, row by row, with what each row was.
//!
//! A path of its own beside the ingest worker, because a save cannot carry a
//! row's state: it counts one access, derives the tier, stores only live rows
//! and runs the contradiction detector. Each guarantee the worker gives a
//! stored row is kept here too: the text masked, its hash, its text index
//! entry, one live row per text and one live save per topic key. An export
//! of another tool takes the same passes once [`super::translate`] has made
//! its notes rows, each masked whole and split as a save of its text; a note
//! whose origin is here already is found where its rows are, as the same
//! version or as a rewrite of theirs that goes where they are.
//!
//! Three passes. The first places every row and link on what it loaded of
//! the store in short calls, off the connection and writing nothing, so rows
//! of one file see each other, a dry run is exactly that pass and every other
//! call goes on meanwhile. The second embeds the rows to insert, outside any
//! transaction.
//! The third writes them in bounded transactions, placing each row again
//! because the worker writes meanwhile, then writes the links. A row that is
//! already there is never inserted again and its links are only filled where
//! empty, so importing the same file twice, or again after a crash between
//! transactions, does no harm.
//!
//! Each step logs how long it took as it ends ([`super::timing`]), and the
//! steps on the connection how much of it was SQLite and how much waiting.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::convert::Infallible;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::memory::backup::Reason;
use crate::memory::chunker::{embedding_text, ChatChunker};
use crate::memory::clock::TimestampMs;
use crate::memory::dedup::content_hash;
use crate::memory::embedder::{EmbedError, EmbedRole, Embedder};
use crate::memory::identity::{Ground, ReadAhead};
use crate::memory::merge::canonical_in;
use crate::memory::project::{plan_ids_in, RootPlan};
use crate::memory::portability::projects::{self, file_projects, registry_in, By, FileProject, FolderHere, Placed, Prints, Registry};
use crate::memory::project_delete::{deleted_in, deleted_message};
use crate::memory::registry::{find_project, new_project_id, project_name, Named};
use crate::memory::subfolder::declared_in;
use crate::memory::language;
use crate::memory::portability::foreign::ForeignExport;
use crate::memory::portability::format::{parse, ConflictLine, FormatError, Invalid, MemoryLine, Parsed, PieceLine};
use crate::memory::portability::timing::{ImportTimings, Step, Timing};
use crate::memory::portability::translate::{
    translate, ForeignReport, LeftOut, MappedProject, MergedNote, NoteRows, TakenKey, TakenProject, TranslateOptions,
    Translated, MIGRATED_AGENT,
};
use crate::memory::redact::redact;
use crate::memory::vector::to_unit;
use crate::memory::{MemoryError, MemoryStore};

/// The largest file an import reads, in bytes. Every row of it is held in
/// memory while it is placed, embedded and written.
pub const MAX_IMPORT_BYTES: usize = 256 * 1024 * 1024;

/// Rows written per transaction, whole notes of another tool each: a note
/// longer than this is written alone.
const WRITE_BATCH: usize = 256;
/// Texts embedded per call.
const EMBED_BATCH: usize = 32;

/// Texts counted in one call to the tokenizer when ordering the rows to
/// embed, so the encodings held at once stay small.
const COUNT_BATCH: usize = 256;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportOptions {
    /// Place every row and report, writing nothing.
    pub dry_run: bool,
    /// The agent every row that has one is stored under instead.
    pub agent: Option<String>,
    /// An exported project root and where it is here.
    pub map_roots: Vec<(String, MapTarget)>,
    /// A project name of another tool and where its notes go here.
    pub project_map: Vec<(String, MapTarget)>,
}

/// Where a map sends a root of a clawg export or a project of another tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MapTarget {
    /// A root here, as a client from before sends it.
    Root(String),
    /// A root here and the path the user named inside it, which a declared
    /// subfolder leads to its project.
    Folder {
        root: String,
        #[serde(default)]
        path: Option<String>,
    },
    /// A project here: an id once the daemon resolved it.
    Project { project: String },
}

impl MapTarget {
    pub fn root(&self) -> Option<&str> {
        match self {
            MapTarget::Root(root) | MapTarget::Folder { root, .. } => Some(root),
            MapTarget::Project { .. } => None,
        }
    }

    fn path(&self) -> Option<&str> {
        match self {
            MapTarget::Folder { path, .. } => path.as_deref(),
            _ => None,
        }
    }
}

impl From<&str> for MapTarget {
    fn from(root: &str) -> Self {
        MapTarget::Root(root.to_owned())
    }
}

impl From<String> for MapTarget {
    fn from(root: String) -> Self {
        MapTarget::Root(root)
    }
}

/// The project a map names, as [`map_project`] finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapProject {
    /// Its live id, past any merge.
    Live(String),
    Unknown,
    /// The name of several live projects, by id.
    Ambiguous(Vec<String>),
}

/// The project `reference` names, by id or by the name of one live project,
/// for a map to send notes to: never created, and refused when deleted.
pub async fn map_project(store: &MemoryStore, reference: &str) -> Result<MapProject, ImportError> {
    let id = match find_project(store, reference).await? {
        Named::Project(id) => id,
        Named::Unknown => return Ok(MapProject::Unknown),
        Named::Ambiguous(ids) => return Ok(MapProject::Ambiguous(ids)),
    };
    let live = store
        .conn
        .call(move |c| -> rusqlite::Result<Result<String, Option<i64>>> {
            let live = canonical_in(c, &id)?;
            Ok(match deleted_in(c, &live)? {
                Some(deleted) => Err(deleted.delete_id),
                None => Ok(live),
            })
        })
        .await
        .map_err(MemoryError::Connection)?;
    match live {
        Ok(id) => Ok(MapProject::Live(id)),
        Err(delete_id) => Err(ImportError::ProjectDeleted { id: reference.trim().to_owned(), delete_id }),
    }
}

/// A live row stored as the live row of the same text that was already here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Merged {
    pub line: usize,
    pub id: i64,
    pub into: i64,
}

/// A live row stored superseded, because another live row of its project
/// already holds its topic key here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicTaken {
    pub line: usize,
    pub id: i64,
    pub topic_key: String,
    pub stored_as: i64,
    pub by: i64,
}

/// What an import did, or would do on a dry run. Row ids are the file's
/// unless named otherwise.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    pub dry_run: bool,
    /// `memory` lines in the file that could be read; for another tool's
    /// export, the rows its notes split into.
    pub rows: usize,
    pub inserted: usize,
    pub by_type: BTreeMap<String, usize>,
    /// `live`, `superseded` or `deleted`.
    pub by_state: BTreeMap<String, usize>,
    /// By the project id the rows got here; `""` for rows without one.
    pub by_project: BTreeMap<String, usize>,
    /// Rows already here, which were neither inserted nor reinforced.
    pub present: usize,
    /// Rows of a clawg export already here in another project, as a move
    /// leaves them, by that project (`""` for none).
    #[serde(default)]
    pub present_elsewhere: BTreeMap<String, usize>,
    pub merged: Vec<Merged>,
    pub topic_taken: Vec<TopicTaken>,
    /// Inserted rows whose text held a secret, stored masked.
    pub redacted: usize,
    pub invalid: Vec<Invalid>,
    /// Links whose other end is neither in the file nor here.
    pub dropped_links: Vec<Invalid>,
    /// The `pre-import` snapshot, when one was written.
    pub snapshot: Option<String>,
    /// The import's number, which `clawg import --undo` takes; `None` on a
    /// dry run and for an import that wrote nothing.
    #[serde(default)]
    pub import_id: Option<i64>,
    /// Inserted rows by the language of their text: `en`, `es` or `und`.
    #[serde(default)]
    pub by_lang: BTreeMap<String, usize>,
    /// For another tool's export, what became of each note. Its lines and
    /// ids are the translation's, so `merged`, `topic_taken` and `invalid`
    /// are told here by note instead.
    #[serde(default)]
    pub foreign: Option<ForeignReport>,
    /// Each project of the file and where it went here.
    #[serde(default)]
    pub projects: Vec<ImportedProject>,
    /// The suggestions it left, or would leave on a dry run.
    #[serde(default)]
    pub suggestions: Vec<ImportedSuggestion>,
    /// The subfolders the file declares, written here or not.
    #[serde(default)]
    pub subfolders: Vec<ImportedSubfolder>,
}

/// A subfolder a repository of the file declares as a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedSubfolder {
    /// The repository and the project it leads to, by their ids in the file.
    pub repository_id: String,
    pub subpath: String,
    pub project_id: String,
    /// The projects here of both, when they went somewhere.
    #[serde(default)]
    pub repository_here: Option<String>,
    #[serde(default)]
    pub project_here: Option<String>,
    /// Whether it is written here, or would be on a dry run.
    pub written: bool,
    /// Why not: `declared_here` (a declaration here keeps the subpath),
    /// `no_repository` (its repository went nowhere) or `same_project`.
    #[serde(default)]
    pub reason: Option<String>,
}

/// A suggestion that two projects here may be one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedSuggestion {
    pub project_id: String,
    pub other_id: String,
    /// `root_commit`, `remote_changed`, `folder_name`, `similar_name` or `merged_at_source`.
    pub reason: String,
}

/// A project of the file and the project it went to here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedProject {
    /// Its id in the source store.
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// The project here, `None` for a line without rows nothing here took.
    #[serde(default)]
    pub to: Option<String>,
    /// The step that decided it.
    #[serde(default)]
    pub by: Option<By>,
    /// Whether the import created `to`.
    #[serde(default)]
    pub created: bool,
    /// Whether a row it inserted live brought `to` back from a delete.
    #[serde(default)]
    pub revived: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error(transparent)]
    Format(#[from] FormatError),
    #[error("the embedder is not ready; nothing was imported")]
    NoEmbedder,
    #[error("could not embed the imported rows, nothing was imported: {0}")]
    Embed(#[from] EmbedError),
    /// A project a map sends notes to was deleted.
    #[error("{}", deleted_message(.id, .delete_id))]
    ProjectDeleted { id: String, delete_id: Option<i64> },
    #[error(transparent)]
    Store(#[from] MemoryError),
}

/// One `memory` line as it will be stored here.
#[derive(Debug, Clone)]
struct Row {
    line: usize,
    src: MemoryLine,
    agent: Option<String>,
    project: Option<String>,
    content: String,
    hash: String,
    redacted: bool,
    /// The head a piece's vector is embedded behind, as the worker does.
    context: String,
    /// Read from the text stored here, never from the file's `lang`: a file
    /// cannot say what its text does not.
    lang: &'static str,
    /// The note of another tool it came from, in [`Prepared::notes`].
    note: Option<usize>,
}

impl Row {
    fn live(&self) -> bool {
        self.src.deleted_at.is_none() && self.src.status != "superseded"
    }

    fn state(&self) -> &'static str {
        match (self.src.deleted_at, self.src.status.as_str()) {
            (Some(_), _) => "deleted",
            (None, "superseded") => "superseded",
            _ => "live",
        }
    }
}

/// The file, worked out before any transaction.
#[derive(Debug)]
struct Prepared {
    rows: Vec<Row>,
    /// Every folder to record: its root here, the root's id and remote, and
    /// the project its rows go to.
    projects: Vec<ProjectHere>,
    /// The projects the import creates, with their names, in order.
    creates: Vec<(String, String)>,
    /// Each remote to bind and the project it goes to, unless one binds it.
    binds: Vec<(String, String)>,
    /// Where each project of the file went.
    placed: Vec<ImportedProject>,
    /// The suggestions to leave, none of which has a pair here yet.
    suggestions: Vec<ImportedSuggestion>,
    /// The deleted projects here, as the plan read them.
    deleted: HashSet<String>,
    /// The declarations of the file, with what becomes of each.
    subfolders: Vec<ImportedSubfolder>,
    /// The projects a map sends notes to by name or id, which must not be
    /// deleted when the import writes.
    mapped_to: Vec<String>,
    parsed: Parsed,
    /// For another tool's export, the notes its rows came from, in the order
    /// of their rows; empty for a clawg file.
    notes: Vec<NoteRows>,
}

impl Prepared {
    /// The rows of each unit a write batch keeps whole, in order: a note of
    /// another tool, or one row of a clawg file.
    fn units(&self) -> Vec<usize> {
        let mut units: Vec<usize> = Vec::new();
        let mut last = None;
        for row in &self.rows {
            match (row.note, units.last_mut()) {
                (Some(n), Some(len)) if last == Some(n) => *len += 1,
                (note, _) => {
                    units.push(1);
                    last = note;
                }
            }
        }
        units
    }

    /// What the import found here of the note `row` came from.
    fn found<'l>(&self, row: &Row, ledger: &'l Ledger) -> Option<&'l Found> {
        ledger.notes.get(&self.notes[row.note?].rows.start)
    }
}

/// A project of the file, as it is recorded here.
#[derive(Debug, Clone)]
struct ProjectHere {
    root: String,
    root_id: String,
    remote: Option<String>,
    project_id: String,
    /// The first commit and the git stamp a registration records in its folder.
    root_commit: Option<String>,
    git_stamp: Option<String>,
}

/// The projects of a clawg file.
fn projects_of(parsed: &Parsed) -> Vec<FileProject> {
    let with_rows: BTreeSet<&str> = parsed.memories.iter().filter_map(|(_, m)| m.project_id.as_deref()).collect();
    file_projects(parsed.projects.iter().map(|(_, p)| p), &with_rows)
}

/// The root here of every folder of `projects`, in order, whether
/// `--map-root` named it, and the path it named inside that root.
fn roots_here(projects: &[FileProject], options: &ImportOptions) -> Vec<(PathBuf, bool, Option<PathBuf>)> {
    projects
        .iter()
        .flat_map(|p| &p.folders)
        .map(|f| {
            let mapped = options.map_roots.iter().find(|(old, _)| *old == f.root_path).map(|(_, new)| new);
            match mapped.and_then(|new| new.root().map(|root| (root, new.path()))) {
                Some((root, path)) => (PathBuf::from(root), true, path.map(PathBuf::from)),
                None => (PathBuf::from(&f.root_path), false, None),
            }
        })
        .collect()
}

/// Runs `f` on the store's connection and counts the call in `timing`: how
/// long it held the connection, and how long it waited for it behind others.
async fn timed<T, F>(store: &MemoryStore, timing: &mut Timing, f: F) -> Result<T, MemoryError>
where
    T: Send + 'static,
    F: FnOnce(&mut rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
{
    let asked = Instant::now();
    let (value, ran) = store
        .conn
        .call(move |c| {
            let started = Instant::now();
            let value = f(c)?;
            Ok::<_, rusqlite::Error>((value, started.elapsed()))
        })
        .await
        .map_err(MemoryError::Connection)?;
    timing.call(asked.elapsed(), ran);
    Ok(value)
}

/// What [`plan`] read: the plan of each root, the registry and whether each
/// root exists here, in one pass on the connection.
#[derive(Debug, Default)]
struct Planning {
    plans: Vec<RootPlan>,
    registry: Registry,
    present: Vec<bool>,
    /// For each path asked about, the project a declared subfolder leads it
    /// to inside its root, as a registration with that path answers.
    declared: Vec<Option<String>>,
}

/// How many times [`plan`] runs on the connection at most: it asks for the
/// checkouts first, then for the first commits no folder recorded, then
/// whether the checkouts a root may have moved from are still there. A store
/// that changed in between may ask again, and the last pass decides without.
const PLAN_PASSES: usize = 4;

/// What a registration of each root would give it now, in order. What the
/// plan needs of `ground` is read on a blocking thread between passes, since
/// a `git` on the connection's thread would hold every other call.
async fn plan(
    store: &MemoryStore,
    roots: &[PathBuf],
    paths: &[(usize, PathBuf)],
    ground: Arc<dyn Ground>,
    timings: &mut ImportTimings,
) -> Result<Planning, ImportError> {
    let (clock, mut timing) = (Instant::now(), Timing::new(Step::Plan));
    let (mut ahead, mut passes) = (ReadAhead::default(), 1);
    let plans = loop {
        let (planned, asks) = (roots.to_vec(), paths.to_vec());
        let (plans, asked) = timed(store, &mut timing, move |c| {
            let plans = plan_ids_in(c, &planned, &ahead, true)?;
            // Whether each root exists here, which tells a folder of this
            // machine from one of the source's.
            let present = planned.iter().map(|root| ahead.recorded_checkout_present(root, None)).collect();
            let mut declared = Vec::with_capacity(asks.len());
            for (k, path) in &asks {
                let subpath = ahead.subpath(&planned[*k], path);
                declared.push(subpath.map(|s| declared_in(c, &plans[*k].project_id, &s)).transpose()?.flatten());
            }
            Ok((Planning { plans, registry: registry_in(c)?, present, declared }, ahead))
        })
        .await?;
        if passes == PLAN_PASSES {
            break plans;
        }
        let ground = ground.clone();
        let read = tokio::task::spawn_blocking(move || {
            let mut ahead = asked;
            let any = ahead.read(&*ground);
            (ahead, any)
        });
        // Nothing asked, or a read that failed: the pass decided without it.
        let Ok((read, true)) = read.await else {
            break plans;
        };
        (ahead, passes) = (read, passes + 1);
    };
    timings.record(Timing { elapsed: clock.elapsed(), ..timing });
    Ok(plans)
}

/// What the texts of a file are.
enum Texts {
    /// As a clawg export carries them: masked here.
    Raw,
    /// The rows of notes of another tool, each masked whole before it was
    /// split, which masking a piece alone could undo.
    Notes(Vec<NoteRows>),
}

/// What the import does with the projects of a file: where each goes, the
/// folders to record, the projects to create and the remotes to bind.
#[derive(Debug, Default)]
struct Settled {
    /// The project here of each project id of the file.
    project_ids: HashMap<String, String>,
    projects: Vec<ProjectHere>,
    creates: Vec<(String, String)>,
    binds: Vec<(String, String)>,
    placed: Vec<ImportedProject>,
    suggestions: Vec<ImportedSuggestion>,
    deleted: HashSet<String>,
    subfolders: Vec<ImportedSubfolder>,
}

/// How many ids a new project of the file tries before it takes its first.
const NEW_ID_TRIES: u32 = 64;

/// The id a new project of the file takes: its own when nothing here and no
/// other new project has it, else the first free one its name gives.
fn free_id(id: &str, name: &str, registry: &Registry, taken: &HashSet<String>, now: i64) -> String {
    let free = |id: &str| !registry.taken(id) && !taken.contains(id);
    if free(id) {
        return id.to_owned();
    }
    (0..NEW_ID_TRIES).map(|n| new_project_id(name, now, n)).find(|id| free(id)).unwrap_or_else(|| id.to_owned())
}

/// Settles the projects of a clawg file (see [`place`]); `roots` are the
/// roots here of their folders, in order, `planning` what [`plan`] read of
/// them and `row_projects` the project ids the file's rows name.
fn settle_projects(
    projects: &[FileProject],
    roots: &[(PathBuf, bool, Option<PathBuf>)],
    planning: &Planning,
    row_projects: &BTreeSet<&str>,
    now: i64,
) -> Settled {
    let registry = &planning.registry;
    // The project a mapped path in a declared subfolder leads to, by root.
    let declared: HashMap<usize, &str> = (0..roots.len())
        .filter(|&k| roots[k].2.is_some())
        .zip(&planning.declared)
        .filter_map(|(k, d)| Some((k, d.as_deref()?)))
        .collect();
    let mut settled = Settled::default();
    settled.deleted = registry.projects.iter().filter(|(_, p)| p.deleted).map(|(id, _)| id.clone()).collect();
    let mut taken: HashSet<String> = HashSet::new();
    let mut at = 0;
    for project in projects {
        let span = at..at + project.folders.len();
        at = span.end;
        let folders: Vec<FolderHere<'_>> = project
            .folders
            .iter()
            .zip(span.clone())
            .map(|(line, k)| FolderHere { line, mapped: roots[k].1, plan: &planning.plans[k], present: planning.present[k] })
            .collect();
        let name = project.name.clone().unwrap_or_else(|| project_name(&project.id).to_owned());
        let (mut to, by) = match projects::place(project, &folders, registry) {
            Placed::Here(to, by) => (Some(to), Some(by)),
            Placed::New => (Some(free_id(&project.id, &name, registry, &taken, now)), Some(By::New)),
            Placed::Nowhere => (None, None),
        };
        // Mapped to a path in a declared subfolder, its rows go to the
        // subproject while its folders and remotes stay the repository's.
        let mut home = None;
        if by == Some(By::Map) {
            if let Some(sub) = span.clone().find(|k| roots[*k].1).and_then(|k| declared.get(&k)) {
                home = to.replace(registry.canonical(sub));
            }
        }
        let created = to.as_ref().is_some_and(|to| !registry.projects.contains_key(to) && taken.insert(to.clone()));
        settled.placed.push(ImportedProject { id: project.id.clone(), name: project.name.clone(), to: to.clone(), by, created, revived: false });
        let Some(to) = to else { continue };
        settled.project_ids.insert(project.id.clone(), to.clone());
        if created {
            settled.creates.push((to.clone(), name));
        }
        let home = home.unwrap_or_else(|| to.clone());
        let file = project.prints();
        for (folder, k) in folders.iter().zip(span) {
            let plan = folder.plan;
            let root = roots[k].0.to_string_lossy().into_owned();
            let here = match folder.present {
                // A checkout of another repository is not this project's folder.
                true if file.contradicts(&Prints {
                    remotes: plan.remote.iter().cloned().collect(),
                    commits: plan.root_commit.iter().cloned().collect(),
                }) =>
                {
                    continue
                }
                true => ProjectHere {
                    root,
                    root_id: plan.root_id.clone(),
                    remote: plan.remote.clone(),
                    project_id: home.clone(),
                    root_commit: plan.root_commit.clone(),
                    git_stamp: plan.git_stamp.clone(),
                },
                // What the source recorded there, which a checkout cloned
                // here later is found by.
                false => ProjectHere {
                    root,
                    root_id: plan.root_id.clone(),
                    remote: folder.line.remote.clone(),
                    project_id: home.clone(),
                    root_commit: folder.line.root_commit.clone(),
                    git_stamp: None,
                },
            };
            if let Some(remote) = &here.remote {
                settled.binds.push((remote.clone(), home.clone()));
            }
            settled.projects.push(here);
        }
        for remote in project.all_remotes() {
            settled.binds.push((remote.to_owned(), home.clone()));
        }
    }
    // The declarations of each repository, with the projects here of both
    // ends; a subproject no row took is made for the declaration to lead to.
    for project in projects {
        for declared in &project.subfolders {
            let repository = settled.project_ids.get(&project.id).cloned();
            let mut target = settled.project_ids.get(&declared.project_id).cloned();
            if let (Some(_), None) = (&repository, &target) {
                let line = projects.iter().find(|p| p.id == declared.project_id);
                let name = line.and_then(|p| p.name.clone()).unwrap_or_else(|| project_name(&declared.project_id).to_owned());
                let id = free_id(&declared.project_id, &name, registry, &taken, now);
                taken.insert(id.clone());
                settled.creates.push((id.clone(), name));
                settled.project_ids.insert(declared.project_id.clone(), id.clone());
                match settled.placed.iter_mut().find(|p| p.id == declared.project_id) {
                    Some(placed) => (placed.to, placed.by, placed.created) = (Some(id.clone()), Some(By::New), true),
                    None => settled.placed.push(ImportedProject {
                        id: declared.project_id.clone(),
                        name: None,
                        to: Some(id.clone()),
                        by: Some(By::New),
                        created: true,
                        revived: false,
                    }),
                }
                target = Some(id);
            }
            let here = repository.as_deref().and_then(|r| {
                registry.subfolders.iter().find(|(parent, subpath, _)| registry.canonical(parent) == r && *subpath == declared.subpath)
            });
            let reason = match (&repository, &target, here) {
                (None, _, _) => Some("no_repository"),
                (_, _, Some(_)) => Some("declared_here"),
                (Some(r), Some(t), None) if r == t => Some("same_project"),
                _ => None,
            };
            settled.subfolders.push(ImportedSubfolder {
                repository_id: project.id.clone(),
                subpath: declared.subpath.clone(),
                project_id: declared.project_id.clone(),
                repository_here: repository,
                project_here: target,
                written: reason.is_none(),
                reason: reason.map(str::to_owned),
            });
        }
    }
    let mut suggest = |a: &str, b: &str, reason: &str| {
        let key = projects::pair(a, b);
        if a != b && !registry.suggested.contains(&key) && !settled.suggestions.iter().any(|s| projects::pair(&s.project_id, &s.other_id) == key) {
            settled.suggestions.push(ImportedSuggestion { project_id: a.to_owned(), other_id: b.to_owned(), reason: reason.to_owned() });
        }
    };
    let mut at = 0;
    for (project, placed) in projects.iter().zip(&settled.placed.clone()) {
        let span = at..at + project.folders.len();
        at = span.end;
        let Some(to) = placed.to.as_deref() else { continue };
        let folders: Vec<FolderHere<'_>> = project
            .folders
            .iter()
            .zip(span.clone())
            .map(|(line, k)| FolderHere { line, mapped: roots[k].1, plan: &planning.plans[k], present: planning.present[k] })
            .collect();
        // What a registration of each folder that exists here would leave.
        for folder in folders.iter().filter(|f| f.present) {
            if settled.projects.iter().any(|p| p.root_id == folder.plan.root_id) {
                for (other, reason) in &folder.plan.suggestions {
                    suggest(to, other, reason);
                }
            }
        }
        if project.merged_into.is_none() {
            let prints = projects::seen_prints(project, &folders);
            for other in projects::similar(project, to, &prints, registry) {
                suggest(to, &other, "similar_name");
            }
            let apart = projects::bound_apart(project, registry);
            for other in apart.iter().skip(1) {
                suggest(&apart[0], other, "merged_at_source");
            }
        } else if let Some(into) = project.merged_into.as_deref().and_then(|m| settled.project_ids.get(m)) {
            let alive = |id: &str| registry.projects.get(id).is_none_or(|p| !p.deleted && p.merged_into.is_none());
            if alive(into) && alive(to) {
                suggest(into, to, "merged_at_source");
            }
        }
    }
    // A row whose project has no line keeps its id, and the import makes it
    // the project a save under it would.
    for id in row_projects {
        if !settled.project_ids.contains_key(*id) && !registry.projects.contains_key(*id) && taken.insert(id.to_string()) {
            settled.creates.push((id.to_string(), project_name(id).to_owned()));
        }
    }
    settled
}

/// The folders of another tool's export: each root the map names, recorded
/// as a registration records it. The rows carry the project the map sends
/// them to already.
fn mapped_projects(roots: &[PathBuf], plans: &[RootPlan]) -> Settled {
    let mut settled = Settled::default();
    for (root, plan) in roots.iter().zip(plans) {
        settled.projects.push(ProjectHere {
            root: root.to_string_lossy().into_owned(),
            root_id: plan.root_id.clone(),
            remote: plan.remote.clone(),
            project_id: plan.project_id.clone(),
            root_commit: plan.root_commit.clone(),
            git_stamp: plan.git_stamp.clone(),
        });
        if let Some(remote) = &plan.remote {
            settled.binds.push((remote.clone(), plan.project_id.clone()));
        }
        settled.creates.push((plan.project_id.clone(), project_name(&plan.project_id).to_owned()));
    }
    settled
}

/// The rows of `parsed` worked out, with the projects `settled` gave them.
fn prepare(mut parsed: Parsed, settled: Settled, options: &ImportOptions, texts: Texts) -> Prepared {
    let Settled { project_ids, projects, creates, binds, placed, suggestions, deleted, subfolders } = settled;
    if let Texts::Raw = texts {
        // The head of a save is stored with its pieces, so it is masked as
        // the text is.
        for (_, piece) in &mut parsed.pieces {
            piece.context = redact(&piece.context).text;
        }
    }
    let mut contexts: HashMap<i64, String> = HashMap::new();
    for (_, piece) in &parsed.pieces {
        if !piece.context.is_empty() {
            contexts.entry(piece.memory).or_insert_with(|| piece.context.clone());
        }
    }
    let notes = match texts {
        Texts::Raw => Vec::new(),
        Texts::Notes(notes) => notes,
    };
    let secret: HashSet<i64> = notes.iter().filter(|n| n.masked).flat_map(|n| n.rows.clone()).collect();
    let note_of: HashMap<i64, usize> = notes.iter().enumerate().flat_map(|(i, n)| n.rows.clone().map(move |id| (id, i))).collect();
    let mut lines = std::mem::take(&mut parsed.memories);
    lines.sort_by_key(|(_, m)| m.id);
    let rows = lines
        .into_iter()
        .map(|(line, src)| {
            let note = note_of.get(&src.id).copied();
            let (content, redacted) = match note {
                None => {
                    let masked = redact(&src.content);
                    (masked.text, !masked.kinds.is_empty())
                }
                Some(_) => (src.content.clone(), secret.contains(&src.id)),
            };
            Row {
                line,
                agent: src.agent_id.as_ref().map(|a| options.agent.clone().unwrap_or_else(|| a.clone())),
                project: src.project_id.as_ref().map(|p| project_ids.get(p).cloned().unwrap_or_else(|| p.clone())),
                hash: content_hash(&content),
                redacted,
                context: contexts.get(&src.id).cloned().unwrap_or_default(),
                lang: language::column_value(language::detect(&content)),
                content,
                src,
                note,
            }
        })
        .collect();
    Prepared { rows, projects, creates, binds, placed, suggestions, deleted, subfolders, mapped_to: Vec::new(), parsed, notes }
}

/// Where a row of the file ended up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    Inserted(i64),
    /// The same row was already here.
    Present(i64),
    /// The same row was already here, in another project.
    Elsewhere(i64),
    /// A live row of the same text was already here.
    Merged(i64),
    /// Inserted superseded by the live row holding its topic key.
    TopicTaken { id: i64, by: i64 },
    /// Inserted superseded by the first live row of a newer version of its
    /// note.
    Older { id: i64, by: i64 },
}

impl Placement {
    fn id(self) -> i64 {
        match self {
            Placement::Inserted(id) | Placement::Present(id) | Placement::Elsewhere(id) | Placement::Merged(id) => id,
            Placement::TopicTaken { id, .. } | Placement::Older { id, .. } => id,
        }
    }

    /// The row here is this file's row, so its links are this file's to fill.
    fn owned(self) -> bool {
        !matches!(self, Placement::Merged(_))
    }

    fn inserted(self) -> bool {
        matches!(self, Placement::Inserted(_) | Placement::TopicTaken { .. } | Placement::Older { .. })
    }
}

/// What the passes so far placed, by the file's row id.
#[derive(Debug, Default, Clone, PartialEq)]
struct Ledger {
    placed: BTreeMap<i64, Placement>,
    /// The agent each row inserted here had in the file, so two agents'
    /// rows merged under one `--agent` still keep one live save per key.
    import_agents: HashMap<i64, Option<String>>,
    /// Project ids a registration gave another root since the ids were
    /// planned, with that root: their rows are left out.
    taken: HashMap<String, String>,
    invalid: Vec<Invalid>,
    dropped: Vec<Invalid>,
    /// What each note of another tool is here, by its first row.
    notes: HashMap<i64, Found>,
    /// Rows of notes that are here whose text is not stored any more: there
    /// is nothing to place, and no link to write or drop.
    absent: HashSet<i64>,
    /// The project of each row found in another project, by its id here.
    elsewhere: BTreeMap<i64, Option<String>>,
}

/// A row here imported from the origin of a note.
#[derive(Debug, Clone, PartialEq)]
struct OriginRow {
    id: i64,
    project: Option<String>,
    key: Option<String>,
    created_at: i64,
    hash: String,
    deleted: bool,
    live: bool,
}

/// What the rows of its origin here make of a note, decided at its first
/// row and kept for the others.
#[derive(Debug, Clone, PartialEq)]
enum Found {
    /// Nothing of its origin is here.
    New,
    /// It is here, in `project`, wherever the map puts it now: each of its
    /// rows is the row of its origin with its text. `kept`: the source
    /// deleted it and its rows here are not deleted.
    Present { project: Option<String>, rows: Vec<OriginRow>, kept: bool },
    /// The source rewrote it: its rows go to the project and the key of the
    /// latest version here, and a row of the text of a live row of its
    /// origin (`shared`) is that row. A newer version replaces the other live
    /// rows (`replaces`); an older one goes under the first live row.
    Revision {
        project: Option<String>,
        key: Option<String>,
        newer: bool,
        shared: Vec<OriginRow>,
        replaces: Vec<i64>,
    },
}

const ORIGIN_SQL: &str = "SELECT id, project_id, topic_key, created_at, content_hash, \
       deleted_at IS NOT NULL, deleted_at IS NULL AND status IN ('active', 'disputed') \
     FROM memory WHERE source = 'import' AND created_by_ref = ?1 ORDER BY id";

const REVISED_SQL: &str = "UPDATE memory SET status = 'superseded', superseded_by = ?2, supersede_cause = 'revision' \
     WHERE id = ?1 AND deleted_at IS NULL AND status IN ('active', 'disputed')";

/// A row to insert, where the rules placed it.
struct NewRow<'a> {
    row: &'a Row,
    project: Option<&'a str>,
    key: Option<&'a str>,
    status: &'a str,
    superseded_by: Option<i64>,
    cause: Option<&'a str>,
    vector: Option<&'a [u8]>,
}

/// A link [`write_links`] writes, translated to the ids here.
enum Link<'a> {
    SupersededBy { id: i64, by: i64 },
    RevivedFrom { id: i64, from: i64 },
    DisputedAgainst { id: i64, against: &'a [i64] },
    /// An inserted row the file has disputed against nothing it placed.
    Undisputed(i64),
    Derivation { memory: i64, source: i64, created_at: i64 },
    Conflict { new_id: i64, old_id: i64, pair: &'a ConflictLine },
    /// The pieces of a new save: each piece's number, row and head.
    Save(&'a [(i64, i64, String)]),
}

/// What the import wrote so far, logged in the transaction of each write so
/// that [`super::undo`] can take back what is still as it was left (V26).
/// Carried from one transaction to the next, as the [`Ledger`] is.
#[derive(Debug)]
struct Journal {
    /// The import's number, which its first write takes.
    id: Option<i64>,
    source: &'static str,
    started_at: i64,
    snapshot: Option<String>,
    /// The rows it inserted: what it writes in them goes with them, so only
    /// what it writes in other rows is an extra.
    inserted: HashSet<i64>,
    /// The store's imports in progress, which hold this one's number until
    /// the journal is dropped.
    running: Arc<std::sync::Mutex<HashSet<i64>>>,
    /// The deleted projects a live row it inserted brought back.
    revived: BTreeSet<String>,
}

impl Journal {
    fn new(store: &MemoryStore, source: &'static str, started_at: i64, snapshot: Option<String>) -> Self {
        Journal {
            id: None,
            source,
            started_at,
            snapshot,
            inserted: HashSet::new(),
            running: store.importing.clone(),
            revived: BTreeSet::new(),
        }
    }

    fn number(&mut self, c: &rusqlite::Connection) -> rusqlite::Result<i64> {
        if let Some(id) = self.id {
            return Ok(id);
        }
        let id: i64 = c.query_row("SELECT COALESCE(MAX(import_id), 0) + 1 FROM memory_import", [], |r| r.get(0))?;
        c.execute(
            "INSERT INTO memory_import (import_id, started_at, source, snapshot) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, self.started_at, self.source, self.snapshot],
        )?;
        self.running.lock().unwrap_or_else(|e| e.into_inner()).insert(id);
        self.id = Some(id);
        Ok(id)
    }

    fn row(&mut self, c: &rusqlite::Connection, id: i64, new: &NewRow<'_>) -> rusqlite::Result<()> {
        let import_id = self.number(c)?;
        c.prepare_cached(
            "INSERT INTO memory_import_row (import_id, row_id, update_count, deleted_at, supersede_cause) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?
        .execute(rusqlite::params![import_id, id, new.row.src.update_count, new.row.src.deleted_at, new.cause])?;
        self.inserted.insert(id);
        Ok(())
    }

    /// What it added to the registry of projects (V31).
    fn registry(&mut self, c: &rusqlite::Connection, kind: &str, a: &str, b: Option<&str>, value: Option<&str>) -> rusqlite::Result<()> {
        let import_id = self.number(c)?;
        c.prepare_cached("INSERT INTO memory_import_registry (import_id, kind, a, b, value) VALUES (?1, ?2, ?3, ?4, ?5)")?
            .execute(rusqlite::params![import_id, kind, a, b, value])?;
        Ok(())
    }

    fn extra(&mut self, c: &rusqlite::Connection, kind: &str, a: Option<i64>, b: &str, value: Option<&str>) -> rusqlite::Result<()> {
        let import_id = self.number(c)?;
        c.prepare_cached("INSERT INTO memory_import_extra (import_id, kind, a, b, value) VALUES (?1, ?2, ?3, ?4, ?5)")?
            .execute(rusqlite::params![import_id, kind, a, b, value])?;
        Ok(())
    }

    /// A link written in row `id` where it was empty, unless the import
    /// inserted that row.
    fn fill(&mut self, c: &rusqlite::Connection, changed: usize, id: i64, column: &str, value: &str) -> rusqlite::Result<()> {
        match changed == 1 && !self.inserted.contains(&id) {
            true => self.extra(c, "fill", Some(id), column, Some(value)),
            false => Ok(()),
        }
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.running.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        }
    }
}

/// What placing rows reads of the store and writes to it. The rules are
/// written once over it: [`SqlSeat`] is the transaction that writes, and
/// [`Planned`] what the pass that only decides loaded of the store, so that
/// pass leaves the connection to every other call.
trait Seat {
    type Error;
    /// The rows imported from `origin`, by id.
    fn origin_rows(&mut self, origin: &str) -> Result<Vec<OriginRow>, Self::Error>;
    /// [`PRESENT_SQL`].
    fn present(&mut self, agent: Option<&str>, project: Option<&str>, hash: &str, created_at: i64) -> Result<Option<i64>, Self::Error>;
    /// [`PRESENT_ELSEWHERE_SQL`]: the row and its project.
    fn present_elsewhere(&mut self, agent: Option<&str>, hash: &str, created_at: i64, by_ref: &str) -> Result<Option<(i64, Option<String>)>, Self::Error>;
    /// [`TWIN_SQL`].
    fn twin(&mut self, project: Option<&str>, hash: &str) -> Result<Option<i64>, Self::Error>;
    /// [`TOPIC_HOLDERS_SQL`].
    fn holders(&mut self, project: Option<&str>, key: &str, origin: Option<&str>) -> Result<Vec<i64>, Self::Error>;
    fn insert(&mut self, new: &NewRow<'_>) -> Result<i64, Self::Error>;
    /// [`REVISED_SQL`].
    fn revise(&mut self, id: i64, by: i64) -> Result<(), Self::Error>;
    /// Whether row `id` is a piece of a save already.
    fn linked(&mut self, id: i64) -> Result<bool, Self::Error>;
    fn link(&mut self, link: Link<'_>) -> Result<(), Self::Error>;
}

/// The rules over the transaction they write in, journaling each write. A
/// project merged since the plan is read and written as the one it went into.
struct SqlSeat<'t, 'c>(&'t Transaction<'c>, &'t mut Journal);

impl SqlSeat<'_, '_> {
    fn live(&self, project: Option<&str>) -> rusqlite::Result<Option<String>> {
        project.map(|p| canonical_in(self.0, p)).transpose()
    }
}

impl Seat for SqlSeat<'_, '_> {
    type Error = rusqlite::Error;

    fn origin_rows(&mut self, origin: &str) -> rusqlite::Result<Vec<OriginRow>> {
        let mut stmt = self.0.prepare_cached(ORIGIN_SQL)?;
        let found = stmt.query_map([origin], |r| {
            Ok(OriginRow {
                id: r.get(0)?,
                project: r.get(1)?,
                key: r.get(2)?,
                created_at: r.get(3)?,
                hash: r.get(4)?,
                deleted: r.get(5)?,
                live: r.get(6)?,
            })
        })?;
        found.collect()
    }

    fn present(&mut self, agent: Option<&str>, project: Option<&str>, hash: &str, created_at: i64) -> rusqlite::Result<Option<i64>> {
        let project = self.live(project)?;
        self.0.prepare_cached(PRESENT_SQL)?.query_row(rusqlite::params![agent, project, hash, created_at], |r| r.get(0)).optional()
    }

    fn present_elsewhere(&mut self, agent: Option<&str>, hash: &str, created_at: i64, by_ref: &str) -> rusqlite::Result<Option<(i64, Option<String>)>> {
        let mut stmt = self.0.prepare_cached(PRESENT_ELSEWHERE_SQL)?;
        stmt.query_row(rusqlite::params![created_at, agent, hash, by_ref], |r| Ok((r.get(0)?, r.get(1)?))).optional()
    }

    fn twin(&mut self, project: Option<&str>, hash: &str) -> rusqlite::Result<Option<i64>> {
        let project = self.live(project)?;
        self.0.prepare_cached(TWIN_SQL)?.query_row(rusqlite::params![project, hash], |r| r.get(0)).optional()
    }

    fn holders(&mut self, project: Option<&str>, key: &str, origin: Option<&str>) -> rusqlite::Result<Vec<i64>> {
        let project = self.live(project)?;
        let mut stmt = self.0.prepare_cached(TOPIC_HOLDERS_SQL)?;
        let holders = stmt.query_map(rusqlite::params![project, key, origin], |r| r.get(0))?.collect();
        holders
    }

    fn insert(&mut self, new: &NewRow<'_>) -> rusqlite::Result<i64> {
        let (row, src) = (new.row, &new.row.src);
        let project = self.live(new.project)?;
        // A deleted project stays deleted under a row that comes deleted, which
        // the insert trigger sees live; a live row brings it back.
        let was: Option<(Option<i64>, bool)> = match &project {
            Some(p) => self
                .0
                .prepare_cached("SELECT deleted_at, reviewed FROM project WHERE id = ?1")?
                .query_row([p], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?,
            None => None,
        };
        self.0.prepare_cached(INSERT_SQL)?.execute(rusqlite::params![
            src.scope,
            row.agent,
            project,
            row.content,
            src.memory_type,
            src.source,
            src.created_at,
            src.created_by_type,
            src.created_by_ref,
            src.source_session_id,
            src.last_modified_at,
            src.last_modified_by,
            src.update_count,
            src.last_accessed_at,
            src.access_count,
            src.importance,
            src.tier,
            new.status,
            new.superseded_by,
            new.cause,
            src.dispute_count,
            src.last_disputed_at,
            row.hash,
            new.key,
            row.lang,
            src.origin_project,
        ])?;
        let id = self.0.last_insert_rowid();
        match src.deleted_at {
            // Inserted live and then deleted, so the text index trigger of V8
            // adds it and takes it out again: a deleted row is not indexed, and a
            // restore can add it back exactly once.
            Some(at) => {
                self.0.execute("UPDATE memory SET deleted_at = ?2 WHERE id = ?1", rusqlite::params![id, at])?;
                if let (Some(p), Some((Some(deleted_at), reviewed))) = (&project, was) {
                    self.0.execute(
                        "UPDATE project SET deleted_at = ?2, reviewed = ?3 WHERE id = ?1",
                        rusqlite::params![p, deleted_at, reviewed],
                    )?;
                }
            }
            None => {
                if let Some(bytes) = new.vector {
                    self.0.execute("INSERT INTO memory_vectors (rowid, embedding) VALUES (?1, ?2)", rusqlite::params![id, bytes])?;
                }
                if let (Some(p), Some((Some(deleted_at), _))) = (&project, was) {
                    self.1.registry(self.0, "revived", p, None, Some(&deleted_at.to_string()))?;
                    self.1.revived.insert(p.clone());
                }
            }
        }
        self.1.row(self.0, id, new)?;
        Ok(id)
    }

    fn revise(&mut self, id: i64, by: i64) -> rusqlite::Result<()> {
        let before: Option<(String, Option<i64>, Option<String>)> = self
            .0
            .query_row("SELECT status, superseded_by, supersede_cause FROM memory WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        if self.0.prepare_cached(REVISED_SQL)?.execute([id, by])? == 1 {
            let (status, superseded_by, cause) = before.expect("a row that was revised is there");
            let import_id = self.1.number(self.0)?;
            self.0.execute(
                "INSERT INTO memory_import_supersede \
                 (import_id, row_id, superseded_by, from_status, from_superseded_by, from_cause) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![import_id, id, by, status, superseded_by, cause],
            )?;
        }
        Ok(())
    }

    fn linked(&mut self, id: i64) -> rusqlite::Result<bool> {
        self.0.prepare_cached(LINKED_SQL)?.query_row([id], |r| r.get(0))
    }

    fn link(&mut self, link: Link<'_>) -> rusqlite::Result<()> {
        let (tx, journal) = (self.0, &mut *self.1);
        match link {
            Link::SupersededBy { id, by } => {
                let changed = tx.execute(
                    "UPDATE memory SET superseded_by = ?2 WHERE id = ?1 AND superseded_by IS NULL AND status = 'superseded'",
                    [id, by],
                )?;
                journal.fill(tx, changed, id, "superseded_by", &by.to_string())?;
            }
            Link::RevivedFrom { id, from } => {
                let changed =
                    tx.execute("UPDATE memory SET revived_from = ?2 WHERE id = ?1 AND revived_from IS NULL", [id, from])?;
                journal.fill(tx, changed, id, "revived_from", &from.to_string())?;
            }
            Link::DisputedAgainst { id, against } => {
                let list = serde_json::to_string(against).expect("Vec<i64> serialises infallibly");
                let changed = tx.execute(
                    "UPDATE memory SET disputed_against = ?2 WHERE id = ?1 AND disputed_against IS NULL AND status = 'disputed'",
                    rusqlite::params![id, list],
                )?;
                journal.fill(tx, changed, id, "disputed_against", &list)?;
            }
            Link::Undisputed(id) => {
                tx.execute("UPDATE memory SET status = 'active' WHERE id = ?1 AND status = 'disputed'", [id])?;
            }
            Link::Derivation { memory, source, created_at } => {
                let changed = tx.execute(
                    "INSERT OR IGNORE INTO memory_derivation (memory_id, source_id, created_at) VALUES (?1, ?2, ?3)",
                    [memory, source, created_at],
                )?;
                if changed == 1 && !journal.inserted.contains(&memory) {
                    journal.extra(tx, "derivation", Some(memory), &source.to_string(), None)?;
                }
            }
            Link::Conflict { new_id, old_id, pair } => {
                let reason = pair.reason.as_deref().map(|r| redact(r).text);
                let changed = tx.execute(
                    "INSERT OR IGNORE INTO memory_conflict \
                     (new_id, old_id, distance, state, verdict, reason, created_at, judged_at, topic_key_warned) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![
                        new_id,
                        old_id,
                        pair.distance,
                        pair.state,
                        pair.verdict,
                        reason,
                        pair.created_at,
                        pair.judged_at,
                        pair.topic_key_warned
                    ],
                )?;
                if changed == 1 {
                    let written = serde_json::json!([pair.state, pair.verdict]).to_string();
                    journal.extra(tx, "pair", Some(new_id), &old_id.to_string(), Some(&written))?;
                }
            }
            Link::Save(pieces) => {
                if pieces.is_empty() {
                    return Ok(());
                }
                let save_id: i64 = tx.query_row(NEXT_SAVE_SQL, [], |r| r.get(0))?;
                for (piece, id, context) in pieces {
                    let changed = tx.execute(
                        "INSERT OR IGNORE INTO memory_piece (save_id, piece, memory_id, context) VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![save_id, piece, id, context],
                    )?;
                    if changed == 1 && !journal.inserted.contains(id) {
                        journal.extra(tx, "piece", Some(save_id), &piece.to_string(), None)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Lookups per call while [`Planned`] is loaded. Each is a seek in an index,
/// so a call lasts a few milliseconds and what else waits on the connection
/// waits no longer, however long the file.
const LOAD_BATCH: usize = 256;

/// The rows of `memory` the rules can read, filtered by `$filter`.
macro_rules! known_rows {
    ($filter:literal) => {
        concat!(
            "SELECT id, agent_id, project_id, content_hash, created_at, topic_key, deleted_at IS NOT NULL, status, \
             CASE WHEN source = 'import' THEN created_by_ref END, created_by_ref FROM memory WHERE ",
            $filter
        )
    };
}

const ORIGIN_ROWS_SQL: &str = known_rows!("source = 'import' AND created_by_ref = ?1");
/// Every row of a text in a project, deleted or not, as [`PRESENT_SQL`] and
/// [`TWIN_SQL`] read them.
const TEXT_ROWS_SQL: &str = known_rows!("COALESCE(project_id, '') = ?1 AND content_hash = ?2");
/// The rows of a key in a project that [`TOPIC_HOLDERS_SQL`] can name.
const KEY_ROWS_SQL: &str = known_rows!("COALESCE(project_id, '') = ?1 AND topic_key = ?2 AND deleted_at IS NULL");
/// The rows of a date, in any project, as [`PRESENT_ELSEWHERE_SQL`] reads them.
const DATE_ROWS_SQL: &str = known_rows!("created_at = ?1");
/// The id `AUTOINCREMENT` gives the next row: above every id `memory` ever
/// had, not only the ones it has.
const NEXT_ID_SQL: &str = "SELECT MAX(COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'memory'), 0), \
     COALESCE((SELECT MAX(id) FROM memory), 0)) + 1";

/// A row of `memory` as the rules read it.
#[derive(Debug, Clone)]
struct Known {
    id: i64,
    agent: Option<String>,
    project: Option<String>,
    hash: Option<String>,
    created_at: i64,
    key: Option<String>,
    deleted: bool,
    status: String,
    /// `created_by_ref` of an imported row.
    origin: Option<String>,
    by_ref: String,
}

impl Known {
    fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Known {
            id: r.get(0)?,
            agent: r.get(1)?,
            project: r.get(2)?,
            hash: r.get(3)?,
            created_at: r.get(4)?,
            key: r.get(5)?,
            deleted: r.get(6)?,
            status: r.get(7)?,
            origin: r.get(8)?,
            by_ref: r.get(9)?,
        })
    }

    fn live(&self) -> bool {
        !self.deleted && matches!(self.status.as_str(), "active" | "disputed")
    }

    fn origin_row(&self) -> OriginRow {
        OriginRow {
            id: self.id,
            project: self.project.clone(),
            key: self.key.clone(),
            created_at: self.created_at,
            hash: self.hash.clone().unwrap_or_default(),
            deleted: self.deleted,
            live: self.live(),
        }
    }
}

/// A project as the SQL compares it, `COALESCE(project_id, '')`.
fn scope(project: Option<&str>) -> String {
    project.unwrap_or_default().to_owned()
}

/// What the rules can read of the store for one file, loaded beforehand,
/// and what placing its rows adds to it. Each list of ids is in the order
/// of the ids, as the SQL reads them.
#[derive(Debug, Default)]
struct Planned {
    rows: HashMap<i64, Known>,
    by_text: HashMap<(String, String), Vec<i64>>,
    by_key: HashMap<(String, String), Vec<i64>>,
    by_origin: HashMap<String, Vec<i64>>,
    by_date: HashMap<i64, Vec<i64>>,
    /// Rows that are a piece of a save.
    linked: HashSet<i64>,
    /// The root of each project id here.
    roots: HashMap<String, String>,
    next_id: i64,
}

impl Planned {
    fn know(&mut self, row: Known) {
        let project = scope(row.project.as_deref());
        if let Some(hash) = &row.hash {
            self.by_text.entry((project.clone(), hash.clone())).or_default().push(row.id);
        }
        if let Some(key) = &row.key {
            self.by_key.entry((project, key.clone())).or_default().push(row.id);
        }
        if let Some(origin) = &row.origin {
            self.by_origin.entry(origin.clone()).or_default().push(row.id);
        }
        self.by_date.entry(row.created_at).or_default().push(row.id);
        self.rows.insert(row.id, row);
    }

    fn among<'a>(&'a self, ids: Option<&'a Vec<i64>>) -> impl Iterator<Item = &'a Known> {
        ids.into_iter().flatten().map(|id| &self.rows[id])
    }
}

impl Seat for Planned {
    type Error = Infallible;

    fn origin_rows(&mut self, origin: &str) -> Result<Vec<OriginRow>, Infallible> {
        Ok(self.among(self.by_origin.get(origin)).map(Known::origin_row).collect())
    }

    fn present(&mut self, agent: Option<&str>, project: Option<&str>, hash: &str, created_at: i64) -> Result<Option<i64>, Infallible> {
        let rows = self.among(self.by_text.get(&(scope(project), hash.to_owned())));
        Ok(rows.filter(|k| k.agent.as_deref() == agent && k.created_at == created_at).map(|k| k.id).next())
    }

    fn present_elsewhere(&mut self, agent: Option<&str>, hash: &str, created_at: i64, by_ref: &str) -> Result<Option<(i64, Option<String>)>, Infallible> {
        let mut rows = self.among(self.by_date.get(&created_at));
        let found = rows.find(|k| k.agent.as_deref() == agent && k.hash.as_deref() == Some(hash) && k.by_ref == by_ref);
        Ok(found.map(|k| (k.id, k.project.clone())))
    }

    fn twin(&mut self, project: Option<&str>, hash: &str) -> Result<Option<i64>, Infallible> {
        let rows = self.among(self.by_text.get(&(scope(project), hash.to_owned())));
        Ok(rows.filter(|k| k.live()).min_by_key(|k| (k.created_at, k.id)).map(|k| k.id))
    }

    fn holders(&mut self, project: Option<&str>, key: &str, origin: Option<&str>) -> Result<Vec<i64>, Infallible> {
        let rows = self.among(self.by_key.get(&(scope(project), key.to_owned())));
        Ok(rows.filter(|k| k.live() && !(origin.is_some() && k.origin.as_deref() == origin)).map(|k| k.id).collect())
    }

    fn insert(&mut self, new: &NewRow<'_>) -> Result<i64, Infallible> {
        let (id, src) = (self.next_id, &new.row.src);
        self.next_id += 1;
        self.know(Known {
            id,
            agent: new.row.agent.clone(),
            project: new.project.map(str::to_owned),
            hash: Some(new.row.hash.clone()),
            created_at: src.created_at,
            key: new.key.map(str::to_owned),
            deleted: src.deleted_at.is_some(),
            status: new.status.to_owned(),
            origin: (src.source == "import").then(|| src.created_by_ref.clone()),
            by_ref: src.created_by_ref.clone(),
        });
        Ok(id)
    }

    fn revise(&mut self, id: i64, _by: i64) -> Result<(), Infallible> {
        if let Some(row) = self.rows.get_mut(&id).filter(|r| r.live()) {
            row.status = "superseded".to_owned();
        }
        Ok(())
    }

    fn linked(&mut self, id: i64) -> Result<bool, Infallible> {
        Ok(self.linked.contains(&id))
    }

    fn link(&mut self, link: Link<'_>) -> Result<(), Infallible> {
        if let Link::Save(pieces) = link {
            self.linked.extend(pieces.iter().map(|(_, id, _)| *id));
        }
        Ok(())
    }
}

/// Runs `sql` once per set of `binds`, [`LOAD_BATCH`] to a call.
async fn known<B: rusqlite::ToSql + Clone + Send + 'static>(
    store: &MemoryStore,
    sql: &'static str,
    binds: Vec<Vec<B>>,
    timing: &mut Timing,
) -> Result<Vec<Known>, MemoryError> {
    let mut rows = Vec::new();
    for batch in binds.chunks(LOAD_BATCH) {
        let batch = batch.to_vec();
        let found = timed(store, timing, move |c| {
            let mut stmt = c.prepare_cached(sql)?;
            let mut rows = Vec::new();
            for bind in &batch {
                let found = stmt.query_map(rusqlite::params_from_iter(bind), Known::read)?;
                for row in found {
                    rows.push(row?);
                }
            }
            Ok(rows)
        })
        .await?;
        rows.extend(found);
    }
    Ok(rows)
}

/// Loads what the rules can read of the store for `prepared`. The rows of
/// each origin of the file come first: with the file's own, their projects
/// and keys are every project and key the rules can ask about.
async fn load(store: &MemoryStore, prepared: &Prepared, timing: &mut Timing) -> Result<Planned, MemoryError> {
    let origins: BTreeSet<&str> = prepared.notes.iter().map(|n| n.origin.as_str()).collect();
    let binds = origins.iter().map(|o| vec![o.to_string()]).collect();
    let mut rows = known(store, ORIGIN_ROWS_SQL, binds, timing).await?;

    let mut reach: HashMap<&str, (BTreeSet<String>, BTreeSet<String>)> = HashMap::new();
    for row in &rows {
        if let Some(origin) = row.origin.as_deref() {
            let (projects, keys) = reach.entry(origin).or_default();
            projects.insert(scope(row.project.as_deref()));
            keys.extend(row.key.clone());
        }
    }
    for row in &prepared.rows {
        if let Some(note) = row.note {
            let (projects, keys) = reach.entry(prepared.notes[note].origin.as_str()).or_default();
            projects.insert(scope(row.project.as_deref()));
            keys.extend(row.src.topic_key.clone());
        }
    }
    let mut texts: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut keys: BTreeSet<Vec<String>> = BTreeSet::new();
    for row in &prepared.rows {
        let own = (BTreeSet::from([scope(row.project.as_deref())]), row.src.topic_key.iter().cloned().collect());
        let (projects, row_keys) = match row.note {
            Some(note) => &reach[prepared.notes[note].origin.as_str()],
            None => &own,
        };
        for project in projects {
            texts.insert(vec![project.clone(), row.hash.clone()]);
            keys.extend(row_keys.iter().map(|key| vec![project.clone(), key.clone()]));
        }
    }
    rows.extend(known(store, TEXT_ROWS_SQL, texts.into_iter().collect(), timing).await?);
    rows.extend(known(store, KEY_ROWS_SQL, keys.into_iter().collect(), timing).await?);
    let dates: BTreeSet<i64> = prepared.rows.iter().filter(|r| r.note.is_none()).map(|r| r.src.created_at).collect();
    rows.extend(known(store, DATE_ROWS_SQL, dates.into_iter().map(|d| vec![d]).collect(), timing).await?);
    rows.sort_by_key(|r| r.id);
    rows.dedup_by_key(|r| r.id);
    timing.rows = Some(rows.len());

    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let mut linked = HashSet::new();
    for batch in ids.chunks(LOAD_BATCH) {
        let batch = batch.to_vec();
        let found = timed(store, timing, move |c| {
            let mut stmt = c.prepare_cached(LINKED_SQL)?;
            let mut linked = Vec::new();
            for id in batch {
                if stmt.query_row([id], |r| r.get(0))? {
                    linked.push(id);
                }
            }
            Ok(linked)
        })
        .await?;
        linked.extend(found);
    }

    let root_ids: Vec<String> = prepared.projects.iter().map(|p| p.root_id.clone()).collect();
    let (roots, next_id) = timed(store, timing, move |c| {
        let mut roots = HashMap::new();
        let mut stmt = c.prepare_cached(PROJECT_ROOT_SQL)?;
        for id in root_ids {
            if let Some(root) = stmt.query_row([&id], |r| r.get(0)).optional()? {
                roots.insert(id, root);
            }
        }
        Ok((roots, c.query_row(NEXT_ID_SQL, [], |r| r.get(0))?))
    })
    .await?;

    let mut planned = Planned { linked, roots, next_id, ..Default::default() };
    for row in rows {
        planned.know(row);
    }
    Ok(planned)
}

/// Decides where every row of `prepared` goes, as the writing pass would on
/// the store `planned` holds, or nothing once `stop` is set.
fn decide(mut planned: Planned, prepared: &Prepared, stop: &AtomicBool) -> Option<Ledger> {
    let (taken, _) = taken_projects(&prepared.projects, &mut planned.roots);
    let mut ledger = Ledger { taken, ..Default::default() };
    for range in note_batches(&prepared.units(), WRITE_BATCH) {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        let Ok(()) = place_rows(&mut planned, prepared, range, &mut ledger, &HashMap::new(), false);
    }
    let Ok(()) = write_links(&mut planned, prepared, &mut ledger);
    Some(ledger)
}

/// Stops [`decide`] when the import waiting for it is dropped, as a shutdown
/// that cuts the request drops it.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// What the rows of `note`'s origin here make of it; `rows` are the note's.
/// A note is the same version as a row of its origin with its date, the one
/// a source gives a text when it writes or rewrites it.
fn find<S: Seat>(seat: &mut S, note: &NoteRows, rows: &[&Row]) -> Result<Found, S::Error> {
    let stored = seat.origin_rows(&note.origin)?;
    let Some(first) = rows.first() else { return Ok(Found::New) };
    if stored.is_empty() {
        return Ok(Found::New);
    }
    let written = first.src.created_at;
    let deleted_at_source = first.src.deleted_at.is_some();
    let undeleted: Vec<&OriginRow> = stored.iter().filter(|r| !r.deleted).collect();
    let present =
        |project: Option<String>| Found::Present { project, rows: stored.clone(), kept: deleted_at_source && !undeleted.is_empty() };
    if let Some(same) = stored.iter().find(|r| r.created_at == written) {
        return Ok(present(same.project.clone()));
    }
    let Some(latest) = newest(undeleted.iter().copied()) else {
        // Deleted here: an import never brings back what the user deleted.
        return Ok(present(newest(&stored).and_then(|r| r.project.clone())));
    };
    // Without a date of its own the note cannot tell its version, and a note
    // the source deleted leaves what is here as it is.
    if note.undated || deleted_at_source {
        return Ok(present(latest.project.clone()));
    }
    let newer = written > latest.created_at;
    let shared: Vec<OriginRow> = stored.iter().filter(|r| r.live).cloned().collect();
    let hashes: HashSet<&str> = rows.iter().map(|r| r.hash.as_str()).collect();
    let replaces = match newer {
        true => shared.iter().filter(|r| !hashes.contains(r.hash.as_str())).map(|r| r.id).collect(),
        false => Vec::new(),
    };
    Ok(Found::Revision {
        project: latest.project.clone(),
        key: latest.key.clone().or_else(|| first.src.topic_key.clone()),
        newer,
        shared,
        replaces,
    })
}

/// The first row of the version with the latest date.
fn newest<'a>(rows: impl IntoIterator<Item = &'a OriginRow>) -> Option<&'a OriginRow> {
    rows.into_iter().min_by_key(|r| (Reverse(r.created_at), r.id))
}

/// The row here of a row of a note that is here: the row of its origin with
/// its text, of the note's own date first, or the live row of its text it was
/// merged into. `None` when its text is not stored here any more.
fn present_at<S: Seat>(seat: &mut S, row: &Row, project: Option<&str>, stored: &[OriginRow]) -> Result<Option<Placement>, S::Error> {
    let own = stored.iter().filter(|r| r.hash == row.hash).min_by_key(|r| (r.created_at != row.src.created_at, r.id));
    if let Some(r) = own {
        return Ok(Some(Placement::Present(r.id)));
    }
    Ok(seat.twin(project, &row.hash)?.map(Placement::Merged))
}

/// The project and key `row` goes to here: its own, or the latest stored
/// version's for a rewrite of a note that is here.
fn target<'a>(row: &'a Row, found: Option<&'a Found>) -> (Option<&'a str>, Option<&'a str>) {
    match found {
        Some(Found::Revision { project, key, .. }) => (project.as_deref(), key.as_deref()),
        _ => (row.project.as_deref(), row.src.topic_key.as_deref()),
    }
}

const PRESENT_SQL: &str = "SELECT id FROM memory \
     WHERE agent_id IS ?1 AND COALESCE(project_id, '') = COALESCE(?2, '') \
       AND content_hash = ?3 AND created_at = ?4 \
     ORDER BY id LIMIT 1";

/// The same row in any project, deleted or not, as a move leaves it. Its
/// `created_by_ref` keeps apart two notes of another tool with one text and
/// date in two projects of the source.
const PRESENT_ELSEWHERE_SQL: &str = "SELECT id, project_id FROM memory \
     WHERE created_at = ?1 AND agent_id IS ?2 AND content_hash = ?3 AND created_by_ref = ?4 \
     ORDER BY id LIMIT 1";

/// The live row of the same text in the project, whoever saved it, as the
/// worker's dedup finds it. An import is the user's, so no policy narrows it.
/// Of two of the same date the first, as [`Planned`] picks it.
const TWIN_SQL: &str = "SELECT id FROM memory \
     WHERE COALESCE(project_id, '') = COALESCE(?1, '') \
       AND content_hash = ?2 AND deleted_at IS NULL AND status IN ('active', 'disputed') \
     ORDER BY created_at ASC, id ASC LIMIT 1";

/// The live rows of the project that hold the key, but for the rows of the
/// origin `?3`, which a revision replaces instead of going under them.
const TOPIC_HOLDERS_SQL: &str = "SELECT id FROM memory \
     WHERE COALESCE(project_id, '') = COALESCE(?1, '') \
       AND topic_key = ?2 AND deleted_at IS NULL AND status IN ('active', 'disputed') \
       AND NOT (source = 'import' AND created_by_ref IS ?3) \
     ORDER BY id";

/// `disputed_against`, `revived_from` and a `superseded_by` from the file
/// are links, written once every row has its id here.
const INSERT_SQL: &str = "INSERT INTO memory \
     (scope, agent_id, project_id, content, memory_type, source, created_at, created_by_type, created_by_ref, \
      source_session_id, last_modified_at, last_modified_by, update_count, last_accessed_at, access_count, \
      importance, tier, status, superseded_by, supersede_cause, dispute_count, last_disputed_at, content_hash, topic_key, \
      lang, origin_project) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)";

const PROJECT_ROOT_SQL: &str = "SELECT root_path FROM project_path WHERE id = ?1";

/// The project ids whose rows stay out, with the root that holds them (an id
/// a registration gave another root since the ids were planned), and the
/// projects whose remote is recorded. `roots` holds the root of each project
/// id here, and gets the ones recording `projects` adds.
fn taken_projects<'p>(
    projects: &'p [ProjectHere],
    roots: &mut HashMap<String, String>,
) -> (HashMap<String, String>, Vec<&'p ProjectHere>) {
    let mut taken = HashMap::new();
    let mut remotes = Vec::new();
    for p in projects {
        let existing = roots.entry(p.root_id.clone()).or_insert_with(|| p.root.clone());
        if *existing != p.root && p.project_id == p.root_id {
            taken.insert(p.project_id.clone(), existing.clone());
            continue;
        }
        if p.remote.is_some() {
            remotes.push(p);
        }
    }
    (taken, remotes)
}

/// Records each project's folder as a registration would, the projects the
/// import creates and the remotes it binds, journaling what it adds, and
/// returns [`taken_projects`].
fn record_projects(tx: &Transaction<'_>, prepared: &Prepared, now: i64, journal: &mut Journal) -> rusqlite::Result<HashMap<String, String>> {
    let projects = &prepared.projects;
    let mut roots = HashMap::new();
    for p in projects {
        let root: Option<String> = tx.prepare_cached(PROJECT_ROOT_SQL)?.query_row([&p.root_id], |r| r.get(0)).optional()?;
        if let Some(root) = root {
            roots.insert(p.root_id.clone(), root);
        }
    }
    let (taken, _) = taken_projects(projects, &mut roots);
    for p in projects {
        // A folder without its project would be read under its own id, which
        // is another project when the plan followed a move. A project merged
        // since the plan leads to the one it went into.
        let added = tx.execute(
            "INSERT OR IGNORE INTO project_path (id, root_path, created_at, index_docs, project_id, remote, root_commit, git_stamp) \
             VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?7)",
            rusqlite::params![p.root_id, p.root, now, canonical_in(tx, &p.project_id)?, p.remote, p.root_commit, p.git_stamp],
        )?;
        if added == 1 {
            journal.extra(tx, "project", None, &p.root_id, None)?;
        }
    }
    for (id, name) in &prepared.creates {
        if taken.contains_key(id) {
            continue;
        }
        let added = tx.execute(
            "INSERT OR IGNORE INTO project (id, name, born, reviewed, created_at) VALUES (?1, ?2, 'import', 0, ?3)",
            rusqlite::params![id, name, now],
        )?;
        if added == 1 {
            journal.registry(tx, "project", id, None, Some(name))?;
        }
    }
    for d in prepared.subfolders.iter().filter(|d| d.written) {
        let (Some(repository), Some(project)) = (&d.repository_here, &d.project_here) else { continue };
        let (repository, project) = (canonical_in(tx, repository)?, canonical_in(tx, project)?);
        if repository == project || taken.contains_key(&repository) || taken.contains_key(&project) {
            continue;
        }
        let added = tx.execute(
            "INSERT OR IGNORE INTO project_subfolder (parent_id, subpath, project_id, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![repository, d.subpath, project, now],
        )?;
        if added == 1 {
            journal.registry(tx, "subfolder", &repository, Some(&d.subpath), Some(&project))?;
        }
    }
    for s in &prepared.suggestions {
        if taken.contains_key(&s.project_id) || taken.contains_key(&s.other_id) {
            continue;
        }
        let added = tx.execute(
            "INSERT OR IGNORE INTO project_suggestion (project_id, other_id, reason, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![s.project_id, s.other_id, s.reason, now],
        )?;
        if added == 1 {
            journal.registry(tx, "suggestion", &s.project_id, Some(&s.other_id), Some(&s.reason))?;
        }
    }
    for (remote, project) in &prepared.binds {
        if taken.contains_key(project) {
            continue;
        }
        let added = tx.execute(
            "INSERT OR IGNORE INTO project_remote (remote, project_id, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![remote, canonical_in(tx, project)?, now],
        )?;
        if added == 1 {
            journal.extra(tx, "remote", None, remote, None)?;
        }
    }
    Ok(taken)
}

/// Places one row where `found` sends the rows of its note: finds it here,
/// or inserts it with `vector`. `None` when the row would be inserted live
/// and has no vector, which only happens on the writing pass when the store
/// changed since the first.
fn place<S: Seat>(
    seat: &mut S,
    row: &Row,
    found: Option<&Found>,
    ledger: &mut Ledger,
    vector: Option<&[u8]>,
    needs_vector: bool,
) -> Result<Option<Placement>, S::Error> {
    let (project, key) = target(row, found);
    let (under, origin) = match found {
        Some(Found::Revision { newer, shared, .. }) => {
            if let Some(r) = shared.iter().find(|r| r.hash == row.hash) {
                return Ok(Some(Placement::Merged(r.id)));
            }
            let under = if *newer { None } else { shared.first().map(|r| r.id) };
            (under, Some(row.src.created_by_ref.as_str()))
        }
        _ => (None, None),
    };
    if let Some(id) = seat.present(row.agent.as_deref(), project, &row.hash, row.src.created_at)? {
        return Ok(Some(Placement::Present(id)));
    }
    if row.note.is_none() {
        if let Some((id, at)) = seat.present_elsewhere(row.agent.as_deref(), &row.hash, row.src.created_at, &row.src.created_by_ref)? {
            ledger.elsewhere.insert(id, at);
            return Ok(Some(Placement::Elsewhere(id)));
        }
    }
    let mut topic_holder = None;
    if under.is_none() && row.live() {
        if let Some(id) = seat.twin(project, &row.hash)? {
            return Ok(Some(Placement::Merged(id)));
        }
        if let Some(key) = key {
            // The pieces of one save in the file share its key.
            topic_holder = seat.holders(project, key, origin)?.into_iter().find(|id| match ledger.import_agents.get(id) {
                Some(agent) => *agent != row.src.agent_id,
                None => true,
            });
        }
    }
    if needs_vector && row.src.deleted_at.is_none() && vector.is_none() {
        return Ok(None);
    }
    let (status, superseded_by, cause) = match under.or(topic_holder) {
        Some(by) => ("superseded", Some(by), Some("revision")),
        None => (row.src.status.as_str(), None, row.src.supersede_cause.as_deref()),
    };
    let id = seat.insert(&NewRow { row, project, key, status, superseded_by, cause, vector })?;
    ledger.import_agents.insert(id, row.src.agent_id.clone());
    Ok(Some(match (under, topic_holder) {
        (Some(by), _) => Placement::Older { id, by },
        (None, Some(by)) => Placement::TopicTaken { id, by },
        (None, None) => Placement::Inserted(id),
    }))
}

/// Places the rows `range` of `prepared`, skipping the ones a project of
/// another root keeps out. The rows of a note go where the rows of its
/// origin here send them, found at its first row, which a batch never
/// leaves out.
fn place_rows<S: Seat>(
    seat: &mut S,
    prepared: &Prepared,
    range: Range<usize>,
    ledger: &mut Ledger,
    vectors: &HashMap<i64, Vec<u8>>,
    needs_vector: bool,
) -> Result<(), S::Error> {
    let rows = &prepared.rows[range];
    let mut current = None;
    for (k, row) in rows.iter().enumerate() {
        let note = row.note.map(|n| &prepared.notes[n]);
        let first = note.is_some_and(|n| n.rows.start == row.src.id);
        if let (Some(n), true) = (note, first) {
            let own: Vec<&Row> = rows[k..].iter().take_while(|r| r.note == row.note).collect();
            let found = find(seat, n, &own)?;
            ledger.notes.insert(n.rows.start, found.clone());
            current = Some(found);
        }
        let found = note.and(current.as_ref());
        // A note here or rewritten goes where it is, not to the project of
        // the map.
        let goes_to_map = matches!(found, None | Some(Found::New));
        if let Some((id, root)) =
            row.project.as_ref().filter(|_| goes_to_map).and_then(|p| ledger.taken.get(p).map(|root| (p, root)))
        {
            ledger.invalid.push(Invalid { line: row.line, reason: format!("project id {id} already belongs to {root}") });
            continue;
        }
        let placement = match found {
            Some(Found::Present { project, rows: stored, .. }) => match present_at(seat, row, project.as_deref(), stored)? {
                Some(placement) => placement,
                None => {
                    ledger.absent.insert(row.src.id);
                    continue;
                }
            },
            _ => match place(seat, row, found, ledger, vectors.get(&row.src.id).map(Vec::as_slice), needs_vector)? {
                Some(placement) => placement,
                None => {
                    ledger.invalid.push(Invalid {
                        line: row.line,
                        reason: "the store changed while importing; import the file again".into(),
                    });
                    continue;
                }
            },
        };
        ledger.placed.insert(row.src.id, placement);
        // In the transaction that stores the note, so no cut between two
        // transactions leaves both versions live.
        if let (true, Some(Found::Revision { replaces, .. })) = (first, found) {
            for id in replaces {
                seat.revise(*id, placement.id())?;
            }
        }
    }
    Ok(())
}

/// Whether row `?1` is a piece of a save already.
const LINKED_SQL: &str = "SELECT EXISTS (SELECT 1 FROM memory_piece WHERE memory_id = ?1)";

const NEXT_SAVE_SQL: &str = "SELECT COALESCE(MAX(save_id), 0) + 1 FROM memory_piece";

/// Writes every link of the file whose ends were placed, translated to the
/// ids here. Rows already here only get the links they lack, and only the
/// ones their state holds: a row here that is live is replaced or disputed by
/// no link of a file.
fn write_links<S: Seat>(seat: &mut S, prepared: &Prepared, ledger: &mut Ledger) -> Result<(), S::Error> {
    let id_of = |src: i64| ledger.placed.get(&src).map(|p| p.id());
    let absent = |src: i64| ledger.absent.contains(&src);
    let mut dropped = Vec::new();

    for row in &prepared.rows {
        let Some(placement) = ledger.placed.get(&row.src.id).copied() else { continue };
        if !placement.owned() {
            continue;
        }
        let id = placement.id();
        if let Some(by) = row.src.superseded_by {
            match id_of(by) {
                Some(by) => seat.link(Link::SupersededBy { id, by })?,
                None if absent(by) => {}
                None => dropped.push(Invalid {
                    line: row.line,
                    reason: format!("memory {} is superseded by {by}, which is not in the file", row.src.id),
                }),
            }
        }
        if let Some(from) = row.src.revived_from {
            match id_of(from) {
                Some(from) => seat.link(Link::RevivedFrom { id, from })?,
                None => dropped.push(Invalid {
                    line: row.line,
                    reason: format!("memory {} was revived by deleting {from}, which is not in the file", row.src.id),
                }),
            }
        }
        let mut against = Vec::new();
        for other in &row.src.disputed_against {
            match id_of(*other) {
                Some(other) => against.push(other),
                None if absent(*other) => {}
                None => dropped.push(Invalid {
                    line: row.line,
                    reason: format!("memory {} is disputed against {other}, which is not in the file", row.src.id),
                }),
            }
        }
        if !against.is_empty() {
            seat.link(Link::DisputedAgainst { id, against: &against })?;
        } else if placement.inserted() {
            seat.link(Link::Undisputed(id))?;
        }
    }

    let owned = |src: i64| ledger.placed.get(&src).is_some_and(|p| p.owned());
    for (line, d) in &prepared.parsed.derivations {
        if !owned(d.memory) {
            continue;
        }
        match (id_of(d.memory), id_of(d.source)) {
            (Some(memory), Some(source)) if memory != source => {
                seat.link(Link::Derivation { memory, source, created_at: d.created_at })?;
            }
            (Some(_), Some(_)) => {}
            _ => dropped.push(Invalid {
                line: *line,
                reason: format!("memory {} is drawn from {}, which is not in the file", d.memory, d.source),
            }),
        }
    }

    for (line, c) in &prepared.parsed.conflicts {
        if !(owned(c.new_id) || owned(c.old_id)) || absent(c.new_id) || absent(c.old_id) {
            continue;
        }
        match (id_of(c.new_id), id_of(c.old_id)) {
            (Some(a), Some(b)) if a != b => {
                // A pair is named by its larger id, as the detector writes it,
                // and an end already here under an older id can turn the
                // file's order around.
                seat.link(Link::Conflict { new_id: a.max(b), old_id: a.min(b), pair: c })?;
            }
            (Some(_), Some(_)) => {}
            _ => dropped.push(Invalid {
                line: *line,
                reason: format!("the pair {}-{} names a row that is not in the file", c.new_id, c.old_id),
            }),
        }
    }

    let mut saves: BTreeMap<i64, Vec<(usize, &PieceLine)>> = BTreeMap::new();
    for (line, p) in &prepared.parsed.pieces {
        saves.entry(p.save).or_default().push((*line, p));
    }
    // A note of another tool shares a row with another note as two saves of
    // one text do, which the worker lets be a piece of both: only the rows of
    // its own origin tell whether its pieces were written.
    let notes = !prepared.notes.is_empty();
    for pieces in saves.values() {
        if !pieces.iter().any(|(_, p)| owned(p.memory)) || pieces.iter().any(|(_, p)| absent(p.memory)) {
            continue;
        }
        let placed: Vec<i64> = pieces.iter().filter(|(_, p)| !notes || owned(p.memory)).filter_map(|(_, p)| id_of(p.memory)).collect();
        let mut linked = false;
        for id in &placed {
            linked |= seat.linked(*id)?;
        }
        if linked {
            continue;
        }
        let mut stored = Vec::with_capacity(pieces.len());
        for (line, p) in pieces {
            match id_of(p.memory) {
                Some(id) => stored.push((p.piece, id, p.context.clone())),
                None => dropped.push(Invalid {
                    line: *line,
                    reason: format!("piece {} is memory {}, which is not in the file", p.piece, p.memory),
                }),
            }
        }
        seat.link(Link::Save(&stored))?;
    }

    ledger.dropped.extend(dropped);
    Ok(())
}

/// Reads `text` and imports it into `store`, writing the `pre-import`
/// snapshot into `backups` first, each step logged in `timings` as it ends.
/// A dry run writes nothing and needs no embedder.
pub async fn import(
    store: &MemoryStore,
    embedder: Option<&dyn Embedder>,
    chunker: Arc<ChatChunker>,
    backups: &Path,
    text: &str,
    options: &ImportOptions,
    ground: Arc<dyn Ground>,
    timings: &mut ImportTimings,
) -> Result<ImportReport, ImportError> {
    let prepared = prepare_file(store, text, options, ground, timings).await?;
    let written = write(store, embedder, &chunker, backups, &prepared, options.dry_run, "clawg", timings).await?;
    Ok(report(&prepared, &written, options.dry_run))
}

/// A clawg export, worked out against the ids its roots would get here.
async fn prepare_file(
    store: &MemoryStore,
    text: &str,
    options: &ImportOptions,
    ground: Arc<dyn Ground>,
    timings: &mut ImportTimings,
) -> Result<Arc<Prepared>, ImportError> {
    let clock = Instant::now();
    let (_, parsed) = parse(text)?;
    let projects = projects_of(&parsed);
    let roots = roots_here(&projects, options);
    let parsing = clock.elapsed();
    let planned: Vec<PathBuf> = roots.iter().map(|(root, _, _)| root.clone()).collect();
    let paths: Vec<(usize, PathBuf)> = roots.iter().enumerate().filter_map(|(k, (_, _, path))| Some((k, path.clone()?))).collect();
    let planning = plan(store, &planned, &paths, ground, timings).await?;
    let clock = Instant::now();
    let row_projects: BTreeSet<&str> = parsed.memories.iter().filter_map(|(_, m)| m.project_id.as_deref()).collect();
    let settled = settle_projects(&projects, &roots, &planning, &row_projects, TimestampMs::now().as_millis_i64());
    let prepared = prepare(parsed, settled, options, Texts::Raw);
    timings.record(Timing {
        elapsed: parsing + clock.elapsed(),
        rows: Some(prepared.rows.len()),
        ..Timing::new(Step::Prepare)
    });
    Ok(Arc::new(prepared))
}

/// Imports the notes of another tool's export as [`import`] imports a file:
/// each note masked whole and split by `chunker` into the rows a save of its
/// text would store, under the agent of `options` or [`MIGRATED_AGENT`], in
/// the project a registration gives the root `options.project_map` names for
/// its project, or in none.
pub async fn import_foreign(
    store: &MemoryStore,
    embedder: Option<&dyn Embedder>,
    chunker: Arc<ChatChunker>,
    backups: &Path,
    export: ForeignExport,
    options: &ImportOptions,
    ground: Arc<dyn Ground>,
    timings: &mut ImportTimings,
) -> Result<ImportReport, ImportError> {
    let source = export.source;
    let (prepared, foreign) = prepare_export(store, chunker.clone(), export, options, ground, timings).await?;
    let written = write(store, embedder, &chunker, backups, &prepared, options.dry_run, source, timings).await?;
    Ok(with_foreign(report(&prepared, &written, options.dry_run), foreign, &prepared.notes, &written.ledger))
}

/// Another tool's export translated into rows, with what the translation
/// counted.
async fn prepare_export(
    store: &MemoryStore,
    chunker: Arc<ChatChunker>,
    export: ForeignExport,
    options: &ImportOptions,
    ground: Arc<dyn Ground>,
    timings: &mut ImportTimings,
) -> Result<(Arc<Prepared>, ForeignReport), ImportError> {
    // Planned before the translation, which tells keys apart by the project
    // each note goes to here.
    let mut seen = HashSet::new();
    let mapped: Vec<&str> =
        options.project_map.iter().filter_map(|(_, target)| target.root()).filter(|root| seen.insert(*root)).collect();
    let roots: Vec<PathBuf> = mapped.iter().map(PathBuf::from).collect();
    let index: HashMap<&str, usize> = mapped.iter().enumerate().map(|(k, root)| (*root, k)).collect();
    let asks: Vec<(usize, PathBuf)> = options
        .project_map
        .iter()
        .filter_map(|(_, target)| Some((index[target.root()?], PathBuf::from(target.path()?))))
        .collect();
    let planning = plan(store, &roots, &asks, ground, timings).await?;
    let mut declared = planning.declared.iter();
    let project_map = options
        .project_map
        .iter()
        .map(|(name, target)| {
            let project_id = match target {
                MapTarget::Project { project } => project.clone(),
                _ => {
                    let repository = &planning.plans[index[target.root().unwrap_or_default()]].project_id;
                    let sub = target.path().and_then(|_| declared.next()).cloned().flatten();
                    sub.unwrap_or_else(|| repository.clone())
                }
            };
            MappedProject { name: name.clone(), root: target.root().map(str::to_owned), project_id }
        })
        .collect();
    let mapped_to: Vec<String> =
        options.project_map.iter().filter_map(|(_, t)| if let MapTarget::Project { project } = t { Some(project.clone()) } else { None }).collect();
    let plans = planning.plans;
    let translate_options = TranslateOptions { agent: options.agent.clone().unwrap_or_else(|| MIGRATED_AGENT.to_owned()), project_map };
    let now = TimestampMs::now().as_millis_i64();
    let clock = Instant::now();
    // Masking and splitting thousands of notes is work the daemon's other
    // requests should not wait behind.
    let (prepared, foreign) = tokio::task::spawn_blocking(move || {
        let Translated { parsed, notes, report } = translate(&export, &translate_options, &chunker, now);
        let settled = mapped_projects(&roots, &plans);
        let mut prepared = prepare(parsed, settled, &ImportOptions::default(), Texts::Notes(notes));
        prepared.mapped_to = mapped_to;
        (prepared, report)
    })
    .await
    .map_err(|e| MemoryError::Internal(format!("could not translate the export: {e}")))?;
    timings.record(Timing { elapsed: clock.elapsed(), rows: Some(prepared.rows.len()), ..Timing::new(Step::Translate) });
    Ok((Arc::new(prepared), foreign))
}

/// The batches the rows are written in, one transaction each, from the rows
/// of each unit in order: whole units up to `limit` rows, or one unit alone
/// when it has more. A cut between two transactions leaves whole notes, each
/// with what its revision replaces, which the next import finds here.
fn note_batches(units: &[usize], limit: usize) -> Vec<Range<usize>> {
    let mut batches = Vec::new();
    let (mut start, mut end) = (0, 0);
    for &len in units {
        if end > start && end - start + len > limit {
            batches.push(start..end);
            start = end;
        }
        end += len;
    }
    if end > start {
        batches.push(start..end);
    }
    batches
}

/// Where every row went, and what the writing left behind.
struct Written {
    ledger: Ledger,
    /// The `pre-import` snapshot.
    snapshot: Option<String>,
    /// The import's number in `memory_import`, when it wrote anything.
    import_id: Option<i64>,
    /// The deleted projects a live row brought back, or would on a dry run.
    revived: BTreeSet<String>,
}

/// Places every row and link of `prepared` and, unless this is a dry run,
/// embeds the rows to insert and writes them after a `pre-import` snapshot,
/// journaled as an import from `source`.
async fn write(
    store: &MemoryStore,
    embedder: Option<&dyn Embedder>,
    chunker: &Arc<ChatChunker>,
    backups: &Path,
    prepared: &Arc<Prepared>,
    dry_run: bool,
    source: &'static str,
    timings: &mut ImportTimings,
) -> Result<Written, ImportError> {
    let now = TimestampMs::now().as_millis_i64();
    let (clock, mut timing) = (Instant::now(), Timing::new(Step::Load));
    let planned = load(store, prepared, &mut timing).await?;
    timings.record(Timing { elapsed: clock.elapsed(), ..timing });

    let clock = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));
    let _stop = StopOnDrop(stop.clone());
    let decide_from = prepared.clone();
    let decided = tokio::task::spawn_blocking(move || decide(planned, &decide_from, &stop))
        .await
        .map_err(|e| MemoryError::Internal(format!("could not plan the import: {e}")))?
        .ok_or_else(|| MemoryError::Internal("the import was stopped".to_owned()))?;
    timings.record(Timing { elapsed: clock.elapsed(), rows: Some(prepared.rows.len()), ..Timing::new(Step::Decide) });

    if dry_run {
        let revived = prepared
            .rows
            .iter()
            .filter(|r| r.src.deleted_at.is_none() && decided.placed.get(&r.src.id).is_some_and(|p| p.inserted()))
            .filter_map(|r| r.project.clone())
            .filter(|p| prepared.deleted.contains(p))
            .collect();
        return Ok(Written { ledger: decided, snapshot: None, import_id: None, revived });
    }
    let embedder = embedder.ok_or(ImportError::NoEmbedder)?;

    let picked = rows_to_embed(prepared, &decided);
    let (clock, mut timing) = (Instant::now(), Timing { rows: Some(picked.len()), ..Timing::new(Step::Embed) });
    let order = embedding_order(prepared, picked, chunker).await?;
    let mut vectors: HashMap<i64, Vec<u8>> = HashMap::with_capacity(order.len());
    for batch in order.chunks(EMBED_BATCH) {
        let asked = Instant::now();
        let rows: Vec<&Row> = batch.iter().map(|&k| &prepared.rows[k]).collect();
        let texts: Vec<String> = rows.iter().map(|r| embedding_text(&r.context, &r.content).into_owned()).collect();
        let embedded = embedder.embed_batch(&texts, EmbedRole::Passage).await?;
        for (row, vector) in rows.iter().zip(embedded) {
            let unit = to_unit(&vector).map_err(|e| EmbedError::InferenceFailed(e.to_string()))?;
            vectors.insert(row.src.id, unit.iter().flat_map(|f| f.to_le_bytes()).collect());
        }
        timing.batch(asked.elapsed());
    }
    timings.record(Timing { elapsed: clock.elapsed(), ..timing });

    let clock = Instant::now();
    let snapshot = store.snapshot(backups.to_path_buf(), Reason::PreImport).await?.display().to_string();
    timings.record(Timing { elapsed: clock.elapsed(), ..Timing::new(Step::Snapshot) });

    let vectors = Arc::new(vectors);
    let mut ledger = Ledger::default();
    // A transaction a cut leaves unwritten journals nothing, and the number
    // is the first write's, so an import that adds nothing has none.
    let mut journal = Journal::new(store, source, now, Some(snapshot.clone()));
    let mut batches = note_batches(&prepared.units(), WRITE_BATCH);
    if batches.is_empty() {
        // The projects are recorded even when there is no row.
        batches.push(0..0);
    }
    let (clock, mut timing) = (Instant::now(), Timing { rows: Some(prepared.rows.len()), ..Timing::new(Step::Write) });
    for (k, range) in batches.into_iter().enumerate() {
        let (batch, vectors) = (prepared.clone(), vectors.clone());
        let mut placed = std::mem::take(&mut ledger);
        let mut logged = journal;
        let written = timed(store, &mut timing, move |c| {
            let tx = c.transaction()?;
            if k == 0 {
                // A project the map names that was deleted since the daemon
                // resolved it would come back under the rows.
                for id in &batch.mapped_to {
                    if let Some(deleted) = deleted_in(&tx, &canonical_in(&tx, id)?)? {
                        return Ok(Err((id.clone(), deleted.delete_id)));
                    }
                }
                placed.taken = record_projects(&tx, &batch, now, &mut logged)?;
            }
            place_rows(&mut SqlSeat(&tx, &mut logged), &batch, range, &mut placed, &vectors, true)?;
            tx.commit()?;
            Ok(Ok((placed, logged)))
        })
        .await?;
        (ledger, journal) = written.map_err(|(id, delete_id)| ImportError::ProjectDeleted { id, delete_id })?;
    }
    timings.record(Timing { elapsed: clock.elapsed(), ..timing });

    let (clock, mut timing) = (Instant::now(), Timing::new(Step::Links));
    let links = prepared.clone();
    let (ledger, mut journal) = timed(store, &mut timing, move |c| {
        let tx = c.transaction()?;
        write_links(&mut SqlSeat(&tx, &mut journal), &links, &mut ledger)?;
        if let Some(id) = journal.id {
            tx.execute("UPDATE memory_import SET finished_at = ?2 WHERE import_id = ?1", [id, TimestampMs::now().as_millis_i64()])?;
        }
        tx.commit()?;
        Ok((ledger, journal))
    })
    .await?;
    timings.record(Timing { elapsed: clock.elapsed(), ..timing });

    let revived = std::mem::take(&mut journal.revived);
    Ok(Written { ledger, snapshot: Some(snapshot), import_id: journal.id, revived })
}

/// The indices in `prepared.rows` of the rows the import inserts and are not
/// deleted, the only ones it embeds, in the order of the file.
fn rows_to_embed(prepared: &Prepared, decided: &Ledger) -> Vec<usize> {
    (0..prepared.rows.len())
        .filter(|&k| {
            let row = &prepared.rows[k];
            row.src.deleted_at.is_none() && decided.placed.get(&row.src.id).is_some_and(|p| p.inserted())
        })
        .collect()
}

/// `picked`, rows of `prepared`, from the one with the most tokens to embed
/// to the one with the fewest, so the rows the embedder pads together are
/// about as long as each other. Counted by the chunker, whose tokenizer is
/// the embedder's, in a blocking task and `COUNT_BATCH` texts at a time.
async fn embedding_order(prepared: &Arc<Prepared>, picked: Vec<usize>, chunker: &Arc<ChatChunker>) -> Result<Vec<usize>, MemoryError> {
    let (prepared, chunker) = (prepared.clone(), chunker.clone());
    tokio::task::spawn_blocking(move || {
        let mut tokens = Vec::with_capacity(picked.len());
        for group in picked.chunks(COUNT_BATCH) {
            let texts: Vec<_> = group.iter().map(|&k| embedding_text(&prepared.rows[k].context, &prepared.rows[k].content)).collect();
            let refs: Vec<&str> = texts.iter().map(|t| &**t).collect();
            tokens.extend(chunker.count_all(&refs));
        }
        longest_first(&tokens).into_iter().map(|k| picked[k]).collect()
    })
    .await
    .map_err(|e| MemoryError::Internal(format!("could not order the rows to embed: {e}")))
}

/// The indices of `tokens` from the largest count to the smallest, equal
/// counts in the order they came.
fn longest_first(tokens: &[usize]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..tokens.len()).collect();
    order.sort_by_key(|&k| Reverse(tokens[k]));
    order
}

fn report(prepared: &Prepared, written: &Written, dry_run: bool) -> ImportReport {
    let ledger = &written.ledger;
    let mut report = ImportReport {
        dry_run,
        rows: prepared.rows.len(),
        invalid: prepared.parsed.invalid.clone(),
        snapshot: written.snapshot.clone(),
        import_id: written.import_id,
        projects: prepared.placed.clone(),
        suggestions: prepared.suggestions.clone(),
        subfolders: prepared.subfolders.clone(),
        ..Default::default()
    };
    for project in &mut report.projects {
        project.revived = project.to.as_ref().is_some_and(|to| written.revived.contains(to));
    }
    report.invalid.extend(ledger.invalid.iter().cloned());
    report.invalid.sort_by_key(|i| i.line);
    report.dropped_links = ledger.dropped.clone();
    report.dropped_links.sort_by_key(|i| i.line);
    let skipped: HashSet<usize> = report.invalid.iter().map(|i| i.line).collect();
    for row in &prepared.rows {
        if skipped.contains(&row.line) {
            continue;
        }
        match ledger.placed.get(&row.src.id) {
            Some(Placement::Present(_)) => report.present += 1,
            Some(Placement::Elsewhere(id)) => {
                let at = ledger.elsewhere.get(id).cloned().flatten().unwrap_or_default();
                *report.present_elsewhere.entry(at).or_default() += 1;
            }
            Some(Placement::Merged(into)) => report.merged.push(Merged { line: row.line, id: row.src.id, into: *into }),
            Some(placement @ (Placement::Inserted(_) | Placement::TopicTaken { .. } | Placement::Older { .. })) => {
                let (project, key) = target(row, prepared.found(row, ledger));
                report.inserted += 1;
                *report.by_type.entry(row.src.memory_type.clone()).or_default() += 1;
                let state = if matches!(placement, Placement::Inserted(_)) { row.state() } else { "superseded" };
                *report.by_state.entry(state.to_string()).or_default() += 1;
                *report.by_project.entry(project.unwrap_or_default().to_owned()).or_default() += 1;
                *report.by_lang.entry(row.lang.to_string()).or_default() += 1;
                report.redacted += usize::from(row.redacted);
                if let Placement::TopicTaken { id, by } = placement {
                    report.topic_taken.push(TopicTaken {
                        line: row.line,
                        id: row.src.id,
                        topic_key: key.unwrap_or_default().to_owned(),
                        stored_as: *id,
                        by: *by,
                    });
                }
            }
            None => {}
        }
    }
    report
}

/// Tells what the import did with the rows of a translated export by the
/// note each came from, since the translation's lines and ids mean nothing
/// outside it.
fn with_foreign(mut report: ImportReport, mut foreign: ForeignReport, notes: &[NoteRows], ledger: &Ledger) -> ImportReport {
    let note_of: HashMap<i64, &NoteRows> = notes.iter().flat_map(|n| n.rows.clone().map(move |id| (id, n))).collect();
    for merged in std::mem::take(&mut report.merged) {
        if let Some(note) = note_of.get(&merged.id) {
            let piece = merged.id - note.rows.start + 1;
            foreign.merged.push(MergedNote { origin: note.origin.clone(), piece, into: merged.into });
        }
    }
    let mut named = HashSet::new();
    for taken in std::mem::take(&mut report.topic_taken) {
        match note_of.get(&taken.id) {
            Some(note) if named.insert(note.origin.as_str()) => foreign.topic_taken.push(TakenKey {
                origin: note.origin.clone(),
                topic_key: taken.topic_key,
                by: taken.by,
            }),
            _ => {}
        }
    }
    let mut named = HashSet::new();
    for invalid in std::mem::take(&mut report.invalid) {
        let Some(note) = note_of.get(&(invalid.line as i64)) else { continue };
        // Told by project below.
        if note.project_id.as_ref().is_some_and(|id| ledger.taken.contains_key(id)) {
            continue;
        }
        if named.insert(note.origin.as_str()) {
            foreign.left_out.push(LeftOut { origin: note.origin.clone(), reason: invalid.reason });
        }
    }
    foreign.taken_projects = foreign
        .by_source_project
        .iter()
        .filter_map(|p| {
            let belongs_to = ledger.taken.get(p.project_id.as_ref()?)?;
            Some(TakenProject {
                project: p.project.clone()?,
                root: p.root.clone()?,
                belongs_to: belongs_to.clone(),
                notes: p.notes,
            })
        })
        .collect();
    for note in notes {
        match ledger.notes.get(&note.rows.start) {
            Some(Found::Present { project, kept, .. }) => {
                *foreign.present.entry(project.clone().unwrap_or_default()).or_default() += 1;
                if *kept {
                    foreign.deleted_at_source.push(note.origin.clone());
                }
            }
            Some(Found::Revision { newer: true, .. }) => foreign.revised.push(note.origin.clone()),
            Some(Found::Revision { newer: false, .. }) => foreign.older_versions.push(note.origin.clone()),
            Some(Found::New) | None => {}
        }
    }
    report.foreign = Some(foreign);
    report
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::memory::admin::{delete_memory_core, restore_memory_core, MemoryKind};
    use crate::memory::db::seed_on_disk;
    use crate::memory::embedder::FakeEmbedder;
    use crate::memory::events::{MemoryEvent, MemorySource};
    use crate::memory::portability::export::{read, Export, ExportFilter};
    use crate::memory::identity::{Disk, FakeGround, RootCommit};
    use crate::memory::portability::fixtures::{other_project, project, seed, OTHER_ROOT, ROOT};
    use crate::memory::portability::foreign::{ForeignNote, ForeignRelation, NoteKind, When};
    use crate::memory::portability::format::tests::EXAMPLE;
    use crate::memory::portability::translate::tests::{export as foreign_export, note as foreign_note, T0};
    use crate::memory::portability::translate::{rewrite_key, KeyChange, ProjectCount, TakenKey, TakenProject, TypeCount};
    use crate::memory::relocate::{move_memories_core, MoveFilter};
    use crate::memory::search::{search_all_memories, SearchScope};
    use crate::memory::worker::save_for_test;
    use crate::models::chat::Role;

    pub(crate) const EVERYTHING: ExportFilter = ExportFilter { project_id: None, agent: None, include_deleted: true };

    pub(crate) async fn dump(store: &MemoryStore, filter: ExportFilter) -> Export {
        store.conn.call(move |c| Ok::<_, rusqlite::Error>(read(c, &filter, 0))).await.unwrap().unwrap()
    }

    pub(crate) async fn seeded_file() -> String {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed(&store).await;
        dump(&store, EVERYTHING).await.to_jsonl()
    }

    /// An empty store, but for one row of its own, so the ids the import
    /// assigns are not the file's by accident.
    pub(crate) async fn destination() -> MemoryStore {
        let store = MemoryStore::new_for_test().await.unwrap();
        insert_row(&store, "claude-code", None, "A row that was here first", None, 1).await;
        store
    }

    pub(crate) async fn insert_row(
        store: &MemoryStore,
        agent: &'static str,
        project: Option<String>,
        content: &'static str,
        topic: Option<&'static str>,
        created_at: i64,
    ) -> i64 {
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory (scope, agent_id, project_id, content, memory_type, source, created_at, \
                     created_by_type, created_by_ref, last_modified_at, last_accessed_at, access_count, importance, \
                     tier, status, content_hash, topic_key) \
                     VALUES ('agent', ?1, ?2, ?3, 'semantic_fact', 'explicit', ?4, 'agent', ?1, ?4, ?4, 1, 5.0, 3, \
                     'active', content_hash(?3), ?5)",
                    rusqlite::params![agent, project, content, created_at, topic],
                )?;
                Ok::<_, rusqlite::Error>(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    pub(crate) async fn run(store: &MemoryStore, text: &str, options: ImportOptions) -> (ImportReport, crate::test_dir::TestDir) {
        run_on(store, text, options, Arc::new(Disk)).await
    }

    /// [`run`] with what `ground` finds at each root.
    async fn run_on(store: &MemoryStore, text: &str, options: ImportOptions, ground: Arc<dyn Ground>) -> (ImportReport, crate::test_dir::TestDir) {
        let backups = crate::test_dir::test_dir();
        let report = import(store, Some(&FakeEmbedder::default()), Arc::new(ChatChunker::with_fallback()), backups.path(), text, &options, ground, &mut ImportTimings::new(false))
            .await
            .unwrap();
        (report, backups)
    }

    pub(crate) async fn count(store: &MemoryStore, sql: &'static str) -> i64 {
        store.conn.call(move |c| c.query_row(sql, [], |r| r.get::<_, i64>(0))).await.unwrap()
    }

    fn by_content(export: &Export) -> HashMap<String, MemoryLine> {
        export.memories.iter().map(|m| (m.content.clone(), m.clone())).collect()
    }

    fn files_in(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|d| d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn a_round_trip_keeps_rows_states_and_links() {
        let source = MemoryStore::new_for_test().await.unwrap();
        seed(&source).await;
        let before = dump(&source, EVERYTHING).await;
        let dst = destination().await;
        let (report, backups) = run(&dst, &before.to_jsonl(), ImportOptions::default()).await;

        assert!(report.invalid.is_empty() && report.dropped_links.is_empty(), "{report:#?}");
        assert_eq!((report.rows, report.inserted, report.present), (17, 17, 0));
        assert_eq!(report.by_state, BTreeMap::from([("deleted".into(), 2), ("live".into(), 13), ("superseded".into(), 2)]));
        assert_eq!(report.by_type["semantic_fact"], 10);
        assert_eq!(report.by_project, BTreeMap::from([("".into(), 3), (other_project(), 1), (project(), 13)]));
        let snapshot = report.snapshot.clone().unwrap();
        assert!(snapshot.ends_with("-pre-import.db") && files_in(backups.path()).len() == 1, "{snapshot}");

        let after = dump(&dst, EVERYTHING).await;
        let (old, new) = (by_content(&before), by_content(&after));
        let old_content = |id: i64| before.memories.iter().find(|m| m.id == id).unwrap().content.clone();
        let new_id = |id: i64| new[&old_content(id)].id;
        for (content, o) in &old {
            let n = &new[content];
            assert_ne!(n.id, o.id, "ids are assigned here");
            assert_eq!(
                (&n.scope, &n.agent_id, &n.project_id, &n.memory_type, &n.source, &n.created_by_type, &n.status),
                (&o.scope, &o.agent_id, &o.project_id, &o.memory_type, &o.source, &o.created_by_type, &o.status),
                "{content}"
            );
            assert_eq!(
                (n.created_at, n.last_accessed_at, n.last_modified_at, n.access_count, n.tier, n.update_count, n.deleted_at),
                (o.created_at, o.last_accessed_at, o.last_modified_at, o.access_count, o.tier, o.update_count, o.deleted_at),
                "{content}"
            );
            assert_eq!((n.importance, &n.topic_key, &n.supersede_cause, &n.source_session_id), (o.importance, &o.topic_key, &o.supersede_cause, &o.source_session_id));
            assert_eq!(n.superseded_by, o.superseded_by.map(new_id), "{content}");
            assert_eq!(n.revived_from, o.revived_from.map(new_id), "{content}");
            assert_eq!(n.disputed_against, o.disputed_against.iter().map(|id| new_id(*id)).collect::<Vec<_>>());
        }
        let pairs = |e: &Export, content: &dyn Fn(i64) -> String| {
            let mut p: Vec<_> = e.conflicts.iter().map(|c| (content(c.new_id), content(c.old_id), c.state.clone(), c.verdict.clone(), c.reason.clone())).collect();
            p.sort();
            p
        };
        let new_content = |id: i64| after.memories.iter().find(|m| m.id == id).unwrap().content.clone();
        assert_eq!(pairs(&before, &old_content), pairs(&after, &new_content), "the pairs travel and the detector adds none");
        assert_eq!(after.derivations.iter().map(|d| (new_content(d.memory), new_content(d.source))).collect::<Vec<_>>(),
                   before.derivations.iter().map(|d| (old_content(d.memory), old_content(d.source))).collect::<Vec<_>>());
        assert_eq!(after.pieces.iter().map(|p| (p.piece, new_content(p.memory), p.context.clone())).collect::<Vec<_>>(),
                   before.pieces.iter().map(|p| (p.piece, old_content(p.memory), p.context.clone())).collect::<Vec<_>>());
        assert_eq!(after.projects.len(), 2);
        assert_eq!(count(&dst, "SELECT count(*) FROM memory_vectors").await, 15, "every imported row but the deleted has a vector");

        let fake = FakeEmbedder::default();
        for m in after.memories.iter().filter(|m| m.deleted_at.is_none() && m.status != "superseded" && m.content != "A row that was here first") {
            let context = after.pieces.iter().find(|p| p.memory == m.id).map(|p| p.context.clone()).unwrap_or_default();
            let vector = fake.embed_with_role(&embedding_text(&context, &m.content), EmbedRole::Passage).await.unwrap();
            let agent = m.agent_id.clone().unwrap_or_else(|| "claude-code".into());
            let hits = search_all_memories(&dst, &vector, None, &agent, SearchScope::all_default().with_project(m.project_id.as_deref()), 50)
                .await
                .unwrap();
            assert!(hits.iter().any(|h| h.id == m.id), "{} is not found", m.content);
        }
    }

    #[tokio::test]
    async fn the_export_carries_the_warning_mark_and_the_import_keeps_it() {
        let source = MemoryStore::new_for_test().await.unwrap();
        seed(&source).await;
        source
            .conn
            .call(|c| c.execute("UPDATE memory_conflict SET topic_key_warned = 1 WHERE new_id = 13 AND old_id = 12", []))
            .await
            .unwrap();
        let before = dump(&source, EVERYTHING).await;
        let marked: Vec<(i64, i64)> =
            before.conflicts.iter().filter(|c| c.topic_key_warned).map(|c| (c.new_id, c.old_id)).collect();
        assert_eq!(marked, vec![(13, 12)]);
        let file = before.to_jsonl();
        assert!(file.contains("\"topic_key_warned\":true"), "{file}");

        let dst = destination().await;
        run(&dst, &file, ImportOptions::default()).await;

        let after = dump(&dst, EVERYTHING).await;
        let by_text = |e: &Export| {
            let content = |id: i64| e.memories.iter().find(|m| m.id == id).unwrap().content.clone();
            let mut p: Vec<(String, String, bool)> =
                e.conflicts.iter().map(|c| (content(c.new_id), content(c.old_id), c.topic_key_warned)).collect();
            p.sort();
            p
        };
        assert_eq!(by_text(&after), by_text(&before));
    }

    #[tokio::test]
    async fn a_second_import_adds_nothing() {
        let file = seeded_file().await;
        let dst = destination().await;
        run(&dst, &file, ImportOptions::default()).await;
        let first = dump(&dst, EVERYTHING).await;
        let (again, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!((again.inserted, again.present), (0, 17), "{again:#?}");
        assert!(again.merged.is_empty() && again.invalid.is_empty() && again.dropped_links.is_empty(), "{again:#?}");
        let second = dump(&dst, EVERYTHING).await;
        assert_eq!(first.memories, second.memories, "no row added and no access counted");
        assert_eq!((first.pieces, first.derivations, first.conflicts), (second.pieces, second.derivations, second.conflicts));
    }

    #[tokio::test]
    async fn links_lost_between_transactions_are_filled_by_importing_again() {
        let file = seeded_file().await;
        let dst = destination().await;
        run(&dst, &file, ImportOptions::default()).await;
        let whole = dump(&dst, EVERYTHING).await;
        dst.conn
            .call(|c| {
                c.execute_batch(
                    "UPDATE memory SET superseded_by = NULL, revived_from = NULL, disputed_against = NULL;
                     DELETE FROM memory_piece; DELETE FROM memory_derivation; DELETE FROM memory_conflict;",
                )
            })
            .await
            .unwrap();
        let (again, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!(again.present, 17);
        assert_eq!(dump(&dst, EVERYTHING).await, whole);

        // Notes of another tool, whose rows are found by their origin.
        let dst = destination().await;
        run_foreign(&dst, sample_notes(), ImportOptions::default()).await;
        let whole = dump(&dst, EVERYTHING).await;
        assert!(!whole.pieces.is_empty() && whole.memories.iter().any(|m| m.superseded_by.is_some()));
        dst.conn
            .call(|c| c.execute_batch("UPDATE memory SET superseded_by = NULL; DELETE FROM memory_piece;"))
            .await
            .unwrap();
        let (again, _) = run_foreign(&dst, sample_notes(), ImportOptions::default()).await;
        assert_eq!((again.inserted, again.present), (0, again.rows), "{again:#?}");
        assert_eq!(dump(&dst, EVERYTHING).await, whole);
    }

    #[tokio::test]
    async fn an_unknown_version_writes_nothing() {
        let file = seeded_file().await.replacen("\"version\":1", "\"version\":2", 1);
        let dst = destination().await;
        let backups = crate::test_dir::test_dir();
        let err = import(&dst, Some(&FakeEmbedder::default()), Arc::new(ChatChunker::with_fallback()), backups.path(), &file, &ImportOptions::default(), Arc::new(Disk), &mut ImportTimings::new(false)).await.unwrap_err();
        assert!(matches!(err, ImportError::Format(FormatError::UnknownVersion(2))), "{err}");
        assert_eq!(count(&dst, "SELECT count(*) FROM memory").await, 1);
        assert!(files_in(backups.path()).is_empty());
    }

    #[tokio::test]
    async fn a_dry_run_reports_without_writing() {
        let file = seeded_file().await;
        let dst = destination().await;
        let backups = crate::test_dir::test_dir();
        let sequence = "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'memory'), 0)";
        let before = count(&dst, sequence).await;
        let dry = import(&dst, None, Arc::new(ChatChunker::with_fallback()), backups.path(), &file, &ImportOptions { dry_run: true, ..Default::default() }, Arc::new(Disk), &mut ImportTimings::new(false)).await.unwrap();
        assert_eq!(count(&dst, "SELECT count(*) FROM memory").await, 1);
        assert_eq!(count(&dst, "SELECT count(*) FROM project_path").await, 0);
        assert_eq!(count(&dst, "SELECT count(*) FROM project_remote").await, 0);
        assert_eq!(count(&dst, "SELECT count(*) FROM memory_fts").await, 1);
        assert_eq!(count(&dst, sequence).await, before);
        assert!(files_in(backups.path()).is_empty());
        let (real, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!(ImportReport { dry_run: false, snapshot: real.snapshot.clone(), import_id: real.import_id, ..dry }, real);
    }

    /// The journal of import `id`: its header (source, snapshot, finished),
    /// the rows it inserted and its extras by kind.
    async fn journal_of(store: &MemoryStore, id: i64) -> (Option<(String, Option<String>, bool)>, Vec<i64>, BTreeMap<String, usize>) {
        store
            .conn
            .call(move |c| {
                let header = c
                    .query_row(
                        "SELECT source, snapshot, finished_at IS NOT NULL FROM memory_import WHERE import_id = ?1",
                        [id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let mut stmt = c.prepare("SELECT row_id FROM memory_import_row WHERE import_id = ?1 ORDER BY row_id")?;
                let rows = stmt.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
                let mut stmt = c.prepare("SELECT kind, COUNT(*) FROM memory_import_extra WHERE import_id = ?1 GROUP BY kind")?;
                let extras = stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
                Ok::<_, rusqlite::Error>((header, rows, extras))
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn an_import_is_numbered_and_journaled() {
        let file = seeded_file().await;
        let dst = destination().await;
        let (report, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!(report.import_id, Some(1), "{report:#?}");
        let (header, rows, extras) = journal_of(&dst, 1).await;
        assert_eq!(header, Some(("clawg".to_owned(), report.snapshot.clone(), true)));
        let inserted: Vec<i64> = dump(&dst, EVERYTHING).await.memories.iter().map(|m| m.id).filter(|id| *id > 1).collect();
        assert_eq!((rows.len(), &rows), (17, &inserted), "every inserted row, and only those");
        let pairs = dump(&dst, EVERYTHING).await.conflicts.len();
        assert_eq!(extras, BTreeMap::from([("pair".to_owned(), pairs), ("project".to_owned(), 2)]), "the links of its own rows go with them");

        let (foreign, _) = run_foreign(&dst, sample_notes(), ImportOptions::default()).await;
        assert_eq!(foreign.import_id, Some(2));
        assert!(dst.importing.lock().unwrap().is_empty(), "an import lets its number go once it ends");
        let (header, rows, _) = journal_of(&dst, 2).await;
        assert_eq!(header.map(|h| h.0), Some("engram".to_owned()));
        assert_eq!(rows.len(), foreign.inserted);
    }

    #[tokio::test]
    async fn a_dry_run_and_an_import_that_adds_nothing_have_no_number() {
        let file = seeded_file().await;
        let dst = destination().await;
        let backups = crate::test_dir::test_dir();
        let dry = import(&dst, None, Arc::new(ChatChunker::with_fallback()), backups.path(), &file, &ImportOptions { dry_run: true, ..Default::default() }, Arc::new(Disk), &mut ImportTimings::new(false)).await.unwrap();
        assert_eq!(dry.import_id, None);
        assert_eq!(count(&dst, "SELECT count(*) FROM memory_import").await, 0);
        let (first, _) = run(&dst, &file, ImportOptions::default()).await;
        let (again, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!((first.import_id, again.import_id), (Some(1), None), "{again:#?}");
        assert_eq!(count(&dst, "SELECT count(*) FROM memory_import").await, 1);
    }

    #[tokio::test]
    async fn without_an_embedder_nothing_is_written() {
        let file = seeded_file().await;
        let dst = destination().await;
        let backups = crate::test_dir::test_dir();
        let err = import(&dst, None, Arc::new(ChatChunker::with_fallback()), backups.path(), &file, &ImportOptions::default(), Arc::new(Disk), &mut ImportTimings::new(false)).await.unwrap_err();
        assert!(matches!(err, ImportError::NoEmbedder));
        assert_eq!(count(&dst, "SELECT count(*) FROM memory").await, 1);
        assert!(files_in(backups.path()).is_empty());
    }

    #[tokio::test]
    async fn imported_text_is_masked_and_indexed_once() {
        let token = format!("ghp_{}", "aB3".repeat(12));
        let file = EXAMPLE
            .replace("Storage engine: SQLite with WAL", &format!("Deploy with token {token} on Fridays"))
            .replace("\"status\":\"superseded\",\"superseded_by\":9", "\"status\":\"active\",\"superseded_by\":null")
            .replace("different scopes", &format!("the token {token} differs"))
            .replace("\"memory\":9,\"context\":\"Storage engine\"", &format!("\"memory\":7,\"context\":\"Deploy with token {token}\""));
        let dst = destination().await;
        let (report, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!((report.inserted, report.redacted), (1, 1), "{report:#?}");
        let row = dump(&dst, EVERYTHING).await.memories.into_iter().find(|m| m.content.contains("Fridays")).unwrap();
        assert!(!row.content.contains(&token) && row.content.contains("[REDACTED:"), "{}", row.content);
        let context = dst.conn.call(|c| c.query_row("SELECT context FROM memory_piece", [], |r| r.get::<_, String>(0))).await.unwrap();
        assert!(!context.contains(&token) && context.contains("[REDACTED:"), "{context}");
        let hash_matches = count(&dst, "SELECT count(*) FROM memory WHERE content_hash = content_hash(content)").await;
        assert_eq!(hash_matches, 2);
        assert_eq!(count(&dst, "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'fridays'").await, 1);
    }

    /// `EXAMPLE` with its one memory live, so the import inserts it.
    fn live_example() -> String {
        EXAMPLE.replace("\"status\":\"superseded\",\"superseded_by\":9", "\"status\":\"active\",\"superseded_by\":null")
    }

    async fn imported_lang(dst: &MemoryStore, needle: &'static str) -> Option<String> {
        dst.conn
            .call(move |c| {
                c.query_row("SELECT lang FROM memory WHERE instr(content, ?1) > 0", [needle], |r| r.get(0))
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn an_import_computes_the_language_whatever_the_file_says() {
        let file = live_example()
            .replace(
                "\"content\":\"Storage engine: SQLite with WAL\"",
                "\"content\":\"El motor de almacenamiento es SQLite con WAL, y se eligió por su única conexión de escritura.\"",
            )
            .replace("\"lang\":\"und\"", "\"lang\":\"en\"");
        let dst = destination().await;
        let (report, _) = run(&dst, &file, ImportOptions::default()).await;
        assert!(report.invalid.is_empty(), "a line with lang parses: {report:#?}");
        assert_eq!(report.inserted, 1);
        assert_eq!(report.by_lang, BTreeMap::from([("es".to_string(), 1)]));
        assert_eq!(imported_lang(&dst, "motor de almacenamiento").await.as_deref(), Some("es"));
    }

    #[tokio::test]
    async fn an_export_without_languages_imports() {
        let dst = destination().await;
        let older = live_example().replacen(",\"lang\":\"und\"", "", 1);
        assert!(!older.contains("\"lang\""));
        let (report, _) = run(&dst, &older, ImportOptions::default()).await;
        assert_eq!((report.inserted, report.invalid.len()), (1, 0), "{report:#?}");
        assert_eq!(imported_lang(&dst, "Storage engine").await.as_deref(), Some("und"), "too few words to tell");
    }

    #[tokio::test]
    async fn an_import_keeps_the_origin_project() {
        let file = live_example()
            .replace("\"source\":\"explicit\"", "\"source\":\"import\"")
            .replace("\"created_by_ref\":\"claude-code\"", "\"created_by_ref\":\"engram:42\"")
            .replace("\"origin_project\":null", "\"origin_project\":\"notes/app\"");
        assert!(file.contains("\"origin_project\":\"notes/app\"") && file.contains("engram:42"));
        let dst = destination().await;
        let (report, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!((report.inserted, report.invalid.len()), (1, 0), "{report:#?}");
        let row: (String, String, Option<String>) = dst
            .conn
            .call(|c| {
                c.query_row(
                    "SELECT source, created_by_ref, origin_project FROM memory WHERE instr(content, 'Storage engine') > 0",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!(row, ("import".to_string(), "engram:42".to_string(), Some("notes/app".to_string())));
    }

    #[tokio::test]
    async fn an_export_without_an_origin_project_imports() {
        let dst = destination().await;
        let older = live_example().replacen(",\"origin_project\":null", "", 1);
        assert!(!older.contains("origin_project"));
        let (report, _) = run(&dst, &older, ImportOptions::default()).await;
        assert_eq!((report.inserted, report.invalid.len()), (1, 0), "{report:#?}");
        let origin: Option<String> = dst
            .conn
            .call(|c| c.query_row("SELECT origin_project FROM memory WHERE instr(content, 'Storage engine') > 0", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(origin, None);
    }

    #[tokio::test]
    async fn a_deleted_row_comes_back_deleted_without_a_vector() {
        let file = seeded_file().await;
        let dst = destination().await;
        run(&dst, &file, ImportOptions::default()).await;
        let deleted = dump(&dst, EVERYTHING).await.memories.into_iter().find(|m| m.content == "Daemon port: 27390").unwrap();
        assert_eq!(deleted.deleted_at, Some(1_756_000_000_000));
        let id = deleted.id;
        let has_vector = dst
            .conn
            .call(move |c| c.query_row("SELECT count(*) FROM memory_vectors WHERE rowid = ?1", [id], |r| r.get::<_, i64>(0)))
            .await
            .unwrap();
        assert_eq!(has_vector, 0);
        assert_eq!(count(&dst, "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH '27390'").await, 0);

        restore_memory_core(&dst, &FakeEmbedder::default(), MemoryKind::Memory, id, None).await.unwrap();
        assert_eq!(count(&dst, "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH '27390'").await, 1, "indexed once");
    }

    #[tokio::test]
    async fn map_root_gives_the_destination_project_id() {
        let file = seeded_file().await;
        let dst = destination().await;
        let elsewhere = "/elsewhere/repo";
        let options = ImportOptions { map_roots: vec![(ROOT.into(), elsewhere.into())], ..Default::default() };
        let (report, _) = run(&dst, &file, options).await;
        let moved = crate::memory::identity::derive_project_id(Path::new(elsewhere));
        assert_eq!(report.by_project.get(&moved), Some(&13));
        let roots = dst
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT id, root_path, index_docs FROM project_path ORDER BY root_path")?;
                let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap();
        assert_eq!(roots, vec![(moved, elsewhere.into(), false), (other_project(), "/work/other".into(), false)]);
    }

    #[tokio::test]
    async fn an_import_gives_a_colliding_root_its_own_id() {
        let file = seeded_file().await;
        let dst = destination().await;
        let id = project();
        dst.conn
            .call(move |c| c.execute("INSERT INTO project_path (id, root_path, created_at) VALUES (?1, '/not/the/repo', 0)", [id]))
            .await
            .unwrap();
        let (report, _) = run(&dst, &file, ImportOptions::default()).await;
        assert!(report.invalid.is_empty(), "{:?}", report.invalid);
        assert_eq!(report.by_project.get(&project()), None, "nothing goes to the other root's project");
        let went = report.projects.iter().find(|p| p.id == project()).unwrap();
        let to = went.to.clone().unwrap();
        assert_eq!((went.by, went.created), (Some(By::New), true), "{went:?}");
        assert_eq!(report.by_project.get(&to), Some(&13));
        let own = crate::memory::identity::candidate_id(Path::new(ROOT), 1);
        let folder: (String, String) = dst
            .conn
            .call(|c| c.query_row("SELECT id, project_id FROM project_path WHERE root_path = ?1", [ROOT], |r| Ok((r.get(0)?, r.get(1)?))))
            .await
            .unwrap();
        assert_eq!(folder, (own, to), "the root gets its own id and leads to the new project");
    }

    /// Answers as the fake embedder does, after leaving `from` a tombstone of
    /// `into` on its first call: the import was planned before that merge.
    struct MergingEmbedder {
        store: MemoryStore,
        from: String,
        into: String,
        merged: std::sync::atomic::AtomicBool,
        fake: FakeEmbedder,
    }

    #[async_trait::async_trait]
    impl Embedder for MergingEmbedder {
        async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            if !self.merged.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let (from, into) = (self.from.clone(), self.into.clone());
                self.store
                    .conn
                    .call(move |c| c.execute("UPDATE project SET merged_into = ?2, merged_at = 1 WHERE id = ?1", [from, into]))
                    .await
                    .unwrap();
            }
            self.fake.embed_with_role(text, role).await
        }
        fn dimensions(&self) -> usize {
            self.fake.dimensions()
        }
        fn model_id(&self) -> &str {
            self.fake.model_id()
        }
        fn query_prefix(&self) -> &str {
            self.fake.query_prefix()
        }
        fn passage_prefix(&self) -> &str {
            self.fake.passage_prefix()
        }
    }

    #[tokio::test]
    async fn an_import_planned_before_a_merge_writes_into_its_target() {
        let file = seeded_file().await;
        let dst = destination().await;
        let (planned, target) = (project(), "web-bbbb".to_string());
        let (p, t) = (planned.clone(), target.clone());
        dst.conn
            .call(move |c| {
                c.execute_batch(&format!(
                    "INSERT INTO project (id, name, born, created_at) VALUES ('{p}', 'repo', 'root', 0), ('{t}', 'web', 'root', 0);"
                ))
            })
            .await
            .unwrap();
        let embedder = MergingEmbedder {
            store: dst.clone(),
            from: planned.clone(),
            into: target.clone(),
            merged: std::sync::atomic::AtomicBool::new(false),
            fake: FakeEmbedder::default(),
        };
        let backups = crate::test_dir::test_dir();

        import(&dst, Some(&embedder), Arc::new(ChatChunker::with_fallback()), backups.path(), &file, &ImportOptions::default(), Arc::new(Disk), &mut ImportTimings::new(false))
            .await
            .unwrap();

        assert!(embedder.merged.load(std::sync::atomic::Ordering::SeqCst), "the merge came between the plan and the writes");
        let rows = dst
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT project_id, COUNT(*) FROM memory GROUP BY project_id ORDER BY 1")?;
                let rows = stmt.query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap();
        assert!(rows.iter().all(|(p, _)| p.as_deref() != Some(planned.as_str())), "nothing under the tombstone: {rows:?}");
        assert!(rows.contains(&(Some(target.clone()), 13)), "{rows:?}");
        let folder: Option<String> = dst
            .conn
            .call(|c| c.query_row("SELECT project_id FROM project_path WHERE root_path = ?1", [ROOT], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(folder, Some(target));
    }

    #[tokio::test]
    async fn an_import_into_a_clone_goes_to_the_project_of_its_remote() {
        let file = seeded_file().await;
        let dst = destination().await;
        dst.conn
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO project_path (id, root_path, created_at) VALUES ('app-1a2b', '/work/app', 0);
                     INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/app', 'app-1a2b', 0);",
                )
            })
            .await
            .unwrap();
        let clone = crate::test_dir::test_dir();
        std::fs::create_dir_all(clone.path().join(".git")).unwrap();
        std::fs::write(clone.path().join(".git").join("config"), "[remote \"origin\"]\n\turl = https://github.com/acme/app\n").unwrap();
        let here = clone.path().to_string_lossy().into_owned();
        let options = ImportOptions { map_roots: vec![(ROOT.into(), here.into())], ..Default::default() };

        let (report, _) = run(&dst, &file, options).await;

        assert_eq!(report.by_project.get("app-1a2b"), Some(&13), "{:?}", report.by_project);
    }

    /// A folder as `project_path` holds it: root, id, project, remote, first
    /// commit and git stamp.
    type FolderRow = (String, String, Option<String>, Option<String>, Option<String>, Option<String>);

    #[tokio::test]
    async fn an_import_records_its_folders_as_a_registration_would() {
        let file = seeded_file().await;
        let dst = destination().await;
        let (old, other) = (crate::memory::identity::derive_project_id(Path::new("/old/repo")), other_project());
        let (recorded, renamed_from) = (old.clone(), other.clone());
        dst.conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO project_path (id, root_path, created_at, project_id, root_commit, git_stamp) \
                     VALUES (?1, '/old/repo', 0, ?1, 'c1', 's1')",
                    [&recorded],
                )?;
                c.execute(
                    "INSERT INTO project_path (id, root_path, created_at, project_id, remote, root_commit, git_stamp) \
                     VALUES (?1, ?2, 0, ?1, 'github.com/acme/old-other', 'c2', 's2')",
                    rusqlite::params![renamed_from, OTHER_ROOT],
                )?;
                c.execute(
                    "INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/old-other', ?1, 0)",
                    [&renamed_from],
                )?;
                c.execute(
                    "INSERT INTO project (id, name, born, created_at) VALUES (?1, ?1, 'root', 0), (?2, ?2, 'root', 0)",
                    [&recorded, &renamed_from],
                )
            })
            .await
            .unwrap();
        // The checkout recorded at /old/repo is now at the file's root, and
        // the other root holds a clone of its remote under a new name.
        let ground = Arc::new(FakeGround::default());
        ground.put(Path::new(ROOT), None, Some("s1"), RootCommit::Known("c1".into()));
        ground.put(Path::new(OTHER_ROOT), Some("github.com/acme/other"), Some("s3"), RootCommit::Known("c2".into()));

        let (report, _) = run_on(&dst, &file, ImportOptions::default(), ground).await;

        let renamed = crate::memory::identity::candidate_id(Path::new(OTHER_ROOT), 1);
        assert_eq!(report.by_project.get(&old), Some(&13), "the moved checkout keeps its project: {:?}", report.by_project);
        assert_eq!(report.by_project.get(&renamed), Some(&1), "{:?}", report.by_project);
        let s = |v: &str| Some(v.to_string());
        let (folders, remotes, mut projects, suggested) = dst
            .conn
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT root_path, id, project_id, remote, root_commit, git_stamp FROM project_path ORDER BY root_path, created_at",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?;
                let folders = rows.collect::<rusqlite::Result<Vec<FolderRow>>>()?;
                let mut stmt = c.prepare("SELECT remote, project_id FROM project_remote ORDER BY remote")?;
                let remotes = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<(String, String)>>>()?;
                let mut stmt = c.prepare("SELECT id, born FROM project")?;
                let projects = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<(String, String)>>>()?;
                let mut stmt = c.prepare("SELECT project_id, other_id, reason FROM project_suggestion")?;
                let suggested =
                    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<(String, String, String)>>>()?;
                Ok::<_, rusqlite::Error>((folders, remotes, projects, suggested))
            })
            .await
            .unwrap();
        assert_eq!(
            folders,
            vec![
                ("/old/repo".into(), old.clone(), s(&old), None, s("c1"), s("s1")),
                (OTHER_ROOT.into(), other.clone(), s(&other), s("github.com/acme/old-other"), s("c2"), s("s2")),
                (OTHER_ROOT.into(), renamed.clone(), s(&renamed), s("github.com/acme/other"), s("c2"), s("s3")),
                (ROOT.into(), project(), s(&old), None, s("c1"), s("s1")),
            ],
            "each folder records its project and fingerprints"
        );
        assert_eq!(
            remotes,
            vec![("github.com/acme/old-other".into(), other.clone()), ("github.com/acme/other".into(), renamed.clone())]
        );
        projects.sort();
        let mut expected = vec![(old, "root".to_string()), (other.clone(), "root".into()), (renamed.clone(), "import".into())];
        expected.sort();
        assert_eq!(projects, expected, "the project the import made is born of it");
        assert_eq!(suggested, vec![(renamed.clone(), other.clone(), "remote_changed".to_owned())], "the remote changed in place");
    }

    /// A disk that notes the thread each first commit is read on.
    #[derive(Default)]
    struct Watched {
        disk: FakeGround,
        threads: std::sync::Mutex<Vec<std::thread::ThreadId>>,
    }

    impl Ground for Watched {
        fn checkout(&self, root: &Path) -> crate::memory::identity::Checkout {
            self.disk.checkout(root)
        }

        fn root_commit(&self, root: &Path) -> RootCommit {
            self.threads.lock().unwrap().push(std::thread::current().id());
            self.disk.root_commit(root)
        }

        fn recorded_checkout_present(&self, path: &Path, stamp: Option<&str>) -> bool {
            self.disk.recorded_checkout_present(path, stamp)
        }

        fn subpath(&self, root: &Path, path: &Path) -> Option<String> {
            self.disk.subpath(root, path)
        }
    }

    /// A clawg file with `projects` as its project lines and one live row per
    /// `(id, project, content)`.
    pub(crate) fn clawg_file(projects: &[&str], rows: &[(i64, Option<&str>, &str)]) -> String {
        let mut lines = vec![EXAMPLE.lines().next().unwrap().to_owned()];
        lines.extend(projects.iter().map(|p| p.to_string()));
        let template: serde_json::Value = serde_json::from_str(live_example().lines().nth(3).unwrap()).unwrap();
        for (id, project, content) in rows {
            let mut row = template.clone();
            row["id"] = (*id).into();
            row["project_id"] = project.map_or(serde_json::Value::Null, Into::into);
            row["content"] = (*content).into();
            row["topic_key"] = serde_json::Value::Null;
            row["supersede_cause"] = serde_json::Value::Null;
            row["created_at"] = (1_750_000_000_000 + id).into();
            lines.push(row.to_string());
        }
        lines.join("\n") + "\n"
    }

    /// Runs `sql` on `store`.
    pub(crate) async fn exec(store: &MemoryStore, sql: &'static str) {
        store.conn.call(move |c| c.execute_batch(sql)).await.unwrap();
    }

    /// The project here of each live row, by its content.
    async fn projects_by_content(store: &MemoryStore) -> BTreeMap<String, Option<String>> {
        let rows = dump(store, EVERYTHING).await.memories;
        rows.into_iter().map(|m| (m.content, m.project_id)).collect()
    }

    async fn project_names(store: &MemoryStore) -> BTreeMap<String, (String, String)> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT id, name, born FROM project")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?;
                rows.collect::<rusqlite::Result<BTreeMap<String, (String, String)>>>()
            })
            .await
            .unwrap()
    }

    fn went<'r>(report: &'r ImportReport, id: &str) -> &'r ImportedProject {
        report.projects.iter().find(|p| p.id == id).unwrap_or_else(|| panic!("{id} in {:#?}", report.projects))
    }

    /// X and Z with a folder, a remote and a row each, and then X merged into
    /// Z as a merge leaves them.
    const MERGED_HERE: &str = "INSERT INTO project (id, name, born, reviewed, created_at) VALUES \
           ('x-1111', 'x', 'root', 1, 1), ('z-2222', 'z', 'root', 1, 2);
         INSERT INTO project_path (id, root_path, created_at, project_id, remote) VALUES \
           ('x-1111', '/work/x', 1, 'x-1111', 'github.com/acme/x'), ('z-2222', '/work/z', 2, 'z-2222', 'github.com/acme/z');
         INSERT INTO project_remote (remote, project_id, created_at) VALUES \
           ('github.com/acme/x', 'x-1111', 1), ('github.com/acme/z', 'z-2222', 2);
         UPDATE project SET merged_into = 'z-2222', merged_at = 3 WHERE id = 'x-1111';
         UPDATE project_path SET project_id = 'z-2222' WHERE project_id = 'x-1111';
         UPDATE project_remote SET project_id = 'z-2222' WHERE project_id = 'x-1111';";

    #[tokio::test]
    async fn an_export_from_before_a_merge_lands_in_its_target() {
        for remote in [true, false] {
            let dst = destination().await;
            exec(&dst, MERGED_HERE).await;
            let line = match remote {
                true => r#"{"kind":"project","id":"x-1111","root_path":"/elsewhere/x","name":"x","folders":[{"root_path":"/elsewhere/x","remote":"github.com/acme/x"}],"remotes":["github.com/acme/x"]}"#,
                false => r#"{"kind":"project","id":"x-1111","root_path":"/elsewhere/x","name":"x"}"#,
            };
            let file = clawg_file(&[line], &[(1, Some("x-1111"), "A note of x before the merge")]);
            let before = project_names(&dst).await;

            let (report, _) = run(&dst, &file, ImportOptions::default()).await;

            let by = if remote { By::Remote } else { By::Id };
            let x = went(&report, "x-1111");
            assert_eq!((x.to.as_deref(), x.by, x.created), (Some("z-2222"), Some(by), false), "remote {remote}");
            assert_eq!(projects_by_content(&dst).await["A note of x before the merge"].as_deref(), Some("z-2222"));
            assert_eq!(project_names(&dst).await, before, "no project is made, and x stays a tombstone");
        }
    }

    #[tokio::test]
    async fn an_export_from_another_machine_lands_in_the_project_of_its_remote() {
        let dst = destination().await;
        exec(
            &dst,
            "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('app-1a2b', 'app', 'root', 1, 1);
             INSERT INTO project_path (id, root_path, created_at, project_id, remote) VALUES ('app-1a2b', '/work/app', 1, 'app-1a2b', 'github.com/acme/app');
             INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/app', 'app-1a2b', 1);",
        )
        .await;
        let line = r#"{"kind":"project","id":"app-9f9f","root_path":"/home/otra/app","name":"app","folders":[{"root_path":"/home/otra/app","remote":"github.com/acme/app"}],"remotes":["github.com/acme/app"]}"#;
        let file = clawg_file(&[line], &[(1, Some("app-9f9f"), "A note from the other machine")]);
        let before = project_names(&dst).await;

        let (report, _) = run(&dst, &file, ImportOptions::default()).await;

        assert_eq!(went(&report, "app-9f9f").to.as_deref(), Some("app-1a2b"));
        assert_eq!(projects_by_content(&dst).await["A note from the other machine"].as_deref(), Some("app-1a2b"));
        assert_eq!(project_names(&dst).await, before, "no project is made");
    }

    #[tokio::test]
    async fn a_project_without_a_folder_goes_and_comes_back_by_name() {
        let a = destination().await;
        exec(&a, "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('ideas-de-producto-3c4d', 'Ideas de producto', 'agent', 1, 1);").await;
        insert_row(&a, "claude-code", Some("ideas-de-producto-3c4d".into()), "An idea from A", None, 10).await;
        let b = destination().await;
        exec(&b, "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('ideas-9a9a', 'ideas de producto', 'agent', 1, 1);").await;
        insert_row(&b, "claude-code", Some("ideas-9a9a".into()), "An idea from B", None, 11).await;
        let from_a = dump(&a, ExportFilter::default()).await.to_jsonl();
        let from_b = dump(&b, ExportFilter::default()).await.to_jsonl();

        let (report, _) = run(&b, &from_a, ImportOptions::default()).await;
        assert_eq!(went(&report, "ideas-de-producto-3c4d").by, Some(By::Name));
        assert_eq!(projects_by_content(&b).await["An idea from A"].as_deref(), Some("ideas-9a9a"));
        run(&a, &from_b, ImportOptions::default()).await;
        assert_eq!(projects_by_content(&a).await["An idea from B"].as_deref(), Some("ideas-de-producto-3c4d"));
        assert_eq!(project_names(&a).await.len(), 1, "no project is made");
        assert_eq!(project_names(&b).await.len(), 1, "no project is made");

        let empty = destination().await;
        let (report, _) = run(&empty, &from_a, ImportOptions::default()).await;
        let created = went(&report, "ideas-de-producto-3c4d");
        assert_eq!((created.to.as_deref(), created.by, created.created), (Some("ideas-de-producto-3c4d"), Some(By::New), true));
        let names = project_names(&empty).await;
        assert_eq!(names["ideas-de-producto-3c4d"], ("Ideas de producto".to_owned(), "import".to_owned()));
    }

    #[tokio::test]
    async fn a_project_contradicted_here_is_another_project() {
        let dst = destination().await;
        let api = crate::memory::identity::derive_project_id(Path::new("/work/api"));
        let sql: &'static str = Box::leak(
            format!(
                "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('{api}', 'api', 'root', 1, 1);
                 INSERT INTO project_path (id, root_path, created_at, project_id, remote) VALUES ('{api}', '/work/api', 1, '{api}', 'github.com/acme/api-b');
                 INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/api-b', '{api}', 1);"
            )
            .into_boxed_str(),
        );
        exec(&dst, sql).await;
        insert_row(&dst, "claude-code", Some(api.clone()), "A note of api-b", None, 10).await;
        let ground = Arc::new(FakeGround::default());
        ground.put(Path::new("/work/api"), Some("github.com/acme/api-b"), Some("s1"), RootCommit::Later);
        let line = format!(
            r#"{{"kind":"project","id":"{api}","root_path":"/work/api","name":"api","folders":[{{"root_path":"/work/api","remote":"github.com/acme/api-a"}}],"remotes":["github.com/acme/api-a"]}}"#
        );
        let file = clawg_file(&[&line], &[(1, Some(&api), "A note of api-a")]);

        let (report, _) = run_on(&dst, &file, ImportOptions::default(), ground).await;

        let other = went(&report, &api).to.clone().unwrap();
        assert_ne!(other, api, "{report:#?}");
        assert_eq!(went(&report, &api).by, Some(By::New));
        let placed = projects_by_content(&dst).await;
        assert_eq!(placed["A note of api-a"], Some(other.clone()));
        assert_eq!(placed["A note of api-b"], Some(api.clone()), "the project here receives nothing");
        let folders = count(&dst, "SELECT COUNT(*) FROM project_path WHERE root_path = '/work/api'").await;
        assert_eq!(folders, 1, "another repository's checkout is not the new project's folder");
        let bound: String = dst
            .conn
            .call(|c| c.query_row("SELECT project_id FROM project_remote WHERE remote = 'github.com/acme/api-a'", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(bound, other, "a clone that comes later lands in it");
    }

    #[tokio::test]
    async fn an_export_from_before_names_and_locators_imports_as_before() {
        let dst = destination().await;
        let id = crate::memory::identity::derive_project_id(Path::new("/work/old"));
        let line = format!(r#"{{"kind":"project","id":"{id}","root_path":"/work/old"}}"#);
        let file = clawg_file(&[&line], &[(1, Some(&id), "A note of an old file")]);

        let (report, _) = run(&dst, &file, ImportOptions::default()).await;

        assert!(report.invalid.is_empty(), "{report:#?}");
        let old = went(&report, &id);
        assert_eq!((old.to.as_deref(), old.by, old.created), (Some(id.as_str()), Some(By::New), true));
        assert_eq!(projects_by_content(&dst).await["A note of an old file"], Some(id.clone()));
        let folder: (String, String, Option<String>) = dst
            .conn
            .call(|c| c.query_row("SELECT id, project_id, remote FROM project_path WHERE root_path = '/work/old'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))))
            .await
            .unwrap();
        assert_eq!(folder, (id.clone(), id.clone(), None));
        assert_eq!(project_names(&dst).await[&id], ("old".to_owned(), "import".to_owned()));
    }

    #[tokio::test]
    async fn a_row_without_a_line_creates_its_project_as_the_import() {
        let dst = destination().await;
        let file = clawg_file(&[], &[(1, Some("loose-7e7e"), "A note of a project without a line")]);

        let (report, _) = run(&dst, &file, ImportOptions::default()).await;

        assert_eq!(report.inserted, 1);
        assert_eq!(projects_by_content(&dst).await["A note of a project without a line"].as_deref(), Some("loose-7e7e"));
        assert_eq!(project_names(&dst).await["loose-7e7e"], ("loose".to_owned(), "import".to_owned()));
        let journaled = count(&dst, "SELECT COUNT(*) FROM memory_import_registry WHERE kind = 'project' AND a = 'loose-7e7e'").await;
        assert_eq!(journaled, 1);
    }

    #[tokio::test]
    async fn a_new_project_keeps_the_id_and_name_of_its_line() {
        let dst = destination().await;
        let file = clawg_file(&[r#"{"kind":"project","id":"ideas-1a2b","name":"Ideas"}"#], &[(1, Some("ideas-1a2b"), "An idea")]);

        let (report, _) = run(&dst, &file, ImportOptions::default()).await;

        assert_eq!(went(&report, "ideas-1a2b").to.as_deref(), Some("ideas-1a2b"));
        assert_eq!(project_names(&dst).await["ideas-1a2b"], ("Ideas".to_owned(), "import".to_owned()));
        let journaled: (String, Option<String>) = dst
            .conn
            .call(|c| c.query_row("SELECT a, value FROM memory_import_registry WHERE kind = 'project'", [], |r| Ok((r.get(0)?, r.get(1)?))))
            .await
            .unwrap();
        assert_eq!(journaled, ("ideas-1a2b".to_owned(), Some("Ideas".to_owned())));
    }

    #[tokio::test]
    async fn an_id_taken_here_gives_the_new_project_another() {
        let dst = destination().await;
        exec(&dst, "INSERT INTO project_path (id, root_path, created_at) VALUES ('ideas-1a2b', '/work/ideas', 1);").await;
        let file = clawg_file(&[r#"{"kind":"project","id":"ideas-1a2b","name":"ideas"}"#], &[(1, Some("ideas-1a2b"), "An idea")]);

        let (report, _) = run(&dst, &file, ImportOptions::default()).await;

        let to = went(&report, "ideas-1a2b").to.clone().unwrap();
        assert_ne!(to, "ideas-1a2b");
        assert!(to.starts_with("ideas-"), "{to}");
        assert_eq!(project_names(&dst).await[&to], ("ideas".to_owned(), "import".to_owned()));
        assert_eq!(projects_by_content(&dst).await["An idea"], Some(to));
    }

    #[tokio::test]
    async fn a_folder_of_another_machine_records_the_fingerprints_of_its_line() {
        let dst = destination().await;
        let line = r#"{"kind":"project","id":"notas-1a2b","root_path":"/home/otra/notas","name":"notas","folders":[{"root_path":"/home/otra/notas","root_commit":"c1"},{"root_path":"/home/otra/notas-2","remote":"github.com/acme/notas","root_commit":"c1"}]}"#;
        let file = clawg_file(&[line], &[(1, Some("notas-1a2b"), "A note")]);

        run(&dst, &file, ImportOptions::default()).await;

        let folders = dst
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT root_path, project_id, remote, root_commit, git_stamp, index_docs FROM project_path WHERE root_path LIKE '/home/otra/%' ORDER BY root_path")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?;
                rows.collect::<rusqlite::Result<Vec<(String, String, Option<String>, Option<String>, Option<String>, bool)>>>()
            })
            .await
            .unwrap();
        let s = |v: &str| Some(v.to_owned());
        assert_eq!(
            folders,
            vec![
                ("/home/otra/notas".into(), "notas-1a2b".into(), None, s("c1"), None, false),
                ("/home/otra/notas-2".into(), "notas-1a2b".into(), s("github.com/acme/notas"), s("c1"), None, false),
            ]
        );
    }

    #[tokio::test]
    async fn a_checkout_without_origin_registered_after_an_import_joins_its_project() {
        let dst = destination().await;
        let line = r#"{"kind":"project","id":"notas-1a2b","root_path":"/home/otra/notas","name":"notas","folders":[{"root_path":"/home/otra/notas","root_commit":"c1"}]}"#;
        let file = clawg_file(&[line], &[(1, Some("notas-1a2b"), "A note")]);
        run(&dst, &file, ImportOptions::default()).await;
        let ground = Arc::new(FakeGround::default());
        ground.put(Path::new("/work/notas"), None, Some("s1"), RootCommit::Known("c1".into()));
        let home = crate::test_dir::test_dir();
        let workspaces = crate::memory::workspace::WorkspaceManager::new(home.path().join("workspaces"));
        let reg = crate::memory::project::Registration {
            root: Path::new("/work/notas"),
            checkout: ground.checkout(Path::new("/work/notas")),
            legacy_ids: &[],
            index_docs: false,
            assigned: true,
            subpath: None,
        };

        let outcome = crate::memory::project::register_project(&dst, &workspaces, home.path(), reg, ground).await.unwrap();

        assert_eq!(outcome.project_id, "notas-1a2b", "a move of the checkout the import recorded");
        assert_eq!(count(&dst, "SELECT COUNT(*) FROM project_suggestion").await, 0);
    }

    /// A repository here bound to the remote of the file's `mono-5e6f`.
    const MONO_HERE: &str = "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('mono-here', 'mono', 'root', 1, 1);
         INSERT INTO project_path (id, root_path, created_at, project_id, remote) \
           VALUES ('mono-here', '/work/mono', 1, 'mono-here', 'github.com/acme/mono');
         INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/mono', 'mono-here', 1);";

    const MONO_LINES: [&str; 2] = [
        r#"{"kind":"project","id":"mono-5e6f","name":"mono","folders":[{"root_path":"/elsewhere/mono","remote":"github.com/acme/mono"}],"remotes":["github.com/acme/mono"],"subfolders":[{"subpath":"apps/web","project_id":"web-9c0d"}]}"#,
        r#"{"kind":"project","id":"web-9c0d","name":"Web"}"#,
    ];

    async fn subfolders_of(store: &MemoryStore) -> Vec<(String, String, String)> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT parent_id, subpath, project_id FROM project_subfolder ORDER BY rowid")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn the_subfolders_of_a_repository_travel_with_it() {
        let dst = destination().await;
        exec(&dst, MONO_HERE).await;
        let file = clawg_file(&MONO_LINES, &[(1, Some("mono-5e6f"), "A note of the monorepo")]);

        let (report, _) = run(&dst, &file, ImportOptions::default()).await;

        assert_eq!(went(&report, "mono-5e6f").to.as_deref(), Some("mono-here"));
        let web = went(&report, "web-9c0d");
        assert!(web.created, "the subproject no row took is made for its declaration");
        let web = web.to.clone().unwrap();
        assert_eq!(subfolders_of(&dst).await, vec![("mono-here".into(), "apps/web".into(), web.clone())]);
        assert_eq!(report.subfolders.len(), 1);
        assert!(report.subfolders[0].written);
        assert_eq!(
            count(&dst, "SELECT COUNT(*) FROM memory_import_registry WHERE kind = 'subfolder' AND a = 'mono-here' AND b = 'apps/web'").await,
            1
        );

        let ground = Arc::new(FakeGround::default());
        ground.put(Path::new("/work/mono"), Some("github.com/acme/mono"), Some("s1"), RootCommit::Known("c1".into()));
        let home = crate::test_dir::test_dir();
        let workspaces = crate::memory::workspace::WorkspaceManager::new(home.path().join("workspaces"));
        let reg = crate::memory::project::Registration {
            root: Path::new("/work/mono"),
            checkout: ground.checkout(Path::new("/work/mono")),
            legacy_ids: &[],
            index_docs: false,
            assigned: true,
            subpath: Some("apps/web/src"),
        };
        let outcome = crate::memory::project::register_project(&dst, &workspaces, home.path(), reg, ground).await.unwrap();

        assert_eq!((outcome.project_id.as_str(), outcome.subproject), ("mono-here", Some(web)));
    }

    #[tokio::test]
    async fn a_declaration_here_is_kept_over_the_file() {
        let dst = destination().await;
        exec(&dst, MONO_HERE).await;
        exec(
            &dst,
            "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('local-3333', 'local', 'user', 1, 2);
             INSERT INTO project_subfolder (parent_id, subpath, project_id, created_at) VALUES ('mono-here', 'apps/web', 'local-3333', 2);",
        )
        .await;
        let file = clawg_file(&MONO_LINES, &[(1, Some("mono-5e6f"), "A note of the monorepo")]);

        let (report, _) = run(&dst, &file, ImportOptions::default()).await;

        assert_eq!(subfolders_of(&dst).await, vec![("mono-here".into(), "apps/web".into(), "local-3333".into())]);
        let declared = &report.subfolders[0];
        assert_eq!((declared.written, declared.reason.as_deref()), (false, Some("declared_here")));
        assert_eq!(count(&dst, "SELECT COUNT(*) FROM memory_import_registry WHERE kind = 'subfolder'").await, 0);
    }

    #[tokio::test]
    async fn an_export_imported_again_after_a_move_adds_nothing() {
        let dst = destination().await;
        let line = r#"{"kind":"project","id":"notes-1a2b","name":"notes"}"#;
        let file = clawg_file(&[line], &[(1, Some("notes-1a2b"), "A note that moves"), (2, Some("notes-1a2b"), "A note that stays")]);
        run(&dst, &file, ImportOptions::default()).await;
        exec(&dst, "UPDATE memory SET project_id = 'other-3c4d' WHERE content = 'A note that moves';").await;
        let before = dump(&dst, EVERYTHING).await;

        let (report, _) = run(&dst, &file, ImportOptions::default()).await;

        assert_eq!((report.inserted, report.present), (0, 1));
        assert_eq!(report.present_elsewhere, BTreeMap::from([("other-3c4d".to_owned(), 1)]));
        assert_eq!(dump(&dst, EVERYTHING).await, before);
    }

    #[tokio::test]
    async fn two_rows_alike_from_two_origins_stay_two() {
        let dst = destination().await;
        let mut lines: Vec<String> = clawg_file(&[], &[(1, Some("a-1111"), "One text"), (2, Some("b-2222"), "One text")])
            .lines()
            .map(str::to_owned)
            .collect();
        for (line, origin) in lines[1..].iter_mut().zip(["engram:a", "engram:b"]) {
            let mut row: serde_json::Value = serde_json::from_str(line).unwrap();
            row["source"] = "import".into();
            row["created_by_ref"] = origin.into();
            row["created_at"] = 1_750_000_000_000i64.into();
            *line = row.to_string();
        }

        let (report, _) = run(&dst, &(lines.join("\n") + "\n"), ImportOptions::default()).await;

        assert_eq!((report.inserted, report.present_elsewhere.len()), (2, 0));
        let rows = dump(&dst, EVERYTHING).await.memories.into_iter().filter(|m| m.content == "One text");
        let projects: Vec<Option<String>> = rows.map(|m| m.project_id).collect();
        assert_eq!(projects, vec![Some("a-1111".to_owned()), Some("b-2222".to_owned())]);
    }

    async fn suggestions_of(store: &MemoryStore) -> Vec<(String, String, String)> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT project_id, other_id, reason FROM project_suggestion ORDER BY rowid")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap()
    }

    async fn journaled_suggestions(store: &MemoryStore) -> Vec<(String, String, String)> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT a, b, value FROM memory_import_registry WHERE kind = 'suggestion' ORDER BY rowid")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap()
    }

    fn suggested(report: &ImportReport) -> Vec<(String, String, String)> {
        report.suggestions.iter().map(|s| (s.project_id.clone(), s.other_id.clone(), s.reason.clone())).collect()
    }

    #[tokio::test]
    async fn an_import_leaves_the_suggestions_a_registration_would() {
        let dst = destination().await;
        exec(
            &dst,
            "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('first-1111', 'first', 'root', 1, 1);
             INSERT INTO project_path (id, root_path, created_at, project_id, root_commit, git_stamp) \
               VALUES ('first-1111', '/work/first', 1, 'first-1111', 'c1', 's1');",
        )
        .await;
        // A second checkout of the same history, the first still in its place.
        let ground = Arc::new(FakeGround::default());
        ground.put(Path::new("/work/first"), None, Some("s1"), RootCommit::Known("c1".into()));
        ground.put(Path::new("/work/second"), None, Some("s2"), RootCommit::Known("c1".into()));
        let line = r#"{"kind":"project","id":"second-2222","root_path":"/work/second","name":"second"}"#;
        let file = clawg_file(&[line], &[(1, Some("second-2222"), "A note of the second checkout")]);

        let (report, _) = run_on(&dst, &file, ImportOptions::default(), ground).await;

        let to = went(&report, "second-2222").to.clone().unwrap();
        let expected = vec![(to, "first-1111".to_owned(), "root_commit".to_owned())];
        assert_eq!(suggested(&report), expected);
        assert_eq!(suggestions_of(&dst).await, expected);
        assert_eq!(journaled_suggestions(&dst).await, expected);
    }

    #[tokio::test]
    async fn a_project_named_like_one_here_is_suggested() {
        let dst = destination().await;
        exec(&dst, "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('mi-app-1111', 'mi-app', 'agent', 1, 1);").await;
        let file = clawg_file(&[r#"{"kind":"project","id":"mi-app-9999","name":"Mi App"}"#], &[(1, Some("mi-app-9999"), "A note")]);

        let (report, _) = run(&dst, &file, ImportOptions::default()).await;

        let made = went(&report, "mi-app-9999");
        assert_eq!((made.by, made.created), (Some(By::New), true));
        assert_eq!(project_names(&dst).await["mi-app-9999"], ("Mi App".to_owned(), "import".to_owned()));
        let expected = vec![("mi-app-9999".to_owned(), "mi-app-1111".to_owned(), "similar_name".to_owned())];
        assert_eq!(suggestions_of(&dst).await, expected);

        let again = destination().await;
        exec(&again, "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('mi-app-1111', 'mi-app', 'agent', 1, 1);").await;
        exec(&again, "INSERT INTO project_suggestion (project_id, other_id, reason, created_at, answer, answered_at) VALUES ('mi-app-1111', 'mi-app-9999', 'user', 1, 'apart', 1);").await;
        let (report, _) = run(&again, &file, ImportOptions::default()).await;
        assert!(report.suggestions.is_empty(), "never again for a pair that has one");
        assert_eq!(journaled_suggestions(&again).await, Vec::new());
    }

    #[tokio::test]
    async fn a_merge_at_the_source_is_suggested_here() {
        let dst = destination().await;
        exec(
            &dst,
            "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('x-1111', 'x', 'root', 1, 1), ('z-2222', 'z', 'root', 1, 2);
             INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/x', 'x-1111', 1), ('github.com/acme/z', 'z-2222', 2);",
        )
        .await;
        insert_row(&dst, "claude-code", Some("x-1111".into()), "A note of x here", None, 10).await;
        insert_row(&dst, "claude-code", Some("z-2222".into()), "A note of z here", None, 11).await;
        let tombstone = r#"{"kind":"project","id":"x-1111","name":"x","merged_into":"z-2222"}"#;
        let target = r#"{"kind":"project","id":"z-2222","name":"z","remotes":["github.com/acme/z"]}"#;
        let file = clawg_file(&[tombstone, target], &[(1, Some("z-2222"), "A note of z there")]);
        let before = project_names(&dst).await;

        let (report, _) = run(&dst, &file, ImportOptions::default()).await;

        assert_eq!(suggestions_of(&dst).await, vec![("z-2222".to_owned(), "x-1111".to_owned(), "merged_at_source".to_owned())]);
        assert_eq!(project_names(&dst).await, before, "neither changes");
        assert_eq!(went(&report, "x-1111").to.as_deref(), Some("x-1111"));

        let fused = destination().await;
        exec(
            &fused,
            "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('x-1111', 'x', 'root', 1, 1), ('z-2222', 'z', 'root', 1, 2);
             INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/x', 'x-1111', 1), ('github.com/acme/z', 'z-2222', 2);",
        )
        .await;
        let one = r#"{"kind":"project","id":"z-9999","name":"z","remotes":["github.com/acme/z","github.com/acme/x"]}"#;
        run(&fused, &clawg_file(&[one], &[(1, Some("z-9999"), "A note of both")]), ImportOptions::default()).await;
        assert_eq!(suggestions_of(&fused).await, vec![("z-2222".to_owned(), "x-1111".to_owned(), "merged_at_source".to_owned())]);
    }

    #[tokio::test]
    async fn a_dry_run_reports_the_suggestions_and_writes_none() {
        let dst = destination().await;
        exec(&dst, "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('mi-app-1111', 'mi-app', 'agent', 1, 1);").await;
        let file = clawg_file(&[r#"{"kind":"project","id":"mi-app-9999","name":"Mi App"}"#], &[(1, Some("mi-app-9999"), "A note")]);

        let (report, _) = run(&dst, &file, ImportOptions { dry_run: true, ..Default::default() }).await;

        assert_eq!(suggested(&report), vec![("mi-app-9999".to_owned(), "mi-app-1111".to_owned(), "similar_name".to_owned())]);
        assert!(suggestions_of(&dst).await.is_empty());
        assert_eq!(project_names(&dst).await.len(), 1, "nothing is created");
    }

    pub(crate) const DELETED_HERE: &str = "INSERT INTO project (id, name, born, reviewed, created_at, deleted_at) VALUES ('gone-1a2b', 'gone', 'agent', 1, 1, 50);
         INSERT INTO project_delete (delete_id, project_id, with_memories, code, snapshot, deleted_at) VALUES (1, 'gone-1a2b', 1, 'abcd1234', NULL, 50);";

    async fn deleted_state(store: &MemoryStore, id: &'static str) -> (Option<i64>, bool) {
        store.conn.call(move |c| c.query_row("SELECT deleted_at, reviewed FROM project WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))).await.unwrap()
    }

    #[tokio::test]
    async fn a_row_imported_deleted_leaves_its_deleted_project_deleted() {
        let dst = destination().await;
        exec(&dst, DELETED_HERE).await;
        let file = clawg_file(&[r#"{"kind":"project","id":"gone-1a2b","name":"gone"}"#], &[(1, Some("gone-1a2b"), "A note deleted at the source")])
            .replace("\"deleted_at\":null", "\"deleted_at\":1760000000000");

        let (report, _) = run(&dst, &file, ImportOptions::default()).await;

        assert_eq!(report.inserted, 1);
        let gone = went(&report, "gone-1a2b");
        assert_eq!((gone.to.as_deref(), gone.by, gone.revived), (Some("gone-1a2b"), Some(By::Id), false));
        assert_eq!(deleted_state(&dst, "gone-1a2b").await, (Some(50), true), "it stays deleted, as it was");
        let row: (String, Option<i64>) = dst
            .conn
            .call(|c| c.query_row("SELECT project_id, deleted_at FROM memory WHERE content = 'A note deleted at the source'", [], |r| Ok((r.get(0)?, r.get(1)?))))
            .await
            .unwrap();
        assert_eq!(row, ("gone-1a2b".to_owned(), Some(1_760_000_000_000)));
        assert_eq!(count(&dst, "SELECT COUNT(*) FROM memory_import_registry").await, 0);
    }

    #[tokio::test]
    async fn a_live_row_imported_into_a_deleted_project_brings_it_back_and_says_so() {
        let dst = destination().await;
        exec(&dst, DELETED_HERE).await;
        let file = clawg_file(&[r#"{"kind":"project","id":"gone-1a2b","name":"gone"}"#], &[(1, Some("gone-1a2b"), "A live note of gone")]);

        let (dry, _) = run(&dst, &file, ImportOptions { dry_run: true, ..Default::default() }).await;
        assert!(went(&dry, "gone-1a2b").revived, "a dry run says it too");
        assert_eq!(deleted_state(&dst, "gone-1a2b").await, (Some(50), true));

        let (report, _) = run(&dst, &file, ImportOptions::default()).await;

        assert!(went(&report, "gone-1a2b").revived, "{report:#?}");
        assert_eq!(deleted_state(&dst, "gone-1a2b").await, (None, false), "back, unreviewed");
        let journaled: (String, Option<String>) = dst
            .conn
            .call(|c| c.query_row("SELECT a, value FROM memory_import_registry WHERE kind = 'revived'", [], |r| Ok((r.get(0)?, r.get(1)?))))
            .await
            .unwrap();
        assert_eq!(journaled, ("gone-1a2b".to_owned(), Some("50".to_owned())));
    }

    #[tokio::test]
    async fn an_import_reads_the_first_commit_off_the_store_s_thread() {
        let file = seeded_file().await;
        let dst = destination().await;
        let ground = Arc::new(Watched::default());
        ground.disk.put(Path::new(ROOT), None, Some("s1"), RootCommit::Known("c1".into()));

        run_on(&dst, &file, ImportOptions::default(), ground.clone()).await;

        let store_thread = dst.conn.call(|_| Ok::<_, rusqlite::Error>(std::thread::current().id())).await.unwrap();
        let threads = ground.threads.lock().unwrap();
        assert_eq!(threads.len(), 1, "the first commit is read once");
        assert_ne!(threads[0], store_thread, "a `git` on the store's thread would hold every other call");
    }

    #[tokio::test]
    async fn a_live_twin_absorbs_the_imported_row() {
        let file = seeded_file().await;
        let dst = destination().await;
        let twin = insert_row(&dst, "claude-code", Some(project()), "Storage engine: SQLite with WAL", None, 5).await;
        let (report, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!(report.merged, vec![Merged { line: report.merged[0].line, id: 2, into: twin }]);
        assert_eq!(report.inserted, 16);
        let after = dump(&dst, EVERYTHING).await;
        let live = after.memories.iter().filter(|m| m.content == "Storage engine: SQLite with WAL").count();
        assert_eq!(live, 1);
        let old = by_content(&after)["Storage engine: Postgres"].clone();
        assert_eq!(old.superseded_by, Some(twin), "links to the row point to its twin");
        let conclusion = by_content(&after)["The project favours embedded stores"].id;
        assert!(after.derivations.iter().any(|d| d.memory == conclusion && d.source == twin));
    }

    #[tokio::test]
    async fn a_taken_topic_key_supersedes_the_imported_row() {
        let file = seeded_file().await;
        let dst = destination().await;
        let holder = insert_row(&dst, "claude-code", Some(project()), "Storage engine: DuckDB", Some("storage/engine"), 5).await;
        let (report, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!(report.topic_taken.len(), 1, "{report:#?}");
        let taken = &report.topic_taken[0];
        assert_eq!((taken.id, taken.topic_key.as_str(), taken.by), (2, "storage/engine", holder));
        assert_eq!(report.by_state["superseded"], 3);
        let after = by_content(&dump(&dst, EVERYTHING).await);
        let imported = &after["Storage engine: SQLite with WAL"];
        assert_eq!((imported.status.as_str(), imported.superseded_by, imported.supersede_cause.as_deref()), ("superseded", Some(holder), Some("revision")));
        assert_eq!(after["Storage engine: Postgres"].superseded_by, Some(imported.id));
        assert_eq!(after["Storage engine: DuckDB"].status, "active");
    }

    #[tokio::test]
    async fn an_import_merges_into_another_agents_live_text_and_key() {
        let file = seeded_file().await;
        let dst = destination().await;
        let twin = insert_row(&dst, "cursor", Some(project()), "Storage engine: SQLite with WAL", None, 5).await;
        let (report, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!(report.merged, vec![Merged { line: report.merged[0].line, id: 2, into: twin }], "the text is the project's");

        let dst = destination().await;
        let holder = insert_row(&dst, "cursor", Some(project()), "Storage engine: DuckDB", Some("storage/engine"), 5).await;
        let (report, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!(report.topic_taken.iter().map(|t| t.by).collect::<Vec<_>>(), vec![holder], "so is the key");
    }

    #[tokio::test]
    async fn agent_reassigns_every_row_that_has_one() {
        let file = seeded_file().await;
        let dst = destination().await;
        run(&dst, &file, ImportOptions { agent: Some("dev".into()), ..Default::default() }).await;
        let after = dump(&dst, EVERYTHING).await;
        let agents: HashSet<Option<String>> = after.memories.iter().skip(1).map(|m| m.agent_id.clone()).collect();
        assert_eq!(agents, HashSet::from([Some("dev".into()), None]));
        assert_eq!(by_content(&after)["Codex keeps notes in English"].created_by_ref, "claude-code", "who wrote it stays");
    }

    #[tokio::test]
    async fn invalid_lines_are_reported_and_the_rest_imports() {
        let mut file = seeded_file().await;
        file.push_str("{\"kind\":\"memory\",\"id\":99}\n");
        let lines = file.lines().count();
        let dst = destination().await;
        let (report, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!(report.inserted, 17);
        assert_eq!(report.invalid.len(), 1);
        assert_eq!(report.invalid[0].line, lines);
    }

    #[tokio::test]
    async fn links_to_rows_outside_the_file_are_dropped_and_reported() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed(&store).await;
        let file = dump(&store, ExportFilter::default()).await.to_jsonl();
        let dst = destination().await;
        let (report, _) = run(&dst, &file, ImportOptions::default()).await;
        let reasons: Vec<&str> = report.dropped_links.iter().map(|d| d.reason.as_str()).collect();
        assert_eq!(reasons, vec!["memory 15 was revived by deleting 16, which is not in the file"], "{report:#?}");
    }

    pub(crate) async fn run_foreign(store: &MemoryStore, notes: Vec<ForeignNote>, options: ImportOptions) -> (ImportReport, crate::test_dir::TestDir) {
        run_export(store, foreign_export(notes), options).await
    }

    pub(crate) async fn run_export(store: &MemoryStore, export: ForeignExport, options: ImportOptions) -> (ImportReport, crate::test_dir::TestDir) {
        let backups = crate::test_dir::test_dir();
        let chunker = Arc::new(ChatChunker::with_fallback());
        let report =
            import_foreign(store, Some(&FakeEmbedder::default()), chunker, backups.path(), export, &options, Arc::new(Disk), &mut ImportTimings::new(false)).await.unwrap();
        (report, backups)
    }

    /// `source` and `target` judged related by `verb`, by an agent.
    pub(crate) fn relation(source: &str, target: &str, verb: &str) -> ForeignRelation {
        ForeignRelation {
            source: format!("engram:{source}"),
            target: format!("engram:{target}"),
            verb: verb.to_owned(),
            judgment: "judged".to_owned(),
            by_user: false,
            reason: None,
            evidence: None,
            created: When::At(T0 + 100),
            updated: When::At(T0 + 200),
            superseded: false,
        }
    }

    /// The note `name`, written `k` ms after `T0`, so the ids here follow `k`.
    pub(crate) fn named(name: &str, k: i64) -> ForeignNote {
        ForeignNote { written: When::At(T0 + k), ..foreign_note(&format!("engram:{name}"), name, &format!("the note {name}")) }
    }

    /// Each pair by the text of its newer and older row: state, verdict, reason.
    async fn pairs(store: &MemoryStore) -> Vec<(String, String, String, Option<String>, Option<String>)> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT n.content, o.content, p.state, p.verdict, p.reason, p.new_id > p.old_id FROM memory_conflict p \
                     JOIN memory n ON n.id = p.new_id JOIN memory o ON o.id = p.old_id ORDER BY p.new_id, p.old_id",
                )?;
                let rows = stmt.query_map([], |r| {
                    assert!(r.get::<_, bool>(5)?, "a pair's new_id is its larger id");
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap()
    }

    /// Each piece of every linked save, in order: its text, the head it was
    /// embedded behind and its vector.
    async fn linked_pieces(store: &MemoryStore) -> Vec<(i64, String, String, Vec<u8>)> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT p.piece, m.content, p.context, v.embedding FROM memory_piece p \
                     JOIN memory m ON m.id = p.memory_id JOIN memory_vectors v ON v.rowid = m.id \
                     ORDER BY p.save_id, p.piece",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap()
    }

    #[derive(Debug)]
    struct Stored {
        id: i64,
        content: String,
        status: String,
        by: Option<i64>,
        cause: Option<String>,
        project: Option<String>,
        key: Option<String>,
        deleted: bool,
        disputed_against: Option<String>,
    }

    /// The rows imported from `origin`, by id.
    async fn imported(store: &MemoryStore, origin: &'static str) -> Vec<Stored> {
        store
            .conn
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT id, content, status, superseded_by, supersede_cause, project_id, topic_key, \
                     deleted_at IS NOT NULL, disputed_against FROM memory \
                     WHERE source = 'import' AND created_by_ref = ?1 ORDER BY id",
                )?;
                let rows = stmt.query_map([origin], |r| {
                    Ok(Stored {
                        id: r.get(0)?,
                        content: r.get(1)?,
                        status: r.get(2)?,
                        by: r.get(3)?,
                        cause: r.get(4)?,
                        project: r.get(5)?,
                        key: r.get(6)?,
                        deleted: r.get(7)?,
                        disputed_against: r.get(8)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap()
    }

    /// The text of the pieces of each linked save, in order.
    async fn saves(store: &MemoryStore) -> Vec<Vec<String>> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT p.save_id, m.content FROM memory_piece p JOIN memory m ON m.id = p.memory_id \
                     ORDER BY p.save_id, p.piece",
                )?;
                let mut saves: Vec<(i64, Vec<String>)> = Vec::new();
                for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
                    let (save, content) = row?;
                    match saves.last_mut() {
                        Some((last, pieces)) if *last == save => pieces.push(content),
                        _ => saves.push((save, vec![content])),
                    }
                }
                Ok::<_, rusqlite::Error>(saves.into_iter().map(|(_, pieces)| pieces).collect())
            })
            .await
            .unwrap()
    }

    /// The distinct pieces a save of `text` stores, in order.
    fn pieces_of(text: &str) -> Vec<String> {
        let mut seen = HashSet::new();
        ChatChunker::with_fallback().split_save(text).pieces.into_iter().map(|c| c.text).filter(|p| seen.insert(content_hash(p))).collect()
    }

    /// Records `root` as a project here and returns its id.
    async fn register(store: &MemoryStore, root: &'static str) -> String {
        let id = crate::memory::identity::derive_project_id(Path::new(root));
        let recorded = id.clone();
        store
            .conn
            .call(move |c| {
                c.execute("INSERT INTO project_path (id, root_path, created_at) VALUES (?1, ?2, 0)", rusqlite::params![recorded, root])?;
                c.execute("INSERT INTO project (id, name, born, created_at) VALUES (?1, ?1, 'root', 0)", [&recorded])
            })
            .await
            .unwrap();
        id
    }

    /// Notes of two projects of the source and of none: a long one, two under
    /// one key, and one the source deleted.
    pub(crate) fn sample_notes() -> Vec<ForeignNote> {
        let in_a = |origin: &str, title: &str, content: &str| ForeignNote { project: Some("a".into()), ..foreign_note(origin, title, content) };
        vec![
            in_a("engram:long", "Notes on clawg-ledger", &"one writer per store, ".repeat(150)),
            ForeignNote { topic_key: Some("sdd/x".into()), ..in_a("engram:old", "Proposal", "the first proposal") },
            ForeignNote {
                topic_key: Some("sdd/x".into()),
                written: When::At(T0 + 1000),
                ..in_a("engram:new", "Proposal", "the second proposal")
            },
            ForeignNote { project: Some("b".into()), ..foreign_note("engram:b", "In b", "a note of b") },
            foreign_note("engram:loose", "Loose", "a note of no project"),
            ForeignNote { deleted: Some(When::At(T0 + 5)), ..in_a("engram:gone", "Gone", "a note the source deleted") },
        ]
    }

    #[tokio::test]
    async fn an_imported_note_has_the_rows_of_a_save_of_its_text() {
        let title = "Notes on clawg-ledger";
        let words: Vec<String> = (0..700).map(|i| format!("w{i}")).collect();
        let content = format!("{}{}", words.join(" "), " word".repeat(600));
        let dst = destination().await;
        let notes = vec![foreign_note("engram:long", title, &content)];
        let backups = crate::test_dir::test_dir();
        let chunker = Arc::new(ChatChunker::with_fallback());
        let dry_run = ImportOptions { dry_run: true, ..Default::default() };
        let dry = import_foreign(&dst, None, chunker, backups.path(), foreign_export(notes.clone()), &dry_run, Arc::new(Disk), &mut ImportTimings::new(false)).await.unwrap();
        assert!(files_in(backups.path()).is_empty());
        let (report, _) = run_foreign(&dst, notes, ImportOptions::default()).await;
        assert_eq!(ImportReport { dry_run: false, snapshot: report.snapshot.clone(), import_id: report.import_id, ..dry }, report);

        let saved = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let event = MemoryEvent {
            agent_id: MIGRATED_AGENT.into(),
            project_id: None,
            role: Role::User,
            content: format!("{title}\n{content}"),
            is_error: None,
            source: MemorySource::Explicit,
            timestamp_ms: 1,
            memory_type: None,
            importance: None,
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
            session_id: None,
        };
        save_for_test(&saved, &event).await;
        let pieces = linked_pieces(&saved).await;
        assert!(pieces.len() > 2, "the text is long enough to split");
        assert_eq!(linked_pieces(&dst).await, pieces, "the same pieces, heads and vectors as a save");

        assert_eq!((report.inserted, report.rows), (pieces.len(), pieces.len()));
        assert_eq!(report.by_lang.values().sum::<usize>(), pieces.len());
        let foreign = report.foreign.clone().unwrap();
        assert_eq!((foreign.source.as_str(), foreign.rows, foreign.split_notes), ("engram", pieces.len(), 1));
        assert!(report.merged.is_empty() && report.topic_taken.is_empty() && report.invalid.is_empty(), "{report:#?}");
    }

    #[tokio::test]
    async fn a_key_held_here_supersedes_the_imported_note() {
        let root = "/work/notes";
        let dst = destination().await;
        let here = crate::memory::identity::derive_project_id(Path::new(root));
        let holder = insert_row(&dst, "claude-code", Some(here), "Storage engine: DuckDB", Some("storage/engine"), 5).await;
        let long = ForeignNote {
            project: Some("a".into()),
            topic_key: Some("storage/engine".into()),
            ..foreign_note("engram:a", "Storage engine", &"SQLite with WAL, one writer ".repeat(150))
        };
        let options = ImportOptions { project_map: vec![("a".into(), root.into())], ..Default::default() };
        let (report, _) = run_foreign(&dst, vec![long], options).await;

        let rows = imported(&dst, "engram:a").await;
        assert!(rows.len() > 1);
        for r in &rows {
            assert_eq!((r.status.as_str(), r.by, r.cause.as_deref()), ("superseded", Some(holder), Some("revision")));
        }
        let foreign = report.foreign.clone().unwrap();
        assert_eq!(foreign.topic_taken, vec![TakenKey { origin: "engram:a".into(), topic_key: "storage/engine".into(), by: holder }]);
        assert!(report.topic_taken.is_empty(), "the translation's ids mean nothing outside it");
        assert_eq!(report.by_state, BTreeMap::from([("superseded".to_string(), rows.len())]));
        let holder_status = dst
            .conn
            .call(move |c| c.query_row("SELECT status FROM memory WHERE id = ?1", [holder], |r| r.get::<_, String>(0)))
            .await
            .unwrap();
        assert_eq!(holder_status, "active");
    }

    #[tokio::test]
    async fn a_mapped_root_whose_id_is_taken_gets_its_own() {
        let root = "/work/notes";
        let dst = destination().await;
        let id = crate::memory::identity::derive_project_id(Path::new(root));
        dst.conn
            .call(move |c| c.execute("INSERT INTO project_path (id, root_path, created_at) VALUES (?1, '/not/the/notes', 0)", [id]))
            .await
            .unwrap();
        let in_a = |origin: &str, content: &str| ForeignNote { project: Some("a".into()), ..foreign_note(origin, "In a", content) };
        let notes = vec![in_a("engram:a1", "one"), in_a("engram:a2", "two"), foreign_note("engram:loose", "Loose", "three")];
        let options = ImportOptions { project_map: vec![("a".into(), root.into())], ..Default::default() };
        let (report, _) = run_foreign(&dst, notes, options).await;

        let own = crate::memory::identity::candidate_id(Path::new(root), 1);
        assert_eq!(report.by_project, BTreeMap::from([(String::new(), 1), (own.clone(), 2)]), "{report:#?}");
        let foreign = report.foreign.clone().unwrap();
        let in_project: Vec<Option<&str>> = foreign.by_source_project.iter().map(|p| p.project_id.as_deref()).collect();
        assert_eq!(in_project, vec![None, Some(own.as_str())], "the report names the project the notes went to");
        assert!(foreign.taken_projects.is_empty() && foreign.left_out.is_empty(), "{report:#?}");
    }

    #[tokio::test]
    async fn notes_mapped_to_clones_of_one_remote_share_its_project() {
        let dst = destination().await;
        dst.conn
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO project_path (id, root_path, created_at) VALUES ('app-1a2b', '/work/app', 0);
                     INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/app', 'app-1a2b', 0);",
                )
            })
            .await
            .unwrap();
        let clone = || {
            let dir = crate::test_dir::test_dir();
            std::fs::create_dir_all(dir.path().join(".git")).unwrap();
            std::fs::write(dir.path().join(".git").join("config"), "[remote \"origin\"]\n\turl = https://github.com/acme/app\n").unwrap();
            dir
        };
        let (one, two) = (clone(), clone());
        let root = |dir: &crate::test_dir::TestDir| dir.path().to_string_lossy().into_owned();
        let keyed = |origin: &str, project: &str, content: &str| ForeignNote {
            project: Some(project.into()),
            topic_key: Some("sdd/x".into()),
            ..foreign_note(origin, "Proposal", content)
        };
        let notes = vec![keyed("engram:a", "a", "the proposal of a"), keyed("engram:b", "b", "the proposal of b")];
        let options = ImportOptions { project_map: vec![("a".into(), root(&one).into()), ("b".into(), root(&two).into())], ..Default::default() };
        let (report, _) = run_foreign(&dst, notes, options).await;

        assert_eq!(report.by_project, BTreeMap::from([("app-1a2b".to_string(), 2)]), "{report:#?}");
        assert_eq!(report.by_state, BTreeMap::from([("live".to_string(), 2)]), "each key qualified by its project of the source");
        let foreign = report.foreign.clone().unwrap();
        let qualified: Vec<&str> = foreign.qualified_keys.iter().map(|k| k.to.as_str()).collect();
        assert_eq!(qualified, vec!["a/sdd/x", "b/sdd/x"]);
    }

    #[tokio::test]
    async fn a_second_import_adds_nothing_even_with_another_map() {
        let dst = destination().await;
        run_foreign(&dst, sample_notes(), ImportOptions::default()).await;
        let first = dump(&dst, EVERYTHING).await;

        let options = ImportOptions { project_map: vec![("a".into(), "/work/notes".into())], ..Default::default() };
        let (again, _) = run_foreign(&dst, sample_notes(), options).await;

        assert_eq!((again.inserted, again.present), (0, again.rows), "{again:#?}");
        assert!(again.merged.is_empty() && again.invalid.is_empty() && again.dropped_links.is_empty(), "{again:#?}");
        let second = dump(&dst, EVERYTHING).await;
        assert_eq!(first.memories, second.memories, "no row added and no access counted");
        assert_eq!((first.pieces, first.conflicts), (second.pieces, second.conflicts));
        let foreign = again.foreign.unwrap();
        assert_eq!(foreign.present, BTreeMap::from([(String::new(), 6)]), "counted where they are, not where the map puts them");
        assert!(foreign.revised.is_empty() && foreign.older_versions.is_empty() && foreign.merged.is_empty(), "{foreign:#?}");
    }

    #[tokio::test]
    async fn a_project_map_to_a_project_places_its_notes_there() {
        let dst = destination().await;
        exec(&dst, "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('notes-1111', 'Notes', 'user', 1, 1);").await;
        let found = map_project(&dst, "notes").await.unwrap();
        assert_eq!(found, MapProject::Live("notes-1111".into()));
        let options = ImportOptions { project_map: vec![("a".into(), MapTarget::Project { project: "notes-1111".into() })], ..Default::default() };

        let (report, _) = run_foreign(&dst, sample_notes(), options).await;

        let rows = dump(&dst, EVERYTHING).await.memories;
        let of_a: BTreeSet<Option<String>> = rows.iter().filter(|m| m.origin_project.as_deref() == Some("a")).map(|m| m.project_id.clone()).collect();
        assert_eq!(of_a, BTreeSet::from([Some("notes-1111".to_owned())]));
        assert_eq!(count(&dst, "SELECT COUNT(*) FROM project_path").await, 0, "no folder is recorded");
        assert_eq!(report.by_project.get("notes-1111").copied().unwrap_or_default(), report.inserted - report.by_project.get("").copied().unwrap_or_default());

        exec(&dst, "UPDATE project SET deleted_at = 5 WHERE id = 'notes-1111';").await;
        assert!(matches!(map_project(&dst, "notes-1111").await, Err(ImportError::ProjectDeleted { .. })));
        assert_eq!(map_project(&dst, "Notes").await.unwrap(), MapProject::Unknown, "a deleted name is free");
        assert_eq!(map_project(&dst, "nothing").await.unwrap(), MapProject::Unknown);
    }

    #[tokio::test]
    async fn a_project_map_path_in_a_declared_subfolder_goes_to_its_project() {
        let dst = destination().await;
        let dir = crate::test_dir::test_dir();
        let root = crate::memory::identity::strip_verbatim(&std::fs::canonicalize(dir.path()).unwrap());
        let inside = root.join("apps").join("web").join("src");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(&inside).unwrap();
        let home = crate::test_dir::test_dir();
        let workspaces = crate::memory::workspace::WorkspaceManager::new(home.path().join("workspaces"));
        let reg = crate::memory::project::Registration {
            root: &root,
            checkout: Disk.checkout(&root),
            legacy_ids: &[],
            index_docs: false,
            assigned: true,
            subpath: None,
        };
        let repository = crate::memory::project::register_project(&dst, &workspaces, home.path(), reg, Arc::new(Disk)).await.unwrap().project_id;
        let repository_id = repository.clone();
        dst.conn
            .call(move |c| {
                c.execute("INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('web-1111', 'web', 'user', 1, 1)", [])?;
                c.execute(
                    "INSERT INTO project_subfolder (parent_id, subpath, project_id, created_at) VALUES (?1, 'apps/web', 'web-1111', 1)",
                    [&repository_id],
                )
            })
            .await
            .unwrap();
        let target = |path: Option<&Path>| MapTarget::Folder {
            root: root.to_string_lossy().into_owned(),
            path: path.map(|p| p.to_string_lossy().into_owned()),
        };
        let options = ImportOptions {
            project_map: vec![("a".into(), target(Some(&inside))), ("b".into(), target(None))],
            ..Default::default()
        };

        run_foreign(&dst, sample_notes(), options).await;

        let rows = dump(&dst, EVERYTHING).await.memories;
        let of = |name: &str| -> BTreeSet<Option<String>> {
            rows.iter().filter(|m| m.origin_project.as_deref() == Some(name)).map(|m| m.project_id.clone()).collect()
        };
        assert_eq!(of("a"), BTreeSet::from([Some("web-1111".to_owned())]));
        assert_eq!(of("b"), BTreeSet::from([Some(repository)]), "the root alone is the repository's");
    }

    #[tokio::test]
    async fn a_moved_note_is_recognized_where_it_is() {
        let dst = destination().await;
        run_foreign(&dst, sample_notes(), ImportOptions::default()).await;
        let moved_to = register(&dst, "/work/moved").await;
        let backups = crate::test_dir::test_dir();
        let filter = MoveFilter { origin_project: Some("a".into()), include_deleted: true, ..Default::default() };
        let moved = move_memories_core(&dst, backups.path(), &filter, Some(moved_to.clone()), false).await.unwrap();
        assert!(!moved.moved.is_empty());
        let before = dump(&dst, EVERYTHING).await;

        let options = ImportOptions { project_map: vec![("a".into(), "/work/other".into())], ..Default::default() };
        let (again, _) = run_foreign(&dst, sample_notes(), options).await;

        assert_eq!((again.inserted, again.present), (0, again.rows), "{again:#?}");
        assert_eq!(dump(&dst, EVERYTHING).await.memories, before.memories);
        let present = again.foreign.unwrap().present;
        assert_eq!(present, BTreeMap::from([(String::new(), 2), (moved_to, 4)]));
    }

    #[tokio::test]
    async fn a_note_deleted_here_is_not_brought_back() {
        let dst = destination().await;
        let port = || foreign_note("engram:port", "Port", "the daemon listens on 27380");
        run_foreign(&dst, vec![port()], ImportOptions::default()).await;
        let id = imported(&dst, "engram:port").await[0].id;
        delete_memory_core(&dst, id, MemoryKind::Memory, None).await.unwrap();
        let before = dump(&dst, EVERYTHING).await;

        let rewritten = ForeignNote { written: When::At(T0 + 1000), ..foreign_note("engram:port", "Port", "the daemon listens on 27390") };
        for notes in [vec![port()], vec![rewritten]] {
            let (again, _) = run_foreign(&dst, notes, ImportOptions::default()).await;
            assert_eq!(again.inserted, 0, "{again:#?}");
            let foreign = again.foreign.unwrap();
            assert_eq!(foreign.present, BTreeMap::from([(String::new(), 1)]));
            assert!(foreign.revised.is_empty() && foreign.deleted_at_source.is_empty(), "{foreign:#?}");
            assert_eq!(dump(&dst, EVERYTHING).await.memories, before.memories);
        }
    }

    #[tokio::test]
    async fn a_note_rewritten_at_the_source_revises_the_stored_one_where_it_is() {
        let dst = destination().await;
        let words: Vec<String> = (0..700).map(|i| format!("w{i}")).collect();
        let long = |end: &str, written: i64| ForeignNote {
            written: When::At(written),
            ..foreign_note("engram:long", "Notes on clawg-ledger", &format!("{} {end}", words.join(" ")))
        };
        let proposal = |project: &str, content: &str, written: i64| ForeignNote {
            project: Some(project.into()),
            topic_key: Some("sdd/x".into()),
            written: When::At(written),
            ..foreign_note(&format!("engram:{project}"), "Proposal", content)
        };
        let first = vec![proposal("a", "the proposal of a", T0), proposal("b", "the proposal of b", T0), long("closing", T0)];
        run_foreign(&dst, first, ImportOptions::default()).await;
        let elsewhere = register(&dst, "/work/elsewhere").await;
        let backups = crate::test_dir::test_dir();
        let filter = MoveFilter { origin_project: Some("a".into()), ..Default::default() };
        move_memories_core(&dst, backups.path(), &filter, Some(elsewhere.clone()), false).await.unwrap();

        let rewritten = vec![proposal("a", "the second proposal of a", T0 + 1000), long("changed", T0 + 1000)];
        let (report, _) = run_foreign(&dst, rewritten, ImportOptions::default()).await;

        let a = imported(&dst, "engram:a").await;
        assert_eq!(a.len(), 2, "{a:#?}");
        let (old, new) = (&a[0], &a[1]);
        assert_eq!(
            (new.content.as_str(), new.status.as_str(), new.project.as_deref(), new.key.as_deref()),
            ("Proposal\nthe second proposal of a", "active", Some(elsewhere.as_str()), Some("a/sdd/x")),
            "where the stored version is, under its key"
        );
        assert_eq!((old.status.as_str(), old.by, old.cause.as_deref()), ("superseded", Some(new.id), Some("revision")));
        assert_eq!(imported(&dst, "engram:b").await[0].status, "active");

        let text = |end: &str| format!("Notes on clawg-ledger\n{} {end}", words.join(" "));
        let (before, after) = (pieces_of(&text("closing")), pieces_of(&text("changed")));
        assert_eq!(before[0], after[0], "the rewrite changes only the end");
        let rows = imported(&dst, "engram:long").await;
        let head = rows.iter().find(|r| r.content == after[0]).unwrap().id;
        let live: Vec<&str> = rows.iter().filter(|r| r.status == "active").map(|r| r.content.as_str()).collect();
        assert_eq!(live, after.iter().map(String::as_str).collect::<Vec<_>>(), "the pieces of the new version, the shared ones kept");
        for r in rows.iter().filter(|r| !after.contains(&r.content)) {
            assert_eq!((r.status.as_str(), r.by, r.cause.as_deref()), ("superseded", Some(head), Some("revision")), "{r:?}");
        }
        assert_eq!(rows.len(), before.len() + after.iter().filter(|p| !before.contains(p)).count(), "no text stored twice");
        let saves = saves(&dst).await;
        assert!(saves.contains(&before) && saves.contains(&after), "each version is a save of its text: {saves:#?}");

        assert_eq!(report.by_project.get(&elsewhere), Some(&1), "{report:#?}");
        let foreign = report.foreign.unwrap();
        assert_eq!(foreign.revised, ["engram:a", "engram:long"]);
        assert!(foreign.present.is_empty() && foreign.older_versions.is_empty(), "{foreign:#?}");
    }

    #[tokio::test]
    async fn an_older_version_enters_superseded() {
        let dst = destination().await;
        let older = || foreign_note("engram:port", "Port", "the daemon listens on 27380");
        let newer = ForeignNote { written: When::At(T0 + 1000), ..foreign_note("engram:port", "Port", "the daemon listens on 27390") };
        run_foreign(&dst, vec![newer], ImportOptions::default()).await;

        let (report, _) = run_foreign(&dst, vec![older()], ImportOptions::default()).await;

        let rows = imported(&dst, "engram:port").await;
        assert_eq!(rows.len(), 2, "{rows:#?}");
        let (live, old) = (&rows[0], &rows[1]);
        assert_eq!((live.status.as_str(), live.by), ("active", None));
        assert_eq!(
            (old.content.as_str(), old.status.as_str(), old.by, old.cause.as_deref()),
            ("Port\nthe daemon listens on 27380", "superseded", Some(live.id), Some("revision"))
        );
        assert_eq!((report.inserted, report.by_state.clone()), (1, BTreeMap::from([("superseded".to_string(), 1)])));
        assert_eq!(report.foreign.unwrap().older_versions, ["engram:port"]);

        let (again, _) = run_foreign(&dst, vec![older()], ImportOptions::default()).await;
        assert_eq!((again.inserted, again.present), (0, 1), "{again:#?}");
    }

    #[tokio::test]
    async fn a_note_deleted_at_the_source_changes_nothing() {
        let dst = destination().await;
        let port = || foreign_note("engram:port", "Port", "the daemon listens on 27380");
        run_foreign(&dst, vec![port()], ImportOptions::default()).await;
        let before = dump(&dst, EVERYTHING).await;

        let deleted = ForeignNote { deleted: Some(When::At(T0 + 5)), ..port() };
        let rewritten_then_deleted = ForeignNote {
            written: When::At(T0 + 1000),
            deleted: Some(When::At(T0 + 2000)),
            ..foreign_note("engram:port", "Port", "the daemon listens on 27390")
        };
        for note in [deleted, rewritten_then_deleted] {
            let (report, _) = run_foreign(&dst, vec![note], ImportOptions::default()).await;
            assert_eq!(report.inserted, 0, "{report:#?}");
            assert_eq!(dump(&dst, EVERYTHING).await.memories, before.memories);
            let foreign = report.foreign.unwrap();
            assert_eq!(foreign.deleted_at_source, ["engram:port"]);
            assert_eq!(foreign.present, BTreeMap::from([(String::new(), 1)]));
        }
        assert!(imported(&dst, "engram:port").await.iter().all(|r| !r.deleted));
    }

    #[tokio::test]
    async fn a_deleted_note_comes_in_deleted_without_a_vector() {
        let dst = destination().await;
        let gone = ForeignNote { deleted: Some(When::At(T0 + 5)), ..foreign_note("engram:gone", "Port", "the daemon listens on 27390") };
        let (report, _) = run_foreign(&dst, vec![gone], ImportOptions::default()).await;

        assert_eq!((report.by_state.clone(), report.foreign.as_ref().unwrap().deleted), (BTreeMap::from([("deleted".to_string(), 1)]), 1));
        let row = &imported(&dst, "engram:gone").await[0];
        assert!(row.deleted);
        let id = row.id;
        let vectors = dst
            .conn
            .call(move |c| c.query_row("SELECT count(*) FROM memory_vectors WHERE rowid = ?1", [id], |r| r.get::<_, i64>(0)))
            .await
            .unwrap();
        assert_eq!(vectors, 0);
        assert_eq!(count(&dst, "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH '27390'").await, 0);

        restore_memory_core(&dst, &FakeEmbedder::default(), MemoryKind::Memory, id, None).await.unwrap();
        assert_eq!(count(&dst, "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH '27390'").await, 1, "indexed once");
    }

    #[tokio::test]
    async fn relations_become_pairs() {
        let notes: Vec<ForeignNote> =
            ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"].iter().enumerate().map(|(k, n)| named(n, k as i64)).collect();
        let relations = vec![
            ForeignRelation { judgment: "pending".into(), ..relation("b", "a", "related") },
            relation("d", "c", "compatible"),
            ForeignRelation {
                reason: Some("each holds in its own crate".into()),
                evidence: Some("see Cargo.toml".into()),
                ..relation("f", "e", "scoped")
            },
            relation("h", "g", "conflicts_with"),
            relation("j", "i", "supersedes"),
            ForeignRelation { judgment: "ignored".into(), ..relation("l", "k", "conflicts_with") },
        ];
        let export = ForeignExport { relations, ..foreign_export(notes.clone()) };
        let dst = destination().await;
        let (report, _) = run_export(&dst, export.clone(), ImportOptions::default()).await;

        let text = |n: &str| format!("{n}\nthe note {n}");
        let pair = |new: &str, old: &str, state: &str, verdict: Option<&str>, reason: Option<&str>| {
            (text(new), text(old), state.to_string(), verdict.map(str::to_owned), reason.map(str::to_owned))
        };
        let expected = vec![
            pair("b", "a", "pending", None, None),
            pair("d", "c", "judged", Some("compatible"), None),
            pair("f", "e", "judged", Some("scoped"), Some("each holds in its own crate\nsee Cargo.toml")),
            pair("h", "g", "escalated", None, None),
            pair("j", "i", "judged", Some("supersedes"), None),
            pair("l", "k", "judged", Some("compatible"), None),
        ];
        assert_eq!(pairs(&dst).await, expected);
        let (g, h) = (&imported(&dst, "engram:g").await[0], &imported(&dst, "engram:h").await[0]);
        assert_eq!((h.status.as_str(), h.disputed_against.clone()), ("disputed", Some(format!("[{}]", g.id))));
        assert_eq!(g.status, "active");
        let (i, j) = (&imported(&dst, "engram:i").await[0], &imported(&dst, "engram:j").await[0]);
        assert_eq!((i.status.as_str(), i.by, i.cause.as_deref()), ("superseded", Some(j.id), Some("verdict")));
        let stored = report.foreign.unwrap().relations;
        let by_state = [("compatible", 2), ("escalated", 1), ("pending", 1), ("scoped", 1), ("supersedes", 1)];
        assert_eq!(stored, by_state.iter().map(|(s, n)| (s.to_string(), *n)).collect::<BTreeMap<_, _>>());

        let before = dump(&dst, EVERYTHING).await;
        let (again, _) = run_export(&dst, export, ImportOptions::default()).await;
        assert_eq!(again.inserted, 0);
        assert_eq!(dump(&dst, EVERYTHING).await, before, "a second import adds no pair");

        // A relation the source drew later between notes already here keeps
        // its pair and changes no row here.
        let dst = destination().await;
        run_foreign(&dst, notes.clone(), ImportOptions::default()).await;
        let later = ForeignExport { relations: vec![relation("j", "i", "supersedes")], ..foreign_export(notes) };
        run_export(&dst, later, ImportOptions::default()).await;
        assert_eq!(pairs(&dst).await, vec![pair("j", "i", "judged", Some("supersedes"), None)]);
        let i = &imported(&dst, "engram:i").await[0];
        assert_eq!((i.status.as_str(), i.by), ("active", None));
    }

    #[tokio::test]
    async fn supersedes_over_a_decision_without_the_user_is_escalated() {
        let notes = || vec![ForeignNote { source_type: Some("decision".into()), ..named("a", 0) }, named("b", 1)];
        let by_agent = ForeignExport { relations: vec![relation("b", "a", "supersedes")], ..foreign_export(notes()) };
        let dst = destination().await;
        run_export(&dst, by_agent, ImportOptions::default()).await;
        let (a, b) = (&imported(&dst, "engram:a").await[0], &imported(&dst, "engram:b").await[0]);
        assert_eq!(a.status, "active", "a decision is replaced only on the user's word");
        assert_eq!((b.status.as_str(), b.disputed_against.clone()), ("disputed", Some(format!("[{}]", a.id))));
        assert_eq!(pairs(&dst).await[0].2, "escalated");

        let by_user = ForeignExport {
            relations: vec![ForeignRelation { by_user: true, ..relation("b", "a", "supersedes") }],
            ..foreign_export(notes())
        };
        let dst = destination().await;
        run_export(&dst, by_user, ImportOptions::default()).await;
        let (a, b) = (&imported(&dst, "engram:a").await[0], &imported(&dst, "engram:b").await[0]);
        assert_eq!((a.status.as_str(), a.by, a.cause.as_deref()), ("superseded", Some(b.id), Some("verdict")));
        assert_eq!((pairs(&dst).await[0].2.as_str(), pairs(&dst).await[0].3.as_deref()), ("judged", Some("supersedes")));
    }

    #[tokio::test]
    async fn what_has_no_place_is_counted() {
        let notes = vec![
            named("a", 0),
            named("b", 1),
            named("c", 2),
            ForeignNote { written: When::At(T0 + 3), ..foreign_note("engram:a2", "a", "the note a") },
            ForeignNote { title: String::new(), content: String::new(), ..named("empty", 4) },
        ];
        let relations = vec![
            ForeignRelation { superseded: true, ..relation("b", "a", "related") },
            ForeignRelation { judgment: "orphaned".into(), ..relation("b", "a", "compatible") },
            relation("c", "missing", "related"),
            relation("c", "empty", "related"),
            relation("c", "b", "mentions"),
            relation("a2", "a", "related"),
            relation("c", "a", "related"),
            ForeignRelation { updated: When::At(T0 + 300), ..relation("a", "c", "conflicts_with") },
        ];
        let unplaced = BTreeMap::from([("observation.tool_name".to_string(), 2)]);
        let export = ForeignExport { relations, unplaced: unplaced.clone(), ..foreign_export(notes) };
        let dst = destination().await;
        let (report, _) = run_export(&dst, export, ImportOptions::default()).await;

        assert_eq!(pairs(&dst).await.into_iter().map(|p| p.2).collect::<Vec<_>>(), ["escalated"], "the newest relation of two notes");
        let foreign = report.foreign.unwrap();
        assert_eq!(foreign.relations, BTreeMap::from([("escalated".to_string(), 1)]));
        let left_out = [("missing_note", 2), ("orphaned", 1), ("replaced", 2), ("same_note", 1), ("unknown:mentions", 1)];
        assert_eq!(foreign.relations_left_out, left_out.iter().map(|(r, n)| (r.to_string(), *n)).collect::<BTreeMap<_, _>>());
        assert_eq!(foreign.unplaced, unplaced);
        assert_eq!(foreign.without_text, ["engram:empty"]);
    }

    #[tokio::test]
    async fn the_foreign_report_counts_what_the_import_did() {
        let root = "/work/notes";
        let here = crate::memory::identity::derive_project_id(Path::new(root));
        let token = format!("ghp_{}", "aB3".repeat(12));
        let in_a = |n: ForeignNote| ForeignNote { project: Some("a".into()), ..n };
        let notes = vec![
            in_a(ForeignNote { source_type: Some("decision".into()), ..named("decision", 0) }),
            in_a(ForeignNote { topic_key: Some("Sdd X".into()), ..named("keyed", 1) }),
            in_a(ForeignNote { content: "one writer per store, ".repeat(150), ..named("long", 2) }),
            ForeignNote { project: Some("b".into()), ..named("unmapped", 3) },
            ForeignNote { content: format!("deploy with {token}"), ..named("secret", 4) },
            ForeignNote { kind: NoteKind::Prompt, source_type: None, ..named("prompt", 5) },
            ForeignNote { written: When::Unreadable, ..named("undated", 6) },
            ForeignNote { deleted: Some(When::At(T0 + 7)), ..named("gone", 7) },
        ];
        let export = ForeignExport {
            relations: vec![relation("unmapped", "decision", "related")],
            unplaced: BTreeMap::from([("observation.tool_name".to_string(), 1)]),
            rewritten: 2,
            discarded_sessions: 1,
            ..foreign_export(notes)
        };
        let options = ImportOptions {
            project_map: vec![("a".into(), root.into()), ("c".into(), "/work/c".into())],
            ..Default::default()
        };
        let dst = destination().await;
        let (report, _) = run_export(&dst, export.clone(), options.clone()).await;

        let foreign = report.foreign.clone().unwrap();
        assert_eq!((foreign.source.as_str(), foreign.version.as_str(), foreign.notes), ("engram", "0.2.0", 8));
        let types = |kind: &str, source_type: Option<&str>, memory_type: &str, notes| TypeCount {
            kind: kind.into(),
            source_type: source_type.map(str::to_owned),
            memory_type: memory_type.into(),
            notes,
        };
        assert_eq!(
            foreign.by_source_type,
            [
                types("observation", Some("decision"), "architectural_decision", 1),
                types("observation", Some("discovery"), "semantic_fact", 6),
                types("prompt", None, "chat_message", 1),
            ]
        );
        let project = |name: Option<&str>, root: Option<&str>, project_id: Option<&str>, notes| ProjectCount {
            project: name.map(str::to_owned),
            personal: false,
            root: root.map(str::to_owned),
            project_id: project_id.map(str::to_owned),
            notes,
        };
        assert_eq!(
            foreign.by_source_project,
            [project(None, None, None, 4), project(Some("a"), Some(root), Some(&here), 3), project(Some("b"), None, None, 1)]
        );
        assert_eq!((foreign.unmapped.clone(), foreign.unused_map.clone()), (vec!["b".to_string()], vec!["c".to_string()]));
        assert_eq!((foreign.rows, foreign.split_notes, foreign.masked_notes), (report.rows, 1, 1));
        assert_eq!(foreign.rewritten_keys, [KeyChange { from: "Sdd X".into(), to: rewrite_key("Sdd X") }]);
        assert_eq!((foreign.clamped_dates.clone(), foreign.deleted), (vec!["engram:undated".to_string()], 1));
        assert_eq!(foreign.relations, BTreeMap::from([("compatible".to_string(), 1)]));
        assert_eq!((foreign.unplaced.clone(), foreign.rewritten_at_source, foreign.discarded_sessions), (export.unplaced.clone(), 2, 1));
        assert!(foreign.present.is_empty() && foreign.left_out.is_empty() && foreign.merged.is_empty(), "{foreign:#?}");
        assert_eq!(report.by_project[&here], report.rows - 5, "the rows of the notes of a");
        assert_eq!(report.by_lang.values().sum::<usize>(), report.inserted);

        let (again, _) = run_export(&dst, export, options).await;
        assert_eq!(again.inserted, 0);
        let present = again.foreign.unwrap().present;
        assert_eq!(present, BTreeMap::from([(String::new(), 5), (here, 3)]));
    }

    #[test]
    fn write_batches_never_split_a_note() {
        assert_eq!(note_batches(&[100, 100, 100, 10, 300, 1], 256), vec![0..200, 200..310, 310..610, 610..611]);
        assert_eq!(note_batches(&[1; 5], 2), vec![0..2, 2..4, 4..5], "a clawg file's rows are units of one");
        assert!(note_batches(&[], 256).is_empty());
    }

    #[test]
    fn a_project_taken_while_importing_is_told_by_its_notes() {
        let (id, root, other) = ("notes-1a2b", "/work/notes", "/not/the/notes");
        let notes = vec![
            NoteRows { origin: "engram:a1".into(), project_id: Some(id.into()), rows: 1..2, masked: false, undated: false },
            NoteRows { origin: "engram:a2".into(), project_id: Some(id.into()), rows: 2..3, masked: false, undated: false },
        ];
        let ledger = Ledger { taken: HashMap::from([(id.to_string(), other.to_string())]), ..Default::default() };
        let invalid = |line| Invalid { line, reason: format!("project id {id} already belongs to {other}") };
        let report = ImportReport { invalid: vec![invalid(1), invalid(2)], ..Default::default() };
        let foreign = ForeignReport {
            by_source_project: vec![ProjectCount {
                project: Some("a".into()),
                personal: false,
                root: Some(root.into()),
                project_id: Some(id.into()),
                notes: 2,
            }],
            ..Default::default()
        };

        let told = with_foreign(report, foreign, &notes, &ledger);

        let foreign = told.foreign.unwrap();
        let taken = TakenProject { project: "a".into(), root: root.into(), belongs_to: other.into(), notes: 2 };
        assert_eq!(foreign.taken_projects, vec![taken]);
        assert!(told.invalid.is_empty() && foreign.left_out.is_empty(), "told by project, not by note: {foreign:#?}");
    }

    #[tokio::test]
    async fn a_store_from_v16_travels_whole() {
        let dir = crate::test_dir::test_dir();
        seed_on_disk(dir.path(), Some(16)).await;
        let exported = crate::memory::portability::export(dir.path(), &ExportFilter::default(), 0).unwrap();
        let dst = destination().await;
        let (report, _) = run(&dst, &exported.to_jsonl(), ImportOptions::default()).await;
        assert_eq!(report.inserted, 1);
        let row = by_content(&dump(&dst, EVERYTHING).await)["seeded on disk"].clone();
        let source = &exported.memories[0];
        assert_eq!(
            (row.agent_id, row.memory_type, row.status, row.created_at, row.access_count, row.tier, row.source),
            (source.agent_id.clone(), source.memory_type.clone(), source.status.clone(), source.created_at, source.access_count, source.tier, source.source.clone())
        );
    }

    /// Each runs once per row or per save of a file, when the pass that
    /// decides loads the store or when the one that writes places its rows,
    /// so a scan makes an import grow with the square of its rows.
    #[tokio::test]
    async fn the_import_lookups_seek_an_index() {
        use rusqlite::types::Value::{Integer, Null, Text};
        let store = MemoryStore::new_for_test().await.unwrap();
        seed(&store).await;
        let hash = || Text(content_hash("Storage engine: SQLite with WAL"));
        let cases = [
            ("ORIGIN_SQL", ORIGIN_SQL, vec![Text("engram:a".into())], "idx_memory_import_origin"),
            ("PRESENT_SQL in a project", PRESENT_SQL, vec![Text("claude-code".into()), Text(project()), hash(), Integer(1)], "idx_memory_dedup"),
            ("PRESENT_SQL without a project", PRESENT_SQL, vec![Text(MIGRATED_AGENT.into()), Null, hash(), Integer(1)], "idx_memory_dedup"),
            ("TWIN_SQL in a project", TWIN_SQL, vec![Text(project()), hash()], "idx_memory_dedup"),
            ("TWIN_SQL without a project", TWIN_SQL, vec![Null, hash()], "idx_memory_dedup"),
            ("TOPIC_HOLDERS_SQL", TOPIC_HOLDERS_SQL, vec![Text(project()), Text("storage/engine".into()), Text("engram:a".into())], "idx_memory_topic"),
            ("REVISED_SQL", REVISED_SQL, vec![Integer(-1), Integer(-2)], "INTEGER PRIMARY KEY"),
            ("LINKED_SQL", LINKED_SQL, vec![Integer(5)], "idx_memory_piece_memory"),
            ("NEXT_SAVE_SQL", NEXT_SAVE_SQL, vec![], "PRIMARY KEY"),
            ("ORIGIN_ROWS_SQL", ORIGIN_ROWS_SQL, vec![Text("engram:a".into())], "idx_memory_import_origin"),
            ("TEXT_ROWS_SQL in a project", TEXT_ROWS_SQL, vec![Text(project()), hash()], "idx_memory_dedup"),
            ("TEXT_ROWS_SQL without a project", TEXT_ROWS_SQL, vec![Text(String::new()), hash()], "idx_memory_dedup"),
            ("KEY_ROWS_SQL", KEY_ROWS_SQL, vec![Text(project()), Text("storage/engine".into())], "idx_memory_topic"),
            ("PRESENT_ELSEWHERE_SQL", PRESENT_ELSEWHERE_SQL, vec![Integer(1), Text("claude-code".into()), hash(), Text("claude-code".into())], "idx_memory_created_at"),
            ("DATE_ROWS_SQL", DATE_ROWS_SQL, vec![Integer(1)], "idx_memory_created_at"),
            ("PROJECT_ROOT_SQL", PROJECT_ROOT_SQL, vec![Text(project())], "sqlite_autoindex_project_path_1"),
        ];
        let mut wrong = Vec::new();
        for (name, sql, binds, index) in cases {
            let (plan, steps) = store
                .conn
                .call(move |c| -> rusqlite::Result<(Vec<String>, i32)> {
                    let plan: Vec<String> = c
                        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?
                        .query_map(rusqlite::params_from_iter(&binds), |r| r.get(3))?
                        .collect::<rusqlite::Result<_>>()?;
                    let mut stmt = c.prepare(sql)?;
                    let mut rows = stmt.query(rusqlite::params_from_iter(&binds))?;
                    while rows.next()?.is_some() {}
                    drop(rows);
                    Ok((plan, stmt.get_status(rusqlite::StatementStatus::FullscanStep)))
                })
                .await
                .unwrap();
            let seeks = plan.iter().any(|l| l.starts_with("SEARCH") && l.contains(index));
            // `SELECT EXISTS (…)` reads the one row of a SELECT without FROM.
            let scans = plan.iter().any(|l| l.starts_with("SCAN") && l != "SCAN CONSTANT ROW");
            if !seeks || scans || steps != 0 {
                wrong.push(format!("{name} should seek `{index}`: {plan:?}, {steps} full scan steps"));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    async fn file_of(store: &MemoryStore, text: &str, options: ImportOptions) -> Arc<Prepared> {
        prepare_file(store, text, &options, Arc::new(Disk), &mut ImportTimings::new(false)).await.unwrap()
    }

    async fn notes_of(store: &MemoryStore, notes: Vec<ForeignNote>, options: ImportOptions) -> Arc<Prepared> {
        prepare_export(store, Arc::new(ChatChunker::with_fallback()), foreign_export(notes), &options, Arc::new(Disk), &mut ImportTimings::new(false)).await.unwrap().0
    }

    /// What the pass that decided on the connection made of `prepared`: the
    /// rules on a transaction that is dropped.
    async fn decided_by_sql(store: &MemoryStore, prepared: &Arc<Prepared>) -> Ledger {
        let prepared = prepared.clone();
        store
            .conn
            .call(move |c| -> rusqlite::Result<Ledger> {
                let tx = c.transaction()?;
                let mut journal = Journal { id: None, source: "clawg", started_at: 0, snapshot: None, inserted: HashSet::new(), running: Arc::default(), revived: BTreeSet::new() };
                let mut ledger = Ledger { taken: record_projects(&tx, &prepared, 0, &mut journal)?, ..Default::default() };
                place_rows(&mut SqlSeat(&tx, &mut journal), &prepared, 0..prepared.rows.len(), &mut ledger, &HashMap::new(), false)?;
                write_links(&mut SqlSeat(&tx, &mut journal), &prepared, &mut ledger)?;
                Ok(ledger)
            })
            .await
            .unwrap()
    }

    async fn decides_alike(store: &MemoryStore, prepared: Arc<Prepared>, case: &str) -> Ledger {
        let by_sql = decided_by_sql(store, &prepared).await;
        let planned = load(store, &prepared, &mut Timing::new(Step::Load)).await.unwrap();
        let by_plan = decide(planned, &prepared, &AtomicBool::new(false)).unwrap();
        assert_eq!(by_plan, by_sql, "{case}");
        by_plan
    }

    #[tokio::test]
    async fn the_planner_decides_as_the_sql_pass() {
        let file = seeded_file().await;
        let plain = ImportOptions::default;

        let dst = destination().await;
        decides_alike(&dst, file_of(&dst, &file, plain()).await, "a clawg file").await;
        run(&dst, &file, plain()).await;
        decides_alike(&dst, file_of(&dst, &file, plain()).await, "the same file again").await;
        dst.conn
            .call(|c| {
                c.execute_batch(
                    "UPDATE memory SET superseded_by = NULL, revived_from = NULL, disputed_against = NULL;
                     DELETE FROM memory_piece; DELETE FROM memory_derivation; DELETE FROM memory_conflict;",
                )
            })
            .await
            .unwrap();
        decides_alike(&dst, file_of(&dst, &file, plain()).await, "links lost between transactions").await;

        let dst = destination().await;
        insert_row(&dst, "cursor", Some(project()), "Storage engine: SQLite with WAL", None, 5).await;
        let ledger = decides_alike(&dst, file_of(&dst, &file, plain()).await, "a live twin").await;
        assert!(ledger.placed.values().any(|p| matches!(p, Placement::Merged(_))));

        let dst = destination().await;
        insert_row(&dst, "claude-code", Some(project()), "Storage engine: DuckDB", Some("storage/engine"), 5).await;
        let ledger = decides_alike(&dst, file_of(&dst, &file, plain()).await, "a taken key").await;
        assert!(ledger.placed.values().any(|p| matches!(p, Placement::TopicTaken { .. })));
        let options = ImportOptions { agent: Some("dev".into()), ..Default::default() };
        decides_alike(&dst, file_of(&dst, &file, options).await, "every row under one agent").await;

        let dst = destination().await;
        for _ in 0..2 {
            insert_row(&dst, "cursor", Some(project()), "Storage engine: SQLite with WAL", None, 5).await;
        }
        decides_alike(&dst, file_of(&dst, &file, plain()).await, "two live rows of a text and a date").await;

        let dst = destination().await;
        let purged = insert_row(&dst, "cursor", None, "A row purged since", None, 2).await;
        dst.conn.call(move |c| c.execute("DELETE FROM memory WHERE id = ?1", [purged])).await.unwrap();
        decides_alike(&dst, file_of(&dst, &file, plain()).await, "a store whose last row was purged").await;

        let dst = destination().await;
        exec(&dst, MERGED_HERE).await;
        exec(&dst, "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('ideas-9a9a', 'Ideas', 'agent', 1, 1);").await;
        insert_row(&dst, "claude-code", Some("z-2222".into()), "A note of x", None, 1_750_000_000_001).await;
        let by_remote = r#"{"kind":"project","id":"x-9f9f","root_path":"/far/x","name":"x","remotes":["github.com/acme/x"]}"#;
        let by_name = r#"{"kind":"project","id":"ideas-0000","name":"ideas"}"#;
        let rows = [(1, Some("x-9f9f"), "A note of x"), (2, Some("ideas-0000"), "An idea"), (3, Some("x-9f9f"), "Another note of x")];
        let prepared = file_of(&dst, &clawg_file(&[by_remote, by_name], &rows), plain()).await;
        let ids: Vec<Option<&str>> = prepared.rows.iter().map(|r| r.project.as_deref()).collect();
        assert_eq!(ids, vec![Some("z-2222"), Some("ideas-9a9a"), Some("z-2222")]);
        let ledger = decides_alike(&dst, prepared, "projects by their remote and by their name").await;
        assert!(ledger.placed.values().any(|p| matches!(p, Placement::Present(_))), "the row is found in the project of the remote");

        let dst = destination().await;
        run(&dst, &file, plain()).await;
        exec(&dst, "UPDATE memory SET project_id = 'moved-1111' WHERE id = (SELECT MIN(row_id) FROM memory_import_row)").await;
        let ledger = decides_alike(&dst, file_of(&dst, &file, plain()).await, "a row moved since").await;
        assert!(ledger.placed.values().any(|p| matches!(p, Placement::Elsewhere(_))));

        let dst = destination().await;
        decides_alike(&dst, notes_of(&dst, sample_notes(), plain()).await, "notes of another tool").await;
        run_foreign(&dst, sample_notes(), plain()).await;
        decides_alike(&dst, notes_of(&dst, sample_notes(), plain()).await, "the same notes again").await;

        let dst = destination().await;
        let port = |content: &str, written: i64| ForeignNote { written: When::At(written), ..foreign_note("engram:port", "Port", content) };
        run_foreign(&dst, vec![port("the daemon listens on 27380", T0)], plain()).await;
        let cases = [
            (vec![port("the daemon listens on 27390", T0 + 1000)], "a newer version"),
            (vec![port("the daemon listens on 27370", T0 - 1000)], "an older version"),
            (vec![ForeignNote { deleted: Some(When::At(T0 + 5)), ..port("the daemon listens on 27380", T0) }], "deleted at the source"),
        ];
        for (notes, case) in cases {
            decides_alike(&dst, notes_of(&dst, notes, plain()).await, case).await;
        }
        let id = imported(&dst, "engram:port").await[0].id;
        delete_memory_core(&dst, id, MemoryKind::Memory, None).await.unwrap();
        decides_alike(&dst, notes_of(&dst, vec![port("the daemon listens on 27380", T0)], plain()).await, "deleted here").await;

        let dst = destination().await;
        let notes = vec![
            named("a", 1),
            ForeignNote { written: When::At(T0 + 2), ..foreign_note("engram:a", "a", "the note a, again") },
            named("b", 3),
            ForeignNote { written: When::At(T0 + 4), ..foreign_note("engram:c", "b", "the note b") },
        ];
        decides_alike(&dst, notes_of(&dst, notes, plain()).await, "two notes of an origin and a text twice").await;

        let dst = destination().await;
        let dir = crate::test_dir::test_dir();
        let options = ImportOptions { project_map: vec![("a".into(), dir.path().to_string_lossy().into_owned().into())], ..Default::default() };
        let prepared = notes_of(&dst, sample_notes(), options).await;
        let root_id = prepared.projects[0].root_id.clone();
        dst.conn
            .call(move |c| c.execute("INSERT INTO project_path (id, root_path, created_at) VALUES (?1, '/work/elsewhere', 0)", [root_id]))
            .await
            .unwrap();
        let ledger = decides_alike(&dst, prepared, "a project another root took").await;
        assert_eq!(ledger.taken.len(), 1);
    }

    #[tokio::test]
    async fn a_dry_run_leaves_the_connection_to_other_calls() {
        const NOTES: usize = 10_000;
        let store = MemoryStore::new_for_test().await.unwrap();
        let notes = (0..NOTES).map(|k| named(&format!("n{k}"), k as i64)).collect();
        let done = AtomicBool::new(false);
        let import = async {
            let imported = run_foreign(&store, notes, ImportOptions { dry_run: true, ..Default::default() }).await;
            done.store(true, Ordering::Relaxed);
            imported
        };
        let probe = async {
            let mut worst = std::time::Duration::ZERO;
            while !done.load(Ordering::Relaxed) {
                let started = std::time::Instant::now();
                store.conn.call(|c| c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))).await.unwrap();
                worst = worst.max(started.elapsed());
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            worst
        };
        let ((report, _), worst) = tokio::join!(import, probe);
        assert_eq!(report.inserted, NOTES);
        assert!(worst < std::time::Duration::from_millis(300), "a call waited {worst:?} behind the dry run");
    }

    #[tokio::test]
    async fn a_stopped_planner_decides_nothing() {
        let dst = destination().await;
        let prepared = file_of(&dst, &seeded_file().await, ImportOptions::default()).await;
        let planned = load(&dst, &prepared, &mut Timing::new(Step::Load)).await.unwrap();
        assert!(decide(planned, &prepared, &AtomicBool::new(true)).is_none());
    }

    #[tokio::test]
    async fn a_dry_run_after_a_purge_reports_the_ids_of_the_import() {
        let file = seeded_file().await;
        let dst = destination().await;
        insert_row(&dst, "claude-code", Some(project()), "Storage engine: DuckDB", Some("storage/engine"), 5).await;
        let purged = insert_row(&dst, "claude-code", None, "A row purged since", None, 6).await;
        dst.conn.call(move |c| c.execute("DELETE FROM memory WHERE id = ?1", [purged])).await.unwrap();
        let backups = crate::test_dir::test_dir();
        let dry = import(&dst, None, Arc::new(ChatChunker::with_fallback()), backups.path(), &file, &ImportOptions { dry_run: true, ..Default::default() }, Arc::new(Disk), &mut ImportTimings::new(false)).await.unwrap();
        assert!(dry.topic_taken.iter().all(|t| t.stored_as > purged) && !dry.topic_taken.is_empty(), "{dry:#?}");
        let (real, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!(ImportReport { dry_run: false, snapshot: real.snapshot.clone(), import_id: real.import_id, ..dry }, real);
    }

    fn steps(timings: &ImportTimings) -> Vec<Step> {
        timings.steps().iter().map(|t| t.step).collect()
    }

    fn timing_of(timings: &ImportTimings, step: Step) -> &Timing {
        timings.steps().iter().find(|t| t.step == step).unwrap()
    }

    async fn timed_foreign(store: &MemoryStore, notes: Vec<ForeignNote>, dry_run: bool) -> (ImportReport, ImportTimings) {
        let backups = crate::test_dir::test_dir();
        let chunker = Arc::new(ChatChunker::with_fallback());
        let options = ImportOptions { dry_run, ..Default::default() };
        let mut timings = ImportTimings::new(dry_run);
        let export = foreign_export(notes);
        let report = import_foreign(store, Some(&FakeEmbedder::default()), chunker, backups.path(), export, &options, Arc::new(Disk), &mut timings)
            .await
            .unwrap();
        (report, timings)
    }

    async fn timed_file(store: &MemoryStore, file: &str) -> (ImportReport, ImportTimings) {
        let backups = crate::test_dir::test_dir();
        let mut timings = ImportTimings::new(false);
        let report =
            import(store, Some(&FakeEmbedder::default()), Arc::new(ChatChunker::with_fallback()), backups.path(), file, &ImportOptions::default(), Arc::new(Disk), &mut timings)
                .await
                .unwrap();
        (report, timings)
    }

    #[tokio::test]
    async fn an_import_times_each_step_once_in_order() {
        let dst = destination().await;
        let (report, timings) = timed_foreign(&dst, sample_notes(), false).await;
        use Step::*;
        assert_eq!(steps(&timings), [Plan, Translate, Load, Decide, Embed, Snapshot, Write, Links]);
        let deleted = report.by_state.get("deleted").copied().unwrap_or(0);
        assert!(deleted > 0, "{report:#?}");
        assert_eq!(timing_of(&timings, Embed).rows, Some(report.inserted - deleted));
        assert_eq!(timing_of(&timings, Translate).rows, Some(report.rows));
        assert_eq!(timing_of(&timings, Decide).rows, Some(report.rows));
        assert_eq!(timing_of(&timings, Write).rows, Some(report.rows));
        assert_eq!(timing_of(&timings, Links).calls.map(|c| c.count), Some(1));
        assert_eq!(timing_of(&timings, Snapshot).calls, None);
    }

    #[tokio::test]
    async fn a_clawg_file_is_prepared_rather_than_translated() {
        let dst = destination().await;
        let (report, timings) = timed_file(&dst, &seeded_file().await).await;
        use Step::*;
        assert_eq!(steps(&timings), [Plan, Prepare, Load, Decide, Embed, Snapshot, Write, Links]);
        assert_eq!(timing_of(&timings, Prepare).rows, Some(report.rows));
    }

    #[tokio::test]
    async fn a_dry_run_times_only_what_it_runs() {
        let dst = destination().await;
        let (_, timings) = timed_foreign(&dst, sample_notes(), true).await;
        use Step::*;
        assert_eq!(steps(&timings), [Plan, Translate, Load, Decide]);
    }

    #[tokio::test]
    async fn time_in_sqlite_never_exceeds_the_call() {
        const ROWS: usize = 600;
        let src = MemoryStore::new_for_test().await.unwrap();
        src.conn
            .call(|c| {
                for k in 0..ROWS {
                    c.execute(
                        "INSERT INTO memory (scope, agent_id, content, memory_type, source, created_at, created_by_type, \
                         created_by_ref, last_modified_at, last_accessed_at, access_count, importance, tier, status, \
                         content_hash) \
                         VALUES ('agent', 'claude-code', ?1, 'semantic_fact', 'explicit', ?2, 'agent', 'claude-code', ?2, \
                         ?2, 1, 5.0, 3, 'active', content_hash(?1))",
                        rusqlite::params![format!("The row number {k}"), k as i64],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .unwrap();
        let file = dump(&src, EVERYTHING).await.to_jsonl();
        let dst = MemoryStore::new_for_test().await.unwrap();
        let (report, timings) = timed_file(&dst, &file).await;
        assert_eq!(report.inserted, ROWS);

        let load = timing_of(&timings, Step::Load);
        let texts = ROWS.div_ceil(LOAD_BATCH);
        assert_eq!(load.calls.map(|c| c.count), Some(texts + texts + 1), "the texts, the dates, then the roots");
        let write = timing_of(&timings, Step::Write);
        assert_eq!(write.calls.map(|c| c.count), Some(note_batches(&[1; ROWS], WRITE_BATCH).len()));
        for timing in [load, write] {
            let calls = timing.calls.unwrap();
            assert!(calls.in_sqlite + calls.queued <= timing.elapsed, "{timing}");
            assert!(timing.slowest.unwrap() <= calls.in_sqlite, "{timing}");
        }
    }

    /// An import's time grows with its rows: twice the notes, of one row each,
    /// take at most three times as long. Only runs of one build compare.
    #[tokio::test]
    #[ignore = "a benchmark: run it with `cargo test -p clawg-core --release --lib -- --ignored twice_the_notes --nocapture`"]
    async fn twice_the_notes_import_in_at_most_three_times_the_time() {
        const NOTES: usize = 10_000;
        async fn timed(count: usize) -> (std::time::Duration, ImportReport) {
            let store = MemoryStore::new_for_test().await.unwrap();
            let notes = (0..count).map(|k| named(&format!("n{k}"), k as i64)).collect();
            let started = std::time::Instant::now();
            let (report, _) = run_foreign(&store, notes, ImportOptions::default()).await;
            let elapsed = started.elapsed();
            // The snapshot's path carries the time it was written; without it
            // the report's hash is the same in every run that imports alike.
            let report = ImportReport { snapshot: None, import_id: None, ..report };
            println!("{count} notes in {elapsed:?}, report {}", content_hash(&serde_json::to_string(&report).unwrap()));
            (elapsed, report)
        }
        let (once, small) = timed(NOTES).await;
        let (twice, large) = timed(2 * NOTES).await;
        assert_eq!((small.inserted, large.inserted), (NOTES, 2 * NOTES));
        let ratio = twice.as_secs_f64() / once.as_secs_f64();
        assert!(ratio <= 3.0, "{NOTES} notes in {once:?}, twice as many in {twice:?}: {ratio:.2} times");
    }

    #[test]
    fn longest_first_puts_the_most_tokens_first_and_keeps_ties_in_order() {
        assert_eq!(longest_first(&[3, 9, 3, 1, 9]), vec![1, 4, 0, 2, 3]);
        assert_eq!(longest_first(&[]), Vec::<usize>::new());
        assert_eq!(longest_first(&[5, 5, 5]), vec![0, 1, 2]);
    }

    /// Answers as the fake embedder does, noting the texts of every call.
    struct Recording {
        calls: std::sync::Mutex<Vec<Vec<String>>>,
        fake: FakeEmbedder,
    }

    impl Recording {
        fn new() -> Self {
            Self { calls: std::sync::Mutex::default(), fake: FakeEmbedder::default() }
        }

        fn texts(&self) -> Vec<String> {
            self.calls.lock().unwrap().iter().flatten().cloned().collect()
        }
    }

    #[async_trait::async_trait]
    impl Embedder for Recording {
        async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            self.fake.embed_with_role(text, role).await
        }
        async fn embed_batch(&self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
            self.calls.lock().unwrap().push(texts.to_vec());
            self.fake.embed_batch(texts, role).await
        }
        fn dimensions(&self) -> usize {
            self.fake.dimensions()
        }
        fn model_id(&self) -> &str {
            self.fake.model_id()
        }
        fn query_prefix(&self) -> &str {
            self.fake.query_prefix()
        }
        fn passage_prefix(&self) -> &str {
            self.fake.passage_prefix()
        }
    }

    fn word_chunker() -> Arc<ChatChunker> {
        Arc::new(ChatChunker::with_tokenizer(crate::memory::chunker::word_tokenizer()))
    }

    /// `count` notes whose texts are `note<i>` and a few more words, of
    /// lengths that repeat and differ.
    fn word_notes(count: usize) -> Vec<ForeignNote> {
        (0..count)
            .map(|i| {
                let words = 3 + (i * 5) % 11;
                let body: Vec<String> = std::iter::once(format!("note{i}")).chain((1..words).map(|k| format!("x{k}"))).collect();
                foreign_note(&format!("engram:n{i:03}"), "t", &body.join(" "))
            })
            .collect()
    }

    async fn import_notes_with(store: &MemoryStore, embedder: &dyn Embedder, notes: Vec<ForeignNote>) -> ImportReport {
        let backups = crate::test_dir::test_dir();
        import_foreign(
            store,
            Some(embedder),
            word_chunker(),
            backups.path(),
            foreign_export(notes),
            &ImportOptions::default(),
            Arc::new(Disk),
            &mut ImportTimings::new(false),
        )
        .await
        .unwrap()
    }

    fn assert_longest_first(embedder: &Recording) {
        let calls = embedder.calls.lock().unwrap();
        assert!(calls.iter().all(|c| c.len() <= EMBED_BATCH), "no call over {EMBED_BATCH} texts");
        let texts: Vec<&str> = calls.iter().flatten().map(String::as_str).collect();
        let counts = word_chunker().count_all(&texts);
        assert!(counts.windows(2).all(|w| w[0] >= w[1]), "{counts:?}");
    }

    #[tokio::test]
    async fn an_import_of_another_tool_embeds_its_rows_longest_first() {
        let dst = destination().await;
        let embedder = Recording::new();
        let report = import_notes_with(&dst, &embedder, word_notes(40)).await;
        assert_eq!(report.inserted, 40);
        assert!(embedder.calls.lock().unwrap().len() >= 2, "more rows than one call takes");
        assert_longest_first(&embedder);
        let texts = embedder.texts();
        assert_eq!(texts.len(), 40);
        let counts = word_chunker().count_all(&texts.iter().map(String::as_str).collect::<Vec<_>>());
        let file_index = |t: &String| t.split_whitespace().find_map(|w| w.strip_prefix("note")).unwrap().parse::<usize>().unwrap();
        let mut ties = 0;
        for k in 1..texts.len() {
            if counts[k - 1] == counts[k] {
                ties += 1;
                assert!(file_index(&texts[k - 1]) < file_index(&texts[k]), "equal lengths keep the order of the file");
            }
        }
        assert!(ties > 0);
    }

    #[tokio::test]
    async fn a_clawg_import_embeds_its_rows_longest_first() {
        let file = seeded_file().await;
        let dst = destination().await;
        let embedder = Recording::new();
        let backups = crate::test_dir::test_dir();
        import(&dst, Some(&embedder), word_chunker(), backups.path(), &file, &ImportOptions::default(), Arc::new(Disk), &mut ImportTimings::new(false))
            .await
            .unwrap();
        assert!(embedder.texts().len() > 1);
        assert_longest_first(&embedder);
    }

    #[tokio::test]
    async fn each_imported_row_keeps_the_vector_of_its_own_text() {
        let long: Vec<String> = (0..700).map(|i| format!("w{i}")).collect();
        let mut notes = word_notes(40);
        notes.push(foreign_note("engram:long", "Long", &long.join(" ")));
        let dst = destination().await;
        import_notes_with(&dst, &Recording::new(), notes).await;
        let stored = dst
            .conn
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT m.content, COALESCE(p.context, ''), v.embedding FROM memory m \
                     JOIN memory_vectors v ON v.rowid = m.id LEFT JOIN memory_piece p ON p.memory_id = m.id \
                     WHERE m.source = 'import' AND m.deleted_at IS NULL",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Vec<u8>>(2)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap();
        assert!(stored.len() > 41, "the long note is several rows: {}", stored.len());
        assert!(stored.iter().any(|(_, context, _)| !context.is_empty()), "pieces behind a head");
        let fake = FakeEmbedder::default();
        for (content, context, bytes) in stored {
            let vector = fake.embed_with_role(&embedding_text(&context, &content), EmbedRole::Passage).await.unwrap();
            let expected: Vec<u8> = to_unit(&vector).unwrap().iter().flat_map(|f| f.to_le_bytes()).collect();
            assert_eq!(bytes, expected, "the vector of {content:.30}");
        }
    }

    /// Tokens the model runs over when rows of these lengths are embedded in
    /// this order, as the import calls the embedder and the embedder cuts its
    /// runs, against the tokens of the texts alone; and the same for the
    /// attention, which grows with the square of a run's length.
    fn padding(lens: &[usize]) -> (f64, f64) {
        let (mut sent, mut real, mut sent_sq, mut real_sq) = (0usize, 0usize, 0usize, 0usize);
        for call in lens.chunks(EMBED_BATCH) {
            for run in call.chunks(crate::memory::embedder::EMBED_BATCH) {
                let longest = run.iter().copied().max().unwrap_or(0);
                sent += run.len() * longest;
                sent_sq += run.len() * longest * longest;
                real += run.iter().sum::<usize>();
                real_sq += run.iter().map(|l| l * l).sum::<usize>();
            }
        }
        (sent as f64 / real.max(1) as f64, sent_sq as f64 / real_sq.max(1) as f64)
    }

    #[test]
    fn padding_counts_each_run_of_the_embedder_apart() {
        assert_eq!(padding(&[4; 40]), (1.0, 1.0));
        let mut lens = vec![1; 16];
        lens[15] = 9;
        assert_eq!(padding(&lens), (16.0 * 9.0 / 24.0, 16.0 * 81.0 / 96.0));
        let (two_runs, _) = padding(&[[1; 16], [9; 16]].concat());
        assert_eq!(two_runs, 1.0, "a run is padded to its own longest text");
    }

    /// What the embedding phase of an import of another tool's export is made
    /// of, over the rows that export gives an empty store: how many tokens
    /// each order of the rows sends to the model and, with the model, how long
    /// each order takes, how much of it is tokenizing and pooling, and how far
    /// a vector moves with the run it is embedded in.
    ///
    /// `CLAWG_E5_MODEL_DIR` is `<data dir>/models/multilingual-e5-small`, as
    /// for `e5_golden`, and `CLAWG_IMPORT_EXPORT` a file `engram export --all`
    /// wrote. `CLAWG_EMBED_ROWS` is how many rows, from the start of the file,
    /// the model embeds (5000 by default; 0 measures the padding alone) and
    /// `CLAWG_EMBED_RUNS` how many times each order embeds them (3 by default).
    #[tokio::test]
    #[ignore = "a measurement: needs CLAWG_E5_MODEL_DIR and CLAWG_IMPORT_EXPORT; run it with `cargo test -p clawg-core --lib -- --ignored the_embedding_phase --nocapture`"]
    async fn the_embedding_phase_of_a_real_export() {
        use crate::memory::embedder::{
            apply_role_prefix, check_tokenizer, expected_contract, pad_right, pool_and_normalize, window_encodings, E5Embedder,
        };
        use crate::memory::model_download::{load_cached_model, load_verified_tokenizer, tokenizer_path};
        use crate::memory::portability::engram;
        use crate::memory::vector::EMBEDDING_DIM;
        use std::time::Duration;

        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        fn features() -> String {
            format!(
                "avx2 {}, avx512f {}, fma {}",
                is_x86_feature_detected!("avx2"),
                is_x86_feature_detected!("avx512f"),
                is_x86_feature_detected!("fma")
            )
        }
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
        fn features() -> String {
            String::new()
        }

        let path = |name: &str| PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is not set")));
        let count = |name: &str, default: usize| std::env::var(name).map_or(default, |v| v.parse().unwrap());
        let model_dir = path("CLAWG_E5_MODEL_DIR");
        let base = model_dir.parent().and_then(Path::parent).unwrap().to_path_buf();
        assert_eq!(tokenizer_path(&base).parent(), Some(model_dir.as_path()), "CLAWG_E5_MODEL_DIR is <data dir>/models/multilingual-e5-small");
        let (sample, runs) = (count("CLAWG_EMBED_ROWS", 5000), count("CLAWG_EMBED_RUNS", 3));
        println!(
            "build {}, {} {}, {} logical cpus, {}",
            if cfg!(debug_assertions) { "debug" } else { "optimized" },
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::thread::available_parallelism().map_or(0, |n| n.get()),
            features(),
        );

        let tokenizer = load_verified_tokenizer(&base).expect("the pinned tokenizer.json");
        let export = engram::read(&std::fs::read_to_string(path("CLAWG_IMPORT_EXPORT")).unwrap()).unwrap();
        let store = MemoryStore::new_for_test().await.unwrap();
        let chunker = Arc::new(ChatChunker::with_tokenizer(tokenizer.clone()));
        let (prepared, _) = prepare_export(&store, chunker.clone(), export, &ImportOptions::default(), Arc::new(Disk), &mut ImportTimings::new(true)).await.unwrap();
        let planned = load(&store, &prepared, &mut Timing::new(Step::Load)).await.unwrap();
        let decided = decide(planned, &prepared, &AtomicBool::new(false)).unwrap();
        let rows: Vec<&Row> = rows_to_embed(&prepared, &decided).into_iter().map(|k| &prepared.rows[k]).collect();

        let contract = expected_contract();
        let texts: Vec<String> = rows.iter().map(|r| embedding_text(&r.context, &r.content).into_owned()).collect();
        let prefixed: Vec<String> =
            texts.iter().map(|t| apply_role_prefix(EmbedRole::Passage, t, &contract.query_prefix, &contract.passage_prefix)).collect();
        let clock = Instant::now();
        let mut tokens = Vec::with_capacity(rows.len());
        for chunk in prefixed.chunks(256) {
            let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
            tokens.extend(window_encodings(&tokenizer, &refs).unwrap().iter().map(|e| e.len()));
        }
        let tokenizing = clock.elapsed();
        let file: Vec<usize> = (0..rows.len()).collect();
        let lens = |order: &[usize]| order.iter().map(|&k| tokens[k]).collect::<Vec<_>>();
        let chars: Vec<usize> = texts.iter().map(|t| t.chars().count()).collect();
        let counted = chunker.count_all(&texts.iter().map(String::as_str).collect::<Vec<_>>());
        let orders = |indices: &[usize]| {
            let by = |key: &dyn Fn(usize) -> usize| {
                let mut order = indices.to_vec();
                order.sort_by_key(|&k| Reverse(key(k)));
                order
            };
            [
                ("file", indices.to_vec()),
                ("bytes", by(&|k| texts[k].len())),
                ("chars", by(&|k| chars[k])),
                ("tokens", by(&|k| tokens[k])),
                ("import", longest_first(&indices.iter().map(|&k| counted[k]).collect::<Vec<_>>()).into_iter().map(|k| indices[k]).collect()),
            ]
        };
        let report_padding = |what: &str, indices: &[usize]| {
            for (name, order) in orders(indices) {
                let (linear, attention) = padding(&lens(&order));
                println!("{what}: order {name:>6}: {linear:.3}x the tokens, {attention:.3}x the attention");
            }
        };
        let total: usize = tokens.iter().sum();
        let bucket = |lo: usize, hi: usize| tokens.iter().filter(|&&t| t > lo && t <= hi).count();
        println!(
            "{} notes, {} rows to embed, {total} tokens ({:.1} a row), tokenized in {tokenizing:.2?}; rows of up to 64 tokens {}, 128 {}, 256 {}, 512 {}",
            prepared.notes.len(),
            rows.len(),
            total as f64 / rows.len().max(1) as f64,
            bucket(0, 64),
            bucket(64, 128),
            bucket(128, 256),
            bucket(256, 512),
        );
        report_padding("all rows", &file);
        let sample = sample.min(rows.len());
        if sample == 0 {
            return;
        }
        let first: Vec<usize> = (0..sample).collect();
        report_padding(&format!("first {sample} rows"), &first);

        let files = load_cached_model(&base, &crate::memory::model_download::CacheLock::new()).await.unwrap();
        let pad_id = check_tokenizer(&tokenizer, &files.tokenizer).unwrap();
        let embedder = E5Embedder::from_parts(&files.onnx, tokenizer.clone(), &files.tokenizer, Arc::default()).unwrap();
        let orders = orders(&first);
        let embed = |order: &[usize]| {
            let (texts, embedder) = (&texts, &embedder);
            let order = order.to_vec();
            async move {
                let clock = Instant::now();
                let mut vectors = vec![Vec::new(); texts.len()];
                for call in order.chunks(EMBED_BATCH) {
                    let batch: Vec<String> = call.iter().map(|&k| texts[k].clone()).collect();
                    for (&k, vector) in call.iter().zip(embedder.embed_batch(&batch, EmbedRole::Passage).await.unwrap()) {
                        vectors[k] = vector;
                    }
                }
                (clock.elapsed(), vectors)
            }
        };
        let mut took: Vec<Vec<Duration>> = vec![Vec::new(); orders.len()];
        let mut vectors: Vec<Vec<Vec<f32>>> = vec![Vec::new(); orders.len()];
        for run in 0..runs {
            for (k, (name, order)) in orders.iter().enumerate() {
                let (elapsed, embedded) = embed(order).await;
                println!("run {run}: order {name:>6}: {elapsed:.2?}, {:.1} rows/s", sample as f64 / elapsed.as_secs_f64());
                took[k].push(elapsed);
                vectors[k] = embedded;
            }
        }
        let median = |runs: &[Duration]| {
            let mut runs = runs.to_vec();
            runs.sort();
            runs[runs.len() / 2]
        };
        let file_took = median(&took[0]);
        for (k, (name, _)) in orders.iter().enumerate() {
            let median = median(&took[k]);
            println!(
                "median of {runs}: order {name:>6}: {median:.2?}, {:.1} rows/s, {:.1}% less time than the file order",
                sample as f64 / median.as_secs_f64(),
                100.0 * (1.0 - median.as_secs_f64() / file_took.as_secs_f64())
            );
        }

        let clock = Instant::now();
        let mut pooling = Duration::ZERO;
        for call in first.chunks(EMBED_BATCH) {
            for run in call.chunks(crate::memory::embedder::EMBED_BATCH) {
                let refs: Vec<&str> = run.iter().map(|&k| prefixed[k].as_str()).collect();
                let batch = pad_right(&window_encodings(&tokenizer, &refs).unwrap(), pad_id);
                let hidden = vec![0.5f32; batch.rows * batch.len * EMBEDDING_DIM];
                let pooled = Instant::now();
                pool_and_normalize(&hidden, &[batch.rows as i64, batch.len as i64, EMBEDDING_DIM as i64], &batch.mask).unwrap();
                pooling += pooled.elapsed();
            }
        }
        let tokenizing = clock.elapsed() - pooling;
        let share = |part: Duration| 100.0 * part.as_secs_f64() / file_took.as_secs_f64();
        println!(
            "of the file order's {file_took:.2?}: tokenizing and padding {tokenizing:.2?} ({:.1}%), pooling {pooling:.2?} ({:.1}%), the rest is the session",
            share(tokenizing),
            share(pooling)
        );

        let difference = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
        let by_tokens = &orders.iter().find(|(name, _)| *name == "tokens").unwrap().1;
        let probes: Vec<usize> = by_tokens.iter().step_by((sample / 64).max(1)).copied().collect();
        let mut alone = HashMap::new();
        for &k in &probes {
            alone.insert(k, embedder.embed_with_role(&texts[k], EmbedRole::Passage).await.unwrap());
        }
        for (k, (name, _)) in orders.iter().enumerate() {
            let against_file: Vec<f32> = first.iter().map(|&row| difference(&vectors[0][row], &vectors[k][row])).collect();
            let against_alone = probes.iter().map(|row| difference(&alone[row], &vectors[k][*row])).fold(0f32, f32::max);
            println!(
                "order {name:>6}: {} of {sample} vectors the same bits as the file order, largest difference {:e}; \
                 largest difference from {} texts embedded alone {against_alone:e}",
                against_file.iter().filter(|&&d| d == 0.0).count(),
                against_file.iter().copied().fold(0f32, f32::max),
                probes.len(),
            );
        }
    }
}
