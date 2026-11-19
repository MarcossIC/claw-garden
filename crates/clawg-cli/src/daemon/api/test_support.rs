//! The daemon built over a temporary store, and the helpers the route tests share.
#![cfg(test)]

use axum::Json;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use clawg_core::memory::embedder::Embedder;
use clawg_core::paths::Profile;
use clawg_core::runtime::MemoryRuntime;
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

use super::{ApiResult, AppCtx, ReaderQuery, SAVE_CONFIRM_TIMEOUT};

/// A daemon over an in-memory store and the directory it keeps its
/// snapshots in.
pub(super) async fn daemon_for_jobs() -> (Arc<AppCtx>, tempfile::TempDir) {
    daemon_embedding_with(None).await
}

/// The same daemon, embedding with `embedder` when there is one.
pub(super) async fn daemon_embedding_with(embedder: Option<Arc<dyn Embedder>>) -> (Arc<AppCtx>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let (runtime, _rx) = MemoryRuntime::new_for_test(dir.path().to_path_buf()).await;
    let runtime = match embedder {
        Some(embedder) => {
            let cell = Arc::new(once_cell::sync::OnceCell::<Arc<dyn Embedder>>::new());
            cell.set(embedder).ok();
            MemoryRuntime { embedder: cell, ..runtime }
        }
        None => runtime,
    };
    (daemon_on(runtime), dir)
}

/// The same daemon before its embedder loaded.
pub(super) async fn daemon_without_embedder() -> (Arc<AppCtx>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let (runtime, _rx) = MemoryRuntime::new_for_test(dir.path().to_path_buf()).await;
    (daemon_on(MemoryRuntime { embedder: Default::default(), ..runtime }), dir)
}

pub(super) fn daemon_on(runtime: MemoryRuntime) -> Arc<AppCtx> {
    let (shutdown, _) = tokio::sync::watch::channel(false);
    Arc::new(AppCtx {
        runtime,
        token: "job-token".to_string(),
        profile: Profile::Dev,
        started_at: Instant::now(),
        shutdown,
        save_timeout: SAVE_CONFIRM_TIMEOUT,
        sessions: Default::default(),
        imports: Default::default(),
    })
}

pub(super) async fn count(app: &AppCtx, sql: &'static str) -> i64 {
    app.runtime.store.conn_for_test().call(move |c| c.query_row(sql, [], |r| r.get(0))).await.expect("count")
}

/// The status and the JSON body of a handler's answer.
pub(super) async fn status_and_body(response: Response) -> (StatusCode, serde_json::Value) {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.expect("the body");
    (status, serde_json::from_slice(&bytes).expect("a JSON body"))
}

pub(super) async fn run_sql(app: &AppCtx, sql: String) {
    app.runtime.store.conn_for_test().call(move |c| c.execute_batch(&sql)).await.expect("run the SQL");
}

/// The status and the JSON body of a handler's answer, accepted or refused.
pub(super) async fn answer_of<T: Serialize>(answer: ApiResult<Json<T>>) -> (StatusCode, serde_json::Value) {
    status_and_body(match answer {
        Ok(body) => body.into_response(),
        Err(e) => e.into_response(),
    })
    .await
}

/// A live row of `memory` saved by `agent`.
pub(super) async fn seed_memory(app: &AppCtx, agent: &str, memory_type: &str, content: &str) -> i64 {
    let (agent, memory_type, content) = (agent.to_string(), memory_type.to_string(), content.to_string());
    app.runtime
        .store
        .conn_for_test()
        .call(move |c| {
            c.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source, created_at, created_by_type, \
                 created_by_ref, last_modified_at, last_accessed_at, importance, tier, status) \
                 VALUES ('agent', ?1, ?2, ?3, 'explicit', 1, 'agent', ?1, 1, 1, 5.0, 3, 'active')",
                [&agent, &content, &memory_type],
            )?;
            Ok::<_, rusqlite::Error>(c.last_insert_rowid())
        })
        .await
        .expect("seed a row")
}

pub(super) fn reader_query(agent: Option<&str>) -> Query<ReaderQuery> {
    Query(ReaderQuery { agent: agent.map(str::to_string) })
}
