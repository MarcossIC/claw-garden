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

use serde::Serialize;

use crate::memory::db::MemoryStore;
use crate::memory::embedder::{Embedder, FakeEmbedder, FastEmbedLocal};

/// Re-exported so the `rag-eval` and `golden-mine` binaries can name a role
/// without reaching into the private `memory` module tree.
pub use crate::memory::embedder::EmbedRole;
use crate::memory::events::MemorySource;
use crate::memory::model_download;
use crate::memory::score::{ImportanceFactor, ScoringConfig};
use crate::memory::search::{search_all_memories_with_scoring, SearchScope};
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
            // rag-eval never carries a project — its whole corpus is one
            // unscoped pool, so this keeps every query bit-identical to
            // retrieval before project scoping existed.
            project_id: None,
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
        scoring: EvalScoringConfig,
    ) -> Result<Vec<EvalHit>, EvalError> {
        let hits = search_all_memories_with_scoring(
            &self.0,
            embedding,
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
}

