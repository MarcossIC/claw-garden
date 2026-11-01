//! One run: its directory laid out from the plan, the client started in its
//! working directory with its stream captured, and the result scored from
//! what it left. [`evaluate`] is pure; the rest is the I/O around it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clawg_core::paths::Profile;
use clawg_core::preferences::Lang;
use serde::Serialize;
use serde_json::{json, Value};

use crate::arm::{Arm, Layout, Options, RunPlan, Step};
use crate::rubric::{self, Mark, SaveMarks};
use crate::score::{self, Score, TracerFate};
use crate::stream::{self, Outcome, Transcript};
use crate::task::{self, Task};

pub const RUN_FILE: &str = "run.json";
pub const STREAM_FILE: &str = "stream.jsonl";
pub const CLIENT_LOG: &str = "client.log";
pub const RESULT_FILE: &str = "result.json";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Session {
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub client_version: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunResult {
    pub task: String,
    pub category: String,
    pub example: bool,
    pub arm: String,
    /// `replay` for a recorded stream; a real client would say `claude`.
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
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> RunError + '_ {
    move |source| RunError::Io { path: path.to_owned(), source }
}

/// Lays the run out in an empty or missing `layout.root`: the fixture copied
/// as the working directory, every planned file and `run.json`.
pub fn prepare(task: &Task, arm: Arm, fixture: &Path, layout: &Layout, options: &Options) -> Result<RunPlan, RunError> {
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
pub fn score_dir(root: &Path, client: &str) -> Result<RunResult, RunError> {
    let path = root.join(RUN_FILE);
    let text = std::fs::read_to_string(&path).map_err(io(&path))?;
    let not_a_run = |why: &str| RunError::NotARun(root.to_owned(), why.to_owned());
    let record: Value = serde_json::from_str(&text).map_err(|e| not_a_run(&e.to_string()))?;
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
    let result = evaluate(&task, &arm, client, profile, lang, &stream::parse(&stream), read_file);
    let out = root.join(RESULT_FILE);
    std::fs::write(&out, serde_json::to_string_pretty(&result).expect("JSON serializes") + "\n").map_err(io(&out))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        Options { clawg, profile: Profile::Dev, lang: None, model: "m".into(), effort: None, max_budget_usd: None, now_ms: 0 }
    }

    #[test]
    fn prepare_lays_out_the_run_and_never_writes_over_another() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("fixture");
        std::fs::create_dir_all(fixture.join("src")).unwrap();
        std::fs::write(fixture.join("src/app.py"), "print(1)\n").unwrap();
        let layout = Layout { root: dir.path().join("run") };
        let opts = options(dir.path().join("clawg"));

        let plan = prepare(&task(), Arm::A3, &fixture, &layout, &opts).unwrap();

        assert_eq!(std::fs::read_to_string(layout.workdir().join("src/app.py")).unwrap(), "print(1)\n");
        for (path, body) in &plan.files {
            assert_eq!(&std::fs::read_to_string(path).unwrap(), body);
        }
        assert!(layout.config_dir().join("settings.json").is_file());
        let record: Value = serde_json::from_str(&std::fs::read_to_string(layout.root.join(RUN_FILE)).unwrap()).unwrap();
        assert_eq!(record["arm"], "a3");
        assert_eq!(record["task"]["id"], "t");
        assert!(matches!(prepare(&task(), Arm::A3, &fixture, &layout, &opts), Err(RunError::NotEmpty(_))));
    }

    #[test]
    fn score_dir_reads_the_run_back_and_writes_its_result() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("fixture");
        std::fs::create_dir_all(&fixture).unwrap();
        std::fs::write(fixture.join("a.txt"), "ok").unwrap();
        let layout = Layout { root: dir.path().join("run") };
        prepare(&task(), Arm::A0, &fixture, &layout, &options(dir.path().join("clawg"))).unwrap();
        std::fs::write(layout.root.join(STREAM_FILE), STREAM).unwrap();

        let r = score_dir(&layout.root, "replay").unwrap();

        assert!(r.score.success);
        assert_eq!(r.arm, "a0");
        let written: Value = serde_json::from_str(&std::fs::read_to_string(layout.root.join(RESULT_FILE)).unwrap()).unwrap();
        assert_eq!(written["score"]["success"], true);
        assert!(matches!(score_dir(dir.path(), "replay"), Err(RunError::Io { .. })));
    }
}
