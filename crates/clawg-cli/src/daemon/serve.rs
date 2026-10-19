//! `clawg serve` — process startup and graceful shutdown.
//!
//! Deliberately thin. Everything with a decision in it lives in
//! [`super::api`] or [`super::lifecycle`], which are tested against an
//! in-memory store; this module is the one place that opens the REAL database
//! and starts the REAL embedder bootstrap, so it is also the one place no test
//! may execute (the bootstrap downloads a 470 MB model). The way to keep that
//! honest is to leave nothing here worth testing.

use std::sync::Arc;
use std::time::Instant;

use clawg_core::build_info::{self, ServeFault};
use clawg_core::memory::bootstrap::{
    run_embedder_bootstrap_with_retry, serve_deps, unloading_when_idle, EmbedderDeps, LogSink,
};
use clawg_core::memory::idle_embedder::{idle_after, DEFAULT_IDLE, IDLE_MINUTES_VAR};
use clawg_core::memory::store_lock::StoreHolder;
use clawg_core::memory::MemoryError;
use clawg_core::paths::{data_dir, PathsError, Profile};
use clawg_core::runtime::{MemoryRuntime, Workers};

use super::api::{router, AppCtx, SAVE_CONFIRM_TIMEOUT};
use super::lifecycle::{
    clear_state, generate_token, now_ms, write_state, write_token, DaemonState, LifecycleError,
};

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error(transparent)]
    Lifecycle(#[from] LifecycleError),
    #[error(transparent)]
    Paths(#[from] PathsError),
    /// Another process (a second daemon or an evaluation run) owns the store.
    /// Its own variant because it is not a fault: the fix is the user's, and
    /// the message has to say so instead of reading like a corrupt database.
    #[error("{0}; only one process can own the data directory, so close it (or run `clawg stop` if it is a daemon) and try again")]
    StoreInUse(MemoryError),
    #[error("could not open the memory store: {0}")]
    Memory(MemoryError),
    #[error("could not bind 127.0.0.1:{port}: {source}")]
    Bind {
        port: u16,
        #[source]
        source: std::io::Error,
    },
    #[error("could not create the data directory {path}: {source}")]
    DataDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// Only a build with test stand-ins can refuse its embedder this early.
    #[error("{0}")]
    Embedder(String),
    #[error("the HTTP server stopped unexpectedly: {0}")]
    Server(std::io::Error),
    /// Only a build with test stand-ins fails on purpose.
    #[error("{0}")]
    Injected(String),
}

/// Splits "someone else has the store" from every other reason opening it can
/// fail; see [`ServeError::StoreInUse`].
fn classify_open_error(err: MemoryError) -> ServeError {
    match err {
        MemoryError::StoreInUse { .. } => ServeError::StoreInUse(err),
        other => ServeError::Memory(other),
    }
}

/// Runs the daemon until Ctrl-C or `POST /v1/shutdown`.
///
/// `port` of `0` asks the OS for an ephemeral port; the real one is what gets
/// published in `daemon.json`, so clients always dial where the socket actually
/// is.
pub async fn run(profile: Profile, port: u16) -> Result<(), ServeError> {
    let base_dir = data_dir(profile)?;
    std::fs::create_dir_all(&base_dir).map_err(|source| ServeError::DataDir {
        path: base_dir.display().to_string(),
        source,
    })?;

    build_info::inject_serve_fault(ServeFault::BeforeOpen).map_err(ServeError::Injected)?;

    // The store lock is taken inside `open`, BEFORE SQLite is touched, and held
    // by `runtime.store` for the life of this process. Two processes on one
    // database would each drain the ingest channel, and the dedup gate is only
    // correct with a single sequential consumer, so losing the race has to
    // mean "do not start", not "start and hope". The same lock also keeps a
    // second daemon out, which is why there is no daemon-specific lock any more.
    let (runtime, ingest_rx) = MemoryRuntime::open(base_dir.clone(), StoreHolder::Daemon)
        .await
        .map_err(classify_open_error)?;
    // The store is migrated by now and the lock is released when this returns.
    build_info::inject_serve_fault(ServeFault::AfterOpen).map_err(ServeError::Injected)?;

    // Background, never awaited: the model may not be on disk yet, and the
    // daemon must answer `/health` and accept saves throughout the download.
    // A failed download is retried until the network comes back.
    let EmbedderDeps { ensure, cached, load, verify } =
        serve_deps(
            runtime.store.clone(),
            clawg_core::paths::backups_dir(&base_dir),
            runtime.tokenizer.clone(),
        )
        .map_err(ServeError::Embedder)?;
    // A wrong value must not keep the daemon from starting under an editor
    // that spawned it; the default is what the user had before setting it.
    let idle = idle_after(std::env::var(IDLE_MINUTES_VAR).ok().as_deref()).unwrap_or_else(|e| {
        log::warn!("{e}; the model is let go after {} minutes without use", DEFAULT_IDLE.as_secs() / 60);
        Some(DEFAULT_IDLE)
    });
    // Wrapped even when it is never let go: the wrapper is also what loads
    // the model again when its inference worker is gone.
    let load = unloading_when_idle(
        cached,
        ensure.clone(),
        load,
        base_dir.clone(),
        idle,
        tokio::runtime::Handle::current(),
    );
    tokio::spawn(run_embedder_bootstrap_with_retry(
        base_dir.clone(),
        runtime.embedder.clone(),
        runtime.embedder_status.clone(),
        Arc::new(LogSink),
        ensure,
        load,
        verify,
        tokio::time::sleep,
    ));
    // The single ingest spawn site for this process.
    let (drain, drain_rx) = tokio::sync::watch::channel(None);
    let workers = Workers {
        ingest: tokio::spawn(runtime.ingest_worker(ingest_rx, drain_rx)),
        maintenance: tokio::spawn(runtime.maintenance_worker()),
        drain,
    };

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|source| ServeError::Bind { port, source })?;
    let bound_port = listener
        .local_addr()
        .map_err(|source| ServeError::Bind { port, source })?
        .port();

    // Token and state are published only once the socket is bound: a client
    // that finds `daemon.json` must find something listening behind it.
    let token = generate_token();
    write_token(&base_dir, &token)?;
    write_state(
        &base_dir,
        &DaemonState {
            pid: std::process::id(),
            port: bound_port,
            profile: profile.as_str().to_string(),
            version: clawg_core::build_info::version().to_string(),
            started_at_ms: now_ms(),
        },
    )?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let ctx = Arc::new(AppCtx {
        runtime,
        token,
        profile,
        started_at: Instant::now(),
        shutdown: shutdown_tx,
        save_timeout: SAVE_CONFIRM_TIMEOUT,
        sessions: Default::default(),
    });

    log::info!(
        "clawg serve {} listening on 127.0.0.1:{bound_port}, data dir {}",
        profile.as_str(),
        base_dir.display()
    );

    let result = axum::serve(listener, router(ctx.clone()))
        .with_graceful_shutdown(shutdown_signal(shutdown_rx))
        .await;

    // No request can queue a save any more, so whatever is queued now is all
    // there is to drain or spill.
    ctx.runtime.shutdown(workers).await;

    // Always, on every exit path: a stale `daemon.json` sends the next client
    // to a port nothing is listening on, and the failure looks like a hang.
    clear_state(&base_dir);
    log::info!("clawg serve {} stopped", profile.as_str());

    result.map_err(ServeError::Server)
}

/// Completes on Ctrl-C or on the API's shutdown flag, whichever comes first.
///
/// When the watch channel's sender is gone the API can no longer ask for
/// anything, so that branch parks forever rather than resolving — otherwise a
/// dropped sender would read as "shut down now" and stop the daemon.
async fn shutdown_signal(mut rx: tokio::sync::watch::Receiver<bool>) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
        log::info!("received Ctrl-C, shutting down");
    };

    let via_api = async move {
        loop {
            let flagged = *rx.borrow_and_update();
            if flagged {
                log::info!("received a shutdown request, shutting down");
                return;
            }
            if rx.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    };

    tokio::select! {
        _ = ctrl_c => {}
        _ = via_api => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawg_core::memory::store_lock::{StoreHolder, StoreOwner};
    use clawg_core::memory::MemoryError;
    use std::time::Duration;

    /// The daemon losing the store to another process is a user-facing event,
    /// not a database fault: the message must name the holder and say what to
    /// do, and must not be dressed up as "could not open the memory store".
    #[test]
    fn a_lost_store_race_is_reported_with_the_holder_and_what_to_do() {
        let err = classify_open_error(MemoryError::StoreInUse {
            owner: Some(StoreOwner { holder: StoreHolder::Eval, pid: 4242, started_at_ms: 1 }),
        });
        let msg = err.to_string();
        assert!(msg.contains("an evaluation run (pid 4242)"), "{msg}");
        assert!(msg.contains("close it"), "{msg}");
        assert!(!msg.contains("could not open"), "{msg}");
    }

    #[test]
    fn a_lost_store_race_with_an_unknown_holder_still_explains_itself() {
        let msg = classify_open_error(MemoryError::StoreInUse { owner: None }).to_string();
        assert!(msg.contains("another process"), "{msg}");
    }

    #[test]
    fn any_other_open_failure_keeps_the_could_not_open_wording() {
        let msg = classify_open_error(MemoryError::Internal("boom".into())).to_string();
        assert!(msg.starts_with("could not open the memory store"), "{msg}");
        assert!(msg.contains("boom"), "{msg}");
    }

    #[tokio::test]
    async fn shutdown_signal_waits_while_the_flag_is_false() {
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let waited = tokio::time::timeout(Duration::from_millis(100), shutdown_signal(rx)).await;
        assert!(waited.is_err(), "an unflagged daemon must keep serving");
    }

    #[tokio::test]
    async fn shutdown_signal_completes_when_the_flag_is_raised() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(shutdown_signal(rx));
        tx.send(true).expect("send");
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("the signal must fire within two seconds")
            .expect("task");
    }

    /// A flag already raised before the future is even created must still be
    /// seen — `changed()` alone would miss it and the daemon would never stop.
    #[tokio::test]
    async fn shutdown_signal_sees_a_flag_raised_before_it_started() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        tx.send(true).expect("send");
        tokio::time::timeout(Duration::from_secs(2), shutdown_signal(rx))
            .await
            .expect("an already-raised flag must resolve immediately");
    }

    /// If every sender is dropped the API can no longer request anything, so
    /// this branch must park — resolving would shut a healthy daemon down.
    #[tokio::test]
    async fn shutdown_signal_does_not_fire_when_the_sender_is_dropped() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        drop(tx);
        let waited = tokio::time::timeout(Duration::from_millis(100), shutdown_signal(rx)).await;
        assert!(waited.is_err(), "a dropped sender is not a shutdown request");
    }
}
