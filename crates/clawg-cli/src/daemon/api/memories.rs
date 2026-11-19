//! The memory routes: save, search, context, list, get, update, move, delete, restore and synthesis.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use clawg_core::memory::admin::{
    delete_memory_core, Deleted, get_memory_core, list_memories_core, ListParams, MemoryAdminRow,
    MemoryPatch, query_has_tokens, restore_memory_core, RestoredRow, RestoreError,
    update_memory_core, UpdatedRow, UpdateError,
};
use clawg_core::memory::bootstrap::EmbedderStatus;
use clawg_core::memory::clock::TimestampMs;
use clawg_core::memory::embedder::{EmbedError, EmbedRole};
use clawg_core::memory::language::{count_saves_in, count_saves_in_projects};
use clawg_core::memory::merge::canonical;
use clawg_core::memory::outbox::OutboxState;
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
use clawg_core::memory::sharing::Reader;
use clawg_core::memory::store_insert::MemoryType;
use clawg_core::memory::synthesis::{
    ConclusionReview, conclusions_to_review_count, LostSource, Synthesis, synthesis_core,
    SynthesisParams, unreviewed_count,
};
use clawg_core::memory::worker::{ChunkOutcome, IngestFailure, IngestReport, SavedChunk};
use clawg_core::memory::{MemoryStatus, RankedHit, read_save_core, SaveText, search_all_memories, SearchScope, StoredStatus, StoredType};
use clawg_core::metrics::sessions::{Activity, UNSPECIFIED};
use clawg_core::metrics::{Recorder, Unit};
use clawg_core::preferences::Lang;
use clawg_core::runtime::{Accepted, AcceptError};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::daemon::lifecycle::now_ms;
use crate::relevance::shown_hits;

use super::sessions::count_empty_search;
use super::{
    ApiError, ApiResult, AppCtx, embed_failure, live_project, named_reader, reader,
    ReaderQuery, ready_embedder,
};

/// Largest `limit` any search will honour. A retrieval request is a vector
/// scan plus a rerank; letting a caller ask for thousands turns one HTTP call
/// into a stall for every other caller sharing this single-threaded store.
pub const MAX_SEARCH_LIMIT: usize = 50;

/// `limit` used when the caller does not say.
pub const DEFAULT_SEARCH_LIMIT: usize = 10;

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

/// Validates a listing request and turns it into the core's parameters.
///
/// Pure, so each refusal is testable without a server. Every refusal is a 400
/// naming what was wrong: a viewer that quietly answered an unfiltered list to
/// a bad filter would present it as the result of that filter.
pub fn list_params_from_query(q: ListQuery) -> ApiResult<ListParams> {
    if let Some(raw) = q.kind.as_deref() {
        parse_kind(raw)?;
    }

    let status = match q.status.as_deref() {
        None => None,
        Some(raw) => Some(MemoryStatus::from_name(raw).ok_or_else(|| {
            ApiError::bad_request(format!(
                "unknown status {raw:?}: accepted values are \"active\", \"disputed\" and \"superseded\""
            ))
        })?),
    };

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
        memory_type: q.memory_type.as_deref().map(StoredType::read),
        agent_id: q.agent,
        project_id: q.project,
        no_project,
        status: status.map(StoredStatus::from),
        origin_project: q.origin_project,
        query: q.q,
        limit: q.limit,
        offset: q.offset,
        deleted,
        ..Default::default()
    })
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
    pub status: StoredStatus,
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
            memory_type: self.memory_type.as_deref().map(StoredType::read),
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
pub(super) async fn save_memory(
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

pub(super) async fn search(
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

/// Builds the 4-block memory prefix for a query.
///
/// The dense half of the query is the raw text: widening it with prior conversation turns needs a conversation
/// and a resolved tokenizer, and a daemon serving stateless HTTP has neither.
/// The lexical half is the raw text in both.
pub(super) async fn context(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<ContextRequest>,
) -> ApiResult<Json<ContextBody>> {
    let budget = check_budget(body.budget)?;
    let lang = check_lang(body.lang.as_deref())?;
    let workspace = app.runtime.workspaces();
    let project_id = live_project(&app, body.project_id.clone()).await?;
    let project_id = project_id.as_deref();
    let query = body.query.as_deref().filter(|q| !q.trim().is_empty());
    // One read of the policy for the passages and the counts alike.
    let caller = reader(&app, &body.agent_id);
    let reach = &caller.reach;

    // Unconditional regen (~2-5 ms). A failure degrades to "no ids were
    // emitted", which can make the prompt repeat a row — a degraded prompt
    // beats a failed request.
    let emitted = workspace
        .regenerate_auto_section(&body.agent_id, project_id, reach, &app.runtime.store)
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
                reach,
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
                reach,
                &emitted,
                budget,
            )
            .await
        }
    }
    .map_err(ApiError::internal)?;

    // Same degradation as the regen above: a reminder that cannot be counted
    // is a reminder skipped, not a failed request.
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

pub(super) async fn synthesis(
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

pub(super) async fn list_memories(
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

/// One row with every column the viewer shows, soft-deleted rows included
/// (they report `deleted_at`). Deliberately not a search: it records no access.
pub(super) async fn get_memory(
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
pub(super) async fn get_save(
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

pub(super) async fn update_memory(
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
pub(super) async fn move_memories(
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

pub(super) async fn delete_memory(
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
pub(super) async fn restore_memory(
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use clawg_core::memory::registry::{Born, resolve_project, ResolveOptions};

    use super::super::MAX_CONTENT_BYTES;
    use super::super::test_support::{
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
        assert_eq!(params.memory_type.as_ref().map(|t| t.as_str()), Some("semantic_fact"));
        assert_eq!(params.agent_id.as_deref(), Some("a1"));
        assert_eq!(params.project_id.as_deref(), Some("p1"));
        assert_eq!(params.status.as_ref().map(|t| t.as_str()), Some("disputed"));
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
