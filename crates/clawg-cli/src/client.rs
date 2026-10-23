//! HTTP client for a running `clawg serve`.
//!
//! Used by `clawg mcp`, `clawg status` and `clawg stop`. It owns the only two
//! pieces of knowledge a client needs: where the daemon is (`daemon.json`) and
//! how to prove it may talk to it (`token`).
//!
//! # Autostart
//!
//! `clawg mcp` is registered with an MCP host that launches it on demand, and a
//! user who just opened their editor has no reason to have started a daemon
//! first. So [`ensure_running`] starts one when none answers. Spawning is
//! behind [`DaemonSpawner`] rather than called directly, because the real
//! implementation launches a process that downloads a 470 MB model — a test
//! substitutes an in-process daemon instead.
//!
//! Autostart is skipped, with [`ClientError::StoreHeld`], when the memory store
//! is owned by something that is not a daemon: such a daemon could never start,
//! and finding that out by polling costs the whole [`AUTOSTART_TIMEOUT`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use clawg_core::memory::dispute::{OpenPairs, Resolution};
use clawg_core::memory::outbox::OutboxSummary;
use clawg_core::memory::portability::ImportReport;
use clawg_core::memory::relocate::MoveReport;
use clawg_core::memory::store_lock::{StoreHolder, StoreLock, StoreOwner};
use clawg_core::build_info;
use clawg_core::paths::{data_dir, socket_file, PathsError, Profile};
use clawg_core::preferences::Lang;
use clawg_core::private_dir::ensure_private_dir;
use serde::{Deserialize, Serialize};

use crate::daemon::api::DocsSearchBody;
use crate::daemon::lifecycle::{read_state, read_token, DaemonState};
use crate::relevance::RELEVANCE_RATIO;

/// How often [`ensure_running`] re-checks a daemon it just asked for.
pub const AUTOSTART_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// How long [`ensure_running`] waits for a spawned daemon to answer `/health`.
///
/// Generous because a first run migrates the database and opens SQLite before
/// it binds — but it does NOT cover the model download: `/health` answers
/// `embedder: "loading"` long before the model exists, so this is a bound on
/// process startup, not on readiness.
pub const AUTOSTART_TIMEOUT: Duration = Duration::from_secs(60);
/// Timeout for a single liveness probe. Short on purpose: the question is
/// "is something listening on loopback?", which is answered in microseconds or
/// not at all.
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(1);

/// Everything that can go wrong talking to a daemon.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("no clawg daemon is running for this profile")]
    NotRunning,
    #[error("the daemon rejected the token; delete the token file and restart `clawg serve`")]
    Unauthorized,
    /// The daemon answered, and said no. `message` is its `{"error": …}` text.
    #[error("daemon returned {status}: {message}")]
    Daemon { status: u16, message: String, details: ErrorDetails },
    #[error("transport error talking to the daemon: {0}")]
    Http(String),
    /// `--here` needs the current directory to name the project, and the
    /// operating system would not give it.
    #[error("cannot read the current directory for --here: {0}")]
    WorkingDir(String),
    #[error("could not start a daemon: {0}")]
    Spawn(String),
    #[error("timed out after {0:?} waiting for the daemon to start")]
    StartTimeout(Duration),
    #[error("the daemon (pid {pid}) accepted the shutdown but is still running after {timeout:?}")]
    StopTimeout { pid: u32, timeout: Duration },
    /// The memory store is owned by something that is not a daemon (an
    /// evaluation run), so no daemon can start and waiting for one is pointless.
    /// Raised at once instead of after [`AUTOSTART_TIMEOUT`].
    #[error(
        "the memory store is held by {} (pid {}), not by a clawg daemon; close it and reconnect",
        .0.holder, .0.pid
    )]
    StoreHeld(StoreOwner),
    #[error(transparent)]
    DataDir(#[from] PathsError),
}

/// The parts of a daemon error body beyond its `error` code, kept so a caller
/// can say why something failed and what a failed save still stored.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ErrorDetails {
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub retryable: Option<bool>,
    #[serde(default)]
    pub memories: Vec<SavedMemory>,
}

/// Builds the error for a non-success answer from its JSON body, if it had
/// one.
fn daemon_error(status: reqwest::StatusCode, body: Option<serde_json::Value>) -> ClientError {
    let message = body
        .as_ref()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned))
        // A body that is not the daemon's error shape (a proxy page, a
        // truncated response) still has to produce something a user can act
        // on.
        .unwrap_or_else(|| status.to_string());
    let details = body
        .and_then(|v| serde_json::from_value::<ErrorDetails>(v).ok())
        .unwrap_or_default();
    ClientError::Daemon { status: status.as_u16(), message, details }
}

// ── Wire types the client reads back ────────────────────────────────────────
//
// Deliberately separate from the daemon's `Serialize`-only response structs.
// Those mirror core types (`RankedHit`, `MemoryAdminRow`) that carry fields no
// client needs and that deliberately do NOT implement `Deserialize`; mirroring
// them here would force a derive onto core for the sake of a consumer.

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Health {
    pub status: String,
    pub version: String,
    pub profile: String,
    pub pid: u32,
    pub embedder: String,
    /// Why the embedder failed; `None` unless `embedder` is `"failed"`, and
    /// `default` so a daemon from before this field still parses.
    #[serde(default)]
    pub embedder_error: Option<String>,
    /// Whether the daemon will retry the failed load on its own.
    #[serde(default)]
    pub embedder_retryable: Option<bool>,
    /// Whether the model is in memory now; `false` when the daemon let it go
    /// for being idle. `None` before the first load, or from a daemon that
    /// always keeps it.
    #[serde(default)]
    pub embedder_resident: Option<bool>,
    pub uptime_s: u64,
    /// Saves accepted and not stored yet, and those that failed after the
    /// answer. `None` from a daemon before the outbox, or when the store did
    /// not answer health in time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingest: Option<OutboxSummary>,
}

impl Health {
    /// Whether retrieval will work right now — i.e. whether `/v1/search` will
    /// answer instead of `503`.
    pub fn embedder_ready(&self) -> bool {
        self.embedder == "ready"
    }
}

/// What `POST /v1/search` answers.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Found {
    pub hits: Vec<Hit>,
    /// Hits the daemon's relevance cut kept back. A daemon that predates the
    /// count does not send it; its cut always left two hits standing.
    #[serde(default)]
    pub withheld: usize,
    /// Live saves of the searched scope in the other language, when the
    /// search named its own. A daemon that predates it never sends it.
    #[serde(default)]
    pub other_language: Option<OtherLanguage>,
}

/// How many memories a search in one language is likely to miss for being
/// in the other. `lang` stays text, so a language this bridge does not know
/// leaves the answer readable.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OtherLanguage {
    pub lang: String,
    pub memories: usize,
}

/// One retrieval hit, as much of it as a caller formats.
#[derive(Debug, Clone, Deserialize)]
pub struct Hit {
    pub id: i64,
    /// `"Memory"` | `"Episodic"` | `"Procedural"` — the core serializes
    /// `TableKind` in PascalCase, and this type does not re-spell it.
    pub kind: String,
    pub content: String,
    pub memory_type: String,
    pub tier: i64,
    pub score: f64,
    /// Who saved it. A daemon older than sharing does not send it, and every
    /// hit it returns is the caller's own.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Which piece of a long save the hit is; absent for a save of one piece
    /// and from a daemon that does not link pieces.
    #[serde(default)]
    pub piece: Option<PiecePosition>,
    /// The hit holds every identifier of the query, so the relevance cut
    /// does not measure the others against it. A daemon that predates the
    /// flag does not send it, and its hits are cut as before.
    #[serde(default)]
    pub exact: bool,
    /// The key a new version of this memory is saved under. A daemon that
    /// predates the field does not send it, and the hit is shown without one.
    #[serde(default)]
    pub topic_key: Option<String>,
    /// The project the row was saved in: `Some(None)` for a row without one,
    /// `None` from a daemon that predates the field.
    #[serde(default, deserialize_with = "sent")]
    pub project_id: Option<Option<String>>,
}

/// A field a daemon sends even when it is `null`, read so that `None` means
/// the daemon did not send it at all.
fn sent<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

/// Position `piece` (from 1) of a save stored as `pieces` rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct PiecePosition {
    pub piece: i64,
    pub pieces: i64,
}

/// One inventory row from `GET /v1/memories`.
///
/// Every field past `access_count` is optional and defaulted: a daemon that
/// predates them omits the keys, and the MCP bridge must keep reading it.
/// Serialized under the API's own field names, which is what `--json` prints.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct MemoryRow {
    pub id: i64,
    /// `"memory"` | `"episodic"` | `"procedural"` (lowercase — `MemoryKind`,
    /// not `TableKind`; the two spellings are a known wart in core).
    pub kind: String,
    pub memory_type: Option<String>,
    /// The row's full content, not a preview.
    pub summary: String,
    pub tier: i64,
    pub importance: f64,
    pub status: String,
    pub access_count: i64,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub last_accessed_at: Option<i64>,
    #[serde(default)]
    pub superseded_by: Option<i64>,
    #[serde(default)]
    pub deleted_at: Option<i64>,
    #[serde(default)]
    pub success_count: Option<i64>,
    #[serde(default)]
    pub trigger: Option<String>,
    #[serde(default)]
    pub source_trajectory_ids: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub source_session_id: Option<String>,
    #[serde(default)]
    pub created_by_type: Option<String>,
    #[serde(default)]
    pub created_by_ref: Option<String>,
    #[serde(default)]
    pub last_modified_at: Option<i64>,
    #[serde(default)]
    pub update_count: Option<i64>,
    #[serde(default)]
    pub dispute_count: Option<i64>,
    /// Raw JSON text of the ids this row disputes.
    #[serde(default)]
    pub disputed_against: Option<String>,
    #[serde(default)]
    pub last_disputed_at: Option<i64>,
    /// The memories this row is a conclusion drawn from; absent on any other
    /// row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<Vec<i64>>,
    /// The topic key the row was saved under; absent without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
}

/// What `POST /v1/context` answers.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Context {
    pub prompt: String,
    /// Memories of the scope that arrived since it was last reviewed for
    /// conclusions. A daemon that predates the count omits it.
    #[serde(default)]
    pub unreviewed: usize,
    /// Conclusions of the scope that lost a source since it was last
    /// reviewed. A daemon that predates the count omits it.
    #[serde(default)]
    pub conclusions_to_review: usize,
    /// Whether memories were searched for the prompt. A daemon that predates
    /// the field omits it, and then nothing can be said either way.
    #[serde(default)]
    pub retrieval: Option<Retrieval>,
    /// As in [`Found`], for a context asked with a query and a language.
    #[serde(default)]
    pub other_language: Option<OtherLanguage>,
}

/// Whether `POST /v1/context` searched memories, and if not, why.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Retrieval {
    /// A daemon that predates the counts sends neither, and then nothing is
    /// said about relevance.
    Searched {
        #[serde(default)]
        hits: usize,
        #[serde(default)]
        withheld: usize,
    },
    EmbedderNotReady,
    EmbedderFailed { detail: String, retryable: bool },
    SearchFailed { detail: String },
    /// A state a newer daemon added: the context is still good to read.
    #[serde(other)]
    Unknown,
}

/// What `POST /v1/synthesis` answers.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Synthesis {
    pub pending: Vec<MemoryRow>,
    pub pending_total: usize,
    pub conclusions: Vec<MemoryRow>,
    /// Conclusions with a lost source. A daemon that predates them omits
    /// both fields.
    #[serde(default)]
    pub to_review: Vec<ConclusionReview>,
    #[serde(default)]
    pub to_review_total: usize,
}

/// A conclusion with a source that was deleted, replaced or purged.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ConclusionReview {
    pub conclusion: MemoryRow,
    /// Sources still live.
    pub live: usize,
    pub lost: Vec<LostSource>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct LostSource {
    pub id: i64,
    /// `deleted`, `superseded` or `purged`.
    pub state: String,
    #[serde(default)]
    pub superseded_by: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SynthesisArgs {
    /// Absent, the review is the store owner's and covers every agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub limit: Option<u32>,
}

/// One bucket of a grouping from `GET /v1/stats`. `key: None` is the NULL
/// bucket (rows with no project, say).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Count {
    pub key: Option<String>,
    pub count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct StatsTotals {
    pub memory: u64,
    pub episodic: u64,
    pub procedural: u64,
    /// Soft-deleted rows across the three tables; not part of the live counts.
    pub deleted: u64,
}

/// The body of `GET /v1/stats`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct MemoryStats {
    pub totals: StatsTotals,
    pub by_type: Vec<Count>,
    pub by_tier: Vec<Count>,
    pub by_status: Vec<Count>,
    pub by_agent: Vec<Count>,
    pub by_project: Vec<Count>,
    pub newest_created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SaveArgs {
    pub content: String,
    pub agent_id: String,
    pub project_id: Option<String>,
    pub memory_type: Option<String>,
    pub importance: Option<f64>,
    /// Left off the wire when empty, so a save that names no sources reads
    /// the same to a daemon that predates them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub derived_from: Vec<i64>,
    /// Left off the wire when absent, for the same reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    /// Left off the wire when false, like `derived_from`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub user_stated: bool,
}

/// A row a save wrote or reinforced, as `POST /v1/memories` reports it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SavedMemory {
    pub kind: String,
    pub id: i64,
    /// `"inserted"` or `"reinforced"`.
    pub outcome: String,
}

/// An earlier row a save was found close to, as `POST /v1/memories` reports it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RelatedMemory {
    pub id: i64,
    /// The pair waits for the user in ACTIVE CONFLICTS.
    pub escalated: bool,
    /// The first line of the earlier row.
    #[serde(default)]
    pub excerpt: String,
}

/// An earlier row a save under a topic key superseded, as `POST /v1/memories`
/// reports it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReplacedMemory {
    pub id: i64,
    /// The first line of the earlier row.
    #[serde(default)]
    pub excerpt: String,
}

/// How a row ended up after `PATCH /v1/memories/{kind}/{id}`.
///
/// Every field but `memory_type` is required: a daemon older than the type
/// change answers only `updated`, and reading that as success would report a
/// tier nobody computed.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Updated {
    pub kind: String,
    pub id: i64,
    pub importance: f64,
    /// Only `memory` rows have one.
    pub memory_type: Option<String>,
    pub tier: i64,
    /// The project the row is in after the change; a daemon that predates
    /// moves sends none.
    #[serde(default)]
    pub project_id: Option<String>,
    /// The other rows of its save or topic history that moved with it.
    #[serde(default)]
    pub pulled_in: Vec<i64>,
}

/// A failed save taken back to pending. `queued` is false when the daemon's
/// queue was full and it waits for the next start.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OutboxRetried {
    pub operation_id: String,
    pub agent_id: String,
    pub queued: bool,
}

/// A failed save dropped, with the memories it had stored, now deleted.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OutboxDiscarded {
    pub operation_id: String,
    pub agent_id: String,
    pub deleted: Vec<i64>,
}

/// How a restored row ended up. A `superseded` one is live but still left out
/// of searches. Every field after `superseded_by` is absent from a daemon
/// that predates them.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Restored {
    pub id: i64,
    pub status: String,
    pub superseded_by: Option<i64>,
    #[serde(default)]
    pub topic_key: Option<String>,
    /// The rows its delete had brought back, superseded by it again.
    #[serde(default)]
    pub replaced: Vec<ReplacedMemory>,
    /// The row that supersedes it is deleted too.
    #[serde(default)]
    pub superseded_by_deleted: bool,
    /// Rows saved while it was deleted that the detector paired with it.
    #[serde(default)]
    pub related: Vec<RelatedMemory>,
}

/// What `DELETE /v1/memories/{kind}/{id}` did. `revived` and `hidden` are
/// absent from a daemon that predates them.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Deleted {
    pub deleted: usize,
    /// The version of a topic the deleted row had replaced, current again.
    #[serde(default)]
    pub revived: Vec<ReplacedMemory>,
    /// Rows a verdict replaced by the deleted row, still superseded by it.
    #[serde(default)]
    pub hidden: Vec<i64>,
}

/// The save a row belongs to, from `GET /v1/memories/{kind}/{id}/save`: a
/// piece of a long save comes back with the whole text, any other row as a
/// save of one piece.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SavedText {
    pub kind: String,
    /// The row asked for, and its position (from 1) in the save.
    pub id: i64,
    pub piece: i64,
    pub pieces: i64,
    /// The row at each position, `None` where it is deleted or not the
    /// caller's to read.
    pub ids: Vec<Option<i64>>,
    /// Positions, from 1, with no row to show.
    pub missing: Vec<i64>,
    pub text: String,
    /// The key of the row asked for. A daemon that predates the field does
    /// not send it, and the save is shown without one.
    #[serde(default)]
    pub topic_key: Option<String>,
    /// The project of the row asked for, read as [`Hit::project_id`] is.
    #[serde(default, deserialize_with = "sent")]
    pub project_id: Option<Option<String>>,
}

#[derive(Serialize)]
struct UpdateBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    importance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_type: Option<&'a str>,
    /// Left out when absent, like `user_answered`.
    #[serde(skip_serializing_if = "Option::is_none")]
    project_id: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    no_project: bool,
    /// Left out when false: a daemon that predates the field refuses any
    /// field it does not know.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    user_answered: bool,
}

/// How the daemon answered a save that did not fail.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveOutcome {
    /// Stored, one entry per chunk. `redacted` names the kinds of secret
    /// masked out before storing; a daemon that predates masking sends none.
    /// `related` names the earlier rows the save was found close to; a daemon
    /// that predates the pair table sends none. `replaced` names the rows of
    /// the save's topic key it superseded, and `key_dropped` says the save
    /// named a key the daemon did not echo: one that predates topic keys
    /// stored it as a new memory and replaced nothing. `revision_candidate`
    /// is the row of `related` a save without a key most likely revises; a
    /// daemon that predates it sends none.
    Saved {
        memories: Vec<SavedMemory>,
        redacted: Vec<String>,
        related: Vec<RelatedMemory>,
        replaced: Vec<ReplacedMemory>,
        key_dropped: bool,
        revision_candidate: Option<i64>,
    },
    /// Accepted but not confirmed; `reason` says why (`"unconfirmed"`,
    /// `"embedder_loading"`, `"already_queued"`, `"partially_saved"`), and
    /// `operation_id` is what it waits under; a daemon that predates the
    /// outbox sends none. A save that stopped part-way names the pieces it
    /// stored in `memories` and why it stopped in `detail`.
    Queued { reason: Option<String>, operation_id: Option<String>, memories: Vec<SavedMemory>, detail: Option<String> },
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchArgs {
    pub query: String,
    pub agent_id: String,
    pub project_id: Option<String>,
    pub limit: Option<usize>,
    /// The language the query is in; the answer then counts the memories in
    /// the other one. Left out without it, as a daemon that predates it
    /// expects.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<Lang>,
}

/// The body of `POST /v1/search`: the caller's arguments plus the bridge's
/// relevance cut.
///
/// The cut rides on every search so the daemon records as accessed only the
/// hits this client goes on to show; asking for the full list would count
/// rows no agent ever read.
#[derive(Serialize)]
struct SearchRequest<'a> {
    #[serde(flatten)]
    args: &'a SearchArgs,
    min_relative_score: f64,
}

fn search_request(args: &SearchArgs) -> SearchRequest<'_> {
    SearchRequest { args, min_relative_score: RELEVANCE_RATIO }
}

#[derive(Debug, Clone, Default)]
pub struct ListArgs {
    /// The agent the listing is for: the daemon lists what it reaches.
    pub reader: Option<String>,
    pub kind: Option<String>,
    pub tier: Option<i64>,
    pub memory_type: Option<String>,
    pub agent: Option<String>,
    pub project: Option<String>,
    /// Lists only the rows saved in no project.
    pub no_project: bool,
    pub status: Option<String>,
    /// Free text over memory content; the daemon then lists `memory` rows only.
    pub q: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    /// Lists the deleted rows instead of the live ones.
    pub deleted: bool,
    /// Only the rows that came from this project of another tool.
    pub origin_project: Option<String>,
}

/// A batch move as `POST /v1/memories/move` takes it: a selection and one
/// target, or a move to undo. Unset fields are left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MoveRequest {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_project: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub from_none: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_project: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub include_deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_project: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub to_none: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<i64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dry_run: bool,
}

/// The query of a call by id an agent makes: no pair at all for the user's
/// own call, because `?agent=` would reach the daemon as the empty agent.
fn reader_query(agent: Option<&str>) -> Vec<(&'static str, &str)> {
    agent.map(|a| ("agent", a)).into_iter().collect()
}

// ── The client ──────────────────────────────────────────────────────────────

/// How a client reaches its daemon: every call goes through [`DaemonClient::send`],
/// which is the only place that touches one.
#[derive(Debug, Clone)]
enum Transport {
    /// TCP, and the socket on Unix.
    Http(reqwest::Client),
    /// The socket on Windows, where reqwest cannot dial one.
    #[cfg(windows)]
    AfUnix(AfUnixClient),
}

#[cfg(windows)]
type AfUnixClient =
    hyper_util::client::legacy::Client<crate::daemon::socket::Connector, http_body_util::Full<hyper::body::Bytes>>;

/// One request to the daemon, before any transport has seen it.
///
/// It has no deadline on purpose: the bridge repeats a call that failed with
/// [`ClientError::NotRunning`], so a call that gave up waiting may already have
/// reached the daemon. Only `health` is bounded, from outside `send`.
#[derive(Debug, Clone)]
pub struct Call {
    method: reqwest::Method,
    path: String,
    query: Vec<(&'static str, String)>,
    body: Option<Vec<u8>>,
    bearer: bool,
}

impl Call {
    /// A call that carries the bearer token.
    pub fn new(method: reqwest::Method, path: impl Into<String>) -> Self {
        Self { method, path: path.into(), query: Vec::new(), body: None, bearer: true }
    }

    pub fn without_bearer(mut self) -> Self {
        self.bearer = false;
        self
    }

    /// Adds query pairs. A call with none has no `?` at all: `?agent=` would
    /// reach the daemon as a scope to the empty agent.
    pub fn query<V: Into<String>>(mut self, pairs: impl IntoIterator<Item = (&'static str, V)>) -> Self {
        self.query.extend(pairs.into_iter().map(|(k, v)| (k, v.into())));
        self
    }

    pub fn json<B: Serialize + ?Sized>(mut self, body: &B) -> Result<Self, ClientError> {
        self.body = Some(serde_json::to_vec(body).map_err(|e| ClientError::Http(e.to_string()))?);
        Ok(self)
    }
}

/// What the daemon answered, whole: its status and the entire body.
#[derive(Debug, Clone)]
pub struct Answer {
    pub status: reqwest::StatusCode,
    pub body: Vec<u8>,
}

/// The URL of `call` under `base`. The query is encoded the way reqwest's
/// `.query()` does it, and is absent when there are no pairs.
fn call_url(base: &str, call: &Call) -> Result<reqwest::Url, ClientError> {
    let mut url = reqwest::Url::parse(&format!("{base}{}", call.path)).map_err(|e| ClientError::Http(e.to_string()))?;
    if !call.query.is_empty() {
        url.query_pairs_mut().extend_pairs(call.query.iter().map(|(k, v)| (*k, v.as_str())));
    }
    Ok(url)
}

/// The failure every non-2xx answer shares: 401 is always `Unauthorized` (a
/// distinct, actionable failure, since the token file is stale), and every
/// other status keeps the daemon's own status and `{"error": …}` message
/// rather than being flattened into a string.
fn check_status(answer: &Answer) -> Result<(), ClientError> {
    if answer.status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(ClientError::Unauthorized);
    }
    if !answer.status.is_success() {
        return Err(daemon_error(answer.status, serde_json::from_slice(&answer.body).ok()));
    }
    Ok(())
}

/// Turns a whole answer into either its parsed body or the right
/// [`ClientError`].
///
/// One funnel for every call so the mapping cannot drift per endpoint.
fn read_response<T: serde::de::DeserializeOwned>(answer: Answer) -> Result<T, ClientError> {
    check_status(&answer)?;
    serde_json::from_slice(&answer.body).map_err(|_| ClientError::Http("error decoding response body".to_string()))
}

/// A connection that was refused, reset or timed out means "no daemon" — the
/// same answer as a missing state file. Anything else is a genuine transport
/// fault worth reporting as itself.
fn classify_transport(is_connect: bool, is_timeout: bool, detail: String) -> ClientError {
    if is_connect || is_timeout {
        ClientError::NotRunning
    } else {
        ClientError::Http(detail)
    }
}

fn map_transport(e: reqwest::Error) -> ClientError {
    classify_transport(e.is_connect(), e.is_timeout(), e.to_string())
}

/// A handle on one daemon: its address, its token, and a reusable HTTP client.
#[derive(Debug, Clone)]
pub struct DaemonClient {
    base_url: String,
    token: String,
    transport: Transport,
    /// The data directory this client was resolved from, so callers can report
    /// it without re-deriving the profile.
    base_dir: PathBuf,
}

/// Where a client dials its daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// The data directory's socket.
    Unix(PathBuf),
    /// `127.0.0.1` on this port.
    Tcp(u16),
}

/// Where the daemon that published `state` is reached: its socket when it
/// published one, whatever port it also has, else its port. `None`
/// when it has neither, which reads as no daemon.
///
/// Never the socket's port as a second try: a process of another user on that
/// port would be the daemon again. `tcp_only` makes a `testing` build behave
/// as a clawg that predates the socket.
pub fn endpoint(state: &DaemonState, base_dir: &Path, tcp_only: bool) -> Option<Endpoint> {
    if state.unix_socket && !tcp_only {
        return Some(Endpoint::Unix(socket_file(base_dir)));
    }
    state.port.map(Endpoint::Tcp)
}

/// The daemon is on loopback and the bearer token travels with every call: a
/// configured or system proxy must never see them.
fn loopback_http() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().expect("build the daemon HTTP client")
}

/// A transport whose every connection goes to the socket at `path`; no proxy
/// and no DNS take part.
#[cfg(unix)]
fn socket_transport(path: &Path) -> Option<Transport> {
    reqwest::Client::builder().no_proxy().unix_socket(path).build().ok().map(Transport::Http)
}

#[cfg(windows)]
fn socket_transport(path: &Path) -> Option<Transport> {
    use hyper_util::rt::{TokioExecutor, TokioTimer};
    // The pool idles 90 s by default, as reqwest's does, but only with a timer.
    let client = hyper_util::client::legacy::Client::builder(TokioExecutor::new())
        .pool_timer(TokioTimer::new())
        .build(crate::daemon::socket::Connector::new(path));
    Some(Transport::AfUnix(client))
}

#[cfg(not(any(unix, windows)))]
fn socket_transport(_path: &Path) -> Option<Transport> {
    None
}

/// Sends `call` over the socket and reads the whole answer, as the reqwest
/// branch does: the bearer is a sensitive header, and a connection that cannot
/// be made is [`ClientError::NotRunning`].
#[cfg(windows)]
async fn send_over_af_unix(
    client: &AfUnixClient,
    token: &str,
    url: &reqwest::Url,
    call: Call,
) -> Result<Answer, ClientError> {
    use http_body_util::BodyExt;
    use hyper::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};

    let http_error = |e: &dyn std::fmt::Display| ClientError::Http(e.to_string());
    let mut request = hyper::Request::builder().method(call.method).uri(url.as_str());
    if call.bearer {
        let mut bearer = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|e| http_error(&e))?;
        bearer.set_sensitive(true);
        request = request.header(AUTHORIZATION, bearer);
    }
    let body = match call.body {
        Some(body) => {
            request = request.header(CONTENT_TYPE, "application/json");
            body
        }
        None => Vec::new(),
    };
    let request = request
        .body(http_body_util::Full::new(hyper::body::Bytes::from(body)))
        .map_err(|e| http_error(&e))?;

    let response = client
        .request(request)
        .await
        .map_err(|e| classify_transport(e.is_connect(), false, e.to_string()))?;
    let status = response.status();
    let body = match response.into_body().collect().await {
        Ok(collected) => collected.to_bytes().to_vec(),
        // As on the reqwest branch: an error answer whose body breaks off is
        // still that error.
        Err(_) if !status.is_success() => Vec::new(),
        Err(e) => return Err(http_error(&e)),
    };
    Ok(Answer { status, body })
}

impl DaemonClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>, base_dir: PathBuf) -> Self {
        Self {
            base_url: base_url.into(),
            token: token.into(),
            transport: Transport::Http(loopback_http()),
            base_dir,
        }
    }

    /// Builds a client from `<base_dir>/daemon.json` + `<base_dir>/token`.
    ///
    /// `None` when either file is missing — which is the same answer as "no
    /// daemon", since a daemon that is up has written both.
    pub fn from_data_dir(base_dir: &Path) -> Option<Self> {
        let state = read_state(base_dir)?;
        let token = read_token(base_dir)?;
        // Always loopback or the derived socket, never a recorded host or
        // path: the daemon binds 127.0.0.1 and `<base_dir>/clawg.sock`, and a
        // state file that claimed otherwise would be a tampered file pointing
        // this client's bearer token elsewhere.
        let (transport, base_url) = match endpoint(&state, base_dir, build_info::tcp_only())? {
            Endpoint::Unix(path) => (socket_transport(&path)?, "http://localhost".to_string()),
            Endpoint::Tcp(port) => (Transport::Http(loopback_http()), format!("http://127.0.0.1:{port}")),
        };
        Some(Self { base_url, token, transport, base_dir: base_dir.to_path_buf() })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Sends `call` and reads the whole answer. Every call of this client
    /// goes through here, so the URL, the bearer and the error mapping cannot
    /// differ per endpoint or per transport.
    pub async fn send(&self, call: Call) -> Result<Answer, ClientError> {
        let url = call_url(&self.base_url, &call)?;
        match &self.transport {
            Transport::Http(http) => {
                let mut request = http.request(call.method, url);
                if call.bearer {
                    request = request.bearer_auth(&self.token);
                }
                if let Some(body) = call.body {
                    request = request.header(reqwest::header::CONTENT_TYPE, "application/json").body(body);
                }
                let response = request.send().await.map_err(map_transport)?;
                let status = response.status();
                let body = match response.bytes().await {
                    Ok(bytes) => bytes.to_vec(),
                    // An error answer whose body breaks off is still that
                    // error: its status is what the caller acts on.
                    Err(_) if !status.is_success() => Vec::new(),
                    Err(e) => return Err(ClientError::Http(e.to_string())),
                };
                Ok(Answer { status, body })
            }
            #[cfg(windows)]
            Transport::AfUnix(client) => send_over_af_unix(client, &self.token, &url, call).await,
        }
    }

    pub async fn health(&self) -> Result<Health, ClientError> {
        let call = Call::new(reqwest::Method::GET, "/health").without_bearer();
        let answer = tokio::time::timeout(HEALTH_TIMEOUT, self.send(call))
            .await
            .map_err(|_| ClientError::NotRunning)??;
        read_response(answer)
    }

    /// Saves a memory and reports what the daemon did with it (see
    /// `daemon::api::save_memory`).
    pub async fn save(&self, args: SaveArgs) -> Result<SaveOutcome, ClientError> {
        #[derive(Deserialize)]
        struct Body {
            status: String,
            #[serde(default)]
            reason: Option<String>,
            #[serde(default)]
            memories: Vec<SavedMemory>,
            #[serde(default)]
            redacted: Vec<String>,
            #[serde(default)]
            related: Vec<RelatedMemory>,
            #[serde(default)]
            replaced: Vec<ReplacedMemory>,
            #[serde(default)]
            revision_candidate: Option<i64>,
            #[serde(default)]
            topic_key: Option<String>,
            #[serde(default)]
            operation_id: Option<String>,
            #[serde(default)]
            detail: Option<String>,
        }
        let keyed = args.topic_key.is_some();
        let answer = self.send(Call::new(reqwest::Method::POST, "/v1/memories").json(&args)?).await?;
        let body: Body = read_response(answer)?;
        Ok(match body.status.as_str() {
            "saved" => SaveOutcome::Saved {
                memories: body.memories,
                redacted: body.redacted,
                related: body.related,
                replaced: body.replaced,
                key_dropped: keyed && body.topic_key.is_none(),
                revision_candidate: body.revision_candidate,
            },
            _ => SaveOutcome::Queued {
                reason: body.reason,
                operation_id: body.operation_id,
                memories: body.memories,
                detail: body.detail,
            },
        })
    }

    pub async fn search(&self, args: SearchArgs) -> Result<Found, ClientError> {
        let answer = self
            .send(Call::new(reqwest::Method::POST, "/v1/search").json(&search_request(&args))?)
            .await?;
        read_response(answer)
    }

    /// `budget: None` leaves the size to the daemon's default; `lang` is as
    /// in [`SearchArgs`].
    pub async fn context(
        &self,
        query: &str,
        agent_id: &str,
        project_id: Option<&str>,
        budget: Option<usize>,
        lang: Option<Lang>,
    ) -> Result<Context, ClientError> {
        let mut body = serde_json::json!({
            "query": query,
            "agent_id": agent_id,
            "project_id": project_id,
        });
        if let Some(budget) = budget {
            body["budget"] = budget.into();
        }
        if let Some(lang) = lang {
            body["lang"] = lang.as_str().into();
        }
        let answer = self.send(Call::new(reqwest::Method::POST, "/v1/context").json(&body)?).await?;
        read_response(answer)
    }

    /// The context without a query: the workspace and the newest saves, as
    /// a session starts, its notes in `lang`. A daemon that predates it
    /// refuses the request.
    pub async fn recent_context(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
        budget: usize,
        lang: Option<Lang>,
    ) -> Result<Context, ClientError> {
        let mut body = serde_json::json!({ "agent_id": agent_id, "project_id": project_id, "budget": budget });
        if let Some(lang) = lang {
            body["lang"] = lang.as_str().into();
        }
        let answer = self.send(Call::new(reqwest::Method::POST, "/v1/context").json(&body)?).await?;
        read_response(answer)
    }

    /// Tells the daemon which host session `agent_id` works in for
    /// `project_id`; the saves that follow carry it. A daemon that predates
    /// it answers 404.
    pub async fn register_session(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
        session_id: &str,
    ) -> Result<(), ClientError> {
        let body = serde_json::json!({ "agent_id": agent_id, "project_id": project_id, "session_id": session_id });
        let answer = self.send(Call::new(reqwest::Method::POST, "/v1/sessions").json(&body)?).await?;
        check_status(&answer)
    }

    pub async fn synthesis(&self, args: &SynthesisArgs) -> Result<Synthesis, ClientError> {
        let answer = self.send(Call::new(reqwest::Method::POST, "/v1/synthesis").json(args)?).await?;
        read_response(answer)
    }

    pub async fn search_docs(
        &self,
        project_id: &str,
        query: &str,
        limit: Option<usize>,
    ) -> Result<DocsSearchBody, ClientError> {
        let body = serde_json::json!({ "project_id": project_id, "query": query, "limit": limit });
        let answer = self
            .send(Call::new(reqwest::Method::POST, "/v1/projects/docs/search").json(&body)?)
            .await?;
        read_response(answer)
    }

    pub async fn list(&self, args: ListArgs) -> Result<Vec<MemoryRow>, ClientError> {
        #[derive(Deserialize)]
        struct Body {
            items: Vec<MemoryRow>,
        }
        // Built as pairs rather than a struct so an absent filter contributes
        // no query parameter at all — `?kind=` would reach the daemon as an
        // empty string and fail the kind parse.
        let mut query: Vec<(&'static str, String)> = Vec::new();
        if let Some(r) = &args.reader {
            query.push(("reader", r.clone()));
        }
        if let Some(k) = &args.kind {
            query.push(("kind", k.clone()));
        }
        if let Some(t) = args.tier {
            query.push(("tier", t.to_string()));
        }
        if let Some(mt) = &args.memory_type {
            query.push(("memory_type", mt.clone()));
        }
        if let Some(a) = &args.agent {
            query.push(("agent", a.clone()));
        }
        if let Some(p) = &args.project {
            query.push(("project", p.clone()));
        }
        if args.no_project {
            query.push(("no_project", "true".into()));
        }
        if let Some(s) = &args.status {
            query.push(("status", s.clone()));
        }
        if let Some(q) = &args.q {
            query.push(("q", q.clone()));
        }
        if let Some(l) = args.limit {
            query.push(("limit", l.to_string()));
        }
        if let Some(o) = args.offset {
            query.push(("offset", o.to_string()));
        }
        if args.deleted {
            query.push(("deleted", "true".into()));
        }
        if let Some(o) = &args.origin_project {
            query.push(("origin_project", o.clone()));
        }

        let answer = self.send(Call::new(reqwest::Method::GET, "/v1/memories").query(query)).await?;
        Ok(read_response::<Body>(answer)?.items)
    }

    /// One row with every column, soft-deleted rows included (they carry
    /// `deleted_at`). A missing row is the daemon's 404.
    pub async fn get(&self, kind: &str, id: i64) -> Result<MemoryRow, ClientError> {
        #[derive(Deserialize)]
        struct Body {
            item: MemoryRow,
        }
        let answer = self.send(Call::new(reqwest::Method::GET, format!("/v1/memories/{kind}/{id}"))).await?;
        Ok(read_response::<Body>(answer)?.item)
    }

    /// The save `id` belongs to. `agent` scopes it to that agent's rows:
    /// another agent's row is the daemon's 404, like a missing one.
    pub async fn read_save(&self, kind: &str, id: i64, agent: Option<&str>) -> Result<SavedText, ClientError> {
        let call = Call::new(reqwest::Method::GET, format!("/v1/memories/{kind}/{id}/save")).query(reader_query(agent));
        read_response(self.send(call).await?)
    }

    /// Counts of what is stored, optionally narrowed to an agent and/or project.
    pub async fn stats(
        &self,
        agent: Option<&str>,
        project: Option<&str>,
    ) -> Result<MemoryStats, ClientError> {
        let mut query: Vec<(&'static str, &str)> = Vec::new();
        if let Some(a) = agent {
            query.push(("agent", a));
        }
        if let Some(p) = project {
            query.push(("project", p));
        }
        read_response(self.send(Call::new(reqwest::Method::GET, "/v1/stats").query(query)).await?)
    }

    /// Changes a row's importance, type or project; the daemon refuses an
    /// empty or invalid change. `project` moves the row's save, `Some(None)`
    /// out of every project. `agent` names the agent the change is for; a row
    /// out of its reach comes back as not found.
    pub async fn update(
        &self,
        kind: &str,
        id: i64,
        importance: Option<f64>,
        memory_type: Option<&str>,
        project: Option<Option<&str>>,
        user_answered: bool,
        agent: Option<&str>,
    ) -> Result<Updated, ClientError> {
        let body = UpdateBody {
            importance,
            memory_type,
            project_id: project.flatten(),
            no_project: project == Some(None),
            user_answered,
        };
        let call = Call::new(reqwest::Method::PATCH, format!("/v1/memories/{kind}/{id}"))
            .query(reader_query(agent))
            .json(&body)?;
        read_response(self.send(call).await?)
    }

    /// Moves memories between projects, or undoes a move, as the user.
    pub async fn move_memories(&self, request: &MoveRequest) -> Result<MoveReport, ClientError> {
        let call = Call::new(reqwest::Method::POST, "/v1/memories/move").json(request)?;
        read_response(self.send(call).await?)
    }

    /// `agent` scopes the delete as in [`DaemonClient::update`].
    pub async fn delete(&self, kind: &str, id: i64, agent: Option<&str>) -> Result<Deleted, ClientError> {
        let call = Call::new(reqwest::Method::DELETE, format!("/v1/memories/{kind}/{id}")).query(reader_query(agent));
        read_response(self.send(call).await?)
    }

    /// Brings back a deleted `memory` row. `agent` scopes it as in
    /// [`DaemonClient::update`].
    pub async fn restore(&self, kind: &str, id: i64, agent: Option<&str>) -> Result<Restored, ClientError> {
        let call =
            Call::new(reqwest::Method::POST, format!("/v1/memories/{kind}/{id}/restore")).query(reader_query(agent));
        read_response(self.send(call).await?)
    }

    /// Takes the failed save of `operation_id` back to pending; `agent` is
    /// needed only when another agent used the same id.
    pub async fn outbox_retry(&self, operation_id: &str, agent: Option<&str>) -> Result<OutboxRetried, ClientError> {
        let body = serde_json::json!({ "operation_id": operation_id, "agent_id": agent });
        read_response(self.send(Call::new(reqwest::Method::POST, "/v1/outbox/retry").json(&body)?).await?)
    }

    /// Drops the failed save of `operation_id` and deletes the memories it
    /// had stored; `agent` as in [`DaemonClient::outbox_retry`].
    pub async fn outbox_discard(&self, operation_id: &str, agent: Option<&str>) -> Result<OutboxDiscarded, ClientError> {
        let body = serde_json::json!({ "operation_id": operation_id, "agent_id": agent });
        read_response(self.send(Call::new(reqwest::Method::POST, "/v1/outbox/discard").json(&body)?).await?)
    }

    /// `agent` scopes the verdict as in [`DaemonClient::update`]. A daemon
    /// that predates verdicts answers a bare 404.
    pub async fn judge(
        &self,
        keep: i64,
        other: i64,
        verdict: &str,
        reason: Option<&str>,
        user_answered: bool,
        agent: Option<&str>,
    ) -> Result<Resolution, ClientError> {
        let body = serde_json::json!({
            "keep": keep, "other": other, "verdict": verdict, "reason": reason,
            "user_answered": user_answered, "agent_id": agent,
        });
        read_response(self.send(Call::new(reqwest::Method::POST, "/v1/disputes/judge").json(&body)?).await?)
    }

    /// The pairs `agent` has not judged yet. A daemon that predates the pair
    /// table answers a bare 404.
    /// The open pairs `agent` judges, in `project_id` when one is given.
    pub async fn open_disputes(&self, agent: &str, project_id: Option<&str>) -> Result<OpenPairs, ClientError> {
        let mut query = vec![("agent_id", agent)];
        query.extend(project_id.map(|p| ("project_id", p)));
        let call = Call::new(reqwest::Method::GET, "/v1/disputes").query(query);
        read_response(self.send(call).await?)
    }

    /// Asks the daemon for a manual snapshot and returns where it wrote it.
    pub async fn backup(&self) -> Result<PathBuf, ClientError> {
        #[derive(Deserialize)]
        struct Body {
            path: PathBuf,
        }
        let answer = self.send(Call::new(reqwest::Method::POST, "/v1/backup")).await?;
        Ok(read_response::<Body>(answer)?.path)
    }

    /// Imports the export `text`. `map_roots` pairs an exported root with
    /// the root it has here, already resolved.
    pub async fn import(
        &self,
        text: &str,
        dry_run: bool,
        agent: Option<&str>,
        map_roots: &[(String, String)],
    ) -> Result<ImportReport, ClientError> {
        let body = serde_json::json!({ "text": text, "dry_run": dry_run, "agent": agent, "map_roots": map_roots });
        read_response(self.send(Call::new(reqwest::Method::POST, "/v1/import").json(&body)?).await?)
    }

    /// Records `root` as a project and moves rows saved under `legacy_ids`
    /// onto it, letting the daemon read its files when `index_docs`. Returns
    /// the id the daemon derived and how many rows moved.
    pub async fn register_project(
        &self,
        root: &std::path::Path,
        legacy_ids: &[String],
        index_docs: bool,
    ) -> Result<(String, usize), ClientError> {
        #[derive(Deserialize)]
        struct Body {
            project_id: String,
            rekeyed: usize,
        }
        let body = serde_json::json!({
            "root_path": root.to_string_lossy(),
            "legacy_ids": legacy_ids,
            "index_docs": index_docs,
        });
        let answer = self.send(Call::new(reqwest::Method::POST, "/v1/projects").json(&body)?).await?;
        let body = read_response::<Body>(answer)?;
        Ok((body.project_id, body.rekeyed))
    }

    pub async fn shutdown(&self) -> Result<(), ClientError> {
        let answer = self.send(Call::new(reqwest::Method::POST, "/v1/shutdown")).await?;
        let _: serde_json::Value = read_response(answer)?;
        Ok(())
    }
}

// ── Spawning ────────────────────────────────────────────────────────────────

/// Starts a detached `clawg serve`.
///
/// A trait with exactly one production implementation, for one reason: the
/// production one launches a process that downloads a 470 MB model, so no test
/// may ever call it. Everything around autostart — the poll loop, the timeout,
/// the not-running branch — is testable only because this seam exists.
pub trait DaemonSpawner: Send + Sync {
    fn spawn_serve(&self, profile: Profile) -> Result<(), ClientError>;
}

/// Launches the current executable as `clawg serve --profile <p>`, detached.
pub struct ProcessSpawner {
    /// Where `logs/serve.log` goes. Passed in rather than derived so it always
    /// matches the data directory the caller resolved.
    pub base_dir: PathBuf,
}

impl DaemonSpawner for ProcessSpawner {
    fn spawn_serve(&self, profile: Profile) -> Result<(), ClientError> {
        let exe = std::env::current_exe()
            .map_err(|e| ClientError::Spawn(format!("cannot locate the clawg executable: {e}")))?;
        // The handle is dropped without a `wait`: this process does not
        // supervise the daemon, and blocking to reap it would defeat the point
        // of detaching. On Unix that leaves one zombie entry until the parent
        // exits — bounded by one, since autostart happens at most once per
        // bridge process.
        spawn_serve_at(&exe, &self.base_dir, profile, None).map(|_child| ())
    }
}

/// The arguments of `clawg serve` for `profile`; without a port the daemon
/// binds the profile's default.
fn serve_args(profile: Profile, port: Option<u16>) -> Vec<String> {
    let mut args = vec!["serve".to_string(), "--profile".to_string(), profile.as_str().to_string()];
    if let Some(port) = port {
        args.push("--port".to_string());
        args.push(port.to_string());
    }
    args
}

/// Launches the binary at `exe` as a detached `clawg serve`, its output
/// appended to `<base_dir>/logs/serve.log`, and returns the child so a caller
/// that cares can watch it.
pub fn spawn_serve_at(
    exe: &Path,
    base_dir: &Path,
    profile: Profile,
    port: Option<u16>,
) -> Result<std::process::Child, ClientError> {
    use std::process::{Command, Stdio};

    // Both streams go to one appended log file. Appending, not truncating:
    // a daemon that died is exactly when the previous run's tail matters.
    // Only a log past its limit is moved aside, whole.
    let log_dir = base_dir.join("logs");
    ensure_private_dir(&log_dir)
        .map_err(|e| ClientError::Spawn(format!("cannot create {}: {e}", log_dir.display())))?;
    let log_path = log_dir.join("serve.log");
    let log = crate::serve_log::open(&log_path)
        .map_err(|e| ClientError::Spawn(format!("cannot open {}: {e}", log_path.display())))?;
    let log_err = log
        .try_clone()
        .map_err(|e| ClientError::Spawn(format!("cannot duplicate the log handle: {e}")))?;

    let mut cmd = Command::new(exe);
    cmd.args(serve_args(profile, port))
        // Null stdin, never inherited: the caller is usually `clawg mcp`,
        // whose stdin IS the MCP transport. A child sharing it would eat
        // protocol frames.
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));

    // Detach so the daemon outlives whatever started it — and, on Windows,
    // so a Ctrl-C in the parent's console does not travel to it.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
        // Detaching the console is not detaching the handles: without
        // this the daemon inherits the host's stdio pipes and the host
        // never sees this bridge exit. See `detach.rs`.
        crate::detach::disinherit_std_handles();
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group, so a SIGINT delivered to the parent's
        // group stops the bridge without taking the daemon with it.
        cmd.process_group(0);
    }

    cmd.spawn()
        .map_err(|e| ClientError::Spawn(format!("could not spawn `clawg serve`: {e}")))
}

/// Returns a client for a live daemon, starting one if allowed.
///
/// Thin wrapper over [`ensure_running_in`] that resolves the profile's real
/// data directory — the same wrapper/core split the memory admin commands use,
/// and for the same reason: the core is testable against a tempdir, the
/// wrapper's only job is `data_dir`.
pub async fn ensure_running(
    profile: Profile,
    autostart: bool,
    spawner: &dyn DaemonSpawner,
) -> Result<DaemonClient, ClientError> {
    ensure_running_in(&data_dir(profile)?, profile, autostart, spawner, AUTOSTART_TIMEOUT).await
}

/// [`ensure_running`] against an explicit data directory and timeout.
pub async fn ensure_running_in(
    base_dir: &Path,
    profile: Profile,
    autostart: bool,
    spawner: &dyn DaemonSpawner,
    timeout: Duration,
) -> Result<DaemonClient, ClientError> {
    match probe_running(base_dir, autostart).await? {
        Probe::Running(client) => Ok(client),
        Probe::NeedsStart => start_daemon(base_dir, profile, spawner, timeout).await,
    }
}

/// What the quick half of [`ensure_running_in`] found.
pub enum Probe {
    /// A daemon answered `/health`.
    Running(DaemonClient),
    /// None answered and one may be started: the slow half, [`start_daemon`].
    NeedsStart,
}

/// The half of [`ensure_running_in`] that is bounded by one short `/health`
/// probe and two file reads, so a caller can run it before it must answer
/// anyone. Everything that can wait on a cold start is in [`start_daemon`].
pub async fn probe_running(base_dir: &Path, autostart: bool) -> Result<Probe, ClientError> {
    // A state file alone proves nothing: it survives a crash, and its pid may
    // belong to something else entirely by now. Only a `/health` answer counts.
    if let Some(client) = DaemonClient::from_data_dir(base_dir) {
        if client.health().await.is_ok() {
            return Ok(Probe::Running(client));
        }
    }

    // Before anything that waits. A store owned by a NON-daemon (an evaluation
    // run) means the `clawg serve` we would spawn dies on the store lock at
    // once, and the poll below would burn the whole timeout on a daemon that
    // cannot come up. A daemon holder — possibly still starting, so not yet
    // publishing `daemon.json` — or an unknown holder falls through to the
    // normal path: only a positively-identified non-daemon short-circuits.
    // Ahead of the `autostart` check because "something else has the store" is
    // the truthful answer even with `--no-autostart`, where "not running" would
    // send the user to start a daemon that cannot start.
    if let Some(owner) = StoreLock::current_owner(base_dir) {
        if owner.holder != StoreHolder::Daemon {
            return Err(ClientError::StoreHeld(owner));
        }
    }

    if !autostart {
        return Err(ClientError::NotRunning);
    }

    Ok(Probe::NeedsStart)
}

/// Spawns a daemon and polls until it answers `/health` or `timeout` passes.
pub async fn start_daemon(
    base_dir: &Path,
    profile: Profile,
    spawner: &dyn DaemonSpawner,
    timeout: Duration,
) -> Result<DaemonClient, ClientError> {
    // The data directory has to exist before the daemon can publish into it —
    // on a first ever run it does not.
    ensure_private_dir(base_dir)
        .map_err(|e| ClientError::Spawn(format!("cannot create {}: {e}", base_dir.display())))?;

    spawner.spawn_serve(profile)?;

    // Poll rather than wait on the child: the daemon is detached, so there is
    // no handle to await, and the only fact worth having is that it answers.
    // The state file appearing is NOT enough — it is written before the first
    // request can be served.
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(client) = DaemonClient::from_data_dir(base_dir) {
            if client.health().await.is_ok() {
                return Ok(client);
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(ClientError::StartTimeout(timeout));
        }
        tokio::time::sleep(AUTOSTART_POLL_INTERVAL).await;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    use clawg_core::memory::store_lock::{StoreHolder, StoreLock, StoreOwner};
    use clawg_core::paths::{store_lock_file, store_owner_file};
    use clawg_core::runtime::MemoryRuntime;

    use crate::daemon::api::{router, AppCtx, SAVE_CONFIRM_TIMEOUT};
    use crate::daemon::lifecycle::{write_state, write_token, DaemonState};

    #[test]
    fn serve_is_started_on_the_profiles_default_port_unless_one_is_given() {
        assert_eq!(serve_args(Profile::Dev, None), ["serve", "--profile", "dev"]);
        assert_eq!(serve_args(Profile::Prod, Some(27380)), ["serve", "--profile", "prod", "--port", "27380"]);
    }

    #[test]
    fn a_hit_tells_a_missing_project_from_none() {
        let hit = |project: &str| -> Hit {
            let json = format!(r#"{{"id":1,"kind":"Memory","content":"c","memory_type":"semantic_fact","tier":3,"score":0.5{project}}}"#);
            serde_json::from_str(&json).expect("a hit")
        };
        assert_eq!(hit("").project_id, None, "a daemon that predates the field");
        assert_eq!(hit(r#","project_id":null"#).project_id, Some(None), "a row without a project");
        assert_eq!(hit(r#","project_id":"p""#).project_id, Some(Some("p".into())));

        let save: SavedText = serde_json::from_str(
            r#"{"kind":"memory","id":1,"piece":1,"pieces":1,"ids":[1],"missing":[],"text":"t","project_id":null}"#,
        )
        .expect("a save");
        assert_eq!(save.project_id, Some(None));
    }

    #[test]
    fn a_health_without_ingest_still_parses() {
        let old = r#"{"status":"ok","version":"0.5.0","profile":"dev","pid":1,"embedder":"ready","uptime_s":3}"#;
        let h: Health = serde_json::from_str(old).expect("a daemon from before the outbox");
        assert!(h.ingest.is_none());
    }

    #[test]
    fn a_health_without_residency_still_parses() {
        let old = r#"{"status":"ok","version":"0.11.0","profile":"dev","pid":1,"embedder":"ready","uptime_s":3}"#;
        let h: Health = serde_json::from_str(old).expect("a daemon that always keeps its model");
        assert_eq!(h.embedder_resident, None);
    }

    #[test]
    fn a_search_answer_without_withheld_still_parses() {
        let old: Found = serde_json::from_str(r#"{"hits":[]}"#).expect("a daemon that never said");
        assert_eq!(old.withheld, 0);
    }

    #[test]
    fn a_search_answer_without_other_language_still_parses() {
        let old: Found = serde_json::from_str(r#"{"hits":[],"withheld":0}"#).expect("a daemon that never said");
        assert_eq!(old.other_language, None);
        let old: Context = serde_json::from_str(r#"{"prompt":""}"#).expect("a daemon that never said");
        assert_eq!(old.other_language, None);
        let new: Found = serde_json::from_str(r#"{"hits":[],"other_language":{"lang":"en","memories":3}}"#).unwrap();
        assert_eq!(new.other_language, Some(OtherLanguage { lang: "en".into(), memories: 3 }));
        let newer: Found = serde_json::from_str(r#"{"hits":[],"other_language":{"lang":"fr","memories":3}}"#)
            .expect("a language this bridge does not know still parses");
        assert_eq!(newer.other_language.map(|o| o.lang), Some("fr".to_string()));
    }

    #[test]
    fn a_search_names_its_language_only_when_it_has_one() {
        let args = |lang| SearchArgs { query: "q".into(), agent_id: "a".into(), project_id: None, limit: None, lang };
        let without = serde_json::to_value(search_request(&args(None))).unwrap();
        assert!(without.get("lang").is_none(), "{without}");
        let with = serde_json::to_value(search_request(&args(Some(Lang::Es)))).unwrap();
        assert_eq!(with["lang"], "es", "{with}");
    }

    #[test]
    fn a_searched_context_without_counts_still_parses() {
        let old: Context =
            serde_json::from_str(r#"{"prompt":"","unreviewed":0,"retrieval":{"state":"searched"}}"#).expect("parses");
        assert_eq!(old.retrieval, Some(Retrieval::Searched { hits: 0, withheld: 0 }));
    }

    #[test]
    fn answers_without_conclusions_to_review_still_parse() {
        let old: Context = serde_json::from_str(r#"{"prompt":"","unreviewed":0}"#).expect("a daemon that never said");
        assert_eq!(old.conclusions_to_review, 0);
        let old: Synthesis = serde_json::from_str(r#"{"pending":[],"pending_total":0,"conclusions":[]}"#)
            .expect("a daemon that never said");
        assert!(old.to_review.is_empty());
        assert_eq!(old.to_review_total, 0);
    }

    #[test]
    fn a_context_without_retrieval_still_parses() {
        let old: Context = serde_json::from_str(r#"{"prompt":"","unreviewed":0}"#).expect("a daemon that never said");
        assert_eq!(old.retrieval, None);

        let failed: Context = serde_json::from_str(
            r#"{"prompt":"","unreviewed":0,"retrieval":{"state":"embedder_failed","detail":"disk full","retryable":true}}"#,
        )
        .unwrap();
        assert_eq!(failed.retrieval, Some(Retrieval::EmbedderFailed { detail: "disk full".into(), retryable: true }));

        let newer: Context = serde_json::from_str(r#"{"prompt":"","unreviewed":0,"retrieval":{"state":"thinking"}}"#)
            .expect("a state from a newer daemon is not a broken answer");
        assert_eq!(newer.retrieval, Some(Retrieval::Unknown));
    }

    /// An in-process daemon plus the tempdir holding its lifecycle files.
    pub(crate) struct TestDaemon {
        pub(crate) dir: tempfile::TempDir,
        addr: SocketAddr,
        token: String,
    }

    pub(crate) async fn start_daemon() -> TestDaemon {
        let dir = tempfile::tempdir().expect("tempdir");
        let (runtime, rx) = MemoryRuntime::new_for_test(dir.path().to_path_buf()).await;
        tokio::spawn(runtime.ingest_worker(rx, tokio::sync::watch::channel(None).1));

        let token = "0123456789abcdef0123456789abcdef".to_string();
        let (shutdown, _rx) = tokio::sync::watch::channel(false);
        let ctx = Arc::new(AppCtx {
            runtime,
            token: token.clone(),
            profile: Profile::Dev,
            started_at: Instant::now(),
            shutdown,
            save_timeout: SAVE_CONFIRM_TIMEOUT,
            sessions: Default::default(),
            metrics: Default::default(),
        });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(ctx)).await;
        });

        TestDaemon { dir, addr, token }
    }

    /// Publishes the daemon's lifecycle files so `from_data_dir` can find it.
    pub(crate) fn publish(d: &TestDaemon) {
        write_state(
            d.dir.path(),
            &DaemonState {
                pid: std::process::id(),
                port: Some(d.addr.port()),
                unix_socket: false,
                profile: "dev".into(),
                version: "0.1.0".into(),
                started_at_ms: 0,
            },
        )
        .expect("write state");
        write_token(d.dir.path(), &d.token).expect("write token");
    }

    /// Stands in for `ProcessSpawner`: brings up an in-process daemon and
    /// publishes its files, exactly as a real `clawg serve` would.
    struct FakeSpawner {
        daemon: TestDaemon,
        calls: AtomicUsize,
    }

    impl DaemonSpawner for FakeSpawner {
        fn spawn_serve(&self, _profile: Profile) -> Result<(), ClientError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            publish(&self.daemon);
            Ok(())
        }
    }

    /// Never brings anything up — the "the daemon failed to start" path.
    struct DeadSpawner;
    impl DaemonSpawner for DeadSpawner {
        fn spawn_serve(&self, _profile: Profile) -> Result<(), ClientError> {
            Ok(())
        }
    }

    /// [`DeadSpawner`] that counts its calls, for the tests that must prove a
    /// spawn did or did not happen.
    struct CountingDeadSpawner {
        calls: AtomicUsize,
    }
    impl CountingDeadSpawner {
        fn new() -> Self {
            Self { calls: AtomicUsize::new(0) }
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }
    impl DaemonSpawner for CountingDeadSpawner {
        fn spawn_serve(&self, _profile: Profile) -> Result<(), ClientError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    // ── from_data_dir ───────────────────────────────────────────────────────

    /// The MCP bridge repeats a call that failed with `NotRunning` (see
    /// `mcp::startup::on_failure`). That is safe only while `NotRunning` means
    /// the connection never opened, which a request timeout would break: a
    /// timed-out save may already have reached the daemon.
    #[test]
    fn only_health_sets_a_timeout() {
        let source = include_str!("client.rs");
        let code = &source[..source.find("#[cfg(test)]").expect("test module")];
        let all = code.replace("is_timeout()", "").matches("timeout(").count();
        let health = &code[code.find("pub async fn health").expect("health")..];
        let health = &health[..health.find("\n    }").expect("end")];
        assert_eq!(all, 1, "a timeout outside `health` would let a repeated call reach the daemon twice");
        assert!(health.contains("tokio::time::timeout(HEALTH_TIMEOUT, self.send(call))"), "{health}");
    }

    // ── The funnel's pure parts ─────────────────────────────────────────────

    fn answer(status: u16, body: &str) -> Answer {
        Answer { status: reqwest::StatusCode::from_u16(status).expect("status"), body: body.as_bytes().to_vec() }
    }

    #[test]
    fn a_401_is_unauthorized_whatever_it_carries() {
        let read = read_response::<Health>(answer(401, r#"{"error":"nope"}"#));
        assert!(matches!(read, Err(ClientError::Unauthorized)), "{read:?}");
    }

    #[test]
    fn an_error_keeps_the_status_and_the_message_of_its_body() {
        let body = r#"{"error":"ingest_failed","detail":"disk full","retryable":true}"#;
        match read_response::<Health>(answer(500, body)) {
            Err(ClientError::Daemon { status, message, details }) => {
                assert_eq!(status, 500);
                assert_eq!(message, "ingest_failed");
                assert_eq!(details.detail.as_deref(), Some("disk full"));
                assert_eq!(details.retryable, Some(true));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_error_without_json_is_named_by_its_status() {
        match read_response::<Health>(answer(502, "<html>bad gateway</html>")) {
            Err(ClientError::Daemon { status, message, details }) => {
                assert_eq!(status, 502);
                assert_eq!(message, "502 Bad Gateway");
                assert_eq!(details, ErrorDetails::default());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_invalid_success_body_is_a_transport_error() {
        match read_response::<Health>(answer(200, "not json")) {
            Err(ClientError::Http(message)) => assert_eq!(message, "error decoding response body"),
            other => panic!("{other:?}"),
        }
        let count: serde_json::Value = read_response(answer(200, r#"{"n":1}"#)).expect("valid");
        assert_eq!(count["n"], 1);
    }

    #[test]
    fn a_status_check_ignores_the_body_of_a_success() {
        assert!(check_status(&answer(201, "")).is_ok());
        assert!(matches!(check_status(&answer(401, "")), Err(ClientError::Unauthorized)));
        assert!(matches!(check_status(&answer(404, "")), Err(ClientError::Daemon { status: 404, .. })));
    }

    #[test]
    fn a_call_url_encodes_its_query_as_reqwest_does() {
        let base = "http://127.0.0.1:27381";
        let client = reqwest::Client::new();
        let sets: [Vec<(&'static str, String)>; 4] = [
            vec![],
            vec![("agent", "a b".to_string())],
            vec![("q", "x&y=z/é?#+".to_string()), ("limit", "5".to_string())],
            vec![("deleted", "true".to_string()), ("project", "C:\\src\\p".to_string())],
        ];
        for pairs in sets {
            let call = Call::new(reqwest::Method::GET, "/v1/memories").query(pairs.clone());
            let expected = client.get(format!("{base}/v1/memories")).query(&pairs).build().expect("request");
            assert_eq!(call_url(base, &call).expect("url"), *expected.url(), "{pairs:?}");
        }
        let bare = call_url(base, &Call::new(reqwest::Method::GET, "/health")).expect("url");
        assert_eq!(bare.as_str(), "http://127.0.0.1:27381/health", "no `?` without pairs");
    }

    #[test]
    fn a_connection_or_a_timeout_means_no_daemon_and_anything_else_is_a_fault() {
        assert!(matches!(classify_transport(true, false, "refused".into()), ClientError::NotRunning));
        assert!(matches!(classify_transport(false, true, "slow".into()), ClientError::NotRunning));
        match classify_transport(false, false, "reset by peer".into()) {
            ClientError::Http(message) => assert_eq!(message, "reset by peer"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn from_data_dir_is_none_without_lifecycle_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(DaemonClient::from_data_dir(dir.path()).is_none());
    }

    #[tokio::test]
    async fn from_data_dir_needs_both_the_state_and_the_token() {
        let d = start_daemon().await;
        write_state(
            d.dir.path(),
            &DaemonState {
                pid: 1,
                port: Some(d.addr.port()),
                unix_socket: false,
                profile: "dev".into(),
                version: "0.1.0".into(),
                started_at_ms: 0,
            },
        )
        .expect("write state");
        assert!(
            DaemonClient::from_data_dir(d.dir.path()).is_none(),
            "a state file without a token cannot authenticate, so it is not a usable client"
        );

        write_token(d.dir.path(), &d.token).expect("write token");
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("now both files exist");
        assert!(c.base_url().contains("127.0.0.1"));
        assert!(c.base_url().contains(&d.addr.port().to_string()));
    }

    fn published(port: Option<u16>, unix_socket: bool) -> DaemonState {
        DaemonState {
            pid: 1,
            port,
            unix_socket,
            profile: "dev".into(),
            version: "0.1.0".into(),
            started_at_ms: 0,
        }
    }

    #[test]
    fn the_socket_wins_when_published_and_tcp_is_used_otherwise() {
        let base = Path::new("/data");
        let both = published(Some(27381), true);

        assert_eq!(endpoint(&both, base, false), Some(Endpoint::Unix(base.join("clawg.sock"))));
        assert_eq!(endpoint(&published(None, true), base, false), Some(Endpoint::Unix(base.join("clawg.sock"))));
        assert_eq!(endpoint(&both, base, true), Some(Endpoint::Tcp(27381)), "a clawg older than the socket");
        assert_eq!(endpoint(&published(Some(27381), false), base, false), Some(Endpoint::Tcp(27381)));
        assert_eq!(endpoint(&published(None, false), base, false), None);
    }

    async fn serve_on_socket(dir: &Path, token: &str) {
        let (runtime, _rx) = MemoryRuntime::new_for_test(dir.to_path_buf()).await;
        let (shutdown, _) = tokio::sync::watch::channel(false);
        let ctx = Arc::new(AppCtx {
            runtime,
            token: token.to_string(),
            profile: Profile::Dev,
            started_at: Instant::now(),
            shutdown,
            save_timeout: SAVE_CONFIRM_TIMEOUT,
            sessions: Default::default(),
            metrics: Default::default(),
        });
        let listener = crate::daemon::socket::bind_socket(&dir.join("clawg.sock")).expect("bind the socket");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(ctx)).await;
        });
    }

    #[tokio::test]
    async fn a_client_reaches_a_daemon_over_its_published_socket() {
        let dir = tempfile::tempdir().expect("tempdir");
        serve_on_socket(dir.path(), "socket-token").await;
        // A port nothing listens on: the socket alone must be dialled.
        write_state(dir.path(), &published(Some(9), true)).expect("write state");
        write_token(dir.path(), "socket-token").expect("write token");

        let c = DaemonClient::from_data_dir(dir.path()).expect("client");

        assert_eq!(c.base_url(), "http://localhost");
        assert_eq!(c.health().await.expect("health").profile, "dev");
        c.stats(None, None).await.expect("the token is sent over the socket too");
        c.search(SearchArgs {
            query: "a body travels over the socket as JSON".into(),
            agent_id: "agent-socket".into(),
            project_id: None,
            limit: Some(5),
            lang: None,
        })
        .await
        .expect("a call with a body");
    }

    /// What a kill leaves: the socket file, with no listener behind it. A
    /// dropped Windows listener closes its socket when its accept thread wakes,
    /// a moment after the drop, so this waits until a connect is refused.
    async fn leave_an_orphan(dir: &Path, probe: &DaemonClient) {
        drop(crate::daemon::socket::bind_socket(&dir.join("clawg.sock")).expect("bind"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !matches!(probe.health().await, Err(ClientError::NotRunning)) {
            assert!(Instant::now() < deadline, "the dropped listener still takes connections");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn a_client_on_a_published_socket_with_nothing_behind_is_not_running() {
        let dir = tempfile::tempdir().expect("tempdir");
        // The port is open and belongs to someone else: the client must not try it.
        let other = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a port");
        other.set_nonblocking(true).expect("non-blocking");
        let port = other.local_addr().expect("addr").port();
        write_state(dir.path(), &published(Some(port), true)).expect("write state");
        write_token(dir.path(), "t").expect("write token");
        let c = DaemonClient::from_data_dir(dir.path()).expect("client");

        assert!(matches!(c.health().await, Err(ClientError::NotRunning)), "no socket file");

        leave_an_orphan(dir.path(), &c).await;
        assert!(matches!(c.health().await, Err(ClientError::NotRunning)), "an orphan socket");

        let touched = other.accept();
        assert!(
            matches!(&touched, Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "the published port was dialled: {touched:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn thirty_two_simultaneous_clients_over_the_socket_all_answer() {
        let dir = tempfile::tempdir().expect("tempdir");
        serve_on_socket(dir.path(), "burst-token").await;
        write_state(dir.path(), &published(None, true)).expect("write state");
        write_token(dir.path(), "burst-token").expect("write token");

        let start = Arc::new(tokio::sync::Barrier::new(32));
        let calls: Vec<_> = (0..32)
            .map(|_| {
                let client = DaemonClient::from_data_dir(dir.path()).expect("client");
                let start = start.clone();
                tokio::spawn(async move {
                    start.wait().await;
                    client.health().await
                })
            })
            .collect();
        for call in calls {
            let answer = call.await.expect("task");
            assert_eq!(answer.expect("every client of the burst is answered").profile, "dev");
        }
    }

    #[tokio::test]
    async fn a_health_against_a_socket_that_never_answers_is_not_running_within_its_timeout() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Listening, and never served: the connection is taken, the request
        // is written and nothing comes back.
        let _listener = crate::daemon::socket::bind_socket(&dir.path().join("clawg.sock")).expect("bind");
        write_state(dir.path(), &published(None, true)).expect("write state");
        write_token(dir.path(), "t").expect("write token");
        let c = DaemonClient::from_data_dir(dir.path()).expect("client");

        let started = Instant::now();
        assert!(matches!(c.health().await, Err(ClientError::NotRunning)));
        assert!(started.elapsed() < HEALTH_TIMEOUT * 3, "{:?}", started.elapsed());
    }

    // ── Happy paths ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn health_reports_the_daemons_profile_and_embedder_state() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");

        let h = c.health().await.expect("health");
        assert_eq!(h.status, "ok");
        assert_eq!(h.profile, "dev");
        assert!(h.embedder_ready(), "new_for_test pre-seeds the embedder");
    }

    #[tokio::test]
    async fn save_then_list_then_search_then_patch_then_delete() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");

        let content = "the release train leaves every second tuesday";
        c.save(SaveArgs {
            content: content.into(),
            agent_id: "agent-client".into(),
            project_id: None,
            memory_type: Some("semantic_fact".into()),
            importance: Some(7.0),
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
        })
        .await
        .expect("save");

        // Ingest is asynchronous by contract; poll rather than sleep.
        let mut rows = Vec::new();
        for _ in 0..40 {
            rows = c
                .list(ListArgs { kind: Some("memory".into()), ..Default::default() })
                .await
                .expect("list");
            if !rows.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(rows.len(), 1, "the saved memory must become exactly one row");
        assert_eq!(rows[0].summary, content);
        assert_eq!(rows[0].memory_type.as_deref(), Some("semantic_fact"));
        let id = rows[0].id;

        let hits = c
            .search(SearchArgs {
                query: content.into(),
                agent_id: "agent-client".into(),
                project_id: None,
                limit: Some(5),
                lang: None,
            })
            .await
            .expect("search")
            .hits;
        assert!(hits.iter().any(|h| h.id == id), "the saved row must be retrievable");

        let context = c.context(content, "agent-client", None, None, None).await.expect("context");
        assert_eq!(context.unreviewed, 1, "the one saved memory has not been reviewed yet");

        let synthesis = c.synthesis(&SynthesisArgs::default()).await.expect("synthesis");
        assert_eq!(synthesis.pending.iter().map(|r| r.id).collect::<Vec<_>>(), vec![id]);
        assert_eq!(synthesis.pending_total, 1);

        assert_eq!(c.update("memory", id, Some(9.5), None, None, false, None).await.expect("patch").tier, 1);
        assert_eq!(c.delete("memory", id, None).await.expect("delete").deleted, 1);
    }

    #[test]
    fn search_request_carries_the_relevance_ratio() {
        let args = SearchArgs {
            query: "q".into(),
            agent_id: "a".into(),
            project_id: None,
            limit: Some(5),
            lang: None,
        };
        let body = serde_json::to_value(search_request(&args)).expect("serializes");
        assert_eq!(body["min_relative_score"], serde_json::json!(crate::relevance::RELEVANCE_RATIO));
        assert_eq!(body["query"], "q");
        assert_eq!(body["limit"], 5);
    }

    // ── Error mapping ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_wrong_token_maps_to_unauthorized() {
        let d = start_daemon().await;
        publish(&d);
        write_token(d.dir.path(), "not-the-real-token").expect("write token");
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");

        let err = c
            .list(ListArgs::default())
            .await
            .expect_err("a bad token must not read rows");
        assert!(
            matches!(err, ClientError::Unauthorized),
            "401 must not be flattened into a generic daemon error: {err:?}"
        );
    }

    #[test]
    fn a_save_sends_user_stated_only_when_declared() {
        let args = SaveArgs {
            content: "use Postgres for the ledger".into(),
            agent_id: "a".into(),
            project_id: None,
            memory_type: None,
            importance: None,
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
        };
        let undeclared = serde_json::to_value(&args).unwrap();
        assert!(undeclared.get("user_stated").is_none(), "{undeclared}");
        let declared = serde_json::to_value(SaveArgs { user_stated: true, ..args }).unwrap();
        assert_eq!(declared["user_stated"], true, "{declared}");
    }

    #[tokio::test]
    async fn save_returns_the_ids_the_daemon_reported() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        let args = SaveArgs {
            content: "the client reads back the id".into(),
            agent_id: "a".into(),
            project_id: None,
            memory_type: None,
            importance: None,
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
        };

        let first = c.save(args.clone()).await.expect("save");
        let SaveOutcome::Saved { memories, .. } = &first else { panic!("expected Saved, got {first:?}") };
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].kind, "memory");
        assert_eq!(memories[0].outcome, "inserted");

        let second = c.save(args).await.expect("save");
        assert_eq!(
            second,
            SaveOutcome::Saved {
                memories: vec![SavedMemory { kind: "memory".into(), id: memories[0].id, outcome: "reinforced".into() }],
                redacted: vec![], related: vec![], replaced: vec![], key_dropped: false,
                revision_candidate: None
            }
        );
    }

    #[test]
    fn a_save_error_keeps_the_memories_detail_and_retryable() {
        let body = serde_json::json!({
            "error": "ingest_failed",
            "detail": "store insert failed: disk full",
            "retryable": false,
            "memories": [{ "kind": "memory", "id": 57, "outcome": "inserted" }],
        });
        match daemon_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, Some(body)) {
            ClientError::Daemon { status, message, details } => {
                assert_eq!(status, 500);
                assert_eq!(message, "ingest_failed");
                assert_eq!(details.detail.as_deref(), Some("store insert failed: disk full"));
                assert_eq!(details.retryable, Some(false));
                assert_eq!(
                    details.memories,
                    vec![SavedMemory { kind: "memory".into(), id: 57, outcome: "inserted".into() }]
                );
            }
            other => panic!("expected a daemon error, got {other:?}"),
        }
    }

    #[test]
    fn an_error_body_without_details_still_has_its_message() {
        match daemon_error(reqwest::StatusCode::NOT_FOUND, Some(serde_json::json!({ "error": "gone" }))) {
            ClientError::Daemon { message, details, .. } => {
                assert_eq!(message, "gone");
                assert_eq!(details, ErrorDetails::default());
            }
            other => panic!("expected a daemon error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_daemon_side_error_keeps_its_status_and_message() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");

        let err = c
            .delete("memory", 999_999, None)
            .await
            .expect_err("deleting a missing row must fail");
        match err {
            ClientError::Daemon { status, message, .. } => {
                assert_eq!(status, 404);
                assert!(message.contains("999999"), "message was: {message}");
            }
            other => panic!("expected a daemon error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn judge_refuses_a_row_against_itself() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");

        let err = c.judge(1, 1, "compatible", None, false, None).await.expect_err("a row cannot be judged against itself");

        assert!(matches!(err, ClientError::Daemon { status: 400, .. }), "{err:?}");
    }

    #[tokio::test]
    async fn an_unreachable_address_maps_to_not_running() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_state(
            dir.path(),
            &DaemonState {
                // Port 1 on loopback: nothing binds it, so connecting fails
                // immediately rather than hanging on a timeout.
                pid: 1,
                port: Some(1),
                unix_socket: false,
                profile: "dev".into(),
                version: "0.1.0".into(),
                started_at_ms: 0,
            },
        )
        .expect("write state");
        write_token(dir.path(), "token").expect("write token");
        let c = DaemonClient::from_data_dir(dir.path()).expect("client");

        let err = c.health().await.expect_err("nothing is listening");
        assert!(
            matches!(err, ClientError::NotRunning),
            "a refused connection is 'not running', not a transport bug: {err:?}"
        );
    }

    // ── ensure_running ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn ensure_running_returns_the_live_daemon_without_spawning() {
        let d = start_daemon().await;
        publish(&d);
        let base = d.dir.path().to_path_buf();
        let spawner = FakeSpawner { daemon: start_daemon().await, calls: AtomicUsize::new(0) };

        let c = ensure_running_in(&base, Profile::Dev, true, &spawner, Duration::from_secs(2))
            .await
            .expect("the running daemon must be used");
        assert_eq!(c.health().await.expect("health").status, "ok");
        assert_eq!(
            spawner.calls.load(Ordering::SeqCst),
            0,
            "a daemon that already answers must never be started a second time"
        );
    }

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn autostart_creates_a_private_data_directory() {
        let home = tempfile::tempdir().expect("tempdir");
        let base = home.path().join(".clawg-dev");

        let _ = super::start_daemon(&base, Profile::Dev, &DeadSpawner, Duration::from_millis(10)).await;

        assert_eq!(mode_of(&base), 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn the_serve_log_goes_in_a_private_directory() {
        let home = tempfile::tempdir().expect("tempdir");
        let base = home.path().join(".clawg-dev");
        std::fs::create_dir(&base).expect("data dir");

        let mut child = spawn_serve_at(Path::new("true"), &base, Profile::Dev, None).expect("spawn");
        child.wait().expect("wait");

        assert_eq!(mode_of(&base.join("logs")), 0o700);
    }

    #[tokio::test]
    async fn ensure_running_without_autostart_reports_not_running() {
        let dir = tempfile::tempdir().expect("tempdir");
        let spawner = FakeSpawner { daemon: start_daemon().await, calls: AtomicUsize::new(0) };

        let err = ensure_running_in(
            dir.path(),
            Profile::Dev,
            false,
            &spawner,
            Duration::from_secs(2),
        )
        .await
        .expect_err("no daemon and no autostart");
        assert!(matches!(err, ClientError::NotRunning), "{err:?}");
        assert_eq!(spawner.calls.load(Ordering::SeqCst), 0, "--no-autostart must not spawn");
    }

    #[tokio::test]
    async fn ensure_running_starts_a_daemon_and_then_connects_to_it() {
        // The spawner's daemon publishes into ITS OWN tempdir, which is the
        // directory `ensure_running_in` is pointed at — exactly how a real
        // `clawg serve` publishes into the profile's data directory.
        let daemon = start_daemon().await;
        let base = daemon.dir.path().to_path_buf();
        let spawner = FakeSpawner { daemon, calls: AtomicUsize::new(0) };

        let c = ensure_running_in(&base, Profile::Dev, true, &spawner, Duration::from_secs(5))
            .await
            .expect("autostart must produce a usable client");
        assert_eq!(spawner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(c.health().await.expect("health").status, "ok");
    }

    /// A spawner that silently fails must not hang the bridge forever; the
    /// timeout has to be reported as such, not as "not running".
    #[tokio::test]
    async fn ensure_running_times_out_when_the_daemon_never_appears() {
        let dir = tempfile::tempdir().expect("tempdir");
        let started = Instant::now();
        let err = ensure_running_in(
            dir.path(),
            Profile::Dev,
            true,
            &DeadSpawner,
            Duration::from_millis(300),
        )
        .await
        .expect_err("nothing ever came up");
        assert!(matches!(err, ClientError::StartTimeout(_)), "{err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the timeout must bound the wait, not merely decorate it"
        );
    }

    // ── ensure_running: who holds the store ─────────────────────────────────

    /// A holder that is not a daemon can never answer `/health`, so spawning
    /// one and polling for a minute is a guaranteed wait for nothing: the
    /// spawned `clawg serve` dies on the store lock at once. Fail at once too,
    /// and name the holder so the user knows what to close.
    #[tokio::test]
    async fn a_store_held_by_a_non_daemon_fails_fast_without_spawning() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _holder = StoreLock::acquire(dir.path(), StoreHolder::Eval).expect("holder has it");
        let spawner = CountingDeadSpawner::new();

        let started = Instant::now();
        let err = ensure_running_in(dir.path(), Profile::Dev, true, &spawner, Duration::from_secs(5))
            .await
            .expect_err("the store is held by a non-daemon");

        match err {
            ClientError::StoreHeld(owner) => assert_eq!(owner.holder, StoreHolder::Eval),
            other => panic!("expected StoreHeld, got {other:?}"),
        }
        assert_eq!(spawner.calls(), 0, "a daemon cannot start while a non-daemon holds the store");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "failing fast means not waiting on the timeout: took {:?}",
            started.elapsed()
        );
    }

    /// A daemon that holds the store but has not published `daemon.json` yet is
    /// just a slow start — the exact case the poll loop exists for.
    #[tokio::test]
    async fn a_store_held_by_a_daemon_still_takes_the_autostart_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _daemon = StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("daemon holds it");
        let spawner = CountingDeadSpawner::new();

        let err = ensure_running_in(dir.path(), Profile::Dev, true, &spawner, Duration::from_millis(300))
            .await
            .expect_err("no daemon.json ever appears");
        assert!(matches!(err, ClientError::StartTimeout(_)), "{err:?}");
        assert_eq!(spawner.calls(), 1, "a daemon holder must not short-circuit the spawn");
    }

    /// A crashed holder leaves `memory.owner` behind with the lock already gone;
    /// believing it would refuse the bridge forever.
    #[tokio::test]
    async fn a_stale_owner_record_does_not_block_autostart() {
        let daemon = start_daemon().await;
        let base = daemon.dir.path().to_path_buf();
        let stale = StoreOwner { holder: StoreHolder::Eval, pid: 4242, started_at_ms: 1 };
        std::fs::write(store_owner_file(&base), serde_json::to_string(&stale).expect("json"))
            .expect("write stale record");
        std::fs::write(store_lock_file(&base), b"").expect("unlocked lock file");
        let spawner = FakeSpawner { daemon, calls: AtomicUsize::new(0) };

        let c = ensure_running_in(&base, Profile::Dev, true, &spawner, Duration::from_secs(5))
            .await
            .expect("a dead holder's leftovers must not stop the daemon from starting");
        assert_eq!(spawner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(c.health().await.expect("health").status, "ok");
    }

    /// With `--no-autostart` the useful answer is still "something else has
    /// it", not "no daemon is running", which would send the user to start a
    /// daemon that cannot start.
    #[tokio::test]
    async fn a_held_store_is_reported_even_without_autostart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _holder = StoreLock::acquire(dir.path(), StoreHolder::Eval).expect("holder has it");
        let spawner = CountingDeadSpawner::new();

        let err = ensure_running_in(dir.path(), Profile::Dev, false, &spawner, Duration::from_secs(5))
            .await
            .expect_err("held by a non-daemon");
        assert!(matches!(err, ClientError::StoreHeld(_)), "{err:?}");
        assert_eq!(spawner.calls(), 0);
    }

    #[test]
    fn store_held_names_the_holder_and_says_it_is_not_a_daemon() {
        let err = ClientError::StoreHeld(StoreOwner {
            holder: StoreHolder::Eval,
            pid: 4242,
            started_at_ms: 1,
        });
        assert_eq!(
            err.to_string(),
            "the memory store is held by an evaluation run (pid 4242), not by a clawg daemon; close it and reconnect"
        );
    }

    #[tokio::test]
    async fn shutdown_asks_the_daemon_to_stop() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        c.shutdown().await.expect("shutdown must be accepted");
    }

    // ── Viewer reads ────────────────────────────────────────────────────────

    /// Queues one memory and waits until the ingest worker has stored it.
    pub(crate) async fn save_and_wait(
        c: &DaemonClient,
        content: &str,
        agent: &str,
        project: Option<&str>,
        want_rows: usize,
    ) {
        c.save(SaveArgs {
            content: content.into(),
            agent_id: agent.into(),
            project_id: project.map(str::to_owned),
            memory_type: Some("semantic_fact".into()),
            importance: Some(6.0),
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
        })
        .await
        .expect("save");
        for _ in 0..40 {
            let rows = c.list(ListArgs::default()).await.expect("list");
            if rows.len() >= want_rows {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("ingest never produced {want_rows} row(s) within 2 s");
    }

    #[tokio::test]
    async fn get_reads_one_row() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        save_and_wait(&c, "the staging database lives in eu-west", "agent-get", Some("proj-x"), 1)
            .await;
        let id = c.list(ListArgs::default()).await.expect("list")[0].id;

        let row = c.get("memory", id).await.expect("get");
        assert_eq!(row.id, id);
        assert_eq!(row.kind, "memory");
        assert_eq!(row.summary, "the staging database lives in eu-west");
        assert_eq!(row.agent_id.as_deref(), Some("agent-get"));
        assert_eq!(row.project_id.as_deref(), Some("proj-x"));
        assert_eq!(row.source.as_deref(), Some("explicit"));
        assert_eq!(row.update_count, Some(0));
        assert!(row.created_at.is_some_and(|t| t > 0), "{row:?}");
        assert_eq!(row.deleted_at, None);
    }

    #[tokio::test]
    async fn get_missing_maps_to_daemon_404() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");

        match c.get("memory", 999_999).await.expect_err("no such row") {
            ClientError::Daemon { status, message, .. } => {
                assert_eq!(status, 404);
                assert!(message.contains("999999"), "message was: {message}");
            }
            other => panic!("expected a daemon error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stats_reads_counts() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        save_and_wait(&c, "first note about queues", "agent-a", None, 1).await;
        save_and_wait(&c, "second note about indexes", "agent-b", Some("proj-1"), 2).await;

        let all = c.stats(None, None).await.expect("stats");
        assert_eq!(all.totals.memory, 2);
        assert_eq!(all.totals.deleted, 0);
        assert_eq!(all.by_agent.len(), 2, "{all:?}");
        assert!(
            all.by_project.iter().any(|b| b.key.is_none() && b.count == 1),
            "the row with no project is its own bucket: {all:?}"
        );
        assert!(all.newest_created_at.is_some());

        let one = c.stats(Some("agent-a"), None).await.expect("stats");
        assert_eq!(one.totals.memory, 1);
    }

    #[tokio::test]
    async fn list_forwards_new_filters() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        save_and_wait(&c, "alpha note about caching layers", "agent-a", Some("proj-1"), 1).await;
        save_and_wait(&c, "beta note about retry budgets", "agent-b", Some("proj-1"), 2).await;
        save_and_wait(&c, "gamma note about schema drift", "agent-a", Some("proj-2"), 3).await;

        let by_agent = c
            .list(ListArgs { agent: Some("agent-a".into()), ..Default::default() })
            .await
            .expect("list");
        assert_eq!(by_agent.len(), 2);

        let by_both = c
            .list(ListArgs {
                agent: Some("agent-a".into()),
                project: Some("proj-1".into()),
                ..Default::default()
            })
            .await
            .expect("list");
        assert_eq!(by_both.len(), 1);
        assert_eq!(by_both[0].summary, "alpha note about caching layers");

        let by_text = c
            .list(ListArgs { q: Some("retry".into()), ..Default::default() })
            .await
            .expect("list");
        assert_eq!(by_text.len(), 1);
        assert_eq!(by_text[0].summary, "beta note about retry budgets");

        let by_status = c
            .list(ListArgs { status: Some("disputed".into()), ..Default::default() })
            .await
            .expect("list");
        assert!(by_status.is_empty(), "nothing is disputed: {by_status:?}");
    }

    /// A daemon older than the viewer answers without the new columns; the
    /// bridge and the viewer must both keep reading it.
    #[test]
    fn memory_row_tolerates_missing_provenance_fields() {
        let old = serde_json::json!({
            "id": 7,
            "kind": "memory",
            "memory_type": "semantic_fact",
            "summary": "kept",
            "tier": 2,
            "importance": 5.0,
            "status": "active",
            "access_count": 1
        });
        let row: MemoryRow = serde_json::from_value(old).expect("old shape still parses");
        assert_eq!(row.id, 7);
        assert_eq!(row.source, None);
        assert_eq!(row.created_at, None);
        assert_eq!(row.update_count, None);
        assert_eq!(row.agent_id, None);
    }

    #[test]
    fn a_hit_from_an_old_daemon_reads_as_own() {
        let old = serde_json::json!({
            "id": 3, "kind": "Memory", "content": "x", "memory_type": "semantic_fact", "tier": 3, "score": 0.5
        });
        let hit: Hit = serde_json::from_value(old).expect("old shape still parses");
        assert_eq!(hit.agent_id, None);
    }

    #[test]
    fn a_hit_without_exact_reads_as_inexact() {
        let old = serde_json::json!({
            "id": 3, "kind": "Memory", "content": "x", "memory_type": "semantic_fact", "tier": 3, "score": 0.5
        });
        let hit: Hit = serde_json::from_value(old).expect("old shape still parses");
        assert!(!hit.exact);
        let new = serde_json::json!({
            "id": 3, "kind": "Memory", "content": "x", "memory_type": "semantic_fact", "tier": 3, "score": 1.5,
            "exact": true
        });
        assert!(serde_json::from_value::<Hit>(new).expect("new shape parses").exact);
    }
}
