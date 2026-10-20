//! The I/O half of `chains`: saves each arm into a store of its own, reads
//! what the detector left, lists the rows through the daemon's router and
//! removes everything it wrote.
//!
//! Every store lives under one scratch directory inside the system temp dir,
//! and the run refuses any other location. Nothing is read from outside it
//! but the tokenizer and, in the caller, the model and the specs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use clawg_cli::client::{ClientError, DaemonClient, ListArgs, MemoryRow, SearchArgs};
use clawg_cli::daemon::api::{serve_ephemeral, AppCtx, SAVE_CONFIRM_TIMEOUT};
use clawg_cli::mcp::tools::{format_context, format_search};
use clawg_core::eval_api::{
    conflict_pairs, is_claim_type, row_distances, EvalEmbedder, EvalError, EvalIngest, EvalSave, EvalSaved,
};
use clawg_core::paths::Profile;
use clawg_core::runtime::MemoryRuntime;
use serde::Serialize;

use crate::chains::{
    arm_saves, classify, compared_rows, context_ids, is_live, m1, m2, observe, own_rows, positions,
    requirement_sentences, search_ids, Arm, Band, ChainView, Outcome, Shown, Transition, M1, M2,
};
use crate::set::ChainSet;

/// The one agent that saves every arm.
const AGENT: &str = "revision-eval";

/// The one project every save goes to, as an agent saves in the project it
/// works in.
const PROJECT: &str = "revision-eval";

/// Bearer token of the in-process server. It never leaves this process, so it
/// only needs to be the same on both ends.
const TOKEN: &str = "revision-eval";

/// The most rows the router lists in one page.
const LIST_PAGE: u32 = 100;

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
    #[error("the daemon API answered with an error: {0}")]
    Client(#[from] ClientError),
}

/// What a run measures.
pub struct ChainsInput<'a> {
    pub set: &'a ChainSet,
    /// Saved as facts in every arm before its chains.
    pub distractors: &'a [String],
    pub embedder: &'a EvalEmbedder,
    /// Where the chunker looks for `tokenizer.json`: the profile directory for
    /// `--real`, so pieces are cut where a real daemon cuts them.
    pub tokenizer_dir: &'a Path,
}

/// A fresh scratch directory path under the system temp dir. Not created.
pub fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join(format!("revision-eval-chains-{}", uuid::Uuid::new_v4()))
}

/// Every MUST sentence of the `spec.md` files under `dir`, each once, in path
/// order so two runs save the same distractors.
pub fn read_distractors(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut specs: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path().join("spec.md")))
        .filter(|p| p.is_file())
        .collect();
    specs.sort();
    let mut sentences: Vec<String> = Vec::new();
    for spec in specs {
        for sentence in requirement_sentences(&std::fs::read_to_string(spec)?) {
            if !sentences.contains(&sentence) {
                sentences.push(sentence);
            }
        }
    }
    Ok(sentences)
}

/// What one arm stored and what the router lists and shows of it.
struct ArmRun<'a> {
    /// The report of each version's save, by chain and version; `None` for a
    /// version the arm does not save.
    saved: Vec<Vec<Option<EvalSaved>>>,
    /// The same, as each version's own rows ([`own_rows`]).
    own: Vec<Vec<Option<Vec<i64>>>>,
    /// Every row of the agent, by id, `superseded` ones included.
    listed: HashMap<i64, MemoryRow>,
    /// Per chain, where each version shows for the chain's query.
    shown: Vec<Shown>,
    /// Only the no-key arm's: what each of its transitions left.
    transitions: Vec<Transition<'a>>,
}

/// How much one arm stored.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArmReport {
    /// Versions the arm saved.
    pub saves: usize,
    /// Distinct rows those saves hold: a long version holds one per piece,
    /// and shares the ones it repeats with the version before.
    pub rows: usize,
    /// Versions with a row of their own ([`own_rows`]) that the router lists
    /// live.
    pub live_versions: usize,
    /// Chains with exactly one live version.
    pub chains_with_one_live_version: usize,
}

#[derive(Debug, Serialize)]
pub struct ChainsReport {
    pub model: String,
    pub set_version: u32,
    pub set_sha256: String,
    pub chains: usize,
    pub transitions: usize,
    pub distractors: usize,
    pub arms: BTreeMap<&'static str, ArmReport>,
    pub m1: M1,
    pub m2: M2,
    /// By arm and chain id, where each version shows for the chain's query.
    pub positions: BTreeMap<&'static str, BTreeMap<String, Shown>>,
}

/// Runs every arm inside `scratch`, then removes it.
pub async fn run(input: &ChainsInput<'_>, scratch: &Path) -> Result<ChainsReport, RunError> {
    let temp = std::env::temp_dir();
    if !scratch.starts_with(&temp) {
        return Err(RunError::OutsideTemp(scratch.to_path_buf(), temp));
    }
    let measured = measure(input, scratch).await;
    remove_scratch(scratch).await;
    measured
}

async fn measure(input: &ChainsInput<'_>, scratch: &Path) -> Result<ChainsReport, RunError> {
    let mut runs = Vec::new();
    for arm in Arm::ALL {
        runs.push((arm, run_arm(input, arm, &scratch.join(arm.as_str())).await?));
    }
    let run_of = |arm: Arm| runs.iter().find(|(a, _)| *a == arm).map(|(_, run)| run).expect("every arm ran");
    let views = |run: &ArmRun<'_>| -> Vec<ChainView<'_>> {
        input
            .set
            .chains
            .iter()
            .zip(live_versions(run))
            .zip(&run.shown)
            .map(|((chain, live_versions), shown)| ChainView { chain, live_versions, shown: shown.clone() })
            .collect()
    };
    let positions = runs
        .iter()
        .map(|(arm, run)| {
            let by_chain = input.set.chains.iter().zip(&run.shown).map(|(c, s)| (c.id.clone(), s.clone())).collect();
            (arm.as_str(), by_chain)
        })
        .collect();
    Ok(ChainsReport {
        model: input.embedder.model_id().to_string(),
        set_version: input.set.version,
        set_sha256: input.set.sha256.clone(),
        chains: input.set.chains.len(),
        transitions: input.set.transitions(),
        distractors: input.distractors.len(),
        arms: runs.iter().map(|(arm, run)| (arm.as_str(), arm_report(run))).collect(),
        m1: m1(&run_of(Arm::NoKey).transitions),
        m2: m2(&views(run_of(Arm::Keyed)), &views(run_of(Arm::Control))),
        positions,
    })
}

/// Saves the distractors and then the arm's versions into a store of its own
/// in `dir`, classifies the no-key arm's transitions, and reads through the
/// router every row and where each version shows for its chain's query.
async fn run_arm<'a>(input: &ChainsInput<'a>, arm: Arm, dir: &Path) -> Result<ArmRun<'a>, RunError> {
    std::fs::create_dir_all(dir).map_err(|source| RunError::Scratch { path: dir.to_path_buf(), source })?;
    let ingest = EvalIngest::open(dir, input.embedder, input.tokenizer_dir).await?;
    for distractor in input.distractors {
        ingest.save(save(distractor, "semantic_fact", None)).await?;
    }
    let mut saved: Vec<Vec<Option<EvalSaved>>> = input.set.chains.iter().map(|c| vec![None; c.versions.len()]).collect();
    for planned in arm_saves(input.set, arm) {
        let content = &planned.chain.versions[planned.version].content;
        let report = ingest.save(save(content, &planned.chain.memory_type, planned.topic_key)).await?;
        saved[planned.index][planned.version] = Some(report);
    }
    let own: Vec<Vec<Option<Vec<i64>>>> = saved
        .iter()
        .map(|versions| own_rows(&versions.iter().map(|s| s.as_ref().map(row_ids)).collect::<Vec<_>>()))
        .collect();
    let runtime = ingest.finish().await?;
    let outcomes = if arm == Arm::NoKey { read_outcomes(&runtime, input.set, &saved).await? } else { Vec::new() };

    let session = serve(runtime).await?;
    let read = read_router(&session.client, input.set, &own).await;
    let released = release(session).await;
    let (listed, shown) = read?;
    released?;

    let transitions = outcomes
        .into_iter()
        .map(|(index, to, outcome)| Transition {
            chain: &input.set.chains[index],
            to,
            outcome,
            above: shown[index].above_at(to),
        })
        .collect();
    Ok(ArmRun { saved, own, listed, shown, transitions })
}

/// Per chain, the versions with a row of their own the router lists live.
fn live_versions(run: &ArmRun<'_>) -> Vec<usize> {
    let live = |id: &i64| run.listed.get(id).is_some_and(|row| is_live(&row.status));
    run.own.iter().map(|versions| versions.iter().flatten().filter(|rows| rows.iter().any(live)).count()).collect()
}

/// Every row of the agent, and where each version of each chain shows when
/// its query goes through `search_memory` and `recall_context` as the bridge
/// renders them.
async fn read_router(
    client: &DaemonClient,
    set: &ChainSet,
    own: &[Vec<Option<Vec<i64>>>],
) -> Result<(HashMap<i64, MemoryRow>, Vec<Shown>), RunError> {
    let listed = list_rows(client).await?;
    let mut shown = Vec::with_capacity(set.chains.len());
    for (chain, own) in set.chains.iter().zip(own) {
        let found = client
            .search(SearchArgs {
                query: chain.query.clone(),
                agent_id: AGENT.to_string(),
                project_id: Some(PROJECT.to_string()),
                limit: None,
            })
            .await?;
        let search = format_search(found, AGENT, Some(PROJECT));
        let prompt = client.context(&chain.query, AGENT, Some(PROJECT), None).await?.prompt;
        let context = format_context(&prompt);
        shown.push(Shown { search: positions(own, &search_ids(&search)), context: positions(own, &context_ids(&context)) });
    }
    Ok((listed, shown))
}

fn save<'a>(content: &'a str, memory_type: &'a str, topic_key: Option<&'a str>) -> EvalSave<'a> {
    EvalSave { agent_id: AGENT, project_id: Some(PROJECT), content, memory_type, importance: 5.0, topic_key }
}

fn row_ids(saved: &EvalSaved) -> Vec<i64> {
    saved.rows.iter().map(|&(id, _)| id).collect()
}

/// The class of every transition of the no-key arm, from the pairs and the
/// distances its store holds, as `(chain index, newer version, class)`.
async fn read_outcomes(
    runtime: &MemoryRuntime,
    set: &ChainSet,
    saved: &[Vec<Option<EvalSaved>>],
) -> Result<Vec<(usize, usize, Outcome)>, RunError> {
    let pairs = conflict_pairs(runtime).await?;
    let mut outcomes = Vec::new();
    for (index, (chain, versions)) in set.chains.iter().zip(saved).enumerate() {
        for to in 1..versions.len() {
            let (Some(newer), Some(older)) = (&versions[to], &versions[to - 1]) else {
                continue;
            };
            let older = row_ids(older);
            let (probes, own) = compared_rows(&row_ids(newer), &older);
            let distances = row_distances(runtime, &probes, &own).await?.into_iter().map(|(_, _, d)| d).collect();
            // A pair with any row of the older save is one the store keeps
            // between the two saves, a row both hold included.
            let observed = observe(&pairs, &probes, &older, is_claim_type(&chain.memory_type), distances);
            outcomes.push((index, to, classify(&observed, Band::detector())));
        }
    }
    Ok(outcomes)
}

fn arm_report(run: &ArmRun<'_>) -> ArmReport {
    let rows: HashSet<i64> = run.saved.iter().flatten().flatten().flat_map(row_ids).collect();
    let live = live_versions(run);
    ArmReport {
        saves: run.saved.iter().map(|versions| versions.iter().flatten().count()).sum(),
        rows: rows.len(),
        live_versions: live.iter().sum(),
        chains_with_one_live_version: live.iter().filter(|&&n| n == 1).count(),
    }
}

/// An arm's store behind the daemon's real HTTP API.
struct Session {
    client: DaemonClient,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
    /// The server's state, kept past the server so that releasing the session
    /// can close the store.
    app: Arc<AppCtx>,
}

async fn serve(runtime: MemoryRuntime) -> Result<Session, RunError> {
    // The context endpoint writes a workspace file under the runtime's base
    // directory, so that is the directory that must be under temp.
    let temp = std::env::temp_dir();
    if !runtime.base_dir.starts_with(&temp) {
        return Err(RunError::OutsideTemp(runtime.base_dir.clone(), temp));
    }
    let base_dir = runtime.base_dir.clone();
    let (shutdown, _) = tokio::sync::watch::channel(false);
    let app = Arc::new(AppCtx {
        runtime,
        token: TOKEN.to_string(),
        profile: Profile::Dev,
        started_at: Instant::now(),
        shutdown,
        save_timeout: SAVE_CONFIRM_TIMEOUT,
        sessions: Default::default(),
    });
    let (addr, server) = serve_ephemeral(app.clone()).await.map_err(RunError::Serve)?;
    let client = DaemonClient::new(format!("http://{addr}"), TOKEN, base_dir);
    Ok(Session { client, server, app })
}

/// Every row of the agent, as the router lists it.
async fn list_rows(client: &DaemonClient) -> Result<HashMap<i64, MemoryRow>, RunError> {
    let mut rows = HashMap::new();
    let mut offset = 0;
    loop {
        let page = client
            .list(ListArgs {
                kind: Some("memory".into()),
                agent: Some(AGENT.into()),
                limit: Some(LIST_PAGE),
                offset: Some(offset),
                ..Default::default()
            })
            .await?;
        let listed = page.len();
        rows.extend(page.into_iter().map(|row| (row.id, row)));
        if listed < LIST_PAGE as usize {
            return Ok(rows);
        }
        offset += LIST_PAGE;
    }
}

/// Stops the server and closes the store, returning once neither holds a file
/// in the scratch directory.
async fn release(session: Session) -> Result<(), RunError> {
    let Session { client, server, app } = session;
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

/// Removal is retried: on Windows another process (a virus scanner, the
/// search indexer) can hold a file it has just seen written for a moment, and
/// a directory holding an open file cannot be deleted. Failing to delete is
/// reported, not fatal: the measurement itself is already complete.
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

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    use crate::set::Chain;

    /// A long decision of three versions that differ only at the end, so they
    /// share their first pieces as the long chains of the set do, and a short
    /// preference of two. The decision's query names an identifier its
    /// opening holds, which the search shows even on the fake embedder.
    fn tiny_set() -> ChainSet {
        let opening: String = (1..=120).map(|i| format!(" Order batch {i} is kept on the primary.")).collect();
        let long = |end: &str| format!("Primary store of orders_service.{opening} {end}");
        let chains: Vec<Chain> = serde_json::from_value(json!([
            {"id": "a", "lang": "en", "memory_type": "architectural_decision", "topic_key": "storage/orders",
             "query": "Which database does orders_service use?",
             "versions": [
                {"content": long("The engine is PostgreSQL 15."), "value": "PostgreSQL 15"},
                {"content": long("The engine is PostgreSQL 16."), "value": "PostgreSQL 16", "change": "value"},
                {"content": long("The engine is PostgreSQL 16, replicated to the reporting replica."),
                 "value": "reporting replica", "change": "extend"}
             ]},
            {"id": "b", "lang": "es", "memory_type": "user_preference", "topic_key": "style/indent",
             "query": "¿Qué indentación quiere el usuario?",
             "versions": [
                {"content": "El usuario quiere 2 espacios de indentación.", "value": "2 espacios"},
                {"content": "El usuario quiere 4 espacios de indentación.", "value": "4 espacios", "change": "value"}
             ]}
        ]))
        .unwrap();
        ChainSet { version: 1, sha256: "test".into(), chains }
    }

    fn distractors() -> Vec<String> {
        vec!["The worker MUST hand the attempt back when the embedder is not ready.".to_string()]
    }

    #[tokio::test]
    async fn the_router_lists_a_revised_row_superseded_by_the_new_one() {
        let set = tiny_set();
        let scratch = scratch_dir();
        let embedder = EvalEmbedder::fake();
        let input = ChainsInput { set: &set, distractors: &distractors(), embedder: &embedder, tokenizer_dir: &scratch };

        let run = run_arm(&input, Arm::Keyed, &scratch.join(Arm::Keyed.as_str())).await;
        remove_scratch(&scratch).await;
        let run = run.expect("the keyed arm must run");

        let rows = |chain: usize, version: usize| run.saved[chain][version].as_ref().expect("saved").rows.clone();
        let (v1, v2) = (rows(1, 0), rows(1, 1));
        let old = &run.listed[&v1[0].0];
        assert_eq!(old.status, "superseded");
        assert_eq!(old.superseded_by, Some(v2[0].0));
        assert_eq!(run.listed[&v2[0].0].status, "active");
        assert_eq!(run.listed[&v2[0].0].topic_key.as_deref(), Some("style/indent"));

        let ids = |version: usize| rows(0, version).into_iter().map(|(id, _)| id).collect::<Vec<_>>();
        let own = own_rows(&[Some(ids(0)), Some(ids(1)), Some(ids(2))]);
        assert!(ids(2).iter().any(|id| ids(0).contains(id)), "the long versions must share a piece to mean anything");
        for id in own[0].iter().chain(&own[1]).flatten() {
            assert_eq!(run.listed[id].status, "superseded", "#{id} is an older version's own row");
        }
        for id in ids(2) {
            assert_eq!(run.listed[&id].status, "active", "#{id} is held by the last version");
        }
    }

    /// The double places texts far apart, so nothing pairs: this checks that
    /// every arm goes through the worker and the router and the report is
    /// whole, never the numbers.
    #[tokio::test]
    async fn a_fake_chains_run_stores_every_arm_and_reports_whole() {
        let set = tiny_set();
        let scratch = scratch_dir();
        let embedder = EvalEmbedder::fake();
        let input = ChainsInput { set: &set, distractors: &distractors(), embedder: &embedder, tokenizer_dir: &scratch };

        let report = run(&input, &scratch).await.expect("the run must succeed");

        assert!(!scratch.exists(), "the scratch directory must be removed once the run is over");
        assert_eq!((report.chains, report.transitions, report.distractors), (2, 3, 1));
        let arm = |name: &str| report.arms[name].clone();
        let no_key = arm("no_key");
        assert_eq!((no_key.saves, no_key.live_versions, no_key.chains_with_one_live_version), (5, 5, 0));
        assert!(no_key.rows > no_key.saves, "the long version holds a row per piece");
        let keyed = arm("keyed");
        assert_eq!((keyed.saves, keyed.live_versions, keyed.chains_with_one_live_version), (5, 2, 2));
        let control = arm("control");
        assert_eq!((control.saves, control.live_versions, control.chains_with_one_live_version), (2, 2, 2));
        let m1 = &report.m1.all;
        assert_eq!(m1.transitions, 3);
        assert_eq!(m1.escalated.k + m1.pending.k + m1.no_mark.k, 3);
        assert_eq!(m1.no_mark_reasons.below_floor.k, 0, "a row two versions share is never compared with itself");

        for arm in ["no_key", "keyed", "control"] {
            for chain in &set.chains {
                let shown = &report.positions[arm][&chain.id];
                assert_eq!((shown.search.len(), shown.context.len()), (chain.versions.len(), chain.versions.len()));
            }
        }
        for arm in ["keyed", "control"] {
            let shown = &report.positions[arm]["a"];
            assert!(shown.search[2].is_some() && shown.context[2].is_some(), "{arm}: the last version shows: {shown:?}");
            assert_eq!((shown.search[..2].to_vec(), shown.context[..2].to_vec()), (vec![None, None], vec![None, None]));
        }
        assert!(report.m2.one_live_version.met && report.m2.no_old_version_shown.met, "{:?}", report.m2);
    }

    #[tokio::test]
    async fn a_scratch_dir_outside_the_temp_dir_is_refused_before_anything_is_created() {
        let set = tiny_set();
        let embedder = EvalEmbedder::fake();
        let outside = std::env::temp_dir()
            .parent()
            .expect("the temp dir has a parent")
            .join(format!("revision-eval-must-not-be-created-{}", std::process::id()));
        let input = ChainsInput { set: &set, distractors: &distractors(), embedder: &embedder, tokenizer_dir: &outside };

        match run(&input, &outside).await {
            Err(RunError::OutsideTemp(path, _)) => assert_eq!(path, outside),
            other => panic!("expected OutsideTemp, got {other:?}"),
        }
        assert!(!outside.exists(), "a refused run must not leave a directory behind");
    }

    #[test]
    fn the_repository_specs_give_at_least_two_hundred_distractors() {
        let specs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../openspec/specs");
        let distractors = read_distractors(&specs).unwrap();
        assert!(distractors.len() >= 200, "{}", distractors.len());
    }
}
