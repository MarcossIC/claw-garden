//! Writes an export into a store, row by row, with what each row was.
//!
//! A path of its own beside the ingest worker, because a save cannot carry a
//! row's state: it counts one access, derives the tier, stores only live rows
//! and runs the contradiction detector. Each guarantee the worker gives a
//! stored row is kept here too: the text masked, its hash, its text index
//! entry, one live row per text and one live save per topic key.
//!
//! Three passes. The first places every row and link inside a transaction it
//! then drops, so rows of one file see each other and a dry run is exactly
//! that pass. The second embeds the rows to insert, outside any transaction.
//! The third writes them in bounded transactions, placing each row again
//! because the worker writes meanwhile, then writes the links. A row that is
//! already there is never inserted again and its links are only filled where
//! empty, so importing the same file twice, or again after a crash between
//! transactions, does no harm.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::memory::backup::Reason;
use crate::memory::chunker::embedding_text;
use crate::memory::clock::TimestampMs;
use crate::memory::dedup::content_hash;
use crate::memory::embedder::{EmbedError, EmbedRole, Embedder};
use crate::memory::identity::derive_project_id;
use crate::memory::portability::format::{parse, FormatError, Invalid, MemoryLine, Parsed};
use crate::memory::redact::redact;
use crate::memory::vector::to_unit;
use crate::memory::{MemoryError, MemoryStore};

/// Rows written per transaction.
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
}

/// A live row stored as the live row of the same text that was already here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Merged {
    pub line: usize,
    pub id: i64,
    pub into: i64,
}

/// A live row stored superseded, because another live row of its agent and
/// project already holds its topic key here.
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
    /// `memory` lines in the file that could be read.
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
    /// Every project to record: id here, root here.
    projects: Vec<(String, String)>,
    parsed: Parsed,
}

fn prepare(mut parsed: Parsed, options: &ImportOptions) -> Prepared {
    let mut project_ids: HashMap<String, String> = HashMap::new();
    let mut projects = Vec::new();
    for (_, p) in &parsed.projects {
        let root = options
            .map_roots
            .iter()
            .find(|(old, _)| *old == p.root_path)
            .map(|(_, new)| new.clone())
            .unwrap_or_else(|| p.root_path.clone());
        let id = derive_project_id(Path::new(&root));
        project_ids.insert(p.id.clone(), id.clone());
        projects.push((id, root));
    }
    let mut contexts: HashMap<i64, String> = HashMap::new();
    for (_, piece) in &parsed.pieces {
        if !piece.context.is_empty() {
            contexts.entry(piece.memory).or_insert_with(|| piece.context.clone());
        }
    }
    let mut lines = std::mem::take(&mut parsed.memories);
    lines.sort_by_key(|(_, m)| m.id);
    let rows = lines
        .into_iter()
        .map(|(line, src)| {
            let masked = redact(&src.content);
            Row {
                line,
                agent: src.agent_id.as_ref().map(|a| options.agent.clone().unwrap_or_else(|| a.clone())),
                project: src.project_id.as_ref().map(|p| project_ids.get(p).cloned().unwrap_or_else(|| p.clone())),
                hash: content_hash(&masked.text),
                redacted: !masked.kinds.is_empty(),
                context: contexts.get(&src.id).cloned().unwrap_or_default(),
                content: masked.text,
                src,
            }
        })
        .collect();
    Prepared { rows, projects, parsed }
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
}

impl Placement {
    fn id(self) -> i64 {
        match self {
            Placement::Inserted(id) | Placement::Present(id) | Placement::Merged(id) => id,
            Placement::TopicTaken { id, .. } => id,
        }
    }

    /// The row here is this file's row, so its links are this file's to fill.
    fn owned(self) -> bool {
        !matches!(self, Placement::Merged(_))
    }

    fn inserted(self) -> bool {
        matches!(self, Placement::Inserted(_) | Placement::TopicTaken { .. })
    }
}

/// What the passes so far placed, by the file's row id.
#[derive(Debug, Default, Clone)]
struct Ledger {
    placed: BTreeMap<i64, Placement>,
    /// The agent each row inserted here had in the file, so two agents'
    /// rows merged under one `--agent` still keep one live save per key.
    import_agents: HashMap<i64, Option<String>>,
    invalid: Vec<Invalid>,
    dropped: Vec<Invalid>,
}

const PRESENT_SQL: &str = "SELECT id FROM memory \
     WHERE agent_id IS ?1 AND COALESCE(project_id, '') = COALESCE(?2, '') \
       AND content_hash = ?3 AND created_at = ?4 \
     ORDER BY id LIMIT 1";

/// The live row of the same text, as the worker's dedup finds it.
const TWIN_SQL: &str = "SELECT id FROM memory \
     WHERE agent_id IS ?1 AND COALESCE(project_id, '') = COALESCE(?2, '') \
       AND content_hash = ?3 AND deleted_at IS NULL AND status IN ('active', 'disputed') \
     ORDER BY created_at ASC LIMIT 1";

const TOPIC_HOLDERS_SQL: &str = "SELECT id FROM memory \
     WHERE agent_id IS ?1 AND COALESCE(project_id, '') = COALESCE(?2, '') \
       AND topic_key = ?3 AND deleted_at IS NULL AND status IN ('active', 'disputed') \
     ORDER BY id";

/// `disputed_against`, `revived_from` and a `superseded_by` from the file
/// are links, written once every row has its id here.
const INSERT_SQL: &str = "INSERT INTO memory \
     (scope, agent_id, project_id, content, memory_type, source, created_at, created_by_type, created_by_ref, \
      source_session_id, last_modified_at, last_modified_by, update_count, last_accessed_at, access_count, \
      importance, tier, status, superseded_by, supersede_cause, dispute_count, last_disputed_at, content_hash, topic_key) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)";

/// Records each project, and returns the ids that already belong to another
/// root, with that root.
fn record_projects(tx: &Transaction<'_>, projects: &[(String, String)], now: i64) -> rusqlite::Result<HashMap<String, String>> {
    let mut taken = HashMap::new();
    for (id, root) in projects {
        tx.execute(
            "INSERT OR IGNORE INTO project (id, root_path, created_at, index_docs) VALUES (?1, ?2, ?3, 0)",
            rusqlite::params![id, root, now],
        )?;
        let existing: String = tx.query_row("SELECT root_path FROM project WHERE id = ?1", [id], |r| r.get(0))?;
        if existing != *root {
            taken.insert(id.clone(), existing);
        }
    }
    Ok(taken)
}

/// Places one row: finds it here, or inserts it with `vector`. `None` when
/// the row would be inserted live and has no vector, which only happens on
/// the writing pass when the store changed since the first.
fn place(
    tx: &Transaction<'_>,
    row: &Row,
    ledger: &mut Ledger,
    vector: Option<&[u8]>,
    needs_vector: bool,
) -> rusqlite::Result<Option<Placement>> {
    let present: Option<i64> = tx
        .query_row(PRESENT_SQL, rusqlite::params![row.agent, row.project, row.hash, row.src.created_at], |r| r.get(0))
        .optional()?;
    if let Some(id) = present {
        return Ok(Some(Placement::Present(id)));
    }
    let mut topic_holder = None;
    if row.live() {
        let twin: Option<i64> =
            tx.query_row(TWIN_SQL, rusqlite::params![row.agent, row.project, row.hash], |r| r.get(0)).optional()?;
        if let Some(id) = twin {
            return Ok(Some(Placement::Merged(id)));
        }
        if let Some(key) = &row.src.topic_key {
            let mut stmt = tx.prepare_cached(TOPIC_HOLDERS_SQL)?;
            let holders: Vec<i64> =
                stmt.query_map(rusqlite::params![row.agent, row.project, key], |r| r.get(0))?.collect::<Result<_, _>>()?;
            // The pieces of one save in the file share its key.
            topic_holder = holders.into_iter().find(|id| match ledger.import_agents.get(id) {
                Some(agent) => *agent != row.src.agent_id,
                None => true,
            });
        }
    }
    if needs_vector && row.src.deleted_at.is_none() && vector.is_none() {
        return Ok(None);
    }
    let (status, superseded_by, cause) = match topic_holder {
        Some(by) => ("superseded", Some(by), Some("revision")),
        None => (row.src.status.as_str(), None, row.src.supersede_cause.as_deref()),
    };
    let src = &row.src;
    tx.prepare_cached(INSERT_SQL)?.execute(rusqlite::params![
        src.scope,
        row.agent,
        row.project,
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
        status,
        superseded_by,
        cause,
        src.dispute_count,
        src.last_disputed_at,
        row.hash,
        src.topic_key,
    ])?;
    let id = tx.last_insert_rowid();
    match src.deleted_at {
        // Inserted live and then deleted, so the text index trigger of V8
        // adds it and takes it out again: a deleted row is not indexed, and a
        // restore can add it back exactly once.
        Some(at) => {
            tx.execute("UPDATE memory SET deleted_at = ?2 WHERE id = ?1", rusqlite::params![id, at])?;
        }
        None => {
            if let Some(bytes) = vector {
                tx.execute("INSERT INTO memory_vectors (rowid, embedding) VALUES (?1, ?2)", rusqlite::params![id, bytes])?;
            }
        }
    }
    ledger.import_agents.insert(id, row.src.agent_id.clone());
    Ok(Some(match topic_holder {
        Some(by) => Placement::TopicTaken { id, by },
        None => Placement::Inserted(id),
    }))
}

/// Places `rows`, skipping the ones a project of another root keeps out.
fn place_rows(
    tx: &Transaction<'_>,
    rows: &[Row],
    taken: &HashMap<String, String>,
    ledger: &mut Ledger,
    vectors: &HashMap<i64, Vec<u8>>,
    needs_vector: bool,
) -> rusqlite::Result<()> {
    for row in rows {
        if let Some(existing) = row.project.as_ref().and_then(|p| taken.get(p).map(|root| (p, root))) {
            ledger.invalid.push(Invalid {
                line: row.line,
                reason: format!("project id {} already belongs to {}", existing.0, existing.1),
            });
            continue;
        }
        match place(tx, row, ledger, vectors.get(&row.src.id).map(Vec::as_slice), needs_vector)? {
            Some(placement) => {
                ledger.placed.insert(row.src.id, placement);
            }
            None => ledger.invalid.push(Invalid {
                line: row.line,
                reason: "the store changed while importing; import the file again".into(),
            }),
        }
    }
    Ok(())
}

/// Writes every link of the file whose ends were placed, translated to the
/// ids here. Rows already here only get the links they lack.
fn write_links(tx: &Transaction<'_>, prepared: &Prepared, ledger: &mut Ledger) -> rusqlite::Result<()> {
    let id_of = |src: i64| ledger.placed.get(&src).map(|p| p.id());
    let mut dropped = Vec::new();

    for row in &prepared.rows {
        let Some(placement) = ledger.placed.get(&row.src.id).copied() else { continue };
        if !placement.owned() {
            continue;
        }
        let id = placement.id();
        if let Some(by) = row.src.superseded_by {
            match id_of(by) {
                Some(by) => {
                    tx.execute("UPDATE memory SET superseded_by = ?2 WHERE id = ?1 AND superseded_by IS NULL", [id, by])?;
                }
                None => dropped.push(Invalid {
                    line: row.line,
                    reason: format!("memory {} is superseded by {by}, which is not in the file", row.src.id),
                }),
            }
        }
        if let Some(from) = row.src.revived_from {
            match id_of(from) {
                Some(from) => {
                    tx.execute("UPDATE memory SET revived_from = ?2 WHERE id = ?1 AND revived_from IS NULL", [id, from])?;
                }
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
                None => dropped.push(Invalid {
                    line: row.line,
                    reason: format!("memory {} is disputed against {other}, which is not in the file", row.src.id),
                }),
            }
        }
        if !against.is_empty() {
            let list = serde_json::to_string(&against).expect("Vec<i64> serialises infallibly");
            tx.execute(
                "UPDATE memory SET disputed_against = ?2 WHERE id = ?1 AND disputed_against IS NULL",
                rusqlite::params![id, list],
            )?;
        } else if placement.inserted() {
            tx.execute("UPDATE memory SET status = 'active' WHERE id = ?1 AND status = 'disputed'", [id])?;
        }
    }

    let owned = |src: i64| ledger.placed.get(&src).is_some_and(|p| p.owned());
    for (line, d) in &prepared.parsed.derivations {
        if !owned(d.memory) {
            continue;
        }
        match (id_of(d.memory), id_of(d.source)) {
            (Some(memory), Some(source)) if memory != source => {
                tx.execute(
                    "INSERT OR IGNORE INTO memory_derivation (memory_id, source_id, created_at) VALUES (?1, ?2, ?3)",
                    [memory, source, d.created_at],
                )?;
            }
            (Some(_), Some(_)) => {}
            _ => dropped.push(Invalid {
                line: *line,
                reason: format!("memory {} is drawn from {}, which is not in the file", d.memory, d.source),
            }),
        }
    }

    for (line, c) in &prepared.parsed.conflicts {
        if !(owned(c.new_id) || owned(c.old_id)) {
            continue;
        }
        match (id_of(c.new_id), id_of(c.old_id)) {
            (Some(new_id), Some(old_id)) if new_id != old_id => {
                let reason = c.reason.as_deref().map(|r| redact(r).text);
                tx.execute(
                    "INSERT OR IGNORE INTO memory_conflict \
                     (new_id, old_id, distance, state, verdict, reason, created_at, judged_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![new_id, old_id, c.distance, c.state, c.verdict, reason, c.created_at, c.judged_at],
                )?;
            }
            (Some(_), Some(_)) => {}
            _ => dropped.push(Invalid {
                line: *line,
                reason: format!("the pair {}-{} names a row that is not in the file", c.new_id, c.old_id),
            }),
        }
    }

    let mut saves: BTreeMap<i64, Vec<(usize, i64, i64)>> = BTreeMap::new();
    for (line, p) in &prepared.parsed.pieces {
        saves.entry(p.save).or_default().push((*line, p.piece, p.memory));
    }
    for pieces in saves.values() {
        let placed: Vec<(i64, i64)> = pieces.iter().filter_map(|(_, piece, m)| id_of(*m).map(|id| (*piece, id))).collect();
        if !pieces.iter().any(|(_, _, m)| owned(*m)) {
            continue;
        }
        let mut linked = false;
        for (_, id) in &placed {
            linked |= tx.query_row("SELECT EXISTS (SELECT 1 FROM memory_piece WHERE memory_id = ?1)", [id], |r| r.get::<_, bool>(0))?;
        }
        if linked {
            continue;
        }
        let save_id: i64 = tx.query_row("SELECT COALESCE(MAX(save_id), 0) + 1 FROM memory_piece", [], |r| r.get(0))?;
        for (line, piece, memory) in pieces {
            let Some(id) = id_of(*memory) else {
                dropped.push(Invalid { line: *line, reason: format!("piece {piece} is memory {memory}, which is not in the file") });
                continue;
            };
            let context = prepared.parsed.pieces.iter().find(|(l, _)| l == line).map(|(_, p)| p.context.clone()).unwrap_or_default();
            tx.execute(
                "INSERT OR IGNORE INTO memory_piece (save_id, piece, memory_id, context) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![save_id, piece, id, context],
            )?;
        }
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
    let (_, parsed) = parse(text)?;
    let prepared = Arc::new(prepare(parsed, options));
    let now = TimestampMs::now().as_millis_i64();

    let decide = prepared.clone();
    let decided = store
        .conn
        .call(move |c| -> rusqlite::Result<Ledger> {
            let tx = c.transaction()?;
            let taken = record_projects(&tx, &decide.projects, now)?;
            let mut ledger = Ledger::default();
            place_rows(&tx, &decide.rows, &taken, &mut ledger, &HashMap::new(), false)?;
            write_links(&tx, &decide, &mut ledger)?;
            // Dropped, never committed: this pass only finds out.
            Ok(ledger)
        })
        .await
        .map_err(MemoryError::Connection)?;

    if options.dry_run {
        return Ok(report(&prepared, decided, true, None));
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
    let mut taken: HashMap<String, String> = HashMap::new();
    for start in (0..prepared.rows.len().max(1)).step_by(WRITE_BATCH) {
        let (batch, vectors) = (prepared.clone(), vectors.clone());
        let (mut placed, mut held) = (std::mem::take(&mut ledger), std::mem::take(&mut taken));
        (ledger, taken) = store
            .conn
            .call(move |c| -> rusqlite::Result<(Ledger, HashMap<String, String>)> {
                let tx = c.transaction()?;
                if start == 0 {
                    held = record_projects(&tx, &batch.projects, now)?;
                }
                let end = (start + WRITE_BATCH).min(batch.rows.len());
                place_rows(&tx, &batch.rows[start..end], &held, &mut placed, &vectors, true)?;
                tx.commit()?;
                Ok((placed, held))
            })
            .await
            .map_err(MemoryError::Connection)?;
    }
    let links = prepared.clone();
    let ledger = store
        .conn
        .call(move |c| -> rusqlite::Result<Ledger> {
            let tx = c.transaction()?;
            write_links(&tx, &links, &mut ledger)?;
            tx.commit()?;
            Ok(ledger)
        })
        .await
        .map_err(MemoryError::Connection)?;

    Ok(report(&prepared, ledger, false, Some(snapshot.display().to_string())))
}

fn report(prepared: &Prepared, ledger: Ledger, dry_run: bool, snapshot: Option<String>) -> ImportReport {
    let mut report = ImportReport {
        dry_run,
        rows: prepared.rows.len(),
        invalid: prepared.parsed.invalid.clone(),
        snapshot,
        ..Default::default()
    };
    report.invalid.extend(ledger.invalid);
    report.invalid.sort_by_key(|i| i.line);
    report.dropped_links = ledger.dropped;
    report.dropped_links.sort_by_key(|i| i.line);
    let skipped: HashSet<usize> = report.invalid.iter().map(|i| i.line).collect();
    for row in &prepared.rows {
        if skipped.contains(&row.line) {
            continue;
        }
        match ledger.placed.get(&row.src.id) {
            Some(Placement::Present(_)) => report.present += 1,
            Some(Placement::Merged(into)) => report.merged.push(Merged { line: row.line, id: row.src.id, into: *into }),
            Some(placement @ (Placement::Inserted(_) | Placement::TopicTaken { .. })) => {
                report.inserted += 1;
                *report.by_type.entry(row.src.memory_type.clone()).or_default() += 1;
                let state = if matches!(placement, Placement::TopicTaken { .. }) { "superseded" } else { row.state() };
                *report.by_state.entry(state.to_string()).or_default() += 1;
                *report.by_project.entry(row.project.clone().unwrap_or_default()).or_default() += 1;
                report.redacted += usize::from(row.redacted);
                if let Placement::TopicTaken { id, by } = placement {
                    report.topic_taken.push(TopicTaken {
                        line: row.line,
                        id: row.src.id,
                        topic_key: row.src.topic_key.clone().unwrap_or_default(),
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::admin::{restore_memory_core, MemoryKind};
    use crate::memory::db::seed_on_disk;
    use crate::memory::embedder::FakeEmbedder;
    use crate::memory::portability::export::{read, Export, ExportFilter};
    use crate::memory::portability::fixtures::{other_project, project, seed, ROOT};
    use crate::memory::portability::format::tests::EXAMPLE;
    use crate::memory::search::{search_all_memories, SearchScope};

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
        let dry = import(&dst, None, backups.path(), &file, &ImportOptions { dry_run: true, ..Default::default() }).await.unwrap();
        assert_eq!(count(&dst, "SELECT count(*) FROM memory").await, 1);
        assert_eq!(count(&dst, "SELECT count(*) FROM project").await, 0);
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
            .replace("different scopes", &format!("the token {token} differs"));
        let dst = destination().await;
        let (report, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!((report.inserted, report.redacted), (1, 1), "{report:#?}");
        let row = dump(&dst, EVERYTHING).await.memories.into_iter().find(|m| m.content.contains("Fridays")).unwrap();
        assert!(!row.content.contains(&token) && row.content.contains("[REDACTED:"), "{}", row.content);
        let hash_matches = count(&dst, "SELECT count(*) FROM memory WHERE content_hash = content_hash(content)").await;
        assert_eq!(hash_matches, 2);
        assert_eq!(count(&dst, "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'fridays'").await, 1);
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
        let moved = derive_project_id(Path::new(elsewhere));
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
    async fn a_project_id_of_another_root_is_reported_and_skipped() {
        let file = seeded_file().await;
        let dst = destination().await;
        let id = project();
        dst.conn
            .call(move |c| c.execute("INSERT INTO project (id, root_path, created_at) VALUES (?1, '/not/the/repo', 0)", [id]))
            .await
            .unwrap();
        let (report, _) = run(&dst, &file, ImportOptions::default()).await;
        assert_eq!(report.inserted, 4, "the rows without a project and the other project's");
        assert_eq!(report.invalid.len(), 13);
        assert!(report.invalid[0].reason.contains("already belongs to /not/the/repo"), "{:?}", report.invalid[0]);
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
}
