//! Embedder bootstrap: download the model, load it, verify the embedding
//! contract, and fill the process-wide embedder cell.
//!
//! Hosts differ only in the event sink and in where the future is spawned, and
//! both of those are parameters here. The wire-shape event is
//! [`StreamEvent::EmbedderDownload`], from `clawg_core::models::chat`.
//!
//! # Testing seams
//!
//! [`run_embedder_bootstrap`] takes the model download, the model load and the
//! contract check as injected closures ([`EnsureModelFn`], [`LoadEmbedderFn`],
//! [`ContractVerifyFn`]) so the whole lifecycle — early exit, happy path,
//! download failure, contract mismatch, and the progress-forwarder pipeline —
//! is assertable in-process. [`production_deps`] builds the real ones.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use once_cell::sync::OnceCell;
use tokio::sync::watch;

use crate::memory::backup::{self, Reason};
use crate::memory::db::{ContractStatus, EmbeddingIdentity, StoredContract};
use crate::memory::embedder::{E5Embedder, Embedder, E5_MODEL_ID, LEGACY_CLS_MODEL_ID};
use crate::memory::idle_embedder::{IdleEmbedder, LoadWatch, ReloadFn};
use crate::memory::maintenance::{reembed_all, ReInferPlan, ReembedMode};
use crate::memory::model_download::{
    ensure_model, load_cached_model, parse_verified_tokenizer, CacheLock, DownloadError, DownloadProgress, ModelFiles,
    TokenizerFiles,
};
use crate::memory::MemoryStore;
use crate::metrics::{EventKind, Recorder, Unit};
use crate::models::chat::StreamEvent;

/// Whether the embedder can be used, as hosts report it.
///
/// Lives next to the embedder cell rather than replacing it: every reader of
/// the cell keeps working unchanged, and this only adds what an empty cell
/// cannot say, namely that the load failed and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedderStatus {
    Loading,
    Ready,
    Failed { error: String, retryable: bool },
}

/// Why one bootstrap attempt ended without an embedder.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BootstrapError {
    #[error("{0}")]
    Download(String),
    #[error("{0}")]
    Load(String),
    #[error(
        "embedding contract mismatch ({0}); memory is disabled until the mismatch is \
         resolved — retrying will not fix this"
    )]
    Mismatch(String),
    #[error("embedding contract verification failed: {0}")]
    Verify(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    Retry,
    GiveUp,
}

/// A mismatch is the only permanent failure: the stored vectors belong to
/// another model, and downloading this one again cannot change that. A
/// verification error is retried because it is a failed read, not a verdict.
pub fn classify(error: &BootstrapError) -> RetryDecision {
    match error {
        BootstrapError::Mismatch(_) => RetryDecision::GiveUp,
        BootstrapError::Download(_) | BootstrapError::Load(_) | BootstrapError::Verify(_) => {
            RetryDecision::Retry
        }
    }
}

/// Wait after the first failed attempt; each later failure doubles it.
const RETRY_BASE_DELAY: Duration = Duration::from_secs(30);
/// Longest wait between attempts: an offline machine still recovers within
/// half an hour of the network coming back, without retrying every minute
/// for days.
const RETRY_MAX_DELAY: Duration = Duration::from_secs(30 * 60);

/// How long to wait after the `attempt`-th failed attempt (1-based):
/// `min(30 s · 2^(attempt-1), 30 min)`, saturating instead of overflowing.
pub fn retry_delay(attempt: u32) -> Duration {
    2u32.checked_pow(attempt.saturating_sub(1))
        .and_then(|factor| RETRY_BASE_DELAY.checked_mul(factor))
        .map_or(RETRY_MAX_DELAY, |d| d.min(RETRY_MAX_DELAY))
}

/// Translate a backend [`DownloadProgress`] into the wire-shape
/// [`StreamEvent::EmbedderDownload`] that the frontend listens for.
///
/// Pure function — no I/O, no allocation beyond the small string field. The
/// translation is one-to-one with the download lifecycle (`start`,
/// `downloading`, `verifying`, `done`, `error`).
pub fn progress_to_stream_event(p: DownloadProgress) -> StreamEvent {
    match p {
        DownloadProgress::Start { bytes_total } => StreamEvent::EmbedderDownload {
            phase: "start".into(),
            bytes_downloaded: Some(0),
            bytes_total: Some(bytes_total),
            error: None,
        },
        DownloadProgress::Chunk {
            bytes_downloaded,
            bytes_total,
        } => StreamEvent::EmbedderDownload {
            phase: "downloading".into(),
            bytes_downloaded: Some(bytes_downloaded),
            bytes_total: Some(bytes_total),
            error: None,
        },
        DownloadProgress::Verify => StreamEvent::EmbedderDownload {
            phase: "verifying".into(),
            bytes_downloaded: None,
            bytes_total: None,
            error: None,
        },
        DownloadProgress::Done => StreamEvent::EmbedderDownload {
            phase: "done".into(),
            bytes_downloaded: None,
            bytes_total: None,
            error: None,
        },
        DownloadProgress::Error { message } => StreamEvent::EmbedderDownload {
            phase: "error".into(),
            bytes_downloaded: None,
            bytes_total: None,
            error: Some(message),
        },
    }
}

// ----------------------------------------------------------------------------
// EmbedderEventSink — abstraction over the host's event transport.
//
// A headless host wires `LogSink`. Tests inject a `MockSink` that records
// emissions to an in-memory `Vec<StreamEvent>`, which is how the "exactly one
// terminal `done`" assertion is covered at the bootstrap layer instead of the
// lower `ensure_model` layer.
// ----------------------------------------------------------------------------

/// Abstraction over the host's "tell the user about the download" transport,
/// so the bootstrap logic is unit-testable without a real host.
pub trait EmbedderEventSink: Send + Sync + 'static {
    fn emit_event(&self, event: StreamEvent);
}

/// Headless sink for hosts with no UI — the `clawg serve` daemon and the
/// `clawg mcp` bridge.
///
/// The lifecycle still has to be observable somewhere: a 470 MB download that
/// silently fails would otherwise present as "memory just doesn't work". Each
/// phase becomes one log line, and only the terminal `error` phase is a
/// `warn!` — the progress chatter is `info!` so a daemon running at the
/// default level still shows the download starting and finishing without
/// per-chunk noise dominating the log.
pub struct LogSink;

impl EmbedderEventSink for LogSink {
    fn emit_event(&self, event: StreamEvent) {
        let StreamEvent::EmbedderDownload {
            phase,
            bytes_downloaded,
            bytes_total,
            error,
        } = event;
        match (phase.as_str(), error) {
            ("error", err) => log::warn!(
                "embedder bootstrap failed: {}",
                err.unwrap_or_else(|| "unspecified error".into())
            ),
            // Per-chunk progress at `debug`: at one line per network chunk it
            // would bury everything else in the daemon's log.
            ("downloading", _) => log::debug!(
                "embedder model downloading: {}/{} bytes",
                bytes_downloaded.unwrap_or(0),
                bytes_total.unwrap_or(0)
            ),
            (other, _) => log::info!("embedder bootstrap: {other}"),
        }
    }
}

/// Type alias for the injectable `ensure_model` seam.
pub type EnsureModelFn = Arc<
    dyn Fn(
            std::path::PathBuf,
            Option<tokio::sync::mpsc::UnboundedSender<DownloadProgress>>,
        ) -> futures::future::BoxFuture<
            'static,
            Result<
                crate::memory::model_download::ModelFiles,
                crate::memory::model_download::DownloadError,
            >,
        > + Send
        + Sync,
>;

/// Type alias for the injectable read of the model from its cache alone, with
/// no download: what a reload of the idle model goes through.
pub type CachedModelFn = Arc<
    dyn Fn(std::path::PathBuf) -> futures::future::BoxFuture<'static, Result<ModelFiles, DownloadError>>
        + Send
        + Sync,
>;

/// Type alias for the injectable model-load seam (replaces
/// [`E5Embedder::from_parts`] in tests).
pub type LoadEmbedderFn = Arc<
    dyn Fn(crate::memory::model_download::ModelFiles) -> Result<Arc<dyn Embedder>, String>
        + Send
        + Sync,
>;

/// Type alias for the injectable embedding-contract verification seam.
///
/// It takes the loaded embedder itself, not only its identity: on a pooling
/// upgrade the check re-embeds the store with it (see [`verify_or_reembed`]).
pub type ContractVerifyFn = Arc<
    dyn Fn(Arc<dyn Embedder>) -> futures::future::BoxFuture<'static, Result<ContractStatus, String>>
        + Send
        + Sync,
>;

/// The model load: takes the process's tokenizer from `tokenizer`, or fills it
/// by parsing the bytes of the files it was handed, and builds the embedder
/// around that one tokenizer. With the cell already full, a reload parses
/// nothing and drops the `tokenizer.json` bytes it was given. `parse` and
/// `build` are parameters so a test can count the parses and skip the session.
pub(crate) fn model_loader<P, B>(
    tokenizer: Arc<OnceCell<Arc<tokenizers::Tokenizer>>>,
    parse: P,
    build: B,
) -> LoadEmbedderFn
where
    P: Fn(&[u8]) -> Result<tokenizers::Tokenizer, String> + Send + Sync + 'static,
    B: Fn(&[u8], Arc<tokenizers::Tokenizer>, &TokenizerFiles) -> Result<Arc<dyn Embedder>, String>
        + Send
        + Sync
        + 'static,
{
    Arc::new(move |files| {
        let shared = tokenizer
            .get_or_try_init(|| parse(&files.tokenizer.tokenizer_file).map(Arc::new))?
            .clone();
        build(&files.onnx, shared, &files.tokenizer)
    })
}

/// The contract check against `store`: the one dependency every host shares,
/// whichever model it loads.
fn contract_verifier(store: Arc<MemoryStore>, backups: std::path::PathBuf) -> ContractVerifyFn {
    // The store is the only host-supplied ingredient — `run_embedder_bootstrap`
    // itself stays store-agnostic and testable without a `MemoryStore` at all.
    Arc::new(move |embedder| {
        let (store, backups) = (store.clone(), backups.clone());
        Box::pin(async move { verify_or_reembed(&store, embedder, backups).await })
    })
}

/// What a host bootstraps the embedder with: the download (`ensure`), the
/// read of the cache alone (`cached`), the model load and the contract check.
pub struct EmbedderDeps {
    pub ensure: EnsureModelFn,
    pub cached: CachedModelFn,
    pub load: LoadEmbedderFn,
    pub verify: ContractVerifyFn,
}

/// The real [`EmbedderDeps`]: the HTTP download, the cache read, the ONNX
/// load, and the store's contract check.
///
/// Exposed as one function because they are a unit — every host needs all
/// three, and the host-specific ingredients are the open [`MemoryStore`] the
/// verifier closes over and the cell holding the process's tokenizer, which
/// the load shares with whoever else needs it. Keeping the closures here
/// rather than at each call site means no two hosts can drift into verifying
/// the contract against different stores or loading the model two different
/// ways.
///
/// `backups` is where the snapshot taken before a pooling upgrade goes.
pub fn production_deps(
    store: Arc<MemoryStore>,
    backups: std::path::PathBuf,
    tokenizer: Arc<OnceCell<Arc<tokenizers::Tokenizer>>>,
) -> EmbedderDeps {
    let lock = CacheLock::new();
    let ensure: EnsureModelFn = Arc::new({
        let lock = lock.clone();
        move |base_dir, progress| {
            let lock = lock.clone();
            Box::pin(async move { ensure_model(&base_dir, progress, &lock).await })
        }
    });
    let cached: CachedModelFn = Arc::new(move |base_dir| {
        let lock = lock.clone();
        Box::pin(async move { load_cached_model(&base_dir, &lock).await })
    });
    let metrics = store.metrics().clone();
    let load = model_loader(tokenizer, parse_verified_tokenizer, {
        let metrics = metrics.clone();
        move |onnx, tokenizer, files| {
            E5Embedder::from_parts(onnx, tokenizer, files, metrics.clone())
                .map(|embedder| Arc::new(embedder) as Arc<dyn Embedder>)
                .map_err(|e| e.to_string())
        }
    });
    EmbedderDeps {
        ensure,
        cached: timed_cache_read(cached, metrics.clone()),
        load: timed_load(load, metrics),
        verify: contract_verifier(store, backups),
    }
}

/// `cached` recording under `embedder.load{phase=verify}` how long a read of
/// the cache took, its hashes included, when it gave the files.
fn timed_cache_read(cached: CachedModelFn, metrics: Arc<Recorder>) -> CachedModelFn {
    Arc::new(move |base_dir| {
        let (cached, metrics) = (cached.clone(), metrics.clone());
        Box::pin(async move {
            let started = std::time::Instant::now();
            let files = cached(base_dir).await;
            if files.is_ok() {
                record_load(&metrics, "verify", started.elapsed());
            }
            files
        })
    })
}

/// `load` recording under `embedder.load{phase=session}` how long building the
/// embedder took, when it was built.
fn timed_load(load: LoadEmbedderFn, metrics: Arc<Recorder>) -> LoadEmbedderFn {
    Arc::new(move |files| {
        let started = std::time::Instant::now();
        let embedder = load(files);
        if embedder.is_ok() {
            record_load(&metrics, "session", started.elapsed());
        }
        embedder
    })
}

fn record_load(metrics: &Recorder, phase: &'static str, took: std::time::Duration) {
    metrics.observe("embedder.load", Unit::Micros, &[("phase", phase)], u64::try_from(took.as_micros()).unwrap_or(u64::MAX));
}

/// Env var that, in a build with the `testing` feature, replaces the model
/// `clawg serve` loads: `fake` loads [`FakeEmbedder`] and a word tokenizer at
/// once, `stalled` never finishes the download and `blocked` downloads at
/// once and never returns from the load, holding its blocking thread. It lets a test run the real binary without
/// the 470 MB model; a release build never reads it.
///
/// [`FakeEmbedder`]: crate::memory::embedder::FakeEmbedder
#[cfg(any(test, feature = "testing"))]
pub const TEST_EMBEDDER_VAR: &str = "CLAWG_TEST_EMBEDDER";

/// The stand-in [`TEST_EMBEDDER_VAR`] names.
#[cfg(any(test, feature = "testing"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestEmbedder {
    Fake,
    Stalled,
    Blocked,
}

#[cfg(any(test, feature = "testing"))]
impl TestEmbedder {
    /// `None` when unset or empty; an unknown value is an error rather than
    /// the real model, which would start a 470 MB download inside a test.
    pub fn parse(raw: Option<&str>) -> Result<Option<Self>, String> {
        match raw.map(str::trim) {
            None | Some("") => Ok(None),
            Some("fake") => Ok(Some(Self::Fake)),
            Some("stalled") => Ok(Some(Self::Stalled)),
            Some("blocked") => Ok(Some(Self::Blocked)),
            Some(other) => Err(format!(
                "{TEST_EMBEDDER_VAR}={other:?} is not a test embedder; use \"fake\", \"stalled\" or \"blocked\""
            )),
        }
    }

    /// The bootstrap dependencies for this stand-in, whose cache is whatever
    /// its download gives. The contract check is the real one, so a test
    /// store records and verifies the fake model's identity. A load fills
    /// `tokenizer` as the model's does, so what splits by tokens (an import
    /// of another tool's export) runs against the binary too.
    pub fn deps(
        self,
        store: Arc<MemoryStore>,
        backups: std::path::PathBuf,
        tokenizer: Arc<OnceCell<Arc<tokenizers::Tokenizer>>>,
    ) -> EmbedderDeps {
        let metrics = store.metrics().clone();
        let verify = contract_verifier(store, backups);
        let ensure: EnsureModelFn = match self {
            Self::Fake | Self::Blocked => Arc::new(|_base_dir, _progress| {
                Box::pin(async {
                    Ok(crate::memory::model_download::ModelFiles {
                        onnx: crate::memory::model_download::OnnxFile::in_memory(Vec::new()),
                        tokenizer: crate::memory::model_download::TokenizerFiles {
                            tokenizer_file: Vec::new(),
                            config_file: Vec::new(),
                            special_tokens_map_file: Vec::new(),
                            tokenizer_config_file: Vec::new(),
                        },
                    })
                })
            }),
            Self::Stalled => Arc::new(|_base_dir, _progress| Box::pin(futures::future::pending())),
        };
        let load: LoadEmbedderFn = match self {
            Self::Blocked => Arc::new(|_files| {
                log::info!("the test model load blocks its thread and never returns");
                loop {
                    std::thread::park();
                }
            }),
            Self::Fake | Self::Stalled => Arc::new(move |_files| {
                tokenizer.get_or_init(crate::memory::chunker::word_tokenizer);
                Ok(Arc::new(crate::memory::embedder::FakeEmbedder::default()) as Arc<dyn Embedder>)
            }),
        };
        let cached: CachedModelFn = {
            let ensure = ensure.clone();
            Arc::new(move |base_dir| ensure(base_dir, None))
        };
        // Measured as the real model's load is, so a test of the binary sees
        // the series.
        EmbedderDeps {
            ensure,
            cached: timed_cache_read(cached, metrics.clone()),
            load: timed_load(load, metrics),
            verify,
        }
    }
}

/// What `clawg serve` bootstraps against: [`production_deps`], or in a
/// `testing` build the stand-in [`TEST_EMBEDDER_VAR`] names.
#[cfg(feature = "testing")]
pub fn serve_deps(
    store: Arc<MemoryStore>,
    backups: std::path::PathBuf,
    tokenizer: Arc<OnceCell<Arc<tokenizers::Tokenizer>>>,
) -> Result<EmbedderDeps, String> {
    let raw = std::env::var(TEST_EMBEDDER_VAR).ok();
    Ok(match TestEmbedder::parse(raw.as_deref())? {
        Some(stand_in) => stand_in.deps(store, backups, tokenizer),
        None => production_deps(store, backups, tokenizer),
    })
}

/// What `clawg serve` bootstraps against: always [`production_deps`].
#[cfg(not(feature = "testing"))]
pub fn serve_deps(
    store: Arc<MemoryStore>,
    backups: std::path::PathBuf,
    tokenizer: Arc<OnceCell<Arc<tokenizers::Tokenizer>>>,
) -> Result<EmbedderDeps, String> {
    Ok(production_deps(store, backups, tokenizer))
}

/// The contract `clawg serve` verifies the store against, without loading a
/// model: the production embedder's, or in a `testing` build the stand-in's
/// when [`TEST_EMBEDDER_VAR`] names one (an unknown value is the production
/// one, which `serve` refuses before it gets that far).
#[cfg(feature = "testing")]
pub fn serving_contract() -> EmbeddingIdentity {
    match TestEmbedder::parse(std::env::var(TEST_EMBEDDER_VAR).ok().as_deref()) {
        Ok(Some(_)) => EmbeddingIdentity::of(&crate::memory::embedder::FakeEmbedder::default()),
        _ => crate::memory::embedder::expected_contract(),
    }
}

/// The contract `clawg serve` verifies the store against, without loading a
/// model: always the production embedder's.
#[cfg(not(feature = "testing"))]
pub fn serving_contract() -> EmbeddingIdentity {
    crate::memory::embedder::expected_contract()
}

/// `load`, with every embedder it builds wrapped in an [`IdleEmbedder`] that
/// lets the model go after `idle` without use, or never with `None`. A reload
/// goes through `cached` and `load` again, so it reads and checks the same
/// files the bootstrap did; the contract is not checked again, because the
/// reload refuses a model with another identity. The sweeper runs on
/// `runtime`.
///
/// A reload never downloads: it runs under the lock every call that needs a
/// vector waits on. When the cache no longer verifies, the reload answers at
/// once and `ensure` downloads the files again on `runtime`, one repair at a
/// time; the next call after it loads them.
pub fn unloading_when_idle(
    cached: CachedModelFn,
    ensure: EnsureModelFn,
    load: LoadEmbedderFn,
    base_dir: std::path::PathBuf,
    idle: Option<Duration>,
    watch: LoadWatch,
    runtime: tokio::runtime::Handle,
) -> LoadEmbedderFn {
    let repairing = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reload: ReloadFn = {
        let load = load.clone();
        let runtime = runtime.clone();
        Arc::new(move || {
            let (cached, load, base_dir) = (cached.clone(), load.clone(), base_dir.clone());
            let (ensure, repairing, runtime) = (ensure.clone(), repairing.clone(), runtime.clone());
            Box::pin(async move {
                let started = std::time::Instant::now();
                let files = match cached(base_dir.clone()).await {
                    Ok(files) => files,
                    Err(e) if !needs_repair(&e) => return Err(e.to_string()),
                    Err(e) => {
                        repair_in_background(&runtime, ensure, base_dir, repairing);
                        return Err(format!("{e}; the model is being downloaded again in the background"));
                    }
                };
                let read = started.elapsed();
                let model = tokio::task::spawn_blocking(move || load(files))
                    .await
                    .map_err(|e| format!("model load task panicked: {e}"))??;
                log::info!(
                    "embedding model files read and checked in {} ms, session built in {} ms",
                    read.as_millis(),
                    (started.elapsed() - read).as_millis()
                );
                Ok(model)
            })
        })
    };
    Arc::new(move |files| {
        let started = std::time::Instant::now();
        let loaded = load(files);
        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let outcome = if loaded.is_ok() { "loaded" } else { "failed" };
        watch.metrics.event(EventKind::ModelLoad, &[("cause", "start".into()), ("outcome", outcome.into()), ("ms", ms.into())]);
        Ok(Arc::new(IdleEmbedder::new(loaded?, reload.clone(), idle, watch.clone(), &runtime)) as Arc<dyn Embedder>)
    })
}

/// Whether a cache that failed to read with `e` needs files downloaded again.
/// A file another program holds is not missing, and downloading over it would
/// only fight that program for it.
fn needs_repair(e: &DownloadError) -> bool {
    !matches!(e, DownloadError::Busy { .. })
}

/// Starts `ensure` on `runtime` unless a repair is already running.
fn repair_in_background(
    runtime: &tokio::runtime::Handle,
    ensure: EnsureModelFn,
    base_dir: std::path::PathBuf,
    repairing: Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;
    /// Clears the mark however the repair ends, a panic included, or no
    /// repair would ever start again.
    struct Running(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    if repairing.swap(true, Ordering::AcqRel) {
        return;
    }
    let running = Running(repairing);
    runtime.spawn(async move {
        let _running = running;
        match ensure(base_dir, None).await {
            Ok(_) => log::info!("the model files were downloaded again; the next call loads them"),
            Err(e) => log::warn!("could not download the model files again: {e}"),
        }
    });
}

/// Whether `stored` describes the vectors this same model made before it
/// pooled by mean: only the pooling differs, so every vector can be remade
/// from its text and nothing else about the store has to change.
pub fn is_pooling_upgrade(stored: &StoredContract, actual: &EmbeddingIdentity) -> bool {
    stored.model_id == LEGACY_CLS_MODEL_ID
        && actual.model_id == E5_MODEL_ID
        && stored.dim == actual.dim as i64
        && stored.query_prefix == actual.query_prefix
        && stored.passage_prefix == actual.passage_prefix
}

/// Checks `embedder` against the store's embedding contract. A pooling
/// upgrade (see [`is_pooling_upgrade`]) is the one mismatch repaired here:
/// the store is snapshotted into `backups` and every vector re-embedded, and
/// the contract only changes once all of them were redone, so a pass that
/// fails part-way still reports the mismatch and the next start tries again.
/// Every other mismatch is returned as it is.
pub async fn verify_or_reembed(
    store: &MemoryStore,
    embedder: Arc<dyn Embedder>,
    backups: std::path::PathBuf,
) -> Result<ContractStatus, String> {
    let status = store
        .ensure_embedding_contract(EmbeddingIdentity::of(embedder.as_ref()))
        .await
        .map_err(|e| e.to_string())?;
    let (stored, actual) = match status {
        ContractStatus::Mismatch { stored, actual } if is_pooling_upgrade(&stored, &actual) => {
            (stored, actual)
        }
        other => return Ok(other),
    };
    log::info!(
        "the stored vectors were pooled by CLS; re-embedding them by mean, after a snapshot"
    );
    let snapshot = store
        .conn
        .call(move |c| -> rusqlite::Result<_> {
            Ok(backup::snapshot_into(c, &backups, Reason::PreReembed))
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("no snapshot before re-embedding: {e}"))?;
    log::info!("snapshot before re-embedding: {}", snapshot.display());
    let report = reembed_all(store, ReembedMode::ReInfer(ReInferPlan::new(embedder))).await;
    if report.contract_updated {
        log::info!("re-embedded the stored vectors: {:?}", report.per_table);
        Ok(ContractStatus::Match)
    } else {
        log::warn!(
            "re-embedding did not finish ({:?}); the store keeps its previous contract",
            report.per_table
        );
        Ok(ContractStatus::Mismatch { stored, actual })
    }
}

/// One bootstrap attempt, parametric in the event sink + dependency closures.
/// Production wires this to the host's sink plus [`production_deps`]. Tests
/// wire a `MockSink` + fake closures so the entire lifecycle (including the
/// forwarder → emit pipeline) can be asserted in-process.
///
/// Publishes `Loading` while it runs, then `Ready` or `Failed` on `status`.
/// On any error the cell stays empty, so a later attempt can still fill it.
pub async fn run_embedder_bootstrap(
    base_dir: std::path::PathBuf,
    embedder_cell: Arc<OnceCell<Arc<dyn Embedder>>>,
    status: watch::Sender<EmbedderStatus>,
    sink: Arc<dyn EmbedderEventSink>,
    ensure: EnsureModelFn,
    load: LoadEmbedderFn,
    verify: ContractVerifyFn,
) -> Result<(), BootstrapError> {
    if embedder_cell.get().is_some() {
        // Defensive: another bootstrap (or a test seed) already filled the
        // cell. Emit a synthetic `done` so the UI exits any pending "loading"
        // state and return. This is one of the two intended sources of the
        // terminal `done` (the other is the post-`from_bytes` branch below).
        sink.emit_event(StreamEvent::EmbedderDownload {
            phase: "done".into(),
            bytes_downloaded: None,
            bytes_total: None,
            error: None,
        });
        status.send_replace(EmbedderStatus::Ready);
        return Ok(());
    }
    status.send_replace(EmbedderStatus::Loading);

    let (progress_tx, mut progress_rx) =
        tokio::sync::mpsc::unbounded_channel::<DownloadProgress>();

    // Forwarder task: DownloadProgress → StreamEvent → sink.
    // Lives in its own task so the download future can keep pushing without
    // back-pressure. Terminates naturally when the last `progress_tx` clone
    // is dropped (which happens when `ensure` returns and `progress_tx` goes
    // out of scope). Since `ensure_model` never emits `Done`, the forwarder's
    // last forwarded event is `Verify` and there is no race with the
    // synthetic terminal `done` emitted below.
    let forwarder_sink = sink.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(p) = progress_rx.recv().await {
            let event = progress_to_stream_event(p);
            forwarder_sink.emit_event(event);
        }
    });

    let outcome = match ensure(base_dir, Some(progress_tx)).await {
        Err(e) => Err(BootstrapError::Download(e.to_string())),
        Ok(files) => {
            // CPU-bound (~100 ms in production): hand off to spawn_blocking so
            // the tokio worker thread isn't stalled. The injected `load`
            // closure in tests returns immediately, but spawn_blocking is fine
            // for both cases.
            let load_fn = load.clone();
            let load_res = tokio::task::spawn_blocking(move || load_fn(files)).await;
            match load_res {
                // Verified AFTER load, not before: `load` is an injected
                // closure (tests substitute `FakeEmbedder`), so `model_id`/
                // `dimensions()` are only known once it has returned — a
                // pre-load check would need a hardcoded model id and would
                // be false in every test. The 470 MB download already
                // happened inside `ensure` above regardless of the
                // contract's outcome; a wasted download on a genuine
                // mismatch is an accepted, named cost of this ordering.
                Ok(Ok(arc)) => {
                    match verify(arc.clone()).await {
                        Ok(ContractStatus::Match) | Ok(ContractStatus::Initialized) => {
                            // `.set` returns Err only if the cell is already filled —
                            // survivable race; treat both outcomes as success.
                            let _ = embedder_cell.set(arc);
                            Ok(())
                        }
                        Ok(ContractStatus::Mismatch { stored, actual }) => {
                            // Cell stays empty — the existing fail-soft reads in
                            // worker.rs and prompt_builder.rs already treat an
                            // empty cell as "memory disabled", so this needs no
                            // new degraded-mode logic of its own.
                            let diffs = crate::memory::db::differing_fields(&stored, &actual);
                            log::warn!(
                                "embedding contract mismatch ({}); memory reads/writes are \
                                 disabled until this is resolved",
                                diffs.join("; ")
                            );
                            Err(BootstrapError::Mismatch(diffs.join("; ")))
                        }
                        Err(e) => {
                            // Fail-closed: a contract we could not verify is a
                            // contract we cannot trust, same as an explicit
                            // mismatch. Not permanent — the next attempt
                            // re-runs the whole bootstrap.
                            log::warn!(
                                "embedding contract verification failed: {e}; treating as a \
                                 mismatch (fail-closed)"
                            );
                            Err(BootstrapError::Verify(e))
                        }
                    }
                }
                Ok(Err(e)) => Err(BootstrapError::Load(e)),
                Err(join_err) => {
                    Err(BootstrapError::Load(format!("model load task panicked: {join_err}")))
                }
            }
        }
    };

    // Await the forwarder so every lifecycle event from `ensure_model` has
    // landed in the sink BEFORE we emit our synthetic terminal event. This
    // guarantees the UI sees the events in source order (start → chunk* →
    // verify → done) instead of racing with the spawned task.
    let _ = forwarder.await;

    // The cell is already filled when `Ready` goes out, so a reader that sees
    // `Ready` can always use the embedder.
    let (final_event, new_status) = match &outcome {
        Ok(()) => (
            StreamEvent::EmbedderDownload {
                phase: "done".into(),
                bytes_downloaded: None,
                bytes_total: None,
                error: None,
            },
            EmbedderStatus::Ready,
        ),
        Err(e) => (
            StreamEvent::EmbedderDownload {
                phase: "error".into(),
                bytes_downloaded: None,
                bytes_total: None,
                error: Some(e.to_string()),
            },
            EmbedderStatus::Failed {
                error: e.to_string(),
                retryable: classify(e) == RetryDecision::Retry,
            },
        ),
    };
    sink.emit_event(final_event);
    status.send_replace(new_status);
    outcome
}

/// Runs [`run_embedder_bootstrap`] until it succeeds or fails for good.
///
/// A download, load or verification failure is retried after
/// [`retry_delay`], forever: a machine that was offline at startup gets
/// memory as soon as the network returns, without a restart. A contract
/// mismatch stops the loop (see [`classify`]). `sleep` is the wait itself,
/// injected so tests run without waiting; production passes
/// `tokio::time::sleep`.
#[allow(clippy::too_many_arguments)]
pub async fn run_embedder_bootstrap_with_retry<S, F>(
    base_dir: std::path::PathBuf,
    embedder_cell: Arc<OnceCell<Arc<dyn Embedder>>>,
    status: watch::Sender<EmbedderStatus>,
    sink: Arc<dyn EmbedderEventSink>,
    ensure: EnsureModelFn,
    load: LoadEmbedderFn,
    verify: ContractVerifyFn,
    sleep: S,
) where
    S: Fn(Duration) -> F,
    F: Future<Output = ()>,
{
    let mut attempt: u32 = 0;
    loop {
        attempt = attempt.saturating_add(1);
        let result = run_embedder_bootstrap(
            base_dir.clone(),
            embedder_cell.clone(),
            status.clone(),
            sink.clone(),
            ensure.clone(),
            load.clone(),
            verify.clone(),
        )
        .await;
        let Err(error) = result else { return };
        if classify(&error) == RetryDecision::GiveUp {
            return;
        }
        let delay = retry_delay(attempt);
        log::info!("embedder bootstrap attempt {attempt} failed; retrying in {}s", delay.as_secs());
        sleep(delay).await;
    }
}

// ============================================================================
// Tests — progress translator and bootstrap lifecycle
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    // ---------- progress_to_stream_event ----------

    #[test]
    fn translates_start() {
        let ev = progress_to_stream_event(DownloadProgress::Start { bytes_total: 100 });
        match ev {
            StreamEvent::EmbedderDownload {
                phase,
                bytes_downloaded,
                bytes_total,
                error,
            } => {
                assert_eq!(phase, "start");
                assert_eq!(bytes_downloaded, Some(0));
                assert_eq!(bytes_total, Some(100));
                assert_eq!(error, None);
            }
        }
    }

    #[test]
    fn translates_chunk_downloading() {
        let ev = progress_to_stream_event(DownloadProgress::Chunk {
            bytes_downloaded: 50,
            bytes_total: 100,
        });
        match ev {
            StreamEvent::EmbedderDownload {
                phase,
                bytes_downloaded,
                bytes_total,
                error,
            } => {
                assert_eq!(phase, "downloading");
                assert_eq!(bytes_downloaded, Some(50));
                assert_eq!(bytes_total, Some(100));
                assert_eq!(error, None);
            }
        }
    }

    #[test]
    fn translates_verify() {
        let ev = progress_to_stream_event(DownloadProgress::Verify);
        match ev {
            StreamEvent::EmbedderDownload {
                phase,
                bytes_downloaded,
                bytes_total,
                error,
            } => {
                assert_eq!(phase, "verifying");
                assert_eq!(bytes_downloaded, None);
                assert_eq!(bytes_total, None);
                assert_eq!(error, None);
            }
        }
    }

    #[test]
    fn translates_done() {
        let ev = progress_to_stream_event(DownloadProgress::Done);
        match ev {
            StreamEvent::EmbedderDownload {
                phase,
                bytes_downloaded,
                bytes_total,
                error,
            } => {
                assert_eq!(phase, "done");
                assert_eq!(bytes_downloaded, None);
                assert_eq!(bytes_total, None);
                assert_eq!(error, None);
            }
        }
    }

    #[test]
    fn translates_error() {
        let ev = progress_to_stream_event(DownloadProgress::Error {
            message: "boom".into(),
        });
        match ev {
            StreamEvent::EmbedderDownload {
                phase,
                bytes_downloaded,
                bytes_total,
                error,
            } => {
                assert_eq!(phase, "error");
                assert_eq!(bytes_downloaded, None);
                assert_eq!(bytes_total, None);
                assert_eq!(error, Some("boom".into()));
            }
        }
    }

    // ---------- run_embedder_bootstrap lifecycle ----------
    //
    // The host-facing entry points cannot be constructed without their
    // runtime, so they are verified by compile-time correctness PLUS this
    // parametric core, which is the unit-test seam: it accepts an injectable
    // `EmbedderEventSink`, `EnsureModelFn`, `LoadEmbedderFn`, and
    // `ContractVerifyFn`, so the full lifecycle (early-exit, happy path, error
    // path, contract mismatch, AND the forwarder→sink pipeline) can be
    // asserted in-process without a host runtime.

    use crate::memory::embedder::FakeEmbedder;
    use crate::memory::model_download::{DownloadError, ModelFiles, OnnxFile};
    use crate::memory::model_download::TokenizerFiles;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// Test-only event sink: records every emitted `StreamEvent` so the test
    /// can assert exact counts and ordering.
    #[derive(Default)]
    struct MockSink {
        events: Mutex<Vec<StreamEvent>>,
    }

    impl MockSink {
        fn snapshot(&self) -> Vec<StreamEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl EmbedderEventSink for MockSink {
        fn emit_event(&self, event: StreamEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn empty_tokenizer() -> TokenizerFiles {
        TokenizerFiles {
            tokenizer_file: Vec::new(),
            config_file: Vec::new(),
            special_tokens_map_file: Vec::new(),
            tokenizer_config_file: Vec::new(),
        }
    }

    fn empty_model_files() -> ModelFiles {
        ModelFiles {
            onnx: OnnxFile::in_memory(Vec::new()),
            tokenizer: empty_tokenizer(),
        }
    }

    #[tokio::test]
    async fn a_load_records_how_long_it_verified_and_built_and_a_failure_records_nothing() {
        use crate::memory::embedder::FakeEmbedder;
        use crate::metrics::SeriesKey;
        let metrics = Arc::new(Recorder::default());
        let counts = |metrics: &Recorder| {
            let run = metrics.run();
            ["verify", "session"].map(|phase| run.get(&SeriesKey::new("embedder.load", &[("phase", phase)])).map_or(0, |h| h.count()))
        };

        let cached: CachedModelFn = Arc::new(|_| Box::pin(async { Ok(empty_model_files()) }));
        let load: LoadEmbedderFn = Arc::new(|_| Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>));
        let files = timed_cache_read(cached, metrics.clone())(std::path::PathBuf::new()).await.unwrap();
        assert!(timed_load(load, metrics.clone())(files).is_ok());
        assert_eq!(counts(&metrics), [1, 1]);

        let cached: CachedModelFn = Arc::new(|_| Box::pin(async { Err(DownloadError::NotCached { files: vec!["model.onnx".into()] }) }));
        let load: LoadEmbedderFn = Arc::new(|_| Err("no session".to_string()));
        assert!(timed_cache_read(cached, metrics.clone())(std::path::PathBuf::new()).await.is_err());
        assert!(timed_load(load, metrics.clone())(empty_model_files()).is_err());
        assert_eq!(counts(&metrics), [1, 1], "a load that did not happen took no time");
    }

    /// Count `phase == "done"` occurrences across recorded `StreamEvent`s.
    fn count_done_phases(events: &[StreamEvent]) -> usize {
        events
            .iter()
            .filter(|e| {
                let StreamEvent::EmbedderDownload { phase, .. } = e;
                phase == "done"
            })
            .count()
    }

    fn phase_of_event(e: &StreamEvent) -> &str {
        let StreamEvent::EmbedderDownload { phase, .. } = e;
        phase
    }

    /// Test-only `ContractVerifyFn` that always reports a match — the
    /// behavior every lifecycle test except the mismatch test itself wants,
    /// so each one that reaches the seam gets a one-line "accept whatever
    /// arrives" verifier instead of repeating the closure inline.
    fn accept_contract() -> ContractVerifyFn {
        Arc::new(|_embedder| Box::pin(async { Ok(ContractStatus::Match) }))
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_unloading_embedder_reloads_through_the_same_files_and_load() {
        let (cached_calls, load_calls) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let cached: CachedModelFn = {
            let calls = cached_calls.clone();
            Arc::new(move |_base| {
                calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(empty_model_files()) })
            })
        };
        let load: LoadEmbedderFn = {
            let calls = load_calls.clone();
            Arc::new(move |_files| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>)
            })
        };
        let wrapped = unloading_when_idle(
            cached,
            never_download(),
            load,
            std::path::PathBuf::from("unused"),
            Some(Duration::from_secs(600)),
            LoadWatch::default(),
            tokio::runtime::Handle::current(),
        );

        let embedder = wrapped(empty_model_files()).expect("first load");
        assert!(embedder.resident());
        assert_eq!((cached_calls.load(Ordering::SeqCst), load_calls.load(Ordering::SeqCst)), (0, 1));

        tokio::time::sleep(Duration::from_secs(1200)).await;
        assert!(!embedder.resident());
        embedder.embed_with_role("again", crate::memory::embedder::EmbedRole::Query).await.expect("reload");
        assert!(embedder.resident());
        assert_eq!((cached_calls.load(Ordering::SeqCst), load_calls.load(Ordering::SeqCst)), (1, 2));
    }

    #[tokio::test(start_paused = true)]
    async fn without_an_idle_time_the_loaded_model_is_still_wrapped() {
        struct Dead;
        #[async_trait::async_trait]
        impl Embedder for Dead {
            async fn embed_with_role(
                &self,
                _: &str,
                _: crate::memory::embedder::EmbedRole,
            ) -> Result<Vec<f32>, crate::memory::embedder::EmbedError> {
                Err(crate::memory::embedder::EmbedError::WorkerGone)
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
        let loads = Arc::new(AtomicUsize::new(0));
        let load: LoadEmbedderFn = {
            let loads = loads.clone();
            Arc::new(move |_files| {
                if loads.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(Arc::new(Dead) as Arc<dyn Embedder>)
                } else {
                    Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>)
                }
            })
        };
        let cached: CachedModelFn = Arc::new(|_base| Box::pin(async { Ok(empty_model_files()) }));
        let wrapped = unloading_when_idle(
            cached,
            never_download(),
            load,
            std::path::PathBuf::from("unused"),
            None,
            LoadWatch::default(),
            tokio::runtime::Handle::current(),
        );

        let embedder = wrapped(empty_model_files()).expect("first load");
        tokio::time::sleep(Duration::from_secs(24 * 3600)).await;
        assert!(embedder.resident(), "never let go");
        embedder.embed_with_role("a text", crate::memory::embedder::EmbedRole::Query).await.expect("recovered");
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    /// A download a test does not expect: the reload of a good cache never
    /// reaches the network.
    fn never_download() -> EnsureModelFn {
        Arc::new(|_base, _progress| panic!("a reload of a good cache downloaded"))
    }

    #[tokio::test(start_paused = true)]
    async fn a_reload_with_a_damaged_cache_is_not_ready_and_repairs_it_once_in_the_background() {
        let repaired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let downloads = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(tokio::sync::Notify::new());
        let cached: CachedModelFn = {
            let repaired = repaired.clone();
            Arc::new(move |_base| {
                let repaired = repaired.load(Ordering::SeqCst);
                Box::pin(async move {
                    if repaired {
                        Ok(empty_model_files())
                    } else {
                        Err(DownloadError::NotCached { files: vec!["tokenizer_config.json".into()] })
                    }
                })
            })
        };
        let ensure: EnsureModelFn = {
            let (repaired, downloads, gate) = (repaired.clone(), downloads.clone(), gate.clone());
            Arc::new(move |_base, progress| {
                assert!(progress.is_none(), "a repair reports no download progress");
                downloads.fetch_add(1, Ordering::SeqCst);
                let (repaired, gate) = (repaired.clone(), gate.clone());
                Box::pin(async move {
                    gate.notified().await;
                    repaired.store(true, Ordering::SeqCst);
                    Ok(empty_model_files())
                })
            })
        };
        let load: LoadEmbedderFn = Arc::new(|_files| Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>));
        let wrapped = unloading_when_idle(
            cached,
            ensure,
            load,
            std::path::PathBuf::from("unused"),
            Some(Duration::from_secs(600)),
            LoadWatch::default(),
            tokio::runtime::Handle::current(),
        );
        let embedder = wrapped(empty_model_files()).expect("first load");
        tokio::time::sleep(Duration::from_secs(1200)).await;
        assert!(!embedder.resident());

        for _ in 0..2 {
            let answer = tokio::time::timeout(
                Duration::from_secs(1),
                embedder.embed_with_role("a text", crate::memory::embedder::EmbedRole::Query),
            )
            .await
            .expect("the call answered without waiting for the download");
            assert!(matches!(answer, Err(crate::memory::embedder::EmbedError::NotReady)), "{answer:?}");
        }
        tokio::task::yield_now().await;
        assert_eq!(downloads.load(Ordering::SeqCst), 1, "one repair at a time");

        gate.notify_one();
        while !repaired.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
        embedder
            .embed_with_role("a text", crate::memory::embedder::EmbedRole::Query)
            .await
            .expect("the repaired cache loads");
        assert!(embedder.resident());
        assert_eq!(downloads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn only_a_busy_file_spares_the_repair() {
        let busy = DownloadError::Busy { file: "onnx/model.onnx".into(), source: std::io::Error::from_raw_os_error(32) };
        assert!(!needs_repair(&busy));
        assert!(needs_repair(&DownloadError::NotCached { files: vec!["config.json".into()] }));
    }

    #[tokio::test(start_paused = true)]
    async fn a_persistent_sharing_violation_answers_not_ready_without_a_repair() {
        let downloads = Arc::new(AtomicUsize::new(0));
        let cached: CachedModelFn = Arc::new(|_base| {
            Box::pin(async {
                Err(DownloadError::Busy {
                    file: "onnx/model.onnx".into(),
                    source: std::io::Error::from_raw_os_error(32),
                })
            })
        });
        let ensure: EnsureModelFn = {
            let downloads = downloads.clone();
            Arc::new(move |_base, _progress| {
                downloads.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(empty_model_files()) })
            })
        };
        let load: LoadEmbedderFn = Arc::new(|_files| Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>));
        let wrapped = unloading_when_idle(
            cached,
            ensure,
            load,
            std::path::PathBuf::from("unused"),
            Some(Duration::from_secs(600)),
            LoadWatch::default(),
            tokio::runtime::Handle::current(),
        );
        let embedder = wrapped(empty_model_files()).expect("first load");
        tokio::time::sleep(Duration::from_secs(1200)).await;
        assert!(!embedder.resident());

        for _ in 0..2 {
            let answer = embedder.embed_with_role("a text", crate::memory::embedder::EmbedRole::Query).await;
            assert!(matches!(answer, Err(crate::memory::embedder::EmbedError::NotReady)), "{answer:?}");
        }
        tokio::task::yield_now().await;
        assert_eq!(downloads.load(Ordering::SeqCst), 0, "a file another program holds is not downloaded again");
    }

    fn tiny_tokenizer() -> tokenizers::Tokenizer {
        use tokenizers::models::wordlevel::WordLevel;
        let vocab = [("uno".to_string(), 0u32)].into_iter().collect();
        let model = WordLevel::builder().vocab(vocab).build().expect("a word-level model");
        tokenizers::Tokenizer::new(model)
    }

    /// A parser that counts how often a `tokenizer.json` reaches it.
    fn counting_parse(
        parses: Arc<AtomicUsize>,
    ) -> impl Fn(&[u8]) -> Result<tokenizers::Tokenizer, String> + Send + Sync + 'static {
        move |_bytes| {
            parses.fetch_add(1, Ordering::SeqCst);
            Ok(tiny_tokenizer())
        }
    }

    /// The `build` of a model load that makes a fake embedder and records the
    /// tokenizer it was handed.
    fn recording_build(
        seen: Arc<Mutex<Vec<Arc<tokenizers::Tokenizer>>>>,
    ) -> impl Fn(&[u8], Arc<tokenizers::Tokenizer>, &TokenizerFiles) -> Result<Arc<dyn Embedder>, String>
           + Send
           + Sync
           + 'static {
        move |_onnx, tokenizer, _files| {
            seen.lock().unwrap().push(tokenizer);
            Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>)
        }
    }

    #[test]
    fn the_worker_and_the_model_load_share_one_parse() {
        for worker_first in [true, false] {
            let cell: Arc<OnceCell<Arc<tokenizers::Tokenizer>>> = Arc::new(OnceCell::new());
            let parses = Arc::new(AtomicUsize::new(0));
            let seen = Arc::new(Mutex::new(Vec::new()));
            let load = model_loader(cell.clone(), counting_parse(parses.clone()), recording_build(seen.clone()));
            let worker = || {
                let parses = parses.clone();
                crate::memory::worker::resolve_tokenizer_and_chunker(
                    std::path::Path::new("unused"),
                    &cell,
                    move |_dir| {
                        parses.fetch_add(1, Ordering::SeqCst);
                        Some(Arc::new(tiny_tokenizer()))
                    },
                )
            };

            let chunker = if worker_first {
                let chunker = worker();
                load(empty_model_files()).expect("the model load");
                chunker
            } else {
                load(empty_model_files()).expect("the model load");
                worker()
            };

            assert_eq!(parses.load(Ordering::SeqCst), 1, "worker first: {worker_first}");
            let shared = cell.get().expect("the cell is filled");
            let loaded = seen.lock().unwrap();
            assert_eq!(loaded.len(), 1);
            assert!(Arc::ptr_eq(shared, &loaded[0]), "the model load got the cell's tokenizer");
            assert_eq!(
                chunker.tokenizer_ptr(),
                Some(Arc::as_ptr(shared)),
                "the chunker got the cell's tokenizer"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_reload_does_not_parse_the_tokenizer_again() {
        let cell: Arc<OnceCell<Arc<tokenizers::Tokenizer>>> = Arc::new(OnceCell::new());
        let parses = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let cached: CachedModelFn = Arc::new(|_base| Box::pin(async { Ok(empty_model_files()) }));
        let load = model_loader(cell.clone(), counting_parse(parses.clone()), recording_build(seen.clone()));
        let wrapped = unloading_when_idle(
            cached,
            never_download(),
            load,
            std::path::PathBuf::from("unused"),
            Some(Duration::from_secs(600)),
            LoadWatch::default(),
            tokio::runtime::Handle::current(),
        );

        let embedder = wrapped(empty_model_files()).expect("first load");
        assert_eq!(parses.load(Ordering::SeqCst), 1);

        tokio::time::sleep(Duration::from_secs(1200)).await;
        assert!(!embedder.resident());
        embedder.embed_with_role("again", crate::memory::embedder::EmbedRole::Query).await.expect("reload");

        assert!(embedder.resident());
        assert_eq!(parses.load(Ordering::SeqCst), 1, "the reload built a session and parsed nothing");
        let loaded = seen.lock().unwrap();
        assert_eq!(loaded.len(), 2, "the model was built again");
        assert!(Arc::ptr_eq(&loaded[0], &loaded[1]), "both builds got the same tokenizer");
    }

    #[tokio::test]
    async fn run_embedder_bootstrap_is_noop_when_cell_filled() {
        // Cell already seeded → bootstrap emits a single synthetic `done`
        // (so the UI exits its loading state) and skips both `ensure` and
        // `load`. We assert: zero `ensure`/`load` calls, exactly one event,
        // and that event is `done`.
        let cell: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
        cell.set(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>)
            .ok();
        let cell = Arc::new(cell);

        let ensure_calls = Arc::new(AtomicUsize::new(0));
        let load_calls = Arc::new(AtomicUsize::new(0));

        let ec = ensure_calls.clone();
        let ensure: EnsureModelFn = Arc::new(move |_base, _tx| {
            let ec = ec.clone();
            Box::pin(async move {
                ec.fetch_add(1, Ordering::SeqCst);
                Ok(empty_model_files())
            })
        });
        let lc = load_calls.clone();
        let load: LoadEmbedderFn = Arc::new(move |_files| {
            lc.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>)
        });

        let sink = Arc::new(MockSink::default());
        let sink_for_run: Arc<dyn EmbedderEventSink> = sink.clone();
        let _ = run_embedder_bootstrap(
            std::env::temp_dir(),
            cell,
            tokio::sync::watch::Sender::new(EmbedderStatus::Loading),
            sink_for_run,
            ensure,
            load,
            accept_contract(),
        )
        .await;

        assert_eq!(
            ensure_calls.load(Ordering::SeqCst),
            0,
            "ensure must NOT be invoked when cell is already filled"
        );
        assert_eq!(
            load_calls.load(Ordering::SeqCst),
            0,
            "load must NOT be invoked when cell is already filled"
        );

        let events = sink.snapshot();
        assert_eq!(events.len(), 1, "expected exactly one event, got {events:?}");
        assert_eq!(phase_of_event(&events[0]), "done");
        assert_eq!(
            count_done_phases(&events),
            1,
            "exactly one terminal `done` on the noop path"
        );
    }

    #[tokio::test]
    async fn run_embedder_bootstrap_calls_ensure_when_cell_empty() {
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());

        let ensure_calls = Arc::new(AtomicUsize::new(0));
        let load_calls = Arc::new(AtomicUsize::new(0));

        let ec = ensure_calls.clone();
        let ensure: EnsureModelFn = Arc::new(move |_base, _tx| {
            let ec = ec.clone();
            Box::pin(async move {
                ec.fetch_add(1, Ordering::SeqCst);
                Ok(empty_model_files())
            })
        });
        let lc = load_calls.clone();
        let load: LoadEmbedderFn = Arc::new(move |_files| {
            lc.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>)
        });

        let sink: Arc<dyn EmbedderEventSink> = Arc::new(MockSink::default());
        let _ = run_embedder_bootstrap(
            std::env::temp_dir(),
            cell.clone(),
            tokio::sync::watch::Sender::new(EmbedderStatus::Loading),
            sink,
            ensure,
            load,
            accept_contract(),
        )
        .await;

        assert_eq!(
            ensure_calls.load(Ordering::SeqCst),
            1,
            "ensure must be called exactly once on the empty-cell path"
        );
        assert_eq!(
            load_calls.load(Ordering::SeqCst),
            1,
            "load must be called exactly once on success"
        );
        assert!(
            cell.get().is_some(),
            "OnceCell must be filled after successful load"
        );
    }

    /// On the happy path the UI MUST receive EXACTLY ONE terminal `done` event,
    /// emitted as the LAST event of the lifecycle by the bootstrap (after
    /// `load` + `OnceCell::set`), NOT by `ensure_model`.
    ///
    /// If the forwarder relayed a `Done` from `ensure_model` AND the bootstrap
    /// emitted its own synthetic `done`, the UI would see `done` twice.
    /// This test pumps a realistic lifecycle (`Start → Chunk → Verify`)
    /// through the channel and asserts (a) exactly one `done`, (b) it is the
    /// LAST event, (c) the preceding events are in source order.
    #[tokio::test]
    async fn run_embedder_bootstrap_emits_exactly_one_done_on_happy_path() {
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());

        let ensure: EnsureModelFn = Arc::new(move |_base, progress| {
            Box::pin(async move {
                // Simulate the real `ensure_model` event sequence per the
                // contract: Start → Chunk → Verify. NO Done — that
                // is the bootstrap's responsibility.
                if let Some(tx) = progress {
                    let _ = tx.send(DownloadProgress::Start { bytes_total: 100 });
                    let _ = tx.send(DownloadProgress::Chunk {
                        bytes_downloaded: 50,
                        bytes_total: 100,
                    });
                    let _ = tx.send(DownloadProgress::Chunk {
                        bytes_downloaded: 100,
                        bytes_total: 100,
                    });
                    let _ = tx.send(DownloadProgress::Verify);
                }
                Ok(empty_model_files())
            })
        });
        let load: LoadEmbedderFn =
            Arc::new(move |_files| Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>));

        let sink = Arc::new(MockSink::default());
        let sink_for_run: Arc<dyn EmbedderEventSink> = sink.clone();
        let _ = run_embedder_bootstrap(
            std::env::temp_dir(),
            cell.clone(),
            tokio::sync::watch::Sender::new(EmbedderStatus::Loading),
            sink_for_run,
            ensure,
            load,
            accept_contract(),
        )
        .await;

        let events = sink.snapshot();
        let phases: Vec<&str> = events.iter().map(phase_of_event).collect();

        // (a) EXACTLY one terminal `done` — the core invariant.
        assert_eq!(
            count_done_phases(&events),
            1,
            "expected exactly one `done` event, got phases {phases:?}"
        );

        // (b) `done` MUST be the LAST event.
        assert_eq!(
            phases.last().copied(),
            Some("done"),
            "`done` must be the LAST event; got {phases:?}"
        );

        // (c) Preceding events arrive in source order: start → downloading*
        // → verifying → done. (Forwarder is awaited before the synthetic
        // emit so ordering is deterministic.) Note the wire phases are
        // `"downloading"` / `"verifying"` — `progress_to_stream_event` maps
        // `DownloadProgress::Chunk` → `"downloading"` and
        // `DownloadProgress::Verify` → `"verifying"`.
        assert_eq!(
            phases.first().copied(),
            Some("start"),
            "first event must be `start`; got {phases:?}"
        );
        let verify_idx = phases
            .iter()
            .position(|p| *p == "verifying")
            .expect("a verifying event must be relayed by the forwarder");
        let done_idx = phases.iter().position(|p| *p == "done").unwrap();
        assert!(
            verify_idx < done_idx,
            "verifying must precede done; got {phases:?}"
        );
        assert!(
            phases.iter().any(|p| *p == "downloading"),
            "at least one `downloading` event expected; got {phases:?}"
        );

        // No error events on the success path.
        assert!(
            !phases.iter().any(|p| *p == "error"),
            "no `error` events on the success path; got {phases:?}"
        );

        // OnceCell was filled.
        assert!(cell.get().is_some(), "OnceCell must be filled");
    }

    /// Error path: when `ensure` returns Err, the bootstrap emits exactly one
    /// terminal `error` event and the cell stays empty.
    #[tokio::test]
    async fn run_embedder_bootstrap_emits_terminal_error_when_ensure_fails() {
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());

        let ensure: EnsureModelFn = Arc::new(move |_base, _progress| {
            Box::pin(async move {
                Err(DownloadError::RetryExhausted {
                    file: "onnx/model.onnx".into(),
                    host: "huggingface.co".into(),
                    attempts: 3,
                    cause: "HTTP 503 Service Unavailable".into(),
                })
            })
        });
        let load: LoadEmbedderFn =
            Arc::new(move |_files| Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>));

        let sink = Arc::new(MockSink::default());
        let sink_for_run: Arc<dyn EmbedderEventSink> = sink.clone();
        let _ = run_embedder_bootstrap(
            std::env::temp_dir(),
            cell.clone(),
            tokio::sync::watch::Sender::new(EmbedderStatus::Loading),
            sink_for_run,
            ensure,
            load,
            accept_contract(),
        )
        .await;

        let events = sink.snapshot();
        let phases: Vec<&str> = events.iter().map(phase_of_event).collect();

        assert_eq!(
            phases.last().copied(),
            Some("error"),
            "`error` must be the LAST event on the error path; got {phases:?}"
        );
        assert_eq!(
            count_done_phases(&events),
            0,
            "no `done` events on the error path; got {phases:?}"
        );
        assert!(cell.get().is_none(), "OnceCell must stay empty on error");
    }

    /// On a contract mismatch, the bootstrap MUST NOT call `embedder_cell.set()`
    /// (the empty `OnceCell` reuses the fail-soft path `worker.rs`/`prompt_builder.rs`
    /// already have for "no embedder") and MUST terminate with an `error` event
    /// carrying a message that identifies the mismatch — not a generic download
    /// failure the user might think a retry can fix.
    #[tokio::test]
    async fn run_embedder_bootstrap_skips_set_on_contract_mismatch() {
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());

        let ensure: EnsureModelFn =
            Arc::new(move |_base, _progress| Box::pin(async move { Ok(empty_model_files()) }));
        let load: LoadEmbedderFn =
            Arc::new(move |_files| Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>));
        let verify: ContractVerifyFn = Arc::new(|_embedder| {
            Box::pin(async move {
                Ok(ContractStatus::Mismatch {
                    stored: crate::memory::db::StoredContract {
                        model_id: "multilingual-e5-small".into(),
                        dim: 384,
                        query_prefix: "".into(),
                        passage_prefix: "".into(),
                    },
                    actual: EmbeddingIdentity {
                        model_id: "fake-embedder".into(),
                        dim: 384,
                        query_prefix: "".into(),
                        passage_prefix: "".into(),
                    },
                })
            })
        });

        let sink = Arc::new(MockSink::default());
        let sink_for_run: Arc<dyn EmbedderEventSink> = sink.clone();
        let _ = run_embedder_bootstrap(
            std::env::temp_dir(),
            cell.clone(),
            tokio::sync::watch::Sender::new(EmbedderStatus::Loading),
            sink_for_run,
            ensure,
            load,
            verify,
        )
        .await;

        assert!(
            cell.get().is_none(),
            "OnceCell must stay empty on a contract mismatch"
        );

        let events = sink.snapshot();
        let phases: Vec<&str> = events.iter().map(phase_of_event).collect();
        assert_eq!(
            phases.last().copied(),
            Some("error"),
            "mismatch must terminate with `error`; got {phases:?}"
        );
        assert_eq!(
            count_done_phases(&events),
            0,
            "no `done` events when the contract mismatches; got {phases:?}"
        );
        match events.last() {
            Some(StreamEvent::EmbedderDownload {
                error: Some(msg), ..
            }) => {
                assert!(
                    msg.to_lowercase().contains("contract")
                        || msg.to_lowercase().contains("mismatch"),
                    "error message must identify a contract mismatch (not a generic \
                     download failure a retry could fix), got: {msg}"
                );
            }
            other => panic!("expected a terminal EmbedderDownload error event, got {other:?}"),
        }
    }

    /// A mismatch on `passage_prefix` ALONE (matching `model_id`/`dim`) must
    /// degrade exactly like a model_id/dim mismatch: cell stays empty, the
    /// terminal event is `error`, and chat is untouched (this test only
    /// covers the memory subsystem — chat has no dependency on this cell at
    /// all, so there is nothing further to assert here about it).
    #[tokio::test]
    async fn run_embedder_bootstrap_skips_set_on_prefix_only_mismatch() {
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());

        let ensure: EnsureModelFn =
            Arc::new(move |_base, _progress| Box::pin(async move { Ok(empty_model_files()) }));
        let load: LoadEmbedderFn =
            Arc::new(move |_files| Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>));
        let verify: ContractVerifyFn = Arc::new(|_embedder| {
            Box::pin(async move {
                Ok(ContractStatus::Mismatch {
                    stored: crate::memory::db::StoredContract {
                        model_id: "fake-embedder".into(),
                        dim: 384,
                        query_prefix: "query: ".into(),
                        passage_prefix: "passage:".into(), // stale: missing trailing space
                    },
                    actual: EmbeddingIdentity {
                        model_id: "fake-embedder".into(),
                        dim: 384,
                        query_prefix: "query: ".into(),
                        passage_prefix: "passage: ".into(),
                    },
                })
            })
        });

        let sink = Arc::new(MockSink::default());
        let sink_for_run: Arc<dyn EmbedderEventSink> = sink.clone();
        let _ = run_embedder_bootstrap(
            std::env::temp_dir(),
            cell.clone(),
            tokio::sync::watch::Sender::new(EmbedderStatus::Loading),
            sink_for_run,
            ensure,
            load,
            verify,
        )
        .await;

        assert!(
            cell.get().is_none(),
            "OnceCell must stay empty on a prefix-only contract mismatch"
        );

        let events = sink.snapshot();
        let phases: Vec<&str> = events.iter().map(phase_of_event).collect();
        assert_eq!(
            phases.last().copied(),
            Some("error"),
            "prefix-only mismatch must terminate with `error`; got {phases:?}"
        );
        assert_eq!(count_done_phases(&events), 0);
        match events.last() {
            Some(StreamEvent::EmbedderDownload {
                error: Some(msg), ..
            }) => {
                assert!(
                    msg.contains("passage_prefix"),
                    "error message must name the differing field, got: {msg}"
                );
            }
            other => panic!("expected a terminal EmbedderDownload error event, got {other:?}"),
        }
    }

    // ---------- LogSink ----------

    /// The headless sink must swallow every phase without panicking — it runs
    /// inside a fire-and-forget bootstrap task where a panic would take the
    /// download reporting down silently.
    #[test]
    fn log_sink_accepts_every_phase_without_panicking() {
        let sink = LogSink;
        for p in [
            DownloadProgress::Start { bytes_total: 10 },
            DownloadProgress::Chunk {
                bytes_downloaded: 5,
                bytes_total: 10,
            },
            DownloadProgress::Verify,
            DownloadProgress::Done,
            DownloadProgress::Error {
                message: "boom".into(),
            },
        ] {
            sink.emit_event(progress_to_stream_event(p));
        }
    }

    /// `LogSink` must be usable as the sink argument itself, not merely as a
    /// struct — that is the whole reason it exists for the daemon.
    #[tokio::test]
    async fn log_sink_can_drive_a_full_bootstrap_run() {
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());
        let ensure: EnsureModelFn =
            Arc::new(move |_base, _progress| Box::pin(async move { Ok(empty_model_files()) }));
        let load: LoadEmbedderFn =
            Arc::new(move |_files| Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>));

        let _ = run_embedder_bootstrap(
            std::env::temp_dir(),
            cell.clone(),
            tokio::sync::watch::Sender::new(EmbedderStatus::Loading),
            Arc::new(LogSink),
            ensure,
            load,
            accept_contract(),
        )
        .await;

        assert!(cell.get().is_some(), "LogSink must not interfere with the cell fill");
    }

    // ---------- production_deps ----------

    /// The dependencies must be buildable from nothing but an open store — the daemon
    /// has no other context to offer.
    #[tokio::test]
    async fn production_deps_builds_all_three_closures_from_a_store() {
        let store = Arc::new(MemoryStore::new_for_test().await.expect("test store"));
        let EmbedderDeps { load, verify, .. } =
            production_deps(store, std::env::temp_dir(), Arc::new(OnceCell::new()));

        // `load` is pure: hand it empty model bytes and it must report a
        // load failure rather than panicking or hanging.
        assert!(
            load(empty_model_files()).is_err(),
            "loading empty ONNX bytes must fail, not panic"
        );

        // `verify` must reach the real store: a fresh store has no contract
        // row, so the first identity initializes it.
        let status = verify(Arc::new(FakeEmbedder::default()))
        .await
        .expect("contract check must reach the store");
        assert!(matches!(status, ContractStatus::Initialized));
    }

    // ---------- test embedder ----------

    #[test]
    fn the_test_embedder_is_named_by_its_variable_or_absent() {
        assert_eq!(TestEmbedder::parse(None), Ok(None));
        assert_eq!(TestEmbedder::parse(Some(" ")), Ok(None));
        assert_eq!(TestEmbedder::parse(Some("fake")), Ok(Some(TestEmbedder::Fake)));
        assert_eq!(TestEmbedder::parse(Some("stalled")), Ok(Some(TestEmbedder::Stalled)));
        assert_eq!(TestEmbedder::parse(Some("blocked")), Ok(Some(TestEmbedder::Blocked)));
        let err = TestEmbedder::parse(Some("real")).expect_err("an unknown value is refused");
        assert!(err.contains(TEST_EMBEDDER_VAR) && err.contains("\"fake\""), "{err}");
    }

    #[tokio::test]
    async fn the_fake_test_embedder_loads_at_once_and_records_its_contract() {
        let store = Arc::new(MemoryStore::new_for_test().await.expect("test store"));
        let tokenizer = Arc::new(OnceCell::new());
        let EmbedderDeps { ensure, cached, load, verify } =
            TestEmbedder::Fake.deps(store, std::env::temp_dir(), tokenizer.clone());

        let files = tokio::time::timeout(Duration::from_secs(1), ensure(std::env::temp_dir(), None))
            .await
            .expect("no download")
            .expect("files");
        tokio::time::timeout(Duration::from_secs(1), cached(std::env::temp_dir()))
            .await
            .expect("no download")
            .expect("the same files from the cache");
        assert!(tokenizer.get().is_none());
        let embedder = load(files).expect("the fake loads");
        assert_eq!(embedder.model_id(), "fake-embedder");
        assert!(tokenizer.get().is_some(), "a load gives the process its tokenizer, as the model's does");
        let status = verify(embedder).await.expect("the real contract check");
        assert!(matches!(status, ContractStatus::Initialized));
    }

    #[tokio::test]
    async fn the_stalled_test_embedder_never_finishes_the_download() {
        let store = Arc::new(MemoryStore::new_for_test().await.expect("test store"));
        let EmbedderDeps { ensure, .. } = TestEmbedder::Stalled.deps(store, std::env::temp_dir(), Arc::new(OnceCell::new()));

        let waited = tokio::time::timeout(Duration::from_millis(200), ensure(std::env::temp_dir(), None)).await;
        assert!(waited.is_err(), "the download must still be running");
    }

    // ---------- embedder status ----------

    fn mismatch_verify() -> ContractVerifyFn {
        Arc::new(|_embedder| {
            Box::pin(async move {
                Ok(ContractStatus::Mismatch {
                    stored: crate::memory::db::StoredContract {
                        model_id: "multilingual-e5-small".into(),
                        dim: 384,
                        query_prefix: "".into(),
                        passage_prefix: "".into(),
                    },
                    actual: EmbeddingIdentity {
                        model_id: "fake-embedder".into(),
                        dim: 384,
                        query_prefix: "".into(),
                        passage_prefix: "".into(),
                    },
                })
            })
        })
    }

    fn refused_download() -> EnsureModelFn {
        Arc::new(|_base, _progress| {
            Box::pin(async move {
                Err(DownloadError::RetryExhausted {
                    file: "onnx/model.onnx".into(),
                    host: "huggingface.co".into(),
                    attempts: 3,
                    cause: "connection refused".into(),
                })
            })
        })
    }

    fn fake_load() -> LoadEmbedderFn {
        Arc::new(|_files| Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>))
    }

    fn ok_download() -> EnsureModelFn {
        Arc::new(|_base, _progress| Box::pin(async move { Ok(empty_model_files()) }))
    }

    #[tokio::test]
    async fn a_failed_download_publishes_failed_and_retryable() {
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());
        let status = tokio::sync::watch::Sender::new(EmbedderStatus::Loading);

        let result = run_embedder_bootstrap(
            std::env::temp_dir(),
            cell,
            status.clone(),
            Arc::new(MockSink::default()),
            refused_download(),
            fake_load(),
            accept_contract(),
        )
        .await;

        assert!(matches!(result, Err(BootstrapError::Download(_))), "got {result:?}");
        let current = status.borrow().clone();
        match current {
            EmbedderStatus::Failed { error, retryable } => {
                assert!(error.contains("connection refused"), "got {error}");
                assert!(retryable);
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_contract_mismatch_publishes_failed_and_not_retryable() {
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());
        let status = tokio::sync::watch::Sender::new(EmbedderStatus::Loading);

        let result = run_embedder_bootstrap(
            std::env::temp_dir(),
            cell,
            status.clone(),
            Arc::new(MockSink::default()),
            ok_download(),
            fake_load(),
            mismatch_verify(),
        )
        .await;

        assert!(matches!(result, Err(BootstrapError::Mismatch(_))), "got {result:?}");
        assert!(
            matches!(&*status.borrow(), EmbedderStatus::Failed { retryable: false, .. }),
            "got {:?}",
            *status.borrow()
        );
    }

    #[tokio::test]
    async fn a_loaded_embedder_fills_the_cell_before_publishing_ready() {
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());
        let status = tokio::sync::watch::Sender::new(EmbedderStatus::Loading);
        let mut seen = status.subscribe();
        let watched_cell = cell.clone();
        let watcher = tokio::spawn(async move {
            seen.wait_for(|s| *s == EmbedderStatus::Ready).await.expect("status sender alive");
            watched_cell.get().is_some()
        });

        let result = run_embedder_bootstrap(
            std::env::temp_dir(),
            cell,
            status.clone(),
            Arc::new(MockSink::default()),
            ok_download(),
            fake_load(),
            accept_contract(),
        )
        .await;

        assert!(result.is_ok());
        assert!(watcher.await.unwrap(), "Ready must never be visible with an empty cell");
    }

    #[test]
    fn classify_retries_everything_but_a_contract_mismatch() {
        assert_eq!(classify(&BootstrapError::Download("x".into())), RetryDecision::Retry);
        assert_eq!(classify(&BootstrapError::Load("x".into())), RetryDecision::Retry);
        assert_eq!(classify(&BootstrapError::Verify("x".into())), RetryDecision::Retry);
        assert_eq!(classify(&BootstrapError::Mismatch("x".into())), RetryDecision::GiveUp);
    }

    // ---------- retry ----------

    #[test]
    fn retry_delay_doubles_from_30s_and_caps_at_30min() {
        let secs = |n| retry_delay(n).as_secs();
        assert_eq!(secs(1), 30);
        assert_eq!(secs(2), 60);
        assert_eq!(secs(3), 120);
        assert_eq!(secs(6), 960);
        assert_eq!(secs(7), 1800);
        assert_eq!(secs(40), 1800);
        assert_eq!(secs(u32::MAX), 1800);
    }

    /// Records each requested wait and the status visible during it, and
    /// returns at once so the test does not sleep.
    fn recording_sleep(
        status: watch::Receiver<EmbedderStatus>,
    ) -> (Arc<Mutex<Vec<(std::time::Duration, EmbedderStatus)>>>, impl Fn(std::time::Duration) -> futures::future::Ready<()>) {
        let waits = Arc::new(Mutex::new(Vec::new()));
        let recorded = waits.clone();
        let sleep = move |d| {
            recorded.lock().unwrap().push((d, status.borrow().clone()));
            futures::future::ready(())
        };
        (waits, sleep)
    }

    #[tokio::test]
    async fn a_failed_download_is_retried_until_it_succeeds() {
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());
        let status = watch::Sender::new(EmbedderStatus::Loading);
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let ensure: EnsureModelFn = Arc::new(move |_base, _progress| {
            let n = counted.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if n == 0 {
                    Err(DownloadError::RetryExhausted {
                        file: "onnx/model.onnx".into(),
                        host: "huggingface.co".into(),
                        attempts: 3,
                        cause: "connection refused".into(),
                    })
                } else {
                    Ok(empty_model_files())
                }
            })
        });
        let (waits, sleep) = recording_sleep(status.subscribe());

        run_embedder_bootstrap_with_retry(
            std::env::temp_dir(),
            cell.clone(),
            status.clone(),
            Arc::new(MockSink::default()),
            ensure,
            fake_load(),
            accept_contract(),
            sleep,
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(cell.get().is_some());
        assert_eq!(*status.borrow(), EmbedderStatus::Ready);
        let waits = waits.lock().unwrap().clone();
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].0, std::time::Duration::from_secs(30));
        assert!(
            matches!(waits[0].1, EmbedderStatus::Failed { retryable: true, .. }),
            "the wait must show a retryable failure, got {:?}",
            waits[0].1
        );
    }

    #[tokio::test]
    async fn a_contract_mismatch_is_never_retried() {
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());
        let status = watch::Sender::new(EmbedderStatus::Loading);
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let ensure: EnsureModelFn = Arc::new(move |_base, _progress| {
            counted.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Ok(empty_model_files()) })
        });
        let (waits, sleep) = recording_sleep(status.subscribe());

        run_embedder_bootstrap_with_retry(
            std::env::temp_dir(),
            cell.clone(),
            status.clone(),
            Arc::new(MockSink::default()),
            ensure,
            fake_load(),
            mismatch_verify(),
            sleep,
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(waits.lock().unwrap().is_empty());
        assert!(matches!(&*status.borrow(), EmbedderStatus::Failed { retryable: false, .. }));
    }

    // ---------- pooling upgrade ----------

    fn contract(model_id: &str, dim: i64, prefix: &str) -> crate::memory::db::StoredContract {
        crate::memory::db::StoredContract {
            model_id: model_id.into(),
            dim,
            query_prefix: prefix.into(),
            passage_prefix: prefix.into(),
        }
    }

    fn identity(model_id: &str, dim: usize, prefix: &str) -> EmbeddingIdentity {
        EmbeddingIdentity {
            model_id: model_id.into(),
            dim,
            query_prefix: prefix.into(),
            passage_prefix: prefix.into(),
        }
    }

    #[test]
    fn only_the_cls_vectors_of_the_same_model_are_a_pooling_upgrade() {
        let mean = identity(E5_MODEL_ID, 384, "query: ");

        assert!(is_pooling_upgrade(&contract(LEGACY_CLS_MODEL_ID, 384, "query: "), &mean));
        assert!(!is_pooling_upgrade(&contract(LEGACY_CLS_MODEL_ID, 768, "query: "), &mean));
        assert!(!is_pooling_upgrade(&contract(LEGACY_CLS_MODEL_ID, 384, "passage: "), &mean));
        assert!(!is_pooling_upgrade(&contract("another-model", 384, "query: "), &mean));
        assert!(!is_pooling_upgrade(
            &contract(LEGACY_CLS_MODEL_ID, 384, "query: "),
            &identity("another-model", 384, "query: ")
        ));
    }

    /// [`FakeEmbedder`]'s vectors under the mean-pooled model's name.
    struct MeanPooled(FakeEmbedder);

    #[async_trait::async_trait]
    impl Embedder for MeanPooled {
        async fn embed_with_role(
            &self,
            text: &str,
            role: crate::memory::embedder::EmbedRole,
        ) -> Result<Vec<f32>, crate::memory::embedder::EmbedError> {
            self.0.embed_with_role(text, role).await
        }
        fn dimensions(&self) -> usize {
            self.0.dimensions()
        }
        fn model_id(&self) -> &str {
            E5_MODEL_ID
        }
        fn query_prefix(&self) -> &str {
            ""
        }
        fn passage_prefix(&self) -> &str {
            ""
        }
    }

    async fn store_with_contract(model_id: &'static str, dim: i64) -> MemoryStore {
        let store = MemoryStore::new_for_test().await.expect("test store");
        store
            .conn
            .call(move |c| -> rusqlite::Result<()> {
                c.execute(
                    "INSERT INTO embedding_contract \
                     (id, model_id, dim, metric, normalized, query_prefix, passage_prefix, created_at) \
                     VALUES (1, ?1, ?2, 'l2_unit', 1, '', '', 0)",
                    rusqlite::params![model_id, dim],
                )?;
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, tier, importance, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', 'a', 'the build uses the pinned toolchain', 'chat_message', \
                             3, 5.0, 0, 0, 0, 'agent', '')",
                    [],
                )?;
                let vector: Vec<u8> = [1.0f32; 384].iter().flat_map(|f| f.to_le_bytes()).collect();
                c.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (?1, ?2)",
                    rusqlite::params![c.last_insert_rowid(), vector],
                )?;
                Ok(())
            })
            .await
            .expect("seed the store");
        store
    }

    async fn stored_model_id(store: &MemoryStore) -> String {
        store
            .conn
            .call(|c| c.query_row("SELECT model_id FROM embedding_contract", [], |r| r.get(0)))
            .await
            .expect("contract row")
    }

    fn files_in(dir: &std::path::Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn cls_vectors_are_snapshotted_then_reembedded_by_mean() {
        let store = store_with_contract(LEGACY_CLS_MODEL_ID, 384).await;
        let backups = crate::test_dir::test_dir();

        let status = verify_or_reembed(
            &store,
            Arc::new(MeanPooled(FakeEmbedder::default())),
            backups.path().to_path_buf(),
        )
        .await
        .expect("verify");

        assert!(matches!(status, ContractStatus::Match), "{status:?}");
        assert_eq!(stored_model_id(&store).await, E5_MODEL_ID);
        let snapshots = files_in(backups.path());
        assert_eq!(snapshots.len(), 1, "{snapshots:?}");
        assert!(snapshots[0].ends_with("-pre-reembed.db"), "{snapshots:?}");
    }

    #[tokio::test]
    async fn any_other_mismatch_is_left_as_it_is() {
        let store = store_with_contract("another-model", 384).await;
        let backups = crate::test_dir::test_dir();

        let status = verify_or_reembed(
            &store,
            Arc::new(MeanPooled(FakeEmbedder::default())),
            backups.path().to_path_buf(),
        )
        .await
        .expect("verify");

        assert!(matches!(status, ContractStatus::Mismatch { .. }), "{status:?}");
        assert_eq!(stored_model_id(&store).await, "another-model");
        assert!(files_in(backups.path()).is_empty());
    }

    #[tokio::test]
    async fn a_store_that_matches_is_not_touched() {
        let store = store_with_contract(E5_MODEL_ID, 384).await;
        let backups = crate::test_dir::test_dir();

        let status = verify_or_reembed(
            &store,
            Arc::new(MeanPooled(FakeEmbedder::default())),
            backups.path().to_path_buf(),
        )
        .await
        .expect("verify");

        assert!(matches!(status, ContractStatus::Match), "{status:?}");
        assert!(files_in(backups.path()).is_empty());
    }
}
