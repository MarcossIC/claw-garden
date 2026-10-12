//! An orderly shutdown must leave every accepted event either in the store or
//! in the spill file; none may be dropped.

#![cfg(feature = "testing")]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use clawg_core::memory::embedder::{EmbedError, EmbedRole, Embedder, FakeEmbedder};
use clawg_core::memory::events::{MemoryEvent, MemorySource};
use clawg_core::memory::spill::SPILL_FILE;
use clawg_core::memory::MemoryStore;
use clawg_core::models::chat::Role;
use clawg_core::runtime::{MemoryRuntime, Workers};
use once_cell::sync::OnceCell;
use tokio::sync::mpsc::UnboundedReceiver;

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
    }
}

fn start(runtime: &MemoryRuntime, rx: UnboundedReceiver<MemoryEvent>) -> Workers {
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

fn spilled_contents(base_dir: &std::path::Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(base_dir.join(SPILL_FILE)).ok()?;
    Some(
        text.lines()
            .map(|l| serde_json::from_str::<MemoryEvent>(l).expect("spill line").content)
            .collect(),
    )
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
        runtime.memory_tx.send(event(content)).unwrap();
    }

    runtime.shutdown(workers).await;

    assert_eq!(stored_contents(&runtime.store).await, sent);
    assert_eq!(spilled_contents(tmp.path()), None);
}

#[tokio::test]
async fn shutdown_spills_events_held_for_an_embedder_that_never_loaded() {
    let tmp = tempfile::tempdir().unwrap();
    let (runtime, rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
    let runtime = MemoryRuntime { embedder: Arc::new(OnceCell::<Arc<dyn Embedder>>::new()), ..runtime };
    let workers = start(&runtime, rx);
    for content in ["first", "second", "third"] {
        runtime.memory_tx.send(event(content)).unwrap();
    }
    // Long enough for the worker to take "first" and hold it.
    tokio::time::sleep(Duration::from_millis(150)).await;

    runtime.shutdown(workers).await;

    assert_eq!(stored_contents(&runtime.store).await, Vec::<String>::new());
    assert_eq!(
        spilled_contents(tmp.path()),
        Some(vec!["first".to_string(), "second".to_string(), "third".to_string()])
    );
}

#[tokio::test]
async fn shutdown_spills_what_the_drain_deadline_left_unstarted() {
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
        runtime.memory_tx.send(event(content)).unwrap();
    }
    // Let the first event start embedding before the drain begins.
    tokio::time::sleep(Duration::from_millis(50)).await;

    runtime.shutdown_within(workers, Duration::from_millis(100)).await;

    assert_eq!(stored_contents(&runtime.store).await, sent[..1].to_vec());
    assert_eq!(spilled_contents(tmp.path()), Some(sent[1..].to_vec()));
}
