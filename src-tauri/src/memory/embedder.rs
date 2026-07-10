//! Embedder trait + concrete impls for the v1 RAG roadmap (Phase 2).
//!
//! Skeleton; filled out across Phase 2.1 — 2.3 per `docs/sdd/rag-embedder/tasks.md`.

use async_trait::async_trait;
use fastembed::{
    InitOptionsUserDefined, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use tokio::sync::{mpsc, oneshot};

/// Async embedder contract. All embedder impls are `Send + Sync` so they can
/// live behind `Arc<dyn Embedder>` in `AppState`.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Compute the embedding for a single text. Vector length MUST equal
    /// [`Embedder::dimensions`].
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError>;

    /// Embedding dimensionality (e.g. 384 for `multilingual-e5-small`).
    fn dimensions(&self) -> usize;

    /// Stable identifier for the active model
    /// (e.g. `"multilingual-e5-small"`, `"fake-embedder"`).
    fn model_id(&self) -> &str;
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
// thread via a tokio::mpsc request/oneshot channel (REQ-EMB-002).
//
// Per design §3 (worker-thread architecture): the inner `fastembed::TextEmbedding`
// is NOT guaranteed `Send`, so it lives on one dedicated `std::thread` for its
// entire life. The public struct holds only channel senders + primitives, which
// keeps it `Send + Sync` — verified by the const-block at the bottom of this file.
// ============================================================================

/// Internal request crossing the channel: text to embed + oneshot reply slot.
pub(crate) struct EmbedRequest {
    #[allow(dead_code)] // Read inside the worker loop (block_on / pattern-match).
    pub(crate) text: String,
    pub(crate) resp: oneshot::Sender<Result<Vec<f32>, EmbedError>>,
}

pub struct FastEmbedLocal {
    req_tx: mpsc::UnboundedSender<EmbedRequest>,
    dimensions: usize,
    model_id: &'static str,
}

#[async_trait]
impl Embedder for FastEmbedLocal {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.req_tx
            .send(EmbedRequest {
                text: text.to_string(),
                resp: resp_tx,
            })
            .map_err(|_| EmbedError::WorkerGone)?;
        // Map the oneshot RecvError (sender dropped) to WorkerGone; otherwise
        // unwrap the inner Result the worker placed inside.
        resp_rx.await.map_err(|_| EmbedError::WorkerGone)?
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn model_id(&self) -> &str {
        self.model_id
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

        let user_model = UserDefinedEmbeddingModel::new(onnx, tokenizer);
        let init_opts = InitOptionsUserDefined::new();

        // Synchronous load on the calling thread: surface load errors immediately.
        // `embed()` takes `&mut self` in fastembed v5 (interior session state).
        let mut model = TextEmbedding::try_new_from_user_defined(user_model, init_opts)
            .map_err(|e| EmbedError::LoadFailed(e.to_string()))?;

        std::thread::Builder::new()
            .name("fastembed-worker".into())
            .spawn(move || {
                // futures::executor::block_on bridges tokio's mpsc receiver into
                // a non-tokio thread. Loop exits naturally when all senders drop.
                while let Some(EmbedRequest { text, resp }) =
                    futures::executor::block_on(req_rx.recv())
                {
                    let out = model
                        .embed(vec![text.as_str()], None)
                        .map(|mut v| v.pop().unwrap_or_default())
                        .map_err(|e| EmbedError::InferenceFailed(e.to_string()));
                    // Dropped oneshot receiver = caller gave up; ignore the send error.
                    let _ = resp.send(out);
                }
            })
            .map_err(|e| EmbedError::LoadFailed(format!("spawn worker: {e}")))?;

        Ok(Self {
            req_tx,
            dimensions: 384,
            model_id: "multilingual-e5-small",
        })
    }
}

#[cfg(test)]
impl FastEmbedLocal {
    /// Test-only constructor: take a pre-wired request sender connected to a
    /// hand-rolled mock worker. The real `from_bytes` spawns the worker itself
    /// and is covered by manual smoke at CHECKPOINT 3 (the real ~470 MB model
    /// load is `[BLOCKED-BY-EXTERNAL]`).
    pub(crate) fn from_mock_worker(req_tx: mpsc::UnboundedSender<EmbedRequest>) -> Self {
        Self {
            req_tx,
            dimensions: 384,
            model_id: "mock-embedder",
        }
    }
}

// ============================================================================
// Compile-time Send + Sync regression guards (REQ-EMB-002 / T021).
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
// Stub embedders (REQ-EMB-003).
//
// Non-feature-gated by design: future phases pattern-match on `Box<dyn Embedder>`
// without `#[cfg]` branches. `embed` panics via `unimplemented!()`; `dimensions`
// and `model_id` return real metadata so the rest of the system can still reason
// about a stub instance (e.g. log "active embedder: openai-placeholder").
// ============================================================================

pub struct OllamaEmbedder;

#[async_trait]
impl Embedder for OllamaEmbedder {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedError> {
        unimplemented!("OllamaEmbedder is a Phase X feature")
    }
    fn dimensions(&self) -> usize {
        384
    }
    fn model_id(&self) -> &str {
        "ollama-placeholder"
    }
}

pub struct OpenAIEmbedder;

#[async_trait]
impl Embedder for OpenAIEmbedder {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedError> {
        unimplemented!("OpenAIEmbedder is a Phase X feature")
    }
    fn dimensions(&self) -> usize {
        1536
    }
    fn model_id(&self) -> &str {
        "openai-placeholder"
    }
}

// ============================================================================
// FakeEmbedder — deterministic test double (REQ-EMB-001 / REQ-TEST-001).
//
// Inline `#[cfg(test)]` per proposal decision #11. Phase 3 may relax this to
// `#[cfg(any(test, feature = "test-helpers"))]` for cross-file reuse.
// ============================================================================

#[cfg(test)]
pub(crate) struct FakeEmbedder {
    dim: usize,
}

#[cfg(test)]
impl Default for FakeEmbedder {
    fn default() -> Self {
        Self { dim: 384 }
    }
}

#[cfg(test)]
#[async_trait]
impl Embedder for FakeEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        // Deterministic algorithm per design §3.4:
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
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- REQ-EMB-001: Embedder trait + FakeEmbedder (T010) ----------

    #[tokio::test]
    async fn fake_embedder_returns_384_dim_vec() {
        let e = FakeEmbedder::default();
        let v = e.embed("hola mundo").await.unwrap();
        assert_eq!(v.len(), 384);
    }

    #[tokio::test]
    async fn fake_embedder_is_deterministic() {
        let e = FakeEmbedder::default();
        let a = e.embed("hola mundo").await.unwrap();
        let b = e.embed("hola mundo").await.unwrap();
        assert_eq!(a, b, "deterministic same-input → same-output");
    }

    #[tokio::test]
    async fn fake_embedder_different_inputs_differ() {
        let e = FakeEmbedder::default();
        let a = e.embed("a").await.unwrap();
        let b = e.embed("b").await.unwrap();
        assert_ne!(a, b, "anti-collision smoke: distinct inputs must yield distinct vectors");
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

    // ---------- REQ-EMB-003: Stub embedders (T015) ----------

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
    #[should_panic(expected = "Phase X")]
    async fn ollama_stub_embed_is_unimplemented() {
        // unwrap() of a panicking unimplemented!() body — should_panic catches it.
        let _ = OllamaEmbedder.embed("x").await;
    }

    // ---------- REQ-EMB-002: FastEmbedLocal worker plumbing (T020) ----------

    #[tokio::test]
    async fn embed_request_round_trip_through_channel() {
        // Hand-rolled mock worker: spawn a thread that receives EmbedRequest
        // and replies with a canned 384-dim vector.
        let (req_tx, mut req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        let canned: Vec<f32> = (0..384).map(|i| i as f32 * 0.001).collect();
        let canned_for_worker = canned.clone();
        std::thread::spawn(move || {
            while let Some(EmbedRequest { text: _, resp }) =
                futures::executor::block_on(req_rx.recv())
            {
                let _ = resp.send(Ok(canned_for_worker.clone()));
            }
        });

        let local = FastEmbedLocal::from_mock_worker(req_tx);
        let out = local.embed("anything").await.unwrap();
        assert_eq!(out.len(), 384);
        assert_eq!(out, canned, "worker reply must flow back unchanged");
    }

    #[tokio::test]
    async fn embed_returns_worker_gone_when_worker_drops() {
        let (req_tx, req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        // Drop receiver immediately to simulate the worker thread exiting.
        drop(req_rx);
        let local = FastEmbedLocal::from_mock_worker(req_tx);
        let err = local.embed("anything").await.unwrap_err();
        assert!(
            matches!(err, EmbedError::WorkerGone),
            "expected WorkerGone, got {err:?}"
        );
    }
}
