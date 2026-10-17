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
    update_memory_core, ListParams, MemoryAdminRow, MemoryKind, MemoryPatch, MemoryStats, StatsParams,
    UpdatedRow, restore_memory_core, RestoreError, RestoredRow,
};
use clawg_core::memory::backup::Reason;
use clawg_core::memory::bootstrap::EmbedderStatus;
use clawg_core::memory::dispute::{resolve_dispute_core, Outcome, Resolution, ResolveError};
use clawg_core::memory::embedder::{EmbedError, EmbedRole, Embedder};
use clawg_core::memory::project_docs::{self, DocHit, DocIndexError, RefreshOutcome};
use clawg_core::memory::worker::{
    ChunkOutcome, IngestFailure, IngestReport, SavedChunk,
};
use clawg_core::memory::events::{MemoryEvent, MemorySource};
use clawg_core::memory::clock::TimestampMs;
use clawg_core::memory::project::{register_project, RegisterError};
use clawg_core::memory::prompt_builder::{build_memory_prompt, QueryTexts, Retrieval, DEFAULT_BUDGET};
use clawg_core::memory::sharing::{self, Reach};
use clawg_core::memory::synthesis::{
    existing_memory_ids, synthesis_core, unreviewed_count, Synthesis, SynthesisParams,
};
use clawg_core::memory::{read_save_core, search_all_memories, RankedHit, SaveText, SearchScope};
use clawg_core::models::chat::Role;
use clawg_core::paths::Profile;
use clawg_core::memory::outbox::{OutboxState, OutboxSummary};
use clawg_core::memory::redact::SecretKind;
use clawg_core::runtime::{AcceptError, Accepted, MemoryRuntime};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::relevance::shown_hits;

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

    let deleted = q.deleted.unwrap_or(false);
    if deleted && q.q.is_some() {
        return Err(ApiError::bad_request(
            "q cannot search deleted rows: deleting a memory takes it out of the text index",
        ));
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
        deleted,
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
/// Most sources one save may name. A conclusion rests on a handful of
/// memories; a list longer than this is a client passing a listing through,
/// and every id costs a lookup before the save is queued.
pub const MAX_SOURCES: usize = 50;

/// The sources a save names, each once and ascending, or why they are refused.
fn distinct_sources(ids: &[i64]) -> Result<Vec<i64>, ApiError> {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() > MAX_SOURCES {
        return Err(ApiError::bad_request(format!(
            "derived_from names {} memories; at most {MAX_SOURCES} are accepted",
            ids.len()
        )));
    }
    Ok(ids)
}

/// The requested sources that are not among the ones that exist, in order.
fn missing_sources(requested: &[i64], existing: &[i64]) -> Vec<i64> {
    requested.iter().copied().filter(|id| !existing.contains(id)).collect()
}

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
    /// Ids of the memories this one is a conclusion drawn from.
    #[serde(default)]
    pub derived_from: Vec<i64>,
    /// The caller's id for this save. Sending it again while the save waits
    /// or failed answers that save instead of recording another; absent, the
    /// daemon makes one up.
    #[serde(default)]
    pub operation_id: Option<String>,
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
    /// `true` lists the deleted rows instead of the live ones.
    #[serde(default)]
    pub deleted: Option<bool>,
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

/// Which agent a change acts for. Absent, the change is the store owner's and
/// reaches any row; present, another agent's row is answered as missing.
/// Strict for the same reason as [`ListQuery`]: a misspelt `agent` would
/// quietly lift the scope.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerQuery {
    #[serde(default)]
    pub agent: Option<String>,
}

/// `deny_unknown_fields` because both fields are optional: a misspelt one
/// would otherwise read as a request that changes nothing, and be answered
/// as if the mistake were the empty request.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRequest {
    #[serde(default)]
    pub importance: Option<f64>,
    #[serde(default)]
    pub memory_type: Option<String>,
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
    /// The kinds of secret masked out of the content before it was stored.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub redacted: Vec<&'static str>,
    /// The id this save is recorded under until it is stored.
    pub operation_id: String,
}

impl SaveBody {
    fn queued(reason: &'static str, operation_id: String) -> Self {
        Self { status: "queued", reason: Some(reason), memories: None, redacted: Vec::new(), operation_id }
    }
}

/// Longest `operation_id` a caller may send.
pub const MAX_OPERATION_ID_LEN: usize = 128;

/// A caller's operation id, or why it is refused: one to
/// [`MAX_OPERATION_ID_LEN`] visible ASCII characters, so it prints and
/// compares the same everywhere it is shown.
fn check_operation_id(id: Option<String>) -> Result<Option<String>, ApiError> {
    match id {
        Some(id) if id.is_empty() || id.len() > MAX_OPERATION_ID_LEN || !id.bytes().all(|b| b.is_ascii_graphic()) => {
            Err(ApiError::bad_request(format!(
                "operation_id must be 1 to {MAX_OPERATION_ID_LEN} visible ASCII characters"
            )))
        }
        id => Ok(id),
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
    /// Memories of the requested scope that arrived since it was last
    /// reviewed for conclusions (`POST /v1/synthesis`).
    pub unreviewed: usize,
    pub retrieval: RetrievalState,
}

/// Whether the context searched memories. Without it a prompt holding only
/// the workspace, or nothing, reads as "there are no memories".
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RetrievalState {
    Searched,
    EmbedderNotReady,
    EmbedderFailed { detail: String, retryable: bool },
    SearchFailed { detail: String },
}

impl RetrievalState {
    /// Reads a missing embedder from `status`, as [`ready_embedder`] does for
    /// a search, so the two never disagree on why memories cannot be reached.
    fn of(retrieval: Retrieval, status: EmbedderStatus) -> Self {
        match retrieval {
            Retrieval::Searched => Self::Searched,
            Retrieval::NoEmbedder => match status {
                EmbedderStatus::Failed { error, retryable } => Self::EmbedderFailed { detail: error, retryable },
                EmbedderStatus::Loading | EmbedderStatus::Ready => Self::EmbedderNotReady,
            },
            Retrieval::EmbedFailed(EmbedError::NotReady) => Self::EmbedderNotReady,
            Retrieval::EmbedFailed(e) => Self::SearchFailed { detail: e.to_string() },
            Retrieval::SearchFailed(detail) => Self::SearchFailed { detail },
        }
    }
}

/// Strict for the same reason as [`OwnerQuery`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthesisRequest {
    /// Reviews only this agent's memories, as [`OwnerQuery`] scopes a change.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Absent: the whole store.
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
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
    #[serde(flatten)]
    pub row: UpdatedRow,
}

#[derive(Debug, Serialize)]
pub struct DeletedBody {
    pub deleted: usize,
}

#[derive(Debug, Serialize)]
pub struct RestoredBody {
    pub restored: usize,
    #[serde(flatten)]
    pub row: RestoredRow,
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
    /// Scopes the resolution to this agent's rows, as [`OwnerQuery`] does.
    #[serde(default)]
    pub agent_id: Option<String>,
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
            get(get_memory).patch(update_memory).delete(delete_memory),
        )
        .route("/memories/{kind}/{id}/save", get(get_save))
        .route("/memories/{kind}/{id}/restore", post(restore_memory))
        .route("/disputes/resolve", post(resolve_dispute))
        .route("/synthesis", post(synthesis))
        .route("/stats", get(stats))
        .route("/backup", post(backup))
        .route("/projects", post(register))
        .route("/projects/docs/search", post(search_docs))
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
        version: env!("CARGO_PKG_VERSION"),
        profile: app.profile.as_str(),
        pid: std::process::id(),
        embedder,
        embedder_error,
        embedder_retryable,
        embedder_resident: app.runtime.embedder.get().map(|e| e.resident()),
        uptime_s: app.started_at.elapsed().as_secs(),
        ingest,
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

/// The answer to a save that waited for the worker. `masked` is what the
/// daemon masked before recording the save; the worker's own pass over the
/// masked text finds nothing more, except for an event nothing recorded.
fn save_response(
    confirmation: Confirmation,
    operation_id: String,
    masked: &[SecretKind],
) -> ApiResult<(StatusCode, SaveBody)> {
    let report = match confirmation {
        Confirmation::Unconfirmed => return Ok((StatusCode::ACCEPTED, SaveBody::queued("unconfirmed", operation_id))),
        Confirmation::Reported(report) => report,
    };
    let memories: Vec<SavedMemory> = report.memories.into_iter().map(SavedMemory::from).collect();
    let error = match report.failure {
        None => {
            return Ok((
                StatusCode::CREATED,
                SaveBody {
                    status: "saved",
                    reason: None,
                    memories: Some(memories),
                    redacted: masked_kinds(masked, &report.redacted),
                    operation_id,
                },
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

/// Each kind once, in the order first masked.
fn masked_kinds(before: &[SecretKind], worker: &[SecretKind]) -> Vec<&'static str> {
    let mut kinds: Vec<&'static str> = Vec::new();
    for kind in before.iter().chain(worker) {
        if !kinds.contains(&kind.as_str()) {
            kinds.push(kind.as_str());
        }
    }
    kinds
}

/// Saves a memory and answers with the rows it wrote: `201` once the worker
/// confirms, `202` when it cannot wait for that. Either way the save is
/// recorded in the outbox first, so a `202` outlives the process.
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
    let operation_id = check_operation_id(body.operation_id)?;
    // Checked here rather than by the worker: once queued, a save can only be
    // reported as failed long after the caller was told it was accepted.
    let derived_from = distinct_sources(&body.derived_from)?;
    if !derived_from.is_empty() {
        let existing = existing_memory_ids(&app.runtime.store, &derived_from, Some(&body.agent_id))
            .await
            .map_err(ApiError::internal)?;
        let missing = missing_sources(&derived_from, &existing);
        if !missing.is_empty() {
            let ids = missing.iter().map(i64::to_string).collect::<Vec<_>>().join(", ");
            return Err(ApiError::bad_request(format!(
                "derived_from names memories that do not exist or were deleted: {ids}"
            )));
        }
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
        derived_from,
    };

    let (reply, report) = if wait {
        let (tx, rx) = oneshot::channel();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };
    let accepted = app.runtime.accept(event, operation_id, reply).await.map_err(|e| match e {
        AcceptError::QueueFull => ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "ingest_queue_full"),
        AcceptError::WorkerStopped => ApiError::internal("the ingest worker has stopped; restart the daemon"),
        AcceptError::Store(e) => ApiError::internal(e),
    })?;
    let (row, operation_id, masked) = match accepted {
        Accepted::Queued { row, operation_id, redacted } => (row, operation_id, redacted),
        Accepted::Existing { operation_id, state: OutboxState::Pending, .. } => {
            return Ok((StatusCode::ACCEPTED, Json(SaveBody::queued("already_queued", operation_id))));
        }
        Accepted::Existing { state: OutboxState::Failed, last_error, .. } => {
            return Err(ApiError { detail: last_error, ..ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "ingest_failed") });
        }
    };

    let Some(report) = report else {
        return Ok((StatusCode::ACCEPTED, Json(SaveBody::queued("embedder_loading", operation_id))));
    };
    let confirmation = match tokio::time::timeout(app.save_timeout, report).await {
        Ok(Ok(report)) => Confirmation::Reported(report),
        Ok(Err(_)) | Err(_) => Confirmation::Unconfirmed,
    };
    // A failure this answer carries reaches the caller here, so nobody needs
    // the row to learn about it. Only the handler knows the report arrived in
    // time: the worker cannot tell a reply that was read from one sent just
    // as the wait ran out.
    if matches!(&confirmation, Confirmation::Reported(report) if report.failure.is_some()) {
        if let Err(e) = app.runtime.store.outbox_forget(row).await {
            log::warn!("could not drop the outbox row of a failed save: {e}; it is retried on the next start");
        }
    }
    save_response(confirmation, operation_id, &masked).map(|(status, body)| (status, Json(body)))
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

async fn search(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<SearchRequest>,
) -> ApiResult<Json<SearchBody>> {
    // A distinct status rather than empty results: "nothing matched" and "I
    // cannot search yet" are different answers, and a client that conflated
    // them would cache an empty corpus for the life of the model download.
    let embedder = ready_embedder(&app)?;

    let limit = clamp_search_limit(body.limit);
    let min_relative_score = check_relative_score(body.min_relative_score)?;
    let embedding = embedder
        .embed_with_role(&body.query, EmbedRole::Query)
        .await
        .map_err(embed_failure)?;

    let scope = SearchScope::all_default()
        .with_project(body.project_id.as_deref())
        .with_reach(reach(&app, &body.agent_id));
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
        let scored: Vec<(f64, bool)> = hits.iter().map(|h| (h.score, h.exact)).collect();
        hits = hits.into_iter().zip(shown_hits(&scored, ratio)).filter_map(|(h, shown)| shown.then_some(h)).collect();
    }
    app.runtime.access_tracker.record_hits(&hits);

    Ok(Json(SearchBody { hits }))
}

/// Whose memories `reader` may read, from the policy in the data directory,
/// read on every call so a change counts from the next request on. A policy
/// that cannot be read shares nothing, so an exclusion the user wrote is never
/// lost to a typo: the agent keeps its own memories and the user finds the
/// reason here and in `memory_status`.
fn reach(app: &AppCtx, reader: &str) -> Reach {
    match sharing::load(&app.runtime.base_dir) {
        Ok(policy) => policy.reach_for(reader),
        Err(e) => {
            log::warn!("{e}; nothing is shared");
            Reach::Own
        }
    }
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
        &reach(&app, &body.agent_id),
        QueryTexts { lexical: &body.query, embedded: &body.query },
        &emitted,
        Some(app.runtime.access_tracker.as_ref()),
        DEFAULT_BUDGET,
    )
    .await
    .map_err(ApiError::internal)?;

    // Same degradation as the regen above: a reminder that cannot be counted
    // is a reminder skipped, not a failed request.
    let unreviewed = unreviewed_count(&app.runtime.store, Some(&body.agent_id), project_id).await.unwrap_or_else(|e| {
        log::warn!("unreviewed count failed: {e}");
        0
    });

    let retrieval = RetrievalState::of(prompt.retrieval, app.runtime.embedder_status());
    Ok(Json(ContextBody { prompt: prompt.text, unreviewed, retrieval }))
}

async fn synthesis(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<SynthesisRequest>,
) -> ApiResult<Json<Synthesis>> {
    let params = SynthesisParams { agent_id: body.agent_id, project_id: body.project_id, limit: body.limit };
    synthesis_core(&app.runtime.store, params, TimestampMs::now())
        .await
        .map(Json)
        .map_err(ApiError::internal)
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

/// The save the row is a piece of, scoped like the other routes by id: a
/// row that is missing, deleted or another agent's is the same 404.
async fn get_save(
    State(app): State<Arc<AppCtx>>,
    Path((kind, id)): Path<(String, i64)>,
    Query(owner): Query<OwnerQuery>,
) -> ApiResult<Json<SaveText>> {
    let kind = parse_kind(&kind).ok_or_else(|| unknown_kind(&kind))?;
    let save = read_save_core(&app.runtime.store, kind, id, owner.agent.as_deref())
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("no {kind:?} row with id {id}")))?;
    Ok(Json(save))
}

async fn update_memory(
    State(app): State<Arc<AppCtx>>,
    Path((kind, id)): Path<(String, i64)>,
    Query(owner): Query<OwnerQuery>,
    Json(body): Json<UpdateRequest>,
) -> ApiResult<Json<UpdatedBody>> {
    let kind = parse_kind(&kind).ok_or_else(|| unknown_kind(&kind))?;
    let patch = MemoryPatch::new(kind, body.importance, body.memory_type.as_deref())
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    // The core answers `None` for "no such row", "already soft-deleted" and
    // "another agent's" alike, and from the caller's side those are the same
    // fact: the row it named is not there to change.
    let row = update_memory_core(&app.runtime.store, id, kind, &patch, owner.agent.as_deref())
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("no {kind:?} row with id {id}")))?;
    Ok(Json(UpdatedBody { updated: 1, row }))
}

async fn delete_memory(
    State(app): State<Arc<AppCtx>>,
    Path((kind, id)): Path<(String, i64)>,
    Query(owner): Query<OwnerQuery>,
) -> ApiResult<Json<DeletedBody>> {
    let kind = parse_kind(&kind).ok_or_else(|| unknown_kind(&kind))?;
    let deleted = delete_memory_core(&app.runtime.store, id, kind, owner.agent.as_deref())
        .await
        .map_err(ApiError::internal)?;

    if deleted == 0 {
        return Err(ApiError::not_found(format!("no {kind:?} row with id {id}")));
    }
    Ok(Json(DeletedBody { deleted }))
}

/// Brings back a deleted `memory` row, scoped like `DELETE`. The kind is
/// checked before the embedder, so a request that could never succeed says so
/// even while the model loads.
async fn restore_memory(
    State(app): State<Arc<AppCtx>>,
    Path((kind, id)): Path<(String, i64)>,
    Query(owner): Query<OwnerQuery>,
) -> ApiResult<Json<RestoredBody>> {
    let kind = parse_kind(&kind).ok_or_else(|| unknown_kind(&kind))?;
    if kind != MemoryKind::Memory {
        return Err(ApiError::bad_request(RestoreError::OnlyMemory(kind).to_string()));
    }
    let embedder = ready_embedder(&app)?;
    match restore_memory_core(&app.runtime.store, embedder.as_ref(), kind, id, owner.agent.as_deref()).await {
        Ok(row) => Ok(Json(RestoredBody { restored: 1, row })),
        Err(e @ RestoreError::OnlyMemory(_)) => Err(ApiError::bad_request(e.to_string())),
        Err(RestoreError::NotFound(_)) => Err(ApiError::not_found(format!("no deleted {kind:?} row with id {id}"))),
        Err(e @ (RestoreError::LiveTwin { .. } | RestoreError::HoldsSecret { .. })) => {
            Err(ApiError::conflict(e.to_string()))
        }
        Err(RestoreError::Embed(_, e)) => Err(embed_failure(e)),
        Err(e) => Err(ApiError::internal(e)),
    }
}

async fn resolve_dispute(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<ResolveRequest>,
) -> ApiResult<Json<Resolution>> {
    let outcome = if body.keep_both { Outcome::KeepBoth } else { Outcome::Supersede };
    resolve_dispute_core(&app.runtime.store, body.keep, body.other, outcome, body.agent_id.as_deref())
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
    /// Whether the daemon may read the root's files. False when missing, so a
    /// caller that does not say only labels memories with the root.
    #[serde(default)]
    pub index_docs: bool,
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
    let root = std::path::Path::new(&body.root_path);
    if body.index_docs && !root.is_dir() {
        return Err(ApiError::bad_request(format!("{} is not a directory, so it has no files to index", body.root_path)));
    }
    let outcome = register_project(
        &app.runtime.store,
        &app.runtime.workspaces(),
        root,
        &body.legacy_ids,
        body.index_docs,
    )
    .await
    .map_err(|e| match e {
        RegisterError::InvalidLegacyId(_) => ApiError::bad_request(e.to_string()),
        RegisterError::RootCollision { id, existing } => ApiError {
            detail: Some(format!("{} gets the project id {id}, which already belongs to {existing}", root.display())),
            ..ApiError::conflict("project_root_collision")
        },
        RegisterError::Store(_) => ApiError::internal(e),
    })?;
    // A bridge registers its root as its session starts, so the model that
    // went away while nobody worked starts loading before the first search.
    if let Some(embedder) = app.runtime.embedder.get() {
        embedder.wake();
    }
    if outcome.index_docs {
        spawn_doc_refresh(app.clone(), outcome.project_id.clone());
    }
    Ok(Json(RegisteredBody { project_id: outcome.project_id, rekeyed: outcome.rekeyed }))
}

/// Refreshes the project's documentation index in the background, once the
/// embedder has loaded: the bridge registers a root as its session starts,
/// usually before the model is ready. Nobody waits on it, so a failure is
/// logged and the next search tries again.
fn spawn_doc_refresh(app: Arc<AppCtx>, project_id: String) {
    tokio::spawn(async move {
        let mut status = app.runtime.embedder_status.subscribe();
        // A closed channel means the daemon is going away; the cell check
        // below then finds no embedder and gives up.
        let _ = status.wait_for(|s| !matches!(s, EmbedderStatus::Loading)).await.map(|_| ());
        let Some(embedder) = app.runtime.embedder.get().cloned() else {
            log::warn!("not indexing the docs of {project_id}: the embedder did not load");
            return;
        };
        let tokenizer = app.runtime.tokenizer.get().cloned();
        match app.runtime.project_docs.try_refresh(&app.runtime.store, embedder.as_ref(), tokenizer, &project_id).await {
            Ok(RefreshOutcome::Refreshed(r)) => log::info!(
                "indexed the docs of {project_id}: {} docs, {} reindexed, {} removed, {} skipped, {} passages",
                r.docs, r.indexed, r.removed, r.skipped, r.passages
            ),
            Ok(RefreshOutcome::AlreadyRunning) => {}
            Err(e) => log::warn!("could not index the docs of {project_id}: {e}"),
        }
    });
}

/// How long a search waits for a project's first index before answering
/// with what there is. Longer than indexing a typical repository's docs takes,
/// and short of what an MCP host waits for a tool.
pub const FIRST_INDEX_WAIT: Duration = Duration::from_secs(20);

async fn docs_in(store: &clawg_core::memory::MemoryStore, project_id: &str) -> ApiResult<usize> {
    project_docs::doc_count(store, project_id).await.map_err(ApiError::internal)
}

/// Largest `limit` a documentation search honours.
pub const MAX_DOCS_LIMIT: usize = 20;
/// `limit` of a documentation search when the caller does not say.
pub const DEFAULT_DOCS_LIMIT: usize = 5;

pub fn clamp_docs_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_DOCS_LIMIT).clamp(1, MAX_DOCS_LIMIT)
}

/// Strict for the same reason as [`OwnerQuery`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocsSearchRequest {
    pub project_id: String,
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DocsSearchBody {
    pub hits: Vec<DocHit>,
    /// Docs the project's index holds.
    pub docs: usize,
    /// A refresh of the project is still running, so the hits may be partial.
    pub indexing: bool,
    /// Why the refresh before this search failed; the hits come from what was
    /// already indexed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// Why a project's docs are not searched, and the one command that changes it.
///
/// The user runs it, not the model: a tool argument the model could send on
/// its own would not be a choice the user made.
fn not_accepted(root: &std::path::Path, profile: Profile) -> String {
    let flag = match profile {
        Profile::Prod => String::new(),
        other => format!(" --profile {}", other.as_str()),
    };
    format!(
        "clawg reads the files of the project a bridge was started in, and of the ones the user \
         added; {root} is neither. Ask the user to run `clawg projects add \"{root}\"{flag}` if \
         they want it searched, then call again",
        root = root.display()
    )
}

/// Searches a registered project's documentation, refreshing its index first
/// when the last refresh is older than [`project_docs::REFRESH_INTERVAL`].
async fn search_docs(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<DocsSearchRequest>,
) -> ApiResult<Json<DocsSearchBody>> {
    if body.query.trim().is_empty() {
        return Err(ApiError::bad_request("query must not be empty"));
    }
    let store = &app.runtime.store;
    project_docs::root_of(store, &body.project_id).await.map_err(|e| match e {
        DocIndexError::UnknownProject(_) => ApiError::not_found(e.to_string()),
        DocIndexError::NotAccepted(root) => ApiError::new(StatusCode::FORBIDDEN, not_accepted(&root, app.profile)),
        other => ApiError::internal(other),
    })?;
    let embedder = ready_embedder(&app)?;
    let index = &app.runtime.project_docs;
    let mut warning = None;
    if index.is_due(&body.project_id, Instant::now()) {
        let tokenizer = app.runtime.tokenizer.get().cloned();
        if let Err(e) = index.try_refresh(store, embedder.as_ref(), tokenizer, &body.project_id).await {
            warning = Some(e.to_string());
        }
    } else if index.is_running(&body.project_id) && docs_in(store, &body.project_id).await? == 0 {
        // The first index of a project is usually started by the bridge's
        // registration a moment before this search; answering from an empty
        // index would read as "the docs say nothing".
        index.wait_idle(&body.project_id, FIRST_INDEX_WAIT).await;
    }
    let hits = project_docs::search(store, embedder.as_ref(), &body.project_id, &body.query, clamp_docs_limit(body.limit))
        .await
        .map_err(ApiError::internal)?;
    let docs = docs_in(store, &body.project_id).await?;
    Ok(Json(DocsSearchBody { hits, docs, indexing: index.is_running(&body.project_id), warning }))
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
    fn the_refusal_names_the_command_for_the_daemons_profile() {
        let root = std::path::Path::new("/work/other");
        assert!(not_accepted(root, Profile::Prod).contains("`clawg projects add \"/work/other\"`"));
        assert!(not_accepted(root, Profile::Dev).contains("`clawg projects add \"/work/other\" --profile dev`"));
    }

    #[test]
    fn a_docs_search_returns_five_by_default_and_never_more_than_twenty() {
        assert_eq!(clamp_docs_limit(None), 5);
        assert_eq!(clamp_docs_limit(Some(0)), 1);
        assert_eq!(clamp_docs_limit(Some(12)), 12);
        assert_eq!(clamp_docs_limit(Some(500)), 20);
    }

    #[test]
    fn sources_are_counted_once_and_sorted() {
        assert_eq!(distinct_sources(&[9, 3, 9, 1]).unwrap(), vec![1, 3, 9]);
        assert_eq!(distinct_sources(&[]).unwrap(), Vec::<i64>::new());
    }

    #[test]
    fn too_many_sources_are_refused() {
        let at_cap: Vec<i64> = (0..MAX_SOURCES as i64).collect();
        assert!(distinct_sources(&at_cap).is_ok());
        let repeated: Vec<i64> = at_cap.iter().chain(at_cap.iter()).copied().collect();
        assert!(distinct_sources(&repeated).is_ok(), "the cap counts distinct ids");
        let over: Vec<i64> = (0..=MAX_SOURCES as i64).collect();
        assert_eq!(distinct_sources(&over).unwrap_err().status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn missing_sources_are_the_ones_that_were_not_found() {
        assert_eq!(missing_sources(&[1, 2, 3, 999], &[1, 3]), vec![2, 999]);
        assert!(missing_sources(&[1, 3], &[1, 3]).is_empty());
    }

    #[test]
    fn save_response_covers_every_worker_answer() {
        let saved = save_response(Confirmation::Reported(IngestReport {
            memories: vec![chunk(57, ChunkOutcome::Inserted), chunk(12, ChunkOutcome::Reinforced)],
            failure: None,
            ..Default::default()
        }), "op-1".into(), &[])
        .expect("a clean report is a success");
        assert_eq!(saved.0, StatusCode::CREATED);
        assert_eq!(
            saved.1.memories,
            Some(vec![
                SavedMemory { kind: "memory", id: 57, outcome: "inserted" },
                SavedMemory { kind: "memory", id: 12, outcome: "reinforced" },
            ])
        );

        let unconfirmed = save_response(Confirmation::Unconfirmed, "op-1".into(), &[]).expect("not an error");
        assert_eq!(unconfirmed, (StatusCode::ACCEPTED, SaveBody::queued("unconfirmed", "op-1".into())));

        let partial = save_response(Confirmation::Reported(IngestReport {
            memories: vec![chunk(57, ChunkOutcome::Inserted)],
            failure: Some(IngestFailure::Failed("store insert failed: disk full".into())),
            ..Default::default()
        }), "op-1".into(), &[])
        .expect_err("a failure is an error");
        assert_eq!(partial.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(partial.message, "ingest_failed");
        assert_eq!(partial.detail.as_deref(), Some("store insert failed: disk full"));
        assert_eq!(partial.memories, Some(vec![SavedMemory { kind: "memory", id: 57, outcome: "inserted" }]));

        let unavailable = save_response(Confirmation::Reported(IngestReport {
            memories: vec![],
            failure: Some(IngestFailure::EmbedderUnavailable),
            ..Default::default()
        }), "op-1".into(), &[])
        .expect_err("a failure is an error");
        assert_eq!(unavailable.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(unavailable.message, "embedder_unavailable");
        assert_eq!(unavailable.memories, Some(vec![]));
    }

    #[test]
    fn operation_ids_are_checked() {
        assert_eq!(check_operation_id(None).unwrap(), None);
        assert_eq!(check_operation_id(Some("hook-42.a_b".into())).unwrap(), Some("hook-42.a_b".into()));
        let longest = "x".repeat(MAX_OPERATION_ID_LEN);
        assert_eq!(check_operation_id(Some(longest.clone())).unwrap(), Some(longest));
        for bad in ["".to_string(), "x".repeat(MAX_OPERATION_ID_LEN + 1), "has space".into(), "acción".into()] {
            assert_eq!(check_operation_id(Some(bad.clone())).unwrap_err().status, StatusCode::BAD_REQUEST, "{bad:?}");
        }
    }

    #[test]
    fn a_confirmed_save_names_its_operation_and_what_was_masked_before_it_was_recorded() {
        let (_, body) = save_response(
            Confirmation::Reported(IngestReport { memories: vec![chunk(57, ChunkOutcome::Inserted)], ..Default::default() }),
            "op-9".into(),
            &[SecretKind::GithubToken],
        )
        .expect("a clean report is a success");
        assert_eq!(body.operation_id, "op-9");
        assert_eq!(body.redacted, vec!["github_token"]);
    }

    #[test]
    fn a_confirmed_save_lists_the_kinds_it_masked() {
        let (_, body) = save_response(Confirmation::Reported(IngestReport {
            memories: vec![chunk(57, ChunkOutcome::Inserted)],
            failure: None,
            redacted: vec![
                clawg_core::memory::redact::SecretKind::GithubToken,
                clawg_core::memory::redact::SecretKind::Jwt,
            ],
        }), "op-1".into(), &[])
        .expect("a clean report is a success");
        assert_eq!(body.redacted, vec!["github_token", "jwt"]);

        let (_, plain) = save_response(Confirmation::Reported(IngestReport {
            memories: vec![chunk(58, ChunkOutcome::Inserted)],
            ..Default::default()
        }), "op-1".into(), &[])
        .expect("a clean report is a success");
        let json = serde_json::to_value(&plain).unwrap();
        assert!(json.get("redacted").is_none(), "{json}");
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
    fn deleted_listing_refuses_a_text_query() {
        let err = list_params_from_query(query(|q| {
            q.deleted = Some(true);
            q.q = Some("deploy".into());
        }))
        .expect_err("deleted rows have no text index");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("deleted"), "{}", err.message);

        assert!(list_params_from_query(query(|q| q.deleted = Some(true))).expect("valid").deleted);
        assert!(!list_params_from_query(query(|_| {})).expect("valid").deleted);
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
