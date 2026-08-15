mod cli;
mod golden;
mod metrics;
mod report;
mod seed;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use agent_office_lib::eval_api::{model_is_cached, EvalEmbedder, EvalScope};

use golden::{CaseStatus, PairLabel};

/// Result-pool sizes handed to `EvalStore::search`. `memory` is the widest
/// pool because every golden-set corpus row seeds into the memory table;
/// `episodic`/`procedural` stay narrow since this harness never seeds either
/// of those tables, so a wider pool there would only cost time.
const K_POOL: EvalScope = EvalScope { memory: Some(30), episodic: Some(5), procedural: Some(5) };

/// Cutoff production truncates search results to. Recall@10/nDCG@10 are
/// defined against exactly this cutoff — raising it here would silently
/// change what those two field names mean.
const K_FINAL: usize = 10;

const EXIT_OK: i32 = 0;
const EXIT_RUNTIME_FAILURE: i32 = 1;
const EXIT_GOLDEN_INVALID: i32 = 2;
const EXIT_MODEL_ABSENT: i32 = 3;
const EXIT_RESULTS_EXISTS: i32 = 4;

// Single-threaded runtime: the crate's declared tokio features (sync, macros,
// time, rt, process, io-util) do not include `rt-multi-thread`, so the
// default multi-thread flavor would not compile here.
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let code = run(std::env::args()).await;
    std::process::exit(code);
}

/// Production's own base directory (`~/.claw-garden`), mirrored here rather
/// than imported from `state.rs` (which is not reachable through `eval_api`
/// and should not become so just for this one path). Pointing `--real` at
/// anywhere else would mean this harness never benefits from a model the
/// running application already downloaded, and `--check-only` would answer a
/// question about a cache the application never looks at.
fn production_base_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".claw-garden")
}

/// Runs the harness end to end for an argv-shaped iterator and returns the
/// process exit code. Split out of `main` so an integration test can drive
/// the whole pipeline — parse, validate, seed, score, report — without a
/// real process boundary.
///
/// Every branch below that can fail prints a message and returns immediately.
/// This is the opposite of the production RAG path's fail-soft posture
/// (`contradiction.rs` swallows a detection failure into `log::warn!` rather
/// than lose a user's memory over it): a measurement tool's only output is a
/// number someone will quote later, and a partially-computed one is worse
/// than none, because nothing downstream can tell the two apart.
async fn run<I: Iterator<Item = String>>(args: I) -> i32 {
    let cli = match cli::parse(args) {
        Ok(cli::ParseOutcome::Help) => {
            println!("{}", cli::HELP_TEXT);
            return EXIT_OK;
        }
        Ok(cli::ParseOutcome::Run(cli)) => cli,
        Err(e) => {
            // Not one of the spec's pinned exit codes (0/2/3/4): a usage
            // error happens before any golden set, model, or database is
            // touched, so it is reported as a generic runtime failure rather
            // than overloading one of the more specific codes with a meaning
            // it does not have.
            eprintln!("error: {e}\n");
            eprintln!("{}", cli::HELP_TEXT);
            return EXIT_RUNTIME_FAILURE;
        }
    };

    let base_dir = production_base_dir();

    // `--check-only` only means something for `--real`; combined with
    // `--fake` it is a harmless no-op per spec, since there is no model to
    // check readiness for.
    if cli.check_only {
        return match cli.mode {
            cli::Mode::Real => {
                if model_is_cached(&base_dir) {
                    println!("model is cached and ready under {}", base_dir.display());
                    EXIT_OK
                } else {
                    eprintln!(
                        "model is not cached under {}; run --real without --check-only to download it",
                        base_dir.display()
                    );
                    EXIT_MODEL_ABSENT
                }
            }
            cli::Mode::Fake => {
                println!("--check-only is a no-op under --fake: there is no model to check");
                EXIT_OK
            }
        };
    }

    let golden_set = match golden::load(&cli.golden_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("golden set validation failed: {e}");
            return EXIT_GOLDEN_INVALID;
        }
    };

    let embedder = match cli.mode {
        cli::Mode::Fake => EvalEmbedder::fake(),
        cli::Mode::Real => match EvalEmbedder::real(&base_dir).await {
            Ok(e) => e,
            Err(e) => {
                eprintln!("failed to load the real embedding model: {e}");
                return EXIT_RUNTIME_FAILURE;
            }
        },
    };

    let seeded = match seed::seed(&golden_set.corpus, &embedder).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("seeding failed: {e}");
            return EXIT_RUNTIME_FAILURE;
        }
    };

    // The temp store is left on disk on purpose: when a number in the results
    // JSON looks wrong, the database that produced it is still there to be
    // opened. Printing the path is what makes that recoverable — a per-run
    // uuid directory nobody was told about is not a forensic trace.
    println!("seeded {} rows into {}", golden_set.corpus.len(), seeded.store_path.display());

    let lang_by_ref: HashMap<&str, &str> = golden_set
        .corpus
        .iter()
        .filter_map(|row| row.lang.as_deref().map(|lang| (row.ref_.as_str(), lang)))
        .collect();

    let mut per_case = Vec::new();
    let mut blocked = Vec::new();
    for case in &golden_set.retrieval_cases {
        let embedding = match embedder.embed(&case.query).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("failed to embed query '{}': {e}", case.query);
                return EXIT_RUNTIME_FAILURE;
            }
        };
        let hits = match seeded.store.search(&embedding, seed::AGENT_ID, K_POOL, K_FINAL).await {
            Ok(h) => h,
            Err(e) => {
                eprintln!("search failed for query '{}': {e}", case.query);
                return EXIT_RUNTIME_FAILURE;
            }
        };
        let ranked_refs: Vec<String> = hits
            .iter()
            .filter_map(|h| seeded.refs.ref_of(h.id))
            .map(|r| r.to_string())
            .collect();
        let expected: HashSet<String> = case.expected_refs.iter().cloned().collect();

        let recall_1 = metrics::recall_at_k(&ranked_refs, &expected, 1);
        let recall_5 = metrics::recall_at_k(&ranked_refs, &expected, 5);
        let recall_10 = metrics::recall_at_k(&ranked_refs, &expected, 10);
        let mrr = metrics::mrr_contribution(&ranked_refs, &expected);
        let ndcg_10 = metrics::ndcg_at_10(&ranked_refs, &expected);

        let result = report::CaseResult {
            query: case.query.clone(),
            expected_refs: case.expected_refs.clone(),
            ranked_refs,
            recall_1,
            recall_5,
            recall_10,
            mrr,
            ndcg_10,
            status: case.status,
            lang: case_lang(&case.expected_refs, &lang_by_ref),
        };

        if case.status == CaseStatus::KnownRed {
            // The concrete, code-free signal that the tier-1 exclusion this
            // case is blocked on has been lifted: a case that structurally
            // could not score now scores.
            if recall_10 > 0.0 {
                println!("KNOWN_RED CASE NOW PASSING: '{}'", case.query);
            }
            blocked.push(result);
        } else {
            per_case.push(result);
        }
    }

    let mut contradiction_d = Vec::new();
    let mut no_contradiction_d = Vec::new();
    let mut identical_d = Vec::new();
    // Same three buckets again, kept per declared language. `lang` is a
    // required field on a contradiction pair precisely so this split is always
    // possible; the production detector applies ONE threshold to both
    // languages, so whether one threshold can serve both is a question the
    // combined curve cannot answer.
    let mut by_lang: BTreeMap<String, (Vec<f64>, Vec<f64>, Vec<f64>)> = BTreeMap::new();
    for pair in &golden_set.contradiction_pairs {
        let a = match embedder.embed(&pair.a).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("failed to embed a contradiction-pair text: {e}");
                return EXIT_RUNTIME_FAILURE;
            }
        };
        let b = match embedder.embed(&pair.b).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("failed to embed a contradiction-pair text: {e}");
                return EXIT_RUNTIME_FAILURE;
            }
        };
        let distance = l2_distance(&a, &b);
        let buckets = by_lang.entry(pair.lang.clone()).or_default();
        match pair.label {
            PairLabel::Contradiction => {
                contradiction_d.push(distance);
                buckets.0.push(distance);
            }
            PairLabel::NoContradiction => {
                no_contradiction_d.push(distance);
                buckets.1.push(distance);
            }
            PairLabel::Identical => {
                identical_d.push(distance);
                buckets.2.push(distance);
            }
        }
    }
    let curve = metrics::threshold_curve(&contradiction_d, &no_contradiction_d, &identical_d);
    let curve_by_lang: Vec<report::LangCurve> = by_lang
        .into_iter()
        .map(|(lang, (c, n, i))| report::LangCurve {
            lang,
            n_pairs: c.len() + n.len() + i.len(),
            curve: metrics::threshold_curve(&c, &n, &i),
        })
        .collect();

    let aggregate = report::aggregate(&per_case);
    let by_lang_aggregate = report::aggregate_by_lang(&per_case);
    let mode = match cli.mode {
        cli::Mode::Fake => report::Mode::Fake,
        cli::Mode::Real => report::Mode::Real,
    };

    let results = report::RunResults {
        date: current_date_string(),
        mode,
        model_id: embedder.model_id().to_string(),
        git_sha: report::git_sha(),
        k_pool: report::KPool { memory: 30, episodic: 5, procedural: 5 },
        k_final: K_FINAL,
        corpus_size: golden_set.corpus.len(),
        n_retrieval_cases: golden_set.retrieval_cases.len(),
        n_contradiction_pairs: golden_set.contradiction_pairs.len(),
        known_bugs_live: known_bugs_live(cli.mode),
        golden_set: report::GoldenSetProvenance {
            provisional: true,
            note: "only the ~13-case template sample is authored so far; the domain cases from real \
                   usage are still pending, so a later run against a fuller golden set will move every \
                   metric for reasons that have nothing to do with retrieval quality"
                .to_string(),
        },
        retrieval: report::RetrievalResults {
            aggregate,
            by_lang: by_lang_aggregate,
            per_case,
            blocked,
        },
        contradiction: report::ContradictionResults { curve, by_lang: curve_by_lang },
        norm_check: report::NormCheck {
            asserted: cli.mode == cli::Mode::Real,
            mean_norm: seeded.mean_norm,
            max_deviation: seeded.max_norm_deviation,
        },
    };

    report::print_stdout(&results);

    match report::write_json(&results, &cli.out_dir, cli.force) {
        Ok(path) => {
            println!("results written to {}", path.display());
            EXIT_OK
        }
        Err(report::ReportError::AlreadyExists { path }) => {
            println!("results file already exists: {}", path.display());
            EXIT_RESULTS_EXISTS
        }
        Err(e) => {
            eprintln!("failed to write results: {e}");
            EXIT_RUNTIME_FAILURE
        }
    }
}

/// The language to attribute a retrieval case to: the `lang` its expected
/// corpus rows agree on, or `None` when any of them leaves it unset or they
/// disagree.
///
/// A case carries no `lang` of its own — only corpus rows do — so this join is
/// the only way the two meet. Requiring unanimity rather than taking the first
/// row's value is deliberate: a case whose expected rows span both languages
/// is genuinely cross-lingual, and filing it under either one would make that
/// language's number describe something other than what it claims to.
fn case_lang(expected_refs: &[String], lang_by_ref: &HashMap<&str, &str>) -> Option<String> {
    let mut langs = expected_refs.iter().map(|r| lang_by_ref.get(r.as_str()).copied());
    let first = langs.next().flatten()?;
    if langs.all(|lang| lang == Some(first)) {
        Some(first.to_string())
    } else {
        None
    }
}

fn l2_distance(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| ((*x as f64) - (*y as f64)).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// Defects known to be live in the retrieval path at run time, recorded so a
/// reader of the results JSON does not mistake a number standing on a bug
/// for a finished retrieval implementation. Listed as facts about the
/// current codebase, not derived by scanning it — this harness measures
/// around these bugs, it does not detect them.
fn known_bugs_live(mode: cli::Mode) -> Vec<String> {
    let mut bugs = vec![];
    if mode == cli::Mode::Fake {
        // FakeEmbedder fills 384 floats uniformly in [-1, 1] with no
        // normalization (‖v‖ ≈ 11.3 rather than 1.0), so distances measured
        // against it are not on the scale the contradiction sweep's
        // 0.10..0.40 thresholds assume. Every fake pair sits far outside
        // that range, so the curve is degenerate under --fake: recall = 0
        // and both false-positive rates = 0 at every threshold.
        bugs.push("fake-embedder-unnormalized-contradiction-sweep-degenerate".to_string());
    }
    bugs
}

/// Converts a day count since the Unix epoch into a proleptic-Gregorian
/// `(year, month, day)` triple. This is Howard Hinnant's public-domain
/// `civil_from_days` algorithm, reproduced here rather than pulling in a
/// date-time crate for a single `YYYY-MM-DD` stamp — nothing else in this
/// harness needs one.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// The current UTC date as `YYYY-MM-DD`, read from the system clock at call
/// time. Never hardcoded: every run's results JSON must be stamped with when
/// it actually ran, not when this file was written.
fn current_date_string() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (seconds / 86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_reference_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2000-01-01 is the well-known "Y2K epoch day" reference: 30 years
        // (10950 days) plus 7 leap days (1972..1996) = 10957.
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
        // 1970 is not a leap year: day 59 = Jan(31) + Feb(28) days in, i.e.
        // the first day of March. Exercises the month-rollover arithmetic.
        assert_eq!(civil_from_days(59), (1970, 3, 1));
    }

    #[test]
    fn case_lang_requires_every_expected_row_to_agree() {
        let lang_by_ref: HashMap<&str, &str> =
            [("c1", "es"), ("c2", "es"), ("c3", "en")].into_iter().collect();

        assert_eq!(
            case_lang(&["c1".to_string(), "c2".to_string()], &lang_by_ref),
            Some("es".to_string()),
            "unanimous rows attribute the case to their shared language"
        );
        assert_eq!(
            case_lang(&["c1".to_string(), "c3".to_string()], &lang_by_ref),
            None,
            "a cross-lingual case belongs to neither language's bucket"
        );
        assert_eq!(
            case_lang(&["c1".to_string(), "c9".to_string()], &lang_by_ref),
            None,
            "one row without a declared lang makes the whole case unattributable"
        );
        assert_eq!(case_lang(&[], &lang_by_ref), None, "no expected rows, nothing to attribute");
    }

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rag-eval-main-test-{tag}-{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn fake_end_to_end_run_produces_a_valid_results_json() {
        // Two queries are set to the exact text of the corpus rows they
        // should retrieve: FakeEmbedder is deterministic, so identical text
        // yields identical vectors and an exact-self-match distance of 0 —
        // the unique global minimum among any set of fake vectors, so the
        // self-matching row's score cannot be beaten by a decoy no matter
        // which way the live L2-vs-cosine scoring bug pushes the ordering
        // among the OTHER (non-zero-distance) candidates.
        let golden_json = r#"{
            "corpus": [
                {"ref": "c1", "content": "the retrieval decay term compares milliseconds against seconds", "memory_type": "chat_message", "importance": 3.0, "lang": "en"},
                {"ref": "c2", "content": "el indice knn mide distancia l2 cruda y no similitud coseno", "memory_type": "chat_message", "importance": 3.0, "lang": "es"},
                {"ref": "c3", "content": "an unrelated third row that no query targets directly", "memory_type": "chat_message", "importance": 3.0}
            ],
            "retrieval_cases": [
                {"query": "the retrieval decay term compares milliseconds against seconds", "corpus_refs": ["c1","c2","c3"], "expected_refs": ["c1"]},
                {"query": "el indice knn mide distancia l2 cruda y no similitud coseno", "corpus_refs": ["c1","c2","c3"], "expected_refs": ["c2"]}
            ],
            "contradiction_pairs": [
                {"a": "a first statement", "b": "a second, unrelated statement", "label": "no_contradiction", "lang": "en"},
                {"a": "una primera afirmacion", "b": "una segunda afirmacion sin relacion", "label": "no_contradiction", "lang": "es"}
            ]
        }"#;

        let golden_path = temp_path("golden.json");
        std::fs::write(&golden_path, golden_json).expect("write golden fixture");
        let out_dir = temp_path("out");

        let args = vec![
            "rag-eval".to_string(),
            "--fake".to_string(),
            "--golden".to_string(),
            golden_path.to_string_lossy().to_string(),
            "--out".to_string(),
            out_dir.to_string_lossy().to_string(),
        ];

        let code = run(args.into_iter()).await;
        assert_eq!(code, EXIT_OK, "a valid --fake run must exit 0");

        let expected_filename = format!("{}-fake.json", current_date_string());
        let results_path = out_dir.join(&expected_filename);
        assert!(results_path.exists(), "results file must exist at {}", results_path.display());

        let contents = std::fs::read_to_string(&results_path).expect("must read the results file");
        let json: serde_json::Value = serde_json::from_str(&contents).expect("results must be valid JSON");

        assert_eq!(json["mode"], "fake");
        assert_eq!(json["corpus_size"], 3);
        assert_eq!(json["n_retrieval_cases"], 2);
        assert_eq!(json["n_contradiction_pairs"], 2);
        assert_eq!(json["golden_set"]["provisional"], true);
        let known_bugs: Vec<&str> =
            json["known_bugs_live"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert!(
            known_bugs.contains(&"fake-embedder-unnormalized-contradiction-sweep-degenerate"),
            "--fake must report its own degenerate-sweep bug: got {known_bugs:?}"
        );
        assert_eq!(json["retrieval"]["per_case"].as_array().unwrap().len(), 2);
        assert_eq!(json["retrieval"]["blocked"].as_array().unwrap().len(), 0);
        assert_eq!(json["contradiction"]["curve"].as_array().unwrap().len(), 7);

        let recall_1 = json["retrieval"]["aggregate"]["recall@1"].as_f64().unwrap();
        assert_eq!(recall_1, 1.0, "both self-matching queries must land at rank 1");

        // Per-language split: the corpus declares one `en` row and one `es`
        // row, each targeted by exactly one case, so both languages get their
        // own bucket of one case. c3 declares no lang and no case expects it.
        let by_lang = json["retrieval"]["by_lang"].as_array().unwrap();
        assert_eq!(by_lang.len(), 2, "one bucket per declared language");
        assert_eq!(by_lang[0]["lang"], "en");
        assert_eq!(by_lang[0]["n_cases"], 1);
        assert_eq!(by_lang[1]["lang"], "es");
        assert_eq!(by_lang[1]["n_cases"], 1);

        let curve_by_lang = json["contradiction"]["by_lang"].as_array().unwrap();
        assert_eq!(curve_by_lang.len(), 2, "one sweep per declared pair language");
        assert_eq!(curve_by_lang[0]["lang"], "en");
        assert_eq!(curve_by_lang[0]["n_pairs"], 1);
        assert_eq!(curve_by_lang[0]["curve"].as_array().unwrap().len(), 7);
        assert_eq!(curve_by_lang[1]["lang"], "es");

        std::fs::remove_file(&golden_path).ok();
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[tokio::test]
    async fn invalid_golden_set_aborts_before_seeding_with_exit_code_2() {
        let golden_json = r#"{
            "corpus": [
                {"ref": "c1", "content": "x", "memory_type": "not_a_real_type", "importance": 1.0}
            ],
            "retrieval_cases": [],
            "contradiction_pairs": []
        }"#;
        let golden_path = temp_path("golden-invalid.json");
        std::fs::write(&golden_path, golden_json).expect("write golden fixture");
        let out_dir = temp_path("out-invalid");

        let args = vec![
            "rag-eval".to_string(),
            "--fake".to_string(),
            "--golden".to_string(),
            golden_path.to_string_lossy().to_string(),
            "--out".to_string(),
            out_dir.to_string_lossy().to_string(),
        ];

        let code = run(args.into_iter()).await;
        assert_eq!(code, EXIT_GOLDEN_INVALID);
        assert!(!out_dir.exists(), "an invalid golden set must abort before anything is seeded or written");

        std::fs::remove_file(&golden_path).ok();
    }

    #[tokio::test]
    async fn missing_required_mode_flag_exits_with_the_runtime_failure_code() {
        let args = vec!["rag-eval".to_string()];
        let code = run(args.into_iter()).await;
        assert_eq!(code, EXIT_RUNTIME_FAILURE);
    }

    #[tokio::test]
    async fn help_flag_exits_zero_without_touching_anything() {
        let args = vec!["rag-eval".to_string(), "--help".to_string()];
        let code = run(args.into_iter()).await;
        assert_eq!(code, EXIT_OK);
    }
}
