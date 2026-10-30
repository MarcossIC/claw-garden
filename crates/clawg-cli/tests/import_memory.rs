//! What importing a large file costs in memory, in the process that asks and
//! in the daemon, with the file sent in the body and with its path.
//!
//! A measurement, not a behavior: the tests are ignored and assert nothing but
//! what they need to run. A peak only rises, so each runs alone, in a process
//! of its own, and prints one JSON line per format:
//! `cargo test -p clawg-cli --test import_memory -- --ignored --exact by_body --nocapture`
//! (likewise `by_path`). The client's peak of the second format includes the
//! first's, so for a clean figure of each, name one with
//! `IMPORT_MEMORY_FORMAT=clawg` or `IMPORT_MEMORY_FORMAT=engram`.
//!
//! The daemon is the `testing` build of the binary under test with the fake
//! model, a new one for every format, so its peak is that format's alone.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use clawg_cli::cli::ImportSource;
use clawg_cli::client::{Call, DaemonClient};
use clawg_cli::daemon::process;
use clawg_cli::import::{self, Maps, Watch};
use clawg_core::preferences::Lang;
use serde_json::{json, Value};
use tokio::sync::mpsc;

const BIN: &str = env!("CARGO_BIN_EXE_clawg");

const START_BUDGET: Duration = Duration::from_secs(60);

/// A little over the minute between two samples of the daemon's process, so
/// one is taken after the import ends.
const SAMPLE_WAIT: Duration = Duration::from_secs(70);

const FILE_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Clone, Copy)]
enum Format {
    Clawg,
    Engram,
}

impl Format {
    fn name(self) -> &'static str {
        self.source().as_str()
    }

    fn source(self) -> ImportSource {
        match self {
            Self::Clawg => ImportSource::Clawg,
            Self::Engram => ImportSource::Engram,
        }
    }
}

fn formats() -> Vec<Format> {
    match std::env::var("IMPORT_MEMORY_FORMAT").as_deref() {
        Ok("clawg") => vec![Format::Clawg],
        Ok("engram") => vec![Format::Engram],
        _ => vec![Format::Clawg, Format::Engram],
    }
}

/// About a kilobyte of words, the same for the same `n`.
fn text_of(n: u64) -> String {
    let mut x = n.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let mut text = format!("Row {n}:");
    for _ in 0..150 {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let _ = write!(text, " w{}", (x >> 33) % 4000);
    }
    text
}

/// Writes an export of `format` of about `FILE_BYTES`, a line at a time so the
/// test process never holds it, and returns how many memories it has.
fn write_export(format: Format, path: &Path) -> u64 {
    let mut out = BufWriter::new(File::create(path).expect("the export"));
    let mut bytes = 0u64;
    let put = |out: &mut BufWriter<File>, line: &str| -> u64 {
        out.write_all(line.as_bytes()).expect("write");
        line.len() as u64
    };
    let mut rows = 0u64;
    match format {
        Format::Clawg => {
            bytes += put(&mut out, &format!("{}\n", json!({ "kind": "header", "format": "clawg-memories", "version": 1, "exported_at": 0, "clawg": "0.0.0" })));
            bytes += put(&mut out, &format!("{}\n", json!({ "kind": "project", "id": "repo-0000", "root_path": "/far/away/repo" })));
            while bytes < FILE_BYTES {
                rows += 1;
                let line = json!({
                    "kind": "memory", "id": rows, "scope": "agent", "agent_id": "a", "project_id": "repo-0000", "content": text_of(rows),
                    "memory_type": "semantic_fact", "source": "explicit", "created_at": 10 + rows, "created_by_type": "agent",
                    "last_modified_at": 10 + rows, "last_accessed_at": 20 + rows, "access_count": 0, "importance": 5.0, "tier": 3, "status": "active",
                });
                bytes += put(&mut out, &format!("{line}\n"));
            }
        }
        Format::Engram => {
            bytes += put(&mut out, r#"{"version":"0.2.0","exported_at":"2025-03-01 10:00:00","sessions":[],"observations":["#);
            while bytes < FILE_BYTES {
                rows += 1;
                let observation = json!({
                    "id": rows, "sync_id": format!("o{rows}"), "session_id": "s1", "type": "discovery", "title": format!("Note {rows}"),
                    "content": text_of(rows), "project": "app", "scope": "project", "revision_count": 1, "duplicate_count": 1,
                    "created_at": "2025-01-01 10:00:00", "updated_at": "2025-01-01 10:00:00",
                });
                bytes += put(&mut out, &format!("{}{observation}", if rows > 1 { "," } else { "" }));
            }
            put(&mut out, r#"],"prompts":[]}"#);
        }
    }
    out.flush().expect("flush");
    rows
}

/// A daemon child, killed through its own handle when the guard drops.
struct Daemon {
    child: Child,
    data_dir: PathBuf,
    log: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn state_of(data_dir: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(data_dir.join("daemon.json")).ok()?).ok()
}

async fn embedder_ready(client: &DaemonClient) -> bool {
    let Ok(answer) = client.send(Call::new(reqwest::Method::GET, "/health").without_bearer()).await else { return false };
    serde_json::from_slice::<Value>(&answer.body).is_ok_and(|h| h["embedder"] == "ready")
}

async fn start_daemon(home: &Path) -> (Daemon, DaemonClient) {
    let log = home.join("serve.log");
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
        .stderr(File::create(&log).expect("log file"))
        .spawn()
        .expect("spawn clawg serve");
    let mut daemon = Daemon { child, data_dir: home.join(".clawg-dev"), log };
    let deadline = Instant::now() + START_BUDGET;
    loop {
        if let Some(status) = daemon.child.try_wait().expect("try_wait") {
            panic!("clawg serve exited with {status}:\n{}", std::fs::read_to_string(&daemon.log).unwrap_or_default());
        }
        let published = state_of(&daemon.data_dir).is_some_and(|s| s["pid"].as_u64() == Some(u64::from(daemon.child.id())));
        if published {
            if let Some(client) = DaemonClient::from_data_dir(&daemon.data_dir) {
                if embedder_ready(&client).await {
                    return (daemon, client);
                }
            }
        }
        assert!(Instant::now() < deadline, "clawg serve did not become ready");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The highest private memory the daemon's samples show, as Prometheus text
/// keeps the last of each gauge, over `SAMPLE_WAIT`: the peak only rises, so
/// the sample taken after the import ended is the largest.
async fn daemon_peak(client: &DaemonClient) -> (&'static str, Option<u64>) {
    let names = [("process.private_peak_bytes", "clawg_process_private_peak_bytes "), ("process.rss_peak_bytes", "clawg_process_rss_peak_bytes ")];
    let mut peaks = [None::<f64>; 2];
    let deadline = Instant::now() + SAMPLE_WAIT;
    while Instant::now() < deadline {
        if let Ok(answer) = client.send(Call::new(reqwest::Method::GET, "/v1/metrics?format=prometheus")).await {
            let text = String::from_utf8_lossy(&answer.body);
            for (peak, (_, series)) in peaks.iter_mut().zip(names) {
                let value = text.lines().find_map(|line| line.strip_prefix(series)).and_then(|v| v.trim().parse::<f64>().ok());
                *peak = (*peak).into_iter().chain(value).reduce(f64::max);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    names.iter().zip(peaks).find_map(|((name, _), peak)| peak.map(|bytes| (*name, Some(bytes as u64)))).unwrap_or(("none", None))
}

/// What the daemon logged as `embed` ended, with its private memory.
fn embed_line(log: &Path) -> Option<String> {
    let text = std::fs::read_to_string(log).ok()?;
    text.lines().rev().find_map(|line| line.split_once("import: embed ").map(|(_, rest)| format!("embed {}", rest.trim())))
}

/// How many bytes the test process had reached: private on Windows, resident
/// on Linux.
fn client_peak() -> Option<u64> {
    let sample = process::sample();
    sample.private_peak_bytes.or(sample.rss_peak_bytes)
}

/// One format with one arm: the import `arm` makes of a file in a new daemon.
async fn measure<F, Fut>(arm: &str, format: Format, import: F)
where
    F: FnOnce(DaemonClient, PathBuf, Format) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let home = tempfile::tempdir().expect("tempdir");
    let file = home.path().join("export");
    let rows = write_export(format, &file);
    let bytes = std::fs::metadata(&file).expect("export size").len();
    let (daemon, client) = start_daemon(home.path()).await;
    let before = client_peak();
    let started = Instant::now();
    let ok = import(client.clone(), file, format).await;
    let seconds = started.elapsed().as_secs_f64();
    let after = client_peak();
    let (metric, daemon_peak) = daemon_peak(&client).await;
    println!(
        "{}",
        json!({
            "arm": arm, "format": format.name(), "bytes": bytes, "rows": rows, "ok": ok, "seconds": seconds,
            "client_peak_before_bytes": before, "client_peak_bytes": after,
            "daemon_peak_metric": metric, "daemon_peak_bytes": daemon_peak,
            "daemon_embed": embed_line(&daemon.log),
        })
    );
}

/// The client before the job: the whole file read, put in a body and sent.
#[tokio::test]
#[ignore = "a measurement: run it alone, see the module docs"]
async fn by_body() {
    for format in formats() {
        measure("by_body", format, |client, file, format| async move {
            let text = std::fs::read_to_string(&file).expect("read the export");
            let body = json!({ "text": text, "from": format.name(), "dry_run": false, "agent": null, "map_roots": [], "project_map": [] });
            let call = Call::new(reqwest::Method::POST, "/v1/import").json(&body).expect("body");
            client.send(call).await.is_ok_and(|answer| answer.status.is_success())
        })
        .await;
    }
}

/// The client of `clawg import`: the path is sent and the job followed.
#[tokio::test]
#[ignore = "a measurement: run it alone, see the module docs"]
async fn by_path() {
    for format in formats() {
        measure("by_path", format, |client, file, format| async move {
            let (_signals, mut interrupts) = mpsc::unbounded_channel();
            let maps = Maps { roots: &[], projects: &[] };
            let watch = Watch { progress: false, every: import::POLL_EVERY };
            import::run(Lang::En, &client, &file, format.source(), false, None, maps, watch, &mut interrupts)
                .await
                .is_ok_and(|finished| finished.ok)
        })
        .await;
    }
}
