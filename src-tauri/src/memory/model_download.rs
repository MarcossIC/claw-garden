//! First-run model download + SHA-256 verification.
//!
//! # Test indirection: HuggingFace base URL
//!
//! Production code hits `https://huggingface.co/intfloat/multilingual-e5-small/resolve/main`.
//! Tests MUST NEVER hit the real HF — that would be slow, flaky, and would make the
//! suite depend on a third-party host being up. So `base_url()` consults the
//! `CLAW_GARDEN_HF_BASE_URL` env var; tests
//! set it to a `mockito::Server::url()` value, prod leaves it unset and falls back to
//! [`HF_BASE_URL`]. The `ensure_model_with_oracle` test seam also accepts an explicit
//! `base_url` parameter so individual tests can be parallel-safe (env vars are global).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

use fastembed::TokenizerFiles;

/// Hard-coded HuggingFace base URL (production). Tests override via `base_url()`.
pub(super) const HF_BASE_URL: &str =
    "https://huggingface.co/intfloat/multilingual-e5-small/resolve/main";

// ============================================================================
// File specifications.
//
// The model is 5 files. Each has:
//   - a relative path under `<base>/models/multilingual-e5-small/`
//   - a hex SHA-256 that downloaded bytes MUST match
//   - a `FileKind` that maps the bytes into `ModelFiles` (onnx vs tokenizer field)
//
// SHA-256 sources (recorded 2026-05-23 against commit
// `614241f622f53c4eeff9890bdc4f31cfecc418b3`):
//
//   onnx/model.onnx           — LFS oid + X-Linked-ETag (470 268 510 bytes)
//   tokenizer.json            — LFS oid                (17 082 730 bytes)
//   config.json               — non-LFS; SHA-256 computed locally on 2026-05-24
//                                from the file served by `/raw/<commit>/config.json`
//                                (uses /raw NOT /resolve so we pin against the
//                                immutable commit, not the moving `main` ref).
//                                Real on-disk size 655 bytes.
//   special_tokens_map.json   — non-LFS; SHA-256 computed locally on 2026-05-24
//                                (167 bytes).
//   tokenizer_config.json     — non-LFS; SHA-256 computed locally on 2026-05-24
//                                (443 bytes).
//
// The 3 small JSON SHAs were pinned by
// downloading from /raw/<commit>/<file> and running PowerShell's
// `Get-FileHash -Algorithm SHA256`. See `scripts/compute_sha.ps1` for the
// repro recipe. Production callers use [`HF_BASE_URL`] which hits /resolve/main
// — the SHA contract holds as long as commit
// `614241f622f53c4eeff9890bdc4f31cfecc418b3` is still on the main branch tip
// (or pointing to identical bytes). Bump the commit + re-run the script to
// rotate.
// ============================================================================

pub const MODEL_SHA256: &str =
    "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665";
pub const TOKENIZER_JSON_SHA256: &str =
    "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39";
pub const CONFIG_JSON_SHA256: &str =
    "69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959";
pub const SPECIAL_TOKENS_MAP_SHA256: &str =
    "d05497f1da52c5e09554c0cd874037a083e1dc1b9cfd48034d1c717f1afc07a7";
pub const TOKENIZER_CONFIG_SHA256: &str =
    "a1d6bc8734a6f635dc158508bef000f8e2e5a759c7d92f984b2c86e5ff53425b";

/// Discriminates how a file's bytes are projected into [`ModelFiles`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Onnx,
    TokenizerJson,
    ConfigJson,
    SpecialTokensMap,
    TokenizerConfig,
}

/// One entry of the model manifest.
#[derive(Debug, Clone)]
pub struct FileSpec {
    pub path: &'static str,
    pub sha256: &'static str,
    pub kind: FileKind,
}

pub const ALL_FILES: [FileSpec; 5] = [
    FileSpec {
        path: "onnx/model.onnx",
        sha256: MODEL_SHA256,
        kind: FileKind::Onnx,
    },
    FileSpec {
        path: "tokenizer.json",
        sha256: TOKENIZER_JSON_SHA256,
        kind: FileKind::TokenizerJson,
    },
    FileSpec {
        path: "config.json",
        sha256: CONFIG_JSON_SHA256,
        kind: FileKind::ConfigJson,
    },
    FileSpec {
        path: "special_tokens_map.json",
        sha256: SPECIAL_TOKENS_MAP_SHA256,
        kind: FileKind::SpecialTokensMap,
    },
    FileSpec {
        path: "tokenizer_config.json",
        sha256: TOKENIZER_CONFIG_SHA256,
        kind: FileKind::TokenizerConfig,
    },
];

/// Returns the path where `tokenizer.json` is expected to live under `base_dir`.
///
/// Path: `<base_dir>/models/multilingual-e5-small/tokenizer.json`.
/// Single source of truth for that path: `ChatChunker::new_from_base_dir` and
/// the query-context token counter both go through here, so they can never
/// drift onto two different tokenizer files.
pub fn tokenizer_path(base_dir: &Path) -> PathBuf {
    base_dir.join("models").join("multilingual-e5-small").join("tokenizer.json")
}

/// Loads the shared tokenizer every consumer (the chat chunker, the query
/// composer's token counter) reads from the same file. Owns the path
/// resolution itself via [`tokenizer_path`] so callers never construct that
/// path a second, independently-editable way.
///
/// Returns `None` — never panics — when the file is absent (still
/// downloading, or download failed) or malformed (corrupted write, wrong
/// format): both are recoverable states a caller degrades from (byte-length
/// chunking, or skipping query-context composition for this turn) rather
/// than states that should crash chat or ingestion.
pub(crate) fn load_tokenizer(base_dir: &Path) -> Option<Arc<tokenizers::Tokenizer>> {
    let path = tokenizer_path(base_dir);
    match tokenizers::Tokenizer::from_file(&path) {
        Ok(tok) => Some(Arc::new(tok)),
        Err(e) => {
            log::warn!("load_tokenizer: not loadable at {} ({e}); returning None", path.display());
            None
        }
    }
}

/// Returns the base URL for HF resolve requests. Honors the
/// `CLAW_GARDEN_HF_BASE_URL` env var to enable mockito-driven tests.
pub(super) fn base_url() -> String {
    std::env::var("CLAW_GARDEN_HF_BASE_URL").unwrap_or_else(|_| HF_BASE_URL.to_string())
}

/// Minimum free disk space required before initiating a fresh download (600 MB).
pub const MIN_FREE_BYTES: u64 = 600 * 1024 * 1024;

/// Disk-space probe seam. Production uses [`Fs2Oracle`]; tests inject
/// `MockDiskOracle::returning(b)` to bypass the real filesystem.
pub trait DiskOracle: Send + Sync {
    fn available_bytes(&self, path: &Path) -> std::io::Result<u64>;
}

/// Production `DiskOracle` backed by the `fs2` crate.
pub struct Fs2Oracle;

impl DiskOracle for Fs2Oracle {
    fn available_bytes(&self, path: &Path) -> std::io::Result<u64> {
        fs2::available_space(path)
    }
}

/// Retry-backoff schedule: 5s before attempt 2, 30s before
/// attempt 3. The 120s entry exists for symmetry but is never consumed (after
/// 3 attempts the retry loop exits with `RetryExhausted`).
pub(crate) const RETRY_BACKOFF_SECS: [u64; 3] = [5, 30, 120];

/// Backoff-policy seam. Production [`RealBackoff`]
/// uses `tokio::time::sleep`; the test-only [`ZeroBackoff`] returns immediately
/// so the suite never sleeps for real seconds.
pub trait BackoffPolicy: Send + Sync {
    /// Wait the prescribed number of seconds for the given retry attempt.
    /// Implementations MUST NOT panic for any `attempt` value.
    fn sleep_for_attempt<'a>(
        &'a self,
        attempt: u32,
    ) -> futures::future::BoxFuture<'a, ()>;

    /// Introspection helper for tests/logging (no sleep). Out-of-range
    /// attempts return the longest delay in the schedule (saturating).
    fn secs_for_attempt(&self, attempt: u32) -> u64;
}

/// Production backoff: consults [`RETRY_BACKOFF_SECS`] and actually sleeps.
pub struct RealBackoff;

impl BackoffPolicy for RealBackoff {
    fn sleep_for_attempt<'a>(
        &'a self,
        attempt: u32,
    ) -> futures::future::BoxFuture<'a, ()> {
        let secs = self.secs_for_attempt(attempt);
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
        })
    }

    fn secs_for_attempt(&self, attempt: u32) -> u64 {
        let i = (attempt as usize).min(RETRY_BACKOFF_SECS.len() - 1);
        RETRY_BACKOFF_SECS[i]
    }
}

/// Test-only backoff: every call resolves with zero delay.
#[cfg(test)]
pub(crate) struct ZeroBackoff;

#[cfg(test)]
impl BackoffPolicy for ZeroBackoff {
    fn sleep_for_attempt<'a>(
        &'a self,
        _attempt: u32,
    ) -> futures::future::BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    fn secs_for_attempt(&self, _attempt: u32) -> u64 {
        0
    }
}

/// Bytes returned from a successful `ensure_model` call.
pub struct ModelFiles {
    pub onnx: Vec<u8>,
    pub tokenizer: TokenizerFiles,
}

impl std::fmt::Debug for ModelFiles {
    // `TokenizerFiles` from fastembed does NOT impl Debug. We summarize sizes
    // instead of the byte contents so test failure output stays readable.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelFiles")
            .field("onnx_len", &self.onnx.len())
            .field("tokenizer_len", &self.tokenizer.tokenizer_file.len())
            .field("config_len", &self.tokenizer.config_file.len())
            .field(
                "special_tokens_map_len",
                &self.tokenizer.special_tokens_map_file.len(),
            )
            .field(
                "tokenizer_config_len",
                &self.tokenizer.tokenizer_config_file.len(),
            )
            .finish()
    }
}

/// Progress events emitted while `ensure_model` runs.
///
/// Lifecycle on success:  `Start` → `Chunk` (1..N, throttled) → `Verify` → `Done`.
/// Lifecycle on error:    any prefix, then exactly one terminal `Error`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum DownloadProgress {
    Start { bytes_total: u64 },
    Chunk { bytes_downloaded: u64, bytes_total: u64 },
    Verify,
    Done,
    Error { message: String },
}

/// Errors returned from the download pipeline.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("insufficient disk space: {free_mb} MB free, {required_mb} MB required")]
    InsufficientDisk { free_mb: u64, required_mb: u64 },
    #[error("http error for {file}: {source}")]
    Http {
        file: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("sha-256 mismatch for {file}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        file: String,
        expected: String,
        actual: String,
    },
    #[error("retry budget exhausted for {file} after {attempts} attempts")]
    RetryExhausted { file: String, attempts: u32 },
    #[error("io error for {file}: {source}")]
    Io {
        file: String,
        #[source]
        source: std::io::Error,
    },
}

/// Production entrypoint: download the embedder model on first run.
///
/// Thin wrapper over [`ensure_model_with_oracle`] that hard-codes the
/// production [`Fs2Oracle`], [`RealBackoff`], and the HF base URL.
pub async fn ensure_model(
    base_dir: &Path,
    progress: Option<UnboundedSender<DownloadProgress>>,
) -> Result<ModelFiles, DownloadError> {
    let url = base_url();
    ensure_model_with_oracle(base_dir, &Fs2Oracle, &url, Some(&RealBackoff), progress).await
}

/// Test seam: same as [`ensure_model`] but with injectable disk oracle,
/// base URL, and backoff policy. Iterates over the production [`ALL_FILES`]
/// manifest. Most tests want to control the SHA constants too, in which case
/// they call [`ensure_model_with_oracle_spec`] directly.
pub(crate) async fn ensure_model_with_oracle(
    base_dir: &Path,
    oracle: &dyn DiskOracle,
    base_url_str: &str,
    backoff: Option<&dyn BackoffPolicy>,
    progress: Option<UnboundedSender<DownloadProgress>>,
) -> Result<ModelFiles, DownloadError> {
    ensure_model_with_oracle_spec(
        base_dir,
        oracle,
        base_url_str,
        backoff,
        progress,
        &ALL_FILES,
    )
    .await
}

/// Lowest-level test seam: identical to [`ensure_model_with_oracle`] but lets
/// the caller pass an arbitrary `files` slice instead of using [`ALL_FILES`].
/// Tests use this to install fixture bytes + matching SHAs that are tiny
/// (a few bytes) rather than the real 470 MB ONNX.
pub(crate) async fn ensure_model_with_oracle_spec(
    base_dir: &Path,
    oracle: &dyn DiskOracle,
    base_url_str: &str,
    backoff: Option<&dyn BackoffPolicy>,
    progress: Option<UnboundedSender<DownloadProgress>>,
    files: &[FileSpec],
) -> Result<ModelFiles, DownloadError> {
    // Run the pipeline and translate any error into a terminal `Error` event.
    // The match-on-Result indirection keeps the happy path readable while
    // guaranteeing exactly one error event on every failure — the UI treats
    // `Error` as terminal, so a second one would resurrect a finished download.
    match ensure_model_with_oracle_spec_inner(
        base_dir,
        oracle,
        base_url_str,
        backoff,
        progress.as_ref(),
        files,
    )
    .await
    {
        Ok(model_files) => Ok(model_files),
        Err(e) => {
            if let Some(tx) = progress.as_ref() {
                let _ = tx.send(DownloadProgress::Error {
                    message: e.to_string(),
                });
            }
            Err(e)
        }
    }
}

async fn ensure_model_with_oracle_spec_inner(
    base_dir: &Path,
    oracle: &dyn DiskOracle,
    base_url_str: &str,
    backoff: Option<&dyn BackoffPolicy>,
    progress: Option<&UnboundedSender<DownloadProgress>>,
    files: &[FileSpec],
) -> Result<ModelFiles, DownloadError> {
    let cache_dir = base_dir.join("models").join("multilingual-e5-small");
    std::fs::create_dir_all(&cache_dir).map_err(|e| DownloadError::Io {
        file: cache_dir.display().to_string(),
        source: e,
    })?;

    // (1) Cache-hit fast path: NO events emitted. Every startup after the first
    // takes this branch, and emitting progress here would flash a download UI
    // for work that never happens — ConfigPanel stays idle until a real
    // download fires.
    if let Some(model_files) = try_load_cached(&cache_dir, files) {
        return Ok(model_files);
    }

    // (2) Disk pre-flight: fail before the first byte rather than half a
    // gigabyte in, when the user can still act on the message.
    let free = oracle.available_bytes(&cache_dir).map_err(|e| DownloadError::Io {
        file: cache_dir.display().to_string(),
        source: e,
    })?;
    if free < MIN_FREE_BYTES {
        return Err(DownloadError::InsufficientDisk {
            free_mb: free / (1024 * 1024),
            required_mb: MIN_FREE_BYTES / (1024 * 1024),
        });
    }

    // (3) Emit `Start`. bytes_total is unknown at this point (no HEAD probes —
    // adds a round trip without observable UX gain because the values arrive
    // on the first `Chunk` anyway). Senders may render an indeterminate bar
    // until the first Chunk arrives.
    if let Some(tx) = progress {
        let _ = tx.send(DownloadProgress::Start { bytes_total: 0 });
    }

    // (4) Download every missing/mismatched file with full retry semantics
    // and chunk-level progress emission.
    let bp: &dyn BackoffPolicy = backoff.unwrap_or(&RealBackoff);
    let mut bytes_downloaded_running: u64 = 0;
    for spec in files {
        let target = cache_dir.join(spec.path);
        if file_matches_sha(&target, spec.sha256) {
            continue;
        }
        download_one(
            &cache_dir,
            spec,
            base_url_str,
            bp,
            progress,
            &mut bytes_downloaded_running,
        )
        .await?;
    }

    // (5) Re-read every committed file and assemble ModelFiles.
    //
    // Emit `Verify` here, but DO NOT emit `Done`.
    // The semantic `Done` event means "the embedder is ready to use" and that
    // is only true after the bootstrap layer (lib.rs::start_embedder_bootstrap)
    // has called `FastEmbedLocal::from_bytes` + filled the OnceCell. Emitting
    // `Done` here as well would deliver TWO `done` events to the UI on the
    // happy path. On disk the model
    // is already valid by the time we return Ok — the `ModelFiles` return value
    // is the contract, the wire event is not.
    if let Some(tx) = progress {
        let _ = tx.send(DownloadProgress::Verify);
    }
    let model_files = load_cached_or_err(&cache_dir, files)?;
    Ok(model_files)
}

/// Cache fast-path: returns `Some(ModelFiles)` iff every file in `files`
/// exists on disk AND its SHA-256 matches `spec.sha256`. Otherwise `None`.
fn try_load_cached(cache_dir: &Path, files: &[FileSpec]) -> Option<ModelFiles> {
    let mut onnx: Option<Vec<u8>> = None;
    let mut tokenizer = TokenizerFiles {
        tokenizer_file: Vec::new(),
        config_file: Vec::new(),
        special_tokens_map_file: Vec::new(),
        tokenizer_config_file: Vec::new(),
    };
    for spec in files {
        let target = cache_dir.join(spec.path);
        if !target.exists() {
            return None;
        }
        let bytes = std::fs::read(&target).ok()?;
        if sha256_hex(&bytes) != spec.sha256 {
            return None;
        }
        place_bytes(&mut onnx, &mut tokenizer, spec.kind, bytes);
    }
    Some(ModelFiles {
        onnx: onnx.unwrap_or_default(),
        tokenizer,
    })
}

/// Cache-presence probe that never touches the network: true iff every file
/// in the manifest is already on disk with a matching SHA-256. Every other
/// entry point into this module downloads on a cache miss, so a caller that
/// only wants to know "is the model ready" without risking a multi-hundred-
/// megabyte download as a side effect delegates to the same on-disk check
/// [`ensure_model`] uses internally rather than a separate reimplementation
/// that could drift from it.
#[cfg(feature = "eval")]
pub(crate) fn model_is_cached(base_dir: &Path) -> bool {
    let cache_dir = base_dir.join("models").join("multilingual-e5-small");
    try_load_cached(&cache_dir, &ALL_FILES).is_some()
}

/// Identical to [`try_load_cached`] but turns a missing file or SHA mismatch
/// into an `Err` (used post-download, when every file MUST be present).
fn load_cached_or_err(
    cache_dir: &Path,
    files: &[FileSpec],
) -> Result<ModelFiles, DownloadError> {
    let mut onnx: Option<Vec<u8>> = None;
    let mut tokenizer = TokenizerFiles {
        tokenizer_file: Vec::new(),
        config_file: Vec::new(),
        special_tokens_map_file: Vec::new(),
        tokenizer_config_file: Vec::new(),
    };
    for spec in files {
        let target = cache_dir.join(spec.path);
        let bytes = std::fs::read(&target).map_err(|e| DownloadError::Io {
            file: spec.path.into(),
            source: e,
        })?;
        let got = sha256_hex(&bytes);
        if got != spec.sha256 {
            return Err(DownloadError::ChecksumMismatch {
                file: spec.path.into(),
                expected: spec.sha256.into(),
                actual: got,
            });
        }
        place_bytes(&mut onnx, &mut tokenizer, spec.kind, bytes);
    }
    Ok(ModelFiles {
        onnx: onnx.unwrap_or_default(),
        tokenizer,
    })
}

fn place_bytes(
    onnx: &mut Option<Vec<u8>>,
    tokenizer: &mut TokenizerFiles,
    kind: FileKind,
    bytes: Vec<u8>,
) {
    match kind {
        FileKind::Onnx => *onnx = Some(bytes),
        FileKind::TokenizerJson => tokenizer.tokenizer_file = bytes,
        FileKind::ConfigJson => tokenizer.config_file = bytes,
        FileKind::SpecialTokensMap => tokenizer.special_tokens_map_file = bytes,
        FileKind::TokenizerConfig => tokenizer.tokenizer_config_file = bytes,
    }
}

/// Compute SHA-256 hex of file contents; `false` if file missing/unreadable.
fn file_matches_sha(path: &Path, expected_sha: &str) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    sha256_hex(&bytes) == expected_sha
}

/// Hex-lowercase SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
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

/// Compute the partial-file path for an in-progress download.
fn partial_path_for(target: &Path) -> std::path::PathBuf {
    let mut name = target
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".partial");
    target.with_file_name(name)
}

/// Throttle Chunk events to one per `CHUNK_PROGRESS_BYTES`.
const CHUNK_PROGRESS_BYTES: u64 = 64 * 1024;

/// Per-file download with retry (3 attempts, 5s/30s/120s backoff). A SHA-256
/// mismatch returns IMMEDIATELY and does NOT consume the retry budget: the
/// bytes arrived intact but wrong, which means tampering or persistent
/// corruption upstream, and retrying would just re-fetch the same wrong bytes.
/// All other errors (HTTP / network / IO) consume the retry budget and clean
/// up the `.partial` file on each attempt.
///
/// Emits `DownloadProgress::Chunk` events throttled to one per
/// `CHUNK_PROGRESS_BYTES` (64 KB) of cumulative download progress across all
/// files in the current run.
async fn download_one(
    cache_dir: &Path,
    spec: &FileSpec,
    base_url_str: &str,
    backoff: &dyn BackoffPolicy,
    progress: Option<&UnboundedSender<DownloadProgress>>,
    bytes_downloaded_running: &mut u64,
) -> Result<(), DownloadError> {
    use futures::StreamExt;
    use std::io::Write;

    let target = cache_dir.join(spec.path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| DownloadError::Io {
            file: spec.path.into(),
            source: e,
        })?;
    }
    let partial = partial_path_for(&target);

    const ATTEMPTS: u32 = 3;

    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            // Backoff before retry; ZeroBackoff makes this instant in tests.
            backoff.sleep_for_attempt(attempt - 1).await;
        }

        // Always start from a clean .partial — no resume in v1.
        let _ = std::fs::remove_file(&partial);
        let mut file = match std::fs::File::create(&partial) {
            Ok(f) => f,
            Err(e) => {
                return Err(DownloadError::Io {
                    file: spec.path.into(),
                    source: e,
                });
            }
        };

        let url = format!("{base_url_str}/{}", spec.path);
        let resp = match reqwest::get(&url).await {
            Ok(r) => r,
            Err(_) => {
                let _ = std::fs::remove_file(&partial);
                continue;
            }
        };
        if !resp.status().is_success() {
            let _ = std::fs::remove_file(&partial);
            continue;
        }

        // Track per-file emission threshold — one Chunk per 64 KB of running
        // total (not per file), so a multi-file run yields a smooth progress
        // ramp without spamming the channel.
        let content_length = resp.content_length().unwrap_or(0);
        let bytes_total_hint = *bytes_downloaded_running + content_length;
        let mut next_emit_threshold = *bytes_downloaded_running + CHUNK_PROGRESS_BYTES;

        let mut stream = resp.bytes_stream();
        let mut io_err: Option<DownloadError> = None;
        let mut had_http_err = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    if let Err(e) = file.write_all(&bytes) {
                        io_err = Some(DownloadError::Io {
                            file: spec.path.into(),
                            source: e,
                        });
                        break;
                    }
                    *bytes_downloaded_running += bytes.len() as u64;
                    if let Some(tx) = progress {
                        if *bytes_downloaded_running >= next_emit_threshold {
                            let _ = tx.send(DownloadProgress::Chunk {
                                bytes_downloaded: *bytes_downloaded_running,
                                bytes_total: bytes_total_hint,
                            });
                            next_emit_threshold =
                                *bytes_downloaded_running + CHUNK_PROGRESS_BYTES;
                        }
                    }
                }
                Err(_) => {
                    had_http_err = true;
                    break;
                }
            }
        }

        if let Some(e) = io_err {
            // IO errors are not transient — bail out without consuming budget.
            let _ = std::fs::remove_file(&partial);
            return Err(e);
        }
        if had_http_err {
            let _ = std::fs::remove_file(&partial);
            continue;
        }

        if let Err(e) = file.flush() {
            let _ = std::fs::remove_file(&partial);
            return Err(DownloadError::Io {
                file: spec.path.into(),
                source: e,
            });
        }
        if let Err(e) = file.sync_all() {
            let _ = std::fs::remove_file(&partial);
            return Err(DownloadError::Io {
                file: spec.path.into(),
                source: e,
            });
        }
        drop(file);

        // Emit one final Chunk so consumers see exact byte count post-write.
        if let Some(tx) = progress {
            let _ = tx.send(DownloadProgress::Chunk {
                bytes_downloaded: *bytes_downloaded_running,
                bytes_total: bytes_total_hint,
            });
        }

        // SHA verification.
        let bytes = match std::fs::read(&partial) {
            Ok(b) => b,
            Err(e) => {
                let _ = std::fs::remove_file(&partial);
                return Err(DownloadError::Io {
                    file: spec.path.into(),
                    source: e,
                });
            }
        };
        let got = sha256_hex(&bytes);
        if got != spec.sha256 {
            // Return immediately, do NOT retry: a checksum mismatch is not a
            // transient fault, so the retry budget would buy nothing.
            let _ = std::fs::remove_file(&partial);
            return Err(DownloadError::ChecksumMismatch {
                file: spec.path.into(),
                expected: spec.sha256.into(),
                actual: got,
            });
        }

        // Atomic rename → final.
        if let Err(e) = commit_file(&partial, &target) {
            let _ = std::fs::remove_file(&partial);
            return Err(DownloadError::Io {
                file: spec.path.into(),
                source: e,
            });
        }
        return Ok(());
    }

    Err(DownloadError::RetryExhausted {
        file: spec.path.into(),
        attempts: ATTEMPTS,
    })
}

/// Atomic rename from a `.partial` working file to the final destination.
fn commit_file(partial: &Path, final_path: &Path) -> std::io::Result<()> {
    std::fs::rename(partial, final_path)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    // Project convention (see state.rs): use std::env::temp_dir() + unique
    // subdir per test rather than the `tempfile` crate.
    struct TestDir {
        path: std::path::PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            // Mix PID + nanos + a thread-local counter for collision-free dirs
            // even when tests run in parallel.
            use std::sync::atomic::{AtomicU64, Ordering};
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let pid = std::process::id();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let n = SEQ.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "claw-garden-{label}-{pid}-{nanos}-{n}"
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// Test double for [`DiskOracle`] that returns a configurable free-bytes value.
    pub(crate) struct MockDiskOracle {
        free_bytes: AtomicU64,
    }

    impl MockDiskOracle {
        pub(crate) fn returning(b: u64) -> Self {
            Self {
                free_bytes: AtomicU64::new(b),
            }
        }
    }

    impl DiskOracle for MockDiskOracle {
        fn available_bytes(&self, _path: &Path) -> std::io::Result<u64> {
            Ok(self.free_bytes.load(Ordering::SeqCst))
        }
    }

    // ---------- Disk pre-flight ----------

    #[tokio::test]
    async fn insufficient_disk_returns_err_no_http() {
        let tmp = TestDir::new("preflight-insufficient");
        // Mockito server present BUT asserts zero hits — pre-flight must short-circuit.
        let mut server = mockito::Server::new_async().await;
        let mock = server.mock("GET", mockito::Matcher::Any).expect(0).create_async().await;

        let oracle = MockDiskOracle::returning(599 * 1024 * 1024);
        let err = ensure_model_with_oracle(tmp.path(), &oracle, &server.url(), None, None)
            .await
            .unwrap_err();

        match err {
            DownloadError::InsufficientDisk { free_mb, required_mb } => {
                assert_eq!(free_mb, 599);
                assert_eq!(required_mb, 600);
            }
            other => panic!("expected InsufficientDisk, got {other:?}"),
        }
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn sufficient_disk_passes_preflight() {
        let tmp = TestDir::new("preflight-sufficient");
        let oracle = MockDiskOracle::returning(600 * 1024 * 1024);

        // Sufficient disk → preflight passes → download path runs. Production
        // ALL_FILES has placeholder SHAs that fixture bytes won't match, but
        // the mockito server immediately returns 503 so we exhaust the retry
        // budget without DNS-resolving real HF. Use ZeroBackoff to keep the
        // test fast. The assertion: NOT InsufficientDisk (any other error
        // shape proves preflight succeeded).
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(503)
            .create_async()
            .await;

        let err = ensure_model_with_oracle(
            tmp.path(),
            &oracle,
            &server.url(),
            Some(&ZeroBackoff),
            None,
        )
        .await
        .unwrap_err();

        assert!(
            !matches!(err, DownloadError::InsufficientDisk { .. }),
            "preflight should have passed, but got InsufficientDisk: {err:?}"
        );
    }

    // ---------- SHA constants + cached fast path ----------

    /// Build a 5-FileSpec slice from in-memory fixture bytes whose SHAs are
    /// computed on the fly. Returns the fixture file list directly.
    fn fixture_files() -> Vec<FixtureFile> {
        vec![
            FixtureFile {
                path: "onnx/model.onnx",
                bytes: b"fake-onnx-model-bytes-T040".to_vec(),
                kind: FileKind::Onnx,
            },
            FixtureFile {
                path: "tokenizer.json",
                bytes: br#"{"version":"1.0","fake":true}"#.to_vec(),
                kind: FileKind::TokenizerJson,
            },
            FixtureFile {
                path: "config.json",
                bytes: br#"{"model_type":"bert","hidden_size":384}"#.to_vec(),
                kind: FileKind::ConfigJson,
            },
            FixtureFile {
                path: "special_tokens_map.json",
                bytes: br#"{"cls_token":"[CLS]","sep_token":"[SEP]"}"#.to_vec(),
                kind: FileKind::SpecialTokensMap,
            },
            FixtureFile {
                path: "tokenizer_config.json",
                bytes: br#"{"do_lower_case":true,"model_max_length":512}"#.to_vec(),
                kind: FileKind::TokenizerConfig,
            },
        ]
    }

    struct FixtureFile {
        path: &'static str,
        bytes: Vec<u8>,
        kind: FileKind,
    }

    fn fixture_specs(files: &[FixtureFile]) -> Vec<FileSpec> {
        files
            .iter()
            .map(|f| FileSpec {
                path: f.path,
                sha256: Box::leak(super::sha256_hex(&f.bytes).into_boxed_str()),
                kind: f.kind,
            })
            .collect()
    }

    fn write_all_to_cache(cache_dir: &Path, files: &[FixtureFile]) {
        for f in files {
            let target = cache_dir.join(f.path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&target, &f.bytes).unwrap();
        }
    }

    #[tokio::test]
    async fn skips_download_when_all_files_cached() {
        let tmp = TestDir::new("cached-skip-full");
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        std::fs::create_dir_all(&cache_dir).unwrap();
        let files = fixture_files();
        write_all_to_cache(&cache_dir, &files);
        let specs = fixture_specs(&files);

        let mut server = mockito::Server::new_async().await;
        // Zero hits expected on any path.
        let mock = server
            .mock("GET", mockito::Matcher::Any)
            .expect(0)
            .create_async()
            .await;

        let oracle = MockDiskOracle::returning(10 * 1024 * 1024); // Way under threshold.
        let res = ensure_model_with_oracle_spec(
            tmp.path(),
            &oracle,
            &server.url(),
            None,
            None,
            &specs,
        )
        .await;
        assert!(res.is_ok(), "expected cache hit, got {:?}", res.err());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn corrupt_cached_file_triggers_redownload() {
        let tmp = TestDir::new("cached-corrupt");
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        std::fs::create_dir_all(&cache_dir).unwrap();
        let files = fixture_files();
        write_all_to_cache(&cache_dir, &files);

        // Tamper byte 0 of onnx/model.onnx so its SHA no longer matches.
        let onnx_path = cache_dir.join("onnx/model.onnx");
        let mut bytes = std::fs::read(&onnx_path).unwrap();
        bytes[0] = bytes[0].wrapping_add(1);
        std::fs::write(&onnx_path, &bytes).unwrap();

        let specs = fixture_specs(&files);

        let mut server = mockito::Server::new_async().await;
        // Mockito should serve the valid onnx bytes for redownload.
        let onnx_fixture = files
            .iter()
            .find(|f| f.path == "onnx/model.onnx")
            .unwrap()
            .bytes
            .clone();
        let m_onnx = server
            .mock("GET", "/onnx/model.onnx")
            .with_status(200)
            .with_body(onnx_fixture)
            .expect(1)
            .create_async()
            .await;
        // The other 4 routes should receive ZERO hits.
        let m_other = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    "^/(tokenizer\\.json|config\\.json|special_tokens_map\\.json|tokenizer_config\\.json)$"
                        .into(),
                ),
            )
            .expect(0)
            .create_async()
            .await;

        // Sufficient disk so preflight passes for the redownload path.
        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let res = ensure_model_with_oracle_spec(
            tmp.path(),
            &oracle,
            &server.url(),
            Some(&ZeroBackoff),
            None,
            &specs,
        )
        .await;
        assert!(
            res.is_ok(),
            "corrupt cache should trigger redownload + recovery, got {:?}",
            res.err()
        );
        m_onnx.assert_async().await;
        m_other.assert_async().await;
    }

    // ---------- Download + retry ----------

    /// Mounts mockito routes for all 5 fixture files with the given response
    /// (status + body). Returns the mockito mocks so tests can assert hit counts.
    async fn mount_all(
        server: &mut mockito::ServerGuard,
        files: &[FixtureFile],
        status: usize,
    ) -> Vec<mockito::Mock> {
        let mut mocks = Vec::with_capacity(files.len());
        for f in files {
            let m = server
                .mock("GET", format!("/{}", f.path).as_str())
                .with_status(status)
                .with_body(f.bytes.clone())
                .create_async()
                .await;
            mocks.push(m);
        }
        mocks
    }

    /// Walk a cache dir recursively and collect any `.partial` files.
    fn collect_partials(root: &Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        walk_partials(root, &mut out);
        out
    }

    fn walk_partials(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk_partials(&p, out);
            } else if p
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains(".partial"))
                .unwrap_or(false)
            {
                out.push(p);
            }
        }
    }

    #[tokio::test]
    async fn successful_download_leaves_no_partial_files() {
        let tmp = TestDir::new("dl-success");
        let files = fixture_files();
        let specs = fixture_specs(&files);

        let mut server = mockito::Server::new_async().await;
        let _mocks = mount_all(&mut server, &files, 200).await;

        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let res = ensure_model_with_oracle_spec(
            tmp.path(),
            &oracle,
            &server.url(),
            Some(&ZeroBackoff),
            None,
            &specs,
        )
        .await;
        assert!(res.is_ok(), "got {:?}", res.err());
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        let partials = collect_partials(&cache_dir);
        assert!(
            partials.is_empty(),
            "expected zero .partial files, found {partials:?}"
        );
    }

    #[tokio::test]
    async fn network_failure_returns_err_no_partial_files() {
        let tmp = TestDir::new("dl-net-fail");
        let files = fixture_files();
        let specs = fixture_specs(&files);

        let mut server = mockito::Server::new_async().await;
        // Always 503 — every attempt fails for every file.
        let _mocks = mount_all(&mut server, &files, 503).await;

        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let res = ensure_model_with_oracle_spec(
            tmp.path(),
            &oracle,
            &server.url(),
            Some(&ZeroBackoff),
            None,
            &specs,
        )
        .await;
        match res {
            Err(DownloadError::RetryExhausted { attempts, .. }) => assert_eq!(attempts, 3),
            other => panic!("expected RetryExhausted, got {other:?}"),
        }
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        let partials = collect_partials(&cache_dir);
        assert!(
            partials.is_empty(),
            "expected zero .partial files, found {partials:?}"
        );
    }

    #[tokio::test]
    async fn retry_succeeds_on_second_attempt() {
        let tmp = TestDir::new("dl-retry-2nd");
        let files = fixture_files();
        let specs = fixture_specs(&files);

        let mut server = mockito::Server::new_async().await;
        // For onnx/model.onnx: serve 500 once, then 200 with valid body.
        let onnx_fixture = files
            .iter()
            .find(|f| f.path == "onnx/model.onnx")
            .unwrap()
            .bytes
            .clone();
        let m_fail = server
            .mock("GET", "/onnx/model.onnx")
            .with_status(500)
            .expect(1)
            .create_async()
            .await;
        let m_ok = server
            .mock("GET", "/onnx/model.onnx")
            .with_status(200)
            .with_body(onnx_fixture)
            .expect(1)
            .create_async()
            .await;
        // The other 4 files succeed on the first hit.
        for f in &files {
            if f.path == "onnx/model.onnx" {
                continue;
            }
            server
                .mock("GET", format!("/{}", f.path).as_str())
                .with_status(200)
                .with_body(f.bytes.clone())
                .create_async()
                .await;
        }

        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let res = ensure_model_with_oracle_spec(
            tmp.path(),
            &oracle,
            &server.url(),
            Some(&ZeroBackoff),
            None,
            &specs,
        )
        .await;
        assert!(res.is_ok(), "got {:?}", res.err());
        m_fail.assert_async().await;
        m_ok.assert_async().await;
    }

    #[tokio::test]
    async fn three_failures_returns_network_err_and_stops() {
        let tmp = TestDir::new("dl-3fail");
        let files = fixture_files();
        // Only need the FIRST file in the manifest to fail to exercise the budget.
        let specs = fixture_specs(&files);

        let mut server = mockito::Server::new_async().await;
        let m_onnx = server
            .mock("GET", "/onnx/model.onnx")
            .with_status(500)
            .expect(3)
            .create_async()
            .await;
        // Other routes — mockito will reject unstubbed routes anyway, but be explicit.
        for f in &files {
            if f.path == "onnx/model.onnx" {
                continue;
            }
            server
                .mock("GET", format!("/{}", f.path).as_str())
                .with_status(200)
                .with_body(f.bytes.clone())
                .expect_at_most(0) // Should never reach these — onnx fails first.
                .create_async()
                .await;
        }

        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let res = ensure_model_with_oracle_spec(
            tmp.path(),
            &oracle,
            &server.url(),
            Some(&ZeroBackoff),
            None,
            &specs,
        )
        .await;
        match res {
            Err(DownloadError::RetryExhausted { attempts, file }) => {
                assert_eq!(attempts, 3);
                assert_eq!(file, "onnx/model.onnx");
            }
            other => panic!("expected RetryExhausted on onnx/model.onnx, got {other:?}"),
        }
        m_onnx.assert_async().await;
    }

    #[tokio::test]
    async fn sha_mismatch_after_retries_returns_checksum_err() {
        // A SHA mismatch must return IMMEDIATELY without consuming the retry
        // budget. Expect exactly 1 hit + ChecksumMismatch + no .partial files
        // left behind.
        let tmp = TestDir::new("dl-sha-mismatch");
        let files = fixture_files();
        let specs = fixture_specs(&files);

        let mut server = mockito::Server::new_async().await;
        let m_onnx = server
            .mock("GET", "/onnx/model.onnx")
            .with_status(200)
            .with_body(b"WRONG-BYTES".to_vec()) // SHA won't match the fixture's.
            .expect(1)
            .create_async()
            .await;
        for f in &files {
            if f.path == "onnx/model.onnx" {
                continue;
            }
            server
                .mock("GET", format!("/{}", f.path).as_str())
                .with_status(200)
                .with_body(f.bytes.clone())
                .expect_at_most(0)
                .create_async()
                .await;
        }

        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let res = ensure_model_with_oracle_spec(
            tmp.path(),
            &oracle,
            &server.url(),
            Some(&ZeroBackoff),
            None,
            &specs,
        )
        .await;
        match res {
            Err(DownloadError::ChecksumMismatch { file, .. }) => {
                assert_eq!(file, "onnx/model.onnx");
            }
            other => panic!("expected ChecksumMismatch, got {other:?}"),
        }
        m_onnx.assert_async().await;
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        let partials = collect_partials(&cache_dir);
        assert!(
            partials.is_empty(),
            "expected zero .partial files, found {partials:?}"
        );
    }

    #[tokio::test]
    async fn downloads_only_missing_files() {
        let tmp = TestDir::new("dl-missing-only");
        let files = fixture_files();
        let specs = fixture_specs(&files);
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        std::fs::create_dir_all(&cache_dir).unwrap();

        // Pre-write 4 of 5 files. The "config.json" is the missing one.
        let missing_path = "config.json";
        for f in &files {
            if f.path == missing_path {
                continue;
            }
            let target = cache_dir.join(f.path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&target, &f.bytes).unwrap();
        }

        let mut server = mockito::Server::new_async().await;
        // Only the missing file should be hit (once).
        let missing_bytes = files
            .iter()
            .find(|f| f.path == missing_path)
            .unwrap()
            .bytes
            .clone();
        let m_missing = server
            .mock("GET", format!("/{missing_path}").as_str())
            .with_status(200)
            .with_body(missing_bytes)
            .expect(1)
            .create_async()
            .await;
        // The other 4 files MUST NOT be re-requested.
        let mut other_mocks = Vec::new();
        for f in &files {
            if f.path == missing_path {
                continue;
            }
            let m = server
                .mock("GET", format!("/{}", f.path).as_str())
                .with_status(200)
                .with_body(f.bytes.clone())
                .expect(0)
                .create_async()
                .await;
            other_mocks.push(m);
        }

        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let res = ensure_model_with_oracle_spec(
            tmp.path(),
            &oracle,
            &server.url(),
            Some(&ZeroBackoff),
            None,
            &specs,
        )
        .await;
        assert!(res.is_ok(), "got {:?}", res.err());
        m_missing.assert_async().await;
        for m in &other_mocks {
            m.assert_async().await;
        }
    }

    // ---------- Progress events ----------

    fn drain_progress(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<DownloadProgress>,
    ) -> Vec<DownloadProgress> {
        let mut out = Vec::new();
        while let Ok(p) = rx.try_recv() {
            out.push(p);
        }
        out
    }

    fn phase_of(p: &DownloadProgress) -> &'static str {
        match p {
            DownloadProgress::Start { .. } => "start",
            DownloadProgress::Chunk { .. } => "chunk",
            DownloadProgress::Verify => "verify",
            DownloadProgress::Done => "done",
            DownloadProgress::Error { .. } => "error",
        }
    }

    #[tokio::test]
    async fn lifecycle_events_in_expected_order() {
        // `ensure_model` MUST emit `start → chunk(s)
        // → verify` and STOP there. The semantic terminal `done` event means
        // "embedder is ready to use" — that is the bootstrap's responsibility
        // to emit AFTER `FastEmbedLocal::from_bytes` + `OnceCell::set` succeed.
        // If `ensure_model` also emitted `Done`, the forwarder→app.emit path
        // in `lib.rs` would deliver TWO `done` events to the UI on the happy
        // path.
        let tmp = TestDir::new("evt-order");
        let files = fixture_files();
        let specs = fixture_specs(&files);

        let mut server = mockito::Server::new_async().await;
        let _mocks = mount_all(&mut server, &files, 200).await;

        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DownloadProgress>();
        let res = ensure_model_with_oracle_spec(
            tmp.path(),
            &oracle,
            &server.url(),
            Some(&ZeroBackoff),
            Some(tx),
            &specs,
        )
        .await;
        assert!(res.is_ok(), "got {:?}", res.err());

        let events = drain_progress(&mut rx);
        let phases: Vec<&'static str> = events.iter().map(phase_of).collect();
        // Expected sequence: Start, >=1 Chunk, Verify (terminal). NO Done.
        assert_eq!(phases.first().copied(), Some("start"), "first must be start");
        assert_eq!(
            phases.last().copied(),
            Some("verify"),
            "last must be verify; the terminal `done` is now emitted by the bootstrap, not ensure_model"
        );
        assert!(
            phases.iter().any(|p| *p == "chunk"),
            "expected at least one chunk event"
        );
        // ensure_model MUST NOT emit Done — that responsibility moved to the
        // bootstrap layer (lib.rs::start_embedder_bootstrap) so the UI receives
        // EXACTLY ONE terminal `done`, after the embedder is actually loadable.
        assert!(
            !phases.iter().any(|p| *p == "done"),
            "ensure_model must NOT emit Done; got {phases:?}"
        );
        // No Error event on the success path.
        assert!(
            !phases.iter().any(|p| *p == "error"),
            "no error events on success path; got {phases:?}"
        );
    }

    #[tokio::test]
    async fn error_path_emits_exactly_one_terminal_error_event() {
        let tmp = TestDir::new("evt-error");
        let files = fixture_files();
        let specs = fixture_specs(&files);

        let mut server = mockito::Server::new_async().await;
        // Every attempt fails — force RetryExhausted.
        let _mocks = mount_all(&mut server, &files, 503).await;

        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DownloadProgress>();
        let _ = ensure_model_with_oracle_spec(
            tmp.path(),
            &oracle,
            &server.url(),
            Some(&ZeroBackoff),
            Some(tx),
            &specs,
        )
        .await;

        let events = drain_progress(&mut rx);
        let error_count = events
            .iter()
            .filter(|p| matches!(p, DownloadProgress::Error { .. }))
            .count();
        assert_eq!(
            error_count, 1,
            "expected exactly one terminal error event; got {:?}",
            events.iter().map(phase_of).collect::<Vec<_>>()
        );
        let last_idx = events.len() - 1;
        assert!(
            matches!(events[last_idx], DownloadProgress::Error { .. }),
            "error must be the LAST event; got phases {:?}",
            events.iter().map(phase_of).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn downloading_events_rate_limited_by_64kb() {
        // Serve a 256 KB body for onnx/model.onnx; cap fixture so chunk-rate
        // throttling is observable. One Chunk per
        // 64 KB → ceil(256 / 64) = 4 chunks, plus a final flush event tolerance
        // of ±1.
        let tmp = TestDir::new("evt-throttle");
        let mut files = fixture_files();
        let big_body = vec![0u8; 256 * 1024]; // 256 KB
        files[0].bytes = big_body.clone();
        let specs = fixture_specs(&files);

        let mut server = mockito::Server::new_async().await;
        let _mocks = mount_all(&mut server, &files, 200).await;

        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DownloadProgress>();
        let res = ensure_model_with_oracle_spec(
            tmp.path(),
            &oracle,
            &server.url(),
            Some(&ZeroBackoff),
            Some(tx),
            &specs,
        )
        .await;
        assert!(res.is_ok(), "got {:?}", res.err());
        let events = drain_progress(&mut rx);
        let chunk_count = events
            .iter()
            .filter(|p| matches!(p, DownloadProgress::Chunk { .. }))
            .count();
        // 256 KB / 64 KB = 4 base emissions; +1 final-flush event per file
        // can stack across the 4 other small files for up to ~5 extras. The
        // hard rate-limit guarantee is "at most one event per 64 KB of body
        // bytes received", which for a 256 KB onnx yields <= 8 (chunked
        // delivery may straddle the 64 KB boundary by one). Lower bound 1
        // proves emission is happening at all.
        assert!(
            chunk_count <= 10,
            "expected <= 10 chunk events for 256 KB body, got {chunk_count}"
        );
        assert!(
            chunk_count >= 1,
            "expected >= 1 chunk event for 256 KB body, got {chunk_count}"
        );
    }

    // ---------- Backoff policy ----------

    #[tokio::test]
    async fn zero_backoff_sleeps_for_zero_duration() {
        let backoff = ZeroBackoff;
        let started = std::time::Instant::now();
        backoff.sleep_for_attempt(1).await;
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "ZeroBackoff should be near-instant, elapsed {elapsed:?}"
        );
    }

    #[test]
    fn real_backoff_uses_correct_secs_table() {
        let b = RealBackoff;
        assert_eq!(b.secs_for_attempt(0), 5);
        assert_eq!(b.secs_for_attempt(1), 30);
        assert_eq!(b.secs_for_attempt(2), 120);
    }

    #[tokio::test]
    async fn cached_path_skips_disk_check() {
        let tmp = TestDir::new("cached-skip");
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        std::fs::create_dir_all(&cache_dir).unwrap();
        let files = fixture_files();
        write_all_to_cache(&cache_dir, &files);
        let specs = fixture_specs(&files);

        // Way under the 600 MB threshold — would fail the preflight if reached.
        let oracle = MockDiskOracle::returning(10 * 1024 * 1024);
        let res = ensure_model_with_oracle_spec(
            tmp.path(),
            &oracle,
            "http://unused.invalid",
            None,
            None,
            &specs,
        )
        .await;

        assert!(
            res.is_ok(),
            "cache hit must skip disk check, but got {:?}",
            res.err()
        );
    }

    // ---------- tokenizer_path helper ----------

    #[test]
    fn tokenizer_path_returns_correct_subpath() {
        use std::path::PathBuf;
        let result = super::tokenizer_path(std::path::Path::new("/base"));
        assert_eq!(
            result,
            PathBuf::from("/base/models/multilingual-e5-small/tokenizer.json"),
            "tokenizer_path must resolve to the expected subpath"
        );
    }

    // ---------- load_tokenizer: shared tokenizer lifecycle ----------

    #[test]
    fn load_tokenizer_returns_none_for_a_missing_file_without_panicking() {
        let tmp = TestDir::new("load-tokenizer-missing");
        assert!(load_tokenizer(tmp.path()).is_none());
    }

    #[test]
    fn load_tokenizer_returns_none_for_a_malformed_file_without_panicking() {
        let tmp = TestDir::new("load-tokenizer-malformed");
        let path = tokenizer_path(tmp.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not a valid tokenizer.json").unwrap();
        assert!(load_tokenizer(tmp.path()).is_none());
    }
}
