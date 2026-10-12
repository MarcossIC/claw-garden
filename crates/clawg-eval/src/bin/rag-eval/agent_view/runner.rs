//! The I/O half of `--agent-view`: ingest, serve, query, tear down.
//!
//! Everything it writes lives under one scratch directory inside the system
//! temp dir, and the run refuses any other location. The only thing read from
//! outside it is the tokenizer (and, in the caller, the model), never written.

use std::path::{Path, PathBuf};
use std::time::Instant;

use clawg_cli::client::{ClientError, DaemonClient, SearchArgs};
use clawg_cli::daemon::api::{serve_ephemeral, AppCtx};
use clawg_cli::mcp::tools::{format_context, format_search};
use clawg_core::eval_api::{EvalEmbedder, EvalError, EvalIngest, EvalTokenCounter};
use clawg_core::paths::Profile;

use super::matcher::Corpus;
use super::parse::ParseError;
use super::{build_results, read_context, read_search, AgentViewResults, CaseView, ContextOutcome, SearchOutcome};
use crate::golden::GoldenSet;
use crate::seed::AGENT_ID;

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
    #[error(transparent)]
    Eval(#[from] EvalError),
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
}

/// Ingests the corpus through the real pipeline and serves the result.
async fn start(input: &AgentViewInput<'_>, scratch: &Path) -> Result<Session, RunError> {
    let temp = std::env::temp_dir();
    if !scratch.starts_with(&temp) {
        return Err(RunError::OutsideTemp(scratch.to_path_buf(), temp));
    }
    std::fs::create_dir_all(scratch)
        .map_err(|source| RunError::Scratch { path: scratch.to_path_buf(), source })?;

    let ingest = EvalIngest::open(scratch, input.embedder, input.tokenizer_dir).await?;
    for row in &input.golden_set.corpus {
        ingest.send(AGENT_ID, &row.content, &row.memory_type, row.importance)?;
    }
    let runtime = ingest.finish().await?;
    // The context endpoint writes a workspace file under the runtime's base
    // directory; checking the directory the runtime reports, not the one that
    // was asked for, is what makes "nothing outside temp is written" hold.
    if !runtime.base_dir.starts_with(&temp) {
        return Err(RunError::OutsideTemp(runtime.base_dir.clone(), temp));
    }

    let (shutdown, _) = tokio::sync::watch::channel(false);
    let app = std::sync::Arc::new(AppCtx {
        runtime,
        token: TOKEN.to_string(),
        profile: Profile::Dev,
        started_at: Instant::now(),
        shutdown,
    });
    let (addr, server) = serve_ephemeral(app).await.map_err(RunError::Serve)?;
    let client = DaemonClient::new(format!("http://{addr}"), TOKEN, scratch.to_path_buf());
    Ok(Session { client, server })
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
    let mut views = Vec::with_capacity(input.golden_set.retrieval_cases.len());
    for case in &input.golden_set.retrieval_cases {
        let started = Instant::now();
        let hits = session
            .client
            .search(SearchArgs {
                query: case.query.clone(),
                agent_id: AGENT_ID.to_string(),
                project_id: None,
                limit: None,
            })
            .await?;
        let search_text = format_search(hits);
        let search_ms = elapsed_ms(started);
        let count_tokens = |line: &str| input.counter.count(line);
        let search_read = read_search(&search_text, &count_tokens, &case.expected_refs, corpus)
            .map_err(|source| RunError::Parse { query: case.query.clone(), source })?;

        let started = Instant::now();
        let prompt = session.client.context(&case.query, AGENT_ID, None).await?;
        let context_text = format_context(&prompt);
        let context_ms = elapsed_ms(started);
        let context_found = read_context(&context_text, &case.expected_refs, corpus);

        views.push(CaseView {
            query: case.query.clone(),
            expected_refs: case.expected_refs.clone(),
            status: case.status,
            ambiguous_refs: case
                .expected_refs
                .iter()
                .filter(|r| ambiguous.contains(r.as_str()))
                .cloned()
                .collect(),
            search: SearchOutcome {
                found: !search_read.found_refs.is_empty(),
                found_refs: search_read.found_refs,
                rank: search_read.rank,
                tokens: input.counter.count(&search_text),
                latency_ms: search_ms,
                truncated: search_read.truncated,
                lines: search_read.lines,
            },
            context: ContextOutcome {
                found: !context_found.is_empty(),
                found_refs: context_found,
                tokens: input.counter.count(&context_text),
                latency_ms: context_ms,
            },
        });
    }
    Ok(views)
}

/// Stops the server, waits until it has let go of the store, and removes the
/// scratch directory.
async fn stop(session: Session, scratch: &Path) -> Result<(), RunError> {
    let Session { client, server } = session;
    client.shutdown().await?;
    drop(client);
    server
        .await
        .map_err(|e| RunError::Server(e.to_string()))?
        .map_err(|e| RunError::Server(e.to_string()))?;
    remove_scratch(scratch).await;
    Ok(())
}

/// Removal is retried because the database connection closes on its own
/// thread, a moment after the store handle is dropped; on Windows a directory
/// holding an open file cannot be deleted. Failing to delete is reported, not
/// fatal: the measurement itself is already complete.
async fn remove_scratch(scratch: &Path) {
    for _ in 0..40 {
        match std::fs::remove_dir_all(scratch) {
            Ok(()) => return,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
        }
    }
    eprintln!("warning: could not remove the scratch directory {}", scratch.display());
}

/// Runs the whole measurement inside `scratch`, then removes it.
pub async fn run(input: &AgentViewInput<'_>, scratch: &Path) -> Result<AgentViewResults, RunError> {
    let session = match start(input, scratch).await {
        Ok(session) => session,
        Err(e) => {
            if !matches!(e, RunError::OutsideTemp(..)) {
                remove_scratch(scratch).await;
            }
            return Err(e);
        }
    };
    let corpus =
        Corpus::new(input.golden_set.corpus.iter().map(|r| (r.ref_.as_str(), r.content.as_str())));

    let measured = async {
        let stored_rows = session.client.stats(Some(AGENT_ID), None).await?.totals.memory;
        let per_case = answer_cases(&session, input, &corpus).await?;
        Ok::<_, RunError>(build_results(input.golden_set.corpus.len(), stored_rows, &corpus, per_case))
    }
    .await;

    let stopped = stop(session, scratch).await;
    let results = measured?;
    stopped?;
    Ok(results)
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
        };

        let results = run(&input, &scratch).await.expect("the run must succeed");

        assert_eq!(results.corpus_size, 3);
        assert_eq!(results.stored_rows, 3, "short rows are neither split nor merged");
        let first = &results.per_case[0];
        assert!(first.search.found, "an exact-text query must surface its own row in search");
        assert_eq!(first.search.rank, Some(1));
        assert!(first.context.found, "and in recall_context");
        assert!(first.search.tokens > 0 && first.context.tokens > 0);
        assert!(!scratch.exists(), "the scratch directory must be removed once the run is over");
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
        };

        let results = run(&input, &scratch).await.expect("the run must succeed");

        assert_eq!(results.search.active.n_cases, 2);
        assert_eq!(results.search.known_red.n_cases, 1);
        assert_eq!(results.context.known_red.n_cases, 1);
        assert!(
            results.per_case[2].context.found,
            "a tier-1 preference reaches the agent through the workspace block"
        );
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
