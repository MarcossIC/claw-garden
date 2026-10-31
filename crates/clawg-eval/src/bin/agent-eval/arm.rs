//! An arm: what one session of a task gets besides the prompt. [`plan`]
//! decides every file, argument and variable of a run without touching disk;
//! the hooks and the protocol block come from `clawg setup`'s own plan, so an
//! arm with them is what `setup` installs.

use std::path::{Path, PathBuf};

use clawg_cli::setup::{self, FileStep, Mode, State};
use clawg_core::paths::Profile;
use clawg_core::preferences::{Lang, Preferences};
use serde::Serialize;
use serde_json::json;

use crate::seed;
use crate::task::Task;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    /// No memory at all.
    A0,
    /// Only clawg's MCP server, seeded: no hooks, no protocol in `CLAUDE.md`.
    A2,
    /// What `clawg setup claude-code` installs, seeded.
    A3,
    /// A3 with the client's tool search off, so clawg's tools load up front.
    A3Eager,
    /// The client's native memory holding the same facts, without clawg.
    A4,
    /// A3 without the tools that read memories back: only what reaches the
    /// session by itself.
    B,
}

pub const ARMS: [Arm; 6] = [Arm::A0, Arm::A2, Arm::A3, Arm::A3Eager, Arm::A4, Arm::B];

/// The tools arm B takes away.
const READ_TOOLS: [&str; 3] = ["search_memory", "read_memory", "list_memories"];

impl Arm {
    pub fn parse(name: &str) -> Option<Arm> {
        ARMS.into_iter().find(|a| a.as_str() == name)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Arm::A0 => "a0",
            Arm::A2 => "a2",
            Arm::A3 => "a3",
            Arm::A3Eager => "a3-eager",
            Arm::A4 => "a4",
            Arm::B => "b",
        }
    }

    fn has_clawg(self) -> bool {
        matches!(self, Arm::A2 | Arm::A3 | Arm::A3Eager | Arm::B)
    }

    fn has_setup(self) -> bool {
        matches!(self, Arm::A3 | Arm::A3Eager | Arm::B)
    }
}

/// Where a run keeps each thing, all below its directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn home(&self) -> PathBuf {
        self.root.join("home")
    }
    pub fn config_dir(&self) -> PathBuf {
        self.root.join("claude-config")
    }
    pub fn workdir(&self) -> PathBuf {
        self.root.join("work")
    }
    pub fn mcp_config(&self) -> PathBuf {
        self.root.join("mcp.json")
    }
    pub fn seed(&self) -> PathBuf {
        self.root.join("seed.jsonl")
    }
    pub fn data_dir(&self, profile: Profile) -> PathBuf {
        self.home().join(profile.dir_name())
    }
}

/// What a run is asked for, besides the task and the arm.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// The `clawg` binary the server and the hooks run; absolute.
    pub clawg: PathBuf,
    pub profile: Profile,
    /// The language preference; `None` leaves `preferences.json` out.
    pub lang: Option<Lang>,
    pub model: String,
    pub effort: Option<String>,
    pub max_budget_usd: Option<f64>,
    pub now_ms: i64,
}

/// One program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    pub program: String,
    pub args: Vec<String>,
}

/// Everything a run does, decided before anything is written.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunPlan {
    pub task: String,
    pub arm: &'static str,
    /// Files written before the session, by absolute path.
    pub files: Vec<(PathBuf, String)>,
    /// Commands run before the session, against a daemon of the run's home.
    pub before: Vec<Step>,
    /// The client session, run in [`Layout::workdir`].
    pub client: Step,
    pub env: Vec<(String, String)>,
    /// Commands run after the session.
    pub after: Vec<Step>,
}

#[derive(Debug, thiserror::Error)]
pub enum ArmError {
    #[error("{0}")]
    Setup(#[from] setup::SetupError),
    #[error("a path of the run is not valid Unicode: {0}")]
    NonUnicode(PathBuf),
}

fn text(path: &Path) -> Result<String, ArmError> {
    path.to_str().map(str::to_owned).ok_or_else(|| ArmError::NonUnicode(path.to_owned()))
}

pub fn plan(task: &Task, arm: Arm, layout: &Layout, options: &Options) -> Result<RunPlan, ArmError> {
    let clawg = text(&options.clawg)?;
    let workdir = text(&layout.workdir())?;
    let server_name = options.profile.mcp_server_name();
    let mut files = Vec::new();
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut servers = serde_json::Map::new();

    if arm.has_clawg() {
        let state = State {
            exe: options.clawg.clone(),
            profile: options.profile,
            servers: setup::File { path: layout.config_dir().join(".claude.json"), text: None },
            settings: setup::File { path: layout.config_dir().join("settings.json"), text: None },
            instructions: setup::File { path: layout.config_dir().join("CLAUDE.md"), text: None },
            language: options.lang,
        };
        let setup_plan = setup::plan(&state, Mode::Install)?;
        servers.insert(
            setup_plan.server.name.to_owned(),
            json!({ "command": setup_plan.server.command, "args": setup_plan.server.args }),
        );
        if arm.has_setup() {
            for (step, file) in [(setup_plan.settings, &state.settings), (setup_plan.instructions, &state.instructions)] {
                if let FileStep::Write(body) = step {
                    files.push((file.path.clone(), body));
                }
            }
        }
        if let Some(lang) = options.lang {
            let data = layout.data_dir(options.profile);
            files.push((clawg_core::paths::preferences_file(&data), Preferences { language: Some(lang) }.to_json()));
        }
        if !task.memories.is_empty() {
            files.push((layout.seed(), seed::export(&task.memories, setup::AGENT, &workdir, &task.fixture, options.now_ms)));
            before.push(Step {
                program: clawg.clone(),
                args: ["import", "--from", "clawg", &text(&layout.seed())?, "--profile", options.profile.as_str()]
                    .map(String::from)
                    .to_vec(),
            });
        }
        after.push(Step { program: clawg, args: ["stop", "--profile", options.profile.as_str()].map(String::from).to_vec() });
    }
    if arm == Arm::A4 && !task.memories.is_empty() {
        let dir = layout.config_dir().join("projects").join(seed::project_slug(&workdir)).join("memory");
        files.push((dir.join("MEMORY.md"), seed::native_memory(&task.memories)));
    }
    files.push((layout.mcp_config(), serde_json::to_string_pretty(&json!({ "mcpServers": servers })).expect("JSON serializes") + "\n"));

    let mut args: Vec<String> = vec![
        "-p".into(),
        task.prompt.clone(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--include-hook-events".into(),
        "--strict-mcp-config".into(),
        "--mcp-config".into(),
        text(&layout.mcp_config())?,
        "--model".into(),
        options.model.clone(),
    ];
    if let Some(effort) = &options.effort {
        args.extend(["--effort".into(), effort.clone()]);
    }
    if let Some(budget) = options.max_budget_usd {
        args.extend(["--max-budget-usd".into(), budget.to_string()]);
    }
    if arm == Arm::B {
        let tools: Vec<String> = READ_TOOLS.iter().map(|t| format!("mcp__{server_name}__{t}")).collect();
        args.extend(["--disallowedTools".into(), tools.join(",")]);
    }

    let home = text(&layout.home())?;
    let mut env = vec![
        ("HOME".to_owned(), home.clone()),
        ("USERPROFILE".to_owned(), home),
        ("CLAUDE_CONFIG_DIR".to_owned(), text(&layout.config_dir())?),
        ("CLAUDE_CODE_DISABLE_AUTO_MEMORY".to_owned(), if arm == Arm::A4 { "0" } else { "1" }.to_owned()),
    ];
    if arm == Arm::A3Eager {
        env.push(("ENABLE_TOOL_SEARCH".to_owned(), "false".to_owned()));
    }

    Ok(RunPlan {
        task: task.id.clone(),
        arm: arm.as_str(),
        files,
        before,
        client: Step { program: "claude".into(), args },
        env,
        after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::parse;

    fn task(memories: bool) -> Task {
        let memories = if memories { r#"[{"content": "Staging listens on port 48213", "age_days": 40}]"# } else { "[]" };
        parse(&format!(
            r#"{{"id": "t", "category": "K1", "fixture": "tally", "prompt": "Which port?", "memories": {memories},
                "checks": [{{"kind": "answer_contains", "text": "48213"}}]}}"#
        ))
        .unwrap()
    }

    fn root() -> PathBuf {
        if cfg!(windows) { PathBuf::from(r"C:\runs\1") } else { PathBuf::from("/runs/1") }
    }

    fn options(lang: Option<Lang>) -> Options {
        Options {
            clawg: root().join("bin").join("clawg"),
            profile: Profile::Dev,
            lang,
            model: "some-model".into(),
            effort: Some("medium".into()),
            max_budget_usd: Some(1.5),
            now_ms: 1_800_000_000_000,
        }
    }

    fn layout() -> Layout {
        Layout { root: root() }
    }

    fn file<'a>(plan: &'a RunPlan, name: &str) -> Option<&'a str> {
        plan.files.iter().find(|(p, _)| p.ends_with(name)).map(|(_, t)| t.as_str())
    }

    fn env<'a>(plan: &'a RunPlan, name: &str) -> Option<&'a str> {
        plan.env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn has_arg_pair(plan: &RunPlan, flag: &str, value: &str) -> bool {
        plan.client.args.windows(2).any(|w| w[0] == flag && w[1] == value)
    }

    #[test]
    fn every_arm_has_a_name() {
        for arm in ARMS {
            assert_eq!(Arm::parse(arm.as_str()), Some(arm));
        }
        assert_eq!(Arm::parse("a1"), None);
    }

    #[test]
    fn a0_has_no_server_no_hooks_no_seed_and_no_native_memory() {
        let plan = plan(&task(true), Arm::A0, &layout(), &options(None)).unwrap();
        let mcp: serde_json::Value = serde_json::from_str(file(&plan, "mcp.json").unwrap()).unwrap();
        assert_eq!(mcp, json!({ "mcpServers": {} }));
        assert_eq!(plan.files.len(), 1);
        assert!(plan.before.is_empty() && plan.after.is_empty());
        assert_eq!(env(&plan, "CLAUDE_CODE_DISABLE_AUTO_MEMORY"), Some("1"));
        assert!(plan.client.args.contains(&"--strict-mcp-config".to_owned()));
        assert!(has_arg_pair(&plan, "-p", "Which port?"));
        assert!(has_arg_pair(&plan, "--output-format", "stream-json"));
        assert!(has_arg_pair(&plan, "--model", "some-model"));
        assert!(has_arg_pair(&plan, "--effort", "medium"));
        assert!(has_arg_pair(&plan, "--max-budget-usd", "1.5"));
        assert_eq!(plan.client.program, "claude");
    }

    #[test]
    fn a2_has_the_server_and_the_seed_but_not_what_setup_writes() {
        let plan = plan(&task(true), Arm::A2, &layout(), &options(None)).unwrap();
        let mcp: serde_json::Value = serde_json::from_str(file(&plan, "mcp.json").unwrap()).unwrap();
        let server = &mcp["mcpServers"]["clawg-dev"];
        assert_eq!(server["command"], json!(options(None).clawg.to_str().unwrap()));
        assert_eq!(server["args"], json!(["mcp", "--profile", "dev", "--agent", "claude-code"]));
        assert!(file(&plan, "settings.json").is_none());
        assert!(file(&plan, "CLAUDE.md").is_none());
        assert!(file(&plan, "preferences.json").is_none());
        assert!(file(&plan, "seed.jsonl").unwrap().contains("Staging listens on port 48213"));
        assert_eq!(plan.before[0].args[..3], ["import", "--from", "clawg"]);
        assert_eq!(plan.after[0].args, ["stop", "--profile", "dev"]);
    }

    #[test]
    fn a3_has_the_hooks_and_the_protocol_setup_writes_in_the_profile_language() {
        let plan = plan(&task(true), Arm::A3, &layout(), &options(Some(Lang::Es))).unwrap();
        let settings: serde_json::Value = serde_json::from_str(file(&plan, "settings.json").unwrap()).unwrap();
        let start = &settings["hooks"]["SessionStart"][0]["hooks"][0];
        assert_eq!(start["args"], json!(["hook", "session-start", "--agent", "claude-code", "--profile", "dev"]));
        assert!(settings["hooks"]["UserPromptSubmit"].is_array());
        let block = file(&plan, "CLAUDE.md").unwrap();
        assert!(block.contains(setup::BEGIN_MARKER));
        assert!(block.contains("Objetivo: lo que pidió la persona"), "the block is the Spanish protocol");
        assert!(file(&plan, "preferences.json").unwrap().contains("\"es\""));
        let settings_path = plan.files.iter().find(|(p, _)| p.ends_with("settings.json")).unwrap();
        assert!(settings_path.0.starts_with(layout().config_dir()));
        assert_eq!(env(&plan, "CLAUDE_CONFIG_DIR"), layout().config_dir().to_str());
        assert_eq!(env(&plan, "HOME"), layout().home().to_str());
        assert!(env(&plan, "ENABLE_TOOL_SEARCH").is_none());
    }

    #[test]
    fn a3_without_memories_imports_nothing() {
        let plan = plan(&task(false), Arm::A3, &layout(), &options(None)).unwrap();
        assert!(plan.before.is_empty());
        assert!(file(&plan, "seed.jsonl").is_none());
        assert_eq!(plan.after.len(), 1);
    }

    #[test]
    fn a3_eager_turns_tool_search_off() {
        let plan = plan(&task(true), Arm::A3Eager, &layout(), &options(None)).unwrap();
        assert_eq!(env(&plan, "ENABLE_TOOL_SEARCH"), Some("false"));
        assert!(file(&plan, "settings.json").is_some());
    }

    #[test]
    fn a4_holds_the_same_facts_in_native_memory_and_keeps_it_on() {
        let plan = plan(&task(true), Arm::A4, &layout(), &options(None)).unwrap();
        let native = plan.files.iter().find(|(p, _)| p.ends_with("MEMORY.md")).unwrap();
        assert!(native.0.starts_with(layout().config_dir().join("projects")));
        assert!(native.1.contains("Staging listens on port 48213"));
        assert_eq!(env(&plan, "CLAUDE_CODE_DISABLE_AUTO_MEMORY"), Some("0"));
        assert!(plan.before.is_empty());
    }

    #[test]
    fn b_takes_away_the_tools_that_read_memories() {
        let plan = plan(&task(true), Arm::B, &layout(), &options(None)).unwrap();
        assert!(has_arg_pair(
            &plan,
            "--disallowedTools",
            "mcp__clawg-dev__search_memory,mcp__clawg-dev__read_memory,mcp__clawg-dev__list_memories"
        ));
        assert!(file(&plan, "CLAUDE.md").is_some());
    }

    #[test]
    fn a_relative_clawg_is_refused() {
        let mut opts = options(None);
        opts.clawg = PathBuf::from("clawg");
        assert!(plan(&task(true), Arm::A3, &layout(), &opts).is_err());
    }
}
