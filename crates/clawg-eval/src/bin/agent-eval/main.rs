mod arm;
mod hooks;
mod replay;
mod report;
mod rubric;
mod rules;
mod run;
mod score;
mod seed;
mod stream;
mod task;

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use clawg_core::paths::Profile;
use clawg_core::preferences::Lang;

use crate::arm::{Arm, Layout, Options, Shape};
use crate::run::Client;

const HELP_TEXT: &str = "\
agent-eval tasks [--data DIR]
agent-eval prepare RUN_OPTIONS
agent-eval run RUN_OPTIONS (--replay STREAM | --max-budget-usd N --claude-version V)
agent-eval score --run DIR
agent-eval report --runs DIR
agent-eval costs --runs DIR
agent-eval replay-client STREAM [--mcp-config FILE]
agent-eval hook-context FILE
agent-eval hook-log FILE

  tasks          reads every task in DIR/tasks/*.json, checks each one and
                 that no tracer is in a file of its fixture, and lists them;
                 then checks DIR/noise.json, rules of other topics, and that
                 none of them holds a tracer.
  prepare        lays out one run of a task under an arm in --out: the
                 fixture copied as the working directory, the client's
                 configuration, the seed (the task's memories and the noise,
                 from the newest to the oldest), the rules an arm l1 or l2
                 delivers and run.json with the command line of the client
                 session. Starts nothing.
  run            prepares the run and runs the session run.json plans. An
                 arm with clawg first starts the daemon of the run's home
                 (clawg serve --port 0, output in serve.log), loads the seed
                 with clawg import and ends with clawg stop, which also runs
                 when the seed or the client failed (their output in
                 clawg.log); a seed that does not load starts no client.
                 The client runs in the working directory, its stream
                 captured in stream.jsonl and scored into result.json. With
                 --replay it is the replay client instead of a real one: it
                 prints STREAM, a recorded stream-json, and makes the file
                 changes its Write and Edit calls record, and no Claude
                 model is called. Without it the client is claude, a paid
                 session: the run needs --max-budget-usd, its hard cap, and
                 --claude-version, the Claude Code version it is pinned to,
                 and refuses before laying anything out a claude whose
                 --version reports another one. It also refuses, naming each
                 problem, an environment without ANTHROPIC_API_KEY or with
                 ANTHROPIC_AUTH_TOKEN or CLAUDE_CODE_EFFORT_LEVEL set, and
                 an --out below a git repository or a CLAUDE.md,
                 CLAUDE.local.md or .claude/CLAUDE.md, or whose working
                 directory holds the task's id or one of its tracers: the
                 client tells the agent where it works. The session may edit
                 its directory and call clawg's tools, and has no shell.
                 On Windows clawg finds its
                 home through the system, so an arm with clawg needs a
                 `testing` build of it, which reads CLAWG_TEST_HOME; any
                 other is refused.
  score          scores a run directory again from the streams of its sessions
                 (stream.jsonl, or stream-K.jsonl for a task of several): each
                 session on its own, with the path of every fact born in a
                 session, and the run as a whole. A run that a session stopped
                 is scored with the sessions that ran, and did not end.
  report         reads the result.json of every directory up to 3 levels
                 below DIR and prints a Markdown table by task and arm: runs,
                 successes, runs that did not end, mean cost, turns, seconds
                 and clawg calls, the tracer fates summed, a totals line and
                 the tokens of every model summed.
  costs          reads the same results and prints a CSV with a line per
                 session and model: when the session started and ended
                 (UTC), the credential that paid, the tokens by kind, the
                 main loop's cache writes for 5 minutes and for 1 hour, and
                 the client's estimate of the cost.
  replay-client  the client a dry run starts: prints STREAM after making
                 its file changes in the current directory. With the
                 --mcp-config of a run that has a server, each call of STREAM
                 to a tool of that server is made to it, started as the file
                 names it, and STREAM is printed with what the server
                 answered in place of the results it recorded; the other
                 calls stay as recorded. A call the server cannot answer
                 fails the client, naming the tool.
  hook-context   the startup hook of the arms l1 and l2: prints FILE as the
                 context a SessionStart hook adds.
  hook-log       the hook of every arm on InstructionsLoaded: appends the
                 input it gets to FILE, one line each.

RUN_OPTIONS:
  --task ID --arm a0|a2|a3|a3-eager|a4|b|l0|l1|l2 --out DIR --clawg PATH --model M
  [--effort E] [--max-budget-usd N] [--claude-version V] [--profile dev|prod]
  [--lang en|es] [--data DIR] [--sessions split|joined|both]

  --clawg is the absolute path of the clawg binary the arms with clawg run;
  --profile (dev by default) picks its data directory under the run's home.
  --data defaults to this crate's data/agent-tasks.
  --sessions says how a task of several sessions is played: split (the
  default) starts a conversation for each session, joined continues the
  conversation of the one before with --continue, and both plays the second
  session of a task of two sessions each way from the same first one. A task of
  one session takes only split.

Exit 0 done, 1 a task, file or run could not be used, 2 bad arguments.";

fn default_data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data").join("agent-tasks")
}

#[derive(Debug, Clone, PartialEq)]
struct RunArgs {
    data: PathBuf,
    task: String,
    arm: Arm,
    out: PathBuf,
    clawg: PathBuf,
    model: String,
    effort: Option<String>,
    max_budget_usd: Option<f64>,
    claude_version: Option<String>,
    profile: Profile,
    lang: Option<Lang>,
    shape: Shape,
}

#[derive(Debug, Clone, PartialEq)]
enum Command {
    Tasks { data: PathBuf },
    Prepare(RunArgs),
    /// Without a replay, a run of the real client.
    Run { args: RunArgs, replay: Option<PathBuf> },
    Score { dir: PathBuf },
    Report { dir: PathBuf },
    Costs { dir: PathBuf },
    ReplayClient { stream: PathBuf, mcp_config: Option<PathBuf> },
    HookContext { file: PathBuf },
    HookLog { file: PathBuf },
}

/// `--name value` pairs, each name at most once and among `allowed`.
fn flags(args: impl Iterator<Item = String>, allowed: &[&str]) -> Result<Vec<(String, String)>, String> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let Some(name) = arg.strip_prefix("--") else { return Err(format!("unexpected argument {arg}")) };
        if !allowed.contains(&name) {
            return Err(format!("unknown argument --{name}"));
        }
        let value = args.next().ok_or_else(|| format!("--{name} needs a value"))?;
        if out.iter().any(|(n, _)| n == name) {
            return Err(format!("--{name} is given twice"));
        }
        out.push((name.to_owned(), value));
    }
    Ok(out)
}

fn take(flags: &mut Vec<(String, String)>, name: &str) -> Option<String> {
    let i = flags.iter().position(|(n, _)| n == name)?;
    Some(flags.remove(i).1)
}

fn required(flags: &mut Vec<(String, String)>, name: &str) -> Result<String, String> {
    take(flags, name).ok_or_else(|| format!("--{name} is required"))
}

const RUN_FLAGS: [&str; 12] = ["task", "arm", "out", "clawg", "model", "effort", "max-budget-usd", "claude-version", "profile", "lang", "data", "sessions"];

fn run_args(f: &mut Vec<(String, String)>) -> Result<RunArgs, String> {
    let arm_name = required(f, "arm")?;
    let arm = Arm::parse(&arm_name).ok_or_else(|| format!("unknown arm {arm_name}"))?;
    let clawg = PathBuf::from(required(f, "clawg")?);
    if !clawg.is_absolute() {
        return Err(format!("--clawg {} is not an absolute path", clawg.display()));
    }
    let max_budget_usd = match take(f, "max-budget-usd") {
        Some(n) => Some(n.parse::<f64>().ok().filter(|n| *n > 0.0).ok_or_else(|| format!("--max-budget-usd {n} is not a positive number"))?),
        None => None,
    };
    let profile = match take(f, "profile").as_deref() {
        None | Some("dev") => Profile::Dev,
        Some("prod") => Profile::Prod,
        Some(other) => return Err(format!("unknown profile {other}")),
    };
    let lang = match take(f, "lang") {
        Some(code) => Some(Lang::parse(&code).ok_or_else(|| format!("unknown language {code}"))?),
        None => None,
    };
    let shape = match take(f, "sessions") {
        Some(name) => Shape::parse(&name).ok_or_else(|| format!("unknown shape {name}"))?,
        None => Shape::Split,
    };
    Ok(RunArgs {
        data: take(f, "data").map_or_else(default_data, PathBuf::from),
        task: required(f, "task")?,
        arm,
        out: PathBuf::from(required(f, "out")?),
        clawg,
        model: required(f, "model")?,
        effort: take(f, "effort"),
        max_budget_usd,
        claude_version: take(f, "claude-version"),
        profile,
        lang,
        shape,
    })
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Command>, String> {
    let args: Vec<String> = args.skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return Ok(None);
    }
    let (kind, rest) = args.split_first().ok_or("name a command")?;
    let rest = rest.iter().cloned();
    let command = match kind.as_str() {
        "tasks" => {
            let mut f = flags(rest, &["data"])?;
            Command::Tasks { data: take(&mut f, "data").map_or_else(default_data, PathBuf::from) }
        }
        "prepare" => Command::Prepare(run_args(&mut flags(rest, &RUN_FLAGS)?)?),
        "run" => {
            let allowed: Vec<&str> = RUN_FLAGS.iter().copied().chain(["replay"]).collect();
            let mut f = flags(rest, &allowed)?;
            let replay = take(&mut f, "replay").map(PathBuf::from);
            let args = run_args(&mut f)?;
            if replay.is_none() {
                if args.max_budget_usd.is_none() {
                    return Err("a run without --replay starts claude, a paid session: cap it with --max-budget-usd".into());
                }
                if args.claude_version.is_none() {
                    return Err("a run without --replay starts claude: pin its version with --claude-version".into());
                }
            }
            Command::Run { args, replay }
        }
        "score" => Command::Score { dir: PathBuf::from(required(&mut flags(rest, &["run"])?, "run")?) },
        "report" => Command::Report { dir: PathBuf::from(required(&mut flags(rest, &["runs"])?, "runs")?) },
        "costs" => Command::Costs { dir: PathBuf::from(required(&mut flags(rest, &["runs"])?, "runs")?) },
        "replay-client" => {
            let rest: Vec<String> = rest.collect();
            let (stream, mcp_config) = match rest.as_slice() {
                [stream] => (stream, None),
                [stream, flag, config] if flag == "--mcp-config" => (stream, Some(PathBuf::from(config))),
                _ => return Err("replay-client takes one stream file and, optionally, --mcp-config FILE".into()),
            };
            Command::ReplayClient { stream: PathBuf::from(stream), mcp_config }
        }
        "hook-context" | "hook-log" => {
            let [file]: [String; 1] = rest.collect::<Vec<_>>().try_into().map_err(|_| format!("{kind} takes one file"))?;
            let file = PathBuf::from(file);
            if kind == "hook-context" { Command::HookContext { file } } else { Command::HookLog { file } }
        }
        other => return Err(format!("unknown command {other}")),
    };
    Ok(Some(command))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn list_tasks(data: &Path) -> Result<(), String> {
    let tasks = task::load(data).map_err(|e| e.to_string())?;
    let noise = task::load_noise(data, &tasks).map_err(|e| e.to_string())?;
    let mut leaks = Vec::new();
    for t in &tasks {
        let mut files = Vec::new();
        read_files(&task::fixture_dir(data, t), "", &mut files).map_err(|e| e.to_string())?;
        leaks.extend(task::leaked_tracers(t, &files).into_iter().map(|l| format!("{}: {l}", t.id)));
        println!(
            "{} {} fixture={} memories={} tracers={} checks={}{}",
            t.id,
            t.category,
            t.fixture,
            t.memories.len(),
            t.tracers.len(),
            t.checks.len(),
            if t.example { " example" } else { "" }
        );
    }
    println!("noise rules={}", noise.len());
    if leaks.is_empty() {
        Ok(())
    } else {
        Err(leaks.join("\n"))
    }
}

/// Every file below `dir` that reads as text, by its path from `dir`.
fn read_files(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
        if entry.file_type()?.is_dir() {
            read_files(&entry.path(), &format!("{name}/"), out)?;
        } else if let Ok(text) = std::fs::read_to_string(entry.path()) {
            out.push((name, text));
        }
    }
    Ok(())
}

/// The task of the run and the noise every arm with memory is seeded with.
fn load_task(args: &RunArgs) -> Result<(task::Task, Vec<task::SeedMemory>), String> {
    let tasks = task::load(&args.data).map_err(|e| e.to_string())?;
    let noise = task::load_noise(&args.data, &tasks).map_err(|e| e.to_string())?;
    let task = task::find(tasks, &args.task).map_err(|e| e.to_string())?;
    check_shape(&task, args.shape)?;
    Ok((task, noise))
}

/// Why `task` cannot be played the way `shape` says, if it cannot.
fn check_shape(task: &task::Task, shape: Shape) -> Result<(), String> {
    shape.check(task.prompts().len())
}

fn prepare(args: &RunArgs, t: &task::Task, noise: Vec<task::SeedMemory>, client: &Client) -> Result<Layout, String> {
    let layout = Layout { root: std::path::absolute(&args.out).map_err(|e| format!("{}: {e}", args.out.display()))? };
    let agent_eval = std::env::current_exe().map_err(|e| format!("cannot locate agent-eval: {e}"))?;
    let options = Options {
        clawg: args.clawg.clone(),
        profile: args.profile,
        lang: args.lang,
        model: args.model.clone(),
        effort: args.effort.clone(),
        max_budget_usd: args.max_budget_usd,
        claude_version: args.claude_version.clone(),
        now_ms: now_ms(),
        noise,
        shape: args.shape,
        agent_eval,
    };
    run::prepare(t, args.arm, &task::fixture_dir(&args.data, t), &layout, &options, client).map_err(|e| e.to_string())?;
    Ok(layout)
}

/// The streams a replay of `sessions` sessions played as `shape` needs, by file name.
fn replay_streams(sessions: usize, shape: Shape) -> Vec<String> {
    if shape == Shape::Both {
        return ["session-1.jsonl", "joined-2.jsonl", "split-2.jsonl"].map(String::from).to_vec();
    }
    (1..=sessions).map(|k| format!("session-{k}.jsonl")).collect()
}

/// The absolute path of what to replay: a stream, for a task of one session, or
/// a directory that holds exactly the streams the sessions need.
fn replay_source(stream: &Path, sessions: usize, shape: Shape) -> Result<PathBuf, String> {
    let stream = std::path::absolute(stream).map_err(|e| format!("{}: {e}", stream.display()))?;
    let said = |why: String| format!("--replay {} {why}", stream.display());
    let meta = std::fs::metadata(&stream).map_err(|e| format!("--replay {}: {e}", stream.display()))?;
    let expected = replay_streams(sessions, shape);
    if meta.is_file() {
        return if sessions == 1 {
            Ok(stream)
        } else {
            Err(said(format!("is a file, and the task has {sessions} sessions: give a directory with {}", expected.join(", "))))
        };
    }
    let mut present = Vec::new();
    for entry in std::fs::read_dir(&stream).map_err(|e| format!("--replay {}: {e}", stream.display()))? {
        let entry = entry.map_err(|e| format!("--replay {}: {e}", stream.display()))?;
        present.push(entry.file_name().to_string_lossy().into_owned());
    }
    let lacks = expected.iter().filter(|name| !present.contains(name)).map(|name| format!("lacks {name}"));
    let extra = present.iter().filter(|name| !expected.contains(name)).map(|name| format!("holds {name}, which no session plays"));
    let problems: Vec<String> = lacks.chain(extra).collect();
    if problems.is_empty() { Ok(stream) } else { Err(said(problems.join("; "))) }
}

fn execute(command: Command) -> Result<(), String> {
    match command {
        Command::Tasks { data } => list_tasks(&data),
        Command::Prepare(args) => {
            let (t, noise) = load_task(&args)?;
            let layout = prepare(&args, &t, noise, &Client::Claude)?;
            println!("prepared {}", layout.root.display());
            Ok(())
        }
        Command::Run { args, replay } => {
            run::check_isolation(cfg!(windows), args.arm, &args.clawg).map_err(|e| e.to_string())?;
            let (t, noise) = load_task(&args)?;
            let client = match replay {
                Some(stream) => Client::Replay(replay_source(&stream, t.prompts().len(), args.shape)?),
                None => Client::Claude,
            };
            if let (Client::Claude, Some(version)) = (&client, &args.claude_version) {
                run::check_client_version(arm::CLIENT, version).map_err(|e| e.to_string())?;
            }
            if client == Client::Claude {
                run::check_paid_run(&args.out, &t).map_err(|e| e.to_string())?;
            }
            let layout = prepare(&args, &t, noise, &client)?;
            let exe = std::env::current_exe().map_err(|e| format!("cannot locate agent-eval: {e}"))?;
            if let Err(failed) = run::execute(&layout.root, &exe) {
                // The sessions that ran before the failure are still a result.
                if t.prompts().len() > 1 {
                    if let Ok(result) = run::score_dir(&layout.root) {
                        println!("{}", run::summary(&result));
                    }
                }
                return Err(failed.to_string());
            }
            let result = run::score_dir(&layout.root).map_err(|e| e.to_string())?;
            println!("{}", run::summary(&result));
            Ok(())
        }
        Command::Score { dir } => {
            let result = run::score_dir(&dir).map_err(|e| e.to_string())?;
            println!("{}", run::summary(&result));
            Ok(())
        }
        Command::Report { dir } => {
            let results = report::load(&dir).map_err(|e| e.to_string())?;
            print!("{}", report::render(&report::aggregate(&results)));
            Ok(())
        }
        Command::Costs { dir } => {
            let runs = report::load_named(&dir).map_err(|e| e.to_string())?;
            print!("{}", report::costs(&runs));
            Ok(())
        }
        Command::ReplayClient { stream, mcp_config } => {
            let cwd = std::env::current_dir().map_err(|e| format!("cannot read the current directory: {e}"))?;
            replay::run(&stream, &cwd, mcp_config.as_deref())
        }
        Command::HookContext { file } => {
            let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            println!("{}", hooks::hook_output(&text));
            Ok(())
        }
        Command::HookLog { file } => {
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input).map_err(|e| format!("cannot read the hook's input: {e}"))?;
            // One write per line, so hooks that run at once do not interleave.
            let mut log = std::fs::OpenOptions::new().create(true).append(true).open(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            log.write_all(hooks::log_line(&input).as_bytes()).map_err(|e| format!("{}: {e}", file.display()))
        }
    }
}

fn main() {
    let code = match parse_args(std::env::args()) {
        Ok(None) => {
            println!("{HELP_TEXT}");
            0
        }
        Err(e) => {
            eprintln!("agent-eval: {e}\n\n{HELP_TEXT}");
            2
        }
        Ok(Some(command)) => match execute(command) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("agent-eval: {e}");
                1
            }
        },
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        let owned: Vec<String> = std::iter::once("agent-eval").chain(list.iter().copied()).map(String::from).collect();
        owned.into_iter()
    }

    fn clawg() -> &'static str {
        if cfg!(windows) { r"C:\bin\clawg.exe" } else { "/bin/clawg" }
    }

    #[test]
    fn run_takes_its_options_with_their_defaults() {
        let parsed = parse_args(args(&["run", "--task", "t", "--arm", "a3-eager", "--out", "r", "--clawg", clawg(), "--model", "m", "--replay", "s.jsonl"]));
        let Ok(Some(Command::Run { args: run, replay })) = parsed else { panic!("{parsed:?}") };
        assert_eq!(replay, Some(PathBuf::from("s.jsonl")));
        assert_eq!((run.arm, run.profile, run.lang, run.effort, run.max_budget_usd), (Arm::A3Eager, Profile::Dev, None, None, None));
        assert_eq!((run.data, run.claude_version), (default_data(), None));

        let parsed = parse_args(args(&[
            "prepare", "--task", "t", "--arm", "b", "--out", "r", "--clawg", clawg(), "--model", "m", "--profile", "prod", "--lang", "es",
            "--effort", "high", "--max-budget-usd", "1.5", "--data", "d", "--claude-version", "2.1.300",
        ]));
        let Ok(Some(Command::Prepare(prepared))) = parsed else { panic!("{parsed:?}") };
        assert_eq!((prepared.profile, prepared.lang, prepared.max_budget_usd), (Profile::Prod, Some(Lang::Es), Some(1.5)));
        assert_eq!((prepared.effort.as_deref(), prepared.data), (Some("high"), PathBuf::from("d")));
        assert_eq!(prepared.claude_version.as_deref(), Some("2.1.300"));
    }

    #[test]
    fn a_run_without_a_replay_starts_claude_only_with_a_budget_and_a_pinned_version() {
        let base = ["run", "--task", "t", "--arm", "a3", "--out", "r", "--clawg", clawg(), "--model", "m"];
        let with = |extra: &[&str]| parse_args(args(&[&base[..], extra].concat()));
        assert!(with(&[]).unwrap_err().contains("--max-budget-usd"));
        assert!(with(&["--claude-version", "2.1.300"]).unwrap_err().contains("--max-budget-usd"));
        assert!(with(&["--max-budget-usd", "1.5"]).unwrap_err().contains("--claude-version"));

        let parsed = with(&["--max-budget-usd", "1.5", "--claude-version", "2.1.300", "--lang", "es"]);
        let Ok(Some(Command::Run { args: run, replay: None })) = parsed else { panic!("{parsed:?}") };
        assert_eq!((run.max_budget_usd, run.claude_version.as_deref(), run.lang), (Some(1.5), Some("2.1.300"), Some(Lang::Es)));
        assert_eq!(run.model, "m");
    }

    #[test]
    fn the_other_commands_parse() {
        assert_eq!(parse_args(args(&["tasks"])), Ok(Some(Command::Tasks { data: default_data() })));
        assert_eq!(parse_args(args(&["score", "--run", "r"])), Ok(Some(Command::Score { dir: "r".into() })));
        assert_eq!(parse_args(args(&["replay-client", "s"])), Ok(Some(Command::ReplayClient { stream: "s".into(), mcp_config: None })));
        assert_eq!(
            parse_args(args(&["replay-client", "s", "--mcp-config", "m.json"])),
            Ok(Some(Command::ReplayClient { stream: "s".into(), mcp_config: Some("m.json".into()) }))
        );
        assert!(parse_args(args(&["replay-client", "s", "--other", "m"])).unwrap_err().contains("--mcp-config"));
        assert_eq!(parse_args(args(&["report", "--runs", "d"])), Ok(Some(Command::Report { dir: "d".into() })));
        assert_eq!(parse_args(args(&["costs", "--runs", "d"])), Ok(Some(Command::Costs { dir: "d".into() })));
        assert!(parse_args(args(&["costs"])).unwrap_err().contains("--runs is required"));
        assert!(parse_args(args(&["report"])).unwrap_err().contains("--runs is required"));
        assert!(parse_args(args(&["report", "--run", "d"])).unwrap_err().contains("unknown argument --run"));
        assert_eq!(parse_args(args(&["--help"])), Ok(None));
        assert_eq!(parse_args(args(&["run", "-h"])), Ok(None));
    }

    #[test]
    fn the_shape_of_a_run_parses_and_is_refused_where_the_task_cannot_take_it() {
        let base = ["prepare", "--task", "t", "--arm", "a3", "--out", "r", "--clawg", clawg(), "--model", "m"];
        let with = |extra: &[&str]| parse_args(args(&[&base[..], extra].concat()));
        let shape = |extra: &[&str]| match with(extra) {
            Ok(Some(Command::Prepare(run))) => run.shape,
            other => panic!("{other:?}"),
        };
        assert_eq!(shape(&[]), Shape::Split);
        assert_eq!(shape(&["--sessions", "joined"]), Shape::Joined);
        assert_eq!(shape(&["--sessions", "both"]), Shape::Both);
        assert!(with(&["--sessions", "sideways"]).unwrap_err().contains("unknown shape sideways"));
        assert!(HELP_TEXT.contains("--sessions split|joined|both"));

        let one = task::parse(r#"{"id": "t", "category": "K1", "fixture": "f", "prompt": "p", "checks": [{"kind": "answer_contains", "text": "x"}]}"#).unwrap();
        assert_eq!(check_shape(&one, Shape::Split), Ok(()));
        for refused in [Shape::Joined, Shape::Both] {
            assert!(check_shape(&one, refused).unwrap_err().contains("one session"), "{refused:?}");
        }
    }

    #[test]
    fn the_hook_commands_parse() {
        assert_eq!(parse_args(args(&["hook-context", "f.md"])), Ok(Some(Command::HookContext { file: "f.md".into() })));
        assert_eq!(parse_args(args(&["hook-log", "l.jsonl"])), Ok(Some(Command::HookLog { file: "l.jsonl".into() })));
        assert!(parse_args(args(&["hook-log"])).unwrap_err().contains("hook-log takes one file"));
        assert!(parse_args(args(&["hook-context", "a", "b"])).unwrap_err().contains("hook-context takes one file"));
    }

    #[test]
    fn a_replay_is_a_stream_or_a_directory_with_one_per_session() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        std::fs::write(&file, "").unwrap();
        assert_eq!(replay_source(&file, 1, Shape::Split), Ok(file.clone()));
        let said = replay_source(&file, 2, Shape::Split).unwrap_err();
        assert!(said.contains("--replay") && said.contains("2 sessions") && said.contains("session-1.jsonl"), "{said}");

        let streams = dir.path().join("streams");
        std::fs::create_dir(&streams).unwrap();
        let put = |name: &str| std::fs::write(streams.join(name), "").unwrap();
        put("session-1.jsonl");
        let said = replay_source(&streams, 2, Shape::Joined).unwrap_err();
        assert!(said.contains("--replay") && said.contains("lacks session-2.jsonl"), "{said}");
        put("session-2.jsonl");
        assert_eq!(replay_source(&streams, 2, Shape::Joined), Ok(streams.clone()));
        put("session-3.jsonl");
        let said = replay_source(&streams, 2, Shape::Split).unwrap_err();
        assert!(said.contains("holds session-3.jsonl, which no session plays"), "{said}");
        assert_eq!(replay_source(&streams, 3, Shape::Split), Ok(streams.clone()));

        assert_eq!(replay_streams(2, Shape::Split), ["session-1.jsonl", "session-2.jsonl"]);
        assert_eq!(replay_streams(1, Shape::Split), ["session-1.jsonl"]);
        assert_eq!(replay_streams(2, Shape::Both), ["session-1.jsonl", "joined-2.jsonl", "split-2.jsonl"]);
        let missing = dir.path().join("missing");
        assert!(replay_source(&missing, 1, Shape::Split).unwrap_err().contains(&format!("--replay {}", missing.display())));
    }

    #[test]
    fn run_refuses_a_replay_that_is_not_a_file_before_laying_out_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("run");
        let args = RunArgs {
            data: default_data(),
            task: "example-count-lines".into(),
            arm: Arm::A0,
            out: out.clone(),
            clawg: clawg().into(),
            model: "m".into(),
            effort: None,
            max_budget_usd: None,
            claude_version: None,
            profile: Profile::Dev,
            lang: None,
            shape: Shape::Split,
        };
        for replay in [dir.path().join("missing.jsonl"), dir.path().to_path_buf()] {
            let err = execute(Command::Run { args: args.clone(), replay: Some(replay.clone()) }).unwrap_err();
            assert!(err.contains(&format!("--replay {}", replay.display())), "{err}");
            assert!(!out.exists(), "the run was laid out for {}", replay.display());
        }
    }

    #[test]
    fn bad_arguments_are_refused_with_why() {
        let base = ["prepare", "--task", "t", "--out", "r", "--model", "m"];
        let with = |extra: &[&str]| parse_args(args(&[&base[..], extra].concat())).unwrap_err();
        assert!(with(&["--arm", "a1", "--clawg", clawg()]).contains("unknown arm a1"));
        assert!(with(&["--arm", "a0", "--clawg", "clawg"]).contains("not an absolute path"));
        assert!(with(&["--arm", "a0", "--clawg", clawg(), "--profile", "test"]).contains("unknown profile"));
        assert!(with(&["--arm", "a0", "--clawg", clawg(), "--lang", "fr"]).contains("unknown language fr"));
        assert!(with(&["--arm", "a0", "--clawg", clawg(), "--max-budget-usd", "-1"]).contains("not a positive number"));
        assert!(with(&["--arm", "a0", "--clawg", clawg(), "--replay", "s"]).contains("unknown argument --replay"));
        assert!(with(&["--arm", "a0", "--arm", "a3", "--clawg", clawg()]).contains("given twice"));
        assert!(with(&["--arm", "a0", "--clawg"]).contains("needs a value"));
        assert!(with(&["--clawg", clawg()]).contains("--arm is required"));
        assert!(parse_args(args(&[])).unwrap_err().contains("name a command"));
        assert!(parse_args(args(&["launch"])).unwrap_err().contains("unknown command"));
        assert!(parse_args(args(&["replay-client"])).unwrap_err().contains("one stream file"));
        assert!(parse_args(args(&["tasks", "extra"])).unwrap_err().contains("unexpected argument"));
    }
}
