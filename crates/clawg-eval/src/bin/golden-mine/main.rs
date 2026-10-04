//! Golden-set miner: turns a large pool of candidate memory documents into a
//! small corpus whose retrieval cases are hard by construction.
//!
//! The problem this exists to solve: a golden set assembled by picking
//! plausible documents and writing a query for each one saturates. Every query
//! has exactly one related document, the embedder finds it, every metric reads
//! 1.0, and the set can no longer detect a ranking regression — which is the
//! only thing it was built to do.
//!
//! The fix is to pick distractors with the same model that will be evaluated.
//! For each expected document, its rivals are its nearest neighbours *of the
//! same memory_type* in the real embedding space. Whether that is hard is not
//! asserted here: `hardest_distractor_cosine` is written into every case, so
//! the difficulty of the set is a number anyone can read.
//!
//! Queries are NOT generated. They need semantic judgement, and a query
//! written as a paraphrase of its own answer measures paraphrase detection
//! rather than retrieval. The draft leaves them empty on purpose.

mod draft;
mod mine;

use std::path::PathBuf;

use clawg_core::eval_api::{cosine, EmbedRole, EvalEmbedder};

use mine::{Embedded, Quota};

const EXIT_OK: i32 = 0;
const EXIT_FAILURE: i32 = 1;

/// Corpus composition. Proportions follow the existing golden set rather than
/// the candidate pool: the pool is whatever engram happens to hold (mostly
/// architectural decisions), while the corpus should look like what the memory
/// subsystem actually stores at retrieval time.
const QUOTAS: &[(&str, usize)] = &[
    ("architectural_decision", 12),
    ("semantic_fact", 9),
    ("tool_error", 8),
    ("user_preference", 7),
    ("session_summary", 6),
    ("code_block", 4),
    ("chat_message", 2),
];

/// Documents per cluster. Three is the smallest number that produces a genuine
/// ranking problem: with two, ranking the expected document first is a coin
/// flip, and a coin flip averaged over 40 cases looks like a signal.
const CLUSTER_SIZE: usize = 3;

/// Distractors per case, and the floor below which a case is dropped instead
/// of shipped easy.
const DISTRACTORS: usize = 4;
const MIN_DISTRACTORS: usize = 2;

#[derive(Debug)]
struct Args {
    candidates: PathBuf,
    out: PathBuf,
    pool_cap: usize,
}

fn parse_args<I: Iterator<Item = String>>(mut args: I) -> Result<Args, String> {
    args.next();
    let mut candidates = None;
    let mut out = None;
    let mut pool_cap = 400usize;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--candidates" => {
                candidates = Some(PathBuf::from(
                    args.next().ok_or("--candidates requires a value")?,
                ))
            }
            "--out" => out = Some(PathBuf::from(args.next().ok_or("--out requires a value")?)),
            "--pool" => {
                pool_cap = args
                    .next()
                    .ok_or("--pool requires a value")?
                    .parse()
                    .map_err(|_| "--pool must be a positive integer".to_string())?
            }
            "--help" | "-h" => return Err(HELP.to_string()),
            other => return Err(format!("unknown flag '{other}'")),
        }
    }

    Ok(Args {
        candidates: candidates.ok_or("--candidates is required")?,
        out: out.ok_or("--out is required")?,
        pool_cap,
    })
}

const HELP: &str = "\
golden-mine - builds a hard golden-set draft from a pool of candidate documents

USAGE:
    golden-mine --candidates <path> --out <path> [--pool N]

OPTIONS:
    --candidates <path>  JSON array from the engram extractor
    --out <path>         Where to write the golden-set draft
    --pool N             Cap on documents to embed (default 400). Every candidate
                         above the cap is ignored, so raising it widens the space
                         the miner can find hard distractors in, at linear cost.
    --help, -h           Print this message
";

fn production_base_dir() -> PathBuf {
    clawg_core::paths::data_dir_from_env()
        .unwrap_or_else(|e| panic!("failed to resolve data directory: {e}"))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    std::process::exit(run(std::env::args()).await);
}

async fn run<I: Iterator<Item = String>>(argv: I) -> i32 {
    let args = match parse_args(argv) {
        Ok(a) => a,
        Err(message) => {
            eprintln!("{message}");
            return EXIT_FAILURE;
        }
    };

    let raw = match std::fs::read_to_string(&args.candidates) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("failed to read {}: {e}", args.candidates.display());
            return EXIT_FAILURE;
        }
    };
    let mut pool: Vec<draft::Candidate> = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("failed to parse candidates: {e}");
            return EXIT_FAILURE;
        }
    };

    // Truncation is per-type, not off the end of the list: the extractor emits
    // grouped by type, so a flat cut would drop whole categories — the last
    // two being `code_block` and `chat_message`, the two already scarcest.
    pool = cap_per_type(pool, args.pool_cap);
    println!("embedding {} candidates with the real model…", pool.len());

    let embedder = match EvalEmbedder::real(&production_base_dir()).await {
        Ok(e) => e,
        Err(e) => {
            eprintln!("failed to load the embedding model: {e}");
            return EXIT_FAILURE;
        }
    };

    let mut embedded = Vec::with_capacity(pool.len());
    for (i, c) in pool.iter().enumerate() {
        // Passage: mining from stored-content-shaped text, so a future
        // re-mine happens in the same space production writes to.
        match embedder.embed_with_role(&c.content, EmbedRole::Passage).await {
            Ok(vector) => embedded.push(Embedded {
                memory_type: c.memory_type.clone(),
                lang: c.lang.clone(),
                vector,
            }),
            Err(e) => {
                eprintln!("embedding candidate {i} failed: {e}");
                return EXIT_FAILURE;
            }
        }
        if (i + 1) % 50 == 0 {
            println!("  {}/{}", i + 1, pool.len());
        }
    }

    let quotas: Vec<Quota> = QUOTAS
        .iter()
        .map(|(t, n)| Quota { memory_type: (*t).to_string(), count: *n })
        .collect();

    let selected = mine::select_corpus(&embedded, &quotas, CLUSTER_SIZE);
    if selected.is_empty() {
        eprintln!("no corpus could be selected — is the candidate pool empty?");
        return EXIT_FAILURE;
    }

    let corpus: Vec<Embedded> = selected.iter().map(|&i| embedded[i].clone()).collect();
    let cases = mine::mine_cases(&corpus, DISTRACTORS, MIN_DISTRACTORS);

    let hardest: Vec<f64> = cases
        .iter()
        .map(|c| {
            c.distractors
                .iter()
                .map(|&d| cosine(&corpus[c.expected].vector, &corpus[d].vector))
                .fold(f64::NEG_INFINITY, f64::max)
        })
        .collect();

    let out = draft::build(&pool, &selected, &cases, &hardest);
    let json = match serde_json::to_string_pretty(&out) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("failed to serialize the draft: {e}");
            return EXIT_FAILURE;
        }
    };
    if let Err(e) = std::fs::write(&args.out, json) {
        eprintln!("failed to write {}: {e}", args.out.display());
        return EXIT_FAILURE;
    }

    let gap = mine::mean_distractor_gap(&corpus, &cases);
    println!("\ncorpus:  {} documents", out.corpus.len());
    println!("cases:   {} (each with up to {DISTRACTORS} same-type distractors)", cases.len());
    println!("mean distractor gap: {gap:.4}   (lower is harder; >0.30 will saturate)");
    println!("wrote {}", args.out.display());
    println!("\nNext: fill in the empty `query` of every draft case, then run rag-eval --audit.");
    EXIT_OK
}

/// Keeps at most `cap / distinct_types` documents of each `memory_type`.
fn cap_per_type(pool: Vec<draft::Candidate>, cap: usize) -> Vec<draft::Candidate> {
    let mut types: Vec<String> = pool.iter().map(|c| c.memory_type.clone()).collect();
    types.sort();
    types.dedup();
    if types.is_empty() {
        return pool;
    }
    let per_type = (cap / types.len()).max(1);

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    pool.into_iter()
        .filter(|c| {
            let n = counts.entry(c.memory_type.clone()).or_insert(0);
            *n += 1;
            *n <= per_type
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(memory_type: &str) -> draft::Candidate {
        draft::Candidate {
            src_id: serde_json::json!(1),
            src_project: None,
            content: "text".into(),
            memory_type: memory_type.into(),
            importance: 5.0,
            lang: "en".into(),
        }
    }

    fn argv(items: &[&str]) -> impl Iterator<Item = String> {
        std::iter::once("golden-mine".to_string())
            .chain(items.iter().map(|s| s.to_string()))
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn parses_the_required_flags() {
        let args = parse_args(argv(&["--candidates", "in.json", "--out", "out.json"])).unwrap();
        assert_eq!(args.candidates, PathBuf::from("in.json"));
        assert_eq!(args.out, PathBuf::from("out.json"));
        assert_eq!(args.pool_cap, 400);
    }

    #[test]
    fn missing_required_flags_is_an_error() {
        assert!(parse_args(argv(&["--out", "o.json"])).is_err());
        assert!(parse_args(argv(&["--candidates", "i.json"])).is_err());
    }

    #[test]
    fn a_non_numeric_pool_is_an_error_not_a_silent_default() {
        let err = parse_args(argv(&["--candidates", "i", "--out", "o", "--pool", "many"]))
            .expect_err("non-numeric --pool must fail");
        assert!(err.contains("positive integer"), "got: {err}");
    }

    #[test]
    fn cap_per_type_keeps_scarce_types_alive() {
        // 300 architectural decisions ahead of 2 chat messages: a flat
        // truncation at 100 would take 100 decisions and zero chat messages.
        let mut pool: Vec<draft::Candidate> =
            (0..300).map(|_| candidate("architectural_decision")).collect();
        pool.extend((0..2).map(|_| candidate("chat_message")));

        let capped = cap_per_type(pool, 100);
        assert_eq!(capped.iter().filter(|c| c.memory_type == "chat_message").count(), 2);
        assert_eq!(
            capped.iter().filter(|c| c.memory_type == "architectural_decision").count(),
            50
        );
    }

    #[test]
    fn cap_per_type_of_an_empty_pool_is_empty() {
        assert!(cap_per_type(vec![], 100).is_empty());
    }

    #[test]
    fn quotas_sum_to_a_corpus_large_enough_to_discriminate() {
        // A golden set needs ~40 retrieval cases to carry a claim; with one case
        // per document, the corpus has to be at least that big.
        let total: usize = QUOTAS.iter().map(|(_, n)| n).sum();
        assert!(total >= 40, "quotas sum to {total}, below the ~40 a golden set requires");
    }
}
