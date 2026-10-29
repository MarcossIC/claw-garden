//! First-run model download + SHA-256 verification.
//!
//! # Origin
//!
//! The daemon downloads from [`HF_BASE_URL`] unless `CLAWG_HF_BASE_URL`
//! ([`BASE_URL_VAR`]) names a mirror: an `https://` URL, or an `http://` one on
//! the machine itself, for a closed network or an install check. The pinned
//! SHA-256 of every file is checked whatever the origin. Tests never use the
//! variable (it is global to the process): the [`ModelSource`] a test builds
//! carries its own local server.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::OwnedRwLockReadGuard;

/// The four tokenizer-side files of the model, as the bytes read from disk.
pub struct TokenizerFiles {
    pub tokenizer_file: Vec<u8>,
    pub config_file: Vec<u8>,
    pub special_tokens_map_file: Vec<u8>,
    pub tokenizer_config_file: Vec<u8>,
}

/// Hugging Face, where the model is downloaded from unless [`BASE_URL_VAR`]
/// names a mirror.
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
pub fn load_tokenizer(base_dir: &Path) -> Option<Arc<tokenizers::Tokenizer>> {
    let path = tokenizer_path(base_dir);
    match tokenizers::Tokenizer::from_file(&path) {
        Ok(tok) => Some(Arc::new(tok)),
        Err(e) => {
            log::warn!("load_tokenizer: not loadable at {} ({e}); returning None", path.display());
            None
        }
    }
}

/// Loads `tokenizer.json` only when its bytes are the pinned ones: the file is
/// read, its SHA-256 compared with [`TOKENIZER_JSON_SHA256`], and only then
/// parsed. A missing, altered or unparsable file gives `None` and a warning;
/// the caller degrades to byte chunking.
pub(crate) fn load_verified_tokenizer(base_dir: &Path) -> Option<Arc<tokenizers::Tokenizer>> {
    load_verified_tokenizer_with(base_dir, |bytes| tokenizers::Tokenizer::from_bytes(bytes).ok())
}

/// Parses the bytes of a `tokenizer.json` already checked against
/// [`TOKENIZER_JSON_SHA256`]. The one place the model load parses it, and the
/// line it logs is what counts the parses of a process.
pub(crate) fn parse_verified_tokenizer(bytes: &[u8]) -> Result<tokenizers::Tokenizer, String> {
    let started = std::time::Instant::now();
    let tokenizer = tokenizers::Tokenizer::from_bytes(bytes).map_err(|e| format!("tokenizer.json does not parse: {e}"))?;
    log::info!("tokenizer.json parsed in {} ms", started.elapsed().as_millis());
    Ok(tokenizer)
}

/// [`load_verified_tokenizer`] with the parser as a parameter, so a test can
/// count how many times a file reaches it.
pub(crate) fn load_verified_tokenizer_with<P>(base_dir: &Path, parse: P) -> Option<Arc<tokenizers::Tokenizer>>
where
    P: FnOnce(&[u8]) -> Option<tokenizers::Tokenizer>,
{
    let path = tokenizer_path(base_dir);
    let bytes = match read_verified(&path, TOKENIZER_JSON_SHA256) {
        Ok(bytes) => bytes,
        Err(Unusable::Unreadable(e)) => {
            log::warn!("tokenizer.json not readable at {} ({e}); chunking by bytes", path.display());
            return None;
        }
        Err(Unusable::Busy(e)) => {
            log::warn!("tokenizer.json at {} is held by another program ({e}); chunking by bytes", path.display());
            return None;
        }
        Err(Unusable::Altered) => {
            log::warn!("tokenizer.json at {} is not the pinned file; chunking by bytes", path.display());
            return None;
        }
    };
    let started = std::time::Instant::now();
    let Some(tokenizer) = parse(&bytes) else {
        log::warn!("tokenizer.json at {} does not parse; chunking by bytes", path.display());
        return None;
    };
    log::info!("tokenizer.json parsed in {} ms", started.elapsed().as_millis());
    Some(Arc::new(tokenizer))
}

/// Env var naming a mirror of [`HF_BASE_URL`].
pub(crate) const BASE_URL_VAR: &str = "CLAWG_HF_BASE_URL";

/// The origin [`BASE_URL_VAR`] asks for, without a trailing `/`: unset or
/// empty is [`HF_BASE_URL`]; otherwise an `https://` URL, or an `http://` one
/// on `localhost`, `127.0.0.1` or `[::1]`. Plain HTTP to another host would send
/// the request in the clear, and a typo must not fall back to Hugging Face
/// unseen, so anything else is an error naming the variable. The error never
/// repeats the value, which may carry credentials.
pub(crate) fn base_url_from(raw: Option<&str>) -> Result<String, String> {
    let text = match raw.map(str::trim) {
        None | Some("") => return Ok(HF_BASE_URL.to_string()),
        Some(text) => text,
    };
    const ACCEPTED: &str = "use an https:// URL, or an http:// one on localhost, 127.0.0.1 or [::1]";
    let url = reqwest::Url::parse(text).map_err(|_| format!("{BASE_URL_VAR} is not a URL; {ACCEPTED}"))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match url.scheme() {
        "https" if url.host_str().is_some() => Ok(text.trim_end_matches('/').to_string()),
        "http" if loopback => Ok(text.trim_end_matches('/').to_string()),
        scheme => Err(format!(
            "{BASE_URL_VAR} points at {scheme}://{}; {ACCEPTED}",
            url.host_str().unwrap_or("")
        )),
    }
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
    pub onnx: OnnxFile,
    pub tokenizer: TokenizerFiles,
}

/// What keeps a load and a repair of the model cache from meeting on
/// `model.onnx`: a load reads under it for as long as it holds the file, a
/// repair writes under it while it renames the new file into place. One per
/// model directory and process; clones share it.
#[derive(Clone, Default)]
pub struct CacheLock(Arc<tokio::sync::RwLock<()>>);

impl CacheLock {
    pub fn new() -> Self {
        Self::default()
    }

    async fn read(&self) -> OwnedRwLockReadGuard<()> {
        self.0.clone().read_owned().await
    }

    /// The read side without waiting, for a caller that is not async and
    /// holds no other lock.
    #[cfg(any(test, feature = "eval"))]
    pub(crate) fn read_now(&self) -> OwnedRwLockReadGuard<()> {
        self.0.clone().try_read_owned().expect("nothing writes while a check reads")
    }
}

/// `model.onnx` as it was hashed: a read-only view of the file, held open.
pub struct OnnxFile(View);

enum View {
    // Field order is drop order: unmap, close the handle, and only then let a
    // waiting repair rename over the file.
    Mapped {
        map: memmap2::Mmap,
        _file: std::fs::File,
        _cache: OwnedRwLockReadGuard<()>,
    },
    #[cfg(any(test, feature = "testing"))]
    InMemory(Vec<u8>),
}

impl OnnxFile {
    /// Bytes no file backs, for a stand-in model that never reads them.
    #[cfg(any(test, feature = "testing"))]
    pub fn in_memory(bytes: Vec<u8>) -> Self {
        OnnxFile(View::InMemory(bytes))
    }

    #[cfg(test)]
    pub(crate) fn is_view(&self) -> bool {
        matches!(self.0, View::Mapped { .. })
    }
}

impl std::ops::Deref for OnnxFile {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match &self.0 {
            View::Mapped { map, .. } => map,
            #[cfg(any(test, feature = "testing"))]
            View::InMemory(bytes) => bytes,
        }
    }
}

impl std::fmt::Debug for ModelFiles {
    // Sizes instead of the byte contents, so test failure output stays readable.
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
    #[error("could not build the HTTP client for the model download: {0}")]
    Client(#[source] reqwest::Error),
    #[error("{0}")]
    BaseUrl(String),
    #[error("the model files in the cache do not verify: {}", files.join(", "))]
    NotCached { files: Vec<String> },
    #[error("{file} of the model is held by another program: {source}")]
    Busy {
        file: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{file} is larger than {limit_mb} MB, more than any file of the model")]
    TooLarge { file: String, limit_mb: u64 },
    #[error("sha-256 mismatch for {file}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        file: String,
        expected: String,
        actual: String,
    },
    #[error("retry budget exhausted for {file} from {host} after {attempts} attempts: {cause}")]
    RetryExhausted { file: String, host: String, attempts: u32, cause: String },
    #[error("io error for {file}: {source}")]
    Io {
        file: String,
        #[source]
        source: std::io::Error,
    },
}

/// Where the model comes from and how it is fetched: the origin, the disk
/// probe, the wait between attempts and the files with their hashes. The
/// daemon uses [`ensure_model`]; a test builds its own to point at a local
/// server with tiny fixture files.
pub(crate) struct ModelSource<'a> {
    pub base_url: &'a str,
    pub oracle: &'a dyn DiskOracle,
    pub backoff: &'a dyn BackoffPolicy,
    pub timeouts: Timeouts,
    pub files: &'a [FileSpec],
    pub lock: CacheLock,
}

/// How long one attempt may wait on the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Timeouts {
    pub connect: std::time::Duration,
    /// Restarted by every read, and running from the request to its headers:
    /// a slow link that keeps sending finishes, a silent one does not.
    pub read: std::time::Duration,
}

/// No total deadline: on a slow link the 470 MB model legitimately takes
/// longer than any bound that would still catch a stall.
pub(crate) const DOWNLOAD_TIMEOUTS: Timeouts = Timeouts {
    connect: std::time::Duration::from_secs(30),
    read: std::time::Duration::from_secs(60),
};

/// Production entrypoint: download the embedder model on first run.
pub async fn ensure_model(
    base_dir: &Path,
    progress: Option<UnboundedSender<DownloadProgress>>,
    lock: &CacheLock,
) -> Result<ModelFiles, DownloadError> {
    let url = match base_url_from(std::env::var(BASE_URL_VAR).ok().as_deref()) {
        Ok(url) => url,
        Err(e) => {
            let e = DownloadError::BaseUrl(e);
            if let Some(tx) = progress.as_ref() {
                let _ = tx.send(DownloadProgress::Error { message: e.to_string() });
            }
            return Err(e);
        }
    };
    let source = ModelSource {
        base_url: &url,
        oracle: &Fs2Oracle,
        backoff: &RealBackoff,
        timeouts: DOWNLOAD_TIMEOUTS,
        files: &ALL_FILES,
        lock: lock.clone(),
    };
    ensure_model_from(base_dir, &source, progress).await
}

/// [`ensure_model`] against any [`ModelSource`].
pub(crate) async fn ensure_model_from(
    base_dir: &Path,
    source: &ModelSource<'_>,
    progress: Option<UnboundedSender<DownloadProgress>>,
) -> Result<ModelFiles, DownloadError> {
    // Exactly one error event on every failure: the UI treats `Error` as
    // terminal, so a second one would resurrect a finished download.
    match fetch(base_dir, source, progress.as_ref()).await {
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

pub(crate) fn cache_dir_of(base_dir: &Path) -> PathBuf {
    base_dir.join("models").join("multilingual-e5-small")
}

async fn fetch(
    base_dir: &Path,
    source: &ModelSource<'_>,
    progress: Option<&UnboundedSender<DownloadProgress>>,
) -> Result<ModelFiles, DownloadError> {
    let cache_dir = cache_dir_of(base_dir);
    crate::private_dir::ensure_private_dir(&cache_dir).map_err(|e| DownloadError::Io {
        file: cache_dir.display().to_string(),
        source: e,
    })?;

    // (1) Cache-hit fast path: NO events emitted. Every startup after the first
    // takes this branch, and emitting progress here would flash a download UI
    // for work that never happens.
    let mut cached = check_cache_off_runtime(&cache_dir, source.files, &source.lock).await?;
    if let Some(busy) = cached.busy_error() {
        return Err(busy);
    }
    if cached.missing.is_empty() {
        return cached.into_model_files();
    }

    // (2) Disk pre-flight: fail before the first byte rather than half a
    // gigabyte in, when the user can still act on the message.
    let free = source.oracle.available_bytes(&cache_dir).map_err(|e| DownloadError::Io {
        file: cache_dir.display().to_string(),
        source: e,
    })?;
    if free < MIN_FREE_BYTES {
        return Err(DownloadError::InsufficientDisk {
            free_mb: free / (1024 * 1024),
            required_mb: MIN_FREE_BYTES / (1024 * 1024),
        });
    }

    let client = reqwest::Client::builder()
        .connect_timeout(source.timeouts.connect)
        .read_timeout(source.timeouts.read)
        .build()
        .map_err(DownloadError::Client)?;

    // (3) bytes_total is unknown here (no HEAD probes: a round trip without
    // observable gain, since the values arrive on the first `Chunk` anyway).
    if let Some(tx) = progress {
        let _ = tx.send(DownloadProgress::Start { bytes_total: 0 });
    }

    // (4) Download every missing or mismatched file.
    let mut bytes_downloaded_running: u64 = 0;
    let mut downloaded_onnx = None;
    for spec in std::mem::take(&mut cached.missing) {
        let bytes = download_one(
            &client,
            &cache_dir,
            &spec,
            source.base_url,
            source.backoff,
            progress,
            &mut bytes_downloaded_running,
            &source.lock,
        )
        .await?;
        if spec.kind == FileKind::Onnx {
            // The model is loaded from the cache it was renamed into, as a
            // reload does: that takes the read side of the lock and maps the
            // file, so no copy of it in memory ever reaches the load.
            downloaded_onnx = Some(spec);
        } else if let Some(bytes) = bytes {
            cached.found.push((spec.kind, bytes));
        }
    }
    if let Some(spec) = downloaded_onnx {
        let mut again = check_cache_off_runtime(&cache_dir, std::slice::from_ref(&spec), &source.lock).await?;
        if let Some(busy) = again.busy_error() {
            return Err(busy);
        }
        if !again.missing.is_empty() {
            return Err(DownloadError::NotCached { files: vec![spec.path.to_string()] });
        }
        cached.onnx = again.onnx;
    }

    // `Verify` but never `Done`: `Done` means "the embedder is ready to use",
    // which only the bootstrap knows, after the model load. Emitting it here
    // too would deliver two `done` events on the happy path.
    if let Some(tx) = progress {
        let _ = tx.send(DownloadProgress::Verify);
    }
    cached.into_model_files()
}

/// The model from the cache alone: a missing or altered file is an error and
/// nothing is downloaded. What a reload of the idle model reads.
pub async fn load_cached_model(base_dir: &Path, lock: &CacheLock) -> Result<ModelFiles, DownloadError> {
    load_cached_from(base_dir, &ALL_FILES, lock).await
}

pub(crate) async fn load_cached_from(
    base_dir: &Path,
    files: &[FileSpec],
    lock: &CacheLock,
) -> Result<ModelFiles, DownloadError> {
    let mut cached = check_cache_off_runtime(&cache_dir_of(base_dir), files, lock).await?;
    if let Some(busy) = cached.busy_error() {
        return Err(busy);
    }
    if !cached.missing.is_empty() {
        return Err(DownloadError::NotCached {
            files: cached.missing.iter().map(|spec| spec.path.to_string()).collect(),
        });
    }
    cached.into_model_files()
}

/// [`check_cache`] on a blocking thread: reading and hashing half a gigabyte
/// would hold up every other task of the runtime thread it ran on. The read
/// side of `lock` is taken first, here, so the wait for a repair that is
/// renaming the model never holds a blocking thread.
async fn check_cache_off_runtime(
    cache_dir: &Path,
    files: &[FileSpec],
    lock: &CacheLock,
) -> Result<CacheCheck, DownloadError> {
    let guard = lock.read().await;
    let (dir, files) = (cache_dir.to_path_buf(), files.to_vec());
    tokio::task::spawn_blocking(move || check_cache(&dir, &files, guard))
        .await
        .map_err(|e| DownloadError::Io {
            file: cache_dir.display().to_string(),
            source: std::io::Error::other(e.to_string()),
        })
}

/// What the cache holds: the bytes of every file that passed its hash, and
/// the files that are missing or do not.
struct CacheCheck {
    found: Vec<(FileKind, Vec<u8>)>,
    /// `model.onnx` when it verified: the view its hash was taken on.
    onnx: Option<OnnxFile>,
    missing: Vec<FileSpec>,
    /// Files another program kept open past the retries: neither found nor
    /// missing, so nothing is downloaded over them.
    busy: Vec<(String, std::io::Error)>,
}

impl CacheCheck {
    /// The error for the first file that stayed busy, when one did.
    fn busy_error(&mut self) -> Option<DownloadError> {
        if self.busy.is_empty() {
            return None;
        }
        let (file, source) = self.busy.swap_remove(0);
        Some(DownloadError::Busy { file, source })
    }

    fn into_model_files(self) -> Result<ModelFiles, DownloadError> {
        let onnx = self
            .onnx
            .ok_or_else(|| DownloadError::NotCached { files: vec!["onnx/model.onnx".to_string()] })?;
        let mut tokenizer = TokenizerFiles {
            tokenizer_file: Vec::new(),
            config_file: Vec::new(),
            special_tokens_map_file: Vec::new(),
            tokenizer_config_file: Vec::new(),
        };
        for (kind, bytes) in self.found {
            match kind {
                FileKind::Onnx => unreachable!("the model is kept as a view, never as bytes"),
                FileKind::TokenizerJson => tokenizer.tokenizer_file = bytes,
                FileKind::ConfigJson => tokenizer.config_file = bytes,
                FileKind::SpecialTokensMap => tokenizer.special_tokens_map_file = bytes,
                FileKind::TokenizerConfig => tokenizer.tokenizer_config_file = bytes,
            }
        }
        Ok(ModelFiles { onnx, tokenizer })
    }
}

/// Reads every file of `files` once, keeping the bytes of those that pass and,
/// for the model, the view they were hashed on, which takes `guard` with it.
/// A guard the model does not keep is released when the check ends.
fn check_cache(cache_dir: &Path, files: &[FileSpec], guard: OwnedRwLockReadGuard<()>) -> CacheCheck {
    let mut check = CacheCheck { found: Vec::new(), onnx: None, missing: Vec::new(), busy: Vec::new() };
    let mut guard = Some(guard);
    for spec in files {
        let path = cache_dir.join(spec.path);
        let checked = match spec.kind {
            FileKind::Onnx => match guard.take() {
                Some(guard) => verified_onnx(&path, spec.sha256, guard).map(|view| check.onnx = Some(view)),
                None => Err(Unusable::Altered),
            },
            kind => read_verified(&path, spec.sha256).map(|bytes| check.found.push((kind, bytes))),
        };
        match checked {
            Ok(()) => {}
            Err(Unusable::Busy(e)) => check.busy.push((spec.path.to_string(), e)),
            Err(_) => check.missing.push(spec.clone()),
        }
    }
    check
}

/// Cache-presence probe that never touches the network: true iff every file
/// in the manifest is already on disk with a matching SHA-256, by the same
/// check [`ensure_model`] makes.
#[cfg(feature = "eval")]
pub(crate) fn model_is_cached(base_dir: &Path) -> bool {
    let check = check_cache(&cache_dir_of(base_dir), &ALL_FILES, CacheLock::new().read_now());
    check.missing.is_empty() && check.busy.is_empty()
}

/// Why a file on disk cannot be used.
#[derive(Debug)]
pub(crate) enum Unusable {
    Unreadable(std::io::Error),
    /// Another program holds the file past the retries of [`read_retrying`].
    Busy(std::io::Error),
    Altered,
}

/// How long [`read_retrying`] waits between attempts to read a file another
/// program holds: 1.9 s in all, short enough for a request to wait on.
const OPEN_RETRY_WAITS_MS: [u64; 7] = [15, 30, 60, 120, 240, 480, 960];

/// Whether the OS error `raw` says another program holds the file: access
/// denied, sharing violation or lock violation on Windows, where an antivirus
/// scanning the file gives any of them. Never read from `ErrorKind`: Rust
/// leaves the sharing violation `Uncategorized`.
fn is_sharing_conflict(raw: Option<i32>) -> bool {
    #[cfg(windows)]
    {
        const ERROR_ACCESS_DENIED: i32 = 5;
        const ERROR_SHARING_VIOLATION: i32 = 32;
        const ERROR_LOCK_VIOLATION: i32 = 33;
        matches!(raw, Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION))
    }
    #[cfg(not(windows))]
    {
        let _ = raw;
        false
    }
}

/// `attempt`, run again over [`OPEN_RETRY_WAITS_MS`] while another program
/// holds the file. Sleeps, so it runs on a blocking thread.
fn retrying<T>(mut attempt: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut waits = OPEN_RETRY_WAITS_MS.iter();
    loop {
        match attempt() {
            Err(e) if is_sharing_conflict(e.raw_os_error()) => match waits.next() {
                Some(ms) => std::thread::sleep(std::time::Duration::from_millis(*ms)),
                None => return Err(e),
            },
            other => return other,
        }
    }
}

/// `std::fs::read` under [`retrying`].
fn read_retrying(path: &Path) -> std::io::Result<Vec<u8>> {
    retrying(|| std::fs::read(path))
}

fn unusable(e: std::io::Error) -> Unusable {
    if is_sharing_conflict(e.raw_os_error()) {
        Unusable::Busy(e)
    } else {
        Unusable::Unreadable(e)
    }
}

/// `path` opened for reading. On Windows it shares reading and nothing else,
/// so no handle can write, truncate, rename or delete the file while this one
/// lives: the constant is Win32's `FILE_SHARE_READ`.
fn open_shared_read(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x1;
        options.share_mode(FILE_SHARE_READ);
    }
    options.open(path)
}

/// `model.onnx` mapped read-only, when the SHA-256 of the mapped bytes is
/// `expected_sha`: the same bytes the load receives. The guard of the cache
/// lock goes into the view and is released with it, or here when the file does
/// not verify; the map and the handle are always closed first.
pub(crate) fn verified_onnx(
    path: &Path,
    expected_sha: &str,
    guard: OwnedRwLockReadGuard<()>,
) -> Result<OnnxFile, Unusable> {
    let file = retrying(|| open_shared_read(path)).map_err(unusable)?;
    // No model file is empty, and mapping nothing is not the same call on
    // every system.
    if file.metadata().map_err(unusable)?.len() == 0 {
        return Err(Unusable::Altered);
    }
    // SAFETY: read-only mapping that lives no longer than `file`. On Windows
    // `file` shares neither writing nor deletion, so no handle can write,
    // truncate, rename or delete it while the view exists; on Unix the data
    // directory is 0700 and clawg never writes model.onnx in place, it
    // downloads to `.partial` and renames. Without probing the handle, the
    // mapping is `PAGE_READONLY` / `FILE_MAP_READ` on Windows.
    let map = unsafe { memmap2::MmapOptions::new().no_probe_handle().map(&file) }.map_err(unusable)?;
    #[cfg(test)]
    READS.lock().unwrap_or_else(|e| e.into_inner()).push(path.to_path_buf());
    if sha256_hex(&map) != expected_sha {
        return Err(Unusable::Altered);
    }
    Ok(OnnxFile(View::Mapped { map, _file: file, _cache: guard }))
}

/// Every file whose bytes were hashed, read from the cache or as they arrived,
/// so a test can count the hashes of its own directory whatever thread did them.
#[cfg(test)]
static READS: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// The bytes of `path` when their SHA-256 is `expected_sha`: the one place the
/// cache is read and checked.
fn read_verified(path: &Path, expected_sha: &str) -> Result<Vec<u8>, Unusable> {
    let bytes = read_retrying(path).map_err(unusable)?;
    #[cfg(test)]
    READS.lock().unwrap_or_else(|e| e.into_inner()).push(path.to_path_buf());
    if sha256_hex(&bytes) != expected_sha {
        return Err(Unusable::Altered);
    }
    Ok(bytes)
}

/// Hex-lowercase SHA-256 of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex_digest(sha2::Sha256::digest(bytes).as_slice())
}

pub(crate) fn hex_digest(digest: &[u8]) -> String {
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
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

/// Per-file download with retry (3 attempts, 5s/30s/120s backoff), returning
/// the bytes it hashed and committed. A SHA-256
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
    client: &reqwest::Client,
    cache_dir: &Path,
    spec: &FileSpec,
    base_url_str: &str,
    backoff: &dyn BackoffPolicy,
    progress: Option<&UnboundedSender<DownloadProgress>>,
    bytes_downloaded_running: &mut u64,
    lock: &CacheLock,
) -> Result<Option<Vec<u8>>, DownloadError> {
    use futures::StreamExt;
    use sha2::Digest;
    use std::io::Write;

    let target = cache_dir.join(spec.path);
    if let Some(parent) = target.parent() {
        crate::private_dir::ensure_private_dir(parent).map_err(|e| DownloadError::Io {
            file: spec.path.into(),
            source: e,
        })?;
    }
    let partial = partial_path_for(&target);

    const ATTEMPTS: u32 = 3;
    let host = host_of(base_url_str);
    let mut cause = String::new();

    for attempt in 0..ATTEMPTS {
        if !cause.is_empty() {
            log::warn!(
                "model download of {} from {host} failed on attempt {attempt} of {ATTEMPTS}: {cause}",
                spec.path
            );
        }
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
        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                let _ = std::fs::remove_file(&partial);
                cause = describe(e);
                continue;
            }
        };
        if !resp.status().is_success() {
            let _ = std::fs::remove_file(&partial);
            cause = format!("HTTP {}", resp.status());
            continue;
        }

        // Track per-file emission threshold — one Chunk per 64 KB of running
        // total (not per file), so a multi-file run yields a smooth progress
        // ramp without spamming the channel.
        let content_length = resp.content_length().unwrap_or(0);
        if content_length > MAX_FILE_BYTES {
            let _ = std::fs::remove_file(&partial);
            return Err(too_large(spec));
        }
        let bytes_total_hint = *bytes_downloaded_running + content_length;
        let mut next_emit_threshold = *bytes_downloaded_running + CHUNK_PROGRESS_BYTES;

        // The model is written to `.partial` and nowhere else; only the small
        // files are kept, to hand them to the load.
        let mut body = (spec.kind != FileKind::Onnx).then(|| Vec::with_capacity(content_length as usize));
        let mut received: u64 = 0;
        let mut digest = sha2::Sha256::new();
        let mut stream = resp.bytes_stream();
        let mut fatal: Option<DownloadError> = None;
        let mut stream_err: Option<reqwest::Error> = None;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    if received + bytes.len() as u64 > MAX_FILE_BYTES {
                        fatal = Some(too_large(spec));
                        break;
                    }
                    if let Err(e) = file.write_all(&bytes) {
                        fatal = Some(DownloadError::Io {
                            file: spec.path.into(),
                            source: e,
                        });
                        break;
                    }
                    digest.update(&bytes);
                    if let Some(body) = body.as_mut() {
                        body.extend_from_slice(&bytes);
                    }
                    received += bytes.len() as u64;
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
                Err(e) => {
                    stream_err = Some(e);
                    break;
                }
            }
        }

        if let Some(e) = fatal {
            // Not transient: a full disk or an oversized file does not change
            // on the next attempt.
            let _ = std::fs::remove_file(&partial);
            return Err(e);
        }
        if let Some(e) = stream_err {
            let _ = std::fs::remove_file(&partial);
            cause = describe(e);
            continue;
        }

        // Emit one final Chunk so consumers see exact byte count post-write.
        if let Some(tx) = progress {
            let _ = tx.send(DownloadProgress::Chunk {
                bytes_downloaded: *bytes_downloaded_running,
                bytes_total: bytes_total_hint,
            });
        }

        // Hashed as the bytes arrived: the file is never read back.
        let got = hex_digest(digest.finalize().as_slice());
        #[cfg(test)]
        READS.lock().unwrap_or_else(|e| e.into_inner()).push(target.clone());
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

        // Syncing half a gigabyte can take seconds: not on a runtime thread.
        let committed = {
            let (partial, target) = (partial.clone(), target.clone());
            // A load holds `model.onnx` open without sharing deletion: the
            // rename waits for it, after the sync so that a load never waits
            // for half a gigabyte to reach the disk.
            let lock = (spec.kind == FileKind::Onnx).then(|| lock.clone());
            tokio::task::spawn_blocking(move || -> std::io::Result<()> {
                file.flush()?;
                file.sync_all()?;
                drop(file);
                let _write = lock.as_ref().map(|lock| lock.0.blocking_write());
                commit_file(&partial, &target)
            })
            .await
            .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string())))
        };
        if let Err(e) = committed {
            let _ = std::fs::remove_file(&partial);
            return Err(DownloadError::Io {
                file: spec.path.into(),
                source: e,
            });
        }
        return Ok(body);
    }

    log::warn!("model download of {} from {host} failed on attempt {ATTEMPTS} of {ATTEMPTS}: {cause}", spec.path);
    Err(DownloadError::RetryExhausted {
        file: spec.path.into(),
        host,
        attempts: ATTEMPTS,
        cause,
    })
}

/// Larger than any file of the model: what the disk pre-flight asks to have
/// free.
const MAX_FILE_BYTES: u64 = MIN_FREE_BYTES;

fn too_large(spec: &FileSpec) -> DownloadError {
    DownloadError::TooLarge {
        file: spec.path.into(),
        limit_mb: MAX_FILE_BYTES / (1024 * 1024),
    }
}

/// The host and port of `base_url`, without the path or any credentials it
/// carries: what an error may show of the origin.
fn host_of(base_url: &str) -> String {
    reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?.to_string();
            Some(match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host,
            })
        })
        .unwrap_or_else(|| "the model origin".to_string())
}

/// `e` and the chain of its causes, where the refused connection, the TLS
/// failure or the timeout is named, without the URL `reqwest` attaches.
fn describe(e: reqwest::Error) -> String {
    let e = e.without_url();
    let mut text = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
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
    // Use std::env::temp_dir() + a unique subdir per test rather than the
    // `tempfile` crate.
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
                "clawg-{label}-{pid}-{nanos}-{n}"
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

    fn source<'a>(base_url: &'a str, oracle: &'a dyn DiskOracle, files: &'a [FileSpec]) -> ModelSource<'a> {
        ModelSource { base_url, oracle, backoff: &ZeroBackoff, timeouts: DOWNLOAD_TIMEOUTS, files, lock: CacheLock::new() }
    }

    // ---------- Disk pre-flight ----------

    #[tokio::test]
    async fn insufficient_disk_returns_err_no_http() {
        let tmp = TestDir::new("preflight-insufficient");
        // Mockito server present BUT asserts zero hits — pre-flight must short-circuit.
        let mut server = mockito::Server::new_async().await;
        let mock = server.mock("GET", mockito::Matcher::Any).expect(0).create_async().await;

        let oracle = MockDiskOracle::returning(599 * 1024 * 1024);
        let err = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &ALL_FILES), None)
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

        let err = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &ALL_FILES), None)
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
                bytes: b"fake-onnx-model-bytes".to_vec(),
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
        let res = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), None)
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
        let res = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), None)
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
        let res = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), None)
        .await;
        assert!(res.is_ok(), "got {:?}", res.err());
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        let partials = collect_partials(&cache_dir);
        assert!(
            partials.is_empty(),
            "expected zero .partial files, found {partials:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_model_cache_is_created_private() {
        let tmp = TestDir::new("dl-private");
        let files = fixture_files();
        let specs = fixture_specs(&files);
        let mut server = mockito::Server::new_async().await;
        let _mocks = mount_all(&mut server, &files, 200).await;
        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);

        ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), None).await.unwrap();

        let models = tmp.path().join("models");
        let cache_dir = models.join("multilingual-e5-small");
        let mut dirs = vec![models, cache_dir.clone()];
        for spec in &specs {
            let mut parent = cache_dir.join(spec.path).parent().unwrap().to_path_buf();
            while parent != cache_dir {
                dirs.push(parent.clone());
                parent = parent.parent().unwrap().to_path_buf();
            }
        }
        for dir in dirs {
            assert_eq!(crate::private_dir::mode_of(&dir), 0o700, "{}", dir.display());
        }
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
        let res = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), None)
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
        let res = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), None)
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
        let res = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), None)
        .await;
        match res {
            Err(DownloadError::RetryExhausted { attempts, file, .. }) => {
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
        let res = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), None)
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
        let res = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), None)
        .await;
        assert!(res.is_ok(), "got {:?}", res.err());
        m_missing.assert_async().await;
        for m in &other_mocks {
            m.assert_async().await;
        }
    }

    // ---------- Deadlines ----------

    /// A server on 127.0.0.1 that reads each request, writes `reply` and then
    /// keeps the connection open without sending anything more.
    fn silent_server(reply: &'static [u8]) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for mut stream in listener.incoming().flatten() {
                let mut request = Vec::new();
                let mut buf = [0u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buf[..n]),
                    }
                }
                let _ = stream.write_all(reply);
                let _ = stream.flush();
                held.push(stream);
            }
        });
        url
    }

    const SHORT: Timeouts = Timeouts {
        connect: std::time::Duration::from_secs(5),
        read: std::time::Duration::from_millis(300),
    };

    async fn ensure_against_silent_server(label: &str, reply: &'static [u8]) {
        let tmp = TestDir::new(label);
        let files = fixture_files();
        let specs = fixture_specs(&files);
        let url = silent_server(reply);
        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let source = ModelSource { timeouts: SHORT, ..source(&url, &oracle, &specs) };

        let res = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            ensure_model_from(tmp.path(), &source, None),
        )
        .await
        .expect("the download returned instead of waiting forever");

        match res {
            Err(e @ DownloadError::RetryExhausted { .. }) => {
                let DownloadError::RetryExhausted { file, attempts, .. } = &e else { unreachable!() };
                assert_eq!((file.as_str(), *attempts), ("onnx/model.onnx", 3));
                assert!(e.to_string().contains("timed out"), "{e}");
            }
            other => panic!("expected RetryExhausted, got {other:?}"),
        }
        let partials = collect_partials(&tmp.path().join("models").join("multilingual-e5-small"));
        assert!(partials.is_empty(), "{partials:?}");
    }

    #[tokio::test]
    async fn a_body_that_stops_arriving_fails_the_attempt() {
        ensure_against_silent_server(
            "dl-stalled-body",
            b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nfake-onnx",
        )
        .await;
    }

    #[tokio::test]
    async fn a_server_that_never_answers_fails_the_attempt() {
        ensure_against_silent_server("dl-no-answer", b"").await;
    }

    // ---------- Off the runtime, one hash per file ----------

    /// The files under `dir` that [`read_verified`] read since the test began.
    fn reads_under(dir: &Path) -> Vec<String> {
        READS
            .lock()
            .unwrap()
            .iter()
            .filter_map(|p| p.strip_prefix(dir).ok())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect()
    }

    #[tokio::test]
    async fn the_cache_check_leaves_the_runtime_thread_free() {
        let tmp = TestDir::new("cache-off-runtime");
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        let mut files = fixture_files();
        files[0].bytes = vec![7u8; 16 * 1024 * 1024];
        write_all_to_cache(&cache_dir, &files);
        let specs = fixture_specs(&files);
        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);

        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ticker = tokio::spawn({
            let done = done.clone();
            async move {
                let mut ticks = 0u32;
                while !done.load(Ordering::SeqCst) {
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    ticks += 1;
                }
                ticks
            }
        });
        tokio::task::yield_now().await;
        let res = ensure_model_from(tmp.path(), &source("http://unused.invalid", &oracle, &specs), None).await;
        done.store(true, Ordering::SeqCst);
        assert!(res.is_ok(), "{:?}", res.err());

        let ticks = ticker.await.unwrap();
        assert!(ticks >= 5, "the runtime thread ticked {ticks} times while the cache was hashed");
    }

    #[tokio::test]
    async fn each_file_is_hashed_once_per_run() {
        let files = fixture_files();
        let specs = fixture_specs(&files);
        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);

        for damaged in ["config.json", "onnx/model.onnx"] {
            let tmp = TestDir::new("hash-once");
            let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
            write_all_to_cache(&cache_dir, &files);
            std::fs::remove_file(cache_dir.join(damaged)).unwrap();
            let mut server = mockito::Server::new_async().await;
            let _mocks = mount_all(&mut server, &files, 200).await;

            let res = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), None).await;
            let model = res.unwrap_or_else(|e| panic!("{damaged}: {e}"));
            assert_eq!(&*model.onnx, &files[0].bytes[..], "{damaged}");

            let mut reads = reads_under(&cache_dir);
            reads.sort();
            let mut expected: Vec<String> = files.iter().map(|f| f.path.to_string()).collect();
            if damaged == "onnx/model.onnx" {
                expected.push(damaged.to_string());
            }
            expected.sort();
            assert_eq!(
                reads, expected,
                "with {damaged} missing, every file hashed once and the downloaded model a second time, from the cache"
            );
        }
    }

    #[tokio::test]
    async fn a_downloaded_onnx_is_not_kept_in_memory_and_is_loaded_from_the_cache() {
        let files = fixture_files();
        let specs = fixture_specs(&files);
        let tmp = TestDir::new("onnx-not-kept");
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        let mut server = mockito::Server::new_async().await;
        let _mocks = mount_all(&mut server, &files, 200).await;
        let client = reqwest::Client::new();
        let lock = CacheLock::new();
        let mut running = 0;
        let url = server.url();

        for spec in &specs {
            let body = download_one(&client, &cache_dir, spec, &url, &ZeroBackoff, None, &mut running, &lock)
                .await
                .expect("downloads");
            assert_eq!(body.is_some(), spec.kind != FileKind::Onnx, "{}: what the run keeps", spec.path);
        }

        let tmp = TestDir::new("onnx-loaded-from-cache");
        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let model = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), None)
            .await
            .expect("a fresh run downloads and loads");
        assert!(model.onnx.is_view(), "the model came from the cache, not from memory");
        assert_eq!(&*model.onnx, &files[0].bytes[..]);
    }

    #[test]
    fn the_wait_for_an_open_is_bounded_and_only_windows_sharing_errors_count() {
        let total: u64 = OPEN_RETRY_WAITS_MS.iter().sum();
        assert!(total <= 2_000, "the retries wait {total} ms");
        for raw in [5, 32, 33] {
            assert_eq!(is_sharing_conflict(Some(raw)), cfg!(windows), "os error {raw}");
        }
        for raw in [Some(2), Some(3), Some(13), Some(0), None] {
            assert!(!is_sharing_conflict(raw), "{raw:?}");
        }
    }

    /// Holds `path` open without sharing, for the length of the guard.
    #[cfg(windows)]
    fn held_without_sharing(path: &Path) -> std::fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).share_mode(0).open(path).expect("hold the file")
    }

    #[cfg(windows)]
    #[test]
    fn a_sharing_violation_on_open_is_retried_within_the_wait() {
        let tmp = TestDir::new("open-retried");
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        let files = fixture_files();
        write_all_to_cache(&cache_dir, &files);
        let specs = fixture_specs(&files);
        let held = held_without_sharing(&cache_dir.join(specs[1].path));
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            drop(held);
        });

        let check = check_cache(&cache_dir, &specs, CacheLock::new().read_now());
        release.join().unwrap();
        assert!(check.missing.is_empty() && check.busy.is_empty(), "missing {:?}", check.missing);
        assert_eq!(check.found.len() + usize::from(check.onnx.is_some()), specs.len());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_persistent_sharing_violation_is_not_a_missing_file_at_start() {
        let tmp = TestDir::new("open-busy");
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        let files = fixture_files();
        write_all_to_cache(&cache_dir, &files);
        let specs = fixture_specs(&files);
        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let _held = held_without_sharing(&cache_dir.join(specs[1].path));
        let server = mockito::Server::new_async().await;

        let res = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), None).await;
        match res {
            Err(DownloadError::Busy { file, .. }) => assert_eq!(file, specs[1].path),
            other => panic!("expected Busy, got {other:?}"),
        }
        assert!(collect_partials(&cache_dir).is_empty(), "nothing was downloaded");
        assert!(matches!(load_cached_from(tmp.path(), &specs, &CacheLock::new()).await, Err(DownloadError::Busy { .. })));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_held_file_wins_over_a_missing_one() {
        let tmp = TestDir::new("open-busy-and-missing");
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        let files = fixture_files();
        write_all_to_cache(&cache_dir, &files);
        let specs = fixture_specs(&files);
        std::fs::remove_file(cache_dir.join(specs[0].path)).expect("remove a file");
        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let _held = held_without_sharing(&cache_dir.join(specs[1].path));
        let server = mockito::Server::new_async().await;

        let res = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), None).await;
        assert!(matches!(res, Err(DownloadError::Busy { .. })), "{res:?}");
        assert!(collect_partials(&cache_dir).is_empty(), "nothing was downloaded");
        let cached = load_cached_from(tmp.path(), &specs, &CacheLock::new()).await;
        assert!(matches!(cached, Err(DownloadError::Busy { .. })), "{cached:?}");
    }

    #[tokio::test]
    async fn the_cached_model_never_touches_the_network() {
        let tmp = TestDir::new("cache-only");
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        let files = fixture_files();
        let specs = fixture_specs(&files);
        write_all_to_cache(&cache_dir, &files);

        let model = load_cached_from(tmp.path(), &specs, &CacheLock::new()).await.expect("a good cache loads");
        assert_eq!(&*model.onnx, &files[0].bytes[..]);

        std::fs::remove_file(cache_dir.join("tokenizer_config.json")).unwrap();
        std::fs::write(cache_dir.join("config.json"), b"altered").unwrap();
        match load_cached_from(tmp.path(), &specs, &CacheLock::new()).await {
            Err(DownloadError::NotCached { files }) => {
                assert_eq!(files, ["config.json", "tokenizer_config.json"]);
            }
            other => panic!("expected NotCached, got {other:?}"),
        }
        assert!(collect_partials(&cache_dir).is_empty());
    }

    // ---------- The model as a read-only view ----------

    #[tokio::test]
    async fn the_cache_check_hands_the_onnx_over_as_a_read_only_view() {
        let tmp = TestDir::new("onnx-view");
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        let files = fixture_files();
        let specs = fixture_specs(&files);
        write_all_to_cache(&cache_dir, &files);
        let lock = CacheLock::new();

        let model = load_cached_from(tmp.path(), &specs, &lock).await.expect("a good cache loads");
        assert!(model.onnx.is_view(), "the model is a view of the file, not a copy");
        assert_eq!(&*model.onnx, &files[0].bytes[..]);
        assert!(lock.0.try_write().is_err(), "the view holds the cache lock");
        drop(model);
        assert!(lock.0.try_write().is_ok(), "the lock goes with the view");
    }

    #[test]
    fn a_view_of_a_changed_onnx_is_refused() {
        let tmp = TestDir::new("onnx-changed");
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        let files = fixture_files();
        let specs = fixture_specs(&files);
        write_all_to_cache(&cache_dir, &files);
        let path = cache_dir.join(specs[0].path);
        let lock = CacheLock::new();

        let mut bytes = std::fs::read(&path).unwrap();
        bytes[0] = bytes[0].wrapping_add(1);
        std::fs::write(&path, &bytes).unwrap();
        let refused = verified_onnx(&path, specs[0].sha256, lock.read_now());
        assert!(matches!(refused, Err(Unusable::Altered)));
        assert!(lock.0.try_write().is_ok(), "a refused file keeps no lock");
        let check = check_cache(&cache_dir, &specs, lock.read_now());
        assert!(check.onnx.is_none() && check.missing.iter().any(|spec| spec.path == specs[0].path));

        std::fs::write(&path, b"").unwrap();
        let empty = verified_onnx(&path, specs[0].sha256, lock.read_now());
        assert!(matches!(empty, Err(Unusable::Altered)), "an empty file is not mapped");
        std::fs::remove_file(&path).expect("a refused file is closed");
    }

    /// The OS error number of a failed call.
    #[cfg(windows)]
    fn raw_error<T>(result: std::io::Result<T>) -> Option<i32> {
        result.err().and_then(|e| e.raw_os_error())
    }

    #[cfg(windows)]
    #[test]
    fn a_load_holds_the_model_file_against_writes_until_the_session_returns() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = TestDir::new("carga-\u{f1}and\u{fa}");
        let onnx = crate::memory::embedder::minimal_onnx();
        let path = tmp.path().join("model.onnx");
        let other = tmp.path().join("other.partial");
        let moved = tmp.path().join("moved.onnx");
        std::fs::write(&path, &onnx).unwrap();
        std::fs::write(&other, b"another file").unwrap();
        let lock = CacheLock::new();

        let view = verified_onnx(&path, &sha256_hex(&onnx), lock.read_now()).expect("the model verifies");
        let refused = |raw: Option<i32>| matches!(raw, Some(5 | 32));
        let write = std::fs::OpenOptions::new().write(true).open(&path);
        assert!(refused(raw_error(write)), "writing");
        let truncate = std::fs::OpenOptions::new().write(true).truncate(true).open(&path);
        assert!(refused(raw_error(truncate)), "truncating");
        assert!(refused(raw_error(std::fs::rename(&other, &path))), "renaming another file over it");
        assert!(refused(raw_error(std::fs::rename(&path, &moved))), "renaming it");
        assert!(refused(raw_error(std::fs::remove_file(&path))), "deleting it");
        let read = std::fs::OpenOptions::new().read(true).share_mode(0x1).open(&path);
        assert!(read.is_ok(), "a reader that shares reading opens it");
        drop(read);

        let session = crate::memory::embedder::build_session(&view).expect("the session is built from the view");
        drop(view);
        std::fs::OpenOptions::new().write(true).open(&path).expect("writable again");
        std::fs::rename(&other, &path).expect("renamed over again");
        std::fs::remove_file(&path).expect("deleted again");
        drop(session);
    }

    #[cfg(windows)]
    #[test]
    fn the_model_file_is_free_again_after_a_failed_load() {
        let tmp = TestDir::new("carga-fallida-\u{f1}");
        let garbage = b"not an onnx model".to_vec();
        let path = tmp.path().join("model.onnx");
        let other = tmp.path().join("other.partial");
        std::fs::write(&path, &garbage).unwrap();
        std::fs::write(&other, b"another file").unwrap();
        let lock = CacheLock::new();

        let view = verified_onnx(&path, &sha256_hex(&garbage), lock.read_now()).expect("the bytes verify");
        let failed = crate::memory::embedder::build_session(&view);
        assert!(failed.is_err(), "garbage is not a model");
        drop(view);
        assert!(lock.0.try_write().is_ok(), "the lock went with the view");
        std::fs::rename(&other, &path).expect("renamed over after the failure");
        std::fs::remove_file(&path).expect("deleted after the failure");
    }

    #[tokio::test]
    async fn a_repair_waits_for_a_load_to_release_the_model_file() {
        let tmp = TestDir::new("repair-waits");
        let cache_dir = tmp.path().join("models").join("multilingual-e5-small");
        let files = fixture_files();
        let specs = fixture_specs(&files);
        write_all_to_cache(&cache_dir, &files);
        let lock = CacheLock::new();
        let load = load_cached_from(tmp.path(), &specs, &lock).await.expect("a good cache loads");

        let mut server = mockito::Server::new_async().await;
        let _mocks = mount_all(&mut server, &files, 200).await;
        let client = reqwest::Client::new();
        let mut running = 0;
        let url = server.url();
        let repair = download_one(&client, &cache_dir, &specs[0], &url, &ZeroBackoff, None, &mut running, &lock);
        tokio::pin!(repair);

        let waiting = tokio::time::timeout(std::time::Duration::from_millis(400), &mut repair).await;
        assert!(waiting.is_err(), "the repair renamed the model while a load held it");
        assert_eq!(&*load.onnx, &files[0].bytes[..], "the loaded bytes are still the ones hashed");

        drop(load);
        let repaired = tokio::time::timeout(std::time::Duration::from_secs(30), &mut repair)
            .await
            .expect("the repair goes on once the load lets go");
        repaired.expect("the repair finishes");
        assert_eq!(std::fs::read(cache_dir.join(specs[0].path)).unwrap(), files[0].bytes);
    }

    // ---------- CLAWG_HF_BASE_URL ----------

    #[test]
    fn the_base_url_defaults_to_hugging_face() {
        for raw in [None, Some(""), Some("  ")] {
            assert_eq!(base_url_from(raw).as_deref(), Ok(HF_BASE_URL), "{raw:?}");
        }
    }

    #[test]
    fn the_base_url_takes_https_and_loopback_http() {
        for (raw, url) in [
            ("https://mirror.example/e5/", "https://mirror.example/e5"),
            ("https://mirror.example:8443/e5", "https://mirror.example:8443/e5"),
            ("http://127.0.0.1:8080", "http://127.0.0.1:8080"),
            ("http://localhost:9/model/", "http://localhost:9/model"),
            ("http://[::1]:9", "http://[::1]:9"),
        ] {
            assert_eq!(base_url_from(Some(raw)).as_deref(), Ok(url), "{raw}");
        }
    }

    #[test]
    fn a_base_url_that_is_not_https_or_loopback_is_refused() {
        for raw in [
            "http://example.com/e5",
            "http://user:s3cret@example.com",
            "http://127.0.0.2:8080",
            "ftp://127.0.0.1/e5",
            "file:///models",
            "not a url",
        ] {
            let err = base_url_from(Some(raw)).expect_err(raw);
            assert!(err.contains(BASE_URL_VAR), "{err}");
            assert!(!err.contains("s3cret"), "{err}");
        }
    }

    #[tokio::test]
    async fn a_file_larger_than_any_model_file_is_refused_without_holding_it() {
        let tmp = TestDir::new("dl-too-large");
        let files = fixture_files();
        let specs = fixture_specs(&files);
        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let url = silent_server(b"HTTP/1.1 200 OK\r\nContent-Length: 1099511627776\r\n\r\nfake-onnx");
        let source = ModelSource { timeouts: SHORT, ..source(&url, &oracle, &specs) };

        match ensure_model_from(tmp.path(), &source, None).await {
            Err(DownloadError::TooLarge { file, .. }) => assert_eq!(file, "onnx/model.onnx"),
            other => panic!("expected TooLarge, got {other:?}"),
        }
        assert!(collect_partials(&tmp.path().join("models").join("multilingual-e5-small")).is_empty());
    }

    #[tokio::test]
    async fn a_body_that_outgrows_the_cap_while_streaming_is_refused() {
        use std::io::{Read, Write};
        let tmp = TestDir::new("dl-streams-too-much");
        let files = fixture_files();
        let specs = fixture_specs(&files);
        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        // No Content-Length: only the count of received bytes can stop it.
        // Not joined: on this runtime the client's socket closes only after the test
        // yields, and the writer ends when it does.
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
            let block = vec![0u8; 1 << 20];
            for _ in 0..(MAX_FILE_BYTES / (1 << 20) + 8) {
                if stream.write_all(&block).is_err() {
                    return;
                }
            }
        });

        match ensure_model_from(tmp.path(), &source(&url, &oracle, &specs), None).await {
            Err(DownloadError::TooLarge { file, .. }) => assert_eq!(file, "onnx/model.onnx"),
            other => panic!("expected TooLarge, got {other:?}"),
        }
        assert!(collect_partials(&tmp.path().join("models").join("multilingual-e5-small")).is_empty());
    }

    // ---------- The cause of a failure ----------

    #[tokio::test]
    async fn an_exhausted_download_names_the_last_status_and_the_host() {
        let tmp = TestDir::new("dl-cause-status");
        let files = fixture_files();
        let specs = fixture_specs(&files);
        let mut server = mockito::Server::new_async().await;
        let _mocks = mount_all(&mut server, &files, 503).await;
        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);

        let err = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), None)
            .await
            .unwrap_err();

        let text = err.to_string();
        assert!(text.contains("HTTP 503"), "{text}");
        assert!(text.contains(&server.host_with_port()), "{text}");
        assert!(!text.contains(&format!("{}/onnx", server.url())), "{text}");
    }

    #[tokio::test]
    async fn the_cause_never_carries_the_credentials_of_the_url() {
        let files = fixture_files();
        let specs = fixture_specs(&files);
        let mut server = mockito::Server::new_async().await;
        let _mocks = mount_all(&mut server, &files, 503).await;
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);

        for base in [
            format!("http://user:s3cret@{}", server.host_with_port()),
            format!("http://user:s3cret@{closed}"),
        ] {
            let tmp = TestDir::new("dl-cause-credentials");
            let err = ensure_model_from(tmp.path(), &source(&base, &oracle, &specs), None)
                .await
                .unwrap_err();
            let text = err.to_string();
            assert!(matches!(err, DownloadError::RetryExhausted { .. }), "{text}");
            assert!(!text.contains("s3cret"), "{text}");
        }
    }

    #[test]
    fn the_download_waits_30_s_to_connect_and_60_s_per_read() {
        assert_eq!(DOWNLOAD_TIMEOUTS.connect, std::time::Duration::from_secs(30));
        assert_eq!(DOWNLOAD_TIMEOUTS.read, std::time::Duration::from_secs(60));
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
        // to emit AFTER the model load + `OnceCell::set` succeed.
        // If `ensure_model` also emitted `Done`, the forwarder→sink path in
        // `memory::bootstrap` would deliver TWO `done` events on the happy
        // path.
        let tmp = TestDir::new("evt-order");
        let files = fixture_files();
        let specs = fixture_specs(&files);

        let mut server = mockito::Server::new_async().await;
        let _mocks = mount_all(&mut server, &files, 200).await;

        let oracle = MockDiskOracle::returning(MIN_FREE_BYTES);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DownloadProgress>();
        let res = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), Some(tx))
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
        // bootstrap layer (`memory::bootstrap`) so a listener receives EXACTLY
        // ONE terminal `done`, after the embedder is actually loadable.
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
        let _ = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), Some(tx))
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
        let res = ensure_model_from(tmp.path(), &source(&server.url(), &oracle, &specs), Some(tx))
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
        let res = ensure_model_from(tmp.path(), &source("http://unused.invalid", &oracle, &specs), None)
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
