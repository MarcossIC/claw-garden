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

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::convert::Infallible;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::memory::backup::Reason;
use crate::memory::chunker::{embedding_text, ChatChunker};
use crate::memory::clock::TimestampMs;
use crate::memory::dedup::content_hash;
use crate::memory::embedder::{EmbedError, EmbedRole, Embedder};
use crate::memory::identity::origin_key;
use crate::memory::project::plan_ids_in;
use crate::memory::language;
use crate::memory::portability::foreign::ForeignExport;
use crate::memory::portability::format::{parse, ConflictLine, FormatError, Invalid, MemoryLine, Parsed, PieceLine};
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportOptions {
    /// Place every row and report, writing nothing.
    pub dry_run: bool,
    /// The agent every row that has one is stored under instead.
    pub agent: Option<String>,
    /// An exported project root and the root it has here.
    pub map_roots: Vec<(String, String)>,
    /// A project name of another tool and the root it has here.
    pub project_map: Vec<(String, String)>,
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
    pub merged: Vec<Merged>,
    pub topic_taken: Vec<TopicTaken>,
    /// Inserted rows whose text held a secret, stored masked.
    pub redacted: usize,
    pub invalid: Vec<Invalid>,
    /// Links whose other end is neither in the file nor here.
    pub dropped_links: Vec<Invalid>,
    /// The `pre-import` snapshot, when one was written.
    pub snapshot: Option<String>,
    /// Inserted rows by the language of their text: `en`, `es` or `und`.
    #[serde(default)]
    pub by_lang: BTreeMap<String, usize>,
    /// For another tool's export, what became of each note. Its lines and
    /// ids are the translation's, so `merged`, `topic_taken` and `invalid`
    /// are told here by note instead.
    #[serde(default)]
    pub foreign: Option<ForeignReport>,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error(transparent)]
    Format(#[from] FormatError),
    #[error("the embedder is not ready; nothing was imported")]
    NoEmbedder,
    #[error("could not embed the imported rows, nothing was imported: {0}")]
    Embed(#[from] EmbedError),
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
    /// Every project to record: its root here, the root's id and remote, and
    /// the project its rows go to.
    projects: Vec<ProjectHere>,
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
}

/// The root here of every project line, with the key of its remote.
fn roots_here(parsed: &Parsed, options: &ImportOptions) -> Vec<(PathBuf, Option<String>)> {
    parsed
        .projects
        .iter()
        .map(|(_, p)| {
            let root = options
                .map_roots
                .iter()
                .find(|(old, _)| *old == p.root_path)
                .map(|(_, new)| new.clone())
                .unwrap_or_else(|| p.root_path.clone());
            let remote = origin_key(Path::new(&root));
            (PathBuf::from(root), remote)
        })
        .collect()
}

/// The ids a registration of each root would give it now, in order.
async fn plan(store: &MemoryStore, roots: &[(PathBuf, Option<String>)]) -> Result<Vec<(String, String)>, ImportError> {
    let planned = roots.to_vec();
    Ok(store.conn.call(move |c| plan_ids_in(c, &planned)).await.map_err(MemoryError::Connection)?)
}

/// What the texts of a file are.
enum Texts {
    /// As a clawg export carries them: masked here.
    Raw,
    /// The rows of notes of another tool, each masked whole before it was
    /// split, which masking a piece alone could undo.
    Notes(Vec<NoteRows>),
}

/// `roots` are the roots here of the file's projects, in order, and `ids`
/// what [`plan`] gave them.
fn prepare(
    mut parsed: Parsed,
    roots: &[(PathBuf, Option<String>)],
    ids: &[(String, String)],
    options: &ImportOptions,
    texts: Texts,
) -> Prepared {
    let mut project_ids: HashMap<String, String> = HashMap::new();
    let mut projects = Vec::new();
    for (((_, p), (root, remote)), (root_id, project_id)) in parsed.projects.iter().zip(roots).zip(ids) {
        project_ids.insert(p.id.clone(), project_id.clone());
        projects.push(ProjectHere {
            root: root.to_string_lossy().into_owned(),
            root_id: root_id.clone(),
            remote: remote.clone(),
            project_id: project_id.clone(),
        });
    }
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
    Prepared { rows, projects, parsed, notes }
}

/// Where a row of the file ended up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    Inserted(i64),
    /// The same row was already here.
    Present(i64),
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
            Placement::Inserted(id) | Placement::Present(id) | Placement::Merged(id) => id,
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

/// The rules over the transaction they write in.
struct SqlSeat<'t, 'c>(&'t Transaction<'c>);

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
        self.0.prepare_cached(PRESENT_SQL)?.query_row(rusqlite::params![agent, project, hash, created_at], |r| r.get(0)).optional()
    }

    fn twin(&mut self, project: Option<&str>, hash: &str) -> rusqlite::Result<Option<i64>> {
        self.0.prepare_cached(TWIN_SQL)?.query_row(rusqlite::params![project, hash], |r| r.get(0)).optional()
    }

    fn holders(&mut self, project: Option<&str>, key: &str, origin: Option<&str>) -> rusqlite::Result<Vec<i64>> {
        let mut stmt = self.0.prepare_cached(TOPIC_HOLDERS_SQL)?;
        let holders = stmt.query_map(rusqlite::params![project, key, origin], |r| r.get(0))?.collect();
        holders
    }

    fn insert(&mut self, new: &NewRow<'_>) -> rusqlite::Result<i64> {
        let (row, src) = (new.row, &new.row.src);
        self.0.prepare_cached(INSERT_SQL)?.execute(rusqlite::params![
            src.scope,
            row.agent,
            new.project,
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
            }
            None => {
                if let Some(bytes) = new.vector {
                    self.0.execute("INSERT INTO memory_vectors (rowid, embedding) VALUES (?1, ?2)", rusqlite::params![id, bytes])?;
                }
            }
        }
        Ok(id)
    }

    fn revise(&mut self, id: i64, by: i64) -> rusqlite::Result<()> {
        self.0.prepare_cached(REVISED_SQL)?.execute([id, by])?;
        Ok(())
    }

    fn linked(&mut self, id: i64) -> rusqlite::Result<bool> {
        self.0.prepare_cached(LINKED_SQL)?.query_row([id], |r| r.get(0))
    }

    fn link(&mut self, link: Link<'_>) -> rusqlite::Result<()> {
        let tx = self.0;
        match link {
            Link::SupersededBy { id, by } => {
                tx.execute(
                    "UPDATE memory SET superseded_by = ?2 WHERE id = ?1 AND superseded_by IS NULL AND status = 'superseded'",
                    [id, by],
                )?;
            }
            Link::RevivedFrom { id, from } => {
                tx.execute("UPDATE memory SET revived_from = ?2 WHERE id = ?1 AND revived_from IS NULL", [id, from])?;
            }
            Link::DisputedAgainst { id, against } => {
                let list = serde_json::to_string(against).expect("Vec<i64> serialises infallibly");
                tx.execute(
                    "UPDATE memory SET disputed_against = ?2 WHERE id = ?1 AND disputed_against IS NULL AND status = 'disputed'",
                    rusqlite::params![id, list],
                )?;
            }
            Link::Undisputed(id) => {
                tx.execute("UPDATE memory SET status = 'active' WHERE id = ?1 AND status = 'disputed'", [id])?;
            }
            Link::Derivation { memory, source, created_at } => {
                tx.execute(
                    "INSERT OR IGNORE INTO memory_derivation (memory_id, source_id, created_at) VALUES (?1, ?2, ?3)",
                    [memory, source, created_at],
                )?;
            }
            Link::Conflict { new_id, old_id, pair } => {
                let reason = pair.reason.as_deref().map(|r| redact(r).text);
                tx.execute(
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
            }
            Link::Save(pieces) => {
                if pieces.is_empty() {
                    return Ok(());
                }
                let save_id: i64 = tx.query_row(NEXT_SAVE_SQL, [], |r| r.get(0))?;
                for (piece, id, context) in pieces {
                    tx.execute(
                        "INSERT OR IGNORE INTO memory_piece (save_id, piece, memory_id, context) VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![save_id, piece, id, context],
                    )?;
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
             CASE WHEN source = 'import' THEN created_by_ref END FROM memory WHERE ",
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
async fn known(store: &MemoryStore, sql: &'static str, binds: Vec<Vec<String>>) -> Result<Vec<Known>, MemoryError> {
    let mut rows = Vec::new();
    for batch in binds.chunks(LOAD_BATCH) {
        let batch = batch.to_vec();
        let found = store
            .conn
            .call(move |c| -> rusqlite::Result<Vec<Known>> {
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
            .await
            .map_err(MemoryError::Connection)?;
        rows.extend(found);
    }
    Ok(rows)
}

/// Loads what the rules can read of the store for `prepared`. The rows of
/// each origin of the file come first: with the file's own, their projects
/// and keys are every project and key the rules can ask about.
async fn load(store: &MemoryStore, prepared: &Prepared) -> Result<Planned, MemoryError> {
    let origins: BTreeSet<&str> = prepared.notes.iter().map(|n| n.origin.as_str()).collect();
    let binds = origins.iter().map(|o| vec![o.to_string()]).collect();
    let mut rows = known(store, ORIGIN_ROWS_SQL, binds).await?;

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
    rows.extend(known(store, TEXT_ROWS_SQL, texts.into_iter().collect()).await?);
    rows.extend(known(store, KEY_ROWS_SQL, keys.into_iter().collect()).await?);
    rows.sort_by_key(|r| r.id);
    rows.dedup_by_key(|r| r.id);

    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let mut linked = HashSet::new();
    for batch in ids.chunks(LOAD_BATCH) {
        let batch = batch.to_vec();
        let found = store
            .conn
            .call(move |c| -> rusqlite::Result<Vec<i64>> {
                let mut stmt = c.prepare_cached(LINKED_SQL)?;
                let mut linked = Vec::new();
                for id in batch {
                    if stmt.query_row([id], |r| r.get(0))? {
                        linked.push(id);
                    }
                }
                Ok(linked)
            })
            .await
            .map_err(MemoryError::Connection)?;
        linked.extend(found);
    }

    let root_ids: Vec<String> = prepared.projects.iter().map(|p| p.root_id.clone()).collect();
    let (roots, next_id) = store
        .conn
        .call(move |c| -> rusqlite::Result<(HashMap<String, String>, i64)> {
            let mut roots = HashMap::new();
            let mut stmt = c.prepare_cached(PROJECT_ROOT_SQL)?;
            for id in root_ids {
                if let Some(root) = stmt.query_row([&id], |r| r.get(0)).optional()? {
                    roots.insert(id, root);
                }
            }
            Ok((roots, c.query_row(NEXT_ID_SQL, [], |r| r.get(0))?))
        })
        .await
        .map_err(MemoryError::Connection)?;

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

const PROJECT_ROOT_SQL: &str = "SELECT root_path FROM project WHERE id = ?1";

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

/// Records each project, and returns [`taken_projects`].
fn record_projects(tx: &Transaction<'_>, projects: &[ProjectHere], now: i64) -> rusqlite::Result<HashMap<String, String>> {
    let mut roots = HashMap::new();
    for p in projects {
        let root: Option<String> = tx.prepare_cached(PROJECT_ROOT_SQL)?.query_row([&p.root_id], |r| r.get(0)).optional()?;
        if let Some(root) = root {
            roots.insert(p.root_id.clone(), root);
        }
    }
    let (taken, remotes) = taken_projects(projects, &mut roots);
    for p in projects {
        tx.execute(
            "INSERT OR IGNORE INTO project (id, root_path, created_at, index_docs) VALUES (?1, ?2, ?3, 0)",
            rusqlite::params![p.root_id, p.root, now],
        )?;
    }
    for p in remotes {
        tx.execute(
            "INSERT OR IGNORE INTO project_remote (remote, project_id, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![p.remote, p.project_id, now],
        )?;
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
/// snapshot into `backups` first. A dry run writes nothing and needs no
/// embedder.
pub async fn import(
    store: &MemoryStore,
    embedder: Option<&dyn Embedder>,
    backups: &Path,
    text: &str,
    options: &ImportOptions,
) -> Result<ImportReport, ImportError> {
    let prepared = prepare_file(store, text, options).await?;
    let (ledger, snapshot) = write(store, embedder, backups, &prepared, options.dry_run).await?;
    Ok(report(&prepared, &ledger, options.dry_run, snapshot))
}

/// A clawg export, worked out against the ids its roots would get here.
async fn prepare_file(store: &MemoryStore, text: &str, options: &ImportOptions) -> Result<Arc<Prepared>, ImportError> {
    let (_, parsed) = parse(text)?;
    let roots = roots_here(&parsed, options);
    let ids = plan(store, &roots).await?;
    Ok(Arc::new(prepare(parsed, &roots, &ids, options, Texts::Raw)))
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
) -> Result<ImportReport, ImportError> {
    let (prepared, foreign) = prepare_export(store, chunker, export, options).await?;
    let (ledger, snapshot) = write(store, embedder, backups, &prepared, options.dry_run).await?;
    Ok(with_foreign(report(&prepared, &ledger, options.dry_run, snapshot), foreign, &prepared.notes, &ledger))
}

/// Another tool's export translated into rows, with what the translation
/// counted.
async fn prepare_export(
    store: &MemoryStore,
    chunker: Arc<ChatChunker>,
    export: ForeignExport,
    options: &ImportOptions,
) -> Result<(Arc<Prepared>, ForeignReport), ImportError> {
    // Planned before the translation, which tells keys apart by the project
    // each note goes to here.
    let mut seen = HashSet::new();
    let mapped: Vec<&str> = options.project_map.iter().map(|(_, root)| root.as_str()).filter(|root| seen.insert(*root)).collect();
    let roots: Vec<(PathBuf, Option<String>)> =
        mapped.iter().map(|root| (PathBuf::from(root), origin_key(Path::new(root)))).collect();
    let ids = plan(store, &roots).await?;
    let project_of: HashMap<&str, &str> = mapped.iter().copied().zip(ids.iter().map(|(_, project_id)| project_id.as_str())).collect();
    let translate_options = TranslateOptions {
        agent: options.agent.clone().unwrap_or_else(|| MIGRATED_AGENT.to_owned()),
        project_map: options
            .project_map
            .iter()
            .map(|(name, root)| MappedProject { name: name.clone(), root: root.clone(), project_id: project_of[root.as_str()].to_owned() })
            .collect(),
    };
    let now = TimestampMs::now().as_millis_i64();
    // Masking and splitting thousands of notes is work the daemon's other
    // requests should not wait behind.
    let (prepared, foreign) = tokio::task::spawn_blocking(move || {
        let Translated { parsed, notes, report } = translate(&export, &translate_options, &chunker, now);
        (prepare(parsed, &roots, &ids, &ImportOptions::default(), Texts::Notes(notes)), report)
    })
    .await
    .map_err(|e| MemoryError::Internal(format!("could not translate the export: {e}")))?;
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

/// Places every row and link of `prepared` and, unless this is a dry run,
/// embeds the rows to insert and writes them after a `pre-import` snapshot,
/// whose path it returns.
async fn write(
    store: &MemoryStore,
    embedder: Option<&dyn Embedder>,
    backups: &Path,
    prepared: &Arc<Prepared>,
    dry_run: bool,
) -> Result<(Ledger, Option<String>), ImportError> {
    let now = TimestampMs::now().as_millis_i64();
    let planned = load(store, prepared).await?;
    let stop = Arc::new(AtomicBool::new(false));
    let _stop = StopOnDrop(stop.clone());
    let decide_from = prepared.clone();
    let decided = tokio::task::spawn_blocking(move || decide(planned, &decide_from, &stop))
        .await
        .map_err(|e| MemoryError::Internal(format!("could not plan the import: {e}")))?
        .ok_or_else(|| MemoryError::Internal("the import was stopped".to_owned()))?;

    if dry_run {
        return Ok((decided, None));
    }
    let embedder = embedder.ok_or(ImportError::NoEmbedder)?;

    let to_embed: Vec<&Row> = prepared
        .rows
        .iter()
        .filter(|r| r.src.deleted_at.is_none() && decided.placed.get(&r.src.id).is_some_and(|p| p.inserted()))
        .collect();
    let mut vectors: HashMap<i64, Vec<u8>> = HashMap::with_capacity(to_embed.len());
    for batch in to_embed.chunks(EMBED_BATCH) {
        let texts: Vec<String> = batch.iter().map(|r| embedding_text(&r.context, &r.content).into_owned()).collect();
        let embedded = embedder.embed_batch(&texts, EmbedRole::Passage).await?;
        for (row, vector) in batch.iter().zip(embedded) {
            let unit = to_unit(&vector).map_err(|e| EmbedError::InferenceFailed(e.to_string()))?;
            vectors.insert(row.src.id, unit.iter().flat_map(|f| f.to_le_bytes()).collect());
        }
    }

    let snapshot = store.snapshot(backups.to_path_buf(), Reason::PreImport).await?;

    let vectors = Arc::new(vectors);
    let mut ledger = Ledger::default();
    let mut batches = note_batches(&prepared.units(), WRITE_BATCH);
    if batches.is_empty() {
        // The projects are recorded even when there is no row.
        batches.push(0..0);
    }
    for (k, range) in batches.into_iter().enumerate() {
        let (batch, vectors) = (prepared.clone(), vectors.clone());
        let mut placed = std::mem::take(&mut ledger);
        ledger = store
            .conn
            .call(move |c| -> rusqlite::Result<Ledger> {
                let tx = c.transaction()?;
                if k == 0 {
                    placed.taken = record_projects(&tx, &batch.projects, now)?;
                }
                place_rows(&mut SqlSeat(&tx), &batch, range, &mut placed, &vectors, true)?;
                tx.commit()?;
                Ok(placed)
            })
            .await
            .map_err(MemoryError::Connection)?;
    }
    let links = prepared.clone();
    let ledger = store
        .conn
        .call(move |c| -> rusqlite::Result<Ledger> {
            let tx = c.transaction()?;
            write_links(&mut SqlSeat(&tx), &links, &mut ledger)?;
            tx.commit()?;
            Ok(ledger)
        })
        .await
        .map_err(MemoryError::Connection)?;

    Ok((ledger, Some(snapshot.display().to_string())))
}

fn report(prepared: &Prepared, ledger: &Ledger, dry_run: bool, snapshot: Option<String>) -> ImportReport {
    let mut report = ImportReport {
        dry_run,
        rows: prepared.rows.len(),
        invalid: prepared.parsed.invalid.clone(),
        snapshot,
        ..Default::default()
    };
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
mod tests {
    use super::*;
    use crate::memory::admin::{delete_memory_core, restore_memory_core, MemoryKind};
    use crate::memory::db::seed_on_disk;
    use crate::memory::embedder::FakeEmbedder;
    use crate::memory::events::{MemoryEvent, MemorySource};
    use crate::memory::portability::export::{read, Export, ExportFilter};
    use crate::memory::portability::fixtures::{other_project, project, seed, ROOT};
    use crate::memory::portability::foreign::{ForeignNote, ForeignRelation, NoteKind, When};
    use crate::memory::portability::format::tests::EXAMPLE;
    use crate::memory::portability::translate::tests::{export as foreign_export, note as foreign_note, T0};
    use crate::memory::portability::translate::{rewrite_key, KeyChange, ProjectCount, TakenKey, TakenProject, TypeCount};
    use crate::memory::relocate::{move_memories_core, MoveFilter};
    use crate::memory::search::{search_all_memories, SearchScope};
    use crate::memory::worker::save_for_test;
    use crate::models::chat::Role;

    const EVERYTHING: ExportFilter = ExportFilter { project_id: None, agent: None, include_deleted: true };

    async fn dump(store: &MemoryStore, filter: ExportFilter) -> Export {
        store.conn.call(move |c| Ok::<_, rusqlite::Error>(read(c, &filter, 0))).await.unwrap().unwrap()
    }

    async fn seeded_file() -> String {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed(&store).await;
        dump(&store, EVERYTHING).await.to_jsonl()
    }

    /// An empty store, but for one row of its own, so the ids the import
    /// assigns are not the file's by accident.
    async fn destination() -> MemoryStore {
        let store = MemoryStore::new_for_test().await.unwrap();
        insert_row(&store, "claude-code", None, "A row that was here first", None, 1).await;
        store
    }

    async fn insert_row(
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

    async fn run(store: &MemoryStore, text: &str, options: ImportOptions) -> (ImportReport, tempfile::TempDir) {
        let backups = tempfile::tempdir().unwrap();
        let report = import(store, Some(&FakeEmbedder::default()), backups.path(), text, &options).await.unwrap();
        (report, backups)
    }

    async fn count(store: &MemoryStore, sql: &'static str) -> i64 {
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
        let backups = tempfile::tempdir().unwrap();
        let err = import(&dst, Some(&FakeEmbedder::default()), backups.path(), &file, &ImportOptions::default()).await.unwrap_err();
        assert!(matches!(err, ImportError::Format(FormatError::UnknownVersion(2))), "{err}");
        assert_eq!(count(&dst, "SELECT count(*) FROM memory").await, 1);
        assert!(files_in(backups.path()).is_empty());
    }

    #[tokio::test]
    async fn a_dry_run_reports_without_writing() {
        let file = seeded_file().await;
        let dst = destination().await;
        let backups = tempfile::tempdir().unwrap();
        let sequence = "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'memory'), 0)";
        let before = count(&dst, sequence).await;
        let dry = import(&dst, None, backups.path(), &file, &ImportOptions { dry_run: true, ..Default::default() }).await.unwrap();
        assert_eq!(count(&dst, "SELECT count(*) FROM memory").await, 1);
        assert_eq!(count(&dst, "SELECT count(*) FROM project").await, 0);
        assert_eq!(count(&dst, "SELECT count(*) FROM project_remote").await, 0);
        assert_eq!(count(&dst, "SELECT count(*) FROM memory_fts").await, 1);
        assert_eq!(count(&dst, sequence).await, before);
        assert!(files_in(backups.path()).is_empty());
        let (real, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!(ImportReport { dry_run: false, snapshot: real.snapshot.clone(), ..dry }, real);
    }

    #[tokio::test]
    async fn without_an_embedder_nothing_is_written() {
        let file = seeded_file().await;
        let dst = destination().await;
        let backups = tempfile::tempdir().unwrap();
        let err = import(&dst, None, backups.path(), &file, &ImportOptions::default()).await.unwrap_err();
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
                let mut stmt = c.prepare("SELECT id, root_path, index_docs FROM project ORDER BY root_path")?;
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
            .call(move |c| c.execute("INSERT INTO project (id, root_path, created_at) VALUES (?1, '/not/the/repo', 0)", [id]))
            .await
            .unwrap();
        let (report, _) = run(&dst, &file, ImportOptions::default()).await;
        assert!(report.invalid.is_empty(), "{:?}", report.invalid);
        let own = crate::memory::identity::candidate_id(Path::new(ROOT), 1);
        assert_eq!(report.by_project.get(&own), Some(&13));
        assert_eq!(report.by_project.get(&project()), None, "nothing goes to the other root's project");
    }

    #[tokio::test]
    async fn an_import_into_a_clone_goes_to_the_project_of_its_remote() {
        let file = seeded_file().await;
        let dst = destination().await;
        dst.conn
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO project (id, root_path, created_at) VALUES ('app-1a2b', '/work/app', 0);
                     INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/app', 'app-1a2b', 0);",
                )
            })
            .await
            .unwrap();
        let clone = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(clone.path().join(".git")).unwrap();
        std::fs::write(clone.path().join(".git").join("config"), "[remote \"origin\"]\n\turl = https://github.com/acme/app\n").unwrap();
        let here = clone.path().to_string_lossy().into_owned();
        let options = ImportOptions { map_roots: vec![(ROOT.into(), here)], ..Default::default() };

        let (report, _) = run(&dst, &file, options).await;

        assert_eq!(report.by_project.get("app-1a2b"), Some(&13), "{:?}", report.by_project);
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

    async fn run_foreign(store: &MemoryStore, notes: Vec<ForeignNote>, options: ImportOptions) -> (ImportReport, tempfile::TempDir) {
        run_export(store, foreign_export(notes), options).await
    }

    async fn run_export(store: &MemoryStore, export: ForeignExport, options: ImportOptions) -> (ImportReport, tempfile::TempDir) {
        let backups = tempfile::tempdir().unwrap();
        let chunker = Arc::new(ChatChunker::with_fallback());
        let report =
            import_foreign(store, Some(&FakeEmbedder::default()), chunker, backups.path(), export, &options).await.unwrap();
        (report, backups)
    }

    /// `source` and `target` judged related by `verb`, by an agent.
    fn relation(source: &str, target: &str, verb: &str) -> ForeignRelation {
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
    fn named(name: &str, k: i64) -> ForeignNote {
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
                c.execute("INSERT INTO project (id, root_path, created_at) VALUES (?1, ?2, 0)", rusqlite::params![recorded, root])
            })
            .await
            .unwrap();
        id
    }

    /// Notes of two projects of the source and of none: a long one, two under
    /// one key, and one the source deleted.
    fn sample_notes() -> Vec<ForeignNote> {
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
        let backups = tempfile::tempdir().unwrap();
        let chunker = Arc::new(ChatChunker::with_fallback());
        let dry_run = ImportOptions { dry_run: true, ..Default::default() };
        let dry = import_foreign(&dst, None, chunker, backups.path(), foreign_export(notes.clone()), &dry_run).await.unwrap();
        assert!(files_in(backups.path()).is_empty());
        let (report, _) = run_foreign(&dst, notes, ImportOptions::default()).await;
        assert_eq!(ImportReport { dry_run: false, snapshot: report.snapshot.clone(), ..dry }, report);

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
            .call(move |c| c.execute("INSERT INTO project (id, root_path, created_at) VALUES (?1, '/not/the/notes', 0)", [id]))
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
                    "INSERT INTO project (id, root_path, created_at) VALUES ('app-1a2b', '/work/app', 0);
                     INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/app', 'app-1a2b', 0);",
                )
            })
            .await
            .unwrap();
        let clone = || {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join(".git")).unwrap();
            std::fs::write(dir.path().join(".git").join("config"), "[remote \"origin\"]\n\turl = https://github.com/acme/app\n").unwrap();
            dir
        };
        let (one, two) = (clone(), clone());
        let root = |dir: &tempfile::TempDir| dir.path().to_string_lossy().into_owned();
        let keyed = |origin: &str, project: &str, content: &str| ForeignNote {
            project: Some(project.into()),
            topic_key: Some("sdd/x".into()),
            ..foreign_note(origin, "Proposal", content)
        };
        let notes = vec![keyed("engram:a", "a", "the proposal of a"), keyed("engram:b", "b", "the proposal of b")];
        let options = ImportOptions { project_map: vec![("a".into(), root(&one)), ("b".into(), root(&two))], ..Default::default() };
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
    async fn a_moved_note_is_recognized_where_it_is() {
        let dst = destination().await;
        run_foreign(&dst, sample_notes(), ImportOptions::default()).await;
        let moved_to = register(&dst, "/work/moved").await;
        let backups = tempfile::tempdir().unwrap();
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
        let backups = tempfile::tempdir().unwrap();
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
        let dir = tempfile::tempdir().unwrap();
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

    /// Each runs once per row or per save of a file, in the pass that decides
    /// and again in the one that writes, so a scan makes an import grow with
    /// the square of its rows.
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
            ("PROJECT_ROOT_SQL", PROJECT_ROOT_SQL, vec![Text(project())], "sqlite_autoindex_project_1"),
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
        prepare_file(store, text, &options).await.unwrap()
    }

    async fn notes_of(store: &MemoryStore, notes: Vec<ForeignNote>, options: ImportOptions) -> Arc<Prepared> {
        prepare_export(store, Arc::new(ChatChunker::with_fallback()), foreign_export(notes), &options).await.unwrap().0
    }

    /// What the pass that decided on the connection made of `prepared`: the
    /// rules on a transaction that is dropped.
    async fn decided_by_sql(store: &MemoryStore, prepared: &Arc<Prepared>) -> Ledger {
        let prepared = prepared.clone();
        store
            .conn
            .call(move |c| -> rusqlite::Result<Ledger> {
                let tx = c.transaction()?;
                let mut ledger = Ledger { taken: record_projects(&tx, &prepared.projects, 0)?, ..Default::default() };
                place_rows(&mut SqlSeat(&tx), &prepared, 0..prepared.rows.len(), &mut ledger, &HashMap::new(), false)?;
                write_links(&mut SqlSeat(&tx), &prepared, &mut ledger)?;
                Ok(ledger)
            })
            .await
            .unwrap()
    }

    async fn decides_alike(store: &MemoryStore, prepared: Arc<Prepared>, case: &str) -> Ledger {
        let by_sql = decided_by_sql(store, &prepared).await;
        let planned = load(store, &prepared).await.unwrap();
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
        let dir = tempfile::tempdir().unwrap();
        let options = ImportOptions { project_map: vec![("a".into(), dir.path().to_string_lossy().into_owned())], ..Default::default() };
        let prepared = notes_of(&dst, sample_notes(), options).await;
        let root_id = prepared.projects[0].root_id.clone();
        dst.conn
            .call(move |c| c.execute("INSERT INTO project (id, root_path, created_at) VALUES (?1, '/work/elsewhere', 0)", [root_id]))
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
        let planned = load(&dst, &prepared).await.unwrap();
        assert!(decide(planned, &prepared, &AtomicBool::new(true)).is_none());
    }

    #[tokio::test]
    async fn a_dry_run_after_a_purge_reports_the_ids_of_the_import() {
        let file = seeded_file().await;
        let dst = destination().await;
        insert_row(&dst, "claude-code", Some(project()), "Storage engine: DuckDB", Some("storage/engine"), 5).await;
        let purged = insert_row(&dst, "claude-code", None, "A row purged since", None, 6).await;
        dst.conn.call(move |c| c.execute("DELETE FROM memory WHERE id = ?1", [purged])).await.unwrap();
        let backups = tempfile::tempdir().unwrap();
        let dry = import(&dst, None, backups.path(), &file, &ImportOptions { dry_run: true, ..Default::default() }).await.unwrap();
        assert!(dry.topic_taken.iter().all(|t| t.stored_as > purged) && !dry.topic_taken.is_empty(), "{dry:#?}");
        let (real, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!(ImportReport { dry_run: false, snapshot: real.snapshot.clone(), ..dry }, real);
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
            let report = ImportReport { snapshot: None, ..report };
            println!("{count} notes in {elapsed:?}, report {}", content_hash(&serde_json::to_string(&report).unwrap()));
            (elapsed, report)
        }
        let (once, small) = timed(NOTES).await;
        let (twice, large) = timed(2 * NOTES).await;
        assert_eq!((small.inserted, large.inserted), (NOTES, 2 * NOTES));
        let ratio = twice.as_secs_f64() / once.as_secs_f64();
        assert!(ratio <= 3.0, "{NOTES} notes in {once:?}, twice as many in {twice:?}: {ratio:.2} times");
    }
}
