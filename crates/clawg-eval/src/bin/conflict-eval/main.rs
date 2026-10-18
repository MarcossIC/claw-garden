//! Measures the contradiction detector end to end, through the same ingest
//! worker the daemon runs: how often each class of labelled pair becomes a
//! candidate and how often it is put to the user, and how many pairs a flow
//! of distinct facts of one project leaves per save.
//!
//! `--real` needs the embedding model already cached in the data profile;
//! `--fake` runs the same pipeline on the deterministic double, which checks
//! the plumbing and never the numbers.

mod measure;

use std::path::{Path, PathBuf};

use clawg_core::eval_api::{conflict_pairs, model_is_cached, EvalEmbedder, EvalIngest};

use measure::{by, flow_report, gate, parse_pairs, requirement_sentences, Class, ClassReport, PairOutcome};

const EXIT_OK: i32 = 0;
const EXIT_RUNTIME_FAILURE: i32 = 1;
const EXIT_INPUT_INVALID: i32 = 2;
const EXIT_MODEL_ABSENT: i32 = 3;

const HELP_TEXT: &str = "\
conflict-eval (--real | --fake) [--pairs FILE] [--specs DIR] [--out FILE]

  --real        the cached embedding model of the data profile
  --fake        the deterministic double: checks the pipeline, not the numbers
  --pairs FILE  labelled pairs (default: crates/clawg-eval/data/conflict-pairs.json)
  --specs DIR   spec files whose MUST sentences make the flow (default: openspec/specs)
  --out FILE    also write the JSON report there

Prints the JSON report. Exit 0 measured, 1 failed, 2 bad input, 3 model not cached.";

#[derive(Debug, Clone, PartialEq)]
struct Options {
    real: bool,
    pairs: PathBuf,
    specs: PathBuf,
    out: Option<PathBuf>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut real = None;
    let mut pairs = root.join("data/conflict-pairs.json");
    let mut specs = root.join("../../openspec/specs");
    let mut out = None;
    let mut args = args.skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--real" | "--fake" if real.is_some() => return Err("pass one of --real and --fake".into()),
            "--real" => real = Some(true),
            "--fake" => real = Some(false),
            "--pairs" => pairs = PathBuf::from(value("--pairs")?),
            "--specs" => specs = PathBuf::from(value("--specs")?),
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let real = real.ok_or("pass --real or --fake")?;
    Ok(Some(Options { real, pairs, specs, out }))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    std::process::exit(run(std::env::args()).await);
}

async fn run(args: impl Iterator<Item = String>) -> i32 {
    let options = match parse_args(args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("{HELP_TEXT}");
            return EXIT_OK;
        }
        Err(e) => {
            eprintln!("error: {e}\n\n{HELP_TEXT}");
            return EXIT_RUNTIME_FAILURE;
        }
    };
    let pairs = match std::fs::read_to_string(&options.pairs).map_err(|e| e.to_string()).and_then(|t| parse_pairs(&t)) {
        Ok(pairs) => pairs,
        Err(e) => {
            eprintln!("{}: {e}", options.pairs.display());
            return EXIT_INPUT_INVALID;
        }
    };
    let facts = match read_flow(&options.specs) {
        Ok(facts) if !facts.is_empty() => facts,
        Ok(_) => {
            eprintln!("{}: no MUST sentence to make a flow of", options.specs.display());
            return EXIT_INPUT_INVALID;
        }
        Err(e) => {
            eprintln!("{}: {e}", options.specs.display());
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
    let embedder = if options.real {
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
    let tokenizer_dir = if options.real { base_dir.clone() } else { std::env::temp_dir() };

    match measure_all(&embedder, &tokenizer_dir, &pairs, &facts, options.real).await {
        Ok(report) => {
            let json = serde_json::to_string_pretty(&report).expect("the report always serializes");
            println!("{json}");
            if let Some(out) = &options.out {
                if let Err(e) = std::fs::write(out, format!("{json}\n")) {
                    eprintln!("{}: {e}", out.display());
                    return EXIT_RUNTIME_FAILURE;
                }
            }
            EXIT_OK
        }
        Err(e) => {
            eprintln!("measurement failed: {e}");
            EXIT_RUNTIME_FAILURE
        }
    }
}

/// Every MUST sentence of the `spec.md` files under `dir`, each once, in path
/// order so two runs save the same flow.
fn read_flow(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut specs: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path().join("spec.md")))
        .filter(|p| p.is_file())
        .collect();
    specs.sort();
    let mut facts: Vec<String> = Vec::new();
    for spec in specs {
        for sentence in requirement_sentences(&std::fs::read_to_string(spec)?) {
            if !facts.contains(&sentence) {
                facts.push(sentence);
            }
        }
    }
    Ok(facts)
}

#[derive(serde::Serialize)]
struct Report<'a> {
    model: &'static str,
    by_class: std::collections::BTreeMap<&'a str, ClassReport>,
    by_class_lang: std::collections::BTreeMap<String, ClassReport>,
    flow: measure::FlowReport,
    gate: measure::Gate,
}

const AGENT: &str = "conflict-eval";

async fn measure_all<'a>(
    embedder: &EvalEmbedder,
    tokenizer_dir: &Path,
    pairs: &'a [measure::LabelledPair],
    facts: &[String],
    real: bool,
) -> Result<Report<'a>, String> {
    let mut outcomes: Vec<(Class, PairOutcome)> = Vec::new();
    let mut by_lang: Vec<(String, PairOutcome)> = Vec::new();
    for pair in pairs {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let ingest = EvalIngest::open(dir.path(), embedder, tokenizer_dir).await.map_err(|e| e.to_string())?;
        ingest.send(AGENT, &pair.old, &pair.memory_type, 5.0).await.map_err(|e| e.to_string())?;
        ingest.send(AGENT, &pair.new, &pair.memory_type, 5.0).await.map_err(|e| e.to_string())?;
        let runtime = ingest.finish().await.map_err(|e| e.to_string())?;
        let found = conflict_pairs(&runtime).await.map_err(|e| e.to_string())?;
        let outcome = PairOutcome { candidate: !found.is_empty(), escalated: found.iter().any(|p| p.escalated) };
        outcomes.push((pair.class, outcome));
        by_lang.push((format!("{}/{}", pair.class.as_str(), pair.lang), outcome));
    }

    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let ingest = EvalIngest::open(dir.path(), embedder, tokenizer_dir).await.map_err(|e| e.to_string())?;
    for fact in facts {
        ingest.send(AGENT, fact, "semantic_fact", 5.0).await.map_err(|e| e.to_string())?;
    }
    let runtime = ingest.finish().await.map_err(|e| e.to_string())?;
    let flow_pairs: Vec<(i64, bool)> = conflict_pairs(&runtime)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|p| (p.new_id, p.escalated))
        .collect();
    let flow = flow_report(facts.len(), &flow_pairs);

    let classes = by(&outcomes);
    let contradiction = classes.get(&Class::Contradiction).cloned().ok_or("no contradiction pair was measured")?;
    let gate = gate(&flow, &contradiction);
    Ok(Report {
        model: if real { "real" } else { "fake" },
        by_class: classes.into_iter().map(|(c, r)| (c.as_str(), r)).collect(),
        by_class_lang: by(&by_lang).into_iter().map(|(k, r)| (k.clone(), r)).collect(),
        flow,
        gate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> std::vec::IntoIter<String> {
        std::iter::once("conflict-eval".to_string()).chain(list.iter().map(|s| s.to_string())).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn the_arguments_parse() {
        let o = parse_args(args(&["--fake", "--out", "r.json"])).unwrap().unwrap();
        assert!(!o.real);
        assert_eq!(o.out, Some(PathBuf::from("r.json")));
        assert!(o.pairs.ends_with("data/conflict-pairs.json"));
        assert_eq!(parse_args(args(&["--help"])).unwrap(), None);
        assert!(parse_args(args(&[])).is_err());
        assert!(parse_args(args(&["--real", "--fake"])).is_err());
        assert!(parse_args(args(&["--pairs"])).is_err());
    }

    #[test]
    fn the_repository_specs_make_a_flow_of_at_least_two_hundred_facts() {
        let specs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../openspec/specs");
        let facts = read_flow(&specs).unwrap();
        assert!(facts.len() >= 200, "{}", facts.len());
    }

    /// The double places texts far apart, so nothing pairs: this checks that
    /// every pair and the flow go through the worker and the report is whole.
    #[tokio::test]
    async fn a_fake_run_measures_every_class_and_the_flow() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/data/conflict-pairs.json")).unwrap();
        let pairs: Vec<_> = parse_pairs(&text).unwrap();
        let few: Vec<_> = Class::ALL
            .iter()
            .flat_map(|c| pairs.iter().filter(move |p| p.class == *c).take(1).cloned())
            .collect();
        let facts = vec!["the first fact of the flow is long enough".to_string(), "a second fact".to_string()];
        let tokenizer = tempfile::tempdir().unwrap();

        let report = measure_all(&EvalEmbedder::fake(), tokenizer.path(), &few, &facts, false).await.unwrap();

        assert_eq!(report.by_class.len(), Class::ALL.len());
        assert_eq!(report.flow.saves, 2);
        assert_eq!(report.model, "fake");
    }
}
