mod arm;
mod replay;
mod rubric;
mod run;
mod score;
mod seed;
mod stream;
mod task;

use std::path::{Path, PathBuf};

use clawg_core::paths::Profile;
use clawg_core::preferences::Lang;

use crate::arm::{Arm, Layout, Options, Step};

const HELP_TEXT: &str = "\
agent-eval tasks [--data DIR]
agent-eval prepare RUN_OPTIONS
agent-eval run RUN_OPTIONS --replay STREAM
agent-eval score --run DIR
agent-eval replay-client STREAM

  tasks          reads every task in DIR/tasks/*.json, checks each one and
                 that no tracer is in a file of its fixture, and lists them.
  prepare        lays out one run of a task under an arm in --out: the
                 fixture copied as the working directory, the client's
                 configuration, the seed and run.json with the command line
                 of the client session. Starts nothing.
  run            prepares the run, then starts the replay client in its
                 working directory instead of a real one: it prints STREAM,
                 a recorded stream-json, and makes the file changes its
                 Write and Edit calls record. The stream is captured in
                 stream.jsonl and scored into result.json. No model is
                 called.
  score          scores a run directory again from its stream.jsonl.
  replay-client  the client a dry run starts: prints STREAM after making
                 its file changes in the current directory.

RUN_OPTIONS:
  --task ID --arm a0|a2|a3|a3-eager|a4|b --out DIR --clawg PATH --model M
  [--effort E] [--max-budget-usd N] [--profile dev|prod] [--lang en|es]
  [--data DIR]

  --clawg is the absolute path of the clawg binary the arms with clawg run;
  --profile (dev by default) picks its data directory under the run's home.
  --data defaults to this crate's data/agent-tasks.

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
    profile: Profile,
    lang: Option<Lang>,
}

#[derive(Debug, Clone, PartialEq)]
enum Command {
    Tasks { data: PathBuf },
    Prepare(RunArgs),
    Run { args: RunArgs, replay: PathBuf },
    Score { dir: PathBuf },
    ReplayClient { stream: PathBuf },
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

const RUN_FLAGS: [&str; 10] = ["task", "arm", "out", "clawg", "model", "effort", "max-budget-usd", "profile", "lang", "data"];

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
    Ok(RunArgs {
        data: take(f, "data").map_or_else(default_data, PathBuf::from),
        task: required(f, "task")?,
        arm,
        out: PathBuf::from(required(f, "out")?),
        clawg,
        model: required(f, "model")?,
        effort: take(f, "effort"),
        max_budget_usd,
        profile,
        lang,
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
            let replay = PathBuf::from(required(&mut f, "replay")?);
            Command::Run { args: run_args(&mut f)?, replay }
        }
        "score" => Command::Score { dir: PathBuf::from(required(&mut flags(rest, &["run"])?, "run")?) },
        "replay-client" => {
            let [stream]: [String; 1] = rest.collect::<Vec<_>>().try_into().map_err(|_| "replay-client takes one stream file")?;
            Command::ReplayClient { stream: PathBuf::from(stream) }
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

fn prepare(args: &RunArgs) -> Result<(arm::RunPlan, Layout), String> {
    let tasks = task::load(&args.data).map_err(|e| e.to_string())?;
    let t = task::find(tasks, &args.task).map_err(|e| e.to_string())?;
    let layout = Layout { root: std::path::absolute(&args.out).map_err(|e| format!("{}: {e}", args.out.display()))? };
    let options = Options {
        clawg: args.clawg.clone(),
        profile: args.profile,
        lang: args.lang,
        model: args.model.clone(),
        effort: args.effort.clone(),
        max_budget_usd: args.max_budget_usd,
        now_ms: now_ms(),
    };
    let plan = run::prepare(&t, args.arm, &task::fixture_dir(&args.data, &t), &layout, &options).map_err(|e| e.to_string())?;
    Ok((plan, layout))
}

fn execute(command: Command) -> Result<(), String> {
    match command {
        Command::Tasks { data } => list_tasks(&data),
        Command::Prepare(args) => {
            let (_, layout) = prepare(&args)?;
            println!("prepared {}", layout.root.display());
            Ok(())
        }
        Command::Run { args, replay } => {
            let replay = std::path::absolute(&replay).map_err(|e| format!("{}: {e}", replay.display()))?;
            let (plan, layout) = prepare(&args)?;
            let exe = std::env::current_exe().map_err(|e| format!("cannot locate agent-eval: {e}"))?;
            let client = Step {
                program: exe.to_string_lossy().into_owned(),
                args: vec!["replay-client".into(), replay.to_string_lossy().into_owned()],
            };
            let code = run::start_client(&plan, &layout, &client).map_err(|e| e.to_string())?;
            if code != Some(0) {
                return Err(format!(
                    "the replay client exited with {code:?}; see {}",
                    layout.root.join(run::CLIENT_LOG).display()
                ));
            }
            let result = run::score_dir(&layout.root, "replay").map_err(|e| e.to_string())?;
            println!("{}", run::summary(&result));
            Ok(())
        }
        Command::Score { dir } => {
            let result = run::score_dir(&dir, "replay").map_err(|e| e.to_string())?;
            println!("{}", run::summary(&result));
            Ok(())
        }
        Command::ReplayClient { stream } => {
            let cwd = std::env::current_dir().map_err(|e| format!("cannot read the current directory: {e}"))?;
            replay::run(&stream, &cwd)
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
        assert_eq!(replay, PathBuf::from("s.jsonl"));
        assert_eq!((run.arm, run.profile, run.lang, run.effort, run.max_budget_usd), (Arm::A3Eager, Profile::Dev, None, None, None));
        assert_eq!(run.data, default_data());

        let parsed = parse_args(args(&[
            "prepare", "--task", "t", "--arm", "b", "--out", "r", "--clawg", clawg(), "--model", "m", "--profile", "prod", "--lang", "es",
            "--effort", "high", "--max-budget-usd", "1.5", "--data", "d",
        ]));
        let Ok(Some(Command::Prepare(prepared))) = parsed else { panic!("{parsed:?}") };
        assert_eq!((prepared.profile, prepared.lang, prepared.max_budget_usd), (Profile::Prod, Some(Lang::Es), Some(1.5)));
        assert_eq!((prepared.effort.as_deref(), prepared.data), (Some("high"), PathBuf::from("d")));
    }

    #[test]
    fn the_other_commands_parse() {
        assert_eq!(parse_args(args(&["tasks"])), Ok(Some(Command::Tasks { data: default_data() })));
        assert_eq!(parse_args(args(&["score", "--run", "r"])), Ok(Some(Command::Score { dir: "r".into() })));
        assert_eq!(parse_args(args(&["replay-client", "s"])), Ok(Some(Command::ReplayClient { stream: "s".into() })));
        assert_eq!(parse_args(args(&["--help"])), Ok(None));
        assert_eq!(parse_args(args(&["run", "-h"])), Ok(None));
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
        assert!(parse_args(args(&["run", "--task", "t"])).unwrap_err().contains("--replay is required"));
        assert!(parse_args(args(&[])).unwrap_err().contains("name a command"));
        assert!(parse_args(args(&["launch"])).unwrap_err().contains("unknown command"));
        assert!(parse_args(args(&["replay-client"])).unwrap_err().contains("one stream file"));
        assert!(parse_args(args(&["tasks", "extra"])).unwrap_err().contains("unexpected argument"));
    }
}
