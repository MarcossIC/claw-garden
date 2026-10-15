//! Anti-corruption layer between the memory subsystem and the retrieval-
//! evaluation binary: an opaque store handle plus this module's OWN border
//! types, never the internal ones.
//!
//! Re-exporting the store's internal types directly was tried first and
//! rejected: the moment any of them crosses this boundary, the store's
//! entire method surface rides along with it — a publicly reachable type
//! drags every one of its public methods into scope, whether this module
//! calls them or not, and their own parameter types then have to widen too.
//! An internal refactor of the store must not be able to break this offline
//! tool, and this tool must not be able to reach store methods it has no
//! business calling. Wrapping the store in an opaque newtype and defining
//! `EvalScope`/`EvalHit`/`EvalKind` as this module's own shapes means the
//! tooling only ever depends on what is explicitly mapped here — nothing
//! more.
//!
//! Compiled only when the `eval` feature is enabled, so the shipped
//! application's public surface is unchanged by this module's existence.

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use crate::memory::access::AccessTracker;
use crate::memory::db::MemoryStore;
use crate::memory::chunker::token_splitter::TokenSplitter;
use crate::memory::embedder::{Embedder, FakeEmbedder, FastEmbedLocal};

/// Re-exported so the `rag-eval` and `golden-mine` binaries can name a role
/// without reaching into the private `memory` module tree.
pub use crate::memory::embedder::EmbedRole;
use crate::memory::events::{MemoryEvent, MemorySource};
use crate::memory::model_download;
use crate::memory::project_docs;
use crate::memory::score::{ImportanceFactor, ScoringConfig};
use crate::memory::search::{search_all_memories_with_scoring, SearchScope};
use crate::memory::store_insert::{CreatedByType, MemoryType, NewMemoryRecord};
use crate::memory::store_lock::StoreHolder;
use crate::memory::worker::{memory_worker, resolve_tokenizer_and_chunker, IngestRequest};
use crate::memory::TableKind;
use crate::models::chat::Role;
use crate::runtime::{MemoryRuntime, INGEST_QUEUE_CAPACITY};

/// Opaque handle to a memory store. Every operation available on it is a
/// method below — the wrapped store itself is never reachable.
pub struct EvalStore(MemoryStore);

/// The lexical-channel candidate budget the shipped application searches
/// with, read off the production default itself rather than restated.
///
/// `rag-eval` exists to measure the retrieval configuration users actually
/// run. A second, independently-edited copy of this number would let the two
/// drift apart silently, and every committed measurement would then describe
/// a configuration nobody ships — the failure mode is invisible precisely
/// because both numbers look deliberate.
pub fn production_lexical_budget() -> Option<usize> {
    SearchScope::all_default().lexical
}

/// The pieces the worker cuts `text` into when no tokenizer is loaded (four
/// bytes per token). It is the coarsest count production gives, so a text
/// that reaches some number of pieces here reaches at least as many with the
/// real tokenizer, whose tokens average fewer bytes.
pub fn fallback_pieces(text: &str) -> Vec<String> {
    crate::memory::chunker::ChatChunker::from_tokenizer(None)
        .split_save(text)
        .pieces
        .into_iter()
        .map(|c| c.text)
        .collect()
}

/// The tokenizer `memory_fts` carries when nothing swaps it — the value the
/// migration itself writes. Named here so a run that never passed
/// `--fts-tokenizer` can still record which tokenizer produced its numbers,
/// instead of recording the absence of a flag.
pub const MIGRATION_FTS_TOKENIZER: &str = "unicode61 remove_diacritics 2";

/// Per-table result-pool sizes for a search call — this module's own copy of
/// the store's scope shape, so a change to the internal one cannot silently
/// change what this tool asks for.
#[derive(Debug, Clone, Copy)]
pub struct EvalScope {
    pub memory: Option<usize>,
    pub episodic: Option<usize>,
    pub procedural: Option<usize>,
    /// Candidate budget for the lexical (FTS5) channel — `None` keeps the
    /// channel off, same meaning as `SearchScope::lexical`. This module's own
    /// copy so a change to the internal field cannot silently change what
    /// this tool asks for, the same "mirror only what's used" discipline
    /// every other field on this struct already follows.
    pub lexical: Option<usize>,
}

impl EvalScope {
    fn into_search_scope(self) -> SearchScope {
        SearchScope {
            memory: self.memory,
            episodic: self.episodic,
            procedural: self.procedural,
            // rag-eval never carries a project — its whole corpus is one
            // unscoped pool, so this keeps every query bit-identical to
            // retrieval before project scoping existed.
            project_id: None,
            lexical: self.lexical,
            // One agent's corpus: nothing to share.
            reach: crate::memory::sharing::Reach::Own,
        }
    }
}

/// This module's own copy of the scoring knobs `rag-eval`'s sweep varies —
/// everything else in `ScoringConfig` stays at `ScoringConfig::DEFAULT`, the
/// same "mirror only what's used" discipline [`EvalScope`] already follows
/// for `SearchScope` (which also carries a `project_id` this module never
/// sets). `Serialize` derives here, not on `ScoringConfig`: a scoring config
/// is only ever written out from this border copy, the same way `EvalScope`
/// rather than `SearchScope` would be if a report ever needed to record it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct EvalScoringConfig {
    pub importance_base: f64,
    pub importance_divisor: f64,
    pub importance_clamp_lo: f64,
    pub importance_clamp_hi: f64,
    pub cosine_floor: Option<f64>,
}

impl EvalScoringConfig {
    pub(crate) fn into_scoring_config(self) -> ScoringConfig {
        ScoringConfig {
            importance: ImportanceFactor {
                base: self.importance_base,
                divisor: self.importance_divisor,
                clamp_lo: self.importance_clamp_lo,
                clamp_hi: self.importance_clamp_hi,
            },
            cosine_floor: self.cosine_floor,
            ..ScoringConfig::DEFAULT
        }
    }
}

impl EvalScoringConfig {
    /// Builds a sweep point whose importance range is derived through
    /// `ImportanceFactor::spanning`. Lives here, not in `main.rs`, because
    /// `ImportanceFactor` is `pub(crate)` inside this library — the binary
    /// crate can never name it directly, only cross the border through a
    /// method this mirror type exposes, the same anti-corruption discipline
    /// `into_scoring_config`/`into_search_scope` already follow.
    pub fn with_importance_span(mut self, min: f64, max: f64) -> Self {
        let f = ImportanceFactor::spanning(min, max);
        self.importance_base = f.base;
        self.importance_divisor = f.divisor;
        self.importance_clamp_lo = f.clamp_lo;
        self.importance_clamp_hi = f.clamp_hi;
        self
    }

    /// Sets the cosine re-scale floor for a sweep point.
    pub fn with_cosine_floor(mut self, floor: f64) -> Self {
        self.cosine_floor = Some(floor);
        self
    }
}

impl Default for EvalScoringConfig {
    fn default() -> Self {
        let imp = ScoringConfig::DEFAULT.importance;
        Self {
            importance_base: imp.base,
            importance_divisor: imp.divisor,
            importance_clamp_lo: imp.clamp_lo,
            importance_clamp_hi: imp.clamp_hi,
            cosine_floor: ScoringConfig::DEFAULT.cosine_floor,
        }
    }
}

/// Which backing table a hit came from — this module's own copy, so callers
/// never need to name the internal table-kind type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalKind {
    Memory,
    Episodic,
    Procedural,
}

impl From<TableKind> for EvalKind {
    fn from(kind: TableKind) -> Self {
        match kind {
            TableKind::Memory => EvalKind::Memory,
            TableKind::Episodic => EvalKind::Episodic,
            TableKind::Procedural => EvalKind::Procedural,
        }
    }
}

/// One ranked search result, translated out of the store's internal shape.
#[derive(Debug, Clone)]
pub struct EvalHit {
    pub id: i64,
    pub kind: EvalKind,
    /// Carried through deliberately: the harness uses it to tell a row the
    /// KNN pool structurally excludes (tier <= 1 never reaches search) apart
    /// from one that was simply outranked.
    pub tier: i64,
    pub score: f64,
    pub distance: f64,
}

/// Errors surfaced by [`EvalStore`]. Its own type rather than a re-export of
/// the store's internal error enums: those are shaped around the production
/// insert/search call sites, and a variant added there for a case this tool
/// can never hit would otherwise force an unrelated match arm here.
#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("failed to open the store: {0}")]
    Open(String),
    #[error("insert failed: {0}")]
    Insert(String),
    #[error("search failed: {0}")]
    Search(String),
    #[error("unknown memory_type: {0}")]
    UnknownMemoryType(String),
    #[error("embedding failed: {0}")]
    Embed(String),
    #[error("model load failed: {0}")]
    ModelLoad(String),
    #[error("unrecognized FTS5 tokenizer '{0}'; allowed: 'unicode61 remove_diacritics 2', 'trigram'")]
    UnknownTokenizer(String),
    #[error("failed to recreate memory_fts: {0}")]
    RecreateFts(String),
}

/// Tokenizer strings [`EvalStore::recreate_fts_with_tokenizer`] accepts — an
/// allowlist, never interpolation of caller text into DDL. This is also what
/// keeps the `sqlite_master` byte-diff guard test finite: every tokenizer
/// this binary can ever request is enumerated here, not typed at a call site.
const ALLOWED_FTS_TOKENIZERS: &[&str] = &[MIGRATION_FTS_TOKENIZER, "trigram"];

/// The `memory_fts` virtual table's CREATE statement, character-for-character
/// identical to `migrations/V8__memory_fts.sql`'s own CREATE VIRTUAL TABLE
/// statement except for the `tokenize=` value. That exactness is not
/// cosmetic: SQLite stores a virtual table's `sqlite_master.sql` verbatim as
/// submitted, so this is what makes the guard test's byte-diff meaningful —
/// any other formatting divergence here would show up as a spurious
/// difference that has nothing to do with the tokenizer.
const FTS_CREATE_SQL_TEMPLATE: &str = "CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(\n    content,\n    agent_id UNINDEXED,\n    project_id UNINDEXED,\n    memory_type UNINDEXED,\n    content='memory',\n    content_rowid='id',\n    tokenize='__TOKENIZER__'\n)";

impl EvalStore {
    /// Opens (creating if absent) a store rooted at `base_dir`, migrations
    /// included — the same path production startup takes.
    pub async fn open(base_dir: &Path) -> Result<Self, EvalError> {
        MemoryStore::open(base_dir, StoreHolder::Eval)
            .await
            .map(EvalStore)
            .map_err(|e| EvalError::Open(e.to_string()))
    }

    /// Inserts one row plus its embedding atomically, the same path
    /// production ingest uses. `memory_type` is the on-disk spelling
    /// (`"chat_message"`, `"user_preference"`, …); an unrecognized value is
    /// an error rather than a silent fallback, because a typo'd golden-set
    /// row must not seed a row the tier gate never meant to accept.
    pub async fn insert(
        &self,
        agent_id: &str,
        content: &str,
        memory_type: &str,
        importance: f64,
        embedding: Vec<f32>,
    ) -> Result<i64, EvalError> {
        let memory_type = parse_memory_type(memory_type)?;
        let created_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let record = NewMemoryRecord {
            scope: "agent",
            agent_id: Some(agent_id.to_string()),
            project_id: None,
            content: content.to_string(),
            memory_type,
            source: MemorySource::Chat,
            created_by_type: CreatedByType::System,
            created_by_ref: "rag-eval".to_string(),
            importance,
            created_at_ms,
            disputed_against: None,
            derived_from: Vec::new(),
        };
        self.0
            .insert_memory_with_vector(record, embedding)
            .await
            .map_err(|e| EvalError::Insert(e.to_string()))
    }

    /// KNN search across all three tables, translated into this module's own
    /// hit shape. `query_text` is the lexical half of the question — fed
    /// through untouched to `search_all_memories_with_scoring`, mirroring
    /// production's own `embedding`/`query_text` pairing (`search.rs`).
    pub async fn search(
        &self,
        embedding: &[f32],
        agent_id: &str,
        scope: EvalScope,
        k_final: usize,
        scoring: EvalScoringConfig,
        query_text: Option<&str>,
    ) -> Result<Vec<EvalHit>, EvalError> {
        let hits = search_all_memories_with_scoring(
            &self.0,
            embedding,
            query_text,
            agent_id,
            scope.into_search_scope(),
            k_final,
            &scoring.into_scoring_config(),
        )
        .await
        .map_err(|e| EvalError::Search(e.to_string()))?;

        Ok(hits
            .into_iter()
            .map(|h| EvalHit {
                id: h.id,
                kind: h.kind.into(),
                tier: h.tier,
                score: h.score,
                distance: h.distance,
            })
            .collect())
    }

    /// Drops and recreates `memory_fts` with a different tokenizer. Eval-only
    /// — no production call site ever needs to swap a live index's
    /// tokenizer, which is why this lives beside `search`/`insert` rather
    /// than on `MemoryStore` itself.
    ///
    /// Issues no backfill statement. This is called from `seed()`, before
    /// the insert loop, so the `AFTER INSERT` trigger — which survives the
    /// `DROP TABLE`, since it names `memory_fts` but lives on `memory` —
    /// indexes every row as it lands. Duplicating the migration's backfill
    /// `INSERT..SELECT` here would be a second copy of it to keep in sync.
    pub async fn recreate_fts_with_tokenizer(&self, tokenizer: &str) -> Result<(), EvalError> {
        if !ALLOWED_FTS_TOKENIZERS.contains(&tokenizer) {
            return Err(EvalError::UnknownTokenizer(tokenizer.to_string()));
        }
        let create_sql = FTS_CREATE_SQL_TEMPLATE.replace("__TOKENIZER__", tokenizer);
        self.0
            .conn
            .call(move |c| -> rusqlite::Result<()> {
                c.execute("DROP TABLE memory_fts", [])?;
                c.execute(&create_sql, [])?;
                Ok(())
            })
            .await
            .map_err(|e| EvalError::RecreateFts(e.to_string()))
    }

    /// The exact `sqlite_master.sql` text for `memory_fts` — used only by the
    /// tokenizer-swap guard test to compare a migrated table's stored CREATE
    /// statement against a rebuilt one, byte for byte.
    #[cfg(test)]
    async fn fts_schema_sql(&self) -> String {
        self.0
            .conn
            .call(|c| {
                c.query_row(
                    "SELECT sql FROM sqlite_master WHERE name = 'memory_fts'",
                    [],
                    |r| r.get::<_, String>(0),
                )
            })
            .await
            .expect("memory_fts must exist in sqlite_master")
    }
}

fn parse_memory_type(raw: &str) -> Result<MemoryType, EvalError> {
    match raw {
        "chat_message" => Ok(MemoryType::ChatMessage),
        "tool_error" => Ok(MemoryType::ToolError),
        "code_block" => Ok(MemoryType::CodeBlock),
        "session_summary" => Ok(MemoryType::SessionSummary),
        "user_preference" => Ok(MemoryType::UserPreference),
        "architectural_decision" => Ok(MemoryType::ArchitecturalDecision),
        "semantic_fact" => Ok(MemoryType::SemanticFact),
        other => Err(EvalError::UnknownMemoryType(other.to_string())),
    }
}

/// Opaque handle to whichever embedder is backing a run. Same wrapper
/// discipline as [`EvalStore`]: the concrete implementation is boxed behind
/// this module's own type, so neither the `Embedder` trait nor either of its
/// implementations is ever named outside this file.
pub struct EvalEmbedder(Arc<dyn Embedder>);

impl EvalEmbedder {
    /// A deterministic double: the same text always yields the same vector,
    /// with no model to load and no network involved. Its vectors are NOT
    /// L2-normalized (`‖v‖ ≈ 11.3` rather than 1.0), so distances measured
    /// against it are not on the same scale as distances measured against
    /// the real model — a run against this embedder validates plumbing and
    /// metric code, not retrieval quality.
    pub fn fake() -> Self {
        EvalEmbedder(Arc::new(FakeEmbedder::default()))
    }

    /// Loads the real embedding model rooted at `base_dir`, downloading it
    /// first if it is not already cached. See [`model_is_cached`] to check
    /// readiness without paying for a download.
    pub async fn real(base_dir: &Path) -> Result<Self, EvalError> {
        let files = model_download::ensure_model(base_dir, None)
            .await
            .map_err(|e| EvalError::ModelLoad(e.to_string()))?;
        let inner = FastEmbedLocal::from_bytes(files.onnx, files.tokenizer)
            .map_err(|e| EvalError::ModelLoad(e.to_string()))?;
        Ok(EvalEmbedder(Arc::new(inner)))
    }

    pub async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EvalError> {
        self.0
            .embed_with_role(text, role)
            .await
            .map_err(|e| EvalError::Embed(e.to_string()))
    }

    pub fn dimensions(&self) -> usize {
        self.0.dimensions()
    }

    pub fn model_id(&self) -> &str {
        self.0.model_id()
    }

    pub fn query_prefix(&self) -> &str {
        self.0.query_prefix()
    }

    pub fn passage_prefix(&self) -> &str {
        self.0.passage_prefix()
    }
}

/// The production ingest pipeline running over an isolated store, for
/// measuring what an agent actually sees after a save — chunking, the dedup
/// gate, contradiction detection and tiering all included, none of which the
/// direct [`EvalStore::insert`] path goes through.
///
/// Events enter exactly as the daemon's `save_memory` handler queues them
/// (`MemorySource::Explicit`, user role), and the same `memory_worker`
/// drains them.
pub struct EvalIngest {
    tx: Sender<IngestRequest>,
    worker: JoinHandle<Vec<MemoryEvent>>,
    runtime: MemoryRuntime,
}

impl EvalIngest {
    /// Opens a store rooted at `base_dir` and starts the ingest worker on it.
    /// `tokenizer_dir` is where the worker looks for `tokenizer.json`: the
    /// profile directory under `--real`, so chunk boundaries match the ones a
    /// real daemon cuts, and any directory without one under `--fake`, which
    /// leaves the worker on its byte-splitting fallback.
    pub async fn open(
        base_dir: &Path,
        embedder: &EvalEmbedder,
        tokenizer_dir: &Path,
    ) -> Result<Self, EvalError> {
        let (mut runtime, _) = MemoryRuntime::open(base_dir.to_path_buf(), StoreHolder::Eval)
            .await
            .map_err(|e| EvalError::Open(e.to_string()))?;
        let _ = runtime.embedder.set(embedder.0.clone());
        runtime.access_tracker = Arc::new(AccessTracker::inert());

        let (tx, rx) = tokio::sync::mpsc::channel(INGEST_QUEUE_CAPACITY);
        let chat_chunker = resolve_tokenizer_and_chunker(tokenizer_dir, &runtime.tokenizer);
        let worker = tokio::spawn(memory_worker(
            rx,
            tokio::sync::watch::channel(None).1,
            crate::memory::spill::Replay::none(),
            runtime.store.clone(),
            runtime.embedder.clone(),
            chat_chunker,
        ));

        Ok(Self { tx, worker, runtime })
    }

    /// Queues one explicit save. `memory_type` is validated here, up front: the
    /// worker would quietly fall back to `semantic_fact` for a typo, and a
    /// golden-set row must not be measured as something it never declared.
    ///
    /// Waits for room when the queue is full: a measurement loads its whole
    /// set before reading, and refusing a row would measure a smaller set.
    pub async fn send(
        &self,
        agent_id: &str,
        content: &str,
        memory_type: &str,
        importance: f64,
    ) -> Result<(), EvalError> {
        parse_memory_type(memory_type)?;
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.tx
            .send(MemoryEvent {
                agent_id: agent_id.to_string(),
                project_id: None,
                role: Role::User,
                content: content.to_string(),
                is_error: None,
                source: MemorySource::Explicit,
                timestamp_ms,
                memory_type: Some(memory_type.to_string()),
                importance: Some(importance),
                derived_from: Vec::new(),
            }
            .into())
            .await
            .map_err(|_| EvalError::Insert("the ingest worker has stopped".to_string()))
    }

    /// Closes the queue and waits for the worker to finish it. The worker only
    /// ends once every sender is gone, so this is the one "everything queued
    /// is now stored" signal the pipeline has.
    ///
    /// Hands back the runtime the daemon's router is built around. Its access
    /// tracker is inert: retrieval during a measurement must not bump
    /// `access_count` or promote tiers, or each query would be answered from a
    /// store the previous query had already changed.
    pub async fn finish(self) -> Result<MemoryRuntime, EvalError> {
        let Self { tx, worker, runtime } = self;
        drop(tx);
        worker.await.map_err(|e| EvalError::Insert(format!("the ingest worker failed: {e}")))?;
        Ok(runtime)
    }
}

/// The token window the live embedder truncates to — re-exported so the eval
/// binary's `--follow-up` budget reads the exact same number production's
/// embedder enforces, rather than a second copy that could drift from it.
pub const EMBED_MAX_TOKENS: usize = crate::memory::embedder::EMBED_MAX_TOKENS;
/// Special-token headroom the query-context budget reserves — see
/// `EMBED_MAX_TOKENS`'s doc comment for why this is a re-export, not a copy.
pub const SPECIAL_TOKEN_RESERVE: usize = crate::memory::embedder::SPECIAL_TOKEN_RESERVE;

/// Delegates to `memory::query_context::compose_query_context` — one source
/// of truth for both production and this harness, so the `--follow-up`
/// treatment arm composes its query exactly the way production does, never a
/// reimplementation that could quietly diverge from it.
pub fn compose_query_context(
    current: &str,
    prior_turns_newest_first: &[String],
    budget_tokens: usize,
    count_tokens: &dyn Fn(&str) -> usize,
) -> String {
    crate::memory::query_context::compose_query_context(
        current,
        prior_turns_newest_first,
        budget_tokens,
        count_tokens,
    )
}

/// Delegates to `memory::query_context::query_budget_tokens` — same
/// one-source-of-truth reasoning as [`compose_query_context`].
pub fn query_budget_tokens(max_tokens: usize, reserve: usize, prefix_tokens: usize) -> usize {
    crate::memory::query_context::query_budget_tokens(max_tokens, reserve, prefix_tokens)
}

/// A token counter for the eval binary's own query-context composition —
/// the same role the counter closure plays inside `compute_memory_prefix` in
/// production, wrapped in a named type so callers outside this module never
/// need to name `tokenizers::Tokenizer` directly (this module's whole reason
/// to exist — see the module doc comment).
pub struct EvalTokenCounter(EvalTokenCounterInner);

enum EvalTokenCounterInner {
    Real(std::sync::Arc<tokenizers::Tokenizer>),
    Fake,
}

impl EvalTokenCounter {
    /// Loads the SAME `tokenizer.json` production loads
    /// (`model_download::tokenizer_path`, via `model_download::load_tokenizer`),
    /// so a `--real --follow-up` run's budget matches exactly what a real
    /// chat turn would compute — never a second, independently-loaded copy.
    pub fn real(base_dir: &Path) -> Result<Self, EvalError> {
        model_download::load_tokenizer(base_dir)
            .map(|tok| Self(EvalTokenCounterInner::Real(tok)))
            .ok_or_else(|| {
                EvalError::ModelLoad(format!(
                    "tokenizer.json not loadable under {}",
                    base_dir.display()
                ))
            })
    }

    /// Whitespace-word counting — documented as plumbing-only, the same
    /// caveat [`EvalEmbedder::fake`] already carries: no learned
    /// tokenization, just enough of a counter shape to exercise the
    /// composition code path without downloading the real model.
    pub fn fake() -> Self {
        Self(EvalTokenCounterInner::Fake)
    }

    /// Counts `s`. Encode failure on the real path admits nothing
    /// (`usize::MAX`) — the same fail-toward-"no extra context" rule
    /// production's own counter closure follows.
    pub fn count(&self, s: &str) -> usize {
        match &self.0 {
            EvalTokenCounterInner::Real(tok) => {
                tok.encode(s, false).map(|e| e.len()).unwrap_or(usize::MAX)
            }
            EvalTokenCounterInner::Fake => s.split_whitespace().count(),
        }
    }
}

/// True iff the real embedding model is already cached at `base_dir` —
/// never triggers a download. Delegates to the module that owns the
/// cache-dir layout and the on-disk verification, so this check can never
/// drift from what a real load would accept.
pub fn model_is_cached(base_dir: &Path) -> bool {
    model_download::model_is_cached(base_dir)
}

/// Cosine similarity of two vectors, computed directly rather than recovered
/// from an L2 distance. The single definition shared by every binary that
/// compares embeddings offline: `golden-mine` uses it to mine hard
/// distractors, `rag-eval` uses it to rank candidates for the similarity
/// measurement block, and a statistic quoted from one is only comparable to a
/// statistic quoted from the other because both ran through this exact
/// function.
pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut norm_a = 0.0f64;
    let mut norm_b = 0.0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += f64::from(*x) * f64::from(*y);
        norm_a += f64::from(*x) * f64::from(*x);
        norm_b += f64::from(*y) * f64::from(*y);
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

/// A project's documentation index over a store of its own, with no daemon
/// and no registered project: it measures the index, not the plumbing.
pub struct EvalDocIndex {
    store: MemoryStore,
    embedder: Arc<dyn Embedder>,
    splitter: TokenSplitter,
}

/// What indexing a root found and wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EvalDocReport {
    pub docs: usize,
    pub passages: usize,
    pub skipped: usize,
}

/// One passage a doc search returned, ranked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalDocHit {
    pub path: String,
    pub breadcrumb: String,
}

const EVAL_DOCS_PROJECT: &str = "rag-eval-docs";

impl EvalDocIndex {
    /// Opens a store rooted at `base_dir`. `tokenizer_dir` plays the same
    /// part as in [`EvalIngest::open`]: passages are capped by the real
    /// tokenizer when it holds one, by bytes otherwise.
    pub async fn open(base_dir: &Path, embedder: &EvalEmbedder, tokenizer_dir: &Path) -> Result<Self, EvalError> {
        let store = MemoryStore::open(base_dir, StoreHolder::Eval)
            .await
            .map_err(|e| EvalError::Open(e.to_string()))?;
        let splitter = match model_download::load_tokenizer(tokenizer_dir) {
            Some(tok) => TokenSplitter::with_tokenizer(tok),
            None => TokenSplitter::fallback(),
        };
        Ok(Self { store, embedder: embedder.0.clone(), splitter })
    }

    pub async fn index(&self, root: &Path) -> Result<EvalDocReport, EvalError> {
        let r = project_docs::refresh(&self.store, self.embedder.as_ref(), &self.splitter, EVAL_DOCS_PROJECT, root)
            .await
            .map_err(|e| EvalError::Insert(e.to_string()))?;
        Ok(EvalDocReport { docs: r.docs, passages: r.passages, skipped: r.skipped })
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<EvalDocHit>, EvalError> {
        let hits = project_docs::search(&self.store, self.embedder.as_ref(), EVAL_DOCS_PROJECT, query, limit)
            .await
            .map_err(|e| EvalError::Search(e.to_string()))?;
        Ok(hits.into_iter().map(|h| EvalDocHit { path: h.path, breadcrumb: h.breadcrumb }).collect())
    }
}

/// Whether the index takes the file at `path` as source code.
pub fn is_code_path(path: &str) -> bool {
    matches!(project_docs::corpus::classify(path), Some(project_docs::corpus::DocKind::Code(_)))
}

/// The relative paths of the docs `root` holds, as the index would take them.
pub fn doc_paths(root: &Path) -> Result<Vec<String>, EvalError> {
    let walk = project_docs::corpus::walk(root).map_err(|e| EvalError::Open(e.to_string()))?;
    Ok(walk.docs.into_iter().map(|d| d.path).collect())
}

/// Hex-lowercase SHA-256 of `bytes`. Shared by every binary that needs to
/// stamp a file's exact content into a report: hashing the same bytes twice
/// through two separate implementations would make "do these two runs agree
/// on which golden set they scored" a question about two unverified digest
/// functions instead of one.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut s = String::with_capacity(out.len() * 2);
    for b in out {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn into_search_scope_maps_lexical_through_unchanged() {
        // Guards the last border crossing before `SearchScope`: a mapping
        // hardcoded to `None` here would switch the lexical channel off for
        // every eval run while the whole sweep plumbing upstream kept
        // carrying its value correctly. Nothing else in the eval binary
        // observes `SearchScope.lexical` directly, so this is the only place
        // that can catch it.
        let on = EvalScope { memory: Some(30), episodic: Some(5), procedural: Some(5), lexical: Some(20) };
        assert_eq!(on.into_search_scope().lexical, Some(20));

        let off = EvalScope { memory: Some(30), episodic: Some(5), procedural: Some(5), lexical: None };
        assert_eq!(off.into_search_scope().lexical, None);
    }

    /// `EvalScope.lexical` reaching `SearchScope` is only half of what the
    /// eval instrument needs: the channel also has to be handed the query
    /// TEXT, which travels as a separate argument. Dropping that argument on
    /// the floor here leaves every scope-mapping assertion above green while
    /// making `--sweep-lexical 0` and `--sweep-lexical 20` produce identical
    /// numbers — a measuring instrument that reports the same reading for
    /// both settings of the thing it measures.
    ///
    /// The fixture makes the lexical channel the ONLY way M can be returned:
    /// 30 rows sit exactly on the query vector and fill the memory pool, so M
    /// (cosine 0.995) is rank 31 and never enters the dense pool; M then wins
    /// its way into the top 10 on importance, which the 30 fillers give up.
    #[tokio::test]
    async fn search_feeds_query_text_to_the_lexical_channel() {
        let dir = tempfile::tempdir().unwrap();
        let store = EvalStore::open(dir.path()).await.expect("open must succeed");
        let agent = "agent-query-text-wiring";

        let mut on_axis = vec![0.0_f32; 384];
        on_axis[0] = 1.0;
        // cosine 0.995 to the query: outside the pool the 30 exact matches
        // fill, but well above `cosine_floor` (0.8793), so the similarity term
        // survives the rescale with room to be outweighed by importance.
        let mut off_axis = vec![0.0_f32; 384];
        off_axis[0] = 0.995;
        off_axis[1] = (1.0_f32 - 0.995 * 0.995).sqrt();

        for i in 0..30 {
            store
                .insert(agent, &format!("filler row {i}"), "chat_message", 1.0, on_axis.clone())
                .await
                .unwrap();
        }
        let m_id = store
            .insert(
                agent,
                "the rollout checklist lives under ZQXIDENT42",
                "chat_message",
                10.0,
                off_axis,
            )
            .await
            .unwrap();

        let scope = EvalScope {
            memory: Some(30),
            episodic: None,
            procedural: None,
            lexical: Some(20),
        };
        let scoring = EvalScoringConfig::default();

        let with_text = store
            .search(&on_axis, agent, scope, 10, scoring, Some("ZQXIDENT42"))
            .await
            .unwrap();
        assert!(
            with_text.iter().any(|h| h.id == m_id),
            "M is reachable only through the lexical channel, so its absence means the query \
             text never reached it: {:?}",
            with_text.iter().map(|h| h.id).collect::<Vec<_>>()
        );

        let without_text = store.search(&on_axis, agent, scope, 10, scoring, None).await.unwrap();
        assert!(
            !without_text.iter().any(|h| h.id == m_id),
            "the control must miss M — otherwise the assertion above proves nothing about the \
             lexical channel"
        );
    }

    // --- recreate_fts_with_tokenizer / tokenizer-diff guard -----------------

    #[tokio::test]
    async fn recreate_fts_with_tokenizer_same_tokenizer_matches_the_migrated_schema_byte_for_byte() {
        let dir_a = tempfile::tempdir().unwrap();
        let store_a = EvalStore::open(dir_a.path()).await.expect("open must succeed");
        let migrated_sql = store_a.fts_schema_sql().await;

        let dir_b = tempfile::tempdir().unwrap();
        let store_b = EvalStore::open(dir_b.path()).await.expect("open must succeed");
        store_b
            .recreate_fts_with_tokenizer("unicode61 remove_diacritics 2")
            .await
            .expect("recreate with the migration's own tokenizer must succeed");
        let rebuilt_same_sql = store_b.fts_schema_sql().await;

        assert_eq!(
            migrated_sql, rebuilt_same_sql,
            "rebuilding with the exact tokenizer the migration used must produce byte-identical DDL"
        );
    }

    #[tokio::test]
    async fn recreate_fts_with_tokenizer_trigram_differs_only_in_the_tokenize_clause() {
        let dir_a = tempfile::tempdir().unwrap();
        let store_a = EvalStore::open(dir_a.path()).await.expect("open must succeed");
        let migrated_sql = store_a.fts_schema_sql().await;

        let dir_c = tempfile::tempdir().unwrap();
        let store_c = EvalStore::open(dir_c.path()).await.expect("open must succeed");
        store_c.recreate_fts_with_tokenizer("trigram").await.expect("recreate with trigram must succeed");
        let rebuilt_trigram_sql = store_c.fts_schema_sql().await;

        assert_eq!(
            migrated_sql.replace("unicode61 remove_diacritics 2", "trigram"),
            rebuilt_trigram_sql,
            "a direct substring substitution of the tokenizer clause must reproduce the rebuilt DDL exactly — \
             any other divergence means the rebuild's formatting drifted from the migration's"
        );
    }

    #[tokio::test]
    async fn recreate_fts_with_tokenizer_rejects_an_unlisted_tokenizer() {
        let dir = tempfile::tempdir().unwrap();
        let store = EvalStore::open(dir.path()).await.expect("open must succeed");
        match store.recreate_fts_with_tokenizer("porter").await {
            Err(EvalError::UnknownTokenizer(t)) => assert_eq!(t, "porter"),
            other => panic!("expected UnknownTokenizer, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn recreate_fts_with_tokenizer_indexes_rows_inserted_after_the_swap() {
        // Confirms no backfill is needed: the AFTER INSERT
        // trigger must survive the DROP TABLE, so a row inserted after the
        // swap is still lexically findable with no extra step.
        let dir = tempfile::tempdir().unwrap();
        let store = EvalStore::open(dir.path()).await.expect("open must succeed");
        store.recreate_fts_with_tokenizer("trigram").await.expect("recreate must succeed");

        let mut embedding = vec![0.0; 384];
        embedding[0] = 1.0;
        let id = store
            .insert("agent-1", "unique_token_after_tokenizer_swap", "chat_message", 3.0, embedding)
            .await
            .expect("insert must succeed");

        let found: i64 = store
            .0
            .conn
            .call(|c| {
                c.query_row(
                    "SELECT rowid FROM memory_fts WHERE memory_fts MATCH 'uni' LIMIT 1",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .expect("trigram tokenizer must find a 3-char substring of the inserted content");
        assert_eq!(found, id);
    }

    #[test]
    fn eval_scoring_config_default_matches_scoring_config_default_field_for_field() {
        assert_eq!(
            EvalScoringConfig::default().into_scoring_config(),
            crate::memory::score::ScoringConfig::DEFAULT,
            "the sweep's implicit zero point must reproduce today's baseline exactly"
        );
    }

    #[test]
    fn with_importance_span_overrides_only_the_importance_fields() {
        let base = EvalScoringConfig::default();
        let spanned = base.with_importance_span(0.7, 1.0);
        assert_ne!(spanned.importance_base, base.importance_base);
        assert_ne!(spanned.importance_divisor, base.importance_divisor);
        assert_eq!(spanned.importance_clamp_lo, base.importance_clamp_lo);
        assert_eq!(spanned.importance_clamp_hi, base.importance_clamp_hi);
        assert_eq!(spanned.cosine_floor, base.cosine_floor, "must not touch the unrelated axis");
    }

    #[test]
    fn with_cosine_floor_overrides_only_the_cosine_floor() {
        let base = EvalScoringConfig::default();
        let floored = base.with_cosine_floor(0.8625);
        assert_eq!(floored.cosine_floor, Some(0.8625));
        assert_eq!(floored.importance_base, base.importance_base, "must not touch the unrelated axis");
    }

    #[test]
    fn cosine_of_identical_vectors_is_one() {
        assert!((cosine(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn cosine_of_orthogonal_vectors_is_zero() {
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-12);
    }

    /// `EvalEmbedder::embed_with_role` must dispatch straight through to the
    /// production trait's `embed_with_role` — mechanical pass-through, no
    /// logic of its own to get wrong beyond wiring the role through.
    #[tokio::test]
    async fn embed_with_role_dispatches_role_to_the_inner_embedder() {
        let e = EvalEmbedder::fake();
        let q = e.embed_with_role("hola", EmbedRole::Query).await.unwrap();
        let p = e.embed_with_role("hola", EmbedRole::Passage).await.unwrap();
        assert_eq!(
            q, p,
            "FakeEmbedder ignores role deterministically; both roles must pass through identically"
        );
        assert_eq!(q.len(), e.dimensions());
    }

    // --- compose_query_context / query_budget_tokens: delegation, not reimplementation ---

    #[test]
    fn compose_query_context_delegates_to_the_memory_module() {
        // A case sensitive to ordering AND to the separator being charged —
        // if this module ever grew its own reimplementation instead of
        // delegating, drifting on either detail would leave this test the
        // only place that would notice.
        let counter = |s: &str| s.split_whitespace().count();
        let out = compose_query_context(
            "current message",
            &["prior turn text".to_string()],
            100,
            &counter,
        );
        assert_eq!(out, "current message\nprior turn text");
    }

    #[test]
    fn compose_query_context_empty_prior_turns_is_the_current_message_unchanged() {
        let counter = |s: &str| s.split_whitespace().count();
        assert_eq!(compose_query_context("hello world", &[], 100, &counter), "hello world");
    }

    #[test]
    fn query_budget_tokens_delegates_and_subtracts_both_terms() {
        assert_eq!(query_budget_tokens(512, 4, 2), 506);
    }

    /// The two constants are re-exports, not copies — the whole point is that
    /// a `--follow-up` run's budget reads the SAME number the live embedder
    /// truncates to. The delegating FUNCTIONS above are covered by their own
    /// behavior; nothing else would notice these two silently becoming
    /// hardcoded literals that drift from `memory::embedder`.
    #[test]
    fn the_embedder_window_constants_are_re_exports_not_second_copies() {
        assert_eq!(EMBED_MAX_TOKENS, crate::memory::embedder::EMBED_MAX_TOKENS);
        assert_eq!(SPECIAL_TOKEN_RESERVE, crate::memory::embedder::SPECIAL_TOKEN_RESERVE);
    }

    // --- EvalTokenCounter ----------------------------------------------------

    #[test]
    fn eval_token_counter_fake_counts_whitespace_words() {
        let counter = EvalTokenCounter::fake();
        assert_eq!(counter.count("hello world"), 2);
        assert_eq!(counter.count(""), 0);
        assert_eq!(counter.count("one two three four"), 4);
    }

    /// `real` must load the exact file `model_download::tokenizer_path`
    /// resolves to — proven by writing a real, minimal tokenizer there and
    /// confirming `count` genuinely tokenizes (not falls back to a word
    /// heuristic under a different name).
    #[test]
    fn eval_token_counter_real_loads_the_same_file_production_loads() {
        use tokenizers::models::wordlevel::WordLevel;
        use tokenizers::pre_tokenizers::whitespace::Whitespace;

        let base_dir = std::env::temp_dir().join(format!(
            "clawg-eval-token-counter-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let target = crate::memory::model_download::tokenizer_path(&base_dir);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let vocab_path = target.parent().unwrap().join("vocab.json");
        std::fs::write(&vocab_path, r#"{"[UNK]":0,"hello":1,"world":2}"#).unwrap();
        let model = WordLevel::from_file(vocab_path.to_str().unwrap(), "[UNK]".to_string()).unwrap();
        let mut tok = tokenizers::Tokenizer::new(model);
        tok.with_pre_tokenizer(Some(Whitespace));
        tok.save(&target, false).unwrap();

        let counter = EvalTokenCounter::real(&base_dir).expect("tokenizer.json is present at the exact path expected");
        assert_eq!(counter.count("hello world"), 2, "must genuinely tokenize via the loaded file, not fall back silently");

        std::fs::remove_dir_all(&base_dir).ok();
    }

    #[test]
    fn eval_token_counter_real_errs_when_no_tokenizer_is_present() {
        let base_dir = std::env::temp_dir().join(format!(
            "clawg-eval-token-counter-test-absent-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        assert!(EvalTokenCounter::real(&base_dir).is_err());
    }

    // --- EvalIngest ----------------------------------------------------------

    /// `(content, memory_type, importance, tier, access_count)`.
    type StoredRow = (String, String, f64, i64, i64);

    async fn stored_rows(runtime: &MemoryRuntime) -> Vec<StoredRow> {
        runtime
            .store
            .conn
            .call(|c| -> rusqlite::Result<Vec<StoredRow>> {
                let mut stmt = c.prepare(
                    "SELECT content, memory_type, importance, tier, access_count FROM memory ORDER BY id",
                )?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
            .expect("select rows")
    }

    #[tokio::test]
    async fn ingest_finish_returns_only_after_every_event_is_stored() {
        let dir = tempfile::tempdir().unwrap();
        let ingest = EvalIngest::open(dir.path(), &EvalEmbedder::fake(), dir.path()).await.unwrap();
        for i in 0..5 {
            ingest
                .send("agent-ingest", &format!("distinct memory number {i} about the ingest drain"), "chat_message", 3.0)
                .await
                .unwrap();
        }

        let runtime = ingest.finish().await.expect("finish must drain the worker");

        assert_eq!(
            stored_rows(&runtime).await.len(),
            5,
            "finish() is the drained signal: every queued event must already be a row when it returns"
        );
    }

    #[tokio::test]
    async fn ingest_honours_the_declared_type_and_importance() {
        let dir = tempfile::tempdir().unwrap();
        let ingest = EvalIngest::open(dir.path(), &EvalEmbedder::fake(), dir.path()).await.unwrap();
        ingest.send("agent-ingest", "always answer in rioplatense spanish", "user_preference", 9.5).await.unwrap();

        let runtime = ingest.finish().await.unwrap();

        let rows = stored_rows(&runtime).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "user_preference");
        assert_eq!(rows[0].2, 9.5);
        assert_eq!(rows[0].3, 1, "importance >= 9 on a user_preference is the tier-1 fast track");
    }

    #[tokio::test]
    async fn ingest_rejects_an_unknown_memory_type_before_queueing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let ingest = EvalIngest::open(dir.path(), &EvalEmbedder::fake(), dir.path()).await.unwrap();

        match ingest.send("agent-ingest", "content", "not_a_real_type", 3.0).await {
            Err(EvalError::UnknownMemoryType(t)) => assert_eq!(t, "not_a_real_type"),
            other => panic!("expected UnknownMemoryType, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ingest_runtime_is_rooted_at_the_given_base_dir() {
        let dir = tempfile::tempdir().unwrap();
        let ingest = EvalIngest::open(dir.path(), &EvalEmbedder::fake(), dir.path()).await.unwrap();

        let runtime = ingest.finish().await.unwrap();

        assert_eq!(
            runtime.base_dir,
            dir.path(),
            "the workspace tree hangs off this directory, so it is what keeps every write inside the caller's temp dir"
        );
        assert!(runtime.embedder_ready(), "the query side needs the same embedder that stored the rows");
    }

    /// 600 ids is past the tracker's 500-entry size trigger, so a LIVE tracker
    /// flushes at once and bumps `access_count` on the spot — no 30 s wait
    /// needed to tell the two apart.
    #[tokio::test]
    async fn ingest_runtime_never_records_access() {
        let dir = tempfile::tempdir().unwrap();
        let ingest = EvalIngest::open(dir.path(), &EvalEmbedder::fake(), dir.path()).await.unwrap();
        ingest.send("agent-ingest", "a single row whose access count must not move", "chat_message", 3.0).await.unwrap();
        let runtime = ingest.finish().await.unwrap();
        let id: i64 = runtime
            .store
            .conn
            .call(|c| c.query_row("SELECT id FROM memory LIMIT 1", [], |r| r.get(0)))
            .await
            .unwrap();

        let before = stored_rows(&runtime).await;
        runtime.access_tracker.record(vec![id; 600]);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert_eq!(
            stored_rows(&runtime).await,
            before,
            "a measurement run must not move the rows it measures"
        );
    }

    #[test]
    fn sha256_hex_of_a_known_fixture_matches_the_pinned_digest() {
        // sha256("hello world") — the canonical example digest, so this test
        // fails loudly (a wrong-but-plausible hex string) if the hashing or
        // hex-encoding ever regresses, rather than only failing when nothing
        // else can compute the reference value.
        assert_eq!(
            sha256_hex(b"hello world"),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[tokio::test]
    async fn a_doc_index_answers_from_the_root_it_indexed() {
        let store_dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("README.md"), "# Guía\n## Instalación\nCompilá con cargo build.\n## Uso\nArrancá el daemon.\n").unwrap();
        std::fs::write(root.path().join("notes.txt"), "not a doc").unwrap();

        let index = EvalDocIndex::open(store_dir.path(), &EvalEmbedder::fake(), store_dir.path()).await.unwrap();
        let report = index.index(root.path()).await.unwrap();
        let hits = index.search("daemon", 5).await.unwrap();

        assert_eq!(report, EvalDocReport { docs: 1, passages: 2, skipped: 0 });
        assert_eq!(doc_paths(root.path()).unwrap(), ["README.md"]);
        assert!(is_code_path("src/lib.rs") && !is_code_path("README.md"));
        assert_eq!(hits[0], EvalDocHit { path: "README.md".into(), breadcrumb: "README.md > Guía > Uso".into() });
    }
}
