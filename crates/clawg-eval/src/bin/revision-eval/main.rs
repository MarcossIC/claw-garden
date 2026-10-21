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

use std::path::{Path, PathBuf};

use clawg_core::eval_api::{model_is_cached, EvalEmbedder, EvalTokenCounter};
use serde::Serialize;

const EXIT_OK: i32 = 0;
const EXIT_RUNTIME_FAILURE: i32 = 1;
const EXIT_INPUT_INVALID: i32 = 2;
const EXIT_MODEL_ABSENT: i32 = 3;

const HELP_TEXT: &str = "\
revision-eval chains (--real | --fake) [--tag SLUG]
revision-eval store --export FILE [--seed N] (--real | --fake)
revision-eval store --labels FILE [--tag SLUG]

  chains   saves crates/clawg-eval/data/revision-chains.json in three arms
           (without a key, with its key, and only the last version), each
           after the MUST sentences of openspec/specs, and classifies every
           transition of the arm without a key
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
  --real   the cached embedding model of the data profile
  --fake   the deterministic double: checks the pipeline, not the numbers
  --seed   the seed of the sample of labelled pairs
  --tag    names a second run of the day:
           docs/eval/results/{date}-real-revision-eval-{chains|store}-{SLUG}.json

Prints the JSON report; --real (and `store --labels` after a --real replay)
also writes it to docs/eval/results, never over a file that is already there.
Exit 0 measured, 1 failed, 2 bad input, 3 model not cached.";

#[derive(Debug, Clone, PartialEq)]
enum Command {
    Chains { real: bool, tag: Option<String> },
    StoreReplay { export: PathBuf, seed: u64, real: bool },
    StoreScore { labels: PathBuf, tag: Option<String> },
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Command>, String> {
    let mut args = args.skip(1);
    let store = match args.next().as_deref() {
        Some("--help" | "-h") => return Ok(None),
        Some("chains") => false,
        Some("store") => true,
        Some(other) => return Err(format!("unknown command {other}")),
        None => return Err("name a command".into()),
    };
    let mut real = None;
    let mut tag = None;
    let mut export: Option<PathBuf> = None;
    let mut labels: Option<PathBuf> = None;
    let mut seed = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--real" | "--fake" if real.is_some() => return Err("pass one of --real and --fake".into()),
            "--real" => real = Some(true),
            "--fake" => real = Some(false),
            "--tag" if tag.is_some() => return Err("pass --tag once".into()),
            "--tag" => tag = Some(args.next().filter(|t| is_slug(t)).ok_or("--tag takes a slug of a-z, 0-9 and '-'")?),
            "--export" if store && export.is_none() => export = Some(args.next().ok_or("--export takes a file")?.into()),
            "--labels" if store && labels.is_none() => labels = Some(args.next().ok_or("--labels takes a file")?.into()),
            "--seed" if store && seed.is_none() => {
                seed = Some(args.next().and_then(|s| s.parse::<u64>().ok()).ok_or("--seed takes a number")?)
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if store {
        return match (export, labels) {
            (Some(export), None) => {
                let real = real.ok_or("pass --real or --fake")?;
                if tag.is_some() {
                    return Err("--tag names the result file, which --labels writes".into());
                }
                Ok(Some(Command::StoreReplay { export, seed: seed.unwrap_or(store::DEFAULT_SEED), real }))
            }
            (None, Some(labels)) => {
                if real.is_some() || seed.is_some() {
                    return Err("--labels scores what the replay left: --real, --fake and --seed belong to --export".into());
                }
                Ok(Some(Command::StoreScore { labels, tag }))
            }
            (Some(_), Some(_)) => Err("pass one of --export and --labels".into()),
            (None, None) => Err("pass --export or --labels".into()),
        };
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
        Ok(Some(Command::Chains { real, tag })) => run_chains(real, tag).await,
        Ok(Some(Command::StoreReplay { export, seed, real })) => run_store_replay(&export, seed, real).await,
        Ok(Some(Command::StoreScore { labels, tag })) => run_store_score(&labels, tag.as_deref()).await,
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
    let result_path = |fake: bool| (!fake).then(|| results_dir().join(results_filename("store", &date, tag)));
    let check = |replay: &store::ReplayFile| match result_path(replay.fake) {
        Some(path) if path.exists() => {
            Err(run::RunError::Refused(format!("{} already exists; pass --tag to name this run", path.display())))
        }
        _ => Ok(()),
    };
    let write = |scored: &store::Scored| {
        let json = store_result_json(&date, scored);
        println!("{json}");
        let Some(path) = result_path(scored.fake) else {
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

async fn run_chains(real: bool, tag: Option<String>) -> i32 {
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
    fn the_result_file_is_named_by_date_and_tag() {
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
}
