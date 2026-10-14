//! End-to-end tests for the daemon's HTTP contract.
//!
//! Every test runs a real `axum::serve` on an ephemeral loopback port inside
//! the test process and talks to it with `reqwest`, so what is under test is
//! the wire behaviour — status codes, JSON shapes, auth — and not the handler
//! functions in isolation.
//!
//! The daemon binary is never spawned: `clawg serve` would download a 470 MB
//! model. `MemoryRuntime::new_for_test` gives the same handles with an
//! in-memory store and a deterministic `FakeEmbedder` instead.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clawg_cli::daemon::api::{router, AppCtx, SAVE_CONFIRM_TIMEOUT};
use clawg_core::memory::bootstrap::EmbedderStatus;
use clawg_core::memory::embedder::{EmbedError, EmbedRole, Embedder, FakeEmbedder};
use clawg_core::memory::worker::IngestRequest;
use clawg_core::paths::Profile;
use clawg_core::runtime::{MemoryRuntime, INGEST_QUEUE_CAPACITY};
use tokio::sync::mpsc::Receiver;

use once_cell::sync::OnceCell;
use serde_json::{json, Value};

/// Which embedder a test daemon was started with.
enum EmbedderState {
    Ready,
    NeverLoaded,
    ServedEmpty,
    /// The bootstrap gave up or is waiting to retry. No worker runs, so
    /// whatever reaches the queue stays there for the test to see.
    Failed { error: &'static str, retryable: bool },
    /// The model is still loading and no worker drains the queue, so a test
    /// can fill it.
    LoadingWithoutWorker,
    /// The embedder is ready but no worker runs, so no save is ever confirmed.
    ReadyWithoutWorker,
    /// The embedder loaded but every embedding call fails.
    FailingEmbedder,
    /// The embedder is ready and something takes each save off the queue but
    /// drops its reply unanswered, as a worker spilling at shutdown does.
    DroppingReplies,
}

/// An embedder that loaded and then fails every call, as a broken ONNX
/// session would.
struct FailingEmbedder;

#[async_trait::async_trait]
impl Embedder for FailingEmbedder {
    async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
        Err(EmbedError::InferenceFailed("onnx session crashed".into()))
    }
    fn dimensions(&self) -> usize {
        384
    }
    fn model_id(&self) -> &str {
        "failing"
    }
    fn query_prefix(&self) -> &str {
        ""
    }
    fn passage_prefix(&self) -> &str {
        ""
    }
}

/// How long a test daemon waits for the worker to confirm a save when the
/// test is about the wait running out.
const SHORT_CONFIRM_TIMEOUT: Duration = Duration::from_millis(100);

/// Swaps in a fresh, EMPTY embedder cell while moving every other handle
/// across untouched — the cheapest way to reproduce a pre-bootstrap daemon
/// without a second constructor in core.
fn with_empty_embedder(runtime: MemoryRuntime, status: EmbedderStatus) -> MemoryRuntime {
    MemoryRuntime {
        embedder: Arc::new(OnceCell::<Arc<dyn Embedder>>::new()),
        embedder_status: tokio::sync::watch::Sender::new(status),
        ..runtime
    }
}

/// A live daemon on a loopback port, plus everything a test needs to talk to it.
struct Harness {
    addr: SocketAddr,
    token: String,
    http: reqwest::Client,
    ctx: Arc<AppCtx>,
    /// The queue's consumer end when no worker drains it, held so the queue
    /// stays open.
    _ingest_rx: Option<Receiver<IngestRequest>>,
    // Held so the temporary data directory outlives the server.
    _dir: tempfile::TempDir,
}

impl Harness {
    /// Normal daemon: embedder ready, ingest worker running.
    async fn start() -> Self {
        Self::start_inner(EmbedderState::Ready).await
    }

    /// Daemon whose embedder cell was never filled — the state a real daemon is
    /// in for the first minutes after a cold start, while the model downloads.
    async fn start_without_embedder() -> Self {
        Self::start_inner(EmbedderState::NeverLoaded).await
    }

    /// Daemon that reads as if the model had not loaded, over rows that were
    /// ingested while it had: the only way to put data in a store a read path
    /// is then exercised against with an empty cell, since ingest needs the
    /// embedder.
    async fn start_serving_without_embedder() -> Self {
        Self::start_inner(EmbedderState::ServedEmpty).await
    }

    /// Daemon whose bootstrap failed with `error`.
    async fn start_failed(error: &'static str, retryable: bool) -> Self {
        Self::start_inner(EmbedderState::Failed { error, retryable }).await
    }

    /// Daemon with a ready embedder whose saves are never confirmed.
    async fn start_without_worker() -> Self {
        Self::start_inner(EmbedderState::ReadyWithoutWorker).await
    }

    /// Daemon whose saves are taken off the queue and never answered.
    async fn start_dropping_replies() -> Self {
        Self::start_inner(EmbedderState::DroppingReplies).await
    }

    /// Daemon whose embedder fails every call.
    async fn start_with_failing_embedder() -> Self {
        Self::start_inner(EmbedderState::FailingEmbedder).await
    }

    /// Daemon whose model is loading, with an undrained queue of `capacity`.
    async fn start_loading_with_queue(capacity: usize) -> Self {
        Self::start_with(EmbedderState::LoadingWithoutWorker, capacity).await
    }

    async fn start_inner(embedder: EmbedderState) -> Self {
        Self::start_with(embedder, INGEST_QUEUE_CAPACITY).await
    }

    async fn start_with(embedder: EmbedderState, capacity: usize) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let (runtime, rx) =
            MemoryRuntime::new_for_test_with_capacity(dir.path().to_path_buf(), capacity).await;

        // The ingest worker is what turns a queued event into a row. Without
        // it `POST /v1/memories` would answer 202 forever and nothing would
        // ever land, which is exactly the bug these tests exist to catch. It
        // clones its handles when it is created, so what it holds is whatever
        // the runtime had at that moment.
        let save_timeout = match embedder {
            EmbedderState::ReadyWithoutWorker => SHORT_CONFIRM_TIMEOUT,
            _ => SAVE_CONFIRM_TIMEOUT,
        };
        let (runtime, ingest_rx) = match embedder {
            EmbedderState::Ready => {
                tokio::spawn(runtime.ingest_worker(rx, tokio::sync::watch::channel(None).1));
                (runtime, None)
            }
            EmbedderState::NeverLoaded => {
                let runtime = with_empty_embedder(runtime, EmbedderStatus::Loading);
                tokio::spawn(runtime.ingest_worker(rx, tokio::sync::watch::channel(None).1));
                (runtime, None)
            }
            EmbedderState::ServedEmpty => {
                tokio::spawn(runtime.ingest_worker(rx, tokio::sync::watch::channel(None).1));
                (with_empty_embedder(runtime, EmbedderStatus::Loading), None)
            }
            EmbedderState::Failed { error, retryable } => {
                let status = EmbedderStatus::Failed { error: error.into(), retryable };
                (with_empty_embedder(runtime, status), Some(rx))
            }
            EmbedderState::LoadingWithoutWorker => {
                (with_empty_embedder(runtime, EmbedderStatus::Loading), Some(rx))
            }
            EmbedderState::ReadyWithoutWorker => (runtime, Some(rx)),
            EmbedderState::FailingEmbedder => {
                let runtime = with_empty_embedder(runtime, EmbedderStatus::Ready);
                runtime.embedder.set(Arc::new(FailingEmbedder) as Arc<dyn Embedder>).ok();
                tokio::spawn(runtime.ingest_worker(rx, tokio::sync::watch::channel(None).1));
                (runtime, None)
            }
            EmbedderState::DroppingReplies => {
                let mut rx = rx;
                tokio::spawn(async move { while rx.recv().await.is_some() {} });
                (runtime, None)
            }
        };

        let token = "test-token-0123456789abcdef".to_string();
        let (shutdown, _rx) = tokio::sync::watch::channel(false);
        let ctx = Arc::new(AppCtx {
            runtime,
            token: token.clone(),
            profile: Profile::Dev,
            started_at: Instant::now(),
            shutdown,
            save_timeout,
        });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral loopback port");
        let addr = listener.local_addr().expect("local_addr");
        let app = router(ctx.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self { addr, token, http: reqwest::Client::new(), ctx, _ingest_rx: ingest_rx, _dir: dir }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.bearer_auth(&self.token)
    }

    async fn get_json(&self, path: &str) -> (u16, Value) {
        let r = self
            .authed(self.http.get(self.url(path)))
            .send()
            .await
            .expect("request");
        let status = r.status().as_u16();
        (status, r.json().await.unwrap_or(Value::Null))
    }

    async fn post_json(&self, path: &str, body: Value) -> (u16, Value) {
        let r = self
            .authed(self.http.post(self.url(path)))
            .json(&body)
            .send()
            .await
            .expect("request");
        let status = r.status().as_u16();
        (status, r.json().await.unwrap_or(Value::Null))
    }

    /// Waits for the ingest worker to turn queued events into rows.
    ///
    /// Polling rather than sleeping a fixed interval: ingest does real work
    /// (chunk, embed, insert) whose duration is not something a test can
    /// predict, and a fixed sleep is either flaky or slow. Bounded at ~2 s so a
    /// genuinely stuck pipeline fails the test instead of hanging CI.
    async fn wait_for_rows(&self, kind: &str, want: usize) -> Vec<Value> {
        for _ in 0..40 {
            let (status, body) = self.get_json(&format!("/v1/memories?kind={kind}")).await;
            assert_eq!(status, 200, "list failed: {body}");
            let items = body["items"].as_array().cloned().unwrap_or_default();
            if items.len() >= want {
                return items;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("ingest never produced {want} {kind} row(s) within 2 s");
    }
}

// ── /health ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn health_answers_without_a_token() {
    let h = Harness::start().await;
    let r = h.http.get(h.url("/health")).send().await.expect("request");
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.expect("json");

    assert_eq!(body["status"], "ok");
    assert_eq!(body["profile"], "dev");
    assert_eq!(body["embedder"], "ready");
    assert!(body["embedder_error"].is_null());
    assert!(body["embedder_retryable"].is_null());
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["pid"], std::process::id());
    assert!(body["uptime_s"].is_u64());
}

/// A daemon still downloading its model must still report healthy — a client
/// that waited for `ready` would block on a 470 MB download before its first
/// save, which does not need an embedder at all.
#[tokio::test]
async fn health_reports_loading_while_the_embedder_cell_is_empty() {
    let h = Harness::start_without_embedder().await;
    let r = h.http.get(h.url("/health")).send().await.expect("request");
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.expect("json");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["embedder"], "loading");
    assert!(body["embedder_error"].is_null());
    assert!(body["embedder_retryable"].is_null());
}

#[tokio::test]
async fn health_reports_failed_with_the_cause_after_a_failed_bootstrap() {
    let h = Harness::start_failed("download failed: connection refused", true).await;
    let r = h.http.get(h.url("/health")).send().await.expect("request");
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.expect("json");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["embedder"], "failed");
    assert!(
        body["embedder_error"].as_str().is_some_and(|e| e.contains("connection refused")),
        "got {body}"
    );
    assert_eq!(body["embedder_retryable"], true);
}

#[tokio::test]
async fn health_reports_a_contract_mismatch_as_not_retryable() {
    let h = Harness::start_failed("embedding contract mismatch (dim)", false).await;
    let body: Value = h.http.get(h.url("/health")).send().await.unwrap().json().await.unwrap();
    assert_eq!(body["embedder"], "failed");
    assert_eq!(body["embedder_retryable"], false);
}

// ── Auth ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn v1_without_a_token_is_unauthorized() {
    let h = Harness::start().await;
    let r = h.http.get(h.url("/v1/memories")).send().await.expect("request");
    assert_eq!(r.status(), 401);
    let body: Value = r.json().await.expect("json");
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn v1_with_the_wrong_token_is_unauthorized() {
    let h = Harness::start().await;
    let r = h
        .http
        .get(h.url("/v1/memories"))
        .bearer_auth("not-the-token")
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 401);
}

/// A token of the right length but wrong content must fail exactly like a short
/// one — the comparison must not leak through an early length check turning
/// into a different status.
#[tokio::test]
async fn a_same_length_wrong_token_is_also_unauthorized() {
    let h = Harness::start().await;
    let wrong: String = h.token.chars().rev().collect();
    let r = h
        .http
        .get(h.url("/v1/memories"))
        .bearer_auth(&wrong)
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn a_non_bearer_authorization_header_is_unauthorized() {
    let h = Harness::start().await;
    let r = h
        .http
        .get(h.url("/v1/memories"))
        .header("authorization", format!("Basic {}", h.token))
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 401);
}

// ── Save while the embedder is not ready ────────────────────────────────────

const FAILED_CAUSE: &str = "download failed: connection refused";

#[tokio::test]
async fn a_save_after_the_bootstrap_failed_is_503_with_the_cause() {
    let h = Harness::start_failed(FAILED_CAUSE, true).await;
    let (status, body) = h
        .post_json("/v1/memories", json!({ "content": "not stored", "agent_id": "a" }))
        .await;

    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"], "embedder_failed");
    assert_eq!(body["detail"], FAILED_CAUSE);
    assert_eq!(body["retryable"], true);
    let tx = &h.ctx.runtime.memory_tx;
    assert_eq!(tx.capacity(), tx.max_capacity(), "a refused save must not reach the queue");
}

#[tokio::test]
async fn search_after_the_bootstrap_failed_says_failed_not_loading() {
    let h = Harness::start_failed(FAILED_CAUSE, false).await;
    let (status, body) = h
        .post_json("/v1/search", json!({ "query": "anything", "agent_id": "a" }))
        .await;

    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"], "embedder_failed");
    assert_eq!(body["detail"], FAILED_CAUSE);
    assert_eq!(body["retryable"], false);
}

#[tokio::test]
async fn a_save_while_the_model_loads_answers_202_and_lands_once_it_loads() {
    let h = Harness::start_without_embedder().await;
    let (status, body) = h
        .post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "agent-http" }))
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["status"], "queued");
    assert_eq!(body["reason"], "embedder_loading");

    h.ctx.runtime.embedder.set(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>).ok();

    let items = h.wait_for_rows("memory", 1).await;
    assert_eq!(items[0]["summary"], CONTENT);
}

#[tokio::test]
async fn a_save_into_a_full_queue_is_503_queue_full() {
    let h = Harness::start_loading_with_queue(2).await;
    let save = |n: u32| h.post_json("/v1/memories", json!({ "content": format!("save {n}"), "agent_id": "a" }));

    assert_eq!(save(1).await.0, 202);
    assert_eq!(save(2).await.0, 202);
    let (status, body) = save(3).await;

    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"], "ingest_queue_full");
}

// ── Save → list → search → context → patch → delete ─────────────────────────

const CONTENT: &str = "the deployment pipeline uses blue green releases on fridays";

#[tokio::test]
async fn a_saved_memory_is_a_listable_row_when_the_save_returns() {
    let h = Harness::start().await;

    let (status, body) = h
        .post_json(
            "/v1/memories",
            json!({
                "content": CONTENT,
                "agent_id": "agent-http",
                "memory_type": "semantic_fact",
                "importance": 8.0
            }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["status"], "saved");

    let (status, list) = h.get_json("/v1/memories?kind=memory").await;
    assert_eq!(status, 200);
    let items = list["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "the row must exist by the time the save answers");
    let row = &items[0];
    assert_eq!(row["kind"], "memory");
    assert_eq!(row["agent_id"], "agent-http");
    assert_eq!(row["summary"], CONTENT);
    assert_eq!(
        row["memory_type"], "semantic_fact",
        "an explicit save must honour the caller's declared type"
    );
    assert_eq!(row["importance"], 8.0);
}

#[tokio::test]
async fn a_save_answers_201_with_the_id_of_the_row_it_wrote() {
    let h = Harness::start().await;
    let (status, body) = h
        .post_json("/v1/memories", json!({ "content": "the train leaves on tuesdays", "agent_id": "a" }))
        .await;

    assert_eq!(status, 201, "{body}");
    assert_eq!(body["status"], "saved");
    let memories = body["memories"].as_array().expect("memories");
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0]["kind"], "memory");
    assert_eq!(memories[0]["outcome"], "inserted");
    let id = memories[0]["id"].as_i64().expect("id");

    let (status, item) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    assert_eq!(status, 200, "{item}");
    assert_eq!(item["item"]["summary"], "the train leaves on tuesdays");
}

#[tokio::test]
async fn a_save_reports_the_secrets_it_masked() {
    let h = Harness::start().await;
    // Built at run time so no literal in the repository looks like a live
    // credential to a secret scanner.
    let token = format!("xoxb-{}", "12345-".repeat(4));
    let (status, body) = h
        .post_json("/v1/memories", json!({ "content": format!("the bot uses {token}"), "agent_id": "a" }))
        .await;

    assert_eq!(status, 201, "{body}");
    assert_eq!(body["redacted"], json!(["slack_token"]));
    let id = body["memories"][0]["id"].as_i64().expect("id");
    let (_, item) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    assert_eq!(item["item"]["summary"], "the bot uses [REDACTED:slack_token]");

    let (_, plain) = h.post_json("/v1/memories", json!({ "content": "nothing secret", "agent_id": "a" })).await;
    assert!(plain.get("redacted").is_none(), "{plain}");
}

#[tokio::test]
async fn a_multi_chunk_save_returns_one_id_per_chunk() {
    let h = Harness::start().await;
    // Distinct words, long enough for three windows of the fallback chunker.
    let content = (0..700).map(|i| format!("w{i:05}")).collect::<Vec<_>>().join(" ");
    let (status, body) = h.post_json("/v1/memories", json!({ "content": content, "agent_id": "a" })).await;

    assert_eq!(status, 201, "{body}");
    let memories = body["memories"].as_array().expect("memories");
    assert_eq!(memories.len(), 3, "{body}");
    assert!(memories.iter().all(|m| m["outcome"] == "inserted"));
    let mut ids: Vec<i64> = memories.iter().map(|m| m["id"].as_i64().unwrap()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3, "each chunk has its own row: {body}");
}

#[tokio::test]
async fn saving_the_same_text_twice_returns_the_same_id_as_reinforced() {
    let h = Harness::start().await;
    let save = || h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a" }));

    let (_, first) = save().await;
    let (status, second) = save().await;

    assert_eq!(status, 201, "{second}");
    assert_eq!(second["memories"].as_array().map(Vec::len), Some(1));
    assert_eq!(second["memories"][0]["id"], first["memories"][0]["id"]);
    assert_eq!(second["memories"][0]["outcome"], "reinforced");
    let (_, list) = h.get_json("/v1/memories?kind=memory").await;
    assert_eq!(list["items"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn a_failed_embed_reaches_the_caller_as_an_error() {
    let h = Harness::start_with_failing_embedder().await;
    let (status, body) = h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a" })).await;

    assert_eq!(status, 500, "{body}");
    assert_eq!(body["error"], "ingest_failed");
    assert!(body["detail"].as_str().is_some_and(|d| d.contains("onnx session crashed")), "{body}");
    assert_eq!(body["memories"], json!([]));
    let (_, list) = h.get_json("/v1/memories?kind=memory").await;
    assert_eq!(list["items"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn a_save_the_worker_does_not_confirm_in_time_answers_202_unconfirmed() {
    let h = Harness::start_without_worker().await;
    let started = Instant::now();
    let (status, body) = h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a" })).await;

    assert_eq!(status, 202, "{body}");
    assert_eq!(body["status"], "queued");
    assert_eq!(body["reason"], "unconfirmed");
    assert!(started.elapsed() < Duration::from_secs(1), "took {:?}", started.elapsed());
}

#[tokio::test]
async fn a_save_whose_reply_is_dropped_answers_202_unconfirmed() {
    let h = Harness::start_dropping_replies().await;
    let started = Instant::now();
    let (status, body) = h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a" })).await;

    assert_eq!(status, 202, "{body}");
    assert_eq!(body["status"], "queued");
    assert_eq!(body["reason"], "unconfirmed");
    assert!(
        started.elapsed() < SAVE_CONFIRM_TIMEOUT / 2,
        "a dropped reply must not wait out the confirmation timeout: took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn saving_empty_content_is_rejected_before_it_reaches_the_queue() {
    let h = Harness::start().await;
    for content in ["", "   \n  "] {
        let (status, body) = h
            .post_json(
                "/v1/memories",
                json!({ "content": content, "agent_id": "agent-http" }),
            )
            .await;
        assert_eq!(status, 400, "content {content:?} must be refused");
        assert!(
            body["error"].as_str().is_some_and(|e| e.contains("content")),
            "the error must name the offending field: {body}"
        );
    }
}

#[tokio::test]
async fn search_finds_a_saved_memory() {
    let h = Harness::start().await;
    h.post_json(
        "/v1/memories",
        json!({ "content": CONTENT, "agent_id": "agent-http" }),
    )
    .await;
    let items = h.wait_for_rows("memory", 1).await;
    let saved_id = items[0]["id"].as_i64().expect("id");

    let (status, body) = h
        .post_json(
            "/v1/search",
            json!({ "query": CONTENT, "agent_id": "agent-http" }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let hits = body["hits"].as_array().expect("hits array");
    assert!(
        hits.iter().any(|hit| hit["id"].as_i64() == Some(saved_id)),
        "the saved row must be retrievable: {body}"
    );

    // Shape check on the row the daemon serializes, since the MCP bridge
    // formats exactly these fields.
    let hit = hits
        .iter()
        .find(|hit| hit["id"].as_i64() == Some(saved_id))
        .expect("hit");
    assert!(hit["content"].is_string());
    assert!(hit["kind"].is_string());
    assert!(hit["memory_type"].is_string());
    assert!(hit["tier"].is_i64());
    assert!(hit["score"].is_number());
}

#[tokio::test]
async fn search_honours_and_caps_the_limit() {
    let h = Harness::start().await;
    for n in 0..3 {
        h.post_json(
            "/v1/memories",
            json!({ "content": format!("{CONTENT} variant {n}"), "agent_id": "agent-http" }),
        )
        .await;
    }
    h.wait_for_rows("memory", 3).await;

    let (status, body) = h
        .post_json(
            "/v1/search",
            json!({ "query": CONTENT, "agent_id": "agent-http", "limit": 1 }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["hits"].as_array().expect("hits").len(), 1);

    // An absurd limit is clamped, not refused.
    let (status, _) = h
        .post_json(
            "/v1/search",
            json!({ "query": CONTENT, "agent_id": "agent-http", "limit": 100_000 }),
        )
        .await;
    assert_eq!(status, 200);
}

/// Saves one row identical to the query and a few unrelated ones, so a search
/// has a clear winner and a tail far below it.
async fn seed_winner_and_tail(h: &Harness) {
    h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "agent-cut" })).await;
    for text in [
        "the garden hose leaks at the tap",
        "quarterly invoices are due on friday",
        "a recipe for lentil soup with cumin",
    ] {
        h.post_json("/v1/memories", json!({ "content": text, "agent_id": "agent-cut" })).await;
    }
    h.wait_for_rows("memory", 4).await;
}

fn hit_ids(body: &Value) -> Vec<i64> {
    body["hits"].as_array().expect("hits").iter().map(|h| h["id"].as_i64().expect("id")).collect()
}

#[tokio::test]
async fn search_without_a_relative_cut_is_unchanged() {
    let h = Harness::start().await;
    seed_winner_and_tail(&h).await;

    let (status, body) = h
        .post_json("/v1/search", json!({ "query": CONTENT, "agent_id": "agent-cut" }))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(hit_ids(&body).len(), 4, "every row is returned when no cut is asked for: {body}");
}

#[tokio::test]
async fn search_applies_the_requested_relative_cut() {
    let h = Harness::start().await;
    seed_winner_and_tail(&h).await;

    let (_, full) = h
        .post_json("/v1/search", json!({ "query": CONTENT, "agent_id": "agent-cut" }))
        .await;
    let scores: Vec<f64> =
        full["hits"].as_array().expect("hits").iter().map(|h| h["score"].as_f64().expect("score")).collect();
    let expected_len = clawg_cli::relevance::relevant_len(&scores, 0.6);
    assert!(expected_len < scores.len(), "the fixture must leave a tail to cut: {full}");

    let (status, cut) = h
        .post_json(
            "/v1/search",
            json!({ "query": CONTENT, "agent_id": "agent-cut", "min_relative_score": 0.6 }),
        )
        .await;
    assert_eq!(status, 200, "{cut}");
    assert_eq!(hit_ids(&cut), hit_ids(&full)[..expected_len]);
}

#[tokio::test]
async fn search_rejects_a_relative_score_outside_zero_to_one() {
    let h = Harness::start().await;
    for bad in [json!(1.5), json!(-0.1)] {
        let (status, body) = h
            .post_json(
                "/v1/search",
                json!({ "query": "anything", "agent_id": "a", "min_relative_score": bad }),
            )
            .await;
        assert_eq!(status, 400, "{bad} must be refused: {body}");
    }
}

/// The whole reason `/health` reports `loading`: retrieval is the one thing a
/// pre-bootstrap daemon genuinely cannot do, and it has to say so in a way a
/// client can branch on.
#[tokio::test]
async fn search_before_the_embedder_is_ready_is_503() {
    let h = Harness::start_without_embedder().await;
    let (status, body) = h
        .post_json("/v1/search", json!({ "query": "anything", "agent_id": "a" }))
        .await;
    assert_eq!(status, 503);
    assert_eq!(body["error"], "embedder_not_ready");
}

#[tokio::test]
async fn context_returns_a_prompt_string() {
    let h = Harness::start().await;
    let (status, body) = h
        .post_json(
            "/v1/context",
            json!({ "query": "what do we know", "agent_id": "agent-http" }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["prompt"].is_string(),
        "an empty prompt is a valid answer, a missing field is not: {body}"
    );
}

#[tokio::test]
async fn context_reports_unreviewed_memories() {
    let h = Harness::start().await;
    for content in ["the go service uses tabs", "the rust crate uses tabs"] {
        h.post_json("/v1/memories", json!({ "content": content, "agent_id": "a", "project_id": "p" })).await;
    }
    let context = || h.post_json("/v1/context", json!({ "query": "tabs", "agent_id": "a", "project_id": "p" }));

    let (status, body) = context().await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["unreviewed"], 2, "{body}");

    let (status, body) = h.post_json("/v1/synthesis", json!({ "agent_id": "b", "project_id": "p" })).await;
    assert_eq!(status, 200, "{body}");
    let (_, body) = context().await;
    assert_eq!(body["unreviewed"], 2, "another agent's review is not this one's: {body}");

    h.post_json("/v1/synthesis", json!({ "agent_id": "a", "project_id": "p" })).await;
    let (_, body) = context().await;
    assert_eq!(body["unreviewed"], 0, "{body}");
}

// ── conclusions and synthesis ───────────────────────────────────────────────

async fn save_id(h: &Harness, body: Value) -> i64 {
    let (status, saved) = h.post_json("/v1/memories", body).await;
    assert_eq!(status, 201, "{saved}");
    saved["memories"][0]["id"].as_i64().expect("id")
}

#[tokio::test]
async fn synthesis_over_http() {
    let h = Harness::start().await;
    let a = save_id(&h, json!({ "content": "the go service uses tabs", "agent_id": "a", "project_id": "p" })).await;
    let b = save_id(&h, json!({ "content": "the rust crate uses tabs", "agent_id": "a", "project_id": "p" })).await;
    let e = save_id(&h, json!({ "content": "deploys happen on fridays", "agent_id": "a", "project_id": "p" })).await;
    let c = save_id(
        &h,
        json!({
            "content": "the user indents with tabs in every language",
            "agent_id": "a",
            "project_id": "p",
            "memory_type": "user_preference",
            "derived_from": [b, a],
        }),
    )
    .await;

    let (status, body) = h.post_json("/v1/synthesis", json!({ "project_id": "p" })).await;

    assert_eq!(status, 200, "{body}");
    let pending: Vec<i64> = body["pending"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect();
    assert_eq!(pending, vec![e], "{body}");
    assert_eq!(body["pending_total"], 1);
    assert_eq!(body["conclusions"][0]["id"], c);
    assert_eq!(body["conclusions"][0]["derived_from"], json!([a, b]));
    let (_, item) = h.get_json(&format!("/v1/memories/memory/{c}")).await;
    assert_eq!(item["item"]["derived_from"], json!([a, b]), "{item}");
}

#[tokio::test]
async fn a_save_from_unknown_sources_is_refused() {
    let h = Harness::start().await;
    let a = save_id(&h, json!({ "content": "kept", "agent_id": "a" })).await;
    let d = save_id(&h, json!({ "content": "deleted", "agent_id": "a" })).await;
    let r = h.authed(h.http.delete(h.url(&format!("/v1/memories/memory/{d}")))).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);

    let (status, body) = h
        .post_json(
            "/v1/memories",
            json!({ "content": "a conclusion", "agent_id": "a", "derived_from": [a, d, 999999] }),
        )
        .await;

    assert_eq!(status, 400, "{body}");
    let message = body.to_string();
    assert!(message.contains(&d.to_string()) && message.contains("999999"), "{message}");
    let (_, list) = h.get_json("/v1/memories?kind=memory").await;
    assert_eq!(list["items"].as_array().map(Vec::len), Some(1), "nothing was saved: {list}");
}

#[tokio::test]
async fn an_agent_neither_reviews_nor_cites_another_agents_memories() {
    let h = Harness::start().await;
    let mine = save_id(&h, json!({ "content": "the go service uses tabs", "agent_id": "a", "project_id": "p" })).await;
    let theirs = save_id(&h, json!({ "content": "the deploy runs on fridays", "agent_id": "b", "project_id": "p" })).await;

    let (status, body) = h.post_json("/v1/synthesis", json!({ "agent_id": "a", "project_id": "p" })).await;
    assert_eq!(status, 200, "{body}");
    let pending: Vec<i64> = body["pending"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect();
    assert_eq!(pending, vec![mine], "{body}");

    let (status, body) = h
        .post_json(
            "/v1/memories",
            json!({ "content": "a conclusion", "agent_id": "a", "project_id": "p", "derived_from": [mine, theirs] }),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body.to_string().contains(&theirs.to_string()), "{body}");
}

#[tokio::test]
async fn a_misspelt_synthesis_field_is_refused() {
    let h = Harness::start().await;
    let (status, body) = h.post_json("/v1/synthesis", json!({ "agnet_id": "a" })).await;
    assert_eq!(status, 422, "{body}");
}

#[tokio::test]
async fn patch_importance_updates_then_404s_on_a_missing_row() {
    let h = Harness::start().await;
    h.post_json(
        "/v1/memories",
        json!({ "content": CONTENT, "agent_id": "agent-http" }),
    )
    .await;
    let items = h.wait_for_rows("memory", 1).await;
    let id = items[0]["id"].as_i64().expect("id");

    let r = h
        .authed(h.http.patch(h.url(&format!("/v1/memories/memory/{id}"))))
        .json(&json!({ "importance": 9.5 }))
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.expect("json");
    assert_eq!(body["updated"], 1);

    let items = h.wait_for_rows("memory", 1).await;
    assert_eq!(items[0]["importance"], 9.5);

    let r = h
        .authed(h.http.patch(h.url("/v1/memories/memory/999999")))
        .json(&json!({ "importance": 1.0 }))
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 404, "a row that changed nothing is not a success");
}

#[tokio::test]
async fn delete_removes_the_row_and_a_second_delete_is_404() {
    let h = Harness::start().await;
    h.post_json(
        "/v1/memories",
        json!({ "content": CONTENT, "agent_id": "agent-http" }),
    )
    .await;
    let items = h.wait_for_rows("memory", 1).await;
    let id = items[0]["id"].as_i64().expect("id");

    let r = h
        .authed(h.http.delete(h.url(&format!("/v1/memories/memory/{id}"))))
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.expect("json");
    assert_eq!(body["deleted"], 1);

    let (status, body) = h.get_json("/v1/memories?kind=memory").await;
    assert_eq!(status, 200);
    assert!(
        body["items"].as_array().expect("items").is_empty(),
        "a deleted row must disappear from the listing: {body}"
    );

    let r = h
        .authed(h.http.delete(h.url(&format!("/v1/memories/memory/{id}"))))
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 404);
}

/// Saves two memories and marks the second as disputing the first, the way
/// the contradiction detector leaves them. Returns `(old, new)`.
async fn seed_dispute(h: &Harness) -> (i64, i64) {
    for content in ["the API listens on port 8080", "the API listens on port 9090"] {
        h.post_json("/v1/memories", json!({ "content": content, "agent_id": "agent-http" })).await;
    }
    let items = h.wait_for_rows("memory", 2).await;
    let mut ids: Vec<i64> = items.iter().map(|i| i["id"].as_i64().expect("id")).collect();
    ids.sort();
    let (old, new) = (ids[0], ids[1]);
    h.ctx
        .runtime
        .store
        .conn
        .call(move |c| {
            c.execute(
                &format!("UPDATE memory SET status = 'disputed', disputed_against = '[{old}]' WHERE id = {new}"),
                [],
            )
        })
        .await
        .expect("mark the dispute");
    (old, new)
}

#[tokio::test]
async fn resolve_supersedes_the_other_row() {
    let h = Harness::start().await;
    let (old, new) = seed_dispute(&h).await;

    let (status, body) = h.post_json("/v1/disputes/resolve", json!({ "keep": new, "other": old })).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["keep"], json!({ "id": new, "status": "active", "superseded_by": null }));
    assert_eq!(body["other"], json!({ "id": old, "status": "superseded", "superseded_by": new }));
    let (_, listed) = h.get_json("/v1/memories?kind=memory&status=superseded").await;
    assert_eq!(listed["items"][0]["id"], old, "{listed}");
}

#[tokio::test]
async fn resolve_can_keep_both_rows() {
    let h = Harness::start().await;
    let (old, new) = seed_dispute(&h).await;

    let (status, body) = h
        .post_json("/v1/disputes/resolve", json!({ "keep": new, "other": old, "keep_both": true }))
        .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["keep"]["status"], "active");
    assert_eq!(body["other"]["status"], "active");
}

#[tokio::test]
async fn resolve_rejects_the_same_row() {
    let h = Harness::start().await;
    let (_, new) = seed_dispute(&h).await;

    let (status, body) = h.post_json("/v1/disputes/resolve", json!({ "keep": new, "other": new })).await;

    assert_eq!(status, 400, "{body}");
}

#[tokio::test]
async fn resolve_unknown_row_is_not_found() {
    let h = Harness::start().await;
    let (_, new) = seed_dispute(&h).await;

    let (status, body) = h.post_json("/v1/disputes/resolve", json!({ "keep": new, "other": 999_999 })).await;

    assert_eq!(status, 404, "{body}");
    assert!(body["error"].as_str().is_some_and(|e| e.contains("999999")), "{body}");
}

#[tokio::test]
async fn resolve_rows_without_a_dispute_conflict() {
    let h = Harness::start().await;
    let (old, new) = seed_dispute(&h).await;
    h.post_json("/v1/disputes/resolve", json!({ "keep": new, "other": old, "keep_both": true })).await;

    let (status, body) = h.post_json("/v1/disputes/resolve", json!({ "keep": new, "other": old })).await;

    assert_eq!(status, 409, "{body}");
}

#[tokio::test]
async fn delete_with_another_agent_is_404_and_keeps_the_row() {
    let h = Harness::start().await;
    h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "agent-http" })).await;
    let id = h.wait_for_rows("memory", 1).await[0]["id"].as_i64().expect("id");

    let r = h
        .authed(h.http.delete(h.url(&format!("/v1/memories/memory/{id}?agent=agent-other"))))
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 404, "another agent's row reads as missing");
    let (_, row) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    assert!(row["item"]["deleted_at"].is_null(), "{row}");

    let r = h
        .authed(h.http.delete(h.url(&format!("/v1/memories/memory/{id}?agent=agent-http"))))
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 200, "the owner can delete it");
}

#[tokio::test]
async fn patch_with_another_agent_is_404_and_keeps_the_importance() {
    let h = Harness::start().await;
    h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "agent-http" })).await;
    let item = h.wait_for_rows("memory", 1).await.remove(0);
    let id = item["id"].as_i64().expect("id");

    let r = h
        .authed(h.http.patch(h.url(&format!("/v1/memories/memory/{id}?agent=agent-other"))))
        .json(&json!({ "importance": 9.5 }))
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 404);
    let (_, row) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    assert_eq!(row["item"]["importance"], item["importance"], "{row}");

    let r = h
        .authed(h.http.patch(h.url(&format!("/v1/memories/memory/{id}?agent=agent-http"))))
        .json(&json!({ "importance": 9.5 }))
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn resolve_with_another_agent_is_404_and_changes_nothing() {
    let h = Harness::start().await;
    let (old, new) = seed_dispute(&h).await;

    let (status, body) = h
        .post_json("/v1/disputes/resolve", json!({ "keep": new, "other": old, "agent_id": "agent-other" }))
        .await;
    assert_eq!(status, 404, "{body}");
    let (_, row) = h.get_json(&format!("/v1/memories/memory/{new}")).await;
    assert_eq!(row["item"]["status"], "disputed", "{row}");

    let (status, body) = h
        .post_json("/v1/disputes/resolve", json!({ "keep": new, "other": old, "agent_id": "agent-http" }))
        .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn backup_endpoint_writes_a_manual_snapshot() {
    let h = Harness::start().await;
    h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "agent-http" })).await;
    h.wait_for_rows("memory", 1).await;

    let unauthorized = h.http.post(h.url("/v1/backup")).send().await.expect("request");
    assert_eq!(unauthorized.status(), 401);

    let (status, body) = h.post_json("/v1/backup", json!({})).await;
    assert_eq!(status, 200, "{body}");
    let path = std::path::PathBuf::from(body["path"].as_str().expect("path"));
    assert_eq!(path.parent(), Some(h.ctx.runtime.base_dir.join("backups").as_path()));
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.ends_with("-manual.db"), "{name}");
    let bytes = std::fs::read(&path).expect("the snapshot exists");
    assert!(bytes.starts_with(b"SQLite format 3\0"), "a whole SQLite database");
}

#[tokio::test]
async fn an_unknown_kind_is_a_400_everywhere_it_appears() {
    let h = Harness::start().await;

    let (status, body) = h.get_json("/v1/memories?kind=bogus").await;
    assert_eq!(status, 400);
    assert!(body["error"].as_str().is_some_and(|e| e.contains("bogus")), "{body}");

    let r = h
        .authed(h.http.delete(h.url("/v1/memories/bogus/1")))
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 400);

    let r = h
        .authed(h.http.patch(h.url("/v1/memories/bogus/1")))
        .json(&json!({ "importance": 1.0 }))
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 400);

    let (status, body) = h.get_json("/v1/memories/bogus/1").await;
    assert_eq!(status, 400);
    assert!(body["error"].as_str().is_some_and(|e| e.contains("bogus")), "{body}");
}

#[tokio::test]
async fn listing_without_a_kind_merges_every_table() {
    let h = Harness::start().await;
    h.post_json(
        "/v1/memories",
        json!({ "content": CONTENT, "agent_id": "agent-http" }),
    )
    .await;
    h.wait_for_rows("memory", 1).await;

    let (status, body) = h.get_json("/v1/memories").await;
    assert_eq!(status, 200);
    assert_eq!(body["items"].as_array().expect("items").len(), 1);
}

// ── Viewer reads: filters, text query ───────────────────────────────────────

#[tokio::test]
async fn listing_filters_by_agent_and_project() {
    let h = Harness::start().await;
    let saves = [
        ("alpha note about caching layers", "agent-a", "proj-1"),
        ("beta note about retry budgets", "agent-b", "proj-1"),
        ("gamma note about schema drift", "agent-a", "proj-2"),
    ];
    for (content, agent, project) in saves {
        h.post_json(
            "/v1/memories",
            json!({ "content": content, "agent_id": agent, "project_id": project }),
        )
        .await;
    }
    h.wait_for_rows("memory", 3).await;

    let (status, body) = h.get_json("/v1/memories?agent=agent-a&project=proj-1").await;
    assert_eq!(status, 200, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["summary"], "alpha note about caching layers");

    let (_, body) = h.get_json("/v1/memories?agent=agent-a").await;
    assert_eq!(body["items"].as_array().expect("items").len(), 2);
}

#[tokio::test]
async fn a_text_query_finds_a_saved_memory_without_the_embedder() {
    let h = Harness::start_serving_without_embedder().await;
    for content in ["la configuración del núcleo cambió", "an unrelated remark"] {
        h.post_json("/v1/memories", json!({ "content": content, "agent_id": "agent-http" }))
            .await;
    }
    h.wait_for_rows("memory", 2).await;

    let (status, body) = h.get_json("/v1/memories?q=configuracion%20nucleo").await;
    assert_eq!(status, 200, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["summary"], "la configuración del núcleo cambió");

    let (status, _) = h.get_json("/v1/memories?kind=episodic&q=configuracion").await;
    assert_eq!(status, 400, "episodic rows have no text index");
}

/// Episodic rows have no text index, so a text query that names no kind must
/// not list them beside the matches.
#[tokio::test]
async fn a_text_query_without_a_kind_lists_only_memory_rows() {
    use clawg_core::memory::store_insert::{CreatedByType, NewEpisodicTrajectoryRecord};

    let h = Harness::start().await;
    h.post_json(
        "/v1/memories",
        json!({ "content": "la configuración del núcleo cambió", "agent_id": "agent-http" }),
    )
    .await;
    h.wait_for_rows("memory", 1).await;

    let record = NewEpisodicTrajectoryRecord {
        scope: "agent",
        agent_id: Some("agent-http".to_string()),
        project_id: None,
        session_id: "sess-text-query".to_string(),
        task: "configuracion de un trabajo ajeno".to_string(),
        tool_calls_json: "[]".to_string(),
        errors_json: "[]".to_string(),
        resolution: None,
        success_score: 0.5,
        created_by_type: CreatedByType::System,
        created_by_ref: "agent-http".to_string(),
        importance: 5.0,
        created_at_ms: 1_000_000,
    };
    h.ctx
        .runtime
        .store
        .insert_episodic_trajectory(record, vec![0.5; 384])
        .await
        .expect("seed an episodic row");

    let (_, body) = h.get_json("/v1/memories").await;
    assert_eq!(body["items"].as_array().expect("items").len(), 2, "both tables are seeded");

    let (status, body) = h.get_json("/v1/memories?q=configuracion").await;
    assert_eq!(status, 200, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["kind"], "memory");
}

#[tokio::test]
async fn an_unknown_list_param_is_a_400() {
    let h = Harness::start().await;
    let (status, _) = h.get_json("/v1/memories?agnet=typo").await;
    assert_eq!(status, 400, "a misspelt filter must not be silently ignored");
}

// ── Viewer reads: one row, stats ────────────────────────────────────────────

#[tokio::test]
async fn show_returns_every_metadata_field_of_a_saved_memory() {
    let h = Harness::start().await;
    h.post_json(
        "/v1/memories",
        json!({
            "content": CONTENT,
            "agent_id": "agent-http",
            "project_id": "proj-show",
            "memory_type": "semantic_fact",
            "importance": 8.0
        }),
    )
    .await;
    let id = h.wait_for_rows("memory", 1).await[0]["id"].as_i64().expect("id");

    let (status, body) = h.get_json(&format!("/v1/memories/memory/{id}")).await;

    assert_eq!(status, 200, "{body}");
    let item = &body["item"];
    assert_eq!(item["id"], id);
    assert_eq!(item["kind"], "memory");
    assert_eq!(item["summary"], CONTENT, "the full content, not a preview");
    assert_eq!(item["source"], "explicit");
    assert_eq!(item["created_by_type"], "user");
    assert_eq!(item["agent_id"], "agent-http");
    assert_eq!(item["project_id"], "proj-show");
    assert_eq!(item["memory_type"], "semantic_fact");
    assert_eq!(item["importance"], 8.0);
    assert_eq!(item["status"], "active");
    assert!(item["deleted_at"].is_null());
    for field in [
        "created_at",
        "last_modified_at",
        "last_accessed_at",
        "update_count",
        "dispute_count",
        "created_by_ref",
        "tier",
        "access_count",
    ] {
        assert!(!item[field].is_null(), "{field} missing from {item}");
    }
}

#[tokio::test]
async fn show_missing_is_404() {
    let h = Harness::start().await;
    for path in ["/v1/memories/memory/999999", "/v1/memories/episodic/1"] {
        let (status, body) = h.get_json(path).await;
        assert_eq!(status, 404, "{path}");
        assert!(body["error"].is_string(), "{body}");
    }
}

/// Looking at a row must not feed the signal that promotes it.
#[tokio::test]
async fn show_is_not_an_access() {
    let h = Harness::start().await;
    h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "agent-http" })).await;
    let id = h.wait_for_rows("memory", 1).await[0]["id"].as_i64().expect("id");
    let path = format!("/v1/memories/memory/{id}");

    let (_, before) = h.get_json(&path).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    for _ in 0..3 {
        h.get_json(&path).await;
    }
    let (_, after) = h.get_json(&path).await;

    assert_eq!(after["item"]["access_count"], before["item"]["access_count"]);
    assert_eq!(after["item"]["last_accessed_at"], before["item"]["last_accessed_at"]);
}

#[tokio::test]
async fn stats_counts_what_was_saved() {
    let h = Harness::start().await;
    let saves = [
        ("alpha note about caching layers", "agent-a", "proj-1"),
        ("beta note about retry budgets", "agent-b", "proj-1"),
        ("gamma note about schema drift", "agent-a", "proj-2"),
    ];
    for (content, agent, project) in saves {
        h.post_json(
            "/v1/memories",
            json!({ "content": content, "agent_id": agent, "project_id": project }),
        )
        .await;
    }
    h.wait_for_rows("memory", 3).await;

    let (status, body) = h.get_json("/v1/stats").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["totals"]["memory"], 3);
    assert_eq!(body["totals"]["deleted"], 0);
    assert_eq!(body["by_agent"][0], json!({ "key": "agent-a", "count": 2 }));
    assert_eq!(body["by_agent"][1], json!({ "key": "agent-b", "count": 1 }));
    assert!(body["newest_created_at"].is_i64(), "{body}");

    let (status, body) = h.get_json("/v1/stats?agent=agent-a&project=proj-1").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["totals"]["memory"], 1);

    let (status, _) = h.get_json("/v1/stats?agnet=typo").await;
    assert_eq!(status, 400, "an unknown parameter must not be ignored");
}

#[tokio::test]
async fn stats_requires_the_token() {
    let h = Harness::start().await;
    let r = h.http.get(h.url("/v1/stats")).send().await.expect("request");
    assert_eq!(r.status(), 401);
}

// ── Shutdown ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn shutdown_accepts_and_signals_the_serve_loop() {
    let h = Harness::start().await;
    let mut rx = h.ctx.shutdown.subscribe();
    assert!(!*rx.borrow_and_update());

    let (status, body) = h.post_json("/v1/shutdown", json!({})).await;
    assert_eq!(status, 202);
    assert_eq!(body["status"], "shutting_down");

    rx.changed().await.expect("the shutdown signal must fire");
    assert!(*rx.borrow());
}

#[tokio::test]
async fn shutdown_requires_the_token() {
    let h = Harness::start().await;
    let r = h.http.post(h.url("/v1/shutdown")).send().await.expect("request");
    assert_eq!(r.status(), 401, "anyone on loopback must not be able to kill the daemon");
}

// ── /v1/projects ────────────────────────────────────────────────────────────

#[tokio::test]
async fn registering_a_root_moves_the_rows_of_its_legacy_id() {
    let h = Harness::start().await;
    let (status, body) = h
        .post_json(
            "/v1/memories",
            json!({ "content": "saved from a subdirectory", "agent_id": "a", "project_id": "src-1a2b" }),
        )
        .await;
    assert_eq!(status, 201, "{body}");

    let root = "/work/repo";
    let (status, body) =
        h.post_json("/v1/projects", json!({ "root_path": root, "legacy_ids": ["src-1a2b"] })).await;

    assert_eq!(status, 200, "{body}");
    let id = clawg_core::memory::derive_project_id(std::path::Path::new(root));
    assert_eq!(body["project_id"], id.as_str());
    assert_eq!(body["rekeyed"], 1);
    let (_, list) = h.get_json(&format!("/v1/memories?kind=memory&project={id}")).await;
    assert_eq!(list["items"].as_array().map(Vec::len), Some(1), "{list}");
}

#[tokio::test]
async fn registering_with_a_malformed_legacy_id_is_a_bad_request() {
    let h = Harness::start().await;
    let (status, body) =
        h.post_json("/v1/projects", json!({ "root_path": "/work/repo", "legacy_ids": ["nope"] })).await;
    assert_eq!(status, 400, "{body}");
}

#[tokio::test]
async fn registering_a_root_whose_id_is_taken_by_another_is_a_conflict() {
    let h = Harness::start().await;
    let root = "/work/repo";
    let id = clawg_core::memory::derive_project_id(std::path::Path::new(root));
    h.ctx
        .runtime
        .store
        .conn
        .call(move |c| {
            c.execute("INSERT INTO project (id, root_path, created_at) VALUES (?1, '/elsewhere/repo', 0)", [id])
        })
        .await
        .expect("seed a colliding project");

    let (status, body) = h.post_json("/v1/projects", json!({ "root_path": root })).await;
    assert_eq!(status, 409, "{body}");
}

#[tokio::test]
async fn registering_needs_the_token() {
    let h = Harness::start().await;
    let status = h
        .http
        .post(h.url("/v1/projects"))
        .json(&json!({ "root_path": "/work/repo" }))
        .send()
        .await
        .expect("request")
        .status();
    assert_eq!(status.as_u16(), 401);
}

// ── Project documentation ───────────────────────────────────────────────────

/// Writes `docs` under a fresh directory and records it as a project, without
/// the background refresh a registration starts, so the test decides when
/// the index is built.
async fn project_with_docs(h: &Harness, docs: &[(&str, &str)]) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (path, text) in docs {
        let file = dir.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let id = clawg_core::memory::derive_project_id(dir.path());
    let (row_id, root) = (id.clone(), dir.path().display().to_string());
    h.ctx
        .runtime
        .store
        .conn
        .call(move |c| c.execute("INSERT INTO project (id, root_path, created_at) VALUES (?1, ?2, 0)", [row_id, root]))
        .await
        .unwrap();
    (dir, id)
}

#[tokio::test]
async fn docs_search_indexes_on_first_use_and_answers() {
    let h = Harness::start().await;
    let (_dir, id) = project_with_docs(
        &h,
        &[
            ("README.md", "# Claw\nLocal memory for agents.\n## Profiles\nThe dev profile lives in its own directory.\n"),
            ("docs/ops.md", "# Ops\nStop the daemon with clawg stop.\n"),
        ],
    )
    .await;

    let (status, body) =
        h.post_json("/v1/projects/docs/search", json!({ "project_id": id, "query": "Local memory for agents" })).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["docs"], 2);
    assert_eq!(body["indexing"], false);
    assert!(body.get("warning").is_none(), "{body}");
    let hits = body["hits"].as_array().expect("hits");
    assert_eq!(hits.len(), 3, "{body}");
    assert_eq!(hits[0]["path"], "README.md");
    assert_eq!(hits[0]["breadcrumb"], "README.md > Claw");
    assert_eq!(hits[0]["text"], "Local memory for agents.");
}

#[tokio::test]
async fn docs_search_takes_at_most_the_limit() {
    let h = Harness::start().await;
    let (_dir, id) = project_with_docs(&h, &[("a.md", "# A\na\n# B\nb\n# C\nc\n")]).await;

    let (status, body) =
        h.post_json("/v1/projects/docs/search", json!({ "project_id": id, "query": "a", "limit": 2 })).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["hits"].as_array().map(Vec::len), Some(2));
}

#[tokio::test]
async fn docs_search_of_an_unregistered_project_is_404() {
    let h = Harness::start().await;
    let (status, body) =
        h.post_json("/v1/projects/docs/search", json!({ "project_id": "nowhere-0000", "query": "x" })).await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn docs_search_with_an_empty_query_is_a_bad_request() {
    let h = Harness::start().await;
    let (_dir, id) = project_with_docs(&h, &[("a.md", "# A\na\n")]).await;
    let (status, body) = h.post_json("/v1/projects/docs/search", json!({ "project_id": id, "query": "  " })).await;
    assert_eq!(status, 400, "{body}");
}

#[tokio::test]
async fn docs_search_waits_for_the_embedder_like_any_search() {
    let h = Harness::start_without_embedder().await;
    let (_dir, id) = project_with_docs(&h, &[("a.md", "# A\na\n")]).await;
    let (status, body) = h.post_json("/v1/projects/docs/search", json!({ "project_id": id, "query": "a" })).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"], "embedder_not_ready");
}

#[tokio::test]
async fn docs_search_needs_a_token() {
    let h = Harness::start().await;
    let r = h
        .http
        .post(h.url("/v1/projects/docs/search"))
        .json(&json!({ "project_id": "x", "query": "y" }))
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn a_root_that_vanished_still_answers_from_its_index() {
    let h = Harness::start().await;
    let (dir, id) = project_with_docs(&h, &[("a.md", "# A\nkept passage\n")]).await;
    // Indexed outside the daemon's own bookkeeping, so the daemon still finds
    // the project due for a refresh at the search below.
    let elsewhere = Arc::new(clawg_core::memory::project_docs::DocIndex::default());
    elsewhere.try_refresh(&h.ctx.runtime.store, &FakeEmbedder::default(), None, &id).await.unwrap();
    drop(dir);

    let (status, body) = h.post_json("/v1/projects/docs/search", json!({ "project_id": id, "query": "kept" })).await;

    assert_eq!(status, 200, "{body}");
    assert!(body["warning"].as_str().is_some_and(|w| w.contains("not a directory")), "{body}");
    assert_eq!(body["hits"][0]["text"], "kept passage");
    assert_eq!(body["docs"], 1);
}

#[tokio::test]
async fn registering_a_root_indexes_its_docs_in_the_background() {
    let h = Harness::start().await;
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("README.md"), "# Readme\nindexed in the background\n").unwrap();

    let (status, body) =
        h.post_json("/v1/projects", json!({ "root_path": dir.path().display().to_string() })).await;
    assert_eq!(status, 200, "{body}");
    let id = body["project_id"].as_str().expect("project_id").to_string();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let docs = clawg_core::memory::project_docs::doc_count(&h.ctx.runtime.store, &id).await.unwrap();
        if docs == 1 {
            break;
        }
        assert!(Instant::now() < deadline, "the docs were never indexed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
