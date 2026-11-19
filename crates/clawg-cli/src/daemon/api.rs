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
mod outbox;
mod portability;
mod projects;
mod sessions;
#[cfg(test)]
mod test_support;

use axum::extract::{Path, Query, State};
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use clawg_core::memory::admin::{
    delete_memory_core, Deleted, get_memory_core, list_memories_core, ListParams, memory_stats_core,
    MemoryAdminRow, MemoryPatch, query_has_tokens, restore_memory_core, RestoredRow, RestoreError,
    StatsParams, update_memory_core, UpdatedRow, UpdateError,
};
use clawg_core::memory::bootstrap::EmbedderStatus;
use clawg_core::memory::clock::TimestampMs;
use clawg_core::memory::embedder::{Embedder, EmbedError, EmbedRole};
use clawg_core::memory::language::{count_saves_in, count_saves_in_projects};
use clawg_core::memory::merge::canonical;
use clawg_core::memory::outbox::{OutboxState, OutboxSummary};
use clawg_core::memory::prompt_builder::{
    build_memory_prompt, build_session_prompt, DEFAULT_BUDGET, MAX_BUDGET, PromptBlock, QueryTexts,
    Retrieval,
};
use clawg_core::memory::redact::SecretKind;
use clawg_core::memory::registry::{find_project, ListedSuggestion, Named, pending_suggestions};
use clawg_core::memory::relocate::{
    move_memories_core, MoveError, MoveFilter, MoveReport, ProjectFilter, undo_move_core,
};
use clawg_core::memory::save::{NewSave, SaveInput, SaveRejected};
use clawg_core::memory::sharing::{Reach, Reader};
use clawg_core::memory::store_insert::MemoryType;
use clawg_core::memory::synthesis::{
    ConclusionReview, conclusions_to_review_count, LostSource, Synthesis, synthesis_core,
    SynthesisParams, unreviewed_count,
};
use clawg_core::memory::worker::{ChunkOutcome, IngestFailure, IngestReport, SavedChunk};
use clawg_core::memory::{
    project_docs, RankedHit, read_save_core, SaveText, search_all_memories, SearchScope, sharing,
};
use clawg_core::metrics::sessions::{Activity, UNSPECIFIED};
use clawg_core::metrics::{Recorder, Unit};
use clawg_core::paths::Profile;
use clawg_core::preferences::Lang;
use clawg_core::runtime::{Accepted, AcceptError, MemoryRuntime};
use disputes::{disputes, judge_dispute, resolve_dispute};
use docs::search_docs;
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
use sessions::{count_empty_search, count_tool_call, end_session, register_session};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

use crate::relevance::shown_hits;

use super::guard::{admits, RefusalLog};
use super::imports::{ImportJobs, JobView};
use super::lifecycle::now_ms;
use super::metrics::{HttpMeter, metrics_body, prometheus_text, time_request, Transport};
use super::sessions::SessionRegistry;
pub use self::docs::{DocsSearchBody, REFRESH_GRACE};

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

/// The kinds whose tables V32 dropped. A bridge of a version that still had
/// them, or a script, can name them.
const RETIRED_KINDS: [&str; 2] = ["episodic", "procedural"];

/// Checks the wire spelling of a kind: `memory` is the only one.
///
/// A free function rather than `serde` on the path segment so the 400 can say
/// what is wrong; a deserializer failure would surface as axum's generic
/// "invalid path" text. A retired kind is told where its rows went, not that
/// the row is missing.
pub fn parse_kind(raw: &str) -> ApiResult<()> {
    match raw {
        "memory" => Ok(()),
        retired if RETIRED_KINDS.contains(&retired) => Err(ApiError::bad_request(retired_kind(retired))),
        other => Err(ApiError::bad_request(format!("unknown kind {other:?}: the only kind is \"memory\""))),
    }
}

/// Where the rows of a retired kind went.
fn retired_kind(kind: &str) -> String {
    format!(
        "kind {kind:?} no longer exists: episodic and procedural rows are memory rows now, of type semantic_fact, \
         each with a new id; find them with search_memory (POST /v1/search)"
    )
}

/// The lifecycle states a memory row can be filtered by.
const STATUSES: [&str; 3] = ["active", "disputed", "superseded"];

/// Validates a listing request and turns it into the core's parameters.
///
/// Pure, so each refusal is testable without a server. Every refusal is a 400
/// naming what was wrong: a viewer that quietly answered an unfiltered list to
/// a bad filter would present it as the result of that filter.
pub fn list_params_from_query(q: ListQuery) -> ApiResult<ListParams> {
    if let Some(raw) = q.kind.as_deref() {
        parse_kind(raw)?;
    }

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
        if !query_has_tokens(text) {
            return Err(ApiError::bad_request(
                "q must contain at least one letter or digit to search for",
            ));
        }
    }

    let no_project = q.no_project.unwrap_or(false);
    if no_project && q.project.is_some() {
        return Err(ApiError::bad_request("project and no_project cannot both be given: a row has one or none"));
    }

    Ok(ListParams {
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

impl From<SaveRequest> for SaveInput {
    fn from(body: SaveRequest) -> Self {
        Self {
            content: body.content,
            agent_id: body.agent_id,
            project_id: body.project_id,
            memory_type: body.memory_type,
            importance: body.importance,
            derived_from: body.derived_from,
            user_stated: body.user_stated,
            operation_id: body.operation_id,
            topic_key: body.topic_key,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub agent_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    /// A group of projects to search at once, each read as `project_id` reads
    /// one. It wins over `project_id`, where a bridge names the group's first
    /// project for a daemon that predates groups.
    #[serde(default)]
    pub project_ids: Option<Vec<String>>,
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

/// The tests of the daemon and of the bridge name it from here.
pub use clawg_core::memory::save::MAX_CONTENT_BYTES;

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

/// A row of `memory`, or a hit, as an answer carries it: with `kind`, which a
/// bridge of a version before V32, that still read trajectories and skills,
/// requires on every row (`memory`) and hit (`Memory`).
#[derive(Debug, Serialize)]
pub struct Kinded<T> {
    pub kind: &'static str,
    #[serde(flatten)]
    pub row: T,
}

impl<T> Kinded<T> {
    fn row(row: T) -> Self {
        Self { kind: "memory", row }
    }

    fn hit(row: T) -> Self {
        Self { kind: "Memory", row }
    }
}

#[derive(Debug, Serialize)]
pub struct SearchBody {
    pub hits: Vec<Kinded<RankedHit>>,
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
    /// The suggestions nobody answered that the project may be another one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<ListedSuggestion>,
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

/// The other language's count for a search in `lang`, with the projects and
/// the reach the search used. A count that fails is logged and left out: the
/// search already answered.
async fn other_language(app: &AppCtx, lang: Lang, projects: Option<&[String]>, reader: &Reader) -> Option<OtherLanguage> {
    let other = lang.other();
    let counted = match projects {
        Some(projects) => count_saves_in_projects(&app.runtime.store, other, projects, reader).await,
        None => count_saves_in(&app.runtime.store, other, None, reader).await,
    };
    match counted {
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
    /// The state as the answer names it.
    fn name(&self) -> &'static str {
        match self {
            Self::Searched { .. } => "searched",
            Self::Recent { .. } => "recent",
            Self::EmbedderNotReady => "embedder_not_ready",
            Self::EmbedderFailed { .. } => "embedder_failed",
            Self::SearchFailed { .. } => "search_failed",
        }
    }

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
    pub items: Vec<Kinded<MemoryAdminRow>>,
}

#[derive(Debug, Serialize)]
pub struct ItemBody {
    pub item: Kinded<MemoryAdminRow>,
}

/// [`Synthesis`] with its rows [`Kinded`].
#[derive(Debug, Serialize)]
pub struct SynthesisBody {
    pub pending: Vec<Kinded<MemoryAdminRow>>,
    pub pending_total: usize,
    pub conclusions: Vec<Kinded<MemoryAdminRow>>,
    pub to_review: Vec<ReviewBody>,
    pub to_review_total: usize,
}

#[derive(Debug, Serialize)]
pub struct ReviewBody {
    pub conclusion: Kinded<MemoryAdminRow>,
    pub live: usize,
    pub lost: Vec<LostSource>,
}

impl From<Synthesis> for SynthesisBody {
    fn from(s: Synthesis) -> Self {
        let rows = |rows: Vec<MemoryAdminRow>| rows.into_iter().map(Kinded::row).collect();
        let review = |r: ConclusionReview| ReviewBody { conclusion: Kinded::row(r.conclusion), live: r.live, lost: r.lost };
        Self {
            pending: rows(s.pending),
            pending_total: s.pending_total,
            conclusions: rows(s.conclusions),
            to_review: s.to_review.into_iter().map(review).collect(),
            to_review_total: s.to_review_total,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct UpdatedBody {
    pub updated: usize,
    #[serde(flatten)]
    pub row: Kinded<UpdatedRow>,
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
#[expect(clippy::large_enum_variant, reason = "one per request, consumed by the handler that parsed it; none is stored")]
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
    let save = NewSave::parse(body.into()).map_err(save_refused)?;
    let admitted = app.runtime.admit(save).await.map_err(save_refused)?;
    // Read now, not by the worker: a replay after a restart must keep the
    // session the save was accepted in.
    let session_id = app.sessions.current(admitted.agent_id(), admitted.project_id());
    let saved_in = session_id.clone();
    let saved_as = admitted.memory_type().map_or(UNSPECIFIED, MemoryType::as_str);
    let topic_key = admitted.topic_key().map(str::to_owned);
    let wait = admitted.waits();
    let (event, operation_id) = admitted.into_event(session_id, now_ms());

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
        Accepted::Queued { row, operation_id, redacted } => {
            if let Some(session) = saved_in.as_deref() {
                app.runtime.metrics.sessions().activity(session, now_ms() as i64, Activity::Save(saved_as));
            }
            (row, operation_id, redacted)
        }
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

/// The answer to a save the core refused: a mistake of the caller is a
/// `400` with the refusal's text, an embedder that will not load a `503`
/// that says whether to retry, and a failing store a `500`.
fn save_refused(e: SaveRejected) -> ApiError {
    match e {
        SaveRejected::EmbedderFailed { error, retryable } => ApiError::embedder_failed(error, retryable),
        SaveRejected::Store(e) => ApiError::internal(e),
        refused @ (SaveRejected::EmptyContent
        | SaveRejected::ContentTooLong { .. }
        | SaveRejected::UnknownType(_)
        | SaveRejected::OperationId(_)
        | SaveRejected::TopicKey(_)
        | SaveRejected::TooManySources { .. }
        | SaveRejected::StatedConclusion
        | SaveRejected::MissingSources(_)
        | SaveRejected::ReplacedSources(_)) => ApiError::bad_request(refused.to_string()),
    }
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
    let projects = searched_projects(&app, body.project_id.clone(), body.project_ids.clone()).await?;
    let scope = SearchScope::all_default().with_reach(caller.reach.clone());
    let scope = match &projects {
        Some(projects) => scope.with_projects(projects.clone()),
        None => scope,
    };
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
    app.runtime.metrics.gauge("search.hits", &[], hits.len() as f64);
    app.runtime.metrics.gauge("search.withheld", &[], withheld as f64);
    if hits.is_empty() {
        count_empty_search(&app, &body.agent_id, projects.as_deref());
    }
    let other_language = match lang {
        Some(lang) => other_language(&app, lang, projects.as_deref(), &caller).await,
        None => None,
    };
    Ok(Json(SearchBody { hits: hits.into_iter().map(Kinded::hit).collect(), withheld, other_language }))
}

/// The live projects a search reads: each of `project_ids` once, whatever
/// `project_id` says, or else the one of `project_id`, or `None` for every
/// project.
async fn searched_projects(
    app: &AppCtx,
    project_id: Option<String>,
    project_ids: Option<Vec<String>>,
) -> ApiResult<Option<Vec<String>>> {
    match project_ids {
        Some(ids) if ids.is_empty() => Err(ApiError::bad_request("project_ids names no project")),
        Some(ids) => {
            let mut live: Vec<String> = Vec::with_capacity(ids.len());
            for id in ids {
                let id = canonical(&app.runtime.store, &id).await.map_err(ApiError::internal)?;
                if !live.contains(&id) {
                    live.push(id);
                }
            }
            Ok(Some(live))
        }
        None => Ok(live_project(app, project_id).await?.map(|id| vec![id])),
    }
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
    let project_id = live_project(&app, body.project_id.clone()).await?;
    let project_id = project_id.as_deref();
    let query = body.query.as_deref().filter(|q| !q.trim().is_empty());
    let reach = reach(&app, &body.agent_id);

    // Unconditional regen (~2-5 ms). A failure degrades to "no ids were
    // emitted", which can make the prompt repeat a row — a degraded prompt
    // beats a failed request.
    let emitted = workspace
        .regenerate_auto_section(&body.agent_id, project_id, &reach, &app.runtime.store)
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
                &reach,
                QueryTexts { lexical: query, embedded: query },
                &emitted,
                Some(app.runtime.access_tracker.as_ref()),
                budget,
            )
            .await
        }
        None => {
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
        (Some(lang), Some(_)) => {
            let projects = project_id.map(|p| vec![p.to_owned()]);
            other_language(&app, lang, projects.as_deref(), &caller).await
        }
        _ => None,
    };

    // As the counts above: a line left out, not a failed request.
    let suggestions = match project_id {
        Some(project_id) => pending_suggestions(&app.runtime.store, project_id).await.unwrap_or_else(|e| {
            log::warn!("pending suggestions failed: {e}");
            Vec::new()
        }),
        None => Vec::new(),
    };

    let retrieval = RetrievalState::of(prompt.retrieval, app.runtime.embedder_status());
    record_context(&app.runtime.metrics, &retrieval, prompt.text.len(), &prompt.cut);
    Ok(Json(ContextBody { prompt: prompt.text, unreviewed, conclusions_to_review, retrieval, other_language, suggestions }))
}

/// Records how a context went: whether it searched, how big its prompt is
/// and which blocks the budget cut. Counts are gauges of 1, one sample each.
fn record_context(metrics: &Recorder, retrieval: &RetrievalState, prompt_bytes: usize, cut: &[PromptBlock]) {
    metrics.gauge("context.retrieval_state", &[("state", retrieval.name())], 1.0);
    metrics.observe("context.prompt_bytes", Unit::Bytes, &[], prompt_bytes as u64);
    for block in cut {
        metrics.gauge("context.cut", &[("block", block.as_str())], 1.0);
    }
}

async fn synthesis(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<SynthesisRequest>,
) -> ApiResult<Json<SynthesisBody>> {
    let reader = named_reader(&app, body.agent_id.as_deref());
    let project_id = live_project(&app, body.project_id).await?;
    let params = SynthesisParams { reader, project_id, limit: body.limit };
    synthesis_core(&app.runtime.store, params, TimestampMs::now())
        .await
        .map(|s| Json(s.into()))
        .map_err(ApiError::internal)
}

async fn list_memories(
    State(app): State<Arc<AppCtx>>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<ListBody>> {
    let reader = named_reader(&app, q.reader.as_deref());
    let mut params = ListParams { reader, ..list_params_from_query(q)? };
    params.project_id = live_project(&app, params.project_id).await?;
    let items = list_memories_core(&app.runtime.store, params)
        .await
        .map_err(ApiError::internal)?;

    Ok(Json(ListBody { items: items.into_iter().map(Kinded::row).collect() }))
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

/// One row with every column the viewer shows, soft-deleted rows included
/// (they report `deleted_at`). Deliberately not a search: it records no access.
async fn get_memory(
    State(app): State<Arc<AppCtx>>,
    Path((kind, id)): Path<(String, i64)>,
) -> ApiResult<Json<ItemBody>> {
    parse_kind(&kind)?;
    let item = get_memory_core(&app.runtime.store, id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("no memory row with id {id}")))?;
    Ok(Json(ItemBody { item: Kinded::row(item) }))
}

/// The save the row is a piece of, as far as the reader reaches it: a row
/// that is missing, deleted or out of reach is the same 404.
async fn get_save(
    State(app): State<Arc<AppCtx>>,
    Path((kind, id)): Path<(String, i64)>,
    Query(q): Query<ReaderQuery>,
) -> ApiResult<Json<Kinded<SaveText>>> {
    parse_kind(&kind)?;
    let reader = named_reader(&app, q.agent.as_deref());
    let save = read_save_core(&app.runtime.store, id, reader.as_ref())
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("no memory row with id {id}")))?;
    Ok(Json(Kinded::row(save)))
}

async fn update_memory(
    State(app): State<Arc<AppCtx>>,
    Path((kind, id)): Path<(String, i64)>,
    Query(q): Query<ReaderQuery>,
    Json(body): Json<UpdateRequest>,
) -> ApiResult<Json<UpdatedBody>> {
    parse_kind(&kind)?;
    let destination = match body.destination()? {
        Some(Some(target)) => Some(Some(target_named(&app, target).await?)),
        other => other,
    };
    let patch = MemoryPatch::new(body.importance, body.memory_type.as_deref(), destination)
        .map_err(|e| ApiError::bad_request(e.to_string()))?
        .answered(body.user_answered);
    // The core answers `None` for "no such row", "already soft-deleted" and
    // "out of reach" alike, and from the caller's side those are the same
    // fact: the row it named is not there to change.
    let reader = named_reader(&app, q.agent.as_deref());
    let row = update_memory_core(&app.runtime.store, id, &patch, reader.as_ref())
        .await
        .map_err(|e| match e {
            UpdateError::NeedsUser { .. } | UpdateError::MoveNeedsUser { .. } => ApiError::conflict(e.to_string()),
            UpdateError::Move(e) => move_error(e),
            UpdateError::Store(e) => ApiError::internal(e),
        })?
        .ok_or_else(|| ApiError::not_found(format!("no memory row with id {id}")))?;
    Ok(Json(UpdatedBody { updated: 1, row: Kinded::row(row) }))
}

/// The project a target names: the id as given, else the one live project
/// of that name. A name nobody holds goes on as given, and the move answers
/// its 404 naming it; one two projects hold is refused, naming their ids.
async fn target_named(app: &AppCtx, reference: String) -> ApiResult<String> {
    match find_project(&app.runtime.store, &reference).await.map_err(ApiError::internal)? {
        Named::Project(id) => Ok(id),
        Named::Unknown => Ok(reference),
        Named::Ambiguous(ids) => Err(ApiError {
            detail: Some(format!("{reference:?} is the name of {}: name one of them by its id", ids.join(", "))),
            ..ApiError::conflict("project_ambiguous")
        }),
    }
}

/// The status a refused move answers with: a clash is a conflict, a target
/// or a move that is not there is missing, a selection that cannot be read
/// is the caller's mistake.
fn move_error(e: MoveError) -> ApiError {
    match e {
        MoveError::Refused(_) => ApiError::conflict(e.to_string()),
        MoveError::Merged { .. } => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("project_merged") },
        MoveError::Deleted { .. } => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("project_deleted") },
        MoveError::PartOfMerge { .. } => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("part_of_merge") },
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
    let plan = match body.plan()? {
        MovePlan::Move { filter, target: Some(target) } => MovePlan::Move { filter, target: Some(target_named(&app, target).await?) },
        plan => plan,
    };
    let report = match plan {
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
    parse_kind(&kind)?;
    let reader = named_reader(&app, q.agent.as_deref());
    let deleted = delete_memory_core(&app.runtime.store, id, reader.as_ref())
        .await
        .map_err(ApiError::internal)?;

    if deleted.deleted == 0 {
        return Err(ApiError::not_found(format!("no memory row with id {id}")));
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
    parse_kind(&kind)?;
    let embedder = ready_embedder(&app)?;
    let reader = named_reader(&app, q.agent.as_deref());
    match restore_memory_core(&app.runtime.store, embedder.as_ref(), id, reader.as_ref()).await {
        Ok(row) => Ok(Json(row.into())),
        Err(e @ RestoreError::NotFound(_)) => Err(ApiError::not_found(e.to_string())),
        Err(e @ (RestoreError::LiveTwin { .. } | RestoreError::TopicTaken { .. } | RestoreError::HoldsSecret { .. })) => {
            Err(ApiError::conflict(e.to_string()))
        }
        Err(RestoreError::Embed(_, e)) => Err(embed_failure(e)),
        Err(e @ RestoreError::Store(_)) => Err(ApiError::internal(e)),
    }
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
    use clawg_core::memory::registry::{Born, resolve_project, ResolveOptions};

    use super::test_support::{
        answer_of, daemon_for_jobs, daemon_without_embedder, reader_query, run_sql, seed_memory,
    };

    fn chunk(id: i64, outcome: ChunkOutcome) -> SavedChunk {
        SavedChunk { id, outcome }
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
    fn parse_kind_takes_memory_alone() {
        assert!(parse_kind("memory").is_ok());
        for other in ["Memory", "", "semantic"] {
            let err = parse_kind(other).expect_err(other);
            assert_eq!(err.status, StatusCode::BAD_REQUEST, "{other:?}");
            assert!(err.message.contains("the only kind is \"memory\""), "{other:?}: {}", err.message);
        }
    }

    /// Answering a retired kind as a missing row would read as the row being
    /// gone, while it is in `memory` under another id.
    #[test]
    fn a_retired_kind_says_where_its_rows_went() {
        for kind in ["episodic", "procedural"] {
            let err = parse_kind(kind).expect_err(kind);
            assert_eq!(err.status, StatusCode::BAD_REQUEST);
            for said in [format!("{kind:?}"), "semantic_fact".to_string(), "new id".to_string(), "search_memory".to_string()] {
                assert!(err.message.contains(&said), "{said} in {}", err.message);
            }
        }
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
    fn a_listing_of_a_retired_kind_is_refused() {
        for kind in ["episodic", "procedural"] {
            let err = list_params_from_query(query(|q| q.kind = Some(kind.into()))).expect_err(kind);
            assert_eq!(err.status, StatusCode::BAD_REQUEST);
            assert!(err.message.contains("semantic_fact"), "{}", err.message);
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
    fn a_text_query_without_any_word_is_rejected() {
        for text in ["", "   ", "-^()\"* :"] {
            let err = list_params_from_query(query(|q| q.q = Some(text.into())))
                .expect_err("nothing to search for");
            assert_eq!(err.status, StatusCode::BAD_REQUEST, "{text:?}");
        }
    }

    #[test]
    fn an_unknown_kind_names_the_value_and_the_only_kind() {
        let msg = parse_kind("bogus").unwrap_err().message;
        assert!(msg.contains("bogus"), "{msg}");
        assert!(msg.contains("\"memory\""), "{msg}");
    }

    /// The status and the whole body `save_memory` answers a refused save
    /// with, for every rule it holds a save to and in the order it checks
    /// them: what a client branches on and what it shows.
    #[tokio::test]
    async fn each_refused_save_keeps_its_status_and_body() {
        use serde_json::{json, Value};

        async fn refused(app: &Arc<AppCtx>, body: Value) -> (StatusCode, Value) {
            let body: SaveRequest = serde_json::from_value(body).expect("a save body");
            let response = save_memory(State(app.clone()), Json(body)).await.expect_err("the save is refused").into_response();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.expect("the body");
            (status, serde_json::from_slice(&bytes).expect("a JSON body"))
        }
        async fn seed(app: &AppCtx, agent: &'static str) -> i64 {
            app.runtime
                .store
                .conn_for_test()
                .call(move |c| {
                    c.execute(
                        "INSERT INTO memory (scope, agent_id, content, memory_type, source, created_at, created_by_type, \
                         created_by_ref, last_modified_at, last_accessed_at, importance, tier, status) \
                         VALUES ('agent', ?1, 'a source', 'semantic_fact', 'explicit', 1, 'agent', ?1, 1, 1, 5.0, 3, 'active')",
                        [agent],
                    )?;
                    Ok::<_, rusqlite::Error>(c.last_insert_rowid())
                })
                .await
                .expect("seed a row")
        }
        async fn run(app: &AppCtx, sql: String) {
            app.runtime.store.conn_for_test().call(move |c| c.execute_batch(&sql)).await.expect("update the rows");
        }
        let bad = |message: &str| (StatusCode::BAD_REQUEST, json!({ "error": message }));

        let (app, dir) = daemon_for_jobs().await;
        let kept = seed(&app, "a").await;
        let deleted = seed(&app, "a").await;
        let theirs = seed(&app, "b").await;
        let old = seed(&app, "a").await;
        let new = seed(&app, "a").await;
        run(&app, format!("UPDATE memory SET deleted_at = 2 WHERE id = {deleted}; UPDATE memory SET status = 'superseded', superseded_by = {new} WHERE id = {old};")).await;
        std::fs::write(dir.path().join("sharing.json"), r#"{"version":1,"hides":{"a":["b"]}}"#).expect("write the policy");
        let types = "semantic_fact, user_preference, architectural_decision, session_summary, code_block, chat_message, tool_error";
        let stated = "user_stated cannot go with derived_from: a conclusion is drawn by the agent, not stated by the user";
        let many: Vec<i64> = (1..=51).collect();

        let cases = [
            (json!({ "content": "  \n", "agent_id": "a" }), bad("content must not be empty")),
            (
                json!({ "content": "x".repeat(MAX_CONTENT_BYTES + 1), "agent_id": "a" }),
                bad("content is 65537 bytes; at most 65536 are accepted: split it into several saves, one fact each"),
            ),
            (
                json!({ "content": "a fact", "agent_id": "a", "memory_type": "preference" }),
                bad(&format!("unknown memory_type \"preference\"; expected one of {types}")),
            ),
            (
                json!({ "content": "a fact", "agent_id": "a", "operation_id": "has space" }),
                bad("operation_id must be 1 to 128 visible ASCII characters"),
            ),
            (
                json!({ "content": "a fact", "agent_id": "a", "topic_key": "Storage Engine" }),
                bad("topic_key is segments of a-z, 0-9, '.', '_' or '-' joined by '/', with no empty segment"),
            ),
            (json!({ "content": "a fact", "agent_id": "a", "topic_key": "" }), bad("topic_key must be 1 to 80 characters")),
            (
                json!({ "content": "a fact", "agent_id": "a", "derived_from": many }),
                bad("derived_from names 51 memories; at most 50 are accepted"),
            ),
            (json!({ "content": "a fact", "agent_id": "a", "derived_from": [kept], "user_stated": true }), bad(stated)),
            (
                json!({ "content": "a fact", "agent_id": "a", "derived_from": [999_999, theirs, kept, deleted, kept] }),
                bad(&format!("derived_from names memories that do not exist or were deleted: {deleted}, {theirs}, 999999")),
            ),
            (
                json!({ "content": "a fact", "agent_id": "a", "derived_from": [old, kept] }),
                bad(&format!("derived_from names memories that were replaced: {old} (by {new}); draw the conclusion from the current ones")),
            ),
            (json!({ "content": "", "agent_id": "a", "memory_type": "preference" }), bad("content must not be empty")),
            (
                json!({ "content": "a fact", "agent_id": "a", "memory_type": "preference", "operation_id": "" }),
                bad(&format!("unknown memory_type \"preference\"; expected one of {types}")),
            ),
            (
                json!({ "content": "a fact", "agent_id": "a", "operation_id": "", "topic_key": "" }),
                bad("operation_id must be 1 to 128 visible ASCII characters"),
            ),
            (
                json!({ "content": "a fact", "agent_id": "a", "topic_key": "", "derived_from": many }),
                bad("topic_key must be 1 to 80 characters"),
            ),
            (
                json!({ "content": "a fact", "agent_id": "a", "derived_from": many, "user_stated": true }),
                bad("derived_from names 51 memories; at most 50 are accepted"),
            ),
            (
                json!({ "content": "a fact", "agent_id": "a", "derived_from": [old, deleted] }),
                bad(&format!("derived_from names memories that do not exist or were deleted: {deleted}")),
            ),
        ];
        for (body, expected) in cases {
            assert_eq!(refused(&app, body.clone()).await, expected, "{body}");
        }

        app.runtime.embedder_status.send_replace(EmbedderStatus::Failed { error: "disk full".into(), retryable: false });
        assert_eq!(
            refused(&app, json!({ "content": "a fact", "agent_id": "a" })).await,
            (StatusCode::SERVICE_UNAVAILABLE, json!({ "error": "embedder_failed", "detail": "disk full", "retryable": false }))
        );
        assert_eq!(
            refused(&app, json!({ "content": "a fact", "agent_id": "a", "derived_from": [deleted] })).await,
            bad(&format!("derived_from names memories that do not exist or were deleted: {deleted}")),
            "the sources are checked before the embedder"
        );
        app.runtime.embedder_status.send_replace(EmbedderStatus::Failed { error: "the model is still downloading".into(), retryable: true });
        assert_eq!(
            refused(&app, json!({ "content": "a fact", "agent_id": "a" })).await,
            (
                StatusCode::SERVICE_UNAVAILABLE,
                json!({ "error": "embedder_failed", "detail": "the model is still downloading", "retryable": true })
            )
        );

        app.runtime.embedder_status.send_replace(EmbedderStatus::Ready);
        app.runtime.close().await.expect("the store closes");
        let cases = [
            (json!({ "content": "a fact", "agent_id": "a", "derived_from": [kept] }), "memory backend channel error: source lookup: ConnectionClosed"),
            (json!({ "content": "a fact", "agent_id": "a", "project_id": "app" }), "Memory DB connection error: ConnectionClosed"),
            (json!({ "content": "a fact", "agent_id": "a" }), "memory backend channel error: outbox_record: ConnectionClosed"),
        ];
        for (body, message) in cases {
            assert_eq!(
                refused(&app, body.clone()).await,
                (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": message })),
                "{body}"
            );
        }
    }

    /// The status and the whole body `update_memory` answers with, for every
    /// refusal in the order it checks them and for each kind of change.
    #[tokio::test]
    async fn each_memory_update_keeps_its_status_and_body() {
        use serde_json::{json, Value};

        async fn updated(app: &Arc<AppCtx>, kind: &str, id: i64, agent: Option<&str>, body: Value) -> (StatusCode, Value) {
            let body: UpdateRequest = serde_json::from_value(body).expect("an update");
            answer_of(update_memory(State(app.clone()), Path((kind.to_string(), id)), reader_query(agent), Json(body)).await).await
        }
        let bad = |message: &str| (StatusCode::BAD_REQUEST, json!({ "error": message }));
        let missing = |id: i64| (StatusCode::NOT_FOUND, json!({ "error": format!("no memory row with id {id}") }));
        let types = "semantic_fact, user_preference, architectural_decision, session_summary, code_block, chat_message, tool_error";

        let (app, dir) = daemon_for_jobs().await;
        let options = ResolveOptions { create: true, new_project: false, born: Born::default() };
        let ideas = resolve_project(&app.runtime.store, "ideas", options).await.expect("ideas").project_id.expect("an id");
        run_sql(
            &app,
            "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('twin-0001', 'twin', 'user', 1, 1), ('twin-0002', 'twin', 'user', 1, 2);"
                .to_string(),
        )
        .await;
        let fact = seed_memory(&app, "a", "semantic_fact", "a fact").await;
        let preference = seed_memory(&app, "a", "user_preference", "a preference").await;
        let deleted = seed_memory(&app, "a", "semantic_fact", "a deleted fact").await;
        let theirs = seed_memory(&app, "b", "semantic_fact", "their fact").await;
        run_sql(&app, format!("UPDATE memory SET deleted_at = 2 WHERE id = {deleted}")).await;
        std::fs::write(dir.path().join("sharing.json"), r#"{"version":1,"hides":{"a":["b"]}}"#).expect("write the policy");

        let cases = [
            (
                ("episodic", fact, None, json!({ "importance": 3 })),
                bad(
                    "kind \"episodic\" no longer exists: episodic and procedural rows are memory rows now, of type semantic_fact, \
                     each with a new id; find them with search_memory (POST /v1/search)",
                ),
            ),
            (("bogus", fact, None, json!({ "importance": 3 })), bad("unknown kind \"bogus\": the only kind is \"memory\"")),
            (("bogus", fact, None, json!({})), bad("unknown kind \"bogus\": the only kind is \"memory\"")),
            (
                ("memory", fact, None, json!({ "project_id": "twin-0001", "no_project": true })),
                bad("pass project_id or no_project, not both"),
            ),
            (
                ("memory", fact, None, json!({ "importance": 11, "project_id": "Twin" })),
                (
                    StatusCode::CONFLICT,
                    json!({ "error": "project_ambiguous", "detail": "\"Twin\" is the name of twin-0001, twin-0002: name one of them by its id" }),
                ),
            ),
            (
                ("memory", fact, None, json!({})),
                bad("nothing to change: pass importance, memory_type, a project to move to, or none"),
            ),
            (
                ("memory", fact, None, json!({ "user_answered": true })),
                bad("nothing to change: pass importance, memory_type, a project to move to, or none"),
            ),
            (("memory", fact, None, json!({ "importance": 11 })), bad("importance must be a number from 0 to 10, got 11")),
            (("memory", fact, None, json!({ "importance": -0.5 })), bad("importance must be a number from 0 to 10, got -0.5")),
            (
                ("memory", fact, None, json!({ "memory_type": "preference" })),
                bad(&format!("unknown memory_type \"preference\"; expected one of {types}")),
            ),
            (
                ("memory", fact, None, json!({ "importance": 11, "memory_type": "preference" })),
                bad("importance must be a number from 0 to 10, got 11"),
            ),
            (
                ("memory", 999_999, None, json!({ "importance": 3, "memory_type": "preference" })),
                bad(&format!("unknown memory_type \"preference\"; expected one of {types}")),
            ),
            (("memory", 999_999, None, json!({ "importance": 3 })), missing(999_999)),
            (("memory", deleted, None, json!({ "importance": 3 })), missing(deleted)),
            (("memory", theirs, Some("a"), json!({ "importance": 3 })), missing(theirs)),
            (
                ("memory", preference, None, json!({ "memory_type": "semantic_fact" })),
                (
                    StatusCode::CONFLICT,
                    json!({
                        "error": format!(
                            "memory {preference} is a user_preference, so it loses that type only on the user's word: \
                             ask the user, then call again with `user_answered`"
                        ),
                    }),
                ),
            ),
            (
                ("memory", preference, None, json!({ "project_id": "ideas" })),
                (
                    StatusCode::CONFLICT,
                    json!({
                        "error": format!(
                            "memory {preference} is a user_preference, so it changes project only on the user's word: \
                             ask the user, then call again with `user_answered`"
                        ),
                    }),
                ),
            ),
            (
                ("memory", fact, None, json!({ "project_id": "nowhere" })),
                (
                    StatusCode::NOT_FOUND,
                    json!({ "error": "no project nowhere is registered: register its root first (`clawg projects add`, or a bridge started in it)" }),
                ),
            ),
        ];
        for ((kind, id, agent, body), expected) in cases {
            assert_eq!(updated(&app, kind, id, agent, body.clone()).await, expected, "{kind} {id} {agent:?} {body}");
        }

        let row = |id: i64, importance: f64, memory_type: &str, tier: i64, project_id: Value| {
            json!({ "updated": 1, "kind": "memory", "id": id, "importance": importance, "memory_type": memory_type, "tier": tier, "project_id": project_id })
        };
        assert_eq!(
            updated(&app, "memory", fact, None, json!({ "importance": 7 })).await,
            (StatusCode::OK, row(fact, 7.0, "semantic_fact", 3, Value::Null))
        );
        assert_eq!(
            updated(&app, "memory", theirs, Some("b"), json!({ "memory_type": "code_block" })).await,
            (StatusCode::OK, row(theirs, 5.0, "code_block", 3, Value::Null))
        );
        assert_eq!(
            updated(&app, "memory", preference, None, json!({ "memory_type": "semantic_fact", "user_answered": true })).await,
            (StatusCode::OK, row(preference, 5.0, "semantic_fact", 3, Value::Null))
        );
        let (status, mut body) = updated(&app, "memory", fact, None, json!({ "project_id": "Ideas" })).await;
        let move_id = body.as_object_mut().expect("an object").remove("move_id").expect("the move");
        assert!(move_id.is_i64(), "{move_id}");
        assert_eq!((status, body), (StatusCode::OK, row(fact, 7.0, "semantic_fact", 3, json!(ideas))));
        let (status, mut body) = updated(&app, "memory", fact, None, json!({ "no_project": true })).await;
        let back = body.as_object_mut().expect("an object").remove("move_id").expect("the move");
        assert!(back.is_i64() && back != move_id, "{back}");
        assert_eq!((status, body), (StatusCode::OK, row(fact, 7.0, "semantic_fact", 3, Value::Null)));

        app.runtime.close().await.expect("the store closes");
        assert_eq!(
            updated(&app, "memory", fact, None, json!({ "importance": 3 })).await,
            (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": "memory backend channel error: connection closed during update_memory_core" }))
        );
    }

    /// The status and the whole body `delete_memory` and `restore_memory`
    /// answer with, for every refusal in the order they check them and for a
    /// delete and a restore that go through.
    #[tokio::test]
    async fn each_memory_delete_and_restore_keeps_its_status_and_body() {
        use serde_json::json;

        async fn deleted(app: &Arc<AppCtx>, kind: &str, id: i64, agent: Option<&str>) -> (StatusCode, serde_json::Value) {
            answer_of(delete_memory(State(app.clone()), Path((kind.to_string(), id)), reader_query(agent)).await).await
        }
        async fn restored(app: &Arc<AppCtx>, kind: &str, id: i64, agent: Option<&str>) -> (StatusCode, serde_json::Value) {
            answer_of(restore_memory(State(app.clone()), Path((kind.to_string(), id)), reader_query(agent)).await).await
        }
        let unknown_kind = (StatusCode::BAD_REQUEST, json!({ "error": "unknown kind \"bogus\": the only kind is \"memory\"" }));
        let not_deleted = |id: i64| (StatusCode::NOT_FOUND, json!({ "error": format!("no deleted memory row with id {id}") }));

        let (app, dir) = daemon_for_jobs().await;
        let fact = seed_memory(&app, "a", "semantic_fact", "a fact").await;
        let theirs = seed_memory(&app, "b", "semantic_fact", "their fact").await;
        std::fs::write(dir.path().join("sharing.json"), r#"{"version":1,"hides":{"a":["b"]}}"#).expect("write the policy");

        assert_eq!(deleted(&app, "bogus", fact, None).await, unknown_kind);
        let gone = (StatusCode::NOT_FOUND, json!({ "error": "no memory row with id 999999" }));
        assert_eq!(deleted(&app, "memory", 999_999, None).await, gone);
        assert_eq!(
            deleted(&app, "memory", theirs, Some("a")).await,
            (StatusCode::NOT_FOUND, json!({ "error": format!("no memory row with id {theirs}") }))
        );
        assert_eq!(deleted(&app, "memory", fact, Some("a")).await, (StatusCode::OK, json!({ "deleted": 1 })));
        assert_eq!(
            deleted(&app, "memory", fact, None).await,
            (StatusCode::NOT_FOUND, json!({ "error": format!("no memory row with id {fact}") }))
        );

        let live = seed_memory(&app, "a", "semantic_fact", "a live fact").await;
        let secret = seed_memory(&app, "a", "semantic_fact", "the key is AKIAQ7ZXQ7ZXQ7ZXQ7ZX").await;
        let twin = seed_memory(&app, "a", "semantic_fact", "a twin").await;
        let saved_again = seed_memory(&app, "a", "semantic_fact", "a twin").await;
        let old_topic = seed_memory(&app, "a", "semantic_fact", "the old engine").await;
        let new_topic = seed_memory(&app, "a", "semantic_fact", "the new engine").await;
        let hidden = seed_memory(&app, "b", "semantic_fact", "their deleted fact").await;
        run_sql(
            &app,
            format!(
                "UPDATE memory SET deleted_at = 2 WHERE id IN ({secret}, {twin}, {old_topic}, {hidden});
                 UPDATE memory SET content_hash = 'same' WHERE id IN ({twin}, {saved_again});
                 UPDATE memory SET topic_key = 'storage/engine' WHERE id IN ({old_topic}, {new_topic});"
            ),
        )
        .await;
        let conflict = |message: String| (StatusCode::CONFLICT, json!({ "error": message }));
        let cases = [
            (("bogus", fact, None), unknown_kind.clone()),
            (("memory", 999_999, None), not_deleted(999_999)),
            (("memory", live, None), not_deleted(live)),
            (("memory", hidden, Some("a")), not_deleted(hidden)),
            (
                ("memory", secret, None),
                conflict(format!(
                    "memory #{secret} holds a secret (aws_access_key), so it stays deleted: restored, it would reach searches \
                     and other agents. `clawg purge {secret} --yes` removes it for good"
                )),
            ),
            (
                ("memory", twin, None),
                conflict(format!(
                    "memory #{twin} was saved again as #{saved_again}, which is live; #{twin} stays deleted so the text is not stored twice"
                )),
            ),
            (
                ("memory", old_topic, None),
                conflict(format!(
                    "memory #{old_topic} was saved under topic storage/engine, which #{new_topic} holds now; \
                     #{old_topic} stays deleted so the topic has one current memory"
                )),
            ),
        ];
        for ((kind, id, agent), expected) in cases {
            assert_eq!(restored(&app, kind, id, agent).await, expected, "{kind} {id} {agent:?}");
        }
        assert_eq!(
            restored(&app, "memory", fact, Some("a")).await,
            (StatusCode::OK, json!({ "restored": 1, "id": fact, "status": "active", "superseded_by": null }))
        );
        assert_eq!(restored(&app, "memory", fact, None).await, not_deleted(fact));

        let (waiting, _dir) = daemon_without_embedder().await;
        assert_eq!(restored(&waiting, "bogus", fact, None).await, unknown_kind, "the kind is checked before the embedder");
        assert_eq!(
            restored(&waiting, "memory", fact, None).await,
            (StatusCode::SERVICE_UNAVAILABLE, json!({ "error": "embedder_not_ready" }))
        );
        waiting.runtime.embedder_status.send_replace(EmbedderStatus::Failed { error: "disk full".into(), retryable: false });
        assert_eq!(
            restored(&waiting, "memory", fact, None).await,
            (StatusCode::SERVICE_UNAVAILABLE, json!({ "error": "embedder_failed", "detail": "disk full", "retryable": false }))
        );

        app.runtime.close().await.expect("the store closes");
        assert_eq!(
            deleted(&app, "memory", live, None).await,
            (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": "memory backend channel error: connection closed during delete_memory_core" }))
        );
        assert_eq!(
            restored(&app, "memory", fact, None).await,
            (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": "memory backend channel error: connection closed during restore_memory_core" }))
        );
    }
}
