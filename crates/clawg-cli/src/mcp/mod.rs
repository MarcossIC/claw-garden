//! `clawg mcp` — the stdio bridge between an MCP host and a running daemon.
//!
//! # stdout is the transport
//!
//! Every byte this process writes to stdout is an MCP frame. A stray `println!`
//! — a progress line, a warning, a panic message — corrupts the stream and the
//! host disconnects with an error that points nowhere near the cause. So
//! logging is initialised against stderr in `main`, and nothing in this module
//! or in [`server`] ever writes to stdout directly.

pub mod handshake;
pub mod server;
pub mod startup;
pub mod tools;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use clawg_core::paths::{data_dir, PathsError, Profile};
use rmcp::transport::async_rw::AsyncRwTransport;
use rmcp::transport::{stdio, Transport};
use rmcp::{RoleServer, ServiceExt};

use crate::client::{
    probe_running, start_daemon, ClientError, DaemonSpawner, Probe, ProcessSpawner, AUTOSTART_TIMEOUT,
};
use handshake::PreInitGuard;
use server::ClawgMcp;
use startup::{ClientHandle, Startup};

/// How long one tool call waits for a daemon that is still starting.
///
/// Under the host's ~30 s limit, so a call that cannot be served ends with the
/// readable "start one with `clawg serve`" answer instead of a host-side
/// timeout that reports nothing. The start itself keeps running past this (it
/// has [`AUTOSTART_TIMEOUT`]), so a retry a little later can still succeed.
const TOOL_READY_WAIT: Duration = Duration::from_secs(25);

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Paths(#[from] PathsError),
    #[error("the MCP stdio server failed: {0}")]
    Serve(String),
}

/// Serves MCP over stdio until the host closes the stream, starting a daemon in
/// the background if none is running.
pub async fn run(profile: Profile, autostart: bool, agent: Option<String>) -> Result<(), McpError> {
    let base_dir = data_dir(profile)?;
    let spawner = Arc::new(ProcessSpawner { base_dir: base_dir.clone() });
    let (stdin, stdout) = stdio();
    run_with(
        profile,
        &base_dir,
        autostart,
        agent,
        spawner,
        AUTOSTART_TIMEOUT,
        TOOL_READY_WAIT,
        AsyncRwTransport::new_server(stdin, stdout),
    )
    .await
}

/// [`run`] with the data directory, spawner, timings and transport supplied, so
/// a test can drive the whole bridge without a real process or real stdio.
///
/// Before the transport opens only the quick checks run (see
/// [`probe_running`]), so a store held by a non-daemon or `--no-autostart`
/// without a daemon still fail at once. The slow part, spawning and polling
/// until the daemon answers, runs in a task started here so the daemon is
/// already warming when the first tool arrives.
pub(crate) async fn run_with<T>(
    profile: Profile,
    base_dir: &Path,
    autostart: bool,
    agent: Option<String>,
    spawner: Arc<dyn DaemonSpawner>,
    start_timeout: Duration,
    tool_wait: Duration,
    transport: T,
) -> Result<(), McpError>
where
    T: Transport<RoleServer> + Send + 'static,
{
    let client = match probe_running(base_dir, autostart).await? {
        Probe::Running(client) => {
            log::info!(
                "clawg mcp {} bridging to {}",
                profile.as_str(),
                client.base_url()
            );
            ClientHandle::ready(client)
        }
        Probe::NeedsStart => {
            let (outcome, handle) = ClientHandle::pending(tool_wait, base_dir.to_path_buf());
            let base_dir = base_dir.to_path_buf();
            tokio::spawn(async move {
                let state = match start_daemon(&base_dir, profile, &*spawner, start_timeout).await {
                    Ok(client) => {
                        log::info!(
                            "clawg mcp {} bridging to {}",
                            profile.as_str(),
                            client.base_url()
                        );
                        Startup::Ready(client)
                    }
                    Err(e) => {
                        log::warn!("the daemon did not start: {e}");
                        Startup::Failed(Arc::new(e))
                    }
                };
                // No receiver left means the bridge already exited.
                let _ = outcome.send(state);
            });
            handle
        }
    };

    // Guarded, never bare: a host that cancels its `server/discover` probe
    // before a session exists must not end the process.
    let transport = PreInitGuard::new(transport);
    let service = ClawgMcp::pending(client, profile)
        .with_agent(agent)
        .serve(transport)
        .await
        .map_err(|e| McpError::Serve(e.to_string()))?;

    // Returns when the host disconnects; the daemon keeps running, which is the
    // point of the split — closing an editor must not discard the memory index.
    service
        .waiting()
        .await
        .map_err(|e| McpError::Serve(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    use clawg_core::memory::store_lock::{StoreHolder, StoreLock};
    use clawg_core::runtime::MemoryRuntime;
    use serde_json::{json, Value};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};
    use tokio::task::JoinHandle;

    use crate::daemon::api::{router, AppCtx, SAVE_CONFIRM_TIMEOUT};
    use crate::daemon::lifecycle::{write_state, write_token, DaemonState};

    /// Spawns fine and then nothing ever answers: a cold start that never ends.
    struct NeverReady {
        calls: AtomicUsize,
    }
    impl NeverReady {
        fn new() -> Arc<Self> {
            Arc::new(Self { calls: AtomicUsize::new(0) })
        }
    }
    impl DaemonSpawner for NeverReady {
        fn spawn_serve(&self, _profile: Profile) -> Result<(), ClientError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// The process could not even be launched.
    struct CannotSpawn;
    impl DaemonSpawner for CannotSpawn {
        fn spawn_serve(&self, _profile: Profile) -> Result<(), ClientError> {
            Err(ClientError::Spawn("boom".to_string()))
        }
    }

    /// An in-process daemon that only publishes its lifecycle files a while
    /// after being asked to start, like a real cold start does.
    struct SlowDaemon {
        dir: tempfile::TempDir,
        addr: SocketAddr,
        token: String,
        delay: Duration,
    }
    impl SlowDaemon {
        async fn new(delay: Duration) -> Arc<Self> {
            let dir = tempfile::tempdir().expect("tempdir");
            let (runtime, rx) = MemoryRuntime::new_for_test(dir.path().to_path_buf()).await;
            tokio::spawn(runtime.ingest_worker(rx, tokio::sync::watch::channel(None).1));
            let token = "slow-daemon-token".to_string();
            let (shutdown, _rx) = tokio::sync::watch::channel(false);
            let ctx = Arc::new(AppCtx {
                runtime,
                token: token.clone(),
                profile: Profile::Dev,
                started_at: Instant::now(),
                shutdown,
                save_timeout: SAVE_CONFIRM_TIMEOUT,
                sessions: Default::default(),
                metrics: Default::default(),
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let addr = listener.local_addr().expect("addr");
            tokio::spawn(async move {
                let _ = axum::serve(listener, router(ctx)).await;
            });
            Arc::new(Self { dir, addr, token, delay })
        }
    }
    impl DaemonSpawner for SlowDaemon {
        fn spawn_serve(&self, _profile: Profile) -> Result<(), ClientError> {
            let dir = self.dir.path().to_path_buf();
            let (port, token, delay) = (self.addr.port(), self.token.clone(), self.delay);
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                write_state(
                    &dir,
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
                write_token(&dir, &token).expect("write token");
            });
            Ok(())
        }
    }

    type HostLines = tokio::io::Lines<BufReader<ReadHalf<DuplexStream>>>;

    /// The host side of an in-memory stdio pipe wired to a running bridge.
    struct Host {
        write: WriteHalf<DuplexStream>,
        lines: HostLines,
        bridge: JoinHandle<Result<(), McpError>>,
    }

    fn connect(
        base_dir: &Path,
        autostart: bool,
        spawner: Arc<dyn DaemonSpawner>,
        start_timeout: Duration,
        tool_wait: Duration,
    ) -> Host {
        let (host, server) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server);
        let base_dir: PathBuf = base_dir.to_path_buf();
        let bridge = tokio::spawn(async move {
            run_with(
                Profile::Dev,
                &base_dir,
                autostart,
                None,
                spawner,
                start_timeout,
                tool_wait,
                AsyncRwTransport::new_server(server_read, server_write),
            )
            .await
        });
        let (host_read, write) = tokio::io::split(host);
        Host { write, lines: BufReader::new(host_read).lines(), bridge }
    }

    impl Host {
        async fn send(&mut self, frame: Value) {
            self.write
                .write_all(format!("{frame}\n").as_bytes())
                .await
                .expect("host write");
        }

        /// The bridge's next frame, or a panic naming the budget that ran out.
        async fn next(&mut self, budget: Duration) -> Value {
            let line = tokio::time::timeout(budget, self.lines.next_line())
                .await
                .unwrap_or_else(|_| panic!("the bridge did not answer within {budget:?}"))
                .expect("readable stream")
                .expect("the bridge is still alive");
            serde_json::from_str(&line).expect("a JSON-RPC line")
        }

        async fn initialize(&mut self, budget: Duration) -> Value {
            self.send(json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "probe-host", "version": "0" }
                }
            }))
            .await;
            let reply = self.next(budget).await;
            self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
            reply
        }

        async fn memory_status(&mut self, id: u32, budget: Duration) -> Value {
            self.send(json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": { "name": "memory_status", "arguments": {} }
            }))
            .await;
            self.next(budget).await
        }
    }

    fn tool_text(reply: &Value) -> String {
        reply["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
    }

    fn tool_failed(reply: &Value) -> bool {
        reply["result"]["isError"].as_bool().unwrap_or(false)
    }

    /// The host cuts the connect at about 30 s, so the handshake must not wait
    /// on a daemon cold start: it needs none of the daemon's data.
    #[tokio::test]
    async fn initialize_is_answered_while_the_daemon_is_still_starting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut host = connect(
            dir.path(),
            true,
            NeverReady::new(),
            Duration::from_secs(30),
            Duration::from_secs(30),
        );

        let reply = host.initialize(Duration::from_secs(1)).await;
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["serverInfo"]["name"], "clawg-dev");
    }

    #[tokio::test]
    async fn a_tool_call_during_a_slow_start_waits_and_then_succeeds() {
        let daemon = SlowDaemon::new(Duration::from_millis(1_200)).await;
        let base = daemon.dir.path().to_path_buf();
        let mut host = connect(
            &base,
            true,
            daemon,
            Duration::from_secs(20),
            Duration::from_secs(15),
        );

        host.initialize(Duration::from_secs(1)).await;
        let reply = host.memory_status(2, Duration::from_secs(15)).await;
        assert!(!tool_failed(&reply), "the call must succeed once the daemon is up: {reply}");
        assert!(tool_text(&reply).contains("\"status\""), "the health JSON must be there: {reply}");
    }

    #[tokio::test]
    async fn a_tool_call_when_the_daemon_never_comes_up_gives_the_readable_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut host = connect(
            dir.path(),
            true,
            NeverReady::new(),
            Duration::from_secs(30),
            Duration::from_millis(300),
        );

        host.initialize(Duration::from_secs(1)).await;
        let first = host.memory_status(2, Duration::from_secs(5)).await;
        assert!(tool_failed(&first), "{first}");
        assert!(tool_text(&first).contains("clawg status"), "must say where the cause is: {first}");

        let second = host.memory_status(3, Duration::from_secs(5)).await;
        assert!(tool_failed(&second), "the bridge must survive the first failure: {second}");
    }

    #[tokio::test]
    async fn a_tool_call_after_the_start_failed_gives_the_readable_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut host = connect(
            dir.path(),
            true,
            Arc::new(CannotSpawn),
            Duration::from_secs(30),
            Duration::from_secs(10),
        );

        host.initialize(Duration::from_secs(1)).await;
        let reply = host.memory_status(2, Duration::from_secs(5)).await;
        assert!(tool_failed(&reply), "{reply}");
        assert!(tool_text(&reply).contains("boom"), "the cause must reach the model: {reply}");
    }

    /// A store held by a non-daemon is a fast, certain "no": the bridge keeps
    /// failing before it ever opens stdio rather than starting degraded.
    #[tokio::test]
    async fn a_store_held_by_a_non_daemon_fails_before_stdio_opens() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _holder = StoreLock::acquire(dir.path(), StoreHolder::Eval).expect("holder has it");
        let spawner = NeverReady::new();
        let host = connect(
            dir.path(),
            true,
            spawner.clone(),
            Duration::from_secs(30),
            Duration::from_secs(30),
        );

        let outcome = tokio::time::timeout(Duration::from_secs(1), host.bridge)
            .await
            .expect("the bridge must give up at once, without waiting for the host")
            .expect("no panic");
        assert!(matches!(outcome, Err(McpError::Client(ClientError::StoreHeld(_)))), "{outcome:?}");
        assert_eq!(spawner.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn no_autostart_without_a_daemon_is_not_running_at_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let spawner = NeverReady::new();
        let host = connect(
            dir.path(),
            false,
            spawner.clone(),
            Duration::from_secs(30),
            Duration::from_secs(30),
        );

        let outcome = tokio::time::timeout(Duration::from_secs(1), host.bridge)
            .await
            .expect("the bridge must give up at once")
            .expect("no panic");
        assert!(matches!(outcome, Err(McpError::Client(ClientError::NotRunning))), "{outcome:?}");
        assert_eq!(spawner.calls.load(Ordering::SeqCst), 0, "--no-autostart never spawns");
    }
}
