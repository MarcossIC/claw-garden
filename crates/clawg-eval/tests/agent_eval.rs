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
    // A batch file keeps the line endings of its source: .gitattributes leaves it as it is.
    let with_cr: Vec<_> = files.iter().filter(|p| p.extension().is_none_or(|e| e != "bat")).filter(|p| std::fs::read(p).unwrap().contains(&b'\r')).collect();
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
    assert!(listed.contains("noise-300 rules=300"), "every noise is listed with its rules: {listed}");
}

#[test]
fn the_large_noise_is_300_distinct_rules_older_than_a_week() {
    let text = std::fs::read_to_string(data().join("noise-300.json")).unwrap();
    let rules: Vec<Value> = serde_json::from_str(&text).unwrap();
    assert_eq!(rules.len(), 300);
    let titles: std::collections::BTreeSet<&str> = rules.iter().map(|r| r["content"].as_str().unwrap().lines().next().unwrap()).collect();
    assert_eq!(titles.len(), 300, "every rule has a title of its own");
    assert!(rules.iter().all(|r| r["age_days"].as_u64().unwrap() > 7), "older than 7 days, outside the recent context of the startup");
    assert!(rules.iter().all(|r| r.get("rule").is_some()), "every one is a rule");
    let always = rules.iter().filter(|r| r["rule"].get("paths").is_none()).count();
    assert!(always >= 100, "enough rules without globs for the startup of l2 to be too long: {always}");
}

#[test]
fn a_run_names_its_noise_and_l1_refuses_the_large_one() {
    let dir = tempfile::tempdir().unwrap();
    let prepare = |arm: &str, out: &str, noise: &[&str]| {
        let out = dir.path().join(out);
        let mut args = vec!["prepare", "--task", "example-count-lines", "--arm", arm, "--out", out.to_str().unwrap(), "--clawg", clawg().to_str().unwrap(), "--model", "m"];
        args.extend(noise);
        (agent_eval(&args), out)
    };

    let (output, small) = prepare("a3", "small", &[]);
    ok(&output);
    let record: Value = serde_json::from_str(&std::fs::read_to_string(small.join("run.json")).unwrap()).unwrap();
    assert_eq!((record["noise"].as_str(), record["noise_rules"].as_u64()), (Some("noise"), Some(noise() as u64)));

    let (output, large) = prepare("a3", "large", &["--noise", "noise-300"]);
    ok(&output);
    let record: Value = serde_json::from_str(&std::fs::read_to_string(large.join("run.json")).unwrap()).unwrap();
    assert_eq!((record["noise"].as_str(), record["noise_rules"].as_u64()), (Some("noise-300"), Some(300)));
    assert_eq!(std::fs::read_to_string(large.join("seed.jsonl")).unwrap().lines().count(), 2 + 300, "the header, the project and the rules");

    let (output, refused) = prepare("l1", "l1", &["--noise", "noise-300"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("10000") && stderr(&output).contains("characters"), "{}", stderr(&output));
    assert!(!refused.exists(), "nothing was laid out");

    let (output, _) = prepare("a3", "missing", &["--noise", "noise-none"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("noise-none.json"), "{}", stderr(&output));
    let (output, _) = prepare("a3", "bad", &["--noise", "tasks"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
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
    assert_eq!(r["sessions"].as_array().unwrap().len(), 2, "and each session has its own record and score");
    assert_eq!(r["sessions"][0]["session"]["session_id"], "replay-relay-port-1");
    assert_eq!(r["shape"], "split");
    assert!(stdout(&output).contains("saves 1 with"), "the summary counts what the first session saved: {}", stdout(&output));
    assert!(stdout(&output).contains("2 sessions split"), "{}", stdout(&output));
    assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(run.join("run.json")).unwrap()).unwrap()["shape"], "split");
}

#[test]
fn the_second_replayed_session_finds_what_the_first_one_saved() {
    let dir = tempfile::tempdir().unwrap();
    let (output, run) = replay_of(dir.path(), "example-relay-port", "a3", &relay_replays(), clawg());
    ok(&output);
    let first_result = |run: &Path| -> String {
        let stream = std::fs::read_to_string(run.join("stream-2.jsonl")).unwrap();
        let line = stream.lines().find(|l| l.contains("\"toolu_1\"") && l.contains("tool_result")).expect("the result of its first call");
        serde_json::from_str::<Value>(line).unwrap()["message"]["content"][0]["content"].as_str().unwrap().to_owned()
    };
    assert!(first_result(&run).contains("48213"), "the first call of session 2 listed what session 1 saved: {}", first_result(&run));
    assert!(!first_result(&run).contains("nothing found"), "the recorded result was replaced");

    let (output, bare) = replay_of(dir.path(), "example-relay-port", "a0", &relay_replays(), clawg());
    ok(&output);
    assert_eq!(first_result(&bare), "nothing found", "an arm without clawg has no server: the stream is as recorded");
}

/// The example relay played `--sessions both` under `arm`: its first session
/// once, and its second one continued and then new.
fn fork(dir: &Path, arm: &str) -> (Output, PathBuf) {
    let out = dir.join(format!("fork.{arm}"));
    let streams = data().join("replays").join(format!("example-relay-port.{arm}.both"));
    let output = agent_eval(&[
        "run",
        "--task",
        "example-relay-port",
        "--arm",
        arm,
        "--out",
        out.to_str().unwrap(),
        "--clawg",
        clawg().to_str().unwrap(),
        "--model",
        "replay",
        "--sessions",
        "both",
        "--replay",
        streams.to_str().unwrap(),
    ]);
    (output, out)
}

/// What a replayed call of a branch's second session got back, by its id.
fn result_of(run: &Path, branch: &str, id: &str) -> String {
    let stream = std::fs::read_to_string(run.join(branch).join("stream-2.jsonl")).unwrap();
    let line = stream.lines().find(|l| l.contains(&format!("\"{id}\"")) && l.contains("tool_result")).expect("the result of the call");
    serde_json::from_str::<Value>(line).unwrap()["message"]["content"][0]["content"].as_str().unwrap().to_owned()
}

#[test]
fn a_forked_run_plays_the_second_session_both_ways_from_one_first_session() {
    let dir = tempfile::tempdir().unwrap();
    let (output, run) = fork(dir.path(), "a3");
    ok(&output);

    assert!(!run.join("result.json").exists(), "the run itself has no result: its branches do");
    assert!(run.join("stream-1.jsonl").is_file() && !run.join("stream-2.jsonl").exists(), "the first session is the run's, once");
    for branch in ["joined", "split"] {
        assert!(run.join(branch).join("stream-2.jsonl").is_file(), "{branch}");
        assert!(run.join(branch).join("client-2.log").is_file(), "{branch}");
        assert!(run.join(branch).join("work").join("notes.md").is_file(), "{branch}");
    }
    assert!(run.join("fork").join("work").join("notes.md").is_file(), "the state after the first session stays in fork/");

    let log = clawg_log(&run);
    assert_eq!(log.matches(" import --from clawg").count(), 1, "the seed loads once:\n{log}");
    assert_eq!(log.matches(" stop --profile dev").count(), 3, "the daemon stops after each part:\n{log}");
    assert!(!run.join("home").join(".clawg-dev").join("daemon.json").exists());

    let (joined, split) = (result(&run.join("joined")), result(&run.join("split")));
    assert_eq!((&joined["shape"], &split["shape"]), (&Value::from("joined"), &Value::from("split")));
    assert_eq!((&joined["paired"], &split["paired"]), (&Value::from("split"), &Value::from("joined")));
    for r in [&joined, &split] {
        assert_eq!(r["sessions"].as_array().unwrap().len(), 2);
        assert_eq!(r["sessions"][0]["session"]["session_id"], "replay-relay-port-1", "both share the first session");
        assert_eq!(r["score"]["success"], true, "{r:#}");
    }
    assert_eq!(joined["session"]["session_id"], "replay-relay-port-joined-2");
    assert_eq!(split["session"]["session_id"], "replay-relay-port-split-2");

    let scored = agent_eval(&["score", "--run", run.to_str().unwrap()]);
    ok(&scored);
    let printed = stdout(&scored);
    assert!(printed.contains("2 sessions joined") && printed.contains("2 sessions split"), "score --run scores both branches: {printed}");
    assert!(stdout(&output).contains("2 sessions joined") && stdout(&output).contains("2 sessions split"), "{}", stdout(&output));
}

#[test]
fn a_branch_of_a_fork_does_not_see_what_the_other_did() {
    let dir = tempfile::tempdir().unwrap();
    let (output, run) = fork(dir.path(), "a3");
    ok(&output);

    let (joined, split) = (run.join("joined").join("work"), run.join("split").join("work"));
    assert!(joined.join("only-joined.txt").is_file(), "the continued session wrote it");
    assert!(!split.join("only-joined.txt").exists(), "the new session starts from the work as the first one left it");
    assert_eq!(
        std::fs::read_to_string(joined.join("notes.md")).unwrap(),
        std::fs::read_to_string(split.join("notes.md")).unwrap(),
        "and both edit the file the first session wrote"
    );

    assert!(result_of(&run, "joined", "toolu_5").contains("7719"), "the continued session finds what it saved: {}", result_of(&run, "joined", "toolu_5"));
    assert!(!result_of(&run, "split", "toolu_2").contains("7719"), "the new session never meets it: {}", result_of(&run, "split", "toolu_2"));
    assert!(result_of(&run, "split", "toolu_1").contains("48213"), "but it finds what the first session saved: {}", result_of(&run, "split", "toolu_1"));
}

#[test]
fn the_tasks_of_several_sessions_are_listed_with_their_sessions() {
    let output = agent_eval(&["tasks"]);
    ok(&output);
    let listed = stdout(&output);
    for line in [
        "requests-timeout-decision-relay R fixture=psf-requests memories=0 tracers=2 checks=5 sessions=2",
        "requests-redact-decision-revised D fixture=psf-requests memories=0 tracers=2 checks=4 sessions=3",
        "requests-history-owner-convention K2 fixture=psf-requests memories=1 tracers=1 checks=2",
        "shelf-eza-missing E fixture=shelf-notes memories=0 tracers=0 checks=3 sessions=3",
        "example-relay-port R fixture=example-tally memories=0 tracers=1 checks=3 sessions=2 example",
        "example-missing-command E fixture=example-tally memories=0 tracers=0 checks=3 sessions=2 example",
    ] {
        assert!(listed.lines().any(|l| l == line), "{line} is not listed in:
{listed}");
    }
}

#[test]
fn the_replayed_relay_example_follows_its_fact_to_the_second_session() {
    let dir = tempfile::tempdir().unwrap();
    let (output, run) = replay_of(dir.path(), "example-relay-port", "a3", &relay_replays(), clawg());
    ok(&output);
    let r = result(&run);
    assert_eq!(r["facts"], serde_json::json!([{"tracer": "48213", "born": 1, "class": "used"}]), "{r:#}");
    assert_eq!(r["sessions"][0]["saved"], serde_json::json!(["48213"]), "the first session wrote it to a memory");
    let followed = &r["sessions"][1]["followed"][0];
    assert_eq!((&followed["tracer"], &followed["fate"]), (&Value::from("48213"), &Value::from("used")));
    assert!(followed["brought_by"].as_array().unwrap().iter().any(|t| t == "search_memory"), "the search of the second session brought it: {followed}");
    assert!(stdout(&output).contains("facts not_saved 0 saved_not_reached 0 reached_not_used 0 used 1"), "{}", stdout(&output));
}

#[test]
fn a_session_that_wrote_what_it_had_to_leave_untouched_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let (output, run) = replay_of(dir.path(), "example-relay-port", "a3", &relay_replays(), clawg());
    ok(&output);
    let held = serde_json::json!([{"path": "notes.md", "text": "confirmed", "present": false}]);
    let r = result(&run);
    assert_eq!(r["sessions"][0]["session"]["untouched"], held, "the stream of the example leaves notes.md as the prompt asks: {r:#}");
    assert_eq!(r["sessions"][1]["session"]["untouched"], serde_json::json!([]));
    let written: Value = serde_json::from_str(&std::fs::read_to_string(run.join("untouched-1.json")).unwrap()).unwrap();
    assert_eq!(written, held, "the entries are written when the session ends, before the next one begins");
    assert!(!run.join("untouched-2.json").exists(), "a session that names nothing leaves no file");

    let early = dir.path().join("early");
    std::fs::create_dir_all(&early).unwrap();
    std::fs::copy(relay_replays().join("session-2.jsonl"), early.join("session-2.jsonl")).unwrap();
    let first = std::fs::read_to_string(relay_replays().join("session-1.jsonl")).unwrap();
    std::fs::write(early.join("session-1.jsonl"), first.replace(r#""content":"staging port: 48213\n""#, r#""content":"staging port: 48213\nconfirmed\n""#)).unwrap();
    let other = tempfile::tempdir().unwrap();
    let (output, again) = replay_of(other.path(), "example-relay-port", "a3", &early, clawg());
    ok(&output);
    let present = serde_json::json!([{"path": "notes.md", "text": "confirmed", "present": true}]);
    assert_eq!(result(&again)["sessions"][0]["session"]["untouched"], present);
    let written: Value = serde_json::from_str(&std::fs::read_to_string(again.join("untouched-1.json")).unwrap()).unwrap();
    assert_eq!(written, present);
}

#[test]
fn the_tasks_that_hold_a_decision_back_name_what_stays_untouched() {
    let tasks = |id: &str| -> Value {
        let text = std::fs::read_to_string(data().join("tasks").join(format!("{id}.json"))).unwrap();
        serde_json::from_str(&text).unwrap()
    };
    let untouched = |id: &str, session: usize| -> Vec<(String, String)> {
        tasks(id)["sessions"][session]["untouched"]
            .as_array()
            .map(|a| a.iter().map(|e| (e["path"].as_str().unwrap().to_owned(), e["text"].as_str().unwrap().to_owned())).collect())
            .unwrap_or_default()
    };
    let pair = |path: &str, text: &str| (path.to_owned(), text.to_owned());
    assert_eq!(
        untouched("requests-timeout-decision-relay", 0),
        [pair("src/requests/adapters.py", "FORK_DEFAULT_TIMEOUT"), pair("HISTORY.md", "FORK_DEFAULT_TIMEOUT"), pair("docs/user/advanced.rst", "4.5, 41")]
    );
    assert!(untouched("requests-timeout-decision-relay", 1).is_empty());
    let redact = [pair("src/requests/utils.py", "def redact_url_credentials")];
    assert_eq!(untouched("requests-redact-decision-revised", 0), redact);
    assert_eq!(untouched("requests-redact-decision-revised", 1), redact);
    assert!(untouched("requests-redact-decision-revised", 2).is_empty(), "the last session is the one that applies it");
    assert_eq!(untouched("example-relay-port", 0), [pair("notes.md", "confirmed")]);
}

#[test]
fn the_replayed_missing_command_example_counts_its_failures() {
    let dir = tempfile::tempdir().unwrap();
    let streams = data().join("replays").join("example-missing-command.a0");
    let (output, run) = replay_of(dir.path(), "example-missing-command", "a0", &streams, clawg());
    ok(&output);
    let r = result(&run);
    assert_eq!(r["score"]["success"], true, "the failure of the first session is not charged to the check of the second: {r:#}");
    assert_eq!(r["sessions"].as_array().unwrap().len(), 2);
    let runs = r["score"]["checks"].as_array().unwrap().iter().find(|c| c["check"] == "session 2: eza was run at most 0 times");
    assert_eq!(runs.map(|c| &c["passed"]), Some(&serde_json::json!(true)), "the first session tried eza and the second did not: {r:#}");

    let repeated = dir.path().join("repeated");
    std::fs::create_dir_all(&repeated).unwrap();
    std::fs::copy(streams.join("session-1.jsonl"), repeated.join("session-1.jsonl")).unwrap();
    let second = std::fs::read_to_string(streams.join("session-2.jsonl")).unwrap();
    std::fs::write(
        repeated.join("session-2.jsonl"),
        second.replace("ls -p | grep -v /", "eza -f").replace("README.md\\ntally.py","bash: line 1: eza: command not found"),
    )
    .unwrap();
    let other = tempfile::tempdir().unwrap();
    let (output, again) = replay_of(other.path(), "example-missing-command", "a0", &repeated, clawg());
    ok(&output);
    let r = result(&again);
    assert_eq!(r["score"]["success"], false, "{r:#}");
    let failed: Vec<&str> = r["score"]["checks"].as_array().unwrap().iter().filter(|c| c["passed"] == false).map(|c| c["check"].as_str().unwrap()).collect();
    assert_eq!(failed, ["session 2: Bash failed at most 0 times with \"eza: command not found\"", "session 2: eza was run at most 0 times"]);
    assert!(stdout(&output).contains("failure"), "{}", stdout(&output));
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
    let r = result(&run);
    assert_eq!(r["sessions"].as_array().unwrap().len(), 1, "the sessions that ran are scored");
    assert_eq!((&r["score"]["ended"], &r["score"]["success"]), (&Value::Bool(false), &Value::Bool(false)));
    assert!(stdout(&output).contains("failure"), "the summary of what ran is still printed: {}", stdout(&output));
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
