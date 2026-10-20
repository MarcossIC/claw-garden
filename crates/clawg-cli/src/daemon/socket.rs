//! The daemon's Unix domain socket (`<data>/clawg.sock`): where it can live and
//! who may connect to it.
//!
//! The guarantee is the data directory: it is private to its owner, so no
//! other account can create the socket there, and so none can stand in for a
//! daemon that died. The uid of each peer is compared too, for a directory
//! that is not in the mode clawg left it in or a system that ignores the
//! socket's own mode, which `unix(7)` does not promise to honour. Root and the
//! same user pass either way; they read `token` anyway.

use std::path::{Path, PathBuf};
use std::time::Instant;

use tokio::net::{UnixListener, UnixStream};

use super::guard::RefusalLog;

/// Whether a connection from `peer_uid` may reach a daemon running as
/// `own_uid`.
pub fn admits_peer(peer_uid: u32, own_uid: u32) -> bool {
    peer_uid == own_uid
}

/// Bytes in `sun_path`, the terminating NUL included: a socket path must be
/// shorter.
#[cfg(target_os = "linux")]
pub const SUN_PATH_LEN: usize = 108;
#[cfg(not(target_os = "linux"))]
pub const SUN_PATH_LEN: usize = 104;

#[derive(Debug, thiserror::Error)]
pub enum SocketError {
    #[error(
        "the socket path {path} is {len} bytes and a Unix socket path must be shorter than {limit}; \
         a shorter home directory fits it, and `clawg serve --port <N>` serves over TCP without it"
    )]
    TooLong { path: PathBuf, len: usize, limit: usize },
    #[error("cannot remove the socket {path} an earlier daemon left: {source}")]
    Stale {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not bind the socket {path}: {source}")]
    Bind {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Refuses a path the kernel would refuse, naming it: std's own error only
/// says it must be shorter than `SUN_LEN`.
pub fn check_socket_path(path: &Path) -> Result<(), SocketError> {
    use std::os::unix::ffi::OsStrExt;
    let len = path.as_os_str().as_bytes().len();
    if len >= SUN_PATH_LEN {
        return Err(SocketError::TooLong { path: path.to_path_buf(), len, limit: SUN_PATH_LEN });
    }
    Ok(())
}

/// Binds the socket at `path`, removing the one an earlier daemon left.
///
/// Only safe with the store lock held: no other live daemon of this data
/// directory can be serving that file then. A daemon killed outright leaves
/// it behind, and `bind` on it fails with `AddrInUse`.
pub fn bind_socket(path: &Path) -> Result<PeerChecked, SocketError> {
    check_socket_path(path)?;
    match std::fs::remove_file(path) {
        Err(source) if source.kind() != std::io::ErrorKind::NotFound => {
            return Err(SocketError::Stale { path: path.to_path_buf(), source });
        }
        _ => {}
    }
    let inner = UnixListener::bind(path).map_err(|source| SocketError::Bind { path: path.to_path_buf(), source })?;
    // SAFETY: geteuid takes no arguments, cannot fail and touches no memory.
    let own_uid = unsafe { libc::geteuid() };
    Ok(PeerChecked { inner, own_uid, path: path.to_path_buf(), refusals: RefusalLog::default() })
}

/// A socket listener that hands axum only the connections of its own user.
pub struct PeerChecked {
    inner: UnixListener,
    own_uid: u32,
    path: PathBuf,
    refusals: RefusalLog,
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
    use std::future::IntoFuture;
    use std::sync::Arc;

    use clawg_core::paths::Profile;
    use clawg_core::runtime::MemoryRuntime;
    use reqwest::StatusCode;

    use crate::daemon::api::{router, AppCtx, SAVE_CONFIRM_TIMEOUT};

    #[test]
    fn the_same_uid_is_admitted_and_another_is_not() {
        assert!(admits_peer(1000, 1000));
        assert!(admits_peer(0, 0));
        assert!(!admits_peer(1001, 1000));
        assert!(!admits_peer(0, 1000), "root is kept out by the directory, not by this check");
    }

    #[test]
    fn the_socket_path_fits_under_the_limit_and_a_longer_one_is_named() {
        let fits = PathBuf::from(format!("/{}", "a".repeat(SUN_PATH_LEN - 2)));
        assert!(check_socket_path(&fits).is_ok());

        let long = PathBuf::from(format!("/{}", "a".repeat(SUN_PATH_LEN - 1)));
        let err = check_socket_path(&long).unwrap_err().to_string();
        assert!(err.contains(&long.display().to_string()), "{err}");
        assert!(err.contains(&SUN_PATH_LEN.to_string()), "{err}");
        assert!(err.contains("--port"), "{err}");
    }

    #[tokio::test]
    async fn a_socket_an_earlier_daemon_left_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clawg.sock");
        drop(bind_socket(&path).expect("first bind"));
        assert!(path.exists(), "dropping the listener leaves the file, as a kill does");

        bind_socket(&path).expect("the second daemon takes the path");
    }

    async fn app(dir: &Path) -> Arc<AppCtx> {
        let (runtime, _rx) = MemoryRuntime::new_for_test(dir.to_path_buf()).await;
        let (shutdown, _) = tokio::sync::watch::channel(false);
        Arc::new(AppCtx {
            runtime,
            token: "socket-token".to_string(),
            profile: Profile::Dev,
            started_at: Instant::now(),
            shutdown,
            save_timeout: SAVE_CONFIRM_TIMEOUT,
            sessions: Default::default(),
        })
    }

    #[tokio::test]
    async fn the_router_answers_over_the_socket_behind_the_guard() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clawg.sock");
        let listener = bind_socket(&path).expect("bind");
        let server = tokio::spawn(axum::serve(listener, router(app(dir.path()).await)).into_future());
        let http = reqwest::Client::builder().unix_socket(path.as_path()).build().unwrap();

        let ok = http.get("http://localhost/health").send().await.expect("health");
        assert_eq!(ok.status(), StatusCode::OK);
        let named = http.get("http://clawg/health").send().await.expect("another host");
        assert_eq!(named.status(), StatusCode::FORBIDDEN);
        let page = http
            .get("http://localhost/health")
            .header("origin", "http://evil.example")
            .send()
            .await
            .expect("an origin");
        assert_eq!(page.status(), StatusCode::FORBIDDEN);
        let unauthorized = http.get("http://localhost/v1/stats").send().await.expect("stats");
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED, "the token still guards /v1");
        server.abort();
    }
}
