//! A performance bench of the daemon that repeats the same way on the next
//! version: the store, the load and the schedule come from a seed, so two
//! runs on one machine differ only by the code.
//!
//! The bench itself is ignored, so CI never runs it; the tests of its pure
//! parts run with the suite. Run it with
//! `cargo test -p clawg-cli --test perf_bench -- --ignored --nocapture`
//! (`--release` measures an optimized daemon), and compare two of its files
//! with `cargo run -p clawg-eval --bin perf-bench -- diff A B`.
//!
//! The daemon is the `testing` build of the binary under test, started with
//! `--port 0` in a temporary home. With the fake model it measures SQLite,
//! ranking and ingest; `CLAWG_BENCH_MODEL_DIR`, a data directory whose
//! `models/` holds the real model, gives it that model instead, copied into
//! the temporary home and checked by the daemon as on any start.
//!
//! The store is seeded through `POST /v1/import`, the path that writes
//! thousands of rows the way production does. Each request leaves at the
//! instant its schedule says, whatever the ones before it are doing, and its
//! latency counts from that instant, so a daemon that stalls is charged for
//! every request that waited behind the stall.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clawg_cli::client::{Call, DaemonClient};
use clawg_core::memory::portability::format::{render, Filter, Header, Line, MemoryLine, ProjectLine, FORMAT, VERSION};
use serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_clawg");

/// What a result file names itself, and the only version `perf-bench diff`
/// reads.
const RESULT_FORMAT: &str = "clawg-perf-bench";
const RESULT_VERSION: u32 = 1;

const DEFAULT_ROWS: usize = 10_000;
const DEFAULT_SEED: u64 = 74;
const MAX_ROWS: usize = 1_000_000;

const AGENTS: [&str; 4] = ["bench-a", "bench-b", "bench-c", "bench-d"];
const PROJECTS: [&str; 3] = ["bench-alpha", "bench-beta", "bench-gamma"];

const TYPES: [&str; 7] = [
    "semantic_fact",
    "architectural_decision",
    "user_preference",
    "session_summary",
    "code_block",
    "tool_error",
    "chat_message",
];

/// Words every text draws from, the first ones far more often than the last.
const FREQUENT: [&str; 64] = [
    "the", "store", "daemon", "save", "search", "memory", "project", "agent", "index", "query", "worker", "vector",
    "table", "row", "piece", "token", "model", "cache", "lock", "socket", "config", "build", "test", "release",
    "schema", "migration", "backup", "snapshot", "outbox", "ranking", "score", "context", "prompt", "budget",
    "session", "hook", "bridge", "profile", "metric", "latency", "thread", "queue", "batch", "embed", "split",
    "chunk", "docs", "refresh", "passage", "heading", "breadcrumb", "path", "file", "error", "retry", "deadline",
    "commit", "branch", "version", "update", "import", "export", "secret", "mask",
];

/// The pieces a save splits into: the first 96 tokens, then 72 more each.
fn words_for_pieces(pieces: usize) -> usize {
    match pieces {
        0 | 1 => 40,
        n => 96 + 72 * (n - 1),
    }
}

/// SplitMix64: a fixed sequence for a seed, whatever version of `rand`.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }

    /// True `percent` times in 100.
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    /// An index skewed toward 0: the square of a uniform draw.
    fn skewed(&mut self, n: usize) -> usize {
        let u = self.below(1 << 20) as f64 / f64::from(1u32 << 20);
        ((u * u) * n as f64) as usize
    }

    fn frequent_word(&mut self) -> &'static str {
        FREQUENT[self.skewed(FREQUENT.len())]
    }

    /// An identifier few rows share: one of `rows / 20` names.
    fn rare_word(&mut self, rows: usize) -> String {
        format!("ident{}", self.below((rows / 20).max(1) as u64))
    }

    fn text(&mut self, words: usize, rows: usize) -> String {
        let mut out = Vec::with_capacity(words);
        for _ in 0..words {
            if self.chance(4) {
                out.push(self.rare_word(rows));
            } else {
                out.push(self.frequent_word().to_owned());
            }
        }
        out.join(" ")
    }
}

/// The seeded store as a `clawg-memories` export: four agents, three projects
/// and rows without one, a tenth superseded and a twentieth deleted.
fn seed_export(seed: u64, rows: usize, now_ms: i64) -> String {
    let mut rng = Rng::new(seed);
    let mut out = render(&Line::Header(Header {
        format: FORMAT.into(),
        version: VERSION,
        exported_at: now_ms,
        clawg: env!("CARGO_PKG_VERSION").into(),
        embedding_model: None,
        filter: Filter::default(),
    }));
    for name in PROJECTS {
        out.push_str(&render(&Line::Project(ProjectLine { id: name.into(), name: Some(name.into()), ..Default::default() })));
    }
    const DAY_MS: i64 = 86_400_000;
    for id in 1..=rows as i64 {
        let agent = AGENTS[rng.below(AGENTS.len() as u64) as usize];
        let project = (!rng.chance(25)).then(|| PROJECTS[rng.below(PROJECTS.len() as u64) as usize].to_owned());
        let words = 12 + rng.below(29) as usize;
        let created_at = now_ms - rng.below(365) as i64 * DAY_MS - rng.below(DAY_MS as u64) as i64;
        let superseded = id < rows as i64 && rng.chance(10);
        let deleted = !superseded && rng.chance(5);
        out.push_str(&render(&Line::Memory(MemoryLine {
            id,
            scope: "agent".into(),
            agent_id: Some(agent.into()),
            project_id: project,
            content: format!("Row {id}: {}", rng.text(words, rows)),
            memory_type: TYPES[rng.below(TYPES.len() as u64) as usize].into(),
            source: "explicit".into(),
            created_at,
            created_by_type: "agent".into(),
            created_by_ref: String::new(),
            source_session_id: None,
            last_modified_at: created_at,
            last_modified_by: None,
            update_count: 0,
            last_accessed_at: created_at,
            access_count: rng.below(8) as i64,
            importance: (1 + rng.below(9)) as f64,
            tier: 1 + rng.below(3) as i64,
            status: if superseded { "superseded" } else { "active" }.into(),
            superseded_by: superseded.then_some(id + 1),
            supersede_cause: superseded.then(|| "revision".into()),
            dispute_count: 0,
            disputed_against: Vec::new(),
            last_disputed_at: None,
            deleted_at: deleted.then_some(created_at + 1),
            topic_key: None,
            revived_from: None,
            lang: None,
            origin_project: None,
        })));
    }
    out
}

/// The queries of a run: two to four frequent words, a third of them with a
/// rare identifier.
fn queries(seed: u64, rows: usize, n: usize) -> Vec<String> {
    let mut rng = Rng::new(seed ^ 0x5155_4552_5953);
    (0..n)
        .map(|_| {
            let words = 2 + rng.below(3) as usize;
            let mut q: Vec<String> = (0..words).map(|_| rng.frequent_word().to_owned()).collect();
            if rng.chance(33) {
                q.push(rng.rare_word(rows));
            }
            q.join(" ")
        })
        .collect()
}

/// The text of the `n`th save of a scenario: unique, so no dedup absorbs it.
fn save_text(seed: u64, scenario: &str, n: usize, pieces: usize, rows: usize) -> String {
    let mut rng = Rng::new(seed ^ fnv(scenario) ^ (n as u64).wrapping_mul(0x9E37_79B9));
    format!("Bench save {scenario} {n}: {}", rng.text(words_for_pieces(pieces), rows))
}

/// Markdown files for the docs refresh: `files` files of eight sections.
fn docs_corpus(seed: u64, files: usize) -> Vec<(String, String)> {
    let mut rng = Rng::new(seed ^ 0x444f_4353);
    (0..files)
        .map(|f| {
            let mut text = format!("# Document {f}\n\n");
            for s in 0..8 {
                let words = 60 + rng.below(80) as usize;
                text.push_str(&format!("## Section {s} of {f}\n\n{}\n\n", rng.text(words, 2_000)));
            }
            (format!("doc-{f:04}.md"), text)
        })
        .collect()
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// When each request of a scenario leaves, from its start.
fn schedule(rate_per_s: f64, count: usize) -> Vec<Duration> {
    (0..count).map(|i| Duration::from_secs_f64(i as f64 / rate_per_s)).collect()
}

/// The `q` quantile of sorted samples, by nearest rank, as `perf-bench diff`
/// reads it.
fn quantile(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn summary(samples: &[u64]) -> Value {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let mean = if sorted.is_empty() { 0.0 } else { sorted.iter().sum::<u64>() as f64 / sorted.len() as f64 };
    json!({
        "count": sorted.len(),
        "mean_us": mean.round(),
        "p50_us": quantile(&sorted, 0.50),
        "p95_us": quantile(&sorted, 0.95),
        "p99_us": quantile(&sorted, 0.99),
        "max_us": sorted.last().copied().unwrap_or(0),
    })
}

/// What a run is asked for, from the environment.
#[derive(Debug, Clone, PartialEq)]
struct Config {
    rows: usize,
    seed: u64,
    tag: Option<String>,
    model_dir: Option<PathBuf>,
}

impl Config {
    fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let rows = match var("CLAWG_BENCH_ROWS") {
            None => DEFAULT_ROWS,
            Some(raw) => match raw.trim().parse::<usize>() {
                Ok(n) if (1..=MAX_ROWS).contains(&n) => n,
                _ => return Err(format!("CLAWG_BENCH_ROWS must be a count of rows from 1 to {MAX_ROWS}, not {raw:?}")),
            },
        };
        let seed = match var("CLAWG_BENCH_SEED") {
            None => DEFAULT_SEED,
            Some(raw) => raw.trim().parse().map_err(|_| format!("CLAWG_BENCH_SEED must be a whole number, not {raw:?}"))?,
        };
        let tag = match var("CLAWG_BENCH_TAG") {
            None => None,
            Some(raw) if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') => {
                Some(raw)
            }
            Some(raw) => return Err(format!("CLAWG_BENCH_TAG must be lowercase letters, digits and dashes, not {raw:?}")),
        };
        let model_dir = var("CLAWG_BENCH_MODEL_DIR").filter(|d| !d.is_empty()).map(PathBuf::from);
        Ok(Self { rows, seed, tag, model_dir })
    }

    fn embedder(&self) -> &'static str {
        if self.model_dir.is_some() {
            "real"
        } else {
            "fake"
        }
    }

    /// `{date}-{embedder}-perf-bench-{rows}[-{tag}].json`
    fn file_name(&self, date: &str) -> String {
        let tag = self.tag.as_deref().map(|t| format!("-{t}")).unwrap_or_default();
        format!("{date}-{}-perf-bench-{}{tag}.json", self.embedder(), self.rows)
    }
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn utc_date(ms: i64) -> String {
    let (y, m, d) = civil_from_days(ms.div_euclid(86_400_000));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Writes a new file and never replaces one: a run is a record.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)
}

/// The user and system CPU time of `/proc/<pid>/stat`, in clock ticks: the
/// fields after the command, which may itself hold spaces and parentheses.
fn cpu_ticks_of_stat(text: &str) -> Option<u64> {
    let rest = &text[text.rfind(')')? + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // `utime` and `stime` are fields 14 and 15; `rest` starts at field 3.
    let user: u64 = fields.get(11)?.parse().ok()?;
    let system: u64 = fields.get(12)?.parse().ok()?;
    Some(user + system)
}

/// What the daemon's process has used so far, by its pid.
#[derive(Debug, Clone, Copy, Default)]
struct Usage {
    cpu: Option<Duration>,
    rss_peak_bytes: Option<u64>,
    private_peak_bytes: Option<u64>,
}

#[cfg(target_os = "linux")]
fn usage_of(pid: u32) -> Usage {
    // SAFETY: `sysconf` reads a constant of the system.
    let ticks_per_s = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let cpu = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|t| cpu_ticks_of_stat(&t))
        .filter(|_| ticks_per_s > 0)
        .map(|ticks| Duration::from_secs_f64(ticks as f64 / ticks_per_s as f64));
    let status = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .map(|t| clawg_cli::daemon::process::parse_proc_status(&t))
        .unwrap_or_default();
    Usage { cpu, rss_peak_bytes: status.rss_peak_bytes, private_peak_bytes: None }
}

#[cfg(windows)]
fn usage_of(pid: u32) -> Usage {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows_sys::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    let mut usage = Usage::default();
    // SAFETY: the handle is checked before use and closed once; each call
    // writes only into the locals it is given, whose size it is told.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return usage;
        }
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // A FILETIME counts 100 ns intervals.
        let of = |t: FILETIME| Duration::from_nanos(((u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)) * 100);
        if GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) != 0 {
            usage.cpu = Some(of(kernel) + of(user));
        }
        let mut counters: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        if GetProcessMemoryInfo(process, &mut counters as *mut _ as *mut PROCESS_MEMORY_COUNTERS, counters.cb) != 0 {
            usage.rss_peak_bytes = Some(counters.PeakWorkingSetSize as u64);
            usage.private_peak_bytes = Some(counters.PeakPagefileUsage as u64);
        }
        CloseHandle(process);
    }
    usage
}

#[cfg(not(any(target_os = "linux", windows)))]
fn usage_of(_pid: u32) -> Usage {
    Usage::default()
}

/// CPU time over wall time between two readings, not divided by cores.
fn cpu_ratio(before: Usage, after: Usage, wall: Duration) -> Option<f64> {
    let used = after.cpu?.checked_sub(before.cpu?)?;
    (wall > Duration::ZERO).then(|| used.as_secs_f64() / wall.as_secs_f64())
}

const START_BUDGET: Duration = Duration::from_secs(180);

/// How long the daemon sits without a request before the load, for the CPU
/// it spends doing nothing.
const IDLE: Duration = Duration::from_secs(5);

/// The longest a background operation is waited for.
const BACKGROUND_BUDGET: Duration = Duration::from_secs(600);

const DOCS_FILES: usize = 400;

/// A scenario of requests at a fixed rate, alone.
struct Steady {
    name: &'static str,
    rate_per_s: f64,
    count: usize,
    kind: Kind,
}

#[derive(Clone, Copy)]
enum Kind {
    Search,
    Context,
    /// The context without a query that the session-start hook asks for.
    SessionStart,
    Save(usize),
}

/// Rates a debug build serves with 10 000 rows without a queue that grows:
/// the bench measures latency, not how long a backlog takes to drain.
const STEADY: [Steady; 6] = [
    Steady { name: "search", rate_per_s: 5.0, count: 100, kind: Kind::Search },
    Steady { name: "context", rate_per_s: 3.0, count: 60, kind: Kind::Context },
    Steady { name: "session_start", rate_per_s: 3.0, count: 60, kind: Kind::SessionStart },
    Steady { name: "save_1", rate_per_s: 5.0, count: 60, kind: Kind::Save(1) },
    Steady { name: "save_10", rate_per_s: 0.5, count: 20, kind: Kind::Save(10) },
    Steady { name: "save_100", rate_per_s: 1.0 / 15.0, count: 4, kind: Kind::Save(100) },
];

/// What runs while searches and contexts are measured.
#[derive(Clone, Copy, PartialEq)]
enum Background {
    Save100,
    Backup,
    DocsRefresh,
}

impl Background {
    const ALL: [Background; 3] = [Self::Save100, Self::Backup, Self::DocsRefresh];

    fn name(self) -> &'static str {
        match self {
            Self::Save100 => "save_100",
            Self::Backup => "backup",
            Self::DocsRefresh => "docs_refresh",
        }
    }
}

/// What is measured while a background operation runs, each at its rate.
const DURING: [(&str, Kind, f64); 4] = [
    ("search", Kind::Search, 5.0),
    ("context", Kind::Context, 2.0),
    ("session_start", Kind::SessionStart, 1.0),
    ("save_1", Kind::Save(1), 1.0),
];

/// The shortest window a repeatable background operation is kept running
/// for, so a fast one still leaves samples.
const MIN_BACKGROUND: Duration = Duration::from_secs(10);

/// A daemon child, killed through its own handle when the guard drops.
struct Daemon {
    child: Child,
    client: DaemonClient,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn health_call() -> Call {
    Call::new(reqwest::Method::GET, "/health").without_bearer()
}

async fn health_of(client: &DaemonClient) -> Option<Value> {
    let answer = client.send(health_call()).await.ok()?;
    serde_json::from_slice(&answer.body).ok()
}

/// Copies the model cache of a data directory into the bench's own, which the
/// daemon checks against its pinned hashes as on any start.
fn copy_models(from_data_dir: &Path, to_data_dir: &Path) {
    let from = from_data_dir.join("models");
    assert!(from.is_dir(), "CLAWG_BENCH_MODEL_DIR has no models/ directory: {}", from.display());
    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).expect("models dir");
        for entry in std::fs::read_dir(from).expect("read models") {
            let entry = entry.expect("entry");
            let target = to.join(entry.file_name());
            if entry.file_type().expect("type").is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).expect("copy a model file");
            }
        }
    }
    copy_dir(&from, &to_data_dir.join("models"));
}

async fn start_daemon(home: &Path, config: &Config) -> Daemon {
    let data_dir = home.join(".clawg-dev");
    if let Some(dir) = &config.model_dir {
        copy_models(dir, &data_dir);
    }
    let log = std::fs::File::create(home.join("serve.log")).expect("log file");
    let mut command = Command::new(BIN);
    command
        .args(["serve", "--profile", "dev", "--port", "0"])
        .env("CLAWG_TEST_HOME", home)
        .env_remove("CLAWG_PROFILE")
        .env_remove("CLAWG_TEST_DEFAULT_PORT")
        .env_remove("CLAWG_TEST_EMBEDDER")
        .env("RUST_LOG", "info")
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    if config.model_dir.is_none() {
        command.env("CLAWG_TEST_EMBEDDER", "fake");
    }
    let mut child = command.spawn().expect("spawn clawg serve");
    let deadline = Instant::now() + START_BUDGET;
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            panic!("clawg serve exited with {status}:\n{}", std::fs::read_to_string(home.join("serve.log")).unwrap_or_default());
        }
        if let Some(client) = DaemonClient::from_data_dir(&data_dir) {
            if health_of(&client).await.is_some_and(|h| h["embedder"] == "ready") {
                return Daemon { child, client };
            }
        }
        assert!(Instant::now() < deadline, "clawg serve did not become ready");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn send_json(client: &DaemonClient, method: reqwest::Method, path: &str, body: Value) -> (u16, Value) {
    let call = Call::new(method, path).json(&body).expect("body");
    let answer = client.send(call).await.expect("send");
    (answer.status.as_u16(), serde_json::from_slice(&answer.body).unwrap_or(Value::Null))
}

async fn get_json(client: &DaemonClient, path: &str) -> Value {
    let answer = client.send(Call::new(reqwest::Method::GET, path)).await.expect("send");
    assert!(answer.status.is_success(), "{path}: {}", answer.status);
    serde_json::from_slice(&answer.body).expect("json")
}

/// The ids the import gave the seeded projects.
async fn project_ids(client: &DaemonClient) -> Vec<String> {
    let listed = get_json(client, "/v1/projects").await;
    let mut ids: Vec<String> = listed["projects"]
        .as_array()
        .expect("projects")
        .iter()
        .filter(|p| p["name"].as_str().is_some_and(|n| PROJECTS.contains(&n)))
        .filter_map(|p| p["id"].as_str().map(str::to_owned))
        .collect();
    ids.sort();
    ids
}

/// Everything one request needs, decided before the scenario starts so the
/// schedule never waits on it.
#[derive(Clone)]
struct Request {
    path: &'static str,
    body: Value,
}

struct Plan {
    seed: u64,
    rows: usize,
    queries: Vec<String>,
    projects: Vec<String>,
}

impl Plan {
    fn request(&self, kind: Kind, scenario: &str, n: usize) -> Request {
        let mut rng = Rng::new(self.seed ^ fnv(scenario) ^ (n as u64) << 20);
        let agent = AGENTS[rng.below(AGENTS.len() as u64) as usize];
        let project = (!self.projects.is_empty() && !rng.chance(25))
            .then(|| self.projects[rng.below(self.projects.len() as u64) as usize].clone());
        let query = self.queries[(fnv(scenario) as usize).wrapping_add(n) % self.queries.len()].clone();
        match kind {
            Kind::Search => Request {
                path: "/v1/search",
                body: json!({ "query": query, "agent_id": agent, "project_id": project, "limit": 10 }),
            },
            Kind::Context => Request {
                path: "/v1/context",
                body: json!({ "query": query, "agent_id": agent, "project_id": project }),
            },
            Kind::SessionStart => Request { path: "/v1/context", body: json!({ "agent_id": agent, "project_id": project }) },
            Kind::Save(pieces) => Request {
                path: "/v1/memories",
                body: json!({
                    "content": save_text(self.seed, scenario, n, pieces, self.rows),
                    "agent_id": agent,
                    "project_id": project,
                }),
            },
        }
    }
}

/// Latencies in microseconds, from the instant each request was due, and how
/// many failed.
#[derive(Default)]
struct Measured {
    samples_us: Vec<u64>,
    failed: usize,
}

impl Measured {
    fn add(&mut self, result: Result<u64, String>) {
        match result {
            Ok(us) => self.samples_us.push(us),
            Err(e) => {
                if self.failed == 0 {
                    eprintln!("a request failed: {e}");
                }
                self.failed += 1;
            }
        }
    }
}

async fn send_due(client: DaemonClient, request: Request, due: Instant) -> Result<u64, String> {
    let call = Call::new(reqwest::Method::POST, request.path).json(&request.body).map_err(|e| e.to_string())?;
    let answer = client.send(call).await.map_err(|e| e.to_string())?;
    let took = due.elapsed().as_micros() as u64;
    if answer.status.is_success() {
        Ok(took)
    } else {
        Err(format!("{} {}: {}", request.path, answer.status, String::from_utf8_lossy(&answer.body)))
    }
}

/// Sends each request at its time and gathers them all, however slow.
async fn open_loop(client: &DaemonClient, requests: Vec<Request>, rate_per_s: f64) -> Measured {
    let start = Instant::now();
    let mut tasks = Vec::with_capacity(requests.len());
    let offsets = schedule(rate_per_s, requests.len());
    for (request, offset) in requests.into_iter().zip(offsets) {
        let due = start + offset;
        tokio::time::sleep_until(due.into()).await;
        tasks.push(tokio::spawn(send_due(client.clone(), request, due)));
    }
    let mut measured = Measured::default();
    for task in tasks {
        measured.add(task.await.expect("request task"));
    }
    measured
}

/// Which stream sends next, and when from the start: the one whose next
/// request is due first, the earlier stream on a tie.
fn earliest(next: &[usize], rates: &[f64]) -> (usize, Duration) {
    next.iter()
        .zip(rates)
        .map(|(&n, &rate)| Duration::from_secs_f64(n as f64 / rate))
        .enumerate()
        .min_by_key(|&(i, due)| (due, i))
        .expect("at least one stream")
}

/// Sends every stream of `DURING` at its rate for as long as `running` says
/// the background operation goes on.
async fn open_loop_while(client: &DaemonClient, plan: &Plan, name: &str, running: impl Fn() -> bool) -> Vec<Measured> {
    let start = Instant::now();
    let names: Vec<String> = DURING.iter().map(|(stream, _, _)| format!("{stream}_during_{name}")).collect();
    let rates: Vec<f64> = DURING.iter().map(|&(_, _, rate)| rate).collect();
    let mut next = vec![0usize; DURING.len()];
    let mut tasks: Vec<Vec<_>> = DURING.iter().map(|_| Vec::new()).collect();
    loop {
        let (stream, offset) = earliest(&next, &rates);
        let due = start + offset;
        tokio::time::sleep_until(due.into()).await;
        if !running() {
            break;
        }
        let request = plan.request(DURING[stream].1, &names[stream], next[stream]);
        tasks[stream].push(tokio::spawn(send_due(client.clone(), request, due)));
        next[stream] += 1;
    }
    let mut measured = Vec::new();
    for stream in tasks {
        let mut m = Measured::default();
        for task in stream {
            m.add(task.await.expect("request task"));
        }
        measured.push(m);
    }
    measured
}

/// How many docs refreshes the daemon has finished.
async fn refreshes_done(client: &DaemonClient) -> u64 {
    let metrics = get_json(client, "/v1/metrics").await;
    metrics["run"]["histograms"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|h| h["name"] == "docs.refresh.duration")
        .map(|h| h["count"].as_u64().unwrap_or(0))
        .sum()
}

/// Runs one background operation while the streams of `DURING` are
/// measured, and returns them with how long the operation took and how many
/// times it ran: a save or a backup again until `MIN_BACKGROUND` has passed,
/// the docs refresh once.
async fn during(client: &DaemonClient, plan: &Plan, background: Background, docs_root: &Path) -> (Vec<Measured>, Duration, usize) {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    let done = Arc::new(AtomicBool::new(false));
    let runs = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();
    let op = {
        let client = client.clone();
        let (done, runs) = (done.clone(), runs.clone());
        let saves: Vec<Request> = (0..64).map(|n| plan.request(Kind::Save(100), "background_save_100", n)).collect();
        let docs_root = docs_root.to_path_buf();
        tokio::spawn(async move {
            let deadline = Instant::now() + BACKGROUND_BUDGET;
            let mut saves = saves.into_iter();
            loop {
                match background {
                    Background::Save100 => {
                        let save = saves.next().expect("a save left for the window");
                        let (status, body) = send_json(&client, reqwest::Method::POST, save.path, save.body).await;
                        assert!((200..300).contains(&status), "background save: {status} {body}");
                    }
                    Background::Backup => {
                        let (status, body) = send_json(&client, reqwest::Method::POST, "/v1/backup", json!({})).await;
                        assert!((200..300).contains(&status), "backup: {status} {body}");
                        // Each snapshot is a copy of the store; only the time it took matters.
                        if let Some(path) = body["path"].as_str() {
                            let _ = std::fs::remove_file(path);
                        }
                    }
                    Background::DocsRefresh => {
                        let before = refreshes_done(&client).await;
                        let body = json!({ "root_path": docs_root.to_string_lossy(), "index_docs": true, "assigned": true });
                        let (status, answer) = send_json(&client, reqwest::Method::POST, "/v1/projects", body).await;
                        assert!((200..300).contains(&status), "register the docs: {status} {answer}");
                        while refreshes_done(&client).await == before {
                            assert!(Instant::now() < deadline, "the docs refresh did not end");
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        }
                    }
                }
                runs.fetch_add(1, Ordering::SeqCst);
                if background == Background::DocsRefresh || started.elapsed() >= MIN_BACKGROUND {
                    break;
                }
            }
            done.store(true, Ordering::SeqCst);
        })
    };
    let measured = open_loop_while(client, plan, background.name(), || !done.load(Ordering::SeqCst)).await;
    op.await.expect("background operation");
    (measured, started.elapsed(), runs.load(Ordering::SeqCst))
}

/// How long the background operation of a scenario ran, and how many times.
#[derive(Clone, Copy)]
struct BackgroundRun {
    seconds: f64,
    runs: usize,
}

fn scenario_value(name: &str, rate_per_s: f64, measured: &Measured, background: Option<BackgroundRun>) -> Value {
    json!({
        "name": name,
        "rate_per_s": rate_per_s,
        "sent": measured.samples_us.len() + measured.failed,
        "failed": measured.failed,
        "background_seconds": background.map(|b| b.seconds),
        "background_runs": background.map(|b| b.runs),
        "summary": summary(&measured.samples_us),
        "samples_us": measured.samples_us,
    })
}

/// The run's histograms of `GET /v1/metrics` without their buckets, and the
/// minute's gauges of the embedder.
fn metrics_breakdown(metrics: &Value) -> Value {
    let strip = |h: &Value| {
        let mut h = h.clone();
        if let Some(map) = h.as_object_mut() {
            map.remove("buckets");
        }
        h
    };
    let histograms: Vec<Value> = metrics["run"]["histograms"].as_array().into_iter().flatten().map(strip).collect();
    let gauges: Vec<Value> = metrics["minute"]["gauges"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|g| g["name"].as_str().is_some_and(|n| n.starts_with("embedder.")))
        .cloned()
        .collect();
    json!({ "histograms": histograms, "embedder_gauges": gauges })
}

fn results_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("docs").join("eval").join("results")
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_millis() as i64
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a benchmark: run it with `cargo test -p clawg-cli --test perf_bench -- --ignored --nocapture`"]
async fn bench() {
    let config = Config::from_vars(|k| std::env::var(k).ok()).unwrap_or_else(|e| panic!("{e}"));
    let started_at_ms = now_ms();
    let out = results_dir().join(config.file_name(&utc_date(started_at_ms)));
    assert!(!out.exists(), "{} already exists; a run never replaces one, name it with CLAWG_BENCH_TAG", out.display());

    let home = tempfile::tempdir().expect("tempdir");
    let daemon = start_daemon(home.path(), &config).await;
    let client = daemon.client.clone();
    let pid = daemon.child.id();

    let seeding = Instant::now();
    let export = seed_export(config.seed, config.rows, started_at_ms);
    let (status, report) = send_json(&client, reqwest::Method::POST, "/v1/import", json!({ "text": export })).await;
    assert_eq!(status, 200, "import: {report}");
    let seed_seconds = seeding.elapsed().as_secs_f64();
    assert_eq!(report["rows"].as_u64(), Some(config.rows as u64), "{report}");
    assert!(report["invalid"].as_array().is_some_and(Vec::is_empty), "{report}");
    println!("seeded {} rows in {seed_seconds:.1} s: {}", config.rows, report["by_state"]);

    let docs_root = home.path().join("docs-corpus");
    std::fs::create_dir_all(&docs_root).expect("docs root");
    for (name, text) in docs_corpus(config.seed, DOCS_FILES) {
        std::fs::write(docs_root.join(name), text).expect("write a doc");
    }

    let plan = Plan {
        seed: config.seed,
        rows: config.rows,
        queries: queries(config.seed, config.rows, 1_000),
        projects: project_ids(&client).await,
    };
    assert_eq!(plan.projects.len(), PROJECTS.len(), "the seeded projects: {:?}", plan.projects);

    let idle_from = usage_of(pid);
    let idle_started = Instant::now();
    tokio::time::sleep(IDLE).await;
    let cpu_idle_ratio = cpu_ratio(idle_from, usage_of(pid), idle_started.elapsed());

    for (kind, n) in [(Kind::Search, 10), (Kind::Context, 5), (Kind::SessionStart, 5), (Kind::Save(1), 5)] {
        let requests = (0..n).map(|i| plan.request(kind, "warm_up", i)).collect();
        open_loop(&client, requests, 5.0).await;
    }

    let load_from = usage_of(pid);
    let load_started = Instant::now();
    let mut scenarios = Vec::new();
    println!("{:<28} {:>6} {:>7} {:>10} {:>10} {:>10} {:>10}", "scenario", "sent", "failed", "p50 ms", "p95 ms", "p99 ms", "max ms");
    let mut report_line = |value: &Value| {
        let s = &value["summary"];
        let ms = |k: &str| s[k].as_f64().unwrap_or(0.0) / 1000.0;
        println!(
            "{:<28} {:>6} {:>7} {:>10.2} {:>10.2} {:>10.2} {:>10.2}",
            value["name"].as_str().unwrap_or(""),
            value["sent"],
            value["failed"],
            ms("p50_us"),
            ms("p95_us"),
            ms("p99_us"),
            ms("max_us")
        );
        scenarios.push(value.clone());
    };
    for steady in &STEADY {
        let requests = (0..steady.count).map(|i| plan.request(steady.kind, steady.name, i)).collect();
        let measured = open_loop(&client, requests, steady.rate_per_s).await;
        report_line(&scenario_value(steady.name, steady.rate_per_s, &measured, None));
    }
    for background in Background::ALL {
        let (measured, took, runs) = during(&client, &plan, background, &docs_root).await;
        let run = BackgroundRun { seconds: took.as_secs_f64(), runs };
        for ((stream, _, rate), m) in DURING.iter().zip(&measured) {
            report_line(&scenario_value(&format!("{stream}_during_{}", background.name()), *rate, m, Some(run)));
        }
    }
    let load_usage = usage_of(pid);
    let cpu_load_ratio = cpu_ratio(load_from, load_usage, load_started.elapsed());

    let metrics = get_json(&client, "/v1/metrics").await;
    let stats = get_json(&client, "/v1/stats").await;
    let result = json!({
        "format": RESULT_FORMAT,
        "version": RESULT_VERSION,
        "clawg": env!("CARGO_PKG_VERSION"),
        "build": if cfg!(debug_assertions) { "debug" } else { "release" },
        "embedder": config.embedder(),
        "machine": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "threads": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        },
        "started_at_ms": started_at_ms,
        "store": {
            "rows": config.rows,
            "seed": config.seed,
            "seed_seconds": seed_seconds,
            "by_state": report["by_state"],
            "docs_files": DOCS_FILES,
            "stats": stats,
        },
        "process": {
            "cpu_idle_ratio": cpu_idle_ratio,
            "cpu_load_ratio": cpu_load_ratio,
            "rss_peak_bytes": load_usage.rss_peak_bytes,
            "private_peak_bytes": load_usage.private_peak_bytes,
        },
        "scenarios": scenarios,
        "metrics": metrics_breakdown(&metrics),
    });
    println!(
        "cpu idle {:?}, under load {:?}, rss peak {:?} bytes",
        cpu_idle_ratio, cpu_load_ratio, load_usage.rss_peak_bytes
    );
    drop(daemon);

    let mut text = serde_json::to_string_pretty(&result).expect("serialize");
    text.push('\n');
    write_new(&out, text.as_bytes()).unwrap_or_else(|e| panic!("write {}: {e}", out.display()));
    println!("wrote {}", out.display());
    let failed: u64 = result["scenarios"].as_array().into_iter().flatten().map(|s| s["failed"].as_u64().unwrap_or(0)).sum();
    assert_eq!(failed, 0, "some requests failed; the result file says which scenarios");
}

#[cfg(test)]
mod pure {
    use super::*;
    use clawg_core::memory::portability::format::parse;

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: BTreeMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn the_seeded_store_is_the_same_for_a_seed_and_another_for_another() {
        let a = seed_export(7, 300, 1_700_000_000_000);
        assert_eq!(a, seed_export(7, 300, 1_700_000_000_000));
        assert_ne!(a, seed_export(8, 300, 1_700_000_000_000));
    }

    #[test]
    fn the_seeded_store_is_an_export_the_import_reads_whole() {
        let rows = 2_000;
        let (_, parsed) = parse(&seed_export(DEFAULT_SEED, rows, 1_700_000_000_000)).expect("a header");
        assert!(parsed.invalid.is_empty(), "{:?}", parsed.invalid.first());
        assert_eq!(parsed.memories.len(), rows);
        assert_eq!(parsed.projects.len(), PROJECTS.len());
        let count = |f: &dyn Fn(&MemoryLine) -> bool| parsed.memories.iter().filter(|(_, m)| f(m)).count();
        let superseded = count(&|m| m.status == "superseded");
        let deleted = count(&|m| m.deleted_at.is_some());
        let unscoped = count(&|m| m.project_id.is_none());
        assert!((150..=250).contains(&superseded), "superseded {superseded}");
        assert!((50..=130).contains(&deleted), "deleted {deleted}");
        assert!((400..=600).contains(&unscoped), "without a project {unscoped}");
        for agent in AGENTS {
            assert!(count(&|m| m.agent_id.as_deref() == Some(agent)) > 300, "{agent}");
        }
        for (_, m) in &parsed.memories {
            if let Some(by) = m.superseded_by {
                assert!(by > m.id && by <= rows as i64, "{} superseded by {by}", m.id);
            }
        }
    }

    #[test]
    fn rows_share_frequent_words_and_few_share_an_identifier() {
        let (_, parsed) = parse(&seed_export(DEFAULT_SEED, 2_000, 1_700_000_000_000)).expect("a header");
        let with = |w: &str| parsed.memories.iter().filter(|(_, m)| m.content.split(' ').any(|x| x == w)).count();
        assert!(with(FREQUENT[0]) > 1_000, "the most frequent word is in most rows");
        assert!(with("ident7") < 40, "an identifier is in a few rows");
    }

    #[test]
    fn queries_and_saves_repeat_for_a_seed() {
        assert_eq!(queries(3, 1_000, 50), queries(3, 1_000, 50));
        assert_ne!(queries(3, 1_000, 50), queries(4, 1_000, 50));
        assert!(queries(3, 1_000, 300).iter().any(|q| q.contains("ident")));
        assert_eq!(save_text(3, "save_10", 2, 10, 1_000), save_text(3, "save_10", 2, 10, 1_000));
        assert_ne!(save_text(3, "save_10", 2, 10, 1_000), save_text(3, "save_10", 3, 10, 1_000));
        assert_ne!(save_text(3, "save_1", 2, 1, 1_000), save_text(3, "save_10", 2, 1, 1_000));
    }

    #[test]
    fn a_save_has_the_words_of_its_pieces_and_fits_the_content_limit() {
        for (pieces, words) in [(1, 40), (10, 744), (100, 7_224)] {
            assert_eq!(words_for_pieces(pieces), words);
            let text = save_text(DEFAULT_SEED, "save", 0, pieces, 100_000);
            assert_eq!(text.split(' ').count(), words + 4, "{pieces} pieces");
            assert!(text.len() < 64 * 1024, "{pieces} pieces: {} bytes", text.len());
        }
    }

    #[test]
    fn the_docs_corpus_repeats_for_a_seed() {
        let docs = docs_corpus(1, 3);
        assert_eq!(docs, docs_corpus(1, 3));
        assert_eq!(docs.len(), 3);
        assert_eq!(docs[0].0, "doc-0000.md");
        assert_eq!(docs[0].1.matches("\n## ").count() + usize::from(docs[0].1.starts_with("## ")), 8);
    }

    #[test]
    fn requests_leave_at_a_fixed_rate() {
        assert_eq!(schedule(4.0, 3), vec![Duration::ZERO, Duration::from_millis(250), Duration::from_millis(500)]);
        assert!(schedule(10.0, 0).is_empty());
    }

    #[test]
    fn streams_send_in_the_order_their_requests_fall_due() {
        let rates = [20.0, 10.0, 2.0];
        let mut next = [0usize; 3];
        let mut order = Vec::new();
        for _ in 0..8 {
            let (stream, due) = earliest(&next, &rates);
            order.push((stream, due.as_millis()));
            next[stream] += 1;
        }
        assert_eq!(order, [(0, 0), (1, 0), (2, 0), (0, 50), (0, 100), (1, 100), (0, 150), (0, 200)]);
    }

    #[test]
    fn a_quantile_is_the_nearest_rank() {
        let sorted: Vec<u64> = (1..=100).collect();
        assert_eq!(quantile(&sorted, 0.50), 50);
        assert_eq!(quantile(&sorted, 0.95), 95);
        assert_eq!(quantile(&sorted, 0.99), 99);
        assert_eq!(quantile(&sorted, 1.0), 100);
        assert_eq!(quantile(&[7], 0.5), 7);
        assert_eq!(quantile(&[3, 9], 0.5), 3);
        assert_eq!(quantile(&[], 0.5), 0);
        assert_eq!(summary(&[5, 1, 3])["p50_us"], 3);
        assert_eq!(summary(&[5, 1, 3])["max_us"], 5);
        assert_eq!(summary(&[5, 1, 3])["mean_us"], 3.0);
    }

    #[test]
    fn the_config_reads_its_defaults_and_refuses_what_it_cannot_use() {
        let default = Config::from_vars(vars(&[])).expect("defaults");
        assert_eq!(default, Config { rows: DEFAULT_ROWS, seed: DEFAULT_SEED, tag: None, model_dir: None });
        assert_eq!(default.embedder(), "fake");
        assert_eq!(default.file_name("2026-10-31"), "2026-10-31-fake-perf-bench-10000.json");

        let asked = Config::from_vars(vars(&[
            ("CLAWG_BENCH_ROWS", "100000"),
            ("CLAWG_BENCH_SEED", "9"),
            ("CLAWG_BENCH_TAG", "sleep-5ms"),
            ("CLAWG_BENCH_MODEL_DIR", "/data"),
        ]))
        .expect("valid");
        assert_eq!(asked.embedder(), "real");
        assert_eq!(asked.file_name("2026-10-31"), "2026-10-31-real-perf-bench-100000-sleep-5ms.json");
        assert_eq!(asked.seed, 9);

        for bad in [("CLAWG_BENCH_ROWS", "0"), ("CLAWG_BENCH_ROWS", "ten"), ("CLAWG_BENCH_ROWS", "1000001")] {
            assert!(Config::from_vars(vars(&[bad])).unwrap_err().contains("CLAWG_BENCH_ROWS"), "{bad:?}");
        }
        assert!(Config::from_vars(vars(&[("CLAWG_BENCH_SEED", "-1")])).unwrap_err().contains("CLAWG_BENCH_SEED"));
        for bad in ["Upper", "a/b", "", "a b"] {
            assert!(Config::from_vars(vars(&[("CLAWG_BENCH_TAG", bad)])).unwrap_err().contains("CLAWG_BENCH_TAG"), "{bad:?}");
        }
    }

    #[test]
    fn the_date_of_a_file_is_utc() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(1_761_868_800_000), "2025-10-31");
        assert_eq!(utc_date(1_761_868_799_999), "2025-10-30");
    }

    #[test]
    fn a_result_file_is_never_replaced() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("results").join("run.json");
        write_new(&path, b"first").expect("a new file");
        assert!(write_new(&path, b"second").is_err());
        assert_eq!(std::fs::read(&path).expect("read"), b"first");
    }

    #[test]
    fn the_cpu_of_a_process_is_read_after_its_command() {
        let stat = "4242 (clawg (x) y) S 1 4242 4242 0 -1 4194560 900 0 0 0 125 37 0 0 20 0 9 0 100 0 0";
        assert_eq!(cpu_ticks_of_stat(stat), Some(162));
        assert_eq!(cpu_ticks_of_stat("garbage"), None);
        let at = |cpu_ms| Usage { cpu: Some(Duration::from_millis(cpu_ms)), ..Usage::default() };
        assert_eq!(cpu_ratio(at(100), at(600), Duration::from_secs(1)), Some(0.5));
        assert_eq!(cpu_ratio(Usage::default(), at(600), Duration::from_secs(1)), None);
        assert_eq!(cpu_ratio(at(100), at(600), Duration::ZERO), None);
    }
}
