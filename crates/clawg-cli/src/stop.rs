//! `clawg stop` — ask the daemon to shut down, then wait until it is gone.
//!
//! "Gone" means the process ended, not that it accepted the request: see
//! [`crate::process`] for what a still-running daemon keeps holding.

use std::path::Path;
use std::time::Duration;

use crate::client::{ClientError, DaemonClient};
use crate::process::{Exit, Watched};

/// How long `clawg stop` waits for the daemon to end. A shutdown goes through
/// five phases, each with its own bound: the requests in flight
/// ([`crate::daemon::serve::HTTP_GRACE`]), the last minute of metrics
/// ([`crate::daemon::serve::METRICS_CLOSE`]), the drain of what is still queued
/// ([`clawg_core::runtime::DRAIN_DEADLINE`], leaving the rest in the outbox),
/// the save in flight, the access flush and the checkpoint
/// ([`clawg_core::runtime::AFTER_DRAIN`]), and the blocking tasks the process
/// waits for before it exits ([`crate::daemon::serve::RUNTIME_TEARDOWN`]).
/// With an idle daemon that is well under a second; this bound has to cover
/// all five with room to spare, so the installer never replaces the binary
/// while the daemon is still writing.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// Stops the daemon published in `base_dir` and returns its pid once the
/// process has ended.
pub async fn stop_in(base_dir: &Path, timeout: Duration) -> Result<u32, ClientError> {
    let client = DaemonClient::from_data_dir(base_dir).ok_or(ClientError::NotRunning)?;
    // The pid comes from `/health`, not from `daemon.json`: a state file can
    // outlive its daemon, an answer cannot.
    let pid = client.health().await?.pid;
    // Opened before the request, while the pid is known to be the daemon's.
    let watched = Watched::open(pid);

    client.shutdown().await?;

    let Some(watched) = watched else {
        // It ended between `/health` and the open: already stopped.
        return Ok(pid);
    };
    // The wait blocks a thread; keep it off the async workers.
    match tokio::task::spawn_blocking(move || watched.wait(timeout)).await {
        Ok(Exit::Exited) => Ok(pid),
        // A panicked wait proved nothing, so it counts as not confirmed.
        Ok(Exit::TimedOut) | Err(_) => Err(ClientError::StopTimeout { pid, timeout }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;

    use clawg_core::paths::Profile;
    use clawg_core::runtime::MemoryRuntime;

    use crate::daemon::api::{router, AppCtx, SAVE_CONFIRM_TIMEOUT};
    use crate::daemon::lifecycle::{write_state, write_token, DaemonState};

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    /// What no phase counts: the request, the wait itself, a loaded machine.
    const SLACK: Duration = Duration::from_secs(5);

    #[test]
    fn every_phase_of_a_shutdown_fits_inside_the_stop_timeout() {
        use crate::daemon::serve::{HTTP_GRACE, METRICS_CLOSE, RUNTIME_TEARDOWN};
        use clawg_core::runtime::{AFTER_DRAIN, DRAIN_DEADLINE};

        let phases = HTTP_GRACE + METRICS_CLOSE + DRAIN_DEADLINE + AFTER_DRAIN + RUNTIME_TEARDOWN;
        assert!(
            phases + SLACK <= STOP_TIMEOUT,
            "a shutdown that uses every phase ({phases:?}) must end before `clawg stop` gives up on the process"
        );
    }

    fn publish(dir: &Path, port: u16) {
        write_state(
            dir,
            &DaemonState {
                pid: std::process::id(),
                port: Some(port),
                unix_socket: false,
                profile: "dev".into(),
                version: "0.1.0".into(),
                started_at_ms: 0,
            },
        )
        .expect("write state");
        write_token(dir, TOKEN).expect("write token");
    }

    #[tokio::test]
    async fn stopping_with_no_daemon_published_reports_not_running() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = stop_in(dir.path(), Duration::from_secs(1)).await.expect_err("nothing to stop");
        assert!(matches!(err, ClientError::NotRunning), "got {err:?}");
    }

    #[tokio::test]
    async fn a_published_daemon_that_does_not_answer_is_not_running() {
        // A crash leaves the state file behind; only `/health` says who is alive.
        let dir = tempfile::tempdir().expect("tempdir");
        publish(dir.path(), 1);
        let err = stop_in(dir.path(), Duration::from_secs(1)).await.expect_err("nothing to stop");
        assert!(matches!(err, ClientError::NotRunning), "got {err:?}");
    }

    /// The in-process daemon reports this test's own pid, a process that will
    /// certainly not end — which is exactly the case where `stop` must say the
    /// daemon is still there instead of claiming success.
    #[tokio::test]
    async fn a_daemon_that_outlives_the_timeout_is_reported_with_its_pid() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (runtime, _rx) = MemoryRuntime::new_for_test(dir.path().to_path_buf()).await;
        let (shutdown, mut shutdown_rx) = tokio::sync::watch::channel(false);
        let ctx = Arc::new(AppCtx {
            runtime,
            token: TOKEN.to_string(),
            profile: Profile::Dev,
            started_at: Instant::now(),
            shutdown,
            save_timeout: SAVE_CONFIRM_TIMEOUT,
            sessions: Default::default(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(ctx)).await;
        });
        publish(dir.path(), port);

        let started = Instant::now();
        let err = stop_in(dir.path(), Duration::from_millis(300))
            .await
            .expect_err("this process does not exit");

        match err {
            ClientError::StopTimeout { pid, .. } => assert_eq!(pid, std::process::id()),
            other => panic!("expected StopTimeout, got {other:?}"),
        }
        assert!(started.elapsed() >= Duration::from_millis(250), "the wait really waited");
        assert!(
            *shutdown_rx.borrow_and_update(),
            "the shutdown request reached the daemon before the wait"
        );
    }
}
