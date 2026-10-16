//! Host-agnostic bootstrap for the memory substrate.
//!
//! [`MemoryRuntime`] is everything a process needs to have working memory —
//! the open store, the ingest channel, the embedder and tokenizer cells, the
//! access tracker — assembled once and handed over as owned handles.
//!
//! # Why this exists
//!
//! Assembling those handles is independent of any UI or process framework:
//! `clawg serve` (the loopback daemon) and `clawg mcp` (the stdio bridge) need
//! the identical set. The only thing a host still chooses for itself is
//! *which executor* the background futures are spawned on — which is why
//! [`MemoryRuntime::ingest_worker`] and [`MemoryRuntime::maintenance_worker`]
//! return futures instead of spawning.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use once_cell::sync::OnceCell;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::memory::access::AccessTracker;
use crate::memory::backup::{self, Reason};
use crate::memory::bootstrap::EmbedderStatus;
use crate::memory::embedder::Embedder;
use crate::memory::events::MemoryEvent;
use crate::memory::store_lock::StoreHolder;
use crate::memory::maintenance::{run_maintenance_loop, MAINTENANCE_PERIOD, STARTUP_DELAY};
use crate::memory::outbox::{OutboxState, Recorded};
use crate::memory::redact::{redact, SecretKind};
use crate::memory::project_docs::DocIndex;
use crate::memory::spill;
use crate::memory::worker::{memory_worker, resolve_tokenizer_and_chunker, IngestReport, IngestRequest, Replay};
use crate::memory::workspace::WorkspaceManager;
use crate::memory::{MemoryError, MemoryStore};

/// How many saves the ingest queue holds before a producer is refused.
///
/// The worker drains an event in tens of milliseconds, so the queue only
/// fills while it is held for the embedder; the bound keeps a model that is
/// still downloading from turning every save into unbounded memory.
pub const INGEST_QUEUE_CAPACITY: usize = 1024;

/// How long a shutdown keeps ingesting what is queued before it leaves the
/// rest in the outbox for the next start. It must leave `clawg stop`'s timeout
/// room for the event in flight, the access flush and the checkpoint, so the stop sees
/// the process end; a test in `clawg-cli` pins that relation. A starting value
/// chosen for that margin, not measured.
pub const DRAIN_DEADLINE: Duration = Duration::from_secs(10);

/// Why [`MemoryRuntime::accept`] refused a save. Nothing of it is kept.
#[derive(Debug, thiserror::Error)]
pub enum AcceptError {
    #[error("the ingest queue is full")]
    QueueFull,
    #[error("the ingest worker has stopped")]
    WorkerStopped,
    #[error(transparent)]
    Store(#[from] MemoryError),
}

/// A save [`MemoryRuntime::accept`] took.
#[derive(Debug, PartialEq, Eq)]
pub enum Accepted {
    /// Recorded in the outbox and queued for the worker.
    Queued { row: i64, operation_id: String, redacted: Vec<SecretKind> },
    /// The agent already has a row under this operation id: nothing new was
    /// recorded or queued.
    Existing { row: i64, operation_id: String, state: OutboxState, last_error: Option<String> },
}

/// The background tasks a host spawned from a [`MemoryRuntime`], handed back
/// to [`MemoryRuntime::shutdown`].
pub struct Workers {
    /// The ingest worker's drain signal; shutdown sends the deadline on it.
    pub drain: watch::Sender<Option<Instant>>,
    /// [`MemoryRuntime::ingest_worker`], spawned.
    pub ingest: JoinHandle<Vec<MemoryEvent>>,
    /// [`MemoryRuntime::maintenance_worker`], spawned.
    pub maintenance: JoinHandle<()>,
}

/// The memory substrate's live handles for one process.
///
/// Every field is deliberately public and deliberately cheap to clone: the
/// hosts can destructure them, and the daemon holds the struct itself.
/// `MemoryStore` is a channel handle,
/// `Sender` is `Arc`-like, and the two cells plus the tracker are
/// already behind `Arc` — so cloning a field is a refcount bump, never a
/// second database or a second flush worker.
pub struct MemoryRuntime {
    /// Profile data directory (`clawg_core::paths::data_dir`). The store, the
    /// tokenizer files and the workspaces tree all hang off it.
    pub base_dir: PathBuf,
    /// SQLite-vec persistent store, already migrated.
    pub store: Arc<MemoryStore>,
    /// Producer side of the ingest pipeline, bounded at
    /// [`INGEST_QUEUE_CAPACITY`]. Whatever hot path produces events (a chat
    /// turn, an MCP `save_memory` call) MUST NOT block behind embedding, so
    /// producers use `try_send` and treat a full queue as a refusal.
    pub memory_tx: Sender<IngestRequest>,
    /// Lazily-filled embedder cell.
    ///
    /// [`MemoryRuntime::open`] leaves it EMPTY; the host fills it by running
    /// `memory::bootstrap::run_embedder_bootstrap` in the background. A
    /// `OnceCell` is write-once by construction, so the embedder cannot be
    /// swapped at runtime — changing the model means a restart, which is the
    /// safe outcome anyway: every vector already stored belongs to the old
    /// model's space and mixing the two silently corrupts similarity.
    pub embedder: Arc<OnceCell<Arc<dyn Embedder>>>,
    /// What the bootstrap last said about the embedder. The bootstrap gets a
    /// clone of this sender; readers use [`MemoryRuntime::embedder_status`].
    pub embedder_status: watch::Sender<EmbedderStatus>,
    /// Shared tokenizer for query-context composition, filled by the ingest
    /// worker's setup step from the same `tokenizer.json` the chat chunker
    /// loads — so the two can never disagree about what a "token" is. Empty at
    /// construction in every constructor; there is no eager load here, only a
    /// shared slot for whoever fills it.
    pub tokenizer: Arc<OnceCell<Arc<tokenizers::Tokenizer>>>,
    /// Fire-and-forget access tracker for the retrieval hot path. Its flush
    /// worker is already spawned by the time this struct exists — see
    /// [`MemoryRuntime::open`]'s note about needing a tokio context.
    pub access_tracker: Arc<AccessTracker>,
    /// Which projects' documentation indexes are being refreshed, and when
    /// each last was, so the daemon never refreshes one project twice at once.
    pub project_docs: Arc<DocIndex>,
}

impl MemoryRuntime {
    /// Open the store at `<base_dir>/memory.db` and assemble the runtime.
    ///
    /// MUST be awaited from inside a tokio runtime: [`AccessTracker::new`]
    /// spawns its flush worker during construction, and `tokio::spawn` outside
    /// a runtime panics. A host with no runtime yet wraps this call in its own
    /// `block_on`.
    ///
    /// Returns the ingest `Receiver` alongside the runtime instead of storing
    /// it on the struct, for the reason an mpsc channel has exactly one
    /// consumer end: it is not `Clone`, and the runtime is shared behind `&`,
    /// so it could never be taken back out. Returning it makes the
    /// single-drainer ownership explicit at the call site rather than a
    /// runtime surprise.
    ///
    /// Fails with [`MemoryError::StoreInUse`] when another process already owns
    /// the store; `holder` is what that process will see if it is refused in
    /// turn. The ownership lasts as long as `store` (or any clone of it) lives.
    pub async fn open(
        base_dir: PathBuf,
        holder: StoreHolder,
    ) -> Result<(Self, Receiver<IngestRequest>), MemoryError> {
        let store = Arc::new(MemoryStore::open(&base_dir, holder).await?);
        let (memory_tx, memory_rx) = tokio::sync::mpsc::channel(INGEST_QUEUE_CAPACITY);
        let access_tracker = Arc::new(AccessTracker::new(store.conn.clone()));

        Ok((
            Self {
                base_dir,
                store,
                memory_tx,
                embedder: Arc::new(OnceCell::new()),
                embedder_status: watch::Sender::new(EmbedderStatus::Loading),
                tokenizer: Arc::new(OnceCell::new()),
                access_tracker,
                project_docs: Arc::default(),
            },
            memory_rx,
        ))
    }

    /// The ingest worker's future — **not** spawned.
    ///
    /// Returning the future rather than spawning it is what lets one body of
    /// code serve any host: each hands it to the `spawn` of its own executor.
    ///
    /// # This must stay the ONLY spawn site per process
    ///
    /// The dedup gate and the absorbed-duplicates counter both rely on
    /// `memory_worker` being the single sequential consumer of the channel
    /// (see `MemoryStore::reinforce_if_duplicate` in `store_insert.rs`). That
    /// guarantee holds only while exactly one task drains the receiver — and
    /// since the receiver is not `Clone`, the type system enforces "one
    /// drainer" as long as nobody spawns this future twice with two receivers
    /// off two channels. A second spawn site would race it.
    ///
    /// `drain` is the receiving end of [`Workers::drain`]: a host that will
    /// call [`MemoryRuntime::shutdown`] keeps the sender there. The future
    /// resolves to the events it did not ingest that no outbox row holds.
    pub fn ingest_worker(
        &self,
        rx: Receiver<IngestRequest>,
        drain: watch::Receiver<Option<Instant>>,
    ) -> impl Future<Output = Vec<MemoryEvent>> + Send + 'static {
        ingest_worker_future(
            self.base_dir.clone(),
            self.store.clone(),
            self.embedder.clone(),
            self.tokenizer.clone(),
            rx,
            drain,
        )
    }

    /// Takes a save: masks its secrets, records it in the outbox under
    /// `operation_id` (a new UUID when `None`), and only then queues it, so an
    /// accepted save outlives the process. A save the queue refuses is not
    /// accepted, and its row goes too.
    pub async fn accept(
        &self,
        mut event: MemoryEvent,
        operation_id: Option<String>,
        reply: Option<tokio::sync::oneshot::Sender<IngestReport>>,
    ) -> Result<Accepted, AcceptError> {
        // Masked before the row is written: the row lives in memory.db and in
        // every snapshot for as long as the save waits or stays failed.
        let masked = redact(&event.content);
        event.content = masked.text;
        let operation_id = operation_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let row = match self.store.outbox_record(operation_id.clone(), event.clone()).await? {
            Recorded::New { row } => row,
            Recorded::Existing { row, state, last_error } => {
                return Ok(Accepted::Existing { row, operation_id, state, last_error });
            }
        };
        match self.memory_tx.try_send(IngestRequest { event, reply, outbox: Some(row) }) {
            Ok(()) => Ok(Accepted::Queued { row, operation_id, redacted: masked.kinds }),
            Err(refused) => {
                if let Err(e) = self.store.outbox_forget(row).await {
                    log::warn!("could not drop the outbox row of a refused save: {e}; the next start ingests it");
                }
                Err(match refused {
                    tokio::sync::mpsc::error::TrySendError::Full(_) => AcceptError::QueueFull,
                    tokio::sync::mpsc::error::TrySendError::Closed(_) => AcceptError::WorkerStopped,
                })
            }
        }
    }

    /// The periodic maintenance loop's future — **not** spawned, same
    /// rationale as [`MemoryRuntime::ingest_worker`]. Never completes: the
    /// host drops or aborts it at shutdown.
    pub fn maintenance_worker(&self) -> impl Future<Output = ()> + Send + 'static {
        maintenance_worker_future(self.store.clone())
    }

    /// Writes a snapshot of the store to `<base_dir>/backups` through the
    /// runtime's own connection, which is the only one while it holds the
    /// store.
    pub async fn snapshot(&self, reason: Reason) -> Result<PathBuf, MemoryError> {
        let dir = crate::paths::backups_dir(&self.base_dir);
        self.store
            .conn
            .call(move |c| -> rusqlite::Result<_> { Ok(backup::snapshot_into(c, &dir, reason)) })
            .await
            .map_err(MemoryError::Connection)?
            .map_err(MemoryError::from)
    }

    /// Orderly shutdown, once nothing can send new events: drains the ingest
    /// queue for up to [`DRAIN_DEADLINE`], leaving what is left in the outbox
    /// for the next start, flushes the access tracker, stops maintenance and
    /// checkpoints the WAL.
    pub async fn shutdown(&self, workers: Workers) {
        self.shutdown_by(workers, DRAIN_DEADLINE).await;
    }

    /// [`MemoryRuntime::shutdown`] with a drain deadline short enough for a test.
    #[cfg(any(test, feature = "testing"))]
    pub async fn shutdown_within(&self, workers: Workers, drain_deadline: Duration) {
        self.shutdown_by(workers, drain_deadline).await;
    }

    async fn shutdown_by(&self, workers: Workers, drain_deadline: Duration) {
        // An error means the worker already ended; awaiting it below says how.
        let _ = workers.drain.send(Some(Instant::now() + drain_deadline));
        match workers.ingest.await {
            // Only an event queued without `accept` lacks a row, and only the
            // in-process paths of tests and evaluation queue one.
            Ok(unrecorded) if !unrecorded.is_empty() => log::warn!(
                "{} events queued without an outbox row were not ingested",
                unrecorded.len()
            ),
            Ok(_) => {}
            Err(e) => log::error!("the ingest worker ended abnormally: {e}; the outbox keeps what it held"),
        }
        self.access_tracker.shutdown().await;
        workers.maintenance.abort();
        let _ = workers.maintenance.await;
        checkpoint(&self.store).await;
    }

    /// Workspace manager rooted at `<base_dir>/workspaces`.
    pub fn workspaces(&self) -> WorkspaceManager {
        WorkspaceManager::new(self.base_dir.join("workspaces"))
    }

    /// Whether the embedder cell has been filled by the bootstrap yet.
    ///
    /// Callers use this to decide between answering and degrading, never to
    /// wait: every read path already treats an empty cell as "memory disabled"
    /// and keeps working.
    pub fn embedder_ready(&self) -> bool {
        self.embedder.get().is_some()
    }

    /// Whether the embedder is loading, ready or failed, and why it failed.
    pub fn embedder_status(&self) -> EmbedderStatus {
        self.embedder_status.borrow().clone()
    }

    /// In-memory runtime for tests: no disk I/O, and the embedder cell
    /// PRE-SEEDED with `FakeEmbedder`.
    ///
    /// Pre-seeding is what keeps tests deterministic. In production the cell
    /// is filled by a background bootstrap task that does not exist under
    /// test, so an unseeded cell would make every `embedder.get()` return
    /// `None` — or race, once a test spawns its own filler. Seeding at
    /// construction removes the window entirely.
    ///
    /// `base_dir` is still taken (and still points at a real, usually
    /// temporary, directory) because the tokenizer lookup and
    /// [`MemoryRuntime::workspaces`] resolve against it; only the database is
    /// in memory.
    #[cfg(any(test, feature = "testing"))]
    pub async fn new_for_test(base_dir: PathBuf) -> (Self, Receiver<IngestRequest>) {
        Self::new_for_test_with_capacity(base_dir, INGEST_QUEUE_CAPACITY).await
    }

    /// [`MemoryRuntime::new_for_test`] with an ingest queue of `capacity`, so
    /// a test can fill it with a handful of saves.
    #[cfg(any(test, feature = "testing"))]
    pub async fn new_for_test_with_capacity(
        base_dir: PathBuf,
        capacity: usize,
    ) -> (Self, Receiver<IngestRequest>) {
        use crate::memory::embedder::FakeEmbedder;

        let store = Arc::new(
            MemoryStore::new_for_test()
                .await
                .expect("in-memory test store"),
        );
        let (memory_tx, memory_rx) = tokio::sync::mpsc::channel(capacity);
        let access_tracker = Arc::new(AccessTracker::new(store.conn.clone()));

        let embedder: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
        // `.ok()` — `set` can only fail on an already-filled cell, which is
        // impossible on a brand-new one.
        embedder
            .set(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>)
            .ok();

        (
            Self {
                base_dir,
                store,
                memory_tx,
                embedder: Arc::new(embedder),
                // The cell is seeded above, so the embedder is already usable.
                embedder_status: watch::Sender::new(EmbedderStatus::Ready),
                tokenizer: Arc::new(OnceCell::new()),
                access_tracker,
                project_docs: Arc::default(),
            },
            memory_rx,
        )
    }
}

/// Folds the WAL into `memory.db` after the last write, so the database file
/// stands on its own once the process is gone (a copy of it, a backup, an
/// installer replacing the binary). Best-effort: everything is already
/// committed in the WAL, and SQLite replays it on the next open.
async fn checkpoint(store: &MemoryStore) {
    let result = store
        .conn
        .call(|c| -> Result<(i64, i64, i64), rusqlite::Error> {
            c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
        })
        .await;
    match result {
        Ok((0, _, _)) => log::info!("checkpointed the WAL into memory.db"),
        Ok((_, log_frames, done)) => log::warn!(
            "WAL checkpoint was blocked by another connection ({done} of {log_frames} frames copied); it completes on the next open"
        ),
        Err(e) => log::warn!("WAL checkpoint failed: {e}; it completes on the next open"),
    }
}

/// Body of [`MemoryRuntime::ingest_worker`].
///
/// Builds the chat chunker once — resolving the tokenizer as a side effect,
/// which is what fills `tokenizer_cell` for the query path — then hands
/// ownership to the worker loop.
fn ingest_worker_future(
    base_dir: PathBuf,
    store: Arc<MemoryStore>,
    embedder: Arc<OnceCell<Arc<dyn Embedder>>>,
    tokenizer: Arc<OnceCell<Arc<tokenizers::Tokenizer>>>,
    rx: Receiver<IngestRequest>,
    drain: watch::Receiver<Option<Instant>>,
) -> impl Future<Output = Vec<MemoryEvent>> + Send + 'static {
    async move {
        let chat_chunker = resolve_tokenizer_and_chunker(&base_dir, &tokenizer);
        import_spill(&base_dir, &store).await;
        let replay = match store.outbox_pending().await {
            Ok(rows) => {
                if !rows.is_empty() {
                    log::info!("ingesting {} saves accepted before this start", rows.len());
                }
                Replay::rows(rows)
            }
            Err(e) => {
                log::error!("could not read the outbox: {e}; its saves wait for the next start");
                Replay::none()
            }
        };
        memory_worker(rx, drain, replay, store, embedder, chat_chunker).await
    }
}

/// Moves the events a previous version spilled to `ingest-pending.jsonl` into
/// the outbox, masked, and removes the file once they are committed. A file
/// that cannot be moved stays for the next start.
async fn import_spill(base_dir: &std::path::Path, store: &MemoryStore) {
    let spilled = spill::load(base_dir).await;
    if spilled.is_empty() {
        return;
    }
    let events: Vec<MemoryEvent> = spilled
        .events
        .iter()
        .cloned()
        .map(|mut event| {
            event.content = redact(&event.content).text;
            event
        })
        .collect();
    match store.outbox_import(events).await {
        Ok(()) => spilled.remove().await,
        Err(e) => log::error!("could not move the spilled ingest events to the outbox: {e}; they stay on disk"),
    }
}

/// Body of [`MemoryRuntime::maintenance_worker`].
///
/// Periodic inactivity-based tier demotion: first pass [`STARTUP_DELAY`] after
/// the future starts, then every [`MAINTENANCE_PERIOD`].
fn maintenance_worker_future(
    store: Arc<MemoryStore>,
) -> impl Future<Output = ()> + Send + 'static {
    run_maintenance_loop(store, STARTUP_DELAY, MAINTENANCE_PERIOD)
}

// Compile-time proof that every handle the two futures capture is
// `Send + 'static`, which is what `tokio::spawn` demands. Stated here, in the
// crate that owns the types — a regression would otherwise surface as a
// confusing error inside a different crate.
const _: fn() = || {
    fn _send_static<T: Send + 'static>() {}
    _send_static::<Receiver<IngestRequest>>();
    _send_static::<watch::Receiver<Option<Instant>>>();
    _send_static::<Arc<MemoryStore>>();
    _send_static::<Arc<OnceCell<Arc<dyn Embedder>>>>();
    _send_static::<Arc<OnceCell<Arc<tokenizers::Tokenizer>>>>();
    _send_static::<Arc<AccessTracker>>();
    _send_static::<PathBuf>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::events::{MemoryEvent, MemorySource};
    use crate::memory::worker::{ChunkOutcome, IngestReport};
    use tokio::sync::mpsc::error::TrySendError;
    use crate::models::chat::Role;

    fn tmp_base() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn chat_event(content: &str) -> MemoryEvent {
        MemoryEvent {
            agent_id: "agent-rt".into(),
            project_id: None,
            role: Role::User,
            content: content.into(),
            is_error: None,
            source: MemorySource::Chat,
            timestamp_ms: 1,
            memory_type: None,
            importance: None,
            derived_from: Vec::new(),
        }
    }

    /// `open()` must produce a real, migrated database on disk — not merely a
    /// constructed handle. `vec_version()` is the sharpest cheap check: it
    /// only answers when the sqlite-vec extension actually loaded.
    #[tokio::test]
    async fn open_creates_memory_db_and_a_usable_store() {
        let tmp = tmp_base();
        let (rt, _rx) = MemoryRuntime::open(tmp.path().to_path_buf(), StoreHolder::Daemon)
            .await
            .expect("open");

        assert!(
            tmp.path().join("memory.db").exists(),
            "open() must create <base_dir>/memory.db"
        );
        let version: String = rt
            .store
            .conn
            .call(|c| -> Result<String, rusqlite::Error> {
                c.query_row("SELECT vec_version()", [], |r| r.get(0))
            })
            .await
            .expect("vec_version");
        assert!(!version.is_empty());
    }

    /// Production `open()` must NOT fill the embedder cell: the 470 MB
    /// download happens in the background, and a constructor that waited for
    /// it would stall every host's startup.
    #[tokio::test]
    async fn open_leaves_the_embedder_and_tokenizer_cells_empty() {
        let tmp = tmp_base();
        let (rt, _rx) = MemoryRuntime::open(tmp.path().to_path_buf(), StoreHolder::Daemon)
            .await
            .expect("open");
        assert!(!rt.embedder_ready());
        assert_eq!(rt.embedder_status(), EmbedderStatus::Loading);
        assert!(rt.tokenizer.get().is_none());
    }

    /// The returned sender and receiver must be two ends of ONE channel, and
    /// sending must succeed before any worker is connected — the hot path
    /// cannot be made to wait for the worker to come up.
    #[tokio::test]
    async fn open_returns_the_two_ends_of_one_live_channel() {
        let tmp = tmp_base();
        let (rt, mut rx) = MemoryRuntime::open(tmp.path().to_path_buf(), StoreHolder::Daemon)
            .await
            .expect("open");
        rt.memory_tx.try_send(chat_event("hello").into()).expect("send");
        let req = rx.recv().await.expect("event");
        assert_eq!(req.event.content, "hello");
    }

    /// The flush worker is spawned inside `AccessTracker::new` during `open`,
    /// so `record` must already be safe to call — it sits on the retrieval hot
    /// path, where blocking or panicking would stall a query.
    #[tokio::test]
    async fn open_starts_the_access_flush_worker() {
        let tmp = tmp_base();
        let (rt, _rx) = MemoryRuntime::open(tmp.path().to_path_buf(), StoreHolder::Daemon)
            .await
            .expect("open");
        rt.access_tracker.record(vec![1, 2, 3]);
        rt.access_tracker.record(vec![]); // empty is a no-op, not an error
    }

    #[tokio::test]
    async fn workspaces_is_rooted_under_base_dir() {
        let tmp = tmp_base();
        let (rt, _rx) = MemoryRuntime::open(tmp.path().to_path_buf(), StoreHolder::Daemon)
            .await
            .expect("open");
        assert_eq!(
            rt.workspaces().workspace_path("agent-1", None),
            tmp.path()
                .join("workspaces")
                .join("agent-1")
                .join("default")
                .join("workspace.md")
        );
    }

    #[tokio::test]
    async fn new_for_test_preseeds_the_embedder_and_touches_no_disk() {
        let tmp = tmp_base();
        let (rt, _rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
        assert!(rt.embedder_ready(), "the cell must be pre-seeded");
        assert_eq!(rt.embedder_status(), EmbedderStatus::Ready);
        assert_eq!(
            rt.embedder.get().expect("seeded").model_id(),
            "fake-embedder"
        );
        assert!(
            !tmp.path().join("memory.db").exists(),
            "new_for_test must stay in memory"
        );
    }

    /// The end-to-end assertion this whole module exists for: on a plain tokio
    /// runtime, with nothing else running, an event sent on the channel becomes a
    /// row.
    #[tokio::test]
    async fn ingest_worker_future_drains_an_event_into_a_row() {
        let tmp = tmp_base();
        let (rt, rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
        let store = rt.store.clone();
        let tx = rt.memory_tx.clone();

        let worker = tokio::spawn(rt.ingest_worker(rx, watch::channel(None).1));
        tx.try_send(chat_event("a row lands on a plain tokio runtime").into())
            .expect("send");
        // Dropping every sender is what ends the worker loop — the runtime
        // holds one of its own, so it has to go too.
        drop(tx);
        drop(rt);
        worker.await.expect("worker exits cleanly when the channel closes");

        let count: i64 = store
            .conn
            .call(|c| -> Result<i64, rusqlite::Error> {
                c.query_row(
                    "SELECT COUNT(*) FROM memory WHERE agent_id = 'agent-rt'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .expect("count");
        assert_eq!(count, 1);
    }

    /// The maintenance loop never returns, so what is actually assertable is
    /// that its future satisfies the bounds every executor demands.
    #[tokio::test]
    async fn maintenance_worker_future_is_send_static_and_spawnable() {
        let tmp = tmp_base();
        let (rt, _rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
        fn assert_spawnable<F: Future<Output = ()> + Send + 'static>(_: &F) {}
        let fut = rt.maintenance_worker();
        assert_spawnable(&fut);
        tokio::spawn(fut).abort();
    }

    /// The free functions take the handles individually, so they must work
    /// standing alone, not only through the methods.
    #[tokio::test]
    async fn free_functions_accept_handles_taken_apart_from_the_runtime() {
        let tmp = tmp_base();
        let (rt, rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
        let MemoryRuntime {
            base_dir,
            store,
            memory_tx,
            embedder,
            embedder_status: _,
            tokenizer,
            access_tracker: _,
            project_docs: _,
        } = rt;

        let worker = tokio::spawn(ingest_worker_future(
            base_dir,
            store.clone(),
            embedder,
            tokenizer,
            rx,
            watch::channel(None).1,
        ));
        memory_tx.try_send(chat_event("through the free function").into()).expect("send");
        drop(memory_tx);
        worker.await.expect("worker exits");

        let count: i64 = store
            .conn
            .call(|c| -> Result<i64, rusqlite::Error> {
                c.query_row(
                    "SELECT COUNT(*) FROM memory WHERE agent_id = 'agent-rt'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .expect("count");
        assert_eq!(count, 1);

        fn assert_spawnable<F: Future<Output = ()> + Send + 'static>(_: &F) {}
        assert_spawnable(&maintenance_worker_future(store));
    }

    #[tokio::test]
    async fn a_request_with_a_reply_channel_receives_its_ingest_report() {
        let tmp = tmp_base();
        let (rt, rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
        let worker = tokio::spawn(rt.ingest_worker(rx, watch::channel(None).1));

        let (reply, report) = tokio::sync::oneshot::channel::<IngestReport>();
        rt.memory_tx
            .try_send(IngestRequest { event: chat_event("the report comes back"), reply: Some(reply), outbox: None })
            .expect("send");
        let report = report.await.expect("the worker must answer a request that asked");

        assert_eq!(report.failure, None);
        assert_eq!(report.memories.len(), 1);
        assert_eq!(report.memories[0].outcome, ChunkOutcome::Inserted);
        drop(rt);
        worker.await.expect("worker exits");
    }

    #[tokio::test]
    async fn the_ingest_queue_refuses_a_send_past_its_capacity() {
        let tmp = tmp_base();
        let (rt, _rx) = MemoryRuntime::new_for_test_with_capacity(tmp.path().to_path_buf(), 2).await;

        rt.memory_tx.try_send(chat_event("one").into()).expect("first fits");
        rt.memory_tx.try_send(chat_event("two").into()).expect("second fits");
        let third = rt.memory_tx.try_send(chat_event("three").into());

        assert!(matches!(third, Err(TrySendError::Full(_))), "got {third:?}");
    }

    async fn outbox_rows(store: &MemoryStore) -> Vec<(i64, String)> {
        store
            .conn
            .call(|c| -> Result<Vec<(i64, String)>, rusqlite::Error> {
                let mut stmt = c.prepare("SELECT id, content FROM ingest_outbox ORDER BY id")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect();
                rows
            })
            .await
            .expect("outbox")
    }

    #[tokio::test]
    async fn accept_writes_the_row_before_it_queues_the_event() {
        let tmp = tmp_base();
        let (rt, mut rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
        // Built at run time so no literal looks like a live credential.
        let token = format!("ghp_{}", "a1B2".repeat(9));

        let accepted = rt.accept(chat_event(&format!("the token is {token}")), None, None).await.unwrap();

        let Accepted::Queued { row, operation_id, redacted } = accepted else { panic!("{accepted:?}") };
        assert_eq!(uuid::Uuid::parse_str(&operation_id).map(|u| u.get_version_num()), Ok(4));
        assert_eq!(redacted, vec![SecretKind::GithubToken]);
        assert_eq!(outbox_rows(&rt.store).await, vec![(row, "the token is [REDACTED:github_token]".to_string())]);
        let queued = rx.recv().await.unwrap();
        assert_eq!(queued.outbox, Some(row));
        assert_eq!(queued.event.content, "the token is [REDACTED:github_token]");
    }

    #[tokio::test]
    async fn an_operation_id_already_recorded_is_not_queued_again() {
        let tmp = tmp_base();
        let (rt, mut rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
        let first = rt.accept(chat_event("once"), Some("op-7".into()), None).await.unwrap();
        let Accepted::Queued { row, .. } = first else { panic!("{first:?}") };

        let again = rt.accept(chat_event("once"), Some("op-7".into()), None).await.unwrap();

        assert_eq!(
            again,
            Accepted::Existing { row, operation_id: "op-7".into(), state: OutboxState::Pending, last_error: None }
        );
        rx.recv().await.unwrap();
        assert!(rx.try_recv().is_err(), "the second save must not be queued");
    }

    #[tokio::test]
    async fn a_refused_event_leaves_no_row() {
        let tmp = tmp_base();
        let (rt, _rx) = MemoryRuntime::new_for_test_with_capacity(tmp.path().to_path_buf(), 1).await;
        rt.accept(chat_event("fits"), None, None).await.unwrap();

        let refused = rt.accept(chat_event("does not fit"), None, None).await;

        assert!(matches!(refused, Err(AcceptError::QueueFull)), "{refused:?}");
        let contents: Vec<String> = outbox_rows(&rt.store).await.into_iter().map(|(_, c)| c).collect();
        assert_eq!(contents, vec!["fits"]);
    }

    #[test]
    fn the_production_ingest_queue_holds_1024_saves() {
        assert_eq!(INGEST_QUEUE_CAPACITY, 1024);
    }
}
