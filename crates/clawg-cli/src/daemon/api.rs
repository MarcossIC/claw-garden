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

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clawg_core::memory::admin::{
    delete_memory_core, get_memory_core, list_memories_core, memory_stats_core, query_has_tokens,
    set_importance_core, ListParams, MemoryAdminRow, MemoryKind, MemoryStats, StatsParams,
};
use clawg_core::memory::backup::Reason;
use clawg_core::memory::bootstrap::EmbedderStatus;
use clawg_core::memory::dispute::{resolve_dispute_core, Outcome, Resolution, ResolveError};
use clawg_core::memory::embedder::EmbedRole;
use clawg_core::memory::worker::{
    ChunkOutcome, IngestFailure, IngestReport, IngestRequest, SavedChunk,
};
use clawg_core::memory::events::{MemoryEvent, MemorySource};
use clawg_core::memory::project::{register_project, RegisterError};
use clawg_core::memory::prompt_builder::{build_memory_prompt, QueryTexts};
use clawg_core::memory::{search_all_memories, RankedHit, SearchScope};
use clawg_core::models::chat::Role;
use clawg_core::paths::Profile;
use clawg_core::runtime::MemoryRuntime;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::oneshot;

use crate::relevance::relevant_len;

use super::lifecycle::now_ms;

/// Largest `limit` any search will honour. A retrieval request is a vector
/// scan plus a rerank; letting a caller ask for thousands turns one HTTP call
/// into a stall for every other caller sharing this single-threaded store.
pub const MAX_SEARCH_LIMIT: usize = 50;
/// `limit` used when the caller does not say.
pub const DEFAULT_SEARCH_LIMIT: usize = 10;

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
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self { status, message: message.into(), detail: None, retryable: None, memories: None }
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
        };
        (self.status, Json(body)).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

// ── Pure helpers ────────────────────────────────────────────────────────────

/// Parses the wire spelling of a memory kind.
///
/// A free function rather than `serde` on the path segment so the 400 can name
/// the accepted values; a deserializer failure would surface as axum's generic
/// "invalid path" text.
pub fn parse_kind(raw: &str) -> Option<MemoryKind> {
    match raw {
        "memory" => Some(MemoryKind::Memory),
        "episodic" => Some(MemoryKind::Episodic),
        "procedural" => Some(MemoryKind::Procedural),
        _ => None,
    }
}

/// The 400 body for a kind that is not one of the three.
fn unknown_kind(raw: &str) -> ApiError {
    ApiError::bad_request(format!(
        "unknown kind {raw:?}: accepted values are \"memory\", \"episodic\" and \"procedural\""
    ))
}

/// The lifecycle states a memory row can be filtered by.
const STATUSES: [&str; 3] = ["active", "disputed", "superseded"];

/// Validates a listing request and turns it into the core's parameters.
///
/// Pure, so each refusal is testable without a server. Every refusal is a 400
/// naming what was wrong: a viewer that quietly answered an unfiltered list to
/// a bad filter would present it as the result of that filter.
pub fn list_params_from_query(q: ListQuery) -> ApiResult<ListParams> {
    let mut kind = match q.kind.as_deref() {
        None => None,
        Some(raw) => Some(parse_kind(raw).ok_or_else(|| unknown_kind(raw))?),
    };

    if let Some(status) = q.status.as_deref() {
        if !STATUSES.contains(&status) {
            return Err(ApiError::bad_request(format!(
                "unknown status {status:?}: accepted values are \"active\", \"disputed\" and \"superseded\""
            )));
        }
    }

    if let Some(text) = q.q.as_deref() {
        // Only `memory` has a text index; another kind would silently ignore
        // the query and list every row.
        if kind.is_some_and(|k| k != MemoryKind::Memory) {
            return Err(ApiError::bad_request(
                "q only applies to kind \"memory\": the other kinds have no text index",
            ));
        }
        if !query_has_tokens(text) {
            return Err(ApiError::bad_request(
                "q must contain at least one letter or digit to search for",
            ));
        }
        // A text search names no kind when the caller means "any that can be
        // searched". Left unset, the merge of all three tables would list every
        // episodic and procedural row next to the matches.
        kind.get_or_insert(MemoryKind::Memory);
    }

    Ok(ListParams {
        kind,
        tier: q.tier,
        memory_type: q.memory_type,
        agent_id: q.agent,
        project_id: q.project,
        status: q.status,
        query: q.q,
        limit: q.limit,
        offset: q.offset,
    })
}

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

/// Clamps a caller-supplied search limit into `1..=MAX_SEARCH_LIMIT`,
/// defaulting when absent.
///
/// Clamps rather than rejects: an over-large `limit` is a caller being
/// optimistic, not a caller being wrong, and failing the request would teach
/// nothing that returning 50 rows does not.
pub fn clamp_search_limit(requested: Option<usize>) -> usize {
    requested
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .clamp(1, MAX_SEARCH_LIMIT)
}

/// Checks a caller's `min_relative_score`: a share of the best score, so
/// anything outside `0..=1` is a mistake rather than a strong preference.
///
/// Rejects instead of clamping, unlike the limit: a ratio above 1 would cut
/// every hit past the minimum, and an answer that short would read as "the
/// store has nothing else" when the caller only mistyped a number.
fn check_relative_score(ratio: Option<f64>) -> Result<Option<f64>, ApiError> {
    match ratio {
        Some(r) if !(0.0..=1.0).contains(&r) => {
            Err(ApiError::bad_request("min_relative_score must be between 0 and 1"))
        }
        r => Ok(r),
    }
}

// ── Request / response bodies ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SaveRequest {
    pub content: String,
    pub agent_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub importance: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub agent_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    /// Share of the best hit's score the others need to be returned. Absent,
    /// every ranked hit up to `limit` comes back.
    #[serde(default)]
    pub min_relative_score: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct ContextRequest {
    pub query: String,
    pub agent_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
}

/// `deny_unknown_fields` because every field here narrows the result: an
/// ignored typo (`agnet=x`) would answer with the unfiltered list, which reads
/// as a successful filter.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub tier: Option<i64>,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    /// Free text over memory content.
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

/// Strict for the same reason as [`ListQuery`]: a misspelt filter would answer
/// with the totals of everything.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatsQuery {
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportanceRequest {
    pub importance: f64,
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
    pub uptime_s: u64,
}

#[derive(Debug, Serialize)]
pub struct QueuedBody {
    pub status: &'static str,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct SaveBody {
    pub status: &'static str,
    /// Why the save is queued instead of confirmed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    /// One entry per stored chunk, in chunk order, once the save is confirmed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memories: Option<Vec<SavedMemory>>,
}

impl SaveBody {
    fn queued(reason: &'static str) -> Self {
        Self { status: "queued", reason: Some(reason), memories: None }
    }
}

/// A row a save wrote or reinforced. `kind` is always `"memory"`, spelled out
/// so the triple is exactly what `DELETE /v1/memories/{kind}/{id}` takes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SavedMemory {
    pub kind: &'static str,
    pub id: i64,
    pub outcome: &'static str,
}

impl From<SavedChunk> for SavedMemory {
    fn from(chunk: SavedChunk) -> Self {
        let outcome = match chunk.outcome {
            ChunkOutcome::Inserted => "inserted",
            ChunkOutcome::Reinforced => "reinforced",
        };
        Self { kind: "memory", id: chunk.id, outcome }
    }
}

#[derive(Debug, Serialize)]
pub struct SearchBody {
    pub hits: Vec<RankedHit>,
}

#[derive(Debug, Serialize)]
pub struct ContextBody {
    pub prompt: String,
}

#[derive(Debug, Serialize)]
pub struct ListBody {
    pub items: Vec<MemoryAdminRow>,
}

#[derive(Debug, Serialize)]
pub struct ItemBody {
    pub item: MemoryAdminRow,
}

#[derive(Debug, Serialize)]
pub struct UpdatedBody {
    pub updated: usize,
}

#[derive(Debug, Serialize)]
pub struct DeletedBody {
    pub deleted: usize,
}

/// Closes the dispute between two `memory` rows. Disputes only ever pair
/// `memory` rows, so there is no `kind`.
#[derive(Debug, Deserialize)]
pub struct ResolveRequest {
    pub keep: i64,
    pub other: i64,
    /// Both rows stand instead of `other` being superseded by `keep`.
    #[serde(default)]
    pub keep_both: bool,
}

// ── Router ──────────────────────────────────────────────────────────────────

/// Builds the daemon's router.
pub fn router(app: Arc<AppCtx>) -> Router {
    let v1 = Router::new()
        .route("/memories", post(save_memory).get(list_memories))
        .route("/search", post(search))
        .route("/context", post(context))
        .route(
            "/memories/{kind}/{id}",
            get(get_memory).patch(patch_importance).delete(delete_memory),
        )
        .route("/disputes/resolve", post(resolve_dispute))
        .route("/stats", get(stats))
        .route("/backup", post(backup))
        .route("/projects", post(register))
        .route("/shutdown", post(shutdown))
        .layer(axum::middleware::from_fn_with_state(app.clone(), require_bearer));

    Router::new()
        .route("/health", get(health))
        .nest("/v1", v1)
        .with_state(app)
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

async fn health(State(app): State<Arc<AppCtx>>) -> Json<HealthBody> {
    let (embedder, embedder_error, embedder_retryable) = match app.runtime.embedder_status() {
        EmbedderStatus::Loading => ("loading", None, None),
        EmbedderStatus::Ready => ("ready", None, None),
        EmbedderStatus::Failed { error, retryable } => ("failed", Some(error), Some(retryable)),
    };
    Json(HealthBody {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        profile: app.profile.as_str(),
        pid: std::process::id(),
        embedder,
        embedder_error,
        embedder_retryable,
        uptime_s: app.started_at.elapsed().as_secs(),
    })
}

/// How waiting for the worker's report on a save ended.
#[derive(Debug)]
enum Confirmation {
    Reported(IngestReport),
    /// The wait ran out, or the worker let go of the request without
    /// answering (it was stopping). The event may still be stored later.
    Unconfirmed,
}

/// The answer to a save that waited for the worker.
fn save_response(confirmation: Confirmation) -> ApiResult<(StatusCode, SaveBody)> {
    let report = match confirmation {
        Confirmation::Unconfirmed => return Ok((StatusCode::ACCEPTED, SaveBody::queued("unconfirmed"))),
        Confirmation::Reported(report) => report,
    };
    let memories: Vec<SavedMemory> = report.memories.into_iter().map(SavedMemory::from).collect();
    let error = match report.failure {
        None => {
            return Ok((
                StatusCode::CREATED,
                SaveBody { status: "saved", reason: None, memories: Some(memories) },
            ));
        }
        Some(IngestFailure::EmbedderUnavailable) => ApiError {
            detail: Some(IngestFailure::EmbedderUnavailable.to_string()),
            ..ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "embedder_unavailable")
        },
        Some(IngestFailure::Failed(detail)) => ApiError {
            detail: Some(detail),
            ..ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "ingest_failed")
        },
    };
    Err(ApiError { memories: Some(memories), ..error })
}

/// Saves a memory and answers with the rows it wrote: `201` once the worker
/// confirms, `202` when it cannot wait for that.
///
/// # Why the worker does the writing
///
/// Ingest chunks the text, embeds every chunk and runs the dedup gate, and
/// that gate is only correct while ONE sequential consumer drains the channel
/// (see `MemoryRuntime::ingest_worker`). Writing here instead would let a
/// second writer in behind the worker's back, which is how a duplicate becomes
/// two rows instead of one reinforcement. So the handler queues the event with
/// a reply channel and waits for the worker's report, up to
/// `AppCtx::save_timeout`.
///
/// While the model loads the handler does not wait at all: the first load can
/// be a 470 MB download, and the save answers `202 embedder_loading` instead.
async fn save_memory(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<SaveRequest>,
) -> ApiResult<(StatusCode, Json<SaveBody>)> {
    if body.content.trim().is_empty() {
        return Err(ApiError::bad_request("content must not be empty"));
    }
    // Refused before it is queued: with no embedder coming, a queued save
    // would sit in memory until the daemon stops and then be lost.
    let wait = match app.runtime.embedder_status() {
        EmbedderStatus::Failed { error, retryable } => {
            return Err(ApiError::embedder_failed(error, retryable));
        }
        EmbedderStatus::Loading => false,
        EmbedderStatus::Ready => true,
    };

    let event = MemoryEvent {
        agent_id: body.agent_id,
        project_id: body.project_id,
        // Explicit saves are attributed to the user: nothing derived them from
        // an assistant turn, someone asserted them.
        role: Role::User,
        content: body.content,
        // Only `Role::Tool` messages carry an error flag.
        is_error: None,
        // The one source that honours the caller's `memory_type`/`importance`.
        source: MemorySource::Explicit,
        timestamp_ms: now_ms(),
        memory_type: body.memory_type,
        importance: body.importance,
    };

    let (reply, report) = if wait {
        let (tx, rx) = oneshot::channel();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };
    app.runtime.memory_tx.try_send(IngestRequest { event, reply }).map_err(|e| match e {
        TrySendError::Full(_) => ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "ingest_queue_full"),
        TrySendError::Closed(_) => {
            ApiError::internal("the ingest worker has stopped; restart the daemon")
        }
    })?;

    let Some(report) = report else {
        return Ok((StatusCode::ACCEPTED, Json(SaveBody::queued("embedder_loading"))));
    };
    let confirmation = match tokio::time::timeout(app.save_timeout, report).await {
        Ok(Ok(report)) => Confirmation::Reported(report),
        Ok(Err(_)) | Err(_) => Confirmation::Unconfirmed,
    };
    save_response(confirmation).map(|(status, body)| (status, Json(body)))
}

async fn search(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<SearchRequest>,
) -> ApiResult<Json<SearchBody>> {
    // A distinct status rather than empty results: "nothing matched" and "I
    // cannot search yet" are different answers, and a client that conflated
    // them would cache an empty corpus for the life of the model download.
    let Some(embedder) = app.runtime.embedder.get().cloned() else {
        return Err(match app.runtime.embedder_status() {
            EmbedderStatus::Failed { error, retryable } => ApiError::embedder_failed(error, retryable),
            EmbedderStatus::Loading | EmbedderStatus::Ready => {
                ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "embedder_not_ready")
            }
        });
    };

    let limit = clamp_search_limit(body.limit);
    let min_relative_score = check_relative_score(body.min_relative_score)?;
    let embedding = embedder
        .embed_with_role(&body.query, EmbedRole::Query)
        .await
        .map_err(ApiError::internal)?;

    let scope = SearchScope::all_default().with_project(body.project_id.as_deref());
    let mut hits = search_all_memories(
        &app.runtime.store,
        &embedding,
        // The raw text drives the lexical (FTS5) channel beside the vector one.
        Some(&body.query),
        &body.agent_id,
        scope,
        limit,
    )
    .await
    .map_err(ApiError::internal)?;

    // The cut runs before recording: a hit the caller never receives was not
    // retrieved for it, and counting it would feed the ranking a row the
    // agent never read.
    if let Some(ratio) = min_relative_score {
        let scores: Vec<f64> = hits.iter().map(|h| h.score).collect();
        hits.truncate(relevant_len(&scores, ratio));
    }
    app.runtime.access_tracker.record_hits(&hits);

    Ok(Json(SearchBody { hits }))
}

/// Builds the 4-block memory prefix for a query.
///
/// The dense half of the query is the raw text: widening it with prior conversation turns needs a conversation
/// and a resolved tokenizer, and a daemon serving stateless HTTP has neither.
/// The lexical half is the raw text in both.
async fn context(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<ContextRequest>,
) -> ApiResult<Json<ContextBody>> {
    let workspace = app.runtime.workspaces();
    let project_id = body.project_id.as_deref();

    // Unconditional regen (~2-5 ms). A failure degrades to "no ids were
    // emitted", which can make the prompt repeat a row — a degraded prompt
    // beats a failed request.
    let emitted = workspace
        .regenerate_auto_section(&body.agent_id, project_id, &app.runtime.store)
        .await
        .unwrap_or_else(|e| {
            log::warn!("workspace regen failed: {e}");
            Default::default()
        });

    let prompt = build_memory_prompt(
        &app.runtime.store,
        &workspace,
        app.runtime.embedder.get().cloned(),
        &body.agent_id,
        project_id,
        QueryTexts { lexical: &body.query, embedded: &body.query },
        &emitted,
        Some(app.runtime.access_tracker.as_ref()),
    )
    .await
    .map_err(ApiError::internal)?;

    Ok(Json(ContextBody { prompt }))
}

async fn list_memories(
    State(app): State<Arc<AppCtx>>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<ListBody>> {
    let items = list_memories_core(&app.runtime.store, list_params_from_query(q)?)
        .await
        .map_err(ApiError::internal)?;

    Ok(Json(ListBody { items }))
}

/// Counts what is stored, optionally narrowed to an agent and/or project.
async fn stats(
    State(app): State<Arc<AppCtx>>,
    Query(q): Query<StatsQuery>,
) -> ApiResult<Json<MemoryStats>> {
    let stats = memory_stats_core(
        &app.runtime.store,
        StatsParams { agent_id: q.agent, project_id: q.project },
    )
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(stats))
}

/// One row with every column the viewer shows, soft-deleted rows included
/// (they report `deleted_at`). Deliberately not a search: it records no access.
async fn get_memory(
    State(app): State<Arc<AppCtx>>,
    Path((kind, id)): Path<(String, i64)>,
) -> ApiResult<Json<ItemBody>> {
    let kind = parse_kind(&kind).ok_or_else(|| unknown_kind(&kind))?;
    let item = get_memory_core(&app.runtime.store, kind, id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("no {kind:?} row with id {id}")))?;
    Ok(Json(ItemBody { item }))
}

async fn patch_importance(
    State(app): State<Arc<AppCtx>>,
    Path((kind, id)): Path<(String, i64)>,
    Json(body): Json<ImportanceRequest>,
) -> ApiResult<Json<UpdatedBody>> {
    let kind = parse_kind(&kind).ok_or_else(|| unknown_kind(&kind))?;
    let updated = set_importance_core(&app.runtime.store, id, kind, body.importance, None)
        .await
        .map_err(ApiError::internal)?;

    // The core answers 0 for "no such row" and for "already soft-deleted"
    // alike, and from the caller's side those are the same fact: the row it
    // named is not there to change.
    if updated == 0 {
        return Err(ApiError::not_found(format!("no {kind:?} row with id {id}")));
    }
    Ok(Json(UpdatedBody { updated }))
}

async fn delete_memory(
    State(app): State<Arc<AppCtx>>,
    Path((kind, id)): Path<(String, i64)>,
) -> ApiResult<Json<DeletedBody>> {
    let kind = parse_kind(&kind).ok_or_else(|| unknown_kind(&kind))?;
    let deleted = delete_memory_core(&app.runtime.store, id, kind, None)
        .await
        .map_err(ApiError::internal)?;

    if deleted == 0 {
        return Err(ApiError::not_found(format!("no {kind:?} row with id {id}")));
    }
    Ok(Json(DeletedBody { deleted }))
}

async fn resolve_dispute(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<ResolveRequest>,
) -> ApiResult<Json<Resolution>> {
    let outcome = if body.keep_both { Outcome::KeepBoth } else { Outcome::Supersede };
    resolve_dispute_core(&app.runtime.store, body.keep, body.other, outcome, None)
        .await
        .map(Json)
        .map_err(|e| match e {
            ResolveError::SameRow => ApiError::bad_request(e.to_string()),
            ResolveError::NotFound(_) => ApiError::not_found(e.to_string()),
            ResolveError::NotInDispute { .. } | ResolveError::SupersededElsewhere { .. } => {
                ApiError::conflict(e.to_string())
            }
            ResolveError::Store(_) => ApiError::internal(e),
        })
}

#[derive(Debug, Serialize)]
pub struct BackupBody {
    pub path: String,
}

async fn backup(State(app): State<Arc<AppCtx>>) -> ApiResult<Json<BackupBody>> {
    let path = app.runtime.snapshot(Reason::Manual).await.map_err(ApiError::internal)?;
    Ok(Json(BackupBody { path: path.display().to_string() }))
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    /// The canonical project root, as the bridge resolved it.
    pub root_path: String,
    /// Ids the same project was saved under before paths were resolved.
    #[serde(default)]
    pub legacy_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisteredBody {
    pub project_id: String,
    pub rekeyed: usize,
}

/// Records a project root and moves rows saved under its legacy ids onto it.
///
/// The id is derived here from `root_path` rather than taken from the caller,
/// so the row in `project` can never name a root that does not hash to it.
async fn register(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<RegisterRequest>,
) -> ApiResult<Json<RegisteredBody>> {
    if body.root_path.trim().is_empty() {
        return Err(ApiError::bad_request("root_path must not be empty"));
    }
    let outcome = register_project(
        &app.runtime.store,
        &app.runtime.workspaces(),
        std::path::Path::new(&body.root_path),
        &body.legacy_ids,
    )
    .await
    .map_err(|e| match e {
        RegisterError::InvalidLegacyId(_) => ApiError::bad_request(e.to_string()),
        RegisterError::RootCollision { .. } => ApiError::conflict(e.to_string()),
        RegisterError::Store(_) => ApiError::internal(e),
    })?;
    Ok(Json(RegisteredBody { project_id: outcome.project_id, rekeyed: outcome.rekeyed }))
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

    fn chunk(id: i64, outcome: ChunkOutcome) -> SavedChunk {
        SavedChunk { id, outcome }
    }

    #[test]
    fn save_response_covers_every_worker_answer() {
        let saved = save_response(Confirmation::Reported(IngestReport {
            memories: vec![chunk(57, ChunkOutcome::Inserted), chunk(12, ChunkOutcome::Reinforced)],
            failure: None,
        }))
        .expect("a clean report is a success");
        assert_eq!(saved.0, StatusCode::CREATED);
        assert_eq!(
            saved.1.memories,
            Some(vec![
                SavedMemory { kind: "memory", id: 57, outcome: "inserted" },
                SavedMemory { kind: "memory", id: 12, outcome: "reinforced" },
            ])
        );

        let unconfirmed = save_response(Confirmation::Unconfirmed).expect("not an error");
        assert_eq!(unconfirmed, (StatusCode::ACCEPTED, SaveBody::queued("unconfirmed")));

        let partial = save_response(Confirmation::Reported(IngestReport {
            memories: vec![chunk(57, ChunkOutcome::Inserted)],
            failure: Some(IngestFailure::Failed("store insert failed: disk full".into())),
        }))
        .expect_err("a failure is an error");
        assert_eq!(partial.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(partial.message, "ingest_failed");
        assert_eq!(partial.detail.as_deref(), Some("store insert failed: disk full"));
        assert_eq!(partial.memories, Some(vec![SavedMemory { kind: "memory", id: 57, outcome: "inserted" }]));

        let unavailable = save_response(Confirmation::Reported(IngestReport {
            memories: vec![],
            failure: Some(IngestFailure::EmbedderUnavailable),
        }))
        .expect_err("a failure is an error");
        assert_eq!(unavailable.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(unavailable.message, "embedder_unavailable");
        assert_eq!(unavailable.memories, Some(vec![]));
    }

    #[test]
    fn a_relative_score_inside_zero_to_one_passes_through() {
        assert_eq!(check_relative_score(None).unwrap(), None);
        assert_eq!(check_relative_score(Some(0.0)).unwrap(), Some(0.0));
        assert_eq!(check_relative_score(Some(0.6)).unwrap(), Some(0.6));
        assert_eq!(check_relative_score(Some(1.0)).unwrap(), Some(1.0));
    }

    #[test]
    fn a_relative_score_outside_zero_to_one_is_a_bad_request() {
        for bad in [-0.1, 1.5, f64::NAN, f64::INFINITY] {
            let err = check_relative_score(Some(bad)).expect_err("must be refused");
            assert_eq!(err.status, StatusCode::BAD_REQUEST, "{bad}");
        }
    }

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
    fn parse_kind_accepts_the_three_wire_spellings() {
        assert_eq!(parse_kind("memory"), Some(MemoryKind::Memory));
        assert_eq!(parse_kind("episodic"), Some(MemoryKind::Episodic));
        assert_eq!(parse_kind("procedural"), Some(MemoryKind::Procedural));
    }

    #[test]
    fn parse_kind_rejects_anything_else() {
        assert_eq!(parse_kind("Memory"), None, "the wire spelling is lowercase");
        assert_eq!(parse_kind(""), None);
        assert_eq!(parse_kind("semantic"), None);
    }

    #[test]
    fn clamp_search_limit_defaults_when_absent() {
        assert_eq!(clamp_search_limit(None), DEFAULT_SEARCH_LIMIT);
    }

    #[test]
    fn clamp_search_limit_passes_a_reasonable_value_through() {
        assert_eq!(clamp_search_limit(Some(7)), 7);
    }

    #[test]
    fn clamp_search_limit_caps_at_the_maximum() {
        assert_eq!(clamp_search_limit(Some(5_000)), MAX_SEARCH_LIMIT);
    }

    /// `limit: 0` would return nothing while looking like a successful query —
    /// a silent empty answer is worse than the smallest useful one.
    #[test]
    fn clamp_search_limit_raises_zero_to_one() {
        assert_eq!(clamp_search_limit(Some(0)), 1);
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

    fn query(f: impl FnOnce(&mut ListQuery)) -> ListQuery {
        let mut q = ListQuery::default();
        f(&mut q);
        q
    }

    #[test]
    fn list_params_carry_every_filter_through() {
        let params = list_params_from_query(query(|q| {
            q.kind = Some("memory".into());
            q.tier = Some(2);
            q.memory_type = Some("semantic_fact".into());
            q.agent = Some("a1".into());
            q.project = Some("p1".into());
            q.status = Some("disputed".into());
            q.q = Some("deploy".into());
            q.limit = Some(5);
            q.offset = Some(10);
        }))
        .expect("valid");

        assert_eq!(params.kind, Some(MemoryKind::Memory));
        assert_eq!(params.tier, Some(2));
        assert_eq!(params.memory_type.as_deref(), Some("semantic_fact"));
        assert_eq!(params.agent_id.as_deref(), Some("a1"));
        assert_eq!(params.project_id.as_deref(), Some("p1"));
        assert_eq!(params.status.as_deref(), Some("disputed"));
        assert_eq!(params.query.as_deref(), Some("deploy"));
        assert_eq!((params.limit, params.offset), (Some(5), Some(10)));
    }

    #[test]
    fn list_params_reject_an_unknown_status_naming_the_accepted_set() {
        let err = list_params_from_query(query(|q| q.status = Some("stale".into())))
            .expect_err("invalid");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        for word in ["stale", "active", "disputed", "superseded"] {
            assert!(err.message.contains(word), "{}", err.message);
        }
    }

    #[test]
    fn list_params_reject_an_unknown_kind() {
        let err = list_params_from_query(query(|q| q.kind = Some("bogus".into())))
            .expect_err("invalid");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("bogus"), "{}", err.message);
    }

    #[test]
    fn a_text_query_with_a_non_memory_kind_is_rejected() {
        for kind in ["episodic", "procedural"] {
            let err = list_params_from_query(query(|q| {
                q.kind = Some(kind.into());
                q.q = Some("deploy".into());
            }))
            .expect_err("only memory has a text index");
            assert_eq!(err.status, StatusCode::BAD_REQUEST);
            assert!(err.message.contains("memory"), "{}", err.message);
        }
    }

    #[test]
    fn a_text_query_is_fine_without_a_kind_or_with_memory() {
        assert!(list_params_from_query(query(|q| q.q = Some("deploy".into()))).is_ok());
        assert!(list_params_from_query(query(|q| {
            q.kind = Some("memory".into());
            q.q = Some("deploy".into());
        }))
        .is_ok());
    }

    /// Without this, the unfiltered merge of the three tables would hand back
    /// every episodic and procedural row beside the matches, since neither has
    /// a text index to narrow it.
    #[test]
    fn a_text_query_without_a_kind_is_restricted_to_memory() {
        let params = list_params_from_query(query(|q| q.q = Some("deploy".into())))
            .expect("valid");
        assert_eq!(params.kind, Some(MemoryKind::Memory));
    }

    #[test]
    fn a_listing_without_a_text_query_keeps_every_table() {
        let params = list_params_from_query(query(|_| {})).expect("valid");
        assert_eq!(params.kind, None);
    }

    #[test]
    fn a_text_query_without_any_word_is_rejected() {
        for text in ["", "   ", "-^()\"* :"] {
            let err = list_params_from_query(query(|q| q.q = Some(text.into())))
                .expect_err("nothing to search for");
            assert_eq!(err.status, StatusCode::BAD_REQUEST, "{text:?}");
        }
    }

    #[test]
    fn unknown_kind_names_the_value_and_the_accepted_set() {
        let msg = unknown_kind("bogus").message;
        assert!(msg.contains("bogus"), "{msg}");
        assert!(msg.contains("memory"), "{msg}");
        assert!(msg.contains("episodic"), "{msg}");
        assert!(msg.contains("procedural"), "{msg}");
    }
}
