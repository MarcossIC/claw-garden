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

use once_cell::sync::OnceCell;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::memory::access::AccessTracker;
use crate::memory::chunker::CodeChunker;
use crate::memory::embedder::Embedder;
use crate::memory::events::MemoryEvent;
use crate::memory::store_lock::StoreHolder;
use crate::memory::maintenance::{run_maintenance_loop, MAINTENANCE_PERIOD, STARTUP_DELAY};
use crate::memory::worker::{memory_worker, resolve_tokenizer_and_chunker};
use crate::memory::workspace::WorkspaceManager;
use crate::memory::{MemoryError, MemoryStore};

/// The memory substrate's live handles for one process.
///
/// Every field is deliberately public and deliberately cheap to clone: the
/// hosts can destructure them, and the daemon holds the struct itself.
/// `MemoryStore` is a channel handle,
/// `UnboundedSender` is `Arc`-like, and the two cells plus the tracker are
/// already behind `Arc` — so cloning a field is a refcount bump, never a
/// second database or a second flush worker.
pub struct MemoryRuntime {
    /// Profile data directory (`clawg_core::paths::data_dir`). The store, the
    /// tokenizer files and the workspaces tree all hang off it.
    pub base_dir: PathBuf,
    /// SQLite-vec persistent store, already migrated.
    pub store: Arc<MemoryStore>,
    /// Producer side of the ingest pipeline. Unbounded on purpose — whatever
    /// hot path produces events (a chat turn, an MCP `save_memory` call) MUST
    /// NOT block behind embedding.
    pub memory_tx: UnboundedSender<MemoryEvent>,
    /// Lazily-filled embedder cell.
    ///
    /// [`MemoryRuntime::open`] leaves it EMPTY; the host fills it by running
    /// `memory::bootstrap::run_embedder_bootstrap` in the background. A
    /// `OnceCell` is write-once by construction, so the embedder cannot be
    /// swapped at runtime — changing the model means a restart, which is the
    /// safe outcome anyway: every vector already stored belongs to the old
    /// model's space and mixing the two silently corrupts similarity.
    pub embedder: Arc<OnceCell<Arc<dyn Embedder>>>,
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
    ) -> Result<(Self, UnboundedReceiver<MemoryEvent>), MemoryError> {
        let store = Arc::new(MemoryStore::open(&base_dir, holder).await?);
        let (memory_tx, memory_rx) = tokio::sync::mpsc::unbounded_channel::<MemoryEvent>();
        let access_tracker = Arc::new(AccessTracker::new(store.conn.clone()));

        Ok((
            Self {
                base_dir,
                store,
                memory_tx,
                embedder: Arc::new(OnceCell::new()),
                tokenizer: Arc::new(OnceCell::new()),
                access_tracker,
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
    pub fn ingest_worker(
        &self,
        rx: UnboundedReceiver<MemoryEvent>,
    ) -> impl Future<Output = ()> + Send + 'static {
        ingest_worker_future(
            self.base_dir.clone(),
            self.store.clone(),
            self.embedder.clone(),
            self.tokenizer.clone(),
            rx,
        )
    }

    /// The periodic maintenance loop's future — **not** spawned, same
    /// rationale as [`MemoryRuntime::ingest_worker`]. Never completes: the
    /// host drops or aborts it at shutdown.
    pub fn maintenance_worker(&self) -> impl Future<Output = ()> + Send + 'static {
        maintenance_worker_future(self.store.clone())
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
    pub async fn new_for_test(base_dir: PathBuf) -> (Self, UnboundedReceiver<MemoryEvent>) {
        use crate::memory::embedder::FakeEmbedder;

        let store = Arc::new(
            MemoryStore::new_for_test()
                .await
                .expect("in-memory test store"),
        );
        let (memory_tx, memory_rx) = tokio::sync::mpsc::unbounded_channel::<MemoryEvent>();
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
                tokenizer: Arc::new(OnceCell::new()),
                access_tracker,
            },
            memory_rx,
        )
    }
}

/// Body of [`MemoryRuntime::ingest_worker`].
///
/// Builds the two chunkers once — resolving the tokenizer as a side effect,
/// which is what fills `tokenizer_cell` for the query path — then hands
/// ownership to the worker loop.
fn ingest_worker_future(
    base_dir: PathBuf,
    store: Arc<MemoryStore>,
    embedder: Arc<OnceCell<Arc<dyn Embedder>>>,
    tokenizer: Arc<OnceCell<Arc<tokenizers::Tokenizer>>>,
    rx: UnboundedReceiver<MemoryEvent>,
) -> impl Future<Output = ()> + Send + 'static {
    async move {
        let chat_chunker = resolve_tokenizer_and_chunker(&base_dir, &tokenizer);
        let code_chunker = Arc::new(CodeChunker::new());
        memory_worker(rx, store, embedder, chat_chunker, code_chunker).await;
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
    _send_static::<UnboundedReceiver<MemoryEvent>>();
    _send_static::<Arc<MemoryStore>>();
    _send_static::<Arc<OnceCell<Arc<dyn Embedder>>>>();
    _send_static::<Arc<OnceCell<Arc<tokenizers::Tokenizer>>>>();
    _send_static::<Arc<AccessTracker>>();
    _send_static::<PathBuf>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::events::MemorySource;
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
        rt.memory_tx.send(chat_event("hello")).expect("send");
        let evt = rx.recv().await.expect("event");
        assert_eq!(evt.content, "hello");
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

        let worker = tokio::spawn(rt.ingest_worker(rx));
        tx.send(chat_event("a row lands on a plain tokio runtime"))
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
            tokenizer,
            access_tracker: _,
        } = rt;

        let worker = tokio::spawn(ingest_worker_future(
            base_dir,
            store.clone(),
            embedder,
            tokenizer,
            rx,
        ));
        memory_tx.send(chat_event("through the free function")).expect("send");
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
}
