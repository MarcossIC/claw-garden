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
mod store;
mod warned;

use std::path::{Path, PathBuf};

use clawg_core::eval_api::{model_is_cached, EvalEmbedder, EvalTokenCounter};
use serde::Serialize;

const EXIT_OK: i32 = 0;
const EXIT_RUNTIME_FAILURE: i32 = 1;
const EXIT_INPUT_INVALID: i32 = 2;
const EXIT_MODEL_ABSENT: i32 = 3;

const HELP_TEXT: &str = "\
revision-eval chains (--real | --fake) [--tag SLUG] [--background FILE]
revision-eval store --export FILE [--seed N] (--real | --fake)
revision-eval store --labels FILE [--tag SLUG]
revision-eval warned --export FILE --since SNAPSHOT [--seed N] (--real | --fake)
revision-eval warned --labels FILE [--tag SLUG]

  chains   saves crates/clawg-eval/data/revision-chains.json in five arms
           (without a key; with its key; with its key and without the
           vectors of the replaced versions; with its key and the replaced
           versions in no index; and only the last version), each after the
           MUST sentences of openspec/specs, and classifies every transition
           of the arm without a key
             --background  first saves in every arm, under the arm's agent
                           and project, the live saves of a `clawg export`
                           file, in the order they were made and with their
                           keys behind bg/; the export must sit in a
                           revision-eval-* directory made directly under the
                           system temp dir, which is removed at the end
  store    measures, on a `clawg export --include-deleted` file, how often a
           save without a key revises an earlier one, in two steps:
             --export  replays the saves through the detector and leaves
                       labels.jsonl and replay.json beside the export
             --labels  scores the labels the owner filled in, prints and
                       writes the result, and removes the working directory
           The export must sit in a revision-eval-* directory made directly
           under the system temp dir, which holds every file with stored text
           and is removed at the end, and on any error but a labels file that
           is not finished yet.
  warned   measures, on a `clawg export --include-deleted` file (no --project
           or --agent), the topic-key warning of `save_memory`, in two steps:
             --export  takes the pairs the warning marked in the saves made
                       from SNAPSHOT on and a sample of the saves it did not
                       fire on, replays every save to pair them, and leaves
                       labels.jsonl and warned.json beside the export
             --labels  scores the three passes the owner filled in (`a` and
                       `b`, and `c` where they differ), prints and writes the
                       result, and removes the working directory
           --since is the name of the pre-migration snapshot (memory-<UTC>-
           pre-migration.db) that the opening which migrated the store wrote;
           a path to it is taken by its file name. The export follows the
           same rules as for store.
  --real   the cached embedding model of the data profile
  --fake   the deterministic double: checks the pipeline, not the numbers
  --seed   the seed of the sample of labelled pairs
  --tag    names a second run of the day:
           docs/eval/results/{date}-real-revision-eval-{chains|store|warned}-{SLUG}.json

Prints the JSON report; --real (and `store --labels` and `warned --labels`
after a --real replay) also writes it to docs/eval/results, never over a file
that is already there.
Exit 0 measured, 1 failed, 2 bad input, 3 model not cached.";

#[derive(Debug, Clone, PartialEq)]
enum Command {
    Chains { real: bool, tag: Option<String>, background: Option<PathBuf> },
    StoreReplay { export: PathBuf, seed: u64, real: bool },
    StoreScore { labels: PathBuf, tag: Option<String> },
    WarnedReplay { export: PathBuf, since: String, seed: u64, real: bool },
    WarnedScore { labels: PathBuf, tag: Option<String> },
}

/// The commands, which `store` and `warned` share the shape of: a first step
/// that reads an export and a second that scores the labels.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Chains,
    Store,
    Warned,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Command>, String> {
    let mut args = args.skip(1);
    let kind = match args.next().as_deref() {
        Some("--help" | "-h") => return Ok(None),
        Some("chains") => Kind::Chains,
        Some("store") => Kind::Store,
        Some("warned") => Kind::Warned,
        Some(other) => return Err(format!("unknown command {other}")),
        None => return Err("name a command".into()),
    };
    let two_steps = kind != Kind::Chains;
    let mut real = None;
    let mut tag = None;
    let mut export: Option<PathBuf> = None;
    let mut labels: Option<PathBuf> = None;
    let mut background: Option<PathBuf> = None;
    let mut since: Option<String> = None;
    let mut seed = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--real" | "--fake" if real.is_some() => return Err("pass one of --real and --fake".into()),
            "--real" => real = Some(true),
            "--fake" => real = Some(false),
            "--tag" if tag.is_some() => return Err("pass --tag once".into()),
            "--tag" => tag = Some(args.next().filter(|t| is_slug(t)).ok_or("--tag takes a slug of a-z, 0-9 and '-'")?),
            "--export" if two_steps && export.is_none() => export = Some(args.next().ok_or("--export takes a file")?.into()),
            "--labels" if two_steps && labels.is_none() => labels = Some(args.next().ok_or("--labels takes a file")?.into()),
            "--background" if kind == Kind::Chains && background.is_none() => {
                background = Some(args.next().ok_or("--background takes a file")?.into())
            }
            "--since" if kind == Kind::Warned && since.is_none() => {
                since = Some(args.next().ok_or("--since takes the name of a pre-migration snapshot")?)
            }
            "--seed" if two_steps && seed.is_none() => {
                seed = Some(args.next().and_then(|s| s.parse::<u64>().ok()).ok_or("--seed takes a number")?)
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if two_steps {
        return match (export, labels) {
            (Some(export), None) => {
                let real = real.ok_or("pass --real or --fake")?;
                if tag.is_some() {
                    return Err("--tag names the result file, which --labels writes".into());
                }
                if kind == Kind::Warned {
                    let since = since.ok_or("pass --since, the pre-migration snapshot the window starts at")?;
                    return Ok(Some(Command::WarnedReplay { export, since, seed: seed.unwrap_or(warned::DEFAULT_SEED), real }));
                }
                Ok(Some(Command::StoreReplay { export, seed: seed.unwrap_or(store::DEFAULT_SEED), real }))
            }
            (None, Some(labels)) => {
                if real.is_some() || seed.is_some() || since.is_some() {
                    let own = if kind == Kind::Warned { "--real, --fake, --seed and --since" } else { "--real, --fake and --seed" };
                    return Err(format!("--labels scores what the replay left: {own} belong to --export"));
                }
                Ok(Some(if kind == Kind::Warned { Command::WarnedScore { labels, tag } } else { Command::StoreScore { labels, tag } }))
            }
            (Some(_), Some(_)) => Err("pass one of --export and --labels".into()),
            (None, None) => Err("pass --export or --labels".into()),
        };
    }
    let real = real.ok_or("pass --real or --fake")?;
    if tag.is_some() && !real {
        return Err("--tag names a file, and only --real writes one".into());
    }
    Ok(Some(Command::Chains { real, tag, background }))
}

fn is_slug(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The name of a run's result file, as `rag-eval` names its own.
fn results_filename(command: &str, date: &str, tag: Option<&str>) -> String {
    match tag {
        Some(tag) => format!("{date}-real-revision-eval-{command}-{tag}.json"),
        None => format!("{date}-real-revision-eval-{command}.json"),
    }
}

/// The directory the dated results go to.
fn results_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/eval/results")
}

/// What the result of `store` holds: the day, the clawg that measured and the
/// scored report, which has counts, rates and bounds and nothing stored.
#[derive(Serialize)]
struct StoreResultFile<'a> {
    date: &'a str,
    clawg_version: &'static str,
    #[serde(flatten)]
    report: &'a store::Scored,
}

fn store_result_json(date: &str, report: &store::Scored) -> String {
    let file = StoreResultFile { date, clawg_version: env!("CARGO_PKG_VERSION"), report };
    serde_json::to_string_pretty(&file).expect("the report always serializes")
}

/// What the result of `warned` holds: the day, the clawg that measured and the
/// scored report, which has counts, rates, intervals and cuts and nothing
/// stored.
#[derive(Serialize)]
struct WarnedResultFile<'a> {
    date: &'a str,
    clawg_version: &'static str,
    #[serde(flatten)]
    report: &'a warned::Scored,
}

fn warned_result_json(date: &str, report: &warned::Scored) -> String {
    let file = WarnedResultFile { date, clawg_version: env!("CARGO_PKG_VERSION"), report };
    serde_json::to_string_pretty(&file).expect("the report always serializes")
}

/// Where the result of a second step goes: nowhere for a replay on the double,
/// which only prints, and for one on the real model a file in `dir` that is not
/// there yet, since a rerun must not replace the baseline it is compared with.
fn result_path(dir: &Path, command: &str, date: &str, tag: Option<&str>, fake: bool) -> Result<Option<PathBuf>, run::RunError> {
    if fake {
        return Ok(None);
    }
    let path = dir.join(results_filename(command, date, tag));
    if path.exists() {
        return Err(run::RunError::Refused(format!("{} already exists; pass --tag to name this run", path.display())));
    }
    Ok(Some(path))
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
    match parse_args(args) {
        Ok(Some(Command::Chains { real, tag, background })) => run_chains(real, tag, background.as_deref()).await,
        Ok(Some(Command::StoreReplay { export, seed, real })) => run_store_replay(&export, seed, real).await,
        Ok(Some(Command::StoreScore { labels, tag })) => run_store_score(&labels, tag.as_deref()).await,
        Ok(Some(Command::WarnedReplay { export, since, seed, real })) => run_warned_replay(&export, &since, seed, real).await,
        Ok(Some(Command::WarnedScore { labels, tag })) => run_warned_score(&labels, tag.as_deref()).await,
        Ok(None) => {
            println!("{HELP_TEXT}");
            EXIT_OK
        }
        Err(e) => {
            eprintln!("error: {e}\n\n{HELP_TEXT}");
            EXIT_RUNTIME_FAILURE
        }
    }
}

/// The embedding model and the token counter of a run, with the data
/// directory they come from: the real ones need the model already cached.
async fn load_models(real: bool) -> Result<(EvalEmbedder, EvalTokenCounter, PathBuf), i32> {
    let base_dir = match clawg_core::paths::data_dir_from_env() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("failed to resolve the data directory: {e}");
            return Err(EXIT_RUNTIME_FAILURE);
        }
    };
    if !real {
        return Ok((EvalEmbedder::fake(), EvalTokenCounter::fake(), base_dir));
    }
    if !model_is_cached(&base_dir) {
        eprintln!("the model is not cached under {}; run rag-eval --real once to download it", base_dir.display());
        return Err(EXIT_MODEL_ABSENT);
    }
    let embedder = EvalEmbedder::real(&base_dir).await.map_err(|e| {
        eprintln!("failed to load the real embedding model: {e}");
        EXIT_RUNTIME_FAILURE
    })?;
    let counter = EvalTokenCounter::real(&base_dir).map_err(|e| {
        eprintln!("failed to load the tokenizer: {e}");
        EXIT_RUNTIME_FAILURE
    })?;
    Ok((embedder, counter, base_dir))
}

fn exit_for(error: &run::RunError) -> i32 {
    if error.is_bad_input() {
        EXIT_INPUT_INVALID
    } else {
        EXIT_RUNTIME_FAILURE
    }
}

/// The first step of `store`: replays the export and leaves the labels to fill.
async fn run_store_replay(export: &Path, seed: u64, real: bool) -> i32 {
    let (embedder, counter, base_dir) = match load_models(real).await {
        Ok(models) => models,
        Err(code) => return code,
    };
    // Under --fake the chunker must not find a tokenizer.
    let tokenizer_dir = if real { base_dir } else { run::scratch_dir() };
    let input = run::ReplayInput { embedder: &embedder, tokenizer_dir: &tokenizer_dir, counter: &counter, seed, fake: !real };
    match run::store_replay(export, &input).await {
        Ok(summary) => {
            println!("{}", serde_json::to_string_pretty(&summary).expect("the summary always serializes"));
            let dir = export.parent().unwrap_or(export);
            eprintln!(
                "fill `label` (`revision` or `other`) of every pair in {}/labels.jsonl, then run:\n  revision-eval store --labels {}/labels.jsonl",
                dir.display(),
                dir.display()
            );
            EXIT_OK
        }
        Err(e) => {
            eprintln!("replay failed: {e}");
            exit_for(&e)
        }
    }
}

/// The second step of `store`: scores the labels, prints the result and, for
/// a replay on the real model, writes it, never over a file that is there.
async fn run_store_score(labels: &Path, tag: Option<&str>) -> i32 {
    let date = current_date();
    let dir = results_dir();
    let check = |replay: &store::ReplayFile| result_path(&dir, "store", &date, tag, replay.fake).map(|_| ());
    let write = |scored: &store::Scored| {
        let json = store_result_json(&date, scored);
        println!("{json}");
        let Some(path) = result_path(&dir, "store", &date, tag, scored.fake)? else {
            return Ok(());
        };
        write_new(&path, &json).map_err(|source| run::RunError::Io { path: path.clone(), source })?;
        eprintln!("wrote {}", path.display());
        Ok(())
    };
    match run::store_score(labels, check, write).await {
        Ok(_) => EXIT_OK,
        Err(e) => {
            eprintln!("scoring failed: {e}");
            exit_for(&e)
        }
    }
}

/// The first step of `warned`: reads the window of the export and leaves the
/// labels to fill. A snapshot name that is not a `pre-migration` one is refused
/// before anything is read or removed.
async fn run_warned_replay(export: &Path, since: &str, seed: u64, real: bool) -> i32 {
    let since_ms = match warned::parse_since(since) {
        Ok(ms) => ms,
        Err(e) => {
            eprintln!("replay failed: {e}");
            return EXIT_INPUT_INVALID;
        }
    };
    let (embedder, counter, base_dir) = match load_models(real).await {
        Ok(models) => models,
        Err(code) => return code,
    };
    // Under --fake the chunker must not find a tokenizer.
    let tokenizer_dir = if real { base_dir } else { run::scratch_dir() };
    let input = run::ReplayInput { embedder: &embedder, tokenizer_dir: &tokenizer_dir, counter: &counter, seed, fake: !real };
    match run::warned_replay(export, since_ms, &input).await {
        Ok(summary) => {
            println!("{}", serde_json::to_string_pretty(&summary).expect("the summary always serializes"));
            let dir = export.parent().unwrap_or(export);
            eprintln!(
                "fill `a` and `b` (`revision` or `other`) of every pair in {0}/labels.jsonl, and `c` where they differ, then run:\n  revision-eval warned --labels {0}/labels.jsonl",
                dir.display()
            );
            EXIT_OK
        }
        Err(e) => {
            eprintln!("replay failed: {e}");
            exit_for(&e)
        }
    }
}

/// The second step of `warned`: scores the labels, prints the result and, for
/// a replay on the real model, writes it, never over a file that is there.
async fn run_warned_score(labels: &Path, tag: Option<&str>) -> i32 {
    let date = current_date();
    let dir = results_dir();
    let check = |file: &warned::WarnedFile| result_path(&dir, "warned", &date, tag, file.fake).map(|_| ());
    let write = |scored: &warned::Scored| {
        let json = warned_result_json(&date, scored);
        println!("{json}");
        let Some(path) = result_path(&dir, "warned", &date, tag, scored.fake)? else {
            return Ok(());
        };
        write_new(&path, &json).map_err(|source| run::RunError::Io { path: path.clone(), source })?;
        eprintln!("wrote {}", path.display());
        Ok(())
    };
    match run::warned_score(labels, check, write).await {
        Ok(_) => EXIT_OK,
        Err(e) => {
            eprintln!("scoring failed: {e}");
            exit_for(&e)
        }
    }
}

async fn run_chains(real: bool, tag: Option<String>, background: Option<&Path>) -> i32 {
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
    let result_path: Option<PathBuf> = real.then(|| results_dir().join(results_filename("chains", &date, tag.as_deref())));
    if let Some(path) = result_path.as_ref().filter(|p| p.exists()) {
        eprintln!("{} already exists; pass --tag to name this run", path.display());
        return EXIT_INPUT_INVALID;
    }

    let (embedder, counter, base_dir) = match load_models(real).await {
        Ok(models) => models,
        Err(code) => return code,
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
        background: None,
    };

    let measured = match background {
        Some(export) => run::run_with_background(&input, &scratch, export).await,
        None => run::run(&input, &scratch).await,
    };
    match measured {
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
            exit_for(&e)
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
        let chains = |real: bool, tag: Option<&str>, background: Option<&str>| {
            Some(Command::Chains { real, tag: tag.map(Into::into), background: background.map(Into::into) })
        };
        assert_eq!(parse_args(args(&["chains", "--fake"])).unwrap(), chains(false, None, None));
        assert_eq!(parse_args(args(&["chains", "--real"])).unwrap(), chains(true, None, None));
        assert_eq!(parse_args(args(&["chains", "--tag", "second-run", "--real"])).unwrap(), chains(true, Some("second-run"), None));
        assert_eq!(
            parse_args(args(&["chains", "--real", "--background", "d/export.jsonl"])).unwrap(),
            chains(true, None, Some("d/export.jsonl"))
        );
        assert_eq!(parse_args(args(&["chains", "--background", "e", "--fake"])).unwrap(), chains(false, None, Some("e")));
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
            &["chains", "--real", "--background"],
            &["chains", "--real", "--background", "a", "--background", "b"],
            &["store", "--real", "--export", "e", "--background", "b"],
        ] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_arguments_of_store_parse() {
        assert_eq!(
            parse_args(args(&["store", "--export", "d/export.jsonl", "--fake"])).unwrap(),
            Some(Command::StoreReplay { export: "d/export.jsonl".into(), seed: store::DEFAULT_SEED, real: false })
        );
        assert_eq!(
            parse_args(args(&["store", "--real", "--seed", "9", "--export", "e"])).unwrap(),
            Some(Command::StoreReplay { export: "e".into(), seed: 9, real: true })
        );
        assert_eq!(
            parse_args(args(&["store", "--labels", "l", "--tag", "again"])).unwrap(),
            Some(Command::StoreScore { labels: "l".into(), tag: Some("again".into()) })
        );
        for bad in [
            &["store"][..],
            &["store", "--export", "e"],
            &["store", "--export", "e", "--labels", "l", "--fake"],
            &["store", "--export", "e", "--fake", "--tag", "x"],
            &["store", "--labels", "l", "--fake"],
            &["store", "--labels", "l", "--seed", "3"],
            &["store", "--export", "e", "--fake", "--seed", "x"],
            &["store", "--export"],
            &["chains", "--fake", "--export", "e"],
        ] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_arguments_of_warned_parse() {
        const SNAPSHOT: &str = "memory-2026-10-22T10-00-00.000Z-pre-migration.db";
        assert_eq!(
            parse_args(args(&["warned", "--export", "d/export.jsonl", "--since", SNAPSHOT, "--fake"])).unwrap(),
            Some(Command::WarnedReplay {
                export: "d/export.jsonl".into(),
                since: SNAPSHOT.into(),
                seed: warned::DEFAULT_SEED,
                real: false
            })
        );
        assert_eq!(
            parse_args(args(&["warned", "--real", "--seed", "9", "--since", "s", "--export", "e"])).unwrap(),
            Some(Command::WarnedReplay { export: "e".into(), since: "s".into(), seed: 9, real: true })
        );
        assert_eq!(
            parse_args(args(&["warned", "--labels", "l", "--tag", "again"])).unwrap(),
            Some(Command::WarnedScore { labels: "l".into(), tag: Some("again".into()) })
        );
        assert_eq!(parse_args(args(&["warned", "--help"])).unwrap(), None);
        for bad in [
            &["warned"][..],
            &["warned", "--export", "e", "--fake"],
            &["warned", "--export", "e", "--since", "s"],
            &["warned", "--export", "e", "--labels", "l", "--since", "s", "--fake"],
            &["warned", "--labels", "l", "--since", "s"],
            &["warned", "--labels", "l", "--fake"],
            &["warned", "--labels", "l", "--seed", "3"],
            &["warned", "--export", "e", "--since", "s", "--fake", "--tag", "x"],
            &["warned", "--export", "e", "--since"],
            &["warned", "--export", "e", "--since", "s", "--since", "t", "--fake"],
            &["warned", "--export", "e", "--since", "s", "--fake", "--seed", "x"],
            &["warned", "--export", "e", "--since", "s", "--fake", "--background", "b"],
            &["store", "--export", "e", "--since", "s", "--fake"],
            &["chains", "--fake", "--since", "s"],
        ] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_result_of_the_real_model_goes_to_a_file_that_is_not_there_yet() {
        let dir = std::env::temp_dir().join(format!("revision-eval-main-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        assert_eq!(result_path(&dir, "warned", "2026-10-22", None, true).unwrap(), None, "the double only prints");
        let path = result_path(&dir, "warned", "2026-10-22", None, false).unwrap().expect("a path");
        assert_eq!(path, dir.join("2026-10-22-real-revision-eval-warned.json"));
        write_new(&path, "{}").unwrap();
        let taken = result_path(&dir, "warned", "2026-10-22", None, false).unwrap_err();
        assert!(matches!(taken, run::RunError::Refused(_)) && taken.is_bad_input(), "{taken}");
        assert!(taken.to_string().contains("--tag"), "{taken}");
        let tagged = result_path(&dir, "warned", "2026-10-22", Some("again"), false).unwrap().expect("a path");
        assert_eq!(tagged, dir.join("2026-10-22-real-revision-eval-warned-again.json"));
        assert!(result_path(&dir, "warned", "2026-10-22", None, true).unwrap().is_none(), "a taken name does not matter to the double");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_result_file_is_named_by_date_and_tag() {
        assert_eq!(results_filename("warned", "2026-10-22", None), "2026-10-22-real-revision-eval-warned.json");
        assert_eq!(results_filename("chains", "2026-10-21", None), "2026-10-21-real-revision-eval-chains.json");
        assert_eq!(
            results_filename("chains", "2026-10-21", Some("again")),
            "2026-10-21-real-revision-eval-chains-again.json"
        );
        assert_eq!(results_filename("store", "2026-10-21", None), "2026-10-21-real-revision-eval-store.json");
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
            background: None,
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
        assert_eq!(arms, ["control", "keyed", "keyed_bare", "keyed_unindexed", "no_key"]);
    }

    /// The whole of `store` on the double: the result of a run whose export
    /// holds recognisable texts and a key must hold none of them.
    #[tokio::test]
    async fn the_results_file_of_store_holds_no_stored_text() {
        use store::fixtures::{small_export, work_with};

        let rows = small_export();
        let (dir, export) = work_with(&rows);
        let embedder = EvalEmbedder::fake();
        let counter = EvalTokenCounter::fake();
        let no_tokenizer = run::scratch_dir();
        let input = run::ReplayInput { embedder: &embedder, tokenizer_dir: &no_tokenizer, counter: &counter, seed: 5, fake: true };
        run::store_replay(&export, &input).await.expect("the replay must succeed");
        let labels_path = dir.join("labels.jsonl");
        let labels = std::fs::read_to_string(&labels_path).unwrap();
        // The labels are the one file that carries texts: the check below is not vacuous.
        assert!(labels.contains("nightly backup"), "{labels}");
        std::fs::write(&labels_path, labels.replace("\"proposed\":null", "\"proposed\":\"revision\"").replace("\"label\":null", "\"label\":\"other\"")).unwrap();

        let mut result = String::new();
        let scored = run::store_score(&labels_path, |_| Ok(()), |scored| {
            result = store_result_json("2026-10-21", scored);
            Ok(())
        })
        .await
        .expect("the labels score");

        assert!(scored.fake);
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["export_format"], "clawg-memories");
        assert_eq!(value["export_version"], 1);
        assert_eq!(value["seed"], 5);
        assert!(value.get("fake").is_none());
        for row in &rows {
            assert!(!result.contains(&row.content), "stored text in the result: {}", row.content);
            for word in row.content.split_whitespace().filter(|w| w.len() > 6) {
                assert!(!result.contains(word), "a word of a stored text in the result: {word}");
            }
            if let Some(key) = row.topic_key {
                assert!(!result.contains(key), "a topic key in the result");
            }
        }
        for field in ["content", "text", "earlier", "later", "topic_key", "\"id\""] {
            assert!(!result.contains(field), "{field} in {result}");
        }
        assert!(!dir.exists(), "the working directory is gone once the result is out");
    }

    /// The whole of `warned` on the double: the result of a run whose export
    /// holds recognisable texts and a key must hold none of them.
    #[tokio::test]
    async fn the_results_file_of_warned_holds_no_stored_text() {
        use store::fixtures::{at, warned_lines, warned_rows, work_with_lines};

        let (dir, export) = work_with_lines(&warned_lines());
        let embedder = EvalEmbedder::fake();
        let counter = EvalTokenCounter::fake();
        let no_tokenizer = run::scratch_dir();
        let input = run::ReplayInput { embedder: &embedder, tokenizer_dir: &no_tokenizer, counter: &counter, seed: 5, fake: true };
        run::warned_replay(&export, at(10), &input).await.expect("the replay must succeed");
        let labels_path = dir.join("labels.jsonl");
        let labels = std::fs::read_to_string(&labels_path).unwrap();
        // The labels are the one file that carries texts: the check below is not vacuous.
        assert!(labels.contains("nightly backup"), "{labels}");
        let passes = ["\"a\":null", "\"b\":null"];
        let filled = labels.replace(passes[0], "\"a\":\"revision\"").replace(passes[1], "\"b\":\"other\"");
        let filled = filled.replace("\"c\":null", "\"c\":\"revision\"");
        std::fs::write(&labels_path, filled).unwrap();

        let mut result = String::new();
        let scored = run::warned_score(&labels_path, |_| Ok(()), |scored| {
            result = warned_result_json("2026-10-22", scored);
            Ok(())
        })
        .await
        .expect("the labels score");

        assert!(scored.fake);
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["date"], "2026-10-22");
        assert_eq!(value["clawg_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["export_format"], "clawg-memories");
        assert_eq!(value["export_version"], 1);
        assert_eq!(value["seed"], 5);
        assert_eq!(value["window_start_ms"], at(10));
        assert_eq!(value["model_contract"], embedder.model_id());
        assert!(value.get("fake").is_none());
        assert_eq!(value["window"]["marked"], 2);
        assert_eq!(value["agreement"]["disagreements"], 5, "the passes differ on every pair of the test");
        for row in &warned_rows() {
            assert!(!result.contains(&row.content), "stored text in the result: {}", row.content);
            for word in row.content.split_whitespace().filter(|w| w.len() > 6) {
                assert!(!result.contains(word), "a word of a stored text in the result: {word}");
            }
            if let Some(key) = row.topic_key {
                assert!(!result.contains(key), "a topic key in the result");
            }
        }
        for field in ["content", "text", "earlier", "later", "topic_key", "\"id\""] {
            assert!(!result.contains(field), "{field} in {result}");
        }
        assert!(!dir.exists(), "the working directory is gone once the result is out");
    }
}
