//! Runs `agent-eval` over the example tasks with their recorded streams: the
//! dry run that lays a run out, replays a session and scores it, with no
//! model and no daemon. Every run is written under a temporary directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data").join("agent-tasks")
}

fn agent_eval(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agent-eval")).args(args).output().expect("agent-eval must start")
}

fn replay(dir: &Path, task: &str, arm: &str) -> (Output, PathBuf) {
    let out = dir.join(format!("{task}.{arm}"));
    let stream = data().join("replays").join(format!("{task}.{arm}.jsonl"));
    let clawg = dir.join("bin").join("clawg");
    let output = agent_eval(&[
        "run",
        "--task",
        task,
        "--arm",
        arm,
        "--out",
        out.to_str().unwrap(),
        "--clawg",
        clawg.to_str().unwrap(),
        "--model",
        "replay",
        "--replay",
        stream.to_str().unwrap(),
    ]);
    (output, out)
}

fn result(run: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(run.join("result.json")).unwrap()).unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn ok(output: &Output) {
    assert!(output.status.success(), "stdout: {}\nstderr: {}", stdout(output), String::from_utf8_lossy(&output.stderr));
}

#[test]
fn the_example_tasks_are_valid_and_leak_no_tracer() {
    let output = agent_eval(&["tasks"]);
    ok(&output);
    let listed = stdout(&output);
    assert!(listed.contains("example-staging-port K1 fixture=example-tally memories=1 tracers=1 checks=1 example"), "{listed}");
    assert!(listed.contains("example-count-lines N"), "{listed}");
}

#[test]
fn a_replayed_session_with_clawg_finds_the_fact_and_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let (output, run) = replay(dir.path(), "example-staging-port", "a3");
    ok(&output);
    assert!(stdout(&output).starts_with("example-staging-port a3: success"), "{}", stdout(&output));

    let r = result(&run);
    assert_eq!(r["client"], "replay");
    assert_eq!(r["session"]["clawg_calls"]["search_memory"], 1);
    assert_eq!(r["session"]["clawg_calls_after_tool_search"], 1);
    assert_eq!(r["session"]["hook_events"], 1);
    assert_eq!(r["score"]["tracers"][0], serde_json::json!(["48213", "used"]));
    assert!(std::fs::read_to_string(run.join("seed.jsonl")).unwrap().contains("48213"));
    assert!(run.join("claude-config").join("settings.json").is_file());
    let plan: Value = serde_json::from_str(&std::fs::read_to_string(run.join("run.json")).unwrap()).unwrap();
    assert_eq!(plan["plan"]["client"]["program"], "claude");
}

#[test]
fn a_replayed_session_without_memory_never_reaches_the_fact() {
    let dir = tempfile::tempdir().unwrap();
    let (output, run) = replay(dir.path(), "example-staging-port", "a0");
    ok(&output);
    let r = result(&run);
    assert_eq!(r["score"]["success"], false);
    assert_eq!(r["score"]["tracers"][0], serde_json::json!(["48213", "never_reached"]));
    assert!(!run.join("seed.jsonl").exists());
}

#[test]
fn a_replayed_edit_changes_the_working_directory_the_checks_read() {
    let dir = tempfile::tempdir().unwrap();
    let (output, run) = replay(dir.path(), "example-count-lines", "a3");
    ok(&output);
    let r = result(&run);
    assert_eq!(r["score"]["success"], true, "{r:#}");
    assert!(std::fs::read_to_string(run.join("work").join("tally.py")).unwrap().contains("def count_lines(path):"));
    assert_eq!(r["saves"].as_array().unwrap().len(), 1);
    assert!(r["saves"][0]["marks"].as_array().unwrap().iter().all(|m| m[1] != "fail"), "{:#}", r["saves"][0]);

    let again = agent_eval(&["score", "--run", run.to_str().unwrap()]);
    ok(&again);
    assert_eq!(result(&run)["score"], r["score"]);
}

#[test]
fn a_run_never_writes_over_another() {
    let dir = tempfile::tempdir().unwrap();
    ok(&replay(dir.path(), "example-staging-port", "a0").0);
    let (output, _) = replay(dir.path(), "example-staging-port", "a0");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("never writes over another"));
}
