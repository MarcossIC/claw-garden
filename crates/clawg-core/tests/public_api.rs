//! Cross-crate guard on the surface `clawg-cli` (the `clawg serve` daemon and
//! the `clawg mcp` stdio bridge) codes against.
//!
//! These assertions are only meaningful from OUTSIDE the crate: a `#[cfg(test)]`
//! module inside `clawg-core` can reach `pub(crate)` items just as easily as
//! `pub` ones, so it cannot tell the two apart. An integration test links
//! `clawg_core` as a foreign crate, which makes "is this actually public?" a
//! compile error instead of an opinion.
//!
//! The cases that need an in-memory store are gated on the `testing` feature,
//! the same gate `MemoryStore::new_for_test` itself carries.

use clawg_core::memory::clock::TimestampMs;
use clawg_core::memory::events::{MemoryEvent, MemorySource};
use clawg_core::memory::store_lock::StoreHolder;
use clawg_core::memory::{
    search_all_memories, MemoryError, MemoryStore, RankedHit, SearchScope, TableKind,
};
use clawg_core::models::chat::Role;
use clawg_core::runtime::MemoryRuntime;

// ── structured search is public ───────────────────────────────────────────

/// The daemon builds a scope, so both constructors and every field must be
/// reachable from another crate.
#[test]
fn search_scope_constructors_and_fields_are_public() {
    let scope = SearchScope::all_default().with_project(Some("proj-1"));
    assert_eq!(scope.memory, Some(30));
    assert_eq!(scope.episodic, Some(5));
    assert_eq!(scope.procedural, Some(5));
    assert_eq!(scope.lexical, Some(20));
    assert_eq!(scope.project_id.as_deref(), Some("proj-1"));
}

/// Coercing to a function pointer pins the exact signature the daemon calls,
/// not merely the name.
#[test]
fn search_all_memories_is_public_with_the_expected_signature() {
    type SearchFut<'a> = std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Vec<RankedHit>, clawg_core::memory::MemoryError>>
                + Send
                + 'a,
        >,
    >;
    type SearchFn = for<'a> fn(
        &'a MemoryStore,
        &'a [f32],
        Option<&'a str>,
        &'a str,
        SearchScope,
        usize,
    ) -> SearchFut<'a>;
    // Never called — the coercion itself is the assertion.
    let _: SearchFn = |s, e, q, a, sc, k| Box::pin(search_all_memories(s, e, q, a, sc, k));
}

/// The daemon answers HTTP/MCP with hits as JSON, so `RankedHit` (and the
/// `TableKind` it carries) must serialize without the caller re-shaping them.
#[test]
fn ranked_hit_serializes_to_json() {
    let hit = RankedHit {
        id: 7,
        kind: TableKind::Memory,
        content: "hello".into(),
        memory_type: "semantic_fact".into(),
        tier: 2,
        status: "active".into(),
        score: 0.75,
        distance: 0.25,
        created_at: TimestampMs::now(),
    };
    let json = serde_json::to_value(&hit).expect("RankedHit must be Serialize");
    assert_eq!(json["id"], 7);
    assert_eq!(json["memory_type"], "semantic_fact");
    assert_eq!(json["kind"], "Memory");
    assert!(json["created_at"].is_i64(), "created_at must serialize as a millisecond number");
}

// ── explicit memories are constructible from outside the crate ────────────

fn explicit_event(content: &str, memory_type: Option<&str>, importance: Option<f64>) -> MemoryEvent {
    MemoryEvent {
        agent_id: "agent-1".into(),
        project_id: None,
        role: Role::User,
        content: content.into(),
        is_error: None,
        source: MemorySource::Explicit,
        timestamp_ms: 1,
        memory_type: memory_type.map(str::to_owned),
        importance,
        derived_from: Vec::new(),
    }
}

// ── Exclusive ownership, seen from outside the crate ──────────────────────

/// The guarantee a host relies on: while one runtime is alive on a data
/// directory, a second `open` — from ANY host — is refused and says who has it.
#[tokio::test]
async fn a_second_runtime_open_on_the_same_dir_is_refused_naming_the_first_holder() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().to_path_buf();

    let (_first, _rx) = MemoryRuntime::open(base.clone(), StoreHolder::Daemon)
        .await
        .expect("first open");

    let err = MemoryRuntime::open(base, StoreHolder::Eval)
        .await
        .err()
        .expect("a second open must be refused while the first runtime lives");
    match err {
        MemoryError::StoreInUse { owner: Some(owner) } => {
            assert_eq!(owner.holder, StoreHolder::Daemon);
            assert_eq!(owner.pid, std::process::id());
        }
        other => panic!("expected StoreInUse naming the daemon, got {other:?}"),
    }
}

// ── MemoryRuntime boots on a plain tokio runtime ──────────────────────────

/// A plain tokio runtime, with no host framework, can open the store and get a
/// usable handle set back.
#[tokio::test]
async fn memory_runtime_open_creates_the_db_and_returns_a_usable_store() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().to_path_buf();

    let (rt, _rx) = MemoryRuntime::open(base.clone(), StoreHolder::Daemon)
        .await
        .expect("MemoryRuntime::open must succeed on a fresh dir");

    assert_eq!(rt.base_dir, base);
    assert!(base.join("memory.db").exists(), "open() must create <base_dir>/memory.db");

    // The store is not merely constructed — the sqlite-vec extension is loaded.
    let version: String = rt
        .store
        .conn
        .call(|c| -> Result<String, rusqlite::Error> {
            c.query_row("SELECT vec_version()", [], |r| r.get(0))
        })
        .await
        .expect("vec_version()");
    assert!(!version.is_empty());

    // Production open() leaves the embedder cell empty — the bootstrap fills it.
    assert!(!rt.embedder_ready());
    assert!(rt.tokenizer.get().is_none());

    // The ingest channel is live end to end.
    rt.memory_tx
        .try_send(explicit_event("hello", None, None).into())
        .expect("memory_tx must accept a send before the worker is spawned");

    // The access tracker's flush worker is running (record must never block).
    rt.access_tracker.record(vec![1, 2, 3]);

    // workspaces() points at <base_dir>/workspaces.
    let wm = rt.workspaces();
    assert_eq!(
        wm.workspace_path("agent-1", None),
        base.join("workspaces").join("agent-1").join("default").join("workspace.md")
    );
}

/// `new_for_test` is a shortcut for hosts' tests: an in-memory store with the
/// embedder already seeded, so nothing races a bootstrap that does not exist
/// under test.
#[cfg(feature = "testing")]
#[tokio::test]
async fn memory_runtime_new_for_test_preseeds_the_embedder() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (rt, _rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
    assert!(rt.embedder_ready(), "new_for_test must pre-seed the embedder cell");
    let e = rt.embedder.get().expect("seeded").clone();
    assert_eq!(e.model_id(), "fake-embedder");
    assert!(
        !tmp.path().join("memory.db").exists(),
        "new_for_test must use an in-memory store, not touch disk"
    );
}

/// End to end: spawn the ingest worker future on plain tokio, push one event,
/// and see a row land.
#[cfg(feature = "testing")]
#[tokio::test]
async fn memory_runtime_ingest_worker_drains_an_event_into_a_row() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (rt, rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
    let store = rt.store.clone();
    let tx = rt.memory_tx.clone();

    let worker = tokio::spawn(rt.ingest_worker(rx, tokio::sync::watch::channel(None).1));

    tx.try_send(explicit_event("the daemon ingests on a plain tokio runtime", None, None).into())
        .expect("send");
    drop(tx);
    drop(rt);
    worker.await.expect("worker must exit cleanly when every sender is dropped");

    let count: i64 = store
        .conn
        .call(|c| -> Result<i64, rusqlite::Error> {
            c.query_row("SELECT COUNT(*) FROM memory WHERE source = 'explicit'", [], |r| r.get(0))
        })
        .await
        .expect("count");
    assert_eq!(count, 1, "the ingest worker must have written one explicit row");
}

/// The maintenance loop runs forever, so the assertion is that its future is
/// spawnable at all (`Send + 'static`).
#[cfg(feature = "testing")]
#[tokio::test]
async fn memory_runtime_maintenance_worker_returns_a_spawnable_future() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (rt, _rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
    let fut = rt.maintenance_worker();
    fn assert_spawnable<F: std::future::Future<Output = ()> + Send + 'static>(_: &F) {}
    assert_spawnable(&fut);
    let handle = tokio::spawn(fut);
    handle.abort(); // it never completes on its own; aborting is the exit.
}

/// The MCP `save_memory` tool's overrides must survive the trip through the
/// worker into the stored row.
#[cfg(feature = "testing")]
#[tokio::test]
async fn explicit_overrides_reach_the_stored_row() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (rt, rx) = MemoryRuntime::new_for_test(tmp.path().to_path_buf()).await;
    let store = rt.store.clone();
    let tx = rt.memory_tx.clone();
    let worker = tokio::spawn(rt.ingest_worker(rx, tokio::sync::watch::channel(None).1));

    tx.try_send(
        explicit_event(
            "the team uses conventional commits",
            Some("user_preference"),
            Some(9.5),
        )
        .into(),
    )
    .expect("send");
    drop(tx);
    drop(rt);
    worker.await.expect("worker exits");

    let (mtype, importance, created_by): (String, f64, String) = store
        .conn
        .call(|c| -> Result<(String, f64, String), rusqlite::Error> {
            c.query_row(
                "SELECT memory_type, importance, created_by_type FROM memory \
                 WHERE source = 'explicit'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
        .await
        .expect("row");
    assert_eq!(mtype, "user_preference");
    assert!((importance - 9.5).abs() < 1e-9);
    assert_eq!(created_by, "user");
}
