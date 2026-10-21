//! The Unix half of the socket: the listener that hands axum only the
//! connections of its own user.
//!
//! The uid of each peer is compared for a directory that is not in the mode
//! clawg left it in or a system that ignores the socket's own mode, which
//! `unix(7)` does not promise to honour. Root and the same user pass either
//! way; they read `token` anyway.

use std::path::PathBuf;
use std::time::Instant;

use tokio::net::{UnixListener, UnixStream};

use crate::daemon::guard::RefusalLog;

/// Whether a connection from `peer_uid` may reach a daemon running as
/// `own_uid`.
pub fn admits_peer(peer_uid: u32, own_uid: u32) -> bool {
    peer_uid == own_uid
}

/// A socket listener that hands axum only the connections of its own user.
pub struct PeerChecked {
    inner: UnixListener,
    own_uid: u32,
    path: PathBuf,
    refusals: RefusalLog,
}

impl PeerChecked {
    pub(super) fn new(inner: UnixListener, own_uid: u32, path: PathBuf) -> Self {
        Self { inner, own_uid, path, refusals: RefusalLog::default() }
    }
}

impl axum::serve::Listener for PeerChecked {
    type Io = UnixStream;
    type Addr = tokio::net::unix::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            // axum's own accept for the socket, which retries what `accept`
            // fails with.
            let (io, addr) = axum::serve::Listener::accept(&mut self.inner).await;
            if io.peer_cred().is_ok_and(|cred| admits_peer(cred.uid(), self.own_uid)) {
                return (io, addr);
            }
            // Dropped before hyper reads a byte of it.
            drop(io);
            if let Some(count) = self.refusals.refused(Instant::now()) {
                log::warn!(
                    "refused {count} connection(s) on {} from another user",
                    self.path.display()
                );
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_uid_is_admitted_and_another_is_not() {
        assert!(admits_peer(1000, 1000));
        assert!(admits_peer(0, 0));
        assert!(!admits_peer(1001, 1000));
        assert!(!admits_peer(0, 1000), "root is kept out by the directory, not by this check");
    }
}
