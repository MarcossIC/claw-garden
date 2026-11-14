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

use crate::arm::{Arm, Daemon, Layout, Options, RunPlan, Shape, Step, TEST_HOME_VAR};
use crate::hooks::{self, Loaded};
use crate::rubric::{self, Mark, SaveMarks};
use crate::score::{self, Journey, Score, SessionScore, TracerFate};
use crate::seed;
use crate::stream::{self, is_tool, Outcome, ToolCall, Transcript};
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
    /// A recorded stream, replayed by `agent-eval replay-client`, or a
    /// directory with a stream for each session.
    Replay(PathBuf),
}

impl Client {
    pub fn name(&self) -> &'static str {
        match self {
            Client::Claude => "claude",
            Client::Replay(_) => "replay",
        }
    }

    /// The command that starts a session: the plan's own, or `agent_eval`
    /// replaying a stream, which is the file `replayed` of the directory a
    /// replay names, with `mcp_config` when the run has a server.
    fn step(&self, planned: &Step, agent_eval: &Path, replayed: &str, mcp_config: Option<&Path>) -> Step {
        match self {
            Client::Claude => planned.clone(),
            Client::Replay(stream) => {
                let stream = if stream.is_dir() { stream.join(replayed) } else { stream.clone() };
                let mut args = vec!["replay-client".to_owned(), stream.to_string_lossy().into_owned()];
                if let Some(config) = mcp_config {
                    args.extend(["--mcp-config".to_owned(), config.to_string_lossy().into_owned()]);
                }
                Step { program: agent_eval.to_string_lossy().into_owned(), args }
            }
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
    /// [`Transcript::first_request_tokens`].
    #[serde(default)]
    pub first_request_tokens: Option<u64>,
    /// The context of every request of the main loop that has one, in order.
    #[serde(default)]
    pub request_tokens: Vec<u64>,
    /// How much of the first request's context the cache gave.
    #[serde(default)]
    pub first_request_cache_read: Option<u64>,
    /// Each instruction file the client reported loading, from the log the
    /// hook of every arm writes.
    #[serde(default)]
    pub instructions_loaded: Vec<Loaded>,
    /// What the task asked this session to leave untouched, and whether it did.
    #[serde(default)]
    pub untouched: Vec<UntouchedEntry>,
    pub outcome: Option<Outcome>,
}

/// An entry of a session's `untouched`, as the working directory held it when
/// the session ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UntouchedEntry {
    pub path: String,
    pub text: String,
    /// The file held the text: the session did what it was asked not to.
    pub present: bool,
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
    /// The record of the last session that ran.
    pub session: Session,
    /// The score of the run: every session and the task.
    pub score: Score,
    /// The `save_memory` calls of the last session, judged.
    pub saves: Vec<SaveMarks>,
    /// Each session that ran, in order; none in a result written before the
    /// sessions of a task, which is a run of one.
    #[serde(default)]
    pub sessions: Vec<SessionResult>,
    #[serde(default)]
    pub shape: Shape,
    /// How many rules of other topics the memory was seeded with.
    #[serde(default)]
    pub noise_rules: usize,
    /// Each fact born in a session, followed to the last one.
    #[serde(default)]
    pub facts: Vec<Fact>,
    /// [`overhead`], for a run of two sessions whose second one is new.
    #[serde(default)]
    pub overhead: Option<u64>,
    /// [`rereads`], for a run of two sessions.
    #[serde(default)]
    pub rereads: Option<Rereads>,
    /// The other branch of the fork this run is one of.
    #[serde(default)]
    pub paired: Option<String>,
}

impl RunResult {
    /// The record of every session that ran: a result written before the
    /// sessions of a task has the one it carries.
    pub fn records(&self) -> Vec<&Session> {
        if self.sessions.is_empty() { vec![&self.session] } else { self.sessions.iter().map(|s| &s.session).collect() }
    }

    /// The saves of every session that ran, judged.
    fn every_save(&self) -> Vec<&SaveMarks> {
        if self.sessions.is_empty() { self.saves.iter().collect() } else { self.sessions.iter().flat_map(|s| &s.saves).collect() }
    }
}

/// One session of a run, scored on its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionResult {
    pub session: Session,
    /// Its own checks, the task's tracers and whether it ended.
    pub score: Score,
    /// Every tracer of the task and each born in an earlier session, with what brought it.
    pub followed: Vec<Journey>,
    /// The tracers born in a session until this one that it wrote to a memory:
    /// in the input of `save_memory` or `update_memory`, or of a `Write` or
    /// `Edit` to the directory of the native memory.
    pub saved: Vec<String>,
    /// How many `Write` and `Edit` went to the directory of the native memory.
    pub native_memory_writes: usize,
    pub saves: Vec<SaveMarks>,
}

/// Where a fact born in a session ended up in the last one, told by what the
/// run shows and without judging it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactClass {
    /// No session since its own wrote it to a memory.
    NotSaved,
    SavedNotReached,
    ReachedNotUsed,
    Used,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    pub tracer: String,
    /// The session it was born in, from 1.
    pub born: usize,
    pub class: FactClass,
}

/// A session as it played, with what reached its context outside its stream.
pub struct Played {
    pub transcript: Transcript,
    pub delivered: Vec<String>,
    pub loaded: Vec<Loaded>,
    pub untouched: Vec<UntouchedEntry>,
}

/// What a run is besides the sessions it played.
pub struct RunInfo<'a> {
    pub arm: &'a str,
    pub client: &'a str,
    pub profile: Profile,
    pub lang: Option<Lang>,
    pub shape: Shape,
    pub noise_rules: usize,
    /// The working directory of the sessions, from which a relative path is read.
    pub workdir: &'a Path,
    /// Where the client keeps the native memory of the run, when its arm has one.
    pub native_dir: Option<&'a Path>,
}

/// A path as it is compared: the client writes it with either separator and any case.
fn norm_path(path: &str) -> String {
    path.trim_start_matches(r"\\?\").replace('\\', "/").to_lowercase()
}

fn in_dir(path: &str, dir: &str) -> bool {
    norm_path(path).starts_with(&format!("{}/", dir.trim_end_matches('/')))
}

fn file_path(call: &ToolCall) -> Option<&str> {
    call.input.get("file_path").and_then(Value::as_str)
}

/// A `Write` or `Edit` to a file in the directory of the native memory.
fn writes_native_memory(call: &ToolCall, native_dir: Option<&str>) -> bool {
    matches!(call.name.as_str(), "Write" | "Edit") && native_dir.is_some_and(|dir| file_path(call).is_some_and(|path| in_dir(path, dir)))
}

/// The tracers born in session `k` (from 0) or before that it wrote to a memory.
fn saved(task: &Task, k: usize, t: &Transcript, native_dir: Option<&str>) -> Vec<String> {
    let written: Vec<String> = t
        .calls
        .iter()
        .filter(|c| is_tool(&c.name, "save_memory") || is_tool(&c.name, "update_memory") || writes_native_memory(c, native_dir))
        .map(|c| c.input.to_string())
        .collect();
    task.sessions
        .iter()
        .take(k + 1)
        .flat_map(|s| &s.tracers)
        .filter(|tracer| written.iter().any(|input| score::shows(input, &task.id, tracer)))
        .cloned()
        .collect()
}

fn record(t: &Transcript, profile: Profile, loaded: Vec<Loaded>, untouched: Vec<UntouchedEntry>) -> Session {
    let server = profile.mcp_server_name();
    let prefix = format!("mcp__{server}__");
    let mut clawg_calls = BTreeMap::new();
    for call in t.server_calls(server) {
        *clawg_calls.entry(call.name[prefix.len()..].to_owned()).or_insert(0) += 1;
    }
    Session {
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
        first_request_tokens: t.first_request_tokens,
        request_tokens: t.requests.iter().filter_map(|r| r.context).collect(),
        first_request_cache_read: t.requests.iter().find(|r| r.context.is_some()).and_then(|r| r.cache_read),
        instructions_loaded: loaded,
        untouched,
        outcome: t.result.clone(),
    }
}

/// The run of `task` scored from the sessions it `played`, each on its own and
/// all together, with the path of each fact and the cost of the relay.
///
/// # Panics
///
/// When `played` is empty: a run has at least its first session.
pub fn evaluate(task: &Task, info: &RunInfo, played: &[Played], read_file: impl Fn(&str) -> Option<String>) -> RunResult {
    let native = info.native_dir.map(|dir| norm_path(&dir.to_string_lossy()));
    let scored: Vec<SessionScore> = played.iter().enumerate().map(|(k, p)| score::score_session(task, k, &p.transcript, &p.delivered)).collect();
    let transcripts: Vec<&Transcript> = played.iter().map(|p| &p.transcript).collect();
    let run_score = score::score(task, &transcripts, &scored, read_file);
    let sessions: Vec<SessionResult> = played
        .iter()
        .zip(scored)
        .enumerate()
        .map(|(k, (p, s))| SessionResult {
            session: record(&p.transcript, info.profile, p.loaded.clone(), p.untouched.clone()),
            score: s.score,
            followed: s.followed,
            saved: saved(task, k, &p.transcript, native.as_deref()),
            native_memory_writes: p.transcript.calls.iter().filter(|c| writes_native_memory(c, native.as_deref())).count(),
            saves: rubric::judge_session(&p.transcript, info.lang),
        })
        .collect();
    let last = sessions.last().expect("a run has its first session");
    let reread = (played.len() == 2).then(|| rereads(&played[0].transcript, &played[1].transcript, info.workdir, info.native_dir));
    let relay = reread.filter(|_| info.shape != Shape::Joined).map(|r| overhead(&played[0].transcript, &played[1].transcript, r.again));
    RunResult {
        task: task.id.clone(),
        category: task.category.clone(),
        example: task.example,
        arm: info.arm.to_owned(),
        client: info.client.to_owned(),
        lang: info.lang,
        session: last.session.clone(),
        score: run_score,
        saves: last.saves.clone(),
        facts: facts(task, &sessions),
        sessions,
        shape: info.shape,
        noise_rules: info.noise_rules,
        overhead: relay,
        rereads: reread,
        paired: None,
    }
}

/// Each fact born in a session that ran before the last, classified over the
/// last. It is saved only by a session from the one it was born in to the one
/// before the last: what the last session saves at the end brought nothing to it.
pub fn facts(task: &Task, sessions: &[SessionResult]) -> Vec<Fact> {
    let Some(last) = sessions.last() else { return Vec::new() };
    let mut told = Vec::new();
    for (i, spec) in task.sessions.iter().enumerate().take(sessions.len() - 1) {
        for tracer in &spec.tracers {
            let written = sessions[i..sessions.len() - 1].iter().any(|s| s.saved.contains(tracer));
            let fate = last.followed.iter().find(|j| &j.tracer == tracer).map(|j| j.fate);
            let class = match (written, fate) {
                (false, _) => FactClass::NotSaved,
                (true, Some(TracerFate::Used)) => FactClass::Used,
                (true, Some(TracerFate::RetrievedButIgnored)) => FactClass::ReachedNotUsed,
                (true, _) => FactClass::SavedNotReached,
            };
            told.push(Fact { tracer: tracer.clone(), born: i + 1, class });
        }
    }
    told
}

/// What a relay costs the new session, in tokens and without what any session
/// would pay: what its first request has over the first of the session before
/// (never below 0) plus `again`, what it reads again ([`rereads`]).
pub fn overhead(first: &Transcript, second: &Transcript, again: u64) -> u64 {
    let growth = match (first.first_request_tokens, second.first_request_tokens) {
        (Some(before), Some(after)) => after.saturating_sub(before),
        _ => 0,
    };
    growth + again
}

/// What the second session reads that the first already had, in tokens at 4
/// bytes a token.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rereads {
    /// The results of the `Read` of a file the first session read or wrote,
    /// except those of a file the second one edits and the first did not write;
    /// of the tools that read a memory; and of the `Read` of the native memory.
    pub again: u64,
    /// The results of the `Read` that `again` leaves out because of the edit:
    /// the client asks for a `Read` in the same session before an `Edit`, so any
    /// new session pays them, with a memory or without it.
    pub to_edit: u64,
}

const EDIT_TOOLS: [&str; 4] = ["Write", "Edit", "MultiEdit", "NotebookEdit"];

/// The tools that read a memory, whose answers a note of the first session would spare.
const MEMORY_READERS: [&str; 4] = ["search_memory", "recall_context", "read_memory", "list_memories"];

/// The file a call to a `Read` or an edit tool names.
fn touched(call: &ToolCall) -> Option<&str> {
    file_path(call).or_else(|| call.input.get("notebook_path").and_then(Value::as_str))
}

/// A path as the session wrote it, resolved from `workdir` (already as
/// [`norm_path`] writes it): the same file is found whatever its separators,
/// case, `.` and `..`.
fn resolved(path: &str, workdir: &str) -> String {
    let path = norm_path(path);
    let drive = path.as_bytes().get(1) == Some(&b':') && path.as_bytes()[0].is_ascii_alphabetic();
    let joined = if path.starts_with('/') || path.starts_with('~') || drive { path } else { format!("{workdir}/{path}") };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    format!("{}{}", if joined.starts_with('/') { "/" } else { "" }, parts.join("/"))
}

/// What `second` reads again after `first`, from `workdir`; a path is compared
/// resolved, with the native memory in `native_dir` when the arm has one.
pub fn rereads(first: &Transcript, second: &Transcript, workdir: &Path, native_dir: Option<&Path>) -> Rereads {
    let workdir = norm_path(&workdir.to_string_lossy());
    let native = native_dir.map(|dir| norm_path(&dir.to_string_lossy()));
    let paths = |t: &Transcript, tools: &[&str]| -> Vec<String> {
        t.calls.iter().filter(|c| tools.contains(&c.name.as_str())).filter_map(touched).map(|p| resolved(p, &workdir)).collect()
    };
    let read_before = paths(first, &["Read"]);
    let written_before = paths(first, &EDIT_TOOLS);
    let edited = paths(second, &EDIT_TOOLS);
    let (mut again, mut to_edit) = (0, 0);
    for call in &second.calls {
        let bytes = call.result.as_deref().map_or(0, str::len);
        if MEMORY_READERS.iter().any(|tool| is_tool(&call.name, tool)) {
            again += bytes;
        } else if call.name == "Read" {
            let Some(path) = touched(call).map(|p| resolved(p, &workdir)) else { continue };
            if native.as_deref().is_some_and(|dir| in_dir(&path, dir)) || written_before.contains(&path) {
                again += bytes;
            } else if read_before.contains(&path) {
                if edited.contains(&path) { to_edit += bytes } else { again += bytes }
            }
        }
    }
    Rereads { again: (again / 4) as u64, to_edit: (to_edit / 4) as u64 }
}

/// The prompts of the sessions before session `k` (from 1) when each continues
/// the conversation: they are in its context. A new session never saw them.
pub fn earlier_prompts(task: &Task, shape: Shape, k: usize) -> Vec<String> {
    if shape == Shape::Joined { task.prompts().iter().take(k.saturating_sub(1)).map(|p| (*p).to_owned()).collect() } else { Vec::new() }
}

/// The texts the plan put in the session's context that the stream does not
/// show: the native memory, as the client loads it at startup from what the
/// file held before the session (`before`, or the file of the plan), and each
/// file of the plan that `loaded` names. Paths match whatever their separators
/// and case, since the client may write them either way.
pub fn delivered(plan: &RunPlan, loaded: &[Loaded], before: Option<&str>) -> Vec<String> {
    let loaded: Vec<String> = loaded.iter().map(|l| norm_path(&l.file_path)).collect();
    plan.files
        .iter()
        .filter_map(|(path, body)| {
            if path.ends_with("memory/MEMORY.md") {
                Some(seed::native_loaded(before.unwrap_or(body)).to_owned())
            } else {
                loaded.contains(&norm_path(&path.to_string_lossy())).then(|| body.clone())
            }
        })
        .collect()
}

/// One line for the terminal.
pub fn summary(r: &RunResult) -> String {
    let passed = r.score.checks.iter().filter(|c| c.passed).count();
    let fate = |f: TracerFate| r.score.tracers.iter().filter(|(_, x)| *x == f).count();
    let records = r.records();
    let clawg: usize = records.iter().map(|s| s.clawg_calls.values().sum::<usize>()).sum();
    let after_search: usize = records.iter().map(|s| s.clawg_calls_after_tool_search).sum();
    let saves = r.every_save();
    let failed: usize = saves.iter().map(|s| s.count(Mark::Fail)).sum();
    let mut line = format!(
        "{} {}: {}, checks {passed}/{}, tracers used {} retrieved_but_ignored {} never_reached {}, clawg calls {clawg} ({after_search} after ToolSearch), saves {} with {failed} failed criteria",
        r.task,
        r.arm,
        if r.score.success { "success" } else { "failure" },
        r.score.checks.len(),
        fate(TracerFate::Used),
        fate(TracerFate::RetrievedButIgnored),
        fate(TracerFate::NeverReached),
        saves.len(),
    );
    if r.sessions.len() > 1 {
        let class = |c: FactClass| r.facts.iter().filter(|f| f.class == c).count();
        line.push_str(&format!(
            "; {} sessions {}, facts not_saved {} saved_not_reached {} reached_not_used {} used {}",
            r.sessions.len(),
            r.shape.as_str(),
            class(FactClass::NotSaved),
            class(FactClass::SavedNotReached),
            class(FactClass::ReachedNotUsed),
            class(FactClass::Used),
        ));
    }
    line
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
    #[error("session {k} did not end: {why}; see {}", .log.display())]
    Session { k: usize, why: String, log: PathBuf },
    #[error("the run asks for {program} {expected}, and `{program} --version` reports {}", .found.as_deref().unwrap_or("no version"))]
    ClientVersion { program: String, expected: String, found: Option<String> },
    #[error("a paid run would be skewed: {}", join_leaks(.0))]
    PaidRun(Vec<Leak>),
    #[error(
        "a paid run needs Claude Code {READ_BLOCK_FROM} or later, the first version that denies the reads outside the working directory, and --claude-version is {}",
        .found.as_ref().map_or_else(|| "no version".to_owned(), |v| format!("{v:?}"))
    )]
    ReadBlock { found: Option<String> },
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
    #[error("{command} is on the PATH, at {}, and the task needs it missing", .path.display())]
    Present { command: String, path: PathBuf },
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
    present: &[(String, PathBuf)],
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
    found.extend(task.every_tracer().filter(|tr| task::contains_ci(&path, tr)).map(|tr| named(format!("the tracer {tr:?}"))));
    found.extend(present.iter().map(|(command, path)| Leak::Present { command: command.clone(), path: path.clone() }));
    found
}

/// `out` and each of its ancestors, with the [`CONTEXT_NAMES`] each holds; one
/// that does not exist yet holds none.
fn probe_ancestors(out: &Path) -> Vec<(PathBuf, Vec<&'static str>)> {
    out.ancestors()
        .map(|dir| (dir.to_owned(), CONTEXT_NAMES.into_iter().filter(|name| dir.join(name).exists()).collect()))
        .collect()
}

/// Each command the task needs missing that a directory of `path` holds, with
/// the file that would run.
fn on_path(task: &Task, path: Option<&std::ffi::OsStr>) -> Vec<(String, PathBuf)> {
    let Some(path) = path else { return Vec::new() };
    let extensions: &[&str] = if cfg!(windows) { &["", ".exe", ".cmd", ".bat", ".com"] } else { &[""] };
    let dirs: Vec<PathBuf> = std::env::split_paths(path).collect();
    task.missing_commands
        .iter()
        .filter_map(|command| {
            dirs.iter()
                .flat_map(|dir| extensions.iter().map(move |ext| dir.join(format!("{command}{ext}"))))
                .find(|file| file.is_file())
                .map(|file| (command.clone(), file))
        })
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
        &on_path(task, std::env::var_os("PATH").as_deref()),
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
    copy_dir(fixture, &layout.workdir(), &[])?;
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
        "shape": options.shape,
        "noise": options.noise_name,
        "noise_rules": options.noise.len(),
        "plan": plan,
    });
    let path = root.join(RUN_FILE);
    std::fs::write(&path, serde_json::to_string_pretty(&record).expect("JSON serializes") + "\n").map_err(io(&path))?;
    Ok(plan)
}

/// Copies what `from` holds into `to`, except the entries named in `skip`,
/// wherever they are.
fn copy_dir(from: &Path, to: &Path, skip: &[&str]) -> Result<(), RunError> {
    std::fs::create_dir_all(to).map_err(io(to))?;
    for entry in std::fs::read_dir(from).map_err(io(from))? {
        let entry = entry.map_err(io(from))?;
        if skip.iter().any(|name| entry.file_name() == *name) {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.file_type().map_err(io(&entry.path()))?.is_dir() {
            copy_dir(&entry.path(), &target, skip)?;
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

/// The first version of the client that denies the reads outside its working
/// directories when `permissions.blockReadsOutsideWorkingDirectories` is set.
pub const READ_BLOCK_FROM: &str = "2.1.257";

/// The three numbers of a version such as `2.1.300`.
fn version_numbers(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.split('.').map(|part| if part.chars().all(|c| c.is_ascii_digit()) { part.parse().ok() } else { None });
    let numbers = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(numbers)
}

/// Refuses a paid run pinned to a client older than [`READ_BLOCK_FROM`]: with it
/// the sessions could read what the run keeps outside their working directory.
pub fn check_read_block(version: Option<&str>) -> Result<(), RunError> {
    let enough = version.and_then(version_numbers).zip(version_numbers(READ_BLOCK_FROM)).is_some_and(|(found, minimum)| found >= minimum);
    if enough { Ok(()) } else { Err(RunError::ReadBlock { found: version.map(str::to_owned) }) }
}

/// The plan `prepare` left in `root`'s `run.json`, and its client.
fn read_run(root: &Path) -> Result<(RunPlan, Client), RunError> {
    let mut record = read_record(root)?;
    let not_a_run = |e: serde_json::Error| RunError::NotARun(root.to_owned(), e.to_string());
    let plan = serde_json::from_value(record["plan"].take()).map_err(not_a_run)?;
    let client = serde_json::from_value(record["client"].take()).map_err(not_a_run)?;
    Ok((plan, client))
}

/// The task `prepare` recorded in `root`'s `run.json`.
fn read_task(root: &Path) -> Result<Task, RunError> {
    let mut record = read_record(root)?;
    serde_json::from_value(record["task"].take()).map_err(|e| RunError::NotARun(root.to_owned(), e.to_string()))
}

fn read_record(root: &Path) -> Result<Value, RunError> {
    let path = root.join(RUN_FILE);
    let text = std::fs::read_to_string(&path).map_err(io(&path))?;
    serde_json::from_str(&text).map_err(|e| RunError::NotARun(root.to_owned(), e.to_string()))
}

/// Runs the sessions `prepare` laid out in `root`, as its `run.json` plans
/// them: with clawg, the daemon of the run's home first, then the commands
/// before the sessions, the sessions in order on one working directory and the
/// commands after them, which also run when the seed or a session failed. A
/// seed that does not load starts no client, and a session that does not end
/// lets no other begin. A run that forks plays its second session twice, each
/// way from the state the first one left ([`fork_steps`]). `agent_eval`, this
/// binary, is the replay client.
pub fn execute(root: &Path, agent_eval: &Path) -> Result<(), RunError> {
    let (plan, client) = read_run(root)?;
    let task = read_task(root)?;
    let layout = Layout { root: root.to_owned() };
    let stage = Stage { plan: &plan, client: &client, layout: &layout, agent_eval, task: &task };
    if read_shape(&read_record(root)?) == Shape::Both {
        return fork(stage);
    }
    let Some(daemon) = &plan.daemon else {
        return play(&stage);
    };
    let served = Served::start(daemon, &plan.env, root, root.join(SERVE_LOG))?;
    served.listening(&daemon.state_file)?;
    let log_path = root.join(CLAWG_LOG);
    let mut log = File::create(&log_path).map_err(io(&log_path))?;
    let session = served
        .ready(&daemon.status, &plan.env, root, &mut log, &log_path)
        .and_then(|()| plan.before.iter().try_for_each(|step| run_step(step, &plan.env, root, &mut log, &log_path)))
        .and_then(|()| play(&stage));
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

/// The shape a `run.json` records; a run written before shapes is `split`.
fn read_shape(record: &Value) -> Shape {
    serde_json::from_value(record["shape"].clone()).unwrap_or_default()
}

/// What playing a session needs besides which one it is.
#[derive(Clone, Copy)]
struct Stage<'a> {
    plan: &'a RunPlan,
    client: &'a Client,
    layout: &'a Layout,
    agent_eval: &'a Path,
    task: &'a Task,
}

/// Plays the sessions of the plan in order.
fn play(stage: &Stage) -> Result<(), RunError> {
    for (i, planned) in stage.plan.sessions().into_iter().enumerate() {
        let k = i + 1;
        play_session(stage, k, planned, &format!("session-{k}.jsonl"))?;
    }
    Ok(())
}

/// Plays session `k` (from 1) with the command `planned`; a replay prints the
/// file `replayed` of its directory. Before it, the native memory the agent
/// finds is copied to `memory-before-K.md`; after it, in a run of several, the
/// instruction files the client loaded move to `instructions-K.jsonl`, so each
/// session has its own, and what the task asked it to leave untouched is read
/// into `untouched-K.json`. A session that is not the last must end well.
fn play_session(stage: &Stage, k: usize, planned: &Step, replayed: &str) -> Result<(), RunError> {
    let Stage { plan, client, layout, agent_eval, task } = *stage;
    let of = plan.sessions().len();
    if let Some(memory) = native_memory_file(plan).filter(|m| m.is_file()) {
        let copy = layout.root.join(format!("memory-before-{k}.md"));
        std::fs::copy(memory, &copy).map_err(io(&copy))?;
    }
    let mcp_config = plan.daemon.as_ref().map(|_| layout.mcp_config());
    let code = start_client(plan, layout, &client.step(planned, agent_eval, replayed, mcp_config.as_deref()), k)?;
    let (stream_name, log_name) = session_files(k, of);
    let log = layout.root.join(log_name);
    if code != Some(0) {
        return Err(RunError::Client { code, log });
    }
    let held = task.sessions.get(k - 1).map_or(&[][..], |s| &s.untouched[..]);
    if !held.is_empty() {
        let workdir = layout.workdir();
        let entries = untouched_entries(held, |rel| task::is_relative_inside(rel).then(|| std::fs::read_to_string(workdir.join(rel)).ok()).flatten());
        let path = layout.root.join(untouched_file(k));
        std::fs::write(&path, serde_json::to_string_pretty(&entries).expect("JSON serializes") + "\n").map_err(io(&path))?;
    }
    if k < of {
        let stream_path = layout.root.join(stream_name);
        let stream = std::fs::read_to_string(&stream_path).map_err(io(&stream_path))?;
        ended(&stream).map_err(|why| RunError::Session { k, why, log })?;
    }
    let instructions = layout.instructions_log();
    if of > 1 && instructions.is_file() {
        let moved = layout.root.join(instructions_file(k, of));
        std::fs::rename(&instructions, &moved).map_err(io(&moved))?;
    }
    Ok(())
}

/// Each file a session had to leave untouched, with whether it holds the text
/// it must not hold yet: a missing file does not, and the text is looked for
/// as a file check does, reading CRLF as LF.
fn untouched_entries(held: &[task::Untouched], read_file: impl Fn(&str) -> Option<String>) -> Vec<UntouchedEntry> {
    held.iter()
        .map(|u| UntouchedEntry { path: u.path.clone(), text: u.text.clone(), present: read_file(&u.path).is_some_and(|body| score::file_has(&body, &u.text)) })
        .collect()
}

/// The name of the file with what session `k` had to leave untouched.
fn untouched_file(k: usize) -> String {
    format!("untouched-{k}.json")
}

/// What a run that forks does, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkStep {
    /// The daemon of the run up with its model ready, and the seed loaded when
    /// `seed`; its log is kept with the branch it serves, if any.
    Start { branch: Option<Shape>, seed: bool },
    /// Session `k` (from 1), the second one continued or new according to its branch.
    Play { k: usize, branch: Option<Shape> },
    Stop,
    /// The state the first session left, copied to `fork/`.
    Save,
    /// The working directory and the files of the second session, moved under its branch.
    Keep(Shape),
    /// The state saved, put back.
    Restore,
}

/// The steps of a run that plays its first session once and its second one
/// twice from the same state, the continued one first so it finds the cache of
/// the first still warm. The daemon is down while the state is copied.
pub fn fork_steps(with_daemon: bool) -> Vec<ForkStep> {
    let part = |branch: Option<Shape>, k: usize, seed: bool| {
        let play = ForkStep::Play { k, branch };
        if with_daemon { vec![ForkStep::Start { branch, seed }, play, ForkStep::Stop] } else { vec![play] }
    };
    [
        part(None, 1, true),
        vec![ForkStep::Save],
        part(Some(Shape::Joined), 2, false),
        vec![ForkStep::Keep(Shape::Joined), ForkStep::Restore],
        part(Some(Shape::Split), 2, false),
        vec![ForkStep::Keep(Shape::Split)],
    ]
    .concat()
}

/// Where a fork keeps the state the first session left.
const FORK_DIR: &str = "fork";

/// The models in the data directory: hundreds of megabytes no session writes.
const MODELS_DIR: &str = "models";

/// The command of the second session as `branch` plays it: the plan's own for
/// the branch that continues, and for the new one the same without `--continue`.
fn branch_step(planned: &Step, branch: Shape) -> Step {
    let mut step = planned.clone();
    if branch == Shape::Split && step.args.last().is_some_and(|a| a == "--continue") {
        step.args.pop();
    }
    step
}

/// A run that forks: its state, the daemon it serves and the log of what its
/// clawg commands printed, as the steps of [`fork_steps`] go by.
struct Fork<'a> {
    stage: Stage<'a>,
    journal: Option<(File, PathBuf)>,
    served: Option<Served>,
}

fn fork(stage: Stage) -> Result<(), RunError> {
    let mut run = Fork { stage, journal: None, served: None };
    let mut outcome = Ok(());
    for step in fork_steps(stage.plan.daemon.is_some()) {
        outcome = run.apply(step);
        if outcome.is_err() {
            break;
        }
    }
    if outcome.is_err() && run.served.is_some() {
        // The daemon is stopped whatever failed; the failure is what is reported.
        let _ = run.apply(ForkStep::Stop);
    }
    outcome
}

impl Fork<'_> {
    fn data_dir(&self) -> Option<&Path> {
        self.stage.plan.daemon.as_ref().and_then(|d| d.state_file.parent())
    }

    fn apply(&mut self, step: ForkStep) -> Result<(), RunError> {
        let (plan, layout) = (self.stage.plan, self.stage.layout);
        let root = &layout.root;
        match step {
            ForkStep::Start { branch, seed } => {
                let daemon = plan.daemon.as_ref().expect("a daemon is started only for an arm that has one");
                let dir = branch.map_or_else(|| root.clone(), |b| root.join(b.as_str()));
                std::fs::create_dir_all(&dir).map_err(io(&dir))?;
                let served = self.served.insert(Served::start(daemon, &plan.env, root, dir.join(SERVE_LOG))?);
                served.listening(&daemon.state_file)?;
                if self.journal.is_none() {
                    let path = root.join(CLAWG_LOG);
                    self.journal = Some((File::create(&path).map_err(io(&path))?, path));
                }
                let (log, log_path) = self.journal.as_mut().expect("the log was just opened");
                served.ready(&daemon.status, &plan.env, root, log, log_path)?;
                if seed {
                    plan.before.iter().try_for_each(|step| run_step(step, &plan.env, root, log, log_path))?;
                }
                Ok(())
            }
            ForkStep::Play { k, branch } => {
                let planned = plan.sessions()[k - 1];
                let (step, replayed) = match branch {
                    None => (planned.clone(), format!("session-{k}.jsonl")),
                    Some(b) => (branch_step(planned, b), format!("{}-{k}.jsonl", b.as_str())),
                };
                play_session(&self.stage, k, &step, &replayed)
            }
            ForkStep::Stop => {
                let served = self.served.take().expect("a daemon is stopped only after it started");
                let (log, log_path) = self.journal.as_mut().expect("a daemon that started has its log");
                let stopped = plan.after.iter().try_for_each(|step| run_step(step, &plan.env, root, log, log_path));
                let gone = served.gone_within(EXIT_BUDGET);
                stopped?;
                if gone { Ok(()) } else { Err(RunError::DaemonStayed(served.log.clone())) }
            }
            ForkStep::Save => save_state(layout, self.data_dir()),
            ForkStep::Keep(branch) => {
                let dir = root.join(branch.as_str());
                std::fs::create_dir_all(&dir).map_err(io(&dir))?;
                let work = dir.join("work");
                std::fs::rename(layout.workdir(), &work).map_err(io(&work))?;
                let (stream, log) = session_files(2, 2);
                for name in [stream, log, instructions_file(2, 2), "memory-before-2.md".to_owned(), untouched_file(2)] {
                    let from = root.join(&name);
                    if from.is_file() {
                        std::fs::rename(&from, dir.join(&name)).map_err(io(&from))?;
                    }
                }
                Ok(())
            }
            ForkStep::Restore => restore_state(layout, self.data_dir()),
        }
    }
}

/// Copies what the first session left to `fork/`: the working directory, the
/// client's configuration and, when the run has clawg, its data directory
/// without the models. The daemon is not running: on Windows a file it holds
/// open cannot be replaced.
fn save_state(layout: &Layout, data: Option<&Path>) -> Result<(), RunError> {
    let fork = layout.root.join(FORK_DIR);
    copy_dir(&layout.workdir(), &fork.join("work"), &[])?;
    copy_dir(&layout.config_dir(), &fork.join("claude-config"), &[])?;
    if let Some(data) = data {
        copy_dir(data, &fork.join("data"), &[MODELS_DIR])?;
    }
    Ok(())
}

/// Puts back what [`save_state`] copied, in the same paths: whatever a branch
/// added or changed is removed first, and the models stay where they are.
fn restore_state(layout: &Layout, data: Option<&Path>) -> Result<(), RunError> {
    let fork = layout.root.join(FORK_DIR);
    let mut parts: Vec<(PathBuf, PathBuf, &[&str])> =
        vec![(fork.join("work"), layout.workdir(), &[]), (fork.join("claude-config"), layout.config_dir(), &[])];
    if let Some(data) = data {
        parts.push((fork.join("data"), data.to_owned(), &[MODELS_DIR]));
    }
    for (saved, live, keep) in parts {
        clear_dir(&live, keep)?;
        copy_dir(&saved, &live, &[])?;
    }
    Ok(())
}

/// Removes what `dir` holds except the entries named in `keep`; a directory
/// that is not there holds nothing.
fn clear_dir(dir: &Path, keep: &[&str]) -> Result<(), RunError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io(dir)(e)),
    };
    for entry in entries {
        let entry = entry.map_err(io(dir))?;
        if keep.iter().any(|name| entry.file_name() == *name) {
            continue;
        }
        let path = entry.path();
        let removed = if entry.file_type().map_err(io(&path))?.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        removed.map_err(io(&path))?;
    }
    Ok(())
}

// The native memory file of the plan, when its arm has one.
fn native_memory_file(plan: &RunPlan) -> Option<&Path> {
    plan.files.iter().map(|(path, _)| path.as_path()).find(|path| path.ends_with("memory/MEMORY.md"))
}

/// Whether a session whose stream is `stream` ended with a result that is not
/// an error, so the next may begin; if not, why.
fn ended(stream: &str) -> Result<(), String> {
    match stream::parse(stream).result {
        None => Err("its stream has no result".to_owned()),
        Some(result) if result.is_error => Err("its result is an error".to_owned()),
        Some(_) => Ok(()),
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
    /// Starts the daemon in `root`, its output into `log`.
    fn start(daemon: &Daemon, env: &[(String, String)], root: &Path, log: PathBuf) -> Result<Self, RunError> {
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
pub fn start_client(plan: &RunPlan, layout: &Layout, client: &Step, k: usize) -> Result<Option<i32>, RunError> {
    let (stream_name, log_name) = session_files(k, plan.sessions().len());
    let stream = layout.root.join(stream_name);
    let log = layout.root.join(log_name);
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

/// The names of the stream and the log of session `k` of `of`: bare when the
/// run has one session, numbered from 1 when it has several.
pub fn session_files(k: usize, of: usize) -> (String, String) {
    if of == 1 { (STREAM_FILE.to_owned(), CLIENT_LOG.to_owned()) } else { (format!("stream-{k}.jsonl"), format!("client-{k}.log")) }
}

/// The name of the log of the instruction files session `k` of `of` loaded.
fn instructions_file(k: usize, of: usize) -> String {
    if of == 1 { "instructions.jsonl".to_owned() } else { format!("instructions-{k}.jsonl") }
}

/// Scores the run in `root` from its `run.json`, the stream of every session
/// that ran, the logs of the instruction files each loaded, the native memory
/// each began with and the working directory, and writes `result.json`. A run
/// stopped by a session that failed is scored with the sessions it played. A
/// run that forked has no result of its own: each branch that ran is scored
/// as a run of two sessions of its shape, in its own directory.
pub fn score_dir(root: &Path) -> Result<Vec<RunResult>, RunError> {
    let record = read_record(root)?;
    if read_shape(&record) != Shape::Both {
        return Ok(vec![score_run(root, &record, None)?]);
    }
    let results: Vec<RunResult> = [Shape::Joined, Shape::Split]
        .into_iter()
        .filter(|branch| root.join(branch.as_str()).is_dir())
        .map(|branch| score_run(root, &record, Some(branch)))
        .collect::<Result<_, _>>()?;
    if results.is_empty() {
        return Err(RunError::NotARun(root.to_owned(), "no branch of the fork ran".to_owned()));
    }
    Ok(results)
}

/// The run of `root` scored as one of `shape`: with `branch`, the second
/// session and the working directory are those of that branch of a fork.
fn score_run(root: &Path, record: &Value, branch: Option<Shape>) -> Result<RunResult, RunError> {
    let not_a_run = |why: &str| RunError::NotARun(root.to_owned(), why.to_owned());
    let client: Client = serde_json::from_value(record["client"].clone()).map_err(|e| not_a_run(&e.to_string()))?;
    let task: Task = serde_json::from_value(record["task"].clone()).map_err(|e| not_a_run(&e.to_string()))?;
    let arm = record["arm"].as_str().ok_or_else(|| not_a_run("no arm"))?.to_owned();
    let profile = match record["profile"].as_str() {
        Some("prod") => Profile::Prod,
        Some("dev") => Profile::Dev,
        _ => return Err(not_a_run("no profile")),
    };
    let lang = record["lang"].as_str().and_then(Lang::parse);
    let plan: RunPlan = serde_json::from_value(record["plan"].clone()).map_err(|e| not_a_run(&e.to_string()))?;
    let shape = branch.unwrap_or_else(|| read_shape(record));
    let noise_rules = record["noise_rules"].as_u64().map_or(0, |n| n as usize);
    let of = plan.sessions().len();
    let branch_dir = branch.map(|b| root.join(b.as_str()));
    // The files of session `k`: the branch has those of the second session.
    let dir_of = |k: usize| match (&branch_dir, k) {
        (Some(dir), 2) => dir.clone(),
        _ => root.to_owned(),
    };
    // A stream is created right before its client starts and closed when it exits.
    let utc = |t: std::io::Result<std::time::SystemTime>| {
        let since = t.ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
        Some(clawg_cli::memories::format_utc(i64::try_from(since.as_millis()).ok()?))
    };
    let read_optional = |path: &Path| match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(path)(e)),
    };
    let mut played = Vec::new();
    let mut times = Vec::new();
    for k in 1..=of {
        let dir = dir_of(k);
        let stream_path = dir.join(session_files(k, of).0);
        let Some(stream) = read_optional(&stream_path)? else {
            if k == 1 {
                return Err(io(&stream_path)(std::io::ErrorKind::NotFound.into()));
            }
            break;
        };
        let loaded = read_optional(&dir.join(instructions_file(k, of)))?.map(|text| hooks::read_log(&text)).unwrap_or_default();
        let before = read_optional(&dir.join(format!("memory-before-{k}.md")))?;
        let mut texts = delivered(&plan, &loaded, before.as_deref());
        texts.extend(earlier_prompts(&task, shape, k));
        let meta = std::fs::metadata(&stream_path).map_err(io(&stream_path))?;
        times.push((utc(meta.created()), utc(meta.modified())));
        let untouched = match read_optional(&dir.join(untouched_file(k)))? {
            Some(text) => serde_json::from_str(&text).map_err(|e| not_a_run(&format!("{}: {e}", untouched_file(k))))?,
            None => Vec::new(),
        };
        played.push(Played { transcript: stream::parse(&stream), delivered: texts, loaded, untouched });
    }
    let out_dir = branch_dir.unwrap_or_else(|| root.to_owned());
    let workdir = Layout { root: out_dir.clone() }.workdir();
    let read_file = |rel: &str| task::is_relative_inside(rel).then(|| std::fs::read_to_string(workdir.join(rel)).ok()).flatten();
    let info = RunInfo { arm: &arm, client: client.name(), profile, lang, shape, noise_rules, workdir: &workdir, native_dir: native_memory_file(&plan).and_then(Path::parent) };
    let mut result = evaluate(&task, &info, &played, read_file);
    for (session, (started, ended)) in result.sessions.iter_mut().zip(times) {
        session.session.started_at = started;
        session.session.ended_at = ended;
    }
    if let Some(last) = result.sessions.last() {
        result.session = last.session.clone();
    }
    result.paired = branch.map(|b| if b == Shape::Joined { Shape::Split } else { Shape::Joined }.as_str().to_owned());
    let out = out_dir.join(RESULT_FILE);
    std::fs::write(&out, serde_json::to_string_pretty(&result).expect("JSON serializes") + "\n").map_err(io(&out))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arm::Shape;

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
        assert_eq!(leaks(Some("sk-key"), None, None, out, &clean_ancestors(out), &[], &staging_port()), []);
    }

    #[test]
    fn a_missing_or_empty_key_a_token_and_an_effort_level_are_each_named() {
        let out = Path::new("/runs/1");
        let ancestors = clean_ancestors(out);
        let t = staging_port();
        assert_eq!(leaks(None, None, None, out, &ancestors, &[], &t), [Leak::NoApiKey]);
        assert_eq!(leaks(Some(""), None, None, out, &ancestors, &[], &t), [Leak::NoApiKey]);
        assert_eq!(
            leaks(None, Some("token"), Some("high"), out, &ancestors, &[], &t),
            [Leak::NoApiKey, Leak::AuthToken, Leak::EffortLevel]
        );
        assert_eq!(leaks(Some("k"), Some(""), Some(""), out, &ancestors, &[], &t), [], "an empty variable sets nothing");
    }

    #[test]
    fn a_paid_run_refuses_a_client_without_the_read_block() {
        for version in ["2.1.257", "2.1.258", "2.1.300", "2.1.1000", "2.2.0", "2.10.0", "3.0.0"] {
            assert!(check_read_block(Some(version)).is_ok(), "{version}");
        }
        for version in ["2.1.256", "2.1.99", "2.0.999", "1.9.9", "2.1", "latest", ""] {
            let said = check_read_block(Some(version)).unwrap_err().to_string();
            assert!(said.contains("2.1.257") && said.contains(&format!("{version:?}")), "{version}: {said}");
        }
        let said = check_read_block(None).unwrap_err().to_string();
        assert!(said.contains("2.1.257") && said.contains("no version"), "{said}");
    }

    #[test]
    fn a_working_directory_that_names_its_task_or_a_tracer_is_named() {
        let t = staging_port();
        let found = |out: &str| leaks(Some("k"), None, None, Path::new(out), &clean_ancestors(Path::new(out)), &[], &t);
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
    fn a_working_directory_that_names_a_tracer_of_a_session_is_named() {
        let relay = task::parse(
            r#"{"id": "relay", "category": "R", "fixture": "f",
                "sessions": [{"prompt": "Decide on 4.5-41.", "tracers": ["4.5-41"]}, {"prompt": "Apply it."}],
                "checks": [{"kind": "answer_contains", "text": "x"}]}"#,
        )
        .unwrap();
        let out = "/runs/4.5-41/1";
        let found = leaks(Some("k"), None, None, Path::new(out), &clean_ancestors(Path::new(out)), &[], &relay);
        let workdir = Layout { root: PathBuf::from(out) }.workdir();
        assert_eq!(found, [Leak::Named { workdir, what: "the tracer \"4.5-41\"".into() }]);
    }

    #[test]
    fn a_paid_run_refuses_a_command_that_should_be_missing() {
        let eza = task::parse(
            r#"{"id": "listing", "category": "E", "fixture": "f", "prompt": "Count.", "shell": ["eza", "ls"], "missing_commands": ["eza"],
                "checks": [{"kind": "answer_contains", "text": "3"}]}"#,
        )
        .unwrap();
        let out = Path::new("/runs/1");
        let present = [("eza".to_owned(), PathBuf::from("/usr/bin/eza"))];
        let found = leaks(Some("k"), None, None, out, &clean_ancestors(out), &present, &eza);
        assert_eq!(found, [Leak::Present { command: "eza".into(), path: PathBuf::from("/usr/bin/eza") }]);
        let said = RunError::PaidRun(found).to_string();
        assert!(said.contains("eza") && said.contains("/usr/bin/eza") && said.contains("missing"), "{said}");
        assert_eq!(leaks(Some("k"), None, None, out, &clean_ancestors(out), &[], &eza), []);

        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let file = if cfg!(windows) { "eza.exe" } else { "eza" };
        std::fs::write(bin.join(file), "").unwrap();
        std::fs::write(bin.join("ls-other"), "").unwrap();
        let path = std::env::join_paths([dir.path().join("empty"), bin.clone()]).unwrap();
        assert_eq!(on_path(&eza, Some(&path)), [("eza".to_owned(), bin.join(file))]);
        assert_eq!(on_path(&eza, None), []);
        assert_eq!(on_path(&staging_port(), Some(&path)), [], "a task with no missing command looks for none");
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
        let found = leaks(Some("k"), None, None, out, &ancestors, &[], &staging_port());
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

    /// The result of a run that did not fork.
    fn scored(root: &Path) -> RunResult {
        let mut results = score_dir(root).unwrap();
        assert_eq!(results.len(), 1);
        results.remove(0)
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

    fn info(arm: &str, profile: Profile) -> RunInfo<'_> {
        RunInfo { arm, client: "replay", profile, lang: None, shape: Shape::Split, noise_rules: 0, workdir: Path::new(WORKDIR), native_dir: None }
    }

    fn played(stream: &str) -> Played {
        Played { transcript: stream::parse(stream), delivered: Vec::new(), loaded: Vec::new(), untouched: Vec::new() }
    }

    #[test]
    fn evaluate_counts_clawg_calls_and_judges_the_saves() {
        let r = evaluate(&task(), &info("a3", Profile::Dev), &[played(STREAM)], |p| (p == "a.txt").then(|| "ok".into()));
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
        let r = evaluate(&task(), &info("a3", Profile::Prod), &[played(STREAM)], |_| None);
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
            noise_name: "noise".into(),
            shape: Shape::Split,
            agent_eval: PathBuf::from("/bin/agent-eval"),
        }
    }

    #[test]
    fn the_client_is_claude_as_planned_or_a_replay_of_a_stream() {
        let planned = Step { program: "claude".into(), args: vec!["-p".into(), "Which port?".into()] };
        let exe = Path::new("/bin/agent-eval");
        assert_eq!(Client::Claude.step(&planned, exe, "session-1.jsonl", Some(Path::new("/runs/mcp.json"))), planned);

        let dir = tempfile::tempdir().unwrap();
        let streams = Client::Replay(dir.path().to_owned());
        assert_eq!(
            streams.step(&planned, exe, "session-2.jsonl", None).args,
            ["replay-client", dir.path().join("session-2.jsonl").to_str().unwrap()],
            "a directory holds a stream for each session"
        );
        let replay = Client::Replay(PathBuf::from("/runs/s.jsonl"));
        assert_eq!(replay.step(&planned, exe, "session-1.jsonl", None), Step { program: "/bin/agent-eval".into(), args: vec!["replay-client".into(), "/runs/s.jsonl".into()] });
        assert_eq!(
            replay.step(&planned, exe, "session-1.jsonl", Some(Path::new("/runs/mcp.json"))).args,
            ["replay-client", "/runs/s.jsonl", "--mcp-config", "/runs/mcp.json"],
            "a run with a server tells the replay where it is"
        );
        assert_eq!((Client::Claude.name(), replay.name()), ("claude", "replay"));
        assert_eq!(serde_json::to_value(&Client::Claude).unwrap(), json!("claude"));
        assert_eq!(serde_json::to_value(&replay).unwrap(), json!({ "replay": "/runs/s.jsonl" }));
    }

    #[test]
    fn a_session_that_ends_in_an_error_starts_no_other() {
        let result = |fields: &str| format!(r#"{{"type":"result","subtype":"success",{fields},"result":"x"}}"#);
        assert_eq!(ended(&result(r#""is_error":false"#)), Ok(()));
        assert!(ended(&result(r#""is_error":true"#)).unwrap_err().contains("error"));
        assert!(ended(r#"{"type":"assistant","message":{"content":[]}}"#).unwrap_err().contains("no result"));
        assert!(ended("").unwrap_err().contains("no result"));
    }

    #[test]
    fn the_files_of_a_session_carry_its_number_only_when_there_are_several() {
        assert_eq!(session_files(1, 1), ("stream.jsonl".to_owned(), "client.log".to_owned()));
        assert_eq!(session_files(1, 2), ("stream-1.jsonl".to_owned(), "client-1.log".to_owned()));
        assert_eq!(session_files(3, 3), ("stream-3.jsonl".to_owned(), "client-3.log".to_owned()));
    }

    #[test]
    fn the_shape_of_a_run_is_recorded_in_its_run_file() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("fixture");
        std::fs::create_dir_all(&fixture).unwrap();
        let layout = Layout { root: dir.path().join("run") };
        prepare(&task(), Arm::A0, &fixture, &layout, &options(dir.path().join("clawg")), &Client::Claude).unwrap();
        let record = read_record(&layout.root).unwrap();
        assert_eq!(record["shape"], "split");
        assert_eq!((&record["noise"], &record["noise_rules"]), (&json!("noise"), &json!(0)), "the noise the run was seeded with, by name and by how many rules");
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

        let r = scored(&layout.root);

        assert!(r.score.success);
        assert_eq!((r.arm.as_str(), r.client.as_str()), ("a0", "replay"));
        let written: Value = serde_json::from_str(&std::fs::read_to_string(layout.root.join(RESULT_FILE)).unwrap()).unwrap();
        assert_eq!(written["score"]["success"], true);
        assert!(matches!(score_dir(dir.path()), Err(RunError::Io { .. })));

        let claude = Layout { root: dir.path().join("claude") };
        prepare(&task(), Arm::A0, &fixture, &claude, &options(dir.path().join("clawg")), &Client::Claude).unwrap();
        std::fs::write(claude.root.join(STREAM_FILE), STREAM).unwrap();
        assert_eq!(scored(&claude.root).client, "claude");
    }

    #[test]
    fn the_native_memory_and_the_loaded_rules_files_are_delivered() {
        let layout = Layout { root: if cfg!(windows) { PathBuf::from(r"C:\runs\r") } else { PathBuf::from("/runs/r") } };
        let opts = options(layout.root.join("clawg"));
        let a4 = crate::arm::plan(&task(), Arm::A4, &layout, &opts).unwrap();
        let native = delivered(&a4, &[], None);
        assert_eq!(native.len(), 1);
        assert!(native[0].contains("Staging port 48213"), "{native:?}");
        assert!(delivered(&crate::arm::plan(&task(), Arm::A3, &layout, &opts).unwrap(), &[], None).is_empty());

        let rules = task::parse(
            r#"{"id": "t", "category": "K3", "fixture": "f", "prompt": "Write the notes.", "checks": [{"kind": "answer_contains", "text": "x"}],
                "memories": [{"content": "Lists use an asterisk", "rule": {"paths": ["**/*.md"]}}, {"content": "Go errors are wrapped", "rule": {"paths": ["**/*.go"]}}]}"#,
        )
        .unwrap();
        let l2 = crate::arm::plan(&rules, Arm::L2, &layout, &opts).unwrap();
        let reported = layout.rules_dir().join("rules-1.md").to_string_lossy().to_uppercase().replace('\\', "/");
        let loaded = [
            Loaded { file_path: reported, load_reason: "path_glob_match".into() },
            Loaded { file_path: "/elsewhere/CLAUDE.md".into(), load_reason: "session_start".into() },
        ];
        let texts = delivered(&l2, &loaded, None);
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(texts[0].contains("asterisk") && !texts[0].contains("Go errors"), "{}", texts[0]);
        assert!(delivered(&l2, &[], None).is_empty(), "a rules file the client did not load reached nothing");
    }

    #[test]
    fn score_dir_counts_what_reached_the_context_outside_the_stream() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("fixture");
        std::fs::create_dir_all(&fixture).unwrap();
        let layout = Layout { root: dir.path().join("run") };
        prepare(&task(), Arm::A4, &fixture, &layout, &options(dir.path().join("clawg")), &Client::Claude).unwrap();
        let stream = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done."}],"usage":{"input_tokens":3,"cache_creation_input_tokens":40,"cache_read_input_tokens":500}}}
{"type":"result","subtype":"success","is_error":false,"result":"done"}
"#;
        std::fs::write(layout.root.join(STREAM_FILE), stream).unwrap();
        std::fs::write(layout.instructions_log(), "{\"file_path\":\"/c/CLAUDE.md\",\"load_reason\":\"session_start\"}\n").unwrap();

        let r = scored(&layout.root);

        assert_eq!(r.score.tracers, vec![("48213".to_owned(), TracerFate::RetrievedButIgnored)], "MEMORY.md is in the context");
        assert_eq!(r.session.first_request_tokens, Some(543));
        assert_eq!(r.session.instructions_loaded, [Loaded { file_path: "/c/CLAUDE.md".into(), load_reason: "session_start".into() }]);
        std::fs::remove_file(layout.instructions_log()).unwrap();
        assert!(scored(&layout.root).session.instructions_loaded.is_empty(), "a run without the log loaded none");
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

        let r = scored(&layout.root);

        assert_eq!(r.session.api_key_source.as_deref(), Some("ANTHROPIC_API_KEY"));
        let ended = r.session.ended_at.expect("every platform knows when a file was last written");
        assert!(ended.ends_with(" UTC"), "{ended}");
        if cfg!(windows) {
            assert!(r.session.started_at.is_some(), "Windows knows when a file was created");
        }
    }

    fn three_sessions() -> Task {
        task::parse(
            r#"{"id": "t", "category": "D", "fixture": "f",
                "sessions": [{"prompt": "one"}, {"prompt": "two"}, {"prompt": "three"}],
                "checks": [{"kind": "answer_contains", "text": "x"}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn a_joined_session_has_the_earlier_prompts_in_its_context() {
        let t = three_sessions();
        assert!(earlier_prompts(&t, Shape::Joined, 1).is_empty());
        assert_eq!(earlier_prompts(&t, Shape::Joined, 2), ["one"]);
        assert_eq!(earlier_prompts(&t, Shape::Joined, 3), ["one", "two"]);
        assert!(earlier_prompts(&t, Shape::Split, 3).is_empty(), "a new session never saw them");
        assert!(earlier_prompts(&task(), Shape::Split, 1).is_empty());
    }

    #[test]
    fn the_native_memory_delivered_is_what_the_file_held_before_the_session() {
        let layout = Layout { root: if cfg!(windows) { PathBuf::from(r"C:\runs\r") } else { PathBuf::from("/runs/r") } };
        let a4 = crate::arm::plan(&task(), Arm::A4, &layout, &options(layout.root.join("clawg"))).unwrap();

        let planned = delivered(&a4, &[], None);
        assert!(planned.len() == 1 && planned[0].contains("Staging port 48213"), "without a copy, the file of the plan: {planned:?}");

        let changed = delivered(&a4, &[], Some("# Memory\n\nThe agent wrote quokka here\n"));
        assert_eq!(changed, ["# Memory\n\nThe agent wrote quokka here\n"], "the copy replaces the file of the plan");

        let long: String = (1..=300).map(|i| format!("line {i}\n")).collect();
        let cut = delivered(&a4, &[], Some(&long));
        assert!(cut[0].contains("line 200\n") && !cut[0].contains("line 201\n"), "the client loads the first 200 lines");
        assert!(delivered(&a4, &[], Some("")).first().is_some_and(String::is_empty), "an empty file is loaded as nothing");
    }

    fn walked(followed: &[(&str, TracerFate)], saved: &[&str]) -> SessionResult {
        SessionResult {
            session: evaluate(&task(), &info("a0", Profile::Dev), &[played("")], |_| None).session,
            score: Score { success: true, ended: true, checks: Vec::new(), tracers: Vec::new() },
            followed: followed.iter().map(|(tracer, fate)| score::Journey { tracer: (*tracer).to_owned(), fate: *fate, brought_by: Vec::new() }).collect(),
            saved: saved.iter().map(|s| (*s).to_owned()).collect(),
            native_memory_writes: 0,
            saves: Vec::new(),
        }
    }

    #[test]
    fn a_fact_is_told_as_not_saved_not_reached_not_used_or_used() {
        let t = task::parse(
            r#"{"id": "t", "category": "D", "fixture": "f",
                "sessions": [
                    {"prompt": "Hide with alpha-1, bravo-2, charlie-3 and delta-4.", "tracers": ["alpha-1", "bravo-2", "charlie-3", "delta-4"]},
                    {"prompt": "Now zulu-9.", "tracers": ["zulu-9"]},
                    {"prompt": "Apply."}],
                "checks": [{"kind": "answer_contains", "text": "x"}]}"#,
        )
        .unwrap();
        let last = [
            ("alpha-1", TracerFate::Used),
            ("bravo-2", TracerFate::RetrievedButIgnored),
            ("charlie-3", TracerFate::NeverReached),
            ("delta-4", TracerFate::NeverReached),
            ("zulu-9", TracerFate::Used),
        ];
        let sessions = [walked(&[], &["alpha-1", "bravo-2"]), walked(&[], &["charlie-3", "zulu-9"]), walked(&last, &[])];
        let told = facts(&t, &sessions);
        let by = |tracer: &str| told.iter().find(|f| f.tracer == tracer).map(|f| (f.born, f.class));
        assert_eq!(by("alpha-1"), Some((1, FactClass::Used)));
        assert_eq!(by("bravo-2"), Some((1, FactClass::ReachedNotUsed)));
        assert_eq!(by("charlie-3"), Some((1, FactClass::SavedNotReached)), "a later session saved it");
        assert_eq!(by("delta-4"), Some((1, FactClass::NotSaved)), "no session wrote it, whatever its fate");
        assert_eq!(by("zulu-9"), Some((2, FactClass::Used)));
        assert_eq!(told.len(), 5);
        assert_eq!(serde_json::to_value(FactClass::SavedNotReached).unwrap(), json!("saved_not_reached"));

        assert!(facts(&t, &sessions[..1]).is_empty(), "a fact born in the last session has no later one to reach");
        assert!(facts(&task(), &sessions[..1]).is_empty());
    }

    #[test]
    fn a_fact_saved_only_by_the_last_session_is_not_saved() {
        let t = task::parse(
            r#"{"id": "t", "category": "R", "fixture": "f",
                "sessions": [{"prompt": "Decide on alpha-1.", "tracers": ["alpha-1"]}, {"prompt": "Apply it."}],
                "checks": [{"kind": "answer_contains", "text": "x"}]}"#,
        )
        .unwrap();
        let last = [("alpha-1", TracerFate::Used)];
        let class = |saved_first: &[&str], saved_last: &[&str]| facts(&t, &[walked(&[], saved_first), walked(&last, saved_last)])[0].class;
        assert_eq!(class(&[], &["alpha-1"]), FactClass::NotSaved, "what the last session saves at the end brought nothing to it");
        assert_eq!(class(&["alpha-1"], &["alpha-1"]), FactClass::Used);
        assert_eq!(class(&["alpha-1"], &[]), FactClass::Used);
    }

    /// A session whose first request has `tokens` of context, followed by each
    /// call as a request of its own with a result of the given size.
    fn session_of(tokens: u64, calls: &[(&str, Value, usize)]) -> Transcript {
        let usage = format!(r#""usage":{{"input_tokens":3,"cache_creation_input_tokens":{},"cache_read_input_tokens":{}}}"#, tokens - 3 - 100, 100);
        let mut stream = format!("{{\"type\":\"assistant\",\"message\":{{\"id\":\"start\",\"content\":[],{usage}}}}}\n");
        for (i, (name, input, result)) in calls.iter().enumerate() {
            stream.push_str(&format!(
                "{}\n{}\n",
                json!({"type": "assistant", "message": {"id": format!("m{i}"), "content": [{"type": "tool_use", "id": format!("c{i}"), "name": name, "input": input}]}}),
                json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": format!("c{i}"), "content": "z".repeat(*result)}]}})
            ));
        }
        stream.push_str("{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\"}\n");
        stream::parse(&stream)
    }

    fn two_sessions() -> Task {
        task::parse(
            r#"{"id": "t", "category": "R", "fixture": "f",
                "sessions": [{"prompt": "one"}, {"prompt": "two"}],
                "checks": [{"kind": "answer_contains", "text": "ok"}]}"#,
        )
        .unwrap()
    }

    fn played_of(first: Transcript, second: Transcript) -> [Played; 2] {
        [first, second].map(|transcript| Played { transcript, delivered: Vec::new(), loaded: Vec::new(), untouched: Vec::new() })
    }

    #[test]
    fn the_relay_overhead_is_what_the_new_session_starts_with_and_reads_again() {
        let memory = if cfg!(windows) { r"C:\runs\r\claude-config\projects\p\memory" } else { "/runs/r/claude-config/projects/p/memory" };
        let memory_file = Path::new(memory).join("MEMORY.md").to_string_lossy().into_owned();
        let work = Path::new(WORKDIR);
        let first = session_of(1000, &[("Read", json!({"file_path": "src/a.py"}), 1)]);
        let second = |tokens: u64| {
            session_of(
                tokens,
                &[
                    ("Read", json!({"file_path": "src/a.py"}), 400),
                    ("Read", json!({"file_path": "src/b.py"}), 4000),
                    ("mcp__clawg-dev__search_memory", json!({"query": "timeout"}), 800),
                    ("Read", json!({"file_path": memory_file}), 200),
                ],
            )
        };
        let later = second(1500);
        assert_eq!((first.first_request_tokens, later.first_request_tokens), (Some(1000), Some(1500)));
        let dir = Path::new(memory);
        let again = |reread: &Rereads| reread.again;
        let reads = rereads(&first, &later, work, Some(dir));
        assert_eq!(reads, Rereads { again: (400 + 800 + 200) / 4, to_edit: 0 }, "src/b.py is new to the second session");
        assert_eq!(again(&rereads(&first, &later, work, None)), (400 + 800) / 4, "without a native memory its directory reads nothing");
        assert_eq!(overhead(&first, &later, reads.again), 500 + 350, "what it starts with and what it reads again");
        assert_eq!(overhead(&first, &second(900), reads.again), 350, "it starts with less: nothing is added, and below 0 it stays at 0");
        assert_eq!(overhead(&stream::parse(""), &later, reads.again), 350, "no first request to compare with");
    }

    const WORKDIR: &str = "/runs/r/work";

    #[test]
    fn the_relay_overhead_leaves_out_the_reads_an_edit_needs() {
        let first = session_of(
            1000,
            &[
                ("Read", json!({"file_path": "a.py"}), 1),
                ("Read", json!({"file_path": "b.py"}), 1),
                ("Write", json!({"file_path": "NOTES.md", "content": "x"}), 1),
            ],
        );
        let second = session_of(
            1500,
            &[
                ("Read", json!({"file_path": "/runs/r/work/./SRC/../a.py"}), 400),
                ("Edit", json!({"file_path": "a.py"}), 1),
                ("Read", json!({"file_path": "b.py"}), 4000),
                ("Read", json!({"file_path": "NOTES.md"}), 800),
                ("Edit", json!({"file_path": "NOTES.md"}), 1),
                ("Read", json!({"file_path": "c.py"}), 2000),
            ],
        );
        let reads = rereads(&first, &second, Path::new(WORKDIR), None);
        assert_eq!(reads, Rereads { again: (4000 + 800) / 4, to_edit: 400 / 4 }, "a.py is read to be edited; c.py is new");

        let info = RunInfo { shape: Shape::Split, ..info("a3", Profile::Dev) };
        let r = evaluate(&two_sessions(), &info, &played_of(first, second), |_| None);
        assert_eq!(r.rereads, Some(reads));
        assert_eq!(r.overhead, Some(500 + reads.again), "the growth of the first request, plus what is read again");
    }

    #[test]
    fn a_file_the_first_session_wrote_always_counts_as_read_again() {
        let first = session_of(
            1000,
            &[("Write", json!({"file_path": "TIMEOUTS.md", "content": "x"}), 1), ("NotebookEdit", json!({"notebook_path": "n.ipynb"}), 1)],
        );
        let second = session_of(
            1000,
            &[("Read", json!({"file_path": "TIMEOUTS.md"}), 2000), ("Edit", json!({"file_path": "TIMEOUTS.md"}), 1), ("Read", json!({"file_path": "n.ipynb"}), 400)],
        );
        assert_eq!(rereads(&first, &second, Path::new(WORKDIR), None), Rereads { again: 600, to_edit: 0 }, "the note the first session left is what a memory replaces");
    }

    #[test]
    fn a_continued_session_has_rereads_and_no_relay_overhead() {
        let first = session_of(1000, &[("Read", json!({"file_path": "a.py"}), 1)]);
        let second = session_of(1800, &[("Read", json!({"file_path": "a.py"}), 400)]);
        let info = RunInfo { shape: Shape::Joined, ..info("a3", Profile::Dev) };
        let r = evaluate(&two_sessions(), &info, &played_of(first, second), |_| None);
        assert_eq!(r.rereads, Some(Rereads { again: 100, to_edit: 0 }));
        assert_eq!(r.overhead, None, "its first request is the whole conversation");
        assert_eq!(evaluate(&two_sessions(), &info, &played_of(session_of(1000, &[]), session_of(1000, &[]))[..1], |_| None).rereads, None, "a run of one session reads nothing again");
    }

    const OLD_RESULT: &str = r#"{"task": "t", "category": "K1", "example": false, "arm": "a3", "client": "replay", "lang": null,
        "session": {"session_id": "s", "model": null, "client_version": null, "mcp_servers": [], "tool_calls": 1,
                    "clawg_calls": {"search_memory": 1}, "clawg_calls_after_tool_search": 0, "clawg_calls_in_subagents": 0,
                    "tool_search_calls": 0, "hook_events": 0, "compactions": 0, "unknown_lines": 0, "invalid_lines": [], "outcome": null},
        "score": {"success": true, "ended": true, "checks": [{"check": "c", "passed": true}], "tracers": [["48213", "used"]]},
        "saves": []}"#;

    #[test]
    fn a_result_written_before_sessions_still_reads() {
        let r: RunResult = serde_json::from_str(OLD_RESULT).unwrap();
        assert!(r.sessions.is_empty() && r.facts.is_empty());
        assert_eq!((r.shape, r.noise_rules, r.overhead, r.paired.as_deref()), (Shape::Split, 0, None, None));
        assert_eq!(r.records().len(), 1, "a run of one session");
        assert_eq!(
            summary(&r),
            "t a3: success, checks 1/1, tracers used 1 retrieved_but_ignored 0 never_reached 0, clawg calls 1 (0 after ToolSearch), saves 0 with 0 failed criteria"
        );
    }

    #[test]
    fn score_dir_scores_every_session_of_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("fixture");
        std::fs::create_dir_all(&fixture).unwrap();
        let layout = Layout { root: dir.path().join("run") };
        let relay = task::parse(
            r#"{"id": "relay", "category": "R", "fixture": "f",
                "memories": [{"content": "Proxy alias quokka"}], "tracers": ["quokka"],
                "sessions": [{"prompt": "Decide on 4.5-41 and write it down.", "tracers": ["4.5-41"], "checks": [{"kind": "tool_called", "name": "Write"}]},
                             {"prompt": "Apply it."}],
                "checks": [{"kind": "answer_contains", "text": "4.5-41"}]}"#,
        )
        .unwrap();
        let mut opts = options(dir.path().join("clawg"));
        opts.shape = Shape::Joined;
        opts.noise = (0..2).map(|i| serde_json::from_str(&format!(r#"{{"content": "Rule {i}", "rule": {{}}}}"#)).unwrap()).collect();
        let plan = prepare(&relay, Arm::A4, &fixture, &layout, &opts, &Client::Replay(dir.path().join("streams"))).unwrap();
        let memory = native_memory_file(&plan).unwrap().parent().unwrap().to_owned();

        let line = |value: Value| value.to_string() + "\n";
        let first = [
            r#"{"type":"system","subtype":"init","session_id":"s1","mcp_servers":[]}"#.to_owned() + "\n",
            line(json!({"type": "assistant", "message": {"content": [
                {"type": "tool_use", "id": "w1", "name": "Write", "input": {"file_path": memory.join("notes.md"), "content": "timeout 4.5-41"}},
                {"type": "tool_use", "id": "m1", "name": "mcp__clawg-dev__save_memory", "input": {"content": "Timeout decision\nUse 4.5-41 as http_timeout."}}]}})),
            line(json!({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "w1", "content": "written"}, {"type": "tool_result", "tool_use_id": "m1", "content": "saved"}]}})),
            r#"{"type":"result","subtype":"success","is_error":false,"result":"Noted."}"#.to_owned() + "\n",
        ]
        .concat();
        let second = "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s2\",\"mcp_servers\":[]}\n{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"Applied 4.5-41\"}\n";
        let put = |name: &str, text: &str| std::fs::write(layout.root.join(name), text).unwrap();
        put("stream-1.jsonl", &first);
        put("stream-2.jsonl", second);
        put("memory-before-1.md", "# Memory\n\nProxy alias quokka\n");
        put("memory-before-2.md", "# Memory\n\nProxy alias quokka\n\ntimeout 4.5-41\n");
        put("instructions-1.jsonl", "{\"file_path\":\"/c/CLAUDE.md\",\"load_reason\":\"session_start\"}\n");

        let r = scored(&layout.root);

        assert_eq!(r.sessions.len(), 2);
        assert_eq!((r.sessions[0].session.session_id.as_deref(), r.sessions[1].session.session_id.as_deref()), (Some("s1"), Some("s2")));
        assert_eq!(r.session, r.sessions[1].session, "above, the record of the last session");
        assert_eq!((r.shape, r.noise_rules, r.paired.as_deref()), (Shape::Joined, 2, None));
        assert_eq!((r.sessions[0].native_memory_writes, r.sessions[1].native_memory_writes), (1, 0));
        assert_eq!((r.sessions[0].saved.as_slice(), r.sessions[1].saved.as_slice()), (["4.5-41".to_owned()].as_slice(), [].as_slice()));
        assert_eq!(r.sessions[0].session.instructions_loaded.len(), 1);
        assert!(r.sessions[1].session.instructions_loaded.is_empty(), "each session has its own log");
        let brought = |s: usize, tracer: &str| r.sessions[s].followed.iter().find(|j| j.tracer == tracer).map(|j| (j.fate, j.brought_by.clone()));
        assert_eq!(brought(0, "quokka"), Some((TracerFate::RetrievedButIgnored, vec!["context".to_owned()])), "the file before session 1");
        assert_eq!(brought(1, "4.5-41"), Some((TracerFate::Used, vec!["context".to_owned()])), "the prompt of the session before, and the file the agent wrote");
        assert_eq!(brought(0, "4.5-41"), None, "a session does not follow its own fact");
        assert_eq!(r.facts, [Fact { tracer: "4.5-41".into(), born: 1, class: FactClass::Used }]);
        assert!(r.score.success && r.score.ended, "{:?}", r.score);
        assert_eq!(r.score.checks.len(), 2);
        assert_eq!((r.overhead, r.rereads), (None, Some(Rereads::default())), "the second session continues the conversation");
        assert!(r.sessions.iter().all(|s| s.session.ended_at.is_some()));
        let text = summary(&r);
        assert!(text.contains("saves 1 with 0 failed criteria") && text.contains("2 sessions joined"), "{text}");
        assert!(text.contains("facts not_saved 0 saved_not_reached 0 reached_not_used 0 used 1"), "{text}");
        let written: RunResult = serde_json::from_str(&std::fs::read_to_string(layout.root.join(RESULT_FILE)).unwrap()).unwrap();
        assert_eq!(written, r);

        std::fs::remove_file(layout.root.join("stream-2.jsonl")).unwrap();
        let cut = scored(&layout.root);
        assert_eq!(cut.sessions.len(), 1, "the sessions that ran are scored");
        assert!(!cut.score.ended && !cut.score.success, "a run that did not play every session did not end");
        assert!(cut.facts.is_empty() && cut.overhead.is_none());
        assert!(summary(&cut).contains("saves 1"), "{}", summary(&cut));

        std::fs::remove_file(layout.root.join("stream-1.jsonl")).unwrap();
        assert!(matches!(score_dir(&layout.root), Err(RunError::Io { .. })));
    }

    #[test]
    fn a_fork_plays_the_second_session_continued_and_then_new() {
        use ForkStep::{Keep, Play, Restore, Save, Start, Stop};
        let (joined, split) = (Some(Shape::Joined), Some(Shape::Split));
        assert_eq!(
            fork_steps(true),
            [
                Start { branch: None, seed: true },
                Play { k: 1, branch: None },
                Stop,
                Save,
                Start { branch: joined, seed: false },
                Play { k: 2, branch: joined },
                Stop,
                Keep(Shape::Joined),
                Restore,
                Start { branch: split, seed: false },
                Play { k: 2, branch: split },
                Stop,
                Keep(Shape::Split),
            ],
            "the daemon goes down while the state is copied, and the seed loads once"
        );
        assert_eq!(
            fork_steps(false),
            [Play { k: 1, branch: None }, Save, Play { k: 2, branch: joined }, Keep(Shape::Joined), Restore, Play { k: 2, branch: split }, Keep(Shape::Split)],
            "an arm without clawg has no daemon to start or stop"
        );
    }

    #[test]
    fn the_new_branch_plays_the_command_of_the_second_session_without_continuing() {
        let continued = Step { program: "claude".into(), args: ["-p", "two", "--model", "m", "--continue"].map(String::from).to_vec() };
        assert_eq!(branch_step(&continued, Shape::Joined), continued);
        assert_eq!(branch_step(&continued, Shape::Split).args, ["-p", "two", "--model", "m"]);
        let alone = Step { program: "claude".into(), args: vec!["-p".into(), "one".into()] };
        assert_eq!(branch_step(&alone, Shape::Split), alone);
    }

    #[test]
    fn a_restore_puts_back_the_work_the_config_and_the_data_but_leaves_the_models() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout { root: dir.path().join("run") };
        let data = layout.home().join(".clawg-dev");
        let put = |path: PathBuf, text: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        let read = |path: PathBuf| std::fs::read_to_string(path).ok();
        put(layout.workdir().join("src").join("a.txt"), "first");
        put(layout.config_dir().join("settings.json"), "config");
        put(data.join("memory.db"), "store");
        put(data.join("models").join("m.bin"), "model");

        save_state(&layout, Some(&data)).unwrap();
        let saved = layout.root.join(FORK_DIR);
        assert_eq!(read(saved.join("work").join("src").join("a.txt")).as_deref(), Some("first"));
        assert_eq!(read(saved.join("data").join("memory.db")).as_deref(), Some("store"));
        assert!(!saved.join("data").join("models").exists(), "the models are not copied: no session writes them");

        put(layout.workdir().join("src").join("a.txt"), "changed");
        put(layout.workdir().join("new.txt"), "new");
        put(layout.config_dir().join("projects").join("p.jsonl"), "talk");
        put(data.join("memory.db"), "grown");
        put(data.join("extra.db"), "extra");
        put(data.join("models").join("m.bin"), "other model");

        restore_state(&layout, Some(&data)).unwrap();

        assert_eq!(read(layout.workdir().join("src").join("a.txt")).as_deref(), Some("first"));
        assert!(!layout.workdir().join("new.txt").exists());
        assert!(!layout.config_dir().join("projects").exists());
        assert_eq!(read(layout.config_dir().join("settings.json")).as_deref(), Some("config"));
        assert_eq!(read(data.join("memory.db")).as_deref(), Some("store"));
        assert!(!data.join("extra.db").exists());
        assert_eq!(read(data.join("models").join("m.bin")).as_deref(), Some("other model"), "the models stay where they are");

        std::fs::remove_dir_all(layout.workdir()).unwrap();
        restore_state(&layout, Some(&data)).unwrap();
        assert_eq!(read(layout.workdir().join("src").join("a.txt")).as_deref(), Some("first"), "a working directory moved away comes back");
    }
}
