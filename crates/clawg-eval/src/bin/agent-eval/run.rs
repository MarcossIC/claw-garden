//! One run: its directory laid out from the plan, the session it plans run
//! (the daemon of its home, the seed, the client in its working directory
//! with its stream captured, the stop), and the result scored from what it
//! left. [`evaluate`] is pure; the rest is the I/O around it.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use clawg_core::paths::Profile;
use clawg_core::preferences::Lang;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::arm::{Arm, Daemon, Layout, Options, RunPlan, Step, TEST_HOME_VAR};
use crate::rubric::{self, Mark, SaveMarks};
use crate::score::{self, Score, TracerFate};
use crate::stream::{self, Outcome, Transcript};
use crate::task::{self, Task};

pub const RUN_FILE: &str = "run.json";
pub const STREAM_FILE: &str = "stream.jsonl";
pub const CLIENT_LOG: &str = "client.log";
pub const RESULT_FILE: &str = "result.json";
/// What the daemon of the run printed.
pub const SERVE_LOG: &str = "serve.log";
/// What each clawg command of the run printed, after its command line.
pub const CLAWG_LOG: &str = "clawg.log";

/// Generous: a cold debug binary on a loaded machine opens SQLite, runs every
/// migration and binds before it publishes itself.
const START_BUDGET: Duration = Duration::from_secs(60);

/// How long a daemon that listens may take to have its model ready: on its
/// first start in a run's home it downloads the model before it loads it.
const READY_BUDGET: Duration = Duration::from_secs(15 * 60);

/// How long the daemon may take to go once `clawg stop`, which already waits
/// for it, has returned.
const EXIT_BUDGET: Duration = Duration::from_secs(10);

const POLL: Duration = Duration::from_millis(50);

/// Between two `clawg status`, each a process of its own.
const STATUS_POLL: Duration = Duration::from_millis(250);

/// Who plays the session of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Client {
    /// The client the plan starts, [`crate::arm::CLIENT`]: a paid session.
    Claude,
    /// A recorded stream, replayed by `agent-eval replay-client`.
    Replay(PathBuf),
}

impl Client {
    pub fn name(&self) -> &'static str {
        match self {
            Client::Claude => "claude",
            Client::Replay(_) => "replay",
        }
    }

    /// The command that starts it: the plan's own, or `agent_eval` replaying
    /// the stream.
    fn step(&self, planned: &Step, agent_eval: &Path) -> Step {
        match self {
            Client::Claude => planned.clone(),
            Client::Replay(stream) => Step {
                program: agent_eval.to_string_lossy().into_owned(),
                args: vec!["replay-client".into(), stream.to_string_lossy().into_owned()],
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub client_version: Option<String>,
    /// The credential that paid, as the client names it.
    #[serde(default)]
    pub api_key_source: Option<String>,
    /// When the client started and last wrote its stream, in UTC: what a
    /// session is found by in the console's usage, which knows no session.
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub ended_at: Option<String>,
    pub mcp_servers: Vec<(String, String)>,
    pub tool_calls: usize,
    /// Calls to clawg's tools, by tool.
    pub clawg_calls: BTreeMap<String, usize>,
    pub clawg_calls_after_tool_search: usize,
    pub clawg_calls_in_subagents: usize,
    pub tool_search_calls: usize,
    pub hook_events: usize,
    pub compactions: usize,
    pub unknown_lines: usize,
    pub invalid_lines: Vec<usize>,
    pub outcome: Option<Outcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    pub task: String,
    pub category: String,
    pub example: bool,
    pub arm: String,
    /// [`Client::name`].
    pub client: String,
    pub lang: Option<Lang>,
    pub session: Session,
    pub score: Score,
    pub saves: Vec<SaveMarks>,
}

pub fn evaluate(
    task: &Task,
    arm: &str,
    client: &str,
    profile: Profile,
    lang: Option<Lang>,
    t: &Transcript,
    read_file: impl Fn(&str) -> Option<String>,
) -> RunResult {
    let server = profile.mcp_server_name();
    let prefix = format!("mcp__{server}__");
    let mut clawg_calls = BTreeMap::new();
    for call in t.server_calls(server) {
        *clawg_calls.entry(call.name[prefix.len()..].to_owned()).or_insert(0) += 1;
    }
    let session = Session {
        session_id: t.session_id.clone(),
        model: t.model.clone(),
        client_version: t.client_version.clone(),
        api_key_source: t.api_key_source.clone(),
        started_at: None,
        ended_at: None,
        mcp_servers: t.mcp_servers.clone(),
        tool_calls: t.calls.len(),
        clawg_calls,
        clawg_calls_after_tool_search: t.server_calls(server).filter(|c| c.after_tool_search).count(),
        clawg_calls_in_subagents: t.server_calls(server).filter(|c| c.subagent).count(),
        tool_search_calls: t.tool_search_calls(),
        hook_events: t.hooks.len(),
        compactions: t.compactions,
        unknown_lines: t.unknown_lines,
        invalid_lines: t.invalid_lines.clone(),
        outcome: t.result.clone(),
    };
    RunResult {
        task: task.id.clone(),
        category: task.category.clone(),
        example: task.example,
        arm: arm.to_owned(),
        client: client.to_owned(),
        lang,
        session,
        score: score::score(task, t, read_file),
        saves: rubric::judge_session(t, lang),
    }
}

/// One line for the terminal.
pub fn summary(r: &RunResult) -> String {
    let passed = r.score.checks.iter().filter(|c| c.passed).count();
    let fate = |f: TracerFate| r.score.tracers.iter().filter(|(_, x)| *x == f).count();
    let clawg: usize = r.session.clawg_calls.values().sum();
    let failed: usize = r.saves.iter().map(|s| s.count(Mark::Fail)).sum();
    format!(
        "{} {}: {}, checks {passed}/{}, tracers used {} retrieved_but_ignored {} never_reached {}, clawg calls {clawg} ({} after ToolSearch), saves {} with {failed} failed criteria",
        r.task,
        r.arm,
        if r.score.success { "success" } else { "failure" },
        r.score.checks.len(),
        fate(TracerFate::Used),
        fate(TracerFate::RetrievedButIgnored),
        fate(TracerFate::NeverReached),
        r.session.clawg_calls_after_tool_search,
        r.saves.len(),
    )
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("{0} already exists and is not empty; a run never writes over another")]
    NotEmpty(PathBuf),
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{0} is not a run directory: {1}")]
    NotARun(PathBuf, String),
    #[error("{0}")]
    Arm(#[from] crate::arm::ArmError),
    #[error(
        "{} is not a `testing` build of clawg: on Windows clawg finds its home through the system and not HOME, so this run would use the profile of the user who runs it; only a build with clawg-core's `testing` feature reads {TEST_HOME_VAR}",
        .0.display()
    )]
    NotIsolated(PathBuf),
    #[error("the daemon of the run exited with {code:?} before {before}; see {}", .log.display())]
    DaemonExited { code: Option<i32>, before: &'static str, log: PathBuf },
    #[error("the daemon of the run did not publish {} within {}s; see {}", .state_file.display(), START_BUDGET.as_secs(), .log.display())]
    DaemonSilent { state_file: PathBuf, log: PathBuf },
    #[error("the daemon of the run did not have its model ready within {}s; see {}", READY_BUDGET.as_secs(), .0.display())]
    ModelNotReady(PathBuf),
    #[error("the daemon of the run cannot load its model: {error}; see {}", .log.display())]
    ModelFailed { error: String, log: PathBuf },
    #[error("the daemon of the run was still there after `clawg stop` and was ended; see {}", .0.display())]
    DaemonStayed(PathBuf),
    #[error("`{command}` exited with {code:?} ({said}); see {}", .log.display())]
    Step { command: String, code: Option<i32>, said: String, log: PathBuf },
    #[error("the client exited with {code:?}; see {}", .log.display())]
    Client { code: Option<i32>, log: PathBuf },
    #[error("the run asks for {program} {expected}, and `{program} --version` reports {}", .found.as_deref().unwrap_or("no version"))]
    ClientVersion { program: String, expected: String, found: Option<String> },
    #[error("a paid run would be skewed: {}", join_leaks(.0))]
    PaidRun(Vec<Leak>),
}

/// What would reach a paid session from outside the run, or keep it from running.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Leak {
    #[error("ANTHROPIC_API_KEY is missing or empty (in `-p` the key pays for the session, and the run's config directory holds no login)")]
    NoApiKey,
    #[error("ANTHROPIC_AUTH_TOKEN is set (it outranks the key)")]
    AuthToken,
    #[error("CLAUDE_CODE_EFFORT_LEVEL is set (it overrides --effort)")]
    EffortLevel,
    #[error(
        "--out {} is below {}, which holds {name} (the client reads the instruction files of every ancestor of its working directory and takes the enclosing git repository as its project)",
        .out.display(), .dir.display()
    )]
    Context { out: PathBuf, dir: PathBuf, name: &'static str },
    #[error("the working directory {} holds {what} (the client tells the session its working directory, so the agent reads it)", .workdir.display())]
    Named { workdir: PathBuf, what: String },
}

fn join_leaks(leaks: &[Leak]) -> String {
    leaks.iter().map(|l| format!("\n- {l}")).collect()
}

/// What the client finds in a directory that changes what a session knows or
/// where its project and native memory are.
const CONTEXT_NAMES: [&str; 4] = [".git", "CLAUDE.md", "CLAUDE.local.md", ".claude/CLAUDE.md"];

/// Every problem of a paid run of `task`, from the variables that matter, from
/// `ancestors`, each directory from `out` up with the [`CONTEXT_NAMES`] it
/// holds, and from the path of its working directory.
fn leaks(
    api_key: Option<&str>,
    auth_token: Option<&str>,
    effort_level: Option<&str>,
    out: &Path,
    ancestors: &[(PathBuf, Vec<&'static str>)],
    task: &Task,
) -> Vec<Leak> {
    let set = |v: Option<&str>| v.is_some_and(|v| !v.is_empty());
    let mut found = Vec::new();
    if !set(api_key) {
        found.push(Leak::NoApiKey);
    }
    if set(auth_token) {
        found.push(Leak::AuthToken);
    }
    if set(effort_level) {
        found.push(Leak::EffortLevel);
    }
    for (dir, names) in ancestors {
        found.extend(names.iter().map(|name| Leak::Context { out: out.to_owned(), dir: dir.clone(), name }));
    }
    let workdir = Layout { root: out.to_owned() }.workdir();
    let path = workdir.to_string_lossy();
    let named = |what: String| Leak::Named { workdir: workdir.clone(), what };
    if task::contains_ci(&path, &task.id) {
        found.push(named(format!("the id of its task, {}", task.id)));
    }
    found.extend(task.tracers.iter().filter(|tr| task::contains_ci(&path, tr)).map(|tr| named(format!("the tracer {tr:?}"))));
    found
}

/// `out` and each of its ancestors, with the [`CONTEXT_NAMES`] each holds; one
/// that does not exist yet holds none.
fn probe_ancestors(out: &Path) -> Vec<(PathBuf, Vec<&'static str>)> {
    out.ancestors()
        .map(|dir| (dir.to_owned(), CONTEXT_NAMES.into_iter().filter(|name| dir.join(name).exists()).collect()))
        .collect()
}

/// Refuses a paid run of `task` whose key or `out` directory would skew it, naming each problem.
pub fn check_paid_run(out: &Path, task: &Task) -> Result<(), RunError> {
    let out = std::path::absolute(out).map_err(io(out))?;
    let var = |name: &str| std::env::var_os(name).map(|v| v.to_string_lossy().into_owned());
    let found = leaks(
        var("ANTHROPIC_API_KEY").as_deref(),
        var("ANTHROPIC_AUTH_TOKEN").as_deref(),
        var("CLAUDE_CODE_EFFORT_LEVEL").as_deref(),
        &out,
        &probe_ancestors(&out),
        task,
    );
    if found.is_empty() { Ok(()) } else { Err(RunError::PaidRun(found)) }
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> RunError + '_ {
    move |source| RunError::Io { path: path.to_owned(), source }
}

/// Lays the run out in an empty or missing `layout.root`: the fixture copied
/// as the working directory, every planned file and `run.json`, which names
/// the client that plays the session.
pub fn prepare(task: &Task, arm: Arm, fixture: &Path, layout: &Layout, options: &Options, client: &Client) -> Result<RunPlan, RunError> {
    let root = &layout.root;
    if std::fs::read_dir(root).is_ok_and(|mut d| d.next().is_some()) {
        return Err(RunError::NotEmpty(root.clone()));
    }
    let plan = crate::arm::plan(task, arm, layout, options)?;
    copy_dir(fixture, &layout.workdir())?;
    for dir in [layout.home(), layout.config_dir()] {
        std::fs::create_dir_all(&dir).map_err(io(&dir))?;
    }
    for (path, body) in &plan.files {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        std::fs::write(path, body).map_err(io(path))?;
    }
    let record = json!({
        "clawg_eval": env!("CARGO_PKG_VERSION"),
        "task": task,
        "arm": arm.as_str(),
        "profile": options.profile.as_str(),
        "lang": options.lang,
        "model": options.model,
        "effort": options.effort,
        "max_budget_usd": options.max_budget_usd,
        "claude_version": options.claude_version,
        "client": client,
        "plan": plan,
    });
    let path = root.join(RUN_FILE);
    std::fs::write(&path, serde_json::to_string_pretty(&record).expect("JSON serializes") + "\n").map_err(io(&path))?;
    Ok(plan)
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), RunError> {
    std::fs::create_dir_all(to).map_err(io(to))?;
    for entry in std::fs::read_dir(from).map_err(io(from))? {
        let entry = entry.map_err(io(from))?;
        let target = to.join(entry.file_name());
        if entry.file_type().map_err(io(&entry.path()))?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(io(&target))?;
        }
    }
    Ok(())
}

/// Refuses, on Windows, a run that would start a clawg that does not keep to
/// the run's home. There clawg finds the home through the system, not `HOME`,
/// and only a `testing` build reads [`TEST_HOME_VAR`]: the name in the binary
/// is the mark of one, the mark `release.yml` checks a release never carries.
pub fn check_isolation(windows: bool, arm: Arm, clawg: &Path) -> Result<(), RunError> {
    if !windows || !arm.has_clawg() {
        return Ok(());
    }
    let file = File::open(clawg).map_err(io(clawg))?;
    if holds(file, TEST_HOME_VAR.as_bytes(), 1 << 20).map_err(io(clawg))? {
        Ok(())
    } else {
        Err(RunError::NotIsolated(clawg.to_owned()))
    }
}

/// Whether `needle`, not empty, is somewhere in what `reader` gives, read
/// `chunk` bytes at a time.
fn holds(mut reader: impl Read, needle: &[u8], chunk: usize) -> std::io::Result<bool> {
    let keep = needle.len() - 1;
    let mut buf = vec![0; keep + chunk];
    let mut held = 0;
    loop {
        let read = match reader.read(&mut buf[held..]) {
            Ok(0) => return Ok(false),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        let end = held + read;
        if buf[..end].windows(needle.len()).any(|w| w == needle) {
            return Ok(true);
        }
        held = keep.min(end);
        buf.copy_within(end - held..end, 0);
    }
}

/// The version `claude --version` printed: its first word, as in
/// `2.1.300 (Claude Code)`.
fn reported_version(output: &str) -> Option<&str> {
    output.split_whitespace().next()
}

/// Refuses a `program` that does not report `expected`, so no session runs
/// on another version than the one the run is pinned to.
pub fn check_client_version(program: &str, expected: &str) -> Result<(), RunError> {
    let output = Command::new(program).arg("--version").stdin(Stdio::null()).output().map_err(io(Path::new(program)))?;
    let printed = String::from_utf8_lossy(&output.stdout);
    let found = if output.status.success() { reported_version(&printed) } else { None };
    if found == Some(expected) {
        Ok(())
    } else {
        Err(RunError::ClientVersion { program: program.to_owned(), expected: expected.to_owned(), found: found.map(str::to_owned) })
    }
}

/// The plan `prepare` left in `root`'s `run.json`, and its client.
fn read_run(root: &Path) -> Result<(RunPlan, Client), RunError> {
    let mut record = read_record(root)?;
    let not_a_run = |e: serde_json::Error| RunError::NotARun(root.to_owned(), e.to_string());
    let plan = serde_json::from_value(record["plan"].take()).map_err(not_a_run)?;
    let client = serde_json::from_value(record["client"].take()).map_err(not_a_run)?;
    Ok((plan, client))
}

fn read_record(root: &Path) -> Result<Value, RunError> {
    let path = root.join(RUN_FILE);
    let text = std::fs::read_to_string(&path).map_err(io(&path))?;
    serde_json::from_str(&text).map_err(|e| RunError::NotARun(root.to_owned(), e.to_string()))
}

/// Runs the session `prepare` laid out in `root`, as its `run.json` plans it:
/// with clawg, the daemon of the run's home first, then the commands before
/// the session, the client and the commands after it, which also run when the
/// seed or the client failed. A seed that does not load starts no client.
/// `agent_eval`, this binary, is the replay client.
pub fn execute(root: &Path, agent_eval: &Path) -> Result<(), RunError> {
    let (plan, client) = read_run(root)?;
    let client = &client.step(&plan.client, agent_eval);
    let layout = Layout { root: root.to_owned() };
    let Some(daemon) = &plan.daemon else {
        return client_ran(start_client(&plan, &layout, client)?, root);
    };
    let served = Served::start(daemon, &plan.env, root)?;
    served.listening(&daemon.state_file)?;
    let log_path = root.join(CLAWG_LOG);
    let mut log = File::create(&log_path).map_err(io(&log_path))?;
    let session = served
        .ready(&daemon.status, &plan.env, root, &mut log, &log_path)
        .and_then(|()| plan.before.iter().try_for_each(|step| run_step(step, &plan.env, root, &mut log, &log_path)))
        .and_then(|()| client_ran(start_client(&plan, &layout, client)?, root));
    let stopped = plan.after.iter().try_for_each(|step| run_step(step, &plan.env, root, &mut log, &log_path));
    let gone = served.gone_within(EXIT_BUDGET);
    session?;
    stopped?;
    if gone {
        Ok(())
    } else {
        Err(RunError::DaemonStayed(served.log.clone()))
    }
}

fn client_ran(code: Option<i32>, root: &Path) -> Result<(), RunError> {
    if code == Some(0) {
        Ok(())
    } else {
        Err(RunError::Client { code, log: root.join(CLIENT_LOG) })
    }
}

/// Runs `step` to its end with the plan's variables, and appends its command
/// line, what it printed and its exit code to `log`.
fn run_step(step: &Step, env: &[(String, String)], root: &Path, log: &mut File, log_path: &Path) -> Result<(), RunError> {
    let output = captured(step, env, root)?;
    write_output(log, log_path, step, &output)?;
    if output.status.success() {
        Ok(())
    } else {
        let said = last_line(&output.stdout, &output.stderr).to_owned();
        Err(RunError::Step { command: command_line(step), code: output.status.code(), said, log: log_path.to_owned() })
    }
}

fn command_line(step: &Step) -> String {
    std::iter::once(&step.program).chain(&step.args).map(String::as_str).collect::<Vec<_>>().join(" ")
}

fn captured(step: &Step, env: &[(String, String)], root: &Path) -> Result<Output, RunError> {
    Command::new(&step.program)
        .args(&step.args)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .map_err(io(Path::new(&step.program)))
}

fn write_output(log: &mut File, log_path: &Path, step: &Step, output: &Output) -> Result<(), RunError> {
    writeln!(log, "$ {}", command_line(step))
        .and_then(|()| log.write_all(&output.stdout))
        .and_then(|()| log.write_all(&output.stderr))
        .and_then(|()| writeln!(log, "exit {:?}\n", output.status.code()))
        .map_err(io(log_path))
}

/// The last line a command that failed printed, its stderr first: the
/// sentence that says why, so the error is read without opening the log.
fn last_line<'a>(stdout: &'a [u8], stderr: &'a [u8]) -> &'a str {
    let last = |bytes: &'a [u8]| std::str::from_utf8(bytes).ok().and_then(|text| text.lines().rev().map(str::trim).find(|l| !l.is_empty()));
    last(stderr).or_else(|| last(stdout)).unwrap_or("")
}

/// What the output of `clawg status` says of the model of the daemon `pid`.
#[derive(Debug, PartialEq, Eq)]
enum Readiness {
    Ready,
    /// Loading, being downloaded, failed in a way the daemon retries, or no
    /// answer from that daemon yet.
    NotYet,
    Failed(String),
}

fn readiness(status: &str, pid: u32) -> Readiness {
    let Ok(status) = serde_json::from_str::<Value>(status) else { return Readiness::NotYet };
    let health = &status["health"];
    if health["pid"].as_u64() != Some(u64::from(pid)) {
        return Readiness::NotYet;
    }
    match health["embedder"].as_str() {
        Some("ready") => Readiness::Ready,
        Some("failed") if health["embedder_retryable"] == Value::Bool(false) => {
            Readiness::Failed(health["embedder_error"].as_str().unwrap_or("no reason given").to_owned())
        }
        _ => Readiness::NotYet,
    }
}

/// Whether `state` is a daemon's state file that names `pid`.
fn names_pid(state: Option<&str>, pid: u32) -> bool {
    state.and_then(|text| serde_json::from_str::<Value>(text).ok()).is_some_and(|v| v["pid"].as_u64() == Some(u64::from(pid)))
}

/// The run's `clawg serve`, a child of this process. A thread reaps it as soon
/// as it exits: on Unix an unreaped child stays a zombie, and `clawg stop`,
/// which waits for its pid to go, would wait it out. Dropped while it still
/// runs, it is ended through its own handle, never by name.
struct Served {
    child: Arc<Mutex<Child>>,
    pid: u32,
    log: PathBuf,
}

fn lock(child: &Mutex<Child>) -> MutexGuard<'_, Child> {
    child.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Served {
    fn start(daemon: &Daemon, env: &[(String, String)], root: &Path) -> Result<Self, RunError> {
        let log = root.join(SERVE_LOG);
        let out = File::create(&log).map_err(io(&log))?;
        let err = out.try_clone().map_err(io(&log))?;
        let child = Command::new(&daemon.serve.program)
            .args(&daemon.serve.args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
            .map_err(io(Path::new(&daemon.serve.program)))?;
        let pid = child.id();
        let child = Arc::new(Mutex::new(child));
        let reaped = Arc::downgrade(&child);
        std::thread::spawn(move || {
            while let Some(child) = reaped.upgrade() {
                if !matches!(lock(&child).try_wait(), Ok(None)) {
                    return;
                }
                drop(child);
                std::thread::sleep(POLL);
            }
        });
        Ok(Self { child, pid, log })
    }

    fn exited(&self) -> Option<ExitStatus> {
        lock(&self.child).try_wait().ok().flatten()
    }

    /// Until `state_file` names it, which it writes once it listens.
    fn listening(&self, state_file: &Path) -> Result<(), RunError> {
        let deadline = Instant::now() + START_BUDGET;
        loop {
            if names_pid(std::fs::read_to_string(state_file).ok().as_deref(), self.pid) {
                return Ok(());
            }
            if let Some(status) = self.exited() {
                return Err(RunError::DaemonExited { code: status.code(), before: "it listened", log: self.log.clone() });
            }
            if Instant::now() >= deadline {
                return Err(RunError::DaemonSilent { state_file: state_file.to_owned(), log: self.log.clone() });
            }
            std::thread::sleep(POLL);
        }
    }

    /// Until `status` says its model is ready. The answer that decides, or
    /// the last one, goes to `log`.
    fn ready(&self, status: &Step, env: &[(String, String)], root: &Path, log: &mut File, log_path: &Path) -> Result<(), RunError> {
        let deadline = Instant::now() + READY_BUDGET;
        loop {
            let output = captured(status, env, root)?;
            let said = readiness(&String::from_utf8_lossy(&output.stdout), self.pid);
            let exited = self.exited();
            let late = Instant::now() >= deadline;
            if said != Readiness::NotYet || exited.is_some() || late {
                write_output(log, log_path, status, &output)?;
            }
            match said {
                Readiness::Ready => return Ok(()),
                Readiness::Failed(error) => return Err(RunError::ModelFailed { error, log: self.log.clone() }),
                Readiness::NotYet => {}
            }
            if let Some(exit) = exited {
                return Err(RunError::DaemonExited { code: exit.code(), before: "its model was ready", log: self.log.clone() });
            }
            if late {
                return Err(RunError::ModelNotReady(log_path.to_owned()));
            }
            std::thread::sleep(STATUS_POLL);
        }
    }

    fn gone_within(&self, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        while self.exited().is_none() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(POLL);
        }
        true
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        let mut child = lock(&self.child);
        if matches!(child.try_wait(), Ok(None)) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Starts `client` in the working directory with the plan's variables, its
/// stdout into `stream.jsonl` and its stderr into `client.log`. The exit code,
/// `None` when it was killed.
pub fn start_client(plan: &RunPlan, layout: &Layout, client: &Step) -> Result<Option<i32>, RunError> {
    let stream = layout.root.join(STREAM_FILE);
    let log = layout.root.join(CLIENT_LOG);
    let status = Command::new(&client.program)
        .args(&client.args)
        .envs(plan.env.iter().map(|(k, v)| (k, v)))
        .current_dir(layout.workdir())
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&stream).map_err(io(&stream))?)
        .stderr(std::fs::File::create(&log).map_err(io(&log))?)
        .status()
        .map_err(io(Path::new(&client.program)))?;
    Ok(status.code())
}

/// Scores the run in `root` from its `run.json`, `stream.jsonl` and working
/// directory, and writes `result.json`.
pub fn score_dir(root: &Path) -> Result<RunResult, RunError> {
    let not_a_run = |why: &str| RunError::NotARun(root.to_owned(), why.to_owned());
    let record = read_record(root)?;
    let client: Client = serde_json::from_value(record["client"].clone()).map_err(|e| not_a_run(&e.to_string()))?;
    let task: Task = serde_json::from_value(record["task"].clone()).map_err(|e| not_a_run(&e.to_string()))?;
    let arm = record["arm"].as_str().ok_or_else(|| not_a_run("no arm"))?.to_owned();
    let profile = match record["profile"].as_str() {
        Some("prod") => Profile::Prod,
        Some("dev") => Profile::Dev,
        _ => return Err(not_a_run("no profile")),
    };
    let lang = record["lang"].as_str().and_then(Lang::parse);
    let stream_path = root.join(STREAM_FILE);
    let stream = std::fs::read_to_string(&stream_path).map_err(io(&stream_path))?;
    let workdir = Layout { root: root.to_owned() }.workdir();
    let read_file = |rel: &str| task::is_relative_inside(rel).then(|| std::fs::read_to_string(workdir.join(rel)).ok()).flatten();
    let mut result = evaluate(&task, &arm, client.name(), profile, lang, &stream::parse(&stream), read_file);
    // The stream is created right before the client starts and closed when it exits.
    let meta = std::fs::metadata(&stream_path).map_err(io(&stream_path))?;
    let utc = |t: std::io::Result<std::time::SystemTime>| {
        let since = t.ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
        Some(clawg_cli::memories::format_utc(i64::try_from(since.as_millis()).ok()?))
    };
    result.session.started_at = utc(meta.created());
    result.session.ended_at = utc(meta.modified());
    let out = root.join(RESULT_FILE);
    std::fs::write(&out, serde_json::to_string_pretty(&result).expect("JSON serializes") + "\n").map_err(io(&out))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean_ancestors(out: &Path) -> Vec<(PathBuf, Vec<&'static str>)> {
        out.ancestors().map(|a| (a.to_owned(), Vec::new())).collect()
    }

    fn staging_port() -> Task {
        task::parse(
            r#"{"id": "staging-port", "category": "K1", "fixture": "f", "prompt": "Which port?",
                "memories": [{"content": "Staging port 48213"}], "tracers": ["48213"],
                "checks": [{"kind": "answer_contains", "text": "48213"}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn a_paid_run_with_its_key_and_a_clean_directory_has_no_leaks() {
        let out = Path::new("/runs/1");
        assert_eq!(leaks(Some("sk-key"), None, None, out, &clean_ancestors(out), &staging_port()), []);
    }

    #[test]
    fn a_missing_or_empty_key_a_token_and_an_effort_level_are_each_named() {
        let out = Path::new("/runs/1");
        let ancestors = clean_ancestors(out);
        let t = staging_port();
        assert_eq!(leaks(None, None, None, out, &ancestors, &t), [Leak::NoApiKey]);
        assert_eq!(leaks(Some(""), None, None, out, &ancestors, &t), [Leak::NoApiKey]);
        assert_eq!(
            leaks(None, Some("token"), Some("high"), out, &ancestors, &t),
            [Leak::NoApiKey, Leak::AuthToken, Leak::EffortLevel]
        );
        assert_eq!(leaks(Some("k"), Some(""), Some(""), out, &ancestors, &t), [], "an empty variable sets nothing");
    }

    #[test]
    fn a_working_directory_that_names_its_task_or_a_tracer_is_named() {
        let t = staging_port();
        let found = |out: &str| leaks(Some("k"), None, None, Path::new(out), &clean_ancestors(Path::new(out)), &t);
        let workdir = |out: &str| Layout { root: PathBuf::from(out) }.workdir();
        assert_eq!(found("/runs/t5f0c2a9e.a0.1"), []);
        assert_eq!(
            found("/runs/Staging-Port.a0.1"),
            [Leak::Named { workdir: workdir("/runs/Staging-Port.a0.1"), what: "the id of its task, staging-port".into() }]
        );
        assert_eq!(found("/runs/48213/1"), [Leak::Named { workdir: workdir("/runs/48213/1"), what: "the tracer \"48213\"".into() }]);
        let said = RunError::PaidRun(found("/runs/staging-port.a0.1")).to_string();
        assert!(said.contains("staging-port") && said.contains("the agent reads it"), "{said}");
    }

    #[test]
    fn an_out_below_a_repository_or_an_instruction_file_is_named_with_the_ancestor_that_holds_it() {
        let out = Path::new("/repo/runs/1");
        let ancestors = vec![
            (PathBuf::from("/repo/runs/1"), vec![]),
            (PathBuf::from("/repo/runs"), vec!["CLAUDE.local.md"]),
            (PathBuf::from("/repo"), vec![".git", ".claude/CLAUDE.md"]),
            (PathBuf::from("/"), vec![]),
        ];
        let found = leaks(Some("k"), None, None, out, &ancestors, &staging_port());
        assert_eq!(
            found,
            [
                Leak::Context { out: out.to_owned(), dir: PathBuf::from("/repo/runs"), name: "CLAUDE.local.md" },
                Leak::Context { out: out.to_owned(), dir: PathBuf::from("/repo"), name: ".git" },
                Leak::Context { out: out.to_owned(), dir: PathBuf::from("/repo"), name: ".claude/CLAUDE.md" },
            ]
        );
        let said = RunError::PaidRun(found).to_string();
        assert!(said.contains(".git") && said.contains("CLAUDE.local.md") && said.contains(".claude/CLAUDE.md"), "{said}");
    }

    #[test]
    fn the_probe_finds_what_an_ancestor_of_a_missing_out_holds() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::write(dir.path().join(".claude").join("CLAUDE.md"), "x").unwrap();
        let out = dir.path().join("a").join("run");
        let ancestors = probe_ancestors(&out);
        let holder = ancestors.iter().find(|(d, _)| d == dir.path()).unwrap();
        assert_eq!(holder.1, ["CLAUDE.md", ".claude/CLAUDE.md"]);
        assert!(ancestors.iter().find(|(d, _)| d == &out).unwrap().1.is_empty());
    }

    fn task() -> Task {
        task::parse(
            r#"{"id": "t", "category": "K1", "fixture": "f", "prompt": "Which port?",
                "memories": [{"content": "Staging port 48213"}], "tracers": ["48213"],
                "checks": [{"kind": "answer_contains", "text": "48213"}, {"kind": "file_contains", "path": "a.txt", "text": "ok"}]}"#,
        )
        .unwrap()
    }

    const STREAM: &str = r#"{"type":"system","subtype":"init","session_id":"s","mcp_servers":[{"name":"clawg-dev","status":"connected"}]}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"0","name":"ToolSearch","input":{}},{"type":"tool_use","id":"1","name":"mcp__clawg-dev__search_memory","input":{"query":"port"}},{"type":"tool_use","id":"2","name":"mcp__clawg-dev__search_memory","input":{"query":"staging"}}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"3","name":"mcp__clawg-dev__save_memory","input":{"content":"Staging port confirmed\nThe port is 48213, set in deploy/staging.env."}}]},"parent_tool_use_id":"x"}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"3","content":"saved"}]}}
{"type":"result","subtype":"success","is_error":false,"result":"48213"}
"#;

    #[test]
    fn evaluate_counts_clawg_calls_and_judges_the_saves() {
        let r = evaluate(&task(), "a3", "replay", Profile::Dev, None, &stream::parse(STREAM), |p| (p == "a.txt").then(|| "ok".into()));
        assert!(r.score.success);
        assert_eq!(r.session.clawg_calls, BTreeMap::from([("save_memory".to_owned(), 1), ("search_memory".to_owned(), 2)]));
        assert_eq!(r.session.clawg_calls_after_tool_search, 3);
        assert_eq!(r.session.clawg_calls_in_subagents, 1);
        assert_eq!(r.session.tool_search_calls, 1);
        assert_eq!(r.saves.len(), 1);
        assert_eq!(r.saves[0].count(Mark::Fail), 0, "{:?}", r.saves[0].marks);
        assert_eq!(
            summary(&r),
            "t a3: success, checks 2/2, tracers used 1 retrieved_but_ignored 0 never_reached 0, clawg calls 3 (3 after ToolSearch), saves 1 with 0 failed criteria"
        );
    }

    #[test]
    fn the_prod_server_is_counted_by_its_own_name() {
        let r = evaluate(&task(), "a3", "replay", Profile::Prod, None, &stream::parse(STREAM), |_| None);
        assert!(r.session.clawg_calls.is_empty());
        assert!(!r.score.success);
    }

    fn options(clawg: PathBuf) -> Options {
        Options {
            clawg,
            profile: Profile::Dev,
            lang: None,
            model: "m".into(),
            effort: None,
            max_budget_usd: None,
            claude_version: Some("2.1.300".into()),
            now_ms: 0,
            noise: Vec::new(),
        }
    }

    #[test]
    fn the_client_is_claude_as_planned_or_a_replay_of_a_stream() {
        let planned = Step { program: "claude".into(), args: vec!["-p".into(), "Which port?".into()] };
        let exe = Path::new("/bin/agent-eval");
        assert_eq!(Client::Claude.step(&planned, exe), planned);
        let replay = Client::Replay(PathBuf::from("/runs/s.jsonl"));
        assert_eq!(replay.step(&planned, exe), Step { program: "/bin/agent-eval".into(), args: vec!["replay-client".into(), "/runs/s.jsonl".into()] });
        assert_eq!((Client::Claude.name(), replay.name()), ("claude", "replay"));
        assert_eq!(serde_json::to_value(&Client::Claude).unwrap(), json!("claude"));
        assert_eq!(serde_json::to_value(&replay).unwrap(), json!({ "replay": "/runs/s.jsonl" }));
    }

    #[test]
    fn the_version_claude_reports_is_its_first_word() {
        assert_eq!(reported_version("2.1.300 (Claude Code)\n"), Some("2.1.300"));
        assert_eq!(reported_version("  2.1.301\n"), Some("2.1.301"));
        assert_eq!(reported_version(""), None);
    }

    #[test]
    fn prepare_lays_out_the_run_and_never_writes_over_another() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("fixture");
        std::fs::create_dir_all(fixture.join("src")).unwrap();
        std::fs::write(fixture.join("src/app.py"), "print(1)\n").unwrap();
        let layout = Layout { root: dir.path().join("run") };
        let opts = options(dir.path().join("clawg"));

        let plan = prepare(&task(), Arm::A3, &fixture, &layout, &opts, &Client::Claude).unwrap();

        assert_eq!(std::fs::read_to_string(layout.workdir().join("src/app.py")).unwrap(), "print(1)\n");
        for (path, body) in &plan.files {
            assert_eq!(&std::fs::read_to_string(path).unwrap(), body);
        }
        assert!(layout.config_dir().join("settings.json").is_file());
        let record: Value = serde_json::from_str(&std::fs::read_to_string(layout.root.join(RUN_FILE)).unwrap()).unwrap();
        assert_eq!(record["arm"], "a3");
        assert_eq!(record["task"]["id"], "t");
        assert_eq!((&record["client"], &record["claude_version"]), (&json!("claude"), &json!("2.1.300")));
        assert_eq!(read_run(&layout.root).unwrap(), (plan, Client::Claude), "a run runs what run.json says");
        assert!(matches!(prepare(&task(), Arm::A3, &fixture, &layout, &opts, &Client::Claude), Err(RunError::NotEmpty(_))));
    }

    #[test]
    fn on_windows_a_run_with_clawg_needs_a_testing_build() {
        let dir = tempfile::tempdir().unwrap();
        let testing = dir.path().join("testing");
        std::fs::write(&testing, b"\x7fELF\0\0CLAWG_TEST_HOME\0\0").unwrap();
        let other = dir.path().join("other");
        std::fs::write(&other, b"\x7fELF\0\0clawg 0.73.0\0").unwrap();
        let missing = dir.path().join("missing");

        assert!(check_isolation(true, Arm::A3, &testing).is_ok());
        let refused = check_isolation(true, Arm::A2, &other).unwrap_err();
        assert!(matches!(&refused, RunError::NotIsolated(path) if *path == other), "{refused}");
        assert!(refused.to_string().contains("testing"), "{refused}");
        assert!(matches!(check_isolation(true, Arm::B, &missing), Err(RunError::Io { .. })));
        assert!(check_isolation(true, Arm::A0, &missing).is_ok(), "an arm without clawg starts none");
        assert!(check_isolation(true, Arm::A4, &other).is_ok(), "an arm without clawg starts none");
        assert!(check_isolation(false, Arm::A3, &other).is_ok(), "elsewhere HOME keeps clawg in the run's home");
    }

    #[test]
    fn the_mark_is_found_wherever_a_read_cuts_it() {
        let needle = b"CLAWG_TEST_HOME";
        for chunk in [1, 2, 5, 14, 15, 16, 64] {
            for at in [0, 3, 13, 29] {
                let mut bytes = vec![b'x'; at];
                bytes.extend_from_slice(needle);
                bytes.extend_from_slice(b"yyyy");
                assert!(holds(std::io::Cursor::new(&bytes), needle, chunk).unwrap(), "chunk {chunk}, at {at}");
            }
            assert!(!holds(std::io::Cursor::new(b"xxCLAWG_TEST_HOMxCLAWG_TEST_"), needle, chunk).unwrap(), "chunk {chunk}");
            assert!(!holds(std::io::Cursor::new(b""), needle, chunk).unwrap());
        }
    }

    #[test]
    fn the_model_is_ready_once_the_status_of_this_daemon_says_so() {
        let status = |pid: u32, embedder: &str, extra: &str| {
            format!(r#"{{"running": true, "profile": "dev", "health": {{"status": "ok", "pid": {pid}, "embedder": "{embedder}"{extra}}}}}"#)
        };
        assert_eq!(readiness(&status(42, "ready", ""), 42), Readiness::Ready);
        assert_eq!(readiness(&status(42, "loading", ""), 42), Readiness::NotYet);
        assert_eq!(readiness(&status(43, "ready", ""), 42), Readiness::NotYet, "another daemon's model");
        let failed = r#", "embedder_error": "no model.onnx", "embedder_retryable": false"#;
        assert_eq!(readiness(&status(42, "failed", failed), 42), Readiness::Failed("no model.onnx".into()));
        let retried = r#", "embedder_error": "HTTP 503", "embedder_retryable": true"#;
        assert_eq!(readiness(&status(42, "failed", retried), 42), Readiness::NotYet, "the daemon retries the load itself");
        assert_eq!(readiness(r#"{"running": false, "health": null}"#, 42), Readiness::NotYet);
        assert_eq!(readiness("", 42), Readiness::NotYet);
    }

    #[test]
    fn a_step_that_fails_is_told_by_its_last_line() {
        assert_eq!(last_line(b"", b"warn: x\nclawg: the import failed: embedder_not_ready\n\n"), "clawg: the import failed: embedder_not_ready");
        assert_eq!(last_line(b"report\n", b""), "report");
        assert_eq!(last_line(b"", b" \n"), "");
    }

    #[test]
    fn a_daemon_listens_once_its_state_file_names_it() {
        let state = r#"{"pid": 4242, "port": 50000, "unix_socket": true, "profile": "dev", "version": "0.73.0", "started_at_ms": 1}"#;
        assert!(names_pid(Some(state), 4242));
        assert!(!names_pid(Some(state), 4243), "a state file another daemon left");
        assert!(!names_pid(Some("{\"pid\": 42"), 42), "a state file still being written");
        assert!(!names_pid(None, 4242));
    }

    #[test]
    fn score_dir_reads_the_run_back_and_writes_its_result() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("fixture");
        std::fs::create_dir_all(&fixture).unwrap();
        std::fs::write(fixture.join("a.txt"), "ok").unwrap();
        let layout = Layout { root: dir.path().join("run") };
        let replay = Client::Replay(dir.path().join("s.jsonl"));
        prepare(&task(), Arm::A0, &fixture, &layout, &options(dir.path().join("clawg")), &replay).unwrap();
        std::fs::write(layout.root.join(STREAM_FILE), STREAM).unwrap();

        let r = score_dir(&layout.root).unwrap();

        assert!(r.score.success);
        assert_eq!((r.arm.as_str(), r.client.as_str()), ("a0", "replay"));
        let written: Value = serde_json::from_str(&std::fs::read_to_string(layout.root.join(RESULT_FILE)).unwrap()).unwrap();
        assert_eq!(written["score"]["success"], true);
        assert!(matches!(score_dir(dir.path()), Err(RunError::Io { .. })));

        let claude = Layout { root: dir.path().join("claude") };
        prepare(&task(), Arm::A0, &fixture, &claude, &options(dir.path().join("clawg")), &Client::Claude).unwrap();
        std::fs::write(claude.root.join(STREAM_FILE), STREAM).unwrap();
        assert_eq!(score_dir(&claude.root).unwrap().client, "claude");
    }

    #[test]
    fn score_dir_dates_the_session_by_its_stream_and_names_the_credential() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("fixture");
        std::fs::create_dir_all(&fixture).unwrap();
        let layout = Layout { root: dir.path().join("run") };
        prepare(&task(), Arm::A0, &fixture, &layout, &options(dir.path().join("clawg")), &Client::Claude).unwrap();
        let stream = STREAM.replacen(r#""subtype":"init","#, r#""subtype":"init","apiKeySource":"ANTHROPIC_API_KEY","#, 1);
        std::fs::write(layout.root.join(STREAM_FILE), stream).unwrap();

        let r = score_dir(&layout.root).unwrap();

        assert_eq!(r.session.api_key_source.as_deref(), Some("ANTHROPIC_API_KEY"));
        let ended = r.session.ended_at.expect("every platform knows when a file was last written");
        assert!(ended.ends_with(" UTC"), "{ended}");
        if cfg!(windows) {
            assert!(r.session.started_at.is_some(), "Windows knows when a file was created");
        }
    }
}
