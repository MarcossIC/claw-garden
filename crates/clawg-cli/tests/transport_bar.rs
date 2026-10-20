//! The AF_UNIX transport against TCP on Windows, over the real daemon.
//!
//! A debug build says nothing about the bar, and a run takes a while, so the
//! test is ignored. Run it optimized:
//! `cargo test -p clawg-cli --release --test transport_bar -- --ignored --nocapture`
//!
//! The daemon is the `testing` build of the binary under test with the fake
//! model. Both sides are `DaemonClient::from_data_dir`, as a hook builds its
//! client: one over the daemon's data directory, which takes the socket, and
//! one over a view of it whose `daemon.json` names the port alone.
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clawg_cli::client::{Call, DaemonClient};
use serde_json::{json, Value};
use tokio::sync::Barrier;

const BIN: &str = env!("CARGO_BIN_EXE_clawg");

const START_BUDGET: Duration = Duration::from_secs(60);

/// What AF_UNIX may cost over TCP, per scenario, in mean latency.
const MAX_RATIO: f64 = 1.10;

const RUNS: usize = 3;
const BURST: usize = 32;
const ROUNDS: usize = 20;
const CONCURRENT: usize = 8;

/// The size of the list answer the last scenario reads, in bytes.
const BODY_RANGE: std::ops::RangeInclusive<usize> = 7_000..=9_000;

struct Stats {
    mean: Duration,
    p99: Duration,
    failures: usize,
}

impl Stats {
    fn of(mut samples: Vec<Duration>, failures: usize) -> Self {
        samples.sort();
        let total: Duration = samples.iter().sum();
        let mean = total / samples.len().max(1) as u32;
        let p99 = samples.get((samples.len() * 99 / 100).min(samples.len().saturating_sub(1))).copied().unwrap_or_default();
        Self { mean, p99, failures }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Scenario {
    Sequential,
    NewClientPerRequest,
    Concurrent,
    Burst,
    List,
}

impl Scenario {
    const ALL: [Scenario; 5] =
        [Self::Sequential, Self::NewClientPerRequest, Self::Concurrent, Self::Burst, Self::List];

    fn name(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::NewClientPerRequest => "new client per request",
            Self::Concurrent => "8 concurrent tasks",
            Self::Burst => "burst of 32, 20 rounds",
            Self::List => "list of about 8 KB",
        }
    }

    /// How many requests a measured run makes, and how many a warm-up does.
    fn size(self, warm: bool) -> usize {
        let full = match self {
            Self::Sequential => 400,
            Self::NewClientPerRequest => 200,
            Self::Concurrent => 100,
            Self::Burst => ROUNDS,
            Self::List => 300,
        };
        if warm {
            match self {
                Self::Burst => 2,
                _ => full / 10,
            }
        } else {
            full
        }
    }
}

/// A daemon child, killed through its own handle when the guard drops.
struct Daemon {
    child: Child,
    data_dir: PathBuf,
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

fn state_of(data_dir: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(data_dir.join("daemon.json")).ok()?).ok()
}

async fn health_of(client: &DaemonClient) -> Option<Value> {
    let answer = client.send(health_call()).await.ok()?;
    serde_json::from_slice(&answer.body).ok()
}

async fn start_daemon(home: &Path) -> Daemon {
    let log = std::fs::File::create(home.join("serve.log")).expect("log file");
    let child = Command::new(BIN)
        .args(["serve", "--profile", "dev", "--port", "0"])
        .env("CLAWG_TEST_HOME", home)
        .env("CLAWG_TEST_EMBEDDER", "fake")
        .env_remove("CLAWG_PROFILE")
        .env_remove("CLAWG_TEST_DEFAULT_PORT")
        .env("RUST_LOG", "info")
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .expect("spawn clawg serve");
    let data_dir = home.join(".clawg-dev");
    let mut daemon = Daemon { child, data_dir };
    let deadline = Instant::now() + START_BUDGET;
    loop {
        if let Some(status) = daemon.child.try_wait().expect("try_wait") {
            panic!("clawg serve exited with {status}:\n{}", std::fs::read_to_string(home.join("serve.log")).unwrap_or_default());
        }
        let published = state_of(&daemon.data_dir)
            .is_some_and(|s| s["pid"].as_u64() == Some(u64::from(daemon.child.id())) && s["unix_socket"] == true && s["port"].is_u64());
        if published {
            if let Some(client) = DaemonClient::from_data_dir(&daemon.data_dir) {
                if health_of(&client).await.is_some_and(|h| h["embedder"] == "ready") {
                    return daemon;
                }
            }
        }
        assert!(Instant::now() < deadline, "clawg serve did not become ready");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A directory the daemon's token and port are copied into, without the
/// socket, so a client built from it takes TCP.
fn tcp_view(daemon: &Daemon, into: &Path) -> PathBuf {
    let dir = into.join("tcp-view");
    std::fs::create_dir_all(&dir).expect("view dir");
    std::fs::copy(daemon.data_dir.join("token"), dir.join("token")).expect("copy the token");
    let mut state = state_of(&daemon.data_dir).expect("daemon.json");
    state["unix_socket"] = json!(false);
    std::fs::write(dir.join("daemon.json"), state.to_string()).expect("write the view's daemon.json");
    dir
}

fn list_call(limit: usize) -> Call {
    Call::new(reqwest::Method::GET, "/v1/memories").query([("limit", limit.to_string())])
}

/// Saves memories through the daemon and returns the `limit` whose list
/// answer falls in `BODY_RANGE`.
async fn seed_and_calibrate(client: &DaemonClient) -> usize {
    const SEEDED: usize = 60;
    for n in 0..SEEDED {
        let content = format!(
            "Memory {n} about the transport bar: the module m{n} keeps its state in table t{n} and is read by worker w{}.",
            n * 7 + 3
        );
        let call = Call::new(reqwest::Method::POST, "/v1/memories")
            .json(&json!({ "content": content, "agent_id": "bar" }))
            .expect("body");
        let answer = client.send(call).await.expect("save");
        assert!(matches!(answer.status.as_u16(), 201 | 202), "{}", String::from_utf8_lossy(&answer.body));
    }
    let deadline = Instant::now() + START_BUDGET;
    loop {
        let health = health_of(client).await.expect("health");
        if health["ingest"]["pending"].as_u64() == Some(0) && health["embedder"] == "ready" {
            break;
        }
        assert!(Instant::now() < deadline, "saves still pending: {health}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for limit in 1..=SEEDED {
        let answer = client.send(list_call(limit)).await.expect("list");
        assert!(answer.status.is_success(), "{}", answer.status);
        if BODY_RANGE.contains(&answer.body.len()) {
            return limit;
        }
    }
    panic!("no limit up to {SEEDED} gives a list answer of {BODY_RANGE:?} bytes");
}

async fn check_list_size(client: &DaemonClient, limit: usize) {
    let answer = client.send(list_call(limit)).await.expect("list");
    assert!(BODY_RANGE.contains(&answer.body.len()), "the list answer is {} bytes", answer.body.len());
}

async fn timed(client: &DaemonClient, call: Call) -> Result<Duration, ()> {
    let started = Instant::now();
    match client.send(call).await {
        Ok(answer) if answer.status.is_success() => Ok(started.elapsed()),
        _ => Err(()),
    }
}

/// One run of `scenario` over the clients built from `dir`.
async fn run(scenario: Scenario, dir: &Path, warm: bool, limit: usize) -> Stats {
    let n = scenario.size(warm);
    let shared = DaemonClient::from_data_dir(dir).expect("client");
    let mut samples = Vec::new();
    let mut failures = 0;
    let mut tally = |result: Result<Duration, ()>| match result {
        Ok(d) => samples.push(d),
        Err(()) => failures += 1,
    };
    match scenario {
        Scenario::Sequential => {
            for _ in 0..n {
                tally(timed(&shared, health_call()).await);
            }
        }
        Scenario::List => {
            for _ in 0..n {
                tally(timed(&shared, list_call(limit)).await);
            }
        }
        Scenario::NewClientPerRequest => {
            for _ in 0..n {
                let started = Instant::now();
                let result = match DaemonClient::from_data_dir(dir) {
                    Some(client) => client.send(health_call()).await.map(|a| a.status.is_success()).unwrap_or(false),
                    None => false,
                };
                tally(if result { Ok(started.elapsed()) } else { Err(()) });
            }
        }
        Scenario::Concurrent => {
            let tasks: Vec<_> = (0..CONCURRENT)
                .map(|_| {
                    let client = shared.clone();
                    tokio::spawn(async move {
                        let mut out = Vec::new();
                        for _ in 0..n {
                            out.push(timed(&client, health_call()).await);
                        }
                        out
                    })
                })
                .collect();
            for task in tasks {
                for result in task.await.expect("task") {
                    tally(result);
                }
            }
        }
        Scenario::Burst => {
            for _ in 0..n {
                let barrier = Arc::new(Barrier::new(BURST));
                let tasks: Vec<_> = (0..BURST)
                    .map(|_| {
                        let barrier = barrier.clone();
                        let dir = dir.to_path_buf();
                        tokio::spawn(async move {
                            let client = DaemonClient::from_data_dir(&dir);
                            barrier.wait().await;
                            match client {
                                Some(client) => timed(&client, health_call()).await,
                                None => Err(()),
                            }
                        })
                    })
                    .collect();
                for task in tasks {
                    tally(task.await.expect("task"));
                }
            }
        }
    }
    Stats::of(samples, failures)
}

/// The run of median mean among `runs`.
fn median(mut runs: Vec<Stats>) -> Stats {
    runs.sort_by_key(|s| s.mean);
    runs.swap_remove(runs.len() / 2)
}

fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a benchmark: run it optimized with `cargo test -p clawg-cli --release --test transport_bar -- --ignored --nocapture`"]
async fn the_socket_is_not_slower_than_tcp() {
    let home = tempfile::tempdir().expect("tempdir");
    let daemon = start_daemon(home.path()).await;
    let unix = daemon.data_dir.clone();
    let tcp = tcp_view(&daemon, home.path());
    let limit = seed_and_calibrate(&DaemonClient::from_data_dir(&unix).expect("client")).await;
    check_list_size(&DaemonClient::from_data_dir(&tcp).expect("client"), limit).await;

    println!("{:<26} {:>12} {:>12} {:>12} {:>12} {:>7} {:>9}", "scenario", "unix mean", "unix p99", "tcp mean", "tcp p99", "ratio", "failures");
    let mut over = Vec::new();
    let mut burst_failures = 0;
    for scenario in Scenario::ALL {
        run(scenario, &unix, true, limit).await;
        run(scenario, &tcp, true, limit).await;
        let (mut unix_runs, mut tcp_runs) = (Vec::new(), Vec::new());
        for _ in 0..RUNS {
            unix_runs.push(run(scenario, &unix, false, limit).await);
            tcp_runs.push(run(scenario, &tcp, false, limit).await);
        }
        let (u, t) = (median(unix_runs), median(tcp_runs));
        let ratio = micros(u.mean) / micros(t.mean);
        println!(
            "{:<26} {:>10.0}us {:>10.0}us {:>10.0}us {:>10.0}us {:>7.2} {:>4}/{:<4}",
            scenario.name(),
            micros(u.mean),
            micros(u.p99),
            micros(t.mean),
            micros(t.p99),
            ratio,
            u.failures,
            t.failures
        );
        if ratio > MAX_RATIO {
            over.push(format!("{}: {ratio:.2}", scenario.name()));
        }
        if scenario == Scenario::Burst {
            burst_failures = u.failures;
        }
    }
    println!("failures are unix/tcp");

    assert_eq!(burst_failures, 0, "the AF_UNIX burst had failures");
    assert!(over.is_empty(), "AF_UNIX is over {MAX_RATIO}x TCP in: {}", over.join(", "));
}
