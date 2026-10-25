//! An index of each registered project's documentation and code: its Markdown
//! files and manifests split by heading, its source split along its syntax
//! tree, embedded and searchable by meaning and by term.
//!
//! Derived from the files on disk and rebuilt from them at will. Nothing here
//! reads or writes a memory.

pub mod chunk;
pub mod code;
pub mod corpus;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::memory::chunker::token_splitter::TokenSplitter;
use crate::memory::clock::TimestampMs;
use crate::memory::embedder::{EmbedError, EmbedRole, Embedder};
use crate::memory::redact::redact;
use crate::memory::search::{fts_match_expression, MAX_LEXICAL_TOKENS};
use crate::memory::vector::to_unit;
use crate::memory::MemoryStore;

use chunk::{chunk_manifest, chunk_markdown, Passage};
use code::chunk_code;
use corpus::{CorpusError, DocFile, DocKind};

/// How old the last refresh of a project may be before a search refreshes it
/// again. Long enough that a burst of searches walks the tree once; short
/// enough that a doc edited during a session is found in its new form.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Passages each channel of a search proposes before fusion. With a top of 20
/// at most, 50 per channel leaves room for a passage that only one channel
/// ranks high.
pub const CANDIDATES_PER_CHANNEL: usize = 50;

/// The constant of reciprocal rank fusion. 60 is the value the method was
/// published with; it flattens the gap between the first ranks so that two
/// channels agreeing on a passage outweigh one channel's favourite.
const RRF_K: f64 = 60.0;

#[derive(Debug, thiserror::Error)]
pub enum DocIndexError {
    #[error("project {0} has no registered root")]
    UnknownProject(String),
    /// Registered, but only as a label for memories: the user never let the
    /// daemon read its files.
    #[error("{} is not a project the daemon may read", .0.display())]
    NotAccepted(PathBuf),
    #[error(transparent)]
    Corpus(#[from] CorpusError),
    #[error("embedding {path} failed: {source}")]
    Embed {
        path: String,
        #[source]
        source: EmbedError,
    },
    /// One vector of the doc was not usable: of the doc, not of the embedder,
    /// so a refresh skips the doc and goes on.
    #[error("the embedder returned an unusable vector for {path}: {reason}")]
    Vector { path: String, reason: String },
    #[error("the embedder returned {vectors} vectors for the {passages} passages of {path}")]
    VectorCount { path: String, vectors: usize, passages: usize },
    #[error("embedding the query failed: {0}")]
    Query(String),
    #[error("documentation index store error: {0}")]
    Store(#[from] tokio_rusqlite::Error),
}

/// What one refresh did, counted in docs except for `passages`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefreshReport {
    /// Docs on disk that were read.
    pub docs: usize,
    pub indexed: usize,
    pub unchanged: usize,
    pub removed: usize,
    /// Too large, past the cap, or not UTF-8.
    pub skipped: usize,
    /// Passages written by this refresh.
    pub passages: usize,
    /// Docs left as they were because a vector of theirs was unusable; the
    /// next refresh tries them again.
    pub failed: Vec<FailedDoc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedDoc {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    Refreshed(RefreshReport),
    /// Another refresh of the project was running; this one did nothing.
    AlreadyRunning,
}

/// What the index holds for one doc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub path: String,
    pub hash: String,
    pub model_id: String,
}

/// Which docs a refresh rewrites, leaves alone and drops.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub index: Vec<String>,
    pub keep: usize,
    pub remove: Vec<String>,
}

/// Decides a refresh from the `(path, sha256)` of every doc on disk and what
/// the index holds. A doc is rewritten when it is new, its bytes changed, or
/// its vectors came from another model.
pub fn plan_refresh(disk: &[(String, String)], stored: &[Fingerprint], model_id: &str) -> Plan {
    let stored_by_path: HashMap<&str, &Fingerprint> =
        stored.iter().map(|f| (f.path.as_str(), f)).collect();
    let mut plan = Plan::default();
    for (path, hash) in disk {
        match stored_by_path.get(path.as_str()) {
            Some(f) if f.hash == *hash && f.model_id == model_id => plan.keep += 1,
            _ => plan.index.push(path.clone()),
        }
    }
    let on_disk: HashSet<&str> = disk.iter().map(|(p, _)| p.as_str()).collect();
    plan.remove = stored.iter().filter(|f| !on_disk.contains(f.path.as_str())).map(|f| f.path.clone()).collect();
    plan
}

/// Whether a project whose last refresh started at `last_started` is due for
/// another at `now`.
pub fn needs_refresh(last_started: Option<Instant>, now: Instant, interval: Duration) -> bool {
    last_started.map_or(true, |t| now.saturating_duration_since(t) >= interval)
}

#[derive(Debug, Default, Clone)]
struct Progress {
    running: bool,
    last_started: Option<Instant>,
    /// Why the last refresh that ran to its end failed; cleared by one that
    /// succeeds.
    last_error: Option<String>,
}

/// Which projects are being refreshed and when each last started, so one
/// project is never refreshed twice at once and a search knows whether to
/// refresh first.
#[derive(Debug, Default)]
pub struct DocIndex {
    projects: Mutex<HashMap<String, Progress>>,
    /// Woken whenever any project's refresh ends.
    finished: tokio::sync::Notify,
}

/// Marks a project's refresh as running for as long as it lives, also when
/// the refresh fails or its task is dropped.
pub struct RefreshGuard {
    index: Arc<DocIndex>,
    project_id: String,
}

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        let mut projects = self.index.projects.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = projects.get_mut(&self.project_id) {
            p.running = false;
        }
        drop(projects);
        self.index.finished.notify_waiters();
    }
}

impl RefreshGuard {
    /// Brings the project's index in step with its registered root and keeps
    /// how it ended for [`DocIndex::last_error`], written before the claim is
    /// released so whoever waited for it reads it already there.
    pub async fn run(
        self,
        store: &MemoryStore,
        embedder: &dyn Embedder,
        tokenizer: Option<Arc<tokenizers::Tokenizer>>,
    ) -> Result<RefreshReport, DocIndexError> {
        let result = async {
            let root = root_of(store, &self.project_id).await?;
            let splitter = match tokenizer {
                Some(tok) => TokenSplitter::with_tokenizer(tok),
                None => TokenSplitter::fallback(),
            };
            refresh(store, embedder, &splitter, &self.project_id, &root).await
        }
        .await;
        let mut projects = self.index.projects.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = projects.get_mut(&self.project_id) {
            p.last_error = result.as_ref().err().map(ToString::to_string);
        }
        drop(projects);
        result
    }
}

impl DocIndex {
    /// Claims the project's refresh, or `None` if one is already running.
    pub fn begin(self: &Arc<Self>, project_id: &str) -> Option<RefreshGuard> {
        let mut projects = self.projects.lock().unwrap_or_else(|e| e.into_inner());
        let progress = projects.entry(project_id.to_string()).or_default();
        if progress.running {
            return None;
        }
        progress.running = true;
        progress.last_started = Some(Instant::now());
        Some(RefreshGuard { index: self.clone(), project_id: project_id.to_string() })
    }

    /// Claims the project's refresh if a search at `now` should start one
    /// (see [`DocIndex::is_due`]), checked and claimed under one lock so two
    /// searches never both start it.
    pub fn begin_if_due(self: &Arc<Self>, project_id: &str, now: Instant) -> Option<RefreshGuard> {
        let mut projects = self.projects.lock().unwrap_or_else(|e| e.into_inner());
        let progress = projects.entry(project_id.to_string()).or_default();
        if progress.running || !needs_refresh(progress.last_started, now, REFRESH_INTERVAL) {
            return None;
        }
        progress.running = true;
        progress.last_started = Some(Instant::now());
        Some(RefreshGuard { index: self.clone(), project_id: project_id.to_string() })
    }

    pub fn is_running(&self, project_id: &str) -> bool {
        self.progress(project_id).running
    }

    /// Whether a search at `now` should refresh the project first: none is
    /// running and the last one started [`REFRESH_INTERVAL`] ago, or never.
    pub fn is_due(&self, project_id: &str, now: Instant) -> bool {
        let p = self.progress(project_id);
        !p.running && needs_refresh(p.last_started, now, REFRESH_INTERVAL)
    }

    /// Waits until no refresh of the project is running, or `limit` passed.
    /// Returns whether it is idle.
    pub async fn wait_idle(&self, project_id: &str, limit: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            // Registered before the check, so an end that lands between the
            // check and the await still wakes this waiter.
            let finished = self.finished.notified();
            tokio::pin!(finished);
            finished.as_mut().enable();
            if !self.is_running(project_id) {
                return true;
            }
            if tokio::time::timeout_at(deadline, finished).await.is_err() {
                return !self.is_running(project_id);
            }
        }
    }

    /// Why the project's last finished refresh failed, until one succeeds.
    pub fn last_error(&self, project_id: &str) -> Option<String> {
        self.progress(project_id).last_error
    }

    fn progress(&self, project_id: &str) -> Progress {
        let projects = self.projects.lock().unwrap_or_else(|e| e.into_inner());
        projects.get(project_id).cloned().unwrap_or_default()
    }

    /// Brings the project's index in step with its registered root, unless a
    /// refresh of it is already running. `tokenizer` is the ingest's, so a
    /// passage is capped by the same count a memory is; without it passages
    /// are capped by bytes.
    pub async fn try_refresh(
        self: &Arc<Self>,
        store: &MemoryStore,
        embedder: &dyn Embedder,
        tokenizer: Option<Arc<tokenizers::Tokenizer>>,
        project_id: &str,
    ) -> Result<RefreshOutcome, DocIndexError> {
        let Some(guard) = self.begin(project_id) else {
            return Ok(RefreshOutcome::AlreadyRunning);
        };
        guard.run(store, embedder, tokenizer).await.map(RefreshOutcome::Refreshed)
    }
}

/// One passage a search found.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DocHit {
    pub path: String,
    pub breadcrumb: String,
    /// The passage's body, without the breadcrumb.
    pub text: String,
    pub score: f64,
}

/// Reciprocal rank fusion of two ranked id lists: each list adds
/// `1 / (RRF_K + rank)` to an id, rank from 1. Highest total first, ties by
/// lower id, at most `limit`.
pub fn fuse(dense: &[i64], lexical: &[i64], limit: usize) -> Vec<(i64, f64)> {
    let mut scores: HashMap<i64, f64> = HashMap::new();
    for list in [dense, lexical] {
        for (rank, id) in list.iter().enumerate() {
            *scores.entry(*id).or_default() += 1.0 / (RRF_K + (rank + 1) as f64);
        }
    }
    let mut fused: Vec<(i64, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    fused.truncate(limit);
    fused
}

/// The passages of `project_id` that best answer `query`, by meaning (the
/// nearest vectors) and by term (BM25), fused by rank.
pub async fn search(
    store: &MemoryStore,
    embedder: &dyn Embedder,
    project_id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<DocHit>, DocIndexError> {
    let vector = embedder
        .embed_with_role(query, EmbedRole::Query)
        .await
        .map_err(|e| DocIndexError::Query(e.to_string()))?;
    let vector = to_unit(&vector).map_err(|e| DocIndexError::Query(e.to_string()))?;
    let bytes = le_bytes(&vector);
    let expression = fts_match_expression(query, MAX_LEXICAL_TOKENS);
    let project_id = project_id.to_string();
    Ok(store
        .conn
        .call(move |c| -> rusqlite::Result<Vec<DocHit>> {
            let k = CANDIDATES_PER_CHANNEL as i64;
            let dense: Vec<i64> = c
                .prepare(
                    "SELECT rowid FROM project_doc_vectors \
                     WHERE embedding MATCH ?1 AND k = ?2 AND project_id = ?3 ORDER BY distance",
                )?
                .query_map(rusqlite::params![bytes, k, project_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let lexical: Vec<i64> = match &expression {
                Some(expression) => c
                    .prepare(
                        "SELECT rowid FROM project_doc_fts \
                         WHERE project_doc_fts MATCH ?1 AND project_id = ?2 ORDER BY rank LIMIT ?3",
                    )?
                    .query_map(rusqlite::params![expression, project_id, k], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?,
                None => Vec::new(),
            };
            let mut stmt = c.prepare("SELECT path, breadcrumb, body FROM project_doc_chunk WHERE id = ?1")?;
            fuse(&dense, &lexical, limit)
                .into_iter()
                .map(|(id, score)| {
                    stmt.query_row([id], |r| {
                        Ok(DocHit { path: r.get(0)?, breadcrumb: r.get(1)?, text: r.get(2)?, score })
                    })
                })
                .collect()
        })
        .await?)
}

/// How many docs the project's index holds.
pub async fn doc_count(store: &MemoryStore, project_id: &str) -> Result<usize, DocIndexError> {
    let id = project_id.to_string();
    let n: i64 = store
        .conn
        .call(move |c| c.query_row("SELECT COUNT(*) FROM project_doc_file WHERE project_id = ?1", [id], |r| r.get(0)))
        .await?;
    Ok(n as usize)
}

struct ScannedDoc {
    file: DocFile,
    hash: String,
}

struct Scan {
    docs: Vec<ScannedDoc>,
    skipped: usize,
}

fn scan(root: &Path) -> Result<Scan, CorpusError> {
    let walk = corpus::walk(root)?;
    let mut scan = Scan { docs: Vec::with_capacity(walk.docs.len()), skipped: walk.skipped };
    for file in walk.docs {
        match corpus::read(&file) {
            Some((_, hash)) => scan.docs.push(ScannedDoc { file, hash }),
            None => scan.skipped += 1,
        }
    }
    Ok(scan)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()))
}

pub(crate) async fn refresh(
    store: &MemoryStore,
    embedder: &dyn Embedder,
    splitter: &TokenSplitter,
    project_id: &str,
    root: &Path,
) -> Result<RefreshReport, DocIndexError> {
    // Walked before anything is read from the index: a root that is gone must
    // fail here, not read as an empty tree whose plan drops every doc.
    let root_buf = root.to_path_buf();
    let scan = blocking(move || scan(&root_buf)).await?;
    let stored = fingerprints(store, project_id).await?;
    let disk: Vec<(String, String)> = scan.docs.iter().map(|d| (d.file.path.clone(), d.hash.clone())).collect();
    let model_id = embedder.model_id().to_string();
    let plan = plan_refresh(&disk, &stored, &model_id);

    let mut report = RefreshReport {
        docs: disk.len(),
        unchanged: plan.keep,
        skipped: scan.skipped,
        ..Default::default()
    };
    let files: HashMap<&str, &DocFile> = scan.docs.iter().map(|d| (d.file.path.as_str(), &d.file)).collect();
    for path in &plan.index {
        let file = files[path.as_str()].clone();
        let kind = file.kind;
        // Read again rather than kept from the scan, so a refresh holds one
        // changed doc in memory at a time, not the whole tree. The hash stored
        // is the one of the bytes actually indexed.
        let Some((text, hash)) = blocking(move || corpus::read(&file)).await else {
            report.skipped += 1;
            continue;
        };
        let masked = redact(&text).text;
        let passages = match kind {
            DocKind::Markdown => chunk_markdown(path, &masked, splitter),
            DocKind::Manifest => chunk_manifest(path, &masked, splitter),
            DocKind::Code(lang) => chunk_code(path, &masked, lang, splitter),
        };
        let vectors = match embed_passages(embedder, path, &passages).await {
            Ok(vectors) => vectors,
            Err(DocIndexError::Vector { path, reason }) => {
                report.failed.push(FailedDoc { path, reason });
                continue;
            }
            Err(e) => return Err(e),
        };
        report.passages += passages.len();
        let doc = IndexedDoc { path: path.clone(), hash, model_id: model_id.clone(), passages, vectors };
        replace_doc(store, project_id, doc).await?;
        report.indexed += 1;
    }
    remove_docs(store, project_id, plan.remove.clone()).await?;
    report.removed = plan.remove.len();
    Ok(report)
}

async fn embed_passages(
    embedder: &dyn Embedder,
    path: &str,
    passages: &[Passage],
) -> Result<Vec<Vec<u8>>, DocIndexError> {
    if passages.is_empty() {
        return Ok(Vec::new());
    }
    let texts: Vec<String> = passages.iter().map(Passage::indexed_text).collect();
    let vectors = embedder
        .embed_batch(&texts, EmbedRole::Passage)
        .await
        .map_err(|source| DocIndexError::Embed { path: path.to_string(), source })?;
    if vectors.len() != texts.len() {
        return Err(DocIndexError::VectorCount {
            path: path.to_string(),
            vectors: vectors.len(),
            passages: texts.len(),
        });
    }
    vectors
        .iter()
        .map(|v| to_unit(v).map(|u| le_bytes(&u)))
        .collect::<Result<_, _>>()
        .map_err(|e| DocIndexError::Vector { path: path.to_string(), reason: e.to_string() })
}

fn le_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// The root `project_id` was registered with, if the daemon may read it.
///
/// Every read of a project's files starts here, so a root that was not
/// accepted is refused before anything walks it.
pub async fn root_of(store: &MemoryStore, project_id: &str) -> Result<PathBuf, DocIndexError> {
    let id = project_id.to_string();
    let row: Option<(String, bool)> = store
        .conn
        .call(move |c| -> rusqlite::Result<_> {
            use rusqlite::OptionalExtension;
            c.query_row("SELECT root_path, index_docs FROM project_path WHERE id = ?1", [&id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()
        })
        .await?;
    match row {
        Some((root, true)) => Ok(PathBuf::from(root)),
        Some((root, false)) => Err(DocIndexError::NotAccepted(PathBuf::from(root))),
        None => Err(DocIndexError::UnknownProject(project_id.to_string())),
    }
}

async fn fingerprints(store: &MemoryStore, project_id: &str) -> Result<Vec<Fingerprint>, DocIndexError> {
    let id = project_id.to_string();
    Ok(store
        .conn
        .call(move |c| -> rusqlite::Result<_> {
            let mut stmt = c.prepare(
                "SELECT path, content_hash, model_id FROM project_doc_file WHERE project_id = ?1",
            )?;
            let rows = stmt
                .query_map([&id], |r| Ok(Fingerprint { path: r.get(0)?, hash: r.get(1)?, model_id: r.get(2)? }))?
                .collect::<rusqlite::Result<Vec<_>>>();
            rows
        })
        .await?)
}

struct IndexedDoc {
    path: String,
    hash: String,
    model_id: String,
    passages: Vec<Passage>,
    vectors: Vec<Vec<u8>>,
}

/// Swaps a doc's passages for new ones in one transaction, so a search sees
/// the old doc or the new one, never half of each.
async fn replace_doc(store: &MemoryStore, project_id: &str, doc: IndexedDoc) -> Result<(), DocIndexError> {
    let project_id = project_id.to_string();
    let now = TimestampMs::now().as_millis_i64();
    store
        .conn
        .call(move |c| -> rusqlite::Result<()> {
            let tx = c.transaction()?;
            delete_doc(&tx, &project_id, &doc.path)?;
            for (passage, vector) in doc.passages.iter().zip(&doc.vectors) {
                tx.execute(
                    "INSERT INTO project_doc_chunk (project_id, path, ordinal, breadcrumb, body) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![project_id, doc.path, passage.ordinal as i64, passage.breadcrumb, passage.body],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO project_doc_vectors (rowid, project_id, embedding) VALUES (?1, ?2, ?3)",
                    rusqlite::params![id, project_id, vector],
                )?;
                tx.execute(
                    "INSERT INTO project_doc_fts (rowid, text, project_id) VALUES (?1, ?2, ?3)",
                    rusqlite::params![id, passage.indexed_text(), project_id],
                )?;
            }
            tx.execute(
                "INSERT INTO project_doc_file (project_id, path, content_hash, model_id, chunks, indexed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![project_id, doc.path, doc.hash, doc.model_id, doc.passages.len() as i64, now],
            )?;
            tx.commit()
        })
        .await?;
    Ok(())
}

async fn remove_docs(store: &MemoryStore, project_id: &str, paths: Vec<String>) -> Result<(), DocIndexError> {
    if paths.is_empty() {
        return Ok(());
    }
    let project_id = project_id.to_string();
    store
        .conn
        .call(move |c| -> rusqlite::Result<()> {
            let tx = c.transaction()?;
            for path in &paths {
                delete_doc(&tx, &project_id, path)?;
            }
            tx.commit()
        })
        .await?;
    Ok(())
}

fn delete_doc(tx: &rusqlite::Transaction<'_>, project_id: &str, path: &str) -> rusqlite::Result<()> {
    let ids: Vec<i64> = tx
        .prepare("SELECT id FROM project_doc_chunk WHERE project_id = ?1 AND path = ?2")?
        .query_map([project_id, path], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in ids {
        tx.execute("DELETE FROM project_doc_vectors WHERE rowid = ?1", [id])?;
        tx.execute("DELETE FROM project_doc_fts WHERE rowid = ?1", [id])?;
    }
    tx.execute("DELETE FROM project_doc_chunk WHERE project_id = ?1 AND path = ?2", [project_id, path])?;
    tx.execute("DELETE FROM project_doc_file WHERE project_id = ?1 AND path = ?2", [project_id, path])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::memory::embedder::FakeEmbedder;

    /// `FakeEmbedder` under another model id, counting every text it embeds.
    struct Counting {
        model: &'static str,
        texts: AtomicUsize,
    }

    impl Counting {
        fn new(model: &'static str) -> Self {
            Self { model, texts: AtomicUsize::new(0) }
        }
        fn embedded(&self) -> usize {
            self.texts.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl Embedder for Counting {
        async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            self.texts.fetch_add(1, Ordering::SeqCst);
            FakeEmbedder::default().embed_with_role(text, role).await
        }
        fn dimensions(&self) -> usize { 384 }
        fn model_id(&self) -> &str { self.model }
        fn query_prefix(&self) -> &str { "" }
        fn passage_prefix(&self) -> &str { "" }
    }

    struct Failing;

    #[async_trait::async_trait]
    impl Embedder for Failing {
        async fn embed_with_role(&self, _: &str, _: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            Err(EmbedError::InferenceFailed("boom".into()))
        }
        fn dimensions(&self) -> usize { 384 }
        fn model_id(&self) -> &str { "m1" }
        fn query_prefix(&self) -> &str { "" }
        fn passage_prefix(&self) -> &str { "" }
    }

    fn fp(path: &str, hash: &str, model: &str) -> Fingerprint {
        Fingerprint { path: path.into(), hash: hash.into(), model_id: model.into() }
    }

    fn on_disk(docs: &[(&str, &str)]) -> Vec<(String, String)> {
        docs.iter().map(|(p, h)| (p.to_string(), h.to_string())).collect()
    }

    #[test]
    fn plan_keeps_what_did_not_change_and_rewrites_what_did() {
        let disk = on_disk(&[("a.md", "h1"), ("b.md", "h2-new"), ("c.md", "h3")]);
        let stored = [fp("a.md", "h1", "m1"), fp("b.md", "h2", "m1"), fp("gone.md", "h4", "m1")];

        let plan = plan_refresh(&disk, &stored, "m1");

        assert_eq!(
            plan,
            Plan { index: vec!["b.md".into(), "c.md".into()], keep: 1, remove: vec!["gone.md".into()] }
        );
    }

    #[test]
    fn plan_rewrites_every_doc_embedded_by_another_model() {
        let disk = on_disk(&[("a.md", "h1"), ("b.md", "h2")]);
        let stored = [fp("a.md", "h1", "m1"), fp("b.md", "h2", "m1")];

        let plan = plan_refresh(&disk, &stored, "m2");

        assert_eq!(plan.index, ["a.md", "b.md"]);
        assert_eq!((plan.keep, plan.remove.len()), (0, 0));
    }

    #[test]
    fn plan_of_an_empty_tree_drops_everything() {
        let plan = plan_refresh(&[], &[fp("a.md", "h1", "m1")], "m1");
        assert_eq!(plan.remove, ["a.md"]);
    }

    #[test]
    fn a_refresh_is_due_when_never_run_or_old_enough() {
        let now = Instant::now();
        let interval = Duration::from_secs(60);
        assert!(needs_refresh(None, now, interval));
        assert!(!needs_refresh(Some(now), now, interval));
        assert!(needs_refresh(now.checked_sub(Duration::from_secs(61)), now, interval));
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    async fn store_with_project(project_id: &str, root: &Path) -> MemoryStore {
        let store = MemoryStore::new_for_test().await.unwrap();
        let (id, root) = (project_id.to_string(), root.display().to_string());
        store
            .conn
            .call(move |c| c.execute("INSERT INTO project_path (id, root_path, created_at) VALUES (?1, ?2, 0)", [id, root]))
            .await
            .unwrap();
        store
    }

    async fn passages(store: &MemoryStore, project_id: &str) -> Vec<(String, String, String)> {
        let id = project_id.to_string();
        store
            .conn
            .call(move |c| -> rusqlite::Result<_> {
                let mut stmt = c.prepare(
                    "SELECT path, breadcrumb, body FROM project_doc_chunk WHERE project_id = ?1 \
                     ORDER BY path, ordinal",
                )?;
                let rows = stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect();
                rows
            })
            .await
            .unwrap()
    }

    /// Rows in each table of the index, across every project.
    async fn index_sizes(store: &MemoryStore) -> (i64, i64, i64, i64) {
        store
            .conn
            .call(|c| {
                c.query_row(
                    "SELECT (SELECT COUNT(*) FROM project_doc_file), (SELECT COUNT(*) FROM project_doc_chunk), \
                            (SELECT COUNT(*) FROM project_doc_vectors), (SELECT COUNT(*) FROM project_doc_fts)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
            })
            .await
            .unwrap()
    }

    async fn refreshed(
        index: &Arc<DocIndex>,
        store: &MemoryStore,
        embedder: &dyn Embedder,
        project_id: &str,
    ) -> RefreshReport {
        match index.try_refresh(store, embedder, None, project_id).await.unwrap() {
            RefreshOutcome::Refreshed(report) => report,
            RefreshOutcome::AlreadyRunning => panic!("nothing else was refreshing"),
        }
    }

    #[tokio::test]
    async fn an_unchanged_project_embeds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "README.md", "# Readme\nhello\n## Usage\nrun it\n");
        write(dir.path(), "Cargo.toml", "[package]\nname = \"x\"\n");
        let store = store_with_project("p", dir.path()).await;
        let index = Arc::new(DocIndex::default());
        let embedder = Counting::new("m1");

        let first = refreshed(&index, &store, &embedder, "p").await;
        let embedded = embedder.embedded();
        let second = refreshed(&index, &store, &embedder, "p").await;

        assert_eq!(first, RefreshReport { docs: 2, indexed: 2, passages: 3, ..Default::default() });
        assert_eq!(embedded, 3);
        assert_eq!(second, RefreshReport { docs: 2, unchanged: 2, ..Default::default() });
        assert_eq!(embedder.embedded(), embedded, "nothing changed, so nothing is embedded");
        assert_eq!(index_sizes(&store).await, (2, 3, 3, 3));
    }

    #[tokio::test]
    async fn a_changed_doc_replaces_its_passages_and_a_deleted_one_loses_them() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "# A\nold text\n## Two\nmore old text\n");
        write(dir.path(), "b.md", "# B\nb text\n");
        let store = store_with_project("p", dir.path()).await;
        let index = Arc::new(DocIndex::default());
        let embedder = Counting::new("m1");
        refreshed(&index, &store, &embedder, "p").await;

        write(dir.path(), "a.md", "# A\nnew text\n");
        std::fs::remove_file(dir.path().join("b.md")).unwrap();
        let report = refreshed(&index, &store, &embedder, "p").await;

        assert_eq!(report, RefreshReport { docs: 1, indexed: 1, removed: 1, passages: 1, ..Default::default() });
        assert_eq!(passages(&store, "p").await, [("a.md".into(), "a.md > A".into(), "new text".into())]);
        assert_eq!(index_sizes(&store).await, (1, 1, 1, 1));
    }

    #[tokio::test]
    async fn another_model_reindexes_every_doc() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "# A\na\n");
        write(dir.path(), "b.md", "# B\nb\n");
        let store = store_with_project("p", dir.path()).await;
        let index = Arc::new(DocIndex::default());
        refreshed(&index, &store, &Counting::new("m1"), "p").await;

        let report = refreshed(&index, &store, &Counting::new("m2"), "p").await;

        assert_eq!((report.indexed, report.unchanged), (2, 0));
        assert_eq!(index_sizes(&store).await, (2, 2, 2, 2));
    }

    #[tokio::test]
    async fn a_missing_root_deletes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        write(&root, "a.md", "# A\na\n");
        let store = store_with_project("p", &root).await;
        let index = Arc::new(DocIndex::default());
        refreshed(&index, &store, &Counting::new("m1"), "p").await;

        std::fs::remove_dir_all(&root).unwrap();
        let result = index.try_refresh(&store, &Counting::new("m1"), None, "p").await;

        assert!(matches!(result, Err(DocIndexError::Corpus(CorpusError::NotADirectory(_)))), "{result:?}");
        assert_eq!(index_sizes(&store).await, (1, 1, 1, 1));
    }

    #[tokio::test]
    async fn a_failed_embedding_keeps_the_doc_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "# A\nfirst\n");
        let store = store_with_project("p", dir.path()).await;
        let index = Arc::new(DocIndex::default());
        refreshed(&index, &store, &Counting::new("m1"), "p").await;
        let before = passages(&store, "p").await;

        write(dir.path(), "a.md", "# A\nsecond\n");
        let failed = index.try_refresh(&store, &Failing, None, "p").await;

        assert!(matches!(failed, Err(DocIndexError::Embed { ref path, .. }) if path == "a.md"), "{failed:?}");
        assert_eq!(passages(&store, "p").await, before);
        let retried = refreshed(&index, &store, &Counting::new("m1"), "p").await;
        assert_eq!(retried.indexed, 1, "the next refresh picks the doc up again");
        assert_eq!(passages(&store, "p").await[0].2, "second");
    }

    /// `FakeEmbedder`, except that a text holding `BROKEN` embeds to a
    /// vector with a NaN in it.
    struct Unusable;

    #[async_trait::async_trait]
    impl Embedder for Unusable {
        async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            let mut v = FakeEmbedder::default().embed_with_role(text, role).await?;
            if text.contains("BROKEN") {
                v[0] = f32::NAN;
            }
            Ok(v)
        }
        fn dimensions(&self) -> usize { 384 }
        fn model_id(&self) -> &str { "m1" }
        fn query_prefix(&self) -> &str { "" }
        fn passage_prefix(&self) -> &str { "" }
    }

    /// Answers a batch with one vector fewer than it was given texts.
    struct ShortBatch;

    #[async_trait::async_trait]
    impl Embedder for ShortBatch {
        async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            FakeEmbedder::default().embed_with_role(text, role).await
        }
        async fn embed_batch(&self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
            let mut out = Vec::new();
            for text in texts.iter().skip(1) {
                out.push(self.embed_with_role(text, role).await?);
            }
            Ok(out)
        }
        fn dimensions(&self) -> usize { 384 }
        fn model_id(&self) -> &str { "m1" }
        fn query_prefix(&self) -> &str { "" }
        fn passage_prefix(&self) -> &str { "" }
    }

    #[tokio::test]
    async fn an_unusable_vector_skips_its_doc_and_the_rest_go_on() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "gone.md", "# Gone\nsoon deleted\n");
        let store = store_with_project("p", dir.path()).await;
        let index = Arc::new(DocIndex::default());
        refreshed(&index, &store, &Counting::new("m1"), "p").await;

        std::fs::remove_file(dir.path().join("gone.md")).unwrap();
        write(dir.path(), "a.md", "# A\nBROKEN passage\n");
        write(dir.path(), "b.md", "# B\nsound passage\n");
        let report = refreshed(&index, &store, &Unusable, "p").await;

        assert_eq!(report.indexed, 1, "{report:?}");
        assert_eq!(report.removed, 1, "{report:?}");
        assert_eq!(report.failed.len(), 1, "{report:?}");
        assert_eq!(report.failed[0].path, "a.md");
        assert!(report.failed[0].reason.contains("finite"), "{report:?}");
        assert_eq!(passages(&store, "p").await, [("b.md".into(), "b.md > B".into(), "sound passage".into())]);

        let retried = refreshed(&index, &store, &Counting::new("m1"), "p").await;
        assert_eq!((retried.indexed, retried.failed.len()), (1, 0), "the next refresh picks the doc up again");
    }

    #[tokio::test]
    async fn a_short_batch_still_aborts_the_refresh() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "# A\na\n");
        write(dir.path(), "b.md", "# B\nb\n");
        let store = store_with_project("p", dir.path()).await;
        let index = Arc::new(DocIndex::default());

        let result = index.try_refresh(&store, &ShortBatch, None, "p").await;

        assert!(matches!(result, Err(DocIndexError::VectorCount { ref path, .. }) if path == "a.md"), "{result:?}");
        assert_eq!(index_sizes(&store).await, (0, 0, 0, 0));
    }

    #[tokio::test]
    async fn the_last_error_lasts_until_a_refresh_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "# A\na\n");
        let store = store_with_project("p", dir.path()).await;
        let index = Arc::new(DocIndex::default());
        assert_eq!(index.last_error("p"), None);

        index.try_refresh(&store, &Failing, None, "p").await.unwrap_err();
        assert!(index.last_error("p").is_some_and(|e| e.contains("boom")), "{:?}", index.last_error("p"));

        let running = index.begin("p").unwrap();
        let skipped = index.try_refresh(&store, &Counting::new("m1"), None, "p").await.unwrap();
        assert_eq!(skipped, RefreshOutcome::AlreadyRunning);
        drop(running);
        assert!(index.last_error("p").is_some(), "a refresh that did not run changes nothing");

        refreshed(&index, &store, &Counting::new("m1"), "p").await;
        assert_eq!(index.last_error("p"), None);
    }

    #[tokio::test]
    async fn a_second_refresh_does_not_wait_for_the_first() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "# A\na\n");
        let store = store_with_project("p", dir.path()).await;
        let index = Arc::new(DocIndex::default());

        let running = index.begin("p").expect("nothing is running yet");
        assert!(index.is_running("p"));
        let second = index.try_refresh(&store, &Counting::new("m1"), None, "p").await.unwrap();
        assert_eq!(second, RefreshOutcome::AlreadyRunning);
        assert!(index.try_refresh(&store, &Counting::new("m1"), None, "other").await.is_err(), "other projects are not held");

        drop(running);
        assert!(!index.is_running("p"));
        assert!(matches!(
            index.try_refresh(&store, &Counting::new("m1"), None, "p").await.unwrap(),
            RefreshOutcome::Refreshed(_)
        ));
    }

    #[tokio::test]
    async fn waiting_for_a_running_refresh_ends_when_it_does() {
        let index = Arc::new(DocIndex::default());
        assert!(index.wait_idle("p", Duration::ZERO).await, "nothing running is idle at once");

        let guard = index.begin("p").unwrap();
        assert!(!index.wait_idle("p", Duration::from_millis(20)).await, "gives up at the limit");

        let waiter = {
            let index = index.clone();
            tokio::spawn(async move { index.wait_idle("p", Duration::from_secs(10)).await })
        };
        tokio::task::yield_now().await;
        drop(guard);
        assert!(waiter.await.unwrap());
    }

    #[tokio::test]
    async fn a_refresh_is_due_again_only_after_the_interval() {
        let index = Arc::new(DocIndex::default());
        let now = Instant::now();
        assert!(index.is_due("p", now));
        let guard = index.begin("p").unwrap();
        assert!(!index.is_due("p", now + REFRESH_INTERVAL * 2), "not while running");
        drop(guard);
        assert!(!index.is_due("p", Instant::now()));
        assert!(index.is_due("p", Instant::now() + REFRESH_INTERVAL));
    }

    #[tokio::test]
    async fn begin_if_due_claims_only_a_due_idle_project() {
        let index = Arc::new(DocIndex::default());
        let now = Instant::now();
        let guard = index.begin_if_due("p", now).expect("never refreshed, so due");
        assert!(index.is_running("p"));
        assert!(index.begin_if_due("p", now + REFRESH_INTERVAL * 2).is_none(), "not while running");
        drop(guard);
        assert!(index.begin_if_due("p", Instant::now()).is_none(), "not before the interval");
        assert!(index.begin_if_due("p", Instant::now() + REFRESH_INTERVAL).is_some());
    }

    #[tokio::test]
    async fn an_unregistered_project_is_refused() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let index = Arc::new(DocIndex::default());
        let result = index.try_refresh(&store, &Counting::new("m1"), None, "nobody").await;
        assert!(matches!(result, Err(DocIndexError::UnknownProject(ref p)) if p == "nobody"), "{result:?}");
    }

    #[tokio::test]
    async fn a_root_that_was_not_accepted_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "# A\na\n");
        let store = store_with_project("p", dir.path()).await;
        store.conn.call(|c| c.execute("UPDATE project_path SET index_docs = 0 WHERE id = 'p'", [])).await.unwrap();
        let index = Arc::new(DocIndex::default());

        let result = index.try_refresh(&store, &Counting::new("m1"), None, "p").await;

        assert!(matches!(result, Err(DocIndexError::NotAccepted(ref root)) if *root == dir.path()), "{result:?}");
        assert_eq!(index_sizes(&store).await, (0, 0, 0, 0));
    }

    #[tokio::test]
    async fn a_secret_in_a_doc_never_reaches_the_index() {
        let token = format!("ghp_{}", "a1B2".repeat(9));
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "docs/setup.md", &format!("# Setup\nexport GITHUB_TOKEN={token}\n"));
        let store = store_with_project("p", dir.path()).await;
        let index = Arc::new(DocIndex::default());

        refreshed(&index, &store, &Counting::new("m1"), "p").await;

        let bodies = passages(&store, "p").await;
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].2.contains("[REDACTED:github_token]"), "{}", bodies[0].2);
        let indexed: Vec<String> = store
            .conn
            .call(|c| -> rusqlite::Result<_> {
                let mut stmt = c.prepare("SELECT text FROM project_doc_fts")?;
                let rows = stmt.query_map([], |r| r.get(0))?.collect();
                rows
            })
            .await
            .unwrap();
        assert!(indexed.iter().all(|t| !t.contains(&token)));
        assert!(bodies.iter().all(|(_, _, b)| !b.contains(&token)));
    }

    /// Places a text on one axis per topic it mentions, so a test decides
    /// which texts mean the same thing without sharing a word.
    struct Topical;

    const TOPICS: &[&[&str]] = &[&["cola", "apagar", "paro", "saves", "pendientes"], &["port", "puerto"]];

    #[async_trait::async_trait]
    impl Embedder for Topical {
        async fn embed_with_role(&self, text: &str, _: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            let lower = text.to_lowercase();
            let mut v = vec![0.0f32; 384];
            v[383] = 0.1;
            for (axis, words) in TOPICS.iter().enumerate() {
                v[axis] = words.iter().filter(|w| lower.contains(*w)).count() as f32;
            }
            Ok(v)
        }
        fn dimensions(&self) -> usize { 384 }
        fn model_id(&self) -> &str { "topical" }
        fn query_prefix(&self) -> &str { "" }
        fn passage_prefix(&self) -> &str { "" }
    }

    async fn indexed(docs: &[(&str, &str)]) -> (tempfile::TempDir, MemoryStore) {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in docs {
            write(dir.path(), path, text);
        }
        let store = store_with_project("p", dir.path()).await;
        refreshed(&Arc::new(DocIndex::default()), &store, &Topical, "p").await;
        (dir, store)
    }

    fn crumbs(hits: &[DocHit]) -> Vec<&str> {
        hits.iter().map(|h| h.breadcrumb.as_str()).collect()
    }

    #[test]
    fn fuse_adds_the_reciprocal_ranks_of_both_lists() {
        let fused = fuse(&[1, 2, 3], &[3, 4], 10);
        let ids: Vec<i64> = fused.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [3, 1, 2, 4]);
        assert!((fused[0].1 - (1.0 / 63.0 + 1.0 / 61.0)).abs() < 1e-12);
        assert!((fused[1].1 - 1.0 / 61.0).abs() < 1e-12);
    }

    #[test]
    fn fuse_breaks_ties_by_id_and_stops_at_the_limit() {
        assert_eq!(fuse(&[7], &[5], 10), [(5, 1.0 / 61.0), (7, 1.0 / 61.0)]);
        assert_eq!(fuse(&[1, 2, 3], &[], 2).len(), 2);
        assert!(fuse(&[], &[], 5).is_empty());
    }

    #[tokio::test]
    async fn search_finds_a_passage_by_meaning() {
        let (_dir, store) = indexed(&[
            ("ops.md", "# Apagado\nAl apagar, la cola de ingesta se vuelca a disco.\n"),
            ("net.md", "# Network\nChoosing the port number.\n"),
        ])
        .await;

        let hits = search(&store, &Topical, "p", "¿qué pasa con los saves pendientes si paro el daemon?", 5)
            .await
            .unwrap();

        assert_eq!(crumbs(&hits)[0], "ops.md > Apagado");
        assert_eq!(hits[0].path, "ops.md");
        assert_eq!(hits[0].text, "Al apagar, la cola de ingesta se vuelca a disco.");
    }

    #[tokio::test]
    async fn search_finds_an_identifier_lexically() {
        let (_dir, store) = indexed(&[
            ("a.md", "# One\nnothing here\n# Two\nnor here\n# Three\nor here\n"),
            ("chunker.md", "# Cap\nEl puerto del tope es CHUNK_MAX_TOKENS.\n"),
        ])
        .await;

        let dense_only = search(&store, &Topical, "p", "zzz", 5).await.unwrap();
        let hits = search(&store, &Topical, "p", "CHUNK_MAX_TOKENS", 5).await.unwrap();

        assert_eq!(crumbs(&dense_only).last(), Some(&"chunker.md > Cap"), "by meaning it is the farthest");
        assert_eq!(crumbs(&hits)[0], "chunker.md > Cap");
    }

    #[tokio::test]
    async fn a_refresh_indexes_code_by_definition() {
        let (_dir, store) = indexed(&[
            ("README.md", "# Readme\nhello\n"),
            ("src/profile.rs", "/// Reads the profile flag.\npub fn parse_profile(raw: &str) -> Option<Profile> {\n    raw.parse().ok()\n}\n"),
            ("src/notes.txt", "parse_profile is not code here"),
        ])
        .await;

        let rows = passages(&store, "p").await;
        let hits = search(&store, &Topical, "p", "parse_profile", 5).await.unwrap();

        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[1].1, "src/profile.rs");
        assert!(rows[1].2.starts_with("/// Reads the profile flag.\npub fn parse_profile"), "{}", rows[1].2);
        assert_eq!(hits[0].path, "src/profile.rs");
    }

    #[tokio::test]
    async fn search_never_crosses_projects() {
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();
        write(one.path(), "README.md", "# Readme\nthe port is chosen at start\n");
        write(two.path(), "README.md", "# Readme\nthe port is chosen at start\n# Elsewhere\nport port port\n");
        let store = store_with_project("one", one.path()).await;
        let (id, root) = ("two".to_string(), two.path().display().to_string());
        store
            .conn
            .call(move |c| c.execute("INSERT INTO project_path (id, root_path, created_at) VALUES (?1, ?2, 0)", [id, root]))
            .await
            .unwrap();
        let index = Arc::new(DocIndex::default());
        refreshed(&index, &store, &Topical, "one").await;
        refreshed(&index, &store, &Topical, "two").await;

        let hits = search(&store, &Topical, "one", "port", 10).await.unwrap();

        assert_eq!(crumbs(&hits), ["README.md > Readme"]);
        assert_eq!(doc_count(&store, "one").await.unwrap(), 1);
        assert_eq!(doc_count(&store, "nobody").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn indexing_docs_leaves_memory_search_unchanged() {
        use crate::memory::chunker::Chunk;
        use crate::memory::events::{MemoryEvent, MemorySource};
        use crate::memory::store_insert::NewMemoryRecord;
        use crate::memory::{search_all_memories, SearchScope};
        use crate::models::chat::Role;

        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "README.md", "# Port\nthe daemon port is 27381\n# Shutdown\nthe daemon spills the queue\n");
        let store = store_with_project("p", dir.path()).await;
        for content in ["the daemon port is 27381", "the daemon spills the queue on stop"] {
            let evt = MemoryEvent {
                agent_id: "agent".into(),
                project_id: Some("p".into()),
                role: Role::User,
                content: content.into(),
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
            let chunk = Chunk { text: content.into() };
            let vector = FakeEmbedder::default().embed_with_role(content, EmbedRole::Passage).await.unwrap();
            store.insert_memory_with_vector(NewMemoryRecord::from_event_and_chunk(&evt, &chunk), vector).await.unwrap();
        }
        let query = "the daemon port is 27381";
        let embedding = FakeEmbedder::default().embed_with_role(query, EmbedRole::Query).await.unwrap();
        let memories = || async {
            search_all_memories(&store, &embedding, Some(query), "agent", SearchScope::all_default().with_project(Some("p")), 10)
                .await
                .unwrap()
                .into_iter()
                .map(|h| (h.id, h.content))
                .collect::<Vec<_>>()
        };
        let before = memories().await;

        refreshed(&Arc::new(DocIndex::default()), &store, &FakeEmbedder::default(), "p").await;

        assert!(!before.is_empty());
        assert_eq!(memories().await, before);
    }

    #[tokio::test]
    async fn refreshing_never_touches_memories() {
        use crate::memory::chunker::Chunk;
        use crate::memory::events::{MemoryEvent, MemorySource};
        use crate::memory::store_insert::NewMemoryRecord;
        use crate::models::chat::Role;

        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "README.md", "# Readme\nthe daemon listens on loopback\n");
        let store = store_with_project("p", dir.path()).await;
        let evt = MemoryEvent {
            agent_id: "agent".into(),
            project_id: Some("p".into()),
            role: Role::User,
            content: "the daemon listens on loopback".into(),
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
        let chunk = Chunk { text: evt.content.clone() };
        let vector = FakeEmbedder::default().embed_with_role(&evt.content, EmbedRole::Passage).await.unwrap();
        store.insert_memory_with_vector(NewMemoryRecord::from_event_and_chunk(&evt, &chunk), vector).await.unwrap();
        let memories = |c: &mut rusqlite::Connection| -> rusqlite::Result<String> {
            c.query_row(
                "SELECT json_array((SELECT json_group_array(json_array(id, content, tier, status, access_count, \
                                    last_accessed_at, deleted_at)) FROM memory), \
                                   (SELECT COUNT(*) FROM memory_vectors), \
                                   (SELECT COUNT(*) FROM memory_fts WHERE memory_fts MATCH 'daemon'))",
                [],
                |r| r.get(0),
            )
        };
        let before = store.conn.call(memories).await.unwrap();

        refreshed(&Arc::new(DocIndex::default()), &store, &Counting::new("m1"), "p").await;

        assert_eq!(store.conn.call(memories).await.unwrap(), before);
        assert_eq!(index_sizes(&store).await, (1, 1, 1, 1));
    }
}
