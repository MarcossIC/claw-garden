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
pub mod protocol;
pub mod roots;
pub mod server;
pub mod startup;
pub mod tools;
pub mod tools_es;

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
use roots::Start;
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
        Start::of_process(),
        AsyncRwTransport::new_server(stdin, stdout),
    )
    .await
}

/// [`run`] with the data directory, spawner, timings, start and transport
/// supplied, so a test can drive the whole bridge without a real process, a
/// real working directory or real stdio.
///
/// Before the transport opens only the quick checks run (see
/// [`probe_running`]), so a store held by a non-daemon or `--no-autostart`
/// without a daemon still fail at once. The slow part, spawning and polling
/// until the daemon answers, runs in a task started here so the daemon is
/// already warming when the first tool arrives.
#[expect(clippy::too_many_arguments, reason = "the parameters have no type that groups them; adding one is a refactor of its own")]
pub(crate) async fn run_with<T>(
    profile: Profile,
    base_dir: &Path,
    autostart: bool,
    agent: Option<String>,
    spawner: Arc<dyn DaemonSpawner>,
    start_timeout: Duration,
    tool_wait: Duration,
    start: Start,
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
    // Read once: a session keeps the language it was opened in, its
    // instructions and its answers alike.
    let service = ClawgMcp::pending(client, profile, start)
        .with_agent(agent)
        .with_language(clawg_core::preferences::language_in(base_dir))
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
                imports: Default::default(),
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

    /// An in-process daemon whose files are already in its data directory, as
    /// a running daemon's are, so a bridge reaches it without starting one.
    struct RunningDaemon {
        dir: tempfile::TempDir,
    }
    impl RunningDaemon {
        async fn start() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let (runtime, rx) = MemoryRuntime::new_for_test(dir.path().to_path_buf()).await;
            tokio::spawn(runtime.ingest_worker(rx, tokio::sync::watch::channel(None).1));
            let token = "running-daemon-token".to_string();
            let (shutdown, _rx) = tokio::sync::watch::channel(false);
            let ctx = Arc::new(AppCtx {
                runtime,
                token: token.clone(),
                profile: Profile::Dev,
                started_at: Instant::now(),
                shutdown,
                save_timeout: SAVE_CONFIRM_TIMEOUT,
                sessions: Default::default(),
                imports: Default::default(),
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let port = listener.local_addr().expect("addr").port();
            tokio::spawn(async move {
                let _ = axum::serve(listener, router(ctx)).await;
            });
            let state = DaemonState {
                pid: std::process::id(),
                port: Some(port),
                unix_socket: false,
                profile: "dev".into(),
                version: "0.1.0".into(),
                started_at_ms: 0,
            };
            write_state(dir.path(), &state).expect("write state");
            write_token(dir.path(), &token).expect("write token");
            Self { dir }
        }

        fn client(&self) -> crate::client::DaemonClient {
            crate::client::DaemonClient::from_data_dir(self.dir.path()).expect("the daemon's files")
        }
    }

    type HostLines = tokio::io::Lines<BufReader<ReadHalf<DuplexStream>>>;

    /// The host side of an in-memory stdio pipe wired to a running bridge: an
    /// MCP client that declares `roots` when it has some, and answers each
    /// `roots/list` with the ones it has at that moment.
    struct Host {
        write: WriteHalf<DuplexStream>,
        lines: HostLines,
        bridge: JoinHandle<Result<(), McpError>>,
        /// `None`: the client does not declare the capability, and being
        /// asked for its roots fails the test.
        roots: Option<Vec<String>>,
        next_id: u32,
    }

    fn connect(
        base_dir: &Path,
        autostart: bool,
        spawner: Arc<dyn DaemonSpawner>,
        start_timeout: Duration,
        tool_wait: Duration,
    ) -> Host {
        connect_with(base_dir, autostart, spawner, start_timeout, tool_wait, Start::default(), None)
    }

    fn connect_with(
        base_dir: &Path,
        autostart: bool,
        spawner: Arc<dyn DaemonSpawner>,
        start_timeout: Duration,
        tool_wait: Duration,
        start: Start,
        roots: Option<Vec<String>>,
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
                start,
                AsyncRwTransport::new_server(server_read, server_write),
            )
            .await
        });
        let (host_read, write) = tokio::io::split(host);
        Host { write, lines: BufReader::new(host_read).lines(), bridge, roots, next_id: 100 }
    }

    /// A bridge started in `start`, with `home` as the user's home directory,
    /// against `daemon`, its session open; its client declares `roots`.
    async fn open(daemon: &RunningDaemon, start: &Path, home: &Path, roots: Option<Vec<String>>) -> Host {
        let start = Start { dir: Some(start.to_path_buf()), home: Some(home.to_path_buf()) };
        let mut host =
            connect_with(daemon.dir.path(), false, Arc::new(CannotSpawn), Duration::from_secs(5), Duration::from_secs(5), start, roots);
        host.initialize(Duration::from_secs(5)).await;
        host
    }

    /// A home directory with three repositories in it, by canonical path.
    struct Folders {
        _dir: tempfile::TempDir,
        home: PathBuf,
        a: PathBuf,
        b: PathBuf,
        c: PathBuf,
    }

    fn folders() -> Folders {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let home = clawg_core::memory::identity::strip_verbatim(&base).join("home");
        let repo = |name: &str| {
            let path = home.join(name);
            std::fs::create_dir_all(path.join(".git")).expect(".git");
            path
        };
        let (a, b, c) = (repo("a"), repo("b"), repo("c"));
        Folders { _dir: dir, home, a, b, c }
    }

    /// A folder as a root of the client, as Cursor sends it: a plain path.
    fn root(path: &Path) -> String {
        path.display().to_string()
    }

    /// `path` as a `file:` URI, every byte but the unreserved ones and `/`
    /// percent-encoded, as VS Code writes them.
    fn file_uri(path: &Path) -> String {
        let text = path.display().to_string().replace('\\', "/");
        let text = if text.starts_with('/') { text } else { format!("/{text}") };
        let mut uri = String::from("file://");
        for byte in text.bytes() {
            if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
                uri.push(char::from(byte));
            } else {
                uri.push_str(&format!("%{byte:02X}"));
            }
        }
        uri
    }

    /// The id the daemon gives a repository with no remote: the one derived
    /// from its root.
    fn project_id(path: &Path) -> String {
        clawg_core::memory::ProjectIdentity::resolve(path).id
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
            let capabilities = match self.roots {
                Some(_) => json!({ "roots": { "listChanged": false } }),
                None => json!({}),
            };
            self.send(json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": capabilities,
                    "clientInfo": { "name": "probe-host", "version": "0" }
                }
            }))
            .await;
            let reply = self.next(budget).await;
            self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
            reply
        }

        /// Calls `tool` and returns its answer, answering every `roots/list`
        /// the bridge sends meanwhile with the roots of the moment.
        async fn call(&mut self, tool: &str, arguments: Value) -> Value {
            self.next_id += 1;
            let id = self.next_id;
            self.send(json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": { "name": tool, "arguments": arguments }
            }))
            .await;
            loop {
                let frame = self.next(Duration::from_secs(20)).await;
                if frame["method"] == "roots/list" {
                    let roots = self
                        .roots
                        .clone()
                        .unwrap_or_else(|| panic!("the bridge asked for roots the client never declared: {frame}"));
                    let listed: Vec<Value> = roots.iter().map(|uri| json!({ "uri": uri })).collect();
                    self.send(json!({ "jsonrpc": "2.0", "id": frame["id"].clone(), "result": { "roots": listed } }))
                        .await;
                } else if frame["id"] == id {
                    return frame;
                }
            }
        }

        /// Saves `content` in the project of `folder`.
        async fn save_in(&mut self, folder: &Path, content: &str) {
            let reply = self.call("save_memory", json!({ "content": content, "project_path": root(folder) })).await;
            assert!(!tool_failed(&reply), "the save must be stored: {reply}");
        }

        /// What `search_memory` answers for `query`, with no project named.
        async fn search(&mut self, query: &str) -> String {
            let reply = self.call("search_memory", json!({ "query": query })).await;
            assert!(!tool_failed(&reply), "{reply}");
            tool_text(&reply)
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

    // ── where a call without a project goes ─────────────────────────────────

    const IN_A: &str = "the staging port of the first repository is 48213";
    const IN_B: &str = "the second repository builds its docs with mdbook";
    const IN_C: &str = "the third repository pins its toolchain to nightly";

    /// How many memories the store holds, in any project.
    async fn stored(daemon: &RunningDaemon) -> usize {
        let listing = crate::client::ListArgs { kind: Some("memory".into()), ..Default::default() };
        daemon.client().list(listing).await.expect("list").len()
    }

    #[tokio::test]
    async fn a_save_without_a_project_is_refused_with_the_roots_as_options() {
        let (daemon, f) = (RunningDaemon::start().await, folders());
        let mut host = open(&daemon, &f.home, &f.home, Some(vec![root(&f.a), root(&f.b)])).await;

        let reply = host.call("save_memory", json!({ "content": IN_A })).await;

        let text = tool_text(&reply);
        assert!(tool_failed(&reply), "{reply}");
        for named in ["project_path", "project", "all_projects", &root(&f.a), &root(&f.b)] {
            assert!(text.contains(named), "the refusal names {named}: {text}");
        }
        assert_eq!(stored(&daemon).await, 0, "nothing is saved");
    }

    #[tokio::test]
    async fn a_save_without_a_project_is_refused_without_roots_too() {
        let (daemon, f) = (RunningDaemon::start().await, folders());
        let mut host = open(&daemon, &f.a, &f.home, None).await;

        let reply = host.call("save_memory", json!({ "content": IN_A })).await;

        let text = tool_text(&reply);
        assert!(tool_failed(&reply), "{reply}");
        assert!(text.contains("project_path") && text.contains(&root(&f.a)), "the start directory is the option: {text}");

        let mut at_home = open(&daemon, &f.home, &f.home, None).await;
        let reply = at_home.call("save_memory", json!({ "content": IN_A })).await;
        let text = tool_text(&reply);
        assert!(tool_failed(&reply), "{reply}");
        assert!(text.contains("all_projects") && !text.contains(&root(&f.home)), "home is never an option: {text}");
        assert_eq!(stored(&daemon).await, 0, "nothing is saved");
    }

    #[tokio::test]
    async fn a_search_with_one_root_reads_its_project() {
        let (daemon, f) = (RunningDaemon::start().await, folders());
        let mut host = open(&daemon, &f.home, &f.home, Some(vec![root(&f.a)])).await;
        host.save_in(&f.a, IN_A).await;
        host.save_in(&f.b, IN_B).await;

        assert!(host.search(IN_A).await.contains(IN_A), "the root's project is searched");
        let other = host.search(IN_B).await;
        assert!(!other.contains(IN_B), "another project is not: {other}");
    }

    #[tokio::test]
    async fn a_search_with_several_roots_reads_all_of_them_and_names_each_project() {
        let (daemon, f) = (RunningDaemon::start().await, folders());
        let mut host = open(&daemon, &f.home, &f.home, Some(vec![root(&f.a), root(&f.b)])).await;
        host.save_in(&f.a, IN_A).await;
        host.save_in(&f.b, IN_B).await;
        host.save_in(&f.c, IN_C).await;

        let found = host.search(IN_A).await;
        assert!(found.contains(IN_A) && found.contains(&format!(" project={}", project_id(&f.a))), "{found}");
        let found = host.search(IN_B).await;
        assert!(found.contains(IN_B) && found.contains(&format!(" project={}", project_id(&f.b))), "{found}");
        let outside = host.search(IN_C).await;
        assert!(!outside.contains(IN_C), "a project no root leads to is not searched: {outside}");
    }

    #[tokio::test]
    async fn a_search_without_roots_reads_the_start_directory() {
        let (daemon, f) = (RunningDaemon::start().await, folders());
        let mut host = open(&daemon, &f.a, &f.home, None).await;
        host.save_in(&f.a, IN_A).await;
        host.save_in(&f.b, IN_B).await;

        assert!(host.search(IN_A).await.contains(IN_A), "the start directory's project is searched");
        let other = host.search(IN_B).await;
        assert!(!other.contains(IN_B), "another project is not: {other}");
    }

    #[tokio::test]
    async fn the_home_directory_is_never_a_project() {
        let (daemon, f) = (RunningDaemon::start().await, folders());
        let mut seeding = open(&daemon, &f.a, &f.home, None).await;
        seeding.save_in(&f.a, IN_A).await;
        seeding.save_in(&f.b, IN_B).await;

        let mut at_home = open(&daemon, &f.home, &f.home, None).await;
        assert!(at_home.search(IN_A).await.contains(IN_A), "without a project a search reaches every project");

        let mut home_as_root = open(&daemon, &f.a, &f.home, Some(vec![root(&f.home), root(&f.b)])).await;
        assert!(home_as_root.search(IN_B).await.contains(IN_B), "the other root is searched");
        let not_home = home_as_root.search(IN_A).await;
        assert!(!not_home.contains(IN_A), "a home root is dropped, not read as its project or as none: {not_home}");

        let listing = daemon.client().list_projects().await.expect("projects");
        let folders: Vec<&str> = listing.projects.iter().flat_map(|p| p.folders.iter().map(|f| f.root_path.as_str())).collect();
        assert!(!folders.iter().any(|path| Path::new(path) == f.home), "home was registered: {folders:?}");
    }

    #[tokio::test]
    async fn a_root_as_a_path_and_as_a_file_uri_is_the_same_project() {
        let (daemon, f) = (RunningDaemon::start().await, folders());
        let mut seeding = open(&daemon, &f.a, &f.home, None).await;
        seeding.save_in(&f.a, IN_A).await;
        seeding.save_in(&f.b, IN_B).await;

        // Cursor sends a Windows root with the drive in lower case, and VS
        // Code a URI with the colon encoded.
        let lower_drive = |text: String| match text.as_bytes() {
            [drive, b':', ..] if cfg!(windows) => format!("{}{}", char::from(drive.to_ascii_lowercase()), &text[1..]),
            _ => text,
        };
        let as_uri = file_uri(Path::new(&lower_drive(root(&f.a))));
        for declared in [lower_drive(root(&f.a)), as_uri] {
            let mut host = open(&daemon, &f.home, &f.home, Some(vec![declared.clone()])).await;
            assert!(host.search(IN_A).await.contains(IN_A), "{declared} leads to the project of {}", f.a.display());
            let other = host.search(IN_B).await;
            assert!(!other.contains(IN_B), "{declared} leads to one project: {other}");
        }
    }

    #[tokio::test]
    async fn a_change_of_roots_between_two_calls_is_seen_in_the_second() {
        let (daemon, f) = (RunningDaemon::start().await, folders());
        let mut host = open(&daemon, &f.home, &f.home, Some(vec![root(&f.a)])).await;
        host.save_in(&f.a, IN_A).await;
        host.save_in(&f.b, IN_B).await;
        assert!(host.search(IN_A).await.contains(IN_A));

        // No `notifications/roots/list_changed`, as Cursor sends none.
        host.roots = Some(vec![root(&f.b)]);

        assert!(host.search(IN_B).await.contains(IN_B), "the second call reads the new root");
        let before = host.search(IN_A).await;
        assert!(!before.contains(IN_A), "and not the old one: {before}");
    }
}
