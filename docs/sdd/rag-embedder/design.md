# Phase 2 — rag-embedder — Design

> **Inputs**: `proposal.md` (the contract — all 15 decisions locked) + `explore.md` (evidence + line refs).
> **Status**: All referenced source patterns verified against current code (state.rs, lib.rs, memory/db.rs, models/chat.rs, commands/chat.rs, chat.service.ts, config.service.ts, config-panel.component.ts/html, Cargo.toml).
>
> **Open contradiction with proposal** (raised, not silently resolved): proposal decision #14 names `sysinfo` for the disk pre-flight, but `sysinfo`'s disk-introspection API is heavier than needed and its enabled feature set is non-trivial on Windows. `fs2` (one function: `available_space(path) -> io::Result<u64>`, ~50 KB, MIT) is the lean fit. **Design proposes `fs2`. If the orchestrator wants the contract preserved verbatim, swap the dep in `Cargo.toml` — every other section here works unchanged with `sysinfo`.** Flagged in §10.

---

## 1. Overview

`AppState` gains a single new field `embedder: Arc<once_cell::sync::OnceCell<Arc<dyn Embedder>>>` (lock-free reads, set once). It starts empty. The Tauri `setup` hook spawns one background task that runs the download/verify/load pipeline and then `cell.set(...)`s the embedder. The same task forwards live progress to the frontend via `app.emit("embedder-download", StreamEvent::EmbedderDownload { ... })` — a NEW codebase pattern, deliberately introduced here. `FastEmbedLocal` confines the non-`Send` `fastembed::TextEmbedding` to a dedicated owning thread accessed through a `tokio::sync::mpsc` request/oneshot channel, so the public type is `Send + Sync` and verifiable at compile time.

---

## 2. Module layout

```
src-tauri/src/
├── memory/
│   ├── mod.rs                    MODIFIED  re-export embedder + model_download
│   ├── embedder.rs               NEW       trait Embedder + EmbedError + FastEmbedLocal
│   │                                       + OllamaEmbedder/OpenAIEmbedder stubs
│   │                                       + FakeEmbedder (#[cfg(test)])
│   │                                       + compile-time Send+Sync assertion
│   └── model_download.rs         NEW       ensure_model + DownloadProgress + DownloadError
│                                           + SHA-256 constants + retry loop
├── commands/
│   ├── mod.rs                    MODIFIED  pub mod embedder;
│   └── embedder.rs               NEW       retry_embedder_download Tauri command
├── models/chat.rs                MODIFIED  StreamEvent::EmbedderDownload variant
├── state.rs                      MODIFIED  embedder field + seed in test ctors
├── lib.rs                        MODIFIED  setup hook spawns bootstrap task,
│                                           registers retry_embedder_download
└── Cargo.toml                    MODIFIED  + fastembed, fs2 (deps); + mockito (dev)

src/app/
├── models/chat.model.ts          MODIFIED  StreamEvent union extended
├── services/
│   ├── config.service.ts         (unchanged — reference for pattern)
│   └── embedder.service.ts       NEW       signal-based status + listen() + retry()
└── components/config-panel/
    ├── config-panel.component.ts  MODIFIED  inject(EmbedderService)
    └── config-panel.component.html MODIFIED + new "Embedder" section
```

---

## 3. Threading & lifecycle

This is the highest-risk area (Risk #6 from explore — `fastembed::TextEmbedding` is not guaranteed `Send`). The mitigation is architectural: the model lives on one owning thread for its entire life. All callers talk to it through a channel.

```text
┌─────────────────────────────────────────────────────────────────┐
│ Tauri setup hook (sync closure)                                 │
│   tauri::async_runtime::spawn(async move {                      │
│       ensure_model(&base_dir, Some(progress_tx)).await?;        │
│       let local = spawn_blocking(|| FastEmbedLocal::from_bytes  │
│                       (onnx, tok)).await??;                     │
│       embedder_cell.set(Arc::new(local) as Arc<dyn Embedder>);  │
│       app.emit("embedder-download", Done event);                │
│   });                                                           │
└─────────────────────────────────────────────────────────────────┘
                              │  (set once)
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ AppState.embedder: Arc<OnceCell<Arc<dyn Embedder>>>             │
└─────────────────────────────────────────────────────────────────┘
                              │  (any caller, any task)
                              ▼
        ┌─────────────────────────────────────┐
        │ Embedder::embed("texto").await      │
        │   → FastEmbedLocal.req_tx.send(     │
        │       (text, oneshot_tx))           │
        │   → resp_rx.await                   │
        └─────────────────────────────────────┘
                              │
                              ▼  std::sync::mpsc-like via tokio::mpsc
┌─────────────────────────────────────────────────────────────────┐
│ Dedicated std::thread "fastembed-worker"                        │
│   owns TextEmbedding (no Send required)                         │
│   loop { req = block_on(req_rx.recv()); resp.send(embed(req)); }│
└─────────────────────────────────────────────────────────────────┘
```

### 3.1 Why a dedicated thread (not `spawn_blocking` per call)

Two reasons:

1. **Send safety, regardless of `TextEmbedding`'s bounds.** `tokio::task::spawn_blocking` requires the closure to be `Send` only at submission. Pinning the model behind a `Mutex<TextEmbedding>` would force the mutex guard to be `Send` (it is) but each call still needs to obtain the model — meaning the model crosses tasks. By keeping `TextEmbedding` permanently on one thread that nobody else can reach, the type-system question goes away entirely.
2. **Predictable latency.** `spawn_blocking` uses Tokio's blocking pool, where each call may land on a different thread; with the worker thread approach, the model warms up exactly once and stays warm.

The public-facing struct holds only `Sender`s and primitive metadata — provably `Send + Sync`.

### 3.2 `FastEmbedLocal` shape

```rust
// memory/embedder.rs

use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use fastembed::{TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel, InitOptionsUserDefined};

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError>;
    fn dimensions(&self) -> usize;
    fn model_id(&self) -> &str;
}

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("embedder worker is gone")]
    WorkerGone,
    #[error("model load failed: {0}")]
    LoadFailed(String),
    #[error("inference failed: {0}")]
    InferenceFailed(String),
    #[error("embedder not ready")]
    NotReady,
}

struct EmbedRequest {
    text: String,
    resp: oneshot::Sender<Result<Vec<f32>, EmbedError>>,
}

pub struct FastEmbedLocal {
    req_tx: mpsc::UnboundedSender<EmbedRequest>,
    dimensions: usize,
    model_id: &'static str,
}

impl FastEmbedLocal {
    /// Construct from already-downloaded bytes. CPU-bound (~100 ms);
    /// call sites should wrap in `tokio::task::spawn_blocking`.
    pub fn from_bytes(onnx: Vec<u8>, tokenizer: TokenizerFiles) -> Result<Self, EmbedError> {
        let (req_tx, mut req_rx) = mpsc::unbounded_channel::<EmbedRequest>();

        let user_model = UserDefinedEmbeddingModel::new(onnx, tokenizer);
        let init_opts = InitOptionsUserDefined::new();

        // Load on the calling thread first — surface load errors synchronously.
        let model = TextEmbedding::try_new_from_user_defined(user_model, init_opts)
            .map_err(|e| EmbedError::LoadFailed(e.to_string()))?;

        std::thread::Builder::new()
            .name("fastembed-worker".into())
            .spawn(move || {
                // futures::executor::block_on bridges tokio mpsc into a non-tokio thread.
                while let Some(EmbedRequest { text, resp }) =
                    futures::executor::block_on(req_rx.recv())
                {
                    let out = model
                        .embed(vec![text.as_str()], None)
                        .map(|mut v| v.pop().unwrap_or_default())
                        .map_err(|e| EmbedError::InferenceFailed(e.to_string()));
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

#[async_trait]
impl Embedder for FastEmbedLocal {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.req_tx
            .send(EmbedRequest { text: text.to_string(), resp: resp_tx })
            .map_err(|_| EmbedError::WorkerGone)?;
        resp_rx.await.map_err(|_| EmbedError::WorkerGone)?
    }
    fn dimensions(&self) -> usize { self.dimensions }
    fn model_id(&self) -> &str { self.model_id }
}

// Compile-time regression guard. Acceptance criterion #9.
const _: fn() = || {
    fn _assert_send<T: Send>() {}
    fn _assert_sync<T: Sync>() {}
    _assert_send::<FastEmbedLocal>();
    _assert_sync::<FastEmbedLocal>();
    _assert_send::<Arc<dyn Embedder>>();
    _assert_sync::<Arc<dyn Embedder>>();
};
```

### 3.3 Stubs (no feature gates)

```rust
pub struct OllamaEmbedder;
#[async_trait]
impl Embedder for OllamaEmbedder {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedError> {
        unimplemented!("OllamaEmbedder is a Phase X feature")
    }
    fn dimensions(&self) -> usize { 384 }
    fn model_id(&self) -> &str { "ollama-placeholder" }
}

pub struct OpenAIEmbedder;
#[async_trait]
impl Embedder for OpenAIEmbedder {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedError> {
        unimplemented!("OpenAIEmbedder is a Phase X feature")
    }
    fn dimensions(&self) -> usize { 1536 }
    fn model_id(&self) -> &str { "openai-placeholder" }
}
```

### 3.4 `FakeEmbedder` (test-only)

```rust
#[cfg(test)]
pub(crate) struct FakeEmbedder {
    dim: usize,
}

#[cfg(test)]
impl Default for FakeEmbedder {
    fn default() -> Self { Self { dim: 384 } }
}

#[cfg(test)]
#[async_trait]
impl Embedder for FakeEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        // Deterministic: SipHash the text, seed a tiny PRNG, fill `dim` floats in [-1, 1].
        use std::hash::{Hash, Hasher};
        let mut h = siphasher::sip::SipHasher::new();
        text.hash(&mut h);
        let mut seed = h.finish();
        let mut out = Vec::with_capacity(self.dim);
        for _ in 0..self.dim {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let f = ((seed >> 33) as f32 / u32::MAX as f32) * 2.0 - 1.0;
            out.push(f);
        }
        Ok(out)
    }
    fn dimensions(&self) -> usize { self.dim }
    fn model_id(&self) -> &str { "fake-embedder" }
}
```

`siphasher` is already in deps (Cargo.toml:49, Phase 1) — reused for free.

---

## 4. Download module (`model_download.rs`)

### 4.1 Public API

```rust
use std::path::Path;
use tokio::sync::mpsc;
use fastembed::TokenizerFiles;

pub struct ModelFiles {
    pub onnx: Vec<u8>,
    pub tokenizer: TokenizerFiles,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum DownloadProgress {
    Start { bytes_total: u64 },
    Chunk { bytes_downloaded: u64, bytes_total: u64 },
    Verify,
    Done,
    Error { message: String },
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("insufficient disk space: {free_mb} MB free, {required_mb} MB required")]
    InsufficientDisk { free_mb: u64, required_mb: u64 },
    #[error("http error for {file}: {source}")]
    Http { file: String, #[source] source: reqwest::Error },
    #[error("sha-256 mismatch for {file}: expected {expected}, got {actual}")]
    ChecksumMismatch { file: String, expected: String, actual: String },
    #[error("retry budget exhausted for {file} after {attempts} attempts")]
    RetryExhausted { file: String, attempts: u32 },
    #[error("io error for {file}: {source}")]
    Io { file: String, #[source] source: std::io::Error },
}

pub async fn ensure_model(
    base_dir: &Path,
    progress: Option<mpsc::UnboundedSender<DownloadProgress>>,
) -> Result<ModelFiles, DownloadError>;
```

### 4.2 SHA-256 constants

```rust
// Authoritative source: HF LFS metadata. Cross-checked via:
//   curl -sI <resolve-url> | grep -i '^x-linked-etag:'
//   curl -s <tree-api>/<path> | jq '.lfs.oid'
//
// Recorded 2026-05-23 against commit 614241f622f53c4eeff9890bdc4f31cfecc418b3.

const MODEL_BASE_URL: &str =
    "https://huggingface.co/intfloat/multilingual-e5-small/resolve/main";

const MIN_FREE_BYTES: u64 = 600 * 1024 * 1024; // 600 MB pre-flight budget

// (relative_path, sha256, kind)
// `kind` switches the in-memory destination (onnx vs tokenizer field).
struct FileSpec {
    path: &'static str,
    sha256: &'static str,
}

const ONNX_FILE: FileSpec = FileSpec {
    path: "onnx/model.onnx",
    sha256: "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665",
};

const TOKENIZER_JSON: FileSpec = FileSpec {
    path: "tokenizer.json",
    sha256: "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
};

// The 3 small JSON files are NOT LFS — HF API only exposes git SHA-1 (blob hash).
// Apply phase MUST: download once, compute SHA-256 locally, paste below.
// Their git blob SHA-1s (from explore.md) are: 698e598c..., e0b1d18e..., 059214673d9d...
const CONFIG_JSON: FileSpec = FileSpec {
    path: "config.json",
    sha256: "TODO_APPLY_PHASE_COMPUTE_SHA256_~653_bytes",
};
const SPECIAL_TOKENS_MAP: FileSpec = FileSpec {
    path: "special_tokens_map.json",
    sha256: "TODO_APPLY_PHASE_COMPUTE_SHA256_~167_bytes",
};
const TOKENIZER_CONFIG: FileSpec = FileSpec {
    path: "tokenizer_config.json",
    sha256: "TODO_APPLY_PHASE_COMPUTE_SHA256_~443_bytes",
};

const ALL_FILES: [FileSpec; 5] = [
    ONNX_FILE, TOKENIZER_JSON, CONFIG_JSON, SPECIAL_TOKENS_MAP, TOKENIZER_CONFIG,
];
```

The 3 TODO constants are explicitly part of an apply-phase task (see proposal Risk #8, design §9). The task downloads the 3 files once (~1.3 KB total), runs `sha256sum`, pastes the value, and the constants compile.

### 4.3 Algorithm (prose)

`ensure_model(base_dir, progress)`:

1. Compute `cache_dir = base_dir.join("models").join("multilingual-e5-small")`. Create it (mkdir -p) on entry.
2. **Cache hit fast path**: for each of the 5 files, if file exists on disk AND `sha256(bytes) == spec.sha256`, collect bytes. If all 5 succeed → return `ModelFiles { onnx, tokenizer }` without emitting any event.
3. **Pre-flight disk check**: `fs2::available_space(cache_dir.parent())`. If `< MIN_FREE_BYTES`, emit `Error { message: ... }` and return `DownloadError::InsufficientDisk`. No HTTP traffic yet.
4. Emit `Start { bytes_total: sum_of_file_sizes_known }`. (For files already cached & SHA-valid, do not re-download — but still account their bytes in `bytes_total` so the progress bar starts at the correct mid-point.)
5. For each missing/mismatched file: call `download_one(spec, cache_dir, progress)` (see §4.4).
6. When all 5 are committed: emit `Verify` (one event after all SHAs verified in step 5), then `Done`. Read final bytes from disk into `ModelFiles`.

### 4.4 `download_one(spec, cache_dir, progress) -> Result<(), DownloadError>`

Per-file retry loop:

```rust
const RETRY_BACKOFF_SECS: [u64; 3] = [5, 30, 120]; // 3 attempts total
```

For attempt index `i in 0..3`:

1. Sleep `Duration::from_secs(RETRY_BACKOFF_SECS[i])` ONLY if `i > 0`. (First attempt is immediate.)
2. Open `{cache_dir}/{spec.path}.partial` for writing. If it exists from a previous failed run, truncate it. **No partial-resume.**
3. `reqwest::get(format!("{MODEL_BASE_URL}/{}", spec.path)).send().await`.
4. Capture `Content-Length`. Stream the body via `resp.bytes_stream()`. After each chunk:
   - Write to `.partial`.
   - Update a running counter.
   - Emit `Chunk { bytes_downloaded, bytes_total }`. Throttle to one event every 64 KB to avoid spamming the channel.
5. After full stream: flush, `sync_all()`.
6. Compute SHA-256 of `.partial` contents. If `!= spec.sha256` → delete `.partial`, return `DownloadError::ChecksumMismatch` IMMEDIATELY (do NOT retry — likely tampering or persistent corruption; retry would just refail).
7. `rename(.partial → final)` (atomic on the same filesystem).
8. Return `Ok(())`.

On HTTP error or IO error in steps 3–5: log, delete `.partial`, continue retry loop.

After 3 attempts exhausted: return `DownloadError::RetryExhausted { file: spec.path, attempts: 3 }`.

### 4.5 Disk pre-flight crate

**Proposed**: `fs2 = "0.4"` (single function `available_space(P: AsRef<Path>) -> io::Result<u64>`, ~50 KB, MIT, cross-platform `statvfs`/`GetDiskFreeSpaceExW`).

**Proposal said**: `sysinfo` (decision #14). `sysinfo` is ~1 MB+ with disk/process/network modules; we'd consume <1% of its surface. `fs2` is the surgical fit. **Flagged as contradiction in §10 — orchestrator decides.**

If the orchestrator keeps `sysinfo`, the call site becomes:

```rust
use sysinfo::{Disks, DiskRefreshKind};
let mut disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing().with_storage());
disks.refresh(true);
let free = disks
    .iter()
    .filter(|d| cache_dir.starts_with(d.mount_point()))
    .max_by_key(|d| d.mount_point().as_os_str().len()) // longest-prefix match
    .map(|d| d.available_space())
    .unwrap_or(u64::MAX); // fail-open if we can't identify the mount
```

The rest of the design is identical.

---

## 5. Event delivery (`app.emit` ↔ Angular `listen`)

### 5.1 Backend — setup hook (`src-tauri/src/lib.rs`)

The setup hook stays small. All real work lives in `start_embedder_bootstrap(app_handle)` for testability and to keep `run()` readable.

```rust
// lib.rs
use tauri::Manager;
use crate::memory::embedder::{Embedder, FastEmbedLocal};
use crate::memory::model_download::{ensure_model, DownloadProgress};
use crate::models::chat::StreamEvent;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            start_embedder_bootstrap(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // ... existing 30+ commands ...
            commands::embedder::retry_embedder_download,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn start_embedder_bootstrap(app_handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app_handle.state::<AppState>();
        let base_dir = state.base_dir.clone();
        let embedder_cell = state.embedder.clone();

        // If already filled (e.g. retry after success — defensive), skip.
        if embedder_cell.get().is_some() {
            return;
        }

        // Spawn forwarder: DownloadProgress -> StreamEvent -> app.emit
        // Same pattern as src-tauri/src/commands/chat.rs:855-897 (forwarder task).
        let (progress_tx, mut progress_rx) =
            tokio::sync::mpsc::unbounded_channel::<DownloadProgress>();
        let forward_handle = app_handle.clone();
        tokio::spawn(async move {
            while let Some(p) = progress_rx.recv().await {
                let event = progress_to_stream_event(p);
                let _ = forward_handle.emit("embedder-download", &event);
            }
        });

        // Run the download/load pipeline; emit final state.
        let final_event = match ensure_model(&base_dir, Some(progress_tx)).await {
            Err(e) => StreamEvent::EmbedderDownload {
                phase: "error".into(),
                bytes_downloaded: None,
                bytes_total: None,
                error: Some(e.to_string()),
            },
            Ok(files) => {
                // Model load is CPU-bound; hand it to a blocking thread.
                let load = tokio::task::spawn_blocking(move || {
                    FastEmbedLocal::from_bytes(files.onnx, files.tokenizer)
                })
                .await;

                match load {
                    Ok(Ok(local)) => {
                        let arc: Arc<dyn Embedder> = Arc::new(local);
                        let _ = embedder_cell.set(arc);
                        StreamEvent::EmbedderDownload {
                            phase: "done".into(),
                            bytes_downloaded: None,
                            bytes_total: None,
                            error: None,
                        }
                    }
                    Ok(Err(e)) => StreamEvent::EmbedderDownload {
                        phase: "error".into(),
                        bytes_downloaded: None, bytes_total: None,
                        error: Some(e.to_string()),
                    },
                    Err(join_err) => StreamEvent::EmbedderDownload {
                        phase: "error".into(),
                        bytes_downloaded: None, bytes_total: None,
                        error: Some(format!("model load task panicked: {join_err}")),
                    },
                }
            }
        };

        let _ = app_handle.emit("embedder-download", &final_event);
    });
}

fn progress_to_stream_event(p: DownloadProgress) -> StreamEvent {
    match p {
        DownloadProgress::Start { bytes_total } => StreamEvent::EmbedderDownload {
            phase: "start".into(),
            bytes_downloaded: Some(0),
            bytes_total: Some(bytes_total),
            error: None,
        },
        DownloadProgress::Chunk { bytes_downloaded, bytes_total } => {
            StreamEvent::EmbedderDownload {
                phase: "downloading".into(),
                bytes_downloaded: Some(bytes_downloaded),
                bytes_total: Some(bytes_total),
                error: None,
            }
        }
        DownloadProgress::Verify => StreamEvent::EmbedderDownload {
            phase: "verifying".into(),
            bytes_downloaded: None, bytes_total: None, error: None,
        },
        DownloadProgress::Done => StreamEvent::EmbedderDownload {
            phase: "done".into(),
            bytes_downloaded: None, bytes_total: None, error: None,
        },
        DownloadProgress::Error { message } => StreamEvent::EmbedderDownload {
            phase: "error".into(),
            bytes_downloaded: None, bytes_total: None, error: Some(message),
        },
    }
}
```

### 5.2 Backend — retry command (`src-tauri/src/commands/embedder.rs`)

```rust
use tauri::{AppHandle, State};
use crate::state::AppState;

/// Re-runs the embedder bootstrap pipeline.
/// Idempotent: if the embedder is already set, returns immediately.
#[tauri::command]
pub async fn retry_embedder_download(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if state.embedder.get().is_some() {
        return Ok(()); // Already loaded — nothing to do.
    }
    crate::start_embedder_bootstrap(app);
    Ok(())
}
```

Note: `start_embedder_bootstrap` must be `pub(crate)` so this command can call it.

### 5.3 Frontend — `embedder.service.ts`

```typescript
import { Injectable, signal, OnDestroy } from '@angular/core';
import type { StreamEvent } from '../models/chat.model';

export interface EmbedderStatus {
  phase: 'idle' | 'start' | 'downloading' | 'verifying' | 'done' | 'error';
  bytesDownloaded?: number;
  bytesTotal?: number;
  error?: string;
}

function isTauriEnv(): boolean {
  return '__TAURI__' in window || '__TAURI_INTERNALS__' in window;
}

@Injectable({ providedIn: 'root' })
export class EmbedderService implements OnDestroy {
  readonly status = signal<EmbedderStatus>({ phase: 'idle' });
  private unlisten?: () => void;

  constructor() {
    this.bootstrap();
  }

  private async bootstrap(): Promise<void> {
    if (!isTauriEnv()) return;
    const { listen } = await import('@tauri-apps/api/event');
    this.unlisten = await listen<StreamEvent>('embedder-download', (e) => {
      const ev = e.payload;
      if (ev.kind !== 'embedder_download') return;
      this.status.set({
        phase: ev.phase,
        bytesDownloaded: ev.bytes_downloaded,
        bytesTotal: ev.bytes_total,
        error: ev.error,
      });
    });
  }

  async retry(): Promise<void> {
    if (!isTauriEnv()) return;
    this.status.set({ phase: 'start' });
    const { invoke } = await import('@tauri-apps/api/core');
    try {
      await invoke('retry_embedder_download');
    } catch (err) {
      this.status.set({ phase: 'error', error: String(err) });
    }
  }

  ngOnDestroy(): void {
    this.unlisten?.();
  }
}
```

### 5.4 Frontend — `chat.model.ts` (StreamEvent extension)

Add one discriminant to the union (current shape verified at lines 56-66):

```typescript
| {
    kind: 'embedder_download';
    phase: 'start' | 'downloading' | 'verifying' | 'done' | 'error';
    bytes_downloaded?: number;
    bytes_total?: number;
    error?: string;
  }
```

### 5.5 Frontend — ConfigPanel additions

`config-panel.component.ts`: one new line.

```typescript
readonly embedderService = inject(EmbedderService);
```

`config-panel.component.html`: one new `<section>` appended. Native `@switch`, OnPush-friendly (signal access in template is reactive).

```html
<section class="config-section">
  <h3>Embedder</h3>
  @switch (embedderService.status().phase) {
    @case ('idle') {
      <p>Embedder not yet started.</p>
    }
    @case ('start') {
      <p>Preparing download...</p>
    }
    @case ('downloading') {
      <label for="embedder-progress">
        Downloading model
        ({{ embedderService.status().bytesDownloaded | number }} /
         {{ embedderService.status().bytesTotal | number }} bytes)
      </label>
      <progress
        id="embedder-progress"
        role="progressbar"
        [attr.aria-valuenow]="embedderService.status().bytesDownloaded ?? 0"
        [attr.aria-valuemin]="0"
        [attr.aria-valuemax]="embedderService.status().bytesTotal ?? 0"
        [attr.max]="embedderService.status().bytesTotal ?? 0"
        [attr.value]="embedderService.status().bytesDownloaded ?? 0">
      </progress>
    }
    @case ('verifying') {
      <p aria-live="polite">Verifying model integrity...</p>
    }
    @case ('done') {
      <p class="status-ok">Embedder ready: multilingual-e5-small (384 dims)</p>
    }
    @case ('error') {
      <p class="status-error" role="alert">
        Embedder failed: {{ embedderService.status().error }}
      </p>
      <button class="btn btn-secondary" (click)="embedderService.retry()">
        Retry
      </button>
    }
  }
</section>
```

ARIA: `role="progressbar"` + `aria-valuenow`/`aria-valuemin`/`aria-valuemax` satisfy WCAG SC 4.1.2. `aria-live="polite"` on the verifying state. `role="alert"` on errors. Native `<progress>` already exposes percentage to screen readers when `value`/`max` are set.

### 5.6 Pattern precedent (NEW in this codebase)

`app.emit` + `listen` is introduced here for the first time. Going forward:

| Use case | Mechanism |
|----------|-----------|
| Request-scoped streaming tied to one invoke call (chat tokens, tool progress) | Per-invoke `tauri::ipc::Channel<T>` passed as the `onEvent` arg. Pattern: `src-tauri/src/commands/chat.rs:855-897` + `src/app/services/chat.service.ts:114-160`. |
| App-wide lifecycle events that begin BEFORE any invoke call (embedder bootstrap, future re-indexing, future model-swap notifications) | `app.emit("event-name", payload)` + Angular `listen()`. Pattern: this design. |

Recommend a follow-up issue: document this rule in `.claude/CLAUDE.md`. **Not** Phase 2 scope to edit CLAUDE.md.

---

## 6. `AppState` changes

### 6.1 New field

```rust
// state.rs — added inside AppState struct
/// Embedder filled lazily by the setup-hook background task.
/// Lock-free reads; set-once; cannot hot-swap (Phase X concern).
pub embedder: Arc<once_cell::sync::OnceCell<Arc<dyn Embedder>>>,
```

Imports needed at top of `state.rs`:

```rust
use once_cell::sync::OnceCell;
use crate::memory::embedder::Embedder;
```

(`once_cell = "1"` already in Cargo.toml line 42 — no new dep.)

### 6.2 Constructors

| Ctor | Embedder initialization | Why |
|------|-------------------------|-----|
| `new()` | `Arc::new(OnceCell::new())` (empty) | Production. Filled by setup-hook task once download/load completes. |
| `new_for_test()` (sync, `#[cfg(test)]`) | `let cell = OnceCell::new(); cell.set(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>).ok(); Arc::new(cell)` | Tests that need a synchronous embed call without spinning up the worker thread. Pre-seeded. |
| `new_for_test_async()` (async, `#[cfg(test)]`) | Same as `new_for_test()` — pre-seed with `FakeEmbedder` | Embedder is sync-constructible (it's a fake), so the async ctor doesn't need to await. The async-ness exists for `MemoryStore`, not for the embedder. |

Pre-seeding in BOTH test helpers eliminates Risk #5 (tests racing the bootstrap task).

---

## 7. `memory/mod.rs` change

Append two `pub mod` lines:

```rust
pub mod embedder;
pub mod model_download;
```

No re-exports needed at this layer — call sites use the fully-qualified path
`crate::memory::embedder::Embedder` etc. (matches how `MemoryStore` is referenced from `state.rs`).

---

## 8. Testing strategy

| Layer | File | Tests | Mock/double |
|-------|------|-------|-------------|
| Trait + FakeEmbedder | `embedder.rs::tests` | `fake_returns_384_dims`, `fake_is_deterministic` (same input → bit-identical output), `fake_different_inputs_differ`, `compile_time_send_sync_assertion` (the `const _: fn() = ...` block IS the test — it's a compile error if it ever regresses) | inline `FakeEmbedder` |
| FastEmbedLocal worker plumbing | `embedder.rs::tests` | `worker_thread_send_recv` — channel send → mock worker echoes → oneshot resolves. Does NOT load the real model. | mock std::thread that returns canned `Vec<f32>` |
| FastEmbedLocal real model load | `embedder.rs::tests` with `#[ignore]` | `from_bytes_with_real_model_loads_and_embeds` — only runs with `cargo test -- --ignored` AND requires the model on disk. CI never runs this. | none (real fastembed) |
| model_download cached skip | `model_download.rs::tests` | `all_files_cached_skips_http`: pre-write all 5 files with correct SHA in a tempdir, run `ensure_model`, assert mockito received ZERO requests | `mockito` server (asserts 0 hits) |
| model_download SHA mismatch | `model_download.rs::tests` | `sha_mismatch_returns_error`: mockito serves bytes that don't match constant → `DownloadError::ChecksumMismatch`, `.partial` is deleted | `mockito` |
| model_download retry success | `model_download.rs::tests` | `retry_succeeds_on_attempt_two`: mockito returns 500 once then valid bytes → final result Ok; verify only 2 attempts made | `mockito` + `mock.expect(2)` |
| model_download retry exhaustion | `model_download.rs::tests` | `retry_exhausted_returns_error`: mockito returns 500 three times → `DownloadError::RetryExhausted { attempts: 3 }` | `mockito` |
| model_download disk pre-flight | `model_download.rs::tests` | `insufficient_disk_returns_error_without_http`: inject mock free-space oracle returning 100 MB → `DownloadError::InsufficientDisk`, mockito gets 0 hits | trait-injected `DiskOracle` or feature-flag the call |
| StreamEvent serde | `models/chat.rs::tests` (extend existing) | `embedder_download_round_trip`: serialize variant → assert exact JSON shape matches Angular union (snake_case `kind: 'embedder_download'`, optional fields omitted when `None`) | none |
| AppState embedder field | `state.rs::tests` (extend existing) | `new_for_test_async_seeds_embedder`: `state.embedder.get()` is Some, call `embed()`, get 384 floats. `new_for_test_seeds_embedder`: same, sync. `production_new_starts_empty`: skipped in CI (no Tauri runtime), documented as smoke. | `FakeEmbedder` |
| Setup hook | manual smoke (no automated test) | Run app, observe console + UI | — |
| Angular service | manual smoke (Tauri devtools) | Open app, watch signal updates | — |

**Disk-oracle injection for the pre-flight test**: extract a tiny trait:

```rust
#[cfg_attr(test, mockall::automock)]
trait DiskOracle { fn available_bytes(&self, path: &Path) -> std::io::Result<u64>; }
struct Fs2Oracle;
impl DiskOracle for Fs2Oracle { /* calls fs2::available_space */ }
```

Default `ensure_model` uses `Fs2Oracle`. Tests pass a mock via a test-only `ensure_model_with_oracle` constructor. If `mockall` is not desired, write a hand-rolled mock — same shape.

### Backoff in tests

`RETRY_BACKOFF_SECS` is `const`. Tests use `tokio::time::pause()` + `tokio::time::advance()` (already enabled via `tokio = { ..., features = ["test-util"] }` in dev-deps line 60) to skip the 5/30/120-second waits. No real sleeps in CI.

---

## 9. Dependencies summary

```toml
# [dependencies] — add after Phase 1 RAG block
# RAG Phase 2 (embedder)
fastembed   = { version = "5", default-features = false, features = ["ort-download-binaries"] }   # corrected during apply CHECKPOINT 1 — `-rustls-tls` suffix does NOT exist on fastembed v5
fs2         = "0.4"   # disk pre-flight; see §10 — proposal locks `sysinfo` instead

# [dev-dependencies] — add
mockito     = "1"
mockall     = "0.13"  # OPTIONAL — only if disk-oracle uses `#[automock]`; otherwise drop
```

Already present (no version change): `sha2`, `reqwest` (with `stream`), `tokio` (+ `test-util` in dev), `async-trait`, `dirs`, `once_cell`, `serde`, `serde_json`, `siphasher`, `thiserror`.

Transitive (informational): `ort 2.0.0-rc.12` (fastembed pins exact), `tokenizers 0.22.2` (with `onig` — first Windows build takes 5-10 min), `ndarray 0.17.2`, `safetensors 0.7.0`.

---

## 10. Open issues to surface before apply

1. **`fs2` vs `sysinfo`** — proposal decision #14 names `sysinfo`. Design recommends `fs2` because its surface (one function, `~50 KB`) matches the need exactly; `sysinfo` is ~20x larger and we'd touch one method of one module. **Both work**. Orchestrator MUST choose before `tasks.md` is written so the dep line is correct on first commit.
2. **3 small JSON SHA-256 constants** — TBD during apply, computed by hashing the files once after first download. Captured as a task in §8 testing and proposal Risk #8.
3. **`mockall` dev-dep** — only needed if the disk-oracle uses `#[automock]`. Hand-rolled mocks are equally valid. Orchestrator chooses; tasks.md reflects.

Everything else is locked.

---

## 11. Rollback plan

If `cargo check` (Phase 2's verification gate; `cargo build` is forbidden per project CLAUDE.md) reveals that `fastembed` or transitive `ort` cannot resolve on Windows:

1. **Revert `Cargo.toml`** — remove `fastembed` + `fs2` (+ `mockito`/`mockall` from dev-deps).
2. The new modules `memory/embedder.rs`, `memory/model_download.rs`, `commands/embedder.rs` are self-contained — comment out the `pub mod` lines in `memory/mod.rs` and the `pub mod embedder;` in `commands/mod.rs`. No other file references them directly (everything goes through `dyn Embedder`).
3. `AppState.embedder` field can stay as `Arc<OnceCell<Arc<dyn Embedder>>>` — empty `OnceCell` is the production default. Disable `start_embedder_bootstrap` call in `lib.rs::run()` setup hook.
4. `StreamEvent::EmbedderDownload` variant is additive — leaves no consumer dangling.
5. Frontend `EmbedderService` is additive; `ConfigPanel` section is additive. No regression in existing flows.

Cost of rollback: ~5 minutes of edits, zero schema migration, zero data loss.
