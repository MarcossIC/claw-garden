//! Insert-path types and DB operations for the memory store.
//!
//! Defines `MemoryType`/`CreatedByType` and `NewMemoryRecord`, and the
//! `MemoryStore` methods that write a memory. Every insert writes its row and
//! matching embedding-vector row atomically in the same transaction. The
//! episodic and procedural writers further down exist only for tests.

use crate::memory::chunker::Chunk;
use crate::memory::clock::TimestampMs;
use crate::memory::events::{MemoryEvent, MemorySource};
use crate::memory::vector::to_unit;
use crate::memory::MemoryStore;
use crate::models::chat::Role;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    ChatMessage,
    ToolError,
    CodeBlock,
    SessionSummary,
    UserPreference,
    ArchitecturalDecision,
    SemanticFact,
}

impl MemoryType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ChatMessage => "chat_message",
            Self::ToolError => "tool_error",
            Self::CodeBlock => "code_block",
            Self::SessionSummary => "session_summary",
            Self::UserPreference => "user_preference",
            Self::ArchitecturalDecision => "architectural_decision",
            Self::SemanticFact => "semantic_fact",
        }
    }

    /// Only a type that asserts something can be contradicted. An explicit
    /// `match` rather than `matches!`: adding an 8th variant is then a
    /// compile error here, not a silent `false`.
    pub(crate) fn is_claim_bearing(self) -> bool {
        match self {
            Self::UserPreference | Self::ArchitecturalDecision | Self::SemanticFact => true,
            Self::ChatMessage | Self::ToolError | Self::CodeBlock | Self::SessionSummary => false,
        }
    }

    /// Parses the raw DB string. Unknown or legacy values return `None` —
    /// treated as not eligible for the contradiction type gate rather than
    /// panicking on a value written by a future or foreign schema version.
    pub(crate) fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "chat_message" => Some(Self::ChatMessage),
            "tool_error" => Some(Self::ToolError),
            "code_block" => Some(Self::CodeBlock),
            "session_summary" => Some(Self::SessionSummary),
            "user_preference" => Some(Self::UserPreference),
            "architectural_decision" => Some(Self::ArchitecturalDecision),
            "semantic_fact" => Some(Self::SemanticFact),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatedByType {
    Agent,
    User,
    System,
}

impl CreatedByType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::User => "user",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewMemoryRecord {
    pub scope: &'static str,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub content: String,
    pub memory_type: MemoryType,
    pub source: MemorySource,
    pub created_by_type: CreatedByType,
    pub created_by_ref: String,
    pub importance: f64,
    pub created_at_ms: u64,
    /// IDs of existing memory rows that conflict with this candidate.
    /// When `Some(_)`, the row is inserted with `status='disputed'`.
    /// Set by the contradiction detector in worker.rs before INSERT.
    pub disputed_against: Option<Vec<i64>>,
    /// The `memory` rows this one was drawn from, linked in the same
    /// transaction as the insert.
    pub derived_from: Vec<i64>,
}

/// Neutral importance for a row nothing has said anything special about.
/// Named because three arms below reach for it and one of them is a fallback
/// from an override that failed validation — a bare `5.0` there would read as
/// an arbitrary number rather than "back to the default".
const DEFAULT_IMPORTANCE: f64 = 5.0;

/// Inclusive bounds of the store's importance scale.
///
/// Not a stylistic range: `tier::compute_tier` fast-tracks `importance >= 9.0`
/// to tier 1 and `score.rs` clamps into the same domain, so a client-supplied
/// value outside it would not merely be unusual — it would sit off the end of
/// every curve that reads the column.
const IMPORTANCE_MIN: f64 = 0.0;
const IMPORTANCE_MAX: f64 = 10.0;

impl NewMemoryRecord {
    pub fn from_event_and_chunk(evt: &MemoryEvent, chunk: &Chunk) -> Self {
        // Source-first: Compaction-sourced events always map to SessionSummary,
        // regardless of role, BEFORE the role-based derivation below runs.
        let (memory_type, created_by_type, importance) = if evt.source == MemorySource::Explicit {
            // A deliberately saved memory is the ONE case where the producer
            // knows more than the pipeline can derive: there is no role to read
            // the type off, and the act of saving is itself the statement of
            // importance. Both overrides are therefore honoured — and only
            // here, so a chat turn can never talk its way out of its derivation.
            //
            // The type arrives as a free string (that is what an MCP client can
            // express) but is mapped onto `MemoryType` rather than stored raw:
            // the `memory_type` column is untyped TEXT, so a raw string WOULD
            // persist, but `compute_tier` and the contradiction gate both
            // classify by this enum, and an unrecognised value silently opts
            // the row out of both. Falling back is the honest failure.
            let memory_type = evt
                .memory_type
                .as_deref()
                .and_then(MemoryType::from_db_str)
                .unwrap_or(MemoryType::SemanticFact);
            // `SemanticFact` as the fallback because a user-declared memory
            // with no stated type IS an asserted fact — and, being
            // claim-bearing, it stays eligible for contradiction resolution
            // instead of landing in the inert `ChatMessage` bucket.
            let importance = evt
                .importance
                .filter(|v| !v.is_nan()) // NaN has no place on the scale to clamp it to.
                .map(|v| v.clamp(IMPORTANCE_MIN, IMPORTANCE_MAX))
                .unwrap_or(DEFAULT_IMPORTANCE);
            (memory_type, CreatedByType::User, importance)
        } else if evt.source == MemorySource::Compaction {
            (MemoryType::SessionSummary, CreatedByType::System, 6.0)
        } else {
            let memory_type = match (evt.role.clone(), evt.is_error) {
                (Role::Tool, Some(true)) => MemoryType::ToolError,
                (Role::Tool, _) => MemoryType::ChatMessage,
                (Role::User, _) | (Role::Assistant, _) => MemoryType::ChatMessage,
                (Role::System, _) => MemoryType::ChatMessage,
            };
            let created_by_type = match evt.role {
                Role::User => CreatedByType::User,
                Role::Assistant => CreatedByType::Agent,
                Role::Tool => CreatedByType::System,
                Role::System => CreatedByType::System,
            };
            let importance = if matches!(memory_type, MemoryType::ToolError) {
                6.0
            } else {
                DEFAULT_IMPORTANCE
            };
            (memory_type, created_by_type, importance)
        };
        Self {
            scope: "agent",
            agent_id: Some(evt.agent_id.clone()),
            project_id: evt.project_id.clone(),
            content: chunk.text.clone(),
            memory_type,
            source: evt.source,
            created_by_type,
            created_by_ref: evt.agent_id.clone(),
            importance,
            created_at_ms: evt.timestamp_ms,
            disputed_against: None,
            derived_from: evt.sources().to_vec(),
        }
    }
}

/// Links `memory_id` to each of `sources` inside the caller's transaction. A
/// pair already linked stays as it was, and a row is never linked to itself:
/// a save that reinforces one of its own sources is that source, not
/// something drawn from it.
fn link_sources(
    tx: &rusqlite::Transaction<'_>,
    memory_id: i64,
    sources: &[i64],
    now: TimestampMs,
) -> rusqlite::Result<()> {
    let mut stmt = tx.prepare_cached(
        "INSERT OR IGNORE INTO memory_derivation (memory_id, source_id, created_at) \
         SELECT ?1, ?2, ?3 WHERE ?1 <> ?2",
    )?;
    for source in sources {
        stmt.execute(rusqlite::params![memory_id, source, now])?;
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryStoreError {
    /// Covers every way an embedding fails the shared vector-space invariant
    /// (`memory::vector::to_unit`): wrong dimension, a non-finite component, or
    /// zero/non-normalizable magnitude. One variant because the caller's response
    /// is identical in all three cases — discard before the write, never partial.
    #[error("invalid embedding: {reason}")]
    InvalidEmbedding { reason: String },
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("tokio_rusqlite error: {0}")]
    Connection(#[from] tokio_rusqlite::Error),
}

/// The write-border dedup lookup, as one shared literal.
///
/// Production (`reinforce_if_duplicate`) and `reinforce_lookup_uses_dedup_index`
/// both read THIS string. Each previously held its own copy, so the EXPLAIN
/// test explained a sentence production no longer said: any drift in the real
/// predicate — most importantly the `COALESCE(project_id,'')` that has to match
/// `idx_memory_dedup`'s indexed expression byte-for-byte — moved production off
/// the index while the test kept confirming the old copy still hit it.
const REINFORCE_LOOKUP_SQL: &str = "SELECT id FROM memory \
     WHERE agent_id = ?1 \
       AND COALESCE(project_id, '') = COALESCE(?2, '') \
       AND content_hash = ?3 \
       AND content_hash IS NOT NULL \
       AND deleted_at IS NULL \
       AND status IN ('active', 'disputed') \
     ORDER BY created_at ASC LIMIT 1";

impl MemoryStore {
    /// Reinforces the existing active row matching `(agent_id,
    /// COALESCE(project_id,''), content_hash)`, if any: `access_count += 1`,
    /// `last_accessed_at = MAX(last_accessed_at, now_ms)`, `tier` recomputed
    /// via `compute_tier`. Returns the reinforced row's id, or `None` on a
    /// miss.
    ///
    /// One `conn.call()` holding one `SELECT .optional()` → branch → `UPDATE`
    /// — atomic for that pair, shaped after `promote_to_procedural_skill`
    /// (`store_insert.rs`). `COALESCE(project_id,'')` is repeated
    /// byte-identically against `idx_memory_dedup` so the planner can use it;
    /// `status IN ('active','disputed')` mirrors `SEARCH_SQL_MEMORY` — a
    /// `superseded` row was explicitly replaced and reinforcing it would
    /// resurrect a retired claim. `deleted_at IS NULL` keeps a soft-deleted
    /// row from being silently un-deleted by a resent chunk. `ORDER BY
    /// created_at ASC LIMIT 1` makes the pick deterministic among
    /// pre-dedup legacy duplicates, converging repeated content onto one
    /// canonical (the oldest) row instead of a run-to-run coin flip.
    ///
    /// Fail-open: an error here degrades to today's behaviour (embed+insert
    /// proceeds as if this were a miss) rather than dropping the ingest —
    /// the caller in `worker.rs` treats `Err` the same as `Ok(None)`.
    ///
    /// The lookup-then-insert race (a chunk could be hashed, missed here,
    /// then embedded and inserted as a duplicate row before another chunk
    /// with the same hash finishes its own miss) is closed by
    /// `memory_worker` being the single sequential consumer of one ingest
    /// channel — not by a DB constraint, because `idx_memory_dedup` cannot
    /// be `UNIQUE` (see the migration). `MemoryRuntime::ingest_worker`
    /// is the only spawn site (`rg -n 'ingest_worker\('` confirms this). If the worker is ever parallelized, this guarantee
    /// breaks, and the fix is, in order: (i) a maintenance pass collapsing
    /// legacy duplicate triples, reassigning `superseded_by` /
    /// `disputed_against`; (ii) a migration swapping the index for a real
    /// `UNIQUE` one; (iii) catching `SQLITE_CONSTRAINT_UNIQUE` on insert and
    /// falling back to this method.
    pub(crate) async fn reinforce_if_duplicate(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
        content_hash: &str,
        derived_from: &[i64],
        now_ms: u64,
    ) -> Result<Option<i64>, MemoryStoreError> {
        let agent_id = agent_id.to_string();
        let project_id = project_id.map(str::to_string);
        let content_hash = content_hash.to_string();
        let derived_from = derived_from.to_vec();
        let now = TimestampMs::from_unix_millis_u64(now_ms);

        self.conn
            .call(move |c| -> rusqlite::Result<Option<i64>> {
                let tx = c.transaction()?;

                let existing_id: Option<i64> = tx
                    .query_row(
                        REINFORCE_LOOKUP_SQL,
                        rusqlite::params![agent_id, project_id, content_hash],
                        |r| r.get(0),
                    )
                    .optional()?;

                let Some(id) = existing_id else {
                    return Ok(None);
                };

                tx.execute(
                    "UPDATE memory \
                     SET last_accessed_at = MAX(last_accessed_at, ?2), \
                         access_count     = access_count + 1, \
                         tier = compute_tier(tier, importance, access_count + 1, memory_type) \
                     WHERE id = ?1",
                    rusqlite::params![id, now],
                )?;
                link_sources(&tx, id, &derived_from, now)?;

                tx.commit()?;
                Ok(Some(id))
            })
            .await
            .map_err(MemoryStoreError::from)
    }

    pub async fn insert_memory_with_vector(
        &self,
        record: NewMemoryRecord,
        embedding: Vec<f32>,
    ) -> Result<i64, MemoryStoreError> {
        let unit = to_unit(&embedding)
            .map_err(|e| MemoryStoreError::InvalidEmbedding { reason: e.to_string() })?;
        let bytes = serialize_f32_vec(&unit);
        let created_at = TimestampMs::from_unix_millis_u64(record.created_at_ms);
        // Derive dispute columns in Rust before .call() so all 15
        // params flow through a single c.transaction() — atomic, no partial writes.
        let da_vec: Option<&Vec<i64>> = record.disputed_against.as_ref().filter(|v| !v.is_empty());
        let status: &'static str = if da_vec.is_some() { "disputed" } else { "active" };
        let da_json: Option<String> = da_vec.map(|v| serde_json::to_string(v).expect("Vec<i64> serialises infallibly"));
        let last_disp: Option<TimestampMs> = da_vec.map(|_| created_at);
        // Compute the initial tier explicitly instead of relying
        // on the schema DEFAULT — the single source of truth for every
        // producer. PROMOTE-ONLY, starts from current=3 (see memory/tier.rs).
        let tier: i64 = crate::memory::tier::compute_tier(3, record.importance, 0, record.memory_type.as_str());
        // Same normalization the write-border dedup gate hashes against
        // (worker.rs, before embed) — writing it here too keeps every row
        // dedup-eligible regardless of which producer inserted it.
        let content_hash = crate::memory::dedup::content_hash(&record.content);

        self.conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, project_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, access_count, importance, tier, \
                      status, disputed_against, last_disputed_at, content_hash) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        record.scope,
                        record.agent_id,
                        record.project_id,
                        record.content,
                        record.memory_type.as_str(),
                        match record.source {
                            MemorySource::Chat => "chat",
                            MemorySource::Compaction => "compaction",
                            MemorySource::CodeFile => "code_file",
                            MemorySource::Explicit => "explicit",
                        },
                        created_at,
                        record.created_by_type.as_str(),
                        record.created_by_ref,
                        created_at,
                        created_at,
                        // Written explicitly as 1, not left at the schema's DEFAULT 0: this
                        // row's creation is itself the first occurrence of this content, so
                        // a chunk resent once (one REINFORCE, `access_count + 1`) reads as
                        // "seen twice" — 2, not 1 — matching what REINFORCE's `+= 1` promises.
                        1,
                        record.importance,
                        tier,
                        status,
                        da_json,
                        last_disp,
                        content_hash,
                    ],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                    rusqlite::params![id, bytes],
                )?;
                link_sources(&tx, id, &record.derived_from, created_at)?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .map_err(MemoryStoreError::from)
    }
}

// Nothing in production writes `episodic_trajectory` or `procedural_skill`
// any more; the desktop app that did is gone, but the rows it wrote are still
// searched, listed and re-embedded. The two writers below stay, for tests
// only, so fixtures land in those tables exactly as the app put them there.

/// One episodic trajectory, inserted with its vector by
/// [`MemoryStore::insert_episodic_trajectory`]. `tool_calls_json` and
/// `errors_json` are already-serialized `Vec<String>` JSON arrays.
#[cfg(any(test, feature = "testing"))]
#[derive(Debug, Clone)]
pub struct NewEpisodicTrajectoryRecord {
    pub scope: &'static str,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub session_id: String,
    pub task: String,
    pub tool_calls_json: String,
    pub errors_json: String,
    pub resolution: Option<String>,
    pub success_score: f64,
    pub created_by_type: CreatedByType,
    pub created_by_ref: String,
    pub importance: f64,
    pub created_at_ms: u64,
}

#[cfg(any(test, feature = "testing"))]
impl MemoryStore {
    /// Atomically inserts one row into `episodic_trajectory` plus its matching
    /// `episodic_vectors` row (same rowid), in a single transaction — same
    /// discipline as [`MemoryStore::insert_memory_with_vector`].
    ///
    /// `session_id` is `UNIQUE`: a second insert for the same session fails
    /// with `Err` (surfaced via `MemoryStoreError::Connection`, no panic) and
    /// leaves no partial row in either table — the transaction is dropped
    /// (and rolled back) before `commit()` is reached.
    ///
    /// `tier` is always derived via `compute_tier(3, importance, 0,
    /// "episodic_trajectory")` — never hardcoded.
    pub async fn insert_episodic_trajectory(
        &self,
        record: NewEpisodicTrajectoryRecord,
        embedding: Vec<f32>,
    ) -> Result<i64, MemoryStoreError> {
        let unit = to_unit(&embedding)
            .map_err(|e| MemoryStoreError::InvalidEmbedding { reason: e.to_string() })?;
        let bytes = serialize_f32_vec(&unit);
        let created_at = TimestampMs::from_unix_millis_u64(record.created_at_ms);
        let tier: i64 =
            crate::memory::tier::compute_tier(3, record.importance, 0, "episodic_trajectory");

        self.conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO episodic_trajectory \
                     (scope, agent_id, project_id, session_id, task, tool_calls, \
                      errors_encountered, resolution, success_score, created_at, \
                      created_by_type, created_by_ref, last_modified_at, \
                      last_accessed_at, importance, tier, status) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        record.scope,
                        record.agent_id,
                        record.project_id,
                        record.session_id,
                        record.task,
                        record.tool_calls_json,
                        record.errors_json,
                        record.resolution,
                        record.success_score,
                        created_at,
                        record.created_by_type.as_str(),
                        record.created_by_ref,
                        created_at,
                        created_at,
                        record.importance,
                        tier,
                        "active",
                    ],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO episodic_vectors(rowid, embedding) VALUES (?, ?)",
                    rusqlite::params![id, bytes],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .map_err(MemoryStoreError::from)
    }
}

/// One procedural skill, inserted (or merged into the row with the same
/// `agent_id` and `tool_sequence`) by [`MemoryStore::promote_to_procedural_skill`].
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct NewProceduralSkillRecord {
    pub agent_id: String,
    pub project_id: Option<String>,
    pub skill_name: String,
    pub trigger: String,
    /// JSON array of tool names: the `tool_sequence` column, and with
    /// `agent_id` the key two promotions of the same skill merge on.
    pub tool_sequence_json: String,
    /// The trajectories the skill was mined from.
    pub source_trajectory_ids: Vec<i64>,
    pub importance: f64,
    pub now_ms: u64,
}

/// Result of [`MemoryStore::promote_to_procedural_skill`].
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromoteOutcome {
    /// A brand-new `procedural_skill` row (+ vector) was inserted; carries its rowid.
    Inserted(i64),
    /// An existing row (same `agent_id` + `tool_sequence`) was updated; carries its id.
    Updated(i64),
    /// No dedup hit and no embedding was available — nothing was written.
    SkippedNoEmbedding,
}

#[cfg(test)]
impl MemoryStore {
    /// Atomic dedup-check + INSERT-or-UPDATE for a promotable skill candidate,
    /// inside a SINGLE `.call()` + `tx.transaction()` —
    /// `tokio_rusqlite` serializes closures on one thread, so the SELECT→branch
    /// is race-free without extra locking.
    ///
    /// - MISS + `Some(embedding)`: INSERT row+vector (`tier` via
    ///   `compute_tier`, `success_count = source_trajectory_ids.len()`) → `Inserted`.
    /// - MISS + `None`: no write → `SkippedNoEmbedding` (new-skill path
    ///   needs a vector).
    /// - HIT: union `stored_ids ∪ source_trajectory_ids` in Rust (preserving
    ///   order, appending only missing ids), `success_count =
    ///   len(unioned)` — the invariant `success_count == len(distinct
    ///   source_trajectory_ids)` — no vector needed → `Updated`.
    pub(crate) async fn promote_to_procedural_skill(
        &self,
        rec: NewProceduralSkillRecord,
        embedding: Option<Vec<f32>>,
    ) -> Result<PromoteOutcome, MemoryStoreError> {
        let embedding = embedding
            .map(|e| to_unit(&e).map_err(|err| MemoryStoreError::InvalidEmbedding { reason: err.to_string() }))
            .transpose()?;
        let bytes = embedding.as_ref().map(|e| serialize_f32_vec(e));
        let now_ms = TimestampMs::from_unix_millis_u64(rec.now_ms);
        // Rust pre-call — NEVER a hardcoded literal.
        let tier: i64 = crate::memory::tier::compute_tier(3, rec.importance, 0, "procedural_skill");

        self.conn
            .call(move |c| -> rusqlite::Result<PromoteOutcome> {
                let tx = c.transaction()?;

                let existing: Option<(i64, String)> = tx
                    .query_row(
                        "SELECT id, source_trajectory_ids FROM procedural_skill \
                         WHERE agent_id = ?1 AND tool_sequence = ?2",
                        rusqlite::params![rec.agent_id, rec.tool_sequence_json],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;

                let outcome = match existing {
                    Some((id, stored_json)) => {
                        let mut unioned: Vec<i64> = serde_json::from_str(&stored_json).unwrap_or_default();
                        for candidate_id in &rec.source_trajectory_ids {
                            if !unioned.contains(candidate_id) {
                                unioned.push(*candidate_id);
                            }
                        }
                        let unioned_json =
                            serde_json::to_string(&unioned).expect("Vec<i64> serialises infallibly");
                        tx.execute(
                            "UPDATE procedural_skill SET source_trajectory_ids = ?1, success_count = ?2, \
                             last_modified_at = ?3, update_count = update_count + 1 WHERE id = ?4",
                            rusqlite::params![unioned_json, unioned.len() as i64, now_ms, id],
                        )?;
                        PromoteOutcome::Updated(id)
                    }
                    None => {
                        let Some(bytes) = bytes else {
                            return Ok(PromoteOutcome::SkippedNoEmbedding);
                        };
                        let ids_json = serde_json::to_string(&rec.source_trajectory_ids)
                            .expect("Vec<i64> serialises infallibly");
                        tx.execute(
                            "INSERT INTO procedural_skill \
                             (scope, agent_id, project_id, skill_name, trigger, tool_sequence, \
                              success_count, promoted_at, source_trajectory_ids, created_at, \
                              created_by_type, created_by_ref, last_modified_at, last_accessed_at, \
                              importance, tier, status) \
                             VALUES ('agent', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, \
                                     'system', ?1, ?9, ?9, ?10, ?11, 'active')",
                            rusqlite::params![
                                rec.agent_id,
                                rec.project_id,
                                rec.skill_name,
                                rec.trigger,
                                rec.tool_sequence_json,
                                rec.source_trajectory_ids.len() as i64,
                                now_ms,
                                ids_json,
                                now_ms,
                                rec.importance,
                                tier,
                            ],
                        )?;
                        let id = tx.last_insert_rowid();
                        tx.execute(
                            "INSERT INTO procedural_vectors(rowid, embedding) VALUES (?, ?)",
                            rusqlite::params![id, bytes],
                        )?;
                        PromoteOutcome::Inserted(id)
                    }
                };

                tx.commit()?;
                Ok(outcome)
            })
            .await
            .map_err(MemoryStoreError::from)
    }
}

fn serialize_f32_vec(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for &f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_type_session_summary_as_str() {
        assert_eq!(MemoryType::SessionSummary.as_str(), "session_summary");
    }

    // ── New pivotal variants ──

    #[test]
    fn as_str_new_variants() {
        assert_eq!(MemoryType::UserPreference.as_str(), "user_preference");
        assert_eq!(MemoryType::ArchitecturalDecision.as_str(), "architectural_decision");
        assert_eq!(MemoryType::SemanticFact.as_str(), "semantic_fact");
    }

    // ── is_claim_bearing — truth table over all 7 variants ─────────────────

    #[test]
    fn is_claim_bearing_true_for_user_preference() {
        assert!(MemoryType::UserPreference.is_claim_bearing());
    }

    #[test]
    fn is_claim_bearing_true_for_architectural_decision() {
        assert!(MemoryType::ArchitecturalDecision.is_claim_bearing());
    }

    #[test]
    fn is_claim_bearing_true_for_semantic_fact() {
        assert!(MemoryType::SemanticFact.is_claim_bearing());
    }

    #[test]
    fn is_claim_bearing_false_for_chat_message() {
        assert!(!MemoryType::ChatMessage.is_claim_bearing());
    }

    #[test]
    fn is_claim_bearing_false_for_tool_error() {
        assert!(!MemoryType::ToolError.is_claim_bearing());
    }

    #[test]
    fn is_claim_bearing_false_for_code_block() {
        assert!(!MemoryType::CodeBlock.is_claim_bearing());
    }

    #[test]
    fn is_claim_bearing_false_for_session_summary() {
        assert!(!MemoryType::SessionSummary.is_claim_bearing());
    }

    // ── from_db_str — round-trips as_str(), unknown → None ─────────────────

    #[test]
    fn from_db_str_round_trips_every_variant() {
        let variants = [
            MemoryType::ChatMessage,
            MemoryType::ToolError,
            MemoryType::CodeBlock,
            MemoryType::SessionSummary,
            MemoryType::UserPreference,
            MemoryType::ArchitecturalDecision,
            MemoryType::SemanticFact,
        ];
        for v in variants {
            assert_eq!(MemoryType::from_db_str(v.as_str()), Some(v), "round-trip failed for {v:?}");
        }
    }

    #[test]
    fn from_db_str_unknown_value_returns_none() {
        assert_eq!(MemoryType::from_db_str("episode"), None);
        assert_eq!(MemoryType::from_db_str("not_a_real_type"), None);
        assert_eq!(MemoryType::from_db_str(""), None);
    }

    #[test]
    fn memory_store_error_is_send_sync() {
        fn _s<T: Send + Sync>() {}
        _s::<MemoryStoreError>();
    }

    #[test]
    fn memory_store_error_displays_message() {
        let e = MemoryStoreError::InvalidEmbedding {
            reason: "expected 384-dim vector, got 100".to_string(),
        };
        let s = format!("{e}");
        assert!(s.contains("384"), "message should contain 384: {s}");
        assert!(s.contains("100"), "message should contain 100: {s}");
    }

    // ---------- insert_memory_with_vector ----------

    fn make_record(agent_id: &str, project_id: Option<String>) -> NewMemoryRecord {
        NewMemoryRecord {
            scope: "agent",
            agent_id: Some(agent_id.to_string()),
            project_id,
            content: "test content".to_string(),
            memory_type: MemoryType::ChatMessage,
            source: crate::memory::events::MemorySource::Chat,
            created_by_type: CreatedByType::User,
            created_by_ref: agent_id.to_string(),
            importance: 5.0,
            created_at_ms: 1_000_000,
            disputed_against: None,
            derived_from: Vec::new(),
        }
    }

    fn make_embedding_384() -> Vec<f32> {
        vec![0.1_f32; 384]
    }

    #[tokio::test]
    async fn insert_memory_with_vector_returns_rowid() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record("a", None);
        let embedding = make_embedding_384();
        let id = store.insert_memory_with_vector(record, embedding).await.unwrap();
        assert!(id > 0, "expected positive rowid, got {id}");
    }

    #[tokio::test]
    async fn insert_memory_with_vector_returns_err_on_wrong_dimension() {
        // Proxy test for connection error: pass a 10-dim embedding and assert
        // Err(MemoryStoreError::DimensionMismatch { expected: 384, actual: 10 })
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record("a", None);
        let small_embedding = vec![0.1_f32; 10];
        let err = store.insert_memory_with_vector(record, small_embedding).await.unwrap_err();
        match err {
            MemoryStoreError::InvalidEmbedding { reason } => {
                assert!(reason.contains("384"), "reason should mention 384: {reason}");
                assert!(reason.contains("10"), "reason should mention 10: {reason}");
            }
            other => panic!("expected InvalidEmbedding, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn both_rows_present_after_successful_insert() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record("a", None);
        let embedding = make_embedding_384();
        let id = store.insert_memory_with_vector(record, embedding).await.unwrap();

        let conn = store.conn.clone();
        let (mem_count, vec_count) = conn
            .call(move |c| -> Result<(i64, i64), rusqlite::Error> {
                let mc: i64 = c.query_row(
                    "SELECT COUNT(*) FROM memory WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )?;
                let vc: i64 = c.query_row(
                    "SELECT COUNT(*) FROM memory_vectors WHERE rowid = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )?;
                Ok((mc, vc))
            })
            .await
            .unwrap();

        assert_eq!(mem_count, 1, "expected 1 row in memory for id={id}");
        assert_eq!(vec_count, 1, "expected 1 row in memory_vectors for rowid={id}");
    }

    #[tokio::test]
    async fn vector_insert_failure_leaves_no_memory_row() {
        // Wrong dimension → DimensionMismatch → NO rows inserted anywhere.
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record("a", None);
        let bad_embedding = vec![0.1_f32; 10];
        let _ = store.insert_memory_with_vector(record, bad_embedding).await;

        let conn = store.conn.clone();
        let count: i64 = conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM memory", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(count, 0, "expected 0 rows in memory after dimension error");
    }

    #[tokio::test]
    async fn insert_agent_scope_no_project_passes_check() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record("a", None);
        let embedding = make_embedding_384();
        let result = store.insert_memory_with_vector(record, embedding).await;
        assert!(result.is_ok(), "expected Ok for agent scope with no project, got {:?}", result.err());
    }

    #[tokio::test]
    async fn insert_agent_scope_with_project_passes_check() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record("a", Some("proj-1".to_string()));
        let embedding = make_embedding_384();
        let result = store.insert_memory_with_vector(record, embedding).await;
        assert!(result.is_ok(), "expected Ok for agent scope with project, got {:?}", result.err());
    }

    // ── NewMemoryRecord.disputed_against defaults None ─────

    #[test]
    fn new_memory_record_disputed_against_defaults_none_in_make_record() {
        let rec = make_record("a", None);
        assert!(rec.disputed_against.is_none(), "make_record must default disputed_against=None");
    }

    #[test]
    fn new_memory_record_from_event_and_chunk_disputed_against_none() {
        use crate::memory::events::{MemoryEvent, MemorySource};
        use crate::memory::chunker::Chunk;
        use crate::models::chat::Role;
        let evt = MemoryEvent {
            agent_id: "agent-x".to_string(),
            project_id: None,
            role: Role::User,
            content: "hello".to_string(),
            is_error: None,
            source: MemorySource::Chat,
            timestamp_ms: 0,
            memory_type: None,
            importance: None,
            derived_from: Vec::new(),
        };
        let chunk = Chunk { text: "hello".to_string(), token_count: 1, header: None };
        let rec = NewMemoryRecord::from_event_and_chunk(&evt, &chunk);
        assert!(rec.disputed_against.is_none(),
            "from_event_and_chunk must default disputed_against=None");
    }

    // ── Source-first arm — Compaction source overrides role ──

    #[test]
    fn from_event_and_chunk_compaction_source_yields_session_summary_system_6() {
        use crate::memory::events::{MemoryEvent, MemorySource};
        use crate::memory::chunker::Chunk;
        use crate::models::chat::Role;
        let evt = MemoryEvent {
            agent_id: "agent-x".to_string(),
            project_id: None,
            role: Role::System,
            content: "delta summary text".to_string(),
            is_error: None,
            source: MemorySource::Compaction,
            timestamp_ms: 0,
            memory_type: None,
            importance: None,
            derived_from: Vec::new(),
        };
        let chunk = Chunk { text: "delta summary text".to_string(), token_count: 3, header: None };
        let rec = NewMemoryRecord::from_event_and_chunk(&evt, &chunk);
        assert_eq!(rec.memory_type, MemoryType::SessionSummary, "Compaction source must map to SessionSummary");
        assert_eq!(rec.created_by_type, CreatedByType::System, "Compaction source must map to System creator");
        assert_eq!(rec.importance, 6.0, "Compaction source must fix importance at 6.0");
    }

    // ── Explicit source — client overrides ─────────────────────────────

    /// Builds an `Explicit` event with the two optional overrides, so each
    /// case below reads as the one thing it varies.
    fn explicit_event(
        memory_type: Option<&str>,
        importance: Option<f64>,
    ) -> crate::memory::events::MemoryEvent {
        crate::memory::events::MemoryEvent {
            agent_id: "agent-x".to_string(),
            project_id: None,
            role: crate::models::chat::Role::User,
            content: "the team uses conventional commits".to_string(),
            is_error: None,
            source: MemorySource::Explicit,
            timestamp_ms: 0,
            memory_type: memory_type.map(str::to_owned),
            importance,
            derived_from: Vec::new(),
        }
    }

    fn chunk_of(text: &str) -> crate::memory::chunker::Chunk {
        crate::memory::chunker::Chunk { text: text.to_string(), token_count: 4, header: None }
    }

    #[test]
    fn explicit_source_honours_both_overrides() {
        let evt = explicit_event(Some("architectural_decision"), Some(8.5));
        let rec = NewMemoryRecord::from_event_and_chunk(&evt, &chunk_of("x"));
        assert_eq!(rec.memory_type, MemoryType::ArchitecturalDecision);
        assert_eq!(rec.importance, 8.5);
        assert_eq!(
            rec.created_by_type,
            CreatedByType::User,
            "an explicitly saved memory is authored by the user, whatever role the \
             transport stamped on the event"
        );
        assert_eq!(rec.source, MemorySource::Explicit);
    }

    /// A client that saves a bare string declares a fact. `SemanticFact` is
    /// the only variant that means exactly that, and it is claim-bearing, so
    /// the contradiction detector can still adjudicate it later.
    #[test]
    fn explicit_source_without_a_type_override_falls_back_to_semantic_fact() {
        let evt = explicit_event(None, None);
        let rec = NewMemoryRecord::from_event_and_chunk(&evt, &chunk_of("x"));
        assert_eq!(rec.memory_type, MemoryType::SemanticFact);
        assert_eq!(rec.importance, 5.0, "no override keeps the store's neutral default");
    }

    /// An unrecognised type string must not be able to write a row the tier
    /// UDF and the contradiction gate cannot classify — it degrades to the
    /// same fallback as no override at all.
    #[test]
    fn explicit_source_with_an_unknown_type_override_falls_back_to_semantic_fact() {
        let evt = explicit_event(Some("not_a_real_type"), None);
        let rec = NewMemoryRecord::from_event_and_chunk(&evt, &chunk_of("x"));
        assert_eq!(rec.memory_type, MemoryType::SemanticFact);
    }

    /// The store's importance domain is 0–10 (`tier.rs` fast-tracks >= 9.0 and
    /// `score.rs` clamps into that range); a client cannot escape it.
    #[test]
    fn explicit_importance_override_is_clamped_to_the_stores_domain() {
        let hi = NewMemoryRecord::from_event_and_chunk(
            &explicit_event(None, Some(42.0)),
            &chunk_of("x"),
        );
        assert_eq!(hi.importance, 10.0, "above-range importance clamps to the maximum");

        let lo = NewMemoryRecord::from_event_and_chunk(
            &explicit_event(None, Some(-3.0)),
            &chunk_of("x"),
        );
        assert_eq!(lo.importance, 0.0, "below-range importance clamps to the minimum");
    }

    /// NaN is not a point on the scale, so it cannot be clamped into one —
    /// it falls back to the default rather than poisoning every comparison
    /// the score formula makes against it.
    #[test]
    fn explicit_importance_override_rejects_nan() {
        let rec = NewMemoryRecord::from_event_and_chunk(
            &explicit_event(None, Some(f64::NAN)),
            &chunk_of("x"),
        );
        assert_eq!(rec.importance, 5.0);
    }

    /// The overrides are scoped to `Explicit` by design: a chat turn's type is
    /// derived from its role, and that derivation must not become overridable.
    #[test]
    fn chat_and_compaction_sources_ignore_the_overrides_entirely() {
        let mut chat = explicit_event(Some("architectural_decision"), Some(9.9));
        chat.source = MemorySource::Chat;
        let rec = NewMemoryRecord::from_event_and_chunk(&chat, &chunk_of("x"));
        assert_eq!(rec.memory_type, MemoryType::ChatMessage);
        assert_eq!(rec.importance, 5.0);
        assert_eq!(rec.created_by_type, CreatedByType::User);

        let mut compaction = explicit_event(Some("architectural_decision"), Some(9.9));
        compaction.source = MemorySource::Compaction;
        let rec = NewMemoryRecord::from_event_and_chunk(&compaction, &chunk_of("x"));
        assert_eq!(rec.memory_type, MemoryType::SessionSummary);
        assert_eq!(rec.importance, 6.0);
        assert_eq!(rec.created_by_type, CreatedByType::System);
    }

    #[tokio::test]
    async fn explicit_source_is_stored_as_the_string_explicit() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let evt = explicit_event(Some("user_preference"), Some(9.5));
        let record = NewMemoryRecord::from_event_and_chunk(&evt, &chunk_of("x"));
        let id = store
            .insert_memory_with_vector(record, make_embedding_384())
            .await
            .expect("insert");
        let source: String = store
            .conn
            .call(move |c| -> Result<String, rusqlite::Error> {
                c.query_row("SELECT source FROM memory WHERE id = ?1", [id], |r| r.get(0))
            })
            .await
            .expect("select");
        assert_eq!(source, "explicit");
    }

    // ── INSERT writes status/disputed_against/last_disputed_at ──

    #[tokio::test]
    async fn insert_with_no_dispute_writes_active_status() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record("a", None); // disputed_against = None
        let embedding = make_embedding_384();
        let id = store.insert_memory_with_vector(record, embedding).await.unwrap();

        let conn = store.conn.clone();
        let (status, da, lda): (String, Option<String>, Option<i64>) = conn
            .call(move |c| {
                c.query_row(
                    "SELECT status, disputed_against, last_disputed_at FROM memory WHERE id = ?1",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
            })
            .await
            .unwrap();

        assert_eq!(status, "active", "non-disputed row must have status='active'");
        assert!(da.is_none(), "non-disputed row must have disputed_against=NULL");
        assert!(lda.is_none(), "non-disputed row must have last_disputed_at=NULL");
    }

    #[tokio::test]
    async fn insert_with_dispute_writes_disputed_status_and_json() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let mut record = make_record("a", None);
        record.disputed_against = Some(vec![12i64, 34]);
        let embedding = make_embedding_384();
        let id = store.insert_memory_with_vector(record, embedding).await.unwrap();

        let conn = store.conn.clone();
        let (status, da, lda): (String, Option<String>, Option<i64>) = conn
            .call(move |c| {
                c.query_row(
                    "SELECT status, disputed_against, last_disputed_at FROM memory WHERE id = ?1",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
            })
            .await
            .unwrap();

        assert_eq!(status, "disputed", "disputed row must have status='disputed'");
        let da_val = da.expect("disputed_against must not be NULL when dispute set");
        let parsed: Vec<i64> = serde_json::from_str(&da_val)
            .expect("disputed_against must be valid JSON array");
        assert_eq!(parsed, vec![12i64, 34], "disputed_against must match the input ids");
        let created_at_ms = 1_000_000i64; // from make_record
        assert_eq!(lda, Some(created_at_ms), "last_disputed_at must equal created_at when disputed");
    }

    #[tokio::test]
    async fn insert_with_dispute_is_atomic_single_transaction() {
        // Verify both memory and memory_vectors rows exist after disputed insert
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let mut record = make_record("a", None);
        record.disputed_against = Some(vec![99i64]);
        let embedding = make_embedding_384();
        let id = store.insert_memory_with_vector(record, embedding).await.unwrap();

        let conn = store.conn.clone();
        let (mem_count, vec_count): (i64, i64) = conn
            .call(move |c| {
                let mc: i64 = c.query_row(
                    "SELECT COUNT(*) FROM memory WHERE id = ?1",
                    rusqlite::params![id], |r| r.get(0))?;
                let vc: i64 = c.query_row(
                    "SELECT COUNT(*) FROM memory_vectors WHERE rowid = ?1",
                    rusqlite::params![id], |r| r.get(0))?;
                Ok::<_, rusqlite::Error>((mc, vc))
            })
            .await
            .unwrap();

        assert_eq!(mem_count, 1, "memory row must exist");
        assert_eq!(vec_count, 1, "memory_vectors row must exist");
    }

    // ── tier fijo en el INSERT ──────────
    // compute_tier(3, importance, 0, memory_type.as_str()) must be written to
    // the `tier` column instead of relying on the schema DEFAULT.

    fn make_record_typed(memory_type: MemoryType, importance: f64) -> NewMemoryRecord {
        let mut rec = make_record("agent-tier", None);
        rec.memory_type = memory_type;
        rec.importance = importance;
        rec
    }

    async fn insert_and_fetch_tier(store: &crate::memory::db::MemoryStore, record: NewMemoryRecord) -> i64 {
        let embedding = make_embedding_384();
        let id = store.insert_memory_with_vector(record, embedding).await.unwrap();
        let conn = store.conn.clone();
        conn.call(move |c| {
            c.query_row(
                "SELECT tier FROM memory WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
        })
        .await
        .unwrap()
    }

    // chat_message imp=5.0 → tier=3 (regression lock).
    #[tokio::test]
    async fn insert_chat_message_importance_5_yields_tier_3() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::ChatMessage, 5.0);
        let tier = insert_and_fetch_tier(&store, record).await;
        assert_eq!(tier, 3, "chat_message imp=5.0 must insert at tier=3");
    }

    // tool_error imp=6.0 → tier=3 (regression lock).
    #[tokio::test]
    async fn insert_tool_error_importance_6_yields_tier_3() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::ToolError, 6.0);
        let tier = insert_and_fetch_tier(&store, record).await;
        assert_eq!(tier, 3, "tool_error imp=6.0 must insert at tier=3");
    }

    // session_summary imp=6.0 → tier=2, not the SQL `DEFAULT 3`: the initial
    // tier follows the INSERT initial-tier rule in tier.rs, with no effect on
    // workspace.md (session_summary is not a workspace producer).
    #[tokio::test]
    async fn insert_session_summary_importance_6_yields_tier_2() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::SessionSummary, 6.0);
        let tier = insert_and_fetch_tier(&store, record).await;
        assert_eq!(tier, 2, "session_summary imp=6.0 must insert at tier=2 (corrected from DEFAULT 3)");
    }

    // user_preference imp=5.0 → tier=1 (pivotal, type-based rule).
    #[tokio::test]
    async fn insert_user_preference_importance_5_yields_tier_1() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::UserPreference, 5.0);
        let tier = insert_and_fetch_tier(&store, record).await;
        assert_eq!(tier, 1, "user_preference imp=5.0 must insert at tier=1");
    }

    // architectural_decision imp=5.0 → tier=2 (pivotal, type-based rule).
    #[tokio::test]
    async fn insert_architectural_decision_importance_5_yields_tier_2() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::ArchitecturalDecision, 5.0);
        let tier = insert_and_fetch_tier(&store, record).await;
        assert_eq!(tier, 2, "architectural_decision imp=5.0 must insert at tier=2");
    }

    // semantic_fact imp=5.0 → tier=3 (no promotion rule).
    #[tokio::test]
    async fn insert_semantic_fact_importance_5_yields_tier_3() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::SemanticFact, 5.0);
        let tier = insert_and_fetch_tier(&store, record).await;
        assert_eq!(tier, 3, "semantic_fact imp=5.0 must insert at tier=3 (no promotion rule)");
    }

    // semantic_fact imp=9.5 → tier=1 (importance>=9 fast-track overrides type).
    #[tokio::test]
    async fn insert_semantic_fact_importance_9_5_yields_tier_1() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::SemanticFact, 9.5);
        let tier = insert_and_fetch_tier(&store, record).await;
        assert_eq!(tier, 1, "semantic_fact imp=9.5 must insert at tier=1 (importance>=9 override)");
    }

    // ── insert_episodic_trajectory ──────────────────────────────────────────

    fn make_episodic_record(session_id: &str, importance: f64) -> NewEpisodicTrajectoryRecord {
        NewEpisodicTrajectoryRecord {
            scope: "agent",
            agent_id: Some("agent-epi".to_string()),
            project_id: None,
            session_id: session_id.to_string(),
            task: "fix the failing test".to_string(),
            tool_calls_json: serde_json::to_string(&vec!["read_file", "write_file"]).unwrap(),
            errors_json: serde_json::to_string::<Vec<String>>(&vec![]).unwrap(),
            resolution: Some("done".to_string()),
            success_score: 0.9,
            created_by_type: CreatedByType::System,
            created_by_ref: "agent-epi".to_string(),
            importance,
            created_at_ms: 1_000_000,
        }
    }

    // valid record + 384-dim embedding → 1 row in
    // episodic_trajectory + 1 row in episodic_vectors, same rowid, same tx.
    #[tokio::test]
    async fn insert_episodic_trajectory_writes_row_and_vector_same_rowid() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_episodic_record("sess-epi-1", 5.0);
        let embedding = make_embedding_384();
        let id = store.insert_episodic_trajectory(record, embedding).await.unwrap();
        assert!(id > 0, "expected positive rowid, got {id}");

        let conn = store.conn.clone();
        let (epi_count, vec_count): (i64, i64) = conn
            .call(move |c| {
                let ec: i64 = c.query_row(
                    "SELECT COUNT(*) FROM episodic_trajectory WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )?;
                let vc: i64 = c.query_row(
                    "SELECT COUNT(*) FROM episodic_vectors WHERE rowid = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )?;
                Ok::<_, rusqlite::Error>((ec, vc))
            })
            .await
            .unwrap();
        assert_eq!(epi_count, 1, "expected 1 row in episodic_trajectory for id={id}");
        assert_eq!(vec_count, 1, "expected 1 row in episodic_vectors for rowid={id}");
    }

    // tier==3 via compute_tier(3, 5.0, 0, "episodic_trajectory")
    // invoked explicitly — never hardcoded. Cross-checked with an
    // importance>=9.0 record, which the SAME compute_tier call promotes to
    // tier=1 (proves the column is NOT hardcoded to a literal `3`).
    #[tokio::test]
    async fn insert_episodic_trajectory_importance_5_yields_tier_3_via_compute_tier() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_episodic_record("sess-epi-2", 5.0);
        let embedding = make_embedding_384();
        let id = store.insert_episodic_trajectory(record, embedding).await.unwrap();

        let conn = store.conn.clone();
        let tier: i64 = conn
            .call(move |c| {
                c.query_row(
                    "SELECT tier FROM episodic_trajectory WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(tier, 3, "importance=5.0 must insert at tier=3 (no promotion rule)");
    }

    #[tokio::test]
    async fn insert_episodic_trajectory_high_importance_is_not_hardcoded_to_tier_3() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_episodic_record("sess-epi-2b", 9.5);
        let embedding = make_embedding_384();
        let id = store.insert_episodic_trajectory(record, embedding).await.unwrap();

        let conn = store.conn.clone();
        let tier: i64 = conn
            .call(move |c| {
                c.query_row(
                    "SELECT tier FROM episodic_trajectory WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(tier, 1, "importance=9.5 fast-track must override the tier=3 default, proving compute_tier is invoked (not hardcoded)");
    }

    // created_by_type=='system', status=='active'.
    #[tokio::test]
    async fn insert_episodic_trajectory_sets_system_creator_and_active_status() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_episodic_record("sess-epi-3", 5.0);
        let embedding = make_embedding_384();
        let id = store.insert_episodic_trajectory(record, embedding).await.unwrap();

        let conn = store.conn.clone();
        let (created_by_type, status): (String, String) = conn
            .call(move |c| {
                c.query_row(
                    "SELECT created_by_type, status FROM episodic_trajectory WHERE id = ?1",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!(created_by_type, "system");
        assert_eq!(status, "active");
    }

    // ── promote_to_procedural_skill ──

    fn make_skill_record(agent_id: &str, seq_json: &str, ids: Vec<i64>, now_ms: u64) -> NewProceduralSkillRecord {
        NewProceduralSkillRecord {
            agent_id: agent_id.to_string(),
            project_id: None,
            skill_name: "Fix login flow".to_string(),
            trigger: "when login fails".to_string(),
            tool_sequence_json: seq_json.to_string(),
            source_trajectory_ids: ids,
            importance: 8.0,
            now_ms,
        }
    }

    async fn count_procedural(store: &MemoryStore) -> i64 {
        store.conn.call(|c| c.query_row("SELECT COUNT(*) FROM procedural_skill", [], |r| r.get(0))).await.unwrap()
    }

    #[tokio::test]
    async fn promote_inserts_row_and_vector_same_rowid() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let seq = r#"["read","edit"]"#.to_string();
        let rec = make_skill_record("agent-a", &seq, vec![42], 1000);
        let embedding = make_embedding_384();

        let outcome = store.promote_to_procedural_skill(rec, Some(embedding)).await.unwrap();
        let id = match outcome {
            PromoteOutcome::Inserted(id) => id,
            other => panic!("expected Inserted, got {other:?}"),
        };
        assert!(id > 0);

        let (skill_count, vec_count): (i64, i64) = store
            .conn
            .call(move |c| {
                let sc: i64 =
                    c.query_row("SELECT COUNT(*) FROM procedural_skill WHERE id = ?1", rusqlite::params![id], |r| r.get(0))?;
                let vc: i64 = c.query_row(
                    "SELECT COUNT(*) FROM procedural_vectors WHERE rowid = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )?;
                Ok::<_, rusqlite::Error>((sc, vc))
            })
            .await
            .unwrap();
        assert_eq!(skill_count, 1, "expected 1 row in procedural_skill for id={id}");
        assert_eq!(vec_count, 1, "expected 1 row in procedural_vectors for rowid={id}");
    }

    #[tokio::test]
    async fn promote_distinct_sequence_inserts_second_row() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let seq1 = r#"["read"]"#.to_string();
        let seq2 = r#"["write"]"#.to_string();
        let rec1 = make_skill_record("agent-a", &seq1, vec![1], 1000);
        let rec2 = make_skill_record("agent-a", &seq2, vec![2], 1000);

        store.promote_to_procedural_skill(rec1, Some(make_embedding_384())).await.unwrap();
        store.promote_to_procedural_skill(rec2, Some(make_embedding_384())).await.unwrap();

        assert_eq!(count_procedural(&store).await, 2, "distinct sequences must insert 2 separate rows");
    }

    #[tokio::test]
    async fn promote_tier_is_compute_tier() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let seq = r#"["read"]"#.to_string();
        let rec = make_skill_record("agent-a", &seq, vec![1], 1000);

        let outcome = store.promote_to_procedural_skill(rec, Some(make_embedding_384())).await.unwrap();
        let id = match outcome {
            PromoteOutcome::Inserted(id) => id,
            other => panic!("expected Inserted, got {other:?}"),
        };

        let tier: i64 = store
            .conn
            .call(move |c| c.query_row("SELECT tier FROM procedural_skill WHERE id = ?1", rusqlite::params![id], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(
            tier,
            crate::memory::tier::compute_tier(3, 8.0, 0, "procedural_skill"),
            "tier must be produced by compute_tier, never hardcoded"
        );
    }

    // NEW-skill path requires a vector — embedder absent => 0 rows written,
    // SkippedNoEmbedding, no panic.
    #[tokio::test]
    async fn promote_none_embedding_new_skill_skips() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let seq = r#"["read"]"#.to_string();
        let rec = make_skill_record("agent-a", &seq, vec![1], 1000);

        let outcome = store.promote_to_procedural_skill(rec, None).await.unwrap();
        assert!(matches!(outcome, PromoteOutcome::SkippedNoEmbedding), "expected SkippedNoEmbedding");
        assert_eq!(count_procedural(&store).await, 0, "no row must be written on the None-embedding new-skill path");
    }

    // dedup-UPDATE path does NOT need a vector — success_count still grows.
    #[tokio::test]
    async fn promote_none_embedding_dedup_updates() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let seq = r#"["read","edit"]"#.to_string();
        let rec1 = make_skill_record("agent-a", &seq, vec![42, 7], 1000);
        store.promote_to_procedural_skill(rec1, Some(make_embedding_384())).await.unwrap();

        let rec2 = make_skill_record("agent-a", &seq, vec![55, 42, 7], 2000);
        let outcome = store.promote_to_procedural_skill(rec2, None).await.unwrap();
        let id = match outcome {
            PromoteOutcome::Updated(id) => id,
            other => panic!("expected Updated, got {other:?}"),
        };

        let success_count: i64 = store
            .conn
            .call(move |c| {
                c.query_row("SELECT success_count FROM procedural_skill WHERE id = ?1", rusqlite::params![id], |r| r.get(0))
            })
            .await
            .unwrap();
        assert_eq!(success_count, 3, "only the new id (55) must increment success_count, invariant == len(distinct ids)");
        assert_eq!(count_procedural(&store).await, 1, "dedup hit must NOT insert a second row");
    }

    // ── Idempotencia pura: todos los ids ya presentes ──

    // "All ids already present → no increment": a re-promotion
    // where EVERY id (current + matched priors) is already in
    // `source_trajectory_ids` must leave `success_count` AND the array
    // untouched — the invariant `success_count == len(distinct ids)` holds
    // with zero appends.
    #[tokio::test]
    async fn promote_all_ids_already_present_no_increment() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let seq = r#"["read","edit"]"#.to_string();
        let rec1 = make_skill_record("agent-a", &seq, vec![42, 7, 55], 1000);
        store.promote_to_procedural_skill(rec1, Some(make_embedding_384())).await.unwrap();

        // Same ids, different order — a pure re-match with nothing new.
        let rec2 = make_skill_record("agent-a", &seq, vec![55, 42, 7], 2000);
        let outcome = store.promote_to_procedural_skill(rec2, None).await.unwrap();
        let id = match outcome {
            PromoteOutcome::Updated(id) => id,
            other => panic!("expected Updated, got {other:?}"),
        };

        let (success_count, source_ids_json): (i64, String) = store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT success_count, source_trajectory_ids FROM procedural_skill WHERE id = ?1",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!(success_count, 3, "no new ids appended ⇒ success_count must stay 3");
        let source_ids: Vec<i64> = serde_json::from_str(&source_ids_json).unwrap();
        assert_eq!(source_ids, vec![42, 7, 55], "source_trajectory_ids must be byte-for-byte intact (order preserved)");
        assert_eq!(count_procedural(&store).await, 1, "still exactly 1 row");
    }

    // Second insert with the same session_id → Err, no panic,
    // no partial row in either table.
    #[tokio::test]
    async fn insert_episodic_trajectory_duplicate_session_id_fails_without_partial_rows() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record1 = make_episodic_record("sess-epi-dup", 5.0);
        let embedding1 = make_embedding_384();
        let id1 = store.insert_episodic_trajectory(record1, embedding1).await.unwrap();

        let record2 = make_episodic_record("sess-epi-dup", 5.0);
        let embedding2 = make_embedding_384();
        let result2 = store.insert_episodic_trajectory(record2, embedding2).await;
        assert!(result2.is_err(), "second insert with same session_id must fail");

        let conn = store.conn.clone();
        let (epi_count, vec_count): (i64, i64) = conn
            .call(|c| {
                let ec: i64 =
                    c.query_row("SELECT COUNT(*) FROM episodic_trajectory", [], |r| r.get(0))?;
                let vc: i64 =
                    c.query_row("SELECT COUNT(*) FROM episodic_vectors", [], |r| r.get(0))?;
                Ok::<_, rusqlite::Error>((ec, vc))
            })
            .await
            .unwrap();
        assert_eq!(epi_count, 1, "only the first insert's row must survive, got {epi_count}");
        assert_eq!(vec_count, 1, "only the first insert's vector must survive, got {vec_count}");
        assert!(id1 > 0);
    }

    // ── Embedding contract: both edges normalize to unit length ──

    /// Reads the raw `memory_vectors.embedding` blob back as `f32`s — little-endian,
    /// matching `serialize_f32_vec`'s encoding.
    async fn read_back_embedding(store: &MemoryStore, id: i64) -> Vec<f32> {
        let bytes: Vec<u8> = store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT embedding FROM memory_vectors WHERE rowid = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    #[tokio::test]
    async fn insert_normalizes_to_unit_length() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record("a", None);
        // Non-unit on purpose: ||v|| = sqrt(384 * 0.01) ≈ 1.9596.
        let embedding = make_embedding_384();
        let id = store.insert_memory_with_vector(record, embedding).await.unwrap();

        let stored = read_back_embedding(&store, id).await;
        let norm: f32 = stored.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "stored vector must have unit norm, got {norm}");
    }

    #[tokio::test]
    async fn insert_writes_content_hash_matching_dedup_content_hash() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record("a", None); // content = "test content"
        let embedding = make_embedding_384();
        let id = store.insert_memory_with_vector(record, embedding).await.unwrap();

        let stored_hash: Option<String> = store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT content_hash FROM memory WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(
            stored_hash.as_deref(),
            Some(crate::memory::dedup::content_hash("test content").as_str()),
            "insert must write content_hash = dedup::content_hash(content)"
        );
    }

    #[tokio::test]
    async fn zero_embedding_is_rejected_and_writes_nothing() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record("a", None);
        let zero_embedding = vec![0.0_f32; 384];
        let result = store.insert_memory_with_vector(record, zero_embedding).await;
        assert!(result.is_err(), "zero-magnitude embedding must be rejected, got {result:?}");

        let count: i64 = store
            .conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM memory", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(count, 0, "a rejected embedding must leave the row unwritten too — never a memory row without a vector row");
    }

    // ── V4 migration ──

    // Proves V4 applied — SELECT root_cause on a fresh row succeeds
    // and defaults to NULL (SQLite ADD COLUMN is nullable by default).
    #[tokio::test]
    async fn root_cause_column_exists_and_defaults_null() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_episodic_record("sess-rca-column", 5.0);
        let embedding = make_embedding_384();
        let id = store.insert_episodic_trajectory(record, embedding).await.unwrap();

        let conn = store.conn.clone();
        let root_cause: Option<String> = conn
            .call(move |c| {
                c.query_row("SELECT root_cause FROM episodic_trajectory WHERE id = ?1", rusqlite::params![id], |r| {
                    r.get(0)
                })
            })
            .await
            .unwrap();
        assert!(root_cause.is_none(), "root_cause must default to NULL for a never-classified row");
    }

    // ── reinforce_if_duplicate ───────────────────────────────────────────────

    /// Raw-inserts a `memory` row with the given `content_hash`, `status`, and
    /// `deleted_at`, bypassing `insert_memory_with_vector` (which does not yet
    /// bind `content_hash`). Returns the new row id.
    async fn seed_hashed_row(
        store: &MemoryStore,
        agent_id: &str,
        project_id: Option<&str>,
        content_hash: &str,
        status: &str,
        deleted_at: Option<i64>,
        created_at: i64,
        access_count: i64,
    ) -> i64 {
        let agent_id = agent_id.to_string();
        let project_id = project_id.map(str::to_string);
        let content_hash = content_hash.to_string();
        let status = status.to_string();
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, project_id, content, memory_type, source, content_hash, \
                      created_at, created_by_type, created_by_ref, last_modified_at, \
                      last_accessed_at, access_count, importance, tier, status, deleted_at) \
                     VALUES ('agent', ?1, ?2, 'dup content', 'chat_message', 'chat', ?3, \
                      ?4, 'user', ?1, ?4, ?4, ?5, 5.0, 3, ?6, ?7)",
                    rusqlite::params![agent_id, project_id, content_hash, created_at, access_count, status, deleted_at],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    async fn fetch_access_count_and_tier(store: &MemoryStore, id: i64) -> (i64, i64) {
        store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT access_count, tier FROM memory WHERE id = ?1",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn reinforce_if_duplicate_hit_increments_access_count_and_recomputes_tier() {
        let store = MemoryStore::new_for_test().await.unwrap();
        // architectural_decision at tier=2, access_count=2 — a 3rd access
        // promotes to tier=1 via compute_tier's flush re-eval rule.
        let hash = "hash-reinforce-1";
        let id = seed_hashed_row(&store, "agent-r1", None, hash, "active", None, 1_000, 2).await;
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET memory_type='architectural_decision' WHERE id = ?1", [id]))
            .await
            .unwrap();

        let result = store.reinforce_if_duplicate("agent-r1", None, hash, &[], 2_000).await.unwrap();
        assert_eq!(result, Some(id), "a matching active row must be reinforced");

        let (access_count, tier) = fetch_access_count_and_tier(&store, id).await;
        assert_eq!(access_count, 3, "access_count must increment by 1");
        assert_eq!(tier, 1, "tier must be recomputed via compute_tier (flush re-eval: access_count>=3)");
    }

    #[tokio::test]
    async fn reinforce_if_duplicate_miss_returns_none() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let result = store.reinforce_if_duplicate("agent-r2", None, "no-such-hash", &[], 1_000).await.unwrap();
        assert_eq!(result, None, "no matching row must return None");
    }

    #[tokio::test]
    async fn reinforce_if_duplicate_soft_deleted_row_returns_none() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let hash = "hash-reinforce-deleted";
        seed_hashed_row(&store, "agent-r3", None, hash, "active", Some(500), 1_000, 0).await;

        let result = store.reinforce_if_duplicate("agent-r3", None, hash, &[], 2_000).await.unwrap();
        assert_eq!(result, None, "a soft-deleted row must never be reinforced");
    }

    #[tokio::test]
    async fn reinforce_if_duplicate_superseded_row_returns_none() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let hash = "hash-reinforce-superseded";
        seed_hashed_row(&store, "agent-r4", None, hash, "superseded", None, 1_000, 0).await;

        let result = store.reinforce_if_duplicate("agent-r4", None, hash, &[], 2_000).await.unwrap();
        assert_eq!(result, None, "a superseded row must never be reinforced");
    }

    #[tokio::test]
    async fn reinforce_if_duplicate_two_project_ids_are_independent() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let hash = "hash-reinforce-scoped";
        let id_p1 = seed_hashed_row(&store, "agent-r5", Some("proj-1"), hash, "active", None, 1_000, 0).await;
        let id_p2 = seed_hashed_row(&store, "agent-r5", Some("proj-2"), hash, "active", None, 1_000, 0).await;
        assert_ne!(id_p1, id_p2);

        let hit_p1 = store.reinforce_if_duplicate("agent-r5", Some("proj-1"), hash, &[], 2_000).await.unwrap();
        assert_eq!(hit_p1, Some(id_p1), "proj-1 lookup must reinforce only the proj-1 row");

        let hit_p2 = store.reinforce_if_duplicate("agent-r5", Some("proj-2"), hash, &[], 2_000).await.unwrap();
        assert_eq!(hit_p2, Some(id_p2), "proj-2 lookup must reinforce only the proj-2 row");

        let (ac_p1, _) = fetch_access_count_and_tier(&store, id_p1).await;
        let (ac_p2, _) = fetch_access_count_and_tier(&store, id_p2).await;
        assert_eq!(ac_p1, 1, "proj-1 row reinforced exactly once");
        assert_eq!(ac_p2, 1, "proj-2 row reinforced exactly once");
    }

    #[tokio::test]
    async fn reinforce_if_duplicate_null_project_ids_match_each_other() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let hash = "hash-reinforce-null-proj";
        let id = seed_hashed_row(&store, "agent-r6", None, hash, "active", None, 1_000, 0).await;

        let result = store.reinforce_if_duplicate("agent-r6", None, hash, &[], 2_000).await.unwrap();
        assert_eq!(result, Some(id), "NULL project_id on both sides must match via COALESCE");
    }

    /// `agent_id` is the outermost column of `idx_memory_dedup` and the first
    /// predicate of the lookup, but every other test in this block seeds and
    /// queries under ONE agent in its own fresh in-memory store — so dropping
    /// the `agent_id = ?1` clause entirely left the whole suite green
    /// (mutation-verified). Identical text from two agents is the ordinary
    /// case, not an exotic one: without this clause agent-b's chunk reinforces
    /// agent-a's row, agent-b silently never gets the memory, and agent-a's
    /// access_count (hence its tier) is inflated by traffic that was never its
    /// own. Two agents, same hash, same store — the isolation is the assertion.
    #[tokio::test]
    async fn reinforce_if_duplicate_other_agents_row_is_not_reinforced() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let hash = "hash-reinforce-cross-agent";
        let id_a = seed_hashed_row(&store, "agent-a", None, hash, "active", None, 1_000, 0).await;

        let miss = store.reinforce_if_duplicate("agent-b", None, hash, &[], 2_000).await.unwrap();
        assert_eq!(
            miss, None,
            "agent-b must not reinforce agent-a's row, even on an identical content hash"
        );

        let (ac_a, _) = fetch_access_count_and_tier(&store, id_a).await;
        assert_eq!(ac_a, 0, "agent-a's access_count must be untouched by agent-b's lookup");

        // And the same call under the owning agent still hits — proving the
        // miss above came from the agent scope, not from a broken fixture.
        let hit = store.reinforce_if_duplicate("agent-a", None, hash, &[], 2_000).await.unwrap();
        assert_eq!(hit, Some(id_a), "the owning agent must still reinforce its own row");
    }

    /// `idx_memory_dedup` indexes `COALESCE(project_id, '')`, so a NULL
    /// project_id and an empty-string one are the SAME dedup key by
    /// construction — the lookup repeats that COALESCE byte-identically to
    /// stay on the index. `..._null_project_ids_match_each_other` only ever
    /// exercises NULL-vs-NULL, and `..._two_project_ids_are_independent` only
    /// non-empty-vs-non-empty, so nothing pinned the one pair the COALESCE
    /// exists for: swapping the predicate to `project_id IS ?2` — which
    /// splits NULL from `''` AND moves production off the indexed expression
    /// — left the whole suite green (mutation-verified).
    #[tokio::test]
    async fn reinforce_if_duplicate_empty_string_project_id_matches_null() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let hash = "hash-reinforce-empty-proj";
        let id = seed_hashed_row(&store, "agent-r8", Some(""), hash, "active", None, 1_000, 0).await;

        let hit = store.reinforce_if_duplicate("agent-r8", None, hash, &[], 2_000).await.unwrap();
        assert_eq!(
            hit,
            Some(id),
            "a project_id='' row and a project_id=NULL lookup are one dedup key under COALESCE"
        );
    }

    #[tokio::test]
    async fn reinforce_if_duplicate_picks_oldest_among_legacy_duplicates() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let hash = "hash-reinforce-legacy-dup";
        let older = seed_hashed_row(&store, "agent-r7", None, hash, "active", None, 1_000, 0).await;
        let _newer = seed_hashed_row(&store, "agent-r7", None, hash, "active", None, 2_000, 0).await;

        let result = store.reinforce_if_duplicate("agent-r7", None, hash, &[], 3_000).await.unwrap();
        assert_eq!(result, Some(older), "must deterministically pick the oldest row among legacy duplicates");
    }

    /// `insert_memory_with_vector` writes `access_count = 1` (see that
    /// function's own comment: the row's creation is itself the first
    /// occurrence of its content), and `reinforce_if_duplicate` recomputes
    /// tier from `access_count + 1` on every hit. `compute_tier`'s rule for
    /// `architectural_decision` / `session_summary` fires at `access_count >=
    /// 3`, so starting from 1 that threshold is crossed by the SECOND
    /// reinforcement, not the third — pinned here so a change to either
    /// number drifts loudly instead of silently.
    #[tokio::test]
    async fn architectural_decision_promotes_to_tier_1_after_two_reinforcements() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::ArchitecturalDecision, 5.0);
        let content = record.content.clone();
        let agent_id = record.agent_id.clone().unwrap();
        let id = store
            .insert_memory_with_vector(record, make_embedding_384())
            .await
            .unwrap();
        let hash = crate::memory::dedup::content_hash(&content);

        let (access_count, tier) = fetch_access_count_and_tier(&store, id).await;
        assert_eq!(access_count, 1, "insert must write access_count = 1");
        assert_eq!(tier, 2, "architectural_decision inserts at tier 2, below the promotion rule");

        store.reinforce_if_duplicate(&agent_id, None, &hash, &[], 2_000).await.unwrap();
        let (access_count, tier) = fetch_access_count_and_tier(&store, id).await;
        assert_eq!(access_count, 2);
        assert_eq!(tier, 2, "one reinforcement is not enough to promote");

        store.reinforce_if_duplicate(&agent_id, None, &hash, &[], 3_000).await.unwrap();
        let (access_count, tier) = fetch_access_count_and_tier(&store, id).await;
        assert_eq!(access_count, 3);
        assert_eq!(tier, 1, "the second reinforcement crosses access_count >= 3 and promotes to tier 1");
    }

    /// The planner must actually use `idx_memory_dedup` for the lookup —
    /// this is measured, not assumed (the index is on a COALESCE expression). A `SCAN memory` in the plan means the
    /// index is not usable for this exact predicate shape and is flagged
    /// here as an open risk rather than silently accepted.
    ///
    /// Explains `REINFORCE_LOOKUP_SQL` itself, never a transcription of it:
    /// the previous version of this test carried its own copy of the SELECT,
    /// so production's predicate could drift off the index shape while this
    /// kept passing against the old text.
    #[tokio::test]
    async fn reinforce_lookup_uses_dedup_index() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let plan: Vec<String> = store
            .conn
            .call(|c| -> rusqlite::Result<Vec<String>> {
                let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {REINFORCE_LOOKUP_SQL}"))?;
                let rows = stmt
                    .query_map(rusqlite::params!["a", Option::<String>::None, "h"], |r| {
                        r.get::<_, String>(3) // EXPLAIN QUERY PLAN's `detail` column
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
            .unwrap();
        let plan_text = plan.join(" | ");
        assert!(
            plan_text.contains("idx_memory_dedup"),
            "expected the planner to use idx_memory_dedup, got plan: {plan_text}"
        );
    }
}
