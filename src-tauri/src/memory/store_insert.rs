// T11-test placeholder — stub to allow T05 compile
// Real implementation added in T11-impl

use crate::memory::chunker::Chunk;
use crate::memory::events::{MemoryEvent, MemorySource};
use crate::memory::MemoryStore;
use crate::models::chat::Role;
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
}
