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
use clawg_core::memory::embedder::E5_MODEL_ID;
use clawg_core::memory::idle_embedder::{idle_after, LoadWatch, DEFAULT_IDLE, IDLE_MINUTES_VAR};
use clawg_core::memory::store_lock::StoreHolder;
use clawg_core::memory::MemoryError;
use clawg_core::metrics::store::RunInfo;
use clawg_core::paths::{data_dir, socket_file, PathsError, Profile};
use clawg_core::private_dir::ensure_private_dir;
use clawg_core::runtime::{MemoryRuntime, Workers};

use super::api::{router_on, AppCtx, SAVE_CONFIRM_TIMEOUT};
use super::metrics::{metrics_path, Transport, Writer, WriterTask, METRICS_VAR};
use super::process::utc_offset_s;
use super::socket::{bind_socket, serves_without_socket, Listener, SocketError};
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

/// How long the metrics writer may take to write the last minute and the end
/// of the run once the servers stopped. Past it the run is left without an
/// end, as a kill leaves it. Another of the phases `clawg stop`'s timeout
/// covers.
pub const METRICS_CLOSE: Duration = Duration::from_secs(1);

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
/// It listens on the data directory's socket, and on TCP as [`tcp_plan`]
/// says. `port` is `--port`: `0` asks the OS for an ephemeral
/// port, and the real one is what gets published in `daemon.json`, so clients
/// always dial where the listener actually is.
pub async fn run(profile: Profile, port: Option<u16>) -> Result<(), ServeError> {
    // First, before anything can fail: `serve.log` outlives binaries, since
    // `update` starts the new one on the same file.
    log::info!("{}", starting_line(profile, build_info::version(), build_info::dist_target()));
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
        LoadWatch {
            memory: Some(clawg_core::metrics::MemoryProbes {
                private: super::process::private_bytes,
                private_peak: super::process::private_peak_bytes,
            }),
            metrics: runtime.metrics.clone(),
        },
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
    let socket = bind_profile_socket(&base_dir, port.is_some())?;
    let has_socket = socket.is_some();

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

    let writer = {
        let path = metrics_path(&base_dir, std::env::var(METRICS_VAR).ok().as_deref());
        let run = run_info(profile, idle);
        // The runtime's, so what the search, the ingest and the embedder
        // measure is written with the daemon's own series.
        let (metrics, path) = (runtime.metrics.clone(), path.clone());
        tokio::task::spawn_blocking(move || Writer::start(path.as_deref(), metrics, &run, now_ms() as i64))
            .await
            .map_err(|e| ServeError::Server(std::io::Error::other(e)))?
    };
    let resident = {
        let embedder = runtime.embedder.clone();
        move || embedder.get().map(|e| e.resident())
    };
    let writer = WriterTask::spawn(writer, resident);

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

    let store_probe = super::store_probe::spawn(ctx.runtime.store.clone(), base_dir.clone(), ctx.runtime.metrics.clone());

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
    let tcp_server = {
        let (app, stopping) = (router_on(ctx.clone(), Transport::Tcp), stopping.clone());
        async move {
            match tcp {
                Some((listener, _)) => axum::serve(listener, app).with_graceful_shutdown(stopped(stopping)).await,
                None => Ok(()),
            }
        }
    };
    let socket_server = {
        let (app, stopping) = (router_on(ctx.clone(), Transport::Unix), stopping.clone());
        async move {
            match socket {
                Some(listener) => axum::serve(listener, app).with_graceful_shutdown(stopped(stopping)).await,
                None => Ok(()),
            }
        }
    };
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

    // Before the drain and the close, so no probe waits in the queue the
    // close empties.
    store_probe.abort();

    // No request comes in any more, so the last minute is complete.
    writer.close(METRICS_CLOSE).await;

    // No request can queue a save any more, so whatever is queued now is all
    // there is to drain or spill.
    // After a shutdown that finished nothing else uses the connection, so the
    // close runs at once; after one its bound cut, a call may still hold the
    // connection, and the close would wait behind it past every bound.
    if ctx.runtime.shutdown(workers).await {
        ctx.runtime.close().await;
    }

    // Always, on every exit path: a stale `daemon.json` sends the next client
    // to a port nothing is listening on, and the failure looks like a hang.
    clear_state(&base_dir);
    log::info!("clawg serve {} stopped", profile.as_str());

    result.map_err(ServeError::Server)
}

/// What this run records about itself in `metrics.db`.
fn run_info(profile: Profile, idle: Option<Duration>) -> RunInfo {
    RunInfo {
        started_at_ms: now_ms() as i64,
        version: build_info::version().to_string(),
        dist_target: build_info::dist_target().map(str::to_string),
        profile: profile.as_str().to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cpus: std::thread::available_parallelism().map_or(1, |n| n.get() as i64),
        embedding_model: E5_MODEL_ID.to_string(),
        idle_minutes: Some(idle.map_or(0, |d| (d.as_secs() / 60) as i64)),
        utc_offset_s: utc_offset_s(),
    }
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

/// The data directory's socket, or `None` when this binary speaks TCP alone,
/// the system has no such sockets, or the path is too long for one and
/// `--port` was given.
fn bind_profile_socket(base_dir: &std::path::Path, explicit_port: bool) -> Result<Option<Listener>, ServeError> {
    if build_info::tcp_only() {
        return Ok(None);
    }
    match bind_socket(&socket_file(base_dir)) {
        Ok(listener) => Ok(Some(listener)),
        Err(e) if serves_without_socket(&e, explicit_port) => {
            log::warn!("{e}; serving over TCP alone");
            Ok(None)
        }
        Err(e) => Err(ServeError::Socket(e)),
    }
}

fn starting_line(profile: Profile, version: &str, dist_target: Option<&str>) -> String {
    match dist_target {
        Some(target) => format!("clawg serve {version} ({target}) starting, profile {}", profile.as_str()),
        None => format!("clawg serve {version} starting, profile {}", profile.as_str()),
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
    fn the_starting_line_names_the_version_and_the_build() {
        assert_eq!(
            starting_line(Profile::Dev, "0.50.0", Some("x86_64-unknown-linux-gnu")),
            "clawg serve 0.50.0 (x86_64-unknown-linux-gnu) starting, profile dev"
        );
        assert_eq!(starting_line(Profile::Prod, "0.50.0", None), "clawg serve 0.50.0 starting, profile prod");
    }

    #[test]
    fn the_start_line_names_every_listener() {
        let socket = std::path::Path::new("/d/clawg.sock");
        assert_eq!(listening_on(Some(1), Some(socket)), "127.0.0.1:1 and /d/clawg.sock");
        assert_eq!(listening_on(Some(1), None), "127.0.0.1:1");
        assert_eq!(listening_on(None, Some(socket)), "/d/clawg.sock");
        let windows = std::path::Path::new(r"C:\d\clawg.sock");
        assert_eq!(listening_on(Some(1), Some(windows)), r"127.0.0.1:1 and C:\d\clawg.sock");
        assert_eq!(listening_on(None, None), "nothing");
    }

    /// A port a child inherits stays open after the daemon lets it go: it
    /// takes connections nobody answers, reset when the child exits, and keeps
    /// the next daemon off the port, for as long as the child lives. The port
    /// is one below every system's ephemeral range (32768 on Linux, 49152 on
    /// Windows): one of those, freed, can go at once to another test's
    /// `bind(0)` or outgoing connection.
    #[tokio::test]
    async fn a_child_started_while_the_port_is_bound_does_not_keep_it() {
        // A test run with its stdin closed hands fd 0 to the first file or
        // socket it opens, and every child any test spawns meanwhile with
        // the stdin it inherits gets that one as its own: the listener must
        // not be it.
        #[cfg(unix)]
        loop {
            use std::os::fd::AsRawFd;
            let null = std::fs::File::open("/dev/null").expect("open /dev/null");
            if null.as_raw_fd() > 2 {
                break;
            }
            std::mem::forget(null);
        }
        let first = 20_000 + (std::process::id() % 10_000) as u16;
        let mut bound = None;
        for k in 0..10_000 {
            if let Ok(b) = bind_tcp(20_000 + (first - 20_000 + k) % 10_000).await {
                bound = Some(b);
                break;
            }
        }
        let (listener, port) = bound.expect("a free port between 20000 and 30000");
        let mut command = if cfg!(windows) {
            let mut c = std::process::Command::new("ping");
            c.args(["-n", "30", "127.0.0.1"]);
            c
        } else {
            let mut c = std::process::Command::new("sleep");
            c.arg("30");
            c
        };
        // Nor any stdio of this process, which may be another test's socket.
        let null = std::process::Stdio::null;
        let mut child = command.stdin(null()).stdout(null()).stderr(null()).spawn().expect("spawn a long-lived process");
        drop(listener);

        // Another test's spawn forks with a copy of every descriptor until it
        // execs, a few milliseconds: only a copy that outlives that is kept.
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let rebound = loop {
            match std::net::TcpListener::bind(("127.0.0.1", port)) {
                Err(_) if std::time::Instant::now() < deadline => tokio::time::sleep(Duration::from_millis(20)).await,
                bound => break bound,
            }
        };
        let _ = child.kill();
        let _ = child.wait();
        rebound.expect("the port is free once the daemon lets it go");
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
