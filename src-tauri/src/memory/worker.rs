//! Memory ingest worker.
//!
//! Spawned once at app startup from `lib.rs::run()` setup hook. Drains
//! `MemoryEvent`s from the channel, selects the appropriate chunker, embeds
//! each chunk, and inserts both `memory` and `memory_vectors` rows atomically.
//! Failure isolation: errors are logged and the worker NEVER panics.

use std::sync::Arc;

use once_cell::sync::OnceCell;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::memory::chunker::{select_chunker, ChatChunker, ChunkerKind, CodeChunker};
use crate::memory::embedder::{EmbedError, Embedder};
use crate::memory::events::{MemoryEvent, MemorySource};
use crate::memory::store_insert::{MemoryStoreError, NewMemoryRecord};
use crate::memory::MemoryStore;

/// Worker-internal error type. Only used for logging via `warn!`.
/// All variants are recoverable — the worker loop never exits on error.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MemoryWorkerError {
    #[error("embed failed: {0}")]
    Embed(#[from] EmbedError),
    #[error("store insert failed: {0}")]
    Store(#[from] MemoryStoreError),
    /// Reserved for a future code-chunker enhancement that surfaces an
    /// explicit "0 chunks produced" signal; currently the worker silently
    /// continues past an empty chunk list.
    #[allow(dead_code)]
    #[error("chunker produced 0 chunks for non-empty content")]
    EmptyChunks,
}

/// Top-level worker loop. One per app instance.
///
/// Drains events FIFO. Each chunk's embed+insert is isolated: a failure on
/// one chunk (an embed error other than `NotReady`/`WorkerGone`, or an
/// insert error) is logged and the worker moves on to the next chunk without
/// aborting the event; the embedder going away mid-stream (`NotReady` /
/// `WorkerGone`) drops the rest of the current event's chunks but leaves the
/// worker itself running for the next event.
/// Exits cleanly when all `Sender`s are dropped (app shutdown).
pub(crate) async fn memory_worker(
    mut rx: UnboundedReceiver<MemoryEvent>,
    store: Arc<MemoryStore>,
    embedder_cell: Arc<OnceCell<Arc<dyn Embedder>>>,
    chat_chunker: Arc<ChatChunker>,
    code_chunker: Arc<CodeChunker>,
) {
    while let Some(evt) = rx.recv().await {
        if let Err(e) = process_event(
            &evt,
            &store,
            &embedder_cell,
            &chat_chunker,
            &code_chunker,
        )
        .await
        {
            log::warn!(
                "ingest event dropped (agent={}, role={:?}): {e}",
                evt.agent_id,
                evt.role
            );
        }
    }
    log::info!("memory_worker shutting down — channel closed");
}

async fn process_event(
    evt: &MemoryEvent,
    store: &Arc<MemoryStore>,
    embedder_cell: &Arc<OnceCell<Arc<dyn Embedder>>>,
    chat_chunker: &Arc<ChatChunker>,
    code_chunker: &Arc<CodeChunker>,
) -> Result<(), MemoryWorkerError> {
    // Source-first: Compaction-sourced events bypass select_chunker (and thus
    // the Role::System→Skip rule that ordinarily drops system-authored
    // content from ingestion) entirely — they always chunk via ChatChunker,
    // because a compaction summary is deliberately synthesized content worth
    // keeping regardless of which role produced the text it summarizes. All
    // other sources keep the existing role-based routing.
    let chunks = if evt.source == MemorySource::Compaction {
        chat_chunker.chunk(&evt.content, 480)
    } else {
        let kind = select_chunker(evt.role.clone(), evt.is_error, &evt.content);
        match kind {
            ChunkerKind::Chat => chat_chunker.chunk(&evt.content, 480),
            ChunkerKind::Code => code_chunker.chunk(&evt.content, None),
            ChunkerKind::Skip => return Ok(()),
        }
    };

    if chunks.is_empty() {
        // Content was all-whitespace or empty after chunker policy. Not an error.
        return Ok(());
    }

    // No embedder loaded yet (contract bootstrap still running, or
    // permanently mismatched): drop the entire event rather than embedding
    // only some of its chunks — a half-embedded event would be unrecoverable
    // without tracking which chunks already made it in, so the whole event
    // is dropped uniformly and cheaply instead.
    let Some(embedder) = embedder_cell.get().cloned() else {
        log::debug!(
            "embedder NotReady; dropping {} chunks for agent={}",
            chunks.len(),
            evt.agent_id
        );
        return Ok(());
    };

    for chunk in chunks {
        match embedder.embed(&chunk.text).await {
            Ok(vec) => {
                // The contradiction hook runs between embed and insert: it needs
                // the freshly computed embedding to find near-duplicate candidates
                // via KNN, and its result (which existing rows this chunk disputes)
                // must be attached to the record before it is written, not after.
                // detect_contradiction is async and requires &MemoryStore; Arc<MemoryStore>
                // derefs to MemoryStore via Deref coercion.
                let mut record = NewMemoryRecord::from_event_and_chunk(evt, &chunk);
                let disputed =
                    crate::memory::contradiction::detect_contradiction(store, &vec, &evt.agent_id)
                        .await;
                if !disputed.is_empty() {
                    record.disputed_against = Some(disputed);
                }
                if let Err(e) = store.insert_memory_with_vector(record, vec).await {
                    log::warn!(
                        "insert_memory_with_vector failed (agent={}): {e}",
                        evt.agent_id
                    );
                    // continue — worker stays alive
                }
            }
            Err(EmbedError::NotReady) | Err(EmbedError::WorkerGone) => {
                // Cell went empty mid-stream or worker thread died — drop rest of event.
                log::warn!(
                    "embedder became unavailable mid-event (agent={}); dropping rest",
                    evt.agent_id
                );
                return Ok(());
            }
            Err(e) => {
                // InferenceFailed / LoadFailed — single-chunk problem, skip & continue.
                log::warn!(
                    "embed failed for chunk (agent={}): {e}; skipping chunk",
                    evt.agent_id
                );
            }
        }
    }
    Ok(())
}

/// Spawn entry point — called from `lib.rs::run()` setup hook.
///
/// Builds the two chunkers once, then hands ownership to the worker future.
/// Returns immediately; the spawned task lives for the app lifetime.
pub(crate) fn start_memory_worker(
    app_handle: tauri::AppHandle,
    memory_rx: UnboundedReceiver<MemoryEvent>,
) {
    use tauri::Manager;
    let state = app_handle.state::<crate::state::AppState>();
    let store = state.memory_store.clone();
    let embedder_cell = state.embedder.clone();
    let base_dir = state.base_dir.clone();

    tauri::async_runtime::spawn(async move {
        let chat_chunker = Arc::new(ChatChunker::new_from_base_dir(&base_dir));
        let code_chunker = Arc::new(CodeChunker::new());
        memory_worker(memory_rx, store, embedder_cell, chat_chunker, code_chunker).await;
    });
}

// Compile-time Send proof — every captured value MUST be Send + 'static so
// tauri::async_runtime::spawn accepts the future.
const _: fn() = || {
    fn _send<T: Send>() {}
    _send::<UnboundedReceiver<MemoryEvent>>();
    _send::<Arc<MemoryStore>>();
    _send::<Arc<OnceCell<Arc<dyn Embedder>>>>();
    _send::<Arc<ChatChunker>>();
    _send::<Arc<CodeChunker>>();
};

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use super::*;
    use crate::memory::MemoryStore;
    use crate::memory::embedder::{EmbedError, Embedder, FakeEmbedder};
    use crate::memory::events::MemorySource;
    use crate::models::chat::Role;
    use tokio::sync::mpsc::UnboundedSender;
    use tokio::task::JoinHandle;

    // ── Test helpers ────────────────────────────────────────────────────────────

    fn make_event(role: Role, content: &str, is_error: Option<bool>, ts: u64) -> MemoryEvent {
        MemoryEvent {
            agent_id: "agent-test".to_string(),
            project_id: None,
            role,
            content: content.to_string(),
            is_error,
            source: MemorySource::Chat,
            timestamp_ms: ts,
        }
    }

    fn make_event_with_project(role: Role, content: &str, project_id: Option<String>, ts: u64) -> MemoryEvent {
        MemoryEvent {
            agent_id: "agent-test".to_string(),
            project_id,
            role,
            content: content.to_string(),
            is_error: None,
            source: MemorySource::Chat,
            timestamp_ms: ts,
        }
    }

    async fn count_memory(store: &MemoryStore) -> i64 {
        store.conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM memory", [], |r| r.get(0)))
            .await
            .unwrap()
    }

    async fn drain_worker(tx: UnboundedSender<MemoryEvent>, handle: JoinHandle<()>) {
        drop(tx);
        tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("worker must exit within timeout after sender dropped")
            .expect("worker must not panic");
    }

    // ── Worker spawn returns a join handle usable for shutdown/panic detection ──

    #[tokio::test]
    async fn spawn_memory_worker_returns_join_handle() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());

        let (tx, handle) = spawn_test_worker(store, embedder).await;

        // Handle should NOT be done immediately
        assert!(!handle.is_finished());

        drop(tx);
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            handle,
        )
        .await;
        assert!(result.is_ok(), "worker must exit within timeout after sender dropped");
        assert!(result.unwrap().is_ok(), "worker must not panic");
    }

    // ── Graceful shutdown and per-event error isolation ──────────────────────

    #[tokio::test]
    async fn worker_exits_gracefully_when_all_senders_dropped() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(store, embedder).await;
        drain_worker(tx, handle).await;
    }

    #[tokio::test]
    async fn worker_continues_after_per_event_error() {
        // First event: Role::Assistant with empty content + tool_calls (Skip path).
        // Second event: valid user message (should be processed + inserted).
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        // Skip event (empty assistant tool-call)
        let skip_evt = MemoryEvent {
            agent_id: "agent-test".to_string(),
            project_id: None,
            role: Role::Assistant,
            content: String::new(),
            is_error: None,
            source: MemorySource::Chat,
            timestamp_ms: 1,
        };
        // Wait: the "tool_calls" filter is in from_message, but the worker just
        // calls select_chunker(Role::Assistant, None, "") → Skip (empty content).
        tx.send(skip_evt).unwrap();

        // Valid user event
        tx.send(make_event(Role::User, "hello world", None, 2)).unwrap();

        // Give the worker time to process
        drop(tx);
        tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("worker must exit within timeout")
            .expect("worker must not panic");

        // Worker must have processed the valid event (1 row)
        let rows = count_memory(&store).await;
        assert_eq!(rows, 1, "expected 1 row for valid event; got {rows}");
    }

    // ── Embedder-not-ready gate: empty cell drops events without inserting ──────

    #[tokio::test]
    async fn embedder_cell_empty_no_rows_inserted() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        // Empty cell — no embedder seeded
        let (tx, handle) = spawn_test_worker_opt(Arc::clone(&store), None).await;
        tx.send(make_event(Role::User, "hello", None, 1)).unwrap();
        drain_worker(tx, handle).await;
        let rows = count_memory(&store).await;
        assert_eq!(rows, 0, "empty embedder cell → 0 rows; got {rows}");
    }

    #[tokio::test]
    async fn embedder_not_ready_no_rows_inserted() {
        // FakeEmbedder with dim=0 → always returns Ok([]) → insert will fail dim
        // check → 0 rows. Alternative: use a dim that triggers DimensionMismatch.
        // Actually the dim check is for wrong dimension (not 384). Let's use a
        // test embedder that returns NotReady.
        struct NotReadyEmbedder;
        #[async_trait::async_trait]
        impl Embedder for NotReadyEmbedder {
            async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedError> {
                Err(EmbedError::NotReady)
            }
            fn dimensions(&self) -> usize { 384 }
            fn model_id(&self) -> &str { "not-ready" }
        }

        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(NotReadyEmbedder);
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;
        tx.send(make_event(Role::User, "hello", None, 1)).unwrap();
        drain_worker(tx, handle).await;
        let rows = count_memory(&store).await;
        assert_eq!(rows, 0, "NotReady embedder → 0 rows; got {rows}");
    }

    #[tokio::test]
    async fn embedder_worker_gone_skips_chunk_continues() {
        // First event: WorkerGone → 0 rows.
        // Second event: FakeEmbedder (we test worker survival by checking rows = 1).
        // Since we can't hot-swap the embedder in the same worker instance, we
        // verify that the WORKER itself stays alive (doesn't exit) after WorkerGone,
        // and that a subsequent event with a good embedder would still be drainable.
        // Practical test: send 1 event with WorkerGone → worker logs + returns Ok().
        // Worker is still alive → send second valid event, but wait — same embedder.
        // So we use a counting embedder that fails on call 1, succeeds on call 2.
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct CountingEmbedder {
            calls: Arc<AtomicUsize>,
        }
        #[async_trait::async_trait]
        impl Embedder for CountingEmbedder {
            async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedError> {
                let n = self.calls.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Err(EmbedError::WorkerGone)
                } else {
                    Ok(vec![0.1_f32; 384])
                }
            }
            fn dimensions(&self) -> usize { 384 }
            fn model_id(&self) -> &str { "counting" }
        }

        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let embedder: Arc<dyn Embedder> = Arc::new(CountingEmbedder { calls });
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        // First event → WorkerGone → 0 rows for this event
        tx.send(make_event(Role::User, "first", None, 1)).unwrap();
        // Second event → Ok → 1 row
        tx.send(make_event(Role::User, "second", None, 2)).unwrap();

        drain_worker(tx, handle).await;

        let rows = count_memory(&store).await;
        // First event: WorkerGone breaks inner loop → 0 rows.
        // Second event: Ok → 1 row.
        assert_eq!(rows, 1, "expected 1 row (second event), got {rows}");
    }

    /// An embedder that returns a vector containing NaN never reaches the
    /// vec0 table: `store_insert.rs` routes every embedding through
    /// `memory::vector::to_unit`, which rejects any non-finite component
    /// before the transaction opens. The chunk is dropped, not the worker —
    /// the same `Err` -> `log::warn!` + continue path already exercised by
    /// `store_write_failure_worker_stays_alive` above handles it with no
    /// NaN-specific branch anywhere in the worker loop.
    #[tokio::test]
    async fn nan_embedding_drops_chunk_worker_stays_alive() {
        struct NanEmbedder;
        #[async_trait::async_trait]
        impl Embedder for NanEmbedder {
            async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedError> {
                Ok(vec![f32::NAN; 384])
            }
            fn dimensions(&self) -> usize {
                384
            }
            fn model_id(&self) -> &str {
                "nan-embedder"
            }
        }

        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(NanEmbedder);
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.send(make_event(Role::User, "hello", None, 1)).unwrap();
        // Second event through the same worker proves the loop kept running
        // past the rejected chunk instead of merely exiting cleanly on drop.
        tx.send(make_event(Role::User, "still alive", None, 2)).unwrap();
        drain_worker(tx, handle).await;

        let rows = count_memory(&store).await;
        assert_eq!(
            rows, 0,
            "NaN embedding must be rejected before any row is written; got {rows} rows"
        );
    }

    #[tokio::test]
    async fn store_write_failure_worker_stays_alive() {
        // Drop memory_vectors between events to cause insert failure.
        // Worker must continue processing and not exit.
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        // First event: process before drop (should succeed)
        tx.send(make_event(Role::User, "event1", None, 1)).unwrap();

        // Give time for event1 to be processed
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        // Drop memory_vectors to cause subsequent inserts to fail
        store.conn
            .call(|c| c.execute("DROP TABLE IF EXISTS memory_vectors", []))
            .await
            .unwrap();

        // Second event: insert fails but worker stays alive
        tx.send(make_event(Role::User, "event2", None, 2)).unwrap();

        drain_worker(tx, handle).await;
        // Worker stayed alive (drain_worker didn't panic or timeout) — test passes.
    }

    // ── memory_type / created_by_type derivation from role and error flag ───

    #[tokio::test]
    async fn user_role_produces_chat_message_type_user_creator() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.send(make_event(Role::User, "hello", None, 1)).unwrap();
        drain_worker(tx, handle).await;

        let (mt, cbt): (String, String) = store.conn
            .call(|c| c.query_row(
                "SELECT memory_type, created_by_type FROM memory LIMIT 1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            ))
            .await
            .unwrap();
        assert_eq!(mt, "chat_message");
        assert_eq!(cbt, "user");
    }

    #[tokio::test]
    async fn assistant_role_produces_agent_creator() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.send(make_event(Role::Assistant, "I can help you", None, 1)).unwrap();
        drain_worker(tx, handle).await;

        let cbt: String = store.conn
            .call(|c| c.query_row(
                "SELECT created_by_type FROM memory LIMIT 1",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert_eq!(cbt, "agent");
    }

    #[tokio::test]
    async fn tool_error_produces_tool_error_type_system_creator_importance_6() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.send(make_event(Role::Tool, "error output", Some(true), 1)).unwrap();
        drain_worker(tx, handle).await;

        let (mt, cbt, imp): (String, String, f64) = store.conn
            .call(|c| c.query_row(
                "SELECT memory_type, created_by_type, importance FROM memory LIMIT 1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?)),
            ))
            .await
            .unwrap();
        assert_eq!(mt, "tool_error");
        assert_eq!(cbt, "system");
        assert!(imp >= 6.0, "expected importance >= 6.0, got {imp}");
    }

    #[tokio::test]
    async fn tool_success_produces_chat_message_type_system_creator() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.send(make_event(Role::Tool, "tool output", Some(false), 1)).unwrap();
        drain_worker(tx, handle).await;

        let (mt, cbt): (String, String) = store.conn
            .call(|c| c.query_row(
                "SELECT memory_type, created_by_type FROM memory LIMIT 1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            ))
            .await
            .unwrap();
        assert_eq!(mt, "chat_message");
        assert_eq!(cbt, "system");
    }

    // ── project_id flow-through from event to stored row ─────────────────────

    #[tokio::test]
    async fn project_id_derived_from_path() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        let evt = make_event_with_project(Role::User, "hello", Some("derived-hash".to_string()), 1);
        tx.send(evt).unwrap();
        drain_worker(tx, handle).await;

        let pid: Option<String> = store.conn
            .call(|c| c.query_row(
                "SELECT project_id FROM memory LIMIT 1",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert_eq!(pid.as_deref(), Some("derived-hash"));
    }

    #[tokio::test]
    async fn project_id_none_when_no_path() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        let evt = make_event_with_project(Role::User, "hello", None, 1);
        tx.send(evt).unwrap();
        drain_worker(tx, handle).await;

        let pid: Option<String> = store.conn
            .call(|c| c.query_row(
                "SELECT project_id FROM memory LIMIT 1",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert!(pid.is_none(), "expected NULL project_id, got {pid:?}");
    }

    // ── 5-turn end-to-end integration: a full chat exchange through the worker ──

    #[tokio::test]
    async fn e2e_5_turn_chat_produces_5_rows() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        let roles = [
            (Role::User, "msg1", None),
            (Role::Assistant, "msg2", None),
            (Role::Tool, "msg3", Some(false)),
            (Role::User, "msg4", None),
            (Role::Assistant, "msg5", None),
        ];
        for (i, (role, content, is_error)) in roles.iter().enumerate() {
            tx.send(MemoryEvent {
                agent_id: "agent-test".to_string(),
                project_id: None,
                role: role.clone(),
                content: content.to_string(),
                is_error: *is_error,
                source: MemorySource::Chat,
                timestamp_ms: (i as u64) + 1,
            }).unwrap();
        }
        drain_worker(tx, handle).await;

        // 5 rows in memory
        let mc = count_memory(&store).await;
        assert_eq!(mc, 5, "expected 5 rows in memory, got {mc}");

        // 5 rows in memory_vectors
        let vc: i64 = store.conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM memory_vectors", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(vc, 5, "expected 5 rows in memory_vectors, got {vc}");

        // All rows have agent_id = "agent-test"
        let agent_count: i64 = store.conn
            .call(|c| c.query_row(
                "SELECT COUNT(*) FROM memory WHERE agent_id = 'agent-test'",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert_eq!(agent_count, 5, "all rows must have agent_id=agent-test");

        // memory_vectors rowids match memory ids
        let mismatch: i64 = store.conn
            .call(|c| c.query_row(
                "SELECT COUNT(*) FROM memory m \
                 LEFT JOIN memory_vectors v ON m.id = v.rowid \
                 WHERE v.rowid IS NULL",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert_eq!(mismatch, 0, "all memory rows must have matching vector rows");

        // FIFO ordering: rows must come back in the same order they were sent.
        // Asserting on (created_at, content) pairs catches a regression where the
        // consumer or the INSERT path scrambles temporal order — the kind of bug
        // that a bare count check would silently let through.
        let ordered: Vec<(i64, String)> = store.conn
            .call(|c| -> Result<Vec<(i64, String)>, rusqlite::Error> {
                let mut stmt = c.prepare(
                    "SELECT created_at, content FROM memory ORDER BY created_at ASC, id ASC",
                )?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                    .collect::<Result<Vec<(i64, String)>, rusqlite::Error>>()?;
                Ok(rows)
            })
            .await
            .unwrap();
        let expected: Vec<(i64, &str)> = vec![
            (1, "msg1"), (2, "msg2"), (3, "msg3"), (4, "msg4"), (5, "msg5"),
        ];
        assert_eq!(
            ordered.iter().map(|(ts, c)| (*ts, c.as_str())).collect::<Vec<_>>(),
            expected,
            "FIFO order broken — rows arrived in {:?}",
            ordered,
        );

        // Repeat with project_id = Some("proj-hash")
        let store2 = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder2: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx2, handle2) = spawn_test_worker(Arc::clone(&store2), embedder2).await;
        for (i, (role, content, is_error)) in roles.iter().enumerate() {
            tx2.send(MemoryEvent {
                agent_id: "agent-test".to_string(),
                project_id: Some("proj-hash".to_string()),
                role: role.clone(),
                content: content.to_string(),
                is_error: *is_error,
                source: MemorySource::Chat,
                timestamp_ms: (i as u64) + 1,
            }).unwrap();
        }
        drain_worker(tx2, handle2).await;
        let proj_count: i64 = store2.conn
            .call(|c| c.query_row(
                "SELECT COUNT(*) FROM memory WHERE project_id IS NOT NULL",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert_eq!(proj_count, 5, "all rows must have project_id set; got {proj_count}");

        // Repeat with project_id = None
        let store3 = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder3: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx3, handle3) = spawn_test_worker(Arc::clone(&store3), embedder3).await;
        for (i, (role, content, is_error)) in roles.iter().enumerate() {
            tx3.send(MemoryEvent {
                agent_id: "agent-test".to_string(),
                project_id: None,
                role: role.clone(),
                content: content.to_string(),
                is_error: *is_error,
                source: MemorySource::Chat,
                timestamp_ms: (i as u64) + 1,
            }).unwrap();
        }
        drain_worker(tx3, handle3).await;
        let null_count: i64 = store3.conn
            .call(|c| c.query_row(
                "SELECT COUNT(*) FROM memory WHERE project_id IS NULL",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert_eq!(null_count, 5, "all rows must have NULL project_id; got {null_count}");
    }

    // ── system_role / empty assistant tool-call filtering ────────────────────

    #[tokio::test]
    async fn system_role_never_reaches_worker() {
        // MemoryEvent::from_message returns None for Role::System — this is
        // tested here to confirm the filter contract, even though the worker
        // also has select_chunker(Role::System, ...) → Skip as a defensive arm.
        use crate::models::chat::ChatMessage;
        let msg = ChatMessage {
            id: "t".to_string(),
            role: Role::System,
            content: "system prompt".to_string(),
            tool_calls: None,
            timestamp_ms: 0,
            is_error: None,
        };
        let result = crate::memory::events::MemoryEvent::from_message(
            "agent".to_string(),
            None,
            &msg,
            MemorySource::Chat,
        );
        assert!(result.is_none(), "system role must return None from from_message");
    }

    #[tokio::test]
    async fn empty_assistant_tool_call_not_ingested() {
        use crate::models::chat::ChatMessage;
        let mut msg = ChatMessage {
            id: "t".to_string(),
            role: Role::Assistant,
            content: String::new(),
            tool_calls: Some(vec![]),
            timestamp_ms: 0,
            is_error: None,
        };
        let result = crate::memory::events::MemoryEvent::from_message(
            "agent".to_string(),
            None,
            &msg,
            MemorySource::Chat,
        );
        assert!(result.is_none(), "empty assistant tool-call must return None from from_message");

        // Also verify non-empty assistant content DOES produce an event
        msg.content = "actual response".to_string();
        msg.tool_calls = None;
        let result2 = crate::memory::events::MemoryEvent::from_message(
            "agent".to_string(),
            None,
            &msg,
            MemorySource::Chat,
        );
        assert!(result2.is_some(), "non-empty assistant content must produce an event");
    }

    // ── Compile-time Send + Sync guards ──────────────────────────────────────

    #[test]
    fn memory_event_and_worker_types_are_send_sync() {
        fn _send<T: Send + 'static>() {}
        fn _sync<T: Sync>() {}
        _send::<MemoryEvent>();
        _send::<Arc<OnceCell<Arc<dyn Embedder>>>>();
        _sync::<Arc<OnceCell<Arc<dyn Embedder>>>>();
    }

    // ── Worker contradiction hook: disputed_against set on match, left None otherwise ──

    /// Helper: insert a memory row with a given embedding directly into the store.
    /// Uses tier=3, status='active' so it is visible to search.
    async fn insert_row_with_embedding(store: &MemoryStore, agent_id: &str, embedding: &[f32]) -> i64 {
        let bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
        let agent = agent_id.to_string();
        store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance, tier, status) \
                     VALUES ('agent', ?1, 'pre content', 'chat_message', 'chat', \
                      0, 'user', ?1, 0, 0, 5.0, 3, 'active')",
                    rusqlite::params![agent],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                    rusqlite::params![id, bytes],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    /// detect_contradiction returns non-empty ids → new row has
    /// disputed_against=Some(ids) and status='disputed'.
    ///
    /// Strategy: send two identical events through the same worker.
    /// The first is inserted with status='active'. When the second arrives,
    /// detect_contradiction finds the first row (distance=0 < 0.20) and
    /// sets disputed_against=[first_id] on the second row.
    #[tokio::test]
    async fn worker_sets_disputed_against_when_contradiction_detected() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        // Event 1: no prior rows → detect_contradiction returns [] → active
        tx.send(make_event(Role::User, "identical content", None, 1)).unwrap();
        // Wait for event 1 to be processed before sending event 2
        // (yield_now a few times to let the async worker drain the channel entry)
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        // Event 2: identical content → same embedding → detect_contradiction finds row 1
        tx.send(make_event(Role::User, "identical content", None, 2)).unwrap();

        drain_worker(tx, handle).await;

        // Fetch all rows ordered by id
        let conn = store.conn.clone();
        let rows: Vec<(i64, String, Option<String>)> = conn
            .call(|c| -> Result<Vec<(i64, String, Option<String>)>, rusqlite::Error> {
                let mut stmt = c.prepare(
                    "SELECT id, status, disputed_against FROM memory ORDER BY id ASC"
                )?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap();

        assert_eq!(rows.len(), 2, "expected exactly 2 rows; got {}", rows.len());
        let (id1, status1, da1) = &rows[0];
        let (_, status2, da2) = &rows[1];

        // Row 1: no prior rows at insertion time → active, no dispute
        assert_eq!(status1, "active", "row 1 must be active");
        assert!(da1.is_none(), "row 1 must have disputed_against=NULL");

        // Row 2: row 1 exists with distance=0 < 0.20 → disputed
        assert_eq!(status2, "disputed", "row 2 must be disputed (detected contradiction)");
        let da_json = da2.as_ref().expect("row 2 must have disputed_against set");
        let parsed: Vec<i64> = serde_json::from_str(da_json)
            .expect("disputed_against must be valid JSON");
        assert!(
            parsed.contains(id1),
            "row 2's disputed_against must contain row 1's id={id1}; got {parsed:?}"
        );
    }

    /// detect_contradiction returns empty → new row has
    /// disputed_against=None and status='active'.
    #[tokio::test]
    async fn worker_leaves_disputed_against_none_when_no_contradiction() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        // Single event, no prior rows → detect_contradiction returns [] → active
        tx.send(make_event(Role::User, "unique content abc123", None, 1)).unwrap();
        drain_worker(tx, handle).await;

        let conn = store.conn.clone();
        let (status, da): (String, Option<String>) = conn
            .call(|c| {
                c.query_row(
                    "SELECT status, disputed_against FROM memory LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap();

        assert_eq!(status, "active", "single event with no prior rows must be active");
        assert!(da.is_none(), "single event with no prior rows must have disputed_against=NULL");
    }

    // ── Regression: the System→Skip guard still applies to non-Compaction events ──

    #[tokio::test]
    async fn system_role_with_chat_source_still_yields_zero_rows() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.send(MemoryEvent {
            agent_id: "agent-test".to_string(),
            project_id: None,
            role: Role::System,
            content: "system prompt".to_string(),
            is_error: None,
            source: MemorySource::Chat,
            timestamp_ms: 1,
        }).unwrap();
        drain_worker(tx, handle).await;

        let rows = count_memory(&store).await;
        assert_eq!(rows, 0, "Role::System + source=Chat must still Skip; got {rows}");
    }

    // ── Compaction events bypass the System→Skip guard, producing 1 session_summary row ──

    #[tokio::test]
    async fn compaction_event_produces_one_session_summary_row() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.send(MemoryEvent {
            agent_id: "agent-test".to_string(),
            project_id: None,
            role: Role::System,
            content: "<summary>\nConversation summary:\n- Scope: 3 messages compacted.\n</summary>".to_string(),
            is_error: None,
            source: MemorySource::Compaction,
            timestamp_ms: 1,
        }).unwrap();
        drain_worker(tx, handle).await;

        let rows = count_memory(&store).await;
        assert_eq!(rows, 1, "Compaction source must bypass the System→Skip guard and insert exactly 1 row; got {rows}");

        let (mt, vec_count): (String, i64) = store.conn
            .call(|c| -> Result<(String, i64), rusqlite::Error> {
                let mt: String = c.query_row(
                    "SELECT memory_type FROM memory LIMIT 1",
                    [],
                    |r| r.get(0),
                )?;
                let vc: i64 = c.query_row(
                    "SELECT COUNT(*) FROM memory_vectors",
                    [],
                    |r| r.get(0),
                )?;
                Ok((mt, vc))
            })
            .await
            .unwrap();
        assert_eq!(mt, "session_summary", "expected memory_type='session_summary'; got {mt}");
        assert_eq!(vec_count, 1, "expected 1 matching memory_vectors row; got {vec_count}");
    }

    // ── Shared fixture: write two real messages, read them back ────────────

    use crate::memory::prompt_builder::{
        build_memory_prompt, SEP_LONGTERM_TEST, SEP_RECENT_TEST,
    };
    use crate::memory::search::{search_all_memories, RankedHit, SearchScope};
    use crate::memory::workspace::WorkspaceManager;
    use crate::models::chat::{current_time_millis, ChatMessage};

    /// State shared by the two gates below: one fresh message and one a month
    /// old, written through the real worker and read back through both
    /// `search_all_memories` and `build_memory_prompt`. Not a test itself and
    /// not gated behind either test's ignore reason — it exists so the write
    /// path runs once instead of once per assertion group.
    struct WrittenMemoryFixture {
        hits: Vec<RankedHit>,
        prompt: String,
        fresh: ChatMessage,
        stale: ChatMessage,
    }

    async fn write_fresh_and_stale_then_read_back() -> WrittenMemoryFixture {
        const AGENT: &str = "agent-invariant";
        const DAY_MS: u64 = 86_400_000;

        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder = Arc::new(FakeEmbedder::default());
        let (tx, handle) =
            spawn_test_worker(Arc::clone(&store), embedder.clone() as Arc<dyn Embedder>).await;

        let now_ms = current_time_millis();
        let fresh = ChatMessage {
            id: "msg-fresh".to_string(),
            role: Role::User,
            content: "el proyecto usa pnpm y no npm".to_string(),
            tool_calls: None,
            timestamp_ms: now_ms,
            is_error: None,
        };
        let stale = ChatMessage {
            id: "msg-stale".to_string(),
            role: Role::User,
            content: "el monorepo entero se instala con pnpm".to_string(),
            tool_calls: None,
            timestamp_ms: now_ms - 30 * DAY_MS,
            is_error: None,
        };

        for msg in [&fresh, &stale] {
            let evt = MemoryEvent::from_message(
                AGENT.to_string(), None, msg, MemorySource::Chat,
            )
            .expect("a user message with content must produce an ingest event");
            tx.send(evt).unwrap();
        }

        // Dropping the sender closes the channel; the worker's recv loop awaits each
        // insert inline, so it can only observe the closed channel after the last
        // write committed. Awaiting the handle is the barrier — no sleeping.
        drain_worker(tx, handle).await;

        let q_vec = embedder.embed(&fresh.content).await.unwrap();
        let hits = search_all_memories(
            &store, &q_vec, AGENT, SearchScope::all_default(), 10,
        )
        .await
        .unwrap();
        assert_eq!(
            hits.len(), 2,
            "both messages must come back as one row each; a different count means \
             the chunker split them, which changes what the rest of this test measures"
        );

        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let prompt = build_memory_prompt(
            &store, &wm, Some(embedder.clone() as Arc<dyn Embedder>),
            AGENT, None, &fresh.content,
            &crate::memory::workspace::EmittedIds::default(), None,
        )
        .await
        .unwrap();

        WrittenMemoryFixture { hits, prompt, fresh, stale }
    }

    // A written memory has to render with a date a human could read and land in
    // the right recency block. Nothing here names an instant: the two messages
    // are stamped by the same clock production uses, one of them a month back,
    // so the assertions cannot be satisfied by a fixture that happens to agree
    // with a broken unit.
    #[tokio::test]
    async fn e2e_real_write_path_produces_sane_date_and_buckets() {
        let fixture = write_fresh_and_stale_then_read_back().await;
        let mut violations: Vec<String> = Vec::new();

        // Every rendered hit opens with `[YYYY-MM-DD]`.
        let years: Vec<i32> = fixture
            .prompt
            .lines()
            .filter_map(|l| l.strip_prefix('['))
            .filter_map(|l| l.split('-').next())
            .filter_map(|y| y.parse::<i32>().ok())
            .collect();
        if years.len() != 2 {
            violations.push(format!("expected one rendered date per hit, got {years:?}"));
        }
        for y in &years {
            if !(2020..=2100).contains(y) {
                violations.push(format!("rendered year {y} is not a date anyone lived through"));
            }
        }

        match (
            fixture.prompt.find(SEP_RECENT_TEST),
            fixture.prompt.find(SEP_LONGTERM_TEST),
        ) {
            (Some(r), Some(l)) => {
                if !fixture.prompt[r..l].contains(&fixture.fresh.content) {
                    violations.push("the just-written message did not bucket as recent".into());
                }
                if !fixture.prompt[l..].contains(&fixture.stale.content) {
                    violations.push("the month-old message did not bucket as long-term".into());
                }
            }
            (Some(_), None) => violations.push(
                "every hit bucketed as recent — the month-old message never reached long-term".into(),
            ),
            (None, _) => violations.push("no recent block was rendered at all".into()),
        }

        assert!(
            violations.is_empty(),
            "unit invariants violated:\n  - {}",
            violations.join("\n  - ")
        );
    }

    // A written memory's score has to land inside its own contract: strictly
    // positive, never above one. The similarity factor is
    // `cosine_from_l2_unit(distance)` (score.rs), which converts the raw L2
    // distance between two UNIT vectors into exact cosine similarity and
    // clamps it to `MAX(0.0, 1.0 - distance^2/2)` — both edges (insert and
    // query) route through `to_unit` (vector.rs) before a distance is ever
    // measured, so the similarity factor is structurally bounded in [0,1]
    // rather than merely bounded for embeddings that happen to be close
    // together. The remaining three multiplicative factors (decay,
    // importance, dispute) are each already clamped to [0,1] on their own,
    // so the product stays inside the contract too. This end-to-end test
    // exercises a real write followed by a real read to prove the whole
    // pipeline honors the bound, not just the UDF in isolation.
    #[tokio::test]
    async fn e2e_real_write_path_produces_bounded_score() {
        let fixture = write_fresh_and_stale_then_read_back().await;
        let mut violations: Vec<String> = Vec::new();

        for h in &fixture.hits {
            if !(h.score > 0.0 && h.score <= 1.0) {
                violations.push(format!("score {} is outside (0.0, 1.0]", h.score));
            }
        }

        assert!(
            violations.is_empty(),
            "score contract violated:\n  - {}",
            violations.join("\n  - ")
        );
    }

    // ── Test harnesses ───────────────────────────────────────────────────────

    /// Test harness — spawns `memory_worker` with a seeded embedder cell.
    pub(crate) async fn spawn_test_worker(
        store: Arc<MemoryStore>,
        embedder: Arc<dyn Embedder>,
    ) -> (tokio::sync::mpsc::UnboundedSender<MemoryEvent>, tokio::task::JoinHandle<()>) {
        let cell: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
        cell.set(embedder).ok();
        let cell = Arc::new(cell);

        let chat_chunker = Arc::new(ChatChunker::with_fallback());
        let code_chunker = Arc::new(CodeChunker::new());

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<MemoryEvent>();
        let handle = tokio::spawn(async move {
            memory_worker(rx, store, cell, chat_chunker, code_chunker).await;
        });
        (tx, handle)
    }

    /// Variant with optional embedder — None = empty cell, for tests that
    /// exercise the embedder-not-ready gate.
    pub(crate) async fn spawn_test_worker_opt(
        store: Arc<MemoryStore>,
        embedder: Option<Arc<dyn Embedder>>,
    ) -> (tokio::sync::mpsc::UnboundedSender<MemoryEvent>, tokio::task::JoinHandle<()>) {
        let cell: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
        if let Some(e) = embedder {
            cell.set(e).ok();
        }
        let cell = Arc::new(cell);

        let chat_chunker = Arc::new(ChatChunker::with_fallback());
        let code_chunker = Arc::new(CodeChunker::new());

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<MemoryEvent>();
        let handle = tokio::spawn(async move {
            memory_worker(rx, store, cell, chat_chunker, code_chunker).await;
        });
        (tx, handle)
    }
}
