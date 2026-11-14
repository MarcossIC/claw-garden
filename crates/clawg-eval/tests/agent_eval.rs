//! Runs `agent-eval` over the example tasks with their recorded streams: the
//! dry run that lays a run out, starts the daemon of its home, seeds it,
//! replays a session, stops the daemon and scores the session, with the fake
//! model and no client. Every run is written under a temporary directory.

use std::ffi::OsStr;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

use serde_json::Value;

fn data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data").join("agent-tasks")
}

/// How many rules of other topics every arm with memory is seeded with.
fn noise() -> usize {
    let text = std::fs::read_to_string(data().join("noise.json")).unwrap();
    serde_json::from_str::<Vec<Value>>(&text).unwrap().len()
}

/// What `clawg import` prints for a seed of `n` memories.
fn imported(n: usize) -> String {
    format!("imported {n} of {n} memories")
}

/// The `clawg` that `cargo test --workspace` builds beside `agent-eval`: a
/// `testing` build, which reads `CLAWG_TEST_HOME` and `CLAWG_TEST_EMBEDDER`.
fn clawg() -> &'static Path {
    static CLAWG: OnceLock<PathBuf> = OnceLock::new();
    CLAWG.get_or_init(|| {
        let path = Path::new(env!("CARGO_BIN_EXE_agent-eval")).with_file_name(format!("clawg{}", std::env::consts::EXE_SUFFIX));
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("{}: {e}; these tests run the clawg that `cargo test --workspace` builds", path.display()));
        assert!(
            bytes.windows(15).any(|w| w == b"CLAWG_TEST_HOME"),
            "{} is not a `testing` build; `cargo test --workspace` builds one",
            path.display()
        );
        path
    })
}

/// `agent-eval`, with what keeps every clawg it starts on the fake model and
/// off the profile's port, and `env` on top.
fn agent_eval_with(args: &[&str], env: &[(&str, &OsStr)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agent-eval"))
        .args(args)
        .env("CLAWG_TEST_EMBEDDER", "fake")
        .env("CLAWG_TEST_DEFAULT_PORT", "off")
        .env("RUST_LOG", "info")
        .env_remove("CLAWG_PROFILE")
        .envs(env.iter().copied())
        .output()
        .expect("agent-eval must start")
}

fn agent_eval(args: &[&str]) -> Output {
    agent_eval_with(args, &[])
}

fn replay(dir: &Path, task: &str, arm: &str) -> (Output, PathBuf) {
    replay_of(dir, task, arm, &data().join("replays").join(format!("{task}.{arm}.jsonl")), clawg())
}

fn replay_of(dir: &Path, task: &str, arm: &str, stream: &Path, clawg: &Path) -> (Output, PathBuf) {
    let out = dir.join(format!("{task}.{arm}"));
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

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// What the clawg commands of a run printed, each after its command line.
fn clawg_log(run: &Path) -> String {
    std::fs::read_to_string(run.join("clawg.log")).unwrap_or_else(|e| panic!("{}: {e}", run.join("clawg.log").display()))
}

fn ok(output: &Output) {
    assert!(output.status.success(), "stdout: {}\nstderr: {}", stdout(output), stderr(output));
}

fn files_below(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_below(&path, out);
        } else {
            out.push(path);
        }
    }
}

// A replayed Edit names the exact text of a fixture, so a checkout that
// turned its line endings into CRLF would make the replay miss it.
#[test]
fn the_task_data_keeps_lf_line_endings_on_every_system() {
    let mut files = Vec::new();
    files_below(&data(), &mut files);
    let with_cr: Vec<_> = files.iter().filter(|p| std::fs::read(p).unwrap().contains(&b'\r')).collect();
    assert!(with_cr.is_empty(), "files with CR: {with_cr:?}");
}

#[test]
fn the_example_tasks_are_valid_and_leak_no_tracer() {
    let output = agent_eval(&["tasks"]);
    ok(&output);
    let listed = stdout(&output);
    assert!(listed.contains("example-staging-port K1 fixture=example-tally memories=1 tracers=1 checks=1 example"), "{listed}");
    assert!(listed.contains("example-count-lines N"), "{listed}");
    assert!(noise() > 0);
    assert!(listed.contains(&format!("noise rules={}", noise())), "{listed}");
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

    let data = run.join("home").join(".clawg-dev");
    assert!(data.join("memory.db").is_file(), "the daemon kept its store in the run's home");
    assert!(!data.join("daemon.json").exists(), "clawg stop ended the daemon");
    let log = clawg_log(&run);
    let ready = log.find("\"embedder\": \"ready\"").unwrap_or_else(|| panic!("the run did not wait for the model:\n{log}"));
    let imported = log.find(&imported(noise() + 1)).unwrap_or_else(|| panic!("the seed and the noise were not imported:\n{log}"));
    assert!(ready < imported, "the seed went before the model was ready:\n{log}");
    assert!(log[imported..].contains(" stop --profile dev"), "{log}");
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
    assert!(!run.join("serve.log").exists() && !run.join("clawg.log").exists(), "an arm without clawg starts none");
    assert!(!run.join("home").join(".clawg-dev").exists());
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
    let log = clawg_log(&run);
    assert!(log.contains(&imported(noise())), "a task without memories seeds the noise:\n{log}");
    assert!(log.contains("\"embedder\": \"ready\""), "the session started before the model was ready:\n{log}");
    assert!(log.contains(" stop --profile dev"), "{log}");
    assert!(!run.join("home").join(".clawg-dev").join("daemon.json").exists());

    let again = agent_eval(&["score", "--run", run.to_str().unwrap()]);
    ok(&again);
    assert_eq!(result(&run)["score"], r["score"]);
}

fn relay_replays() -> PathBuf {
    data().join("replays").join("example-relay-port.a3")
}

#[test]
fn a_replayed_run_of_two_sessions_shares_its_daemon_and_its_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    let (output, run) = replay_of(dir.path(), "example-relay-port", "a3", &relay_replays(), clawg());
    ok(&output);

    let log = clawg_log(&run);
    assert_eq!(log.matches(" import --from clawg").count(), 1, "the seed loads once:\n{log}");
    assert_eq!(log.matches(" stop --profile dev").count(), 1, "the daemon stops once:\n{log}");
    assert!(log.find(" import --from clawg").unwrap() < log.find(" stop --profile dev").unwrap(), "{log}");
    assert!(!run.join("home").join(".clawg-dev").join("daemon.json").exists(), "clawg stop ended the daemon");
    assert!(run.join("stream-1.jsonl").is_file() && run.join("stream-2.jsonl").is_file() && !run.join("stream.jsonl").exists());
    assert!(run.join("client-1.log").is_file() && run.join("client-2.log").is_file());
    assert_eq!(
        std::fs::read_to_string(run.join("work").join("notes.md")).unwrap(),
        "staging port: 48213\nconfirmed in a later session\n",
        "the second session edited the file the first one wrote"
    );
    let r = result(&run);
    assert_eq!(r["score"]["success"], true, "{r:#}");
    assert_eq!(r["session"]["session_id"], "replay-relay-port-2", "the result is the last session's");
    assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(run.join("run.json")).unwrap()).unwrap()["shape"], "split");
}

#[test]
fn a_session_that_fails_stops_the_run_and_still_stops_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let replays = dir.path().join("replays");
    std::fs::create_dir_all(&replays).unwrap();
    for name in ["session-1.jsonl", "session-2.jsonl"] {
        let text = std::fs::read_to_string(relay_replays().join(name)).unwrap();
        std::fs::write(replays.join(name), text.replace("\"is_error\":false", "\"is_error\":true")).unwrap();
    }

    let (output, run) = replay_of(dir.path(), "example-relay-port", "a3", &replays, clawg());

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("session 1") && stderr(&output).contains("client-1.log"), "{}", stderr(&output));
    assert!(run.join("stream-1.jsonl").is_file() && !run.join("stream-2.jsonl").exists(), "the second session never started");
    assert_eq!(std::fs::read_to_string(run.join("work").join("notes.md")).unwrap(), "staging port: 48213\n");
    let log = clawg_log(&run);
    assert!(log.contains(&imported(noise())) && log.contains(" stop --profile dev"), "{log}");
    assert!(!run.join("home").join(".clawg-dev").join("daemon.json").exists(), "clawg stop ended the daemon");
}

#[test]
fn the_harness_hooks_print_the_context_and_log_their_input() {
    let dir = tempfile::tempdir().unwrap();
    let rules = dir.path().join("startup-rules.md");
    std::fs::write(&rules, "Rules\n- [3 days ago] Use f-strings\n").unwrap();
    let output = agent_eval(&["hook-context", rules.to_str().unwrap()]);
    ok(&output);
    let printed: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(printed["hookSpecificOutput"]["hookEventName"], "SessionStart");
    assert_eq!(printed["hookSpecificOutput"]["additionalContext"], "Rules\n- [3 days ago] Use f-strings\n");
    assert!(!agent_eval(&["hook-context", dir.path().join("missing.md").to_str().unwrap()]).status.success());

    let log = dir.path().join("instructions.jsonl");
    for input in ["{\n  \"file_path\": \"/w/.claude/rules/clawg/rules-1.md\",\n  \"load_reason\": \"path_glob_match\"\n}\n", "not json"] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_agent-eval")).args(["hook-log", log.to_str().unwrap()]).stdin(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        assert!(child.wait().unwrap().success());
    }
    let lines: Vec<Value> = std::fs::read_to_string(&log).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["load_reason"], "path_glob_match");
    assert_eq!(lines[1]["unparsed"], "not json");
}

#[test]
fn a_prepared_l2_run_writes_the_rules_of_its_task_by_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("run");
    let task = "madr-0011-asterisk-list-marker";
    ok(&agent_eval(&["prepare", "--task", task, "--arm", "l2", "--out", out.to_str().unwrap(), "--clawg", clawg().to_str().unwrap(), "--model", "m"]));

    let mut by_file = Vec::new();
    files_below(&out.join("work").join(".claude").join("rules").join("clawg"), &mut by_file);
    let markdown: Vec<String> = by_file.iter().map(|p| std::fs::read_to_string(p).unwrap()).filter(|t| t.contains("\"**/*.md\"")).collect();
    assert_eq!(markdown.len(), 1, "{by_file:?}");
    assert!(markdown[0].contains("asterisk") && markdown[0].contains("Markdown headings are in sentence case"), "{}", markdown[0]);
    let startup = std::fs::read_to_string(out.join("startup-rules.md")).unwrap();
    assert!(startup.contains("Commit messages follow conventional commits") && !startup.contains("asterisk"), "{startup}");

    let settings: Value = serde_json::from_str(&std::fs::read_to_string(out.join("claude-config").join("settings.json")).unwrap()).unwrap();
    let hook = &settings["hooks"]["SessionStart"][1]["hooks"][0];
    let command = Path::new(hook["command"].as_str().unwrap());
    assert_eq!(command.file_stem().and_then(OsStr::to_str), Some("agent-eval"), "{hook}");
    assert_eq!(hook["args"][0], "hook-context");
}

#[test]
fn a_run_never_writes_over_another() {
    let dir = tempfile::tempdir().unwrap();
    ok(&replay(dir.path(), "example-staging-port", "a0").0);
    let (output, _) = replay(dir.path(), "example-staging-port", "a0");
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("never writes over another"));
}

#[test]
fn a_client_that_fails_still_stops_the_daemon_of_its_run() {
    let dir = tempfile::tempdir().unwrap();
    let stream = dir.path().join("escape.jsonl");
    let write = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"1","name":"Write","input":{"file_path":"../escaped.txt","content":"x"}}]}}"#;
    std::fs::write(&stream, format!("{write}\n")).unwrap();

    let (output, run) = replay_of(dir.path(), "example-staging-port", "a3", &stream, clawg());

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("client.log"), "{}", stderr(&output));
    assert!(!run.join("escaped.txt").exists());
    let log = clawg_log(&run);
    assert!(log.contains(&imported(noise() + 1)) && log.contains(" stop --profile dev"), "{log}");
    assert!(!run.join("home").join(".clawg-dev").join("daemon.json").exists(), "clawg stop ended the daemon");
    assert!(!run.join("result.json").exists());
}

#[test]
fn a_daemon_that_never_listens_starts_no_client() {
    let dir = tempfile::tempdir().unwrap();
    // agent-eval has no `serve` and exits at once, as a clawg that cannot
    // open its store would. It names CLAWG_TEST_HOME itself, so on Windows it
    // passes for a `testing` build.
    let not_clawg = Path::new(env!("CARGO_BIN_EXE_agent-eval"));
    let stream = data().join("replays").join("example-staging-port.a3.jsonl");

    let (output, run) = replay_of(dir.path(), "example-staging-port", "a3", &stream, not_clawg);

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("before it listened") && stderr(&output).contains("serve.log"), "{}", stderr(&output));
    assert!(std::fs::read_to_string(run.join("serve.log")).unwrap().contains("unknown command serve"));
    assert!(!run.join("clawg.log").exists() && !run.join("stream.jsonl").exists());
}

#[test]
fn a_run_with_claude_refuses_another_version_before_laying_out_the_run() {
    let dir = tempfile::tempdir().unwrap();
    // agent-eval knows no `--version`, so as `claude` it reports none.
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_agent-eval"), bin.join(format!("claude{}", std::env::consts::EXE_SUFFIX))).unwrap();
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()))).unwrap();
    let out = dir.path().join("run");

    let output = agent_eval_with(
        &[
            "run",
            "--task",
            "example-staging-port",
            "--arm",
            "a3",
            "--out",
            out.to_str().unwrap(),
            "--clawg",
            clawg().to_str().unwrap(),
            "--model",
            "some-model",
            "--max-budget-usd",
            "0.5",
            "--claude-version",
            "2.1.300",
        ],
        &[("PATH", &path)],
    );

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("claude 2.1.300") && stderr(&output).contains("no version"), "{}", stderr(&output));
    assert!(!out.exists(), "nothing of the run was laid out");
}
