//! The daemon's socket (`<data>/clawg.sock`): where it can live and who may
//! connect to it.
//!
//! The guarantee is the data directory: it is private to its owner, so no
//! other account can create the socket there, and so none can stand in for a
//! daemon that died. This module holds what both systems share; the listener
//! itself is one half per system.

use std::path::{Path, PathBuf};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{admits_peer, PeerChecked};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::AfUnixListener;
#[cfg(windows)]
pub(crate) use windows::Connector;

/// Bytes in `sun_path`, the terminating NUL included: a socket path must be
/// shorter.
#[cfg(any(target_os = "linux", windows))]
pub const SUN_PATH_LEN: usize = 108;
#[cfg(not(any(target_os = "linux", windows)))]
pub const SUN_PATH_LEN: usize = 104;

#[derive(Debug, thiserror::Error)]
pub enum SocketError {
    #[error(
        "the socket path {path} is {len} bytes and a Unix socket path must be shorter than {limit}; \
         a shorter home directory fits it, and `clawg serve --port <N>` serves over TCP without it"
    )]
    TooLong { path: PathBuf, len: usize, limit: usize },
    #[cfg(windows)]
    #[error(
        "the socket path {path} is not valid Unicode and a Windows socket path must be; \
         `clawg serve --port <N>` serves over TCP without it"
    )]
    NotUnicode { path: PathBuf },
    #[cfg(windows)]
    #[error("this system has no Unix domain sockets for {path}: {source}")]
    Unsupported {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
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

/// The bytes the kernel counts in `sun_path` for `path`.
#[cfg(unix)]
fn socket_path_len(path: &Path) -> Result<usize, SocketError> {
    use std::os::unix::ffi::OsStrExt;
    Ok(path.as_os_str().as_bytes().len())
}

/// The UTF-8 bytes of `path`: the socket is built from `to_str()`, which
/// refuses what is not Unicode.
#[cfg(windows)]
fn socket_path_len(path: &Path) -> Result<usize, SocketError> {
    path.to_str().map(str::len).ok_or_else(|| SocketError::NotUnicode { path: path.to_path_buf() })
}

/// Refuses a path the system would refuse, naming it: std's own error only
/// says it must be shorter than `SUN_LEN`.
pub fn check_socket_path(path: &Path) -> Result<(), SocketError> {
    let len = socket_path_len(path)?;
    if len >= SUN_PATH_LEN {
        return Err(SocketError::TooLong { path: path.to_path_buf(), len, limit: SUN_PATH_LEN });
    }
    Ok(())
}

/// Whether a daemon whose socket failed with `err` goes on without it, over
/// TCP, instead of stopping: a path that cannot hold a socket only when
/// `--port` says where to serve, a system without sockets always, anything
/// else never.
pub fn serves_without_socket(err: &SocketError, explicit_port: bool) -> bool {
    match err {
        SocketError::TooLong { .. } => explicit_port,
        #[cfg(windows)]
        SocketError::NotUnicode { .. } => explicit_port,
        #[cfg(windows)]
        SocketError::Unsupported { .. } => true,
        SocketError::Stale { .. } | SocketError::Bind { .. } => false,
    }
}

/// Removes the socket an earlier daemon left.
///
/// Only safe with the store lock held: no other live daemon of this data
/// directory can be serving that file then. A daemon killed outright leaves
/// it behind, and `bind` on it fails with `AddrInUse`.
pub fn remove_stale_socket(path: &Path) -> Result<(), SocketError> {
    match std::fs::remove_file(path) {
        Err(source) if source.kind() != std::io::ErrorKind::NotFound => {
            Err(SocketError::Stale { path: path.to_path_buf(), source })
        }
        _ => Ok(()),
    }
}

/// What `bind_socket` returns.
#[cfg(unix)]
pub type Listener = PeerChecked;
#[cfg(windows)]
pub type Listener = AfUnixListener;

/// Binds the socket at `path`, removing the one an earlier daemon left (see
/// `remove_stale_socket`).
#[cfg(unix)]
pub fn bind_socket(path: &Path) -> Result<Listener, SocketError> {
    check_socket_path(path)?;
    remove_stale_socket(path)?;
    let inner =
        tokio::net::UnixListener::bind(path).map_err(|source| SocketError::Bind { path: path.to_path_buf(), source })?;
    // SAFETY: geteuid takes no arguments, cannot fail and touches no memory.
    let own_uid = unsafe { libc::geteuid() };
    Ok(PeerChecked::new(inner, own_uid, path.to_path_buf()))
}

#[cfg(windows)]
pub fn bind_socket(path: &Path) -> Result<Listener, SocketError> {
    check_socket_path(path)?;
    remove_stale_socket(path)?;
    windows::bind(path)
}

/// The state the router needs, over an in-memory store in `dir`.
#[cfg(test)]
async fn test_app(dir: &Path) -> std::sync::Arc<crate::daemon::api::AppCtx> {
    use crate::daemon::api::{AppCtx, SAVE_CONFIRM_TIMEOUT};
    let (runtime, _rx) = clawg_core::runtime::MemoryRuntime::new_for_test(dir.to_path_buf()).await;
    let (shutdown, _) = tokio::sync::watch::channel(false);
    std::sync::Arc::new(AppCtx {
        runtime,
        token: "socket-token".to_string(),
        profile: clawg_core::paths::Profile::Dev,
        started_at: std::time::Instant::now(),
        shutdown,
        save_timeout: SAVE_CONFIRM_TIMEOUT,
        sessions: Default::default(),
        metrics: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::future::IntoFuture;
    #[cfg(unix)]
    use reqwest::StatusCode;

    #[cfg(unix)]
    use crate::daemon::api::router;

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

    #[test]
    fn a_non_ascii_path_counts_its_utf8_bytes() {
        // Two bytes each, so the path is under the limit in characters and over it in bytes.
        let chars = SUN_PATH_LEN / 2 + 1;
        let long = PathBuf::from(format!("/{}", "é".repeat(chars)));
        assert!(long.as_os_str().len() >= SUN_PATH_LEN);
        match check_socket_path(&long) {
            Err(SocketError::TooLong { len, limit, .. }) => {
                assert_eq!(len, long.as_os_str().len());
                assert_eq!(limit, SUN_PATH_LEN);
            }
            other => panic!("expected TooLong, got {other:?}"),
        }
        let fits = PathBuf::from(format!("/{}", "é".repeat(SUN_PATH_LEN / 2 - 1)));
        assert!(check_socket_path(&fits).is_ok());
    }

    #[test]
    fn only_a_path_that_is_too_long_falls_back_to_tcp_and_only_with_a_port() {
        let too_long = SocketError::TooLong { path: PathBuf::from("/d/clawg.sock"), len: 200, limit: SUN_PATH_LEN };
        assert!(!serves_without_socket(&too_long, false));
        assert!(serves_without_socket(&too_long, true));

        let io = || std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let bind = SocketError::Bind { path: PathBuf::from("/d/clawg.sock"), source: io() };
        let stale = SocketError::Stale { path: PathBuf::from("/d/clawg.sock"), source: io() };
        for explicit_port in [false, true] {
            assert!(!serves_without_socket(&bind, explicit_port), "a failed bind stops the start");
            assert!(!serves_without_socket(&stale, explicit_port), "a stale socket that stays stops the start");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_path_that_is_not_unicode_or_a_system_without_sockets_is_decided_apart() {
        let not_unicode = SocketError::NotUnicode { path: PathBuf::from("C:/d/clawg.sock") };
        assert!(!serves_without_socket(&not_unicode, false));
        assert!(serves_without_socket(&not_unicode, true));

        let unsupported = SocketError::Unsupported {
            path: PathBuf::from("C:/d/clawg.sock"),
            source: std::io::Error::from_raw_os_error(10047),
        };
        for explicit_port in [false, true] {
            assert!(serves_without_socket(&unsupported, explicit_port), "no AF_UNIX always falls back");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_path_that_is_not_unicode_is_named() {
        use std::os::windows::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_wide(&[0x43, 0x3a, 0x5c, 0xd800]));
        match check_socket_path(&path) {
            Err(SocketError::NotUnicode { path: named }) => assert_eq!(named, path),
            other => panic!("expected NotUnicode, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_socket_an_earlier_daemon_left_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clawg.sock");
        drop(bind_socket(&path).expect("first bind"));
        assert!(path.exists(), "dropping the listener leaves the file, as a kill does");

        bind_socket(&path).expect("the second daemon takes the path");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_router_answers_over_the_socket_behind_the_guard() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clawg.sock");
        let listener = bind_socket(&path).expect("bind");
        let server = tokio::spawn(axum::serve(listener, router(test_app(dir.path()).await)).into_future());
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
