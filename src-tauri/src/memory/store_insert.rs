// T11-test placeholder — stub to allow T05 compile
// Real implementation added in T11-impl

use crate::memory::chunker::Chunk;
use crate::memory::events::{MemoryEvent, MemorySource};
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
}

impl NewMemoryRecord {
    pub fn from_event_and_chunk(evt: &MemoryEvent, chunk: &Chunk) -> Self {
        // Source-first: Compaction-sourced events always map to SessionSummary,
        // regardless of role, BEFORE the role-based derivation below runs.
        let (memory_type, created_by_type, importance) = if evt.source == MemorySource::Compaction {
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
            let importance = if matches!(memory_type, MemoryType::ToolError) { 6.0 } else { 5.0 };
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
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryStoreError {
    #[error("vector dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("tokio_rusqlite error: {0}")]
    Connection(#[from] tokio_rusqlite::Error),
}

impl MemoryStore {
    pub async fn insert_memory_with_vector(
        &self,
        record: NewMemoryRecord,
        embedding: Vec<f32>,
    ) -> Result<i64, MemoryStoreError> {
        if embedding.len() != 384 {
            return Err(MemoryStoreError::DimensionMismatch {
                expected: 384,
                actual: embedding.len(),
            });
        }
        let bytes = serialize_f32_vec(&embedding);
        let created_at = record.created_at_ms as i64;
        // T06 [IMPL] REQ-06: derive dispute columns in Rust before .call() so all 15
        // params flow through a single c.transaction() — atomic, no partial writes.
        let da_vec: Option<&Vec<i64>> = record.disputed_against.as_ref().filter(|v| !v.is_empty());
        let status: &'static str = if da_vec.is_some() { "disputed" } else { "active" };
        let da_json: Option<String> = da_vec.map(|v| serde_json::to_string(v).expect("Vec<i64> serialises infallibly"));
        let last_disp: Option<i64> = da_vec.map(|_| created_at);
        // T2 [IMPL] REQ: compute the initial tier explicitly instead of relying
        // on the schema DEFAULT — single source of truth shared by every
        // producer (worker.rs, extractor.rs). PROMOTE-ONLY, starts from
        // current=3 (see memory/tier.rs).
        let tier: i64 = crate::memory::tier::compute_tier(3, record.importance, 0, record.memory_type.as_str());

        self.conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, project_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance, tier, \
                      status, disputed_against, last_disputed_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                        },
                        created_at,
                        record.created_by_type.as_str(),
                        record.created_by_ref,
                        created_at,
                        created_at,
                        record.importance,
                        tier,
                        status,
                        da_json,
                        last_disp,
                    ],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                    rusqlite::params![id, bytes],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .map_err(MemoryStoreError::from)
    }
}

/// One aggregated episodic trajectory, ready for atomic row+vector insertion
/// via [`MemoryStore::insert_episodic_trajectory`]. Built (outside this
/// module, Lote 5) from a [`crate::memory::episodic_writer::TrajectoryDraft`]
/// — `tool_calls_json`/`errors_json` are the already-serialized
/// `Vec<String>` JSON arrays (serialization happens before this boundary so
/// this struct stays a thin DB-row mirror, same split as `NewMemoryRecord`).
#[derive(Debug, Clone)]
pub(crate) struct NewEpisodicTrajectoryRecord {
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

impl MemoryStore {
    /// Atomically inserts one row into `episodic_trajectory` plus its matching
    /// `episodic_vectors` row (same rowid), in a single transaction — same
    /// discipline as [`MemoryStore::insert_memory_with_vector`].
    ///
    /// `session_id` is `UNIQUE`: a second insert for the same session fails
    /// with `Err` (surfaced via `MemoryStoreError::Connection`, no panic) and
    /// leaves no partial row in either table — the transaction is dropped
    /// (and rolled back) before `commit()` is reached (S8).
    ///
    /// `tier` is always derived via `compute_tier(3, importance, 0,
    /// "episodic_trajectory")` — never hardcoded (R3).
    pub async fn insert_episodic_trajectory(
        &self,
        record: NewEpisodicTrajectoryRecord,
        embedding: Vec<f32>,
    ) -> Result<i64, MemoryStoreError> {
        if embedding.len() != 384 {
            return Err(MemoryStoreError::DimensionMismatch {
                expected: 384,
                actual: embedding.len(),
            });
        }
        let bytes = serialize_f32_vec(&embedding);
        let created_at = record.created_at_ms as i64;
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

    /// Fire-and-forget RCA persistence (Fase 3, design point 3): a single
    /// UPDATE by `id`, NO `c.transaction()` (single-statement, precedent
    /// `fetch_prior_trajectories`) — the caller (`miner.rs`) always warns and
    /// never propagates `Err` upward, so this stays a plain `.execute()`.
    pub(crate) async fn update_trajectory_root_cause(
        &self,
        trajectory_id: i64,
        root_cause: &str,
    ) -> Result<(), MemoryStoreError> {
        let value = root_cause.to_string(); // own for 'static closure
        self.conn
            .call(move |c| -> rusqlite::Result<()> {
                c.execute(
                    "UPDATE episodic_trajectory SET root_cause = ?1 WHERE id = ?2",
                    rusqlite::params![value, trajectory_id],
                )?;
                Ok(())
            })
            .await
            .map_err(MemoryStoreError::from)
    }
}

/// One candidate procedural skill, ready for atomic dedup-check +
/// INSERT-or-UPDATE via [`MemoryStore::promote_to_procedural_skill`]. Built
/// (in `miner.rs`) from a [`crate::memory::miner::MinedSkillCandidate`] that
/// cleared the recurrence threshold.
#[derive(Debug, Clone)]
pub(crate) struct NewProceduralSkillRecord {
    pub agent_id: String,
    pub project_id: Option<String>,
    pub skill_name: String,
    pub trigger: String,
    /// `serialize_tool_sequence(normalized)` — the ONLY producer of this
    /// value (spec R5), used both as the dedup SELECT key and the
    /// `tool_sequence` column value.
    pub tool_sequence_json: String,
    /// `[current_trajectory_id, ...matched_prior_ids]`.
    pub source_trajectory_ids: Vec<i64>,
    pub importance: f64,
    pub now_ms: u64,
}

/// Result of [`MemoryStore::promote_to_procedural_skill`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromoteOutcome {
    /// A brand-new `procedural_skill` row (+ vector) was inserted; carries its rowid.
    Inserted(i64),
    /// An existing row (same `agent_id` + `tool_sequence`) was updated; carries its id.
    Updated(i64),
    /// No dedup hit and no embedding was available — nothing was written (R9).
    SkippedNoEmbedding,
}

impl MemoryStore {
    /// Prior Golden trajectories eligible as `filter_recurring` inputs (spec R6).
    ///
    /// Returns raw `tool_calls` JSON (normalization happens in `miner.rs`, to
    /// keep this store→miner-free) for rows where `agent_id = ?`, `id !=
    /// exclude_id`, `status = 'active'`, `deleted_at IS NULL`, `success_score
    /// > min_success_score`, ordered by `last_accessed_at DESC`, bounded by
    /// `limit`. Uses `idx_epi_agent_active`.
    pub(crate) async fn fetch_prior_trajectories(
        &self,
        agent_id: &str,
        exclude_id: i64,
        min_success_score: f64,
        limit: i64,
    ) -> Result<Vec<(i64, String)>, MemoryStoreError> {
        let agent_id = agent_id.to_string();
        self.conn
            .call(move |c| -> rusqlite::Result<Vec<(i64, String)>> {
                let mut stmt = c.prepare(
                    "SELECT id, tool_calls FROM episodic_trajectory \
                     WHERE agent_id = ?1 AND id != ?2 AND status = 'active' AND deleted_at IS NULL \
                       AND success_score > ?3 \
                     ORDER BY last_accessed_at DESC LIMIT ?4",
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![agent_id, exclude_id, min_success_score, limit], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
            .map_err(MemoryStoreError::from)
    }

    /// Atomic dedup-check + INSERT-or-UPDATE for a promotable skill candidate
    /// (spec R7/R8/R9), inside a SINGLE `.call()` + `tx.transaction()` —
    /// `tokio_rusqlite` serializes closures on one thread, so the SELECT→branch
    /// is race-free without extra locking.
    ///
    /// - MISS + `Some(embedding)`: INSERT row+vector (`tier` via
    ///   `compute_tier`, `success_count = source_trajectory_ids.len()`) → `Inserted`.
    /// - MISS + `None`: no write → `SkippedNoEmbedding` (R9, new-skill path
    ///   needs a vector).
    /// - HIT: union `stored_ids ∪ source_trajectory_ids` in Rust (preserving
    ///   order, appending only missing ids), `success_count =
    ///   len(unioned)` — the invariant `success_count == len(distinct
    ///   source_trajectory_ids)` (R8 amendment) — no vector needed → `Updated`.
    pub(crate) async fn promote_to_procedural_skill(
        &self,
        rec: NewProceduralSkillRecord,
        embedding: Option<Vec<f32>>,
    ) -> Result<PromoteOutcome, MemoryStoreError> {
        if let Some(emb) = &embedding {
            if emb.len() != 384 {
                return Err(MemoryStoreError::DimensionMismatch { expected: 384, actual: emb.len() });
            }
        }
        let bytes = embedding.as_ref().map(|e| serialize_f32_vec(e));
        let now_ms = rec.now_ms as i64;
        // Rust pre-call — NEVER a hardcoded literal (R7).
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

    // ── T2-test (REQ-2): MemoryType::SessionSummary.as_str() == "session_summary" ──

    #[test]
    fn memory_type_session_summary_as_str() {
        assert_eq!(MemoryType::SessionSummary.as_str(), "session_summary");
    }

    // ── Phase 1 (memoria-3-capas-fase-1) — Task 1.1: new pivotal variants ──

    #[test]
    fn as_str_new_variants() {
        assert_eq!(MemoryType::UserPreference.as_str(), "user_preference");
        assert_eq!(MemoryType::ArchitecturalDecision.as_str(), "architectural_decision");
        assert_eq!(MemoryType::SemanticFact.as_str(), "semantic_fact");
    }

    #[test]
    fn memory_store_error_is_send_sync() {
        fn _s<T: Send + Sync>() {}
        _s::<MemoryStoreError>();
    }

    #[test]
    fn memory_store_error_displays_message() {
        let e = MemoryStoreError::DimensionMismatch { expected: 384, actual: 100 };
        let s = format!("{e}");
        assert!(s.contains("384"), "message should contain 384: {s}");
        assert!(s.contains("100"), "message should contain 100: {s}");
    }

    // ---------- T12-tests: insert_memory_with_vector (REQ-INS-001, REQ-INS-003) ----------

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
        // TODO: R-NEW-7 — connection-close testing deferred
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record("a", None);
        let small_embedding = vec![0.1_f32; 10];
        let err = store.insert_memory_with_vector(record, small_embedding).await.unwrap_err();
        match err {
            MemoryStoreError::DimensionMismatch { expected, actual } => {
                assert_eq!(expected, 384);
                assert_eq!(actual, 10);
            }
            other => panic!("expected DimensionMismatch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn both_rows_present_after_successful_insert() {
        use tokio_rusqlite::Connection;
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

    // ── T03 [TEST] REQ-05: NewMemoryRecord.disputed_against defaults None ─────

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
        };
        let chunk = Chunk { text: "hello".to_string(), token_count: 1, header: None };
        let rec = NewMemoryRecord::from_event_and_chunk(&evt, &chunk);
        assert!(rec.disputed_against.is_none(),
            "from_event_and_chunk must default disputed_against=None");
    }

    // ── T3-test (REQ-3): source-first arm — Compaction source overrides role ──

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
        };
        let chunk = Chunk { text: "delta summary text".to_string(), token_count: 3, header: None };
        let rec = NewMemoryRecord::from_event_and_chunk(&evt, &chunk);
        assert_eq!(rec.memory_type, MemoryType::SessionSummary, "Compaction source must map to SessionSummary");
        assert_eq!(rec.created_by_type, CreatedByType::System, "Compaction source must map to System creator");
        assert_eq!(rec.importance, 6.0, "Compaction source must fix importance at 6.0");
    }

    // ── T05 [TEST] REQ-06: INSERT writes status/disputed_against/last_disputed_at ──

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

    // ── Phase 2 (memoria-3-capas-fase-1) — tier fijo en el INSERT ──────────
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

    // Task 2.1: chat_message imp=5.0 → tier=3 (regression lock).
    #[tokio::test]
    async fn insert_chat_message_importance_5_yields_tier_3() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::ChatMessage, 5.0);
        let tier = insert_and_fetch_tier(&store, record).await;
        assert_eq!(tier, 3, "chat_message imp=5.0 must insert at tier=3");
    }

    // Task 2.2: tool_error imp=6.0 → tier=3 (regression lock).
    #[tokio::test]
    async fn insert_tool_error_importance_6_yields_tier_3() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::ToolError, 6.0);
        let tier = insert_and_fetch_tier(&store, record).await;
        assert_eq!(tier, 3, "tool_error imp=6.0 must insert at tier=3");
    }

    // Task 2.3: session_summary imp=6.0 → tier=2. CORRECTION vs the old SQL
    // `DEFAULT 3` — locked per tier.rs:38-44 (INSERT initial-tier rule), no
    // effect on workspace.md (session_summary is not a workspace producer).
    #[tokio::test]
    async fn insert_session_summary_importance_6_yields_tier_2() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::SessionSummary, 6.0);
        let tier = insert_and_fetch_tier(&store, record).await;
        assert_eq!(tier, 2, "session_summary imp=6.0 must insert at tier=2 (corrected from DEFAULT 3)");
    }

    // Task 2.4: user_preference imp=5.0 → tier=1 (pivotal, type-based rule).
    #[tokio::test]
    async fn insert_user_preference_importance_5_yields_tier_1() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::UserPreference, 5.0);
        let tier = insert_and_fetch_tier(&store, record).await;
        assert_eq!(tier, 1, "user_preference imp=5.0 must insert at tier=1");
    }

    // Task 2.5: architectural_decision imp=5.0 → tier=2 (pivotal, type-based rule).
    #[tokio::test]
    async fn insert_architectural_decision_importance_5_yields_tier_2() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::ArchitecturalDecision, 5.0);
        let tier = insert_and_fetch_tier(&store, record).await;
        assert_eq!(tier, 2, "architectural_decision imp=5.0 must insert at tier=2");
    }

    // Task 2.6: semantic_fact imp=5.0 → tier=3 (no promotion rule, doc 7).
    #[tokio::test]
    async fn insert_semantic_fact_importance_5_yields_tier_3() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::SemanticFact, 5.0);
        let tier = insert_and_fetch_tier(&store, record).await;
        assert_eq!(tier, 3, "semantic_fact imp=5.0 must insert at tier=3 (no promotion rule)");
    }

    // Task 2.7: semantic_fact imp=9.5 → tier=1 (importance>=9 fast-track overrides type).
    #[tokio::test]
    async fn insert_semantic_fact_importance_9_5_yields_tier_1() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record_typed(MemoryType::SemanticFact, 9.5);
        let tier = insert_and_fetch_tier(&store, record).await;
        assert_eq!(tier, 1, "semantic_fact imp=9.5 must insert at tier=1 (importance>=9 override)");
    }

    // ── Lote 3 (memoria-3-capas-fase-2) — insert_episodic_trajectory ────────

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

    // Task 3.1 (S7): valid record + 384-dim embedding → 1 row in
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

    // Task 3.2 (R3): tier==3 via compute_tier(3, 5.0, 0, "episodic_trajectory")
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

    // Task 3.3 (S7): created_by_type=='system', status=='active'.
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

    // ── Lote 2 (trajectory-miner-fase-2) — fetch_prior_trajectories + promote_to_procedural_skill ──

    async fn seed_episodic_raw(
        store: &MemoryStore,
        agent_id: &str,
        session_id: &str,
        status: &str,
        deleted_at: Option<i64>,
        success_score: f64,
        last_accessed_at: i64,
    ) -> i64 {
        let agent_id = agent_id.to_string();
        let session_id = session_id.to_string();
        let status = status.to_string();
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO episodic_trajectory \
                     (scope, agent_id, session_id, task, tool_calls, success_score, \
                      created_at, last_modified_at, last_accessed_at, status, deleted_at) \
                     VALUES ('agent', ?1, ?2, 'task', '[\"a\"]', ?3, 0, 0, ?4, ?5, ?6)",
                    rusqlite::params![agent_id, session_id, success_score, last_accessed_at, status, deleted_at],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    // R6: only active, non-deleted, Golden (score > 0.85), same-agent, non-self
    // rows are returned, ordered by last_accessed_at DESC.
    #[tokio::test]
    async fn fetch_prior_trajectories_filters() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let exclude_id = seed_episodic_raw(&store, "agent-a", "self", "active", None, 0.9, 999).await;
        let golden_low = seed_episodic_raw(&store, "agent-a", "golden-low", "active", None, 0.9, 150).await;
        let golden_high = seed_episodic_raw(&store, "agent-a", "golden-high", "active", None, 0.9, 500).await;
        seed_episodic_raw(&store, "agent-a", "non-golden", "active", None, 0.4, 200).await;
        seed_episodic_raw(&store, "agent-a", "deleted-golden", "active", Some(999), 0.9, 300).await;
        seed_episodic_raw(&store, "agent-b", "other-agent-golden", "active", None, 0.9, 400).await;

        let result = store.fetch_prior_trajectories("agent-a", exclude_id, 0.85, 500).await.unwrap();

        let ids: Vec<i64> = result.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![golden_high, golden_low], "expected only same-agent Golden rows, ordered by last_accessed_at DESC");
    }

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
        let seq = crate::memory::miner::serialize_tool_sequence(&["read".to_string(), "edit".to_string()]);
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
        let seq1 = crate::memory::miner::serialize_tool_sequence(&["read".to_string()]);
        let seq2 = crate::memory::miner::serialize_tool_sequence(&["write".to_string()]);
        let rec1 = make_skill_record("agent-a", &seq1, vec![1], 1000);
        let rec2 = make_skill_record("agent-a", &seq2, vec![2], 1000);

        store.promote_to_procedural_skill(rec1, Some(make_embedding_384())).await.unwrap();
        store.promote_to_procedural_skill(rec2, Some(make_embedding_384())).await.unwrap();

        assert_eq!(count_procedural(&store).await, 2, "distinct sequences must insert 2 separate rows");
    }

    #[tokio::test]
    async fn promote_tier_is_compute_tier() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let seq = crate::memory::miner::serialize_tool_sequence(&["read".to_string()]);
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

    // R9: NEW-skill path requires a vector — embedder absent => 0 rows written,
    // SkippedNoEmbedding, no panic. Unreachable via mine_session_core (the
    // episodic writer gates on the embedder first) — tested here at store level.
    #[tokio::test]
    async fn promote_none_embedding_new_skill_skips() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let seq = crate::memory::miner::serialize_tool_sequence(&["read".to_string()]);
        let rec = make_skill_record("agent-a", &seq, vec![1], 1000);

        let outcome = store.promote_to_procedural_skill(rec, None).await.unwrap();
        assert!(matches!(outcome, PromoteOutcome::SkippedNoEmbedding), "expected SkippedNoEmbedding");
        assert_eq!(count_procedural(&store).await, 0, "no row must be written on the None-embedding new-skill path");
    }

    // R9: dedup-UPDATE path does NOT need a vector — success_count still grows.
    #[tokio::test]
    async fn promote_none_embedding_dedup_updates() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let seq = crate::memory::miner::serialize_tool_sequence(&["read".to_string(), "edit".to_string()]);
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

    // ── Post-verify S1 (R8) — idempotencia pura: todos los ids ya presentes ──

    // R8 scenario "All ids already present → no increment": a re-promotion
    // where EVERY id (current + matched priors) is already in
    // `source_trajectory_ids` must leave `success_count` AND the array
    // untouched — the invariant `success_count == len(distinct ids)` holds
    // with zero appends.
    #[tokio::test]
    async fn promote_all_ids_already_present_no_increment() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let seq = crate::memory::miner::serialize_tool_sequence(&["read".to_string(), "edit".to_string()]);
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

    // Task 3.4 (S8): second insert with the same session_id → Err, no panic,
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

    // ── Phase 1 (trajectory-miner-fase-3) — V4 migration + update_trajectory_root_cause ──

    // Task 1.1: proves V4 applied — SELECT root_cause on a fresh row succeeds
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

    // Task 1.3a: update_trajectory_root_cause targets exactly one row — the
    // sibling row's root_cause must remain untouched (NULL).
    #[tokio::test]
    async fn update_trajectory_root_cause_targets_exactly_one_row() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record1 = make_episodic_record("sess-rca-target", 5.0);
        let record2 = make_episodic_record("sess-rca-sibling", 5.0);
        let id1 = store.insert_episodic_trajectory(record1, make_embedding_384()).await.unwrap();
        let id2 = store.insert_episodic_trajectory(record2, make_embedding_384()).await.unwrap();

        store.update_trajectory_root_cause(id1, "missing_context").await.unwrap();

        let conn = store.conn.clone();
        let (rc1, rc2): (Option<String>, Option<String>) = conn
            .call(move |c| {
                let a: Option<String> =
                    c.query_row("SELECT root_cause FROM episodic_trajectory WHERE id = ?1", rusqlite::params![id1], |r| r.get(0))?;
                let b: Option<String> =
                    c.query_row("SELECT root_cause FROM episodic_trajectory WHERE id = ?1", rusqlite::params![id2], |r| r.get(0))?;
                Ok::<_, rusqlite::Error>((a, b))
            })
            .await
            .unwrap();
        assert_eq!(rc1.as_deref(), Some("missing_context"), "target row must be updated");
        assert!(rc2.is_none(), "sibling row must remain untouched");
    }

    // Task 1.3b: overwrite semantics = last-write-wins — a second UPDATE on
    // the same row replaces the already-set value.
    #[tokio::test]
    async fn update_trajectory_root_cause_overwrite_is_last_write_wins() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_episodic_record("sess-rca-overwrite", 5.0);
        let id = store.insert_episodic_trajectory(record, make_embedding_384()).await.unwrap();

        store.update_trajectory_root_cause(id, "missing_context").await.unwrap();
        store.update_trajectory_root_cause(id, "model_limitation").await.unwrap();

        let conn = store.conn.clone();
        let root_cause: Option<String> = conn
            .call(move |c| {
                c.query_row("SELECT root_cause FROM episodic_trajectory WHERE id = ?1", rusqlite::params![id], |r| {
                    r.get(0)
                })
            })
            .await
            .unwrap();
        assert_eq!(root_cause.as_deref(), Some("model_limitation"), "second UPDATE must overwrite the first (last-write-wins)");
    }
}
