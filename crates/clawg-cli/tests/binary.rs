//! The `clawg` binary itself, run as child processes.
//!
//! Everything in `api.rs` runs the daemon inside the test process, so what
//! only exists between processes goes unseen there: a hard kill, the store
//! lock between two daemons, the files a bridge reads to find its daemon, and
//! MCP over a real stdin and stdout. Each test here gets its own home through
//! `CLAWG_TEST_HOME` and the fake model through `CLAWG_TEST_EMBEDDER`, which
//! only a `testing` build (this one) reads. Every daemon the tests start binds
//! `--port 0`, and one started without `--port` (by a bridge, by `update`)
//! keeps to its socket (`CLAWG_TEST_DEFAULT_PORT=off`).
//!
//! Children are ended through their own handle when their guard drops, also
//! when a test panics, and never by name: a `clawg` the user's editor runs
//! must survive the suite.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Output, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use clawg_cli::client::{Call, DaemonClient};
use serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_clawg");

/// Generous: a cold debug binary on a loaded CI runner opens SQLite, runs
/// every migration and binds before it answers.
const START_BUDGET: Duration = Duration::from_secs(60);

const REPLY_BUDGET: Duration = Duration::from_secs(30);

/// A temporary home with its `.clawg-dev` data directory.
struct Sandbox {
    home: PathBuf,
    _owned: Option<tempfile::TempDir>,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        Self { home: dir.path().to_path_buf(), _owned: Some(dir) }
    }

    /// A second view of a home another sandbox owns.
    fn at(home: &Path) -> Self {
        Self { home: home.to_path_buf(), _owned: None }
    }

    fn data_dir(&self) -> PathBuf {
        self.home.join(".clawg-dev")
    }

    /// Where the children's stderr goes, outside the data directory so a
    /// restore never touches it.
    fn log(&self, name: &str) -> PathBuf {
        self.home.join(format!("{name}.log"))
    }

    fn command(&self, args: &[&str]) -> Command {
        self.command_of(Path::new(BIN), args)
    }

    fn command_of(&self, bin: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(bin);
        cmd.args(args)
            .env("CLAWG_TEST_HOME", &self.home)
            .env("CLAWG_TEST_EMBEDDER", "fake")
            .env_remove("CLAWG_PROFILE")
            .env_remove("CLAWG_TEST_DEFAULT_PORT")
            .env("RUST_LOG", "info")
            .current_dir(&self.home)
            .stdin(Stdio::null())
            .envs(socket_only());
        cmd
    }

    /// Runs a command that ends on its own.
    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("run clawg")
    }

    async fn serve(&self) -> Daemon {
        self.serve_with("fake").await
    }

    async fn serve_with(&self, embedder: &str) -> Daemon {
        self.serve_binary(Path::new(BIN), embedder).await
    }

    /// A daemon run from the copy of the binary at `bin`.
    async fn serve_binary(&self, bin: &Path, embedder: &str) -> Daemon {
        let log = self.log(&format!("serve-{}", now_ns()));
        let child = self
            .command_of(bin, &["serve", "--profile", "dev", "--port", "0"])
            .env("CLAWG_TEST_EMBEDDER", embedder)
            .stdout(Stdio::null())
            .stderr(File::create(&log).expect("log file"))
            .spawn_copy()
            .expect("spawn clawg serve");
        Daemon::watch(child, log).answering(&self.data_dir()).await
    }

    /// A daemon run as `args` say, `--port` or not, with `env` on top.
    async fn serve_as(&self, args: &[&str], env: &[(&str, &str)]) -> Daemon {
        let log = self.log(&format!("serve-{}", now_ns()));
        let child = self
            .command(args)
            .envs(env.iter().copied())
            .stdout(Stdio::null())
            .stderr(File::create(&log).expect("log file"))
            .spawn()
            .expect("spawn clawg serve");
        Daemon::watch(child, log).answering(&self.data_dir()).await
    }

    /// A bridge started in an empty folder of its own, so the root it
    /// registers has nothing to index.
    fn bridge(&self) -> Bridge {
        self.bridge_of(Path::new(BIN))
    }

    /// A bridge run from the copy of the binary at `bin`.
    fn bridge_of(&self, bin: &Path) -> Bridge {
        self.bridge_with(bin, &["mcp", "--profile", "dev", "--no-autostart", "--agent", "smoke"])
    }

    /// A bridge that starts a daemon when none answers. That daemon is no
    /// child of the test: a `StopOnDrop` ends it.
    fn bridge_starting_its_daemon(&self) -> Bridge {
        self.bridge_with(Path::new(BIN), &["mcp", "--profile", "dev", "--agent", "smoke"])
    }

    fn bridge_with(&self, bin: &Path, args: &[&str]) -> Bridge {
        let root = self.home.join(format!("project-{}", now_ns()));
        std::fs::create_dir_all(&root).expect("project dir");
        let log = self.log(&format!("mcp-{}", now_ns()));
        let mut child = self
            .command_of(bin, args)
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(File::create(&log).expect("log file"))
            .spawn_copy()
            .expect("spawn clawg mcp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Bridge { child, stdin, lines, log }
    }
}

/// What keeps a daemon started without `--port` (by a bridge, by `update`)
/// to its socket instead of the profile's port on this machine.
fn socket_only() -> Vec<(&'static str, &'static str)> {
    vec![("CLAWG_TEST_DEFAULT_PORT", "off")]
}

fn now_ns() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A `clawg serve` child.
///
/// A thread reaps it as soon as it exits: on Unix an unreaped child stays a
/// zombie, and `clawg stop`, which polls the pid, would wait it out.
struct Daemon {
    child: Arc<Mutex<Child>>,
    pid: u32,
    log: PathBuf,
    token: String,
    client: Option<DaemonClient>,
}

impl Daemon {
    fn watch(child: Child, log: PathBuf) -> Self {
        let pid = child.id();
        let child = Arc::new(Mutex::new(child));
        let reaper = Arc::downgrade(&child);
        std::thread::spawn(move || {
            while let Some(child) = reaper.upgrade() {
                if !matches!(child.lock().expect("child lock").try_wait(), Ok(None)) {
                    return;
                }
                drop(child);
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        Self { child, pid, log, token: String::new(), client: None }
    }

    fn exit_status(&self) -> Option<ExitStatus> {
        self.child.lock().expect("child lock").try_wait().expect("try_wait")
    }

    fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Once `daemon.json` names this child and its `/health` answers.
    async fn answering(mut self, data_dir: &Path) -> Self {
        let deadline = Instant::now() + START_BUDGET;
        loop {
            if let Some(status) = self.exit_status() {
                panic!("clawg serve exited with {status} before answering:\n{}", self.log_text());
            }
            if let Some(client) = published(data_dir, self.pid) {
                if client.send(health_call()).await.is_ok_and(|a| a.status.is_success()) {
                    self.token = std::fs::read_to_string(data_dir.join("token")).unwrap_or_default().trim().to_string();
                    self.client = Some(client);
                    return self;
                }
            }
            assert!(Instant::now() < deadline, "clawg serve did not answer:\n{}", self.log_text());
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn client(&self) -> &DaemonClient {
        self.client.as_ref().expect("the daemon answered")
    }

    async fn health(&self) -> Value {
        let answer = self.client().send(health_call()).await.expect("health");
        serde_json::from_slice(&answer.body).expect("health json")
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let call = Call::new(reqwest::Method::POST, path).json(&body).expect("request body");
        let answer = self.client().send(call).await.expect("request");
        (answer.status.as_u16(), serde_json::from_slice(&answer.body).unwrap_or(Value::Null))
    }

    async fn list(&self) -> String {
        let call = Call::new(reqwest::Method::GET, "/v1/memories").query([("limit", "100")]);
        let answer = self.client().send(call).await.expect("list");
        assert!(answer.status.is_success(), "{}", answer.status);
        String::from_utf8(answer.body).expect("list body")
    }

    /// Until the model is loaded. `/health` answers before it is, and a save
    /// sent meanwhile is queued (202) instead of stored (201).
    async fn wait_until_ready(&self) {
        let deadline = Instant::now() + START_BUDGET;
        loop {
            let health = self.health().await;
            if health["embedder"] == "ready" {
                return;
            }
            assert!(Instant::now() < deadline, "the model did not load: {health}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Until no accepted save is still waiting to be stored.
    async fn wait_until_ingested(&self) {
        let deadline = Instant::now() + START_BUDGET;
        loop {
            let health = self.health().await;
            let pending = health["ingest"]["pending"].as_u64();
            if pending == Some(0) && health["embedder"] == "ready" {
                return;
            }
            assert!(Instant::now() < deadline, "saves still pending: {health}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Ends the process abruptly, with no shutdown of any kind: SIGKILL on
    /// Unix, TerminateProcess on Windows.
    fn kill(&self) {
        let mut child = self.child.lock().expect("child lock");
        let _ = child.kill();
        let _ = child.wait();
    }

    /// Until the process has ended.
    async fn wait_for_exit(&self, budget: Duration) -> ExitStatus {
        let deadline = Instant::now() + budget;
        loop {
            if let Some(status) = self.exit_status() {
                return status;
            }
            assert!(Instant::now() < deadline, "clawg serve is still running:\n{}", self.log_text());
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.kill();
    }
}

fn health_call() -> Call {
    Call::new(reqwest::Method::GET, "/health").without_bearer()
}

/// `daemon.json` as JSON, if it is there.
fn state_of(data_dir: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(data_dir.join("daemon.json")).ok()?).ok()
}

/// A client of the daemon, once `daemon.json` names `pid`: clawg's own, so it
/// goes by the socket when the daemon published one and by its port otherwise.
fn published(data_dir: &Path, pid: u32) -> Option<DaemonClient> {
    if state_of(data_dir)?["pid"].as_u64() != Some(u64::from(pid)) {
        return None;
    }
    DaemonClient::from_data_dir(data_dir)
}

/// A `clawg mcp` child with the test as its host.
struct Bridge {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    log: PathBuf,
}

impl Bridge {
    fn send(&mut self, frame: Value) {
        writeln!(self.stdin, "{frame}").expect("write to the bridge");
        self.stdin.flush().expect("flush");
    }

    /// The bridge's answer to request `id`. Every line on its stdout must be
    /// a JSON-RPC frame: anything else is a write that corrupts the protocol.
    fn reply(&mut self, id: &Value) -> Value {
        let deadline = Instant::now() + REPLY_BUDGET;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = self.lines.recv_timeout(left).unwrap_or_else(|_| {
                panic!(
                    "no answer to {id} from clawg mcp:\n{}",
                    std::fs::read_to_string(&self.log).unwrap_or_default()
                )
            });
            let frame: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("stdout carried a line that is not JSON-RPC ({e}): {line}"));
            if &frame["id"] == id {
                return frame;
            }
        }
    }

    fn request(&mut self, id: u32, method: &str, params: Value) -> Value {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        self.reply(&json!(id))
    }

    fn initialize(&mut self) -> Value {
        let reply = self.request(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "smoke-host", "version": "0" }
            }),
        );
        self.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        reply
    }

    fn call(&mut self, id: u32, tool: &str, arguments: Value) -> Value {
        self.request(id, "tools/call", json!({ "name": tool, "arguments": arguments }))
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `_meta` a 2026-07-28 host puts on every request instead of `initialize`.
fn modern_params() -> Value {
    json!({ "_meta": {
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": { "name": "smoke-host", "version": "0" }
    }})
}

fn tool_text(reply: &Value) -> String {
    reply["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

fn tool_failed(reply: &Value) -> bool {
    reply["result"]["isError"].as_bool().unwrap_or(false) || reply.get("error").is_some()
}

fn tool_names(reply: &Value) -> Vec<String> {
    reply["result"]["tools"]
        .as_array()
        .map(|tools| tools.iter().filter_map(|t| t["name"].as_str().map(String::from)).collect())
        .unwrap_or_default()
}

// ── Kill ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_save_accepted_before_a_hard_kill_is_stored_after_the_restart() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve_with("stalled").await;
    let (status, body) = daemon
        .post(
            "/v1/memories",
            json!({ "content": "the smoke test survives a hard kill", "agent_id": "smoke" }),
        )
        .await;
    assert_eq!(status, 202, "the model is still loading, so the save waits: {body}");

    daemon.kill();
    drop(daemon);

    let daemon = sandbox.serve().await;
    daemon.wait_until_ingested().await;
    assert!(daemon.list().await.contains("the smoke test survives a hard kill"));
    let (status, hits) = daemon
        .post(
            "/v1/search",
            json!({ "query": "the smoke test survives a hard kill", "agent_id": "smoke" }),
        )
        .await;
    assert_eq!(status, 200, "{hits}");
    assert!(hits.to_string().contains("the smoke test survives a hard kill"), "{hits}");
}

// ── Data directory mode ─────────────────────────────────────────────────────

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).expect("metadata").permissions().mode() & 0o777
}

#[tokio::test]
async fn a_daemon_names_its_version_first() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve().await;

    let log = daemon.log_text();
    let first = log.lines().next().unwrap_or_default();
    assert!(first.contains(&format!("clawg serve {} starting, profile dev", env!("CARGO_PKG_VERSION"))), "{log}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_daemon_on_a_fresh_home_leaves_a_private_data_directory() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve().await;

    assert_eq!(mode_of(&sandbox.data_dir()), 0o700);
    assert!(!daemon.log_text().contains("made it"), "a new directory is born private:\n{}", daemon.log_text());
}

#[cfg(unix)]
#[tokio::test]
async fn an_open_data_directory_is_tightened_and_keeps_its_rows() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let first = sandbox.serve().await;
    first.wait_until_ready().await;
    let (status, body) = first.post("/v1/memories", json!({ "content": "kept across the tightening", "agent_id": "smoke" })).await;
    assert_eq!(status, 201, "{body}");
    first.wait_until_ingested().await;
    assert_eq!(sandbox.run(&["stop", "--profile", "dev"]).status.code(), Some(0));
    first.wait_for_exit(Duration::from_secs(10)).await;
    std::fs::set_permissions(sandbox.data_dir(), std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let second = sandbox.serve().await;

    assert_eq!(mode_of(&sandbox.data_dir()), 0o700);
    assert!(second.log_text().contains("was 0755; made it 0700"), "{}", second.log_text());
    second.wait_until_ingested().await;
    assert!(second.list().await.contains("kept across the tightening"));
}

// ── Lock ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_second_daemon_on_the_same_data_directory_is_refused_and_names_the_first() {
    let sandbox = Sandbox::new();
    let first = sandbox.serve().await;

    let second = sandbox.run(&["serve", "--profile", "dev", "--port", "0"]);

    assert_eq!(second.status.code(), Some(1), "{}", stderr_of(&second));
    let err = stderr_of(&second);
    assert!(err.contains(&format!("pid {}", first.pid)), "the owner must be named: {err}");
    assert!(first.exit_status().is_none(), "the first daemon keeps running");
    assert!(first.health().await["status"].is_string());
}

// ── status and stop ────────────────────────────────────────────────────────

#[tokio::test]
async fn the_daemon_client_ignores_an_http_proxy() {
    let sandbox = Sandbox::new();
    let _daemon = sandbox.serve().await;
    let proxy = std::net::TcpListener::bind("127.0.0.1:0").expect("bind the stand-in proxy");
    proxy.set_nonblocking(true).expect("non-blocking accept");
    let url = format!("http://{}", proxy.local_addr().expect("proxy address"));

    let status = sandbox
        .command(&["status", "--profile", "dev"])
        .env("HTTP_PROXY", &url)
        .env("http_proxy", &url)
        .env("ALL_PROXY", &url)
        .env("all_proxy", &url)
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .output()
        .expect("run clawg status");

    assert_eq!(status.status.code(), Some(0), "{}", stderr_of(&status));
    let report: Value = serde_json::from_slice(&status.stdout).expect("status prints JSON");
    assert_eq!(report["running"], true);
    assert!(proxy.accept().is_err(), "the loopback traffic went to the proxy");
}

#[tokio::test]
async fn status_and_stop_follow_the_daemon_process() {
    let sandbox = Sandbox::new();
    let before = sandbox.run(&["status", "--profile", "dev"]);
    assert_eq!(before.status.code(), Some(1), "no daemon yet: {}", stdout_of(&before));

    let daemon = sandbox.serve().await;
    let running = sandbox.run(&["status", "--profile", "dev"]);
    assert_eq!(running.status.code(), Some(0), "{}", stderr_of(&running));
    let report: Value = serde_json::from_slice(&running.stdout).expect("status prints JSON");
    assert_eq!(report["running"], true);
    let data_dir = report["data_dir"].as_str().expect("data_dir");
    assert_eq!(Path::new(data_dir), sandbox.data_dir(), "the test home, never the user's");

    let stop = sandbox.run(&["stop", "--profile", "dev"]);
    assert_eq!(stop.status.code(), Some(0), "{}", stderr_of(&stop));
    assert!(stdout_of(&stop).contains(&format!("pid {}", daemon.pid)), "{}", stdout_of(&stop));
    let exit = daemon.wait_for_exit(Duration::from_secs(5)).await;
    assert!(exit.success(), "a requested stop is a clean exit: {exit}\n{}", daemon.log_text());
    assert!(!sandbox.data_dir().join("daemon.json").exists(), "the state file goes with it");

    let after = sandbox.run(&["status", "--profile", "dev"]);
    assert_eq!(after.status.code(), Some(1));
}

/// `systemctl stop` or a plain `kill` asks with SIGTERM: the daemon stops as
/// it does for `clawg stop`, and leaves no state file sending clients nowhere.
#[cfg(unix)]
#[tokio::test]
async fn sigterm_stops_the_daemon_in_order() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve().await;
    let pid = libc::pid_t::try_from(daemon.pid).expect("pid");

    // SAFETY: plain FFI call on this test's own child.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0);

    let exit = daemon.wait_for_exit(Duration::from_secs(30)).await;
    assert!(exit.success(), "a SIGTERM is an orderly stop: {exit}\n{}", daemon.log_text());
    assert!(!sandbox.data_dir().join("daemon.json").exists(), "{}", daemon.log_text());
    assert!(!sandbox.data_dir().join("token").exists(), "{}", daemon.log_text());
    assert!(daemon.log_text().contains("checkpointed the WAL"), "{}", daemon.log_text());
}

/// A model load runs on a blocking thread nothing can cancel; the process
/// must still end within what `clawg stop` waits for.
#[tokio::test]
async fn stop_ends_a_daemon_whose_model_load_never_returns() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve_with("blocked").await;
    let deadline = Instant::now() + START_BUDGET;
    while !daemon.log_text().contains("the test model load blocks") {
        assert!(Instant::now() < deadline, "the load did not start:\n{}", daemon.log_text());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let stop = sandbox.run(&["stop", "--profile", "dev"]);

    assert_eq!(stop.status.code(), Some(0), "{}\n{}", stderr_of(&stop), daemon.log_text());
    let exit = daemon.wait_for_exit(Duration::from_secs(5)).await;
    assert!(exit.success(), "{exit}\n{}", daemon.log_text());
}

// ── Metrics ─────────────────────────────────────────────────────────────────

fn metrics_report(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["metrics", "report", "--profile", "dev"];
    all.extend_from_slice(args);
    let out = sandbox.run(&all);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    serde_json::from_slice(&out.stdout).expect("report prints JSON")
}

/// The run's `http.duration` count of `route` over `transport`, as
/// `GET /v1/metrics` answers it.
fn run_count(metrics: &Value, route: &str, transport: &str) -> u64 {
    metrics["run"]["histograms"]
        .as_array()
        .expect("histograms")
        .iter()
        .filter(|h| h["name"] == "http.duration" && h["labels"]["route"] == route && h["labels"]["transport"] == transport)
        .map(|h| h["count"].as_u64().unwrap_or(0))
        .sum()
}

#[tokio::test]
async fn a_daemon_records_its_requests_and_report_reads_them_after_stop() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve().await;
    daemon.wait_until_ready().await;
    let data_dir = sandbox.data_dir();
    let port = state_of(&data_dir).and_then(|s| s["port"].as_u64()).expect("--port 0 serves TCP too");
    let tcp = DaemonClient::new(format!("http://127.0.0.1:{port}"), daemon.token.clone(), data_dir.clone());

    let save = json!({ "content": "the metrics smoke test saves this", "agent_id": "smoke" });
    let search = json!({ "query": "metrics smoke test", "agent_id": "smoke" });
    assert_eq!(daemon.post("/v1/memories", save.clone()).await.0, 201);
    assert_eq!(daemon.post("/v1/search", search.clone()).await.0, 200);
    for (path, body) in [("/v1/memories", &save), ("/v1/search", &search)] {
        let call = Call::new(reqwest::Method::POST, path).json(body).expect("request body");
        assert!(tcp.send(call).await.expect("over TCP").status.is_success());
    }

    let answer = daemon.client().send(Call::new(reqwest::Method::GET, "/v1/metrics")).await.expect("metrics");
    assert_eq!(answer.status.as_u16(), 200);
    let metrics: Value = serde_json::from_slice(&answer.body).expect("metrics JSON");
    assert_eq!(metrics["writing"], true, "{metrics}");
    assert_eq!(run_count(&metrics, "/v1/search", "unix"), 1, "{metrics}");
    assert_eq!(run_count(&metrics, "/v1/search", "tcp"), 1, "{metrics}");
    assert!(run_count(&metrics, "/v1/memories", "unix") >= 1, "{metrics}");

    let stop = sandbox.run(&["stop", "--profile", "dev"]);
    assert_eq!(stop.status.code(), Some(0), "{}", stderr_of(&stop));
    daemon.wait_for_exit(Duration::from_secs(5)).await;
    assert!(data_dir.join("metrics.db").is_file());

    let report = metrics_report(&sandbox, &["--metric", "http.duration", "--label", "route=/v1/search"]);
    assert_eq!(report["groups"][0]["key"], clawg_core::build_info::version(), "{report}");
    assert_eq!(report["groups"][0]["count"], 2, "both transports: {report}");
    assert_eq!(report["groups"][0]["not_stopped"], 0, "{report}");
    assert_eq!(report["runs"], json!({ "total": 1, "not_stopped": 0 }), "{report}");
    let unix = metrics_report(&sandbox, &["--metric", "http.duration", "--label", "route=/v1/search", "--label", "transport=unix"]);
    assert_eq!(unix["groups"][0]["count"], 1, "{unix}");
    let memory = metrics_report(&sandbox, &["--metric", "process.rss_bytes", "--by", "day"]);
    assert_eq!(memory["kind"], "gauge", "{memory}");
    assert!(memory["groups"][0]["max"].as_f64().is_some_and(|b| b > 0.0) || !cfg!(any(target_os = "linux", windows)), "{memory}");
}

#[tokio::test]
async fn a_killed_daemon_leaves_its_run_not_stopped() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve().await;
    assert_eq!(sandbox.run(&["stop", "--profile", "dev"]).status.code(), Some(0));
    daemon.wait_for_exit(Duration::from_secs(5)).await;
    drop(daemon);

    let killed = sandbox.serve().await;
    killed.kill();
    drop(killed);

    let report = metrics_report(&sandbox, &["--metric", "http.duration"]);
    assert_eq!(report["runs"], json!({ "total": 2, "not_stopped": 1 }), "{report}");
}

#[tokio::test]
async fn metrics_off_leaves_no_metrics_file() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve_as(&["serve", "--profile", "dev", "--port", "0"], &[("CLAWG_METRICS", "off")]).await;
    let answer = daemon.client().send(Call::new(reqwest::Method::GET, "/v1/metrics")).await.expect("metrics");
    let metrics: Value = serde_json::from_slice(&answer.body).expect("metrics JSON");
    assert_eq!(metrics["writing"], false, "{metrics}");
    assert_eq!(sandbox.run(&["stop", "--profile", "dev"]).status.code(), Some(0));
    daemon.wait_for_exit(Duration::from_secs(5)).await;

    assert!(!sandbox.data_dir().join("metrics.db").exists());
    let out = sandbox.run(&["metrics", "report", "--profile", "dev", "--metric", "http.duration"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr_of(&out).contains("CLAWG_METRICS=off"), "{}", stderr_of(&out));
}

#[tokio::test]
async fn a_debug_binary_refuses_prod_even_with_a_test_home() {
    let sandbox = Sandbox::new();
    let out = sandbox.run(&["status", "--profile", "prod"]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout_of(&out));
    assert!(stderr_of(&out).contains("debug build"), "{}", stderr_of(&out));
    assert!(!sandbox.home.join(".clawg").exists());
}

// ── backup and restore ─────────────────────────────────────────────────────

#[tokio::test]
async fn a_restored_backup_brings_back_its_rows_and_keeps_the_replaced_store() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve().await;
    daemon.wait_until_ready().await;
    let (status, body) = daemon
        .post("/v1/memories", json!({ "content": "kept by the backup", "agent_id": "smoke" }))
        .await;
    assert_eq!(status, 201, "{body}");

    let backup = sandbox.run(&["backup", "--profile", "dev"]);
    assert_eq!(backup.status.code(), Some(0), "{}", stderr_of(&backup));
    let snapshot = stdout_of(&backup).trim().to_string();
    assert!(Path::new(&snapshot).is_file(), "backup prints the snapshot it wrote: {snapshot}");

    let (status, body) = daemon
        .post("/v1/memories", json!({ "content": "saved after the backup", "agent_id": "smoke" }))
        .await;
    assert_eq!(status, 201, "{body}");

    let refused = sandbox.run(&["restore", &snapshot, "--profile", "dev"]);
    assert_eq!(refused.status.code(), Some(1), "restore needs the daemon stopped");
    assert!(stderr_of(&refused).contains("clawg stop"), "{}", stderr_of(&refused));

    assert_eq!(sandbox.run(&["stop", "--profile", "dev"]).status.code(), Some(0));
    daemon.wait_for_exit(Duration::from_secs(5)).await;
    let restore = sandbox.run(&["restore", &snapshot, "--profile", "dev"]);
    assert_eq!(restore.status.code(), Some(0), "{}", stderr_of(&restore));
    assert!(stdout_of(&restore).contains("pre-restore"), "{}", stdout_of(&restore));

    let daemon = sandbox.serve().await;
    let rows = daemon.list().await;
    assert!(rows.contains("kept by the backup"), "{rows}");
    assert!(!rows.contains("saved after the backup"), "{rows}");
}

// ── export and import ──────────────────────────────────────────────────────

#[tokio::test]
async fn export_and_import_round_trip_through_the_binary() {
    let source = Sandbox::new();
    let daemon = source.serve().await;
    daemon.wait_until_ready().await;
    for content in ["exported while the daemon runs", "a second exported memory"] {
        let (status, body) = daemon.post("/v1/memories", json!({ "content": content, "agent_id": "smoke" })).await;
        assert_eq!(status, 201, "{body}");
    }
    let file = source.home.join("memories.jsonl");
    let exported = source.run(&["export", "--out", &file.to_string_lossy(), "--profile", "dev"]);
    assert_eq!(exported.status.code(), Some(0), "{}", stderr_of(&exported));
    assert!(stdout_of(&exported).starts_with("wrote 2 memories"), "{}", stdout_of(&exported));
    #[cfg(unix)]
    assert_eq!(mode_of(&file), 0o600);
    let again = source.run(&["export", "--out", &file.to_string_lossy(), "--profile", "dev"]);
    assert_eq!(again.status.code(), Some(1), "an existing file is not overwritten");

    let target = Sandbox::new();
    let refused = target.run(&["import", &file.to_string_lossy(), "--profile", "dev"]);
    assert_eq!(refused.status.code(), Some(1), "import needs a running daemon");
    assert!(!target.data_dir().join("daemon.json").exists(), "and starts none");

    let receiver = target.serve().await;
    receiver.wait_until_ready().await;
    let imported = target.run(&["import", &file.to_string_lossy(), "--agent", "moved", "--profile", "dev"]);
    assert_eq!(imported.status.code(), Some(0), "{}", stderr_of(&imported));
    assert!(stdout_of(&imported).starts_with("imported 2 of 2 memories"), "{}", stdout_of(&imported));
    let rows = receiver.list().await;
    assert!(rows.contains("exported while the daemon runs") && rows.contains("\"agent_id\":\"moved\""), "{rows}");

    let view = target.home.join("memories.md");
    let markdown = target.run(&["export", "--out", &view.to_string_lossy(), "--format", "markdown", "--profile", "dev"]);
    assert_eq!(markdown.status.code(), Some(0), "{}", stderr_of(&markdown));
    let text = std::fs::read_to_string(&view).expect("the view");
    assert!(text.contains("> a second exported memory"), "{text}");
}

#[tokio::test]
async fn an_engram_export_imports_through_the_binary() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve().await;
    daemon.wait_until_ready().await;
    let app = sandbox.home.join("app");
    std::fs::create_dir_all(app.join(".git")).expect("repo");
    let long = (0..700).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ");
    let observation = |id: i64, sync_id: &str, project: Option<&str>, title: &str, content: &str| {
        json!({
            "id": id, "sync_id": sync_id, "session_id": "s1", "type": "decision", "title": title, "content": content,
            "project": project, "scope": "project", "revision_count": 1, "duplicate_count": 1,
            "created_at": "2025-01-01 10:00:00", "updated_at": "2025-01-01 10:00:00",
        })
    };
    let export = json!({
        "version": "0.2.0",
        "sessions": [{ "id": "s1", "project": "app", "started_at": "2025-01-01 09:00:00", "ended_at": "2025-01-01 12:00:00",
                       "summary": "Wired the importer" }],
        "observations": [
            observation(1, "a", Some("app"), "Notes on clawg-ledger", &long),
            observation(2, "b", None, "Port", "the daemon listens on 27380"),
        ],
        "prompts": [{ "id": 1, "sync_id": "p1", "session_id": "s1", "content": "how does the importer work?",
                      "project": "app", "created_at": "2025-01-01 09:30:00" }],
    });
    let file = sandbox.home.join("engram.json");
    std::fs::write(&file, export.to_string()).expect("the export");
    let (file, map) = (file.to_string_lossy().into_owned(), format!("app={}", app.display()));
    let import = |more: &[&str]| {
        let mut argv = vec!["import", "--from", "engram", file.as_str(), "--project-map", map.as_str()];
        argv.extend_from_slice(more);
        argv.extend_from_slice(&["--profile", "dev"]);
        sandbox.run(&argv)
    };

    let dry = import(&["--dry-run"]);
    assert_eq!(dry.status.code(), Some(0), "{}", stderr_of(&dry));
    assert!(stdout_of(&dry).starts_with("would import "), "{}", stdout_of(&dry));
    assert!(stdout_of(&dry).contains("from engram 0.2.0: notes 4,"), "{}", stdout_of(&dry));
    assert!(!daemon.list().await.contains("Wired the importer"), "a dry run writes nothing");

    let done = import(&[]);
    assert_eq!(done.status.code(), Some(0), "{}", stderr_of(&done));
    assert!(stdout_of(&done).contains("`clawg restore` brings it back"), "{}", stdout_of(&done));
    let rows = daemon.list().await;
    assert!(rows.contains("Wired the importer") && rows.contains("\"agent_id\":\"migrated\""), "{rows}");

    let again = import(&[]);
    assert_eq!(again.status.code(), Some(0), "{}", stderr_of(&again));
    assert!(stdout_of(&again).starts_with("imported 0 of "), "{}", stdout_of(&again));
    assert!(stdout_of(&again).contains("notes already here, by the project they are in:"), "{}", stdout_of(&again));

    let mixed = sandbox.run(&["import", "--from", "engram", &file, "--map-root", "/a=.", "--profile", "dev"]);
    assert_eq!(mixed.status.code(), Some(2), "a map of a clawg export is a usage error: {}", stderr_of(&mixed));
}

#[tokio::test]
async fn projects_move_moves_and_undoes_a_seeded_store() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve().await;
    daemon.wait_until_ready().await;
    for content in ["an unfiled note about the parser", "an unfiled note about the lexer"] {
        let (status, body) = daemon.post("/v1/memories", json!({ "content": content, "agent_id": "smoke" })).await;
        assert_eq!(status, 201, "{body}");
    }
    let repo = sandbox.home.join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("repo");
    let to = repo.to_string_lossy().into_owned();
    let run = |args: &[&str]| {
        let mut argv = vec!["projects", "move"];
        argv.extend_from_slice(args);
        argv.extend_from_slice(&["--profile", "dev"]);
        let output = sandbox.run(&argv);
        assert_eq!(output.status.code(), Some(0), "{argv:?}: {}", stderr_of(&output));
        stdout_of(&output)
    };

    let dry = run(&["--from-none", "--to", &to, "--dry-run"]);
    assert!(dry.starts_with("would move 2 memories to project repo-"), "{dry}");
    assert!(!daemon.list().await.contains("\"project_id\":\"repo-"), "a dry run moves nothing");

    let moved = run(&["--from-none", "--to", &to]);
    assert!(moved.starts_with("moved 2 memories to project repo-"), "{moved}");
    assert!(moved.contains("-pre-move.db; `clawg restore` brings it back"), "{moved}");
    assert!(moved.contains("this is move 1"), "{moved}");
    assert_eq!(daemon.list().await.matches("\"project_id\":\"repo-").count(), 2);

    let undone = run(&["--undo", "1"]);
    assert!(undone.starts_with("moved 2 memories to no project"), "{undone}");
    assert!(!daemon.list().await.contains("\"project_id\":\"repo-"));
}

// ── The data directory's socket ────────────────────────────────────────────

const SERVE_WITHOUT_PORT: [&str; 3] = ["serve", "--profile", "dev"];

#[tokio::test]
async fn a_daemon_without_a_port_serves_status_stop_bridge_and_hooks_over_its_socket() {
    let sandbox = Sandbox::new();
    let root = sandbox.home.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let project = clawg_core::memory::ProjectIdentity::resolve(&root).id;
    let daemon = sandbox.serve_as(&SERVE_WITHOUT_PORT, &[]).await;
    let socket = sandbox.data_dir().join("clawg.sock");
    assert!(socket.exists());
    let state = state_of(&sandbox.data_dir()).expect("daemon.json");
    assert_eq!(state["unix_socket"], true, "{state}");
    assert!(state.get("port").is_none(), "no TCP was asked for: {state}");

    let status = sandbox.run(&["status", "--profile", "dev"]);
    assert_eq!(status.status.code(), Some(0), "{}", stderr_of(&status));
    let report: Value = serde_json::from_slice(&status.stdout).expect("status prints JSON");
    assert_eq!(report["running"], true);
    assert_eq!(report["daemon"]["unix_socket"], true, "{report}");

    let mut bridge = sandbox.bridge();
    bridge.initialize();
    let saved = bridge.call(2, "save_memory", json!({ "content": "the bridge reaches the daemon over its socket" }));
    assert!(!tool_failed(&saved), "{saved}");

    let (status, body) = daemon
        .post(
            "/v1/memories",
            json!({ "content": "the deploy script lives in ops/deploy.sh", "agent_id": "smoke", "project_id": project }),
        )
        .await;
    assert!(status == 201 || status == 202, "{body}");
    daemon.wait_until_ingested().await;
    let input = json!({ "session_id": "s", "cwd": root, "hook_event_name": "SessionStart", "source": "startup" });
    let out = hook(&sandbox, &["session-start", "--agent", "smoke", "--profile", "dev"], &input.to_string());
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert!(stdout_of(&out).contains("ops/deploy.sh"), "the hook reached the daemon: {}", stdout_of(&out));

    let stop = sandbox.run(&["stop", "--profile", "dev"]);
    assert_eq!(stop.status.code(), Some(0), "{}", stderr_of(&stop));
    assert!(daemon.wait_for_exit(Duration::from_secs(5)).await.success());
    assert!(!socket.exists(), "the socket goes with the daemon");
    assert!(!sandbox.data_dir().join("daemon.json").exists());
}

#[tokio::test]
async fn an_orphan_socket_left_by_a_kill_does_not_keep_the_next_daemon_out() {
    let sandbox = Sandbox::new();
    let first = sandbox.serve_as(&SERVE_WITHOUT_PORT, &[]).await;
    first.kill();
    drop(first);
    assert!(sandbox.data_dir().join("clawg.sock").exists(), "a kill leaves the socket behind");

    let second = sandbox.serve_as(&SERVE_WITHOUT_PORT, &[]).await;

    assert_eq!(second.health().await["status"], "ok");
}

#[tokio::test]
async fn a_socket_path_over_the_limit_is_refused_by_name() {
    let (_owned, home) = home_over_the_socket_limit();
    let sandbox = Sandbox::at(&home);
    let socket = sandbox.data_dir().join("clawg.sock");

    let refused = sandbox.run(&SERVE_WITHOUT_PORT);

    assert_eq!(refused.status.code(), Some(1), "{}", stderr_of(&refused));
    assert!(stderr_of(&refused).contains(&socket.display().to_string()), "{}", stderr_of(&refused));
    let daemon = sandbox.serve().await;
    assert_eq!(state_of(&sandbox.data_dir()).expect("daemon.json")["unix_socket"], false);
    assert!(daemon.log_text().contains("serving over TCP alone"), "{}", daemon.log_text());
}

#[tokio::test]
async fn a_taken_profile_port_leaves_the_daemon_on_its_socket() {
    let sandbox = Sandbox::new();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = taken.local_addr().expect("addr").port().to_string();

    let daemon = sandbox.serve_as(&SERVE_WITHOUT_PORT, &[("CLAWG_TEST_DEFAULT_PORT", &port)]).await;

    let state = state_of(&sandbox.data_dir()).expect("daemon.json");
    assert_eq!(state["unix_socket"], true, "{state}");
    assert!(state.get("port").is_none(), "a port it does not have is not published: {state}");
    assert!(daemon.log_text().contains(&format!("127.0.0.1:{port}")), "{}", daemon.log_text());
}

#[tokio::test]
async fn a_taken_explicit_port_is_fatal_and_publishes_nothing() {
    let sandbox = Sandbox::new();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = taken.local_addr().expect("addr").port().to_string();

    let refused = sandbox.run(&["serve", "--profile", "dev", "--port", &port]);

    assert_eq!(refused.status.code(), Some(1), "{}", stderr_of(&refused));
    assert!(stderr_of(&refused).contains(&port), "{}", stderr_of(&refused));
    assert!(!sandbox.data_dir().join("daemon.json").exists());
    assert!(!sandbox.data_dir().join("token").exists());
}

/// The thread that accepts on Windows blocks in `accept`; if the shutdown did
/// not wake it, the socket would stay open and the file could not be removed.
#[tokio::test]
async fn a_shutdown_ends_the_socket_listener_within_its_bounds() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve().await;
    let socket = sandbox.data_dir().join("clawg.sock");
    assert!(socket.exists());

    let started = Instant::now();
    let stop = sandbox.run(&["stop", "--profile", "dev"]);

    assert_eq!(stop.status.code(), Some(0), "{}\n{}", stderr_of(&stop), daemon.log_text());
    assert!(started.elapsed() < clawg_cli::stop::STOP_TIMEOUT, "stop took {:?}", started.elapsed());
    let exit = daemon.wait_for_exit(Duration::from_secs(5)).await;
    assert!(exit.success(), "{exit}\n{}", daemon.log_text());
    assert!(!socket.exists(), "the socket goes with the daemon:\n{}", daemon.log_text());
}

#[tokio::test]
async fn a_bridge_starts_its_daemon_through_the_binary() {
    let sandbox = Sandbox::new();
    let _stop = StopOnDrop { home: sandbox.home.clone() };
    let mut bridge = sandbox.bridge_starting_its_daemon();

    bridge.initialize();
    let status = bridge.call(2, "memory_status", json!({}));

    assert!(!tool_failed(&status), "{status}");
    let state = state_of(&sandbox.data_dir()).expect("the bridge started a daemon");
    assert_eq!(state["unix_socket"], true, "{state}");
    assert!(state.get("port").is_none(), "{state}");
}

#[tokio::test]
async fn an_autostart_rotates_a_full_serve_log() {
    let sandbox = Sandbox::new();
    let _stop = StopOnDrop { home: sandbox.home.clone() };
    let logs = sandbox.data_dir().join("logs");
    std::fs::create_dir_all(&logs).expect("logs dir");
    let full = vec![b'x'; clawg_cli::serve_log::SERVE_LOG_LIMIT as usize + 1];
    std::fs::write(logs.join("serve.log"), &full).expect("full log");
    let mut bridge = sandbox.bridge_starting_its_daemon();

    bridge.initialize();
    let status = bridge.call(2, "memory_status", json!({}));

    assert!(!tool_failed(&status), "{status}");
    assert_eq!(std::fs::read(logs.join("serve.log.1")).expect("the old log moved aside"), full);
    let log = std::fs::read_to_string(logs.join("serve.log")).expect("a new log");
    let first = log.lines().next().unwrap_or_default();
    let opening: String = first.chars().take(200).collect();
    assert!(first.contains(&format!("clawg serve {} starting", env!("CARGO_PKG_VERSION"))), "{opening}");
}

// ── MCP over real stdio ────────────────────────────────────────────────────

#[tokio::test]
async fn a_bridge_on_real_stdio_answers_initialize_and_the_tools_reach_the_daemon() {
    let sandbox = Sandbox::new();
    let _daemon = sandbox.serve().await;
    let mut bridge = sandbox.bridge();

    let init = bridge.initialize();
    assert_eq!(init["result"]["serverInfo"]["name"], "clawg-dev", "{init}");
    assert_eq!(init["result"]["serverInfo"]["version"], env!("CARGO_PKG_VERSION"), "{init}");

    let tools = bridge.request(2, "tools/list", json!({}));
    assert_eq!(tool_names(&tools).len(), 13, "{tools}");

    let status = bridge.call(3, "memory_status", json!({}));
    assert!(!tool_failed(&status), "{status}");
    assert!(tool_text(&status).contains("\"status\""), "{status}");

    let saved = bridge.call(4, "save_memory", json!({ "content": "the bridge speaks real stdio" }));
    assert!(!tool_failed(&saved), "{saved}");
    let found = bridge.call(5, "search_memory", json!({ "query": "the bridge speaks real stdio" }));
    assert!(!tool_failed(&found), "{found}");
    assert!(tool_text(&found).contains("the bridge speaks real stdio"), "{found}");
}

#[tokio::test]
async fn a_bridge_on_real_stdio_serves_a_host_that_never_initializes() {
    let sandbox = Sandbox::new();
    let _daemon = sandbox.serve().await;
    let mut bridge = sandbox.bridge();

    let discover = bridge.request(1, "server/discover", modern_params());
    let versions = discover["result"]["supportedVersions"].to_string();
    assert!(versions.contains("2026-07-28") && versions.contains("2025-06-18"), "{discover}");

    let tools = bridge.request(2, "tools/list", modern_params());
    assert_eq!(tool_names(&tools).len(), 13, "{tools}");

    let mut params = modern_params();
    params["name"] = json!("memory_status");
    params["arguments"] = json!({});
    let status = bridge.request(3, "tools/call", params);
    assert!(!tool_failed(&status), "{status}");
}

#[tokio::test]
async fn a_bridge_left_open_reaches_the_daemon_after_a_restart() {
    let sandbox = Sandbox::new();
    let first = sandbox.serve().await;
    let mut bridge = sandbox.bridge();
    bridge.initialize();
    assert!(!tool_failed(&bridge.call(2, "memory_status", json!({}))));

    assert_eq!(sandbox.run(&["stop", "--profile", "dev"]).status.code(), Some(0));
    first.wait_for_exit(Duration::from_secs(5)).await;
    let second = sandbox.serve().await;
    assert_ne!(first.token, second.token, "a restart issues a new token");

    let reply = bridge.call(3, "memory_status", json!({}));
    assert!(!tool_failed(&reply), "the open bridge must find the new daemon: {reply}");
}

// ── Client hooks ───────────────────────────────────────────────────────────

/// Runs `clawg hook` with `input` on stdin, as a client does.
fn hook(sandbox: &Sandbox, args: &[&str], input: &str) -> Output {
    let mut child = sandbox
        .command(&[&["hook"], args].concat())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn clawg hook");
    // A usage error exits before stdin is read, and the write may then find
    // the pipe closed; the exit code is what such a test checks.
    match child.stdin.take().expect("stdin").write_all(input.as_bytes()) {
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => panic!("write stdin: {e}"),
        _ => {}
    }
    child.wait_with_output().expect("clawg hook")
}

#[tokio::test]
async fn a_hook_without_a_daemon_prints_nothing() {
    let sandbox = Sandbox::new();
    let input = json!({ "cwd": sandbox.home, "source": "startup" }).to_string();
    let out = hook(&sandbox, &["session-start", "--agent", "smoke", "--profile", "dev"], &input);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert_eq!(stdout_of(&out), "");
    assert!(!sandbox.data_dir().join("daemon.json").exists(), "no daemon was started");
}

#[tokio::test]
async fn session_start_without_a_daemon_still_names_the_language() {
    let sandbox = Sandbox::new();
    let set = sandbox.run(&["preferences", "set", "language", "es", "--profile", "dev"]);
    assert_eq!(set.status.code(), Some(0), "{}", stderr_of(&set));
    let input = json!({ "cwd": sandbox.home, "source": "startup" }).to_string();

    let out = hook(&sandbox, &["session-start", "--agent", "smoke", "--profile", "dev"], &input);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    let printed: Value = serde_json::from_str(&stdout_of(&out)).expect("the hook's envelope");
    let context = printed["hookSpecificOutput"]["additionalContext"].as_str().unwrap_or_default();
    assert!(context.starts_with("clawg: el idioma de la persona es el español."), "{printed}");
    assert!(!sandbox.data_dir().join("daemon.json").exists(), "no daemon was started");

    let out = hook(&sandbox, &["user-prompt-submit", "--agent", "smoke", "--profile", "dev"], &input);
    assert_eq!((out.status.code(), stdout_of(&out)), (Some(0), String::new()), "only the session's start names it");
}

#[tokio::test]
async fn a_hook_with_input_that_is_not_json_prints_nothing() {
    let sandbox = Sandbox::new();
    let _daemon = sandbox.serve().await;
    for event in ["session-start", "user-prompt-submit"] {
        let out = hook(&sandbox, &[event, "--agent", "smoke", "--profile", "dev"], "not json");
        assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
        assert_eq!(stdout_of(&out), "");
    }
}

#[tokio::test]
async fn a_hook_usage_error_exits_1() {
    let sandbox = Sandbox::new();
    for args in [&["session-start"][..], &["session-end", "--agent", "a"], &["session-start", "--agent", "a", "--profile", "prod"]] {
        let out = hook(&sandbox, args, "{}");
        assert_eq!(out.status.code(), Some(1), "2 would block the client: {args:?} {}", stderr_of(&out));
        assert!(!stderr_of(&out).is_empty(), "{args:?}");
    }
}

/// What a client that does not know a hook's `args` runs.
#[tokio::test]
async fn a_bare_clawg_exits_1() {
    let sandbox = Sandbox::new();
    let out = sandbox.command(&[]).output().expect("run clawg");
    assert_eq!(out.status.code(), Some(1), "2 would block the client: {}", stderr_of(&out));
    assert!(stderr_of(&out).contains("Usage"), "{}", stderr_of(&out));
    assert!(stdout_of(&out).is_empty(), "{}", stdout_of(&out));
    let typo = sandbox.command(&["statsu"]).output().expect("run clawg");
    assert_eq!(typo.status.code(), Some(2), "any other usage error stays 2");
}

#[tokio::test]
async fn session_start_injects_the_saved_memories() {
    let sandbox = Sandbox::new();
    let root = sandbox.home.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let project = clawg_core::memory::ProjectIdentity::resolve(&root).id;
    let daemon = sandbox.serve().await;
    let (status, body) = daemon
        .post(
            "/v1/memories",
            json!({ "content": "the deploy script lives in ops/deploy.sh", "agent_id": "smoke", "project_id": project }),
        )
        .await;
    assert!(status == 201 || status == 202, "{body}");
    daemon.wait_until_ingested().await;

    let input = json!({ "session_id": "s", "cwd": root, "hook_event_name": "SessionStart", "source": "startup" });
    let out = hook(&sandbox, &["session-start", "--agent", "smoke", "--profile", "dev"], &input.to_string());
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    let reply: Value = serde_json::from_str(&stdout_of(&out)).expect("one JSON object");
    assert_eq!(reply["hookSpecificOutput"]["hookEventName"], "SessionStart");
    let context = reply["hookSpecificOutput"]["additionalContext"].as_str().expect("context");
    assert!(context.contains("ops/deploy.sh"), "{context}");

    let other = hook(&sandbox, &["session-start", "--agent", "other", "--profile", "dev"], &input.to_string());
    assert_eq!(stdout_of(&other), "", "another agent's session has nothing of its own");

    let transcript = sandbox.home.join("transcript.jsonl");
    std::fs::write(&transcript, "").unwrap();
    let prompt = json!({ "cwd": root, "prompt": "go on", "transcript_path": transcript });
    let out = hook(&sandbox, &["user-prompt-submit", "--agent", "smoke", "--profile", "dev"], &prompt.to_string());
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert_eq!(stdout_of(&out), "", "the session just started and a save just landed");
}

#[tokio::test]
async fn two_clones_share_their_memories_through_a_real_daemon() {
    let sandbox = Sandbox::new();
    let clone = |name: &str| {
        let root = sandbox.home.join(name).join("app");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git").join("config"), "[remote \"origin\"]\n\turl = git@github.com:acme/app.git\n")
            .unwrap();
        root
    };
    let (work, review) = (clone("work"), clone("review"));
    let daemon = sandbox.serve().await;
    let (status, registered) =
        daemon.post("/v1/projects", json!({ "root_path": work.to_string_lossy(), "assigned": true })).await;
    assert_eq!(status, 200, "{registered}");
    let (status, body) = daemon
        .post(
            "/v1/memories",
            json!({ "content": "the deploy script lives in ops/deploy.sh", "agent_id": "smoke", "project_id": registered["project_id"] }),
        )
        .await;
    assert!(status == 201 || status == 202, "{body}");
    daemon.wait_until_ingested().await;

    let input = json!({ "session_id": "s", "cwd": review, "hook_event_name": "SessionStart", "source": "startup" });
    let out = hook(&sandbox, &["session-start", "--agent", "smoke", "--profile", "dev"], &input.to_string());

    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    let reply: Value = serde_json::from_str(&stdout_of(&out)).expect("one JSON object");
    let context = reply["hookSpecificOutput"]["additionalContext"].as_str().expect("context");
    assert!(context.contains("ops/deploy.sh"), "the other clone's memory: {context}");

    let listed = sandbox.command(&["memories", "list", "--here", "--profile", "dev"]).current_dir(&review).output().unwrap();
    assert_eq!(listed.status.code(), Some(0), "{}", stderr_of(&listed));
    assert!(stdout_of(&listed).contains("ops/deploy.sh"), "{}", stdout_of(&listed));
}

#[tokio::test]
async fn a_hook_registers_the_session_of_the_saves() {
    let sandbox = Sandbox::new();
    let root = sandbox.home.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let project = clawg_core::memory::ProjectIdentity::resolve(&root).id;
    let daemon = sandbox.serve().await;
    let save = |content: &'static str| {
        daemon.post("/v1/memories", json!({ "content": content, "agent_id": "smoke", "project_id": project }))
    };
    let session_of = |list: &str, content: &str| -> Value {
        let list: Value = serde_json::from_str(list).expect("list json");
        let item = list["items"].as_array().unwrap().iter().find(|i| i["summary"] == content).cloned();
        item.unwrap_or_else(|| panic!("{content} not listed: {list}"))["source_session_id"].clone()
    };

    let start = json!({ "session_id": "s1", "cwd": root, "hook_event_name": "SessionStart", "source": "startup" });
    let out = hook(&sandbox, &["session-start", "--agent", "smoke", "--profile", "dev"], &start.to_string());
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    let (status, body) = save("the deploy script lives in ops/deploy.sh").await;
    assert!(status == 201 || status == 202, "{body}");

    let prompt = json!({ "session_id": "s2", "cwd": root, "prompt": "go on", "hook_event_name": "UserPromptSubmit" });
    let out = hook(&sandbox, &["user-prompt-submit", "--agent", "smoke", "--profile", "dev"], &prompt.to_string());
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    let (status, body) = save("the release notes live in docs/releases").await;
    assert!(status == 201 || status == 202, "{body}");
    daemon.wait_until_ingested().await;

    let list = daemon.list().await;
    assert_eq!(session_of(&list, "the deploy script lives in ops/deploy.sh"), "s1", "{list}");
    assert_eq!(session_of(&list, "the release notes live in docs/releases"), "s2", "a later prompt of another session wins: {list}");
}

// ── doctor ─────────────────────────────────────────────────────────────────

/// `clawg doctor` with a PATH that holds no `clawg` and no Claude Code
/// configuration, so its findings are about the sandbox alone.
fn doctor(sandbox: &Sandbox) -> (Output, Value) {
    let empty = sandbox.home.join("empty-path");
    std::fs::create_dir_all(&empty).expect("empty PATH directory");
    let output = sandbox
        .command(&["doctor", "--profile", "dev"])
        .env("PATH", &empty)
        .env_remove("CLAUDE_CONFIG_DIR")
        .output()
        .expect("run clawg doctor");
    let report = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("doctor prints JSON ({e}): {}\n{}", stdout_of(&output), stderr_of(&output)));
    (output, report)
}

fn findings<'a>(report: &'a Value, check_id: &str) -> Vec<(&'a str, &'a str)> {
    report["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .filter(|f| f["check_id"] == check_id)
        .map(|f| (f["severity"].as_str().unwrap(), f["reason_code"].as_str().unwrap()))
        .collect()
}

/// Every file of the data directory with its bytes, the logs aside.
fn store_files(data_dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(data_dir)
        .expect("data directory")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_file())
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(&p).expect("read")))
        .collect();
    files.sort();
    files
}

#[tokio::test]
async fn doctor_reads_a_running_daemons_store_without_changing_it() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve().await;
    daemon.wait_until_ready().await;
    let (status, body) = daemon.post("/v1/memories", json!({ "content": "doctor reads this store", "agent_id": "smoke" })).await;
    assert_eq!(status, 201, "{body}");

    let (output, report) = doctor(&sandbox);
    assert_eq!(output.status.code(), Some(0), "a warning does not fail: {report}");
    assert_eq!(report["data_dir"].as_str().map(PathBuf::from), Some(sandbox.data_dir()));
    assert_eq!(findings(&report, "store.owner"), [("info", "held")]);
    let owner = report["checks"].as_array().unwrap().iter().find(|f| f["check_id"] == "store.owner").unwrap();
    assert_eq!(owner["evidence"]["pid"], daemon.pid);
    assert_eq!(findings(&report, "store.schema"), [("ok", "healthy")]);
    assert_eq!(findings(&report, "store.contract"), [("ok", "healthy")], "the stand-in model's contract: {report}");
    assert_eq!(findings(&report, "backup.age"), [("warn", "none")]);
    assert_eq!(findings(&report, "socket"), [("ok", "healthy")]);
    assert_eq!(findings(&report, "memory.topic"), [("ok", "healthy")]);
    assert_eq!(findings(&report, "outbox"), [("ok", "healthy")]);
    assert_eq!(report["summary"]["error"], 0, "{report}");

    assert_eq!(sandbox.run(&["stop", "--profile", "dev"]).status.code(), Some(0));
    daemon.wait_for_exit(Duration::from_secs(5)).await;
    let before = store_files(&sandbox.data_dir());
    let (output, report) = doctor(&sandbox);
    assert_eq!(output.status.code(), Some(0), "{report}");
    assert_eq!(findings(&report, "store.owner"), [("ok", "healthy")]);
    assert_eq!(findings(&report, "agents"), [("ok", "healthy")]);
    assert_eq!(store_files(&sandbox.data_dir()), before, "a stopped store gains no file and no byte");
}

#[tokio::test]
async fn preferences_set_and_unset_write_the_profiles_file() {
    let sandbox = Sandbox::new();
    let file = sandbox.data_dir().join("preferences.json");
    let shown = sandbox.run(&["preferences", "--profile", "dev"]);
    assert_eq!((shown.status.code(), stdout_of(&shown).trim()), (Some(0), "no language preference: clawg uses English"));

    let set = sandbox.run(&["preferences", "set", "language", "es", "--profile", "dev"]);
    assert_eq!(set.status.code(), Some(0), "{}", stderr_of(&set));
    assert!(stdout_of(&set).starts_with("Idioma: español."), "{}", stdout_of(&set));
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&file).expect("the file")).unwrap();
    assert_eq!(written, json!({ "version": 1, "language": "es" }));
    assert_eq!(stdout_of(&sandbox.run(&["preferences", "--profile", "dev"])).trim(), "idioma: español");

    let unset = sandbox.run(&["preferences", "unset", "language", "--profile", "dev"]);
    assert_eq!(unset.status.code(), Some(0), "{}", stderr_of(&unset));
    assert!(stdout_of(&unset).starts_with("No language preference: clawg uses English."), "{}", stdout_of(&unset));
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&file).expect("the file")).unwrap();
    assert_eq!(written, json!({ "version": 1 }));

    let bad = sandbox.run(&["preferences", "set", "language", "fr", "--profile", "dev"]);
    assert_ne!(bad.status.code(), Some(0), "an unknown language is a usage error");
}

#[tokio::test]
async fn doctor_names_a_broken_preferences_file() {
    let sandbox = Sandbox::new();
    let (_, report) = doctor(&sandbox);
    assert!(findings(&report, "preferences").is_empty(), "no file, no finding: {report}");

    std::fs::create_dir_all(sandbox.data_dir()).expect("data dir");
    std::fs::write(sandbox.data_dir().join("preferences.json"), r#"{"version":1,"language":"fr"}"#).expect("write");
    let (_, report) = doctor(&sandbox);
    assert_eq!(findings(&report, "preferences"), [("warn", "invalid")]);
    assert!(findings(&report, "protocol_block").is_empty(), "no preference to hold the block to: {report}");

    let set = sandbox.run(&["preferences", "set", "language", "es", "--profile", "dev"]);
    assert_ne!(set.status.code(), Some(0), "an invalid file is never overwritten");
    std::fs::write(sandbox.data_dir().join("preferences.json"), r#"{"version":1,"language":"es"}"#).expect("write");
    let (_, report) = doctor(&sandbox);
    assert_eq!(findings(&report, "preferences"), [("ok", "healthy")]);
    assert_eq!(findings(&report, "protocol_block"), [("info", "not_installed")], "{report}");
}

/// Every finding but its message, so two reports of the same facts compare.
fn without_messages(report: &Value) -> Vec<Value> {
    let mut checks = report["checks"].as_array().expect("checks").clone();
    for finding in &mut checks {
        finding["message"] = Value::Null;
    }
    checks
}

#[tokio::test]
async fn doctor_speaks_the_preferred_language() {
    let sandbox = Sandbox::new();
    std::fs::create_dir_all(sandbox.data_dir()).expect("data dir");
    std::fs::write(sandbox.data_dir().join("preferences.json"), r#"{"version":1,"language":"en"}"#).expect("write");
    let (en_output, en) = doctor(&sandbox);
    std::fs::write(sandbox.data_dir().join("preferences.json"), r#"{"version":1,"language":"es"}"#).expect("write");
    let (es_output, es) = doctor(&sandbox);

    assert_eq!(es_output.status.code(), en_output.status.code());
    assert_eq!(without_messages(&es), without_messages(&en), "only the messages change");
    let message = |report: &Value, id: &str| {
        let checks = report["checks"].as_array().unwrap();
        checks.iter().find(|f| f["check_id"] == id).unwrap()["message"].as_str().unwrap().to_string()
    };
    assert_eq!(message(&en, "store.schema"), "there is no memory.db yet: the daemon creates it on its first start");
    assert_eq!(message(&es, "store.schema"), "todavía no hay memory.db: el daemon lo crea la primera vez que arranca");
    assert_eq!(message(&es, "preferences"), "las preferencias se pueden leer");
}

#[tokio::test]
async fn stop_speaks_the_preferred_language() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve().await;
    daemon.wait_until_ready().await;
    let set = sandbox.run(&["preferences", "set", "language", "es", "--profile", "dev"]);
    assert_eq!(set.status.code(), Some(0), "{}", stderr_of(&set));

    let stopped = sandbox.run(&["stop", "--profile", "dev"]);
    assert_eq!(stopped.status.code(), Some(0), "{}", stderr_of(&stopped));
    assert!(stdout_of(&stopped).starts_with("se detuvo el daemon dev (pid "), "{}", stdout_of(&stopped));
    daemon.wait_for_exit(Duration::from_secs(5)).await;

    let again = sandbox.run(&["stop", "--profile", "dev"]);
    assert_eq!(again.status.code(), Some(1));
    assert!(
        stderr_of(&again).contains("clawg: no hay ningún daemon de clawg corriendo para el perfil dev"),
        "{}",
        stderr_of(&again)
    );

    std::fs::write(sandbox.data_dir().join("preferences.json"), "{ not json").expect("write");
    let broken = sandbox.run(&["stop", "--profile", "dev"]);
    assert_eq!(broken.status.code(), Some(1), "a broken file changes no exit code");
    assert!(stderr_of(&broken).contains("clawg: no clawg daemon is running for profile dev"), "{}", stderr_of(&broken));
}

#[tokio::test]
async fn doctor_prints_no_row_content() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve().await;
    daemon.wait_until_ready().await;
    for content in ["the quartz heron config lives in a.toml", "the quartz heron config lives in b.toml"] {
        let (status, body) = daemon
            .post("/v1/memories", json!({ "content": content, "agent_id": "smoke", "topic_key": "heron/config" }))
            .await;
        assert_eq!(status, 201, "{body}");
    }
    assert_eq!(sandbox.run(&["stop", "--profile", "dev"]).status.code(), Some(0));
    daemon.wait_for_exit(Duration::from_secs(5)).await;
    // Two live versions of one topic, which no clawg writes on its own.
    let c = rusqlite::Connection::open(sandbox.data_dir().join("memory.db")).expect("open the store");
    let revived = c
        .execute("UPDATE memory SET status = 'active', superseded_by = NULL, supersede_cause = NULL WHERE status = 'superseded'", [])
        .expect("revive the replaced version");
    assert_eq!(revived, 1);
    drop(c);

    let (output, report) = doctor(&sandbox);
    assert_eq!(output.status.code(), Some(0), "{report}");
    assert_eq!(findings(&report, "memory.topic"), [("warn", "several_live_saves")]);
    let topic = report["checks"].as_array().unwrap().iter().find(|f| f["check_id"] == "memory.topic").unwrap();
    assert_eq!(topic["evidence"]["saves"].as_array().map(Vec::len), Some(2), "{topic}");
    let printed = stdout_of(&output);
    for text in ["quartz heron", "a.toml", "b.toml", "heron/config"] {
        assert!(!printed.contains(text), "doctor printed {text:?}: {printed}");
    }
}

// ── Cleanup ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_test_that_fails_midway_leaves_no_daemon_holding_the_store() {
    let sandbox = Sandbox::new();
    let home = sandbox.home.clone();
    let failed = tokio::spawn(async move {
        let _daemon = Sandbox::at(&home).serve().await;
        panic!("a test failing while its daemon runs");
    })
    .await;
    assert!(failed.is_err_and(|e| e.is_panic()));

    // Only a daemon that is gone has let go of the store lock.
    let daemon = sandbox.serve().await;
    assert!(daemon.exit_status().is_none());
}

// ── Bytes after the executable ─────────────────────────────────────────────

/// A copy of the test binary with `0x00 CLAWG-TEST-STAMP <json>` after its
/// image, which only a `testing` build reads.
fn stamped_copy(dir: &Path, json: &str) -> PathBuf {
    let copy = dir.join(format!("clawg-copy{}", std::env::consts::EXE_SUFFIX));
    write_executable(&copy, &stamped_bytes(json));
    copy
}

/// The test binary with `0x00 CLAWG-TEST-STAMP <json>` after its image.
fn stamped_bytes(json: &str) -> Vec<u8> {
    let mut bytes = std::fs::read(BIN).expect("read the test binary");
    bytes.push(0);
    bytes.extend_from_slice(format!("CLAWG-TEST-STAMP {json}").as_bytes());
    bytes
}

/// Runs a copy of the binary this process wrote. While the copy was open for
/// writing, a child another test forked meanwhile inherited that descriptor
/// and holds it until it execs, and running the copy in that moment fails
/// with "text file busy", so the run is tried again.
trait RunCopy {
    fn spawn_copy(&mut self) -> std::io::Result<Child>;
    fn output_copy(&mut self) -> std::io::Result<Output>;
}

impl RunCopy for Command {
    fn spawn_copy(&mut self) -> std::io::Result<Child> {
        retry_while_busy(|| self.spawn())
    }

    fn output_copy(&mut self) -> std::io::Result<Output> {
        retry_while_busy(|| self.output())
    }
}

fn retry_while_busy<T>(mut run: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    for _ in 0..50 {
        match run() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => std::thread::sleep(Duration::from_millis(20)),
            done => return done,
        }
    }
    run()
}

fn write_executable(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).expect("write the copy");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

fn run_binary(sandbox: &Sandbox, bin: &Path, args: &[&str]) -> Output {
    Command::new(bin)
        .args(args)
        .env("CLAWG_TEST_HOME", &sandbox.home)
        .env_remove("CLAWG_PROFILE")
        .envs(socket_only())
        .current_dir(&sandbox.home)
        .stdin(Stdio::null())
        .output_copy()
        .expect("run clawg")
}

#[tokio::test]
async fn trailing_bytes_after_the_executable_image_do_not_change_execution() {
    let sandbox = Sandbox::new();
    let copy = stamped_copy(&sandbox.home, "{}");

    for args in [&["--version"][..], &["status", "--profile", "dev"][..]] {
        let original = run_binary(&sandbox, Path::new(BIN), args);
        let stamped = run_binary(&sandbox, &copy, args);
        assert_eq!(original.status.code(), stamped.status.code(), "{args:?}: {}", stderr_of(&stamped));
        assert_eq!(original.stdout, stamped.stdout, "{args:?}");
        assert_eq!(original.stderr, stamped.stderr, "{args:?}");
    }
}

#[tokio::test]
async fn a_test_build_reads_its_version_from_the_stamp() {
    let sandbox = Sandbox::new();
    let copy = stamped_copy(&sandbox.home, r#"{"version":"0.0.1"}"#);

    let out = run_binary(&sandbox, &copy, &["--version"]);

    assert_eq!(stdout_of(&out).trim(), "clawg 0.0.1", "{}", stderr_of(&out));
    let unstamped = run_binary(&sandbox, Path::new(BIN), &["--version"]);
    assert_eq!(stdout_of(&unstamped).trim(), format!("clawg {}", env!("CARGO_PKG_VERSION")));
}

// ── update --check ─────────────────────────────────────────────────────────

/// The target the stamped copies claim to be distributed for.
const STAMPED_TARGET: &str =
    if cfg!(windows) { "x86_64-pc-windows-msvc" } else { "x86_64-unknown-linux-gnu" };

/// A release server on an ephemeral port that answers the listing with
/// `releases` and counts the requests it gets.
struct ReleaseServer {
    base: String,
    hits: Arc<std::sync::atomic::AtomicUsize>,
}

impl ReleaseServer {
    async fn start(releases: Value) -> Self {
        Self::serving(releases, std::collections::HashMap::new()).await
    }

    /// Also serves `files`, by name, as the downloads of any release tag.
    async fn serving(releases: Value, files: std::collections::HashMap<String, Vec<u8>>) -> Self {
        use axum::extract::{Path as UrlPath, State};
        use axum::http::StatusCode;
        use axum::routing::get;
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = hits.clone();
        let app = axum::Router::new()
            .route(
                "/repos/MarcossIC/claw-garden/releases",
                get(move || {
                    counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let body = releases.to_string();
                    async move { ([("content-type", "application/json")], body) }
                }),
            )
            .route(
                "/MarcossIC/claw-garden/releases/download/{tag}/{name}",
                get(
                    |State(files): State<Arc<std::collections::HashMap<String, Vec<u8>>>>,
                     UrlPath((_tag, name)): UrlPath<(String, String)>| async move {
                        match files.get(&name) {
                            Some(bytes) => Ok(bytes.clone()),
                            None => Err(StatusCode::NOT_FOUND),
                        }
                    },
                ),
            )
            .with_state(Arc::new(files));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
        Self { base, hits }
    }

    /// An address nothing listens on.
    async fn down() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        format!("http://{}", listener.local_addr().expect("addr"))
    }

    fn hits(&self) -> usize {
        self.hits.load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn listing(tags: &[&str]) -> Value {
    Value::Array(
        tags.iter()
            .map(|tag| json!({ "tag_name": tag, "draft": false, "prerelease": false, "assets": [] }))
            .collect(),
    )
}

fn update(sandbox: &Sandbox, bin: &Path, server: &str, args: &[&str]) -> Output {
    Command::new(bin)
        .arg("update")
        .args(args)
        .env("CLAWG_TEST_HOME", &sandbox.home)
        .env("CLAWG_TEST_RELEASE_URL", server)
        .env_remove("CLAWG_PROFILE")
        .envs(socket_only())
        .current_dir(&sandbox.home)
        .stdin(Stdio::null())
        .output_copy()
        .expect("run clawg update")
}

// The server runs in this process while the binary is waited on.
#[tokio::test(flavor = "multi_thread")]
async fn update_check_reports_a_newer_version_and_an_up_to_date_one() {
    let sandbox = Sandbox::new();
    let installed = stamped_copy(
        &sandbox.home,
        &format!(r#"{{"version":"0.18.0","dist_target":"{STAMPED_TARGET}"}}"#),
    );
    let before = std::fs::read(&installed).expect("read the installed binary");

    let newer = ReleaseServer::start(listing(&["v0.18.0", "v0.19.0"])).await;
    let out = update(&sandbox, &installed, &newer.base, &["--check"]);
    assert_eq!(out.status.code(), Some(100), "{}", stderr_of(&out));
    assert!(stdout_of(&out).contains("0.19.0"), "{}", stdout_of(&out));
    assert_eq!(newer.hits(), 1);

    let same = ReleaseServer::start(listing(&["v0.18.0", "v0.17.0"])).await;
    let out = update(&sandbox, &installed, &same.base, &["--check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert!(stdout_of(&out).contains("up to date"), "{}", stdout_of(&out));

    let out = update(&sandbox, &installed, &ReleaseServer::down().await, &["--check"]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout_of(&out));
    assert!(!stderr_of(&out).is_empty());

    assert_eq!(std::fs::read(&installed).expect("read again"), before, "--check changes nothing");
}

// The server runs in this process while the binary is waited on.
#[tokio::test(flavor = "multi_thread")]
async fn update_refuses_a_binary_without_a_stamp() {
    let sandbox = Sandbox::new();
    let server = ReleaseServer::start(listing(&["v99.0.0"])).await;

    for args in [&["--check"][..], &[][..]] {
        let out = update(&sandbox, Path::new(BIN), &server.base, args);
        assert_eq!(out.status.code(), Some(1), "{}", stdout_of(&out));
        assert!(stderr_of(&out).contains("built locally"), "{}", stderr_of(&out));
    }
    assert_eq!(server.hits(), 0, "a refused binary asks nothing");
}

// ── update ─────────────────────────────────────────────────────────────────

/// A release whose files are signed on the spot, with the public key that
/// verifies them.
struct SignedRelease {
    listing: Value,
    files: std::collections::HashMap<String, Vec<u8>>,
    /// The text of the `.pub` file.
    public: String,
    /// What the release's executable holds.
    binary: Vec<u8>,
}

fn stamp_json(version: &str, extra: &str) -> String {
    format!(r#"{{"version":"{version}","dist_target":"{STAMPED_TARGET}"{extra}}}"#)
}

/// A release `version` whose executable is the test binary stamped as that
/// version, with `extra` fields (`,"migrations_up_to":7`) in its stamp.
fn signed_release(version: &str, extra: &str) -> SignedRelease {
    use sha2::{Digest, Sha256};
    let asset = format!("clawg-{STAMPED_TARGET}{}", if cfg!(windows) { ".exe" } else { "" });
    let binary = stamped_bytes(&stamp_json(version, extra));
    let hash: String = Sha256::digest(&binary).iter().map(|b| format!("{b:02x}")).collect();
    let sums = format!("{hash}  {asset}\n");
    let pair = minisign::KeyPair::generate_unencrypted_keypair().expect("key pair");
    let public = pair.pk.to_box().expect("public key").into_string();
    let signature = minisign::sign(
        None,
        &pair.sk,
        std::io::Cursor::new(sums.as_bytes()),
        Some(&format!("clawg v{version} SHA256SUMS")),
        None,
    )
    .expect("sign")
    .into_string();
    let listing = json!([{
        "tag_name": format!("v{version}"),
        "draft": false,
        "prerelease": false,
        "assets": [{ "name": asset }, { "name": "SHA256SUMS" }, { "name": "SHA256SUMS.minisig" }],
    }]);
    let files = std::collections::HashMap::from([
        (asset, binary.clone()),
        ("SHA256SUMS".to_string(), sums.into_bytes()),
        ("SHA256SUMS.minisig".to_string(), signature.into_bytes()),
    ]);
    SignedRelease { listing, files, public, binary }
}

/// Where the installed copy lives, as an installer would leave it.
fn install(sandbox: &Sandbox, stamp: &str) -> PathBuf {
    let dir = sandbox.home.join("bin");
    std::fs::create_dir_all(&dir).expect("bin dir");
    let installed = dir.join(format!("clawg{}", std::env::consts::EXE_SUFFIX));
    write_executable(&installed, &stamped_bytes(stamp));
    installed
}

/// `clawg update`, from `installed`, against `server`, trusting `public`.
fn update_from(sandbox: &Sandbox, installed: &Path, server: &str, public: &str) -> Output {
    Command::new(installed)
        .arg("update")
        .env("CLAWG_TEST_HOME", &sandbox.home)
        .env("CLAWG_TEST_EMBEDDER", "fake")
        .env("CLAWG_TEST_RELEASE_URL", server)
        .env("CLAWG_TEST_RELEASE_KEYS", json!([public]).to_string())
        .env_remove("CLAWG_PROFILE")
        .envs(socket_only())
        .current_dir(&sandbox.home)
        .stdin(Stdio::null())
        .output_copy()
        .expect("run clawg update")
}

/// Stops, when it drops, whatever daemon the profile has: the one `update`
/// starts is not a child of the test.
struct StopOnDrop {
    home: PathBuf,
}

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        let _ = Command::new(BIN)
            .args(["stop", "--profile", "dev"])
            .env("CLAWG_TEST_HOME", &self.home)
            .env_remove("CLAWG_PROFILE")
            .stdin(Stdio::null())
            .output();
    }
}

/// `/health` of the daemon the data directory names.
async fn published_health(data_dir: &Path) -> Value {
    let client = DaemonClient::from_data_dir(data_dir).expect("a way in");
    let answer = client.send(health_call()).await.expect("health");
    serde_json::from_slice(&answer.body).expect("health json")
}

/// The files beside the installed binary whose name starts with `prefix`.
fn beside(installed: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(installed.parent().expect("dir"))
        .expect("read the install dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(prefix)))
        .collect();
    found.sort();
    found
}

/// The newest migration this build of clawg knows.
fn last_migration() -> i32 {
    clawg_core::memory::backup::known_migrations()
        .iter()
        .map(|m| m.version)
        .max()
        .expect("the binary has migrations")
}

/// What the daemon the data directory names answers to `method` on `path`.
async fn published_send(data_dir: &Path, method: reqwest::Method, path: &str) -> String {
    let client = DaemonClient::from_data_dir(data_dir).expect("a way in");
    let answer = client.send(Call::new(method, path)).await.expect("request");
    assert!(answer.status.is_success(), "{path}: {}", answer.status);
    String::from_utf8(answer.body).expect("body")
}

async fn published_get(data_dir: &Path, path: &str) -> String {
    published_send(data_dir, reqwest::Method::GET, path).await
}

async fn published_delete(data_dir: &Path, path: &str) -> Value {
    serde_json::from_str(&published_send(data_dir, reqwest::Method::DELETE, path).await).expect("delete json")
}

const SEEDED: [&str; 2] = [
    "the lighthouse keeper logs every ship that passes",
    "the harbour master closes the gate at midnight",
];

/// A daemon of the installed copy with `SEEDED` saved through it.
async fn seeded_store(sandbox: &Sandbox) -> (PathBuf, Daemon) {
    let installed = install(sandbox, &stamp_json("0.18.0", ""));
    let old = sandbox.serve_binary(&installed, "fake").await;
    old.wait_until_ready().await;
    for text in SEEDED {
        let (status, body) = old.post("/v1/memories", json!({ "content": text, "agent_id": "smoke" })).await;
        assert_eq!(status, 201, "{body}");
    }
    old.wait_until_ingested().await;
    (installed, old)
}

/// The schema the installed binary of a store with a pending migration is
/// frozen at. Fixed, not "the last one minus one": the rows are inserted by
/// SQL against this schema, so a later migration only adds one more pending.
const OLD_SCHEMA: i32 = 15;

/// A daemon of an installed copy frozen at `schema`, over a store `seed`
/// wrote into while no daemon held it.
async fn store_at(sandbox: &Sandbox, schema: i32, seed: impl FnOnce(&rusqlite::Connection)) -> (PathBuf, Daemon) {
    let installed = install(sandbox, &stamp_json("0.18.0", &format!(r#","migrations_up_to":{schema}"#)));
    // The first start only creates the store at that schema.
    let first = sandbox.serve_binary(&installed, "fake").await;
    let stopped = sandbox.command_of(&installed, &["stop", "--profile", "dev"]).output_copy().expect("run clawg stop");
    assert!(stopped.status.success(), "{}", stderr_of(&stopped));
    first.wait_for_exit(Duration::from_secs(10)).await;

    let db = rusqlite::Connection::open(sandbox.data_dir().join("memory.db")).expect("open the store");
    seed(&db);
    drop(db);
    (installed.clone(), sandbox.serve_binary(&installed, "fake").await)
}

/// A daemon of an installed copy frozen at [`OLD_SCHEMA`], over a store with
/// `SEEDED` in it.
async fn store_behind(sandbox: &Sandbox) -> (PathBuf, Daemon) {
    store_at(sandbox, OLD_SCHEMA, |db| {
        for text in SEEDED {
            db.execute(
                "INSERT INTO memory (agent_id, content, created_at, last_modified_at, last_accessed_at) \
                 VALUES ('smoke', ?1, 0, 0, 0)",
                [text],
            )
            .expect("seed a row");
        }
    })
    .await
}

/// The last schema before `supersede_cause`. A store at it holds revisions
/// and verdicts with no cause recorded, and the migration has to tell them
/// apart by what they left in `topic_key` and `memory_conflict`.
const UNCAUSED_SCHEMA: i32 = 16;

type Row = (i64, String, String, Option<i64>, Option<String>, bool);

/// `(id, content, status, superseded_by, topic_key, deleted)`. 2 revised 1;
/// 4 revised 3 until a verdict undid it; 6 won a verdict over 5; 8 revised 7
/// and was deleted.
const UNCAUSED_ROWS: [(i64, &str, &str, Option<i64>, Option<&str>, bool); 8] = [
    (1, "storage engine is SQLite", "superseded", Some(2), Some("storage/engine"), false),
    (2, "storage engine moved to Postgres", "active", None, Some("storage/engine"), false),
    (3, "the daemon listens on port 27380", "active", None, Some("api/port"), false),
    (4, "the daemon listens on port 27390", "superseded", Some(3), Some("api/port"), false),
    (5, "backups are written weekly", "superseded", Some(6), None, false),
    (6, "backups are written every night", "active", None, None, false),
    (7, "logs rotate at 10 MB", "superseded", Some(8), Some("logs/rotation"), false),
    (8, "logs rotate at 50 MB", "active", None, Some("logs/rotation"), true),
];

fn seed_uncaused(db: &rusqlite::Connection) {
    for (id, content, status, by, key, deleted) in UNCAUSED_ROWS {
        db.execute(
            "INSERT INTO memory (id, agent_id, content, status, superseded_by, topic_key, deleted_at, \
             created_at, last_modified_at, last_accessed_at) \
             VALUES (?1, 'smoke', ?2, ?3, ?4, ?5, ?6, ?1, ?1, ?1)",
            rusqlite::params![id, content, status, by, key, deleted.then_some(100)],
        )
        .expect("seed a row");
    }
    db.execute_batch(
        "INSERT INTO memory_conflict (new_id, old_id, distance, state, verdict, created_at, judged_at) \
         VALUES (4, 3, NULL, 'judged', 'supersedes', 4, 4), (6, 5, 0.2, 'judged', 'supersedes', 6, 6);",
    )
    .expect("seed the verdicts");
}

/// The rows of the store at `path`, as [`UNCAUSED_ROWS`] lists them.
fn rows_in(path: &Path) -> Vec<Row> {
    let db = rusqlite::Connection::open(path).expect("open the store");
    db.prepare(
        "SELECT id, content, status, superseded_by, topic_key, deleted_at IS NOT NULL FROM memory ORDER BY id",
    )
    .and_then(|mut q| {
        q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
            .and_then(Iterator::collect)
    })
    .expect("read the rows")
}

/// The `pre-migration` snapshots in the data directory.
fn pre_migrations(data_dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(data_dir.join("backups"))
        .map(|rd| rd.map(|e| e.expect("entry").path()).collect())
        .unwrap_or_default();
    found.retain(|p| {
        let name = p.file_name().expect("name").to_string_lossy().into_owned();
        clawg_core::memory::backup::parse_snapshot_name(&name) == Some(clawg_core::memory::backup::Reason::PreMigration)
    });
    found.sort();
    found
}

// The server runs in this process while the binary is waited on.
#[tokio::test(flavor = "multi_thread")]
async fn an_update_over_a_pending_migration_leaves_a_pre_migration_snapshot_and_the_data() {
    let sandbox = Sandbox::new();
    let (installed, old) = store_behind(&sandbox).await;
    let release = signed_release("0.19.0", "");
    let server = ReleaseServer::serving(release.listing.clone(), release.files.clone()).await;
    let _stop = StopOnDrop { home: sandbox.home.clone() };

    let out = update_from(&sandbox, &installed, &server.base, &release.public);

    assert_eq!(out.status.code(), Some(0), "{}\n{}", stdout_of(&out), stderr_of(&out));
    old.wait_for_exit(Duration::from_secs(10)).await;
    let health = published_health(&sandbox.data_dir()).await;
    assert_eq!(health["version"], "0.19.0", "{health}");

    let snapshots = pre_migrations(&sandbox.data_dir());
    assert_eq!(snapshots.len(), 1, "{snapshots:?}");
    let report = stdout_of(&out);
    assert!(report.contains(&snapshots[0].display().to_string()), "the snapshot is named: {report}");
    let before = rusqlite::Connection::open(&snapshots[0]).expect("open the snapshot");
    let version: i32 = before
        .query_row("SELECT max(version) FROM refinery_schema_history", [], |r| r.get(0))
        .expect("snapshot schema");
    assert_eq!(version, OLD_SCHEMA, "the snapshot is the store as the old binary left it");

    let rows = published_get(&sandbox.data_dir(), "/v1/memories?limit=100").await;
    for text in SEEDED {
        assert!(rows.contains(text), "{text:?} survived the migration: {rows}");
    }
}

// The server runs in this process while the binary is waited on.
#[tokio::test(flavor = "multi_thread")]
async fn an_update_over_a_store_without_supersede_causes_keeps_its_rows_and_tells_revisions_from_verdicts() {
    let sandbox = Sandbox::new();
    let (installed, old) = store_at(&sandbox, UNCAUSED_SCHEMA, seed_uncaused).await;
    let release = signed_release("0.19.0", "");
    let server = ReleaseServer::serving(release.listing.clone(), release.files.clone()).await;
    let _stop = StopOnDrop { home: sandbox.home.clone() };

    let out = update_from(&sandbox, &installed, &server.base, &release.public);

    assert_eq!(out.status.code(), Some(0), "{}\n{}", stdout_of(&out), stderr_of(&out));
    old.wait_for_exit(Duration::from_secs(10)).await;
    let health = published_health(&sandbox.data_dir()).await;
    assert_eq!(health["version"], "0.19.0", "{health}");

    let seeded: Vec<Row> = UNCAUSED_ROWS
        .iter()
        .map(|&(id, content, status, by, key, deleted)| {
            (id, content.to_string(), status.to_string(), by, key.map(str::to_string), deleted)
        })
        .collect();
    let snapshots = pre_migrations(&sandbox.data_dir());
    assert_eq!(snapshots.len(), 1, "{snapshots:?}");
    let version: i32 = rusqlite::Connection::open(&snapshots[0])
        .and_then(|db| db.query_row("SELECT max(version) FROM refinery_schema_history", [], |r| r.get(0)))
        .expect("snapshot schema");
    assert_eq!(version, UNCAUSED_SCHEMA, "the snapshot is the store as the old binary left it");
    assert_eq!(rows_in(&snapshots[0]), seeded, "the snapshot holds every row");

    let store = sandbox.data_dir().join("memory.db");
    assert_eq!(rows_in(&store), seeded, "the migration changed no row");
    let added: Vec<(Option<String>, Option<i64>)> = rusqlite::Connection::open(&store)
        .expect("open the store")
        .prepare("SELECT supersede_cause, revived_from FROM memory ORDER BY id")
        .and_then(|mut q| q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).and_then(Iterator::collect))
        .expect("read the causes");
    let causes: Vec<Option<&str>> = added.iter().map(|(cause, _)| cause.as_deref()).collect();
    assert_eq!(
        causes,
        [Some("revision"), None, None, Some("verdict"), Some("verdict"), None, Some("revision"), None]
    );
    assert!(added.iter().all(|(_, from)| from.is_none()), "{added:?}");

    let revised = published_delete(&sandbox.data_dir(), "/v1/memories/memory/2").await;
    assert_eq!(revised["revived"][0]["id"], 1, "the version before is current again: {revised}");
    assert!(revised.get("hidden").is_none(), "{revised}");
    let undone = published_delete(&sandbox.data_dir(), "/v1/memories/memory/3").await;
    assert!(undone.get("revived").is_none(), "a revision a verdict undid stays replaced: {undone}");
    assert_eq!(undone["hidden"], json!([4]), "{undone}");
    let judged = published_delete(&sandbox.data_dir(), "/v1/memories/memory/6").await;
    assert_eq!(judged["hidden"], json!([5]), "{judged}");
}

// The server runs in this process while the binary is waited on.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_binary_that_migrated_and_failed_gets_its_pre_migration_restored() {
    let sandbox = Sandbox::new();
    let (installed, old) = store_behind(&sandbox).await;
    let original = std::fs::read(&installed).expect("read the installed binary");
    let release = signed_release("0.19.0", r#","serve_fault":"after_open""#);
    let server = ReleaseServer::serving(release.listing.clone(), release.files.clone()).await;
    let _stop = StopOnDrop { home: sandbox.home.clone() };

    let out = update_from(&sandbox, &installed, &server.base, &release.public);

    assert_eq!(out.status.code(), Some(1), "{}\n{}", stdout_of(&out), stderr_of(&out));
    let report = stderr_of(&out);
    assert!(report.contains("starting the new daemon"), "{report}");
    assert!(report.contains("Everything was put back: clawg 0.18.0 is installed"), "{report}");
    let snapshots = pre_migrations(&sandbox.data_dir());
    assert_eq!(snapshots.len(), 1, "{snapshots:?}");
    assert!(report.contains(&format!("The store was restored from {}", snapshots[0].display())), "{report}");
    assert!(report.contains("What the new daemon had migrated is kept at"), "{report}");
    assert!(report.contains("Its daemon is running again"), "{report}");
    assert_eq!(std::fs::read(&installed).expect("read the installed binary"), original);
    assert!(beside(&installed, "clawg.new-").is_empty(), "the staged file is gone");

    old.wait_for_exit(Duration::from_secs(10)).await;
    let health = published_health(&sandbox.data_dir()).await;
    assert_eq!(health["version"], "0.18.0", "the old binary's daemon answers: {health}");
    let kept: Vec<PathBuf> = std::fs::read_dir(sandbox.data_dir().join("backups"))
        .expect("backups")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.to_string_lossy().ends_with("-pre-restore.db"))
        .collect();
    assert_eq!(kept.len(), 1, "the migrated store is kept: {kept:?}");
    let migrated = rusqlite::Connection::open(&kept[0]).expect("open the kept store");
    let version: i32 = migrated
        .query_row("SELECT max(version) FROM refinery_schema_history", [], |r| r.get(0))
        .expect("kept schema");
    assert_eq!(version, last_migration(), "the kept store is the one the new daemon migrated");
    // Read from the file: a daemon frozen at an old schema runs the current
    // queries, which name columns that schema lacks, so its list is a 500.
    let restored = rusqlite::Connection::open(sandbox.data_dir().join("memory.db")).expect("open the store");
    let version: i32 = restored
        .query_row("SELECT max(version) FROM refinery_schema_history", [], |r| r.get(0))
        .expect("store schema");
    assert_eq!(version, OLD_SCHEMA, "the store is the one from before the migration");
    let rows: Vec<String> = restored
        .prepare("SELECT content FROM memory ORDER BY id")
        .and_then(|mut q| q.query_map([], |r| r.get(0)).and_then(Iterator::collect))
        .expect("read the rows");
    assert_eq!(rows, SEEDED, "the seeded rows are back");
}

// The server runs in this process while the binary is waited on.
#[tokio::test(flavor = "multi_thread")]
async fn an_update_whose_candidate_refuses_the_store_leaves_the_old_binary_and_its_daemon() {
    let sandbox = Sandbox::new();
    // The installed binary knows every migration and has migrated the store;
    // the candidate knows one less, as an older release would.
    let (installed, old) = seeded_store(&sandbox).await;
    let original = std::fs::read(&installed).expect("read the installed binary");
    let behind = format!(r#","migrations_up_to":{}"#, last_migration() - 1);
    let release = signed_release("0.19.0", &behind);
    let server = ReleaseServer::serving(release.listing.clone(), release.files.clone()).await;
    let _stop = StopOnDrop { home: sandbox.home.clone() };
    let backups_before = std::fs::read_dir(sandbox.data_dir().join("backups")).map(|rd| rd.count()).unwrap_or(0);

    let out = update_from(&sandbox, &installed, &server.base, &release.public);

    assert_eq!(out.status.code(), Some(1), "{}
{}", stdout_of(&out), stderr_of(&out));
    let report = stderr_of(&out);
    assert!(report.contains("newer than the"), "the refusal is named: {report}");
    assert!(report.contains("Everything was put back: clawg 0.18.0 is installed"), "{report}");
    assert!(report.contains("Its daemon is running again"), "{report}");
    assert_eq!(std::fs::read(&installed).expect("read the installed binary"), original);
    assert!(beside(&installed, "clawg.new-").is_empty(), "the staged file is gone");

    old.wait_for_exit(Duration::from_secs(10)).await;
    let health = published_health(&sandbox.data_dir()).await;
    assert_eq!(health["version"], "0.18.0", "{health}");
    let backups_after = std::fs::read_dir(sandbox.data_dir().join("backups")).map(|rd| rd.count()).unwrap_or(0);
    assert_eq!(backups_after, backups_before, "the refused store was not snapshotted or touched");
    let rows = published_get(&sandbox.data_dir(), "/v1/memories?limit=100").await;
    for text in SEEDED {
        assert!(rows.contains(text), "{text:?} is still there: {rows}");
    }
}

// The server runs in this process while the binary is waited on.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_binary_that_does_not_start_brings_the_old_one_and_its_daemon_back() {
    let sandbox = Sandbox::new();
    let installed = install(&sandbox, &stamp_json("0.18.0", ""));
    let original = std::fs::read(&installed).expect("read the installed binary");
    let old = sandbox.serve_binary(&installed, "fake").await;
    let release = signed_release("0.19.0", r#","serve_fault":"before_open""#);
    let server = ReleaseServer::serving(release.listing.clone(), release.files.clone()).await;
    let _stop = StopOnDrop { home: sandbox.home.clone() };

    let out = update_from(&sandbox, &installed, &server.base, &release.public);

    assert_eq!(out.status.code(), Some(1), "{}\n{}", stdout_of(&out), stderr_of(&out));
    let report = stderr_of(&out);
    assert!(report.contains("starting the new daemon"), "{report}");
    assert!(report.contains("Everything was put back: clawg 0.18.0 is installed"), "{report}");
    assert!(report.contains("Its daemon is running again"), "{report}");
    assert_eq!(std::fs::read(&installed).expect("read the installed binary"), original);
    assert!(beside(&installed, "clawg.new-").is_empty(), "the staged file is gone");
    assert!(beside(&installed, &format!("clawg{}.old-", std::env::consts::EXE_SUFFIX)).is_empty());

    old.wait_for_exit(Duration::from_secs(10)).await;
    let health = published_health(&sandbox.data_dir()).await;
    assert_eq!(health["version"], "0.18.0", "the old binary's daemon answers: {health}");
    assert_ne!(health["pid"].as_u64(), Some(u64::from(old.pid)), "started again, not the one that was stopped");
}

// The daemon runs from another copy than the one that updates, as after a
// file replaced under a running daemon: it reports a version the installed
// binary does not have, and the one brought back must be waited for by the
// installed binary's own version.
// The server runs in this process while the binary is waited on.
#[tokio::test(flavor = "multi_thread")]
async fn a_stale_daemon_is_brought_back_by_the_installed_binary_when_the_new_one_does_not_start() {
    let sandbox = Sandbox::new();
    let installed = install(&sandbox, &stamp_json("0.18.0", ""));
    let original = std::fs::read(&installed).expect("read the installed binary");
    let other = sandbox.home.join("other").join(installed.file_name().expect("name"));
    std::fs::create_dir_all(other.parent().expect("dir")).expect("other dir");
    write_executable(&other, &stamped_bytes(&stamp_json("0.17.0", "")));
    let old = sandbox.serve_binary(&other, "fake").await;
    let release = signed_release("0.19.0", r#","serve_fault":"before_open""#);
    let server = ReleaseServer::serving(release.listing.clone(), release.files.clone()).await;
    let _stop = StopOnDrop { home: sandbox.home.clone() };

    // The wait for a daemon of the wrong version lasts ten minutes, so the
    // update is given a deadline instead of being waited on.
    let (out_log, err_log) = (sandbox.log("update-out"), sandbox.log("update-err"));
    let mut update = Command::new(&installed)
        .arg("update")
        .env("CLAWG_TEST_HOME", &sandbox.home)
        .env("CLAWG_TEST_EMBEDDER", "fake")
        .env("CLAWG_TEST_RELEASE_URL", &server.base)
        .env("CLAWG_TEST_RELEASE_KEYS", json!([release.public]).to_string())
        .env_remove("CLAWG_PROFILE")
        .envs(socket_only())
        .current_dir(&sandbox.home)
        .stdin(Stdio::null())
        .stdout(File::create(&out_log).expect("log file"))
        .stderr(File::create(&err_log).expect("log file"))
        .spawn_copy()
        .expect("run clawg update");
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = update.try_wait().expect("wait for the update") {
            break status;
        }
        if std::time::Instant::now() > deadline {
            let _ = update.kill();
            let _ = update.wait();
            panic!(
                "the update did not finish in 120 s\n{}",
                std::fs::read_to_string(&err_log).unwrap_or_default()
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };

    let report = std::fs::read_to_string(&err_log).expect("update stderr");
    assert_eq!(status.code(), Some(1), "{}\n{report}", std::fs::read_to_string(&out_log).unwrap_or_default());
    assert!(report.contains("Everything was put back: clawg 0.18.0 is installed"), "{report}");
    assert!(report.contains("Its daemon is running again"), "{report}");
    assert_eq!(std::fs::read(&installed).expect("read the installed binary"), original);

    old.wait_for_exit(Duration::from_secs(10)).await;
    let health = published_health(&sandbox.data_dir()).await;
    assert_eq!(health["version"], "0.18.0", "the installed binary's daemon answers: {health}");
}

// The server runs in this process while the binary is waited on.
#[tokio::test(flavor = "multi_thread")]
async fn a_full_update_with_a_daemon_replaces_the_binary_and_a_bridge_reaches_the_new_daemon() {
    let sandbox = Sandbox::new();
    let installed = install(&sandbox, &stamp_json("0.18.0", ""));
    let original = std::fs::read(&installed).expect("read the installed binary");
    let old = sandbox.serve_binary(&installed, "fake").await;
    let mut bridge = sandbox.bridge_of(&installed);
    bridge.initialize();
    assert!(!tool_failed(&bridge.call(2, "memory_status", json!({}))));
    let release = signed_release("0.19.0", "");
    let server = ReleaseServer::serving(release.listing.clone(), release.files.clone()).await;
    let _stop = StopOnDrop { home: sandbox.home.clone() };

    let out = update_from(&sandbox, &installed, &server.base, &release.public);

    assert_eq!(out.status.code(), Some(0), "{}\n{}", stdout_of(&out), stderr_of(&out));
    let report = stdout_of(&out);
    assert!(report.contains("0.18.0") && report.contains("0.19.0"), "{report}");
    old.wait_for_exit(Duration::from_secs(10)).await;
    let health = published_health(&sandbox.data_dir()).await;
    assert_eq!(health["version"], "0.19.0", "{health}");
    assert_ne!(health["pid"].as_u64(), Some(u64::from(old.pid)), "a new daemon, not the old one");

    assert_eq!(std::fs::read(&installed).expect("read the installed binary"), release.binary);
    let aside = beside(&installed, &format!("clawg{}.old-", std::env::consts::EXE_SUFFIX));
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert_eq!(std::fs::read(&aside[0]).expect("read the old binary"), original);
    assert!(beside(&installed, "clawg.new-").is_empty(), "the staged file is gone");

    let state = state_of(&sandbox.data_dir()).expect("daemon.json");
    assert!(state.get("port").is_none(), "the new daemon was started without a port: {state}");

    let reply = bridge.call(3, "memory_status", json!({}));
    assert!(!tool_failed(&reply), "the open bridge must find the new daemon: {reply}");
}

/// A home whose socket path is over the limit of `sun_path` and under
/// Windows' `MAX_PATH`: a daemon started there with `--port` serves TCP alone.
fn home_over_the_socket_limit() -> (tempfile::TempDir, PathBuf) {
    let owned = tempfile::tempdir().expect("tempdir");
    let used = owned.path().join(".clawg-dev").join("clawg.sock").as_os_str().len();
    let home = owned.path().join("h".repeat(130usize.saturating_sub(used)));
    std::fs::create_dir_all(&home).unwrap();
    (owned, home)
}

// The server runs in this process while the binary is waited on.
#[tokio::test(flavor = "multi_thread")]
async fn an_update_keeps_the_port_of_a_daemon_that_serves_tcp_alone() {
    let (_owned, home) = home_over_the_socket_limit();
    let sandbox = Sandbox::at(&home);
    let installed = install(&sandbox, &stamp_json("0.18.0", ""));
    let old = sandbox.serve_binary(&installed, "fake").await;
    let before = state_of(&sandbox.data_dir()).expect("daemon.json");
    assert!(before.get("unix_socket").is_none_or(|v| v == false), "the path does not fit: {before}");
    let release = signed_release("0.19.0", "");
    let server = ReleaseServer::serving(release.listing.clone(), release.files.clone()).await;
    let _stop = StopOnDrop { home: sandbox.home.clone() };

    let out = update_from(&sandbox, &installed, &server.base, &release.public);

    assert_eq!(out.status.code(), Some(0), "{}\n{}", stdout_of(&out), stderr_of(&out));
    old.wait_for_exit(Duration::from_secs(10)).await;
    let after = state_of(&sandbox.data_dir()).expect("daemon.json");
    assert_eq!(after["port"], before["port"], "the new daemon kept the port of the old one: {after}");
    assert!(after.get("unix_socket").is_none_or(|v| v == false), "{after}");
    assert_eq!(published_health(&sandbox.data_dir()).await["version"], "0.19.0");
}

// The server runs in this process while the binary is waited on.
#[tokio::test(flavor = "multi_thread")]
async fn an_undone_update_restarts_a_daemon_that_served_tcp_alone_on_its_port() {
    let (_owned, home) = home_over_the_socket_limit();
    let sandbox = Sandbox::at(&home);
    let installed = install(&sandbox, &stamp_json("0.18.0", ""));
    let old = sandbox.serve_binary(&installed, "fake").await;
    let before = state_of(&sandbox.data_dir()).expect("daemon.json");
    let release = signed_release("0.19.0", r#","serve_fault":"before_open""#);
    let server = ReleaseServer::serving(release.listing.clone(), release.files.clone()).await;
    let _stop = StopOnDrop { home: sandbox.home.clone() };

    let out = update_from(&sandbox, &installed, &server.base, &release.public);

    assert_eq!(out.status.code(), Some(1), "{}\n{}", stdout_of(&out), stderr_of(&out));
    let report = stderr_of(&out);
    assert!(report.contains("Its daemon is running again"), "{report}");
    old.wait_for_exit(Duration::from_secs(10)).await;
    let after = state_of(&sandbox.data_dir()).expect("daemon.json");
    assert_eq!(after["port"], before["port"], "the daemon started again kept its port: {after}");
    assert_eq!(published_health(&sandbox.data_dir()).await["version"], "0.18.0");
}

// ── Clients older than the socket ──────────────────────────────────────────

/// A stamp's extra field that makes the copy speak TCP alone, as a clawg
/// older than the socket.
const OLDER_TRANSPORT: &str = r#","transport":"tcp""#;

#[tokio::test(flavor = "multi_thread")]
async fn an_older_updater_installs_the_socket_version_and_sees_it_answer() {
    let sandbox = Sandbox::new();
    let installed = install(&sandbox, &stamp_json("0.18.0", OLDER_TRANSPORT));
    let old = sandbox.serve_binary(&installed, "fake").await;
    let before = state_of(&sandbox.data_dir()).expect("daemon.json");
    assert_eq!(before["unix_socket"], false, "{before}");
    let release = signed_release("0.19.0", "");
    let server = ReleaseServer::serving(release.listing.clone(), release.files.clone()).await;
    let _stop = StopOnDrop { home: sandbox.home.clone() };

    let out = update_from(&sandbox, &installed, &server.base, &release.public);

    assert_eq!(out.status.code(), Some(0), "{}\n{}", stdout_of(&out), stderr_of(&out));
    old.wait_for_exit(Duration::from_secs(10)).await;
    let after = state_of(&sandbox.data_dir()).expect("daemon.json");
    assert_eq!(after["unix_socket"], true, "{after}");
    assert_eq!(after["port"], before["port"], "the older updater passed the port on and watched it: {after}");
    assert_eq!(published_health(&sandbox.data_dir()).await["version"], "0.19.0");
}

#[tokio::test]
async fn an_older_bridge_reaches_a_new_daemon_over_the_profile_port() {
    let sandbox = Sandbox::new();
    let port = std::net::TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").port();
    let _daemon = sandbox.serve_as(&SERVE_WITHOUT_PORT, &[("CLAWG_TEST_DEFAULT_PORT", &port.to_string())]).await;
    let state = state_of(&sandbox.data_dir()).expect("daemon.json");
    assert_eq!((state["unix_socket"].clone(), state["port"].clone()), (json!(true), json!(port)), "{state}");
    let older = stamped_copy(&sandbox.home, &stamp_json(env!("CARGO_PKG_VERSION"), OLDER_TRANSPORT));
    let mut bridge = sandbox.bridge_of(&older);

    bridge.initialize();
    let saved = bridge.call(2, "save_memory", json!({ "content": "an older bridge still reaches the daemon" }));

    assert!(!tool_failed(&saved), "{saved}");
}

// ── Client setup ───────────────────────────────────────────────────────────

/// `clawg setup` with Claude Code's configuration in `<home>/claude-config`
/// and `path` as the whole PATH, so neither the real configuration nor a
/// real `claude` is ever reached.
fn setup(sandbox: &Sandbox, args: &[&str], path: &Path) -> Output {
    sandbox
        .command(&[&["setup", "claude-code", "--profile", "dev"], args].concat())
        .env("CLAUDE_CONFIG_DIR", sandbox.home.join("claude-config"))
        .env("PATH", path)
        .output()
        .expect("run clawg setup")
}

#[tokio::test]
async fn setup_dry_run_writes_nothing() {
    let sandbox = Sandbox::new();
    let empty = sandbox.home.join("empty-path");
    std::fs::create_dir_all(&empty).unwrap();
    let out = setup(&sandbox, &["--dry-run"], &empty);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    let text = stdout_of(&out);
    assert!(text.contains("mcp server clawg-dev: would register it"), "{text}");
    assert!(text.contains("would add SessionStart, UserPromptSubmit"), "{text}");
    assert!(!sandbox.home.join("claude-config").exists());
    assert!(!sandbox.data_dir().exists());
}

#[cfg(unix)]
#[tokio::test]
async fn setup_without_claude_prints_the_command_and_installs_the_rest() {
    let sandbox = Sandbox::new();
    let empty = sandbox.home.join("empty-path");
    std::fs::create_dir_all(&empty).unwrap();
    let out = setup(&sandbox, &[], &empty);
    assert_eq!(out.status.code(), Some(1), "{}", stderr_of(&out));
    let text = stdout_of(&out);
    let bin = std::fs::canonicalize(BIN).unwrap();
    let line = format!("claude mcp add --scope user clawg-dev -- {} mcp --profile dev --agent claude-code", bin.display());
    assert!(text.contains(&line), "{text}");
    let settings: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.home.join("claude-config").join("settings.json")).unwrap()).unwrap();
    let hook = &settings["hooks"]["SessionStart"][0]["hooks"][0];
    assert_eq!(hook["command"], json!(bin), "{settings}");
    assert_eq!(hook["args"], json!(["hook", "session-start", "--agent", "claude-code", "--profile", "dev"]), "{settings}");
    let md = std::fs::read_to_string(sandbox.home.join("claude-config").join("CLAUDE.md")).unwrap();
    assert!(md.starts_with("<!-- BEGIN CLAWG MEMORY PROTOCOL -->"), "{md}");
}

#[cfg(unix)]
#[tokio::test]
async fn setup_registers_through_a_fake_claude_and_remove_undoes_it() {
    let sandbox = Sandbox::new();
    let fake = sandbox.home.join("fake-bin");
    std::fs::create_dir_all(&fake).unwrap();
    write_executable(&fake.join("claude"), b"#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"${0%/*}/calls\"\n");
    let config = sandbox.home.join("claude-config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("CLAUDE.md"), "# Mine\n").unwrap();

    let out = setup(&sandbox, &[], &fake);
    assert_eq!(out.status.code(), Some(0), "{}{}", stdout_of(&out), stderr_of(&out));
    let bin = std::fs::canonicalize(BIN).unwrap();
    let calls = std::fs::read_to_string(fake.join("calls")).unwrap();
    let expected = format!(
        "mcp\nadd\n--scope\nuser\nclawg-dev\n--\n{}\nmcp\n--profile\ndev\n--agent\nclaude-code\n",
        bin.display()
    );
    assert_eq!(calls, expected);
    assert!(stdout_of(&out).contains("previous file copied to"), "{}", stdout_of(&out));

    // What `claude mcp add` would have written.
    let registered = json!({ "mcpServers": { "clawg-dev": {
        "type": "stdio", "command": bin, "args": ["mcp", "--profile", "dev", "--agent", "claude-code"], "env": {} } } });
    std::fs::write(config.join(".claude.json"), registered.to_string()).unwrap();
    let again = setup(&sandbox, &[], &fake);
    assert_eq!(again.status.code(), Some(0), "{}", stdout_of(&again));
    assert!(stdout_of(&again).contains("mcp server clawg-dev: already registered"), "{}", stdout_of(&again));
    assert_eq!(std::fs::read_to_string(fake.join("calls")).unwrap(), expected, "claude is not run again");

    let removed = setup(&sandbox, &["--remove"], &fake);
    assert_eq!(removed.status.code(), Some(0), "{}", stdout_of(&removed));
    let calls = std::fs::read_to_string(fake.join("calls")).unwrap();
    assert!(calls.ends_with("mcp\nremove\n--scope\nuser\nclawg-dev\n"), "{calls}");
    assert_eq!(std::fs::read_to_string(config.join("CLAUDE.md")).unwrap(), "# Mine\n");
    assert_eq!(std::fs::read_to_string(config.join("settings.json")).unwrap(), "{}\n");
}

// ── purging a project delete ───────────────────────────────────────────────

#[tokio::test]
async fn a_deleted_project_is_purged_once_the_daemon_stops() {
    let sandbox = Sandbox::new();
    let daemon = sandbox.serve().await;
    daemon.wait_until_ready().await;
    let root = sandbox.home.join("work").join("app");
    std::fs::create_dir_all(root.join(".git")).expect("repo");
    let (status, registered) =
        daemon.post("/v1/projects", json!({ "root_path": root.to_string_lossy(), "assigned": true })).await;
    assert_eq!(status, 200, "{registered}");
    let project = registered["project_id"].as_str().expect("a project").to_string();
    let (status, body) = daemon
        .post(
            "/v1/memories",
            json!({ "content": "the vault passphrase is zebra-quokka-7741", "agent_id": "smoke", "project_id": project }),
        )
        .await;
    assert!(status == 201 || status == 202, "{body}");
    daemon.wait_until_ingested().await;
    let run = |args: &[&str]| {
        let mut argv = args.to_vec();
        argv.extend_from_slice(&["--profile", "dev"]);
        sandbox.run(&argv)
    };

    let report = run(&["projects", "delete", &project, "--with-memories"]);
    assert_eq!(report.status.code(), Some(0), "{}", stderr_of(&report));
    let text = stdout_of(&report);
    let code = text.lines().last().and_then(|l| l.split("--confirm ").nth(1)).expect("the exact command").trim_end_matches('`');
    let deleted = run(&["projects", "delete", &project, "--with-memories", "--confirm", code]);
    assert_eq!(deleted.status.code(), Some(0), "{}", stderr_of(&deleted));
    assert!(stdout_of(&deleted).contains("as delete 1") && stdout_of(&deleted).contains("pre-delete"), "{}", stdout_of(&deleted));

    let without_yes = run(&["purge", "--project-delete", "1"]);
    assert_eq!(without_yes.status.code(), Some(1));
    assert!(stderr_of(&without_yes).contains("--yes"), "{}", stderr_of(&without_yes));
    let running = run(&["purge", "--project-delete", "1", "--yes"]);
    assert_eq!(running.status.code(), Some(1), "the daemon holds the store");
    assert!(stderr_of(&running).contains("clawg stop"), "{}", stderr_of(&running));

    assert_eq!(run(&["stop"]).status.code(), Some(0));
    daemon.wait_for_exit(Duration::from_secs(5)).await;
    let purged = run(&["purge", "--project-delete", "1", "--yes"]);
    assert_eq!(purged.status.code(), Some(0), "{}", stderr_of(&purged));
    let said = stdout_of(&purged);
    assert!(said.starts_with(&format!("purged delete 1 of project {project}: 1 memory removed for good")), "{said}");
    assert!(said.contains("pre-delete"), "the snapshot that still holds it: {said}");
    for name in ["memory.db", "memory.db-wal"] {
        let bytes = std::fs::read(sandbox.data_dir().join(name)).unwrap_or_default();
        assert!(!bytes.windows(6).any(|w| w == b"quokka"), "{name} still holds the text");
    }
    let again = run(&["purge", "--project-delete", "1", "--yes"]);
    assert_eq!(again.status.code(), Some(1));
    assert!(stderr_of(&again).contains("already purged"), "{}", stderr_of(&again));
}
