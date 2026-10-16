mod agent_view;
mod audit;
mod cli;
mod diff;
mod docs;
mod follow_up;
mod golden;
mod metrics;
mod report;
mod seed;
mod similarity;
mod stats;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use clawg_core::eval_api::{
    compose_query_context, model_is_cached, query_budget_tokens,
    EmbedRole, EvalEmbedder, EvalScope, EvalScoringConfig, EvalTokenCounter, MIGRATION_FTS_TOKENIZER,
    EMBED_MAX_TOKENS, SPECIAL_TOKEN_RESERVE,
};

use golden::{CaseStatus, CaseSubset, PairLabel};

/// Result-pool sizes handed to `EvalStore::search`. `memory` is the widest
/// pool because every golden-set corpus row seeds into the memory table;
/// `episodic`/`procedural` stay narrow since this harness never seeds either
/// of those tables, so a wider pool there would only cost time.
///
/// `lexical` matches the shipped application's own default, and
/// `k_pool_mirrors_the_production_lexical_budget` is what keeps it matching.
/// A harness whose baseline runs a channel configuration production does not
/// run measures a system nobody uses: every committed number would describe a
/// retrieval path no user's query ever takes, and a comparison against those
/// numbers would be a comparison against fiction. Turning the channel off for
/// a run is what `--sweep-lexical 0` is for — it is a question the operator
/// asks, not the default state of the instrument.
const K_POOL: EvalScope = EvalScope { memory: Some(30), episodic: Some(5), procedural: Some(5), lexical: Some(20) };

/// Cutoff production truncates search results to. Recall@10/nDCG@10 are
/// defined against exactly this cutoff — raising it here would silently
/// change what those two field names mean.
const K_FINAL: usize = 10;

/// Tag of a `--lexical-only` results file.
const LEXICAL_ONLY_SLUG: &str = "lexical-only";

const EXIT_OK: i32 = 0;
const EXIT_RUNTIME_FAILURE: i32 = 1;
const EXIT_GOLDEN_INVALID: i32 = 2;
const EXIT_MODEL_ABSENT: i32 = 3;
const EXIT_RESULTS_EXISTS: i32 = 4;
/// The golden set loaded and scored, but `--audit` proved it cannot support a
/// claim. Distinct from `EXIT_GOLDEN_INVALID`, which means the file is
/// structurally broken: this one means the file is well-formed and useless.
const EXIT_GOLDEN_UNFIT: i32 = 5;
/// `--diff --fail-on-regression` found a primary metric worse at p < 0.05.
const EXIT_REGRESSION: i32 = 6;

// Single-threaded runtime: the crate's declared tokio features (sync, macros,
// time, rt, process, io-util) do not include `rt-multi-thread`, so the
// default multi-thread flavor would not compile here.
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let code = run(std::env::args()).await;
    std::process::exit(code);
}

/// Production's own base directory, resolved through the shared
/// `clawg_core::paths` resolver. Pointing `--real` at anywhere else would mean this
/// harness never benefits from a model the running application already
/// downloaded, and `--check-only` would answer a question about a cache the
/// application never looks at.
fn production_base_dir() -> PathBuf {
    clawg_core::paths::data_dir_from_env()
        .unwrap_or_else(|e| panic!("failed to resolve data directory: {e}"))
}

async fn load_embedder(mode: cli::Mode, base_dir: &std::path::Path) -> Result<EvalEmbedder, i32> {
    match mode {
        cli::Mode::Fake => Ok(EvalEmbedder::fake()),
        cli::Mode::Real => EvalEmbedder::real(base_dir).await.map_err(|e| {
            eprintln!("failed to load the real embedding model: {e}");
            EXIT_RUNTIME_FAILURE
        }),
    }
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
        Ok(cli::ParseOutcome::Diff { a, b, fail_on_regression }) => return run_diff(&a, &b, fail_on_regression),
        Ok(cli::ParseOutcome::Run(cli)) => cli,
        Err(e) => {
            // Not one of the dedicated exit codes (0/2/3/4): a usage
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
    // `--fake` it is a harmless no-op, since there is no model to
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

    if cli.docs {
        return match load_embedder(cli.mode, &base_dir).await {
            Ok(embedder) => docs::run(&cli, &embedder, &base_dir).await,
            Err(code) => code,
        };
    }

    let loaded_golden = match golden::load(&cli.golden_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("golden set validation failed: {e}");
            return EXIT_GOLDEN_INVALID;
        }
    };
    let golden_set = &loaded_golden.set;

    let embedder = match load_embedder(cli.mode, &base_dir).await {
        Ok(e) => e,
        Err(code) => return code,
    };

    // The token counter query-context composition needs, for both the
    // always-on history composition below and the `--follow-up` arms.
    // `--real`'s tokenizer.json is already on disk by this point —
    // `EvalEmbedder::real` downloads and verifies the whole model bundle,
    // tokenizer file included, before returning.
    let counter = match cli.mode {
        cli::Mode::Fake => EvalTokenCounter::fake(),
        cli::Mode::Real => match EvalTokenCounter::real(&base_dir) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("failed to load the tokenizer for query-context composition: {e}");
                return EXIT_RUNTIME_FAILURE;
            }
        },
    };
    let budget = query_budget_tokens(
        EMBED_MAX_TOKENS,
        SPECIAL_TOKEN_RESERVE,
        counter.count(embedder.query_prefix()),
    );

    // Branches off before the direct seeding below: this mode stores the corpus
    // through the real ingest pipeline into a store of its own.
    if cli.agent_view {
        return run_agent_view(&cli, &loaded_golden, &embedder, &counter, &base_dir).await;
    }

    let seeded = match seed::seed(&golden_set.corpus, &embedder, cli.fts_tokenizer.as_deref()).await {
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

    // Every query embedded exactly once, up front — shared unchanged across
    // every grid point's search below, so an N-point sweep never re-embeds
    // the same case N times. Search itself (the part that actually varies
    // per point) happens later, inside the grid loop.
    let mut embedded_cases: Vec<EmbeddedCase> = Vec::with_capacity(golden_set.retrieval_cases.len());
    for case in &golden_set.retrieval_cases {
        // `history` is chronological oldest-first, as a human writes it;
        // `compose_query_context` wants newest-first, hence the reverse.
        // Empty history (every case in the current golden.json) makes this
        // byte-identical to embedding `case.query` alone — one code path,
        // not an `if history.is_empty()` branch to rot.
        let prior_turns: Vec<String> = case.history.iter().rev().cloned().collect();
        let embed_text =
            compose_query_context(&case.query, &prior_turns, budget, &|s: &str| counter.count(s));

        // Query: must match production's prompt_builder.rs role — this vector
        // also feeds the similarity block's ranking below, so a mismatched
        // role here would make that measurement describe a different space
        // than the one production actually searches.
        let embedding = match embedder.embed_with_role(&embed_text, EmbedRole::Query).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("failed to embed query '{}': {e}", case.query);
                return EXIT_RUNTIME_FAILURE;
            }
        };
        embedded_cases.push(EmbeddedCase {
            query: case.query.clone(),
            expected_refs: case.expected_refs.clone(),
            corpus_refs: case.corpus_refs.clone(),
            status: case.status,
            query_lang: case.query_lang.clone(),
            lang: case_lang(&case.expected_refs, &lang_by_ref),
            subset: case.subset,
            stale_refs: case.stale_refs.clone(),
            embedding,
        });
    }

    // Pure cosine geometry: independent of every scoring config, so it is
    // computed exactly once and embedded identically into every grid point's
    // results file — the invariant the REVERT check (`similarity` byte-
    // identical to baseline) depends on being trivially checkable across
    // points. Populated only under --real — see `EmbeddedCase`'s query
    // vector, FakeEmbedder has no learned geometry for this to describe.
    let similarity_results = if cli.mode == cli::Mode::Real {
        let sim_sources: Vec<SimCaseSource> = embedded_cases
            .iter()
            .filter(|c| c.status != CaseStatus::KnownRed && c.subset != CaseSubset::Unanswerable)
            .map(|c| SimCaseSource {
                query: c.query.clone(),
                expected_refs: c.expected_refs.clone(),
                corpus_refs: c.corpus_refs.clone(),
                query_vector: c.embedding.clone(),
                query_lang: c.query_lang.clone(),
                lang: c.lang.clone(),
                subset: c.subset,
            })
            .collect();
        Some(build_similarity_results(
            &sim_sources,
            &seeded.vectors,
            K_FINAL,
            embedder.query_prefix(),
            embedder.passage_prefix(),
        ))
    } else {
        None
    };

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
        // Passage/Passage, matching production: `worker.rs`'s contradiction
        // hook compares a Passage chunk against Passage-embedded stored rows,
        // so the harness's pair loop must mirror that exactly — otherwise
        // `contradiction.curve` describes a space production never searches.
        let a = match embedder.embed_with_role(&pair.a, EmbedRole::Passage).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("failed to embed a contradiction-pair text: {e}");
                return EXIT_RUNTIME_FAILURE;
            }
        };
        let b = match embedder.embed_with_role(&pair.b, EmbedRole::Passage).await {
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

    let mode = match cli.mode {
        cli::Mode::Fake => report::Mode::Fake,
        cli::Mode::Real => report::Mode::Real,
    };
    let date = current_date_string();
    let git_sha = report::git_sha();
    let model_id = embedder.model_id().to_string();
    let known_bugs = known_bugs_live(cli.mode);
    let golden_provenance = report::GoldenSetProvenance::derive(
        golden_set.retrieval_cases.len(),
        golden_set.contradiction_pairs.len(),
        loaded_golden.content_sha256.clone(),
    );

    // The tokenizer is a property of the whole run, not of a grid point:
    // `memory_fts` is rebuilt once during seeding, so it is folded into the
    // tag ONCE, ahead of any per-point slug — computed here (rather than
    // just below the grid, where it used to live) because the `--follow-up`
    // branch needs it too and returns before the grid is ever built.
    let run_tag = fold_tag(cli.tag.as_deref(), tokenizer_slug(cli.fts_tokenizer.as_deref()).as_deref());
    // Recorded as the tokenizer actually in effect, never as "the flag was
    // absent": a results file exists to answer what produced its numbers, and
    // the absence of a flag is a fact about a command line, not about an index.
    let fts_tokenizer =
        cli.fts_tokenizer.clone().unwrap_or_else(|| MIGRATION_FTS_TOKENIZER.to_string());

    if cli.follow_up {
        return run_follow_up(FollowUpRunContext {
            golden_set,
            embedder: &embedder,
            seeded: &seeded,
            counter: &counter,
            budget,
            lang_by_ref: &lang_by_ref,
            date,
            mode,
            model_id,
            git_sha,
            known_bugs,
            golden_provenance,
            fts_tokenizer,
            run_tag,
            contradiction_curve: curve.clone(),
            contradiction_curve_by_lang: curve_by_lang.clone(),
            out_dir: &cli.out_dir,
            force: cli.force,
        })
        .await;
    }

    // Zero sweep flags on all three axes degenerates `build_grid` to exactly
    // one point: `EvalScoringConfig::default()` — never
    // `with_importance_span`/`with_cosine_floor` — with `K_POOL`'s own lexical
    // budget left untouched. That is the regression guard on the sweep
    // plumbing itself: a flagless run measures the shipped configuration, not
    // one the flag parser invented along the way.
    //
    // It is deliberately NOT the claim that a flagless run reproduces the
    // numbers from before the lexical channel existed. `K_POOL.lexical` is
    // `Some(20)`, matching the application's own default, so a flagless run
    // exercises that channel exactly as production does — which is the only
    // way committed numbers can describe what users actually get. Reproducing
    // the pre-channel numbers is what `--sweep-lexical 0` is for, and every
    // results file records the `k_pool.lexical` that produced it, so the two
    // are never confusable after the fact.
    let grid = build_grid(&cli.sweep_imp, &cli.sweep_cos_floor, &cli.sweep_lexical);
    let is_sweep =
        !cli.sweep_imp.is_empty() || !cli.sweep_cos_floor.is_empty() || !cli.sweep_lexical.is_empty();

    let mut sweep_points: Vec<report::SweepPoint> = Vec::new();
    let mut any_unusable = false;

    for point in &grid {
        // Resolved once per point rather than per case, since it does not
        // vary across `embedded_cases`.
        let scope = EvalScope { lexical: resolve_lexical_scope(point.lexical, K_POOL.lexical), ..K_POOL };

        let mut per_case = Vec::new();
        let mut blocked = Vec::new();
        let mut unanswerable = Vec::new();
        for ec in &embedded_cases {
            let ids: Vec<i64> = if cli.lexical_only {
                seeded.store.lexical_search(&ec.query, seed::AGENT_ID, K_FINAL).await
            } else {
                match seeded
                    .store
                    .search(&ec.embedding, seed::AGENT_ID, scope, K_FINAL, point.scoring, Some(&ec.query))
                    .await
                {
                    Ok(h) => h.iter().map(|h| h.id).collect(),
                    Err(e) => {
                        eprintln!("search failed for query '{}': {e}", ec.query);
                        return EXIT_RUNTIME_FAILURE;
                    }
                }
            };
            let ranked_refs: Vec<String> =
                ids.iter().filter_map(|id| seeded.refs.ref_of(*id)).map(|r| r.to_string()).collect();
            let expected: HashSet<String> = ec.expected_refs.iter().cloned().collect();

            let recall_1 = metrics::recall_at_k(&ranked_refs, &expected, 1);
            let recall_5 = metrics::recall_at_k(&ranked_refs, &expected, 5);
            let recall_10 = metrics::recall_at_k(&ranked_refs, &expected, 10);
            let mrr = metrics::mrr_contribution(&ranked_refs, &expected);
            let ndcg_10 = metrics::ndcg_at_10(&ranked_refs, &expected);

            let abstained = (ec.subset == CaseSubset::Unanswerable).then_some(ranked_refs.is_empty());
            let fresh_first = (ec.subset == CaseSubset::Update)
                .then(|| metrics::fresh_first(&ranked_refs, &ec.expected_refs[0], &ec.stale_refs));
            let result = report::CaseResult {
                query: ec.query.clone(),
                expected_refs: ec.expected_refs.clone(),
                ranked_refs,
                recall_1,
                recall_5,
                recall_10,
                mrr,
                ndcg_10,
                status: ec.status,
                query_lang: ec.query_lang.clone(),
                lang: ec.lang.clone(),
                subset: ec.subset,
                abstained,
                fresh_first,
            };

            if abstained.is_some() {
                unanswerable.push(result);
            } else if ec.status == CaseStatus::KnownRed {
                // The concrete, code-free signal that the tier-1 exclusion
                // this case is blocked on has been lifted: a case that
                // structurally could not score now scores.
                if recall_10 > 0.0 {
                    println!("KNOWN_RED CASE NOW PASSING: '{}'", ec.query);
                }
                blocked.push(result);
            } else {
                per_case.push(result);
            }
        }

        let retrieval = report::RetrievalResults::from_cases(per_case, blocked, unanswerable);
        let aggregate = retrieval.aggregate;
        let per_case = &retrieval.per_case;
        // Computed unconditionally, not gated behind `cli.audit`: a sweep's
        // SweepPoint carries these two numbers regardless of whether the
        // full AuditReport (saturation/mirror-case gating) was asked for —
        // the gate that reads them needs every point's value, not just the
        // ones a human happened to also request `--audit` for.
        let dominance = audit::importance_rank_dominance(per_case, &golden_set.corpus);
        let modal = audit::modal_rank_agreement(per_case);

        // Audited over the active cases only, matching what `aggregate`
        // reports: a `known_red` case is blocked by a change that does not
        // exist yet, so its ranking says nothing about whether the golden
        // set can discriminate.
        let audit_report = cli.audit.then(|| audit::audit(per_case, &golden_set.corpus));

        let arm = if cli.lexical_only { Some(LEXICAL_ONLY_SLUG.to_string()) } else { point_slug(point) };
        let tag = fold_tag(run_tag.as_deref(), arm.as_deref());

        let results = report::RunResults {
            date: date.clone(),
            mode,
            tag,
            model_id: model_id.clone(),
            git_sha: git_sha.clone(),
            k_pool: if cli.lexical_only {
                report::KPool { memory: 0, episodic: 0, procedural: 0, lexical: Some(K_FINAL) }
            } else {
                report::KPool { memory: 30, episodic: 5, procedural: 5, lexical: scope.lexical }
            },
            fts_tokenizer: fts_tokenizer.clone(),
            k_final: K_FINAL,
            corpus_size: golden_set.corpus.len(),
            n_retrieval_cases: golden_set.retrieval_cases.len(),
            n_contradiction_pairs: golden_set.contradiction_pairs.len(),
            known_bugs_live: known_bugs.clone(),
            golden_set: golden_provenance.clone(),
            scoring_config: point.scoring,
            retrieval,
            contradiction: report::ContradictionResults { curve: curve.clone(), by_lang: curve_by_lang.clone() },
            norm_check: report::NormCheck {
                asserted: cli.mode == cli::Mode::Real,
                mean_norm: seeded.mean_norm,
                max_deviation: seeded.max_norm_deviation,
            },
            audit: audit_report,
            similarity: similarity_results.clone(),
            agent_view: None,
        };

        report::print_stdout(&results);
        if let Some(a) = &results.audit {
            audit::print_report(a);
        }
        if results.audit.as_ref().is_some_and(|a| a.verdict == audit::Verdict::Unusable) {
            any_unusable = true;
        }

        // The results file is written even when the audit fails. A run that
        // proved its own golden set unfit is exactly the evidence a later
        // reader needs in order to discount the numbers above it, and
        // withholding the artifact would leave that conclusion undocumented.
        let path = match report::write_json(&results, &cli.out_dir, cli.force) {
            Ok(path) => {
                println!("results written to {}", path.display());
                path
            }
            Err(report::ReportError::AlreadyExists { path }) => {
                println!("results file already exists: {}", path.display());
                return EXIT_RESULTS_EXISTS;
            }
            Err(e) => {
                eprintln!("failed to write results: {e}");
                return EXIT_RUNTIME_FAILURE;
            }
        };

        if is_sweep {
            sweep_points.push(report::SweepPoint {
                slug: point_slug(point).unwrap_or_default(),
                results_file: path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
                scoring_config: point.scoring,
                aggregate,
                importance_rank_dominance: dominance,
                modal_rank_agreement: modal,
            });
        }
    }

    if is_sweep {
        report::sort_sweep_points(&mut sweep_points);
        let index = report::SweepIndex {
            date,
            mode,
            tag: run_tag.clone(),
            model_id,
            git_sha,
            points: sweep_points,
        };
        match report::write_sweep_index(&index, &cli.out_dir, cli.force) {
            Ok(path) => println!("sweep index written to {}", path.display()),
            Err(report::ReportError::AlreadyExists { path }) => {
                println!("sweep index already exists: {}", path.display());
                return EXIT_RESULTS_EXISTS;
            }
            Err(e) => {
                eprintln!("failed to write sweep index: {e}");
                return EXIT_RUNTIME_FAILURE;
            }
        }
    }

    // A failed audit only overrides a run that was otherwise clean: an
    // unwritten results file is the more urgent problem, and masking it
    // behind the audit code would hide it — checked only after every point
    // has been written.
    if any_unusable {
        return EXIT_GOLDEN_UNFIT;
    }
    EXIT_OK
}

/// Runs `--agent-view`: scores what the MCP tools would hand an agent for
/// every case, and writes a results file whose `agent_view` block carries it.
async fn run_agent_view(
    cli: &cli::Cli,
    loaded_golden: &golden::LoadedGolden,
    embedder: &EvalEmbedder,
    counter: &EvalTokenCounter,
    base_dir: &std::path::Path,
) -> i32 {
    let scratch = agent_view::runner::scratch_dir();
    let input = agent_view::runner::AgentViewInput {
        golden_set: &loaded_golden.set,
        embedder,
        counter,
        // `--real` chunks with the tokenizer the profile already holds; under
        // `--fake` the scratch directory has none, which selects the byte
        // fallback the fake embedder is paired with.
        tokenizer_dir: match cli.mode {
            cli::Mode::Real => base_dir,
            cli::Mode::Fake => &scratch,
        },
    };
    let view = match agent_view::runner::run(&input, &scratch).await {
        Ok(view) => view,
        Err(e) => {
            eprintln!("agent-view run failed: {e}");
            return EXIT_RUNTIME_FAILURE;
        }
    };
    print!("{}", agent_view::render_summary(&view));

    // Only the agent-view summary is printed: the ordinary report would show
    // an empty retrieval block, as if the ranking had been scored and failed.
    let results = build_agent_view_run_results(cli, loaded_golden, embedder, view);
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

/// Wraps an agent-view reading in the common results envelope. The store's own
/// ranking is not scored in this mode, so `retrieval` is present but empty and
/// the contradiction and norm blocks carry nothing — the same "present, empty"
/// shape a golden set with no cases would produce.
fn build_agent_view_run_results(
    cli: &cli::Cli,
    loaded_golden: &golden::LoadedGolden,
    embedder: &EvalEmbedder,
    view: agent_view::AgentViewResults,
) -> report::RunResults {
    let golden_set = &loaded_golden.set;
    let mode = match cli.mode {
        cli::Mode::Fake => report::Mode::Fake,
        cli::Mode::Real => report::Mode::Real,
    };
    report::RunResults {
        date: current_date_string(),
        mode,
        tag: fold_tag(cli.tag.as_deref(), Some("agent-view")),
        model_id: embedder.model_id().to_string(),
        git_sha: report::git_sha(),
        k_pool: report::KPool { memory: 30, episodic: 5, procedural: 5, lexical: K_POOL.lexical },
        fts_tokenizer: MIGRATION_FTS_TOKENIZER.to_string(),
        k_final: K_FINAL,
        corpus_size: golden_set.corpus.len(),
        n_retrieval_cases: golden_set.retrieval_cases.len(),
        n_contradiction_pairs: golden_set.contradiction_pairs.len(),
        known_bugs_live: known_bugs_live(cli.mode),
        golden_set: report::GoldenSetProvenance::derive(
            golden_set.retrieval_cases.len(),
            golden_set.contradiction_pairs.len(),
            loaded_golden.content_sha256.clone(),
        ),
        scoring_config: EvalScoringConfig::default(),
        retrieval: report::RetrievalResults::from_cases(vec![], vec![], vec![]),
        contradiction: report::ContradictionResults { curve: vec![], by_lang: vec![] },
        norm_check: report::NormCheck { asserted: false, mean_norm: 0.0, max_deviation: 0.0 },
        audit: None,
        similarity: None,
        agent_view: Some(view),
    }
}

/// Runs `--diff`: reads two existing results files, joins their
/// `retrieval.per_case`/`similarity.per_case` on `query`, and prints the
/// better/worse/net report. Not one of the dedicated exit codes: like a
/// CLI usage error, this happens before any golden set, model, or database is
/// touched, so a failure here is reported as a generic runtime failure rather
/// than overloading a more specific code with a meaning it does not have.
fn run_diff(a_path: &std::path::Path, b_path: &std::path::Path, fail_on_regression: bool) -> i32 {
    match diff::diff_files(a_path, b_path) {
        Ok(d) => {
            diff::print_stdout(&d);
            if fail_on_regression && diff::regressed(&d) {
                eprintln!("a primary metric got worse with p < 0.05");
                EXIT_REGRESSION
            } else {
                EXIT_OK
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            EXIT_RUNTIME_FAILURE
        }
    }
}

/// Everything [`run_follow_up`] needs, gathered in one place because it
/// branches off `run()` before the grid loop and would otherwise need a
/// dozen positional parameters. Borrowed rather than owned where an owned
/// copy already lives in `run()`'s scope, since `run_follow_up` runs to
/// completion before `run()` would need those values again anyway (it always
/// returns).
struct FollowUpRunContext<'a> {
    golden_set: &'a golden::GoldenSet,
    embedder: &'a EvalEmbedder,
    seeded: &'a seed::SeedResult,
    counter: &'a EvalTokenCounter,
    budget: usize,
    lang_by_ref: &'a HashMap<&'a str, &'a str>,
    date: String,
    mode: report::Mode,
    model_id: String,
    git_sha: String,
    known_bugs: Vec<String>,
    golden_provenance: report::GoldenSetProvenance,
    fts_tokenizer: String,
    run_tag: Option<String>,
    contradiction_curve: Vec<metrics::ThresholdCurveRow>,
    contradiction_curve_by_lang: Vec<report::LangCurve>,
    out_dir: &'a std::path::Path,
    force: bool,
}

/// Runs the `--follow-up` synthetic Ceiling/Floor/Treatment measurement:
/// scores Ceiling and Floor first, computes the resolvability verdict BEFORE
/// ever scoring Treatment (a void run has no treatment number to report
/// weakly), and writes both computed arms' results files regardless of the
/// verdict — the evidence for discounting the numbers is itself an artifact,
/// same principle as the sweep-index write further down in `run()`.
async fn run_follow_up(ctx: FollowUpRunContext<'_>) -> i32 {
    let active_cases: Vec<&golden::RetrievalCase> = ctx
        .golden_set
        .retrieval_cases
        .iter()
        .filter(|c| c.status == CaseStatus::Active && c.subset != CaseSubset::Unanswerable)
        .collect();
    let n_active = active_cases.len();

    let mut arms_by_case: Vec<(&golden::RetrievalCase, follow_up::FollowUpArms)> =
        Vec::with_capacity(n_active);
    for (i, case) in active_cases.iter().enumerate() {
        let lang = case_lang(&case.expected_refs, ctx.lang_by_ref);
        let template = follow_up::template_for(lang.as_deref(), i);
        let arms = follow_up::build_arms(&case.query, template, ctx.budget, &|s: &str| {
            ctx.counter.count(s)
        });
        arms_by_case.push((case, arms));
    }

    let ceiling_per_case =
        match score_follow_up_arm(&ctx, arms_by_case.iter().map(|(c, a)| (*c, &a.ceiling))).await {
            Ok(v) => v,
            Err(code) => return code,
        };
    let floor_per_case =
        match score_follow_up_arm(&ctx, arms_by_case.iter().map(|(c, a)| (*c, &a.floor))).await {
            Ok(v) => v,
            Err(code) => return code,
        };

    let ceiling_agg = report::aggregate(&ceiling_per_case);
    let floor_agg = report::aggregate(&floor_per_case);
    let verdict = follow_up::verdict(ceiling_agg.recall_10, floor_agg.recall_10, n_active);
    println!(
        "--follow-up resolvability gate: ceiling recall@10={:.4} floor recall@10={:.4} n_active_cases={n_active} -> {verdict:?}",
        ceiling_agg.recall_10, floor_agg.recall_10
    );

    let ceiling_results = build_follow_up_run_results(&ctx, "ceiling", ceiling_per_case);
    let floor_results = build_follow_up_run_results(&ctx, "floor", floor_per_case);
    for results in [&ceiling_results, &floor_results] {
        report::print_stdout(results);
        if let Err(e) = write_follow_up_arm_results(results, ctx.out_dir, ctx.force) {
            return e;
        }
    }

    if let Some(code) = follow_up::follow_up_exit_code(verdict) {
        println!(
            "--follow-up gate: Void — the golden set at n={n_active} cannot resolve the claimed \
             recovery effect; exiting before scoring the treatment arm"
        );
        return code;
    }

    let treatment_per_case =
        match score_follow_up_arm(&ctx, arms_by_case.iter().map(|(c, a)| (*c, &a.treatment))).await {
            Ok(v) => v,
            Err(code) => return code,
        };
    let treatment_results = build_follow_up_run_results(&ctx, "treatment", treatment_per_case);
    report::print_stdout(&treatment_results);
    if let Err(e) = write_follow_up_arm_results(&treatment_results, ctx.out_dir, ctx.force) {
        return e;
    }

    EXIT_OK
}

/// Embeds and searches one arm across every active case, in the SAME
/// `(embedding, lexical text)` shape production's own retrieval call site
/// uses — `arm.embedded` goes to the dense channel, `arm.lexical` to the
/// lexical/FTS5 channel via `Some(&arm.lexical)`. `K_POOL`/
/// `EvalScoringConfig::default()`: `--follow-up` is mutually exclusive with
/// every sweep flag, so there is exactly one scoring config to measure with.
async fn score_follow_up_arm<'a>(
    ctx: &FollowUpRunContext<'_>,
    items: impl Iterator<Item = (&'a golden::RetrievalCase, &'a follow_up::ArmQuery)>,
) -> Result<Vec<report::CaseResult>, i32> {
    let mut per_case = Vec::new();
    for (case, arm) in items {
        let embedding = match ctx.embedder.embed_with_role(&arm.embedded, EmbedRole::Query).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("failed to embed follow-up arm text for '{}': {e}", case.query);
                return Err(EXIT_RUNTIME_FAILURE);
            }
        };
        let hits = match ctx
            .seeded
            .store
            .search(&embedding, seed::AGENT_ID, K_POOL, K_FINAL, EvalScoringConfig::default(), Some(&arm.lexical))
            .await
        {
            Ok(h) => h,
            Err(e) => {
                eprintln!("search failed for '{}': {e}", case.query);
                return Err(EXIT_RUNTIME_FAILURE);
            }
        };
        let ranked_refs: Vec<String> =
            hits.iter().filter_map(|h| ctx.seeded.refs.ref_of(h.id)).map(|r| r.to_string()).collect();
        let expected: HashSet<String> = case.expected_refs.iter().cloned().collect();

        let recall_1 = metrics::recall_at_k(&ranked_refs, &expected, 1);
        let recall_5 = metrics::recall_at_k(&ranked_refs, &expected, 5);
        let recall_10 = metrics::recall_at_k(&ranked_refs, &expected, 10);
        let mrr = metrics::mrr_contribution(&ranked_refs, &expected);
        let ndcg_10 = metrics::ndcg_at_10(&ranked_refs, &expected);

        per_case.push(report::CaseResult {
            query: case.query.clone(),
            expected_refs: case.expected_refs.clone(),
            ranked_refs,
            recall_1,
            recall_5,
            recall_10,
            mrr,
            ndcg_10,
            status: case.status,
            query_lang: case.query_lang.clone(),
            lang: case_lang(&case.expected_refs, ctx.lang_by_ref),
            subset: case.subset,
            abstained: None,
            fresh_first: None,
        });
    }
    Ok(per_case)
}

/// Assembles one arm's full `RunResults`. `audit`/`similarity` stay `None` —
/// neither describes a single synthetic arm meaningfully — and
/// `scoring_config` is always `EvalScoringConfig::default()`, the one config
/// `--follow-up` ever scores with.
fn build_follow_up_run_results(
    ctx: &FollowUpRunContext<'_>,
    arm_name: &str,
    per_case: Vec<report::CaseResult>,
) -> report::RunResults {
    report::RunResults {
        date: ctx.date.clone(),
        mode: ctx.mode,
        tag: fold_tag(ctx.run_tag.as_deref(), Some(&format!("followup-{arm_name}"))),
        model_id: ctx.model_id.clone(),
        git_sha: ctx.git_sha.clone(),
        k_pool: report::KPool { memory: 30, episodic: 5, procedural: 5, lexical: K_POOL.lexical },
        fts_tokenizer: ctx.fts_tokenizer.clone(),
        k_final: K_FINAL,
        corpus_size: ctx.golden_set.corpus.len(),
        n_retrieval_cases: ctx.golden_set.retrieval_cases.len(),
        n_contradiction_pairs: ctx.golden_set.contradiction_pairs.len(),
        known_bugs_live: ctx.known_bugs.clone(),
        golden_set: ctx.golden_provenance.clone(),
        scoring_config: EvalScoringConfig::default(),
        retrieval: report::RetrievalResults::from_cases(per_case, vec![], vec![]),
        contradiction: report::ContradictionResults {
            curve: ctx.contradiction_curve.clone(),
            by_lang: ctx.contradiction_curve_by_lang.clone(),
        },
        norm_check: report::NormCheck {
            asserted: ctx.mode == report::Mode::Real,
            mean_norm: ctx.seeded.mean_norm,
            max_deviation: ctx.seeded.max_norm_deviation,
        },
        audit: None,
        similarity: None,
        agent_view: None,
    }
}

/// Writes one arm's results file, mapping every failure onto
/// `EXIT_RUNTIME_FAILURE`/`EXIT_RESULTS_EXISTS` the same way the normal
/// per-point write further down in `run()` does.
fn write_follow_up_arm_results(
    results: &report::RunResults,
    out_dir: &std::path::Path,
    force: bool,
) -> Result<(), i32> {
    match report::write_json(results, out_dir, force) {
        Ok(path) => {
            println!("results written to {}", path.display());
            Ok(())
        }
        Err(report::ReportError::AlreadyExists { path }) => {
            println!("results file already exists: {}", path.display());
            Err(EXIT_RESULTS_EXISTS)
        }
        Err(e) => {
            eprintln!("failed to write results: {e}");
            Err(EXIT_RUNTIME_FAILURE)
        }
    }
}

/// One golden-set retrieval case with its query embedded exactly once,
/// before the grid loop. Deliberately its own type rather than reusing
/// `golden::RetrievalCase`: that type has no query vector and no resolved
/// `lang`, and adding both to it would give every OTHER reader of a
/// `RetrievalCase` fields that mean nothing outside this one path.
struct EmbeddedCase {
    query: String,
    expected_refs: Vec<String>,
    corpus_refs: Vec<String>,
    status: CaseStatus,
    query_lang: Option<String>,
    lang: Option<String>,
    subset: CaseSubset,
    stale_refs: Vec<String>,
    embedding: Vec<f32>,
}

/// One cell of the sweep grid: the config it scores with, plus the raw
/// `(min,max)`/`floor` recipe that built it. The recipe rides alongside the
/// config — not recovered by inverting it — so [`point_slug`] names a point
/// after exactly what the operator typed on the command line, not a value
/// reconstructed through floating-point division.
struct GridPoint {
    scoring: EvalScoringConfig,
    imp_range: Option<(f64, f64)>,
    cos_floor: Option<f64>,
    /// The raw `--sweep-lexical` value this point was built from, or `None`
    /// when the axis was never swept at all. Deliberately NOT the same thing
    /// as "the lexical channel is off": `Some(0)` also means off, but was
    /// asked for explicitly and gets its own `lexoff` slug so its results
    /// file is distinguishable from a run that never touched this axis.
    lexical: Option<usize>,
}

/// Builds the cartesian product of `sweep_imp` x `sweep_cos_floor` x
/// `sweep_lexical`. Each empty axis contributes exactly one `None` cell
/// rather than being dropped from the product — that is what makes zero
/// flags on every axis degenerate to exactly one point built from
/// `EvalScoringConfig::default()` (never `with_importance_span`/
/// `with_cosine_floor`, so it is bit-identical to the pre-sweep baseline)
/// with the lexical channel left at `K_POOL`'s own value, and what lets
/// sweeping only one axis leave the others at their default instead of
/// requiring all three to be specified together.
fn build_grid(sweep_imp: &[(f64, f64)], sweep_cos_floor: &[f64], sweep_lexical: &[usize]) -> Vec<GridPoint> {
    let imp_axis: Vec<Option<(f64, f64)>> =
        if sweep_imp.is_empty() { vec![None] } else { sweep_imp.iter().copied().map(Some).collect() };
    let cos_axis: Vec<Option<f64>> = if sweep_cos_floor.is_empty() {
        vec![None]
    } else {
        sweep_cos_floor.iter().copied().map(Some).collect()
    };
    let lex_axis: Vec<Option<usize>> = if sweep_lexical.is_empty() {
        vec![None]
    } else {
        sweep_lexical.iter().copied().map(Some).collect()
    };

    let mut grid = Vec::with_capacity(imp_axis.len() * cos_axis.len() * lex_axis.len());
    for imp in &imp_axis {
        for cos in &cos_axis {
            for lex in &lex_axis {
                let mut scoring = EvalScoringConfig::default();
                if let Some((min, max)) = imp {
                    scoring = scoring.with_importance_span(*min, *max);
                }
                if let Some(floor) = cos {
                    scoring = scoring.with_cosine_floor(*floor);
                }
                grid.push(GridPoint { scoring, imp_range: *imp, cos_floor: *cos, lexical: *lex });
            }
        }
    }
    grid
}

/// Resolves one point's `--sweep-lexical` value into the `EvalScope.lexical`
/// it actually searches with. `point_lexical = None` (axis never swept)
/// leaves `default` (K_POOL's own value) untouched — that is the "leave
/// K_POOL's value" contract. `Some(0)` (typed explicitly) and `Some(n)` both
/// override it: `0` turns the channel off, same end state as `None` would
/// have produced, but reached because the operator asked for it rather than
/// because the axis was never touched — which is why it still gets its own
/// `lexoff` slug from [`point_slug`] even though the resulting scope is the
/// same as the untouched case.
fn resolve_lexical_scope(point_lexical: Option<usize>, default: Option<usize>) -> Option<usize> {
    point_lexical.map(|n| (n != 0).then_some(n)).unwrap_or(default)
}

/// The filename-safe fragment identifying one grid point, or `None` for the
/// implicit default point (no sweep flags typed at all) — `None` is what
/// keeps that point's filename identical to a pre-sweep run's, satisfying the
/// "zero flags reproduces today's filename" regression guard. Fragments are
/// joined in a fixed axis order (imp, cos, lexical) so two runs of the same
/// grid always name their points identically.
fn point_slug(point: &GridPoint) -> Option<String> {
    let mut parts = Vec::with_capacity(3);
    if let Some((min, max)) = point.imp_range {
        parts.push(format!("imp{min:.2}-{max:.2}"));
    }
    if let Some(floor) = point.cos_floor {
        parts.push(format!("cos{floor:.4}"));
    }
    if let Some(n) = point.lexical {
        parts.push(if n == 0 { "lexoff".to_string() } else { format!("lex{n}") });
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("_"))
    }
}

/// The filename-safe fragment identifying the tokenizer a run rebuilt
/// `memory_fts` with, or `None` when no `--fts-tokenizer` was given.
///
/// `None` rather than the migration's own tokenizer name, deliberately: a run
/// that passes no flag must keep writing the exact filename it wrote before
/// this fragment existed, or every committed results file becomes unfindable
/// by the name it was referenced under. The distinguishing fragment is only
/// needed to separate a swapped-tokenizer run from the default one, and one
/// of the two can carry it.
///
/// Non-alphanumeric runs collapse to `-` because an allowed tokenizer name
/// can contain spaces (`unicode61 remove_diacritics 2`) and a filename
/// carrying those is a filename that needs quoting everywhere it is typed.
fn tokenizer_slug(tokenizer: Option<&str>) -> Option<String> {
    let name = tokenizer?;
    let cleaned = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    Some(format!("tok-{cleaned}"))
}

/// Folds a per-point slug into the operator-supplied `--tag`. The slug
/// becomes part of the SAME `tag` field `results_filename`/`write_json`
/// already know how to turn into a filename and guard with `--force`, rather
/// than a second axis those functions would need to learn about.
fn fold_tag(base: Option<&str>, slug: Option<&str>) -> Option<String> {
    match (base, slug) {
        (Some(b), Some(s)) => Some(format!("{b}-{s}")),
        (Some(b), None) => Some(b.to_string()),
        (None, Some(s)) => Some(s.to_string()),
        (None, None) => None,
    }
}

/// One active retrieval case's data, captured during the ranked pass, ready
/// for the similarity block to score independently. Deliberately its own
/// type rather than reusing `golden::RetrievalCase`: that type has no query
/// vector, and adding one to it would give every OTHER reader of a
/// `RetrievalCase` a field that means nothing outside this one path.
struct SimCaseSource {
    query: String,
    expected_refs: Vec<String>,
    corpus_refs: Vec<String>,
    query_vector: Vec<f32>,
    query_lang: Option<String>,
    lang: Option<String>,
    subset: CaseSubset,
}

/// Builds the pure-cosine similarity block from the query vectors captured
/// during the ranked retrieval pass and the corpus vectors captured during
/// seeding — zero extra inference either way. Every case is scored through
/// the SAME `metrics.rs` functions the ranked path already calls; there is no
/// second metric implementation a `sim` number could disagree with a
/// `ranked` number by being wrong in its own, different way.
fn build_similarity_results(
    sources: &[SimCaseSource],
    corpus_vectors: &[(String, Vec<f32>)],
    k: usize,
    query_prefix: &str,
    passage_prefix: &str,
) -> report::SimilarityResults {
    let vector_by_ref: HashMap<&str, &Vec<f32>> =
        corpus_vectors.iter().map(|(r, v)| (r.as_str(), v)).collect();

    let mut per_case = Vec::with_capacity(sources.len());
    let mut expected_gap_cases = Vec::with_capacity(sources.len());
    let mut corpus_gap_cases = Vec::with_capacity(sources.len());
    // Every (query, corpus-doc) cosine value gathered below, pooled across
    // all cases — the corpus-wide distribution `cosine_percentiles` summarizes.
    // No extra embedding call: these are the same vectors `rank_by_cosine`
    // already reduces to build `ranked_refs`, just not discarded this time.
    let mut all_cosines: Vec<f64> = Vec::with_capacity(sources.len() * corpus_vectors.len());

    for src in sources {
        let expected_set: HashSet<String> = src.expected_refs.iter().cloned().collect();
        all_cosines.extend(similarity::cosine_scores(&src.query_vector, corpus_vectors));
        let ranked_refs = similarity::rank_by_cosine(&src.query_vector, corpus_vectors, k);

        let recall_1 = metrics::recall_at_k(&ranked_refs, &expected_set, 1);
        let recall_5 = metrics::recall_at_k(&ranked_refs, &expected_set, 5);
        let recall_10 = metrics::recall_at_k(&ranked_refs, &expected_set, 10);
        let mrr = metrics::mrr_contribution(&ranked_refs, &expected_set);
        let ndcg_10 = metrics::ndcg_at_10(&ranked_refs, &expected_set);

        per_case.push(report::CaseResult {
            query: src.query.clone(),
            expected_refs: src.expected_refs.clone(),
            ranked_refs,
            recall_1,
            recall_5,
            recall_10,
            mrr,
            ndcg_10,
            status: CaseStatus::Active,
            query_lang: src.query_lang.clone(),
            lang: src.lang.clone(),
            subset: src.subset,
            abstained: None,
            fresh_first: None,
        });

        // Query-anchored rivals: every OTHER corpus vector, not just this
        // case's own pool — production search runs against the whole store,
        // so scoping rivals to `corpus_refs` would measure an easier problem.
        let expected_vectors: Vec<&[f32]> = src
            .expected_refs
            .iter()
            .filter_map(|r| vector_by_ref.get(r.as_str()))
            .map(|v| v.as_slice())
            .collect();
        let rivals: Vec<&[f32]> = corpus_vectors
            .iter()
            .filter(|(r, _)| !src.expected_refs.contains(r))
            .map(|(_, v)| v.as_slice())
            .collect();
        expected_gap_cases.push(similarity::ExpectedGapCase {
            query: &src.query_vector,
            expected: expected_vectors.clone(),
            rivals,
        });

        // Doc-anchored distractors: scoped to THIS case's own declared pool,
        // matching the mined golden set's distractor structure exactly.
        let distractors: Vec<&[f32]> = src
            .corpus_refs
            .iter()
            .filter(|r| !src.expected_refs.contains(r))
            .filter_map(|r| vector_by_ref.get(r.as_str()))
            .map(|v| v.as_slice())
            .collect();
        corpus_gap_cases.push(similarity::CorpusGapCase { expected: expected_vectors, distractors });
    }

    let cosine_percentiles = similarity::cosine_percentiles(&all_cosines);

    report::SimilarityResults {
        aggregate: report::aggregate(&per_case),
        per_case,
        mean_expected_distractor_gap: similarity::mean_expected_distractor_gap(&expected_gap_cases),
        mean_corpus_distractor_gap: similarity::mean_corpus_distractor_gap(&corpus_gap_cases),
        cosine_min: cosine_percentiles.min,
        cosine_p5: cosine_percentiles.p5,
        cosine_p25: cosine_percentiles.p25,
        cosine_p50: cosine_percentiles.p50,
        cosine_p75: cosine_percentiles.p75,
        cosine_p95: cosine_percentiles.p95,
        cosine_max: cosine_percentiles.max,
        // The live embedder's own prefixes — copied here (not hardcoded) so a
        // results file states the space it actually measured. A run whose
        // wiring silently dropped the prefix would show up as an empty
        // string here, self-refuting rather than silently wrong.
        query_prefix: query_prefix.to_string(),
        passage_prefix: passage_prefix.to_string(),
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
        // 0.20..0.50 thresholds assume. Every fake pair sits far outside
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
    use clawg_core::eval_api::production_lexical_budget;

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

    /// Runs `--fake` on `golden_json` under `tag` and returns the results file.
    async fn run_fake(golden_json: &str, tag: &str) -> serde_json::Value {
        let golden_path = temp_path(&format!("{tag}-golden.json"));
        std::fs::write(&golden_path, golden_json).expect("write golden fixture");
        let out_dir = temp_path(&format!("{tag}-out"));
        let args = [
            "rag-eval",
            "--fake",
            "--golden",
            &golden_path.to_string_lossy(),
            "--out",
            &out_dir.to_string_lossy(),
            "--tag",
            tag,
        ]
        .map(str::to_string);
        assert_eq!(run(args.into_iter()).await, EXIT_OK);
        let results = read_results(&out_dir, tag);
        std::fs::remove_file(&golden_path).ok();
        std::fs::remove_dir_all(&out_dir).ok();
        results
    }

    fn with_extra_case(golden: &str, case: &str) -> String {
        golden.replacen(r#""retrieval_cases": ["#, &format!(r#""retrieval_cases": [{case},"#), 1)
    }

    #[tokio::test]
    async fn a_golden_without_the_new_classes_measures_exactly_as_before() {
        let json = run_fake(sweep_golden_json(), "plain").await;
        let retrieval = &json["retrieval"];
        for key in ["unanswerable", "abstention", "freshness"] {
            assert!(retrieval.get(key).is_none(), "{key} must be absent: {retrieval}");
        }
        assert_eq!(retrieval["per_case"].as_array().unwrap().len(), 2);
        assert_eq!(retrieval["aggregate"]["recall@1"], 1.0);
        assert_eq!(retrieval["hits"]["at_1"]["strict"]["hits"], 2);
        assert!(retrieval["per_case"][0].get("abstained").is_none());
    }

    #[tokio::test]
    async fn an_unanswerable_case_stays_out_of_the_ranked_metrics_and_reports_abstention() {
        let plain = run_fake(sweep_golden_json(), "without-unanswerable").await;
        let golden = with_extra_case(
            sweep_golden_json(),
            r#"{"query": "a question nobody ever saved an answer to", "corpus_refs": ["c1"], "expected_refs": [], "subset": "unanswerable"}"#,
        );
        let json = run_fake(&golden, "with-unanswerable").await;

        let (before, after) = (&plain["retrieval"], &json["retrieval"]);
        assert_eq!(before["per_case"], after["per_case"]);
        assert_eq!(before["aggregate"], after["aggregate"]);
        let unanswerable = after["unanswerable"].as_array().expect("its own list");
        assert_eq!(unanswerable.len(), 1);
        assert_eq!(unanswerable[0]["abstained"], false, "the search returns rows for any query today");
        assert_eq!(after["abstention"]["hits"], 0);
        assert_eq!(after["abstention"]["n_cases"], 1);
    }

    #[tokio::test]
    async fn an_update_case_is_scored_and_reports_freshness() {
        let golden = with_extra_case(
            sweep_golden_json(),
            r#"{"query": "the retrieval decay term compares milliseconds against seconds", "corpus_refs": ["c1","c3"], "expected_refs": ["c1"], "stale_refs": ["c3"], "subset": "update"}"#,
        )
        .replacen(r#""importance": 3.0}"#, r#""importance": 3.0, "age_days": 10}"#, 1);
        let json = run_fake(&golden, "update").await;

        let retrieval = &json["retrieval"];
        assert_eq!(retrieval["per_case"].as_array().unwrap().len(), 3, "an update case stays in the ranked metrics");
        assert_eq!(retrieval["per_case"][0]["fresh_first"], true, "its query is the newer row's own text");
        assert_eq!(retrieval["freshness"]["hits"], 1);
        assert_eq!(retrieval["freshness"]["n_cases"], 1);
    }

    #[tokio::test]
    async fn a_lexical_only_run_writes_a_tagged_file_that_diffs() {
        let golden_path = temp_path("lexical-only-golden.json");
        std::fs::write(&golden_path, lexical_wiring_golden_json()).expect("write golden fixture");
        let out_dir = temp_path("lexical-only-out");
        let args = |extra: &[&str]| {
            ["rag-eval", "--fake", "--golden", &golden_path.to_string_lossy(), "--out", &out_dir.to_string_lossy()]
                .iter()
                .map(|s| s.to_string())
                .chain(extra.iter().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(args(&["--lexical-only"]).into_iter()).await, EXIT_OK);
        assert_eq!(run(args(&["--tag", "full"]).into_iter()).await, EXIT_OK);

        let lexical = read_results(&out_dir, LEXICAL_ONLY_SLUG);
        assert_eq!(lexical["tag"], LEXICAL_ONLY_SLUG);
        assert_eq!(lexical["k_pool"]["memory"], 0);
        let ranked: Vec<&str> = lexical["retrieval"]["per_case"][0]["ranked_refs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(ranked.len(), 5, "only the five rows carrying the token match: {ranked:?}");
        assert!(ranked.iter().all(|r| ["c1", "c2", "c3", "c4", "c5"].contains(r)), "{ranked:?}");

        let full_path = out_dir.join(format!("{}-fake-full.json", current_date_string()));
        let lexical_path = out_dir.join(format!("{}-fake-{LEXICAL_ONLY_SLUG}.json", current_date_string()));
        diff::diff_files(&lexical_path, &full_path).expect("the two arms compare");

        std::fs::remove_file(&golden_path).ok();
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[tokio::test]
    async fn diff_exits_six_on_a_significant_regression_only_with_the_flag() {
        let case = |i: usize, hit: f64| {
            serde_json::json!({
                "query": format!("q{i}"), "expected_refs": ["c1"], "ranked_refs": ["c1"],
                "recall@1": hit, "recall@5": hit, "recall@10": hit, "mrr": hit, "ndcg@10": hit,
                "status": "active", "lang": null
            })
        };
        let file = |hit: f64| serde_json::json!({ "retrieval": { "per_case": (0..6).map(|i| case(i, hit)).collect::<Vec<_>>() } });
        let dir = tempfile::tempdir().expect("temp dir");
        let (a, b) = (dir.path().join("a.json"), dir.path().join("b.json"));
        std::fs::write(&a, file(1.0).to_string()).expect("write a");
        std::fs::write(&b, file(0.0).to_string()).expect("write b");
        let (a, b) = (a.to_string_lossy().into_owned(), b.to_string_lossy().into_owned());
        let argv = |extra: &[&str]| {
            ["rag-eval", "--diff", a.as_str(), b.as_str()].iter().chain(extra).map(|s| s.to_string()).collect::<Vec<_>>()
        };

        assert_eq!(run(argv(&[]).into_iter()).await, EXIT_OK);
        assert_eq!(run(argv(&["--fail-on-regression"]).into_iter()).await, EXIT_REGRESSION);
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

        // FakeEmbedder has no learned geometry: a cosine measurement over it
        // would describe nothing, so the similarity block must be gated
        // strictly on embedder kind and stay entirely absent under --fake —
        // not present-and-empty, which would read as "measured, found
        // nothing" rather than "not applicable to this run".
        assert!(
            json.get("similarity").is_none(),
            "similarity key must be absent under --fake, got {:?}",
            json.get("similarity")
        );

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

    #[test]
    fn sim_and_ranked_paths_score_identical_input_through_the_same_metric_functions() {
        // The similarity block's whole claim to being a like-for-like
        // comparison rests on feeding `metrics.rs` through the SAME function
        // calls the ranked path already uses — never a second
        // implementation. Labeling one call "ranked" and the other "sim" over
        // identical input must produce numerically identical output, because
        // underneath they are the same function.
        let ranked_refs = vec!["c2".to_string(), "c1".to_string(), "c3".to_string()];
        let sim_ranked_refs = ranked_refs.clone();
        let expected: HashSet<String> = ["c1".to_string()].into_iter().collect();

        let ranked_recall_10 = metrics::recall_at_k(&ranked_refs, &expected, 10);
        let sim_recall_10 = metrics::recall_at_k(&sim_ranked_refs, &expected, 10);
        assert_eq!(ranked_recall_10, sim_recall_10);

        let ranked_mrr = metrics::mrr_contribution(&ranked_refs, &expected);
        let sim_mrr = metrics::mrr_contribution(&sim_ranked_refs, &expected);
        assert_eq!(ranked_mrr, sim_mrr);

        let ranked_ndcg = metrics::ndcg_at_10(&ranked_refs, &expected);
        let sim_ndcg = metrics::ndcg_at_10(&sim_ranked_refs, &expected);
        assert_eq!(ranked_ndcg, sim_ndcg);
    }

    #[tokio::test]
    async fn the_query_language_reaches_the_retrieval_and_similarity_cases() {
        let golden = sweep_golden_json().replacen(
            r#""expected_refs": ["c1"]}"#,
            r#""expected_refs": ["c1"], "query_lang": "es"}"#,
            1,
        );
        let json = run_fake(&golden, "query-lang").await;
        let per_case = &json["retrieval"]["per_case"];
        assert_eq!(per_case[0]["query_lang"], "es");
        assert_eq!(per_case[0]["lang"], "en");
        assert!(per_case[1].get("query_lang").is_none(), "an unlabelled case writes no key: {}", per_case[1]);

        let source = SimCaseSource {
            query: "q".to_string(),
            expected_refs: vec!["c1".to_string()],
            corpus_refs: vec!["c1".to_string()],
            query_vector: vec![1.0, 0.0],
            query_lang: Some("es".to_string()),
            lang: Some("en".to_string()),
            subset: CaseSubset::Natural,
        };
        let sim = build_similarity_results(&[source], &[("c1".to_string(), vec![1.0, 0.0])], K_FINAL, "", "");
        assert_eq!(sim.per_case[0].query_lang.as_deref(), Some("es"));
        assert_eq!(sim.per_case[0].lang.as_deref(), Some("en"));
    }

    #[test]
    fn build_similarity_results_reports_the_live_embedders_own_prefixes() {
        let result = build_similarity_results(&[], &[], K_FINAL, "query: ", "passage: ");
        assert_eq!(result.query_prefix, "query: ");
        assert_eq!(result.passage_prefix, "passage: ");
    }

    fn sweep_golden_json() -> &'static str {
        r#"{
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
                {"a": "a first statement", "b": "a second, unrelated statement", "label": "no_contradiction", "lang": "en"}
            ]
        }"#
    }

    /// A golden set big enough that the memory pool (30) cannot hold it, with
    /// the query's rare token confined to the five lowest-id rows.
    ///
    /// Both halves matter. The corpus has to outsize the pool, or the dense
    /// arm returns every row and there is nothing left for a second channel
    /// to add. And the token has to be rare, or every row is a lexical
    /// candidate and which ones get promoted stops being a property of the
    /// fixture. Under the fake embedder every hit's similarity term clamps to
    /// zero, so the final ordering is decided entirely by the tie-break on
    /// id — which is what makes "a promoted row shows up at the front"
    /// something this fixture guarantees rather than hopes for.
    fn lexical_wiring_golden_json() -> String {
        let mut corpus = Vec::new();
        for i in 1..=5 {
            corpus.push(format!(
                r#"{{"ref": "c{i}", "content": "rollout checklist ZQXIDENT42 note number {i}", "memory_type": "chat_message", "importance": 3.0}}"#
            ));
        }
        for i in 6..=200 {
            corpus.push(format!(
                r#"{{"ref": "c{i}", "content": "unrelated background material entry number {i} with no shared identifier", "memory_type": "chat_message", "importance": 3.0}}"#
            ));
        }
        format!(
            r#"{{
                "corpus": [{}],
                "retrieval_cases": [
                    {{"query": "ZQXIDENT42", "corpus_refs": ["c1"], "expected_refs": ["c1"]}}
                ],
                "contradiction_pairs": [
                    {{"a": "a first statement", "b": "a second, unrelated statement", "label": "no_contradiction", "lang": "en"}}
                ]
            }}"#,
            corpus.join(",\n")
        )
    }

    fn read_results(out_dir: &std::path::Path, tag: &str) -> serde_json::Value {
        let filename = format!("{}-fake-{tag}.json", current_date_string());
        let raw = std::fs::read_to_string(out_dir.join(&filename))
            .unwrap_or_else(|e| panic!("{filename} must exist: {e}"));
        serde_json::from_str(&raw).unwrap()
    }

    /// `--sweep-lexical` is only a measurement if every link between the flag
    /// and the search actually carries it. Each link is a one-line
    /// pass-through — the grid point into the scope, and the case's query text
    /// into `EvalStore::search` — and severing any one of them leaves a fully
    /// green suite while making both sweep points report identical numbers.
    /// An instrument that reads the same for both settings of the thing it
    /// measures is worse than no instrument, because its output looks like
    /// evidence.
    ///
    /// Asserted on the written artifacts rather than on internals, because
    /// the artifacts are what a later reader draws the conclusion from.
    #[tokio::test]
    async fn the_two_sweep_lexical_points_search_differently_and_say_so_in_their_results() {
        let golden_path = temp_path("lexical-wiring-golden.json");
        std::fs::write(&golden_path, lexical_wiring_golden_json()).expect("write golden fixture");
        let out_dir = temp_path("lexical-wiring-out");

        let args = vec![
            "rag-eval".to_string(),
            "--fake".to_string(),
            "--golden".to_string(),
            golden_path.to_string_lossy().to_string(),
            "--out".to_string(),
            out_dir.to_string_lossy().to_string(),
            "--sweep-lexical".to_string(),
            "0".to_string(),
            "--sweep-lexical".to_string(),
            "20".to_string(),
        ];
        let code = run(args.into_iter()).await;
        assert_eq!(code, EXIT_OK);

        let off = read_results(&out_dir, "lexoff");
        let on = read_results(&out_dir, "lex20");

        // The channel config each file was produced under. Without this the
        // two files are two numbers with no record of what separates them.
        assert_eq!(off["k_pool"]["lexical"], serde_json::Value::Null);
        assert_eq!(on["k_pool"]["lexical"], serde_json::json!(20));

        let off_ranked = off["retrieval"]["per_case"][0]["ranked_refs"].clone();
        let on_ranked = on["retrieval"]["per_case"][0]["ranked_refs"].clone();
        assert_ne!(
            off_ranked, on_ranked,
            "the lexical channel promoted nothing the dense arm had not already found, so this \
             run measured the same thing twice: {off_ranked:?}"
        );

        // Named concretely rather than left at "they differ": the rows the
        // channel is supposed to promote are the ones carrying the query's
        // token, and at least one of them must be a row only the lexical
        // point returned.
        let on_refs: HashSet<String> =
            on_ranked.as_array().unwrap().iter().map(|r| r.as_str().unwrap().to_string()).collect();
        let off_refs: HashSet<String> =
            off_ranked.as_array().unwrap().iter().map(|r| r.as_str().unwrap().to_string()).collect();
        let promoted: Vec<&String> = on_refs
            .difference(&off_refs)
            .filter(|r| ["c1", "c2", "c3", "c4", "c5"].contains(&r.as_str()))
            .collect();
        assert!(
            !promoted.is_empty(),
            "expected at least one token-carrying row to reach the ranked list only with the \
             lexical channel on; on={on_refs:?} off={off_refs:?}"
        );

        std::fs::remove_file(&golden_path).ok();
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[tokio::test]
    async fn zero_sweep_flags_writes_exactly_one_file_matching_todays_filename() {
        // The regression guard on the sweep plumbing itself: with no sweep
        // flags, the grid must degenerate to exactly one point built from
        // EvalScoringConfig::default() — not a spanning-derived point that
        // merely targets the same numbers — so the single results file this
        // run writes is byte-for-byte what a pre-sweep run would have written.
        let golden_path = temp_path("sweep-golden-zero.json");
        std::fs::write(&golden_path, sweep_golden_json()).expect("write golden fixture");
        let out_dir = temp_path("sweep-out-zero");

        let args = vec![
            "rag-eval".to_string(),
            "--fake".to_string(),
            "--golden".to_string(),
            golden_path.to_string_lossy().to_string(),
            "--out".to_string(),
            out_dir.to_string_lossy().to_string(),
        ];
        let code = run(args.into_iter()).await;
        assert_eq!(code, EXIT_OK);

        let entries: Vec<_> = std::fs::read_dir(&out_dir).expect("out dir must exist").collect();
        assert_eq!(entries.len(), 1, "zero sweep flags must write exactly one file, got {entries:?}");
        let expected_filename = format!("{}-fake.json", current_date_string());
        assert!(out_dir.join(&expected_filename).exists());

        let written = std::fs::read_to_string(out_dir.join(&expected_filename)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert!(json.get("scoring_config").is_some());
        // Derived from EvalScoringConfig::default() rather than a bare
        // literal: this test's subject is "the zero-flags point carries the
        // default's own floor", not "the floor happens to be this specific
        // number" — it must keep testing the former even if the latter
        // changes again.
        let expected_floor = match EvalScoringConfig::default().cosine_floor {
            Some(f) => serde_json::json!(f),
            None => serde_json::Value::Null,
        };
        assert_eq!(json["scoring_config"]["cosine_floor"], expected_floor);

        std::fs::remove_file(&golden_path).ok();
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[tokio::test]
    async fn sweep_flags_write_n_point_files_plus_a_sweep_index() {
        let golden_path = temp_path("sweep-golden.json");
        std::fs::write(&golden_path, sweep_golden_json()).expect("write golden fixture");
        let out_dir = temp_path("sweep-out");

        let args = vec![
            "rag-eval".to_string(),
            "--fake".to_string(),
            "--golden".to_string(),
            golden_path.to_string_lossy().to_string(),
            "--out".to_string(),
            out_dir.to_string_lossy().to_string(),
            "--sweep-imp".to_string(),
            "0.55:1.0".to_string(),
            "--sweep-imp".to_string(),
            "0.7:1.0".to_string(),
            "--sweep-cos-floor".to_string(),
            "0.86".to_string(),
        ];
        let code = run(args.into_iter()).await;
        assert_eq!(code, EXIT_OK);

        // 2 imp points x 1 cos-floor point = 2 result files + 1 index file.
        let entries: Vec<String> = std::fs::read_dir(&out_dir)
            .expect("out dir must exist")
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries.len(), 3, "expected 2 point files + 1 sweep index, got {entries:?}");

        let index_file = entries.iter().find(|f| f.ends_with("-sweep.json"));
        assert!(index_file.is_some(), "a *-sweep.json index must be written, got {entries:?}");
        let index_json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(out_dir.join(index_file.unwrap())).unwrap(),
        )
        .unwrap();
        let points = index_json["points"].as_array().expect("points must be an array");
        assert_eq!(points.len(), 2);
        for p in points {
            assert!(p.get("slug").is_some());
            assert!(p.get("results_file").is_some());
            assert!(p.get("scoring_config").is_some());
            assert!(p.get("aggregate").is_some());
            assert!(p.get("importance_rank_dominance").is_some());
            assert!(p.get("modal_rank_agreement").is_some());
            // Every point file the index references must actually exist.
            let filename = p["results_file"].as_str().unwrap();
            assert!(out_dir.join(filename).exists(), "referenced file {filename} must exist");
        }
        // Sorted ascending by imp_min: 0.55 before 0.70.
        let imp_mins: Vec<f64> = points
            .iter()
            .map(|p| {
                let cfg = &p["scoring_config"];
                cfg["importance_base"].as_f64().unwrap()
                    + cfg["importance_clamp_lo"].as_f64().unwrap() / cfg["importance_divisor"].as_f64().unwrap()
            })
            .collect();
        assert!(imp_mins[0] < imp_mins[1], "points must be sorted ascending by imp_min: {imp_mins:?}");

        std::fs::remove_file(&golden_path).ok();
        std::fs::remove_dir_all(&out_dir).ok();
    }

    /// The harness must ask the store for the same lexical configuration the
    /// application does. Derived from the production default rather than
    /// restated as a literal, so the two cannot be edited apart: a run whose
    /// channel config differs from the shipped one produces numbers that are
    /// about a system nobody uses, and nothing about the output would say so.
    #[test]
    fn k_pool_mirrors_the_production_lexical_budget() {
        assert_eq!(K_POOL.lexical, production_lexical_budget());
    }

    #[test]
    fn tokenizer_slug_is_absent_without_the_flag_and_filename_safe_with_it() {
        // Absent by default is what keeps a plain run's filename identical to
        // the one every committed results file was referenced under.
        assert_eq!(tokenizer_slug(None), None);
        assert_eq!(tokenizer_slug(Some("trigram")), Some("tok-trigram".to_string()));
        assert_eq!(
            tokenizer_slug(Some(MIGRATION_FTS_TOKENIZER)),
            Some("tok-unicode61-remove-diacritics-2".to_string()),
            "an allowed tokenizer name carries spaces; a filename must not"
        );
    }

    /// The tokenizer comparison is two runs whose ONLY difference is the
    /// tokenizer, so the two things it needs are that the runs land in
    /// separate files and that each file names its own tokenizer. Without the
    /// first the second run cannot even be written; without the second the
    /// two files are a pair of numbers whose provenance lives only in
    /// whatever terminal produced them.
    #[tokio::test]
    async fn two_runs_differing_only_in_tokenizer_write_separate_self_describing_files() {
        let golden_path = temp_path("tokenizer-provenance-golden.json");
        std::fs::write(&golden_path, sweep_golden_json()).expect("write golden fixture");
        let out_dir = temp_path("tokenizer-provenance-out");

        let base_args = |extra: Vec<&str>| {
            let mut args = vec![
                "rag-eval".to_string(),
                "--fake".to_string(),
                "--golden".to_string(),
                golden_path.to_string_lossy().to_string(),
                "--out".to_string(),
                out_dir.to_string_lossy().to_string(),
            ];
            args.extend(extra.into_iter().map(str::to_string));
            args
        };

        assert_eq!(run(base_args(vec![]).into_iter()).await, EXIT_OK);
        assert_eq!(
            run(base_args(vec!["--fts-tokenizer", "trigram"]).into_iter()).await,
            EXIT_OK,
            "the second run must not collide with the first — a collision is reported as \
             'results file already exists' and leaves the comparison with one half of its data"
        );

        let date = current_date_string();
        let default_run: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(out_dir.join(format!("{date}-fake.json"))).unwrap(),
        )
        .unwrap();
        let trigram_run: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(out_dir.join(format!("{date}-fake-tok-trigram.json"))).unwrap(),
        )
        .unwrap();

        assert_eq!(default_run["fts_tokenizer"], serde_json::json!(MIGRATION_FTS_TOKENIZER));
        assert_eq!(trigram_run["fts_tokenizer"], serde_json::json!("trigram"));

        // Both halves of a tokenizer comparison are meaningless unless the
        // lexical channel was actually queried — a tokenizer only changes how
        // `memory_fts` is indexed, and an index nothing reads produces the
        // same numbers whatever is in it.
        assert_eq!(default_run["k_pool"]["lexical"], serde_json::json!(20));
        assert_eq!(trigram_run["k_pool"]["lexical"], serde_json::json!(20));

        std::fs::remove_file(&golden_path).ok();
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[tokio::test]
    async fn fts_tokenizer_flag_reaches_the_store_and_a_run_still_succeeds() {
        // End-to-end proof that `cli.fts_tokenizer` actually reaches
        // `seed::seed`'s third argument, not just that `cli.rs` parses it:
        // a wrong or dropped pass-through here would either error out or
        // silently run against the migration's own tokenizer instead.
        let golden_path = temp_path("fts-tokenizer-golden.json");
        std::fs::write(&golden_path, sweep_golden_json()).expect("write golden fixture");
        let out_dir = temp_path("fts-tokenizer-out");

        let args = vec![
            "rag-eval".to_string(),
            "--fake".to_string(),
            "--golden".to_string(),
            golden_path.to_string_lossy().to_string(),
            "--out".to_string(),
            out_dir.to_string_lossy().to_string(),
            "--fts-tokenizer".to_string(),
            "trigram".to_string(),
        ];
        let code = run(args.into_iter()).await;
        assert_eq!(code, EXIT_OK, "a run under an allowed tokenizer must still succeed end to end");

        std::fs::remove_file(&golden_path).ok();
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[tokio::test]
    async fn unlisted_fts_tokenizer_flag_fails_the_run_before_writing_anything() {
        let golden_path = temp_path("fts-tokenizer-bad-golden.json");
        std::fs::write(&golden_path, sweep_golden_json()).expect("write golden fixture");
        let out_dir = temp_path("fts-tokenizer-bad-out");

        let args = vec![
            "rag-eval".to_string(),
            "--fake".to_string(),
            "--golden".to_string(),
            golden_path.to_string_lossy().to_string(),
            "--out".to_string(),
            out_dir.to_string_lossy().to_string(),
            "--fts-tokenizer".to_string(),
            "porter".to_string(),
        ];
        let code = run(args.into_iter()).await;
        assert_eq!(code, EXIT_RUNTIME_FAILURE, "an unlisted tokenizer must fail the run, not silently ignore the flag");
        assert!(!out_dir.exists(), "a rejected tokenizer must abort before any results file is written");

        std::fs::remove_file(&golden_path).ok();
    }

    // --- --follow-up ---------------------------------------------------------

    /// A golden set built so the resolvability gate passes with a wide
    /// margin under `--fake`: 10 target rows, each an EXACT self-match for
    /// its own query (guarantees ceiling recall@10 == 1.0, the same
    /// deterministic-distance-0 property `fake_end_to_end_run_produces_a_
    /// valid_results_json` relies on), padded out to 100 total rows with
    /// unrelated filler so the floor arm's content-free template — whose
    /// fake vector shares no structure with any target row — has only
    /// roughly a 10-in-100 chance per case of landing a target in the
    /// top 10, pulling floor recall@10 well below the `5.0/n` bar.
    fn follow_up_golden_json() -> String {
        let mut corpus = Vec::new();
        let mut cases = Vec::new();
        for i in 1..=10 {
            let content = format!("target content about topic number {i} with distinct wording {i}");
            corpus.push(format!(
                r#"{{"ref": "t{i}", "content": "{content}", "memory_type": "chat_message", "importance": 3.0}}"#
            ));
            cases.push(format!(
                r#"{{"query": "{content}", "corpus_refs": ["t{i}"], "expected_refs": ["t{i}"]}}"#
            ));
        }
        for i in 1..=90 {
            corpus.push(format!(
                r#"{{"ref": "f{i}", "content": "unrelated filler background entry number {i}", "memory_type": "chat_message", "importance": 3.0}}"#
            ));
        }
        format!(
            r#"{{
                "corpus": [{}],
                "retrieval_cases": [{}],
                "contradiction_pairs": []
            }}"#,
            corpus.join(",\n"),
            cases.join(",\n")
        )
    }

    fn read_follow_up_results(out_dir: &std::path::Path, arm: &str) -> serde_json::Value {
        let filename = format!("{}-fake-followup-{arm}.json", current_date_string());
        let raw = std::fs::read_to_string(out_dir.join(&filename))
            .unwrap_or_else(|e| panic!("{filename} must exist: {e}"));
        serde_json::from_str(&raw).unwrap()
    }

    #[tokio::test]
    async fn fake_follow_up_writes_three_tagged_files_with_differing_numbers() {
        let golden_path = temp_path("follow-up-golden.json");
        std::fs::write(&golden_path, follow_up_golden_json()).expect("write golden fixture");
        let out_dir = temp_path("follow-up-out");

        let args = vec![
            "rag-eval".to_string(),
            "--fake".to_string(),
            "--golden".to_string(),
            golden_path.to_string_lossy().to_string(),
            "--out".to_string(),
            out_dir.to_string_lossy().to_string(),
            "--follow-up".to_string(),
        ];
        let code = run(args.into_iter()).await;
        assert_eq!(code, EXIT_OK, "a resolvable --follow-up run must exit 0");

        let ceiling = read_follow_up_results(&out_dir, "ceiling");
        let floor = read_follow_up_results(&out_dir, "floor");
        let treatment = read_follow_up_results(&out_dir, "treatment");

        let ceiling_r10 = ceiling["retrieval"]["aggregate"]["recall@10"].as_f64().unwrap();
        let floor_r10 = floor["retrieval"]["aggregate"]["recall@10"].as_f64().unwrap();
        assert_eq!(ceiling_r10, 1.0, "every case's ceiling query is an exact self-match");
        assert!(
            ceiling_r10 - floor_r10 > 5.0 / 10.0,
            "fixture must clear the resolvability gate: ceiling={ceiling_r10} floor={floor_r10}"
        );

        assert_ne!(
            ceiling["retrieval"]["per_case"], floor["retrieval"]["per_case"],
            "ceiling and floor must score differently"
        );
        assert_ne!(
            floor["retrieval"]["per_case"], treatment["retrieval"]["per_case"],
            "floor and treatment must score differently — the treatment arm has the query folded in"
        );

        // The treatment arm's LEXICAL channel gets the content-free template,
        // never the composed text — handing the golden query
        // to BM25 in this arm is forbidden, because a lexical channel that can see the
        // query stops describing what production does with a real follow-up.
        //
        // The ceiling is exactly the arm whose lexical channel DOES hold the
        // golden query, and every target row here is an exact self-match for
        // it, so the ceiling scores 1.0. The treatment arm cannot reach that
        // without the same lexical advantage: its dense text is
        // `template + query`, which the fake embedder hashes to something
        // unrelated to any row. Measured here: treatment 0.5 against a
        // ceiling of 1.0; routing `arm.embedded` to the lexical channel
        // instead lifts treatment to exactly 1.0, which is what this
        // comparison catches.
        let treatment_r10 = treatment["retrieval"]["aggregate"]["recall@10"].as_f64().unwrap();
        assert!(
            treatment_r10 < ceiling_r10,
            "the treatment arm tied the ceiling (treatment={treatment_r10} ceiling={ceiling_r10}) — \
             it can only do that if the composed text reached the lexical/FTS5 channel, letting \
             BM25 score against the golden query the arm is supposed to be blind to"
        );

        std::fs::remove_file(&golden_path).ok();
        std::fs::remove_dir_all(&out_dir).ok();
    }

    /// A golden set the gate must VOID: the corpus is exactly `K_FINAL` rows,
    /// so every query — the golden one and the content-free template alike —
    /// returns the whole corpus in its top 10 and both arms score recall@10
    /// = 1.0. The gap is 0, which resolves nothing, and that is precisely
    /// what the gate exists to refuse to report on.
    fn void_follow_up_golden_json() -> String {
        let mut corpus = Vec::new();
        let mut cases = Vec::new();
        for i in 1..=10 {
            let content = format!("target content about topic number {i} with distinct wording {i}");
            corpus.push(format!(
                r#"{{"ref": "t{i}", "content": "{content}", "memory_type": "chat_message", "importance": 3.0}}"#
            ));
            cases.push(format!(
                r#"{{"query": "{content}", "corpus_refs": ["t{i}"], "expected_refs": ["t{i}"]}}"#
            ));
        }
        format!(
            r#"{{
                "corpus": [{}],
                "retrieval_cases": [{}],
                "contradiction_pairs": []
            }}"#,
            corpus.join(",\n"),
            cases.join(",\n")
        )
    }

    /// A void run is a hard error, and the verdict is decided BEFORE the
    /// treatment arm is scored — so the run exits 5 with no treatment number
    /// to report weakly, while both arms that WERE computed still leave their
    /// results files behind (the evidence for discounting a run is itself an
    /// artifact).
    ///
    /// The missing treatment file is the load-bearing assertion: it is the
    /// only thing that can tell "gate before treatment" apart from "gate
    /// after treatment", since the exit code is 5 either way.
    #[tokio::test]
    async fn a_void_follow_up_run_exits_golden_unfit_without_scoring_the_treatment_arm() {
        let golden_path = temp_path("follow-up-void-golden.json");
        std::fs::write(&golden_path, void_follow_up_golden_json()).expect("write golden fixture");
        let out_dir = temp_path("follow-up-void-out");

        let args = vec![
            "rag-eval".to_string(),
            "--fake".to_string(),
            "--golden".to_string(),
            golden_path.to_string_lossy().to_string(),
            "--out".to_string(),
            out_dir.to_string_lossy().to_string(),
            "--follow-up".to_string(),
        ];
        let code = run(args.into_iter()).await;
        assert_eq!(
            code, EXIT_GOLDEN_UNFIT,
            "a gap the golden set cannot resolve must be a hard error, not a weak result"
        );

        let date = current_date_string();
        assert!(
            out_dir.join(format!("{date}-fake-followup-ceiling.json")).exists(),
            "the ceiling arm was computed, so its results file must still be written"
        );
        assert!(
            out_dir.join(format!("{date}-fake-followup-floor.json")).exists(),
            "the floor arm was computed, so its results file must still be written"
        );
        assert!(
            !out_dir.join(format!("{date}-fake-followup-treatment.json")).exists(),
            "the treatment arm must never be scored on a void run — a treatment file here means \
             the gate was evaluated AFTER it instead of before"
        );

        std::fs::remove_file(&golden_path).ok();
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[tokio::test]
    async fn follow_up_with_sweep_lexical_exits_runtime_failure_before_any_run() {
        let golden_path = temp_path("follow-up-sweep-golden.json");
        std::fs::write(&golden_path, sweep_golden_json()).expect("write golden fixture");
        let out_dir = temp_path("follow-up-sweep-out");

        let args = vec![
            "rag-eval".to_string(),
            "--fake".to_string(),
            "--golden".to_string(),
            golden_path.to_string_lossy().to_string(),
            "--out".to_string(),
            out_dir.to_string_lossy().to_string(),
            "--follow-up".to_string(),
            "--sweep-lexical".to_string(),
            "20".to_string(),
        ];
        let code = run(args.into_iter()).await;
        assert_eq!(
            code, EXIT_RUNTIME_FAILURE,
            "--follow-up + --sweep-lexical must be rejected at the CLI layer before any model, \
             seeding, or scoring work happens"
        );
        assert!(
            !out_dir.exists(),
            "the CLI conflict must be caught before the out dir is even created, let alone a file written into it"
        );

        std::fs::remove_file(&golden_path).ok();
    }

    const AGENT_VIEW_ROW_A: &str = "the retrieval decay term compares milliseconds against seconds";
    const AGENT_VIEW_ROW_B: &str = "el indice knn mide distancia l2 cruda y no similitud coseno";

    fn agent_view_golden_json() -> String {
        format!(
            r#"{{
                "corpus": [
                    {{"ref": "c1", "content": "{AGENT_VIEW_ROW_A}", "memory_type": "chat_message", "importance": 3.0}},
                    {{"ref": "c2", "content": "{AGENT_VIEW_ROW_B}", "memory_type": "chat_message", "importance": 3.0}}
                ],
                "retrieval_cases": [
                    {{"query": "{AGENT_VIEW_ROW_A}", "corpus_refs": ["c1","c2"], "expected_refs": ["c1"]}},
                    {{"query": "{AGENT_VIEW_ROW_B}", "corpus_refs": ["c1","c2"], "expected_refs": ["c2"]}}
                ],
                "contradiction_pairs": []
            }}"#
        )
    }

    fn agent_view_args(golden: &std::path::Path, out_dir: &std::path::Path, extra: &[&str]) -> Vec<String> {
        ["rag-eval", "--fake", "--agent-view", "--golden"]
            .into_iter()
            .map(String::from)
            .chain([golden.to_string_lossy().to_string(), "--out".to_string(), out_dir.to_string_lossy().to_string()])
            .chain(extra.iter().map(|s| s.to_string()))
            .collect()
    }

    #[tokio::test]
    async fn agent_view_run_writes_its_own_results_file_carrying_the_block() {
        let golden_path = temp_path("agent-view-golden.json");
        std::fs::write(&golden_path, agent_view_golden_json()).expect("write golden fixture");
        let out_dir = temp_path("agent-view-out");

        let code = run(agent_view_args(&golden_path, &out_dir, &[]).into_iter()).await;
        assert_eq!(code, EXIT_OK);

        let date = current_date_string();
        assert!(
            !out_dir.join(format!("{date}-fake.json")).exists(),
            "an agent-view run must not pose as the ordinary retrieval baseline"
        );
        let path = out_dir.join(format!("{date}-fake-agent-view.json"));
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("results file")).expect("valid JSON");
        assert_eq!(json["tag"], "agent-view");
        assert_eq!(json["agent_view"]["corpus_size"], 2);
        assert_eq!(json["agent_view"]["stored_rows"], 2);
        assert_eq!(json["agent_view"]["per_case"].as_array().map(Vec::len), Some(2));
        assert_eq!(json["agent_view"]["search"]["active"]["hit_rate"], 1.0);
        assert_eq!(json["agent_view"]["context"]["active"]["hit_rate"], 1.0);
        assert_eq!(
            json["retrieval"]["per_case"].as_array().map(Vec::len),
            Some(0),
            "the store's own ranking is not scored in this mode"
        );

        std::fs::remove_file(&golden_path).ok();
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[tokio::test]
    async fn agent_view_run_refuses_to_overwrite_its_results_file_without_force() {
        let golden_path = temp_path("agent-view-again-golden.json");
        std::fs::write(&golden_path, agent_view_golden_json()).expect("write golden fixture");
        let out_dir = temp_path("agent-view-again-out");

        assert_eq!(run(agent_view_args(&golden_path, &out_dir, &[]).into_iter()).await, EXIT_OK);
        assert_eq!(run(agent_view_args(&golden_path, &out_dir, &[]).into_iter()).await, EXIT_RESULTS_EXISTS);
        assert_eq!(run(agent_view_args(&golden_path, &out_dir, &["--force"]).into_iter()).await, EXIT_OK);

        std::fs::remove_file(&golden_path).ok();
        std::fs::remove_dir_all(&out_dir).ok();
    }

    #[tokio::test]
    async fn help_flag_exits_zero_without_touching_anything() {
        let args = vec!["rag-eval".to_string(), "--help".to_string()];
        let code = run(args.into_iter()).await;
        assert_eq!(code, EXIT_OK);
    }

    #[test]
    fn zero_sweep_lexical_flags_leaves_the_lexical_axis_untouched() {
        // Mirrors the imp/cos-floor regression guard: an empty third axis
        // must degenerate to exactly one `None` cell, not be dropped from
        // the product — that is what lets "leave K_POOL's value" hold and
        // what keeps a plain run's grid at exactly one point.
        let grid = build_grid(&[], &[], &[]);
        assert_eq!(grid.len(), 1);
        assert_eq!(grid[0].lexical, None);
    }

    #[test]
    fn sweep_lexical_flags_populate_the_third_axis() {
        let grid = build_grid(&[], &[], &[0, 20]);
        assert_eq!(grid.len(), 2);
        assert_eq!(grid.iter().map(|p| p.lexical).collect::<Vec<_>>(), vec![Some(0), Some(20)]);
    }

    #[test]
    fn all_three_axes_cross_multiply() {
        let grid = build_grid(&[(0.55, 1.0), (0.7, 1.0)], &[0.86], &[0, 20]);
        assert_eq!(grid.len(), 4, "2 imp x 1 cos x 2 lexical = 4 points");
    }

    #[test]
    fn point_slug_renders_lexoff_for_zero_and_lex_n_for_nonzero() {
        let grid = build_grid(&[], &[], &[0, 20]);
        let slugs: Vec<Option<String>> = grid.iter().map(point_slug).collect();
        assert_eq!(slugs, vec![Some("lexoff".to_string()), Some("lex20".to_string())]);
    }

    #[test]
    fn point_slug_is_none_when_no_axis_was_swept() {
        let grid = build_grid(&[], &[], &[]);
        assert_eq!(point_slug(&grid[0]), None);
    }

    #[test]
    fn point_slug_combines_all_three_fragments_in_order() {
        let grid = build_grid(&[(0.55, 1.0)], &[0.86], &[20]);
        assert_eq!(point_slug(&grid[0]), Some("imp0.55-1.00_cos0.8600_lex20".to_string()));
    }

    #[test]
    fn resolve_lexical_scope_untouched_axis_falls_back_to_the_default() {
        assert_eq!(resolve_lexical_scope(None, Some(20)), Some(20));
        assert_eq!(resolve_lexical_scope(None, None), None);
    }

    #[test]
    fn resolve_lexical_scope_explicit_zero_turns_the_channel_off() {
        assert_eq!(resolve_lexical_scope(Some(0), Some(20)), None);
    }

    #[test]
    fn resolve_lexical_scope_explicit_nonzero_overrides_the_default() {
        assert_eq!(resolve_lexical_scope(Some(45), Some(20)), Some(45));
        assert_eq!(resolve_lexical_scope(Some(45), None), Some(45));
    }
}
