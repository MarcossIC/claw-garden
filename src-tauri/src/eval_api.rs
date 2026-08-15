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

use crate::memory::db::MemoryStore;
use crate::memory::embedder::{Embedder, FakeEmbedder, FastEmbedLocal};
use crate::memory::events::MemorySource;
use crate::memory::model_download;
use crate::memory::search::{search_all_memories, SearchScope};
use crate::memory::store_insert::{CreatedByType, MemoryType, NewMemoryRecord};
use crate::memory::tier::compute_tier;
use crate::memory::TableKind;

/// Opaque handle to a memory store. Every operation available on it is a
/// method below — the wrapped store itself is never reachable.
pub struct EvalStore(MemoryStore);

/// Per-table result-pool sizes for a search call — this module's own copy of
/// the store's scope shape, so a change to the internal one cannot silently
/// change what this tool asks for.
#[derive(Debug, Clone, Copy)]
pub struct EvalScope {
    pub memory: Option<usize>,
    pub episodic: Option<usize>,
    pub procedural: Option<usize>,
}

impl EvalScope {
    fn into_search_scope(self) -> SearchScope {
        SearchScope {
            memory: self.memory,
            episodic: self.episodic,
            procedural: self.procedural,
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
}

impl EvalStore {
    /// Opens (creating if absent) a store rooted at `base_dir`, migrations
    /// included — the same path production startup takes.
    pub async fn open(base_dir: &Path) -> Result<Self, EvalError> {
        MemoryStore::open(base_dir)
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
        };
        self.0
            .insert_memory_with_vector(record, embedding)
            .await
            .map_err(|e| EvalError::Insert(e.to_string()))
    }

    /// KNN search across all three tables, translated into this module's own
    /// hit shape.
    pub async fn search(
        &self,
        embedding: &[f32],
        agent_id: &str,
        scope: EvalScope,
        k_final: usize,
    ) -> Result<Vec<EvalHit>, EvalError> {
        let hits = search_all_memories(
            &self.0,
            embedding,
            agent_id,
            scope.into_search_scope(),
            k_final,
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
pub struct EvalEmbedder(Box<dyn Embedder>);

impl EvalEmbedder {
    /// A deterministic double: the same text always yields the same vector,
    /// with no model to load and no network involved. Its vectors are NOT
    /// L2-normalized (`‖v‖ ≈ 11.3` rather than 1.0), so distances measured
    /// against it are not on the same scale as distances measured against
    /// the real model — a run against this embedder validates plumbing and
    /// metric code, not retrieval quality.
    pub fn fake() -> Self {
        EvalEmbedder(Box::new(FakeEmbedder::default()))
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
        Ok(EvalEmbedder(Box::new(inner)))
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>, EvalError> {
        self.0
            .embed(text)
            .await
            .map_err(|e| EvalError::Embed(e.to_string()))
    }

    pub fn dimensions(&self) -> usize {
        self.0.dimensions()
    }

    pub fn model_id(&self) -> &str {
        self.0.model_id()
    }
}

/// True iff the real embedding model is already cached at `base_dir` —
/// never triggers a download. Delegates to the module that owns the
/// cache-dir layout and the on-disk verification, so this check can never
/// drift from what a real load would accept.
pub fn model_is_cached(base_dir: &Path) -> bool {
    model_download::model_is_cached(base_dir)
}

