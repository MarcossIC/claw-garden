mod gate;
mod measure;
mod plan;

use std::path::PathBuf;

use serde::Serialize;

const HELP_TEXT: &str = "\
load-eval gate [--model-dir D] [--loads 10] [--tag SLUG]
load-eval child --arm vec|file|view --model-dir D

  gate   downloads the embedding model into a load-eval-<uuid> directory of
         the system temp dir (or uses the one --model-dir names, which must
         be such a directory), loads it once per process in three arms (vec,
         file, view), one warm-up round and --loads counted rounds in rotated
         order, and decides. The directory is removed at the end, failed or
         not; a data profile is never read.
  child  one load; run by gate, which assigns it to a job object first

On Windows the peak is the job's peak commit and the run decides View, File
or Revert; elsewhere it is VmHWM and the run reports without deciding.
Prints the JSON report and writes docs/eval/results/{date}-real-load-eval
[-SLUG].json, never over a file already there.
Exit 0 measured, 1 failed.";

#[derive(Debug, PartialEq)]
enum Command {
    Gate { model_dir: Option<PathBuf>, loads: usize, tag: Option<String> },
    Child { arm: plan::Arm, model_dir: PathBuf },
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Command>, String> {
    let mut args = args.skip(1);
    let kind = args.next().ok_or("name a command")?;
    if kind == "--help" || kind == "-h" {
        return Ok(None);
    }
    let mut model_dir = None;
    let mut loads = 10usize;
    let mut tag = None;
    let mut arm = None;
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match (kind.as_str(), arg.as_str()) {
            (_, "--help" | "-h") => return Ok(None),
            (_, "--model-dir") => model_dir = Some(PathBuf::from(value("--model-dir")?)),
            ("gate", "--loads") => {
                loads = value("--loads")?.parse().ok().filter(|n| *n > 0).ok_or("--loads is a number above 0")?
            }
            ("gate", "--tag") => {
                let t = value("--tag")?;
                if !gate::is_slug(&t) {
                    return Err("--tag is lowercase letters, digits and hyphens".into());
                }
                tag = Some(t);
            }
            ("child", "--arm") => {
                let a = value("--arm")?;
                arm = Some(plan::Arm::parse(&a).ok_or(format!("unknown arm {a}"))?);
            }
            (_, other) => return Err(format!("unknown argument {other}")),
        }
    }
    match kind.as_str() {
        "gate" => Ok(Some(Command::Gate { model_dir, loads, tag })),
        "child" => Ok(Some(Command::Child {
            arm: arm.ok_or("child needs --arm")?,
            model_dir: model_dir.ok_or("child needs --model-dir")?,
        })),
        other => Err(format!("unknown command {other}")),
    }
}

#[derive(Serialize)]
struct SampleRow {
    round: usize,
    arm: plan::Arm,
    #[serde(flatten)]
    load: measure::Load,
}

#[derive(Serialize)]
struct Report {
    date: String,
    clawg_version: &'static str,
    os: &'static str,
    loads: usize,
    /// Whether this system's peak is one the gate may decide on.
    decides: bool,
    decision: Option<plan::Decision>,
    arms: Vec<plan::ArmSummary>,
    samples: Vec<SampleRow>,
}

fn fetch(dir: &std::path::Path) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    runtime.block_on(clawg_core::eval_api::fetch_model(dir)).map_err(|e| e.to_string())
}

fn gate(model_dir: Option<PathBuf>, loads: usize, tag: Option<String>) -> Result<(), String> {
    let date = gate::current_date();
    let result_path = gate::results_dir().join(gate::results_filename(&date, tag.as_deref()));
    if result_path.exists() {
        return Err(format!("{} already exists; pass --tag to name this run", result_path.display()));
    }
    let decides = cfg!(windows);
    let report = gate::with_model_dir(model_dir, &std::env::temp_dir(), |dir| {
        eprintln!("fetching the model into {}", dir.display());
        fetch(dir)?;
        let mut samples = Vec::new();
        let mut rows = Vec::new();
        for slot in plan::schedule(loads) {
            let load = measure::measure(slot.arm, dir)?;
            eprintln!(
                "round {} {}: peak {} MiB, verify {} ms, session {} ms{}",
                slot.round,
                slot.arm.name(),
                load.peak_bytes >> 20,
                load.child.verify_ms,
                load.child.session_ms,
                if slot.counted { "" } else { " (warm-up)" }
            );
            if slot.counted {
                samples.push(plan::Sample {
                    arm: slot.arm,
                    peak_bytes: load.peak_bytes,
                    total_ms: load.child.verify_ms + load.child.session_ms,
                });
                rows.push(SampleRow { round: slot.round, arm: slot.arm, load });
            }
        }
        Ok(Report {
            date: date.clone(),
            clawg_version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            loads,
            decides,
            decision: plan::verdict(&samples, decides),
            arms: plan::summarize(&samples),
            samples: rows,
        })
    })?;
    let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    println!("{text}");
    gate::write_new(&result_path, &text).map_err(|e| format!("cannot write {}: {e}", result_path.display()))?;
    eprintln!("wrote {}", result_path.display());
    Ok(())
}

fn main() {
    let code = match parse_args(std::env::args()) {
        Ok(None) => {
            println!("{HELP_TEXT}");
            0
        }
        Ok(Some(Command::Gate { model_dir, loads, tag })) => finish(gate(model_dir, loads, tag)),
        Ok(Some(Command::Child { arm, model_dir })) => finish(measure::child(arm, &model_dir)),
        Err(e) => {
            eprintln!("error: {e}\n\n{HELP_TEXT}");
            1
        }
    };
    std::process::exit(code);
}

fn finish(outcome: Result<(), String>) -> i32 {
    match outcome {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        std::iter::once("load-eval".to_string()).chain(list.iter().map(|s| s.to_string())).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn the_gate_defaults_to_ten_loads() {
        assert_eq!(parse_args(args(&["gate"])), Ok(Some(Command::Gate { model_dir: None, loads: 10, tag: None })));
    }

    #[test]
    fn the_gate_takes_a_dir_a_count_and_a_tag() {
        let parsed = parse_args(args(&["gate", "--model-dir", "d", "--loads", "4", "--tag", "again"]));
        assert_eq!(
            parsed,
            Ok(Some(Command::Gate { model_dir: Some(PathBuf::from("d")), loads: 4, tag: Some("again".into()) }))
        );
    }

    #[test]
    fn the_child_needs_its_arm_and_its_dir() {
        assert_eq!(
            parse_args(args(&["child", "--arm", "view", "--model-dir", "d"])),
            Ok(Some(Command::Child { arm: plan::Arm::View, model_dir: PathBuf::from("d") }))
        );
        assert!(parse_args(args(&["child", "--arm", "view"])).is_err());
        assert!(parse_args(args(&["child", "--model-dir", "d"])).is_err());
        assert!(parse_args(args(&["child", "--arm", "mmap", "--model-dir", "d"])).is_err());
    }

    #[test]
    fn nonsense_is_refused() {
        assert!(parse_args(args(&[])).is_err());
        assert!(parse_args(args(&["measure"])).is_err());
        assert!(parse_args(args(&["gate", "--loads", "0"])).is_err());
        assert!(parse_args(args(&["gate", "--loads", "many"])).is_err());
        assert!(parse_args(args(&["gate", "--tag", "Bad Tag"])).is_err());
        assert!(parse_args(args(&["gate", "--what"])).is_err());
        assert!(parse_args(args(&["gate", "--arm", "view"])).is_err());
    }
}
