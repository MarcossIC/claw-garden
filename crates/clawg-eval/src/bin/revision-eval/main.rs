//! Measures what a `topic_key` does for a memory that is revised: how often an
//! old version stays live without a mark when the save names no key, whether
//! a key hides every old version, and what showing the key costs.
//!
//! `--real` needs the embedding model already cached in the data profile;
//! `--fake` runs the same pipeline on the deterministic double, which checks
//! the plumbing and never the numbers.

mod chains;
mod cost;
mod run;
mod set;
mod stats;

use std::path::{Path, PathBuf};

use clawg_core::eval_api::{model_is_cached, EvalEmbedder, EvalTokenCounter};
use serde::Serialize;

const EXIT_OK: i32 = 0;
const EXIT_RUNTIME_FAILURE: i32 = 1;
const EXIT_INPUT_INVALID: i32 = 2;
const EXIT_MODEL_ABSENT: i32 = 3;

const HELP_TEXT: &str = "\
revision-eval chains (--real | --fake) [--tag SLUG]

  chains   saves crates/clawg-eval/data/revision-chains.json in three arms
           (without a key, with its key, and only the last version), each
           after the MUST sentences of openspec/specs, and classifies every
           transition of the arm without a key
  --real   the cached embedding model of the data profile
  --fake   the deterministic double: checks the pipeline, not the numbers
  --tag    with --real, names a second run of the day:
           docs/eval/results/{date}-real-revision-eval-chains-{SLUG}.json

Prints the JSON report; --real also writes it to docs/eval/results, never over
a file that is already there.
Exit 0 measured, 1 failed, 2 bad input, 3 model not cached.";

#[derive(Debug, Clone, PartialEq)]
enum Command {
    Chains { real: bool, tag: Option<String> },
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Command>, String> {
    let mut args = args.skip(1);
    match args.next().as_deref() {
        Some("--help" | "-h") => return Ok(None),
        Some("chains") => {}
        Some(other) => return Err(format!("unknown command {other}")),
        None => return Err("name a command".into()),
    }
    let mut real = None;
    let mut tag = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--real" | "--fake" if real.is_some() => return Err("pass one of --real and --fake".into()),
            "--real" => real = Some(true),
            "--fake" => real = Some(false),
            "--tag" if tag.is_some() => return Err("pass --tag once".into()),
            "--tag" => tag = Some(args.next().filter(|t| is_slug(t)).ok_or("--tag takes a slug of a-z, 0-9 and '-'")?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let real = real.ok_or("pass --real or --fake")?;
    if tag.is_some() && !real {
        return Err("--tag names a file, and only --real writes one".into());
    }
    Ok(Some(Command::Chains { real, tag }))
}

fn is_slug(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The name of a run's result file, as `rag-eval` names its own.
fn results_filename(date: &str, tag: Option<&str>) -> String {
    match tag {
        Some(tag) => format!("{date}-real-revision-eval-chains-{tag}.json"),
        None => format!("{date}-real-revision-eval-chains.json"),
    }
}

/// What a result file holds: the day, the clawg that measured and the whole
/// report, which names the model contract and the set it measured.
#[derive(Serialize)]
struct ResultFile<'a> {
    date: &'a str,
    clawg_version: &'static str,
    #[serde(flatten)]
    report: &'a run::ChainsReport,
}

fn result_json(date: &str, report: &run::ChainsReport) -> String {
    let file = ResultFile { date, clawg_version: env!("CARGO_PKG_VERSION"), report };
    serde_json::to_string_pretty(&file).expect("the report always serializes")
}

/// Writes `text` at `path` unless a file is already there: a rerun must not
/// replace the baseline it is compared with.
fn write_new(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    writeln!(file, "{text}")
}

/// Howard Hinnant's `civil_from_days`, as `rag-eval` carries it: this binary
/// has no date crate for one stamp.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The current UTC date as `YYYY-MM-DD`.
fn current_date() -> String {
    let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let (y, m, d) = civil_from_days((seconds / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    std::process::exit(run(std::env::args()).await);
}

async fn run(args: impl Iterator<Item = String>) -> i32 {
    let (real, tag) = match parse_args(args) {
        Ok(Some(Command::Chains { real, tag })) => (real, tag),
        Ok(None) => {
            println!("{HELP_TEXT}");
            return EXIT_OK;
        }
        Err(e) => {
            eprintln!("error: {e}\n\n{HELP_TEXT}");
            return EXIT_RUNTIME_FAILURE;
        }
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = root.join("data/revision-chains.json");
    let set = match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| set::parse_set(&t)) {
        Ok(set) => set,
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            return EXIT_INPUT_INVALID;
        }
    };
    let specs = root.join("../../openspec/specs");
    let distractors = match run::read_distractors(&specs) {
        Ok(distractors) if !distractors.is_empty() => distractors,
        Ok(_) => {
            eprintln!("{}: no MUST sentence to save as a distractor", specs.display());
            return EXIT_INPUT_INVALID;
        }
        Err(e) => {
            eprintln!("{}: {e}", specs.display());
            return EXIT_INPUT_INVALID;
        }
    };

    // Checked before the run: a real one takes minutes of model time.
    let date = current_date();
    let result_path: Option<PathBuf> =
        real.then(|| root.join("../../docs/eval/results").join(results_filename(&date, tag.as_deref())));
    if let Some(path) = result_path.as_ref().filter(|p| p.exists()) {
        eprintln!("{} already exists; pass --tag to name this run", path.display());
        return EXIT_INPUT_INVALID;
    }

    let base_dir = match clawg_core::paths::data_dir_from_env() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("failed to resolve the data directory: {e}");
            return EXIT_RUNTIME_FAILURE;
        }
    };
    let embedder = if real {
        if !model_is_cached(&base_dir) {
            eprintln!("the model is not cached under {}; run rag-eval --real once to download it", base_dir.display());
            return EXIT_MODEL_ABSENT;
        }
        match EvalEmbedder::real(&base_dir).await {
            Ok(e) => e,
            Err(e) => {
                eprintln!("failed to load the real embedding model: {e}");
                return EXIT_RUNTIME_FAILURE;
            }
        }
    } else {
        EvalEmbedder::fake()
    };
    let counter = if real {
        match EvalTokenCounter::real(&base_dir) {
            Ok(counter) => counter,
            Err(e) => {
                eprintln!("failed to load the tokenizer: {e}");
                return EXIT_RUNTIME_FAILURE;
            }
        }
    } else {
        EvalTokenCounter::fake()
    };
    let scratch = run::scratch_dir();
    // Under --fake the chunker must not find a tokenizer, and the scratch
    // directory holds none.
    let tokenizer_dir = if real { base_dir.clone() } else { scratch.clone() };
    let input = run::ChainsInput {
        set: &set,
        distractors: &distractors,
        embedder: &embedder,
        tokenizer_dir: &tokenizer_dir,
        counter: &counter,
    };

    match run::run(&input, &scratch).await {
        Ok(report) => {
            let json = result_json(&date, &report);
            println!("{json}");
            let Some(path) = result_path else {
                return EXIT_OK;
            };
            match write_new(&path, &json) {
                Ok(()) => {
                    eprintln!("wrote {}", path.display());
                    EXIT_OK
                }
                Err(e) => {
                    eprintln!("could not write {}: {e}", path.display());
                    EXIT_RUNTIME_FAILURE
                }
            }
        }
        Err(e) => {
            eprintln!("measurement failed: {e}");
            EXIT_RUNTIME_FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> std::vec::IntoIter<String> {
        std::iter::once("revision-eval".to_string()).chain(list.iter().map(|s| s.to_string())).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn the_arguments_parse() {
        assert_eq!(parse_args(args(&["chains", "--fake"])).unwrap(), Some(Command::Chains { real: false, tag: None }));
        assert_eq!(parse_args(args(&["chains", "--real"])).unwrap(), Some(Command::Chains { real: true, tag: None }));
        assert_eq!(
            parse_args(args(&["chains", "--tag", "second-run", "--real"])).unwrap(),
            Some(Command::Chains { real: true, tag: Some("second-run".into()) })
        );
        assert_eq!(parse_args(args(&["--help"])).unwrap(), None);
        assert_eq!(parse_args(args(&["chains", "--help"])).unwrap(), None);
        for bad in [
            &[][..],
            &["chains"],
            &["chains", "--real", "--fake"],
            &["store"],
            &["chains", "--fake", "--out"],
            &["chains", "--fake", "--tag", "x"],
            &["chains", "--real", "--tag"],
            &["chains", "--real", "--tag", "Bad Tag"],
        ] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_result_file_is_named_by_date_and_tag() {
        assert_eq!(results_filename("2026-10-21", None), "2026-10-21-real-revision-eval-chains.json");
        assert_eq!(results_filename("2026-10-21", Some("again")), "2026-10-21-real-revision-eval-chains-again.json");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
    }

    #[test]
    fn a_result_is_never_written_over_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("revision-eval-main-{}", uuid::Uuid::new_v4()));
        let path = dir.join("results").join("out.json");
        write_new(&path, "first").unwrap();
        let again = write_new(&path, "second").unwrap_err();
        let kept = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(again.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(kept.trim(), "first");
    }

    #[tokio::test]
    async fn a_result_header_names_the_model_contract_and_the_set_hash() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let set = set::parse_set(&std::fs::read_to_string(root.join("data/revision-chains.json")).unwrap()).unwrap();
        let distractors = run::read_distractors(&root.join("../../openspec/specs")).unwrap();
        let embedder = EvalEmbedder::fake();
        let counter = EvalTokenCounter::fake();
        let scratch = run::scratch_dir();
        let input = run::ChainsInput {
            set: &set,
            distractors: &distractors,
            embedder: &embedder,
            tokenizer_dir: &scratch,
            counter: &counter,
        };
        let report = run::run(&input, &scratch).await.expect("the fake run must succeed");

        let result: serde_json::Value = serde_json::from_str(&result_json("2026-10-21", &report)).unwrap();

        assert_eq!(result["date"], "2026-10-21");
        assert_eq!(result["clawg_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(result["model_contract"], embedder.model_id());
        assert_eq!(result["set_version"], set.version);
        assert_eq!(result["set_sha256"], set.sha256.as_str());
        assert_eq!(result["distractors"], distractors.len());
        let arms: Vec<&str> = result["arms"].as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(arms, ["control", "keyed", "no_key"]);
    }
}
