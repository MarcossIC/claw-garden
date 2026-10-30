//! The I/O half of `--agent-view`: ingest, serve, query, tear down.
//!
//! Everything it writes lives under one scratch directory inside the system
//! temp dir, and the run refuses any other location. The only thing read from
//! outside it is the tokenizer (and, in the caller, the model), never written.

use std::path::{Path, PathBuf};
use std::time::Instant;

use clawg_cli::client::{ClientError, DaemonClient, SearchArgs};
use clawg_cli::daemon::api::{serve_ephemeral, AppCtx, SAVE_CONFIRM_TIMEOUT};
use clawg_cli::mcp::tools::{format_context, format_search};
use clawg_core::eval_api::{
    copy_background, EvalCaseRef, EvalEmbedder, EvalError, EvalIngest, EvalResolveError, EvalResolved,
    EvalTokenCounter,
};
use clawg_core::paths::Profile;
use clawg_core::preferences::Lang;

use super::matcher::Corpus;
use super::parse::ParseError;
use super::{
    answer_reachable, answer_visible, build_results, context_section_tokens, read_context, read_search, AgentViewResults,
    AnswerView, CaseView, ContextOutcome, SearchOutcome,
};
use crate::golden::{CaseSubset, GoldenSet};
use crate::seed::{age_ms, remove_scratch, AGENT_ID};

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("scratch directory {} is not under the system temp directory {}", .0.display(), .1.display())]
    OutsideTemp(PathBuf, PathBuf),
    #[error("could not prepare the scratch directory {}: {source}", path.display())]
    Scratch {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not serve the daemon API in-process: {0}")]
    Serve(std::io::Error),
    #[error("the in-process server failed: {0}")]
    Server(String),
    #[error("could not close the store: {0}")]
    Close(String),
    #[error(transparent)]
    Eval(#[from] EvalError),
    /// A case names a project or an origin the background does not hold.
    #[error(transparent)]
    Golden(#[from] EvalResolveError),
    #[error("the daemon API answered with an error: {0}")]
    Client(#[from] ClientError),
    #[error("could not read the search output for '{query}': {source}")]
    Parse {
        query: String,
        #[source]
        source: ParseError,
    },
}

/// What a run measures against.
pub struct AgentViewInput<'a> {
    pub golden_set: &'a GoldenSet,
    pub embedder: &'a EvalEmbedder,
    pub counter: &'a EvalTokenCounter,
    /// Where the chunker looks for `tokenizer.json`: the profile directory for
    /// `--real`, so chunk boundaries match a real daemon's.
    pub tokenizer_dir: &'a Path,
    /// A snapshot the corpus is saved on top of, copied into the scratch
    /// directory and only read where it is.
    pub background: Option<&'a Path>,
}

/// What a run over a background found in it before the corpus went in.
struct Over {
    background: crate::report::Background,
    /// Each case's project and cited saves, in the order of the cases.
    resolved: Vec<EvalResolved>,
}

/// The search a case asks the daemon for, under the background's own id of
/// the project it names.
fn search_request(query: &str, project_id: Option<&str>) -> SearchArgs {
    SearchArgs {
        query: query.to_string(),
        agent_id: AGENT_ID.to_string(),
        project_id: project_id.map(str::to_string),
        limit: None,
        lang: None,
    }
}

/// A fresh scratch directory path under the system temp dir. Not created.
pub fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join(format!("rag-eval-agent-view-{}", uuid::Uuid::new_v4()))
}

/// Bearer token of the in-process server. It never leaves this process, so it
/// only needs to be the same on both ends.
const TOKEN: &str = "rag-eval-agent-view";

/// A populated store behind the daemon's real HTTP API.
struct Session {
    client: DaemonClient,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
    /// The server's state, kept past the server so that releasing the session
    /// can close the store.
    app: std::sync::Arc<AppCtx>,
    over: Option<Over>,
}

/// Ingests the corpus through the real pipeline and serves the result.
async fn start(input: &AgentViewInput<'_>, scratch: &Path) -> Result<Session, RunError> {
    let temp = std::env::temp_dir();
    if !scratch.starts_with(&temp) {
        return Err(RunError::OutsideTemp(scratch.to_path_buf(), temp));
    }
    std::fs::create_dir_all(scratch)
        .map_err(|source| RunError::Scratch { path: scratch.to_path_buf(), source })?;

    let copied = match input.background {
        Some(snapshot) => Some((snapshot, copy_background(snapshot, scratch, input.embedder)?)),
        None => None,
    };
    let ingest = EvalIngest::open(scratch, input.embedder, input.tokenizer_dir).await?;
    let fed = async {
        let over = match copied {
            Some((snapshot, copied)) => {
                let cases: Vec<EvalCaseRef> = input
                    .golden_set
                    .retrieval_cases
                    .iter()
                    .map(|c| EvalCaseRef { id: c.query.clone(), project: c.project.clone(), origins: c.expected_origins.clone() })
                    .collect();
                let resolved = ingest.resolve_cases(&cases).await?;
                let composition = ingest.composition().await?;
                let texts = input.golden_set.corpus.iter().map(|r| r.content.clone()).collect();
                let mut golden_twins: Vec<String> = ingest
                    .holding(texts)
                    .await?
                    .into_iter()
                    .map(|i| input.golden_set.corpus[i].ref_.clone())
                    .collect();
                golden_twins.sort();
                let background = crate::report::Background {
                    path: snapshot.to_string_lossy().to_string(),
                    sha256: copied.sha256,
                    contract: copied.contract,
                    schema: crate::report::SchemaSpan { from: copied.schema_from, to: copied.schema_to },
                    searched_at_ms: crate::now_ms(),
                    composition,
                    golden_twins,
                };
                Some(Over { background, resolved })
            }
            None => None,
        };
        for row in &input.golden_set.corpus {
            ingest.send(AGENT_ID, &row.content, &row.memory_type, row.importance, age_ms(row.age_days)).await?;
        }
        Ok::<_, RunError>(over)
    }
    .await;
    // The caller removes the scratch directory on a failure, which only works
    // once nothing holds the store.
    let over = match fed {
        Ok(over) => over,
        Err(e) => {
            ingest.close().await;
            return Err(e);
        }
    };
    let runtime = ingest.finish().await?;
    // The context endpoint writes a workspace file under the runtime's base
    // directory; checking the directory the runtime reports, not the one that
    // was asked for, is what makes "nothing outside temp is written" hold.
    if !runtime.base_dir.starts_with(&temp) {
        runtime.close().await;
        return Err(RunError::OutsideTemp(runtime.base_dir.clone(), temp));
    }

    let (shutdown, _) = tokio::sync::watch::channel(false);
    let app = std::sync::Arc::new(AppCtx {
        runtime,
        token: TOKEN.to_string(),
        profile: Profile::Dev,
        started_at: Instant::now(),
        shutdown,
        save_timeout: SAVE_CONFIRM_TIMEOUT,
        sessions: Default::default(),
        imports: Default::default(),
    });
    let (addr, server) = match serve_ephemeral(app.clone()).await {
        Ok(served) => served,
        Err(e) => {
            app.runtime.close().await;
            return Err(RunError::Serve(e));
        }
    };
    let client = DaemonClient::new(format!("http://{addr}"), TOKEN, scratch.to_path_buf());
    Ok(Session { client, server, app, over })
}

fn elapsed_ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// Asks the search and context endpoints about every case and scores the text
/// the bridge would render from each answer.
async fn answer_cases(
    session: &Session,
    input: &AgentViewInput<'_>,
    corpus: &Corpus,
) -> Result<Vec<CaseView>, RunError> {
    let ambiguous = corpus.ambiguous_refs();
    let lang_by_ref = crate::row_langs(&input.golden_set.corpus);
    let mut views = Vec::with_capacity(input.golden_set.retrieval_cases.len());
    for (i, case) in input.golden_set.retrieval_cases.iter().enumerate() {
        let resolution = session.over.as_ref().and_then(|o| o.resolved.get(i));
        let project_id = resolution.and_then(|r| r.project_id.as_deref());
        let expected: Vec<String> = case.expected_refs.iter().chain(&case.expected_origins).cloned().collect();
        let started = Instant::now();
        let found = session.client.search(search_request(&case.query, project_id)).await?;
        let search_text = format_search(found, AGENT_ID, None, None);
        let search_ms = elapsed_ms(started);
        let count_tokens = |line: &str| input.counter.count(line);
        let search_read = read_search(&search_text, &count_tokens, &expected, corpus)
            .map_err(|source| RunError::Parse { query: case.query.clone(), source })?;
        let search_lines_printed = search_read.lines.len();

        let started = Instant::now();
        let prompt = session.client.context(&case.query, AGENT_ID, project_id, None, None).await?.prompt;
        let context_text = format_context(Lang::En, &prompt);
        let context_ms = elapsed_ms(started);
        let context_found = read_context(&context_text, &expected, corpus);

        let answer = match case.answer.as_deref() {
            None => None,
            Some(answer) => {
                let search_visible = answer_visible(&search_text, answer);
                let saved = match search_read.first_expected_id {
                    Some(id) if !search_visible => read_save_text(&session.client, id).await?,
                    _ => None,
                };
                Some(AnswerView {
                    search_visible,
                    search_reachable: answer_reachable(search_visible, saved.as_deref(), answer),
                    context_visible: answer_visible(&context_text, answer),
                })
            }
        };

        views.push(CaseView {
            query: case.query.clone(),
            status: case.status,
            ambiguous_refs: expected.iter().filter(|r| ambiguous.contains(r.as_str())).cloned().collect(),
            expected_refs: expected,
            search: SearchOutcome {
                found: !search_read.found_refs.is_empty(),
                found_refs: search_read.found_refs,
                rank: search_read.rank,
                tokens: input.counter.count(&search_text),
                latency_ms: search_ms,
                truncated: search_read.truncated,
                lines: search_read.lines,
                repeated_rows: search_read.repeated_rows,
            },
            context: ContextOutcome {
                found: !context_found.is_empty(),
                found_refs: context_found,
                tokens: input.counter.count(&context_text),
                latency_ms: context_ms,
                sections: context_section_tokens(&context_text, &count_tokens),
            },
            subset: case.subset,
            answer,
            abstained: (case.subset == CaseSubset::Unanswerable).then_some(search_lines_printed == 0),
            query_lang: case.query_lang.clone(),
            row_lang: match resolution.and_then(|r| r.expected.first()) {
                Some(save) => Some(save.lang.clone()),
                None => crate::case_lang(&case.expected_refs, &lang_by_ref),
            },
        });
    }
    Ok(views)
}

/// The whole save of row `id`, or `None` when the daemon has no way to read
/// one: a daemon from before saves were linked answers 404 for the route.
async fn read_save_text(client: &DaemonClient, id: i64) -> Result<Option<String>, RunError> {
    match client.read_save("memory", id, Some(AGENT_ID)).await {
        Ok(saved) => Ok(Some(saved.text)),
        Err(ClientError::Daemon { status: 404, .. }) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Stops the server and closes the store, returning once neither holds a file
/// in the scratch directory.
async fn release(session: Session) -> Result<(), RunError> {
    let Session { client, server, app, .. } = session;
    client.shutdown().await?;
    drop(client);
    server
        .await
        .map_err(|e| RunError::Server(e.to_string()))?
        .map_err(|e| RunError::Server(e.to_string()))?;
    // Dropping the last handle to the store only signals the connection's
    // thread, which closes the database a moment later; on Windows its files
    // cannot be deleted until then, so the close is awaited instead.
    app.runtime.store.conn.clone().close().await.map_err(|e| RunError::Close(e.to_string()))?;
    Ok(())
}

/// Releases the session and removes the scratch directory.
async fn stop(session: Session, scratch: &Path) -> Result<(), RunError> {
    release(session).await?;
    remove_scratch(scratch).await;
    Ok(())
}

/// Runs the whole measurement inside `scratch`, then removes it. Over a
/// background, it also hands back what the background held.
pub async fn run(
    input: &AgentViewInput<'_>,
    scratch: &Path,
) -> Result<(AgentViewResults, Option<crate::report::Background>), RunError> {
    let session = match start(input, scratch).await {
        Ok(session) => session,
        Err(e) => {
            if !matches!(e, RunError::OutsideTemp(..)) {
                remove_scratch(scratch).await;
            }
            return Err(e);
        }
    };
    // A save cited by origin is matched by its joined text, so a fragment of
    // any of its pieces finds it.
    let cited: std::collections::BTreeMap<&str, &str> = session
        .over
        .iter()
        .flat_map(|o| o.resolved.iter().flat_map(|r| r.expected.iter()))
        .map(|save| (save.origin.as_str(), save.text.as_str()))
        .collect();
    let corpus = Corpus::new(
        input.golden_set.corpus.iter().map(|r| (r.ref_.as_str(), r.content.as_str())).chain(cited.into_iter()),
    );

    let measured = async {
        let stored_rows = session.client.stats(Some(AGENT_ID), None).await?.totals.memory;
        let per_case = answer_cases(&session, input, &corpus).await?;
        Ok::<_, RunError>(build_results(input.golden_set.corpus.len(), stored_rows, &corpus, per_case))
    }
    .await;

    let background = session.over.as_ref().map(|o| o.background.clone());
    let stopped = stop(session, scratch).await;
    let results = measured?;
    stopped?;
    Ok((results, background))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "the retrieval decay term compares milliseconds against seconds";
    const B: &str = "el indice knn mide distancia l2 cruda y no similitud coseno";
    const PREF: &str = "always answer in rioplatense spanish and keep explanations short";

    fn golden_set() -> GoldenSet {
        serde_json::from_value(serde_json::json!({
            "corpus": [
                {"ref": "c1", "content": A, "memory_type": "chat_message", "importance": 3.0},
                {"ref": "c2", "content": B, "memory_type": "chat_message", "importance": 3.0},
                {"ref": "c3", "content": PREF, "memory_type": "user_preference", "importance": 9.5}
            ],
            "retrieval_cases": [
                {"query": A, "corpus_refs": ["c1", "c2", "c3"], "expected_refs": ["c1"]},
                {"query": B, "corpus_refs": ["c1", "c2", "c3"], "expected_refs": ["c2"]},
                {"query": PREF, "corpus_refs": ["c1", "c2", "c3"], "expected_refs": ["c3"],
                 "status": "known_red", "blocked_by": "tier-1 rows are not returned by search"}
            ],
            "contradiction_pairs": []
        }))
        .expect("golden fixture must deserialize")
    }

    fn scratch() -> PathBuf {
        scratch_dir()
    }

    async fn rows(session: &Session) -> Vec<clawg_cli::client::MemoryRow> {
        session
            .client
            .list(clawg_cli::client::ListArgs { limit: Some(500), ..Default::default() })
            .await
            .expect("list rows")
    }

    #[test]
    fn the_search_of_a_case_names_its_project() {
        let asked = search_request("which pier", Some("app-1111"));

        assert_eq!(asked.project_id.as_deref(), Some("app-1111"));
        assert_eq!((asked.query.as_str(), asked.agent_id.as_str()), ("which pier", AGENT_ID));
        assert_eq!(search_request("which pier", None).project_id, None);
    }

    #[test]
    fn the_scratch_dir_is_unique_and_under_the_system_temp_dir() {
        let (a, b) = (scratch_dir(), scratch_dir());
        assert_ne!(a, b);
        assert!(a.starts_with(std::env::temp_dir()));
    }

    #[tokio::test]
    async fn a_scratch_dir_outside_the_temp_dir_is_refused_before_anything_is_created() {
        let golden = golden_set();
        let input = AgentViewInput {
            golden_set: &golden,
            embedder: &EvalEmbedder::fake(),
            counter: &EvalTokenCounter::fake(),
            tokenizer_dir: Path::new("."),
            background: None,
        };
        // Derived from the temp dir itself, not the working directory: a checkout
        // can live under the temp dir, and then the cwd is not "outside" at all.
        let outside = std::env::temp_dir()
            .parent()
            .expect("the temp dir has a parent")
            .join(format!("rag-eval-must-not-be-created-{}", std::process::id()));

        match run(&input, &outside).await {
            Err(RunError::OutsideTemp(path, _)) => assert_eq!(path, outside),
            other => panic!("expected OutsideTemp, got {other:?}"),
        }
        assert!(!outside.exists(), "a refused run must not leave a directory behind");
    }

    #[tokio::test]
    async fn a_run_finds_a_stored_row_in_both_outputs_and_cleans_up_after_itself() {
        let golden = golden_set();
        let scratch = scratch();
        let input = AgentViewInput {
            golden_set: &golden,
            embedder: &EvalEmbedder::fake(),
            counter: &EvalTokenCounter::fake(),
            tokenizer_dir: &scratch,
            background: None,
        };

        let (results, _) = run(&input, &scratch).await.expect("the run must succeed");

        assert_eq!(results.corpus_size, 3);
        assert_eq!(results.stored_rows, 3, "short rows are neither split nor merged");
        let first = &results.per_case[0];
        assert!(first.search.found, "an exact-text query must surface its own row in search");
        assert_eq!(first.search.rank, Some(1));
        assert!(first.context.found, "and in recall_context");
        assert!(first.search.tokens > 0 && first.context.tokens > 0);
        assert!(!scratch.exists(), "the scratch directory must be removed once the run is over");
    }

    /// On Windows a file SQLite still holds cannot be deleted, so a removal that
    /// needs a retry means the session let go of the store before it was closed.
    #[tokio::test]
    async fn a_released_session_holds_no_file_in_the_scratch_directory() {
        let golden = golden_set();
        let scratch = scratch();
        let input = AgentViewInput {
            golden_set: &golden,
            embedder: &EvalEmbedder::fake(),
            counter: &EvalTokenCounter::fake(),
            tokenizer_dir: &scratch,
            background: None,
        };
        let session = start(&input, &scratch).await.expect("start");

        release(session).await.expect("release");

        std::fs::remove_dir_all(&scratch).expect("the scratch directory must come off at the first attempt");
    }

    /// The ingest refuses a corpus row of an unknown type once the store is
    /// open, so the start fails while the store and its worker still hold it.
    #[tokio::test]
    async fn a_start_that_fails_after_opening_the_store_holds_no_file_in_the_scratch_directory() {
        let golden: GoldenSet = serde_json::from_value(serde_json::json!({
            "corpus": [{"ref": "c1", "content": A, "memory_type": "not_a_real_type", "importance": 3.0}],
            "retrieval_cases": [],
            "contradiction_pairs": []
        }))
        .expect("golden fixture must deserialize");
        let scratch = scratch();
        let input = AgentViewInput {
            golden_set: &golden,
            embedder: &EvalEmbedder::fake(),
            counter: &EvalTokenCounter::fake(),
            tokenizer_dir: &scratch,
            background: None,
        };

        match start(&input, &scratch).await {
            Err(RunError::Eval(EvalError::UnknownMemoryType(_))) => {}
            Err(other) => panic!("expected UnknownMemoryType, got {other}"),
            Ok(_) => panic!("a corpus row of an unknown type must stop the start"),
        }

        std::fs::remove_dir_all(&scratch).expect("the scratch directory must come off at the first attempt");
    }

    #[tokio::test]
    async fn a_tier_one_row_is_reported_under_known_red_with_its_own_summary() {
        let golden = golden_set();
        let scratch = scratch();
        let input = AgentViewInput {
            golden_set: &golden,
            embedder: &EvalEmbedder::fake(),
            counter: &EvalTokenCounter::fake(),
            tokenizer_dir: &scratch,
            background: None,
        };

        let (results, _) = run(&input, &scratch).await.expect("the run must succeed");

        assert_eq!(results.search.active.n_cases, 2);
        assert_eq!(results.search.known_red.n_cases, 1);
        assert_eq!(results.context.known_red.n_cases, 1);
        assert!(
            results.per_case[2].context.found,
            "a tier-1 preference reaches the agent through the workspace block"
        );
    }

    /// The answer sits in the last piece of a row the worker splits, so a
    /// fragment of the row being visible says nothing about it.
    #[tokio::test]
    async fn a_case_that_names_its_answer_reports_whether_the_answer_reached_the_agent() {
        let filler = "The team reviewed the dashboards and found nothing that needed action. ".repeat(80);
        let content = format!("Notes on amber-falcon. {filler}Batches were capped at 40 records in the end.");
        let golden: GoldenSet = serde_json::from_value(serde_json::json!({
            "corpus": [{"ref": "l1", "content": content, "memory_type": "semantic_fact", "importance": 5.0}],
            "retrieval_cases": [{
                "query": "what is the batch size limit of amber-falcon",
                "corpus_refs": ["l1"], "expected_refs": ["l1"],
                "subset": "long", "answer": "capped at 40 records"
            }],
            "contradiction_pairs": []
        }))
        .expect("golden fixture must deserialize");
        let scratch = scratch();
        let input = AgentViewInput {
            golden_set: &golden,
            embedder: &EvalEmbedder::fake(),
            counter: &EvalTokenCounter::fake(),
            tokenizer_dir: &scratch,
            background: None,
        };

        let (results, _) = run(&input, &scratch).await.expect("the run must succeed");

        assert!(results.stored_rows > 1, "the row must be split for the case to mean anything");
        let answer = results.per_case[0].answer.expect("a case with an answer is scored on it");
        assert!(answer.search_reachable || !answer.search_visible, "visible implies reachable");
        assert_eq!(results.by_subset["long"].answers.n_cases, 1);
    }

    /// 40 passes over three cases record well over the tracker's 500-id size
    /// trigger, so a live tracker would already have flushed by the time the
    /// rows are read back.
    #[tokio::test]
    async fn answering_cases_never_changes_a_stored_row() {
        let golden = golden_set();
        let scratch = scratch();
        let input = AgentViewInput {
            golden_set: &golden,
            embedder: &EvalEmbedder::fake(),
            counter: &EvalTokenCounter::fake(),
            tokenizer_dir: &scratch,
            background: None,
        };
        let session = start(&input, &scratch).await.expect("start");
        let corpus = Corpus::new(golden.corpus.iter().map(|r| (r.ref_.as_str(), r.content.as_str())));
        let rows_before = rows(&session).await;

        for _ in 0..40 {
            answer_cases(&session, &input, &corpus).await.expect("answer");
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        let rows_after = rows(&session).await;
        assert_eq!(rows_before.len(), 3);
        assert_eq!(rows_after, rows_before, "access_count and tier must not move");
        stop(session, &scratch).await.expect("stop");
    }
}
