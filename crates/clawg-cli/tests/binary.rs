//! The `clawg` binary itself, run as child processes.
//!
//! Everything in `api.rs` runs the daemon inside the test process, so what
//! only exists between processes goes unseen there: a hard kill, the store
//! lock between two daemons, the files a bridge reads to find its daemon, and
//! MCP over a real stdin and stdout. Each test here gets its own home through
//! `CLAWG_TEST_HOME` and the fake model through `CLAWG_TEST_EMBEDDER`, which
//! only a `testing` build (this one) reads. Every daemon binds `--port 0`.
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
        let mut cmd = Command::new(BIN);
        cmd.args(args)
            .env("CLAWG_TEST_HOME", &self.home)
            .env("CLAWG_TEST_EMBEDDER", "fake")
            .env_remove("CLAWG_PROFILE")
            .env("RUST_LOG", "info")
            .current_dir(&self.home)
            .stdin(Stdio::null());
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
        let log = self.log(&format!("serve-{}", now_ns()));
        let child = self
            .command(&["serve", "--profile", "dev", "--port", "0"])
            .env("CLAWG_TEST_EMBEDDER", embedder)
            .stdout(Stdio::null())
            .stderr(File::create(&log).expect("log file"))
            .spawn()
            .expect("spawn clawg serve");
        Daemon::watch(child, log).answering(&self.data_dir()).await
    }

    /// A bridge started in an empty folder of its own, so the root it
    /// registers has nothing to index.
    fn bridge(&self) -> Bridge {
        let root = self.home.join(format!("project-{}", now_ns()));
        std::fs::create_dir_all(&root).expect("project dir");
        let log = self.log(&format!("mcp-{}", now_ns()));
        let mut child = self
            .command(&["mcp", "--profile", "dev", "--no-autostart", "--agent", "smoke"])
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(File::create(&log).expect("log file"))
            .spawn()
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
    port: u16,
    token: String,
    http: reqwest::Client,
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
        Self { child, pid, log, port: 0, token: String::new(), http: reqwest::Client::new() }
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
            if let Some((port, token)) = published(data_dir, self.pid) {
                let health = self.http.get(format!("http://127.0.0.1:{port}/health")).send().await;
                if health.is_ok_and(|r| r.status().is_success()) {
                    self.port = port;
                    self.token = token;
                    return self;
                }
            }
            assert!(Instant::now() < deadline, "clawg serve did not answer:\n{}", self.log_text());
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    async fn health(&self) -> Value {
        let r = self.http.get(self.url("/health")).send().await.expect("health");
        r.json().await.expect("health json")
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let r = self
            .http
            .post(self.url(path))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .expect("request");
        let status = r.status().as_u16();
        (status, r.json().await.unwrap_or(Value::Null))
    }

    async fn list(&self) -> String {
        let r = self
            .http
            .get(self.url("/v1/memories?limit=100"))
            .bearer_auth(&self.token)
            .send()
            .await
            .expect("list");
        assert!(r.status().is_success(), "{}", r.status());
        r.text().await.expect("list body")
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

/// Port and token, once `daemon.json` names `pid`.
fn published(data_dir: &Path, pid: u32) -> Option<(u16, String)> {
    let state: Value =
        serde_json::from_str(&std::fs::read_to_string(data_dir.join("daemon.json")).ok()?).ok()?;
    if state["pid"].as_u64() != Some(u64::from(pid)) {
        return None;
    }
    let port = u16::try_from(state["port"].as_u64()?).ok()?;
    let token = std::fs::read_to_string(data_dir.join("token")).ok()?;
    Some((port, token.trim().to_string()))
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
    assert_eq!(tool_names(&tools).len(), 12, "{tools}");

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
    assert_eq!(tool_names(&tools).len(), 12, "{tools}");

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
    let mut bytes = std::fs::read(BIN).expect("read the test binary");
    bytes.push(0);
    bytes.extend_from_slice(format!("CLAWG-TEST-STAMP {json}").as_bytes());
    std::fs::write(&copy, bytes).expect("write the copy");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    copy
}

fn run_binary(sandbox: &Sandbox, bin: &Path, args: &[&str]) -> Output {
    Command::new(bin)
        .args(args)
        .env("CLAWG_TEST_HOME", &sandbox.home)
        .env_remove("CLAWG_PROFILE")
        .current_dir(&sandbox.home)
        .stdin(Stdio::null())
        .output()
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
        use axum::routing::get;
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = hits.clone();
        let app = axum::Router::new().route(
            "/repos/MarcossIC/claw-garden/releases",
            get(move || {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let body = releases.to_string();
                async move { ([("content-type", "application/json")], body) }
            }),
        );
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
        .current_dir(&sandbox.home)
        .stdin(Stdio::null())
        .output()
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
