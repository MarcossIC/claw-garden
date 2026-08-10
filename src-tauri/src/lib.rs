mod commands;
mod compaction;
mod config;
mod hooks;
mod mcp;
mod memory;
mod models;
mod tools;
mod providers;
mod recovery;
mod services;
mod session;
mod state;

/// Narrow, feature-gated door into retrieval internals for the evaluation
/// binary. `memory` itself stays a private module — see `eval_api.rs` for
/// why an opaque wrapper is preferred over widening the module tree.
#[cfg(feature = "eval")]
pub mod eval_api;

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};

use memory::db::ContractStatus;
use memory::embedder::{Embedder, FastEmbedLocal};
use memory::model_download::{ensure_model, DownloadProgress};
use models::chat::StreamEvent;
use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (state, memory_rx) = AppState::new();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .setup(move |app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            // Spawn the embedder bootstrap task. It downloads the model in the
            // background and forwards progress events via
            // `app.emit("embedder-download", …)`. Non-blocking — failure
            // leaves the OnceCell empty and the rest of the app keeps running.
            start_embedder_bootstrap(app.handle().clone());

            // Spawn the memory ingest worker. Receives MemoryEvents from
            // push_and_persist and writes rows to memory + memory_vectors.
            // memory_rx is moved here and consumed by the worker task for the
            // app lifetime.
            crate::memory::worker::start_memory_worker(app.handle().clone(), memory_rx);

            // Access tracker lifecycle hook. The flush worker is already
            // running (spawned inside AccessTracker::new() during
            // AppState::new()). This call exists for lifecycle parity with
            // start_memory_worker and is the extension point for a future
            // supervision/reconnect mechanism, should the flush worker ever
            // need one.
            start_access_worker(app.handle().clone());

            // memoria-3-capas-fase-3: spawn the maintenance job (periodic
            // inactivity-based tier demotion). First pass fires 5 minutes
            // after startup, then every 24h — see `memory::maintenance`.
            crate::memory::maintenance::start_maintenance_worker(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::agents::get_agents,
            commands::chat::stream_chat,
            commands::config::set_project_path,
            commands::config::get_project_path,
            commands::config::get_config,
            commands::config::set_provider,
            commands::config::check_ollama,
            commands::config::openrouter_list_models,
            commands::config::get_auth_status,
            commands::auth::oauth_status,
            commands::auth::oauth_login,
            commands::auth::oauth_logout,
            commands::auth::oauth_refresh,
            commands::providers::list_providers,
            commands::providers::add_provider,
            commands::providers::update_provider,
            commands::providers::delete_provider,
            commands::agent_crud::create_agent,
            commands::agent_crud::update_agent,
            commands::agent_crud::delete_agent,
            commands::session::list_sessions,
            commands::session::current_session,
            commands::session::new_session,
            commands::session::resume_session,
            commands::session::delete_session,
            commands::session::get_agent_messages,
            commands::workspace::get_workspace_config,
            commands::workspace::set_workspace_config,
            commands::confirmations::resolve_tool_confirmation,
            commands::embedder::retry_embedder_download,
            commands::memory_admin::list_memories,
            commands::memory_admin::set_importance,
            commands::memory_admin::delete_memory,
            commands::memory_admin::get_skill_steps_trend,
            commands::memory_admin::get_failure_histogram,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ============================================================================
// Access tracker lifecycle hook
// ============================================================================

/// Lifecycle hook called from `run()` after `start_memory_worker`.
///
/// The access tracker flush worker is already running — it is spawned inside
/// [`crate::memory::access::AccessTracker::new`] during `AppState::new()`.
/// This function exists for lifecycle parity with `start_memory_worker` and
/// is the extension point for a future supervision/reconnect mechanism,
/// should the flush worker ever need one.
fn start_access_worker(_app_handle: AppHandle) {
    // No-op today: the worker is already alive for the duration of AppState,
    // spawned by AccessTracker::new() rather than by this hook.
}

// ============================================================================
// Embedder bootstrap orchestration
// ============================================================================

/// Translate a backend [`DownloadProgress`] into the wire-shape
/// [`StreamEvent::EmbedderDownload`] that the frontend listens for.
///
/// Pure function — no I/O, no allocation beyond the small string field. The
/// translation is one-to-one with the lifecycle defined in design §5.1.
pub(crate) fn progress_to_stream_event(p: DownloadProgress) -> StreamEvent {
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
// EmbedderEventSink — abstraction over `app.emit("embedder-download", …)`.
//
// Production wires `AppHandleSink(AppHandle)` to forward each `StreamEvent`
// to Tauri's event bus. Tests inject a `MockSink` that records emissions to
// an in-memory `Vec<StreamEvent>`, which is how the "exactly one terminal
// `done`" assertion (REQ-EVT-002, verify-report WARNING #1) is now covered
// at the bootstrap layer instead of the lower `ensure_model` layer.
// ----------------------------------------------------------------------------

/// Abstraction over `app.emit("embedder-download", …)` so the bootstrap
/// logic is unit-testable without a real Tauri runtime.
pub(crate) trait EmbedderEventSink: Send + Sync + 'static {
    fn emit_event(&self, event: StreamEvent);
}

/// Production sink: forwards to `AppHandle::emit("embedder-download", …)`.
struct AppHandleSink {
    app: AppHandle,
}

impl EmbedderEventSink for AppHandleSink {
    fn emit_event(&self, event: StreamEvent) {
        let _ = self.app.emit("embedder-download", &event);
    }
}

/// Type alias for the injectable `ensure_model` seam.
type EnsureModelFn = Arc<
    dyn Fn(
            std::path::PathBuf,
            Option<tokio::sync::mpsc::UnboundedSender<DownloadProgress>>,
        ) -> futures::future::BoxFuture<
            'static,
            Result<
                memory::model_download::ModelFiles,
                memory::model_download::DownloadError,
            >,
        > + Send
        + Sync,
>;

/// Type alias for the injectable model-load seam (replaces
/// `FastEmbedLocal::from_bytes` in tests).
type LoadEmbedderFn = Arc<
    dyn Fn(memory::model_download::ModelFiles) -> Result<Arc<dyn Embedder>, String>
        + Send
        + Sync,
>;

/// Type alias for the injectable embedding-contract verification seam.
///
/// Owned `(String, usize)` rather than `&dyn Embedder`: a borrow cannot live
/// inside a `'static` future, and the owned pair also makes "compare only
/// model_id + dim" structural in the signature itself instead of a
/// convention callers have to remember.
type ContractVerifyFn = Arc<
    dyn Fn(String, usize) -> futures::future::BoxFuture<'static, Result<ContractStatus, String>>
        + Send
        + Sync,
>;

/// Public entry point used by both the setup hook and the
/// [`commands::embedder::retry_embedder_download`] command. Spawns the
/// embedder bootstrap on `tauri::async_runtime` and returns immediately.
///
/// The task:
/// 1. Short-circuits if the embedder cell is already filled.
/// 2. Spawns a forwarder that drains `DownloadProgress` → `app.emit`.
/// 3. Awaits `ensure_model`, then loads `FastEmbedLocal` in a blocking task.
/// 4. Emits the terminal `done` or `error` event.
///
/// On any error inside the task, the cell stays empty (defensible state — the
/// retry command can re-invoke). Errors are not allowed to propagate to the
/// Tauri runtime and crash the app.
pub(crate) fn start_embedder_bootstrap(app_handle: AppHandle) {
    let state = app_handle.state::<AppState>();
    let base_dir = state.base_dir.clone();
    let embedder_cell = state.embedder.clone();
    let store = state.memory_store.clone();
    let sink = Arc::new(AppHandleSink {
        app: app_handle.clone(),
    });

    let ensure: EnsureModelFn = Arc::new(|base_dir, progress| {
        Box::pin(async move { ensure_model(&base_dir, progress).await })
    });
    let load: LoadEmbedderFn = Arc::new(|files| {
        FastEmbedLocal::from_bytes(files.onnx, files.tokenizer)
            .map(|local| Arc::new(local) as Arc<dyn Embedder>)
            .map_err(|e| e.to_string())
    });
    // `state.memory_store` is the only place that sees both the app's
    // `AppHandle` and the open store, so it is the only place that can build
    // this closure — `run_embedder_bootstrap` itself stays store-agnostic
    // and testable without a `MemoryStore` at all.
    let verify: ContractVerifyFn = Arc::new(move |model_id, dim| {
        let store = store.clone();
        Box::pin(async move {
            store
                .ensure_embedding_contract(&model_id, dim)
                .await
                .map_err(|e| e.to_string())
        })
    });

    tauri::async_runtime::spawn(async move {
        run_embedder_bootstrap(base_dir, embedder_cell, sink, ensure, load, verify).await;
    });
}

/// Inner bootstrap loop, parametric in the event sink + dependency closures.
/// Production wires this to `AppHandleSink` + real `ensure_model` + real
/// `FastEmbedLocal::from_bytes`. Tests wire a `MockSink` + fake closures so
/// the entire lifecycle (including the forwarder → emit pipeline) can be
/// asserted in-process.
pub(crate) async fn run_embedder_bootstrap(
    base_dir: std::path::PathBuf,
    embedder_cell: Arc<once_cell::sync::OnceCell<Arc<dyn Embedder>>>,
    sink: Arc<dyn EmbedderEventSink>,
    ensure: EnsureModelFn,
    load: LoadEmbedderFn,
    verify: ContractVerifyFn,
) {
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
        return;
    }

    let (progress_tx, mut progress_rx) =
        tokio::sync::mpsc::unbounded_channel::<DownloadProgress>();

    // Forwarder task: DownloadProgress → StreamEvent → sink.
    // Lives in its own task so the download future can keep pushing without
    // back-pressure. Terminates naturally when the last `progress_tx` clone
    // is dropped (which happens when `ensure` returns and `progress_tx` goes
    // out of scope) — see verify-report WARNING #1 fix notes (2026-05-23):
    // since `ensure_model` no longer emits `Done`, the forwarder's last
    // forwarded event is `Verify` and there is no race with the synthetic
    // terminal `done` emitted below.
    let forwarder_sink = sink.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(p) = progress_rx.recv().await {
            let event = progress_to_stream_event(p);
            forwarder_sink.emit_event(event);
        }
    });

    let final_event = match ensure(base_dir, Some(progress_tx)).await {
        Err(e) => StreamEvent::EmbedderDownload {
            phase: "error".into(),
            bytes_downloaded: None,
            bytes_total: None,
            error: Some(e.to_string()),
        },
        Ok(files) => {
            // CPU-bound (~100 ms in production): hand off to spawn_blocking so
            // the tokio worker thread isn't stalled. The injected `load`
            // closure in tests returns immediately, but spawn_blocking is fine
            // for both cases.
            let load_fn = load.clone();
            let load_res =
                tokio::task::spawn_blocking(move || load_fn(files)).await;
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
                    let model_id = arc.model_id().to_string();
                    let dim = arc.dimensions();
                    match verify(model_id, dim).await {
                        Ok(ContractStatus::Match) | Ok(ContractStatus::Initialized) => {
                            // `.set` returns Err only if the cell is already filled —
                            // survivable race; treat both outcomes as success.
                            let _ = embedder_cell.set(arc);
                            StreamEvent::EmbedderDownload {
                                phase: "done".into(),
                                bytes_downloaded: None,
                                bytes_total: None,
                                error: None,
                            }
                        }
                        Ok(ContractStatus::Mismatch {
                            stored_model_id,
                            stored_dim,
                            actual_model_id,
                            actual_dim,
                        }) => {
                            // Cell stays empty — the existing fail-soft reads in
                            // worker.rs and prompt_builder.rs already treat an
                            // empty cell as "memory disabled", so this needs no
                            // new degraded-mode logic of its own.
                            log::warn!(
                                "embedding contract mismatch: store holds vectors from \
                                 '{stored_model_id}' (dim {stored_dim}), active embedder is \
                                 '{actual_model_id}' (dim {actual_dim}); memory reads/writes \
                                 are disabled until this is resolved"
                            );
                            StreamEvent::EmbedderDownload {
                                phase: "error".into(),
                                bytes_downloaded: None,
                                bytes_total: None,
                                error: Some(format!(
                                    "embedding contract mismatch: the store has vectors from \
                                     '{stored_model_id}' (dim {stored_dim}), but the active \
                                     embedder is '{actual_model_id}' (dim {actual_dim}); memory \
                                     is disabled until the mismatch is resolved — retrying will \
                                     not fix this"
                                )),
                            }
                        }
                        Err(e) => {
                            // Fail-closed: a contract we could not verify is a
                            // contract we cannot trust, same as an explicit
                            // mismatch. Not permanent — the next app start or
                            // manual retry re-runs the whole bootstrap.
                            log::warn!(
                                "embedding contract verification failed: {e}; treating as a \
                                 mismatch (fail-closed)"
                            );
                            StreamEvent::EmbedderDownload {
                                phase: "error".into(),
                                bytes_downloaded: None,
                                bytes_total: None,
                                error: Some(format!(
                                    "embedding contract verification failed: {e}"
                                )),
                            }
                        }
                    }
                }
                Ok(Err(e)) => StreamEvent::EmbedderDownload {
                    phase: "error".into(),
                    bytes_downloaded: None,
                    bytes_total: None,
                    error: Some(e),
                },
                Err(join_err) => StreamEvent::EmbedderDownload {
                    phase: "error".into(),
                    bytes_downloaded: None,
                    bytes_total: None,
                    error: Some(format!("model load task panicked: {join_err}")),
                },
            }
        }
    };

    // Await the forwarder so every lifecycle event from `ensure_model` has
    // landed in the sink BEFORE we emit our synthetic terminal event. This
    // guarantees the UI sees the events in source order (start → chunk* →
    // verify → done) instead of racing with the spawned task.
    let _ = forwarder.await;

    sink.emit_event(final_event);
}

// ============================================================================
// Embedder bootstrap: progress-translator and lifecycle tests
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
            other => panic!("expected EmbedderDownload, got {other:?}"),
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
            other => panic!("expected EmbedderDownload, got {other:?}"),
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
            other => panic!("expected EmbedderDownload, got {other:?}"),
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
            other => panic!("expected EmbedderDownload, got {other:?}"),
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
            other => panic!("expected EmbedderDownload, got {other:?}"),
        }
    }

    // ---------- start_embedder_bootstrap factoring ----------
    //
    // Tauri's `AppHandle` cannot be constructed without a real Tauri runtime,
    // so the public `start_embedder_bootstrap` is verified by compile-time
    // correctness (the setup hook compiles) PLUS the parametric
    // `run_embedder_bootstrap` which is the unit-test seam: it accepts an
    // injectable `EmbedderEventSink`, `EnsureModelFn`, `LoadEmbedderFn`, and
    // `ContractVerifyFn`, so the full lifecycle (early-exit, happy path,
    // error path, contract mismatch, AND the forwarder→sink pipeline) can be
    // asserted in-process without Tauri.

    use crate::memory::embedder::FakeEmbedder;
    use crate::memory::model_download::{DownloadError, ModelFiles};
    use fastembed::TokenizerFiles;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

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
            onnx: Vec::new(),
            tokenizer: empty_tokenizer(),
        }
    }

    /// Count `phase == "done"` occurrences across recorded `StreamEvent`s.
    fn count_done_phases(events: &[StreamEvent]) -> usize {
        events
            .iter()
            .filter(|e| match e {
                StreamEvent::EmbedderDownload { phase, .. } => phase == "done",
                _ => false,
            })
            .count()
    }

    fn phase_of_event(e: &StreamEvent) -> &str {
        match e {
            StreamEvent::EmbedderDownload { phase, .. } => phase,
            _ => "<not-embedder>",
        }
    }

    /// Test-only `ContractVerifyFn` that always reports a match — the
    /// behavior every lifecycle test except the mismatch test itself wants,
    /// so each one that reaches the seam gets a one-line "accept whatever
    /// arrives" verifier instead of repeating the closure inline.
    fn accept_contract() -> ContractVerifyFn {
        Arc::new(|_model_id, _dim| Box::pin(async { Ok(ContractStatus::Match) }))
    }

    #[tokio::test]
    async fn run_embedder_bootstrap_is_noop_when_cell_filled() {
        // Cell already seeded → bootstrap emits a single synthetic `done`
        // (so the UI exits its loading state) and skips both `ensure` and
        // `load`. We assert: zero `ensure`/`load` calls, exactly one event,
        // and that event is `done`.
        let cell: once_cell::sync::OnceCell<Arc<dyn Embedder>> =
            once_cell::sync::OnceCell::new();
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
        run_embedder_bootstrap(
            std::env::temp_dir(),
            cell,
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
        let cell: Arc<once_cell::sync::OnceCell<Arc<dyn Embedder>>> =
            Arc::new(once_cell::sync::OnceCell::new());

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
        run_embedder_bootstrap(
            std::env::temp_dir(),
            cell.clone(),
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

    /// REQ-EVT-002 compliance (verify-report WARNING #1, 2026-05-23): on the
    /// happy path the UI MUST receive EXACTLY ONE terminal `done` event,
    /// emitted as the LAST event of the lifecycle by the bootstrap (after
    /// `load` + `OnceCell::set`), NOT by `ensure_model`.
    ///
    /// Before the fix the forwarder relayed a `Done` from `ensure_model` AND
    /// the bootstrap emitted its own synthetic `done` → UI saw `done` twice.
    /// This test pumps a realistic lifecycle (`Start → Chunk → Verify`)
    /// through the channel and asserts (a) exactly one `done`, (b) it is the
    /// LAST event, (c) the preceding events are in source order.
    #[tokio::test]
    async fn run_embedder_bootstrap_emits_exactly_one_done_on_happy_path() {
        let cell: Arc<once_cell::sync::OnceCell<Arc<dyn Embedder>>> =
            Arc::new(once_cell::sync::OnceCell::new());

        let ensure: EnsureModelFn = Arc::new(move |_base, progress| {
            Box::pin(async move {
                // Simulate the real `ensure_model` event sequence per the
                // post-fix contract: Start → Chunk → Verify. NO Done — that
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
        let load: LoadEmbedderFn = Arc::new(move |_files| {
            Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>)
        });

        let sink = Arc::new(MockSink::default());
        let sink_for_run: Arc<dyn EmbedderEventSink> = sink.clone();
        run_embedder_bootstrap(
            std::env::temp_dir(),
            cell.clone(),
            sink_for_run,
            ensure,
            load,
            accept_contract(),
        )
        .await;

        let events = sink.snapshot();
        let phases: Vec<&str> = events.iter().map(phase_of_event).collect();

        // (a) EXACTLY one terminal `done` — the core REQ-EVT-002 invariant.
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
    /// terminal `error` event and the cell stays empty. Confirms the chosen
    /// fix did not break the error-path invariant (verify-report mentioned the
    /// error path was already healthy; this is the regression guard).
    #[tokio::test]
    async fn run_embedder_bootstrap_emits_terminal_error_when_ensure_fails() {
        let cell: Arc<once_cell::sync::OnceCell<Arc<dyn Embedder>>> =
            Arc::new(once_cell::sync::OnceCell::new());

        let ensure: EnsureModelFn = Arc::new(move |_base, _progress| {
            Box::pin(async move {
                Err(DownloadError::RetryExhausted {
                    file: "onnx/model.onnx".into(),
                    attempts: 3,
                })
            })
        });
        let load: LoadEmbedderFn = Arc::new(move |_files| {
            Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>)
        });

        let sink = Arc::new(MockSink::default());
        let sink_for_run: Arc<dyn EmbedderEventSink> = sink.clone();
        run_embedder_bootstrap(
            std::env::temp_dir(),
            cell.clone(),
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
        let cell: Arc<once_cell::sync::OnceCell<Arc<dyn Embedder>>> =
            Arc::new(once_cell::sync::OnceCell::new());

        let ensure: EnsureModelFn =
            Arc::new(move |_base, _progress| Box::pin(async move { Ok(empty_model_files()) }));
        let load: LoadEmbedderFn = Arc::new(move |_files| {
            Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>)
        });
        let verify: ContractVerifyFn = Arc::new(|_model_id, _dim| {
            Box::pin(async move {
                Ok(ContractStatus::Mismatch {
                    stored_model_id: "multilingual-e5-small".into(),
                    stored_dim: 384,
                    actual_model_id: "fake-embedder".into(),
                    actual_dim: 384,
                })
            })
        });

        let sink = Arc::new(MockSink::default());
        let sink_for_run: Arc<dyn EmbedderEventSink> = sink.clone();
        run_embedder_bootstrap(
            std::env::temp_dir(),
            cell.clone(),
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
}
