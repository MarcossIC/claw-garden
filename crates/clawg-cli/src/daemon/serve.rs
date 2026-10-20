//! `clawg serve` — process startup and graceful shutdown.
//!
//! Deliberately thin. Everything with a decision in it lives in
//! [`super::api`] or [`super::lifecycle`], which are tested against an
//! in-memory store; this module is the one place that opens the REAL database
//! and starts the REAL embedder bootstrap, so it is also the one place no test
//! may execute (the bootstrap downloads a 470 MB model). The way to keep that
//! honest is to leave nothing here worth testing.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clawg_core::build_info::{self, DefaultPort, ServeFault};
use clawg_core::memory::bootstrap::{
    run_embedder_bootstrap_with_retry, serve_deps, unloading_when_idle, EmbedderDeps, LogSink,
};
use clawg_core::memory::idle_embedder::{idle_after, DEFAULT_IDLE, IDLE_MINUTES_VAR};
use clawg_core::memory::store_lock::StoreHolder;
use clawg_core::memory::MemoryError;
use clawg_core::paths::{data_dir, socket_file, PathsError, Profile};
use clawg_core::private_dir::ensure_private_dir;
use clawg_core::runtime::{MemoryRuntime, Workers};

use super::api::{router, AppCtx, SAVE_CONFIRM_TIMEOUT};
#[cfg(unix)]
use super::socket::{bind_socket, PeerChecked, SocketError};
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
    #[cfg(unix)]
    #[error(transparent)]
    Socket(SocketError),
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

/// How long the requests in flight may go on once a shutdown was asked for.
/// Past it they are cut: a save already has its outbox row, and a snapshot is
/// only renamed to its final name once complete. One of the phases that
/// `clawg stop`'s timeout covers; a test in `stop.rs` pins the sum.
pub const HTTP_GRACE: Duration = Duration::from_secs(3);

/// How long `clawg serve`, once `run` returned, waits for blocking tasks still
/// running (a model load cannot be cancelled) before the process exits without
/// them. Another of the phases `clawg stop`'s timeout covers.
pub const RUNTIME_TEARDOWN: Duration = Duration::from_secs(2);

/// Splits "someone else has the store" from every other reason opening it can
/// fail; see [`ServeError::StoreInUse`].
fn classify_open_error(err: MemoryError) -> ServeError {
    match err {
        MemoryError::StoreInUse { .. } => ServeError::StoreInUse(err),
        other => ServeError::Memory(other),
    }
}

/// Runs the daemon until Ctrl-C, SIGTERM or `POST /v1/shutdown`.
///
/// On Unix it listens on the data directory's socket, and on TCP as
/// [`tcp_plan`] says. `port` is `--port`: `0` asks the OS for an ephemeral
/// port, and the real one is what gets published in `daemon.json`, so clients
/// always dial where the listener actually is.
pub async fn run(profile: Profile, port: Option<u16>) -> Result<(), ServeError> {
    let base_dir = data_dir(profile)?;
    ensure_private_dir(&base_dir).map_err(|source| ServeError::DataDir {
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

    // With the store lock held, so the socket an earlier daemon left behind is
    // no live daemon's.
    #[cfg(unix)]
    let socket = bind_profile_socket(&base_dir, port.is_some())?;
    #[cfg(unix)]
    let has_socket = socket.is_some();
    #[cfg(not(unix))]
    let has_socket = false;

    let tcp = match tcp_plan(port, profile.default_port(), has_socket, build_info::default_port_override()) {
        TcpPlan::Required(port) => Some(bind_tcp(port).await?),
        TcpPlan::Optional(port) => match bind_tcp(port).await {
            Ok(bound) => Some(bound),
            Err(e) => {
                log::warn!("{e}; serving on the data directory's socket alone, which a clawg older than it cannot reach");
                None
            }
        },
        TcpPlan::Off => None,
    };
    let bound_port = tcp.as_ref().map(|(_, port)| *port);

    // Token and state are published only once the sockets are bound: a client
    // that finds `daemon.json` must find something listening behind it.
    let token = generate_token();
    write_token(&base_dir, &token)?;
    write_state(
        &base_dir,
        &DaemonState {
            pid: std::process::id(),
            port: bound_port,
            unix_socket: has_socket,
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
        "clawg serve {} listening on {}, data dir {}",
        profile.as_str(),
        listening_on(bound_port, has_socket.then(|| socket_file(&base_dir)).as_deref()),
        base_dir.display()
    );

    // The signal is observed once and stops both servers; the grace counts
    // from it once.
    let (stopping_tx, stopping) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal(shutdown_rx).await;
        let _ = stopping_tx.send(true);
    });
    let app = router(ctx.clone());
    let tcp_server = {
        let (app, stopping) = (app.clone(), stopping.clone());
        async move {
            match tcp {
                Some((listener, _)) => axum::serve(listener, app).with_graceful_shutdown(stopped(stopping)).await,
                None => Ok(()),
            }
        }
    };
    #[cfg(unix)]
    let socket_server = {
        let (app, stopping) = (app, stopping.clone());
        async move {
            match socket {
                Some(listener) => axum::serve(listener, app).with_graceful_shutdown(stopped(stopping)).await,
                None => Ok(()),
            }
        }
    };
    #[cfg(not(unix))]
    let socket_server = async { Ok(()) };
    let servers = async { tokio::try_join!(tcp_server, socket_server).map(drop) };
    let result = match within_grace(servers, stopped(stopping), HTTP_GRACE).await {
        Some(result) => result,
        None => {
            log::warn!(
                "requests were still in flight {} s after the shutdown began; they were cut",
                HTTP_GRACE.as_secs()
            );
            Ok(())
        }
    };

    // No request can queue a save any more, so whatever is queued now is all
    // there is to drain or spill.
    ctx.runtime.shutdown(workers).await;

    // Always, on every exit path: a stale `daemon.json` sends the next client
    // to a port nothing is listening on, and the failure looks like a hang.
    clear_state(&base_dir);
    log::info!("clawg serve {} stopped", profile.as_str());

    result.map_err(ServeError::Server)
}

/// How the daemon listens on loopback TCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpPlan {
    /// A port asked for, or the only transport there is: a failed bind stops
    /// the start.
    Required(u16),
    /// The profile's port beside the socket, for the clients older than it:
    /// a failed bind (another user's daemon, another program) is logged and
    /// the daemon goes on over the socket.
    Optional(u16),
    Off,
}

/// The TCP side of a start: `explicit` is `--port`, `socket` whether the
/// daemon has its socket, and `stand_in` what a `testing` build puts in place
/// of the profile's port.
pub fn tcp_plan(explicit: Option<u16>, profile_port: u16, socket: bool, stand_in: Option<DefaultPort>) -> TcpPlan {
    if let Some(port) = explicit {
        return TcpPlan::Required(port);
    }
    if !socket {
        return TcpPlan::Required(match stand_in {
            Some(DefaultPort::Port(port)) => port,
            _ => profile_port,
        });
    }
    match stand_in {
        Some(DefaultPort::Off) => TcpPlan::Off,
        Some(DefaultPort::Port(port)) => TcpPlan::Optional(port),
        None => TcpPlan::Optional(profile_port),
    }
}

async fn bind_tcp(port: u16) -> Result<(tokio::net::TcpListener, u16), ServeError> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|source| ServeError::Bind { port, source })?;
    let bound = listener.local_addr().map_err(|source| ServeError::Bind { port, source })?.port();
    Ok((listener, bound))
}

/// The data directory's socket, or `None` when this binary speaks TCP alone
/// or the path is too long for one and `--port` was given.
#[cfg(unix)]
fn bind_profile_socket(base_dir: &std::path::Path, explicit_port: bool) -> Result<Option<PeerChecked>, ServeError> {
    if build_info::tcp_only() {
        return Ok(None);
    }
    match bind_socket(&socket_file(base_dir)) {
        Ok(listener) => Ok(Some(listener)),
        Err(e @ SocketError::TooLong { .. }) if explicit_port => {
            log::warn!("{e}; serving over TCP alone");
            Ok(None)
        }
        Err(e) => Err(ServeError::Socket(e)),
    }
}

fn listening_on(port: Option<u16>, socket: Option<&std::path::Path>) -> String {
    match (port, socket) {
        (Some(port), Some(socket)) => format!("127.0.0.1:{port} and {}", socket.display()),
        (Some(port), None) => format!("127.0.0.1:{port}"),
        (None, Some(socket)) => socket.display().to_string(),
        (None, None) => "nothing".to_string(),
    }
}

/// Completes once the shutdown was observed.
async fn stopped(mut stopping: tokio::sync::watch::Receiver<bool>) {
    let _ = stopping.wait_for(|stopping| *stopping).await;
}

/// Runs `server` to its end, or until `grace` after `stopping` resolved,
/// whichever comes first; `None` when the grace ran out first.
async fn within_grace<S: Future>(server: S, stopping: impl Future, grace: Duration) -> Option<S::Output> {
    tokio::pin!(server);
    tokio::select! {
        out = &mut server => return Some(out),
        _ = stopping => {}
    }
    tokio::time::timeout(grace, server).await.ok()
}

/// Completes on Ctrl-C, on SIGTERM (Unix) or on the API's shutdown flag,
/// whichever comes first.
///
/// When the watch channel's sender is gone the API can no longer ask for
/// anything, so that branch parks forever rather than resolving — otherwise a
/// dropped sender would read as "shut down now" and stop the daemon.
async fn shutdown_signal(mut rx: tokio::sync::watch::Receiver<bool>) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
        log::info!("received Ctrl-C, shutting down");
    };

    let terminate = terminated();

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
        _ = terminate => {}
        _ = via_api => {}
    }
}

/// Completes on SIGTERM, what `systemctl stop` and a plain `kill` send.
#[cfg(unix)]
async fn terminated() {
    use tokio::signal::unix::{signal, SignalKind};
    match signal(SignalKind::terminate()) {
        Ok(mut sigterm) => {
            sigterm.recv().await;
            log::info!("received SIGTERM, shutting down");
        }
        Err(e) => {
            log::warn!("cannot listen for SIGTERM: {e}; it ends the daemon without a shutdown");
            std::future::pending::<()>().await;
        }
    }
}

/// Windows has no SIGTERM; its console events are not handled.
#[cfg(not(unix))]
async fn terminated() {
    std::future::pending::<()>().await;
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

    #[test]
    fn the_tcp_plan_follows_the_port_and_the_socket() {
        use TcpPlan::*;
        assert_eq!(tcp_plan(Some(0), 27381, true, None), Required(0), "--port is always honoured");
        assert_eq!(tcp_plan(Some(4000), 27381, false, Some(DefaultPort::Off)), Required(4000));
        assert_eq!(tcp_plan(None, 27381, true, None), Optional(27381), "beside the socket for older clients");
        assert_eq!(tcp_plan(None, 27381, false, None), Required(27381), "the only transport");
        assert_eq!(tcp_plan(None, 27381, true, Some(DefaultPort::Off)), Off);
        assert_eq!(tcp_plan(None, 27381, false, Some(DefaultPort::Off)), Required(27381), "never no transport");
        assert_eq!(tcp_plan(None, 27381, true, Some(DefaultPort::Port(4001))), Optional(4001));
        assert_eq!(tcp_plan(None, 27381, false, Some(DefaultPort::Port(4001))), Required(4001));
    }

    #[test]
    fn the_start_line_names_every_listener() {
        let socket = std::path::Path::new("/d/clawg.sock");
        assert_eq!(listening_on(Some(1), Some(socket)), "127.0.0.1:1 and /d/clawg.sock");
        assert_eq!(listening_on(Some(1), None), "127.0.0.1:1");
        assert_eq!(listening_on(None, Some(socket)), "/d/clawg.sock");
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

    #[tokio::test(start_paused = true)]
    async fn a_server_that_does_not_finish_is_left_after_the_grace() {
        let started = tokio::time::Instant::now();

        let out = within_grace(std::future::pending::<()>(), async {}, Duration::from_secs(3)).await;

        assert_eq!(out, None);
        assert_eq!(started.elapsed(), Duration::from_secs(3));
    }

    #[tokio::test(start_paused = true)]
    async fn a_server_that_finishes_in_time_gives_its_result() {
        let server = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            7
        };

        assert_eq!(within_grace(server, async {}, Duration::from_secs(3)).await, Some(7));
    }

    /// The grace only starts with a shutdown: a long request in normal
    /// operation (a backup of a big store) is never cut.
    #[tokio::test(start_paused = true)]
    async fn without_a_shutdown_the_server_has_no_deadline() {
        let server = async {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            7
        };

        assert_eq!(within_grace(server, std::future::pending::<()>(), Duration::from_secs(3)).await, Some(7));
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
