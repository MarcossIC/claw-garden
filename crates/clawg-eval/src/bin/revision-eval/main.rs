//! Measures what a `topic_key` does for a memory that is revised: how often an
//! old version stays live without a mark when the save names no key, whether
//! a key hides every old version, and what showing the key costs.
//!
//! `--real` needs the embedding model already cached in the data profile;
//! `--fake` runs the same pipeline on the deterministic double, which checks
//! the plumbing and never the numbers.

mod chains;
mod run;
mod set;
mod stats;

use std::path::Path;

use clawg_core::eval_api::{model_is_cached, EvalEmbedder};

const EXIT_OK: i32 = 0;
const EXIT_RUNTIME_FAILURE: i32 = 1;
const EXIT_INPUT_INVALID: i32 = 2;
const EXIT_MODEL_ABSENT: i32 = 3;

const HELP_TEXT: &str = "\
revision-eval chains (--real | --fake)

  chains   saves crates/clawg-eval/data/revision-chains.json in three arms
           (without a key, with its key, and only the last version), each
           after the MUST sentences of openspec/specs, and classifies every
           transition of the arm without a key
  --real   the cached embedding model of the data profile
  --fake   the deterministic double: checks the pipeline, not the numbers

Prints the JSON report. Exit 0 measured, 1 failed, 2 bad input, 3 model not cached.";

#[derive(Debug, Clone, PartialEq)]
enum Command {
    Chains { real: bool },
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
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--real" | "--fake" if real.is_some() => return Err("pass one of --real and --fake".into()),
            "--real" => real = Some(true),
            "--fake" => real = Some(false),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let real = real.ok_or("pass --real or --fake")?;
    Ok(Some(Command::Chains { real }))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    std::process::exit(run(std::env::args()).await);
}

async fn run(args: impl Iterator<Item = String>) -> i32 {
    let real = match parse_args(args) {
        Ok(Some(Command::Chains { real })) => real,
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
    let scratch = run::scratch_dir();
    // Under --fake the chunker must not find a tokenizer, and the scratch
    // directory holds none.
    let tokenizer_dir = if real { base_dir.clone() } else { scratch.clone() };
    let input = run::ChainsInput { set: &set, distractors: &distractors, embedder: &embedder, tokenizer_dir: &tokenizer_dir };

    match run::run(&input, &scratch).await {
        Ok(report) => {
            println!("{}", serde_json::to_string_pretty(&report).expect("the report always serializes"));
            EXIT_OK
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
        assert_eq!(parse_args(args(&["chains", "--fake"])).unwrap(), Some(Command::Chains { real: false }));
        assert_eq!(parse_args(args(&["chains", "--real"])).unwrap(), Some(Command::Chains { real: true }));
        assert_eq!(parse_args(args(&["--help"])).unwrap(), None);
        assert_eq!(parse_args(args(&["chains", "--help"])).unwrap(), None);
        for bad in [&[][..], &["chains"], &["chains", "--real", "--fake"], &["store"], &["chains", "--fake", "--out"]] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?}");
        }
    }
}
