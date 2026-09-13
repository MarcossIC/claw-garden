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
use std::time::Instant;

use clawg_cli::daemon::api::{router, AppCtx};
use clawg_core::memory::embedder::Embedder;
use clawg_core::paths::Profile;
use clawg_core::runtime::MemoryRuntime;
use once_cell::sync::OnceCell;
use serde_json::{json, Value};

/// A live daemon on a loopback port, plus everything a test needs to talk to it.
struct Harness {
    addr: SocketAddr,
    token: String,
    http: reqwest::Client,
    ctx: Arc<AppCtx>,
    // Held so the temporary data directory outlives the server.
    _dir: tempfile::TempDir,
}

impl Harness {
    /// Normal daemon: embedder ready, ingest worker running.
    async fn start() -> Self {
        Self::start_inner(true).await
    }

    /// Daemon whose embedder cell was never filled — the state a real daemon is
    /// in for the first minutes after a cold start, while the model downloads.
    async fn start_without_embedder() -> Self {
        Self::start_inner(false).await
    }

    async fn start_inner(with_embedder: bool) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let (runtime, rx) = MemoryRuntime::new_for_test(dir.path().to_path_buf()).await;

        // Struct-update syntax swaps in a fresh, EMPTY cell while moving every
        // other handle across untouched — the cheapest way to reproduce a
        // pre-bootstrap daemon without a second constructor in core.
        let runtime = if with_embedder {
            runtime
        } else {
            MemoryRuntime {
                embedder: Arc::new(OnceCell::<Arc<dyn Embedder>>::new()),
                ..runtime
            }
        };

        // The ingest worker is what turns a queued event into a row. Without
        // it `POST /v1/memories` would answer 202 forever and nothing would
        // ever land, which is exactly the bug these tests exist to catch.
        tokio::spawn(runtime.ingest_worker(rx));

        let token = "test-token-0123456789abcdef".to_string();
        let (shutdown, _rx) = tokio::sync::watch::channel(false);
        let ctx = Arc::new(AppCtx {
            runtime,
            token: token.clone(),
            profile: Profile::Dev,
            started_at: Instant::now(),
            shutdown,
        });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral loopback port");
        let addr = listener.local_addr().expect("local_addr");
        let app = router(ctx.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self { addr, token, http: reqwest::Client::new(), ctx, _dir: dir }
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

// ── Save → list → search → context → patch → delete ─────────────────────────

const CONTENT: &str = "the deployment pipeline uses blue green releases on fridays";

#[tokio::test]
async fn a_saved_memory_is_queued_then_becomes_a_listable_row() {
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
    assert_eq!(status, 202, "save must be accepted, not completed: {body}");
    assert_eq!(body["status"], "queued");

    let items = h.wait_for_rows("memory", 1).await;
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
