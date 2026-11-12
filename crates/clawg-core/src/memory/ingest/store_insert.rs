//! Insert-path types and DB operations for the memory store.
//!
//! Defines `NewMemoryRecord` and the `MemoryStore` methods that write a
//! memory. Every insert writes its row and matching embedding-vector row
//! atomically in the same transaction. The episodic and procedural writers
//! further down exist only for tests.

use crate::memory::chunker::Chunk;
use crate::memory::clock::TimestampMs;
use crate::memory::contradiction::Candidate;
use crate::memory::events::{MemoryEvent, MemorySource};
use crate::memory::language;
use crate::memory::vector::to_unit;
use crate::memory::outbox::{apply_note, OutboxNote};
use crate::memory::provenance::{is_stated, seal};
pub use crate::memory::record::{MemoryType, UnknownMemoryType};
pub(crate) use crate::memory::record::CreatedByType;
use crate::memory::search::{reach_predicate, reader_bind};
use crate::memory::sharing::Reader;
use crate::memory::MemoryStore;
use crate::models::chat::Role;
use rusqlite::OptionalExtension;

#[derive(Debug, Clone)]
pub(crate) struct NewMemoryRecord {
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
    /// The existing rows the contradiction detector found close to this one,
    /// set in worker.rs before INSERT. Each becomes a `memory_conflict` pair;
    /// an escalated one is also named in `disputed_against`, and makes the
    /// row `disputed`.
    pub candidates: Vec<Candidate>,
    /// The `memory` rows this one was drawn from, linked in the same
    /// transaction as the insert.
    pub derived_from: Vec<i64>,
    /// The topic key of the save this row is a piece of (`topic.rs`).
    pub topic_key: Option<String>,
    /// The host session the save came from, when the daemon knew it.
    pub source_session_id: Option<String>,
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
    pub(crate) fn from_event_and_chunk(evt: &MemoryEvent, chunk: &Chunk) -> Self {
        // Source-first: Compaction-sourced events always map to SessionSummary,
        // regardless of role, BEFORE the role-based derivation below runs.
        let (memory_type, importance) = if evt.source == MemorySource::Explicit {
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
            (memory_type, deliberate_importance(evt))
        } else if evt.source == MemorySource::Compaction {
            (MemoryType::SessionSummary, 6.0)
        } else {
            let memory_type = match (evt.role.clone(), evt.is_error) {
                (Role::Tool, Some(true)) => MemoryType::ToolError,
                (Role::Tool, _) => MemoryType::ChatMessage,
                (Role::User, _) | (Role::Assistant, _) => MemoryType::ChatMessage,
                (Role::System, _) => MemoryType::ChatMessage,
            };
            let importance = if matches!(memory_type, MemoryType::ToolError) {
                6.0
            } else {
                DEFAULT_IMPORTANCE
            };
            (memory_type, importance)
        };
        let (source, created_by_type) = seal(evt.source, &evt.role, !evt.sources().is_empty(), evt.user_stated);
        Self {
            scope: "agent",
            agent_id: Some(evt.agent_id.clone()),
            project_id: evt.project_id.clone(),
            content: chunk.text.clone(),
            memory_type,
            source,
            created_by_type,
            created_by_ref: evt.agent_id.clone(),
            importance,
            created_at_ms: evt.timestamp_ms,
            candidates: Vec::new(),
            derived_from: evt.sources().to_vec(),
            topic_key: evt.topic().map(str::to_string),
            source_session_id: evt.session_id.clone(),
        }
    }
}

/// The importance a deliberate save asks for, on the scale, or the default.
pub(crate) fn deliberate_importance(evt: &MemoryEvent) -> f64 {
    evt.importance
        .filter(|v| !v.is_nan()) // NaN has no place on the scale to clamp it to.
        .map(|v| v.clamp(IMPORTANCE_MIN, IMPORTANCE_MAX))
        .unwrap_or(DEFAULT_IMPORTANCE)
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

/// Writes one `memory_conflict` pair per candidate of the row `new_id`,
/// inside the caller's transaction.
fn note_candidates(
    tx: &rusqlite::Transaction<'_>,
    new_id: i64,
    candidates: &[Candidate],
    now: TimestampMs,
) -> rusqlite::Result<()> {
    let mut stmt = tx.prepare_cached(
        "INSERT OR IGNORE INTO memory_conflict (new_id, old_id, distance, state, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for c in candidates {
        let state = if c.escalated { "escalated" } else { "pending" };
        stmt.execute(rusqlite::params![new_id, c.id, c.distance, state, now])?;
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum MemoryStoreError {
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
///
/// The text is the project's, not the agent's: a save reinforces the live row
/// of its text in its project whoever saved it, so a shared brain keeps one
/// row per text. `?1` is the saver's reach (`reach_bind`): a row whose author
/// `sharing.json` hides from the saver is left alone, never written through.
const REINFORCE_LOOKUP_SQL: &str = concat!(
    "SELECT id FROM memory \
     WHERE COALESCE(project_id, '') = COALESCE(?2, '') \
       AND content_hash = ?3 \
       AND content_hash IS NOT NULL \
       AND deleted_at IS NULL \
       AND status IN ('active', 'disputed') \
       AND ",
    reach_predicate!("agent_id", "?1"),
    " ORDER BY created_at ASC LIMIT 1"
);

/// What a save the user stated does to the row it repeats: gives it the seal
/// such a save gets (`provenance::seal`) and raises it to the save's importance,
/// never lowering it, or what the user said could stay out of the workspace for
/// having been inferred first at a lower one. An import keeps its `source`, which
/// a second import and `clawg doctor` find it by, and a conclusion stays the
/// agent's whoever's words it rests on.
const SEAL_AS_STATED_SQL: &str = "UPDATE memory SET source = 'explicit', created_by_type = 'user', \
         importance = MAX(importance, ?2), \
         tier = compute_tier(tier, MAX(importance, ?2), access_count, memory_type) \
     WHERE id = ?1 AND source NOT IN ('import', 'synthesis')";

/// What a repeated save brings to the row it reinforces.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Repeat<'a> {
    pub(crate) derived_from: &'a [i64],
    /// The importance of a save sealed as the user's
    /// (`provenance::is_stated`), and `None` for any other.
    pub(crate) stated_importance: Option<f64>,
}

impl MemoryStore {
    /// [`MemoryStore::reinforce_noting`] with nothing to note.
    #[cfg(test)]
    pub(crate) async fn reinforce_if_duplicate(
        &self,
        saver: &Reader,
        project_id: Option<&str>,
        content_hash: &str,
        derived_from: &[i64],
        now_ms: u64,
    ) -> Result<Option<i64>, MemoryStoreError> {
        self.reinforce_noting(saver, project_id, content_hash, Repeat { derived_from, stated_importance: None }, now_ms, |_| None).await
    }

    /// Reinforces the existing active row matching `(COALESCE(project_id,''),
    /// content_hash)` within `saver`'s reach, if any: `access_count += 1`,
    /// `last_accessed_at = MAX(last_accessed_at, now_ms)`, `tier` recomputed
    /// via `compute_tier`, and the seal and importance of a save the user
    /// stated. Returns the reinforced row's id, or `None` on a miss.
    ///
    /// One `conn.call()` holding one `SELECT .optional()` → branch → `UPDATE`
    /// — atomic for that pair. `COALESCE(project_id,'')` is repeated
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
    /// be `UNIQUE` (see the migration). A restore can still bring a twin back
    /// to life while the worker embeds, so the worker's insert looks again
    /// ([`MemoryStore::insert_unless_twin`]). `MemoryRuntime::ingest_worker`
    /// is the only spawn site (`rg -n 'ingest_worker\('` confirms this). If the worker is ever parallelized, this guarantee
    /// breaks, and the fix is, in order: (i) a maintenance pass collapsing
    /// legacy duplicate triples, reassigning `superseded_by` /
    /// `disputed_against`; (ii) a migration swapping the index for a real
    /// `UNIQUE` one; (iii) catching `SQLITE_CONSTRAINT_UNIQUE` on insert and
    /// falling back to this method.
    ///
    /// On a hit it also writes the outbox progress `note(id)` names, in the
    /// same transaction, so a replay never reinforces the row a second time.
    pub(crate) async fn reinforce_noting(
        &self,
        saver: &Reader,
        project_id: Option<&str>,
        content_hash: &str,
        repeat: Repeat<'_>,
        now_ms: u64,
        note: impl FnOnce(i64) -> Option<OutboxNote> + Send + 'static,
    ) -> Result<Option<i64>, MemoryStoreError> {
        let reach = reader_bind(Some(saver));
        let project_id = project_id.map(str::to_string);
        let content_hash = content_hash.to_string();
        let derived_from = repeat.derived_from.to_vec();
        let stated_importance = repeat.stated_importance;
        let now = TimestampMs::from_unix_millis_u64(now_ms);

        self.conn
            .call(move |c| -> rusqlite::Result<Option<i64>> {
                let tx = c.transaction()?;

                let existing_id: Option<i64> = tx
                    .query_row(
                        REINFORCE_LOOKUP_SQL,
                        rusqlite::params![reach, project_id, content_hash],
                        |r| r.get(0),
                    )
                    .optional()?;

                let Some(id) = existing_id else {
                    return Ok(None);
                };

                reinforce_in(&tx, id, now, Repeat { derived_from: &derived_from, stated_importance })?;
                if let Some(note) = note(id) {
                    apply_note(&tx, &note)?;
                }

                tx.commit()?;
                Ok(Some(id))
            })
            .await
            .map_err(MemoryStoreError::from)
    }

    #[cfg(any(test, feature = "eval"))]
    pub(crate) async fn insert_memory_with_vector(
        &self,
        record: NewMemoryRecord,
        embedding: Vec<f32>,
    ) -> Result<i64, MemoryStoreError> {
        self.insert_noting(record, embedding, |_| None).await
    }

    /// [`MemoryStore::insert_memory_with_vector`] that writes the outbox
    /// progress `note(id)` names in the same transaction as the row, so a
    /// process that dies after it leaves both or neither.
    #[cfg(any(test, feature = "eval"))]
    pub(crate) async fn insert_noting(
        &self,
        record: NewMemoryRecord,
        embedding: Vec<f32>,
        note: impl FnOnce(i64) -> Option<OutboxNote> + Send + 'static,
    ) -> Result<i64, MemoryStoreError> {
        let row = PreparedRow::new(record, &embedding)?;
        self.conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                let id = row.insert(&tx)?;
                if let Some(note) = note(id) {
                    apply_note(&tx, &note)?;
                }
                tx.commit()?;
                Ok(id)
            })
            .await
            .map_err(MemoryStoreError::from)
    }

    /// [`MemoryStore::insert_noting`] that first looks, in the same
    /// transaction, for a live row of the same project and text within
    /// `saver`'s reach, and reinforces it instead. The worker's own lookup runs before the embedding, and a
    /// restore can bring the same text back to life in between: inserting
    /// then would leave two live copies that nothing collapses. `inserted`
    /// notes an insert, `absorbed` a reinforcement.
    pub(crate) async fn insert_unless_twin(
        &self,
        saver: &Reader,
        record: NewMemoryRecord,
        embedding: Vec<f32>,
        inserted: impl FnOnce(i64) -> Option<OutboxNote> + Send + 'static,
        absorbed: impl FnOnce(i64) -> Option<OutboxNote> + Send + 'static,
    ) -> Result<Stored, MemoryStoreError> {
        let row = PreparedRow::new(record, &embedding)?;
        let reach = reader_bind(Some(saver));
        self.conn
            .call(move |c| -> Result<Stored, rusqlite::Error> {
                let tx = c.transaction()?;
                let twin: Option<i64> = tx
                    .query_row(
                        REINFORCE_LOOKUP_SQL,
                        rusqlite::params![reach, row.record.project_id, row.content_hash],
                        |r| r.get(0),
                    )
                    .optional()?;
                let (stored, note) = match twin {
                    Some(id) => {
                        let stated_importance =
                            is_stated(row.record.source, row.record.created_by_type).then_some(row.record.importance);
                        reinforce_in(&tx, id, row.created_at, Repeat { derived_from: &row.record.derived_from, stated_importance })?;
                        (Stored::Absorbed(id), absorbed(id))
                    }
                    None => {
                        let id = row.insert(&tx)?;
                        (Stored::Inserted(id), inserted(id))
                    }
                };
                if let Some(note) = note {
                    apply_note(&tx, &note)?;
                }
                tx.commit()?;
                Ok(stored)
            })
            .await
            .map_err(MemoryStoreError::from)
    }
}

/// What [`MemoryStore::insert_unless_twin`] did with a piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stored {
    Inserted(i64),
    Absorbed(i64),
}

/// One more access to an existing row, at `now`, its tier recomputed, the save's
/// sources linked to it and, when the user stated the save, its seal and
/// importance.
fn reinforce_in(
    tx: &rusqlite::Transaction<'_>,
    id: i64,
    now: TimestampMs,
    repeat: Repeat<'_>,
) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE memory \
         SET last_accessed_at = MAX(last_accessed_at, ?2), \
             access_count     = access_count + 1, \
             tier = compute_tier(tier, importance, access_count + 1, memory_type) \
         WHERE id = ?1",
        rusqlite::params![id, now],
    )?;
    if let Some(importance) = repeat.stated_importance {
        tx.execute(SEAL_AS_STATED_SQL, rusqlite::params![id, importance])?;
    }
    link_sources(tx, id, repeat.derived_from, now)
}

/// A `memory` row and its vector, worked out before the transaction that
/// writes them.
struct PreparedRow {
    record: NewMemoryRecord,
    bytes: Vec<u8>,
    created_at: TimestampMs,
    tier: i64,
    status: &'static str,
    disputed_against: Option<String>,
    last_disputed_at: Option<TimestampMs>,
    content_hash: String,
    lang: &'static str,
}

impl PreparedRow {
    fn new(record: NewMemoryRecord, embedding: &[f32]) -> Result<Self, MemoryStoreError> {
        let unit = to_unit(embedding)
            .map_err(|e| MemoryStoreError::InvalidEmbedding { reason: e.to_string() })?;
        let created_at = TimestampMs::from_unix_millis_u64(record.created_at_ms);
        // Derive dispute columns in Rust before .call() so every
        // param flows through a single c.transaction() — atomic, no partial writes.
        let escalated: Vec<i64> = record.candidates.iter().filter(|c| c.escalated).map(|c| c.id).collect();
        let disputed = !escalated.is_empty();
        Ok(Self {
            bytes: serialize_f32_vec(&unit),
            created_at,
            // Compute the initial tier explicitly instead of relying
            // on the schema DEFAULT — the single source of truth for every
            // producer. PROMOTE-ONLY, starts from current=3 (see memory/tier.rs).
            tier: crate::memory::tier::compute_tier(3, record.importance, 0, record.memory_type.as_str()),
            status: if disputed { "disputed" } else { "active" },
            disputed_against: disputed
                .then(|| serde_json::to_string(&escalated).expect("Vec<i64> serialises infallibly")),
            last_disputed_at: disputed.then_some(created_at),
            // Same normalization the write-border dedup gate hashes against
            // (worker.rs, before embed) — writing it here too keeps every row
            // dedup-eligible regardless of which producer inserted it.
            content_hash: crate::memory::dedup::content_hash(&record.content),
            // The row's own text, already masked: a piece is long enough to
            // tell, and the import and the maintenance pass see rows too.
            lang: language::column_value(language::detect(&record.content)),
            record,
        })
    }

    /// Writes the row, its vector, its source links and its candidate pairs.
    fn insert(&self, tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<i64> {
        let record = &self.record;
        tx.execute(
            "INSERT INTO memory \
             (scope, agent_id, project_id, content, memory_type, source, \
              created_at, created_by_type, created_by_ref, \
              last_modified_at, last_accessed_at, access_count, importance, tier, \
              status, disputed_against, last_disputed_at, content_hash, topic_key, source_session_id, lang) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                    MemorySource::Synthesis => "synthesis",
                },
                self.created_at,
                record.created_by_type.as_str(),
                record.created_by_ref,
                self.created_at,
                self.created_at,
                // Written explicitly as 1, not left at the schema's DEFAULT 0: this
                // row's creation is itself the first occurrence of this content, so
                // a chunk resent once (one REINFORCE, `access_count + 1`) reads as
                // "seen twice" — 2, not 1 — matching what REINFORCE's `+= 1` promises.
                1,
                record.importance,
                self.tier,
                self.status,
                self.disputed_against,
                self.last_disputed_at,
                self.content_hash,
                record.topic_key,
                record.source_session_id,
                self.lang,
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
            rusqlite::params![id, self.bytes],
        )?;
        link_sources(tx, id, &record.derived_from, self.created_at)?;
        note_candidates(tx, id, &record.candidates, self.created_at)?;
        Ok(id)
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
    use crate::memory::sharing::Reach;

    /// `agent` as a policy that hides nothing lets it save.
    fn shared(agent: &str) -> Reader {
        Reader::new(agent, Reach::AllBut(Vec::new()))
    }
    use super::*;

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
            candidates: Vec::new(),
            derived_from: Vec::new(),
            topic_key: None,
            source_session_id: None,
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

    // ── NewMemoryRecord.candidates defaults empty ─────

    #[test]
    fn new_memory_record_candidates_default_empty_in_make_record() {
        let rec = make_record("a", None);
        assert!(rec.candidates.is_empty(), "make_record must default to no candidates");
    }

    #[test]
    fn new_memory_record_from_event_and_chunk_has_no_candidates() {
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
            topic_key: None,
            user_stated: false,
            session_id: None,
        };
        let chunk = Chunk { text: "hello".to_string() };
        let rec = NewMemoryRecord::from_event_and_chunk(&evt, &chunk);
        assert!(rec.candidates.is_empty(), "from_event_and_chunk must default to no candidates");
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
            topic_key: None,
            user_stated: false,
            session_id: None,
        };
        let chunk = Chunk { text: "delta summary text".to_string() };
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
            topic_key: None,
            user_stated: false,
            session_id: None,
        }
    }

    fn chunk_of(text: &str) -> crate::memory::chunker::Chunk {
        crate::memory::chunker::Chunk { text: text.to_string() }
    }

    #[test]
    fn explicit_source_honours_both_overrides() {
        let evt = explicit_event(Some("architectural_decision"), Some(8.5));
        let rec = NewMemoryRecord::from_event_and_chunk(&evt, &chunk_of("x"));
        assert_eq!(rec.memory_type, MemoryType::ArchitecturalDecision);
        assert_eq!(rec.importance, 8.5);
        assert_eq!(
            rec.created_by_type,
            CreatedByType::Agent,
            "an undeclared save is the agent's, whatever role the transport stamped on the event"
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

    #[tokio::test]
    async fn a_save_records_the_language_of_each_row() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let mut ids = Vec::new();
        for content in [
            "El daemon guarda cada memoria en una sola transacción, y la búsqueda las encuentra.",
            "The daemon stores every memory in one transaction, and the search finds them.",
            "`MemoryStore::open` in crates/clawg-core/src/memory/db.rs",
        ] {
            let record = NewMemoryRecord { content: content.to_string(), ..make_record("a", None) };
            ids.push(store.insert_memory_with_vector(record, make_embedding_384()).await.unwrap());
        }
        let langs: Vec<Option<String>> = store
            .conn
            .call(|c| -> rusqlite::Result<_> {
                let mut stmt = c.prepare("SELECT lang FROM memory ORDER BY id")?;
                let langs = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                Ok(langs)
            })
            .await
            .unwrap();
        assert_eq!(ids.len(), 3);
        assert_eq!(langs, [Some("es".to_string()), Some("en".to_string()), Some("und".to_string())]);
    }

    // ── INSERT writes status/disputed_against/last_disputed_at ──

    #[tokio::test]
    async fn insert_with_no_dispute_writes_active_status() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let record = make_record("a", None);
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

    fn candidate(id: i64, distance: f64, escalated: bool) -> Candidate {
        Candidate { id, distance, escalated, excerpt: String::new(), same_scope: true }
    }

    #[tokio::test]
    async fn insert_with_dispute_writes_disputed_status_and_json() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let mut record = make_record("a", None);
        record.candidates = vec![candidate(12, 0.2, true), candidate(34, 0.3, true)];
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

    async fn conflict_rows(store: &crate::memory::db::MemoryStore) -> Vec<(i64, i64, Option<f64>, String, i64)> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT new_id, old_id, distance, state, created_at FROM memory_conflict ORDER BY old_id",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect();
                rows
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn insert_writes_one_conflict_row_per_candidate() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let mut record = make_record("a", None);
        record.candidates = vec![candidate(7, 0.25, false), candidate(9, 0.31, true)];
        let id = store.insert_memory_with_vector(record, make_embedding_384()).await.unwrap();

        let rows = conflict_rows(&store).await;
        assert_eq!(
            rows,
            vec![
                (id, 7, Some(0.25), "pending".to_string(), 1_000_000),
                (id, 9, Some(0.31), "escalated".to_string(), 1_000_000),
            ]
        );
        let (status, da): (String, Option<String>) = store
            .conn
            .call(move |c| c.query_row("SELECT status, disputed_against FROM memory WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))))
            .await
            .unwrap();
        assert_eq!((status.as_str(), da.as_deref()), ("disputed", Some("[9]")), "only the escalated pair disputes");
    }

    #[tokio::test]
    async fn pending_candidates_alone_leave_the_row_active() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let mut record = make_record("a", None);
        record.candidates = vec![candidate(7, 0.25, false)];
        let id = store.insert_memory_with_vector(record, make_embedding_384()).await.unwrap();

        let (status, da, lda): (String, Option<String>, Option<i64>) = store
            .conn
            .call(move |c| {
                c.query_row("SELECT status, disputed_against, last_disputed_at FROM memory WHERE id = ?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
            })
            .await
            .unwrap();
        assert_eq!((status.as_str(), da, lda), ("active", None, None));
        assert_eq!(conflict_rows(&store).await.len(), 1);
    }

    #[tokio::test]
    async fn insert_with_dispute_is_atomic_single_transaction() {
        // Verify both memory and memory_vectors rows exist after disputed insert
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let mut record = make_record("a", None);
        record.candidates = vec![candidate(99, 0.2, true)];
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
        bytes.as_chunks::<4>().0.iter().copied().map(f32::from_le_bytes).collect()
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

    // ── reinforce_if_duplicate ───────────────────────────────────────────────

    /// Raw-inserts a `memory` row with the given `content_hash`, `status`, and
    /// `deleted_at`, bypassing `insert_memory_with_vector` (which does not yet
    /// bind `content_hash`). Returns the new row id.
    #[expect(clippy::too_many_arguments, reason = "a test fixture that takes each column of the row it seeds")]
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

        let result = store.reinforce_if_duplicate(&shared("agent-r1"), None, hash, &[], 2_000).await.unwrap();
        assert_eq!(result, Some(id), "a matching active row must be reinforced");

        let (access_count, tier) = fetch_access_count_and_tier(&store, id).await;
        assert_eq!(access_count, 3, "access_count must increment by 1");
        assert_eq!(tier, 1, "tier must be recomputed via compute_tier (flush re-eval: access_count>=3)");
    }

    #[tokio::test]
    async fn reinforce_if_duplicate_miss_returns_none() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let result = store.reinforce_if_duplicate(&shared("agent-r2"), None, "no-such-hash", &[], 1_000).await.unwrap();
        assert_eq!(result, None, "no matching row must return None");
    }

    #[tokio::test]
    async fn reinforce_if_duplicate_soft_deleted_row_returns_none() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let hash = "hash-reinforce-deleted";
        seed_hashed_row(&store, "agent-r3", None, hash, "active", Some(500), 1_000, 0).await;

        let result = store.reinforce_if_duplicate(&shared("agent-r3"), None, hash, &[], 2_000).await.unwrap();
        assert_eq!(result, None, "a soft-deleted row must never be reinforced");
    }

    #[tokio::test]
    async fn reinforce_if_duplicate_superseded_row_returns_none() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let hash = "hash-reinforce-superseded";
        seed_hashed_row(&store, "agent-r4", None, hash, "superseded", None, 1_000, 0).await;

        let result = store.reinforce_if_duplicate(&shared("agent-r4"), None, hash, &[], 2_000).await.unwrap();
        assert_eq!(result, None, "a superseded row must never be reinforced");
    }

    #[tokio::test]
    async fn reinforce_if_duplicate_two_project_ids_are_independent() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let hash = "hash-reinforce-scoped";
        let id_p1 = seed_hashed_row(&store, "agent-r5", Some("proj-1"), hash, "active", None, 1_000, 0).await;
        let id_p2 = seed_hashed_row(&store, "agent-r5", Some("proj-2"), hash, "active", None, 1_000, 0).await;
        assert_ne!(id_p1, id_p2);

        let hit_p1 = store.reinforce_if_duplicate(&shared("agent-r5"), Some("proj-1"), hash, &[], 2_000).await.unwrap();
        assert_eq!(hit_p1, Some(id_p1), "proj-1 lookup must reinforce only the proj-1 row");

        let hit_p2 = store.reinforce_if_duplicate(&shared("agent-r5"), Some("proj-2"), hash, &[], 2_000).await.unwrap();
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

        let result = store.reinforce_if_duplicate(&shared("agent-r6"), None, hash, &[], 2_000).await.unwrap();
        assert_eq!(result, Some(id), "NULL project_id on both sides must match via COALESCE");
    }

    /// A shared brain keeps one row per text in a project: identical text from
    /// another agent reinforces the row that is already there, whoever saved
    /// it, and not a row the policy hides from the saver.
    #[tokio::test]
    async fn reinforce_if_duplicate_reinforces_another_agents_row() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let hash = "hash-reinforce-cross-agent";
        let id_a = seed_hashed_row(&store, "agent-a", Some("p"), hash, "active", None, 1_000, 0).await;

        let hit = store.reinforce_if_duplicate(&shared("agent-b"), Some("p"), hash, &[], 2_000).await.unwrap();
        assert_eq!(hit, Some(id_a), "agent-b's save is agent-a's text in the same project");
        let (ac_a, _) = fetch_access_count_and_tier(&store, id_a).await;
        assert_eq!(ac_a, 1);

        let elsewhere = store.reinforce_if_duplicate(&shared("agent-b"), Some("q"), hash, &[], 2_000).await.unwrap();
        assert_eq!(elsewhere, None, "another project keeps its own row");
    }

    #[tokio::test]
    async fn reinforce_if_duplicate_leaves_a_hidden_authors_row() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let hash = "hash-reinforce-hidden";
        let id_a = seed_hashed_row(&store, "agent-a", None, hash, "active", None, 1_000, 0).await;

        let hidden = Reader::new("agent-b", Reach::AllBut(vec!["agent-a".into()]));
        assert_eq!(store.reinforce_if_duplicate(&hidden, None, hash, &[], 2_000).await.unwrap(), None);
        assert_eq!(fetch_access_count_and_tier(&store, id_a).await.0, 0, "a hidden author's row is never written through");
        let own = Reader::new("agent-b", Reach::Own);
        assert_eq!(store.reinforce_if_duplicate(&own, None, hash, &[], 2_000).await.unwrap(), None);
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

        let hit = store.reinforce_if_duplicate(&shared("agent-r8"), None, hash, &[], 2_000).await.unwrap();
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

        let result = store.reinforce_if_duplicate(&shared("agent-r7"), None, hash, &[], 3_000).await.unwrap();
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

        store.reinforce_if_duplicate(&shared(&agent_id), None, &hash, &[], 2_000).await.unwrap();
        let (access_count, tier) = fetch_access_count_and_tier(&store, id).await;
        assert_eq!(access_count, 2);
        assert_eq!(tier, 2, "one reinforcement is not enough to promote");

        store.reinforce_if_duplicate(&shared(&agent_id), None, &hash, &[], 3_000).await.unwrap();
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
                    .query_map(rusqlite::params![reader_bind(Some(&shared("a"))), Option::<String>::None, "h"], |r| {
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
        // Any index that hands the project its rows by `created_at` competes
        // for `ORDER BY created_at ASC LIMIT 1`; the dedup index wins only while
        // it gives the order too.
        assert!(!plan_text.contains("TEMP B-TREE"), "the lookup sorts its rows: {plan_text}");
    }
}
