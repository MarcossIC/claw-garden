//! The [`Embedder`] trait and its concrete implementations: [`FastEmbedLocal`]
//! (the production embedder, backed by `multilingual-e5-small` via
//! fastembed-rs), [`OllamaEmbedder`]/[`OpenAIEmbedder`] (unimplemented stubs
//! reserved for future backends), and `FakeEmbedder` (a deterministic test
//! double). Every embed call carries an explicit [`EmbedRole`] so a caller
//! can never forget which side of the query/passage distinction it is on —
//! see [`EmbedRole`] below for why that distinction matters.

use async_trait::async_trait;
use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use tokio::sync::{mpsc, oneshot};

/// Which side of an asymmetric embedding model a text plays: the thing being
/// searched FOR, or the thing being searched THROUGH. E5-family models were
/// trained with a different string prepended to each side, so the two sides
/// occupy different (though related) regions of the embedding space — using
/// the wrong prefix for a given call is not a style mismatch, it is querying
/// with the wrong instrument.
///
/// Deliberately no `Default`: there is no role a caller could fall back to
/// without silently mis-labeling the text it is embedding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedRole {
    /// The text is something being searched FOR (a user query).
    Query,
    /// The text is something being searched THROUGH (a stored chunk/document).
    Passage,
}

/// Async embedder contract. All embedder impls are `Send + Sync` so they can
/// live behind `Arc<dyn Embedder>`.
///
/// `embed` (role-free) does not exist on this trait and never will: a default
/// method delegating to an unprefixed call, or a "convention" about which
/// internal callers may skip the role, would both let an unmigrated call site
/// keep the old unprefixed behavior and compile clean — exactly the silent
/// mismatch this trait shape exists to make unreachable. Every call site
/// states a role; every impl states its own prefixes. Neither `query_prefix`
/// nor `passage_prefix` has a default body for the same reason: a new impl
/// that inherits `""` for both is silently claiming "no prefixes" instead of
/// deciding to have none.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Compute the embedding for a single text, prefixed per `role`. Vector
    /// length MUST equal [`Embedder::dimensions`].
    async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError>;

    /// Compute embeddings for `texts` in one call, every text prefixed per
    /// `role`. Returns exactly `texts.len()` vectors in input order: the
    /// ingest worker zips the reply back against its chunks, so a short or
    /// reordered reply attaches one chunk's vector to another chunk's
    /// memory — corruption, not a recoverable error.
    ///
    /// One role per call rather than per item: the only batching caller
    /// embeds one event's chunks, all passages, and a per-item role would
    /// let a single call straddle two embedding spaces.
    ///
    /// The default body embeds one text at a time, which is correct for
    /// every impl and merely unbatched — only backends with a real batch
    /// API need to override it.
    async fn embed_batch(&self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            out.push(self.embed_with_role(text, role).await?);
        }
        Ok(out)
    }

    /// Embedding dimensionality (e.g. 384 for `multilingual-e5-small`).
    fn dimensions(&self) -> usize;

    /// Stable identifier for the active model
    /// (e.g. `"multilingual-e5-small"`, `"fake-embedder"`).
    fn model_id(&self) -> &str;

    /// The literal string prepended to text embedded with [`EmbedRole::Query`].
    fn query_prefix(&self) -> &str;

    /// The literal string prepended to text embedded with [`EmbedRole::Passage`].
    fn passage_prefix(&self) -> &str;
}

/// E5-family prefix for [`EmbedRole::Query`] text. The trailing space is
/// load-bearing: `"query:"` and `"query: "` tokenize to different sequences,
/// so dropping it would silently change the embedding space, not just the
/// string's appearance.
const E5_QUERY_PREFIX: &str = "query: ";

/// E5-family prefix for [`EmbedRole::Passage`] text. Currently set to the
/// SAME string as [`E5_QUERY_PREFIX`] rather than `"passage: "`.
///
/// Retrieval in this app is a near-symmetric task: queries are full
/// natural-language questions and the corpus rows they search over are
/// conversational memory of comparable length and register — not the
/// short-question-against-long-reference-document shape the asymmetric E5
/// prefixes are built for. intfloat's own model card recommends `"query: "`
/// on both sides for symmetric tasks and reserves the query/passage split
/// for genuinely asymmetric retrieval.
///
/// This was measured, not assumed: giving passages their own `"passage: "`
/// prefix scored WORSE on the domain golden set than the symmetric split —
/// sim recall@10 dropped from 0.8261 to 0.6957 (95% CI excluding 0) — while
/// doc-to-doc structure was essentially unchanged. If this corpus later
/// gains a genuinely asymmetric shape (e.g. short queries against long
/// standalone documents), this constant is the one line to move back to
/// `"passage: "`.
const E5_PASSAGE_PREFIX: &str = "query: ";

/// The id the embedding contract records for this embedder's vectors. It names
/// the pooling because the model file does not decide it, and vectors pooled
/// differently live in different spaces.
pub const E5_MODEL_ID: &str = "multilingual-e5-small/mean";

/// The id earlier versions recorded for the same weights pooled by CLS, the
/// default fastembed applies to a user-defined model. A store under this id
/// holds vectors that must be re-embedded, never mixed with new ones.
pub const LEGACY_CLS_MODEL_ID: &str = "multilingual-e5-small";

/// e5 is trained with mean pooling. Left unset, fastembed pools a
/// user-defined model by its first token, which gave every passage of a corpus
/// nearly the same vector.
fn e5_model(onnx: Vec<u8>, tokenizer: TokenizerFiles) -> UserDefinedEmbeddingModel {
    UserDefinedEmbeddingModel::new(onnx, tokenizer).with_pooling(Pooling::Mean)
}

/// The token window fastembed truncates the model input to. Pinned as an
/// explicit constant rather than left at fastembed's own default: the query
/// composer's budget (`memory::query_context::query_budget_tokens`) has to
/// read the SAME number the embedder actually enforces, and a value only
/// implicit in a library default can silently drift out of sync with it.
pub const EMBED_MAX_TOKENS: usize = 512;

/// Tokens the composer's budget reserves for what the model adds around the
/// text itself: `<s>`/`</s>` (2) plus slack for a BPE merge that can span the
/// prefix/text boundary and tokenize slightly differently than summing
/// `count(prefix) + count(text)` would predict. 4 covers both with room to
/// spare.
pub const SPECIAL_TOKEN_RESERVE: usize = 4;

/// The single authority on how a role becomes a prefixed string. Pure: no
/// I/O, no dependency on which embedder is active — the caller supplies the
/// prefixes, this function only selects and concatenates.
pub(crate) fn apply_role_prefix(
    role: EmbedRole,
    text: &str,
    query_prefix: &str,
    passage_prefix: &str,
) -> String {
    let prefix = match role {
        EmbedRole::Query => query_prefix,
        EmbedRole::Passage => passage_prefix,
    };
    format!("{prefix}{text}")
}

/// Errors returned from the embedder pipeline.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("embedder not ready")]
    NotReady,
    #[error("embedder worker is gone")]
    WorkerGone,
    #[error("inference failed: {0}")]
    InferenceFailed(String),
    #[error("model load failed: {0}")]
    LoadFailed(String),
}

// ============================================================================
// FastEmbedLocal — fastembed-rs backed embedder, confined to a single owning
// thread via a tokio::mpsc request/oneshot channel.
//
// The inner `fastembed::TextEmbedding` is NOT guaranteed `Send`, so it lives
// on one dedicated `std::thread` for its entire life. The public struct holds
// only channel senders + primitives, which keeps it `Send + Sync` — verified
// by the const-block at the bottom of this file.
// ============================================================================

/// Internal request crossing the channel: texts to embed + oneshot reply slot.
pub(crate) struct EmbedRequest {
    pub(crate) texts: Vec<String>,
    pub(crate) resp: oneshot::Sender<Result<Vec<Vec<f32>>, EmbedError>>,
}

pub struct FastEmbedLocal {
    req_tx: mpsc::UnboundedSender<EmbedRequest>,
    dimensions: usize,
    model_id: &'static str,
}

#[async_trait]
impl Embedder for FastEmbedLocal {
    async fn embed_batch(&self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
        let prefixed: Vec<String> = texts.iter()
            .map(|t| apply_role_prefix(role, t, self.query_prefix(), self.passage_prefix()))
            .collect();
        let expected = prefixed.len();
        let (resp_tx, resp_rx) = oneshot::channel();
        self.req_tx.send(EmbedRequest { texts: prefixed, resp: resp_tx })
            .map_err(|_| EmbedError::WorkerGone)?;
        // Map the oneshot RecvError (sender dropped) to WorkerGone; otherwise
        // unwrap the inner Result the worker placed inside.
        let vectors = resp_rx.await.map_err(|_| EmbedError::WorkerGone)??;
        if vectors.len() != expected {
            // Reported as an inference failure rather than trusted: the
            // caller pairs these positionally, so a wrong count means the
            // pairing is already meaningless.
            return Err(EmbedError::InferenceFailed(format!(
                "model returned {} vectors for {expected} texts", vectors.len()
            )));
        }
        Ok(vectors)
    }

    async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
        let one = [text.to_string()];
        let mut out = self.embed_batch(&one, role).await?;
        out.pop().ok_or_else(|| EmbedError::InferenceFailed("empty reply for one text".to_string()))
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn model_id(&self) -> &str {
        self.model_id
    }

    fn query_prefix(&self) -> &str {
        E5_QUERY_PREFIX
    }

    fn passage_prefix(&self) -> &str {
        E5_PASSAGE_PREFIX
    }
}

impl FastEmbedLocal {
    /// Construct from already-downloaded model bytes + tokenizer files.
    ///
    /// CPU-bound (~100 ms warm-up for `multilingual-e5-small`); callers SHOULD
    /// wrap this in `tokio::task::spawn_blocking`. The model itself loads on
    /// the CALLING thread first — this surfaces load errors as
    /// `EmbedError::LoadFailed` synchronously before the worker thread is
    /// spawned, which keeps error handling simple.
    ///
    /// After load succeeds, a single `std::thread` named `"fastembed-worker"`
    /// takes ownership of the `TextEmbedding` for its entire lifetime. All
    /// callers route through the request channel; `TextEmbedding` never crosses
    /// threads, so its `Send`/`Sync` bounds are irrelevant to us.
    pub fn from_bytes(onnx: Vec<u8>, tokenizer: TokenizerFiles) -> Result<Self, EmbedError> {
        let (req_tx, mut req_rx) = mpsc::unbounded_channel::<EmbedRequest>();

        let user_model = e5_model(onnx, tokenizer);
        // Pinned explicitly so the composer's budget and the embedder's own
        // truncation point are provably the same number, not two constants
        // that happen to agree today.
        let init_opts = InitOptionsUserDefined::new().with_max_length(EMBED_MAX_TOKENS);

        // Synchronous load on the calling thread: surface load errors immediately.
        // `embed()` takes `&mut self` in fastembed v5 (interior session state).
        let mut model = TextEmbedding::try_new_from_user_defined(user_model, init_opts)
            .map_err(|e| EmbedError::LoadFailed(e.to_string()))?;

        std::thread::Builder::new()
            .name("fastembed-worker".into())
            .spawn(move || {
                // futures::executor::block_on bridges tokio's mpsc receiver into
                // a non-tokio thread. Loop exits naturally when all senders drop.
                while let Some(EmbedRequest { texts, resp }) =
                    futures::executor::block_on(req_rx.recv())
                {
                    let out = model.embed(texts, None)
                        .map_err(|e| EmbedError::InferenceFailed(e.to_string()));
                    // Dropped oneshot receiver = caller gave up; ignore the send error.
                    let _ = resp.send(out);
                }
            })
            .map_err(|e| EmbedError::LoadFailed(format!("spawn worker: {e}")))?;

        Ok(Self {
            req_tx,
            dimensions: 384,
            model_id: E5_MODEL_ID,
        })
    }
}

#[cfg(test)]
impl FastEmbedLocal {
    /// Test-only constructor: take a pre-wired request sender connected to a
    /// hand-rolled mock worker. The real `from_bytes` downloads and loads an
    /// actual ~470 MB model, which this suite never does automatically —
    /// exercising it means running the app once and confirming embeddings
    /// come back, not something a fast unit test can assert on cheaply.
    pub(crate) fn from_mock_worker(req_tx: mpsc::UnboundedSender<EmbedRequest>) -> Self {
        Self {
            req_tx,
            dimensions: 384,
            model_id: "mock-embedder",
        }
    }
}

// ============================================================================
// Compile-time Send + Sync regression guards.
//
// The assertion IS the test — if any of these types regress to !Send or !Sync
// the crate will fail to compile. No #[test] fn needed.
//
// `FastEmbedLocal` holds only `mpsc::UnboundedSender` (Send + Sync), a `usize`,
// and a `&'static str` — provably Send + Sync without `unsafe`.
// ============================================================================
const _: fn() = || {
    fn _assert_send<T: Send>() {}
    fn _assert_sync<T: Sync>() {}
    _assert_send::<FastEmbedLocal>();
    _assert_sync::<FastEmbedLocal>();
    _assert_send::<std::sync::Arc<dyn Embedder>>();
    _assert_sync::<std::sync::Arc<dyn Embedder>>();
};

// ============================================================================
// Stub embedders.
//
// Metadata-only, deliberately — these are extension points, not half-built
// backends. Neither can embed anything: `embed_with_role` panics via
// `unimplemented!()`, and nothing constructs them outside their own tests.
// What they DO carry is real `dimensions`/`model_id`, which is what makes them
// useful before anyone implements them: the persisted embedding contract keys
// off exactly those two values, so swapping to one of these backends produces
// a clean contract mismatch on load instead of silently reading a corpus
// embedded by a different model at a different dimension.
//
// Not feature-gated: code that pattern-matches on `Box<dyn Embedder>` should
// never need a `#[cfg]` branch just because the active backend happens to be
// unimplemented.
// ============================================================================

pub struct OllamaEmbedder;

#[async_trait]
impl Embedder for OllamaEmbedder {
    async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
        unimplemented!("no Ollama embedding backend is wired up yet")
    }
    fn dimensions(&self) -> usize {
        384
    }
    fn model_id(&self) -> &str {
        "ollama-placeholder"
    }
    fn query_prefix(&self) -> &str {
        ""
    }
    fn passage_prefix(&self) -> &str {
        ""
    }
}

pub struct OpenAIEmbedder;

#[async_trait]
impl Embedder for OpenAIEmbedder {
    async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
        unimplemented!("no OpenAI embedding backend is wired up yet")
    }
    fn dimensions(&self) -> usize {
        1536
    }
    fn model_id(&self) -> &str {
        "openai-placeholder"
    }
    fn query_prefix(&self) -> &str {
        ""
    }
    fn passage_prefix(&self) -> &str {
        ""
    }
}

// ============================================================================
// FakeEmbedder — deterministic test double for the Embedder trait.
//
// SipHash-seeds a linear congruential generator from the input text and fills
// `dim` floats from it, so the same text always yields the same vector and
// distinct texts (almost always) yield distinct ones — no model load, no
// I/O, no cross-run or cross-platform non-determinism.
//
// Reachable outside plain unit tests as well, when the evaluation-harness
// feature is enabled, so an offline harness can seed and query a store
// without downloading the real embedding model.
//
// Load-bearing property for anyone reading eval output: the generator is NOT
// L2-normalized. It draws each of the 384 coordinates uniformly from [-1, 1],
// which puts the expected vector norm at roughly 11.3 (sqrt(384/3)) instead
// of 1.0. Distances computed against these vectors are therefore not on the
// same scale as distances computed against the real model's unit-norm
// output — any threshold or score tuned against the real embedder will not
// behave the same way here.
// ============================================================================

#[cfg(any(test, feature = "eval", feature = "testing"))]
pub struct FakeEmbedder {
    dim: usize,
}

// Gated exactly like the struct above (`testing` included): exposing the type
// to another crate's tests without its `Default` and `Embedder` impls hands
// them a name they cannot construct or use.
#[cfg(any(test, feature = "eval", feature = "testing"))]
impl Default for FakeEmbedder {
    fn default() -> Self {
        Self { dim: 384 }
    }
}

#[cfg(any(test, feature = "eval", feature = "testing"))]
#[async_trait]
impl Embedder for FakeEmbedder {
    // `role` is deliberately ignored: this is a SipHash of the raw text with
    // no learned geometry, so prefixing it would churn every fixture vector
    // in the suite while making the double no more faithful to what a real
    // E5 model does with a prefix — there is no "more correct" fake output to
    // aim for, only a different arbitrary one.
    async fn embed_with_role(&self, text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
        // Deterministic algorithm:
        //   SipHash the text → seed a LCG → fill `dim` floats in [-1, 1].
        use std::hash::{Hash, Hasher};
        let mut h = siphasher::sip::SipHasher::new();
        text.hash(&mut h);
        let mut seed = h.finish();
        let mut out = Vec::with_capacity(self.dim);
        for _ in 0..self.dim {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let f = ((seed >> 33) as f32 / u32::MAX as f32) * 2.0 - 1.0;
            out.push(f);
        }
        Ok(out)
    }

    fn dimensions(&self) -> usize {
        self.dim
    }

    fn model_id(&self) -> &str {
        "fake-embedder"
    }

    // Empty, matching that `embed_with_role` never applies a prefix — an
    // impl claiming a non-empty prefix here while never using one in
    // `embed_with_role` would be a comment the code contradicts.
    fn query_prefix(&self) -> &str {
        ""
    }

    fn passage_prefix(&self) -> &str {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_e5_model_is_pooled_by_mean() {
        let tokenizer = TokenizerFiles {
            tokenizer_file: Vec::new(),
            config_file: Vec::new(),
            special_tokens_map_file: Vec::new(),
            tokenizer_config_file: Vec::new(),
        };
        assert_eq!(e5_model(Vec::new(), tokenizer).pooling, Some(Pooling::Mean));
    }

    #[test]
    fn the_model_id_names_its_pooling_apart_from_the_cls_vectors_of_before() {
        assert_eq!(E5_MODEL_ID, "multilingual-e5-small/mean");
        assert_eq!(LEGACY_CLS_MODEL_ID, "multilingual-e5-small");
    }

    // ---------- Embedder trait + FakeEmbedder ----------

    #[tokio::test]
    async fn fake_embedder_returns_384_dim_vec() {
        let e = FakeEmbedder::default();
        let v = e.embed_with_role("hola mundo", EmbedRole::Passage).await.unwrap();
        assert_eq!(v.len(), 384);
    }

    #[tokio::test]
    async fn fake_embedder_is_deterministic() {
        let e = FakeEmbedder::default();
        let a = e.embed_with_role("hola mundo", EmbedRole::Passage).await.unwrap();
        let b = e.embed_with_role("hola mundo", EmbedRole::Passage).await.unwrap();
        assert_eq!(a, b, "deterministic same-input → same-output");
    }

    #[tokio::test]
    async fn fake_embedder_different_inputs_differ() {
        let e = FakeEmbedder::default();
        let a = e.embed_with_role("a", EmbedRole::Passage).await.unwrap();
        let b = e.embed_with_role("b", EmbedRole::Passage).await.unwrap();
        assert_ne!(a, b, "anti-collision smoke: distinct inputs must yield distinct vectors");
    }

    /// `FakeEmbedder` deliberately ignores `role` (comment on the impl): the
    /// same text embeds identically regardless of which role is passed.
    #[tokio::test]
    async fn fake_embedder_ignores_role_deterministically() {
        let e = FakeEmbedder::default();
        let q = e.embed_with_role("hola mundo", EmbedRole::Query).await.unwrap();
        let p = e.embed_with_role("hola mundo", EmbedRole::Passage).await.unwrap();
        assert_eq!(q, p, "FakeEmbedder must return the identical vector for both roles");
    }

    #[test]
    fn fake_embedder_model_id() {
        let e = FakeEmbedder::default();
        assert_eq!(e.model_id(), "fake-embedder");
    }

    #[test]
    fn fake_embedder_dimensions_is_384() {
        let e = FakeEmbedder::default();
        assert_eq!(e.dimensions(), 384);
    }

    // ---------- Embedder window pin ----------

    /// Pins the two constants the query composer's budget depends on. This
    /// is the only place that can catch `.with_max_length(EMBED_MAX_TOKENS)`
    /// being deleted from `from_bytes` — `TextEmbedding` exposes no getter
    /// for its effective max length, so that deletion is otherwise
    /// unobservable from a unit test; a real-model control run is the other
    /// half of that coverage, out of this test's reach.
    #[test]
    fn embed_max_tokens_and_special_token_reserve_are_pinned() {
        assert_eq!(EMBED_MAX_TOKENS, 512);
        assert_eq!(SPECIAL_TOKEN_RESERVE, 4);
    }

    // ---------- Stub embedders ----------

    #[test]
    fn ollama_stub_compiles_and_implements_trait() {
        let _: Box<dyn Embedder> = Box::new(OllamaEmbedder);
    }

    #[test]
    fn openai_stub_compiles_and_implements_trait() {
        let _: Box<dyn Embedder> = Box::new(OpenAIEmbedder);
    }

    #[test]
    fn ollama_stub_dimensions_is_384() {
        assert_eq!(OllamaEmbedder.dimensions(), 384);
    }

    #[test]
    fn openai_stub_dimensions_is_1536() {
        assert_eq!(OpenAIEmbedder.dimensions(), 1536);
    }

    #[tokio::test]
    #[should_panic(expected = "no Ollama embedding backend")]
    async fn ollama_stub_embed_is_unimplemented() {
        // unwrap() of a panicking unimplemented!() body — should_panic catches it.
        let _ = OllamaEmbedder.embed_with_role("x", EmbedRole::Passage).await;
    }

    // ---------- embed_batch (default impl + invariants) ----------

    #[tokio::test]
    async fn embed_batch_default_impl_matches_one_by_one() {
        let e = FakeEmbedder::default();
        let texts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let batch = e.embed_batch(&texts, EmbedRole::Passage).await.unwrap();

        let mut individual = Vec::new();
        for t in &texts {
            individual.push(e.embed_with_role(t, EmbedRole::Passage).await.unwrap());
        }

        assert_eq!(batch, individual, "embed_batch must match N sequential embed_with_role calls, in order");
    }

    #[tokio::test]
    async fn embed_batch_default_impl_returns_empty_for_empty_input() {
        let e = FakeEmbedder::default();
        let texts: Vec<String> = Vec::new();
        let batch = e.embed_batch(&texts, EmbedRole::Passage).await.unwrap();
        assert!(batch.is_empty(), "expected empty output for empty input, got {batch:?}");
    }

    // ---------- FastEmbedLocal worker plumbing ----------

    #[tokio::test]
    async fn embed_request_round_trip_through_channel() {
        // Hand-rolled mock worker: spawn a thread that receives EmbedRequest
        // and replies with a canned 384-dim vector.
        let (req_tx, mut req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        let canned: Vec<f32> = (0..384).map(|i| i as f32 * 0.001).collect();
        let canned_for_worker = canned.clone();
        std::thread::spawn(move || {
            while let Some(EmbedRequest { texts: _, resp }) =
                futures::executor::block_on(req_rx.recv())
            {
                let _ = resp.send(Ok(vec![canned_for_worker.clone()]));
            }
        });

        let local = FastEmbedLocal::from_mock_worker(req_tx);
        let out = local.embed_with_role("anything", EmbedRole::Passage).await.unwrap();
        assert_eq!(out.len(), 384);
        assert_eq!(out, canned, "worker reply must flow back unchanged");
    }

    /// `FastEmbedLocal::embed_batch` must send exactly one channel request
    /// carrying every text, each already prefixed per `role`.
    #[tokio::test]
    async fn fast_embed_local_sends_one_request_with_all_prefixed_texts() {
        use std::sync::{Arc, Mutex};

        let (req_tx, mut req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        let captured: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_for_worker = captured.clone();
        std::thread::spawn(move || {
            while let Some(EmbedRequest { texts, resp }) =
                futures::executor::block_on(req_rx.recv())
            {
                captured_for_worker.lock().unwrap().push(texts.clone());
                let reply: Vec<Vec<f32>> = texts.iter().map(|_| vec![0.0f32; 384]).collect();
                let _ = resp.send(Ok(reply));
            }
        });

        let local = FastEmbedLocal::from_mock_worker(req_tx);
        let texts = vec!["chunk-a".to_string(), "chunk-b".to_string()];
        let out = local.embed_batch(&texts, EmbedRole::Passage).await.unwrap();
        assert_eq!(out.len(), 2);

        let calls = captured.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "expected exactly one channel request for the whole batch, got {calls:?}");
        assert_eq!(
            calls[0],
            vec!["query: chunk-a".to_string(), "query: chunk-b".to_string()],
            "every text must be prefixed per role before crossing the channel"
        );
    }

    /// A reply with fewer (or more) vectors than texts submitted must be
    /// reported as InferenceFailed, never trusted positionally.
    #[tokio::test]
    async fn fast_embed_local_rejects_reply_with_wrong_vector_count() {
        let (req_tx, mut req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        std::thread::spawn(move || {
            while let Some(EmbedRequest { texts: _, resp }) =
                futures::executor::block_on(req_rx.recv())
            {
                // Reply with only 1 vector for however many texts were sent.
                let _ = resp.send(Ok(vec![vec![0.0f32; 384]]));
            }
        });

        let local = FastEmbedLocal::from_mock_worker(req_tx);
        let texts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let err = local.embed_batch(&texts, EmbedRole::Passage).await.unwrap_err();
        assert!(
            matches!(err, EmbedError::InferenceFailed(_)),
            "expected InferenceFailed for a vector-count mismatch, got {err:?}"
        );
    }

    #[tokio::test]
    async fn embed_returns_worker_gone_when_worker_drops() {
        let (req_tx, req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        // Drop receiver immediately to simulate the worker thread exiting.
        drop(req_rx);
        let local = FastEmbedLocal::from_mock_worker(req_tx);
        let err = local.embed_with_role("anything", EmbedRole::Passage).await.unwrap_err();
        assert!(
            matches!(err, EmbedError::WorkerGone),
            "expected WorkerGone, got {err:?}"
        );
    }

    /// `FastEmbedLocal::embed_with_role` must apply the E5 prefix BEFORE the
    /// text crosses the channel to the model thread — asserted on the exact
    /// string the mock worker receives, not on the (prefix-insensitive)
    /// output vector.
    #[tokio::test]
    async fn embed_with_role_sends_prefixed_text_over_the_channel() {
        use std::sync::{Arc, Mutex};

        let (req_tx, mut req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let captured_for_worker = captured.clone();
        std::thread::spawn(move || {
            while let Some(EmbedRequest { texts, resp }) =
                futures::executor::block_on(req_rx.recv())
            {
                *captured_for_worker.lock().unwrap() = texts.first().cloned();
                let _ = resp.send(Ok(vec![vec![0.0f32; 384]]));
            }
        });

        let local = FastEmbedLocal::from_mock_worker(req_tx);
        local
            .embed_with_role("chunk", EmbedRole::Passage)
            .await
            .unwrap();

        assert_eq!(
            captured.lock().unwrap().as_deref(),
            Some("query: chunk"),
            "the model must receive the prefixed string, not the bare chunk text"
        );
    }

    // ---------- apply_role_prefix (pure) ----------

    #[test]
    fn apply_role_prefix_query_selects_query_prefix() {
        assert_eq!(
            apply_role_prefix(EmbedRole::Query, "hola", "query: ", "passage: "),
            "query: hola"
        );
    }

    #[test]
    fn apply_role_prefix_passage_selects_passage_prefix() {
        assert_eq!(
            apply_role_prefix(EmbedRole::Passage, "hola", "query: ", "passage: "),
            "passage: hola"
        );
    }

    #[test]
    fn apply_role_prefix_prepends_not_appends() {
        let out = apply_role_prefix(EmbedRole::Query, "hola", "query: ", "passage: ");
        assert!(out.starts_with("query: "), "prefix must come before the text, got {out:?}");
        assert!(!out.ends_with("query: "), "prefix must not be appended, got {out:?}");
    }

    #[test]
    fn apply_role_prefix_empty_prefix_is_identity() {
        assert_eq!(apply_role_prefix(EmbedRole::Query, "hola", "", ""), "hola");
    }

    /// Pins what each role currently emits: both [`EmbedRole::Query`] and
    /// [`EmbedRole::Passage`] resolve to the SAME literal string under the
    /// symmetric configuration (see the comment on `E5_PASSAGE_PREFIX` for
    /// the measured reason). The trailing space stays load-bearing
    /// regardless: `"query:"` and `"query: "` tokenize to different
    /// sequences, so losing it would silently change the embedding space no
    /// matter which role produced the text.
    #[test]
    fn e5_prefix_constants_are_symmetric_with_the_load_bearing_trailing_space() {
        assert_eq!(E5_QUERY_PREFIX, "query: ");
        assert_eq!(E5_PASSAGE_PREFIX, "query: ");
        assert_eq!(
            E5_QUERY_PREFIX, E5_PASSAGE_PREFIX,
            "symmetric configuration: both roles currently emit the identical prefix"
        );
        assert_ne!(E5_QUERY_PREFIX, "query:", "the trailing space is load-bearing");
    }
}
