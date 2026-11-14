//! An arm: what one session of a task gets besides the prompt. [`plan`]
//! decides every file, argument and variable of a run without touching disk;
//! the hooks and the protocol block come from `clawg setup`'s own plan, so an
//! arm with them is what `setup` installs.

use std::path::{Path, PathBuf};

use clawg_cli::setup::{self, FileStep, Mode, State};
use clawg_core::paths::Profile;
use clawg_core::preferences::{Lang, Preferences};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::task::{SeedMemory, Task};
use crate::{rules, seed};

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
    /// A3 without the protocol block in `CLAUDE.md`.
    L0,
    /// L0 with the seeded rules added at startup, by a hook of the harness.
    L1,
    /// L1 with the rules that have globs in rules files the client loads
    /// when the agent works on a file they match, and not at startup.
    L2,
}

/// How the sessions of a task are played on one working directory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// Every session starts a conversation of its own.
    #[default]
    Split,
    /// Each session after the first continues the conversation of the one before.
    Joined,
    /// The second session is played both ways from the same first one.
    Both,
}

impl Shape {
    pub fn parse(name: &str) -> Option<Shape> {
        [Shape::Split, Shape::Joined, Shape::Both].into_iter().find(|s| s.as_str() == name)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Shape::Split => "split",
            Shape::Joined => "joined",
            Shape::Both => "both",
        }
    }

    /// Why a task of `sessions` sessions cannot be played this way, if it cannot.
    pub fn check(self, sessions: usize) -> Result<(), String> {
        match (self, sessions) {
            (Shape::Split, _) | (Shape::Joined, 2..) | (Shape::Both, 2) => Ok(()),
            (_, 1) => Err(format!("a task of one session is played only with --sessions split, not {}", self.as_str())),
            (_, n) => Err(format!("--sessions both needs a task of two sessions, and this one has {n}")),
        }
    }
}

pub const ARMS: [Arm; 9] = [Arm::A0, Arm::A2, Arm::A3, Arm::A3Eager, Arm::A4, Arm::B, Arm::L0, Arm::L1, Arm::L2];

/// The tools arm B takes away.
const READ_TOOLS: [&str; 3] = ["search_memory", "read_memory", "list_memories"];

/// The most characters of the output of a hook the client puts in the context.
const HOOK_CONTEXT_MAX_CHARS: usize = 10_000;

/// The client a real run starts, found on the `PATH`.
pub const CLIENT: &str = "claude";

/// The variable a `testing` build of clawg reads its home from instead of the
/// system's: clawg-core's `paths::TEST_HOME_VAR`, which only such a build has.
pub const TEST_HOME_VAR: &str = "CLAWG_TEST_HOME";

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
            Arm::L0 => "l0",
            Arm::L1 => "l1",
            Arm::L2 => "l2",
        }
    }

    /// Whether a run of this arm starts clawg: its daemon, its bridge, its hooks.
    pub fn has_clawg(self) -> bool {
        matches!(self, Arm::A2 | Arm::A3 | Arm::A3Eager | Arm::B | Arm::L0 | Arm::L1 | Arm::L2)
    }

    /// Whether the arm has the hooks `setup` writes in `settings.json`.
    fn has_setup(self) -> bool {
        matches!(self, Arm::A3 | Arm::A3Eager | Arm::B | Arm::L0 | Arm::L1 | Arm::L2)
    }

    /// Whether the arm has the protocol block `setup` writes in `CLAUDE.md`.
    fn has_protocol(self) -> bool {
        matches!(self, Arm::A3 | Arm::A3Eager | Arm::B)
    }

    /// Whether the arm delivers the seeded rules and, if it does, whether
    /// the rules with globs go by file instead of at startup.
    fn rules_by_file(self) -> Option<bool> {
        match self {
            Arm::L1 => Some(false),
            Arm::L2 => Some(true),
            _ => None,
        }
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
    /// The rules the startup hook of an arm with rules prints.
    pub fn startup_rules(&self) -> PathBuf {
        self.root.join("startup-rules.md")
    }
    /// Where the hook of every arm logs each instruction file the client loads.
    pub fn instructions_log(&self) -> PathBuf {
        self.root.join("instructions.jsonl")
    }
    /// The project's rules files that an arm with rules by file writes.
    pub fn rules_dir(&self) -> PathBuf {
        self.workdir().join(".claude").join("rules").join("clawg")
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
    /// The Claude Code version the run is pinned to; recorded, not planned.
    pub claude_version: Option<String>,
    pub now_ms: i64,
    /// The rules of other topics every arm with memory is seeded with.
    pub noise: Vec<SeedMemory>,
    /// The name of that noise, which a run records.
    pub noise_name: String,
    pub shape: Shape,
    /// The `agent-eval` binary the hooks of the harness run; absolute.
    pub agent_eval: PathBuf,
}

/// One program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub program: String,
    pub args: Vec<String>,
}

/// The `clawg serve` of the run's home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Daemon {
    pub serve: Step,
    /// The file it publishes, naming its pid, once it listens.
    pub state_file: PathBuf,
    /// What says whether its model is ready, once it listens: a seed or a
    /// session sent before would meet `embedder_not_ready`.
    pub status: Step,
}

/// Everything a run does, decided before anything is written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunPlan {
    pub task: String,
    pub arm: String,
    /// Files written before the session, by absolute path.
    pub files: Vec<(PathBuf, String)>,
    /// Started before every command, and ended by the last of [`RunPlan::after`].
    pub daemon: Option<Daemon>,
    /// Commands run before the session, against [`RunPlan::daemon`].
    pub before: Vec<Step>,
    /// The first client session, run in [`Layout::workdir`].
    pub client: Step,
    /// The sessions after the first, in order, each with its own prompt; a
    /// shape that continues the conversation has `--continue` in them, and a
    /// fork plans its second session so, which the new one is without it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub later: Vec<Step>,
    /// What every process of the run gets on top of the environment it inherits.
    pub env: Vec<(String, String)>,
    /// Commands run after the session.
    pub after: Vec<Step>,
}

impl RunPlan {
    /// Every session of the run, in order.
    pub fn sessions(&self) -> Vec<&Step> {
        std::iter::once(&self.client).chain(&self.later).collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArmError {
    #[error("{0}")]
    Shape(String),
    #[error("{0}")]
    Setup(#[from] setup::SetupError),
    #[error("a path of the run is not valid Unicode: {0}")]
    NonUnicode(PathBuf),
    #[error("the rules that start the session are {chars} characters, and the client injects at most {max} from the output of a hook")]
    StartupTooLong { chars: usize, max: usize },
}

fn text(path: &Path) -> Result<String, ArmError> {
    path.to_str().map(str::to_owned).ok_or_else(|| ArmError::NonUnicode(path.to_owned()))
}

/// A hook that runs `agent-eval COMMAND FILE`.
fn harness_hook(agent_eval: &str, command: &str, file: &str) -> Value {
    json!({ "type": "command", "command": agent_eval, "args": [command, file], "timeout": 10 })
}

/// Adds `hook` to the hooks of `event` in `settings`, after any already there.
fn add_hook(settings: &mut Value, event: &str, hook: Value) {
    let groups = &mut settings["hooks"][event];
    if !groups.is_array() {
        *groups = json!([]);
    }
    if let Some(groups) = groups.as_array_mut() {
        groups.push(json!({ "hooks": [hook] }));
    }
}

pub fn plan(task: &Task, arm: Arm, layout: &Layout, options: &Options) -> Result<RunPlan, ArmError> {
    let prompts = task.prompts();
    options.shape.check(prompts.len()).map_err(ArmError::Shape)?;
    let clawg = text(&options.clawg)?;
    let workdir = text(&layout.workdir())?;
    let server_name = options.profile.mcp_server_name();
    let mut files = Vec::new();
    let mut daemon = None;
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut servers = serde_json::Map::new();
    let mut settings = json!({});
    let memories = seed::seeded(&task.memories, &options.noise);

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
        if let (true, FileStep::Write(body)) = (arm.has_setup(), setup_plan.settings) {
            settings = serde_json::from_str(&body).expect("setup writes its settings as JSON");
        }
        if let (true, FileStep::Write(body)) = (arm.has_protocol(), setup_plan.instructions) {
            files.push((state.instructions.path.clone(), body));
        }
        let data = layout.data_dir(options.profile);
        // An ephemeral port, so a run never takes the profile's port from a
        // daemon of the user's.
        daemon = Some(Daemon {
            serve: Step { program: clawg.clone(), args: ["serve", "--profile", options.profile.as_str(), "--port", "0"].map(String::from).to_vec() },
            state_file: clawg_core::paths::daemon_state_file(&data),
            status: Step { program: clawg.clone(), args: ["status", "--profile", options.profile.as_str()].map(String::from).to_vec() },
        });
        if let Some(lang) = options.lang {
            files.push((clawg_core::paths::preferences_file(&data), Preferences { language: Some(lang) }.to_json()));
        }
        if !memories.is_empty() {
            files.push((layout.seed(), seed::export(&memories, setup::AGENT, &workdir, &task.fixture, options.now_ms)));
            before.push(Step {
                program: clawg.clone(),
                args: ["import", "--from", "clawg", &text(&layout.seed())?, "--profile", options.profile.as_str()]
                    .map(String::from)
                    .to_vec(),
            });
        }
        after.push(Step { program: clawg, args: ["stop", "--profile", options.profile.as_str()].map(String::from).to_vec() });
    }
    if arm == Arm::A4 && !memories.is_empty() {
        let dir = layout.config_dir().join("projects").join(seed::project_slug(&workdir)).join("memory");
        files.push((dir.join("MEMORY.md"), seed::native_memory(&memories)));
    }
    let harness = text(&options.agent_eval)?;
    if let Some(by_file) = arm.rules_by_file() {
        if let Some(rules) = rules::startup(&memories, by_file, options.lang) {
            let chars = rules.chars().count();
            if chars > HOOK_CONTEXT_MAX_CHARS {
                return Err(ArmError::StartupTooLong { chars, max: HOOK_CONTEXT_MAX_CHARS });
            }
            add_hook(&mut settings, "SessionStart", harness_hook(&harness, "hook-context", &text(&layout.startup_rules())?));
            files.push((layout.startup_rules(), rules));
        }
        if by_file {
            files.extend(rules::files(&memories, options.lang).into_iter().map(|(name, body)| (layout.rules_dir().join(name), body)));
        }
    }
    add_hook(&mut settings, "InstructionsLoaded", harness_hook(&harness, "hook-log", &text(&layout.instructions_log())?));
    files.push((layout.config_dir().join("settings.json"), serde_json::to_string_pretty(&settings).expect("JSON serializes") + "\n"));
    files.push((layout.mcp_config(), serde_json::to_string_pretty(&json!({ "mcpServers": servers })).expect("JSON serializes") + "\n"));

    let mut args: Vec<String> = vec![
        "-p".into(),
        prompts[0].to_owned(),
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
    // Nothing can approve a prompt in `-p`, so without a mode the client denies edits and MCP calls.
    // Edits are approved inside the working directory only; a shell is the one way an unattended
    // session could act outside it, and no task needs one. A denial outranks an allowance.
    // A task that names shell commands lifts the denial and allows each as a scoped rule, which
    // leaves every other command denied.
    let mut denied: Vec<String> = if task.shell.is_empty() { vec!["Bash".into()] } else { Vec::new() };
    if arm == Arm::B {
        denied.extend(READ_TOOLS.iter().map(|t| format!("mcp__{server_name}__{t}")));
    }
    args.extend(["--permission-mode".into(), "acceptEdits".into()]);
    if !denied.is_empty() {
        args.extend(["--disallowedTools".into(), denied.join(",")]);
    }
    let mut allowed: Vec<String> = Vec::new();
    if arm.has_clawg() {
        allowed.push(format!("mcp__{server_name}"));
    }
    allowed.extend(task.shell.iter().map(|command| format!("Bash({command}:*)")));
    if !allowed.is_empty() {
        args.extend(["--allowedTools".into(), allowed.join(",")]);
    }

    let home = text(&layout.home())?;
    let mut env = vec![
        ("HOME".to_owned(), home.clone()),
        ("USERPROFILE".to_owned(), home.clone()),
        (TEST_HOME_VAR.to_owned(), home),
        ("CLAUDE_CONFIG_DIR".to_owned(), text(&layout.config_dir())?),
        ("CLAUDE_CODE_DISABLE_AUTO_MEMORY".to_owned(), if arm == Arm::A4 { "0" } else { "1" }.to_owned()),
    ];
    if arm == Arm::A3Eager {
        env.push(("ENABLE_TOOL_SEARCH".to_owned(), "false".to_owned()));
    }

    let later = prompts[1..]
        .iter()
        .map(|prompt| {
            let mut args = args.clone();
            args[1] = (*prompt).to_owned();
            if options.shape != Shape::Split {
                args.push("--continue".into());
            }
            Step { program: CLIENT.into(), args }
        })
        .collect();

    Ok(RunPlan {
        task: task.id.clone(),
        arm: arm.as_str().to_owned(),
        files,
        daemon,
        before,
        client: Step { program: CLIENT.into(), args },
        later,
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
            claude_version: Some("2.1.300".into()),
            now_ms: 1_800_000_000_000,
            noise: Vec::new(),
            noise_name: "noise".into(),
            shape: Shape::Split,
            agent_eval: harness(),
        }
    }

    fn harness() -> PathBuf {
        root().join("bin").join("agent-eval")
    }

    fn settings(plan: &RunPlan) -> serde_json::Value {
        serde_json::from_str(file(plan, "settings.json").unwrap()).unwrap()
    }

    fn task_with_rules() -> Task {
        parse(
            r#"{"id": "t", "category": "K3", "fixture": "tally", "prompt": "Write the notes.",
                "memories": [{"content": "Lists use an asterisk\nNot a hyphen.", "age_days": 120, "rule": {"paths": ["**/*.md"]}},
                             {"content": "Staging listens on port 48213", "age_days": 40}],
                "checks": [{"kind": "file_contains", "path": "notes.md", "text": "* "}]}"#,
        )
        .unwrap()
    }

    fn always_noise() -> Vec<SeedMemory> {
        let task = parse(
            r#"{"id": "n", "category": "N", "fixture": "f", "prompt": "p", "checks": [{"kind": "answer_contains", "text": "x"}],
                "memories": [{"content": "Commit messages\nConventional commits.", "age_days": 23, "rule": {}}]}"#,
        )
        .unwrap();
        task.memories
    }

    fn startup_of(plan: &RunPlan) -> Option<&str> {
        plan.files.iter().find(|(p, _)| *p == layout().startup_rules()).map(|(_, t)| t.as_str())
    }

    fn rules_files(plan: &RunPlan) -> Vec<&(PathBuf, String)> {
        plan.files.iter().filter(|(p, _)| p.starts_with(layout().rules_dir())).collect()
    }

    #[test]
    fn the_startup_rules_must_fit_what_the_client_injects() {
        let rules = |n: usize, paths: &str| -> Vec<SeedMemory> {
            (0..n)
                .map(|i| {
                    serde_json::from_str(&format!(
                        r#"{{"content": "Rule number {i}\nThe rule says something about case {i} that takes a few words to say.", "rule": {paths}}}"#
                    ))
                    .unwrap()
                })
                .collect()
        };
        let with = |noise: Vec<SeedMemory>, arm: Arm| {
            let mut opts = options(None);
            opts.noise = noise;
            plan(&task(true), arm, &layout(), &opts)
        };

        assert!(with(rules(15, "{}"), Arm::L1).is_ok());
        assert!(with(rules(15, "{}"), Arm::L2).is_ok());

        let refused = with(rules(300, "{}"), Arm::L1).unwrap_err();
        assert!(matches!(refused, ArmError::StartupTooLong { chars, max: 10_000 } if chars > 10_000), "{refused:?}");
        let said = refused.to_string();
        let ArmError::StartupTooLong { chars, .. } = refused else { unreachable!() };
        assert!(said.contains(&chars.to_string()) && said.contains("10000") && said.contains("hook"), "the size and the limit are named: {said}");
        assert!(matches!(with(rules(300, "{}"), Arm::L2), Err(ArmError::StartupTooLong { .. })), "the rules without globs are delivered at startup by l2 too");

        let by_glob = rules(300, r#"{"paths": ["**/*.py"]}"#);
        assert!(with(by_glob.clone(), Arm::L2).is_ok(), "l2 delivers the rules with globs by file, so nothing goes at startup");
        assert!(matches!(with(by_glob, Arm::L1), Err(ArmError::StartupTooLong { .. })));

        for arm in [Arm::A3, Arm::A4, Arm::L0] {
            assert!(with(rules(300, "{}"), arm).is_ok(), "{arm:?} has no startup hook of the harness");
        }
    }

    #[test]
    fn l0_is_a3_without_the_protocol() {
        let a3 = plan(&task(true), Arm::A3, &layout(), &options(None)).unwrap();
        let l0 = plan(&task(true), Arm::L0, &layout(), &options(None)).unwrap();
        assert!(file(&a3, "CLAUDE.md").is_some());
        assert!(file(&l0, "CLAUDE.md").is_none());
        for name in ["mcp.json", "settings.json", "seed.jsonl"] {
            assert_eq!(file(&l0, name), file(&a3, name), "{name}");
        }
        assert_eq!((&l0.daemon, &l0.before, &l0.after, &l0.client.args), (&a3.daemon, &a3.before, &a3.after, &a3.client.args));
    }

    #[test]
    fn l1_adds_the_rules_at_startup_through_a_hook_of_the_harness() {
        let mut opts = options(None);
        opts.noise = always_noise();
        let plan = plan(&task_with_rules(), Arm::L1, &layout(), &opts).unwrap();
        let startup = startup_of(&plan).unwrap();
        assert_eq!(Some(startup.to_owned()), crate::rules::startup(&seed::seeded(&task_with_rules().memories, &opts.noise), false, None));
        assert!(startup.contains("[23 days ago] Commit messages") && startup.contains("[120 days ago] Lists use an asterisk"), "{startup}");
        assert!(!startup.contains("48213"), "a memory that is not a rule is not delivered");
        let start = &settings(&plan)["hooks"]["SessionStart"];
        assert_eq!(start.as_array().unwrap().len(), 2, "clawg's own hook stays: {start}");
        assert_eq!(
            start[1]["hooks"][0],
            json!({ "type": "command", "command": harness().to_str().unwrap(), "args": ["hook-context", layout().startup_rules().to_str().unwrap()], "timeout": 10 })
        );
        assert!(file(&plan, "CLAUDE.md").is_none());
        assert!(rules_files(&plan).is_empty());
    }

    #[test]
    fn l2_moves_the_rules_with_globs_to_rules_files() {
        let mut opts = options(Some(Lang::Es));
        opts.noise = always_noise();
        let plan = plan(&task_with_rules(), Arm::L2, &layout(), &opts).unwrap();
        let startup = startup_of(&plan).unwrap();
        assert!(startup.contains("Commit messages") && !startup.contains("asterisk"), "{startup}");
        let by_file = rules_files(&plan);
        assert_eq!(by_file.len(), 1);
        assert_eq!(by_file[0].0, layout().workdir().join(".claude").join("rules").join("clawg").join("rules-1.md"));
        let body = &by_file[0].1;
        assert!(body.starts_with("---\npaths:\n  - \"**/*.md\"\n---\n") && body.contains("[hace 120 días] Lists use an asterisk"), "{body}");
        assert_eq!(settings(&plan)["hooks"]["SessionStart"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn l1_without_rules_adds_no_hook() {
        for arm in [Arm::L1, Arm::L2] {
            let plan = plan(&task(true), arm, &layout(), &options(None)).unwrap();
            assert!(startup_of(&plan).is_none() && rules_files(&plan).is_empty(), "{}", arm.as_str());
            assert_eq!(settings(&plan)["hooks"]["SessionStart"].as_array().unwrap().len(), 1, "{}", arm.as_str());
        }
    }

    #[test]
    fn every_arm_logs_the_instruction_files_the_client_loads() {
        let logged = json!([{ "hooks": [{
            "type": "command", "command": harness().to_str().unwrap(),
            "args": ["hook-log", layout().instructions_log().to_str().unwrap()], "timeout": 10
        }] }]);
        for arm in ARMS {
            let plan = plan(&task(true), arm, &layout(), &options(None)).unwrap();
            assert!(plan.files.iter().any(|(p, _)| *p == layout().config_dir().join("settings.json")), "{}", arm.as_str());
            let settings = settings(&plan);
            assert_eq!(settings["hooks"]["InstructionsLoaded"], logged, "{}", arm.as_str());
            let with_setup = matches!(arm, Arm::A3 | Arm::A3Eager | Arm::B | Arm::L0 | Arm::L1 | Arm::L2);
            assert_eq!(settings["hooks"]["UserPromptSubmit"].is_array(), with_setup, "setup's hooks stay: {}", arm.as_str());
        }
    }

    fn noise_rule() -> crate::task::SeedMemory {
        let task = parse(
            r#"{"id": "n", "category": "N", "fixture": "f", "prompt": "p", "checks": [{"kind": "answer_contains", "text": "x"}],
                "memories": [{"content": "Go errors are wrapped\nWrap returned errors with %w.", "age_days": 20, "rule": {"paths": ["**/*.go"]}}]}"#,
        )
        .unwrap();
        task.memories[0].clone()
    }

    #[test]
    fn every_arm_with_memory_seeds_the_noise_with_the_task() {
        let mut opts = options(None);
        opts.noise = vec![noise_rule()];
        for arm in ARMS {
            let plan = plan(&task(true), arm, &layout(), &opts).unwrap();
            let (seed, native) = (file(&plan, "seed.jsonl"), file(&plan, "MEMORY.md"));
            match arm {
                Arm::A0 => assert!(seed.is_none() && native.is_none()),
                Arm::A4 => {
                    let native = native.unwrap();
                    assert!(native.find("48213").unwrap() < native.find("Go errors").unwrap(), "the oldest memory goes first:\n{native}");
                }
                _ => {
                    let seed = seed.unwrap();
                    assert!(seed.contains("Go errors") && seed.contains("48213"), "{}: {seed}", arm.as_str());
                }
            }
        }
        let plan = plan(&task(false), Arm::A3, &layout(), &opts).unwrap();
        assert!(file(&plan, "seed.jsonl").unwrap().contains("Go errors"), "a task without memories seeds the noise");
        assert_eq!(plan.before.len(), 1);
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
    fn the_arms_with_clawg_start_a_daemon_of_the_run_on_an_ephemeral_port() {
        let with_daemon: Vec<Arm> = ARMS.into_iter().filter(|arm| plan(&task(true), *arm, &layout(), &options(None)).unwrap().daemon.is_some()).collect();
        assert_eq!(with_daemon, [Arm::A2, Arm::A3, Arm::A3Eager, Arm::B, Arm::L0, Arm::L1, Arm::L2]);

        let plan = plan(&task(true), Arm::A3, &layout(), &options(None)).unwrap();
        let daemon = plan.daemon.unwrap();
        assert_eq!(daemon.serve.program, options(None).clawg.to_str().unwrap());
        assert_eq!(daemon.serve.args, ["serve", "--profile", "dev", "--port", "0"]);
        assert_eq!(daemon.state_file, layout().home().join(".clawg-dev").join("daemon.json"));
        assert_eq!(daemon.status.program, daemon.serve.program);
        assert_eq!(daemon.status.args, ["status", "--profile", "dev"]);
    }

    #[test]
    fn every_process_of_a_run_finds_its_home_also_in_a_testing_build() {
        for arm in ARMS {
            let plan = plan(&task(true), arm, &layout(), &options(None)).unwrap();
            assert_eq!(env(&plan, "CLAWG_TEST_HOME"), layout().home().to_str(), "{}", arm.as_str());
            assert_eq!(env(&plan, "CLAWG_TEST_HOME"), env(&plan, "HOME"));
        }
    }

    #[test]
    fn the_plan_reads_back_from_its_json() {
        let plan = plan(&task(true), Arm::B, &layout(), &options(Some(Lang::Es))).unwrap();
        let back: RunPlan = serde_json::from_str(&serde_json::to_string(&plan).unwrap()).unwrap();
        assert_eq!(back, plan);
    }

    #[test]
    fn a0_has_no_server_no_memory_hook_no_seed_and_no_native_memory() {
        let plan = plan(&task(true), Arm::A0, &layout(), &options(None)).unwrap();
        let mcp: serde_json::Value = serde_json::from_str(file(&plan, "mcp.json").unwrap()).unwrap();
        assert_eq!(mcp, json!({ "mcpServers": {} }));
        assert_eq!(plan.files.len(), 2, "mcp.json and settings.json");
        let settings = settings(&plan);
        let hooks: Vec<&String> = settings["hooks"].as_object().unwrap().keys().collect();
        assert_eq!(hooks, ["InstructionsLoaded"], "only the log of instruction files, which adds no context");
        assert!(plan.daemon.is_none() && plan.before.is_empty() && plan.after.is_empty());
        assert_eq!(env(&plan, "CLAUDE_CODE_DISABLE_AUTO_MEMORY"), Some("1"));
        assert!(plan.client.args.contains(&"--strict-mcp-config".to_owned()));
        assert!(has_arg_pair(&plan, "-p", "Which port?"));
        assert!(has_arg_pair(&plan, "--output-format", "stream-json"));
        assert!(has_arg_pair(&plan, "--model", "some-model"));
        assert!(has_arg_pair(&plan, "--effort", "medium"));
        assert!(has_arg_pair(&plan, "--max-budget-usd", "1.5"));
        assert_eq!(plan.client.program, CLIENT);
        assert_eq!(CLIENT, "claude");
    }

    #[test]
    fn a2_has_the_server_and_the_seed_but_not_what_setup_writes() {
        let plan = plan(&task(true), Arm::A2, &layout(), &options(None)).unwrap();
        let mcp: serde_json::Value = serde_json::from_str(file(&plan, "mcp.json").unwrap()).unwrap();
        let server = &mcp["mcpServers"]["clawg-dev"];
        assert_eq!(server["command"], json!(options(None).clawg.to_str().unwrap()));
        assert_eq!(server["args"], json!(["mcp", "--profile", "dev", "--agent", "claude-code"]));
        assert!(settings(&plan)["hooks"]["SessionStart"].is_null(), "no hook of setup");
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
            "Bash,mcp__clawg-dev__search_memory,mcp__clawg-dev__read_memory,mcp__clawg-dev__list_memories"
        ));
        assert!(file(&plan, "CLAUDE.md").is_some());
    }

    fn pair_value<'a>(plan: &'a RunPlan, flag: &str) -> Option<&'a str> {
        plan.client.args.windows(2).find(|w| w[0] == flag).map(|w| w[1].as_str())
    }

    #[test]
    fn every_arm_may_edit_its_directory_and_none_may_run_a_shell() {
        for arm in ARMS {
            let plan = plan(&task(true), arm, &layout(), &options(None)).unwrap();
            assert!(has_arg_pair(&plan, "--permission-mode", "acceptEdits"), "{}", arm.as_str());
            let denied = pair_value(&plan, "--disallowedTools").unwrap();
            assert!(denied.split(',').any(|t| t == "Bash"), "{}: {denied}", arm.as_str());
        }
    }

    #[test]
    fn a_task_with_a_shell_allows_its_commands_and_no_other() {
        let shelled = parse(
            r#"{"id": "t", "category": "E", "fixture": "tally", "prompt": "Count the files.", "shell": ["eza", "ls"],
                "checks": [{"kind": "answer_contains", "text": "3"}]}"#,
        )
        .unwrap();
        for arm in ARMS {
            let plan = plan(&shelled, arm, &layout(), &options(None)).unwrap();
            let allowed: Vec<&str> = pair_value(&plan, "--allowedTools").unwrap().split(',').collect();
            assert!(allowed.contains(&"Bash(eza:*)") && allowed.contains(&"Bash(ls:*)"), "{}: {allowed:?}", arm.as_str());
            assert!(!allowed.contains(&"Bash"), "{}: a bare Bash allows every command", arm.as_str());
            assert_eq!(allowed.contains(&"mcp__clawg-dev"), arm.has_clawg(), "{}", arm.as_str());
            let denied = pair_value(&plan, "--disallowedTools");
            assert!(denied.is_none_or(|d| !d.split(',').any(|t| t == "Bash")), "{}: {denied:?}", arm.as_str());
        }
        let b = plan(&shelled, Arm::B, &layout(), &options(None)).unwrap();
        assert_eq!(
            pair_value(&b, "--disallowedTools"),
            Some("mcp__clawg-dev__search_memory,mcp__clawg-dev__read_memory,mcp__clawg-dev__list_memories")
        );
        let a0 = plan(&shelled, Arm::A0, &layout(), &options(None)).unwrap();
        assert_eq!(pair_value(&a0, "--disallowedTools"), None, "nothing to deny");
        assert_eq!(pair_value(&a0, "--allowedTools"), Some("Bash(eza:*),Bash(ls:*)"));
    }

    fn two_sessions() -> Task {
        parse(
            r#"{"id": "t", "category": "R", "fixture": "tally",
                "sessions": [{"prompt": "Inventory the timeouts."}, {"prompt": "Apply what was decided."}],
                "checks": [{"kind": "answer_contains", "text": "done"}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn a_task_of_one_session_plans_what_it_planned_before() {
        for shape in [Shape::Split, Shape::Joined] {
            let mut opts = options(None);
            opts.shape = shape;
            let plan = plan(&task(true), Arm::A3, &layout(), &opts);
            assert_eq!(plan.is_ok(), shape == Shape::Split, "{shape:?}: a task of one session takes only split");
        }
        let plan = plan(&task(true), Arm::A3, &layout(), &options(None)).unwrap();
        assert!(plan.later.is_empty());
        assert_eq!(plan.sessions(), [&plan.client]);
        assert!(serde_json::to_value(&plan).unwrap().get("later").is_none(), "an older reader finds the plan it knew");
        assert!(has_arg_pair(&plan, "-p", "Which port?") && !plan.client.args.contains(&"--continue".to_owned()));
    }

    #[test]
    fn each_session_has_its_prompt_and_a_joined_one_continues_the_conversation() {
        let planned = |shape| {
            let mut opts = options(None);
            opts.shape = shape;
            plan(&two_sessions(), Arm::A3, &layout(), &opts).unwrap()
        };
        let split = planned(Shape::Split);
        let sessions = split.sessions();
        assert_eq!(sessions.len(), 2);
        assert_eq!((sessions[0].args[1].as_str(), sessions[1].args[1].as_str()), ("Inventory the timeouts.", "Apply what was decided."));
        assert!(sessions.iter().all(|s| !s.args.contains(&"--continue".to_owned())));
        assert_eq!(sessions[0].args[2..], sessions[1].args[2..], "the same arguments");

        let joined = planned(Shape::Joined);
        let sessions = joined.sessions();
        assert!(!sessions[0].args.contains(&"--continue".to_owned()));
        assert_eq!(sessions[1].args.last().map(String::as_str), Some("--continue"));
        assert_eq!(sessions[1].args[..sessions[1].args.len() - 1], split.sessions()[1].args[..], "only --continue differs");

        let both = planned(Shape::Both);
        assert_eq!(both.sessions().len(), 2, "a fork plans the second session continued; the new one is that without --continue");
        assert_eq!(both.later, joined.later);
        assert_eq!((both.files.len(), both.before.len(), both.after.len()), (split.files.len(), split.before.len(), split.after.len()));
    }

    #[test]
    fn a_plan_of_several_sessions_reads_back_and_an_older_plan_reads_as_one() {
        let mut opts = options(None);
        opts.shape = Shape::Joined;
        let plan = plan(&two_sessions(), Arm::A3, &layout(), &opts).unwrap();
        let back: RunPlan = serde_json::from_str(&serde_json::to_string(&plan).unwrap()).unwrap();
        assert_eq!(back, plan);
        let mut older = serde_json::to_value(&plan).unwrap();
        older.as_object_mut().unwrap().remove("later");
        let older: RunPlan = serde_json::from_value(older).unwrap();
        assert_eq!(older.sessions().len(), 1);
        assert_eq!(older.client, plan.client);
    }

    #[test]
    fn the_shape_is_split_joined_or_both_and_both_needs_two_sessions() {
        assert_eq!(["split", "joined", "both", "sideways"].map(Shape::parse), [Some(Shape::Split), Some(Shape::Joined), Some(Shape::Both), None]);
        let mut opts = options(None);
        opts.shape = Shape::Both;
        let error = plan(&task(true), Arm::A3, &layout(), &opts).unwrap_err().to_string();
        assert!(error.contains("one session") && error.contains("split"), "{error}");
        let mut three = two_sessions();
        three.sessions.push(three.sessions[0].clone());
        let error = plan(&three, Arm::A3, &layout(), &opts).unwrap_err().to_string();
        assert!(error.contains("both") && error.contains("two sessions") && error.contains('3'), "{error}");
        opts.shape = Shape::Joined;
        assert!(plan(&three, Arm::A3, &layout(), &opts).is_ok());
    }

    #[test]
    fn the_arms_with_clawg_may_call_its_server_without_a_prompt() {
        for arm in ARMS {
            let plan = plan(&task(true), arm, &layout(), &options(None)).unwrap();
            let allowed = pair_value(&plan, "--allowedTools");
            if arm.has_clawg() {
                assert_eq!(allowed, Some("mcp__clawg-dev"), "{}", arm.as_str());
            } else {
                assert_eq!(allowed, None, "{}", arm.as_str());
            }
        }
    }

    #[test]
    fn a_denies_only_bash_while_b_also_denies_its_read_tools_in_one_value() {
        let a3 = plan(&task(true), Arm::A3, &layout(), &options(None)).unwrap();
        assert!(has_arg_pair(&a3, "--disallowedTools", "Bash"));
        assert_eq!(a3.client.args.iter().filter(|a| *a == "--disallowedTools").count(), 1);
        let b = plan(&task(true), Arm::B, &layout(), &options(None)).unwrap();
        assert_eq!(b.client.args.iter().filter(|a| *a == "--disallowedTools").count(), 1);
        assert!(has_arg_pair(
            &b,
            "--disallowedTools",
            "Bash,mcp__clawg-dev__search_memory,mcp__clawg-dev__read_memory,mcp__clawg-dev__list_memories"
        ));
    }

    #[test]
    fn a_relative_clawg_is_refused() {
        let mut opts = options(None);
        opts.clawg = PathBuf::from("clawg");
        assert!(plan(&task(true), Arm::A3, &layout(), &opts).is_err());
    }
}
