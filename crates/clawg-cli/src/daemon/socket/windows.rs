//! The Windows half of the socket: a native AF_UNIX listener.
//!
//! The access is the data directory's DACL, which the socket file inherits
//! when it is created: an account without `FILE_WRITE_DATA` on the file cannot
//! connect, so no peer check is made here.
//!
//! `std::os::windows::net` is not stable, so the sockets are built with
//! socket2 and handed to tokio as TCP streams. Nothing on the path reads their
//! address, which is why `peer_addr` failing on them does no harm.

use std::io;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hyper::Uri;
use hyper_util::rt::TokioIo;
use socket2::{Domain, SockAddr, Socket, Type};
use tokio::sync::mpsc;
use windows_sys::Win32::Networking::WinSock::WSAEAFNOSUPPORT;

use super::SocketError;

/// What the kernel queues before `accept`: the backlog mio gives the TCP
/// listener on Windows, so both transports hold the same burst.
const BACKLOG: i32 = 128;

/// The connections accepted and not yet taken by the runtime, as many as the
/// backlog holds.
const PENDING: usize = BACKLOG as usize;

/// How long the accept thread rests after an error that is not a refused
/// connection, as axum does for TCP, so a full descriptor table is not spun on.
const ACCEPT_BACKOFF: Duration = Duration::from_secs(1);

/// Whether `err` says this system has no AF_UNIX: `WSASocketW` fails with
/// `WSAEAFNOSUPPORT` for a family no provider serves.
pub fn lacks_af_unix(err: &io::Error) -> bool {
    err.raw_os_error() == Some(WSAEAFNOSUPPORT)
}

/// A listener on a socket file that hands axum the connections a thread of its
/// own accepts.
///
/// The thread is a plain one, not a blocking task: the runtime waits for those
/// when it shuts down, and an `accept` that never returns would take all of
/// `RUNTIME_TEARDOWN`.
pub struct AfUnixListener {
    path: PathBuf,
    incoming: mpsc::Receiver<TcpStream>,
}

/// Binds and listens on `path`, which must not exist.
pub(super) fn bind(path: &Path) -> Result<AfUnixListener, SocketError> {
    let bind_error = |source| SocketError::Bind { path: path.to_path_buf(), source };
    let socket = Socket::new(Domain::UNIX, Type::STREAM, None).map_err(|source| {
        if lacks_af_unix(&source) {
            SocketError::Unsupported { path: path.to_path_buf(), source }
        } else {
            bind_error(source)
        }
    })?;
    socket.bind(&SockAddr::unix(path).map_err(bind_error)?).map_err(bind_error)?;
    socket.listen(BACKLOG).map_err(bind_error)?;

    let (outgoing, incoming) = mpsc::channel(PENDING);
    std::thread::Builder::new()
        .name("clawg-accept".to_string())
        .spawn(move || accept_loop(socket, outgoing))
        .map_err(bind_error)?;
    Ok(AfUnixListener { path: path.to_path_buf(), incoming })
}

/// Connects to the socket at `path`. The stream is already non-blocking, as
/// tokio's `from_std` needs it.
pub(crate) fn connect(path: &Path) -> io::Result<TcpStream> {
    let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
    socket.connect(&SockAddr::unix(path)?)?;
    socket.set_nonblocking(true)?;
    Ok(socket.into())
}

/// What a hyper-util client dials with: every connection goes to the socket at
/// `path`, whatever the URI names, so no proxy and no DNS take part.
///
/// The connect runs on the calling runtime thread. A local AF_UNIX connect is
/// queued by the kernel without waiting for the daemon's `accept`, so it ends
/// at once; a hop to a blocking thread would cost every new connection, which is
/// what each hook pays.
#[derive(Debug, Clone)]
pub(crate) struct Connector {
    path: PathBuf,
}

impl Connector {
    pub(crate) fn new(path: &Path) -> Self {
        Self { path: path.to_path_buf() }
    }
}

impl tower_service::Service<Uri> for Connector {
    type Response = TokioIo<tokio::net::TcpStream>;
    type Error = io::Error;
    type Future = std::future::Ready<io::Result<Self::Response>>;

    fn poll_ready(&mut self, _: &mut std::task::Context<'_>) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, _: Uri) -> Self::Future {
        std::future::ready(connect(&self.path).and_then(tokio::net::TcpStream::from_std).map(TokioIo::new))
    }
}

fn accept_loop(listener: Socket, outgoing: mpsc::Sender<TcpStream>) {
    loop {
        match listener.accept() {
            Ok((connection, _)) => {
                if let Err(e) = connection.set_nonblocking(true) {
                    log::warn!("dropped a connection on the socket: {e}");
                    continue;
                }
                // Fails only once the listener was dropped: the connection is
                // its own wake-up call, and with it the thread ends and the
                // socket it listens on closes.
                if outgoing.blocking_send(connection.into()).is_err() {
                    return;
                }
            }
            // Refused, aborted or reset before it was taken: nothing to wait for.
            Err(e) if is_connection_error(&e) => {}
            Err(e) => {
                if outgoing.is_closed() {
                    return;
                }
                log::error!("accept error: {e}");
                std::thread::sleep(ACCEPT_BACKOFF);
            }
        }
    }
}

fn is_connection_error(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionAborted | io::ErrorKind::ConnectionReset
    )
}

impl Drop for AfUnixListener {
    fn drop(&mut self) {
        self.incoming.close();
        // The thread is blocked in `accept`; a connection of our own gets it
        // out. If it fails the thread stays blocked until the process ends,
        // which nobody waits for.
        let _ = connect(&self.path);
    }
}

impl axum::serve::Listener for AfUnixListener {
    type Io = tokio::net::TcpStream;
    type Addr = ();

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.incoming.recv().await {
                Some(stream) => match tokio::net::TcpStream::from_std(stream) {
                    Ok(io) => return (io, ()),
                    Err(e) => log::warn!("dropped a connection on the socket: {e}"),
                },
                None => {
                    log::error!("the thread accepting on {} ended; the socket serves no more", self.path.display());
                    std::future::pending::<()>().await;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::io::ErrorKind;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use http_body_util::Empty;
    use hyper::body::Bytes;
    use hyper::StatusCode;
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;

    use super::super::{bind_socket, test_app};
    use super::*;
    use crate::client::{ClientError, DaemonClient};
    use crate::daemon::api::router;
    use crate::daemon::lifecycle::{write_state, write_token, DaemonState};

    /// The `DOMAIN\name` of the account running the test.
    fn account() -> String {
        let name = std::env::var("USERNAME").expect("USERNAME");
        match std::env::var("USERDOMAIN") {
            Ok(domain) => format!("{domain}\\{name}"),
            Err(_) => name,
        }
    }

    /// Whether `icacls` accepted the arguments; its output depends on the
    /// system's language, so only the exit code is read.
    fn icacls(args: &[&OsStr]) -> bool {
        Command::new("icacls")
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("icacls")
            .success()
    }

    async fn status_of(path: &Path, host: &str, origin: Option<&str>, uri: &str) -> StatusCode {
        let client = Client::builder(TokioExecutor::new()).build::<_, Empty<Bytes>>(Connector::new(path));
        let mut request = hyper::Request::builder().uri(format!("http://localhost{uri}")).header("host", host);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        client.request(request.body(Empty::<Bytes>::new()).unwrap()).await.expect("answer").status()
    }

    #[tokio::test]
    async fn the_router_answers_over_the_socket_behind_the_guard() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clawg.sock");
        let listener = bind_socket(&path).expect("bind");
        let server = tokio::spawn(std::future::IntoFuture::into_future(axum::serve(
            listener,
            router(test_app(dir.path()).await),
        )));

        assert_eq!(status_of(&path, "localhost", None, "/health").await, StatusCode::OK);
        assert_eq!(status_of(&path, "clawg", None, "/health").await, StatusCode::FORBIDDEN, "another host");
        assert_eq!(
            status_of(&path, "localhost", Some("http://evil.example"), "/health").await,
            StatusCode::FORBIDDEN,
            "an origin"
        );
        assert_eq!(
            status_of(&path, "localhost", None, "/v1/stats").await,
            StatusCode::UNAUTHORIZED,
            "the token still guards /v1"
        );
        server.abort();
    }

    #[tokio::test]
    async fn dropping_the_listener_closes_its_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clawg.sock");
        let listener = bind_socket(&path).expect("bind");
        connect(&path).expect("a listening socket takes the connection");

        drop(listener);
        let started = Instant::now();
        loop {
            match connect(&path) {
                Err(e) if e.kind() == ErrorKind::ConnectionRefused => break,
                Err(e) => panic!("expected ConnectionRefused, got {e}"),
                Ok(_) => assert!(started.elapsed() < Duration::from_secs(1), "the socket still listens"),
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[tokio::test]
    async fn a_socket_whose_file_denies_write_data_refuses_the_connect() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clawg.sock");
        let listener = bind_socket(&path).expect("bind");
        let server = tokio::spawn(std::future::IntoFuture::into_future(axum::serve(
            listener,
            router(test_app(dir.path()).await),
        )));
        write_state(
            dir.path(),
            &DaemonState {
                pid: 1,
                port: None,
                unix_socket: true,
                profile: "dev".into(),
                version: "0.1.0".into(),
                started_at_ms: 0,
            },
        )
        .expect("write state");
        write_token(dir.path(), "socket-token").expect("write token");
        let client = DaemonClient::from_data_dir(dir.path()).expect("client");
        let who = account();

        assert!(icacls(&[path.as_os_str(), "/deny".as_ref(), format!("{who}:(WD)").as_ref()]), "deny");
        let refused = connect(&path);
        let health = client.health().await;
        // Undone before anything can fail, so the directory can be removed.
        assert!(icacls(&[path.as_os_str(), "/remove:d".as_ref(), who.as_ref()]), "remove the deny");

        let err = refused.expect_err("the deny refuses the connect");
        assert_eq!(err.raw_os_error(), Some(10013), "{err}");
        assert!(matches!(health, Err(ClientError::NotRunning)), "a refused connect reads as not running: {health:?}");
        connect(&path).expect("the connect enters once the deny is gone");
        server.abort();
    }

    #[tokio::test]
    async fn the_socket_file_inherits_the_data_directory_acl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clawg.sock");
        let who = account();

        // Inherit-only: it applies to what the directory will hold, not to the
        // directory, which the socket still has to be created in.
        assert!(
            icacls(&[dir.path().as_os_str(), "/deny".as_ref(), format!("{who}:(OI)(CI)(IO)(WD)").as_ref()]),
            "deny on the directory"
        );
        let bound = bind_socket(&path).map(|listener| (listener, connect(&path)));
        assert!(icacls(&[dir.path().as_os_str(), "/remove:d".as_ref(), who.as_ref()]), "remove the deny");
        assert!(icacls(&[path.as_os_str(), "/remove:d".as_ref(), who.as_ref()]), "remove the inherited deny");

        let (_listener, refused) = bound.expect("bind");
        let err = refused.expect_err("the entry the directory passed down refuses the connect");
        assert_eq!(err.raw_os_error(), Some(10013), "{err}");
        connect(&path).expect("the connect enters once the entry is gone");
    }

    #[test]
    fn only_an_address_family_without_a_provider_lacks_af_unix() {
        let code = |raw| lacks_af_unix(&std::io::Error::from_raw_os_error(raw));
        assert!(code(10047), "WSAEAFNOSUPPORT");
        assert!(!code(10013), "WSAEACCES");
        assert!(!code(10048), "WSAEADDRINUSE");
        assert!(!code(10022), "WSAEINVAL");
    }
}
