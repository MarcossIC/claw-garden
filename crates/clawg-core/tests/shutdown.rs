//! Every accepted save ends in the store or waits in the outbox for the next
//! start, whether the process stops in order or dies.

#![cfg(feature = "testing")]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use clawg_core::memory::embedder::{EmbedError, EmbedRole, Embedder, FakeEmbedder};
use clawg_core::memory::events::{MemoryEvent, MemorySource};
use clawg_core::memory::spill::{REJECTED_FILE, SPILL_FILE};
use clawg_core::memory::store_lock::StoreHolder;
use clawg_core::memory::MemoryStore;
use clawg_core::models::chat::Role;
use clawg_core::runtime::{MemoryRuntime, Workers};
use once_cell::sync::OnceCell;
use clawg_core::memory::worker::IngestRequest;
use clawg_core::runtime::Accepted;
use tokio::sync::mpsc::Receiver;

fn event(content: &str) -> MemoryEvent {
    MemoryEvent {
        agent_id: "agent-shutdown".into(),
        project_id: None,
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
    }
}

fn start(runtime: &MemoryRuntime, rx: Receiver<IngestRequest>) -> Workers {
    let (drain, drain_rx) = tokio::sync::watch::channel(None);
    Workers {
        ingest: tokio::spawn(runtime.ingest_worker(rx, drain_rx)),
        maintenance: tokio::spawn(runtime.maintenance_worker()),
        drain,
    }
}

async fn stored_contents(store: &MemoryStore) -> Vec<String> {
    store
        .conn
        .call(|c| -> Result<Vec<String>, rusqlite::Error> {
            let mut stmt = c.prepare(
                "SELECT content FROM memory WHERE agent_id = 'agent-shutdown' ORDER BY id",
            )?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect()
        })
        .await
        .expect("query")
}

async fn outbox_contents(store: &MemoryStore) -> Vec<String> {
    store
        .conn
        .call(|c| -> Result<Vec<String>, rusqlite::Error> {
            let mut stmt = c.prepare("SELECT content FROM ingest_outbox WHERE state = 'pending' ORDER BY id")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect()
        })
        .await
        .expect("query")
}

async fn accept(runtime: &MemoryRuntime, content: &str) {
    let accepted = runtime.accept(event(content), None, None).await.expect("accept");
    assert!(matches!(accepted, Accepted::Queued { .. }), "{accepted:?}");
}

/// Wraps the fake embedder with a fixed delay per call, standing in for a
/// real model that is slower than the drain deadline.
struct SlowEmbedder {
    inner: FakeEmbedder,
    delay: Duration,
}

#[async_trait]
impl Embedder for SlowEmbedder {
    async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
        tokio::time::sleep(self.delay).await;
        self.inner.embed_with_role(text, role).await
    }
    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
    fn query_prefix(&self) -> &str {
        self.inner.query_prefix()
    }
    fn passage_prefix(&self) -> &str {
        self.inner.passage_prefix()
    }
}

#[tokio::test]
async fn shutdown_ingests_every_queued_event_when_the_embedder_is_ready() {
    let tmp = tempfile::tempdir().unwrap();
    let (runtime, rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
    let workers = start(&runtime, rx);
    let sent: Vec<String> = (0..20).map(|i| format!("queued fact number {i}")).collect();
    for content in &sent {
        accept(&runtime, content).await;
    }

    runtime.shutdown(workers).await;

    assert_eq!(stored_contents(&runtime.store).await, sent);
    assert!(outbox_contents(&runtime.store).await.is_empty());
    assert!(!tmp.path().join(SPILL_FILE).exists());
}

#[tokio::test]
async fn shutdown_leaves_saves_held_for_the_embedder_in_the_outbox() {
    let tmp = tempfile::tempdir().unwrap();
    let (runtime, rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
    let runtime = MemoryRuntime { embedder: Arc::new(OnceCell::<Arc<dyn Embedder>>::new()), ..runtime };
    let workers = start(&runtime, rx);
    for content in ["first", "second", "third"] {
        accept(&runtime, content).await;
    }
    // Long enough for the worker to take "first" and hold it.
    tokio::time::sleep(Duration::from_millis(150)).await;

    runtime.shutdown(workers).await;

    assert_eq!(stored_contents(&runtime.store).await, Vec::<String>::new());
    assert_eq!(outbox_contents(&runtime.store).await, vec!["first", "second", "third"]);
    assert!(!tmp.path().join(SPILL_FILE).exists(), "nothing is spilled to a file any more");
}

#[tokio::test]
async fn shutdown_leaves_what_the_drain_deadline_left_unstarted_in_the_outbox() {
    let tmp = tempfile::tempdir().unwrap();
    let (runtime, rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
    let slow: Arc<dyn Embedder> =
        Arc::new(SlowEmbedder { inner: FakeEmbedder::default(), delay: Duration::from_millis(400) });
    let cell = OnceCell::new();
    cell.set(slow).ok();
    let runtime = MemoryRuntime { embedder: Arc::new(cell), ..runtime };
    let workers = start(&runtime, rx);
    let sent: Vec<String> = (0..5).map(|i| format!("slow fact number {i}")).collect();
    for content in &sent {
        accept(&runtime, content).await;
    }
    // Let the first event start embedding before the drain begins.
    tokio::time::sleep(Duration::from_millis(50)).await;

    runtime.shutdown_within(workers, Duration::from_millis(100)).await;

    assert_eq!(stored_contents(&runtime.store).await, sent[..1].to_vec());
    assert_eq!(outbox_contents(&runtime.store).await, sent[1..].to_vec());
}

fn write_spill(base_dir: &std::path::Path, lines: &[String]) {
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(base_dir.join(SPILL_FILE), text).unwrap();
}

fn spill_line(content: &str) -> String {
    serde_json::to_string(&event(content)).unwrap()
}

async fn wait_for_rows(store: &MemoryStore, n: usize) -> Vec<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let rows = stored_contents(store).await;
        if rows.len() >= n || tokio::time::Instant::now() >= deadline {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn pending_rows_are_ingested_first_on_startup() {
    let tmp = tempfile::tempdir().unwrap();
    let (runtime, rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
    let runtime = MemoryRuntime { embedder: Arc::new(OnceCell::<Arc<dyn Embedder>>::new()), ..runtime };
    let workers = start(&runtime, rx);
    accept(&runtime, "accepted one").await;
    accept(&runtime, "accepted two").await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    runtime.shutdown(workers).await;
    let store = runtime.store.clone();

    // The same store, a new channel and a ready embedder: what a restart sees.
    let (memory_tx, rx) = tokio::sync::mpsc::channel(16);
    let cell = OnceCell::new();
    cell.set(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>).ok();
    let restarted = MemoryRuntime { memory_tx, embedder: Arc::new(cell), ..runtime };
    accept(&restarted, "saved after the restart").await;
    let workers = start(&restarted, rx);
    let rows = wait_for_rows(&store, 3).await;

    assert_eq!(rows, vec!["accepted one", "accepted two", "saved after the restart"]);
    assert!(outbox_contents(&store).await.is_empty());
    restarted.shutdown(workers).await;
}

#[tokio::test]
async fn an_old_spill_is_imported_masked_and_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let truncated = "{\"agent_id\":\"agent-shutdown\",\"proj".to_string();
    // Built at run time so no literal looks like a live credential.
    let token = format!("ghp_{}", "a1B2".repeat(9));
    write_spill(
        tmp.path(),
        &[spill_line("before"), truncated.clone(), spill_line(&format!("the token is {token}"))],
    );
    let (runtime, rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;

    let workers = start(&runtime, rx);
    let rows = wait_for_rows(&runtime.store, 2).await;
    runtime.shutdown(workers).await;

    assert_eq!(rows, vec!["before", "the token is [REDACTED:github_token]"]);
    let rejected = std::fs::read_to_string(tmp.path().join(REJECTED_FILE)).unwrap();
    assert_eq!(rejected, format!("{truncated}\n"));
    assert!(!tmp.path().join(SPILL_FILE).exists());
    assert!(outbox_contents(&runtime.store).await.is_empty());
}

#[tokio::test]
async fn a_save_accepted_while_the_model_loads_survives_a_process_that_dies() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let (runtime, rx) = MemoryRuntime::open(tmp.path().to_path_buf(), StoreHolder::Daemon)
            .await
            .unwrap();
        let worker = tokio::spawn(runtime.ingest_worker(rx, tokio::sync::watch::channel(None).1));
        accept(&runtime, "accepted before the process died").await;
        // No shutdown: the worker is gone and the runtime dropped, as a kill
        // would leave them.
        worker.abort();
        let _ = worker.await;
    }

    let (runtime, rx) = MemoryRuntime::open(tmp.path().to_path_buf(), StoreHolder::Daemon)
        .await
        .unwrap();
    runtime.embedder.set(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>).ok();
    let workers = start(&runtime, rx);
    let rows = wait_for_rows(&runtime.store, 1).await;
    runtime.shutdown(workers).await;

    assert_eq!(rows, vec!["accepted before the process died"]);
    assert!(outbox_contents(&runtime.store).await.is_empty());
}

#[tokio::test]
async fn shutdown_checkpoints_the_wal_into_memory_db() {
    let tmp = tempfile::tempdir().unwrap();
    let (runtime, rx) = MemoryRuntime::open(tmp.path().to_path_buf(), StoreHolder::Daemon)
        .await
        .unwrap();
    runtime.embedder.set(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>).ok();
    let workers = start(&runtime, rx);
    accept(&runtime, "written just before the stop").await;
    assert_eq!(wait_for_rows(&runtime.store, 1).await.len(), 1);

    runtime.shutdown(workers).await;

    let wal = tmp.path().join("memory.db-wal");
    let wal_len = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
    assert_eq!(wal_len, 0, "the WAL must be folded into memory.db");

    let copy_dir = tempfile::tempdir().unwrap();
    let copy = copy_dir.path().join("memory.db");
    std::fs::copy(tmp.path().join("memory.db"), &copy).unwrap();
    let conn = rusqlite::Connection::open(&copy).unwrap();
    let content: String = conn
        .query_row("SELECT content FROM memory WHERE agent_id = 'agent-shutdown'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(content, "written just before the stop");
}
