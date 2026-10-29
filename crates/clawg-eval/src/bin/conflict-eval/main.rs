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

use measure::{
    by, flow_report, gate, parse_flow, parse_pairs, requirement_sentences, Class, ClassReport, FlowSave, PairOutcome,
};

const EXIT_OK: i32 = 0;
const EXIT_RUNTIME_FAILURE: i32 = 1;
const EXIT_INPUT_INVALID: i32 = 2;
const EXIT_MODEL_ABSENT: i32 = 3;

const HELP_TEXT: &str = "\
conflict-eval (--real | --fake) [--pairs FILE] [--specs DIR | --flow FILE] [--out FILE]

  --real        the cached embedding model of the data profile
  --fake        the deterministic double: checks the pipeline, not the numbers
  --pairs FILE  labelled pairs (default: crates/clawg-eval/data/conflict-pairs.json)
  --specs DIR   spec files whose MUST sentences make the flow, each saved as a
                semantic_fact (default: openspec/specs)
  --flow FILE   the flow instead: a JSON array of {\"content\", \"memory_type\"},
                saved in its order
  --out FILE    also write the JSON report there

Prints the JSON report. Exit 0 measured, 1 failed, 2 bad input, 3 model not cached.";

#[derive(Debug, Clone, PartialEq)]
struct Options {
    real: bool,
    pairs: PathBuf,
    specs: PathBuf,
    flow: Option<PathBuf>,
    out: Option<PathBuf>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut real = None;
    let mut pairs = root.join("data/conflict-pairs.json");
    let mut specs = root.join("../../openspec/specs");
    let mut flow = None;
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
            "--flow" => flow = Some(PathBuf::from(value("--flow")?)),
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let real = real.ok_or("pass --real or --fake")?;
    Ok(Some(Options { real, pairs, specs, flow, out }))
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
    let source = options.flow.as_deref().unwrap_or(&options.specs);
    let flow = match flow_saves(options.flow.as_deref(), &options.specs) {
        Ok(flow) if !flow.is_empty() => flow,
        Ok(_) => {
            eprintln!("{}: no save to make a flow of", source.display());
            return EXIT_INPUT_INVALID;
        }
        Err(e) => {
            eprintln!("{}: {e}", source.display());
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

    match measure_all(&embedder, &tokenizer_dir, &std::env::temp_dir(), &pairs, &flow, options.real).await {
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

/// The saves of `flow` when given, else every MUST sentence under `specs`
/// saved as a fact.
fn flow_saves(flow: Option<&Path>, specs: &Path) -> Result<Vec<FlowSave>, String> {
    match flow {
        Some(file) => parse_flow(&std::fs::read_to_string(file).map_err(|e| e.to_string())?),
        None => Ok(read_flow(specs)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|content| FlowSave { content, memory_type: "semantic_fact".into() })
            .collect()),
    }
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

/// Measures every pair and the flow, each in a store of its own made in `work`.
async fn measure_all<'a>(
    embedder: &EvalEmbedder,
    tokenizer_dir: &Path,
    work: &Path,
    pairs: &'a [measure::LabelledPair],
    flow: &[FlowSave],
    real: bool,
) -> Result<Report<'a>, String> {
    let mut outcomes: Vec<(Class, PairOutcome)> = Vec::new();
    let mut by_lang: Vec<(String, PairOutcome)> = Vec::new();
    for pair in pairs {
        let dir = tempfile::tempdir_in(work).map_err(|e| e.to_string())?;
        let ingest = EvalIngest::open(dir.path(), embedder, tokenizer_dir).await.map_err(|e| e.to_string())?;
        ingest.send(AGENT, &pair.old, &pair.memory_type, 5.0, 0).await.map_err(|e| e.to_string())?;
        ingest.send(AGENT, &pair.new, &pair.memory_type, 5.0, 0).await.map_err(|e| e.to_string())?;
        let runtime = ingest.finish().await.map_err(|e| e.to_string())?;
        let found = conflict_pairs(&runtime).await.map_err(|e| e.to_string())?;
        // On Windows a store still open cannot be deleted with its directory.
        runtime.close().await;
        let outcome = PairOutcome { candidate: !found.is_empty(), escalated: found.iter().any(|p| p.escalated) };
        outcomes.push((pair.class, outcome));
        by_lang.push((format!("{}/{}", pair.class.as_str(), pair.lang), outcome));
    }

    let dir = tempfile::tempdir_in(work).map_err(|e| e.to_string())?;
    let ingest = EvalIngest::open(dir.path(), embedder, tokenizer_dir).await.map_err(|e| e.to_string())?;
    for save in flow {
        ingest.send(AGENT, &save.content, &save.memory_type, 5.0, 0).await.map_err(|e| e.to_string())?;
    }
    let runtime = ingest.finish().await.map_err(|e| e.to_string())?;
    let flow_pairs: Vec<(i64, bool)> = conflict_pairs(&runtime)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|p| (p.new_id, p.escalated))
        .collect();
    runtime.close().await;
    let flow = flow_report(flow.len(), &flow_pairs);

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
        assert_eq!(o.flow, None);
        let o = parse_args(args(&["--real", "--flow", "saves.json"])).unwrap().unwrap();
        assert_eq!(o.flow, Some(PathBuf::from("saves.json")));
        assert_eq!(parse_args(args(&["--help"])).unwrap(), None);
        assert!(parse_args(args(&[])).is_err());
        assert!(parse_args(args(&["--real", "--fake"])).is_err());
        assert!(parse_args(args(&["--pairs"])).is_err());
        assert!(parse_args(args(&["--fake", "--flow"])).is_err());
    }

    #[test]
    fn without_a_flow_file_the_flow_is_the_spec_sentences_as_facts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("a")).unwrap();
        std::fs::write(dir.path().join("a/spec.md"), "- La fila MUST quedar `active` cuando no queda ninguna disputa.\n").unwrap();
        let flow = flow_saves(None, dir.path()).unwrap();
        assert_eq!(
            flow,
            vec![FlowSave {
                content: "La fila MUST quedar `active` cuando no queda ninguna disputa.".into(),
                memory_type: "semantic_fact".into(),
            }]
        );
    }

    #[test]
    fn the_repository_specs_make_a_flow_of_at_least_two_hundred_facts() {
        let specs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../openspec/specs");
        let facts = read_flow(&specs).unwrap();
        assert!(facts.len() >= 200, "{}", facts.len());
    }

    /// One shipped pair of each class, and a flow of two saves.
    fn one_pair_of_each_class_and_a_flow() -> (Vec<measure::LabelledPair>, Vec<FlowSave>) {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/data/conflict-pairs.json")).unwrap();
        let pairs: Vec<_> = parse_pairs(&text).unwrap();
        let few = Class::ALL
            .iter()
            .flat_map(|c| pairs.iter().filter(move |p| p.class == *c).take(1).cloned())
            .collect();
        let flow = vec![
            FlowSave { content: "the first fact of the flow is long enough".into(), memory_type: "semantic_fact".into() },
            FlowSave { content: "a preference of the flow".into(), memory_type: "user_preference".into() },
        ];
        (few, flow)
    }

    /// The double places texts far apart, so nothing pairs: this checks that
    /// every pair and the flow go through the worker and the report is whole.
    #[tokio::test]
    async fn a_fake_run_measures_every_class_and_the_flow() {
        let (few, flow) = one_pair_of_each_class_and_a_flow();
        let (tokenizer, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());

        let report = measure_all(&EvalEmbedder::fake(), tokenizer.path(), work.path(), &few, &flow, false).await.unwrap();

        assert_eq!(report.by_class.len(), Class::ALL.len());
        assert_eq!(report.flow.saves, 2);
        assert_eq!(report.model, "fake");
    }

    #[tokio::test]
    async fn a_run_removes_every_store_it_made() {
        let (few, flow) = one_pair_of_each_class_and_a_flow();
        let (tokenizer, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());

        measure_all(&EvalEmbedder::fake(), tokenizer.path(), work.path(), &few, &flow, false).await.unwrap();

        let left: Vec<_> = std::fs::read_dir(work.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert!(left.is_empty(), "every store is removed when its measurement ends: {left:?}");
    }
}
