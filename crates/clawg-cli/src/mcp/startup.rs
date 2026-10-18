//! The bridge's daemon client, which may not exist yet when the host connects.
//!
//! A cold-started daemon opens SQLite, migrates and binds before it answers,
//! which can outlast the host's connect timeout. The bridge therefore answers
//! the handshake first and lets that start finish in the background; tools
//! come here for their client and wait for it, but only for a bounded time.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;

use crate::client::{probe_running, ClientError, DaemonClient, Probe};

/// Where the background start stands.
pub enum Startup {
    Starting,
    Ready(DaemonClient),
    Failed(Arc<ClientError>),
}

/// What a tool does with a failure to reach the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Look the daemon up again from its files and repeat the call once.
    Reresolve,
    /// Tell the model, as before.
    Report,
}

/// Whether a fresh look at `daemon.json` and `token` can fix `err`.
///
/// Only failures the daemon never acted on qualify, so repeating the call
/// cannot apply anything twice: a 401 is refused before any handler runs, and
/// `NotRunning` is a connection that never opened (no call but `/health` sets
/// a timeout). A restarted daemon has a new token and maybe a new port; a
/// start that gave up may have finished since.
pub fn on_failure(err: &ClientError) -> Next {
    match err {
        ClientError::Unauthorized | ClientError::NotRunning | ClientError::StartTimeout(_) => Next::Reresolve,
        _ => Next::Report,
    }
}

/// A shared view of the background start, cloned into every tool call.
#[derive(Clone)]
pub struct ClientHandle {
    rx: watch::Receiver<Startup>,
    /// How long one tool call may wait for [`Startup::Starting`] to resolve.
    wait: Duration,
    /// Where [`ClientHandle::reresolve`] looks for the daemon.
    base_dir: PathBuf,
    /// The daemon the last [`ClientHandle::reresolve`] found. It outranks the
    /// start's outcome, which only describes the daemon this bridge began with.
    current: Arc<Mutex<Option<DaemonClient>>>,
}

impl ClientHandle {
    /// A handle on a daemon that is already answering.
    pub fn ready(client: DaemonClient) -> Self {
        let base_dir = client.base_dir().to_path_buf();
        let (_tx, rx) = watch::channel(Startup::Ready(client));
        Self { rx, wait: Duration::ZERO, base_dir, current: Arc::default() }
    }

    /// A handle on a start that has not finished. The sender publishes the
    /// outcome; dropping it unresolved reads as "no daemon".
    pub fn pending(wait: Duration, base_dir: PathBuf) -> (watch::Sender<Startup>, Self) {
        let (tx, rx) = watch::channel(Startup::Starting);
        (tx, Self { rx, wait, base_dir, current: Arc::default() })
    }

    /// The client, waiting at most the configured time for the start to finish.
    pub async fn get(&self) -> Result<DaemonClient, Arc<ClientError>> {
        if let Some(client) = self.current.lock().expect("current lock").clone() {
            return Ok(client);
        }
        let mut rx = self.rx.clone();
        let settled = tokio::time::timeout(
            self.wait,
            rx.wait_for(|state| !matches!(state, Startup::Starting)),
        )
        .await;
        match settled {
            Ok(Ok(state)) => match &*state {
                Startup::Ready(client) => Ok(client.clone()),
                Startup::Failed(err) => Err(Arc::clone(err)),
                Startup::Starting => Err(Arc::new(ClientError::StartTimeout(self.wait))),
            },
            Ok(Err(_)) => Err(Arc::new(ClientError::NotRunning)),
            Err(_) => Err(Arc::new(ClientError::StartTimeout(self.wait))),
        }
    }

    /// Looks the daemon up again from `daemon.json` and `token` and, when one
    /// answers, makes it the client of every later call.
    ///
    /// Never starts one: an open bridge may be the binary an update just
    /// replaced, and a daemon stopped on purpose (for `restore`, say) must
    /// stay stopped.
    pub async fn reresolve(&self) -> Result<DaemonClient, ClientError> {
        let client = match probe_running(&self.base_dir, false).await? {
            Probe::Running(client) => client,
            Probe::NeedsStart => return Err(ClientError::NotRunning),
        };
        *self.current.lock().expect("current lock") = Some(client.clone());
        match client.health().await {
            Ok(health) => log::info!(
                "clawg mcp reached the daemon again at {} (daemon {}, bridge {})",
                client.base_url(),
                health.version,
                clawg_core::build_info::version()
            ),
            Err(e) => log::info!("clawg mcp reached the daemon again at {} ({e})", client.base_url()),
        }
        Ok(client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawg_core::memory::store_lock::{StoreHolder, StoreOwner};

    #[test]
    fn only_a_refused_token_a_refused_connection_or_an_unfinished_start_is_reresolved() {
        let owner = StoreOwner { holder: StoreHolder::Eval, pid: 7, started_at_ms: 0 };
        let cases = [
            (ClientError::Unauthorized, Next::Reresolve),
            (ClientError::NotRunning, Next::Reresolve),
            (ClientError::StartTimeout(Duration::from_secs(1)), Next::Reresolve),
            (
                ClientError::Daemon { status: 500, message: "x".into(), details: Default::default() },
                Next::Report,
            ),
            (ClientError::Http("reset".into()), Next::Report),
            (ClientError::Spawn("boom".into()), Next::Report),
            (ClientError::StoreHeld(owner), Next::Report),
            (ClientError::WorkingDir("gone".into()), Next::Report),
            (ClientError::StopTimeout { pid: 1, timeout: Duration::from_secs(1) }, Next::Report),
        ];
        for (err, expected) in cases {
            assert_eq!(on_failure(&err), expected, "{err:?}");
        }
    }
}
