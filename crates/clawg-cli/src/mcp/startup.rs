//! The bridge's daemon client, which may not exist yet when the host connects.
//!
//! A cold-started daemon opens SQLite, migrates and binds before it answers,
//! which can outlast the host's connect timeout. The bridge therefore answers
//! the handshake first and lets that start finish in the background; tools
//! come here for their client and wait for it, but only for a bounded time.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::client::{ClientError, DaemonClient};

/// Where the background start stands.
pub enum Startup {
    Starting,
    Ready(DaemonClient),
    Failed(Arc<ClientError>),
}

/// A shared view of the background start, cloned into every tool call.
#[derive(Clone)]
pub struct ClientHandle {
    rx: watch::Receiver<Startup>,
    /// How long one tool call may wait for [`Startup::Starting`] to resolve.
    wait: Duration,
}

impl ClientHandle {
    /// A handle on a daemon that is already answering.
    pub fn ready(client: DaemonClient) -> Self {
        let (_tx, rx) = watch::channel(Startup::Ready(client));
        Self { rx, wait: Duration::ZERO }
    }

    /// A handle on a start that has not finished. The sender publishes the
    /// outcome; dropping it unresolved reads as "no daemon".
    pub fn pending(wait: Duration) -> (watch::Sender<Startup>, Self) {
        let (tx, rx) = watch::channel(Startup::Starting);
        (tx, Self { rx, wait })
    }

    /// The client, waiting at most the configured time for the start to finish.
    pub async fn get(&self) -> Result<DaemonClient, Arc<ClientError>> {
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
}
