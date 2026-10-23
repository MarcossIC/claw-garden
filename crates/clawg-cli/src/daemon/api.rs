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
    Deleted, delete_memory_core, get_memory_core, list_memories_core, memory_stats_core, query_has_tokens,
    update_memory_core, ListParams, MemoryAdminRow, MemoryKind, MemoryPatch, MemoryStats, StatsParams,
    UpdateError, UpdatedRow, restore_memory_core, RestoreError, RestoredRow,
};
use clawg_core::memory::backup::Reason;
use clawg_core::memory::bootstrap::EmbedderStatus;
use clawg_core::memory::dispute::{judge_pair, open_pairs, resolve_dispute_core, OpenPairs, Outcome, Resolution, ResolveError, Verdict};
use clawg_core::memory::embedder::{EmbedError, EmbedRole, Embedder};
use clawg_core::memory::project_docs::{self, DocHit, DocIndexError, RefreshGuard};
use clawg_core::memory::worker::{
    ChunkOutcome, IngestFailure, IngestReport, SavedChunk,
};
use clawg_core::memory::events::{MemoryEvent, MemorySource};
use clawg_core::memory::clock::TimestampMs;
use clawg_core::memory::language::count_saves_in;
use clawg_core::memory::portability::{self, ImportError, ImportOptions, ImportReport, MAX_IMPORT_BYTES};
use clawg_core::memory::identity::origin_key;
use clawg_core::memory::project::{register_project, RegisterError, Registration};
use clawg_core::memory::relocate::{move_memories_core, undo_move_core, MoveError, MoveFilter, MoveReport, ProjectFilter};
use clawg_core::memory::prompt_builder::{build_memory_prompt, build_session_prompt, QueryTexts, Retrieval, DEFAULT_BUDGET, MAX_BUDGET};
use clawg_core::memory::sharing::{self, Reach, Reader};
use clawg_core::memory::topic::validate_topic_key;
use clawg_core::memory::synthesis::{
    conclusions_to_review_count, existing_memory_ids, replaced_memory_ids, synthesis_core, unreviewed_count, Synthesis, SynthesisParams,
};
use clawg_core::memory::{read_save_core, search_all_memories, RankedHit, SaveText, SearchScope};
use clawg_core::metrics::Recorder;
use clawg_core::models::chat::Role;
use clawg_core::paths::Profile;
use clawg_core::preferences::Lang;
use clawg_core::memory::outbox::{OutboxActionError, OutboxState, OutboxSummary};
use clawg_core::memory::redact::SecretKind;
use clawg_core::memory::store_insert::MemoryType;
use clawg_core::runtime::{AcceptError, Accepted, MemoryRuntime};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::relevance::shown_hits;

use super::guard::{admits, RefusalLog};
use super::lifecycle::now_ms;
use super::metrics::{metrics_body, prometheus_text, time_request, HttpMeter, Transport};
use super::sessions::{check_session_id, SessionRegistry};

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
    /// The host session each agent works in, per project, that saves are
    /// stamped with.
    pub sessions: SessionRegistry,
    /// The metrics of this run: every request, and what the writer samples.
    pub metrics: Arc<Recorder>,
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

    let no_project = q.no_project.unwrap_or(false);
    if no_project && q.project.is_some() {
        return Err(ApiError::bad_request("project and no_project cannot both be given: a row has one or none"));
    }

    Ok(ListParams {
        kind,
        tier: q.tier,
        memory_type: q.memory_type,
        agent_id: q.agent,
        project_id: q.project,
        no_project,
        status: q.status,
        origin_project: q.origin_project,
        query: q.q,
        limit: q.limit,
        offset: q.offset,
        deleted,
        ..Default::default()
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

/// `5 (by 9), 7` for replaced sources, each with what replaced it when known.
fn replaced_list(replaced: &[(i64, Option<i64>)]) -> String {
    replaced
        .iter()
        .map(|(id, by)| match by {
            Some(by) => format!("{id} (by {by})"),
            None => id.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn check_budget(budget: Option<usize>) -> Result<usize, ApiError> {
    match budget {
        Some(0) => Err(ApiError::bad_request("budget must be at least 1 byte")),
        Some(b) => Ok(b.min(MAX_BUDGET)),
        None => Ok(DEFAULT_BUDGET),
    }
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
    /// The content is what the user said, not something the agent inferred.
    #[serde(default)]
    pub user_stated: bool,
    /// The caller's id for this save. Sending it again while the save waits
    /// or failed answers that save instead of recording another; absent, the
    /// daemon makes one up.
    #[serde(default)]
    pub operation_id: Option<String>,
    /// The topic this save is a version of: it replaces the caller's live
    /// memories saved under the same key in the same project.
    #[serde(default)]
    pub topic_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SessionRequest {
    pub agent_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    pub session_id: String,
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
    /// The language the caller searches in, `en` or `es`. With it, the answer
    /// counts the live saves of the scope in the other one.
    #[serde(default)]
    pub lang: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ContextRequest {
    /// Absent or blank, the prompt holds the newest saves instead of a
    /// search, as a session starts with no question yet.
    #[serde(default)]
    pub query: Option<String>,
    pub agent_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    /// Most bytes the prompt may take. Absent, [`DEFAULT_BUDGET`]; above
    /// [`MAX_BUDGET`], that.
    #[serde(default)]
    pub budget: Option<usize>,
    /// As in [`SearchRequest`], counted only when there is a query; with or
    /// without one, the language of the notes the prompt adds.
    #[serde(default)]
    pub lang: Option<String>,
}

/// `deny_unknown_fields` because every field here narrows the result: an
/// ignored typo (`agnet=x`) would answer with the unfiltered list, which reads
/// as a successful filter.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    /// The agent the listing is for: it lists what that agent reaches, as a
    /// call by id does ([`ReaderQuery`]). `agent` stays a filter by author.
    #[serde(default)]
    pub reader: Option<String>,
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
    /// `true` lists only the rows saved in no project.
    #[serde(default)]
    pub no_project: Option<bool>,
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
    /// Only the rows that came from this project of another tool.
    #[serde(default)]
    pub origin_project: Option<String>,
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

/// Which agent a call by id acts for: it reaches what `sharing.json` lets
/// that agent read, whoever saved it, and a row out of its reach is answered
/// as missing. Absent, the call is the user's and reaches any row. Strict for
/// the same reason as [`ListQuery`]: a misspelt `agent` would quietly lift the
/// reach.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderQuery {
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
    /// The project to move the row's save to; it must be registered.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Moves the row's save out of every project.
    #[serde(default)]
    pub no_project: bool,
    /// The caller declares the user answered, which taking a preference or a
    /// decision out of its type, or out of its project, needs.
    #[serde(default)]
    pub user_answered: bool,
}

impl UpdateRequest {
    /// Where the request moves the row, `Some(None)` out of every project.
    pub fn destination(&self) -> ApiResult<Option<Option<String>>> {
        match (&self.project_id, self.no_project) {
            (Some(_), true) => Err(ApiError::bad_request("pass project_id or no_project, not both")),
            (Some(p), false) => Ok(Some(Some(p.clone()))),
            (None, true) => Ok(Some(None)),
            (None, false) => Ok(None),
        }
    }
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
    /// The earlier memories the contradiction detector paired with this save.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<RelatedMemory>,
    /// The memories of the save's topic key it superseded.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replaced: Vec<ReplacedMemory>,
    /// For a save without a topic key, the `memory` row of `related` in its
    /// own agent and project it is closest to: if the save revises it, the
    /// subject belongs under a key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_candidate: Option<i64>,
    /// The key the save was taken under. A bridge that sent one and finds it
    /// missing is talking to a daemon that ignored it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    /// Why a save that is still queued stopped, when it stopped part-way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The id this save is recorded under until it is stored.
    pub operation_id: String,
}

impl SaveBody {
    fn queued(reason: &'static str, operation_id: String) -> Self {
        Self {
            status: "queued",
            reason: Some(reason),
            memories: None,
            redacted: Vec::new(),
            related: Vec::new(),
            replaced: Vec::new(),
            revision_candidate: None,
            topic_key: None,
            detail: None,
            operation_id,
        }
    }
}

/// An earlier `memory` row a save under a topic key superseded.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplacedMemory {
    pub kind: &'static str,
    pub id: i64,
    pub excerpt: String,
}

/// An earlier `memory` row a save was found close to. `escalated` says the
/// pair waits for the user in ACTIVE CONFLICTS; otherwise it waits for the
/// caller's own verdict.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RelatedMemory {
    pub kind: &'static str,
    pub id: i64,
    pub escalated: bool,
    pub excerpt: String,
}

/// Most bytes a save's `content` may hold. Every piece of a long save
/// rewrites its outbox row with the progress so far, so the bytes a save
/// writes grow with the square of its length; this keeps the worst case near
/// 230 pieces and under 10 MB, and still takes the longest note an imported
/// history was measured to hold (45 372 characters).
pub const MAX_CONTENT_BYTES: usize = 64 * 1024;

/// A save's `content`, or why it is refused: empty, or over
/// [`MAX_CONTENT_BYTES`] as received. Refused rather than cut, since a cut
/// save would store a text the caller never wrote.
fn check_content(content: &str) -> Result<(), ApiError> {
    if content.trim().is_empty() {
        return Err(ApiError::bad_request("content must not be empty"));
    }
    if content.len() > MAX_CONTENT_BYTES {
        return Err(ApiError::bad_request(format!(
            "content is {} bytes; at most {MAX_CONTENT_BYTES} are accepted: split it into several saves, one fact each",
            content.len()
        )));
    }
    Ok(())
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
    /// Hits the relevance cut kept back, so a caller handed none can tell
    /// "nothing came close" from "nothing is stored".
    pub withheld: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_language: Option<OtherLanguage>,
}

#[derive(Debug, Serialize)]
pub struct ContextBody {
    pub prompt: String,
    /// Memories of the requested scope that arrived since it was last
    /// reviewed for conclusions (`POST /v1/synthesis`).
    pub unreviewed: usize,
    /// Conclusions of the scope that lost a source since it was last
    /// reviewed for conclusions.
    pub conclusions_to_review: usize,
    pub retrieval: RetrievalState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_language: Option<OtherLanguage>,
}

/// How many live saves of the searched scope are in the language the caller
/// did not search in, which a query in its own language is likely to miss.
#[derive(Debug, Serialize)]
pub struct OtherLanguage {
    pub lang: Lang,
    pub memories: usize,
}

/// The caller's language, or the 400 that lists the ones there are.
fn check_lang(lang: Option<&str>) -> Result<Option<Lang>, ApiError> {
    lang.map(|code| Lang::parse(code).ok_or_else(|| ApiError::bad_request(format!("lang must be one of: en, es (got {code:?})"))))
        .transpose()
}

/// The other language's count for a search in `lang`, with the scope and the
/// reach the search used. A count that fails is logged and left out: the
/// search already answered.
async fn other_language(app: &AppCtx, lang: Lang, project_id: Option<&str>, reader: &Reader) -> Option<OtherLanguage> {
    let other = lang.other();
    match count_saves_in(&app.runtime.store, other, project_id, reader).await {
        Ok(memories) => Some(OtherLanguage { lang: other, memories }),
        Err(e) => {
            log::warn!("could not count the memories in {}: {e}", other.name());
            None
        }
    }
}

/// Whether the context searched memories. Without it a prompt holding only
/// the workspace, or nothing, reads as "there are no memories".
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RetrievalState {
    /// `hits` went into the prompt; `withheld` scored 0 and were left out.
    Searched { hits: usize, withheld: usize },
    /// Asked for without a query: `hits` are the newest saves, by date.
    Recent { hits: usize },
    EmbedderNotReady,
    EmbedderFailed { detail: String, retryable: bool },
    SearchFailed { detail: String },
}

impl RetrievalState {
    /// Reads a missing embedder from `status`, as [`ready_embedder`] does for
    /// a search, so the two never disagree on why memories cannot be reached.
    fn of(retrieval: Retrieval, status: EmbedderStatus) -> Self {
        match retrieval {
            Retrieval::Searched { hits, withheld } => Self::Searched { hits, withheld },
            Retrieval::Recent { hits } => Self::Recent { hits },
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

/// Strict for the same reason as [`ReaderQuery`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthesisRequest {
    /// Reviews what this agent reaches, as [`ReaderQuery`] does.
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

/// What a delete did: deleting the current version of a topic brings back
/// the one its revision replaced (`revived`); a row a verdict replaced stays
/// superseded by the deleted row (`hidden`).
#[derive(Debug, Serialize)]
pub struct DeletedBody {
    pub deleted: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub revived: Vec<ReplacedMemory>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<i64>,
}

impl From<Deleted> for DeletedBody {
    fn from(d: Deleted) -> Self {
        Self {
            deleted: d.deleted,
            revived: d.revived.into_iter().map(|r| ReplacedMemory { kind: "memory", id: r.id, excerpt: r.excerpt }).collect(),
            hidden: d.hidden,
        }
    }
}

/// How a restored row ended up, with the rows its delete had brought back
/// and it supersedes again (`replaced`), and the rows saved while it was
/// deleted that the detector paired with it (`related`).
#[derive(Debug, Serialize)]
pub struct RestoredBody {
    pub restored: usize,
    pub id: i64,
    pub status: String,
    pub superseded_by: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replaced: Vec<ReplacedMemory>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub superseded_by_deleted: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<RelatedMemory>,
}

impl From<RestoredRow> for RestoredBody {
    fn from(r: RestoredRow) -> Self {
        Self {
            restored: 1,
            id: r.id,
            status: r.status,
            superseded_by: r.superseded_by,
            topic_key: r.topic_key,
            replaced: r.replaced.into_iter().map(|r| ReplacedMemory { kind: "memory", id: r.id, excerpt: r.excerpt }).collect(),
            superseded_by_deleted: r.superseded_by_deleted,
            related: r
                .related
                .into_iter()
                .map(|c| RelatedMemory { kind: "memory", id: c.id, escalated: c.escalated, excerpt: c.excerpt })
                .collect(),
        }
    }
}

/// A batch move: a selection and one target, or the number of a move to
/// undo. Strict because every field narrows or aims it: a misspelt filter
/// would move more than was meant.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveRequest {
    #[serde(default)]
    pub ids: Vec<i64>,
    #[serde(default)]
    pub from_project: Option<String>,
    #[serde(default)]
    pub from_none: bool,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    /// Free text over memory content, as `GET /v1/memories` reads `q`.
    #[serde(default)]
    pub q: Option<String>,
    /// The project the rows had in the tool they were imported from, whole.
    #[serde(default)]
    pub origin_project: Option<String>,
    #[serde(default)]
    pub include_deleted: bool,
    #[serde(default)]
    pub to_project: Option<String>,
    #[serde(default)]
    pub to_none: bool,
    #[serde(default)]
    pub undo: Option<i64>,
    #[serde(default)]
    pub dry_run: bool,
}

/// What a [`MoveRequest`] asks for, once it is known to ask for one thing.
#[derive(Debug, PartialEq)]
pub enum MovePlan {
    Move { filter: MoveFilter, target: Option<String> },
    Undo(i64),
}

impl MoveRequest {
    /// Pure, so each refusal is testable without a server.
    pub fn plan(self) -> ApiResult<MovePlan> {
        let from = match (self.from_project, self.from_none) {
            (Some(_), true) => return Err(ApiError::bad_request("pass from_project or from_none, not both")),
            (Some(p), false) => Some(ProjectFilter::Project(p)),
            (None, true) => Some(ProjectFilter::None),
            (None, false) => None,
        };
        let filter = MoveFilter {
            ids: self.ids,
            from,
            agent_id: self.agent,
            memory_type: self.memory_type,
            source: self.source,
            session_id: self.session_id,
            query: self.q,
            origin_project: self.origin_project,
            include_deleted: self.include_deleted,
        };
        match (self.undo, self.to_project, self.to_none) {
            (Some(id), None, false) if filter == MoveFilter::default() => Ok(MovePlan::Undo(id)),
            (Some(_), None, false) => Err(ApiError::bad_request("undo names a move: it takes no selection")),
            (None, Some(p), false) => Ok(MovePlan::Move { filter, target: Some(p) }),
            (None, None, true) => Ok(MovePlan::Move { filter, target: None }),
            (None, None, false) => Err(ApiError::bad_request("name a target: to_project or to_none, or a move to undo")),
            _ => Err(ApiError::bad_request("pass one of to_project, to_none or undo")),
        }
    }
}

/// Names a save the daemon gave up on: its operation id, and its agent when
/// another agent holds the same id.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboxRequest {
    pub operation_id: String,
    #[serde(default)]
    pub agent_id: Option<String>,
}

/// A failed save taken back to pending; `queued` is false when the queue was
/// full and it waits for the next start.
#[derive(Debug, Serialize)]
pub struct RetriedBody {
    pub operation_id: String,
    pub agent_id: String,
    pub queued: bool,
}

/// A failed save dropped, with the memories it had inserted, now deleted.
#[derive(Debug, Serialize)]
pub struct DiscardedBody {
    pub operation_id: String,
    pub agent_id: String,
    pub deleted: Vec<i64>,
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
    /// The agent the resolution acts for, as in [`ReaderQuery`].
    #[serde(default)]
    pub agent_id: Option<String>,
}

/// A verdict on a pair the contradiction detector found.
#[derive(Debug, Deserialize)]
pub struct JudgeRequest {
    pub keep: i64,
    pub other: i64,
    /// `supersedes`, `compatible`, `scoped` or `conflicts`.
    pub verdict: String,
    /// Why, required for `scoped`.
    #[serde(default)]
    pub reason: Option<String>,
    /// The caller declares the user answered, with the answer in `reason`.
    #[serde(default)]
    pub user_answered: bool,
    /// The agent the verdict acts for, as in [`ReaderQuery`].
    #[serde(default)]
    pub agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisputesQuery {
    /// The agent that judges, as in [`ReaderQuery`].
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Only the pairs whose newer row this project reaches, as a search reads
    /// it; absent, the whole store.
    #[serde(default)]
    pub project_id: Option<String>,
}

/// How many pending pairs `GET /v1/disputes` names.
const OPEN_PAIRS_SHOWN: usize = 5;

// ── Router ──────────────────────────────────────────────────────────────────

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
        .route("/projects", post(register))
        .route("/projects/docs/search", post(search_docs))
        .route("/shutdown", post(shutdown))
        .layer(axum::middleware::from_fn_with_state(app.clone(), require_bearer));

    let meter = Arc::new(HttpMeter::new(app.metrics.clone()));
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
                    related: report
                        .related
                        .into_iter()
                        .map(|c| RelatedMemory { kind: "memory", id: c.id, escalated: c.escalated, excerpt: c.excerpt })
                        .collect(),
                    replaced: report
                        .replaced
                        .into_iter()
                        .map(|r| ReplacedMemory { kind: "memory", id: r.id, excerpt: r.excerpt })
                        .collect(),
                    revision_candidate: report.revision_candidate,
                    topic_key: None,
                    detail: None,
                    operation_id,
                },
            ));
        }
        // The pieces stored so far stay noted in the save's row, which the
        // next start finishes: the save is accepted, not failed.
        Some(IngestFailure::Failed(detail)) if !memories.is_empty() => {
            return Ok((
                StatusCode::ACCEPTED,
                SaveBody {
                    memories: Some(memories),
                    detail: Some(detail),
                    ..SaveBody::queued("partially_saved", operation_id)
                },
            ));
        }
        // The worker keeps the save and tries it again once the embedder can
        // take it.
        Some(IngestFailure::EmbedderUnavailable) => {
            return Ok((
                StatusCode::ACCEPTED,
                SaveBody {
                    memories: Some(memories).filter(|m| !m.is_empty()),
                    detail: Some(IngestFailure::EmbedderUnavailable.to_string()),
                    ..SaveBody::queued("retrying", operation_id)
                },
            ));
        }
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
    check_content(&body.content)?;
    // Refused here, as `PATCH` refuses it: the worker would store it as
    // `semantic_fact` long after the caller was told the save was accepted.
    if let Some(raw) = body.memory_type.as_deref() {
        raw.parse::<MemoryType>().map_err(|e| ApiError::bad_request(e.to_string()))?;
    }
    let operation_id = check_operation_id(body.operation_id)?;
    if let Some(key) = body.topic_key.as_deref() {
        validate_topic_key(key).map_err(|e| ApiError::bad_request(e.to_string()))?;
    }
    let topic_key = body.topic_key.clone();
    // Checked here rather than by the worker: once queued, a save can only be
    // reported as failed long after the caller was told it was accepted.
    let derived_from = distinct_sources(&body.derived_from)?;
    // A conclusion is the agent's even when its sources are the user's words;
    // storing it as the agent's after this declaration would hide the refusal.
    if body.user_stated && !derived_from.is_empty() {
        return Err(ApiError::bad_request(
            "user_stated cannot go with derived_from: a conclusion is drawn by the agent, not stated by the user",
        ));
    }
    if !derived_from.is_empty() {
        let saver = reader(&app, &body.agent_id);
        let existing = existing_memory_ids(&app.runtime.store, &derived_from, Some(&saver))
            .await
            .map_err(ApiError::internal)?;
        let missing = missing_sources(&derived_from, &existing);
        if !missing.is_empty() {
            let ids = missing.iter().map(i64::to_string).collect::<Vec<_>>().join(", ");
            return Err(ApiError::bad_request(format!(
                "derived_from names memories that do not exist or were deleted: {ids}"
            )));
        }
        let replaced = replaced_memory_ids(&app.runtime.store, &derived_from, Some(&saver))
            .await
            .map_err(ApiError::internal)?;
        if !replaced.is_empty() {
            return Err(ApiError::bad_request(format!(
                "derived_from names memories that were replaced: {}; draw the conclusion from the current ones",
                replaced_list(&replaced)
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

    // Read now, not by the worker: a replay after a restart must keep the
    // session the save was accepted in.
    let session_id = app.sessions.current(&body.agent_id, body.project_id.as_deref());
    let event = MemoryEvent {
        agent_id: body.agent_id,
        project_id: body.project_id,
        // Not read for an explicit save: `provenance::seal` decides who
        // stated it.
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
        topic_key: body.topic_key,
        user_stated: body.user_stated,
        session_id,
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
            let body = SaveBody { topic_key, ..SaveBody::queued("already_queued", operation_id) };
            return Ok((StatusCode::ACCEPTED, Json(body)));
        }
        Accepted::Existing { state: OutboxState::Failed, last_error, .. } => {
            return Err(ApiError { detail: last_error, ..ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "ingest_failed") });
        }
    };

    let Some(report) = report else {
        let body = SaveBody { topic_key, ..SaveBody::queued("embedder_loading", operation_id) };
        return Ok((StatusCode::ACCEPTED, Json(body)));
    };
    let confirmation = match tokio::time::timeout(app.save_timeout, report).await {
        Ok(Ok(report)) => Confirmation::Reported(report),
        Ok(Err(_)) | Err(_) => Confirmation::Unconfirmed,
    };
    // A failure this answer carries reaches the caller here, so nobody needs
    // the row to learn about it, unless the save stored some of its pieces or
    // the worker tries it again: the row is then the only way to finish it.
    // Only the handler knows the report arrived in time: the worker cannot
    // tell a reply that was read from one sent just as the wait ran out.
    if matches!(
        &confirmation,
        Confirmation::Reported(IngestReport { failure: Some(IngestFailure::Failed(_)), memories, .. }) if memories.is_empty()
    ) {
        if let Err(e) = app.runtime.store.outbox_forget(row).await {
            log::warn!("could not drop the outbox row of a failed save: {e}; it is retried on the next start");
        }
    }
    save_response(confirmation, operation_id, &masked).map(|(status, body)| (status, Json(SaveBody { topic_key, ..body })))
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
    let lang = check_lang(body.lang.as_deref())?;
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

    let caller = reader(&app, &body.agent_id);
    let scope = SearchScope::all_default()
        .with_project(body.project_id.as_deref())
        .with_reach(caller.reach.clone());
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
    let found = hits.len();
    if let Some(ratio) = min_relative_score {
        let scored: Vec<(f64, bool)> = hits.iter().map(|h| (h.score, h.exact)).collect();
        hits = hits.into_iter().zip(shown_hits(&scored, ratio)).filter_map(|(h, shown)| shown.then_some(h)).collect();
    }
    app.runtime.access_tracker.record_hits(&hits);

    let withheld = found - hits.len();
    let other_language = match lang {
        Some(lang) => other_language(&app, lang, body.project_id.as_deref(), &caller).await,
        None => None,
    };
    Ok(Json(SearchBody { hits, withheld, other_language }))
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

/// Builds the 4-block memory prefix for a query.
///
/// The dense half of the query is the raw text: widening it with prior conversation turns needs a conversation
/// and a resolved tokenizer, and a daemon serving stateless HTTP has neither.
/// The lexical half is the raw text in both.
async fn context(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<ContextRequest>,
) -> ApiResult<Json<ContextBody>> {
    let budget = check_budget(body.budget)?;
    let lang = check_lang(body.lang.as_deref())?;
    let workspace = app.runtime.workspaces();
    let project_id = body.project_id.as_deref();
    let query = body.query.as_deref().filter(|q| !q.trim().is_empty());

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

    let prompt = match query {
        Some(query) => {
            build_memory_prompt(
                lang.unwrap_or(Lang::En),
                &app.runtime.store,
                &workspace,
                app.runtime.embedder.get().cloned(),
                &body.agent_id,
                project_id,
                &reach(&app, &body.agent_id),
                QueryTexts { lexical: query, embedded: query },
                &emitted,
                Some(app.runtime.access_tracker.as_ref()),
                budget,
            )
            .await
        }
        None => {
            let reach = reach(&app, &body.agent_id);
            build_session_prompt(
                lang.unwrap_or(Lang::En),
                &app.runtime.store,
                &workspace,
                &body.agent_id,
                project_id,
                &reach,
                &emitted,
                budget,
            )
            .await
        }
    }
    .map_err(ApiError::internal)?;

    // Same degradation as the regen above: a reminder that cannot be counted
    // is a reminder skipped, not a failed request.
    let caller = reader(&app, &body.agent_id);
    let unreviewed = unreviewed_count(&app.runtime.store, Some(&caller), project_id).await.unwrap_or_else(|e| {
        log::warn!("unreviewed count failed: {e}");
        0
    });
    let conclusions_to_review =
        conclusions_to_review_count(&app.runtime.store, Some(&caller), project_id).await.unwrap_or_else(|e| {
            log::warn!("conclusions to review count failed: {e}");
            0
        });

    let other_language = match (lang, query) {
        (Some(lang), Some(_)) => other_language(&app, lang, project_id, &caller).await,
        _ => None,
    };

    let retrieval = RetrievalState::of(prompt.retrieval, app.runtime.embedder_status());
    Ok(Json(ContextBody { prompt: prompt.text, unreviewed, conclusions_to_review, retrieval, other_language }))
}

/// Records the host session `agent_id` works in for `project_id`, as a hook
/// reports it; the saves that follow are stamped with it.
async fn register_session(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<SessionRequest>,
) -> ApiResult<StatusCode> {
    check_session_id(&body.session_id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    app.sessions.register(&body.agent_id, body.project_id.as_deref(), &body.session_id);
    Ok(StatusCode::NO_CONTENT)
}

async fn synthesis(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<SynthesisRequest>,
) -> ApiResult<Json<Synthesis>> {
    let reader = named_reader(&app, body.agent_id.as_deref());
    let params = SynthesisParams { reader, project_id: body.project_id, limit: body.limit };
    synthesis_core(&app.runtime.store, params, TimestampMs::now())
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

async fn list_memories(
    State(app): State<Arc<AppCtx>>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<ListBody>> {
    let reader = named_reader(&app, q.reader.as_deref());
    let params = ListParams { reader, ..list_params_from_query(q)? };
    let items = list_memories_core(&app.runtime.store, params)
        .await
        .map_err(ApiError::internal)?;

    Ok(Json(ListBody { items }))
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
            Ok(Json(metrics_body(&app.metrics, clawg_core::build_info::version(), started_at_ms)).into_response())
        }
        Some("prometheus") => Ok((
            [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
            prometheus_text(&app.metrics),
        )
            .into_response()),
        Some(other) => Err(ApiError::bad_request(format!("unknown format {other:?}; use json or prometheus"))),
    }
}

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

/// The save the row is a piece of, as far as the reader reaches it: a row
/// that is missing, deleted or out of reach is the same 404.
async fn get_save(
    State(app): State<Arc<AppCtx>>,
    Path((kind, id)): Path<(String, i64)>,
    Query(q): Query<ReaderQuery>,
) -> ApiResult<Json<SaveText>> {
    let kind = parse_kind(&kind).ok_or_else(|| unknown_kind(&kind))?;
    let reader = named_reader(&app, q.agent.as_deref());
    let save = read_save_core(&app.runtime.store, kind, id, reader.as_ref())
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("no {kind:?} row with id {id}")))?;
    Ok(Json(save))
}

async fn update_memory(
    State(app): State<Arc<AppCtx>>,
    Path((kind, id)): Path<(String, i64)>,
    Query(q): Query<ReaderQuery>,
    Json(body): Json<UpdateRequest>,
) -> ApiResult<Json<UpdatedBody>> {
    let kind = parse_kind(&kind).ok_or_else(|| unknown_kind(&kind))?;
    let patch = MemoryPatch::new(kind, body.importance, body.memory_type.as_deref(), body.destination()?)
        .map_err(|e| ApiError::bad_request(e.to_string()))?
        .answered(body.user_answered);
    // The core answers `None` for "no such row", "already soft-deleted" and
    // "out of reach" alike, and from the caller's side those are the same
    // fact: the row it named is not there to change.
    let reader = named_reader(&app, q.agent.as_deref());
    let row = update_memory_core(&app.runtime.store, id, kind, &patch, reader.as_ref())
        .await
        .map_err(|e| match e {
            UpdateError::NeedsUser { .. } | UpdateError::MoveNeedsUser { .. } => ApiError::conflict(e.to_string()),
            UpdateError::Move(e) => move_error(e),
            UpdateError::Store(e) => ApiError::internal(e),
        })?
        .ok_or_else(|| ApiError::not_found(format!("no {kind:?} row with id {id}")))?;
    Ok(Json(UpdatedBody { updated: 1, row }))
}

/// The status a refused move answers with: a clash is a conflict, a target
/// or a move that is not there is missing, a selection that cannot be read
/// is the caller's mistake.
fn move_error(e: MoveError) -> ApiError {
    match e {
        MoveError::Refused(_) => ApiError::conflict(e.to_string()),
        MoveError::UnknownProject(_) | MoveError::UnknownMove(_) => ApiError::not_found(e.to_string()),
        MoveError::NoSelection | MoveError::TextOnDeleted | MoveError::EmptyText => ApiError::bad_request(e.to_string()),
        MoveError::Store(e) => ApiError::internal(e),
    }
}

/// Moves memories between projects, or undoes a move, for the user: no
/// reader, so a clash is judged against every row of the target. Needs no
/// embedder.
async fn move_memories(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<MoveRequest>,
) -> ApiResult<Json<MoveReport>> {
    let dry_run = body.dry_run;
    let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
    let store = &app.runtime.store;
    let report = match body.plan()? {
        MovePlan::Move { filter, target } => move_memories_core(store, &backups, &filter, target, dry_run).await,
        MovePlan::Undo(id) => undo_move_core(store, &backups, id, dry_run).await,
    };
    report.map(Json).map_err(move_error)
}

async fn delete_memory(
    State(app): State<Arc<AppCtx>>,
    Path((kind, id)): Path<(String, i64)>,
    Query(q): Query<ReaderQuery>,
) -> ApiResult<Json<DeletedBody>> {
    let kind = parse_kind(&kind).ok_or_else(|| unknown_kind(&kind))?;
    let reader = named_reader(&app, q.agent.as_deref());
    let deleted = delete_memory_core(&app.runtime.store, id, kind, reader.as_ref())
        .await
        .map_err(ApiError::internal)?;

    if deleted.deleted == 0 {
        return Err(ApiError::not_found(format!("no {kind:?} row with id {id}")));
    }
    Ok(Json(deleted.into()))
}

/// Brings back a deleted `memory` row, scoped like `DELETE`. The kind is
/// checked before the embedder, so a request that could never succeed says so
/// even while the model loads.
async fn restore_memory(
    State(app): State<Arc<AppCtx>>,
    Path((kind, id)): Path<(String, i64)>,
    Query(q): Query<ReaderQuery>,
) -> ApiResult<Json<RestoredBody>> {
    let kind = parse_kind(&kind).ok_or_else(|| unknown_kind(&kind))?;
    if kind != MemoryKind::Memory {
        return Err(ApiError::bad_request(RestoreError::OnlyMemory(kind).to_string()));
    }
    let embedder = ready_embedder(&app)?;
    let reader = named_reader(&app, q.agent.as_deref());
    match restore_memory_core(&app.runtime.store, embedder.as_ref(), kind, id, reader.as_ref()).await {
        Ok(row) => Ok(Json(row.into())),
        Err(e @ RestoreError::OnlyMemory(_)) => Err(ApiError::bad_request(e.to_string())),
        Err(RestoreError::NotFound(_)) => Err(ApiError::not_found(format!("no deleted {kind:?} row with id {id}"))),
        Err(e @ (RestoreError::LiveTwin { .. } | RestoreError::TopicTaken { .. } | RestoreError::HoldsSecret { .. })) => {
            Err(ApiError::conflict(e.to_string()))
        }
        Err(RestoreError::Embed(_, e)) => Err(embed_failure(e)),
        Err(e) => Err(ApiError::internal(e)),
    }
}

fn outbox_error(e: OutboxActionError) -> ApiError {
    match e {
        OutboxActionError::NotFound(_) => ApiError::not_found(e.to_string()),
        OutboxActionError::NotFailed(_) | OutboxActionError::Ambiguous { .. } | OutboxActionError::Unreadable(_) => {
            ApiError::conflict(e.to_string())
        }
        OutboxActionError::Store(_) => ApiError::internal(e),
    }
}

/// The operation id of an outbox request, checked as a save checks it.
fn requested_operation(body: &OutboxRequest) -> ApiResult<String> {
    Ok(check_operation_id(Some(body.operation_id.clone()))?.unwrap_or_default())
}

async fn retry_outbox(State(app): State<Arc<AppCtx>>, Json(body): Json<OutboxRequest>) -> ApiResult<Json<RetriedBody>> {
    let operation_id = requested_operation(&body)?;
    let retried = app.runtime.retry_failed(&operation_id, body.agent_id.as_deref()).await.map_err(outbox_error)?;
    Ok(Json(RetriedBody { operation_id, agent_id: retried.agent_id, queued: retried.queued }))
}

async fn discard_outbox(State(app): State<Arc<AppCtx>>, Json(body): Json<OutboxRequest>) -> ApiResult<Json<DiscardedBody>> {
    let operation_id = requested_operation(&body)?;
    let discarded = app.runtime.discard_failed(&operation_id, body.agent_id.as_deref()).await.map_err(outbox_error)?;
    Ok(Json(DiscardedBody { operation_id, agent_id: discarded.agent_id, deleted: discarded.deleted }))
}

/// The answer every verdict route gives a refused one.
fn resolve_error(e: ResolveError) -> ApiError {
    match e {
            ResolveError::SameRow | ResolveError::ReasonRequired | ResolveError::AnswerRequired => {
                ApiError::bad_request(e.to_string())
            }
            ResolveError::NotFound(_) => ApiError::not_found(e.to_string()),
            ResolveError::NotInDispute { .. }
            | ResolveError::SupersededElsewhere { .. }
            | ResolveError::NeedsUser { .. } => ApiError::conflict(e.to_string()),
            ResolveError::Store(_) => ApiError::internal(e),
    }
}

async fn resolve_dispute(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<ResolveRequest>,
) -> ApiResult<Json<Resolution>> {
    let outcome = if body.keep_both { Outcome::KeepBoth } else { Outcome::Supersede };
    let reader = named_reader(&app, body.agent_id.as_deref());
    resolve_dispute_core(&app.runtime.store, body.keep, body.other, outcome, reader.as_ref())
        .await
        .map(Json)
        .map_err(resolve_error)
}

async fn judge_dispute(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<JudgeRequest>,
) -> ApiResult<Json<Resolution>> {
    let verdict: Verdict = body.verdict.parse().map_err(|e: clawg_core::memory::dispute::UnknownVerdict| {
        ApiError::bad_request(e.to_string())
    })?;
    let reader = named_reader(&app, body.agent_id.as_deref());
    judge_pair(&app.runtime.store, body.keep, body.other, verdict, body.reason, body.user_answered, reader.as_ref())
        .await
        .map(Json)
        .map_err(resolve_error)
}

/// The pairs not judged yet, counted, with the oldest pending ones.
async fn disputes(
    State(app): State<Arc<AppCtx>>,
    Query(q): Query<DisputesQuery>,
) -> ApiResult<Json<OpenPairs>> {
    let reader = named_reader(&app, q.agent_id.as_deref());
    open_pairs(&app.runtime.store, reader.as_ref(), q.project_id.as_deref(), OPEN_PAIRS_SHOWN)
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

#[derive(Debug, Serialize)]
pub struct BackupBody {
    pub path: String,
}

async fn backup(State(app): State<Arc<AppCtx>>) -> ApiResult<Json<BackupBody>> {
    let path = app.runtime.snapshot(Reason::Manual).await.map_err(ApiError::internal)?;
    Ok(Json(BackupBody { path: path.display().to_string() }))
}

/// The body of `POST /v1/import`: an export as text, which JSON escaping
/// makes longer than the file it came from.
const IMPORT_BODY_LIMIT: usize = 2 * MAX_IMPORT_BYTES;

#[derive(Debug, Deserialize)]
pub struct ImportRequest {
    /// The export, as `clawg export` wrote it.
    pub text: String,
    #[serde(default)]
    pub dry_run: bool,
    /// The agent every row that has one is stored under instead.
    #[serde(default)]
    pub agent: Option<String>,
    /// Exported project roots and the roots they have here.
    #[serde(default)]
    pub map_roots: Vec<(String, String)>,
}

/// Imports an export. The file is read before the embedder is asked for, so
/// a file this binary cannot read is a 400 even while the model loads, and a
/// dry run needs no model at all.
async fn import_memories(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<ImportRequest>,
) -> ApiResult<Json<ImportReport>> {
    if body.text.len() > MAX_IMPORT_BYTES {
        return Err(ApiError::bad_request(format!(
            "the file is {} bytes; at most {MAX_IMPORT_BYTES} are imported",
            body.text.len()
        )));
    }
    let agent = match body.agent.as_deref().map(str::trim) {
        Some("") => return Err(ApiError::bad_request("agent must not be blank")),
        agent => agent.map(str::to_string),
    };
    portability::format::parse(&body.text).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let embedder = if body.dry_run { None } else { Some(ready_embedder(&app)?) };
    let options = ImportOptions { dry_run: body.dry_run, agent, map_roots: body.map_roots };
    let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
    match portability::import(&app.runtime.store, embedder.as_deref(), &backups, &body.text, &options).await {
        Ok(report) => Ok(Json(report)),
        Err(ImportError::Format(e)) => Err(ApiError::bad_request(e.to_string())),
        Err(ImportError::NoEmbedder) => Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "embedder_not_ready")),
        Err(ImportError::Embed(e)) => Err(embed_failure(e)),
        Err(e @ ImportError::Store(_)) => Err(ApiError::internal(e)),
    }
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
    /// The caller reads and writes under the ids this answer gives. False
    /// when missing: a caller from before derives the id from the root.
    #[serde(default)]
    pub assigned: bool,
}

#[derive(Debug, Serialize)]
pub struct RegisteredBody {
    pub project_id: String,
    pub root_id: String,
    pub rekeyed: usize,
}

/// Records a project root and joins into its project the rows saved under
/// its legacy ids and, for a clone of a remote another root registered, its
/// own.
///
/// The ids are settled here from `root_path` and the remote its git
/// directory names, never taken from the caller, so a row in `project` can
/// never name a root it was not given to.
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
    let remote = if body.assigned {
        let root = root.to_path_buf();
        tokio::task::spawn_blocking(move || origin_key(&root)).await.ok().flatten()
    } else {
        None
    };
    let registration = Registration {
        root,
        remote: remote.as_deref(),
        legacy_ids: &body.legacy_ids,
        index_docs: body.index_docs,
        assigned: body.assigned,
    };
    let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
    let outcome = register_project(&app.runtime.store, &app.runtime.workspaces(), &backups, registration)
        .await
        .map_err(|e| match e {
            RegisterError::InvalidLegacyId(_) => ApiError::bad_request(e.to_string()),
            RegisterError::RootCollision { id, existing } => ApiError {
                detail: Some(format!("{} gets the project id {id}, which already belongs to {existing}", root.display())),
                ..ApiError::conflict("project_root_collision")
            },
            RegisterError::NoFreeId(_) => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("no_free_project_id") },
            RegisterError::Store(_) => ApiError::internal(e),
        })?;
    if outcome.rekeyed > 0 {
        log::info!(
            "{} joined {} rows of its earlier ids into project {}{}{}",
            root.display(),
            outcome.rekeyed,
            outcome.project_id,
            outcome.move_id.map(|n| format!("; `clawg projects move --undo {n}` moves its memories back")).unwrap_or_default(),
            outcome.snapshot.as_ref().map(|s| format!("; the store before is {}", s.display())).unwrap_or_default()
        );
    }
    // A bridge registers its root as its session starts, so the model that
    // went away while nobody worked starts loading before the first search.
    if let Some(embedder) = app.runtime.embedder.get() {
        embedder.wake();
    }
    if outcome.index_docs {
        spawn_doc_refresh(app.clone(), outcome.root_id.clone());
    }
    Ok(Json(RegisteredBody { project_id: outcome.project_id, root_id: outcome.root_id, rekeyed: outcome.rekeyed }))
}

/// Refreshes the project's documentation index in the background, once the
/// embedder has loaded: the bridge registers a root as its session starts,
/// usually before the model is ready.
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
        if let Some(guard) = app.runtime.project_docs.begin(&project_id) {
            refresh_docs(&app, guard, embedder.as_ref(), &project_id).await;
        }
    });
}

/// Runs a claimed refresh in a task of its own, so it ends even when whoever
/// started it stops waiting.
fn refresh_docs_in_background(app: Arc<AppCtx>, guard: RefreshGuard, embedder: Arc<dyn Embedder>, project_id: String) {
    tokio::spawn(async move { refresh_docs(&app, guard, embedder.as_ref(), &project_id).await });
}

/// Nobody waits on the outcome: a failure is logged and kept for the next
/// search to name.
async fn refresh_docs(app: &AppCtx, guard: RefreshGuard, embedder: &dyn Embedder, project_id: &str) {
    let tokenizer = app.runtime.tokenizer.get().cloned();
    match guard.run(&app.runtime.store, embedder, tokenizer).await {
        Ok(r) => {
            log::info!(
                "indexed the docs of {project_id}: {} docs, {} reindexed, {} removed, {} skipped, {} passages",
                r.docs, r.indexed, r.removed, r.skipped, r.passages
            );
            for f in &r.failed {
                log::warn!("left {} of {project_id} as it was: {}", f.path, f.reason);
            }
        }
        Err(e) => log::warn!("could not index the docs of {project_id}: {e}"),
    }
}

/// How long a search waits for a project's first index before answering
/// with what there is. Longer than indexing a typical repository's docs takes,
/// and short of what an MCP host waits for a tool.
pub const FIRST_INDEX_WAIT: Duration = Duration::from_secs(20);

/// How long a search waits for the refresh it started over an index that
/// already holds docs before answering from it. About what one doc took in
/// the one full reindex measured (a debug build), so an edit is usually found
/// by the search that finds it, and a checkout costs that search no more.
pub const REFRESH_GRACE: Duration = Duration::from_secs(3);

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

/// Strict for the same reason as [`ReaderQuery`].
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
    /// Why the project's last finished refresh failed, until one succeeds;
    /// the hits come from what was already indexed.
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
        "clawg reads the files of the projects the user added, and of the one a bridge was \
         started in when that folder has a .git or a manifest and is not the home directory or a \
         filesystem root; {root} is neither. Ask the user to run `clawg projects add \"{root}\"{flag}` \
         if they want it searched, then call again",
        root = root.display()
    )
}

/// Searches a registered project's documentation. When the last refresh is
/// older than [`project_docs::REFRESH_INTERVAL`] it starts one in the
/// background and waits for it at most [`REFRESH_GRACE`].
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
    // An empty index waits longer: answering from it would read as "the docs
    // say nothing". The first index is usually started by the bridge's
    // registration a moment before this search.
    if let Some(guard) = index.begin_if_due(&body.project_id, Instant::now()) {
        refresh_docs_in_background(app.clone(), guard, embedder.clone(), body.project_id.clone());
        let limit = if docs_in(store, &body.project_id).await? == 0 { FIRST_INDEX_WAIT } else { REFRESH_GRACE };
        index.wait_idle(&body.project_id, limit).await;
    } else if index.is_running(&body.project_id) && docs_in(store, &body.project_id).await? == 0 {
        index.wait_idle(&body.project_id, FIRST_INDEX_WAIT).await;
    }
    let hits = project_docs::search(store, embedder.as_ref(), &body.project_id, &body.query, clamp_docs_limit(body.limit))
        .await
        .map_err(ApiError::internal)?;
    let docs = docs_in(store, &body.project_id).await?;
    let warning = index.last_error(&body.project_id);
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
    fn content_is_checked_in_bytes() {
        assert!(check_content(&"x".repeat(MAX_CONTENT_BYTES)).is_ok());
        assert!(check_content(&"ñ".repeat(MAX_CONTENT_BYTES / 2)).is_ok());
        let over = check_content(&"ñ".repeat(MAX_CONTENT_BYTES / 2 + 1)).unwrap_err();
        assert_eq!(over.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            over.message,
            format!(
                "content is {} bytes; at most {MAX_CONTENT_BYTES} are accepted: split it into several saves, one fact each",
                MAX_CONTENT_BYTES + 2
            )
        );
        for empty in ["", "  \n "] {
            assert_eq!(check_content(empty).unwrap_err().message, "content must not be empty");
        }
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
    fn replaced_sources_name_what_replaced_them() {
        assert_eq!(replaced_list(&[(5, Some(9)), (7, None)]), "5 (by 9), 7");
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

        let failed = save_response(Confirmation::Reported(IngestReport {
            memories: vec![],
            failure: Some(IngestFailure::Failed("embed failed: onnx".into())),
            ..Default::default()
        }), "op-1".into(), &[])
        .expect_err("a failure that stored nothing is an error");
        assert_eq!(failed.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(failed.message, "ingest_failed");
        assert_eq!(failed.detail.as_deref(), Some("embed failed: onnx"));
        assert_eq!(failed.memories, Some(vec![]));

        let (status, partial) = save_response(Confirmation::Reported(IngestReport {
            memories: vec![chunk(57, ChunkOutcome::Inserted)],
            failure: Some(IngestFailure::Failed("store insert failed: disk full".into())),
            ..Default::default()
        }), "op-1".into(), &[])
        .expect("a save that stored a piece is accepted");
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(partial.reason, Some("partially_saved"));
        assert_eq!(partial.detail.as_deref(), Some("store insert failed: disk full"));
        assert_eq!(partial.memories, Some(vec![SavedMemory { kind: "memory", id: 57, outcome: "inserted" }]));
        assert_eq!(partial.operation_id, "op-1");

        let (status, retrying) = save_response(Confirmation::Reported(IngestReport {
            memories: vec![],
            failure: Some(IngestFailure::EmbedderUnavailable),
            ..Default::default()
        }), "op-1".into(), &[])
        .expect("the worker tries it again");
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(retrying.reason, Some("retrying"));
        assert_eq!(retrying.detail.as_deref(), Some("the embedder is not available"));
        assert_eq!(retrying.memories, None);
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
    fn save_body_leaves_related_out_when_empty() {
        let (_, body) = save_response(
            Confirmation::Reported(IngestReport { memories: vec![chunk(57, ChunkOutcome::Inserted)], ..Default::default() }),
            "op-1".into(),
            &[],
        )
        .expect("a clean report is a success");
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("related").is_none(), "{json}");

        let candidate = clawg_core::memory::Candidate {
            id: 12,
            distance: 0.3,
            escalated: true,
            excerpt: "Ports".into(),
            same_scope: true,
        };
        let (_, body) = save_response(
            Confirmation::Reported(IngestReport {
                memories: vec![chunk(57, ChunkOutcome::Inserted)],
                related: vec![candidate],
                ..Default::default()
            }),
            "op-1".into(),
            &[],
        )
        .expect("a clean report is a success");
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["related"], serde_json::json!([{ "kind": "memory", "id": 12, "escalated": true, "excerpt": "Ports" }]));
    }

    #[test]
    fn a_confirmed_save_names_its_revision_candidate_only_when_it_has_one() {
        let report = |revision_candidate| IngestReport {
            memories: vec![chunk(57, ChunkOutcome::Inserted)],
            revision_candidate,
            ..Default::default()
        };
        let (_, body) = save_response(Confirmation::Reported(report(Some(12))), "op-1".into(), &[])
            .expect("a clean report is a success");
        assert_eq!(serde_json::to_value(&body).unwrap()["revision_candidate"], 12);

        let (_, body) = save_response(Confirmation::Reported(report(None)), "op-1".into(), &[])
            .expect("a clean report is a success");
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("revision_candidate").is_none(), "{json}");
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
            ..Default::default()
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
            sessions: Default::default(),
            metrics: Default::default(),
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
    fn a_move_asks_for_one_target_or_an_undo() {
        let ids = |ids: Vec<i64>| MoveRequest { ids, ..MoveRequest::default() };
        assert_eq!(
            MoveRequest { to_project: Some("repo-1a2b".into()), ..ids(vec![3]) }.plan().expect("valid"),
            MovePlan::Move {
                filter: MoveFilter { ids: vec![3], ..MoveFilter::default() },
                target: Some("repo-1a2b".into())
            }
        );
        assert_eq!(
            MoveRequest { from_none: true, to_none: true, ..MoveRequest::default() }.plan().expect("valid"),
            MovePlan::Move { filter: MoveFilter { from: Some(ProjectFilter::None), ..MoveFilter::default() }, target: None }
        );
        assert_eq!(MoveRequest { undo: Some(4), ..MoveRequest::default() }.plan().expect("valid"), MovePlan::Undo(4));
        for bad in [
            ids(vec![3]),
            MoveRequest { to_project: Some("repo-1a2b".into()), to_none: true, ..ids(vec![3]) },
            MoveRequest { undo: Some(4), to_none: true, ..MoveRequest::default() },
            MoveRequest { undo: Some(4), ..ids(vec![3]) },
            MoveRequest { from_project: Some("a-0000".into()), from_none: true, to_none: true, ..MoveRequest::default() },
        ] {
            assert_eq!(bad.plan().expect_err("refused").status, StatusCode::BAD_REQUEST);
        }
    }

    #[test]
    fn no_project_and_project_together_are_refused() {
        assert!(list_params_from_query(query(|q| q.no_project = Some(true))).expect("valid").no_project);
        let err = list_params_from_query(query(|q| {
            q.no_project = Some(true);
            q.project = Some("repo-1a2b".into());
        }))
        .expect_err("a row has one project or none");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn an_update_names_one_destination_at_most() {
        let request = |project_id: Option<&str>, no_project| UpdateRequest {
            importance: None,
            memory_type: None,
            project_id: project_id.map(str::to_string),
            no_project,
            user_answered: false,
        };
        assert_eq!(request(None, false).destination().expect("valid"), None);
        assert_eq!(request(Some("repo-1a2b"), false).destination().expect("valid"), Some(Some("repo-1a2b".into())));
        assert_eq!(request(None, true).destination().expect("valid"), Some(None));
        assert_eq!(request(Some("repo-1a2b"), true).destination().expect_err("both").status, StatusCode::BAD_REQUEST);
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
