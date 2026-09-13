//! Embedder bootstrap: download the model, load it, verify the embedding
//! contract, and fill the process-wide embedder cell.
//!
//! Lives in `clawg-core` rather than in the desktop crate because there is now
//! more than one host: the desktop app fills its `AppState.embedder` cell from a
//! setup hook, and the `clawg serve` daemon fills the identical cell on
//! `MemoryRuntime` with no UI to report progress to. The logic is the same in
//! both; only the event sink and where the future is spawned differ, and both
//! of those are already parameters here.
//!
//! No desktop types are involved: the wire-shape event is
//! [`StreamEvent::EmbedderDownload`], which has always lived in
//! `clawg_core::models::chat`.
//!
//! # Testing seams
//!
//! [`run_embedder_bootstrap`] takes the model download, the model load and the
//! contract check as injected closures ([`EnsureModelFn`], [`LoadEmbedderFn`],
//! [`ContractVerifyFn`]) so the whole lifecycle — early exit, happy path,
//! download failure, contract mismatch, and the progress-forwarder pipeline —
//! is assertable in-process. [`production_deps`] builds the real three.

use std::sync::Arc;

use once_cell::sync::OnceCell;

use crate::memory::db::{ContractStatus, EmbeddingIdentity};
use crate::memory::embedder::{Embedder, FastEmbedLocal};
use crate::memory::model_download::{ensure_model, DownloadProgress};
use crate::memory::MemoryStore;
use crate::models::chat::StreamEvent;

/// Translate a backend [`DownloadProgress`] into the wire-shape
/// [`StreamEvent::EmbedderDownload`] that the frontend listens for.
///
/// Pure function — no I/O, no allocation beyond the small string field. The
/// translation is one-to-one with the lifecycle defined in design §5.1.
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
// The desktop wires `AppHandleSink(AppHandle)` (which stays in the desktop
// crate — it is the only piece that needs the GUI framework) to forward each
// `StreamEvent` to the desktop event bus. A headless host wires `LogSink`. Tests
// inject a `MockSink` that records emissions to an in-memory
// `Vec<StreamEvent>`, which is how the "exactly one terminal `done`"
// assertion (REQ-EVT-002, verify-report WARNING #1) is covered at the
// bootstrap layer instead of the lower `ensure_model` layer.
// ----------------------------------------------------------------------------

/// Abstraction over the host's "tell the user about the download" transport,
/// so the bootstrap logic is unit-testable without a real desktop runtime.
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
        } = event
        else {
            // The sink is declared over the whole `StreamEvent` enum for the
            // desktop's sake (it forwards onto a bus typed that way); the
            // bootstrap only ever emits `EmbedderDownload`, so anything else
            // is not this sink's business.
            return;
        };
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

/// Type alias for the injectable model-load seam (replaces
/// `FastEmbedLocal::from_bytes` in tests).
pub type LoadEmbedderFn = Arc<
    dyn Fn(crate::memory::model_download::ModelFiles) -> Result<Arc<dyn Embedder>, String>
        + Send
        + Sync,
>;

/// Type alias for the injectable embedding-contract verification seam.
///
/// Owned `EmbeddingIdentity` rather than `&dyn Embedder`: a borrow cannot
/// live inside a `'static` future, and the owned struct also makes "compare
/// model_id + dim + query_prefix + passage_prefix" structural in the
/// signature itself instead of a convention callers have to remember.
pub type ContractVerifyFn = Arc<
    dyn Fn(EmbeddingIdentity) -> futures::future::BoxFuture<'static, Result<ContractStatus, String>>
        + Send
        + Sync,
>;

/// The three real dependencies [`run_embedder_bootstrap`] runs against in
/// production: the HTTP download, the ONNX load, and the store's contract
/// check.
///
/// Exposed as one function because the trio is a unit — every host needs all
/// three, and the only host-specific ingredient is the open [`MemoryStore`]
/// the verifier closes over. Keeping the closures here rather than at each
/// call site means the daemon and the desktop cannot drift into verifying the
/// contract against different stores or loading the model two different ways.
pub fn production_deps(
    store: Arc<MemoryStore>,
) -> (EnsureModelFn, LoadEmbedderFn, ContractVerifyFn) {
    let ensure: EnsureModelFn = Arc::new(|base_dir, progress| {
        Box::pin(async move { ensure_model(&base_dir, progress).await })
    });
    let load: LoadEmbedderFn = Arc::new(|files| {
        FastEmbedLocal::from_bytes(files.onnx, files.tokenizer)
            .map(|local| Arc::new(local) as Arc<dyn Embedder>)
            .map_err(|e| e.to_string())
    });
    // The store is the only host-supplied ingredient — `run_embedder_bootstrap`
    // itself stays store-agnostic and testable without a `MemoryStore` at all.
    let verify: ContractVerifyFn = Arc::new(move |identity| {
        let store = store.clone();
        Box::pin(async move {
            store
                .ensure_embedding_contract(identity)
                .await
                .map_err(|e| e.to_string())
        })
    });
    (ensure, load, verify)
}

/// Inner bootstrap loop, parametric in the event sink + dependency closures.
/// Production wires this to the host's sink plus [`production_deps`]. Tests
/// wire a `MockSink` + fake closures so the entire lifecycle (including the
/// forwarder → emit pipeline) can be asserted in-process.
///
/// On any error the cell stays empty (defensible state — a retry can
/// re-invoke). Errors are never propagated to the caller: the host spawns this
/// fire-and-forget and must not be brought down by a failed download.
pub async fn run_embedder_bootstrap(
    base_dir: std::path::PathBuf,
    embedder_cell: Arc<OnceCell<Arc<dyn Embedder>>>,
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
                    let identity = EmbeddingIdentity::of(arc.as_ref());
                    match verify(identity).await {
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
                            StreamEvent::EmbedderDownload {
                                phase: "error".into(),
                                bytes_downloaded: None,
                                bytes_total: None,
                                error: Some(format!(
                                    "embedding contract mismatch ({}); memory is disabled until \
                                     the mismatch is resolved — retrying will not fix this",
                                    diffs.join("; ")
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
                                error: Some(format!("embedding contract verification failed: {e}")),
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

    // ---------- run_embedder_bootstrap lifecycle ----------
    //
    // The host-facing entry points (`clawg_desktop_lib::start_embedder_bootstrap`,
    // and the daemon's equivalent) cannot be constructed without their runtime,
    // so they are verified by compile-time correctness PLUS this parametric
    // core, which is the unit-test seam: it accepts an injectable
    // `EmbedderEventSink`, `EnsureModelFn`, `LoadEmbedderFn`, and
    // `ContractVerifyFn`, so the full lifecycle (early-exit, happy path, error
    // path, contract mismatch, AND the forwarder→sink pipeline) can be
    // asserted in-process without a desktop runtime.

    use crate::memory::embedder::FakeEmbedder;
    use crate::memory::model_download::{DownloadError, ModelFiles};
    use fastembed::TokenizerFiles;
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
        Arc::new(|_identity| Box::pin(async { Ok(ContractStatus::Match) }))
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
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());

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
        let load: LoadEmbedderFn =
            Arc::new(move |_files| Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>));

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
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());

        let ensure: EnsureModelFn = Arc::new(move |_base, _progress| {
            Box::pin(async move {
                Err(DownloadError::RetryExhausted {
                    file: "onnx/model.onnx".into(),
                    attempts: 3,
                })
            })
        });
        let load: LoadEmbedderFn =
            Arc::new(move |_files| Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>));

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
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());

        let ensure: EnsureModelFn =
            Arc::new(move |_base, _progress| Box::pin(async move { Ok(empty_model_files()) }));
        let load: LoadEmbedderFn =
            Arc::new(move |_files| Ok(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>));
        let verify: ContractVerifyFn = Arc::new(|_identity| {
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
        let verify: ContractVerifyFn = Arc::new(|_identity| {
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

        run_embedder_bootstrap(
            std::env::temp_dir(),
            cell.clone(),
            Arc::new(LogSink),
            ensure,
            load,
            accept_contract(),
        )
        .await;

        assert!(cell.get().is_some(), "LogSink must not interfere with the cell fill");
    }

    // ---------- production_deps ----------

    /// The trio must be buildable from nothing but an open store — the daemon
    /// has no other context to offer.
    #[tokio::test]
    async fn production_deps_builds_all_three_closures_from_a_store() {
        let store = Arc::new(MemoryStore::new_for_test().await.expect("test store"));
        let (_ensure, load, verify) = production_deps(store);

        // `load` is pure: hand it empty model bytes and it must report a
        // load failure rather than panicking or hanging.
        assert!(
            load(empty_model_files()).is_err(),
            "loading empty ONNX bytes must fail, not panic"
        );

        // `verify` must reach the real store: a fresh store has no contract
        // row, so the first identity initializes it.
        let status = verify(EmbeddingIdentity {
            model_id: "fake-embedder".into(),
            dim: 384,
            query_prefix: "query: ".into(),
            passage_prefix: "passage: ".into(),
        })
        .await
        .expect("contract check must reach the store");
        assert!(matches!(status, ContractStatus::Initialized));
    }
}
