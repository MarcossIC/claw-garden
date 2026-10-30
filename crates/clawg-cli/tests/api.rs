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

use clawg_cli::daemon::api::{router, AppCtx, MAX_CONTENT_BYTES, REFRESH_GRACE, SAVE_CONFIRM_TIMEOUT};
use clawg_core::memory::bootstrap::EmbedderStatus;
use clawg_core::memory::embedder::{EmbedError, EmbedRole, Embedder, FakeEmbedder};
use clawg_core::memory::idle_embedder::{IdleEmbedder, ReloadFn};
use std::sync::atomic::{AtomicUsize, Ordering};
use clawg_core::memory::worker::{IngestFailure, IngestReport, IngestRequest};
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
    /// The model was let go for being idle; loading it again works or fails.
    Unloaded { reload_fails: bool },
    /// The embedder is ready and something takes each save off the queue and
    /// answers that the embedder could not take it, as the worker does before
    /// it tries the save again.
    AnsweringUnavailable,
    /// Texts holding one of these keys embed to its vector, so a test decides
    /// the cosines; any other text embeds as the fake model would.
    Fixed(Vec<(&'static str, Vec<f32>)>),
    /// Queries embed as the fake model does; passages wait for a gate to
    /// open, and may then fail.
    Docs(DocsEmbedder),
}

/// `FakeEmbedder` under the same model id, except that a passage waits until
/// the gate reads true and then fails if `fail`, so a test holds or breaks a
/// docs refresh without touching searches.
struct DocsEmbedder {
    open: tokio::sync::watch::Receiver<bool>,
    fail: bool,
}

/// The embedder and the sender of its gate.
fn docs_embedder(open: bool, fail: bool) -> (DocsEmbedder, tokio::sync::watch::Sender<bool>) {
    let (gate, open) = tokio::sync::watch::channel(open);
    (DocsEmbedder { open, fail }, gate)
}

#[async_trait::async_trait]
impl Embedder for DocsEmbedder {
    async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
        if role == EmbedRole::Passage {
            let _ = self.open.clone().wait_for(|open| *open).await;
            if self.fail {
                return Err(EmbedError::InferenceFailed("passages refused".into()));
            }
        }
        FakeEmbedder::default().embed_with_role(text, role).await
    }
    fn dimensions(&self) -> usize {
        384
    }
    fn model_id(&self) -> &str {
        "fake-embedder"
    }
    fn query_prefix(&self) -> &str {
        ""
    }
    fn passage_prefix(&self) -> &str {
        ""
    }
}

struct FixedEmbedder(Vec<(&'static str, Vec<f32>)>);

#[async_trait::async_trait]
impl Embedder for FixedEmbedder {
    async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
        match self.0.iter().find(|(key, _)| text.contains(key)) {
            Some((_, v)) => Ok(v.clone()),
            None => FakeEmbedder::default().embed_with_role(text, role).await,
        }
    }
    fn dimensions(&self) -> usize {
        384
    }
    fn model_id(&self) -> &str {
        "fixed"
    }
    fn query_prefix(&self) -> &str {
        ""
    }
    fn passage_prefix(&self) -> &str {
        ""
    }
}

/// A unit vector at `cosine` from the first axis, leaning on axis `axis` so
/// that two vectors built on different axes are no closer than they must be.
fn at_cosine(cosine: f32, axis: usize) -> Vec<f32> {
    let mut v = vec![0.0; 384];
    v[0] = cosine;
    v[axis] = (1.0 - cosine * cosine).sqrt();
    v
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
    /// How many times an unloaded model was loaded again.
    reloads: Arc<AtomicUsize>,
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

    /// Daemon whose model was let go for being idle.
    async fn start_unloaded(reload_fails: bool) -> Self {
        Self::start_inner(EmbedderState::Unloaded { reload_fails }).await
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
        let reloads = Arc::new(AtomicUsize::new(0));
        let (runtime, ingest_rx) = match embedder {
            EmbedderState::Unloaded { reload_fails } => {
                let runtime = with_empty_embedder(runtime, EmbedderStatus::Ready);
                let reload: ReloadFn = {
                    let reloads = reloads.clone();
                    Arc::new(move || {
                        reloads.fetch_add(1, Ordering::SeqCst);
                        Box::pin(async move {
                            if reload_fails {
                                Err("the model files are gone".to_string())
                            } else {
                                Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>)
                            }
                        })
                    })
                };
                let idle = IdleEmbedder::new(
                    Arc::new(FakeEmbedder::default()),
                    reload,
                    Some(Duration::from_secs(600)),
                    None,
                    &tokio::runtime::Handle::current(),
                );
                idle.unload().await;
                runtime.embedder.set(Arc::new(idle) as Arc<dyn Embedder>).ok();
                tokio::spawn(runtime.ingest_worker(rx, tokio::sync::watch::channel(None).1));
                (runtime, None)
            }
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
            EmbedderState::Fixed(vectors) => {
                let runtime = with_empty_embedder(runtime, EmbedderStatus::Ready);
                runtime.embedder.set(Arc::new(FixedEmbedder(vectors)) as Arc<dyn Embedder>).ok();
                tokio::spawn(runtime.ingest_worker(rx, tokio::sync::watch::channel(None).1));
                (runtime, None)
            }
            EmbedderState::Docs(embedder) => {
                let runtime = with_empty_embedder(runtime, EmbedderStatus::Ready);
                runtime.embedder.set(Arc::new(embedder) as Arc<dyn Embedder>).ok();
                tokio::spawn(runtime.ingest_worker(rx, tokio::sync::watch::channel(None).1));
                (runtime, None)
            }
            EmbedderState::DroppingReplies => {
                let mut rx = rx;
                tokio::spawn(async move { while rx.recv().await.is_some() {} });
                (runtime, None)
            }
            EmbedderState::AnsweringUnavailable => {
                let mut rx = rx;
                tokio::spawn(async move {
                    while let Some(request) = rx.recv().await {
                        let report = IngestReport { failure: Some(IngestFailure::EmbedderUnavailable), ..Default::default() };
                        if let Some(reply) = request.reply {
                            let _ = reply.send(report);
                        }
                    }
                });
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
            sessions: Default::default(),
        });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral loopback port");
        let addr = listener.local_addr().expect("local_addr");
        let app = router(ctx.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self { addr, token, http: reqwest::Client::new(), ctx, reloads, _ingest_rx: ingest_rx, _dir: dir }
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

#[tokio::test]
async fn health_reports_a_failed_save_with_its_operation_id() {
    let h = Harness::start().await;
    h.ctx
        .runtime
        .store
        .conn
        .call(|c| {
            c.execute(
                "INSERT INTO ingest_outbox (operation_id, agent_id, content, event, state, attempts, last_error, created_at, updated_at) \
                 VALUES ('op-1', 'claude-code', 'x', '{}', 'failed', 3, 'the daemon stopped 3 times while ingesting this save', 1, 1)",
                [],
            )
        })
        .await
        .expect("seed");

    let body: Value = h.http.get(h.url("/health")).send().await.unwrap().json().await.unwrap();
    assert_eq!(body["ingest"]["pending"], 0, "got {body}");
    assert_eq!(body["ingest"]["failed"], 1, "got {body}");
    let problem = &body["ingest"]["problems"][0];
    assert_eq!(problem["operation_id"], "op-1");
    assert_eq!(problem["agent_id"], "claude-code");
    assert!(problem["error"].as_str().is_some_and(|e| e.contains("stopped 3 times")), "got {body}");
    assert!(problem.get("content").is_none(), "health must not carry the text: {body}");
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

// ── Host and Origin ─────────────────────────────────────────────────────────

/// What a DNS-rebinding page sends: the attacker's name, resolved to loopback.
#[tokio::test]
async fn a_foreign_host_is_forbidden_on_health_and_v1_with_or_without_a_token() {
    let h = Harness::start().await;
    let foreign = format!("evil.example:{}", h.addr.port());
    for (path, token) in [("/health", false), ("/health", true), ("/v1/stats", false), ("/v1/stats", true)] {
        let mut req = h.http.get(h.url(path)).header("host", &foreign);
        if token {
            req = req.bearer_auth(&h.token);
        }
        let r = req.send().await.expect("request");
        assert_eq!(r.status(), 403, "{path} with token={token}");
        let body: Value = r.json().await.expect("json");
        assert_eq!(body, json!({ "error": "forbidden" }), "{path} with token={token}");
    }
}

/// A browser sends `Origin` on every write, cross-site or not; no client of
/// the daemon sends one.
#[tokio::test]
async fn an_origin_is_forbidden_even_with_the_token() {
    let h = Harness::start().await;
    h.authed(h.http.post(h.url("/v1/memories")))
        .json(&json!({ "content": "Kept in the store: the origin test row.", "agent_id": "a" }))
        .send()
        .await
        .expect("save");

    let r = h
        .authed(h.http.post(h.url("/v1/search")))
        .header("origin", "http://evil.example")
        .json(&json!({ "query": "origin test row", "agent_id": "a" }))
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 403);
    let body: Value = r.json().await.expect("json");
    assert_eq!(body, json!({ "error": "forbidden" }));

    let r = h.http.get(h.url("/health")).header("origin", "null").send().await.expect("request");
    assert_eq!(r.status(), 403);
}

/// One answer for both reasons, as for the token: telling them apart would
/// tell a page which half to change.
#[tokio::test]
async fn both_refusals_answer_the_same_body() {
    let h = Harness::start().await;
    let by_host = h.http.get(h.url("/health")).header("host", "evil.example").send().await.expect("request");
    let by_origin = h
        .http
        .get(h.url("/health"))
        .header("origin", "http://evil.example")
        .send()
        .await
        .expect("request");
    assert_eq!(by_host.status(), 403);
    assert_eq!(by_origin.status(), 403);
    assert_eq!(by_host.text().await.expect("body"), by_origin.text().await.expect("body"));
}

#[tokio::test]
async fn a_daemon_client_passes_the_guard() {
    use clawg_cli::client::{DaemonClient, SaveArgs, SearchArgs};
    let h = Harness::start().await;
    let client = DaemonClient::new(format!("http://{}", h.addr), &h.token, h._dir.path().to_path_buf());

    assert_eq!(client.health().await.expect("health").status, "ok");
    client
        .save(SaveArgs {
            content: "The guard lets the daemon client through.".into(),
            agent_id: "a".into(),
            project_id: None,
            memory_type: None,
            importance: None,
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
        })
        .await
        .expect("save");
    client
        .search(SearchArgs {
            query: "guard daemon client".into(),
            agent_id: "a".into(),
            project_id: None,
            limit: None,
            lang: None,
        })
        .await
        .expect("search");
    client.shutdown().await.expect("shutdown");
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
async fn a_save_with_an_unknown_memory_type_is_refused_before_the_outbox() {
    let h = Harness::start_without_worker().await;
    let (status, body) = h
        .post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a", "memory_type": "preference" }))
        .await;

    assert_eq!(status, 400, "{body}");
    let message = body["error"].as_str().expect("error");
    assert!(message.contains("\"preference\""), "{message}");
    for t in ["semantic_fact", "user_preference", "architectural_decision", "session_summary", "code_block", "chat_message", "tool_error"] {
        assert!(message.contains(t), "{t} missing from {message}");
    }
    assert!(outbox(&h).await.is_empty());
}

#[tokio::test]
async fn save_and_patch_accept_the_same_memory_types() {
    let h = Harness::start().await;
    let id = save_id(&h, json!({ "content": CONTENT, "agent_id": "a" })).await;
    let types = ["semantic_fact", "user_preference", "architectural_decision", "session_summary", "code_block", "chat_message", "tool_error"];

    for (n, raw) in ["preference", "Semantic_Fact", ""].into_iter().chain(types).enumerate() {
        let (saved, body) = h
            .post_json("/v1/memories", json!({ "content": format!("note {n} about the build"), "agent_id": "a", "memory_type": raw }))
            .await;
        let patched = h
            .authed(h.http.patch(h.url(&format!("/v1/memories/memory/{id}"))))
            .json(&json!({ "memory_type": raw, "user_answered": true }))
            .send()
            .await
            .expect("request")
            .status()
            .as_u16();
        let accepted = types.contains(&raw);
        assert_eq!(saved == 201, accepted, "save of {raw:?}: {saved} {body}");
        assert_eq!(patched == 200, accepted, "patch of {raw:?}: {patched}");
        if accepted {
            assert_eq!(body["memories"][0]["outcome"], "inserted", "{body}");
        } else {
            assert_eq!((saved, patched), (400, 400), "{raw:?}");
        }
    }
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
    // Distinct words, long enough for fifteen pieces of the fallback chunker.
    let content = (0..600).map(|i| format!("w{i:05}")).collect::<Vec<_>>().join(" ");
    let (status, body) = h.post_json("/v1/memories", json!({ "content": content, "agent_id": "a" })).await;

    assert_eq!(status, 201, "{body}");
    let memories = body["memories"].as_array().expect("memories");
    assert_eq!(memories.len(), 15, "{body}");
    assert!(memories.iter().all(|m| m["outcome"] == "inserted"));
    let mut ids: Vec<i64> = memories.iter().map(|m| m["id"].as_i64().unwrap()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 15, "each chunk has its own row: {body}");
}

#[tokio::test]
async fn get_save_returns_the_whole_text() {
    let h = Harness::start().await;
    let content = (0..600).map(|i| format!("w{i:05}")).collect::<Vec<_>>().join(" ");
    let (_, body) = h.post_json("/v1/memories", json!({ "content": content, "agent_id": "a" })).await;
    let ids: Vec<i64> = body["memories"].as_array().expect("memories").iter().map(|m| m["id"].as_i64().unwrap()).collect();

    let (status, save) = h.get_json(&format!("/v1/memories/memory/{}/save?agent=a", ids[1])).await;

    assert_eq!(status, 200, "{save}");
    assert_eq!(save["id"], ids[1]);
    assert_eq!(save["piece"], 2);
    assert_eq!(save["pieces"], 15);
    assert_eq!(save["ids"], json!(ids));
    assert_eq!(save["missing"], json!([]));
    assert_eq!(save["text"], content);
}

#[tokio::test]
async fn get_save_of_another_agent_reads_whole_unless_hidden() {
    let h = Harness::start().await;
    let (_, body) = h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "agent-http" })).await;
    let id = body["memories"][0]["id"].as_i64().expect("id");

    let (status, save) = h.get_json(&format!("/v1/memories/memory/{id}/save?agent=agent-other")).await;
    assert_eq!(status, 200, "another agent reads it whole: {save}");
    assert_eq!((save["pieces"].as_i64(), save["text"].as_str()), (Some(1), Some(CONTENT)));
    let (status, _) = h.get_json("/v1/memories/memory/999999/save?agent=agent-http").await;
    assert_eq!(status, 404);

    write_policy(&h, HIDE_AUTHOR);
    let (status, _) = h.get_json(&format!("/v1/memories/memory/{id}/save?agent=agent-other")).await;
    assert_eq!(status, 404, "a hidden author's save reads as missing");
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

// ── The outbox: an accepted save is recorded before the answer ──────────────

/// `(state, content, last_error)` of every outbox row, oldest first.
async fn outbox(h: &Harness) -> Vec<(String, String, Option<String>)> {
    h.ctx
        .runtime
        .store
        .conn
        .call(|c| {
            let mut stmt = c.prepare("SELECT state, content, last_error FROM ingest_outbox ORDER BY id")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect::<Result<Vec<_>, _>>()
        })
        .await
        .expect("outbox")
}

#[tokio::test]
async fn a_save_accepted_while_the_model_loads_is_in_the_outbox_before_the_answer() {
    let h = Harness::start_without_embedder().await;
    let (status, body) = h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a" })).await;

    assert_eq!(status, 202, "{body}");
    assert!(body["operation_id"].as_str().is_some_and(|id| !id.is_empty()), "{body}");
    assert_eq!(outbox(&h).await, vec![("pending".into(), CONTENT.into(), None)]);
}

#[tokio::test]
async fn an_unconfirmed_save_is_in_the_outbox() {
    let h = Harness::start_without_worker().await;
    let (status, body) = h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a" })).await;

    assert_eq!((status, body["reason"].as_str()), (202, Some("unconfirmed")), "{body}");
    assert_eq!(outbox(&h).await, vec![("pending".into(), CONTENT.into(), None)]);
}

#[tokio::test]
async fn a_save_into_a_full_queue_leaves_no_outbox_row() {
    let h = Harness::start_loading_with_queue(2).await;
    for n in 1..=3 {
        h.post_json("/v1/memories", json!({ "content": format!("save {n}"), "agent_id": "a" })).await;
    }

    let contents: Vec<String> = outbox(&h).await.into_iter().map(|(_, c, _)| c).collect();
    assert_eq!(contents, vec!["save 1", "save 2"]);
}

#[tokio::test]
async fn a_confirmed_save_leaves_no_outbox_row() {
    let h = Harness::start().await;
    let long = (0..600).map(|i| format!("w{i:05}")).collect::<Vec<_>>().join(" ");
    for content in [CONTENT.to_string(), long] {
        let (status, body) = h.post_json("/v1/memories", json!({ "content": content, "agent_id": "a" })).await;
        assert_eq!(status, 201, "{body}");
    }

    assert!(outbox(&h).await.is_empty());
}

#[tokio::test]
async fn a_failure_the_caller_was_told_leaves_no_outbox_row() {
    let h = Harness::start_with_failing_embedder().await;
    let (status, body) = h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a" })).await;

    assert_eq!(status, 500, "{body}");
    assert!(outbox(&h).await.is_empty());
}

#[tokio::test]
async fn a_partial_save_answers_202_and_keeps_its_row() {
    // Only the last pieces hold this word, and the store refuses a vector that
    // is not finite, so the pieces before them are stored and these are not.
    let h = Harness::start_inner(EmbedderState::Fixed(vec![("w00590", vec![f32::NAN; 384])])).await;
    let content = (0..600).map(|i| format!("w{i:05}")).collect::<Vec<_>>().join(" ");

    let (status, body) = h.post_json("/v1/memories", json!({ "content": content, "agent_id": "a" })).await;

    assert_eq!((status, body["status"].as_str(), body["reason"].as_str()), (202, Some("queued"), Some("partially_saved")), "{body}");
    let stored: Vec<i64> = body["memories"].as_array().expect("memories").iter().map(|m| m["id"].as_i64().unwrap()).collect();
    assert!(!stored.is_empty() && stored.len() < 15, "{body}");
    assert!(body["detail"].as_str().is_some_and(|d| d.starts_with("store insert failed")), "{body}");
    assert!(body["operation_id"].as_str().is_some(), "{body}");
    let rows = outbox(&h).await;
    assert_eq!(rows.len(), 1, "the row keeps what is left of the save");
    assert_eq!(rows[0].0, "pending");
    let progress: String = h
        .ctx
        .runtime
        .store
        .conn
        .call(|c| c.query_row("SELECT progress FROM ingest_outbox", [], |r| r.get(0)))
        .await
        .expect("progress");
    let noted: Vec<i64> = serde_json::from_str::<Vec<Value>>(&progress)
        .expect("progress parses")
        .iter()
        .map(|p| p["id"].as_i64().unwrap())
        .collect();
    assert_eq!(noted, stored, "the next start resumes after the stored pieces");
}

/// Writes a save the daemon gave up on, as `agent`, with the progress of the
/// pieces it stored.
async fn seed_failed(h: &Harness, operation_id: &str, agent: &str, content: &str, progress: Value) {
    let event = json!({
        "agent_id": agent, "project_id": null, "role": "user", "content": "", "is_error": null,
        "source": "explicit", "timestamp_ms": 1, "memory_type": "semantic_fact",
    })
    .to_string();
    let (op, agent, content, progress) = (operation_id.to_string(), agent.to_string(), content.to_string(), progress.to_string());
    h.ctx
        .runtime
        .store
        .conn
        .call(move |c| {
            c.execute(
                "INSERT INTO ingest_outbox (operation_id, agent_id, content, event, state, attempts, last_error, progress, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, 'failed', 3, 'store insert failed: disk full', ?5, 1, 1)",
                rusqlite::params![op, agent, content, event, progress],
            )
        })
        .await
        .expect("seed");
}

#[tokio::test]
async fn a_failed_save_is_retried_by_its_operation_id() {
    let h = Harness::start().await;
    seed_failed(&h, "op-1", "a", "the release train leaves on tuesday", json!([])).await;

    let (status, body) = h.post_json("/v1/outbox/retry", json!({ "operation_id": "op-1" })).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "operation_id": "op-1", "agent_id": "a", "queued": true }));
    let rows = h.wait_for_rows("memory", 1).await;
    assert_eq!(rows[0]["summary"], "the release train leaves on tuesday");
    for _ in 0..40 {
        if outbox(&h).await.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(outbox(&h).await.is_empty());

    let (status, body) = h.post_json("/v1/outbox/retry", json!({ "operation_id": "op-1" })).await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn discarding_a_failed_save_deletes_the_pieces_it_inserted() {
    let h = Harness::start().await;
    let (_, saved) = h.post_json("/v1/memories", json!({ "content": "the first piece of a long save", "agent_id": "a" })).await;
    let piece = saved["memories"][0]["id"].as_i64().expect("an id");
    let progress = json!([{ "position": 0, "hash": "h0", "id": piece, "inserted": true, "context": "" }]);
    seed_failed(&h, "op-1", "a", "the first piece of a long save, and the rest", progress).await;

    let (_, health) = h.get_json("/health").await;
    assert_eq!(health["ingest"]["problems"][0]["pieces"], json!([piece]), "{health}");
    let (status, pending) = h.post_json("/v1/outbox/discard", json!({ "operation_id": "op-2" })).await;
    assert_eq!(status, 404, "{pending}");
    let (status, body) = h.post_json("/v1/outbox/discard", json!({ "operation_id": "op-1", "agent_id": "a" })).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "operation_id": "op-1", "agent_id": "a", "deleted": [piece] }));
    assert!(outbox(&h).await.is_empty());
    let (_, deleted) = h.get_json("/v1/memories?deleted=true").await;
    assert_eq!(deleted["items"][0]["id"], piece, "{deleted}");
    let (status, again) =
        h.post_json("/v1/memories", json!({ "content": "saved again", "agent_id": "a", "operation_id": "op-1" })).await;
    assert_eq!(status, 201, "the operation id is free again: {again}");
}

#[tokio::test]
async fn a_pending_or_shared_operation_id_is_refused() {
    let h = Harness::start_loading_with_queue(4).await;
    h.post_json("/v1/memories", json!({ "content": "waiting for the model", "agent_id": "a", "operation_id": "op-1" })).await;
    seed_failed(&h, "op-2", "a", "x", json!([])).await;
    seed_failed(&h, "op-2", "b", "y", json!([])).await;

    let (status, pending) = h.post_json("/v1/outbox/discard", json!({ "operation_id": "op-1" })).await;
    assert_eq!(status, 409, "{pending}");
    assert!(pending["error"].as_str().is_some_and(|e| e.contains("still pending")), "{pending}");
    let (status, shared) = h.post_json("/v1/outbox/retry", json!({ "operation_id": "op-2" })).await;
    assert_eq!(status, 409, "{shared}");
    assert!(shared["error"].as_str().is_some_and(|e| e.contains("a, b")), "{shared}");
    let (status, bad) = h.post_json("/v1/outbox/retry", json!({ "operation_id": "has space" })).await;
    assert_eq!(status, 400, "{bad}");
}

#[tokio::test]
async fn a_save_answers_with_an_operation_id() {
    let h = Harness::start().await;
    let (_, made_up) = h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a" })).await;
    let (status, given) = h
        .post_json("/v1/memories", json!({ "content": "another fact", "agent_id": "a", "operation_id": "hook-1" }))
        .await;

    let id = made_up["operation_id"].as_str().expect("an operation id on a 201");
    assert_eq!((id.len(), id.chars().nth(14)), (36, Some('4')), "a UUID v4: {made_up}");
    assert_eq!((status, given["operation_id"].as_str()), (201, Some("hook-1")), "{given}");
    let (status, bad) = h
        .post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a", "operation_id": "has space" }))
        .await;
    assert_eq!(status, 400, "{bad}");
}

#[tokio::test]
async fn an_operation_id_sent_twice_keeps_one_row() {
    let h = Harness::start_loading_with_queue(4).await;
    let save = || h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a", "operation_id": "op-1" }));

    let (_, first) = save().await;
    let (status, again) = save().await;

    assert_eq!(first["reason"], "embedder_loading");
    assert_eq!((status, again["reason"].as_str(), again["operation_id"].as_str()), (202, Some("already_queued"), Some("op-1")));
    assert_eq!(outbox(&h).await.len(), 1);
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
async fn a_save_over_the_content_limit_is_refused_before_the_outbox() {
    let h = Harness::start_without_worker().await;
    let (status, body) = h
        .post_json("/v1/memories", json!({ "content": "x".repeat(MAX_CONTENT_BYTES + 1), "agent_id": "a" }))
        .await;

    assert_eq!(status, 400, "{body}");
    let message = body["error"].as_str().expect("error");
    assert!(message.starts_with(&format!("content is {} bytes; at most {MAX_CONTENT_BYTES}", MAX_CONTENT_BYTES + 1)), "{message}");
    assert!(outbox(&h).await.is_empty());
}

#[tokio::test]
async fn a_save_at_the_content_limit_is_stored() {
    let h = Harness::start().await;
    let words: Vec<String> = (0..MAX_CONTENT_BYTES / 7).map(|i| format!("w{i:05}")).collect();
    let mut content = format!("Notes on the ledger crate. {}", words.join(" "));
    content.truncate(MAX_CONTENT_BYTES);

    let (status, body) = h.post_json("/v1/memories", json!({ "content": content, "agent_id": "a" })).await;

    assert!(status == 201 || status == 202, "{status} {body}");
    assert!(h.wait_for_rows("memory", 2).await.len() >= 2, "a long save is stored in pieces");
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
async fn search_returns_the_author_of_each_hit() {
    let h = Harness::start().await;
    h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "agent-http" })).await;
    h.wait_for_rows("memory", 1).await;

    let (status, body) = h.post_json("/v1/search", json!({ "query": CONTENT, "agent_id": "agent-http" })).await;
    assert_eq!(status, 200, "{body}");
    let hits = body["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty(), "{body}");
    assert!(hits.iter().all(|hit| hit["agent_id"] == "agent-http"), "{body}");
}

/// Writes `<data>/sharing.json` the way `clawg sharing` would leave it.
fn write_policy(h: &Harness, text: &str) {
    std::fs::write(clawg_core::paths::sharing_file(&h.ctx.runtime.base_dir), text).expect("write sharing.json");
}

async fn finds(h: &Harness, agent: &str, id: i64) -> bool {
    let (status, body) = h.post_json("/v1/search", json!({ "query": CONTENT, "agent_id": agent })).await;
    assert_eq!(status, 200, "{body}");
    body["hits"].as_array().expect("hits array").iter().any(|hit| hit["id"].as_i64() == Some(id))
}

#[tokio::test]
async fn search_and_context_follow_the_sharing_policy_from_the_next_call() {
    let h = Harness::start().await;
    let saved = save_id(&h, json!({ "content": CONTENT, "agent_id": "a" })).await;
    assert!(finds(&h, "b", saved).await, "everything is shared without a policy");
    let (_, body) = h.post_json("/v1/search", json!({ "query": CONTENT, "agent_id": "b" })).await;
    let hit = body["hits"].as_array().unwrap().iter().find(|hit| hit["id"].as_i64() == Some(saved)).cloned().unwrap();
    assert_eq!(hit["agent_id"], "a", "{body}");
    let (status, body) = h.post_json("/v1/context", json!({ "query": CONTENT, "agent_id": "b" })).await;
    assert_eq!(status, 200, "{body}");
    assert!(body["prompt"].as_str().unwrap().contains(" by a] "), "{body}");

    write_policy(&h, r#"{"version":1,"hides":{"b":["a"]}}"#);
    assert!(!finds(&h, "b", saved).await, "hiding counts without a restart");
    let (_, body) = h.post_json("/v1/context", json!({ "query": CONTENT, "agent_id": "b" })).await;
    assert!(!body["prompt"].as_str().unwrap().contains(CONTENT), "{body}");
    assert!(finds(&h, "c", saved).await, "only b was told not to read a");

    write_policy(&h, r#"{"version":1,"hides":{}}"#);
    assert!(finds(&h, "b", saved).await, "unhiding counts without a restart");
}

#[tokio::test]
async fn an_unreadable_policy_shares_nothing_over_http() {
    let h = Harness::start().await;
    let theirs = save_id(&h, json!({ "content": CONTENT, "agent_id": "a" })).await;
    let mine = save_id(&h, json!({ "content": format!("{CONTENT} too"), "agent_id": "b" })).await;
    write_policy(&h, r#"{"version":2,"hides":{}}"#);
    assert!(!finds(&h, "b", theirs).await);
    assert!(finds(&h, "b", mine).await, "an agent keeps its own memories");
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
    let scored: Vec<(f64, bool)> = full["hits"]
        .as_array()
        .expect("hits")
        .iter()
        .map(|h| (h["score"].as_f64().expect("score"), h["exact"].as_bool().expect("exact")))
        .collect();
    let shown = clawg_cli::relevance::shown_hits(&scored, 0.6);
    let expected_len = shown.iter().filter(|s| **s).count();
    assert!(expected_len < scored.len(), "the fixture must leave a tail to cut: {full}");
    assert!(shown[..expected_len].iter().all(|s| *s), "without an identifier the cut keeps a prefix: {full}");

    let (status, cut) = h
        .post_json(
            "/v1/search",
            json!({ "query": CONTENT, "agent_id": "agent-cut", "min_relative_score": 0.6 }),
        )
        .await;
    assert_eq!(status, 200, "{cut}");
    assert_eq!(hit_ids(&cut), hit_ids(&full)[..expected_len]);
}

/// Under the fake model a text other than the query lands at a cosine near 0,
/// below the floor, so every row here scores 0.
#[tokio::test]
async fn search_withholds_hits_that_did_not_score() {
    let h = Harness::start().await;
    seed_winner_and_tail(&h).await;
    let query = "which port does the staging proxy listen on";

    let (status, cut) = h
        .post_json("/v1/search", json!({ "query": query, "agent_id": "agent-cut", "min_relative_score": 0.35 }))
        .await;
    assert_eq!(status, 200, "{cut}");
    assert_eq!(cut["hits"], json!([]), "{cut}");
    assert_eq!(cut["withheld"], 4, "{cut}");

    let (status, full) = h.post_json("/v1/search", json!({ "query": query, "agent_id": "agent-cut" })).await;
    assert_eq!(status, 200, "{full}");
    assert_eq!(hit_ids(&full).len(), 4, "without a cut every hit comes back: {full}");
    assert_eq!(full["withheld"], 0, "{full}");
}

const IDENTIFIER_QUERY: &str = "how does parseLedger4821 load the nightly batch";
const IDENTIFIER_ROW: &str = "We kept `parseLedger4821` on the old import path.";
const RELATED_ROWS: [&str; 3] = [
    "The ledger import reads the nightly batch from the bucket.",
    "The nightly batch is loaded into the ledger in one transaction.",
    "A late nightly batch leaves the ledger totals stale until morning.",
];
const NOISE_ROW: &str = "a recipe for lentil soup with cumin";

/// The query on the first axis; the row holding its identifier at cosine
/// 0.85, three rows about the same topic without it at 0.88, 0.87 and 0.86,
/// and one unrelated row. Measured against the identifier match, whose bonus
/// lifts it a point above any cosine, none of the three passes a cut at 0.35;
/// measured against the best of them, all three do.
fn identifier_and_related_vectors() -> Vec<(&'static str, Vec<f32>)> {
    let mut v = vec![(IDENTIFIER_QUERY, at_cosine(1.0, 1)), (IDENTIFIER_ROW, at_cosine(0.85, 2))];
    for (i, (row, cosine)) in RELATED_ROWS.iter().zip([0.88, 0.87, 0.86]).enumerate() {
        v.push((row, at_cosine(cosine, 3 + i)));
    }
    v.push((NOISE_ROW, at_cosine(0.3, 6)));
    v
}

async fn save_as(h: &Harness, content: &str, agent: &str) {
    let (status, body) = h
        .post_json(
            "/v1/memories",
            json!({ "content": content, "agent_id": agent, "memory_type": "semantic_fact", "importance": 5 }),
        )
        .await;
    assert!(status == 201 || status == 202, "{status}: {body}");
}

async fn seed_identifier_and_related(h: &Harness, identifier_author: &str) {
    save_as(h, IDENTIFIER_ROW, identifier_author).await;
    for row in RELATED_ROWS.iter().chain([&NOISE_ROW]) {
        save_as(h, row, "agent-reader").await;
    }
    h.wait_for_rows("memory", 5).await;
}

fn hit_contents(body: &Value) -> Vec<&str> {
    body["hits"].as_array().expect("hits").iter().map(|h| h["content"].as_str().expect("content")).collect()
}

#[tokio::test]
async fn search_shows_the_rows_an_identifier_match_used_to_hide() {
    let h = Harness::start_inner(EmbedderState::Fixed(identifier_and_related_vectors())).await;
    seed_identifier_and_related(&h, "agent-reader").await;

    let (status, body) = h
        .post_json(
            "/v1/search",
            json!({ "query": IDENTIFIER_QUERY, "agent_id": "agent-reader", "min_relative_score": 0.35 }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let mut want = vec![IDENTIFIER_ROW];
    want.extend(RELATED_ROWS);
    assert_eq!(hit_contents(&body), want, "{body}");
    let exact: Vec<bool> = body["hits"].as_array().unwrap().iter().map(|h| h["exact"].as_bool().unwrap()).collect();
    assert_eq!(exact, [true, false, false, false]);

    let hits: Vec<clawg_cli::client::Hit> = serde_json::from_value(body["hits"].clone()).expect("client hits");
    let shown = clawg_cli::mcp::tools::relevant_hits(hits);
    assert_eq!(shown.len(), 4, "the bridge must keep what the daemon sent");
}

#[tokio::test]
async fn an_identifier_match_from_another_agent_is_exact_and_attributed() {
    let h = Harness::start_inner(EmbedderState::Fixed(identifier_and_related_vectors())).await;
    seed_identifier_and_related(&h, "agent-author").await;

    let (status, body) = h
        .post_json(
            "/v1/search",
            json!({ "query": IDENTIFIER_QUERY, "agent_id": "agent-reader", "min_relative_score": 0.35 }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let first = &body["hits"][0];
    assert_eq!(first["content"], IDENTIFIER_ROW, "{body}");
    assert_eq!(first["exact"], true);
    assert_eq!(first["agent_id"], "agent-author");
    assert_eq!(hit_contents(&body).len(), 4, "{body}");
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

const ENGLISH_NOTE: &str = "The deploy script lives in the ops folder, and it runs after every merge.";
const SPANISH_NOTE: &str = "El script de despliegue vive en la carpeta ops y corre después de cada merge.";

#[tokio::test]
async fn a_search_with_a_language_counts_the_other_one() {
    let h = Harness::start().await;
    save_id(&h, json!({ "content": ENGLISH_NOTE, "agent_id": "a" })).await;
    save_id(&h, json!({ "content": SPANISH_NOTE, "agent_id": "a" })).await;
    save_id(&h, json!({ "content": format!("{ENGLISH_NOTE} Twice."), "agent_id": "b" })).await;

    let (status, body) = h.post_json("/v1/search", json!({ "query": "deploy", "agent_id": "a", "lang": "es" })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["other_language"], json!({ "lang": "en", "memories": 2 }), "{body}");
    let (_, body) = h.post_json("/v1/search", json!({ "query": "deploy", "agent_id": "a", "lang": "en" })).await;
    assert_eq!(body["other_language"], json!({ "lang": "es", "memories": 1 }), "{body}");

    write_policy(&h, r#"{"version":1,"hides":{"a":["b"]}}"#);
    let (_, body) = h.post_json("/v1/search", json!({ "query": "deploy", "agent_id": "a", "lang": "es" })).await;
    assert_eq!(body["other_language"], json!({ "lang": "en", "memories": 1 }), "a hidden author counts for nothing: {body}");
}

#[tokio::test]
async fn a_search_without_a_language_has_no_count() {
    let h = Harness::start().await;
    save_id(&h, json!({ "content": ENGLISH_NOTE, "agent_id": "a" })).await;
    let (status, body) = h.post_json("/v1/search", json!({ "query": "deploy", "agent_id": "a" })).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.get("other_language").is_none(), "{body}");
    let (status, body) = h.post_json("/v1/context", json!({ "agent_id": "a", "lang": "es" })).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.get("other_language").is_none(), "a context without a query searches nothing: {body}");
}

#[tokio::test]
async fn a_context_with_a_query_counts_the_other_language() {
    let h = Harness::start().await;
    save_id(&h, json!({ "content": ENGLISH_NOTE, "agent_id": "a" })).await;
    let (status, body) = h.post_json("/v1/context", json!({ "query": "despliegue", "agent_id": "a", "lang": "es" })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["other_language"], json!({ "lang": "en", "memories": 1 }), "{body}");
}

#[tokio::test]
async fn an_unknown_language_is_a_bad_request() {
    let h = Harness::start().await;
    for (path, body) in [
        ("/v1/search", json!({ "query": "deploy", "agent_id": "a", "lang": "fr" })),
        ("/v1/context", json!({ "query": "deploy", "agent_id": "a", "lang": "ES" })),
    ] {
        let (status, body) = h.post_json(path, body).await;
        assert_eq!(status, 400, "{path}: {body}");
        let error = body["error"].as_str().unwrap_or_default();
        assert!(error.contains("en") && error.contains("es"), "{path}: {body}");
    }
}

/// An empty or workspace-only prompt reads as "no memories" unless the
/// answer says the memories were never searched.
#[tokio::test]
async fn context_says_whether_memories_were_searched() {
    async fn ask(h: &Harness) -> (u16, Value) {
        h.post_json("/v1/context", json!({ "query": "tabs", "agent_id": "a" })).await
    }

    let h = Harness::start().await;
    let (status, body) = ask(&h).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["retrieval"], json!({ "state": "searched", "hits": 0, "withheld": 0 }));

    for h in [Harness::start_without_embedder().await, Harness::start_unloaded(true).await] {
        let (status, body) = ask(&h).await;
        assert_eq!(status, 200, "the workspace still comes back: {body}");
        assert_eq!(body["retrieval"], json!({ "state": "embedder_not_ready" }));
    }

    let h = Harness::start_failed("disk full", false).await;
    let (status, body) = ask(&h).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["retrieval"], json!({ "state": "embedder_failed", "detail": "disk full", "retryable": false }));

    let h = Harness::start_with_failing_embedder().await;
    let (status, body) = ask(&h).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["retrieval"]["state"], "search_failed", "{body}");
    assert!(body["retrieval"]["detail"].as_str().is_some_and(|d| d.contains("onnx session crashed")), "{body}");
}

#[tokio::test]
async fn context_without_a_query_does_not_need_the_embedder() {
    let h = Harness::start().await;
    h.post_json("/v1/memories", json!({ "content": "the deploy script lives in ops/deploy.sh", "agent_id": "a" })).await;
    h.wait_for_rows("memory", 1).await;
    for body in [json!({ "agent_id": "a" }), json!({ "query": "  ", "agent_id": "a" })] {
        let (status, body) = h.post_json("/v1/context", body).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["retrieval"], json!({ "state": "recent", "hits": 1 }));
        assert!(body["prompt"].as_str().unwrap().contains("ops/deploy.sh"), "{body}");
    }

    let h = Harness::start_with_failing_embedder().await;
    let (status, body) = h.post_json("/v1/context", json!({ "agent_id": "a" })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["retrieval"], json!({ "state": "recent", "hits": 0 }), "nothing was embedded: {body}");
}

#[tokio::test]
async fn a_context_in_spanish_keeps_its_markers() {
    let h = Harness::start().await;
    let summary = json!({
        "content": "Objetivo: cortar la rama. Próximos pasos: etiquetar la versión.",
        "agent_id": "b",
        "project_id": "p",
        "memory_type": "session_summary",
    });
    h.post_json("/v1/memories", summary).await;
    h.wait_for_rows("memory", 1).await;
    let (status, body) = h.post_json("/v1/context", json!({ "agent_id": "a", "project_id": "p", "lang": "es" })).await;
    assert_eq!(status, 200, "{body}");
    let prompt = body["prompt"].as_str().unwrap();
    for marker in ["=== LAST SESSION ===", "session_summary by b]", "=== SHARED ==="] {
        assert!(prompt.contains(marker), "{marker:?} stays as it is: {prompt}");
    }
    assert!(prompt.contains("Las líneas marcadas \"by <agent>\" las guardó otro agente."), "{prompt}");
    let (_, english) = h.post_json("/v1/context", json!({ "agent_id": "a", "project_id": "p" })).await;
    assert!(english["prompt"].as_str().unwrap().contains("Lines marked \"by <agent>\" were saved by another agent."), "{english}");
}

/// Under the fake model only the row identical to the query scores; the
/// others land below the cosine floor.
#[tokio::test]
async fn context_says_how_many_hits_it_withheld() {
    let h = Harness::start().await;
    seed_winner_and_tail(&h).await;
    let ask = |query: &'static str| {
        let h = &h;
        async move { h.post_json("/v1/context", json!({ "query": query, "agent_id": "agent-cut" })).await }
    };

    let (status, body) = ask(CONTENT).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["retrieval"], json!({ "state": "searched", "hits": 1, "withheld": 3 }));

    let (status, body) = ask("which port does the staging proxy listen on").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["retrieval"], json!({ "state": "searched", "hits": 0, "withheld": 4 }));
}

/// Standing preferences are never cut, so a workspace of many long ones
/// overflows any budget and the closing note names the one that applied.
#[tokio::test]
async fn context_takes_a_budget_and_clamps_it() {
    let h = Harness::start().await;
    for i in 0..20 {
        let content = format!("preference {i}: {}", "always keep the error path explicit ".repeat(40));
        let (status, body) = h
            .post_json("/v1/memories", json!({ "content": content, "agent_id": "a", "memory_type": "user_preference", "importance": 9.0 }))
            .await;
        assert_eq!(status, 201, "{body}");
    }
    let context = |budget: Value| {
        let mut request = json!({ "query": "error path", "agent_id": "a" });
        if !budget.is_null() {
            request["budget"] = budget;
        }
        request
    };

    for (budget, applied) in [(Value::Null, 8_000), (json!(2_000), 2_000), (json!(1_000_000), 24_000)] {
        let (status, body) = h.post_json("/v1/context", context(budget)).await;
        assert_eq!(status, 200, "{body}");
        let prompt = body["prompt"].as_str().unwrap();
        assert!(prompt.contains(&format!("Cut to fit {applied} bytes")), "{prompt}");
    }
}

#[tokio::test]
async fn context_refuses_a_zero_budget() {
    let h = Harness::start().await;
    let (status, body) = h.post_json("/v1/context", json!({ "query": "q", "agent_id": "a", "budget": 0 })).await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = h.post_json("/v1/context", json!({ "query": "q", "agent_id": "a", "budget": -5 })).await;
    assert!((400..500).contains(&status), "a negative budget is not a budget: {status} {body}");
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
    assert_eq!(body["unreviewed"], 0, "a review is the project's, whichever agent made it: {body}");

    h.post_json("/v1/memories", json!({ "content": "the python package uses spaces", "agent_id": "b", "project_id": "p" })).await;
    let (_, body) = context().await;
    assert_eq!(body["unreviewed"], 1, "another agent's new memory is this one's to review too: {body}");
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
    assert_eq!(body["to_review"], json!([]), "every source is live: {body}");

    let r = h.authed(h.http.delete(h.url(&format!("/v1/memories/memory/{a}")))).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let (_, body) = h.post_json("/v1/synthesis", json!({ "project_id": "p" })).await;
    assert_eq!(body["to_review_total"], 1, "{body}");
    assert_eq!(body["to_review"][0]["conclusion"]["id"], c, "{body}");
    assert_eq!(body["to_review"][0]["live"], 1, "{body}");
    assert_eq!(body["to_review"][0]["lost"], json!([{ "id": a, "state": "deleted" }]), "{body}");
}

#[tokio::test]
async fn context_reports_conclusions_to_review() {
    let h = Harness::start().await;
    let a = save_id(&h, json!({ "content": "the go service uses tabs", "agent_id": "a", "project_id": "p" })).await;
    let b = save_id(&h, json!({ "content": "the rust crate uses tabs", "agent_id": "a", "project_id": "p" })).await;
    save_id(
        &h,
        json!({ "content": "the user indents with tabs", "agent_id": "a", "project_id": "p", "derived_from": [a, b] }),
    )
    .await;
    let context = || h.post_json("/v1/context", json!({ "query": "tabs", "agent_id": "a", "project_id": "p" }));
    let (status, body) = context().await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["conclusions_to_review"], 0, "{body}");

    let r = h.authed(h.http.delete(h.url(&format!("/v1/memories/memory/{a}")))).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let (_, body) = context().await;
    assert_eq!(body["conclusions_to_review"], 1, "{body}");

    h.post_json("/v1/synthesis", json!({ "agent_id": "a", "project_id": "p" })).await;
    let (_, body) = context().await;
    assert_eq!(body["conclusions_to_review"], 0, "the review turns the reminder off: {body}");
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
async fn a_save_from_a_replaced_source_is_refused() {
    let h = Harness::start().await;
    let old = save_id(&h, json!({ "content": "the deploy runs on fridays", "agent_id": "a", "topic_key": "deploy/day" })).await;
    let new = save_id(&h, json!({ "content": "the deploy runs on mondays", "agent_id": "a", "topic_key": "deploy/day" })).await;
    let kept = save_id(&h, json!({ "content": "the release is tagged by CI", "agent_id": "a" })).await;

    let (status, body) = h
        .post_json(
            "/v1/memories",
            json!({ "content": "deploys follow the calendar", "agent_id": "a", "derived_from": [kept, old] }),
        )
        .await;

    assert_eq!(status, 400, "{body}");
    let message = body.to_string();
    assert!(message.contains(&format!("{old} (by {new})")), "the refusal names what replaced it: {message}");
    let (status, body) = h
        .post_json(
            "/v1/memories",
            json!({ "content": "deploys follow the calendar", "agent_id": "a", "derived_from": [kept, new] }),
        )
        .await;
    assert_eq!(status, 201, "the current version is a source: {body}");
}

/// `source` and `created_by_type` of a memory row, as the viewer reads them.
async fn origin(h: &Harness, id: i64) -> (String, String) {
    let (status, body) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    assert_eq!(status, 200, "{body}");
    let field = |name: &str| body["item"][name].as_str().unwrap_or_default().to_string();
    (field("source"), field("created_by_type"))
}

/// `source_session_id` of a memory row, as the viewer reads it.
async fn session_of(h: &Harness, id: i64) -> Option<String> {
    let (status, body) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    assert_eq!(status, 200, "{body}");
    body["item"]["source_session_id"].as_str().map(str::to_string)
}

#[tokio::test]
async fn a_save_carries_the_registered_session() {
    let h = Harness::start().await;
    let register = |agent: &'static str, project: Option<&'static str>, session: &'static str| {
        h.post_json("/v1/sessions", json!({ "agent_id": agent, "project_id": project, "session_id": session }))
    };
    assert_eq!(register("a", Some("p"), "s1").await.0, 204);
    assert_eq!(register("a", Some("p"), "s2").await.0, 204);

    let mine = save_id(&h, json!({ "content": "the cache key holds the locale", "agent_id": "a", "project_id": "p" })).await;
    let other_agent = save_id(&h, json!({ "content": "the retry budget is three", "agent_id": "b", "project_id": "p" })).await;
    let other_project = save_id(&h, json!({ "content": "deploys happen on fridays", "agent_id": "a", "project_id": "q" })).await;

    assert_eq!(session_of(&h, mine).await.as_deref(), Some("s2"));
    assert_eq!(session_of(&h, other_agent).await, None);
    assert_eq!(session_of(&h, other_project).await, None);

    let (status, body) = h.post_json("/v1/sessions", json!({ "agent_id": "a", "session_id": "has space" })).await;
    assert_eq!(status, 400, "{body}");
    assert!(body.to_string().contains("session_id"), "{body}");
}

#[tokio::test]
async fn a_queued_save_keeps_its_session_in_the_outbox() {
    let h = Harness::start_without_embedder().await;
    h.post_json("/v1/sessions", json!({ "agent_id": "a", "session_id": "s1" })).await;
    let (status, body) = h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a" })).await;
    assert_eq!(status, 202, "{body}");

    let stamped: Vec<Option<String>> = h
        .ctx
        .runtime
        .store
        .conn
        .call(|c| {
            let mut stmt = c.prepare("SELECT json_extract(event, '$.session_id') FROM ingest_outbox")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<Result<Vec<_>, _>>()
        })
        .await
        .expect("outbox");
    assert_eq!(stamped, vec![Some("s1".to_string())], "a replay after a restart keeps the session it was accepted in");
}

#[tokio::test]
async fn rows_without_a_session_stay_reachable() {
    let h = Harness::start().await;
    let id = save_id(&h, json!({ "content": "the parser rejects tabs in yaml", "agent_id": "a" })).await;
    assert_eq!(session_of(&h, id).await, None);

    let (_, list) = h.get_json("/v1/memories?kind=memory&agent=a").await;
    assert_eq!(list["items"].as_array().map(Vec::len), Some(1), "{list}");
    let (status, found) = h.post_json("/v1/search", json!({ "query": "the parser rejects tabs in yaml", "agent_id": "a" })).await;
    assert_eq!(status, 200, "{found}");
    assert_eq!(found["hits"][0]["id"], id, "{found}");
}

#[tokio::test]
async fn an_undeclared_save_is_the_agents() {
    let h = Harness::start().await;
    let id = save_id(&h, json!({ "content": "the parser seems to reject tabs", "agent_id": "a" })).await;
    assert_eq!(origin(&h, id).await, ("explicit".into(), "agent".into()));
}

#[tokio::test]
async fn a_declared_save_is_the_users() {
    let h = Harness::start().await;
    let id = save_id(&h, json!({ "content": "use Postgres for the ledger", "agent_id": "a", "user_stated": true })).await;
    assert_eq!(origin(&h, id).await, ("explicit".into(), "user".into()));
}

#[tokio::test]
async fn a_conclusion_is_sealed_as_synthesis() {
    let h = Harness::start().await;
    let a = save_id(&h, json!({ "content": "the go service uses tabs", "agent_id": "a", "user_stated": true })).await;
    let b = save_id(&h, json!({ "content": "the rust crate uses tabs", "agent_id": "a" })).await;
    let c = save_id(&h, json!({ "content": "the user indents with tabs", "agent_id": "a", "derived_from": [a, b] })).await;
    assert_eq!(origin(&h, c).await, ("synthesis".into(), "agent".into()));
}

#[tokio::test]
async fn user_stated_with_derived_from_is_refused() {
    let h = Harness::start().await;
    let a = save_id(&h, json!({ "content": "the go service uses tabs", "agent_id": "a" })).await;

    let (status, body) = h
        .post_json(
            "/v1/memories",
            json!({ "content": "a conclusion", "agent_id": "a", "derived_from": [a], "user_stated": true }),
        )
        .await;

    assert_eq!(status, 400, "{body}");
    assert!(body.to_string().contains("user_stated"), "{body}");
    let (_, list) = h.get_json("/v1/memories?kind=memory").await;
    assert_eq!(list["items"].as_array().map(Vec::len), Some(1), "nothing was saved: {list}");
}

#[tokio::test]
async fn an_agent_reviews_another_agents_memories_unless_hidden() {
    let h = Harness::start().await;
    let mine = save_id(&h, json!({ "content": "the go service uses tabs", "agent_id": "a", "project_id": "p" })).await;
    let theirs = save_id(&h, json!({ "content": "the deploy runs on fridays", "agent_id": "b", "project_id": "p" })).await;
    let pending = || async {
        let (status, body) = h.post_json("/v1/synthesis", json!({ "agent_id": "a", "project_id": "p" })).await;
        assert_eq!(status, 200, "{body}");
        body["pending"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect::<Vec<i64>>()
    };

    assert_eq!(pending().await, vec![theirs, mine]);
    write_policy(&h, r#"{"version":1,"hides":{"a":["b"]}}"#);
    assert_eq!(pending().await, vec![mine]);
}

#[tokio::test]
async fn a_conclusion_may_cite_another_agents_memory_unless_hidden() {
    let h = Harness::start().await;
    let mine = save_id(&h, json!({ "content": "the go service uses tabs", "agent_id": "a", "project_id": "p" })).await;
    let theirs = save_id(&h, json!({ "content": "the deploy runs on fridays", "agent_id": "b", "project_id": "p" })).await;
    let conclude = |content: &'static str| {
        h.post_json(
            "/v1/memories",
            json!({ "content": content, "agent_id": "a", "project_id": "p", "derived_from": [mine, theirs] }),
        )
    };

    write_policy(&h, r#"{"version":1,"hides":{"a":["b"]}}"#);
    let (status, body) = conclude("a conclusion the policy refuses").await;
    assert_eq!(status, 400, "{body}");
    assert!(body.to_string().contains(&theirs.to_string()), "{body}");

    write_policy(&h, r#"{"version":1,"hides":{}}"#);
    let (status, body) = conclude("a conclusion drawn from both agents").await;
    assert_eq!(status, 201, "{body}");
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
async fn resolve_with_keep_both_still_works() {
    let h = Harness::start().await;
    let (old, new) = seed_dispute(&h).await;

    let (status, body) = h
        .post_json("/v1/disputes/resolve", json!({ "keep": new, "other": old, "keep_both": true }))
        .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["pair"], "judged", "{body}");
}

#[tokio::test]
async fn judge_records_a_verdict() {
    let h = Harness::start().await;
    let (old, new) = seed_dispute(&h).await;

    let (status, body) = h
        .post_json(
            "/v1/disputes/judge",
            json!({ "keep": old, "other": new, "verdict": "scoped", "reason": "8080 is dev, 9090 is prod", "agent_id": "agent-http" }),
        )
        .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["keep"]["status"], "active");
    assert_eq!(body["other"]["status"], "active");
    assert_eq!(body["pair"], "judged");
    let (status, body) = h
        .post_json("/v1/disputes/judge", json!({ "keep": old, "other": new, "verdict": "keep_both" }))
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().is_some_and(|e| e.contains("supersedes, compatible, scoped, conflicts")), "{body}");
}

#[tokio::test]
async fn judge_refuses_scoped_without_reason() {
    let h = Harness::start().await;
    let (old, new) = seed_dispute(&h).await;

    let (status, body) =
        h.post_json("/v1/disputes/judge", json!({ "keep": old, "other": new, "verdict": "scoped" })).await;

    assert_eq!(status, 400, "{body}");
    let (_, listed) = h.get_json("/v1/memories?kind=memory&status=disputed").await;
    assert_eq!(listed["items"][0]["id"], new, "a refused verdict changes nothing: {listed}");
}

#[tokio::test]
async fn a_protected_verdict_without_the_answer_is_a_409() {
    let h = Harness::start().await;
    let (old, new) = seed_dispute(&h).await;
    h.ctx
        .runtime
        .store
        .conn
        .call(move |c| c.execute("UPDATE memory SET memory_type = 'user_preference' WHERE id = ?1", [old]))
        .await
        .expect("make the old row a preference");
    let judge = |extra: serde_json::Value| {
        let mut body = json!({ "keep": new, "other": old, "verdict": "supersedes", "agent_id": "agent-http" });
        body.as_object_mut().expect("object").extend(extra.as_object().expect("object").clone());
        body
    };

    let (status, body) = h.post_json("/v1/disputes/judge", judge(json!({}))).await;
    assert_eq!(status, 409, "{body}");
    assert!(body["error"].as_str().is_some_and(|e| e.contains("user_answered")), "{body}");

    let (status, body) = h.post_json("/v1/disputes/judge", judge(json!({ "user_answered": true }))).await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = h
        .post_json("/v1/disputes/judge", judge(json!({ "user_answered": true, "reason": "port 9090 from now on" })))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["other"], json!({ "id": old, "status": "superseded", "superseded_by": new }));
}

#[tokio::test]
async fn disputes_lists_the_pending_pairs_an_agent_reaches_in_a_project() {
    let h = Harness::start().await;
    let (old, new) = seed_dispute(&h).await;
    h.ctx
        .runtime
        .store
        .conn
        .call(move |c| {
            c.execute(
                "INSERT INTO memory_conflict (new_id, old_id, distance, state, created_at) VALUES (?1, ?2, 0.3, 'pending', 1)",
                [new, old],
            )
        })
        .await
        .expect("note the pair");

    let (status, body) = h.get_json("/v1/disputes?agent_id=agent-http").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["pending"], 1, "{body}");
    assert_eq!(body["pairs"][0]["new_id"], new, "{body}");
    assert_eq!(body["pairs"][0]["old_id"], old, "{body}");

    let (_, other) = h.get_json("/v1/disputes?agent_id=agent-other").await;
    assert_eq!(other["pending"], 1, "another agent judges it too: {other}");
    let (_, elsewhere) = h.get_json("/v1/disputes?agent_id=agent-other&project_id=p").await;
    assert_eq!(elsewhere["pending"], 1, "a pair without a project is every project's: {elsewhere}");

    write_policy(&h, HIDE_AUTHOR);
    let (_, hidden) = h.get_json("/v1/disputes?agent_id=agent-other").await;
    assert_eq!(hidden["pending"], 0, "{hidden}");
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
    for content in ["the API listens on port 8080", "the logo is green"] {
        h.post_json("/v1/memories", json!({ "content": content, "agent_id": "agent-http" })).await;
    }
    let items = h.wait_for_rows("memory", 2).await;
    let (a, b) = (items[0]["id"].as_i64().expect("id"), items[1]["id"].as_i64().expect("id"));

    let (status, body) = h.post_json("/v1/disputes/resolve", json!({ "keep": a, "other": b })).await;

    assert_eq!(status, 409, "{body}");
}

/// `agent-other` does not read `agent-http`.
const HIDE_AUTHOR: &str = r#"{"version":1,"hides":{"agent-other":["agent-http"]}}"#;

#[tokio::test]
async fn delete_with_a_hidden_author_is_404_and_another_agent_deletes_otherwise() {
    let h = Harness::start().await;
    h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "agent-http" })).await;
    let id = h.wait_for_rows("memory", 1).await[0]["id"].as_i64().expect("id");
    let delete = || h.authed(h.http.delete(h.url(&format!("/v1/memories/memory/{id}?agent=agent-other")))).send();

    write_policy(&h, HIDE_AUTHOR);
    assert_eq!(delete().await.expect("request").status(), 404, "a hidden author's row reads as missing");
    let (_, row) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    assert!(row["item"]["deleted_at"].is_null(), "{row}");

    write_policy(&h, r#"{"version":1,"hides":{}}"#);
    assert_eq!(delete().await.expect("request").status(), 200, "another agent deletes what it reaches");
    let (_, row) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    assert!(!row["item"]["deleted_at"].is_null(), "{row}");
}

#[tokio::test]
async fn an_unreadable_policy_leaves_an_agent_its_own_rows_and_the_authorless() {
    let h = Harness::start().await;
    h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "agent-http" })).await;
    let id = h.wait_for_rows("memory", 1).await[0]["id"].as_i64().expect("id");

    write_policy(&h, "{ not json");
    let (status, _) = h.get_json(&format!("/v1/memories/memory/{id}/save?agent=agent-other")).await;
    assert_eq!(status, 404, "an unreadable policy shares nothing");
    let (status, _) = h.get_json(&format!("/v1/memories/memory/{id}/save?agent=agent-http")).await;
    assert_eq!(status, 200, "its author still reads it");
}

#[tokio::test]
async fn patch_changes_importance_and_type_and_reports_the_row() {
    let h = Harness::start().await;
    h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "agent-http" })).await;
    let id = h.wait_for_rows("memory", 1).await[0]["id"].as_i64().expect("id");

    let r = h
        .authed(h.http.patch(h.url(&format!("/v1/memories/memory/{id}"))))
        .json(&json!({ "importance": 8, "memory_type": "user_preference" }))
        .send()
        .await
        .expect("request");
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.expect("json");
    assert_eq!(body["updated"], 1, "{body}");
    assert_eq!(body["importance"], 8.0, "{body}");
    assert_eq!(body["memory_type"], "user_preference", "{body}");
    assert_eq!(body["tier"], 1, "{body}");

    let (_, row) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    assert_eq!(row["item"]["memory_type"], "user_preference", "{row}");
    assert_eq!(row["item"]["tier"], 1, "{row}");
    assert_eq!(row["item"]["update_count"], 1, "{row}");
}

#[tokio::test]
async fn a_demotion_without_the_answer_is_a_409() {
    let h = Harness::start().await;
    h.post_json(
        "/v1/memories",
        json!({ "content": CONTENT, "agent_id": "agent-http", "memory_type": "user_preference" }),
    )
    .await;
    let id = h.wait_for_rows("memory", 1).await[0]["id"].as_i64().expect("id");
    let patch = |body: Value| h.authed(h.http.patch(h.url(&format!("/v1/memories/memory/{id}")))).json(&body).send();

    let r = patch(json!({ "memory_type": "semantic_fact" })).await.expect("request");
    assert_eq!(r.status(), 409);
    let body: Value = r.json().await.expect("json");
    assert!(body["error"].as_str().is_some_and(|e| e.contains("user_answered")), "{body}");
    let (_, row) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    assert_eq!(row["item"]["memory_type"], "user_preference", "{row}");

    let r = patch(json!({ "memory_type": "semantic_fact", "user_answered": true })).await.expect("request");
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.expect("json");
    assert_eq!(body["memory_type"], "semantic_fact", "{body}");
}

#[tokio::test]
async fn an_invalid_patch_changes_nothing() {
    let h = Harness::start().await;
    h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "agent-http" })).await;
    let item = h.wait_for_rows("memory", 1).await.remove(0);
    let id = item["id"].as_i64().expect("id");

    for body in [
        json!({ "importance": 11 }),
        json!({}),
        json!({ "memory_type": "preference" }),
        json!({ "importanse": 3 }),
    ] {
        let r = h
            .authed(h.http.patch(h.url(&format!("/v1/memories/memory/{id}"))))
            .json(&body)
            .send()
            .await
            .expect("request");
        assert!(matches!(r.status().as_u16(), 400 | 422), "{body}: {}", r.status());
    }

    let (_, row) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    assert_eq!(row["item"]["importance"], item["importance"], "{row}");
    assert_eq!(row["item"]["memory_type"], item["memory_type"], "{row}");
    assert_eq!(row["item"]["update_count"], 0, "{row}");
}

#[tokio::test]
async fn patch_with_a_hidden_author_is_404_and_another_agent_patches_otherwise() {
    let h = Harness::start().await;
    h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "agent-http" })).await;
    let item = h.wait_for_rows("memory", 1).await.remove(0);
    let id = item["id"].as_i64().expect("id");
    let patch = || {
        h.authed(h.http.patch(h.url(&format!("/v1/memories/memory/{id}?agent=agent-other"))))
            .json(&json!({ "importance": 9.5 }))
            .send()
    };

    write_policy(&h, HIDE_AUTHOR);
    assert_eq!(patch().await.expect("request").status(), 404);
    let (_, row) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    assert_eq!(row["item"]["importance"], item["importance"], "{row}");

    write_policy(&h, r#"{"version":1,"hides":{}}"#);
    assert_eq!(patch().await.expect("request").status(), 200);
    let (_, row) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    assert_eq!(row["item"]["importance"], 9.5, "{row}");
}

#[tokio::test]
async fn resolve_with_a_hidden_author_is_404_and_another_agent_resolves_otherwise() {
    let h = Harness::start().await;
    let (old, new) = seed_dispute(&h).await;
    let resolve = || h.post_json("/v1/disputes/resolve", json!({ "keep": new, "other": old, "agent_id": "agent-other" }));

    write_policy(&h, HIDE_AUTHOR);
    let (status, body) = resolve().await;
    assert_eq!(status, 404, "{body}");
    let (_, row) = h.get_json(&format!("/v1/memories/memory/{new}")).await;
    assert_eq!(row["item"]["status"], "disputed", "{row}");

    write_policy(&h, r#"{"version":1,"hides":{}}"#);
    let (status, body) = resolve().await;
    assert_eq!(status, 200, "{body}");
    let (_, row) = h.get_json(&format!("/v1/memories/memory/{old}")).await;
    assert_eq!(row["item"]["status"], "superseded", "{row}");
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
    assert_eq!(item["created_by_type"], "agent", "an undeclared save is the agent's");
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
            c.execute("INSERT INTO project_path (id, root_path, created_at) VALUES (?1, '/elsewhere/repo', 0)", [id])
        })
        .await
        .expect("seed a colliding project");

    let (status, body) = h.post_json("/v1/projects", json!({ "root_path": root })).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "project_root_collision", "{body}");
    let detail = body["detail"].as_str().expect("detail");
    assert!(detail.contains(root) && detail.contains("/elsewhere/repo"), "{detail}");
}

#[tokio::test]
async fn registering_two_roots_that_collide_gives_each_its_own_id() {
    let h = Harness::start().await;
    let root = "/work/repo";
    let taken = clawg_core::memory::derive_project_id(std::path::Path::new(root));
    let seeded = taken.clone();
    h.ctx
        .runtime
        .store
        .conn
        .call(move |c| {
            c.execute("INSERT INTO project_path (id, root_path, created_at) VALUES (?1, '/elsewhere/repo', 0)", [seeded])
        })
        .await
        .expect("seed a colliding project");

    let (status, body) = h.post_json("/v1/projects", json!({ "root_path": root, "assigned": true })).await;

    assert_eq!(status, 200, "{body}");
    let id = body["project_id"].as_str().expect("project_id");
    assert_ne!(id, taken);
    assert!(clawg_core::memory::project::is_project_id(id), "{id}");
    assert_eq!(body["root_id"], id, "a root without a remote keeps its memories under its own id");
}

#[tokio::test]
async fn two_clones_of_one_remote_register_into_one_project() {
    let h = Harness::start().await;
    let base = tempfile::tempdir().expect("tempdir");
    let mut ids = Vec::new();
    for clone in ["work", "review"] {
        let git = base.path().join(clone).join("app").join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("config"), "[remote \"origin\"]\n\turl = git@github.com:acme/app.git\n").unwrap();
        let root = base.path().join(clone).join("app");
        let (status, body) =
            h.post_json("/v1/projects", json!({ "root_path": root.to_string_lossy(), "assigned": true })).await;
        assert_eq!(status, 200, "{body}");
        ids.push((body["project_id"].as_str().unwrap().to_string(), body["root_id"].as_str().unwrap().to_string()));
    }
    assert_eq!(ids[0].0, ids[1].0, "one project: {ids:?}");
    assert_eq!(ids[0].0, ids[0].1, "the first clone gives the project its id");
    assert_ne!(ids[1].1, ids[1].0, "the second keeps its own root id: {ids:?}");
}

#[tokio::test]
async fn registering_without_assigned_ids_keeps_the_old_answer() {
    let h = Harness::start().await;
    let base = tempfile::tempdir().expect("tempdir");
    let git = base.path().join("app").join(".git");
    std::fs::create_dir_all(&git).unwrap();
    std::fs::write(git.join("config"), "[remote \"origin\"]\n\turl = https://github.com/acme/app\n").unwrap();
    let root = base.path().join("app");

    let (status, body) = h.post_json("/v1/projects", json!({ "root_path": root.to_string_lossy() })).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["project_id"], clawg_core::memory::derive_project_id(&root));
    let remotes: i64 = h
        .ctx
        .runtime
        .store
        .conn
        .call(|c| c.query_row("SELECT COUNT(*) FROM project_remote", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(remotes, 0, "a caller that derives its id joins no remote");
}

fn set_origin(root: &std::path::Path, url: &str) {
    let git = root.join(".git");
    std::fs::create_dir_all(&git).unwrap();
    std::fs::write(git.join("config"), format!("[remote \"origin\"]\n\turl = {url}\n")).unwrap();
}

#[tokio::test]
async fn a_repository_cloned_where_another_was_registers_into_a_project_of_its_own() {
    let h = Harness::start().await;
    let base = tempfile::tempdir().expect("tempdir");
    let root = base.path().join("app");
    set_origin(&root, "git@github.com:acme/a.git");
    let register = |assigned: bool| {
        h.post_json("/v1/projects", json!({ "root_path": root.to_string_lossy(), "assigned": assigned }))
    };
    let (_, a) = register(true).await;

    set_origin(&root, "git@github.com:acme/b.git");
    let (status, old) = register(false).await;
    assert_eq!(status, 409, "{old}");
    assert_eq!(old["error"], "project_root_collision", "{old}");
    let detail = old["detail"].as_str().expect("detail");
    assert!(detail.contains("gives that root another"), "{detail}");
    let (status, b) = register(true).await;

    assert_eq!(status, 200, "{b}");
    assert_ne!(b["project_id"], a["project_id"], "{a} {b}");
    assert_ne!(b["root_id"], a["root_id"], "{a} {b}");
    set_origin(&root, "git@github.com:acme/a.git");
    let (_, back) = register(true).await;
    assert_eq!(back["project_id"], a["project_id"], "the first repository keeps its project");
}

/// Runs the real `git` in `dir` apart from the caller's git: none of its
/// `GIT_*` (a hook that started the tests would point it at the repository
/// being tested), no global or system config, and a fixed author.
fn git(dir: &std::path::Path, args: &[&str]) {
    let mut command = std::process::Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.as_encoded_bytes().get(..4).is_some_and(|p| p.eq_ignore_ascii_case(b"GIT_")) {
            command.env_remove(key);
        }
    }
    let null_device = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let output = command
        .current_dir(dir)
        .args(["-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null_device)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("this test needs git on the PATH");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

#[tokio::test]
async fn a_repository_without_origin_replaced_in_its_path_registers_into_a_project_of_its_own() {
    let h = Harness::start().await;
    let base = tempfile::tempdir().expect("tempdir");
    let root = base.path().join("app");
    let register = || h.post_json("/v1/projects", json!({ "root_path": root.to_string_lossy(), "assigned": true }));
    git(base.path(), &["init", "app"]);
    git(&root, &["commit", "--allow-empty", "-m", "the first commit of a"]);
    let (status, a) = register().await;
    assert_eq!(status, 200, "{a}");

    std::fs::rename(&root, base.path().join("a")).expect("move the first repository aside");
    git(base.path(), &["init", "app"]);
    git(&root, &["commit", "--allow-empty", "-m", "the first commit of b"]);
    let (status, b) = register().await;

    assert_eq!(status, 200, "{b}");
    assert_ne!(b["project_id"], a["project_id"], "{a} {b}");
    assert_ne!(b["root_id"], a["root_id"], "{a} {b}");
    let (_, again) = register().await;
    assert_eq!(again["project_id"], b["project_id"], "{again}");
}

#[tokio::test]
async fn a_folder_moved_without_origin_registers_into_its_project() {
    let h = Harness::start().await;
    let base = tempfile::tempdir().expect("tempdir");
    let (old, new) = (base.path().join("app"), base.path().join("app2"));
    let register = |root: &std::path::Path| {
        h.post_json("/v1/projects", json!({ "root_path": root.to_string_lossy(), "assigned": true }))
    };
    git(base.path(), &["init", "app"]);
    git(&old, &["commit", "--allow-empty", "-m", "the first commit of app"]);
    let (status, a) = register(&old).await;
    assert_eq!(status, 200, "{a}");
    let (status, saved) =
        h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a", "project_id": a["project_id"] })).await;
    assert_eq!(status, 201, "{saved}");

    std::fs::rename(&old, &new).expect("move the repository");
    let (status, moved) = register(&new).await;

    assert_eq!(status, 200, "{moved}");
    assert_eq!(moved["project_id"], a["project_id"], "{a} {moved}");
    assert_ne!(moved["root_id"], a["root_id"], "{a} {moved}");
    let project = moved["project_id"].as_str().expect("project_id");
    let (_, list) = h.get_json(&format!("/v1/memories?kind=memory&project={project}")).await;
    assert_eq!(list["items"].as_array().map(Vec::len), Some(1), "what was saved there is found from the new path: {list}");
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

#[tokio::test]
async fn the_projects_are_listed_with_their_folders_and_suggestions() {
    let h = Harness::start().await;
    let base = tempfile::tempdir().expect("tempdir");
    let (app, copy, gone) = (base.path().join("app"), base.path().join("copia"), base.path().join("gone"));
    let register = |root: &std::path::Path| {
        h.post_json("/v1/projects", json!({ "root_path": root.to_string_lossy(), "assigned": true }))
    };
    git(base.path(), &["init", "app"]);
    git(&app, &["commit", "--allow-empty", "-m", "the first commit of app"]);
    let (_, a) = register(&app).await;
    // Its origin is a path, so no remote tells the clone from a copy.
    git(base.path(), &["clone", "app", "copia"]);
    let (_, c) = register(&copy).await;
    std::fs::create_dir(&gone).expect("a folder");
    let (_, g) = register(&gone).await;
    std::fs::remove_dir(&gone).expect("remove the folder");
    let (status, saved) =
        h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a", "project_id": "loose-1a2b" })).await;
    assert_eq!(status, 201, "{saved}");

    let (status, body) = h.get_json("/v1/projects").await;

    assert_eq!(status, 200, "{body}");
    let listed = body["projects"].as_array().expect("projects");
    let project = |id: &Value| listed.iter().find(|p| p["id"] == *id).cloned().unwrap_or_else(|| panic!("{id} in {body}"));
    let folder = |p: &Value| {
        let folders = p["folders"].as_array().expect("folders");
        assert_eq!(folders.len(), 1, "{p}");
        let f = &folders[0];
        (f["root_id"].clone(), f["root_path"].clone(), f["remote"].clone(), f["index_docs"].clone(), f["present"].clone())
    };
    let suggestion = |p: &Value| {
        let s = &p["suggestions"][0];
        (s["other_id"].clone(), s["other_name"].clone(), s["reason"].clone(), s["created_at"].is_i64())
    };
    let (pa, pc, pg, loose) = (project(&a["project_id"]), project(&c["project_id"]), project(&g["project_id"]), project(&json!("loose-1a2b")));
    assert_eq!((pa["name"].as_str(), pa["born"].as_str(), pa["reviewed"].as_bool()), (Some("app"), Some("root"), Some(false)), "{pa}");
    assert_eq!(folder(&pa), (a["root_id"].clone(), json!(app.to_string_lossy()), Value::Null, json!(false), json!(true)));
    assert_eq!(suggestion(&pa), (c["project_id"].clone(), json!("copia"), json!("root_commit"), true), "{pa}");
    assert_eq!(suggestion(&pc), (a["project_id"].clone(), json!("app"), json!("root_commit"), true), "{pc}");
    assert_eq!(folder(&pg).4, json!(false), "a folder that is gone: {pg}");
    assert_eq!((loose["born"].as_str(), loose["memories"].as_i64()), (Some("adopted"), Some(1)), "{loose}");
    assert_eq!(loose["folders"], json!([]), "a project without a folder: {loose}");
    assert_eq!(pa["memories"], json!(0));
}

impl Harness {
    async fn rename(&self, id: &str, name: &str) -> (u16, Value) {
        self.send(self.http.patch(self.url(&format!("/v1/projects/{id}"))).json(&json!({ "name": name }))).await
    }
}

#[tokio::test]
async fn renaming_a_project_keeps_what_was_saved_under_it() {
    let h = Harness::start().await;
    let base = tempfile::tempdir().expect("tempdir");
    let (app, web) = (base.path().join("app"), base.path().join("web"));
    let mut ids = Vec::new();
    for root in [&app, &web] {
        std::fs::create_dir(root).expect("a folder");
        let (status, registered) =
            h.post_json("/v1/projects", json!({ "root_path": root.to_string_lossy(), "assigned": true })).await;
        assert_eq!(status, 200, "{registered}");
        ids.push(registered["project_id"].as_str().expect("project_id").to_string());
    }
    let (project, other) = (ids[0].as_str(), ids[1].as_str());
    let (status, saved) = h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a", "project_id": project })).await;
    assert_eq!(status, 201, "{saved}");

    let (status, renamed) = h.rename(project, "  App web ").await;

    assert_eq!(status, 200, "{renamed}");
    assert_eq!((renamed["id"].as_str(), renamed["name"].as_str(), renamed["reviewed"].as_bool()), (Some(project), Some("App web"), Some(true)));
    let (_, body) = h.get_json("/v1/projects").await;
    let listed = body["projects"].as_array().and_then(|l| l.iter().find(|p| p["id"] == project)).cloned().expect("listed");
    assert_eq!((listed["name"].as_str(), listed["reviewed"].as_bool(), listed["memories"].as_i64()), (Some("App web"), Some(true), Some(1)));
    let (_, list) = h.get_json(&format!("/v1/memories?kind=memory&project={project}")).await;
    assert_eq!(list["items"].as_array().map(Vec::len), Some(1), "what was saved under it is found as before: {list}");
    let (status, taken) = h.rename(other, "app WEB").await;
    assert_eq!((status, taken["error"].as_str()), (409, Some("name_taken")), "{taken}");
    let (status, blank) = h.rename(project, "   ").await;
    assert_eq!(status, 400, "{blank}");
}

#[tokio::test]
async fn renaming_an_unknown_project_is_404() {
    let h = Harness::start().await;

    let (status, body) = h.rename("nobody-0000", "anything").await;

    assert_eq!(status, 404, "{body}");
    assert!(body["error"].as_str().is_some_and(|e| e.contains("nobody-0000")), "the answer names the id: {body}");
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
        .call(move |c| c.execute("INSERT INTO project_path (id, root_path, created_at) VALUES (?1, ?2, 0)", [row_id, root]))
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

    let (status, body) = h
        .post_json("/v1/projects", json!({ "root_path": dir.path().display().to_string(), "index_docs": true }))
        .await;
    assert_eq!(status, 200, "{body}");
    let id = body["project_id"].as_str().expect("project_id").to_string();

    wait_for_docs(&h, &id, 1).await;
}

async fn wait_for_docs(h: &Harness, id: &str, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let docs = clawg_core::memory::project_docs::doc_count(&h.ctx.runtime.store, id).await.unwrap();
        if docs == expected {
            break;
        }
        assert!(Instant::now() < deadline, "the docs were never indexed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn index_docs_of(h: &Harness, id: &str) -> Option<i64> {
    let id = id.to_string();
    h.ctx
        .runtime
        .store
        .conn
        .call(move |c| c.query_row("SELECT (SELECT index_docs FROM project_path WHERE id = ?1)", [id], |r| r.get(0)))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_root_registered_without_its_docs_is_not_indexed() {
    let h = Harness::start().await;
    let label = tempfile::tempdir().expect("tempdir");
    std::fs::write(label.path().join("README.md"), "# Readme\nnever read\n").unwrap();
    let accepted = tempfile::tempdir().expect("tempdir");
    std::fs::write(accepted.path().join("README.md"), "# Readme\nread\n").unwrap();

    let (status, body) = h.post_json("/v1/projects", json!({ "root_path": label.path().display().to_string() })).await;
    assert_eq!(status, 200, "{body}");
    let label_id = body["project_id"].as_str().expect("project_id").to_string();
    // Registered after, so once its docs are in, the first root's refresh
    // would have run too if it had been started.
    let (_, body) = h
        .post_json("/v1/projects", json!({ "root_path": accepted.path().display().to_string(), "index_docs": true }))
        .await;
    wait_for_docs(&h, body["project_id"].as_str().expect("project_id"), 1).await;

    assert_eq!(index_docs_of(&h, &label_id).await, Some(0));
    assert_eq!(clawg_core::memory::project_docs::doc_count(&h.ctx.runtime.store, &label_id).await.unwrap(), 0);
}

#[tokio::test]
async fn registering_an_accepted_root_without_asking_leaves_its_index_alone() {
    let h = Harness::start().await;
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("README.md"), "# Readme\nindexed once\n").unwrap();
    let root = dir.path().display().to_string();
    let (_, body) = h.post_json("/v1/projects", json!({ "root_path": root, "index_docs": true })).await;
    let id = body["project_id"].as_str().expect("project_id").to_string();
    wait_for_docs(&h, &id, 1).await;
    while h.ctx.runtime.project_docs.is_running(&id) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    std::fs::write(dir.path().join("NOTES.md"), "# Notes\nwritten after\n").unwrap();
    let other = tempfile::tempdir().expect("tempdir");
    std::fs::write(other.path().join("README.md"), "# Readme\nread\n").unwrap();

    let (status, body) = h.post_json("/v1/projects", json!({ "root_path": root })).await;
    assert_eq!(status, 200, "{body}");
    // Registered after, so once its docs are in, a refresh of the first root
    // would have run too if the registration above had started one.
    let (_, body) = h
        .post_json("/v1/projects", json!({ "root_path": other.path().display().to_string(), "index_docs": true }))
        .await;
    wait_for_docs(&h, body["project_id"].as_str().expect("project_id"), 1).await;

    assert_eq!(index_docs_of(&h, &id).await, Some(1), "the root stays accepted");
    assert_eq!(clawg_core::memory::project_docs::doc_count(&h.ctx.runtime.store, &id).await.unwrap(), 1);
}

#[tokio::test]
async fn searching_the_docs_of_a_root_that_was_not_accepted_is_forbidden() {
    let h = Harness::start().await;
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("README.md"), "# Readme\nnever read\n").unwrap();
    let root = dir.path().display().to_string();
    let (_, body) = h.post_json("/v1/projects", json!({ "root_path": root })).await;
    let id = body["project_id"].as_str().expect("project_id").to_string();

    let (status, body) = h.post_json("/v1/projects/docs/search", json!({ "project_id": id, "query": "never read" })).await;

    assert_eq!(status, 403, "{body}");
    let error = body["error"].as_str().expect("error");
    assert!(error.contains(&format!("clawg projects add \"{root}\" --profile dev")), "{error}");
    assert_eq!(clawg_core::memory::project_docs::doc_count(&h.ctx.runtime.store, &id).await.unwrap(), 0);
}

#[tokio::test]
async fn registering_a_missing_root_to_index_it_is_a_bad_request() {
    let h = Harness::start().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("gone").display().to_string();

    let (status, body) = h.post_json("/v1/projects", json!({ "root_path": missing, "index_docs": true })).await;
    assert_eq!(status, 400, "{body}");
    let id = clawg_core::memory::derive_project_id(std::path::Path::new(&missing));
    assert_eq!(index_docs_of(&h, &id).await, None);

    // As a label for memories it is still taken, as it always was.
    let (status, body) = h.post_json("/v1/projects", json!({ "root_path": missing })).await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn a_background_refresh_that_failed_warns_the_next_search() {
    let (embedder, _gate) = docs_embedder(true, true);
    let h = Harness::start_inner(EmbedderState::Docs(embedder)).await;
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("README.md"), "# Readme\nnever embedded\n").unwrap();

    let (status, body) = h
        .post_json("/v1/projects", json!({ "root_path": dir.path().display().to_string(), "index_docs": true }))
        .await;
    assert_eq!(status, 200, "{body}");
    let id = body["project_id"].as_str().expect("project_id").to_string();
    let index = &h.ctx.runtime.project_docs;
    let deadline = Instant::now() + Duration::from_secs(10);
    while index.is_due(&id, Instant::now()) || index.is_running(&id) {
        assert!(Instant::now() < deadline, "the background refresh never ended");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let (status, body) = h.post_json("/v1/projects/docs/search", json!({ "project_id": id, "query": "readme" })).await;

    assert_eq!(status, 200, "{body}");
    assert!(body["warning"].as_str().is_some_and(|w| w.contains("passages refused")), "{body}");
    assert_eq!(body["docs"], 0);
}

/// A project indexed with `a.md` outside the daemon's own bookkeeping, so the
/// daemon finds it due for a refresh, whose `a.md` then changed and which got a
/// `b.md`.
async fn indexed_project_that_changed(h: &Harness) -> (tempfile::TempDir, String) {
    let (dir, id) = project_with_docs(h, &[("a.md", "# A\nold passage\n")]).await;
    let elsewhere = Arc::new(clawg_core::memory::project_docs::DocIndex::default());
    elsewhere.try_refresh(&h.ctx.runtime.store, &FakeEmbedder::default(), None, &id).await.unwrap();
    std::fs::write(dir.path().join("a.md"), "# A\nnew passage\n").unwrap();
    std::fs::write(dir.path().join("b.md"), "# B\nanother passage\n").unwrap();
    (dir, id)
}

fn texts(body: &Value) -> Vec<&str> {
    body["hits"].as_array().expect("hits").iter().filter_map(|h| h["text"].as_str()).collect()
}

#[tokio::test]
async fn a_search_answers_from_the_index_while_a_long_refresh_runs() {
    let (embedder, gate) = docs_embedder(false, false);
    let h = Harness::start_inner(EmbedderState::Docs(embedder)).await;
    let (_dir, id) = indexed_project_that_changed(&h).await;

    let search = h.post_json("/v1/projects/docs/search", json!({ "project_id": id, "query": "passage" }));
    let (status, body) = tokio::time::timeout(REFRESH_GRACE + Duration::from_secs(5), search)
        .await
        .expect("the search waited for the whole refresh");

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["indexing"], true, "{body}");
    assert_eq!(body["docs"], 1, "{body}");
    assert_eq!(texts(&body), ["old passage"]);

    gate.send(true).unwrap();
    assert!(h.ctx.runtime.project_docs.wait_idle(&id, Duration::from_secs(10)).await);
    let (status, body) = h.post_json("/v1/projects/docs/search", json!({ "project_id": id, "query": "passage" })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["indexing"], false, "{body}");
    assert_eq!(body["docs"], 2, "{body}");
    let mut found = texts(&body);
    found.sort();
    assert_eq!(found, ["another passage", "new passage"]);
}

#[tokio::test]
async fn a_small_edit_is_found_by_the_search_that_refreshes() {
    let h = Harness::start().await;
    let (_dir, id) = indexed_project_that_changed(&h).await;

    let (status, body) = h.post_json("/v1/projects/docs/search", json!({ "project_id": id, "query": "passage" })).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["indexing"], false, "{body}");
    assert_eq!(body["docs"], 2, "{body}");
    assert!(texts(&body).contains(&"new passage"), "{body}");
    assert!(!texts(&body).contains(&"old passage"), "{body}");
}

#[tokio::test]
async fn the_refresh_outlives_the_search_that_started_it() {
    let (embedder, gate) = docs_embedder(false, false);
    let h = Harness::start_inner(EmbedderState::Docs(embedder)).await;
    let (_dir, id) = indexed_project_that_changed(&h).await;

    let gone = h
        .authed(h.http.post(h.url("/v1/projects/docs/search")))
        .json(&json!({ "project_id": id, "query": "passage" }))
        .timeout(Duration::from_millis(200))
        .send()
        .await;
    assert!(gone.is_err(), "the client was meant to give up first: {gone:?}");
    // Let the server see the closed connection before the refresh can go on.
    tokio::time::sleep(Duration::from_millis(200)).await;

    gate.send(true).unwrap();
    wait_for_docs(&h, &id, 2).await;
}

#[tokio::test]
async fn a_search_during_the_first_index_waits_for_it() {
    let h = Harness::start().await;
    let (_dir, id) = project_with_docs(&h, &[("a.md", "# A\nfound after the wait\n")]).await;
    let running = h.ctx.runtime.project_docs.begin(&id).expect("no refresh is running yet");
    let store = h.ctx.runtime.store.clone();
    let project = id.clone();
    let indexer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let elsewhere = Arc::new(clawg_core::memory::project_docs::DocIndex::default());
        elsewhere.try_refresh(&store, &FakeEmbedder::default(), None, &project).await.unwrap();
        drop(running);
    });

    let (status, body) = h.post_json("/v1/projects/docs/search", json!({ "project_id": id, "query": "wait" })).await;

    indexer.await.unwrap();
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["indexing"], false, "{body}");
    assert_eq!(body["docs"], 1);
    assert_eq!(body["hits"][0]["text"], "found after the wait");
}

// ── restore ─────────────────────────────────────────────────────────────────

impl Harness {
    async fn send(&self, req: reqwest::RequestBuilder) -> (u16, Value) {
        let r = self.authed(req).send().await.expect("request");
        let status = r.status().as_u16();
        (status, r.json().await.unwrap_or(Value::Null))
    }

    async fn delete_row(&self, id: i64) {
        let (status, body) = self.send(self.http.delete(self.url(&format!("/v1/memories/memory/{id}")))).await;
        assert_eq!(status, 200, "{body}");
    }

    async fn restore(&self, kind: &str, id: i64, agent: Option<&str>) -> (u16, Value) {
        let query = agent.map(|a| format!("?agent={a}")).unwrap_or_default();
        self.send(self.http.post(self.url(&format!("/v1/memories/{kind}/{id}/restore{query}")))).await
    }
}

#[tokio::test]
async fn restore_brings_a_deleted_memory_back_for_any_agent_that_reaches_it() {
    let h = Harness::start().await;
    let id = save_id(&h, json!({ "content": CONTENT, "agent_id": "a" })).await;
    h.delete_row(id).await;
    assert!(!finds(&h, "a", id).await);

    write_policy(&h, r#"{"version":1,"hides":{"b":["a"]}}"#);
    let (status, body) = h.restore("memory", id, Some("b")).await;
    assert_eq!(status, 404, "a hidden author's row reads as missing: {body}");
    assert!(!finds(&h, "a", id).await);

    write_policy(&h, r#"{"version":1,"hides":{}}"#);
    let (status, body) = h.restore("memory", id, Some("b")).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "restored": 1, "id": id, "status": "active", "superseded_by": null }));
    assert!(finds(&h, "a", id).await, "it is found again");
    let (_, item) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    assert_eq!(item["item"]["deleted_at"], Value::Null, "{item}");
    assert_eq!(item["item"]["update_count"], 1, "{item}");
}

#[tokio::test]
async fn restore_answers_each_refusal_with_its_status() {
    let h = Harness::start().await;
    let twin = save_id(&h, json!({ "content": CONTENT, "agent_id": "a" })).await;
    h.delete_row(twin).await;
    let live = save_id(&h, json!({ "content": CONTENT, "agent_id": "a" })).await;

    let (status, body) = h.restore("memory", twin, None).await;
    assert_eq!(status, 409, "{body}");
    assert!(body["error"].as_str().unwrap().contains(&format!("#{live}")), "{body}");

    let (status, body) = h.restore("memory", live, None).await;
    assert_eq!(status, 404, "a live row is not deleted: {body}");
    let (status, body) = h.restore("episodic", 1, None).await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = h.restore("bogus", 1, None).await;
    assert_eq!(status, 400, "{body}");

    let without = Harness::start_without_embedder().await;
    let (status, body) = without.restore("memory", 1, None).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"], "embedder_not_ready");
}

#[tokio::test]
async fn a_save_refuses_an_invalid_topic_key() {
    let h = Harness::start().await;

    let (status, body) =
        h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a", "topic_key": "Storage Engine" })).await;

    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().unwrap().contains("topic_key"), "{body}");
    let (_, listed) = h.get_json("/v1/memories").await;
    assert_eq!(listed["items"], json!([]), "nothing was queued: {listed}");
}

#[tokio::test]
async fn a_save_echoes_its_topic_key() {
    let h = Harness::start().await;
    let save = |content: &str| json!({ "content": content, "agent_id": "a", "topic_key": "storage/engine" });

    let (status, first) = h.post_json("/v1/memories", save("Storage engine is SQLite")).await;
    assert_eq!(status, 201, "{first}");
    assert_eq!(first["topic_key"], "storage/engine", "{first}");
    assert!(first.get("replaced").is_none(), "{first}");
    let old = first["memories"][0]["id"].as_i64().unwrap();

    let (status, second) = h.post_json("/v1/memories", save("Storage engine moved to Postgres")).await;
    assert_eq!(status, 201, "{second}");
    assert_eq!(second["replaced"], json!([{ "kind": "memory", "id": old, "excerpt": "Storage engine is SQLite" }]));
    let (_, item) = h.get_json(&format!("/v1/memories/memory/{old}")).await;
    assert_eq!(item["item"]["status"], "superseded", "{item}");
    assert_eq!(item["item"]["topic_key"], "storage/engine", "{item}");

    let (_, plain) = h.post_json("/v1/memories", json!({ "content": "deploys happen on fridays", "agent_id": "a" })).await;
    assert!(plain.get("topic_key").is_none(), "{plain}");
}

#[tokio::test]
async fn a_search_hit_and_the_save_read_carry_the_topic_key() {
    let h = Harness::start().await;
    let keyed = save_id(&h, json!({ "content": CONTENT, "agent_id": "a", "topic_key": "storage/engine" })).await;
    let plain = save_id(&h, json!({ "content": "deploys happen on fridays", "agent_id": "a" })).await;

    let (status, body) = h.post_json("/v1/search", json!({ "query": CONTENT, "agent_id": "a" })).await;
    assert_eq!(status, 200, "{body}");
    let hits = body["hits"].as_array().expect("hits array");
    let hit = |id: i64| hits.iter().find(|hit| hit["id"].as_i64() == Some(id));
    assert_eq!(hit(keyed).expect("keyed hit")["topic_key"], "storage/engine", "{body}");
    if let Some(plain) = hit(plain) {
        assert!(plain.get("topic_key").is_none(), "{body}");
    }

    let (status, save) = h.get_json(&format!("/v1/memories/memory/{keyed}/save?agent=a")).await;
    assert_eq!(status, 200, "{save}");
    assert_eq!(save["topic_key"], "storage/engine", "{save}");
    let (_, save) = h.get_json(&format!("/v1/memories/memory/{plain}/save?agent=a")).await;
    assert!(save.get("topic_key").is_none(), "{save}");
}

/// `null` is sent, not left out: a bridge reads a missing field as a daemon
/// that predates it.
#[tokio::test]
async fn a_search_hit_and_the_save_read_carry_the_project() {
    let h = Harness::start().await;
    let placed = save_id(&h, json!({ "content": CONTENT, "agent_id": "a", "project_id": "p" })).await;
    let unplaced = save_id(&h, json!({ "content": "deploys happen on fridays", "agent_id": "a" })).await;

    let (status, body) = h.post_json("/v1/search", json!({ "query": CONTENT, "agent_id": "a", "limit": 10 })).await;
    assert_eq!(status, 200, "{body}");
    let hits = body["hits"].as_array().expect("hits array");
    let hit = |id: i64| hits.iter().find(|hit| hit["id"].as_i64() == Some(id)).cloned();
    assert_eq!(hit(placed).expect("placed hit")["project_id"], "p", "{body}");
    if let Some(unplaced) = hit(unplaced) {
        assert_eq!(unplaced.get("project_id"), Some(&Value::Null), "{body}");
    }

    let (status, save) = h.get_json(&format!("/v1/memories/memory/{placed}/save?agent=a")).await;
    assert_eq!(status, 200, "{save}");
    assert_eq!(save["project_id"], "p", "{save}");
    let (_, save) = h.get_json(&format!("/v1/memories/memory/{unplaced}/save?agent=a")).await;
    assert_eq!(save.get("project_id"), Some(&Value::Null), "{save}");
}

#[tokio::test]
async fn restore_refuses_a_row_whose_topic_is_live_again() {
    let h = Harness::start().await;
    let save = |content: &str| json!({ "content": content, "agent_id": "a", "topic_key": "storage/engine" });
    let old = save_id(&h, save("Storage engine is SQLite")).await;
    h.delete_row(old).await;
    let current = save_id(&h, save("Storage engine moved to Postgres")).await;

    let (status, body) = h.restore("memory", old, Some("a")).await;

    assert_eq!(status, 409, "{body}");
    assert!(body["error"].as_str().unwrap().contains(&format!("#{current}")), "{body}");
}

#[tokio::test]
async fn deleting_the_current_revision_answers_what_it_revived() {
    let h = Harness::start().await;
    let save = |content: &str| json!({ "content": content, "agent_id": "a", "topic_key": "storage/engine" });
    let old = save_id(&h, save(CONTENT)).await;
    let current = save_id(&h, save("Storage engine moved to Postgres")).await;
    assert!(!finds(&h, "a", old).await, "the revision replaced it");

    let (status, body) =
        h.send(h.http.delete(h.url(&format!("/v1/memories/memory/{current}?agent=a")))).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["revived"][0]["id"].as_i64(), Some(old), "{body}");
    assert_eq!(body["revived"][0]["kind"], "memory", "{body}");
    assert!(body.get("hidden").is_none(), "{body}");
    assert!(finds(&h, "a", old).await, "the version before is current again");

    let (status, body) = h.restore("memory", current, Some("a")).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["topic_key"], "storage/engine", "{body}");
    assert_eq!(body["replaced"][0]["id"].as_i64(), Some(old), "{body}");
    assert!(!finds(&h, "a", old).await, "the restore replaced it again");
}

/// `v` normalized, as the store keeps it.
fn unit(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter().map(|x| x / norm).collect()
}

#[tokio::test]
async fn a_restore_answers_its_key_and_its_new_pairs() {
    let h = Harness::start().await;
    let text = "Storage engine is SQLite";
    let restored = save_id(&h, json!({ "content": text, "agent_id": "a", "topic_key": "storage/engine" })).await;
    h.delete_row(restored).await;
    let newer = save_id(&h, json!({ "content": "the store keeps one local file", "agent_id": "a" })).await;
    // The fake model places unrelated texts far apart; this puts the newer
    // row inside the detector's band of the restored one.
    let a = unit(&FakeEmbedder::default().embed_with_role(text, EmbedRole::Passage).await.unwrap());
    let e = unit(&FakeEmbedder::default().embed_with_role("another direction", EmbedRole::Passage).await.unwrap());
    let along: f32 = a.iter().zip(&e).map(|(x, y)| x * y).sum();
    let across = unit(&e.iter().zip(&a).map(|(y, x)| y - along * x).collect::<Vec<_>>());
    let angle = 2.0 * (0.3f32 / 2.0).asin();
    let near: Vec<u8> = a
        .iter()
        .zip(&across)
        .map(|(x, y)| x * angle.cos() + y * angle.sin())
        .flat_map(|f| f.to_le_bytes())
        .collect();
    h.ctx
        .runtime
        .store
        .conn
        .call(move |c| {
            c.execute("UPDATE memory SET memory_type = 'semantic_fact' WHERE id = ?1", [restored])?;
            c.execute("UPDATE memory SET memory_type = 'semantic_fact' WHERE id = ?1", [newer])?;
            c.execute("UPDATE memory_vectors SET embedding = ?2 WHERE rowid = ?1", rusqlite::params![newer, near])
        })
        .await
        .expect("place");

    let (status, body) = h.restore("memory", restored, Some("a")).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["topic_key"], "storage/engine", "{body}");
    assert_eq!(
        body["related"],
        json!([{ "kind": "memory", "id": newer, "escalated": false, "excerpt": "the store keeps one local file" }]),
        "{body}"
    );
    assert!(body.get("replaced").is_none() && body.get("superseded_by_deleted").is_none(), "{body}");
}

#[tokio::test]
async fn listing_deleted_rows_shows_only_them_newest_deletion_first() {
    let h = Harness::start().await;
    let kept = save_id(&h, json!({ "content": "the go service uses tabs", "agent_id": "a" })).await;
    let first = save_id(&h, json!({ "content": "the rust crate uses tabs", "agent_id": "a" })).await;
    let second = save_id(&h, json!({ "content": "deploys happen on fridays", "agent_id": "a" })).await;
    h.delete_row(second).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    h.delete_row(first).await;

    let ids = |body: &Value| -> Vec<i64> {
        body["items"].as_array().expect("items").iter().map(|r| r["id"].as_i64().unwrap()).collect()
    };
    let (status, deleted) = h.get_json("/v1/memories?deleted=true").await;
    assert_eq!(status, 200, "{deleted}");
    assert_eq!(ids(&deleted), vec![first, second]);
    assert!(deleted["items"][0]["deleted_at"].is_i64(), "{deleted}");
    let (_, live) = h.get_json("/v1/memories").await;
    assert_eq!(ids(&live), vec![kept]);
    let (status, body) = h.get_json("/v1/memories?deleted=true&q=tabs").await;
    assert_eq!(status, 400, "{body}");
}

#[tokio::test]
async fn health_says_whether_the_model_is_in_memory() {
    let (_, loaded) = Harness::start().await.get_json("/health").await;
    assert_eq!(loaded["embedder_resident"], true, "{loaded}");

    let (_, unloaded) = Harness::start_unloaded(false).await.get_json("/health").await;
    assert_eq!(unloaded["embedder"], "ready", "an unloaded model is still usable: {unloaded}");
    assert_eq!(unloaded["embedder_resident"], false, "{unloaded}");

    let (_, loading) = Harness::start_without_embedder().await.get_json("/health").await;
    assert!(loading["embedder_resident"].is_null(), "{loading}");
}

#[tokio::test]
async fn a_search_with_the_model_unloaded_loads_it_and_answers() {
    let h = Harness::start_unloaded(false).await;
    let (status, body) = h.post_json("/v1/search", json!({ "query": "anything", "agent_id": "a" })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(h.reloads.load(Ordering::SeqCst), 1);
    let (_, health) = h.get_json("/health").await;
    assert_eq!(health["embedder_resident"], true, "{health}");
}

#[tokio::test]
async fn a_save_with_the_model_unloaded_is_stored() {
    let h = Harness::start_unloaded(false).await;
    let (status, body) = h
        .post_json("/v1/memories", json!({ "content": "saved while the model was away", "agent_id": "a" }))
        .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(h.reloads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_save_the_model_cannot_take_answers_202_retrying_and_keeps_its_row() {
    let h = Harness::start_inner(EmbedderState::AnsweringUnavailable).await;
    let (status, body) = h
        .post_json("/v1/memories", json!({ "content": "saved while the model files are gone", "agent_id": "a" }))
        .await;

    assert_eq!((status, body["reason"].as_str()), (202, Some("retrying")), "{body}");
    assert_eq!(body["detail"], "the embedder is not available", "{body}");
    assert!(body["operation_id"].as_str().is_some(), "{body}");
    let rows = outbox(&h).await;
    assert_eq!(rows.len(), 1, "the worker retries the row: {rows:?}");
    assert_eq!(rows[0].0, "pending");
}

#[tokio::test]
async fn a_search_whose_model_cannot_load_again_answers_not_ready() {
    let h = Harness::start_unloaded(true).await;
    let (status, body) = h.post_json("/v1/search", json!({ "query": "anything", "agent_id": "a" })).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"], "embedder_not_ready", "{body}");
}

#[tokio::test]
async fn registering_a_project_wakes_an_unloaded_model_without_waiting_for_it() {
    let h = Harness::start_unloaded(false).await;
    let root = tempfile::tempdir().expect("root");
    let (status, body) =
        h.post_json("/v1/projects", json!({ "root_path": root.path().to_string_lossy() })).await;
    assert_eq!(status, 200, "{body}");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (_, health) = h.get_json("/health").await;
        if health["embedder_resident"] == true {
            break;
        }
        assert!(Instant::now() < deadline, "the model never came back: {health}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(h.reloads.load(Ordering::SeqCst), 1);
}

// ── /v1/import ──────────────────────────────────────────────────────────────

/// An export of two rows of agent `a`, the second replaced by the first.
fn export_file(version: u32) -> String {
    let header = json!({
        "kind": "header", "format": "clawg-memories", "version": version, "exported_at": 0, "clawg": "0.0.0",
        "filter": {"project": null, "agent": null, "include_deleted": false},
    });
    let row = |id: i64, content: &str, status: &str, by: Option<i64>| {
        json!({
            "kind": "memory", "id": id, "scope": "agent", "agent_id": "a", "content": content,
            "memory_type": "semantic_fact", "source": "explicit", "created_at": 1_000 + id,
            "created_by_type": "agent", "last_modified_at": 1_000 + id, "last_accessed_at": 9_000,
            "access_count": 7, "importance": 6.0, "tier": 2, "status": status, "superseded_by": by,
            "supersede_cause": by.map(|_| "verdict"),
        })
    };
    [header, row(1, "Imported port is 4000", "active", None), row(2, "Imported port was 3000", "superseded", Some(1))]
        .iter()
        .map(|l| format!("{l}\n"))
        .collect()
}

#[tokio::test]
async fn an_import_stores_rows_with_their_state_and_a_dry_run_stores_none() {
    let h = Harness::start().await;
    let (status, dry) = h.post_json("/v1/import", json!({ "text": export_file(1), "dry_run": true })).await;
    assert_eq!(status, 200, "{dry}");
    assert_eq!((dry["dry_run"].clone(), dry["inserted"].clone(), dry["snapshot"].clone()), (json!(true), json!(2), Value::Null));
    let (_, listed) = h.get_json("/v1/memories?kind=memory&agent=a").await;
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(0), "{listed}");

    let (status, report) = h.post_json("/v1/import", json!({ "text": export_file(1), "agent": "b" })).await;
    assert_eq!(status, 200, "{report}");
    assert_eq!(report["inserted"], 2, "{report}");
    assert_eq!(report["by_state"], json!({ "live": 1, "superseded": 1 }));
    let snapshot = report["snapshot"].as_str().expect("a snapshot");
    assert!(snapshot.ends_with("-pre-import.db") && std::path::Path::new(snapshot).exists(), "{snapshot}");

    let (_, listed) = h.get_json("/v1/memories?kind=memory&agent=b&status=active").await;
    let live = listed["items"].as_array().and_then(|r| r.first()).cloned().unwrap_or(Value::Null);
    assert_eq!(
        (live["summary"].clone(), live["access_count"].clone(), live["tier"].clone()),
        (json!("Imported port is 4000"), json!(7), json!(2)),
        "{listed}"
    );
    let (_, found) = h.post_json("/v1/search", json!({ "query": "Imported port is 4000", "agent_id": "b" })).await;
    assert_eq!(found["hits"][0]["id"], live["id"], "{found}");
}

#[tokio::test]
async fn an_import_is_undone_by_its_number() {
    let h = Harness::start().await;
    let (status, report) = h.post_json("/v1/import", json!({ "text": export_file(1) })).await;
    assert_eq!((status, report["import_id"].clone()), (200, json!(1)), "{report}");
    let count = || async { h.get_json("/v1/memories?kind=memory&agent=a").await.1["items"].as_array().map(Vec::len) };

    let (status, dry) = h.post_json("/v1/import/undo", json!({ "import_id": 1, "dry_run": true })).await;
    assert_eq!(status, 200, "{dry}");
    assert_eq!((dry["removed"].as_array().map(Vec::len), dry["snapshot"].clone()), (Some(2), Value::Null), "{dry}");
    assert_eq!(count().await, Some(2), "a dry run writes nothing");

    let (status, undone) = h.post_json("/v1/import/undo", json!({ "import_id": 1 })).await;
    assert_eq!(status, 200, "{undone}");
    assert_eq!(undone["removed"], dry["removed"], "{undone}");
    assert!(undone["snapshot"].as_str().is_some_and(|s| s.ends_with("-pre-undo.db")), "{undone}");
    assert_eq!(count().await, Some(0));

    let (status, again) = h.post_json("/v1/import/undo", json!({ "import_id": 1 })).await;
    assert_eq!((status, again["removed"].clone(), again["snapshot"].clone()), (200, json!([]), Value::Null), "{again}");
    let (status, body) = h.post_json("/v1/import/undo", json!({ "import_id": 9 })).await;
    assert_eq!(status, 404, "{body}");
    let (status, body) = h.post_json("/v1/import/undo", json!({ "import": 1 })).await;
    assert!((400..500).contains(&status), "a misspelt field undoes nothing: {status} {body}");

    let h = Harness::start_without_embedder().await;
    let (status, body) = h.post_json("/v1/import/undo", json!({ "import_id": 1 })).await;
    assert_eq!(status, 404, "an undo needs no model: {body}");
}

#[tokio::test]
async fn import_answers_400_for_an_unknown_version_before_the_embedder() {
    let h = Harness::start_without_embedder().await;
    let (status, body) = h.post_json("/v1/import", json!({ "text": export_file(2) })).await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().unwrap_or("").contains("version 2"), "{body}");
    let (status, body) = h.post_json("/v1/import", json!({ "text": export_file(1) })).await;
    assert_eq!((status, body["error"].clone()), (503, json!("embedder_not_ready")), "{body}");
    let (status, body) = h.post_json("/v1/import", json!({ "text": export_file(1), "dry_run": true })).await;
    assert_eq!(status, 200, "a dry run needs no model: {body}");
    let (status, body) = h.post_json("/v1/import", json!({ "text": export_file(1), "agent": "  " })).await;
    assert_eq!(status, 400, "{body}");
}

/// An engram export of format 0.2.0 holding `observations`.
fn engram_export(observations: Vec<Value>) -> String {
    json!({ "version": "0.2.0", "exported_at": "2025-03-01 10:00:00", "sessions": [], "observations": observations, "prompts": [] })
        .to_string()
}

fn observation(sync_id: &str, project: Option<&str>, title: &str, content: &str) -> Value {
    json!({
        "id": 1, "sync_id": sync_id, "session_id": "s1", "type": "discovery", "title": title, "content": content,
        "project": project, "scope": "project", "revision_count": 1, "duplicate_count": 1,
        "created_at": "2025-01-01 10:00:00", "updated_at": "2025-01-01 10:00:00",
    })
}

/// 700 distinct words, long enough to be split into pieces.
fn long_note() -> String {
    (0..700).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ")
}

impl Harness {
    /// Gives the daemon a tokenizer, as loading the model does.
    fn with_tokenizer(self) -> Self {
        self.ctx.runtime.tokenizer.set(clawg_core::memory::chunker::word_tokenizer()).ok();
        self
    }
}

#[tokio::test]
async fn an_engram_import_stores_notes_and_a_dry_run_reports_the_same() {
    let h = Harness::start().await.with_tokenizer();
    let root = tempfile::tempdir().expect("tempdir");
    let text = engram_export(vec![
        observation("a", Some("app"), "Notes on clawg-ledger", &long_note()),
        observation("b", None, "Port", "the daemon listens on 27380"),
    ]);
    let body = |dry_run| {
        json!({ "text": text, "from": "engram", "dry_run": dry_run, "project_map": [["app", root.path().to_string_lossy()]] })
    };

    let (status, dry) = h.post_json("/v1/import", body(true)).await;
    assert_eq!(status, 200, "{dry}");
    let (_, listed) = h.get_json("/v1/memories?kind=memory&agent=migrated").await;
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(0), "a dry run writes nothing: {listed}");
    let (status, report) = h.post_json("/v1/import", body(false)).await;
    assert_eq!(status, 200, "{report}");

    let unmarked = |mut r: Value| {
        r["dry_run"] = Value::Null;
        r["snapshot"] = Value::Null;
        r["import_id"] = Value::Null;
        r
    };
    assert!(report["snapshot"].is_string(), "{report}");
    assert_eq!((dry["import_id"].clone(), report["import_id"].clone()), (Value::Null, json!(1)), "only what writes has a number");
    assert_eq!(unmarked(dry), unmarked(report.clone()));
    assert_eq!((report["foreign"]["source"].clone(), report["foreign"]["notes"].clone()), (json!("engram"), json!(2)));
    assert_eq!(report["foreign"]["split_notes"], 1, "{report}");
    let inserted = report["inserted"].as_u64().expect("inserted");
    assert!(inserted > 2, "{report}");
    let (_, listed) = h.get_json("/v1/memories?kind=memory&agent=migrated&limit=100").await;
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(inserted as usize), "{listed}");
}

#[tokio::test]
async fn an_engram_export_without_notes_imports_nothing() {
    let h = Harness::start().await.with_tokenizer();
    let text = r#"{"version":"0.2.0","sessions":null,"observations":[],"prompts":null}"#;
    let (status, report) = h.post_json("/v1/import", json!({ "text": text, "from": "engram" })).await;
    assert_eq!(status, 200, "{report}");
    assert_eq!((report["rows"].clone(), report["inserted"].clone(), report["foreign"]["notes"].clone()), (json!(0), json!(0), json!(0)));
    let (_, listed) = h.get_json("/v1/memories?kind=memory").await;
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(0), "{listed}");
}

/// A clawg export of another store: `project_lines`, then one row of each
/// project by its id in the file.
fn export_with(project_lines: &[Value], rows: &[(&str, &str)]) -> String {
    let header = json!({
        "kind": "header", "format": "clawg-memories", "version": 1, "exported_at": 0, "clawg": "0.0.0",
        "filter": {"project": null, "agent": null, "include_deleted": false},
    });
    let mut lines = vec![header];
    lines.extend(project_lines.iter().cloned());
    for (k, (project, content)) in rows.iter().enumerate() {
        let id = k as i64 + 1;
        lines.push(json!({
            "kind": "memory", "id": id, "scope": "agent", "agent_id": "a", "project_id": project, "content": content,
            "memory_type": "semantic_fact", "source": "explicit", "created_at": 1_000 + id,
            "created_by_type": "agent", "last_modified_at": 1_000 + id, "last_accessed_at": 9_000,
            "access_count": 1, "importance": 5.0, "tier": 2, "status": "active",
        }));
    }
    lines.iter().map(|l| format!("{l}\n")).collect()
}

#[tokio::test]
async fn an_export_of_another_store_imports_by_remote_and_by_name() {
    let h = Harness::start().await;
    let base = tempfile::tempdir().expect("tempdir");
    let root = base.path().join("app");
    set_origin(&root, "git@github.com:acme/app.git");
    let app = registered_dir(&h, &root).await["project_id"].as_str().expect("project_id").to_string();
    let ideas = resolve(&h, json!({ "project": "Ideas", "create": true, "born": "user" })).await["project_id"]
        .as_str()
        .expect("a project")
        .to_string();
    let lines = [
        json!({ "kind": "project", "id": "app-0000", "name": "app", "folders": [{ "root_path": "/far/app", "remote": "github.com/acme/app" }], "remotes": ["github.com/acme/app"] }),
        json!({ "kind": "project", "id": "ideas-0000", "name": "ideas" }),
    ];
    let text = export_with(&lines, &[("app-0000", "the app of the other machine uses Rust"), ("ideas-0000", "an idea of the other machine")]);

    let (status, report) = h.post_json("/v1/import", json!({ "text": text })).await;

    assert_eq!(status, 200, "{report}");
    let went: Vec<(Value, Value)> =
        report["projects"].as_array().expect("projects").iter().map(|p| (p["to"].clone(), p["by"].clone())).collect();
    assert_eq!(went, vec![(json!(app), json!("remote")), (json!(ideas), json!("name"))], "{report}");
    assert_eq!(report["by_project"], json!({ app.clone(): 1, ideas.clone(): 1 }), "{report}");
}

#[tokio::test]
async fn a_project_map_to_a_deleted_ambiguous_or_unknown_project_is_refused() {
    let h = Harness::start().await.with_tokenizer();
    let text = engram_export(vec![observation("a", Some("notes"), "Port", "the daemon listens on 27380")]);
    let (gone, _, _) = deleted_project(&h, "/work/gone").await;
    for name in ["twin-1111", "twin-2222"] {
        h.ctx
            .runtime
            .store
            .conn
            .call(move |c| c.execute("INSERT INTO project (id, name, born, reviewed, created_at) VALUES (?1, 'Twin', 'user', 1, 1)", [name]))
            .await
            .unwrap();
    }
    for (project, status, error) in [(gone.as_str(), 409, "project_deleted"), ("twin", 409, "project_ambiguous"), ("nowhere", 404, "")] {
        let ask = json!({ "text": text, "from": "engram", "project_map": [["notes", { "project": project }]] });
        let (got, body) = h.post_json("/v1/import", ask).await;
        assert_eq!(got, status, "{project}: {body}");
        if !error.is_empty() {
            assert_eq!(body["error"], json!(error), "{project}: {body}");
        }
    }
    let (status, body) = h.post_json("/v1/import", json!({ "text": export_file(1), "map_roots": [["/a", { "project": "twin-1111" }]] })).await;
    assert_eq!(status, 400, "a root maps to a root: {body}");
    let (_, listed) = h.get_json("/v1/memories?kind=memory&agent=migrated").await;
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(0), "{listed}");
}

#[tokio::test]
async fn a_map_given_as_a_root_is_read_as_before() {
    let h = Harness::start().await.with_tokenizer();
    let root = tempfile::tempdir().expect("tempdir");
    let root = canonical(root.path());
    let text = engram_export(vec![observation("a", Some("notes"), "Port", "the daemon listens on 27380")]);
    let as_text = json!({ "text": text, "from": "engram", "project_map": [["notes", root.to_string_lossy()]] });
    let (status, before) = h.post_json("/v1/import", as_text).await;
    assert_eq!(status, 200, "{before}");
    let (_, listed) = h.get_json("/v1/memories?kind=memory&agent=migrated").await;
    let id = ids_in(&listed["items"])[0];
    let project = project_of(&h, id).await;
    assert!(project.is_string(), "{listed}");

    let text = engram_export(vec![observation("b", Some("notes"), "Host", "the daemon binds 127.0.0.1")]);
    let as_object = json!({ "text": text, "from": "engram", "project_map": [["notes", { "root": root.to_string_lossy() }]] });
    let (status, after) = h.post_json("/v1/import", as_object).await;
    assert_eq!(status, 200, "{after}");
    let (_, listed) = h.get_json("/v1/memories?kind=memory&agent=migrated").await;
    for id in ids_in(&listed["items"]) {
        assert_eq!(project_of(&h, id).await, project, "both forms lead to the root's project");
    }
}

#[tokio::test]
async fn import_refuses_an_unknown_source_or_a_mixed_map() {
    let h = Harness::start().await.with_tokenizer();
    let engram = engram_export(vec![observation("a", None, "Port", "the daemon listens on 27380")]);
    let map = json!([["app", "/work/app"]]);
    for (body, names) in [
        (json!({ "text": engram, "from": "notion" }), "notion"),
        (json!({ "text": export_file(1), "from": "clawg", "project_map": map }), "project_map"),
        (json!({ "text": export_file(1), "project_map": map }), "project_map"),
        (json!({ "text": engram, "from": "engram", "map_roots": [["/a", "/b"]] }), "map_roots"),
    ] {
        let (status, answer) = h.post_json("/v1/import", body).await;
        assert_eq!(status, 400, "{answer}");
        assert!(answer["error"].as_str().unwrap_or("").contains(names), "{answer}");
    }
    let (_, listed) = h.get_json("/v1/memories?kind=memory").await;
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(0), "{listed}");
}

#[tokio::test]
async fn an_engram_export_without_from_is_a_400() {
    let h = Harness::start().await.with_tokenizer();
    let engram = engram_export(vec![observation("a", None, "Port", "the daemon listens on 27380")]);
    let (status, answer) = h.post_json("/v1/import", json!({ "text": engram })).await;
    assert_eq!(status, 400, "{answer}");
    assert!(answer["error"].as_str().unwrap_or("").contains("header"), "{answer}");
}

#[tokio::test]
async fn a_foreign_import_without_the_tokenizer_is_a_503() {
    let h = Harness::start_without_embedder().await;
    for (text, names) in [("{not json", "not JSON"), (r#"{"version":"0.3.0"}"#, "0.3.0")] {
        let (status, answer) = h.post_json("/v1/import", json!({ "text": text, "from": "engram" })).await;
        assert_eq!(status, 400, "a file the adapter cannot read is refused first: {answer}");
        assert!(answer["error"].as_str().unwrap_or("").contains(names), "{answer}");
    }
    let engram = engram_export(vec![observation("a", None, "Port", "the daemon listens on 27380")]);
    for dry_run in [false, true] {
        let (status, answer) = h.post_json("/v1/import", json!({ "text": engram, "from": "engram", "dry_run": dry_run })).await;
        assert_eq!((status, answer["error"].clone()), (503, json!("tokenizer_not_ready")), "dry run {dry_run}: {answer}");
    }

    let h = h.with_tokenizer();
    let (status, answer) = h.post_json("/v1/import", json!({ "text": engram, "from": "engram", "dry_run": true })).await;
    assert_eq!(status, 200, "a dry run needs no model: {answer}");
    let (status, answer) = h.post_json("/v1/import", json!({ "text": engram, "from": "engram" })).await;
    assert_eq!((status, answer["error"].clone()), (503, json!("embedder_not_ready")), "{answer}");
}

#[tokio::test]
async fn an_imported_long_note_reads_whole_and_changes_for_the_bridge_agent() {
    let h = Harness::start().await.with_tokenizer();
    let text = engram_export(vec![observation("a", None, "Notes on clawg-ledger", &long_note())]);
    let (status, report) = h.post_json("/v1/import", json!({ "text": text, "from": "engram" })).await;
    assert_eq!(status, 200, "{report}");
    let (_, listed) = h.get_json("/v1/memories?kind=memory&agent=migrated&limit=100").await;
    let ids: Vec<i64> = listed["items"].as_array().expect("items").iter().filter_map(|m| m["id"].as_i64()).collect();
    assert!(ids.len() > 1, "{listed}");
    let id = ids[ids.len() / 2];

    let (status, save) = h.get_json(&format!("/v1/memories/memory/{id}/save?agent=claude-code")).await;
    assert_eq!(status, 200, "{save}");
    assert_eq!(save["text"], format!("Notes on clawg-ledger\n{}", long_note()), "{save}");
    let patch = h.http.patch(h.url(&format!("/v1/memories/memory/{id}?agent=claude-code"))).json(&json!({ "importance": 8.0 }));
    let (status, patched) = h.send(patch).await;
    assert_eq!(status, 200, "{patched}");
    let (status, deleted) = h.send(h.http.delete(h.url(&format!("/v1/memories/memory/{id}?agent=claude-code")))).await;
    assert_eq!(status, 200, "{deleted}");
}

#[tokio::test]
async fn an_unkeyed_save_names_the_row_a_key_would_revise() {
    let vectors = vec![
        ("builds with cargo", at_cosine(1.0, 1)),
        ("builds with make", at_cosine(0.96, 2)),
        ("builds with ninja", at_cosine(0.96, 3)),
    ];
    let h = Harness::start_inner(EmbedderState::Fixed(vectors)).await;
    let save = |content: &str, key: Option<&str>| {
        let mut body = json!({ "content": content, "agent_id": "a", "project_id": "p", "memory_type": "semantic_fact" });
        if let Some(key) = key {
            body["topic_key"] = json!(key);
        }
        body
    };
    let older = save_id(&h, save("the project builds with cargo", None)).await;

    let (status, unkeyed) = h.post_json("/v1/memories", save("the project builds with make", None)).await;
    assert_eq!(status, 201, "{unkeyed}");
    assert_eq!(unkeyed["related"][0]["id"], older, "{unkeyed}");
    assert_eq!(unkeyed["revision_candidate"], older, "{unkeyed}");

    let (status, keyed) = h.post_json("/v1/memories", save("the project builds with ninja", Some("build/tool"))).await;
    assert_eq!(status, 201, "{keyed}");
    assert!(keyed["related"].as_array().is_some_and(|r| !r.is_empty()), "{keyed}");
    assert!(keyed.get("revision_candidate").is_none(), "a key already revises: {keyed}");
}

// ── Metrics ─────────────────────────────────────────────────────────────────

/// The `http.duration` series of the run, as label strings with their count.
fn http_durations(h: &Harness) -> Vec<(String, u64)> {
    h.ctx
        .runtime
        .metrics
        .run()
        .iter()
        .filter(|(key, _)| key.name == "http.duration")
        .map(|(key, histogram)| (key.labels.clone(), histogram.count()))
        .collect()
}

fn count_of(series: &[(String, u64)], labels: &str) -> u64 {
    series.iter().find(|(l, _)| l == labels).map(|(_, n)| *n).unwrap_or(0)
}

#[tokio::test]
async fn every_request_is_timed_by_route_status_and_transport() {
    let h = Harness::start().await;
    h.http.get(h.url("/health")).send().await.expect("health");
    let (status, _) = h.post_json("/v1/search", json!({ "query": "anything", "agent_id": "a" })).await;
    assert_eq!(status, 200);
    let (status, _) = h.get_json("/v1/memories/memory/999999").await;
    assert_eq!(status, 404);

    let series = http_durations(&h);
    assert_eq!(count_of(&series, "method=GET,route=/health,status=2xx,transport=tcp"), 1, "{series:?}");
    assert_eq!(count_of(&series, "method=POST,route=/v1/search,status=2xx,transport=tcp"), 1, "{series:?}");
    assert_eq!(count_of(&series, "method=GET,route=/v1/memories/{kind}/{id},status=4xx,transport=tcp"), 1, "{series:?}");

    let run = h.ctx.runtime.metrics.run();
    let bytes = run
        .iter()
        .find(|(key, _)| key.name == "http.request_bytes" && key.labels == "method=POST,route=/v1/search")
        .map(|(_, histogram)| histogram.clone())
        .expect("the size of the search is recorded");
    assert!(bytes.max() > 0, "the size comes from Content-Length");
    let minute = h.ctx.runtime.metrics.minute();
    let in_flight = minute.gauges.iter().find(|(key, _)| key.name == "http.in_flight").expect("in flight").1;
    assert!(in_flight.max >= 1.0 && in_flight.last == 0.0, "{in_flight:?}");
}

/// The samples and the last value of a gauge of the minute in progress.
fn gauge_of(h: &Harness, name: &str, labels: &[(&str, &str)]) -> Option<(u64, f64)> {
    let key = clawg_core::metrics::SeriesKey::new(name, labels);
    h.ctx.runtime.metrics.minute().gauges.get(&key).map(|g| (g.count, g.last))
}

#[tokio::test]
async fn a_search_records_its_arms_its_hits_and_what_it_withheld() {
    let h = Harness::start().await;
    save_id(&h, json!({ "content": ENGLISH_NOTE, "agent_id": "a" })).await;
    let (status, body) = h.post_json("/v1/search", json!({ "query": "deploy", "agent_id": "a" })).await;
    assert_eq!(status, 200, "{body}");

    let shown = body["hits"].as_array().unwrap().len() as f64;
    let withheld = body["withheld"].as_f64().unwrap();
    assert_eq!(gauge_of(&h, "search.hits", &[]), Some((1, shown)));
    assert_eq!(gauge_of(&h, "search.withheld", &[]), Some((1, withheld)));
}

#[tokio::test]
async fn a_server_marked_unix_labels_its_requests_so() {
    let h = Harness::start().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = clawg_cli::daemon::api::router_on(h.ctx.clone(), clawg_cli::daemon::metrics::Transport::Unix);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    h.http.get(format!("http://{addr}/health")).send().await.expect("health");

    let series = http_durations(&h);
    assert_eq!(count_of(&series, "method=GET,route=/health,status=2xx,transport=unix"), 1, "{series:?}");
}

#[tokio::test]
async fn refused_requests_are_timed_too() {
    let h = Harness::start().await;
    let r = h.http.post(h.url("/v1/search")).json(&json!({ "query": "q" })).send().await.expect("request");
    assert_eq!(r.status(), 401);
    let r = h.http.get(h.url("/health")).header("origin", "http://evil.example").send().await.expect("request");
    assert_eq!(r.status(), 403);

    let series = http_durations(&h);
    assert_eq!(count_of(&series, "method=POST,route=/v1/search,status=4xx,transport=tcp"), 1, "{series:?}");
    assert_eq!(count_of(&series, "method=GET,route=/health,status=4xx,transport=tcp"), 1, "{series:?}");
}

#[tokio::test]
async fn no_label_comes_from_the_request() {
    let h = Harness::start().await;
    h.authed(h.http.get(h.url("/v1/memories/memory/424242?agent=secretagent")))
        .body("zebra-colored secret text")
        .send()
        .await
        .expect("request");
    h.authed(h.http.post(h.url("/nowhere/qwerty"))).body("zebra").send().await.expect("request");
    h.authed(h.http.request(reqwest::Method::from_bytes(b"BREW").unwrap(), h.url("/health")))
        .send()
        .await
        .expect("request");

    let labels: Vec<String> = h.ctx.runtime.metrics.run().keys().map(|key| key.labels.clone()).collect();
    for label in &labels {
        for leaked in ["424242", "secretagent", "zebra", "qwerty", "nowhere", "BREW"] {
            assert!(!label.contains(leaked), "{label} carries {leaked}");
        }
    }
    assert!(labels.iter().any(|l| l.contains("route=/v1/memories/{kind}/{id}")), "{labels:?}");
    assert!(labels.iter().any(|l| l.contains("method=POST,route=unmatched")), "{labels:?}");
    assert!(labels.iter().any(|l| l.contains("method=other")), "{labels:?}");
}

#[tokio::test]
async fn metrics_needs_the_token_and_answers_the_minute_and_the_run() {
    let h = Harness::start().await;
    let r = h.http.get(h.url("/v1/metrics")).send().await.expect("request");
    assert_eq!(r.status(), 401);
    h.http.get(h.url("/health")).send().await.expect("health");

    let (status, body) = h.get_json("/v1/metrics").await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["version"], clawg_core::build_info::version());
    assert!(body["started_at_ms"].as_u64().is_some_and(|ms| ms > 0), "{body}");
    assert_eq!(body["writing"], false, "a daemon in a test keeps no file");
    let health = |list: &Value| {
        list.as_array()
            .and_then(|l| {
                l.iter().find(|s| s["name"] == "http.duration" && s["labels"]["route"] == "/health" && s["labels"]["status"] == "2xx")
            })
            .cloned()
    };
    let minute = health(&body["minute"]["histograms"]).expect("the health check of this minute");
    assert_eq!(minute["unit"], "us");
    assert_eq!(minute["count"], 1);
    assert_eq!(minute["labels"]["transport"], "tcp");
    assert!(minute["p50"].as_u64().is_some() && minute["p99"].as_u64().is_some(), "{minute}");
    assert!(minute["buckets"].as_array().is_some_and(|b| !b.is_empty()), "{minute}");
    let run = health(&body["run"]["histograms"]).expect("the health check of the run");
    assert_eq!(run["count"], 1);
    assert!(body["minute"]["gauges"].as_array().is_some_and(|g| g.iter().any(|g| g["name"] == "http.in_flight")), "{body}");
}

#[tokio::test]
async fn metrics_speaks_prometheus_text() {
    let h = Harness::start().await;
    h.http.get(h.url("/health")).send().await.expect("health");

    let r = h.authed(h.http.get(h.url("/v1/metrics?format=prometheus"))).send().await.expect("request");

    assert_eq!(r.status(), 200);
    let kind = r.headers()["content-type"].to_str().unwrap().to_string();
    assert!(kind.starts_with("text/plain") && kind.contains("version=0.0.4"), "{kind}");
    let text = r.text().await.expect("text");
    assert!(text.contains("# TYPE clawg_http_duration_seconds histogram"), "{text}");
    let health = r#"method="GET",route="/health",status="2xx",transport="tcp""#;
    assert!(text.contains(&format!("clawg_http_duration_seconds_count{{{health}}} 1")), "{text}");
    assert!(text.contains(&format!("clawg_http_duration_seconds_bucket{{{health},le=\"+Inf\"}} 1")), "{text}");
    assert!(text.contains(&format!("clawg_http_duration_seconds_bucket{{{health},le=\"0.00005\"}}")), "{text}");
    assert!(text.contains("# TYPE clawg_http_request_bytes histogram"), "{text}");
    assert!(text.contains("# TYPE clawg_http_in_flight gauge"), "{text}");
    let r = h.authed(h.http.get(h.url("/v1/metrics?format=yaml"))).send().await.expect("request");
    assert_eq!(r.status(), 400);
}

// ── Moving memories between projects ───────────────────────────────────────

/// Registers `root` with the daemon and answers its project id.
async fn registered(h: &Harness, root: &str) -> String {
    let (status, body) = h.post_json("/v1/projects", json!({ "root_path": root })).await;
    assert_eq!(status, 200, "{body}");
    body["project_id"].as_str().expect("project_id").to_string()
}

async fn patch(h: &Harness, id: i64, body: Value) -> (u16, Value) {
    let r = h
        .authed(h.http.patch(h.url(&format!("/v1/memories/memory/{id}"))))
        .json(&body)
        .send()
        .await
        .expect("request");
    let status = r.status().as_u16();
    (status, r.json().await.unwrap_or(Value::Null))
}

async fn project_of(h: &Harness, id: i64) -> Value {
    let (_, row) = h.get_json(&format!("/v1/memories/memory/{id}")).await;
    row["item"]["project_id"].clone()
}

fn saved_ids(body: &Value) -> Vec<i64> {
    body["memories"].as_array().expect("memories").iter().map(|m| m["id"].as_i64().unwrap()).collect()
}

#[tokio::test]
async fn patch_moves_a_whole_save() {
    let h = Harness::start().await;
    let target = registered(&h, "/work/target").await;
    let content = (0..600).map(|i| format!("w{i:05}")).collect::<Vec<_>>().join(" ");
    let (_, body) = h.post_json("/v1/memories", json!({ "content": content, "agent_id": "a" })).await;
    let pieces = saved_ids(&body);

    let (status, body) = patch(&h, pieces[3], json!({ "project_id": target })).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["project_id"], json!(target), "{body}");
    assert_eq!(body["move_id"], 1, "{body}");
    assert_eq!(body["pulled_in"].as_array().map(Vec::len), Some(pieces.len() - 1), "{body}");
    for id in pieces {
        assert_eq!(project_of(&h, id).await, json!(target));
    }
}

#[tokio::test]
async fn patch_with_a_clash_is_409_and_changes_nothing() {
    let h = Harness::start().await;
    let target = registered(&h, "/work/target").await;
    let (_, body) = h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a" })).await;
    let loose = saved_ids(&body)[0];
    let (_, body) = h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a", "project_id": target })).await;
    let twin = saved_ids(&body)[0];

    let (status, body) = patch(&h, loose, json!({ "project_id": target, "importance": 9.0 })).await;

    assert_eq!(status, 409, "{body}");
    assert!(body["error"].as_str().unwrap_or_default().contains(&format!("#{twin}")), "{body}");
    assert_eq!(project_of(&h, loose).await, Value::Null);
    let (_, row) = h.get_json(&format!("/v1/memories/memory/{loose}")).await;
    assert_ne!(row["item"]["importance"], 9.0, "{row}");
}

#[tokio::test]
async fn patch_with_both_destinations_is_400() {
    let h = Harness::start().await;
    let target = registered(&h, "/work/target").await;
    let (_, body) = h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a" })).await;
    let id = saved_ids(&body)[0];

    let (status, body) = patch(&h, id, json!({ "project_id": target, "no_project": true })).await;

    assert_eq!(status, 400, "{body}");
}

#[tokio::test]
async fn a_move_to_an_unregistered_project_is_404() {
    let h = Harness::start().await;
    let (_, body) = h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a" })).await;
    let id = saved_ids(&body)[0];

    let (status, body) = patch(&h, id, json!({ "project_id": "typo-0000" })).await;

    assert_eq!(status, 404, "{body}");
    assert!(body["error"].as_str().unwrap_or_default().contains("typo-0000"), "{body}");
}

#[tokio::test]
async fn patch_moves_while_the_embedder_loads() {
    let h = Harness::start_serving_without_embedder().await;
    let target = registered(&h, "/work/target").await;
    h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a", "project_id": target })).await;
    let id = h.wait_for_rows("memory", 1).await[0]["id"].as_i64().unwrap();

    let (status, body) = patch(&h, id, json!({ "no_project": true })).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(project_of(&h, id).await, Value::Null);
}

#[tokio::test]
async fn the_listing_reaches_the_rows_without_a_project() {
    let h = Harness::start().await;
    let target = registered(&h, "/work/target").await;
    h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "a", "project_id": target })).await;
    let (_, body) = h.post_json("/v1/memories", json!({ "content": "a note filed nowhere", "agent_id": "a" })).await;
    let loose = saved_ids(&body)[0];

    let (status, body) = h.get_json("/v1/memories?no_project=true").await;
    assert_eq!(status, 200, "{body}");
    let ids: Vec<i64> = body["items"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![loose]);

    let (status, body) = h.get_json(&format!("/v1/memories?no_project=true&project={target}")).await;
    assert_eq!(status, 400, "{body}");
}

#[tokio::test]
async fn the_batch_moves_reports_and_snapshots() {
    let h = Harness::start().await;
    let target = registered(&h, "/work/target").await;
    let mut loose = Vec::new();
    for content in ["the first unfiled note", "the second unfiled note"] {
        let (_, body) = h.post_json("/v1/memories", json!({ "content": content, "agent_id": "a" })).await;
        loose.extend(saved_ids(&body));
    }
    let request = json!({ "from_none": true, "to_project": target });

    let (status, dry) = h.post_json("/v1/memories/move", json!({ "from_none": true, "to_project": target, "dry_run": true })).await;
    assert_eq!(status, 200, "{dry}");
    assert_eq!(dry["moved"].as_array().map(Vec::len), Some(2), "{dry}");
    assert_eq!(project_of(&h, loose[0]).await, Value::Null, "a dry run moves nothing");

    let (status, done) = h.post_json("/v1/memories/move", request).await;
    assert_eq!(status, 200, "{done}");
    assert_eq!(done["move_id"], 1, "{done}");
    let snapshot = done["snapshot"].as_str().expect("snapshot");
    assert!(snapshot.ends_with("-pre-move.db") && std::path::Path::new(snapshot).exists(), "{snapshot}");
    for &id in &loose {
        assert_eq!(project_of(&h, id).await, json!(target));
    }

    let (status, undone) = h.post_json("/v1/memories/move", json!({ "undo": 1 })).await;
    assert_eq!(status, 200, "{undone}");
    assert_eq!(undone["move_id"], 2, "{undone}");
    assert_eq!(project_of(&h, loose[1]).await, Value::Null);
}

#[tokio::test]
async fn move_and_list_select_by_origin_project() {
    let h = Harness::start().await;
    let target = registered(&h, "/work/target").await;
    let from = |origin: &'static str, content: &'static str| {
        let h = &h;
        async move {
            let (_, body) = h.post_json("/v1/memories", json!({ "content": content, "agent_id": "a" })).await;
            let id = saved_ids(&body)[0];
            h.ctx
                .runtime
                .store
                .conn
                .call(move |c| c.execute("UPDATE memory SET origin_project = ?1 WHERE id = ?2", rusqlite::params![origin, id]))
                .await
                .expect("seed");
            id
        }
    };
    let (a1, a2) = (from("a", "the first note of a").await, from("a", "the second note of a").await);
    let b = from("b", "the only note of b").await;
    let ids_of = |body: &Value| -> Vec<i64> {
        let mut ids: Vec<i64> = body["items"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect();
        ids.sort_unstable();
        ids
    };

    let (status, body) = h.get_json("/v1/memories?origin_project=a").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(ids_of(&body), vec![a1, a2]);
    let (_, body) = h.get_json("/v1/memories?origin_project=b&agent=a").await;
    assert_eq!(ids_of(&body), vec![b]);

    let (status, dry) = h.post_json("/v1/memories/move", json!({ "origin_project": "a", "to_project": target, "dry_run": true })).await;
    assert_eq!(status, 200, "{dry}");
    assert_eq!(dry["moved"].as_array().map(Vec::len), Some(2), "{dry}");

    let (status, done) = h.post_json("/v1/memories/move", json!({ "origin_project": "a", "to_project": target })).await;
    assert_eq!(status, 200, "{done}");
    assert_eq!(project_of(&h, a1).await, json!(target));
    assert_eq!(project_of(&h, a2).await, json!(target));
    assert_eq!(project_of(&h, b).await, Value::Null);

    let (status, answer) = h.post_json("/v1/memories/move", json!({ "origin_project": "a", "undo": 1 })).await;
    assert_eq!(status, 400, "an undo takes no selection: {answer}");
}

#[tokio::test]
async fn a_batch_without_a_selection_or_a_target_is_400() {
    let h = Harness::start().await;

    for body in [json!({ "to_none": true }), json!({ "ids": [1] }), json!({ "ids": [1], "to_none": true, "undo": 1 }), json!({ "ids": [1], "to_none": true, "projects": "x" })] {
        let (status, answer) = h.post_json("/v1/memories/move", body.clone()).await;
        assert!(status == 400 || status == 422, "{body} -> {status} {answer}");
    }
    let (status, answer) = h.post_json("/v1/memories/move", json!({ "undo": 9 })).await;
    assert_eq!(status, 404, "{answer}");
}

// ── Merged projects ─────────────────────────────────────────────────────────

/// What each route of `router()` takes of a project, as `api.rs` writes the
/// route: the field that holds a project id, which must lead past a merge's
/// tombstone to the live project, or none. A route the router gains fails
/// `every_route_says_which_project_it_takes` until it is here, and one that
/// takes a project fails `every_route_that_takes_a_project_follows_a_merge`
/// until that test checks it.
const PROJECT_INPUTS: &[(&str, &str, Option<&str>)] = &[
    ("GET", "/health", None),
    ("POST", "/memories", Some("body.project_id")),
    ("GET", "/memories", Some("query.project")),
    ("POST", "/memories/move", Some("body.to_project")),
    ("POST", "/search", Some("body.project_id")),
    ("POST", "/context", Some("body.project_id")),
    ("POST", "/sessions", Some("body.project_id")),
    ("GET", "/memories/{kind}/{id}", None),
    ("PATCH", "/memories/{kind}/{id}", Some("body.project_id")),
    ("DELETE", "/memories/{kind}/{id}", None),
    ("GET", "/memories/{kind}/{id}/save", None),
    ("POST", "/memories/{kind}/{id}/restore", None),
    ("GET", "/disputes", Some("query.project_id")),
    ("POST", "/disputes/resolve", None),
    ("POST", "/disputes/judge", None),
    ("POST", "/synthesis", Some("body.project_id")),
    // The worker follows the project of the save it takes back.
    ("POST", "/outbox/retry", None),
    ("POST", "/outbox/discard", None),
    ("GET", "/stats", Some("query.project")),
    ("GET", "/metrics", None),
    ("POST", "/backup", None),
    // Roots, whose projects the plan and the writes follow, and the
    // projects another tool's projects are mapped to.
    ("POST", "/import", Some("body.project_map[].project")),
    ("POST", "/import/undo", None),
    // A root, whose project the registration follows.
    ("POST", "/projects", None),
    ("GET", "/projects", None),
    ("PATCH", "/projects/{id}", Some("path.id")),
    ("POST", "/projects/merge", Some("body.target")),
    ("POST", "/projects/keep-apart", Some("body.project_id")),
    ("POST", "/projects/delete", Some("body.project_id")),
    ("POST", "/projects/resolve", Some("body.project")),
    ("POST", "/projects/suggestions/answer", Some("body.project_id")),
    ("POST", "/projects/locate", Some("body.project")),
    // A folder's root id, which a merge does not change.
    ("POST", "/projects/docs/search", None),
    ("POST", "/shutdown", None),
];

/// `(method, path)` of every route `router()` declares, read from its source.
fn routes_in_router() -> Vec<(String, String)> {
    // A Windows checkout ends its lines with CRLF.
    let source = include_str!("../src/daemon/api.rs").replace("\r\n", "\n");
    let start = source.find("pub fn router(").expect("router() in api.rs");
    let body = &source[start..];
    let body = &body[..body.find("\n}\n").expect("the end of router()")];
    let mut routes = Vec::new();
    for segment in body.split(".route(").skip(1) {
        let path = segment.split('"').nth(1).expect("a quoted path");
        for method in ["get", "post", "patch", "delete", "put"] {
            let call = format!("{method}(");
            let called = segment
                .match_indices(&call)
                .any(|(at, _)| !segment[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_'));
            if called {
                routes.push((method.to_uppercase(), path.to_string()));
            }
        }
    }
    routes.sort();
    routes
}

#[test]
fn every_route_says_which_project_it_takes() {
    let mut table: Vec<(String, String)> = PROJECT_INPUTS.iter().map(|(m, p, _)| (m.to_string(), p.to_string())).collect();
    table.sort();
    assert_eq!(routes_in_router(), table, "router() and PROJECT_INPUTS name different routes");
}

/// Merges `source` into `target` as `clawg projects merge` does.
async fn merge_projects(h: &Harness, source: &str, target: &str) -> clawg_core::memory::merge::MergeReport {
    let backups = clawg_core::paths::backups_dir(&h.ctx.runtime.base_dir);
    clawg_core::memory::merge::merge_core(&h.ctx.runtime.store, &h.ctx.runtime.workspaces(), &backups, source, target, false)
        .await
        .expect("merge")
}

/// Saves `content` under `project` and answers its id.
async fn save_in(h: &Harness, project: &str, content: &str) -> i64 {
    let (status, body) =
        h.post_json("/v1/memories", json!({ "agent_id": "agent-http", "content": content, "project_id": project })).await;
    assert_eq!(status, 201, "{body}");
    saved_ids(&body)[0]
}

fn ids_in(rows: &Value) -> Vec<i64> {
    rows.as_array().expect("rows").iter().filter_map(|r| r["id"].as_i64()).collect()
}

#[tokio::test]
async fn every_route_that_takes_a_project_follows_a_merge() {
    let h = Harness::start().await;
    let (app, web, other) =
        (registered(&h, "/work/app").await, registered(&h, "/work/web").await, registered(&h, "/work/other").await);
    save_in(&h, &app, "the parser of app is written by hand").await;
    let on_web = save_in(&h, &web, "the web server listens on port 8080").await;
    save_in(&h, &other, "the other service is written in Go").await;
    merge_projects(&h, &app, &web).await;

    for (method, path, field) in PROJECT_INPUTS {
        let Some(field) = field else { continue };
        let route = format!("{method} {path} ({field})");
        match (*method, *path) {
            ("POST", "/memories") => {
                let id = save_in(&h, &app, "a note saved after the merge").await;
                assert_eq!(project_of(&h, id).await, json!(web), "{route}");
            }
            ("GET", "/memories") => {
                let (status, body) = h.get_json(&format!("/v1/memories?project={app}")).await;
                assert_eq!(status, 200, "{route}: {body}");
                assert!(ids_in(&body["items"]).contains(&on_web), "{route}: {body}");
            }
            ("POST", "/memories/move") => {
                let row = save_in(&h, &other, "a note to move").await;
                let (status, body) = h.post_json("/v1/memories/move", json!({ "ids": [row], "to_project": app })).await;
                assert_eq!(status, 200, "{route}: {body}");
                assert_eq!(project_of(&h, row).await, json!(web), "{route}");
            }
            ("POST", "/search") => {
                let ask = json!({ "agent_id": "agent-http", "query": "web server port 8080", "project_id": app });
                let (status, body) = h.post_json("/v1/search", ask).await;
                assert_eq!(status, 200, "{route}: {body}");
                assert!(ids_in(&body["hits"]).contains(&on_web), "{route}: {body}");
            }
            ("POST", "/context") => {
                // Without a query it lists the caller's recent saves of the
                // project, which the fake embedder cannot rank.
                let ask = json!({ "agent_id": "agent-http", "project_id": app });
                let (status, body) = h.post_json("/v1/context", ask).await;
                assert_eq!(status, 200, "{route}: {body}");
                assert!(body["prompt"].as_str().is_some_and(|p| p.contains("listens on port 8080")), "{route}: {body}");
            }
            ("POST", "/sessions") => {
                let (status, body) = h
                    .post_json("/v1/sessions", json!({ "agent_id": "agent-http", "project_id": app, "session_id": "s-merged" }))
                    .await;
                assert_eq!(status, 204, "{route}: {body}");
                let id = save_in(&h, &web, "a note of that session").await;
                assert_eq!(session_of(&h, id).await.as_deref(), Some("s-merged"), "{route}");
            }
            ("PATCH", "/memories/{kind}/{id}") => {
                let row = save_in(&h, &other, "a note to patch").await;
                let (status, body) = patch(&h, row, json!({ "project_id": app })).await;
                assert_eq!(status, 200, "{route}: {body}");
                assert_eq!(project_of(&h, row).await, json!(web), "{route}");
            }
            ("GET", "/disputes") => {
                let older = save_in(&h, &web, "the port of the web server is 8080").await;
                h.ctx
                    .runtime
                    .store
                    .conn
                    .call(move |c| {
                        c.execute(
                            "INSERT INTO memory_conflict (new_id, old_id, distance, state, created_at) VALUES (?1, ?2, 0.3, 'pending', 1)",
                            [older, on_web],
                        )
                    })
                    .await
                    .expect("note the pair");
                let pending = |project: String| {
                    let h = &h;
                    async move { h.get_json(&format!("/v1/disputes?agent_id=agent-http&project_id={project}")).await.1 }
                };
                let (from_app, from_web, from_other) = (pending(app.clone()).await, pending(web.clone()).await, pending(other.clone()).await);
                assert_eq!(from_app, from_web, "{route}");
                assert!(from_app["pending"].as_i64() > from_other["pending"].as_i64(), "{route}: {from_app} {from_other}");
            }
            ("POST", "/synthesis") => {
                let (status, body) = h.post_json("/v1/synthesis", json!({ "project_id": app })).await;
                assert_eq!(status, 200, "{route}: {body}");
                assert!(ids_in(&body["pending"]).contains(&on_web), "{route}: {body}");
            }
            ("GET", "/stats") => {
                let (_, from_app) = h.get_json(&format!("/v1/stats?project={app}")).await;
                let (_, from_web) = h.get_json(&format!("/v1/stats?project={web}")).await;
                assert_eq!(from_app, from_web, "{route}");
            }
            ("PATCH", "/projects/{id}") => {
                let (status, body) = h.rename(&app, "App").await;
                assert_eq!((status, body["error"].as_str()), (409, Some("project_merged")), "{route}: {body}");
                assert!(body["detail"].as_str().is_some_and(|d| d.contains(&web)), "{route}: {body}");
            }
            ("POST", "/projects/merge") => {
                let (status, body) = h.post_json("/v1/projects/merge", json!({ "source": other, "target": app })).await;
                assert_eq!((status, body["error"].as_str()), (409, Some("project_merged")), "{route}: {body}");
                assert!(body["detail"].as_str().is_some_and(|d| d.contains(&web)), "{route}: {body}");
            }
            ("POST", "/projects/keep-apart") => {
                let (status, body) = h.post_json("/v1/projects/keep-apart", json!({ "project_id": app, "other_id": other })).await;
                assert_eq!((status, body["error"].as_str()), (409, Some("project_merged")), "{route}: {body}");
            }
            ("POST", "/projects/delete") => {
                let (status, body) = h.post_json("/v1/projects/delete", json!({ "project_id": app })).await;
                assert_eq!((status, body["error"].as_str()), (409, Some("project_merged")), "{route}: {body}");
                assert!(body["detail"].as_str().is_some_and(|d| d.contains(&web)), "{route}: {body}");
            }
            ("POST", "/projects/resolve") => {
                let (status, body) = h.post_json("/v1/projects/resolve", json!({ "project": app })).await;
                assert_eq!((status, body["project_id"].as_str()), (200, Some(web.as_str())), "{route}: {body}");
            }
            ("POST", "/projects/suggestions/answer") => {
                let answer = json!({ "project_id": app, "other_id": other, "answer": "apart", "user_answered": true, "reason": "two things" });
                let (status, body) = h.post_json("/v1/projects/suggestions/answer", answer).await;
                assert_eq!((status, body["error"].as_str()), (409, Some("project_merged")), "{route}: {body}");
            }
            ("POST", "/projects/locate") => {
                let folder = tempfile::tempdir().expect("tempdir");
                let (status, body) =
                    h.post_json("/v1/projects/locate", json!({ "project": app, "root_path": folder.path().to_string_lossy() })).await;
                assert_eq!((status, body["error"].as_str()), (409, Some("project_merged")), "{route}: {body}");
                assert!(body["detail"].as_str().is_some_and(|d| d.contains(&web)), "{route}: {body}");
                let sub = checkout_with(folder.path(), "apps/web");
                let ask = json!({ "project": app, "root_path": sub.to_string_lossy(), "subfolder": true });
                let (status, body) = h.post_json("/v1/projects/locate", ask).await;
                assert_eq!((status, body["error"].as_str()), (409, Some("project_merged")), "{route} subfolder: {body}");
                assert!(body["detail"].as_str().is_some_and(|d| d.contains(&web)), "{route} subfolder: {body}");
            }
            ("POST", "/import") => {
                h.ctx.runtime.tokenizer.set(clawg_core::memory::chunker::word_tokenizer()).ok();
                let text = engram_export(vec![observation("merged-map", Some("notes"), "Port", "the mapped service listens on 7070")]);
                let ask = json!({ "text": text, "from": "engram", "project_map": [["notes", { "project": app }]] });
                let (status, body) = h.post_json("/v1/import", ask).await;
                assert_eq!(status, 200, "{route}: {body}");
                let (_, listed) = h.get_json("/v1/memories?kind=memory&agent=migrated&limit=10").await;
                let id = ids_in(&listed["items"])[0];
                assert_eq!(project_of(&h, id).await, json!(web), "{route}");
            }
            _ => panic!("{route} takes a project and no check here follows it past a merge"),
        }
    }
}

#[tokio::test]
async fn a_bridge_opened_before_a_merge_reads_and_writes_its_target() {
    let h = Harness::start().await;
    // What the bridge registered at its start, and keeps.
    let app = registered(&h, "/work/app").await;
    let early = save_in(&h, &app, "the parser of app is written by hand").await;
    let web = registered(&h, "/work/web").await;
    merge_projects(&h, &app, &web).await;

    let late = save_in(&h, &app, "the parser of app has a fuzz test").await;

    assert_eq!((project_of(&h, early).await, project_of(&h, late).await), (json!(web), json!(web)));
    let ask = json!({ "agent_id": "agent-http", "query": "parser of app", "project_id": app });
    let (_, found) = h.post_json("/v1/search", ask).await;
    let ids = ids_in(&found["hits"]);
    assert!(ids.contains(&early) && ids.contains(&late), "{found}");
    let (_, context) = h.post_json("/v1/context", json!({ "agent_id": "agent-http", "project_id": app })).await;
    assert!(context["prompt"].as_str().is_some_and(|p| p.contains("has a fuzz test")), "{context}");
}

#[tokio::test]
async fn a_move_from_a_merged_project_is_refused() {
    let h = Harness::start().await;
    let (app, web, other) =
        (registered(&h, "/work/app").await, registered(&h, "/work/web").await, registered(&h, "/work/other").await);
    save_in(&h, &app, "the parser of app is written by hand").await;
    merge_projects(&h, &app, &web).await;

    let (status, body) = h.post_json("/v1/memories/move", json!({ "from_project": app, "to_project": other })).await;

    assert_eq!((status, body["error"].as_str()), (409, Some("project_merged")), "{body}");
    assert!(body["detail"].as_str().is_some_and(|d| d.contains(&web)), "{body}");
}

#[tokio::test]
async fn the_move_of_a_merge_is_undone_only_with_its_merge() {
    let h = Harness::start().await;
    let (app, web) = (registered(&h, "/work/app").await, registered(&h, "/work/web").await);
    let row = save_in(&h, &app, "the parser of app is written by hand").await;
    let merged = merge_projects(&h, &app, &web).await;

    let (status, body) = h.post_json("/v1/memories/move", json!({ "undo": merged.move_id.expect("a move") })).await;

    assert_eq!((status, body["error"].as_str()), (409, Some("part_of_merge")), "{body}");
    let merge_id = merged.merge_id.expect("a merge").to_string();
    assert!(body["detail"].as_str().is_some_and(|d| d.contains(&format!("--undo {merge_id}"))), "{body}");
    assert_eq!(project_of(&h, row).await, json!(web), "nothing moved");
}

#[tokio::test]
async fn merging_through_the_daemon_leaves_everything_under_the_target() {
    let h = Harness::start().await;
    let (app, web) = (registered(&h, "/work/app").await, registered(&h, "/work/web").await);
    let row = save_in(&h, &app, "the parser of app is written by hand").await;

    let (status, dry) = h.post_json("/v1/projects/merge", json!({ "source": app, "target": web, "dry_run": true })).await;
    assert_eq!(status, 200, "{dry}");
    assert_eq!((dry["dry_run"].as_bool(), dry["memories"].as_i64(), &dry["merge_id"]), (Some(true), Some(1), &Value::Null));
    assert_eq!(project_of(&h, row).await, json!(app), "a dry run moves nothing");

    let (status, merged) = h.post_json("/v1/projects/merge", json!({ "source": app, "target": web })).await;
    assert_eq!(status, 200, "{merged}");
    let merge_id = merged["merge_id"].as_i64().expect("a merge");
    assert!(merged["snapshot"].as_str().is_some_and(|s| s.contains("pre-merge")), "{merged}");
    assert_eq!(project_of(&h, row).await, json!(web));

    let refused = [
        (json!({ "source": web, "target": app }), 409, "project_merged"),
        (json!({ "source": "nowhere-9f9f", "target": web }), 404, "no project nowhere-9f9f"),
        (json!({ "source": web, "target": web }), 400, "cannot be merged into itself"),
        (json!({ "source": web }), 400, "name a source and a target"),
        (json!({ "undo": merge_id, "source": web }), 400, "name a source and a target"),
        (json!({ "undo": 99 }), 404, "no merge 99"),
    ];
    for (body, want, error) in refused {
        let (status, answer) = h.post_json("/v1/projects/merge", body.clone()).await;
        assert_eq!(status, want, "{body}: {answer}");
        assert!(answer["error"].as_str().is_some_and(|e| e.contains(error)), "{body}: {answer}");
    }

    let (status, undone) = h.post_json("/v1/projects/merge", json!({ "undo": merge_id })).await;
    assert_eq!(status, 200, "{undone}");
    assert!(undone["snapshot"].as_str().is_some_and(|s| s.contains("pre-undo")), "{undone}");
    assert_eq!(project_of(&h, row).await, json!(app));
    let (status, again) = h.post_json("/v1/projects/merge", json!({ "undo": merge_id })).await;
    assert_eq!((status, again["error"].as_str()), (409, Some("merge_undone")), "{again}");

    let (status, kept) = h.post_json("/v1/projects/keep-apart", json!({ "project_id": app, "other_id": web })).await;
    assert_eq!(status, 200, "{kept}");
    assert_eq!(kept, json!({ "project_id": app, "other_id": web, "answered": 0 }));
    let (status, _) = h.post_json("/v1/projects/keep-apart", json!({ "project_id": app, "other_id": app })).await;
    assert_eq!(status, 400);
    let (status, _) = h.post_json("/v1/projects/keep-apart", json!({ "project_id": app, "other_id": "nowhere-9f9f" })).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn the_listing_after_a_merge_shows_the_target_alone() {
    let h = Harness::start().await;
    let (app, web) = (registered(&h, "/work/app").await, registered(&h, "/work/web").await);
    save_in(&h, &app, "the parser of app is written by hand").await;
    let (_, merged) = h.post_json("/v1/projects/merge", json!({ "source": app, "target": web })).await;

    let (status, body) = h.get_json("/v1/projects").await;

    assert_eq!(status, 200, "{body}");
    let projects = body["projects"].as_array().expect("projects");
    assert_eq!(projects.len(), 1, "{body}");
    assert_eq!((&projects[0]["id"], &projects[0]["memories"]), (&json!(web), &json!(1)));
    assert_eq!(projects[0]["merged"][0]["id"], json!(app), "{body}");
    assert_eq!(projects[0]["merged"][0]["merge_id"], merged["merge_id"], "{body}");
    let folders: Vec<&Value> = projects[0]["folders"].as_array().expect("folders").iter().map(|f| &f["root_path"]).collect();
    assert_eq!(folders, vec![&json!("/work/app"), &json!("/work/web")], "{body}");
}

/// Writes a save still in the outbox for `project`, as one the worker has not finished.
async fn pending_save(h: &Harness, operation: &'static str, project: &str) {
    let event = json!({ "agent_id": "agent-http", "project_id": project, "content": "" }).to_string();
    h.ctx
        .runtime
        .store
        .conn
        .call(move |c| {
            c.execute(
                "INSERT INTO ingest_outbox (operation_id, agent_id, content, event, created_at, updated_at) \
                 VALUES (?1, 'agent-http', 'a save', ?2, 1, 1)",
                [operation, event.as_str()],
            )
        })
        .await
        .expect("a pending save");
}

#[tokio::test]
async fn a_merge_waits_for_the_saves_of_its_projects() {
    let h = Harness::start().await;
    let (app, web, other) =
        (registered(&h, "/work/app").await, registered(&h, "/work/web").await, registered(&h, "/work/other").await);
    pending_save(&h, "op-1", &app).await;
    let store = h.ctx.runtime.store.clone();
    // Taken before the spawn: the task may start its sleep before this line runs.
    let started = Instant::now();
    let finish = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        store.conn.call(|c| c.execute("DELETE FROM ingest_outbox", [])).await.expect("the save ends");
    });

    let (status, body) = h.post_json("/v1/projects/merge", json!({ "source": app, "target": web })).await;

    assert_eq!(status, 200, "{body}");
    assert!(started.elapsed() >= Duration::from_millis(300), "it waited for the save");
    finish.await.unwrap();

    pending_save(&h, "op-2", &web).await;
    let (status, body) = h.post_json("/v1/projects/merge", json!({ "source": web, "target": other })).await;
    assert_eq!((status, body["error"].as_str(), body["retryable"].as_bool()), (409, Some("saves_pending"), Some(true)), "{body}");
}

// ── Deleted projects ────────────────────────────────────────────────────────

/// Registers a root, saves a memory in it and deletes the project with it,
/// as `clawg projects delete --with-memories` does. Answers the project, the
/// memory and the delete's number.
async fn deleted_project(h: &Harness, root: &str) -> (String, i64, i64) {
    let id = registered(h, root).await;
    let row = save_in(h, &id, &format!("the notes of {root} are kept in a file")).await;
    let (status, report) = h.post_json("/v1/projects/delete", json!({ "project_id": id, "with_memories": true })).await;
    assert_eq!(status, 200, "{report}");
    let (status, done) =
        h.post_json("/v1/projects/delete", json!({ "project_id": id, "with_memories": true, "confirm": report["code"] })).await;
    assert_eq!(status, 200, "{done}");
    (id, row, done["delete_id"].as_i64().expect("a delete"))
}

fn listed_ids(listing: &Value) -> Vec<&str> {
    listing["projects"].as_array().expect("projects").iter().filter_map(|p| p["id"].as_str()).collect()
}

#[tokio::test]
async fn deleting_through_the_daemon_takes_two_steps() {
    let h = Harness::start().await;
    let (gone, empty) = (registered(&h, "/work/gone").await, registered(&h, "/work/empty").await);
    save_in(&h, &gone, "the notes of gone are kept in a file").await;

    let (status, refused) = h.post_json("/v1/projects/delete", json!({ "project_id": gone })).await;
    assert_eq!((status, refused["refused"].as_str(), refused["live"]["memories"].as_i64()), (200, Some("not_empty"), Some(1)), "{refused}");
    assert_eq!(refused["delete_id"], Value::Null);
    assert!(listed_ids(&h.get_json("/v1/projects").await.1).contains(&gone.as_str()), "nothing was written");

    let (status, report) = h.post_json("/v1/projects/delete", json!({ "project_id": gone, "with_memories": true })).await;
    assert_eq!(status, 200, "{report}");
    let code = report["code"].as_str().expect("a code").to_string();
    assert_eq!((&report["refused"], &report["delete_id"]), (&Value::Null, &Value::Null), "{report}");

    save_in(&h, &gone, "a save between the two steps").await;
    let (status, stale) =
        h.post_json("/v1/projects/delete", json!({ "project_id": gone, "with_memories": true, "confirm": code })).await;
    assert_eq!((status, stale["refused"].as_str(), stale["live"]["memories"].as_i64()), (200, Some("code_changed"), Some(2)), "{stale}");
    let fresh = stale["code"].as_str().expect("the new code").to_string();
    assert_ne!(fresh, code);
    assert!(listed_ids(&h.get_json("/v1/projects").await.1).contains(&gone.as_str()), "a stale code writes nothing");

    let (status, done) =
        h.post_json("/v1/projects/delete", json!({ "project_id": gone, "with_memories": true, "confirm": fresh })).await;
    assert_eq!(status, 200, "{done}");
    assert!(done["delete_id"].as_i64().is_some() && done["snapshot"].as_str().is_some_and(|s| s.contains("pre-delete")), "{done}");
    let (_, listing) = h.get_json("/v1/projects").await;
    assert!(!listed_ids(&listing).contains(&gone.as_str()), "{listing}");
    let deletes = listing["deletes"].as_array().expect("deletes");
    assert_eq!((deletes.len(), &deletes[0]["project_id"], &deletes[0]["rows"], &deletes[0]["back"]), (1, &json!(gone), &json!(2), &json!(false)), "{listing}");

    let (status, empty_done) = h.post_json("/v1/projects/delete", json!({ "project_id": empty })).await;
    assert_eq!(status, 200, "{empty_done}");
    assert!(empty_done["delete_id"].as_i64().is_some() && empty_done["refused"].is_null(), "{empty_done}");

    let again = h.post_json("/v1/projects/delete", json!({ "project_id": gone })).await;
    assert_eq!((again.0, again.1["error"].as_str()), (409, Some("project_deleted")), "{}", again.1);
    assert!(again.1["detail"].as_str().is_some_and(|d| d.contains("--undo")), "{}", again.1);
    let shapes = [
        (json!({}), 400),
        (json!({ "undo": 1, "project_id": gone }), 400),
        (json!({ "project_id": gone, "dry_run": true }), 400),
        (json!({ "project_id": "app-aaaa", "confirm": "abcd1234" }), 400),
        (json!({ "project_id": "nowhere-9f9f" }), 404),
        (json!({ "undo": 99 }), 404),
    ];
    for (body, want) in shapes {
        let (status, answer) = h.post_json("/v1/projects/delete", body.clone()).await;
        assert_eq!(status, want, "{body}: {answer}");
    }
    let (status, undone) = h.post_json("/v1/projects/delete", json!({ "undo": done["delete_id"] })).await;
    assert_eq!(status, 200, "{undone}");
    assert_eq!(undone["restored"]["memories"], json!(2), "{undone}");
    let (status, twice) = h.post_json("/v1/projects/delete", json!({ "undo": done["delete_id"] })).await;
    assert_eq!((status, twice["error"].as_str()), (409, Some("delete_undone")), "{twice}");
}

#[tokio::test]
async fn undoing_a_delete_needs_the_model() {
    let h = Harness::start_serving_without_embedder().await;
    // Saves are taken while the model reads as not loaded, only answered 202.
    let gone = registered(&h, "/work/gone").await;
    h.post_json("/v1/memories", json!({ "content": CONTENT, "agent_id": "agent-http", "project_id": gone })).await;
    h.wait_for_rows("memory", 1).await;
    let (_, report) = h.post_json("/v1/projects/delete", json!({ "project_id": gone, "with_memories": true })).await;
    let confirm = json!({ "project_id": gone, "with_memories": true, "confirm": report["code"] });
    let (status, done) = h.post_json("/v1/projects/delete", confirm).await;
    assert_eq!(status, 200, "{done}");
    let with_rows = done["delete_id"].as_i64().expect("a delete");
    let empty = registered(&h, "/work/empty").await;
    let (_, emptied) = h.post_json("/v1/projects/delete", json!({ "project_id": empty })).await;

    let (status, body) = h.post_json("/v1/projects/delete", json!({ "undo": with_rows })).await;
    assert_eq!((status, body["error"].as_str()), (503, Some("embedder_not_ready")), "{body}");
    let (_, listing) = h.get_json("/v1/projects").await;
    assert_eq!(listing["deletes"].as_array().map(Vec::len), Some(2), "the refused undo left the delete open");

    let (status, dry) = h.post_json("/v1/projects/delete", json!({ "undo": with_rows, "dry_run": true })).await;
    assert_eq!((status, dry["dry_run"].as_bool(), dry["restored"]["memories"].as_i64()), (200, Some(true), Some(1)), "{dry}");
    let (status, undone) = h.post_json("/v1/projects/delete", json!({ "undo": emptied["delete_id"] })).await;
    assert_eq!(status, 200, "an undo with nothing to restore needs no model: {undone}");
    assert!(undone["snapshot"].as_str().is_some_and(|s| s.contains("pre-undo")), "{undone}");
}

#[tokio::test]
async fn a_bridge_opened_before_a_delete_brings_its_project_back() {
    let h = Harness::start().await;
    // What the bridge registered at its start, and keeps.
    let (gone, old, delete_id) = deleted_project(&h, "/work/gone").await;

    let late = save_in(&h, &gone, "a note from a bridge opened before the delete").await;

    assert_eq!(project_of(&h, late).await, json!(gone));
    let (_, listing) = h.get_json("/v1/projects").await;
    let back = listing["projects"].as_array().unwrap().iter().find(|p| p["id"] == json!(gone)).expect("listed again");
    assert_eq!((&back["reviewed"], &back["memories"]), (&json!(false), &json!(1)), "{listing}");
    let deletes = listing["deletes"].as_array().unwrap();
    assert_eq!((&deletes[0]["delete_id"], &deletes[0]["back"]), (&json!(delete_id), &json!(true)), "{listing}");
    let (_, row) = h.get_json(&format!("/v1/memories/memory/{old}")).await;
    assert!(row["item"]["deleted_at"].as_i64().is_some() || row["item"].is_null(), "what the delete deleted stays deleted: {row}");
    let live = h.get_json(&format!("/v1/memories?project={gone}")).await.1;
    assert_eq!(ids_in(&live["items"]), vec![late], "{live}");
}

#[tokio::test]
async fn every_route_that_takes_a_project_treats_a_deleted_one_by_its_rule() {
    let h = Harness::start().await;
    let other = registered(&h, "/work/other").await;
    let on_other = save_in(&h, &other, "the other service is written in Go").await;
    let refused = |route: &str, status: u16, body: &Value| {
        assert_eq!((status, body["error"].as_str()), (409, Some("project_deleted")), "{route}: {body}");
        assert!(body["detail"].as_str().is_some_and(|d| d.contains("--undo")), "{route}: {body}");
    };
    let still_deleted = |id: String| {
        let h = &h;
        async move {
            let (_, listing) = h.get_json("/v1/projects").await;
            assert!(!listed_ids(&listing).contains(&id.as_str()), "{id} was brought back by what only reads it: {listing}");
        }
    };

    for (method, path, field) in PROJECT_INPUTS {
        let Some(field) = field else { continue };
        let route = format!("{method} {path} ({field})");
        let root = format!("/work/{}", route.replace(|c: char| !c.is_alphanumeric(), "-"));
        let (gone, gone_row, _) = deleted_project(&h, &root).await;
        match (*method, *path) {
            ("POST", "/memories") => {
                let id = save_in(&h, &gone, "a note saved after the delete").await;
                assert_eq!(project_of(&h, id).await, json!(gone), "{route}");
                let (_, listing) = h.get_json("/v1/projects").await;
                assert!(listed_ids(&listing).contains(&gone.as_str()), "{route}: a save brings it back");
                continue;
            }
            ("GET", "/memories") => {
                let (status, body) = h.get_json(&format!("/v1/memories?project={gone}")).await;
                assert_eq!(status, 200, "{route}: {body}");
                assert!(ids_in(&body["items"]).is_empty(), "{route}: {body}");
            }
            ("POST", "/memories/move") => {
                let (status, body) = h.post_json("/v1/memories/move", json!({ "ids": [on_other], "to_project": gone })).await;
                refused(&route, status, &body);
                let from = json!({ "from_project": gone, "include_deleted": true, "to_project": other });
                let (status, body) = h.post_json("/v1/memories/move", from).await;
                assert_eq!(status, 200, "{route}: its deleted rows can be taken out: {body}");
                assert_eq!(project_of(&h, gone_row).await, json!(other), "{route}");
            }
            ("POST", "/search") => {
                let ask = json!({ "agent_id": "agent-http", "query": "notes kept in a file", "project_id": gone });
                let (status, body) = h.post_json("/v1/search", ask).await;
                assert_eq!(status, 200, "{route}: {body}");
                assert!(!ids_in(&body["hits"]).contains(&gone_row), "{route}: {body}");
            }
            ("POST", "/context") => {
                let (status, body) = h.post_json("/v1/context", json!({ "agent_id": "agent-http", "project_id": gone })).await;
                assert_eq!(status, 200, "{route}: {body}");
                assert!(body["prompt"].as_str().is_some_and(|p| !p.contains("kept in a file")), "{route}: {body}");
            }
            ("POST", "/sessions") => {
                let (status, body) = h
                    .post_json("/v1/sessions", json!({ "agent_id": "agent-http", "project_id": gone, "session_id": "s-gone" }))
                    .await;
                assert_eq!(status, 204, "{route}: {body}");
            }
            ("PATCH", "/memories/{kind}/{id}") => {
                let (status, body) = patch(&h, on_other, json!({ "project_id": gone })).await;
                refused(&route, status, &body);
                assert_eq!(project_of(&h, on_other).await, json!(other), "{route}");
            }
            ("GET", "/disputes") => {
                let (status, body) = h.get_json(&format!("/v1/disputes?agent_id=agent-http&project_id={gone}")).await;
                assert_eq!(status, 200, "{route}: {body}");
            }
            ("POST", "/synthesis") => {
                let (status, body) = h.post_json("/v1/synthesis", json!({ "project_id": gone })).await;
                assert_eq!(status, 200, "{route}: {body}");
                assert!(!ids_in(&body["pending"]).contains(&gone_row), "{route}: {body}");
            }
            ("GET", "/stats") => {
                let (status, body) = h.get_json(&format!("/v1/stats?project={gone}")).await;
                assert_eq!(status, 200, "{route}: {body}");
            }
            ("PATCH", "/projects/{id}") => {
                let (status, body) = h.rename(&gone, "Gone").await;
                refused(&route, status, &body);
            }
            ("POST", "/projects/merge") => {
                for body in [json!({ "source": other, "target": gone }), json!({ "source": gone, "target": other })] {
                    let (status, answer) = h.post_json("/v1/projects/merge", body).await;
                    refused(&route, status, &answer);
                }
                // The undo of a merge whose target was deleted since.
                let (a, b) = (registered(&h, &format!("{root}-a")).await, registered(&h, &format!("{root}-b")).await);
                save_in(&h, &a, "a note of a project merged and then deleted").await;
                let (_, merged) = h.post_json("/v1/projects/merge", json!({ "source": a, "target": b })).await;
                let (_, report) = h.post_json("/v1/projects/delete", json!({ "project_id": b, "with_memories": true })).await;
                h.post_json("/v1/projects/delete", json!({ "project_id": b, "with_memories": true, "confirm": report["code"] })).await;
                let (status, answer) = h.post_json("/v1/projects/merge", json!({ "undo": merged["merge_id"] })).await;
                refused(&route, status, &answer);
            }
            ("POST", "/projects/keep-apart") => {
                for body in [json!({ "project_id": gone, "other_id": other }), json!({ "project_id": other, "other_id": gone })] {
                    let (status, answer) = h.post_json("/v1/projects/keep-apart", body).await;
                    refused(&route, status, &answer);
                }
            }
            ("POST", "/projects/delete") => {
                for body in [json!({ "project_id": gone }), json!({ "project_id": gone, "with_memories": true })] {
                    let (status, answer) = h.post_json("/v1/projects/delete", body).await;
                    refused(&route, status, &answer);
                }
            }
            ("POST", "/projects/resolve") => {
                let (status, body) = h.post_json("/v1/projects/resolve", json!({ "project": gone, "create": true })).await;
                assert_eq!(status, 200, "{route}: {body}");
                assert_eq!((body["project_id"].as_str(), body["deleted"].as_bool()), (Some(gone.as_str()), Some(true)), "{route}: {body}");
            }
            ("POST", "/projects/suggestions/answer") => {
                for (one, two) in [(&gone, &other), (&other, &gone)] {
                    let answer = json!({ "project_id": one, "other_id": two, "answer": "apart", "user_answered": true, "reason": "two things" });
                    let (status, body) = h.post_json("/v1/projects/suggestions/answer", answer).await;
                    refused(&route, status, &body);
                }
            }
            ("POST", "/projects/locate") => {
                let folder = tempfile::tempdir().expect("tempdir");
                let (status, body) =
                    h.post_json("/v1/projects/locate", json!({ "project": gone, "root_path": folder.path().to_string_lossy() })).await;
                refused(&route, status, &body);
                let sub = checkout_with(folder.path(), "apps/web");
                let ask = json!({ "project": gone, "root_path": sub.to_string_lossy(), "subfolder": true });
                let (status, body) = h.post_json("/v1/projects/locate", ask).await;
                refused(&route, status, &body);
            }
            ("POST", "/import") => {
                h.ctx.runtime.tokenizer.set(clawg_core::memory::chunker::word_tokenizer()).ok();
                let text = engram_export(vec![observation("deleted-map", Some("notes"), "Port", "a note for a deleted project")]);
                for dry_run in [true, false] {
                    let ask = json!({ "text": text, "from": "engram", "dry_run": dry_run, "project_map": [["notes", { "project": gone }]] });
                    let (status, body) = h.post_json("/v1/import", ask).await;
                    refused(&route, status, &body);
                }
            }
            _ => panic!("{route} takes a project and no check here says what it does with a deleted one"),
        }
        still_deleted(gone).await;
    }
}

// ── projects by name, their suggestions and their folders ───────────────────

async fn resolve(h: &Harness, ask: Value) -> Value {
    let (status, body) = h.post_json("/v1/projects/resolve", ask).await;
    assert_eq!(status, 200, "{body}");
    body
}

#[tokio::test]
async fn a_reference_resolves_through_the_daemon() {
    let h = Harness::start().await;
    let app = registered(&h, "/work/mi-app").await;

    let similar = resolve(&h, json!({ "project": "Mi App", "create": true })).await;
    assert_eq!((similar["refused"].as_str(), similar["candidates"][0]["id"].as_str()), (Some("similar"), Some(app.as_str())), "{similar}");
    let unknown = resolve(&h, json!({ "project": "ideas de producto" })).await;
    assert_eq!(unknown["refused"].as_str(), Some("unknown"), "{unknown}");

    let created = resolve(&h, json!({ "project": "ideas de producto", "create": true })).await;
    let ideas = created["project_id"].as_str().expect("created").to_string();
    assert_eq!((created["created"].as_bool(), created["folders"].as_array().map(Vec::len)), (Some(true), Some(0)), "{created}");
    let row = save_in(&h, &ideas, "a pricing page for teams").await;
    assert_eq!(project_of(&h, row).await, json!(ideas));
    let (_, listing) = h.get_json("/v1/projects").await;
    let listed = listing["projects"].as_array().unwrap().iter().find(|p| p["id"] == json!(ideas)).cloned().expect("listed");
    assert_eq!((listed["name"].as_str(), listed["born"].as_str(), listed["reviewed"].as_bool()), (Some("ideas de producto"), Some("agent"), Some(false)));

    let user = resolve(&h, json!({ "project": "Roadmap", "create": true, "born": "user" })).await;
    assert_eq!(user["created"].as_bool(), Some(true), "{user}");
    let (status, body) = h.post_json("/v1/projects/resolve", json!({ "project": "  " })).await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = h.post_json("/v1/projects/resolve", json!({ "project": "x", "new_project": true })).await;
    assert_eq!(status, 400, "new_project goes with create: {body}");
}

#[tokio::test]
async fn a_move_and_a_patch_take_a_project_by_name() {
    let h = Harness::start().await;
    let (app, web) = (registered(&h, "/work/app").await, registered(&h, "/work/web").await);
    let row = save_in(&h, &app, "the parser of app is written by hand").await;
    let other = save_in(&h, &app, "the lexer of app is generated").await;

    let (status, body) = h.post_json("/v1/memories/move", json!({ "ids": [row], "to_project": "WEB" })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(project_of(&h, row).await, json!(web));
    let (status, body) = patch(&h, other, json!({ "project_id": "web" })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(project_of(&h, other).await, json!(web));
    let (status, body) = h.post_json("/v1/memories/move", json!({ "ids": [row], "to_project": "nothing here" })).await;
    assert_eq!(status, 404, "{body}");
    assert!(body["error"].as_str().is_some_and(|e| e.contains("nothing here")), "{body}");
}

#[tokio::test]
async fn a_name_two_projects_hold_is_refused_as_a_target() {
    let h = Harness::start().await;
    let (one, two) = (registered(&h, "/work/a/api").await, registered(&h, "/work/b/api").await);
    let row = save_in(&h, &one, "the api of a answers in json").await;

    let (status, body) = h.post_json("/v1/memories/move", json!({ "ids": [row], "to_project": "api" })).await;
    assert_eq!((status, body["error"].as_str()), (409, Some("project_ambiguous")), "{body}");
    assert!(body["detail"].as_str().is_some_and(|d| d.contains(&one) && d.contains(&two)), "{body}");
    let (status, body) = patch(&h, row, json!({ "project_id": "api" })).await;
    assert_eq!((status, body["error"].as_str()), (409, Some("project_ambiguous")), "{body}");
    assert_eq!(project_of(&h, row).await, json!(one), "nothing moved");
}

/// Leaves a pending suggestion that `a` and `b` may be one project.
async fn suggest(h: &Harness, a: &str, b: &str) {
    let (a, b) = (a.to_string(), b.to_string());
    h.ctx
        .runtime
        .store
        .conn
        .call(move |c| {
            c.execute(
                "INSERT INTO project_suggestion (project_id, other_id, reason, created_at) VALUES (?1, ?2, 'root_commit', 1)",
                [a, b],
            )
        })
        .await
        .expect("a suggestion");
}

async fn context_suggestions(h: &Harness, project: &str) -> Value {
    let (status, body) = h.post_json("/v1/context", json!({ "agent_id": "agent-http", "project_id": project })).await;
    assert_eq!(status, 200, "{body}");
    body["suggestions"].clone()
}

#[tokio::test]
async fn the_context_names_the_pending_suggestions_of_its_project() {
    let h = Harness::start().await;
    let (app, copy, other) = (registered(&h, "/work/app").await, registered(&h, "/work/copia").await, registered(&h, "/work/other").await);
    suggest(&h, &copy, &app).await;

    let named = context_suggestions(&h, &app).await;
    assert_eq!(named[0]["other_id"].as_str(), Some(copy.as_str()), "{named}");
    assert_eq!((named[0]["other_name"].as_str(), named[0]["reason"].as_str()), (Some("copia"), Some("root_commit")), "{named}");
    assert_eq!(context_suggestions(&h, &copy).await[0]["other_id"].as_str(), Some(app.as_str()));
    assert_eq!(context_suggestions(&h, &other).await, Value::Null, "none is left out");
    let (_, query) = h.post_json("/v1/context", json!({ "agent_id": "agent-http", "project_id": app, "query": "the parser" })).await;
    assert_eq!(query["suggestions"][0]["other_id"].as_str(), Some(copy.as_str()), "with a query too: {query}");
}

fn answer(project: &str, other: &str, answer: &str) -> Value {
    json!({ "project_id": project, "other_id": other, "answer": answer, "user_answered": true, "reason": "they said so" })
}

#[tokio::test]
async fn answering_a_suggestion_needs_the_users_answer() {
    let h = Harness::start().await;
    let (app, copy) = (registered(&h, "/work/app").await, registered(&h, "/work/copia").await);
    suggest(&h, &copy, &app).await;

    let asks = [
        json!({ "project_id": app, "other_id": copy, "answer": "apart", "reason": "they said so" }),
        json!({ "project_id": app, "other_id": copy, "answer": "apart", "user_answered": true }),
        json!({ "project_id": app, "other_id": copy, "answer": "apart", "user_answered": true, "reason": "  " }),
        json!({ "project_id": app, "other_id": copy, "answer": "maybe", "user_answered": true, "reason": "they said so" }),
    ];
    for ask in asks {
        let (status, body) = h.post_json("/v1/projects/suggestions/answer", ask.clone()).await;
        assert_eq!(status, 400, "{ask}: {body}");
    }
    assert_eq!(context_suggestions(&h, &app).await[0]["other_id"].as_str(), Some(copy.as_str()), "still pending");
}

#[tokio::test]
async fn answering_apart_keeps_the_pair_from_coming_back() {
    let h = Harness::start().await;
    let (app, copy) = (registered(&h, "/work/app").await, registered(&h, "/work/copia").await);
    suggest(&h, &copy, &app).await;

    let (status, body) = h.post_json("/v1/projects/suggestions/answer", answer(&app, &copy, "apart")).await;

    assert_eq!((status, body["answer"].as_str()), (200, Some("apart")), "{body}");
    assert_eq!(context_suggestions(&h, &app).await, Value::Null);
    let (status, body) = h.post_json("/v1/projects/suggestions/answer", answer(&app, &copy, "apart")).await;
    assert_eq!((status, body["error"].as_str()), (404, Some("no_suggestion")), "answered once: {body}");
}

#[tokio::test]
async fn answering_merge_merges_and_is_undone() {
    let h = Harness::start().await;
    let (app, copy) = (registered(&h, "/work/app").await, registered(&h, "/work/copia").await);
    let row = save_in(&h, &copy, "the copy keeps its own config").await;
    suggest(&h, &copy, &app).await;

    let (status, body) = h.post_json("/v1/projects/suggestions/answer", answer(&app, &copy, "merge")).await;

    assert_eq!((status, body["answer"].as_str()), (200, Some("merge")), "{body}");
    assert_eq!(project_of(&h, row).await, json!(app), "the other goes into the project");
    let merge_id = body["merge"]["merge_id"].as_i64().expect("a merge");
    assert_eq!(context_suggestions(&h, &app).await, Value::Null, "a merge answers it");
    let (status, undone) = h.post_json("/v1/projects/merge", json!({ "undo": merge_id })).await;
    assert_eq!(status, 200, "{undone}");
    assert_eq!(project_of(&h, row).await, json!(copy));
}

#[tokio::test]
async fn answering_without_a_pending_suggestion_is_404() {
    let h = Harness::start().await;
    let (app, web) = (registered(&h, "/work/app").await, registered(&h, "/work/web").await);

    for ask in [answer(&app, &web, "merge"), answer(&app, "nobody-0000", "apart")] {
        let (status, body) = h.post_json("/v1/projects/suggestions/answer", ask.clone()).await;
        assert_eq!(status, 404, "{ask}: {body}");
    }
    let (_, listing) = h.get_json("/v1/projects").await;
    assert!(listed_ids(&listing).contains(&web.as_str()), "nothing was merged");
}

async fn registered_dir(h: &Harness, root: &std::path::Path) -> Value {
    let (status, body) = h.post_json("/v1/projects", json!({ "root_path": root.to_string_lossy(), "assigned": true })).await;
    assert_eq!(status, 200, "{body}");
    body
}

#[tokio::test]
async fn locating_a_folder_merges_its_own_project_into_the_named_one() {
    let h = Harness::start().await;
    let ideas = resolve(&h, json!({ "project": "ideas", "create": true })).await["project_id"].as_str().unwrap().to_string();
    let base = tempfile::tempdir().expect("tempdir");
    let notes = base.path().join("notes");
    std::fs::create_dir(&notes).expect("a folder");
    let own = registered_dir(&h, &notes).await["project_id"].as_str().unwrap().to_string();
    let row = save_in(&h, &own, "the notes of the week").await;

    let ask = json!({ "project": "Ideas", "root_path": notes.to_string_lossy() });
    let (status, body) = h.post_json("/v1/projects/locate", ask.clone()).await;

    assert_eq!((status, body["project_id"].as_str(), body["already"].as_bool()), (200, Some(ideas.as_str()), Some(false)), "{body}");
    assert!(body["merge"]["merge_id"].as_i64().is_some(), "{body}");
    assert_eq!(project_of(&h, row).await, json!(ideas));
    assert_eq!(registered_dir(&h, &notes).await["project_id"].as_str(), Some(ideas.as_str()), "the folder leads to it now");
    let (status, again) = h.post_json("/v1/projects/locate", ask).await;
    assert_eq!((status, again["already"].as_bool()), (200, Some(true)), "{again}");
    let (status, body) =
        h.post_json("/v1/projects/locate", json!({ "project": "nothing here", "root_path": notes.to_string_lossy() })).await;
    assert_eq!(status, 404, "{body}");
    let (status, body) = h.post_json("/v1/projects/locate", json!({ "project": "ideas", "root_path": "/no/such/dir" })).await;
    assert_eq!(status, 400, "{body}");
}

#[tokio::test]
async fn a_folder_shared_by_its_project_is_not_located() {
    let h = Harness::start().await;
    resolve(&h, json!({ "project": "ideas", "create": true })).await;
    let base = tempfile::tempdir().expect("tempdir");
    let (one, two) = (base.path().join("one"), base.path().join("two"));
    std::fs::create_dir(&one).expect("a folder");
    std::fs::create_dir(&two).expect("a folder");
    let own = registered_dir(&h, &one).await["project_id"].as_str().unwrap().to_string();
    // A second folder of the same project, as a clone or a move leaves it.
    let shared = registered_dir(&h, &two).await["root_id"].as_str().unwrap().to_string();
    let (own2, shared2) = (own.clone(), shared.clone());
    h.ctx
        .runtime
        .store
        .conn
        .call(move |c| c.execute("UPDATE project_path SET project_id = ?1 WHERE id = ?2", [own2, shared2]))
        .await
        .expect("share the project");

    let (status, body) = h.post_json("/v1/projects/locate", json!({ "project": "ideas", "root_path": one.to_string_lossy() })).await;

    assert_eq!((status, body["error"].as_str()), (409, Some("folder_shared")), "{body}");
    assert!(body["detail"].as_str().is_some_and(|d| d.contains(&own) && d.contains("projects merge")), "{body}");
}

#[tokio::test]
async fn a_folder_without_vcs_moved_is_suggested_and_shown_until_answered() {
    let h = Harness::start().await;
    let base = tempfile::tempdir().expect("tempdir");
    let (old, moved) = (base.path().join("notas"), base.path().join("archivo").join("notas"));
    std::fs::create_dir(&old).expect("a folder");
    let before = registered_dir(&h, &old).await["project_id"].as_str().unwrap().to_string();
    save_in(&h, &before, "the notes of the week").await;
    std::fs::create_dir(moved.parent().unwrap()).expect("a folder");
    std::fs::rename(&old, &moved).expect("move the folder");

    let after = registered_dir(&h, &moved).await["project_id"].as_str().unwrap().to_string();

    assert_ne!(after, before, "a name alone does not join them");
    let shown = context_suggestions(&h, &after).await;
    assert_eq!((shown[0]["other_id"].as_str(), shown[0]["reason"].as_str()), (Some(before.as_str()), Some("folder_name")), "{shown}");
    let (status, body) = h.post_json("/v1/projects/suggestions/answer", answer(&after, &before, "apart")).await;
    assert_eq!(status, 200, "{body}");
    registered_dir(&h, &moved).await;
    assert_eq!(context_suggestions(&h, &after).await, Value::Null, "answered for good");
}

// ── the subfolders of a monorepo ────────────────────────────────────────────

/// `dir` as a checkout (a `.git` directory, which is all a root needs) with
/// `subpath` inside it, by its canonical name.
fn checkout_with(dir: &std::path::Path, subpath: &str) -> std::path::PathBuf {
    std::fs::create_dir_all(dir.join(".git")).expect("a .git");
    let sub = dir.join(subpath);
    std::fs::create_dir_all(&sub).expect("the subfolder");
    canonical(&sub)
}

fn canonical(path: &std::path::Path) -> std::path::PathBuf {
    clawg_core::memory::identity::strip_verbatim(&std::fs::canonicalize(path).expect("canonicalize"))
}

async fn register_from(h: &Harness, root: &std::path::Path, path: &std::path::Path) -> Value {
    let ask = json!({ "root_path": root.to_string_lossy(), "assigned": true, "path": path.to_string_lossy() });
    let (status, body) = h.post_json("/v1/projects", ask).await;
    assert_eq!(status, 200, "{body}");
    body
}

async fn locate_subfolder(h: &Harness, project: &str, path: &std::path::Path) -> (u16, Value) {
    h.post_json("/v1/projects/locate", json!({ "project": project, "root_path": path.to_string_lossy(), "subfolder": true })).await
}

#[tokio::test]
async fn a_registration_without_a_path_or_with_one_elsewhere_answers_as_before() {
    let h = Harness::start().await;
    let base = tempfile::tempdir().expect("tempdir");
    let a = checkout_with(&base.path().join("a"), "src").parent().unwrap().to_path_buf();
    let b_src = checkout_with(&base.path().join("b"), "src");
    let plain = base.path().join("plain");
    std::fs::create_dir(&plain).expect("a folder");
    let before = registered_dir(&h, &a).await;

    for path in [b_src.clone(), canonical(&plain), std::path::PathBuf::from("src"), a.join("src")] {
        assert_eq!(register_from(&h, &a, &path).await, before, "{}", path.display());
    }
    let derived = json!({ "root_path": a.to_string_lossy(), "path": a.join("src").to_string_lossy() });
    let (status, body) = h.post_json("/v1/projects", derived).await;
    let (_, without) = h.post_json("/v1/projects", json!({ "root_path": a.to_string_lossy() })).await;
    assert_eq!((status, body), (200, without), "a caller that derives its id ignores the path");
}

#[tokio::test]
async fn a_declared_subfolder_takes_what_is_saved_inside_it_in_every_clone() {
    let h = Harness::start().await;
    let base = tempfile::tempdir().expect("tempdir");
    let base = canonical(base.path());
    git(&base, &["init", "monorepo"]);
    let mono = base.join("monorepo");
    git(&mono, &["remote", "add", "origin", "https://github.com/acme/monorepo.git"]);
    git(&mono, &["commit", "--allow-empty", "-m", "first"]);
    git(&base, &["clone", "monorepo", "elsewhere"]);
    let clone = base.join("elsewhere");
    git(&clone, &["remote", "set-url", "origin", "https://github.com/acme/monorepo.git"]);
    for root in [&mono, &clone] {
        for sub in ["apps/web/src", "apps/webapp"] {
            std::fs::create_dir_all(root.join(sub)).expect("the subfolders");
        }
    }
    let m = registered_dir(&h, &mono).await["project_id"].as_str().unwrap().to_string();
    let web = resolve(&h, json!({ "project": "web", "create": true })).await["project_id"].as_str().unwrap().to_string();

    let (status, declared) = locate_subfolder(&h, "web", &mono.join("apps").join("web")).await;
    assert_eq!(status, 200, "{declared}");
    assert_eq!((declared["project_id"].as_str(), declared["repository_id"].as_str()), (Some(web.as_str()), Some(m.as_str())), "{declared}");
    assert_eq!(declared["subpath"].as_str(), Some("apps/web"), "{declared}");

    let inside = register_from(&h, &mono, &mono.join("apps").join("web").join("src")).await;
    assert_eq!(inside["project_id"].as_str(), Some(web.as_str()), "{inside}");
    let row = save_in(&h, inside["project_id"].as_str().unwrap(), "the web app renders on the server").await;
    assert_eq!(project_of(&h, row).await, json!(web));
    assert_eq!(register_from(&h, &mono, &mono).await["project_id"].as_str(), Some(m.as_str()));
    assert_eq!(register_from(&h, &mono, &mono.join("apps").join("webapp")).await["project_id"].as_str(), Some(m.as_str()));
    assert_eq!(inside["root_id"], registered_dir(&h, &mono).await["root_id"], "the root keeps its own id");

    let cloned = register_from(&h, &clone, &clone.join("apps").join("web").join("src")).await;
    assert_eq!(cloned["project_id"].as_str(), Some(web.as_str()), "a clone of the remote reaches it: {cloned}");
    assert_eq!(register_from(&h, &clone, &clone).await["project_id"].as_str(), Some(m.as_str()));
}

#[tokio::test]
async fn a_subfolder_is_declared_through_the_daemon_and_refused_at_a_root() {
    let h = Harness::start().await;
    let base = tempfile::tempdir().expect("tempdir");
    let web_dir = checkout_with(&base.path().join("mono"), "apps/web");
    let mono = web_dir.parent().unwrap().parent().unwrap().to_path_buf();
    let plain = base.path().join("plain");
    std::fs::create_dir(&plain).expect("a folder");
    resolve(&h, json!({ "project": "web", "create": true })).await;
    let m = registered_dir(&h, &mono).await["project_id"].as_str().unwrap().to_string();

    for refused in [mono.clone(), canonical(&plain), base.path().join("no-such-dir")] {
        let (status, body) = locate_subfolder(&h, "web", &refused).await;
        assert_eq!(status, 400, "{}: {body}", refused.display());
    }
    let (status, body) = locate_subfolder(&h, "web", &mono).await;
    assert!(body["error"].as_str().is_some_and(|e| e.contains("without `subfolder`")), "{status} {body}");
    let (status, body) = locate_subfolder(&h, "nothing here", &web_dir).await;
    assert_eq!(status, 404, "{body}");

    let (status, declared) = locate_subfolder(&h, "web", &web_dir).await;
    assert_eq!((status, declared["already"].as_bool(), declared["removed"].as_bool()), (200, Some(false), Some(false)), "{declared}");
    let (_, again) = locate_subfolder(&h, "web", &web_dir).await;
    assert_eq!(again["already"].as_bool(), Some(true), "{again}");
    let (_, listing) = h.get_json("/v1/projects").await;
    let web = listing["projects"].as_array().unwrap().iter().find(|p| p["name"] == "web").unwrap().clone();
    let shown = &web["subfolders"][0];
    assert_eq!((shown["repository_id"].as_str(), shown["subpath"].as_str()), (Some(m.as_str()), Some("apps/web")), "{web}");
    assert_eq!(shown["paths"][0]["present"].as_bool(), Some(true), "{web}");

    let (status, removed) = locate_subfolder(&h, &m, &web_dir).await;
    assert_eq!((status, removed["removed"].as_bool(), removed["project_id"].as_str()), (200, Some(true), Some(m.as_str())), "{removed}");
    assert_eq!(register_from(&h, &mono, &web_dir).await["project_id"].as_str(), Some(m.as_str()), "it leads to the repository again");
}
