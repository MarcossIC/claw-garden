//! The daemon's loopback HTTP API.
//!
//! # Why plain JSON over HTTP and not MCP streamable HTTP
//!
//! MCP is the `clawg mcp` bridge's problem, not the daemon's; making the daemon
//! an MCP server would force every other client to speak a protocol designed
//! for LLM tool calls in order to list rows in a table. So the daemon exposes
//! an ordinary JSON API, and the bridge is the only thing that knows what a
//! tool call is.
//!
//! # Auth
//!
//! Everything under `/v1` requires `Authorization: Bearer <token>`, where the
//! token is the one in `<data_dir>/token`. `GET /health` is deliberately open:
//! it answers "is something listening?", which a caller has to be able to ask
//! *before* it can be expected to have found the token file — and it discloses
//! nothing a port scan would not.

mod disputes;
mod docs;
mod memories;
mod outbox;
mod portability;
mod projects;
mod sessions;
#[cfg(test)]
mod test_support;

use axum::extract::{Query, State};
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use clawg_core::memory::admin::{memory_stats_core, StatsParams};
use clawg_core::memory::bootstrap::EmbedderStatus;
use clawg_core::memory::embedder::{Embedder, EmbedError};
use clawg_core::memory::merge::canonical;
use clawg_core::memory::outbox::OutboxSummary;
use clawg_core::memory::sharing::{Reach, Reader};
use clawg_core::memory::{project_docs, sharing};
use clawg_core::paths::Profile;
use clawg_core::runtime::MemoryRuntime;
use disputes::{disputes, judge_dispute, resolve_dispute};
use docs::search_docs;
use memories::{
    context, delete_memory, get_memory, get_save, list_memories, move_memories, restore_memory,
    save_memory, SavedMemory, search, synthesis, update_memory,
};
use outbox::{discard_outbox, retry_outbox};
use portability::{
    backup, cancel_import_job, IMPORT_BODY_LIMIT, import_job, import_memories, start_import_job,
    undo_import,
};
use projects::{
    answer_suggestion, delete_projects, keep_projects_apart, locate_project, merge_projects,
    projects, register, rename, resolve_projects,
};
use serde::{Deserialize, Serialize};
use sessions::{count_tool_call, end_session, register_session};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::guard::{admits, RefusalLog};
use super::imports::{ImportJobs, JobView};
use super::lifecycle::now_ms;
use super::metrics::{HttpMeter, metrics_body, prometheus_text, time_request, Transport};
use super::sessions::SessionRegistry;
pub use self::docs::{DocsSearchBody, REFRESH_GRACE};

/// Everything a request handler is allowed to reach.
///
/// Held behind an `Arc` and never mutated: the runtime's own handles are
/// already interior-mutable where they need to be, so a lock here would only
/// serialize requests that the store serializes anyway.
pub struct AppCtx {
    pub runtime: MemoryRuntime,
    pub token: String,
    pub profile: Profile,
    pub started_at: Instant,
    /// Flipped to `true` by `POST /v1/shutdown`; `serve::run` awaits it beside
    /// Ctrl-C. A `watch` rather than a oneshot so the graceful-shutdown future
    /// can hold a receiver without consuming the sender's only send.
    pub shutdown: tokio::sync::watch::Sender<bool>,
    /// How long a save waits for the worker to confirm it before answering
    /// `202 unconfirmed`. [`SAVE_CONFIRM_TIMEOUT`] outside tests.
    pub save_timeout: Duration,
    /// The host session each agent works in, per project, that saves are
    /// stamped with.
    pub sessions: SessionRegistry,
    /// The import job that runs and the last one that ended.
    pub imports: ImportJobs,
}

/// How long a save waits for the worker. Long enough for a multi-chunk save
/// on a slow CPU; past it the save is still queued and answers honestly that
/// it is not confirmed.
pub const SAVE_CONFIRM_TIMEOUT: Duration = Duration::from_secs(10);

// ── Error shape ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retryable: Option<bool>,
    /// What a failed save stored before it stopped.
    #[serde(skip_serializing_if = "Option::is_none")]
    memories: Option<Vec<SavedMemory>>,
    /// The import job that keeps a new one out.
    #[serde(skip_serializing_if = "Option::is_none")]
    job_id: Option<u64>,
}

/// Every failure this API can answer with, rendered as
/// `{"error": "<message>"}` at the status it carries.
///
/// A struct rather than an enum of cases: the status and the message are the
/// entire contract, and enumerating causes here would duplicate a taxonomy the
/// core already owns.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
    detail: Option<String>,
    retryable: Option<bool>,
    memories: Option<Vec<SavedMemory>>,
    job_id: Option<u64>,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self { status, message: message.into(), detail: None, retryable: None, memories: None, job_id: None }
    }
    /// The embedder will not load (or not yet, if `retryable`), so nothing
    /// that needs it can be served. `message` is a code a client branches on
    /// and `detail` the cause a person reads.
    fn embedder_failed(cause: String, retryable: bool) -> Self {
        Self {
            detail: Some(cause),
            retryable: Some(retryable),
            ..Self::new(StatusCode::SERVICE_UNAVAILABLE, "embedder_failed")
        }
    }
    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }
    fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }
    /// Anything the core reported. The `Display` text is forwarded verbatim
    /// because this API only ever listens on loopback for the user who started
    /// it — there is no third party to leak internals to, and a redacted
    /// message would make the one user who can see it debug blind.
    fn internal(e: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: self.message,
            detail: self.detail,
            retryable: self.retryable,
            memories: self.memories,
            job_id: self.job_id,
        };
        (self.status, Json(body)).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

// ── Pure helpers ────────────────────────────────────────────────────────────

/// Length-then-content comparison that does not stop at the first differing
/// byte.
///
/// The threat is modest — an attacker already needs to reach loopback — but the
/// fix costs one fold, and the alternative is a `==` whose early return leaks
/// how many leading bytes of the token a guess got right.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ── Request / response bodies ───────────────────────────────────────────────

/// Strict for the same reason as [`ListQuery`](memories::ListQuery): a misspelt filter would answer
/// with the totals of everything.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatsQuery {
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
}

/// Which agent a call by id acts for: it reaches what `sharing.json` lets
/// that agent read, whoever saved it, and a row out of its reach is answered
/// as missing. Absent, the call is the user's and reaches any row. Strict for
/// the same reason as [`ListQuery`](memories::ListQuery): a misspelt `agent` would quietly lift the
/// reach.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderQuery {
    #[serde(default)]
    pub agent: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct HealthBody {
    pub status: &'static str,
    pub version: &'static str,
    pub profile: &'static str,
    pub pid: u32,
    /// `"ready"` once the embedder cell is filled, `"loading"` until then,
    /// `"failed"` when the bootstrap could not load it. Health never waits on
    /// it: a daemon whose model is still downloading is fully usable for
    /// listing, and a client that could not tell the difference would sit
    /// through a 470 MB download before its first request.
    pub embedder: &'static str,
    /// The cause when `embedder` is `"failed"`, `null` otherwise.
    pub embedder_error: Option<String>,
    /// Whether the daemon retries the failed load by itself; `null` unless
    /// `embedder` is `"failed"`.
    pub embedder_retryable: Option<bool>,
    /// Whether the model is in memory now. `false` means a `"ready"` daemon
    /// let it go for being idle and the next request that needs it waits for
    /// it to load again; `null` until the bootstrap loaded it once.
    pub embedder_resident: Option<bool>,
    pub uptime_s: u64,
    /// Saves accepted and not yet stored, and those that failed after the
    /// caller was answered. Left out when the store did not answer in time:
    /// health must not hang behind a busy writer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingest: Option<OutboxSummary>,
    /// The import job that runs, if one does. Read from memory, so it is
    /// there when the store is too busy to answer `ingest`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import: Option<JobView>,
    /// The docs refresh that runs and the last one that ended, for each
    /// project refreshed since the daemon started. Read from memory like
    /// `import`, so it is there when the store is too busy to answer `ingest`.
    pub docs_index: Vec<project_docs::IndexStatus>,
}

#[derive(Debug, Serialize)]
pub struct QueuedBody {
    pub status: &'static str,
}

/// The tests of the daemon and of the bridge name it from here.
pub use clawg_core::memory::save::MAX_CONTENT_BYTES;

/// Builds the daemon's router.
///
/// Every route, `/health` included, sits behind [`admit_loopback`], so a route
/// added later is covered whether or not it asks for the token.
pub fn router(app: Arc<AppCtx>) -> Router {
    let v1 = Router::new()
        .route("/memories", post(save_memory).get(list_memories))
        .route("/memories/move", post(move_memories))
        .route("/search", post(search))
        .route("/context", post(context))
        .route("/sessions", post(register_session))
        .route("/sessions/end", post(end_session))
        .route("/sessions/calls", post(count_tool_call))
        .route(
            "/memories/{kind}/{id}",
            get(get_memory).patch(update_memory).delete(delete_memory),
        )
        .route("/memories/{kind}/{id}/save", get(get_save))
        .route("/memories/{kind}/{id}/restore", post(restore_memory))
        .route("/disputes", get(disputes))
        .route("/disputes/resolve", post(resolve_dispute))
        .route("/disputes/judge", post(judge_dispute))
        .route("/synthesis", post(synthesis))
        .route("/outbox/retry", post(retry_outbox))
        .route("/outbox/discard", post(discard_outbox))
        .route("/stats", get(stats))
        .route("/metrics", get(metrics))
        .route("/backup", post(backup))
        .route("/import", post(import_memories).layer(axum::extract::DefaultBodyLimit::max(IMPORT_BODY_LIMIT)))
        .route("/import/undo", post(undo_import))
        .route("/imports", post(start_import_job))
        .route("/imports/{id}", get(import_job).delete(cancel_import_job))
        .route("/projects", post(register).get(projects))
        .route("/projects/{id}", patch(rename))
        .route("/projects/merge", post(merge_projects))
        .route("/projects/keep-apart", post(keep_projects_apart))
        .route("/projects/delete", post(delete_projects))
        .route("/projects/resolve", post(resolve_projects))
        .route("/projects/suggestions/answer", post(answer_suggestion))
        .route("/projects/locate", post(locate_project))
        .route("/projects/docs/search", post(search_docs))
        .route("/shutdown", post(shutdown))
        .layer(axum::middleware::from_fn_with_state(app.clone(), require_bearer));

    let meter = Arc::new(HttpMeter::new(app.runtime.metrics.clone()));
    Router::new()
        .route("/health", get(health))
        .nest("/v1", v1)
        .with_state(app)
        .layer(axum::middleware::from_fn_with_state(
            Arc::new(RefusalLog::default()),
            admit_loopback,
        ))
        .layer(axum::middleware::from_fn_with_state(meter, time_request))
}

/// [`router`] with its requests labelled as coming in on `transport`.
pub fn router_on(app: Arc<AppCtx>, transport: Transport) -> Router {
    router(app).layer(axum::Extension(transport))
}

/// Serves [`router`] on an OS-picked loopback port in a background task and
/// returns where it listens, with the task's handle.
///
/// For in-process callers (the evaluation harness) that need the real HTTP
/// surface without a daemon process, a data directory or a state file. The
/// server stops on `POST /v1/shutdown`, like the daemon, and awaiting the
/// handle is how a caller knows it has let go of the runtime it was built
/// around.
pub async fn serve_ephemeral(
    app: Arc<AppCtx>,
) -> std::io::Result<(std::net::SocketAddr, tokio::task::JoinHandle<std::io::Result<()>>)> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let addr = listener.local_addr()?;
    let mut stop = app.shutdown.subscribe();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(app))
            .with_graceful_shutdown(async move {
                let _ = stop.wait_for(|requested| *requested).await;
            })
            .await
    });
    Ok((addr, server))
}

/// Rejects a request a web page could have sent (see [`guard`](super::guard)).
async fn admit_loopback(
    State(refusals): State<Arc<RefusalLog>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let headers = req.headers();
    // A header that is not text is still a header: it reads as no name at all.
    let host = headers.get(axum::http::header::HOST).map(|v| v.to_str().unwrap_or(""));
    let origin = headers.get(axum::http::header::ORIGIN).map(|v| v.to_str().unwrap_or(""));
    if admits(host, req.uri().authority().map(|a| a.as_str()), origin) {
        return next.run(req).await;
    }
    if let Some(count) = refusals.refused(Instant::now()) {
        // The values stay out of the log: they are whatever the page chose.
        log::warn!(
            "refused {count} request(s) not addressed to a loopback name or carrying an Origin \
             header; a web page may be trying to reach the daemon"
        );
    }
    ApiError::new(StatusCode::FORBIDDEN, "forbidden").into_response()
}

/// Rejects anything under `/v1` without the daemon's bearer token.
async fn require_bearer(
    State(app): State<Arc<AppCtx>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let presented = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match presented {
        Some(t) if constant_time_eq(t.as_bytes(), app.token.as_bytes()) => next.run(req).await,
        // One answer for every way of failing — missing header, wrong scheme,
        // wrong token. Distinguishing them would tell a caller which half of
        // the guess to keep.
        _ => ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// How long health waits for the outbox counts before answering without them.
const HEALTH_INGEST_WAIT: Duration = Duration::from_millis(250);

async fn health(State(app): State<Arc<AppCtx>>) -> Json<HealthBody> {
    let (embedder, embedder_error, embedder_retryable) = match app.runtime.embedder_status() {
        EmbedderStatus::Loading => ("loading", None, None),
        EmbedderStatus::Ready => ("ready", None, None),
        EmbedderStatus::Failed { error, retryable } => ("failed", Some(error), Some(retryable)),
    };
    let ingest = match tokio::time::timeout(HEALTH_INGEST_WAIT, app.runtime.store.outbox_summary()).await {
        Ok(Ok(summary)) => Some(summary),
        Ok(Err(e)) => {
            log::warn!("health could not read the ingest outbox: {e}");
            None
        }
        Err(_) => None,
    };
    Json(HealthBody {
        status: "ok",
        version: clawg_core::build_info::version(),
        profile: app.profile.as_str(),
        pid: std::process::id(),
        embedder,
        embedder_error,
        embedder_retryable,
        embedder_resident: app.runtime.embedder.get().map(|e| e.resident()),
        uptime_s: app.started_at.elapsed().as_secs(),
        ingest,
        import: app.imports.active(),
        docs_index: app.runtime.project_docs.statuses(),
    })
}

/// A model that was let go for being idle and could not load again is not
/// ready, the same answer as before the first load: the next call retries.
fn embed_failure(e: EmbedError) -> ApiError {
    match e {
        EmbedError::NotReady => ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "embedder_not_ready"),
        other => ApiError::internal(other),
    }
}

/// The loaded embedder, or the 503 a search answers while there is none.
fn ready_embedder(app: &AppCtx) -> ApiResult<Arc<dyn Embedder>> {
    app.runtime.embedder.get().cloned().ok_or_else(|| match app.runtime.embedder_status() {
        EmbedderStatus::Failed { error, retryable } => ApiError::embedder_failed(error, retryable),
        EmbedderStatus::Loading | EmbedderStatus::Ready => {
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "embedder_not_ready")
        }
    })
}

/// The live project a request's `project` leads to: an open bridge keeps the
/// id of a project merged since for its whole session, and old exports carry
/// it. Every entry that reads under a project goes through it; the ones that
/// write follow it again in the transaction that writes.
async fn live_project(app: &AppCtx, project: Option<String>) -> ApiResult<Option<String>> {
    match project {
        Some(id) => canonical(&app.runtime.store, &id).await.map(Some).map_err(ApiError::internal),
        None => Ok(None),
    }
}

/// `agent` with what the policy in the data directory lets it reach, read on
/// every call so a change counts from the next request on. A policy that
/// cannot be read shares nothing (`sharing::reader_in`), and the user finds
/// the reason in the log and in `memory_status`.
fn reader(app: &AppCtx, agent: &str) -> Reader {
    sharing::reader_in(&app.runtime.base_dir, agent)
}

/// The reader a call by id names, or `None` for the user's own call.
fn named_reader(app: &AppCtx, agent: Option<&str>) -> Option<Reader> {
    agent.map(|a| reader(app, a))
}

/// What `agent`'s searches reach.
fn reach(app: &AppCtx, agent: &str) -> Reach {
    reader(app, agent).reach
}

/// Counts what is stored, optionally narrowed to an agent and/or project.
#[derive(Debug, Deserialize)]
struct MetricsQuery {
    format: Option<String>,
}

/// What this run measured: the minute in progress and the whole run, as JSON
/// or, with `format=prometheus`, as Prometheus text.
async fn metrics(State(app): State<Arc<AppCtx>>, Query(q): Query<MetricsQuery>) -> ApiResult<Response> {
    match q.format.as_deref() {
        None | Some("json") => {
            let started_at_ms = now_ms().saturating_sub(app.started_at.elapsed().as_millis() as u64);
            Ok(Json(metrics_body(&app.runtime.metrics, clawg_core::build_info::version(), started_at_ms)).into_response())
        }
        Some("prometheus") => Ok((
            [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
            prometheus_text(&app.runtime.metrics),
        )
            .into_response()),
        Some(other) => Err(ApiError::bad_request(format!("unknown format {other:?}; use json or prometheus"))),
    }
}

/// The stats of the store. A bridge of a version before V32 requires the
/// counts of trajectories and skills, so they are sent as 0.
async fn stats(
    State(app): State<Arc<AppCtx>>,
    Query(q): Query<StatsQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let project_id = live_project(&app, q.project).await?;
    let stats = memory_stats_core(
        &app.runtime.store,
        StatsParams { agent_id: q.agent, project_id },
    )
    .await
    .map_err(ApiError::internal)?;
    let mut body = serde_json::to_value(stats).map_err(ApiError::internal)?;
    if let Some(totals) = body.get_mut("totals").and_then(serde_json::Value::as_object_mut) {
        totals.insert("episodic".into(), 0.into());
        totals.insert("procedural".into(), 0.into());
    }
    Ok(Json(body))
}

async fn shutdown(State(app): State<Arc<AppCtx>>) -> (StatusCode, Json<QueuedBody>) {
    // `send` only fails when every receiver is gone, which means the serve loop
    // has already left — there is nothing left to ask, and nothing to report.
    let _ = app.shutdown.send(true);
    // 202, not 200: the response has to be written and flushed BEFORE the
    // listener closes, so the work this acknowledges necessarily outlives it.
    (StatusCode::ACCEPTED, Json(QueuedBody { status: "shutting_down" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serve_ephemeral_answers_over_http_on_the_address_it_returns() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (runtime, _rx) = MemoryRuntime::new_for_test(dir.path().to_path_buf()).await;
        let (shutdown, _) = tokio::sync::watch::channel(false);
        let app = Arc::new(AppCtx {
            runtime,
            token: "ephemeral-token".to_string(),
            profile: Profile::Dev,
            started_at: Instant::now(),
            shutdown,
            save_timeout: SAVE_CONFIRM_TIMEOUT,
            sessions: Default::default(),
            imports: Default::default(),
        });

        let (addr, server) = serve_ephemeral(app).await.expect("bind an ephemeral loopback port");

        assert!(addr.ip().is_loopback());
        assert_ne!(addr.port(), 0, "the returned address must carry the port the OS picked");
        let health = reqwest::get(format!("http://{addr}/health")).await.expect("health");
        assert_eq!(health.status(), StatusCode::OK);
        let unauthorized = reqwest::get(format!("http://{addr}/v1/stats")).await.expect("stats");
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED, "it is the real router, bearer layer included");

        let stop = reqwest::Client::new()
            .post(format!("http://{addr}/v1/shutdown"))
            .bearer_auth("ephemeral-token")
            .send()
            .await
            .expect("shutdown");
        assert_eq!(stop.status(), StatusCode::ACCEPTED);
        tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .expect("the server must stop once shutdown is requested")
            .expect("the server task must not panic")
            .expect("a graceful stop is not an error");
    }

    #[test]
    fn constant_time_eq_matches_equality_for_equal_and_unequal_inputs() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"), "length must be part of it");
        assert!(constant_time_eq(b"", b""));
    }

    /// The property the fold buys: a mismatch in the FIRST byte and a mismatch
    /// in the LAST byte must both be a plain `false`, with no early exit
    /// between them for a caller to time.
    #[test]
    fn constant_time_eq_rejects_a_first_byte_and_a_last_byte_miss_alike() {
        assert!(!constant_time_eq(b"Xbcdef", b"abcdef"));
        assert!(!constant_time_eq(b"abcdeX", b"abcdef"));
    }
}
