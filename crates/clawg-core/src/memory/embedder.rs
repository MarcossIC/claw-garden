//! The [`Embedder`] trait and its concrete implementations: [`E5Embedder`]
//! (the production embedder: `multilingual-e5-small` on an ONNX Runtime session
//! of our own, with the process's shared tokenizer),
//! [`OllamaEmbedder`]/[`OpenAIEmbedder`] (unimplemented stubs reserved for
//! future backends), and `FakeEmbedder` (a deterministic test double). Every
//! embed call carries an explicit [`EmbedRole`] so a caller can never forget
//! which side of the query/passage distinction it is on — see [`EmbedRole`]
//! below for why that distinction matters.

use std::sync::Arc;

use async_trait::async_trait;
use ort::session::{builder::{GraphOptimizationLevel, SessionBuilder}, Session};
use ort::value::Tensor;
use tokio::sync::{mpsc, oneshot};

use crate::memory::db::EmbeddingIdentity;
use crate::memory::model_download::TokenizerFiles;
use crate::memory::vector::EMBEDDING_DIM;
use crate::metrics::{Recorder, Unit};
use tokenizers::{Encoding, Tokenizer};

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

impl EmbedRole {
    /// The role as a metric label names it.
    pub fn as_str(self) -> &'static str {
        match self {
            EmbedRole::Query => "query",
            EmbedRole::Passage => "passage",
        }
    }
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

    /// Whether the model is in memory now, so the next call needs no load.
    /// Only an embedder that can let its model go (see
    /// [`IdleEmbedder`](crate::memory::idle_embedder::IdleEmbedder)) ever
    /// answers `false`.
    fn resident(&self) -> bool {
        true
    }

    /// Start bringing the model back into memory without waiting for it, for
    /// a caller that knows a request is coming. A resident model has nothing
    /// to do.
    fn wake(&self) {}
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
/// doc-to-doc structure was essentially unchanged. That was measured with
/// CLS pooling and has not been re-measured under mean. If this corpus later
/// gains a genuinely asymmetric shape (e.g. short queries against long
/// standalone documents), this constant is the one line to move back to
/// `"passage: "`.
const E5_PASSAGE_PREFIX: &str = "query: ";

/// The id the embedding contract records for this embedder's vectors. It names
/// the pooling because the model file does not decide it, and vectors pooled
/// differently live in different spaces.
pub const E5_MODEL_ID: &str = "multilingual-e5-small/mean";

/// The id earlier versions recorded for the same weights pooled by their first
/// token (CLS), which gave every passage of a corpus nearly the same vector,
/// since e5 is trained with mean pooling. A store under this id holds vectors
/// that must be re-embedded, never mixed with new ones.
pub const LEGACY_CLS_MODEL_ID: &str = "multilingual-e5-small";

/// The contract this binary's embedder records its vectors under, known
/// without loading the model, so a stopped store can be checked against it.
pub fn expected_contract() -> EmbeddingIdentity {
    EmbeddingIdentity {
        model_id: E5_MODEL_ID.to_string(),
        dim: EMBEDDING_DIM,
        query_prefix: E5_QUERY_PREFIX.to_string(),
        passage_prefix: E5_PASSAGE_PREFIX.to_string(),
    }
}

/// ONNX Runtime's CPU provider with its arena off. The arena keeps the largest
/// block an inference run ever asked for and never gives it back, so the
/// daemon would hold its indexing peak while idle. Without it the memory goes
/// back after each run.
fn cpu_provider() -> ort::ep::CPU {
    ort::ep::CPU::default().with_arena_allocator(false)
}

/// The token window the model input is cut to, `<s>` and `</s>` included. One
/// constant, because the query composer's budget
/// (`memory::query_context::query_budget_tokens`) has to read the SAME number
/// the embedder actually enforces.
pub const EMBED_MAX_TOKENS: usize = 512;

/// The most texts one inference run embeds. A run pads every text to the
/// longest one, and its attention grows with texts × tokens², so the 248
/// passages of one large source file at up to 512 tokens took gigabytes in a
/// single run, which the process then kept. At 16 the peak stays in the
/// hundreds of megabytes and indexing runs as fast as with one big batch.
pub const EMBED_BATCH: usize = 16;

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

/// A request's encodings laid out as the model takes them: `rows` texts of
/// `len` tokens each, row after row.
pub(crate) struct PaddedBatch {
    pub(crate) rows: usize,
    pub(crate) len: usize,
    pub(crate) ids: Vec<i64>,
    pub(crate) mask: Vec<i64>,
    pub(crate) type_ids: Vec<i64>,
}

/// The model's input for each text: the tokenizer's encoding cut to
/// [`EMBED_MAX_TOKENS`] counting the `<s>` and `</s>` the post-processor adds,
/// so `</s>` stays at the end of a cut text.
///
/// The cut is made here, on a text encoded without special tokens, and not by
/// setting a truncation on the tokenizer: that one is shared with the chunker,
/// which counts untruncated tokens with it. These are the three steps the
/// tokenizer's own `post_process` takes when it has a truncation set.
pub(crate) fn window_encodings(tokenizer: &Tokenizer, texts: &[&str]) -> Result<Vec<Encoding>, EmbedError> {
    use tokenizers::{PostProcessor, TruncationDirection};

    let added = tokenizer.get_post_processor().map_or(0, |p| p.added_tokens(false));
    let keep = EMBED_MAX_TOKENS.saturating_sub(added);
    let failed = |e: tokenizers::Error| EmbedError::InferenceFailed(format!("tokenization failed: {e}"));
    tokenizer
        .encode_batch(texts.to_vec(), false)
        .map_err(failed)?
        .into_iter()
        .map(|mut encoding| {
            encoding.truncate(keep, 0, TruncationDirection::Right);
            tokenizer.post_process(encoding, None, true).map_err(failed)
        })
        .collect()
}

/// Lays the encodings out as one rectangle, padding every text on the right to
/// the longest with `pad_id`, attention mask 0 and token type 0.
pub(crate) fn pad_right(encodings: &[Encoding], pad_id: u32) -> PaddedBatch {
    let rows = encodings.len();
    let len = encodings.iter().map(Encoding::len).max().unwrap_or(0);
    let mut batch = PaddedBatch {
        rows,
        len,
        ids: Vec::with_capacity(rows * len),
        mask: Vec::with_capacity(rows * len),
        type_ids: Vec::with_capacity(rows * len),
    };
    for encoding in encodings {
        let padding = len - encoding.len();
        batch.ids.extend(encoding.get_ids().iter().map(|x| *x as i64));
        batch.ids.extend(std::iter::repeat(pad_id as i64).take(padding));
        batch.mask.extend(encoding.get_attention_mask().iter().map(|x| *x as i64));
        batch.mask.extend(std::iter::repeat(0).take(padding));
        batch.type_ids.extend(encoding.get_type_ids().iter().map(|x| *x as i64));
        batch.type_ids.extend(std::iter::repeat(0).take(padding));
    }
    batch
}

/// Which of the model's outputs holds the embeddings: the only one, else the
/// first of `text_embeds`, `last_hidden_state` and `sentence_embedding` the
/// model has. Returns its index in `names`.
pub(crate) fn pick_output(names: &[&str]) -> Result<usize, EmbedError> {
    if names.len() == 1 {
        return Ok(0);
    }
    ["text_embeds", "last_hidden_state", "sentence_embedding"]
        .iter()
        .find_map(|wanted| names.iter().position(|name| name == wanted))
        .ok_or_else(|| EmbedError::LoadFailed(format!("no usable output among {names:?}")))
}

/// Mean pooling over the attention mask, then L2 normalization, in the exact
/// order of floating-point operations fastembed uses, so the vectors are the
/// same bit for bit: each coordinate is the sum over the tokens in token order
/// of `mask * state` (padded positions included, they add zero), divided by the
/// sum of the mask, and then every coordinate is divided by `norm + 1e-12`.
///
/// A two-dimensional output is already pooled and is only normalized.
pub(crate) fn pool_and_normalize(hidden: &[f32], shape: &[i64], mask: &[i64]) -> Result<Vec<Vec<f32>>, EmbedError> {
    let wrong = |why: String| EmbedError::InferenceFailed(why);
    let pooled: Vec<Vec<f32>> = match *shape {
        [rows, dim] => {
            let (rows, dim) = (rows as usize, dim as usize);
            if hidden.len() != rows * dim {
                return Err(wrong(format!("output of {} values for shape {shape:?}", hidden.len())));
            }
            hidden.chunks(dim.max(1)).take(rows).map(<[f32]>::to_vec).collect()
        }
        [rows, len, dim] => {
            let (rows, len, dim) = (rows as usize, len as usize, dim as usize);
            if hidden.len() != rows * len * dim || mask.len() != rows * len {
                return Err(wrong(format!(
                    "output of {} values and mask of {} for shape {shape:?}",
                    hidden.len(),
                    mask.len()
                )));
            }
            (0..rows)
                .map(|row| {
                    let mut sum = vec![0f32; dim];
                    let mut mask_sum = 0f32;
                    for token in 0..len {
                        let m = mask[row * len + token] as f32;
                        let states = &hidden[(row * len + token) * dim..][..dim];
                        for (acc, x) in sum.iter_mut().zip(states) {
                            *acc += m * x;
                        }
                        mask_sum += m;
                    }
                    let mask_sum = if mask_sum == 0f32 { 1.0 } else { mask_sum };
                    sum.iter().map(|x| x / mask_sum).collect()
                })
                .collect()
        }
        _ => return Err(wrong(format!("invalid output shape {shape:?}; expected 2D or 3D"))),
    };
    Ok(pooled.into_iter().map(|v| normalize(&v)).collect())
}

fn normalize(v: &[f32]) -> Vec<f32> {
    let norm = (v.iter().map(|val| val * val).sum::<f32>()).sqrt();
    let epsilon = 1e-12;
    v.iter().map(|&val| val / (norm + epsilon)).collect()
}

/// What the load checks before it trusts a tokenizer to encode like
/// fastembed's did, and the id to pad with (`pad_token_id` of `config.json`,
/// 0 when it has none). Refuses:
///
/// - a tokenizer with a truncation or a padding of its own, whose `encode`
///   would cut or pad before the window and the padding applied here;
/// - a token of `special_tokens_map.json` that the tokenizer does not hold as
///   special: fastembed added those as special tokens, which changes nothing
///   only for a token that already is;
/// - a `model_max_length` shorter than [`EMBED_MAX_TOKENS`], which fastembed
///   would have cut to.
pub(crate) fn check_tokenizer(tokenizer: &Tokenizer, files: &TokenizerFiles) -> Result<u32, EmbedError> {
    let refuse = |why: String| EmbedError::LoadFailed(why);
    let json = |bytes: &[u8], name: &str| {
        serde_json::from_slice::<serde_json::Value>(bytes)
            .map_err(|e| refuse(format!("{name} is not valid JSON: {e}")))
    };
    let config = json(&files.config_file, "config.json")?;
    let special_tokens_map = json(&files.special_tokens_map_file, "special_tokens_map.json")?;
    let tokenizer_config = json(&files.tokenizer_config_file, "tokenizer_config.json")?;

    if tokenizer.get_truncation().is_some() {
        return Err(refuse("tokenizer.json sets a truncation of its own".into()));
    }
    if tokenizer.get_padding().is_some() {
        return Err(refuse("tokenizer.json sets a padding of its own".into()));
    }
    let max_length = tokenizer_config["model_max_length"]
        .as_f64()
        .ok_or_else(|| refuse("tokenizer_config.json has no model_max_length".into()))?;
    if max_length < EMBED_MAX_TOKENS as f64 {
        return Err(refuse(format!(
            "model_max_length {max_length} of tokenizer_config.json is shorter than the {EMBED_MAX_TOKENS} tokens embedded"
        )));
    }
    let special: Vec<String> = tokenizer
        .get_added_tokens_decoder()
        .into_values()
        .filter(|token| token.special)
        .map(|token| token.content)
        .collect();
    for value in special_tokens_map.as_object().into_iter().flat_map(|map| map.values()) {
        let content = match value {
            serde_json::Value::String(content) => Some(content.as_str()),
            serde_json::Value::Object(_) => value["content"].as_str(),
            _ => None,
        };
        if let Some(content) = content {
            if !special.iter().any(|token| token == content) {
                return Err(refuse(format!(
                    "special_tokens_map.json names {content:?}, which tokenizer.json does not hold as a special token"
                )));
            }
        }
    }
    Ok(config["pad_token_id"].as_u64().unwrap_or(0) as u32)
}

// ============================================================================
// E5Embedder — the ONNX Runtime session of `multilingual-e5-small`, confined to
// a single owning thread via a tokio::mpsc request/oneshot channel.
//
// The session lives on one dedicated `std::thread` for its entire life. The
// public struct holds only channel senders + primitives, which keeps it
// `Send + Sync` — verified by the const-block at the bottom of this file.
// ============================================================================

/// Internal request crossing the channel: texts to embed + oneshot reply slot.
pub(crate) struct EmbedRequest {
    pub(crate) texts: Vec<String>,
    pub(crate) resp: oneshot::Sender<Result<Vec<Vec<f32>>, EmbedError>>,
    pub(crate) role: EmbedRole,
    /// When the request was sent, so the worker can tell how long it waited.
    pub(crate) queued: std::time::Instant,
}

pub struct E5Embedder {
    req_tx: mpsc::UnboundedSender<EmbedRequest>,
    dimensions: usize,
    model_id: &'static str,
    /// Where each batch is measured, shared with the worker thread.
    metrics: Arc<Recorder>,
}

#[async_trait]
impl Embedder for E5Embedder {
    async fn embed_batch(&self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in texts.chunks(EMBED_BATCH) {
            let prefixed: Vec<String> = batch.iter()
                .map(|t| apply_role_prefix(role, t, self.query_prefix(), self.passage_prefix()))
                .collect();
            let expected = prefixed.len();
            self.metrics.gauge("embedder.batch", &[("role", role.as_str())], expected as f64);
            let (resp_tx, resp_rx) = oneshot::channel();
            let queued = std::time::Instant::now();
            self.req_tx.send(EmbedRequest { texts: prefixed, resp: resp_tx, role, queued })
                .map_err(|_| EmbedError::WorkerGone)?;
            // Map the oneshot RecvError (sender dropped) to WorkerGone; otherwise
            // unwrap the inner Result the worker placed inside.
            let reply = resp_rx.await.map_err(|_| EmbedError::WorkerGone)??;
            if reply.len() != expected {
                // Reported as an inference failure rather than trusted: the
                // caller pairs these positionally, so a wrong count means the
                // pairing is already meaningless.
                return Err(EmbedError::InferenceFailed(format!(
                    "model returned {} vectors for {expected} texts", reply.len()
                )));
            }
            vectors.extend(reply);
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

/// What the worker thread owns: the session and everything a request needs to
/// become vectors, so the pieces around the session stay the pure functions
/// above.
struct Inference {
    session: Session,
    tokenizer: Arc<Tokenizer>,
    pad_id: u32,
    /// Whether the model declares a `token_type_ids` input.
    wants_token_type_ids: bool,
    /// The output holding the embeddings, picked when the session was built.
    output: String,
    metrics: Arc<Recorder>,
}

impl Inference {
    /// One inference run over `texts`, already prefixed: the window, the
    /// padding, the session, then pooling and normalization.
    fn embed(&mut self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
        let failed = |e: ort::Error| EmbedError::InferenceFailed(e.to_string());
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let encodings = window_encodings(&self.tokenizer, &refs)?;
        let batch = pad_right(&encodings, self.pad_id);
        let shape = [batch.rows, batch.len];
        // What the session runs over, padding included.
        self.metrics.gauge("embedder.batch_tokens", &[("role", role.as_str())], (batch.rows * batch.len) as f64);

        let mut inputs = ort::inputs![
            "input_ids" => Tensor::from_array((shape, batch.ids)).map_err(failed)?,
            "attention_mask" => Tensor::from_array((shape, batch.mask.clone())).map_err(failed)?,
        ];
        if self.wants_token_type_ids {
            inputs.push((
                "token_type_ids".into(),
                Tensor::from_array((shape, batch.type_ids)).map_err(failed)?.into(),
            ));
        }
        let outputs = self.session.run(inputs).map_err(failed)?;
        let (output_shape, hidden) = outputs[self.output.as_str()]
            .try_extract_tensor::<f32>()
            .map_err(failed)?;
        pool_and_normalize(hidden, output_shape, &batch.mask)
    }
}

/// The intra-op threads when the machine cannot say how many it has. The
/// standard library documents that the count can fail; one thread would slow
/// every load and refresh for a failure that is rare.
pub(crate) const FALLBACK_SESSION_THREADS: usize = 2;

/// The intra-op threads for what `available_parallelism` reported.
fn threads_from(reported: std::io::Result<std::num::NonZeroUsize>) -> usize {
    reported.map_or(FALLBACK_SESSION_THREADS, |n| n.get())
}

/// The intra-op threads a session is built with.
pub(crate) fn session_threads() -> usize {
    let reported = std::thread::available_parallelism();
    if let Err(e) = &reported {
        log::warn!("the machine's parallelism is unknown ({e}); the embedder uses {FALLBACK_SESSION_THREADS} threads");
    }
    threads_from(reported)
}

/// The session options the embeddings were always made with: the CPU provider
/// without its arena, graph optimization level 3 and as many intra-op threads
/// as the machine has. On a 4-thread Linux x86-64 machine with AVX-512F, 1, 2
/// and 4 threads gave the same bits for every `e5_golden` vector; other
/// machines are not measured, so eval memos still record the count.
pub(crate) fn session_builder() -> Result<SessionBuilder, EmbedError> {
    let failed = |e: String| EmbedError::LoadFailed(e);
    let threads = session_threads();
    Session::builder()
        .map_err(|e| failed(e.to_string()))?
        .with_execution_providers(vec![cpu_provider().build()])
        .map_err(|e| failed(e.to_string()))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| failed(e.to_string()))?
        .with_intra_threads(threads)
        .map_err(|e| failed(e.to_string()))
}

pub(crate) fn build_session(onnx: &[u8]) -> Result<Session, EmbedError> {
    session_builder()?
        .commit_from_memory(onnx)
        .map_err(|e| EmbedError::LoadFailed(e.to_string()))
}

/// The smallest model ONNX Runtime accepts: one `Identity` node from `x` to
/// `y`, a float vector of one element.
#[cfg(test)]
pub(crate) fn minimal_onnx() -> Vec<u8> {
    fn field(tag: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![tag, payload.len() as u8];
        out.extend_from_slice(payload);
        out
    }
    fn value_info(name: &str) -> Vec<u8> {
        let dim = field(0x0a, &[0x08, 1]);
        let tensor = [&[0x08, 1][..], &field(0x12, &dim)].concat();
        [field(0x0a, name.as_bytes()), field(0x12, &field(0x0a, &tensor))].concat()
    }
    let node = [field(0x0a, b"x"), field(0x12, b"y"), field(0x22, b"Identity")].concat();
    let graph = [
        field(0x0a, &node),
        field(0x12, b"g"),
        field(0x5a, &value_info("x")),
        field(0x62, &value_info("y")),
    ]
    .concat();
    [vec![0x08, 8], field(0x42, &[0x10, 13]), field(0x3a, &graph)].concat()
}

impl E5Embedder {
    /// Construct from the model bytes, the shared tokenizer and the model's
    /// small JSON files.
    ///
    /// CPU-bound (~100 ms warm-up for `multilingual-e5-small`); callers SHOULD
    /// wrap this in `tokio::task::spawn_blocking`. The checks and the session
    /// are made on the CALLING thread first — this surfaces load errors as
    /// `EmbedError::LoadFailed` synchronously before the worker thread is
    /// spawned, which keeps error handling simple. `files.tokenizer_file` is
    /// not read: the tokenizer arrives already parsed, and is never
    /// reconfigured here because the chunker counts tokens with the same one.
    ///
    /// After load succeeds, a single `std::thread` named `"embedder-worker"`
    /// takes ownership of the session for its entire lifetime. All callers
    /// route through the request channel; the session never crosses threads.
    ///
    /// Every batch is measured into `metrics`.
    pub fn from_parts(
        onnx: &[u8],
        tokenizer: Arc<Tokenizer>,
        files: &TokenizerFiles,
        metrics: Arc<Recorder>,
    ) -> Result<Self, EmbedError> {
        let pad_id = check_tokenizer(&tokenizer, files)?;
        let session = build_session(onnx)?;
        let wants_token_type_ids = session.inputs().iter().any(|input| input.name() == "token_type_ids");
        let names: Vec<&str> = session.outputs().iter().map(|output| output.name()).collect();
        let output = names[pick_output(&names)?].to_string();
        let mut inference =
            Inference { session, tokenizer, pad_id, wants_token_type_ids, output, metrics: metrics.clone() };
        let waits = metrics.clone();

        let (req_tx, req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        std::thread::Builder::new()
            .name("embedder-worker".into())
            .spawn(move || {
                serve_requests(req_rx, &waits, |texts, role| inference.embed(texts, role));
                drop(inference.session);
                return_freed_memory();
            })
            .map_err(|e| EmbedError::LoadFailed(format!("spawn worker: {e}")))?;

        Ok(Self {
            req_tx,
            dimensions: EMBEDDING_DIM,
            model_id: E5_MODEL_ID,
            metrics,
        })
    }
}

/// The worker thread's loop: answers each request with `infer` until every
/// sender is gone or an inference panics, recording in `metrics` how long each
/// waited to be taken.
///
/// A panic is answered to its caller and ends the loop, so the session that
/// panicked is never run again; the requests still queued lose their reply,
/// which their callers see as [`EmbedError::WorkerGone`].
fn serve_requests(
    mut req_rx: mpsc::UnboundedReceiver<EmbedRequest>,
    metrics: &Recorder,
    mut infer: impl FnMut(&[String], EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError>,
) {
    // block_on bridges tokio's receiver into a thread outside the runtime.
    while let Some(EmbedRequest { texts, resp, role, queued }) = futures::executor::block_on(req_rx.recv()) {
        let waited = u64::try_from(queued.elapsed().as_micros()).unwrap_or(u64::MAX);
        metrics.observe("embedder.wait", Unit::Micros, &[("role", role.as_str())], waited);
        if resp.is_closed() {
            continue;
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| infer(&texts, role))) {
            Ok(reply) => {
                // A caller that left during the inference drops the reply.
                let _ = resp.send(reply);
            }
            Err(payload) => {
                let cause = panic_cause(payload.as_ref());
                log::error!("the embedding model panicked: {cause}; its session is dropped");
                let _ = resp.send(Err(EmbedError::InferenceFailed(format!("inference panicked: {cause}"))));
                return;
            }
        }
    }
}

fn panic_cause(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message")
}

/// Hands the pages freed by a dropped model back to the system. glibc keeps
/// freed memory in its per-thread arenas: measured on Linux, dropping the
/// session left ~650 MB of the ~1 GB resident until a trim brought the process
/// down to ~40 MB. Other allocators return large blocks on their own.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn return_freed_memory() {
    extern "C" {
        fn malloc_trim(pad: usize) -> std::ffi::c_int;
    }
    // SAFETY: malloc_trim only releases free heap pages and takes no pointers.
    unsafe {
        malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn return_freed_memory() {}

/// Keeps every thread of the process on glibc's main malloc arena. Each load
/// builds, runs and drops the session on threads that are new to it, and each
/// new thread can open another arena; measured on Linux, every arena kept 6 to
/// 24 MB after [`return_freed_memory`], so the idle daemon grew with each
/// reload until glibc ran out of arenas. With one arena it stayed flat, 7 MB
/// over the daemon without a model, at the same speed.
///
/// glibc gives a thread its arena on its first allocation, so this has to run
/// before the process starts any other thread.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn keep_one_malloc_arena() {
    const M_ARENA_MAX: std::ffi::c_int = -8;
    extern "C" {
        fn mallopt(param: std::ffi::c_int, value: std::ffi::c_int) -> std::ffi::c_int;
    }
    // SAFETY: mallopt only sets an allocator tunable and takes no pointers.
    unsafe {
        mallopt(M_ARENA_MAX, 1);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn keep_one_malloc_arena() {}

#[cfg(test)]
impl E5Embedder {
    /// Test-only constructor: take a pre-wired request sender connected to a
    /// hand-rolled mock worker. The real `from_parts` builds a session of an
    /// actual ~470 MB model, which this suite never does automatically —
    /// exercising it means running the app once and confirming embeddings
    /// come back, not something a fast unit test can assert on cheaply.
    pub(crate) fn from_mock_worker(req_tx: mpsc::UnboundedSender<EmbedRequest>) -> Self {
        Self {
            req_tx,
            dimensions: 384,
            model_id: "mock-embedder",
            metrics: Arc::default(),
        }
    }
}

// ============================================================================
// Compile-time Send + Sync regression guards.
//
// The assertion IS the test — if any of these types regress to !Send or !Sync
// the crate will fail to compile. No #[test] fn needed.
//
// `E5Embedder` holds only `mpsc::UnboundedSender` (Send + Sync), a `usize`,
// and a `&'static str` — provably Send + Sync without `unsafe`.
// ============================================================================
const _: fn() = || {
    fn _assert_send<T: Send>() {}
    fn _assert_sync<T: Sync>() {}
    _assert_send::<E5Embedder>();
    _assert_sync::<E5Embedder>();
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

/// What a test holds of a [`GatedEmbedder`]: it learns that a batch arrived and
/// lets batches through.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone)]
pub struct Gate(Arc<GateState>);

#[cfg(any(test, feature = "testing"))]
struct GateState {
    entered: tokio::sync::Notify,
    permits: tokio::sync::Semaphore,
}

#[cfg(any(test, feature = "testing"))]
impl Gate {
    /// Waits until a batch has reached the embedder. Arrivals are remembered,
    /// so one that came before the call still counts.
    pub async fn entered(&self) {
        self.0.entered.notified().await;
    }

    /// Lets `batches` batches through, the ones waiting now or arriving later.
    pub fn release(&self, batches: usize) {
        self.0.permits.add_permits(batches);
    }
}

/// `FakeEmbedder` whose batches wait for the [`Gate`] to let them through, so a
/// test decides what happens while an import is embedding. Single texts are
/// not held.
#[cfg(any(test, feature = "testing"))]
pub struct GatedEmbedder {
    gate: Gate,
}

#[cfg(any(test, feature = "testing"))]
impl GatedEmbedder {
    pub fn new() -> (Self, Gate) {
        let gate = Gate(Arc::new(GateState {
            entered: tokio::sync::Notify::new(),
            permits: tokio::sync::Semaphore::new(0),
        }));
        (Self { gate: gate.clone() }, gate)
    }
}

#[cfg(any(test, feature = "testing"))]
#[async_trait]
impl Embedder for GatedEmbedder {
    async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
        FakeEmbedder::default().embed_with_role(text, role).await
    }

    async fn embed_batch(&self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.gate.0.entered.notify_one();
        let permit = self.gate.0.permits.acquire().await.expect("the gate's semaphore is never closed");
        permit.forget();
        FakeEmbedder::default().embed_batch(texts, role).await
    }

    fn dimensions(&self) -> usize {
        384
    }

    fn model_id(&self) -> &str {
        "fake-embedder"
    }

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
    fn the_session_takes_every_thread_the_machine_reports() {
        let reported = std::num::NonZeroUsize::new(12).unwrap();
        assert_eq!(threads_from(Ok(reported)), 12);
    }

    #[test]
    fn an_unknown_parallelism_falls_back_to_two_threads() {
        let unknown = Err(std::io::Error::other("no cpu count"));
        assert_eq!(threads_from(unknown), FALLBACK_SESSION_THREADS);
        assert_eq!(FALLBACK_SESSION_THREADS, 2);
    }

    #[test]
    fn a_session_is_built_from_the_verified_view() {
        let dir = crate::test_dir::test_dir();
        let onnx = minimal_onnx();
        let path = dir.path().join("model.onnx");
        std::fs::write(&path, &onnx).unwrap();
        let sha = crate::memory::model_download::sha256_hex(&onnx);
        let lock = crate::memory::model_download::CacheLock::new();

        let view = crate::memory::model_download::verified_onnx(&path, &sha, lock.read_now()).expect("the model verifies");
        let session = build_session(&view).expect("the session is built from the view");
        assert_eq!(session.inputs().iter().map(|i| i.name()).collect::<Vec<_>>(), ["x"]);
        assert_eq!(session.outputs().iter().map(|o| o.name()).collect::<Vec<_>>(), ["y"]);
        drop(view);
        drop(session);
    }

    // ---------- inference pieces, against fastembed's own formulas ----------

    const WORDS: [&str; 7] = ["<s>", "<pad>", "</s>", "<unk>", "uno", "dos", "tres"];

    /// A tokenizer built in code with the shape of the model's: word-level
    /// ids, `<s>`/`</s>` added by a Roberta post-processor, no truncation and
    /// no padding.
    fn test_tokenizer() -> Tokenizer {
        use tokenizers::models::wordlevel::WordLevel;
        use tokenizers::pre_tokenizers::whitespace::WhitespaceSplit;
        use tokenizers::processors::roberta::RobertaProcessing;
        use tokenizers::AddedToken;

        let vocab = WORDS.iter().enumerate().map(|(i, w)| (w.to_string(), i as u32)).collect();
        let model = WordLevel::builder().vocab(vocab).unk_token("<unk>".into()).build().unwrap();
        let mut tokenizer = Tokenizer::new(model);
        tokenizer.with_pre_tokenizer(Some(WhitespaceSplit));
        tokenizer.with_post_processor(Some(RobertaProcessing::new(
            ("</s>".into(), 2),
            ("<s>".into(), 0),
        )));
        let specials: Vec<AddedToken> =
            WORDS[..4].iter().map(|w| AddedToken::from(*w, true)).collect();
        tokenizer.add_special_tokens(&specials);
        tokenizer
    }

    /// The tokenizer as fastembed configures its own copy
    /// (`fastembed-5.13.4/src/common.rs`): padding to the longest text with
    /// `pad_id` and truncation to the window.
    fn configured_like_fastembed(base: &Tokenizer, pad_id: u32) -> Tokenizer {
        use tokenizers::{PaddingParams, PaddingStrategy, TruncationParams};
        let mut tokenizer = base.clone();
        tokenizer
            .with_padding(Some(PaddingParams {
                strategy: PaddingStrategy::BatchLongest,
                pad_token: "<pad>".into(),
                pad_id,
                ..Default::default()
            }))
            .with_truncation(Some(TruncationParams {
                max_length: EMBED_MAX_TOKENS,
                ..Default::default()
            }))
            .unwrap();
        tokenizer
    }

    fn text_of(words: usize) -> String {
        (0..words).map(|i| WORDS[4 + i % 3]).collect::<Vec<_>>().join(" ")
    }

    fn flatten(encodings: &[Encoding]) -> (usize, Vec<i64>, Vec<i64>, Vec<i64>) {
        let len = encodings[0].len();
        let widen = |f: &dyn Fn(&Encoding) -> &[u32]| -> Vec<i64> {
            encodings.iter().flat_map(|e| f(e).iter().map(|x| *x as i64)).collect()
        };
        (
            len,
            widen(&|e| e.get_ids()),
            widen(&|e| e.get_attention_mask()),
            widen(&|e| e.get_type_ids()),
        )
    }

    #[test]
    fn an_encoding_matches_the_tokenizer_configured_like_fastembed() {
        let ours = test_tokenizer();
        let pad_id = 0;
        let theirs = configured_like_fastembed(&ours, pad_id);
        let at_the_edge = EMBED_MAX_TOKENS - 2;
        let batches: Vec<Vec<String>> = vec![
            vec![text_of(3)],
            vec![String::new(), text_of(1), text_of(5)],
            vec![text_of(at_the_edge - 1), text_of(at_the_edge), text_of(at_the_edge + 1)],
            vec![text_of(2), text_of(EMBED_MAX_TOKENS + 90), text_of(40)],
            vec![text_of(at_the_edge), text_of(4), text_of(at_the_edge + 7), text_of(0)],
        ];
        for batch in batches {
            let refs: Vec<&str> = batch.iter().map(String::as_str).collect();
            let expected = theirs.encode_batch(refs.clone(), true).unwrap();
            let (len, ids, mask, type_ids) = flatten(&expected);

            let padded = pad_right(&window_encodings(&ours, &refs).unwrap(), pad_id);

            assert_eq!(padded.rows, refs.len());
            assert_eq!(padded.len, len, "the padded length");
            assert_eq!(padded.ids, ids);
            assert_eq!(padded.mask, mask);
            assert_eq!(padded.type_ids, type_ids);
        }
    }

    #[test]
    fn a_long_text_is_cut_to_the_window_and_keeps_its_closing_token() {
        let tokenizer = test_tokenizer();
        let long = text_of(EMBED_MAX_TOKENS + 100);
        let encodings = window_encodings(&tokenizer, &[long.as_str()]).unwrap();
        let ids = encodings[0].get_ids();
        assert_eq!(ids.len(), EMBED_MAX_TOKENS);
        assert_eq!(ids[0], 0, "opening <s>");
        assert_eq!(ids[EMBED_MAX_TOKENS - 1], 2, "closing </s>");
        let untruncated = tokenizer.encode(long.as_str(), false).unwrap();
        assert_eq!(&ids[1..EMBED_MAX_TOKENS - 1], &untruncated.get_ids()[..EMBED_MAX_TOKENS - 2]);
        assert_eq!(
            untruncated.len(),
            EMBED_MAX_TOKENS + 100,
            "the tokenizer it was cut with is left untruncated"
        );
    }

    /// Random hidden states from a fixed seed, with exponents from 2^-6 to 2^2.
    fn hidden_states(count: usize) -> Vec<f32> {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        (0..count)
            .map(|_| {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let mantissa = ((state >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0;
                let exponent = ((state >> 20) % 9) as i32 - 6;
                mantissa * 2f32.powi(exponent) * 3.0
            })
            .collect()
    }

    /// `pooling::mean` and `common::normalize` of fastembed-5.13.4, written
    /// with ndarray as they are there.
    fn fastembed_pooling(hidden: &[f32], shape: [usize; 3], mask: &[i64]) -> Vec<Vec<f32>> {
        use ndarray::{Array2, ArrayView3, Axis};
        let [rows, len, _] = shape;
        let token_embeddings = ArrayView3::from_shape(shape, hidden).unwrap();
        let attention_mask = Array2::from_shape_vec((rows, len), mask.to_vec())
            .unwrap()
            .insert_axis(Axis(2))
            .broadcast(token_embeddings.dim())
            .unwrap()
            .mapv(|x| x as f32);
        let masked_tensor = &attention_mask * &token_embeddings;
        let sum = masked_tensor.sum_axis(Axis(1));
        let mask_sum = attention_mask.sum_axis(Axis(1));
        let mask_sum = mask_sum.mapv(|x| if x == 0f32 { 1.0 } else { x });
        let pooled = &sum / &mask_sum;
        pooled
            .rows()
            .into_iter()
            .map(|row| {
                let v = row.as_slice().unwrap();
                let norm = (v.iter().map(|val| val * val).sum::<f32>()).sqrt();
                let epsilon = 1e-12;
                v.iter().map(|&val| val / (norm + epsilon)).collect()
            })
            .collect()
    }

    #[test]
    fn mean_pooling_matches_fastembeds_formula_bit_for_bit() {
        let (rows, len, dim) = (4, 9, 24);
        let hidden = hidden_states(rows * len * dim);
        let mut mask = vec![1i64; rows * len];
        // Right padding of different widths, and a row with nothing but padding.
        for (row, keep) in [(0, 5), (1, 9), (2, 2), (3, 0)] {
            for t in keep..len {
                mask[row * len + t] = 0;
            }
        }
        let expected = fastembed_pooling(&hidden, [rows, len, dim], &mask);
        let ours = pool_and_normalize(&hidden, &[rows as i64, len as i64, dim as i64], &mask).unwrap();

        assert_eq!(ours.len(), rows);
        for (row, (a, b)) in ours.iter().zip(&expected).enumerate() {
            let a: Vec<u32> = a.iter().map(|x| x.to_bits()).collect();
            let b: Vec<u32> = b.iter().map(|x| x.to_bits()).collect();
            assert_eq!(a, b, "row {row}");
        }
    }

    #[test]
    fn a_two_dimensional_output_is_taken_as_pooled_and_normalized() {
        let ours = pool_and_normalize(&[3.0, 4.0, 0.0, 2.0], &[2, 2], &[1, 1, 1, 1]).unwrap();
        assert_eq!(ours.len(), 2);
        assert!((ours[0][0] - 0.6).abs() < 1e-6 && (ours[0][1] - 0.8).abs() < 1e-6);
        assert!((ours[1][1] - 1.0).abs() < 1e-6);
        assert!(matches!(
            pool_and_normalize(&[1.0], &[1], &[1]),
            Err(EmbedError::InferenceFailed(_))
        ));
    }

    #[test]
    fn the_output_is_picked_like_fastembed() {
        assert_eq!(pick_output(&["anything"]).unwrap(), 0, "the only output");
        assert_eq!(
            pick_output(&["sentence_embedding", "last_hidden_state", "text_embeds"]).unwrap(),
            2,
            "text_embeds first, whatever the order the model lists them in"
        );
        assert_eq!(pick_output(&["x", "sentence_embedding", "last_hidden_state"]).unwrap(), 2);
        assert_eq!(pick_output(&["sentence_embedding", "other"]).unwrap(), 0);
        assert!(matches!(pick_output(&["a", "b"]), Err(EmbedError::LoadFailed(_))));
        assert!(matches!(pick_output(&[]), Err(EmbedError::LoadFailed(_))));
    }

    const SPECIAL_TOKENS_MAP: &str = r#"{
        "bos_token": {"content": "<s>", "lstrip": false, "normalized": false, "rstrip": false, "single_word": false},
        "eos_token": "</s>",
        "pad_token": "<pad>",
        "unk_token": "<unk>"
    }"#;

    fn files(config: &str, special_tokens_map: &str, tokenizer_config: &str) -> TokenizerFiles {
        TokenizerFiles {
            tokenizer_file: Vec::new(),
            config_file: config.as_bytes().to_vec(),
            special_tokens_map_file: special_tokens_map.as_bytes().to_vec(),
            tokenizer_config_file: tokenizer_config.as_bytes().to_vec(),
        }
    }

    fn model_files() -> TokenizerFiles {
        files(r#"{"pad_token_id": 0}"#, SPECIAL_TOKENS_MAP, r#"{"model_max_length": 512}"#)
    }

    fn load_error(result: Result<u32, EmbedError>) -> String {
        match result {
            Err(EmbedError::LoadFailed(why)) => why,
            other => panic!("expected LoadFailed, got {other:?}"),
        }
    }

    #[test]
    fn the_load_accepts_the_files_of_the_model() {
        assert_eq!(check_tokenizer(&test_tokenizer(), &model_files()).unwrap(), 0);
    }

    #[test]
    fn the_pad_id_comes_from_config_json_and_defaults_to_zero() {
        let with = files(r#"{"pad_token_id": 7}"#, SPECIAL_TOKENS_MAP, r#"{"model_max_length": 512}"#);
        assert_eq!(check_tokenizer(&test_tokenizer(), &with).unwrap(), 7);
        let without = files("{}", SPECIAL_TOKENS_MAP, r#"{"model_max_length": 512}"#);
        assert_eq!(check_tokenizer(&test_tokenizer(), &without).unwrap(), 0);
        let broken = files("not json", SPECIAL_TOKENS_MAP, r#"{"model_max_length": 512}"#);
        assert!(matches!(check_tokenizer(&test_tokenizer(), &broken), Err(EmbedError::LoadFailed(_))));
    }

    #[test]
    fn the_load_refuses_a_tokenizer_with_its_own_truncation_or_padding() {
        use tokenizers::{PaddingParams, TruncationParams};
        let mut truncating = test_tokenizer();
        truncating.with_truncation(Some(TruncationParams::default())).unwrap();
        assert!(load_error(check_tokenizer(&truncating, &model_files())).contains("truncation"));

        let mut padding = test_tokenizer();
        padding.with_padding(Some(PaddingParams::default()));
        assert!(load_error(check_tokenizer(&padding, &model_files())).contains("padding"));
    }

    #[test]
    fn the_load_refuses_a_special_token_the_tokenizer_does_not_treat_as_special() {
        let map_with_mask = files(
            r#"{"pad_token_id": 0}"#,
            r#"{"eos_token": "</s>", "mask_token": {"content": "<mask>", "lstrip": true, "normalized": true, "rstrip": false, "single_word": false}}"#,
            r#"{"model_max_length": 512}"#,
        );
        // Not known to the tokenizer at all.
        assert!(load_error(check_tokenizer(&test_tokenizer(), &map_with_mask)).contains("<mask>"));

        // Known, but added as an ordinary token.
        let mut ordinary = test_tokenizer();
        ordinary.add_tokens(&[tokenizers::AddedToken::from("<mask>", false)]);
        assert!(load_error(check_tokenizer(&ordinary, &map_with_mask)).contains("<mask>"));

        // Added as special, it passes.
        let mut special = test_tokenizer();
        special.add_special_tokens(&[tokenizers::AddedToken::from("<mask>", true)]);
        assert!(check_tokenizer(&special, &map_with_mask).is_ok());
    }

    #[test]
    fn the_load_refuses_a_model_window_shorter_than_embed_max_tokens() {
        let short = files(r#"{}"#, SPECIAL_TOKENS_MAP, r#"{"model_max_length": 128}"#);
        assert!(load_error(check_tokenizer(&test_tokenizer(), &short)).contains("model_max_length"));
        let missing = files(r#"{}"#, SPECIAL_TOKENS_MAP, r#"{}"#);
        assert!(load_error(check_tokenizer(&test_tokenizer(), &missing)).contains("model_max_length"));
        let unbounded = files(r#"{}"#, SPECIAL_TOKENS_MAP, r#"{"model_max_length": 1e30}"#);
        assert!(check_tokenizer(&test_tokenizer(), &unbounded).is_ok());
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

    /// Pins the two constants the query composer's budget depends on, so a
    /// change to either is a decision and not a side effect.
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

    // ---------- E5Embedder worker plumbing ----------

    #[tokio::test]
    async fn every_batch_records_its_texts_by_role_and_the_worker_its_wait() {
        use crate::metrics::SeriesKey;
        let (req_tx, req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        let waits = Arc::new(Recorder::default());
        let worker_waits = waits.clone();
        std::thread::spawn(move || {
            serve_requests(req_rx, &worker_waits, |texts, _| Ok(vec![vec![0.5_f32; 384]; texts.len()]));
        });
        let local = E5Embedder::from_mock_worker(req_tx);

        let texts: Vec<String> = (0..EMBED_BATCH + 3).map(|i| format!("text {i}")).collect();
        local.embed_batch(&texts, EmbedRole::Passage).await.unwrap();
        local.embed_with_role("a query", EmbedRole::Query).await.unwrap();

        let gauges = local.metrics.minute().gauges;
        let passage = gauges[&SeriesKey::new("embedder.batch", &[("role", "passage")])];
        assert_eq!((passage.count, passage.max, passage.last), (2, EMBED_BATCH as f64, 3.0), "one sample per batch");
        assert_eq!(gauges[&SeriesKey::new("embedder.batch", &[("role", "query")])].last, 1.0);
        let run = waits.run();
        assert_eq!(run[&SeriesKey::new("embedder.wait", &[("role", "passage")])].count(), 2);
        assert_eq!(run[&SeriesKey::new("embedder.wait", &[("role", "query")])].count(), 1);
    }

    #[tokio::test]
    async fn embed_request_round_trip_through_channel() {
        // Hand-rolled mock worker: spawn a thread that receives EmbedRequest
        // and replies with a canned 384-dim vector.
        let (req_tx, mut req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        let canned: Vec<f32> = (0..384).map(|i| i as f32 * 0.001).collect();
        let canned_for_worker = canned.clone();
        std::thread::spawn(move || {
            while let Some(EmbedRequest { resp, .. }) =
                futures::executor::block_on(req_rx.recv())
            {
                let _ = resp.send(Ok(vec![canned_for_worker.clone()]));
            }
        });

        let local = E5Embedder::from_mock_worker(req_tx);
        let out = local.embed_with_role("anything", EmbedRole::Passage).await.unwrap();
        assert_eq!(out.len(), 384);
        assert_eq!(out, canned, "worker reply must flow back unchanged");
    }

    #[test]
    fn the_expected_contract_is_what_the_embedder_records() {
        let (req_tx, _req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        let embedder = E5Embedder::from_mock_worker(req_tx);
        let recorded = EmbeddingIdentity::of(&embedder);
        let expected = expected_contract();
        assert_eq!(expected.model_id, E5_MODEL_ID);
        assert_eq!(
            (expected.dim, expected.query_prefix, expected.passage_prefix),
            (recorded.dim, recorded.query_prefix, recorded.passage_prefix)
        );
    }

    /// `E5Embedder::embed_batch` must send exactly one channel request
    /// carrying every text, each already prefixed per `role`.
    #[tokio::test]
    async fn e5_embedder_sends_one_request_with_all_prefixed_texts() {
        use std::sync::{Arc, Mutex};

        let (req_tx, mut req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        let captured: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_for_worker = captured.clone();
        std::thread::spawn(move || {
            while let Some(EmbedRequest { texts, resp, .. }) =
                futures::executor::block_on(req_rx.recv())
            {
                captured_for_worker.lock().unwrap().push(texts.clone());
                let reply: Vec<Vec<f32>> = texts.iter().map(|_| vec![0.0f32; 384]).collect();
                let _ = resp.send(Ok(reply));
            }
        });

        let local = E5Embedder::from_mock_worker(req_tx);
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

    #[test]
    fn the_cpu_provider_turns_its_arena_off() {
        assert_eq!(format!("{:?}", cpu_provider()), "CPU { use_arena: false }");
    }

    /// A batch longer than `EMBED_BATCH` crosses the channel as requests of at
    /// most `EMBED_BATCH` texts, and its vectors come back in input order.
    #[tokio::test]
    async fn e5_embedder_sends_at_most_embed_batch_texts_per_request() {
        use std::sync::{Arc, Mutex};

        let (req_tx, mut req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        let sizes: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
        let sizes_for_worker = sizes.clone();
        std::thread::spawn(move || {
            while let Some(EmbedRequest { texts, resp, .. }) =
                futures::executor::block_on(req_rx.recv())
            {
                sizes_for_worker.lock().unwrap().push(texts.len());
                // Each vector carries the number its text ends with, so the
                // order of the joined reply can be checked.
                let reply: Vec<Vec<f32>> = texts
                    .iter()
                    .map(|t| vec![t.rsplit('-').next().unwrap().parse::<f32>().unwrap(); 384])
                    .collect();
                let _ = resp.send(Ok(reply));
            }
        });

        let local = E5Embedder::from_mock_worker(req_tx);
        let n = 2 * EMBED_BATCH + 3;
        let texts: Vec<String> = (0..n).map(|i| format!("chunk-{i}")).collect();
        let out = local.embed_batch(&texts, EmbedRole::Passage).await.unwrap();

        assert_eq!(*sizes.lock().unwrap(), vec![EMBED_BATCH, EMBED_BATCH, 3]);
        let order: Vec<f32> = out.iter().map(|v| v[0]).collect();
        assert_eq!(order, (0..n).map(|i| i as f32).collect::<Vec<_>>());
    }

    /// A reply with fewer (or more) vectors than texts submitted must be
    /// reported as InferenceFailed, never trusted positionally.
    #[tokio::test]
    async fn e5_embedder_rejects_reply_with_wrong_vector_count() {
        let (req_tx, mut req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        std::thread::spawn(move || {
            while let Some(EmbedRequest { resp, .. }) =
                futures::executor::block_on(req_rx.recv())
            {
                // Reply with only 1 vector for however many texts were sent.
                let _ = resp.send(Ok(vec![vec![0.0f32; 384]]));
            }
        });

        let local = E5Embedder::from_mock_worker(req_tx);
        let texts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let err = local.embed_batch(&texts, EmbedRole::Passage).await.unwrap_err();
        assert!(
            matches!(err, EmbedError::InferenceFailed(_)),
            "expected InferenceFailed for a vector-count mismatch, got {err:?}"
        );
    }

    #[tokio::test]
    async fn a_panicking_inference_answers_its_caller_and_ends_the_worker() {
        let (req_tx, req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        let worker = std::thread::spawn(move || {
            serve_requests(req_rx, &Recorder::default(), |_, _| panic!("the session broke"));
        });
        let local = E5Embedder::from_mock_worker(req_tx);

        let first = local.embed_with_role("anything", EmbedRole::Query).await.unwrap_err();
        match &first {
            EmbedError::InferenceFailed(cause) => {
                assert!(cause.contains("inference panicked"), "{cause}");
                assert!(cause.contains("the session broke"), "{cause}");
            }
            other => panic!("expected InferenceFailed, got {other:?}"),
        }
        worker.join().expect("the worker returns instead of unwinding");
        let second = local.embed_with_role("anything", EmbedRole::Query).await.unwrap_err();
        assert!(matches!(second, EmbedError::WorkerGone), "{second:?}");
    }

    #[tokio::test]
    async fn a_request_whose_caller_is_gone_is_not_inferred() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (req_tx, req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        let (gone_tx, gone_rx) = oneshot::channel();
        drop(gone_rx);
        req_tx
            .send(EmbedRequest {
                texts: vec!["nobody waits".into()],
                resp: gone_tx,
                role: EmbedRole::Passage,
                queued: std::time::Instant::now(),
            })
            .unwrap();

        let inferred = Arc::new(AtomicUsize::new(0));
        let counted = inferred.clone();
        std::thread::spawn(move || {
            serve_requests(req_rx, &Recorder::default(), |texts, _| {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok(texts.iter().map(|_| vec![0.0; 384]).collect())
            });
        });
        let local = E5Embedder::from_mock_worker(req_tx);
        local.embed_with_role("someone waits", EmbedRole::Query).await.expect("embed");
        assert_eq!(inferred.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn embed_returns_worker_gone_when_worker_drops() {
        let (req_tx, req_rx) = mpsc::unbounded_channel::<EmbedRequest>();
        // Drop receiver immediately to simulate the worker thread exiting.
        drop(req_rx);
        let local = E5Embedder::from_mock_worker(req_tx);
        let err = local.embed_with_role("anything", EmbedRole::Passage).await.unwrap_err();
        assert!(
            matches!(err, EmbedError::WorkerGone),
            "expected WorkerGone, got {err:?}"
        );
    }

    /// `E5Embedder::embed_with_role` must apply the E5 prefix BEFORE the
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
            while let Some(EmbedRequest { texts, resp, .. }) =
                futures::executor::block_on(req_rx.recv())
            {
                *captured_for_worker.lock().unwrap() = texts.first().cloned();
                let _ = resp.send(Ok(vec![vec![0.0f32; 384]]));
            }
        });

        let local = E5Embedder::from_mock_worker(req_tx);
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

    /// Everything that decides which vector a text gets, written as literals.
    /// Stored vectors are reused under the id of this contract, so a change to
    /// any of them that keeps the id would leave vectors of two spaces side by
    /// side.
    #[test]
    fn the_embedding_contract_is_pinned() {
        use crate::memory::model_download as md;
        let ask = "does E5_MODEL_ID have to change, so that stored vectors are embedded again?";

        assert_eq!(E5_MODEL_ID, "multilingual-e5-small/mean", "{ask}");
        assert!(E5_MODEL_ID.ends_with("/mean"), "the pooling is part of the id: {ask}");
        assert_eq!(E5_QUERY_PREFIX, "query: ", "{ask}");
        assert_eq!(E5_PASSAGE_PREFIX, "query: ", "{ask}");
        assert_eq!(EMBED_MAX_TOKENS, 512, "{ask}");
        assert_eq!(crate::memory::vector::EMBEDDING_DIM, 384, "{ask}");
        assert_eq!(
            md::MODEL_SHA256,
            "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665",
            "{ask}"
        );
        assert_eq!(
            md::TOKENIZER_JSON_SHA256,
            "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
            "{ask}"
        );
        assert_eq!(
            md::CONFIG_JSON_SHA256,
            "69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959",
            "{ask}"
        );
        assert_eq!(
            md::SPECIAL_TOKENS_MAP_SHA256,
            "d05497f1da52c5e09554c0cd874037a083e1dc1b9cfd48034d1c717f1afc07a7",
            "{ask}"
        );
        assert_eq!(
            md::TOKENIZER_CONFIG_SHA256,
            "a1d6bc8734a6f635dc158508bef000f8e2e5a759c7d92f984b2c86e5ff53425b",
            "{ask}"
        );

        // Two tokens count, the third is masked out: mean [2, 3], then L2.
        let hidden = [1.0f32, 2.0, 3.0, 4.0, 100.0, 200.0];
        let pooled = pool_and_normalize(&hidden, &[1, 3, 2], &[1, 1, 0]).unwrap();
        let bits: Vec<u32> = pooled[0].iter().map(|x| x.to_bits()).collect();
        assert_eq!(bits, vec![0x3f0e00d5, 0x3f550140], "pooling and normalization changed: {ask}");
    }
}
