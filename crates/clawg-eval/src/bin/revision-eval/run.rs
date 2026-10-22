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
    conflict_pairs, drop_superseded_text, drop_superseded_vectors, is_claim_type, row_distances, EvalEmbedder,
    EvalError, EvalIngest, EvalSave, EvalSaved, EvalTokenCounter,
};
use clawg_core::paths::Profile;
use clawg_core::runtime::MemoryRuntime;
use serde::Serialize;

use crate::chains::{
    arm_saves, classify, compared_rows, context_ids, is_live, m1, m2, observe, own_rows, positions,
    requirement_sentences, search_ids, Arm, Band, ChainView, Outcome, Shown, Transition, M1, M2,
};
use crate::cost::{self, CostError, Query, M4};
use crate::set::ChainSet;
use crate::store::{self, ReplayFile, Scored, StoreError};
use crate::warned::{self, PickKind, ReplayRows, WarnedError, WarnedFile};

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
    #[error(transparent)]
    Cost(#[from] CostError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Warned(#[from] WarnedError),
    #[error("could not read or write {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0}")]
    Refused(String),
}

impl RunError {
    /// Whether the run failed on what the owner handed it, not on the run.
    pub fn is_bad_input(&self) -> bool {
        match self {
            Self::Store(e) => !matches!(e, StoreError::Replay(_)),
            Self::Warned(e) => !matches!(e, WarnedError::Replay(_)),
            Self::OutsideTemp(..) | Self::Refused(_) => true,
            _ => false,
        }
    }

    /// Whether the working directory stays for another try: the owner's
    /// labels are in it.
    fn keeps_work(&self) -> bool {
        match self {
            Self::Store(e) => e.keeps_work(),
            Self::Warned(e) => e.keeps_work(),
            Self::Refused(_) | Self::OutsideTemp(..) => true,
            _ => false,
        }
    }
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
    /// Counts the tokens of M4; the same tokenizer the chunker loads for
    /// `--real`.
    pub counter: &'a EvalTokenCounter,
    /// Saved in every arm before anything else, under the arm's agent and
    /// project: the history of a real store the chains are saved on top of.
    pub background: Option<&'a store::Background>,
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
    /// What the router answered each chain's query, per chain.
    queries: Vec<Query>,
    /// Vectors of `superseded` rows removed before the queries.
    old_vectors_dropped: usize,
    /// `superseded` rows taken out of `memory_fts` before the queries.
    old_texts_dropped: usize,
    /// Background saves stored before the distractors.
    background_saves: usize,
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
    /// Vectors of `superseded` rows the arm removed before its queries; only
    /// the bare arms remove any.
    pub old_vectors_dropped: usize,
    /// `superseded` rows the arm took out of `memory_fts` before its queries;
    /// only the unindexed arm takes any.
    pub old_texts_dropped: usize,
    /// Background saves the arm stored before its distractors.
    pub background_saves: usize,
}

#[derive(Debug, Serialize)]
pub struct ChainsReport {
    /// The embedding model's id, which names its contract.
    pub model_contract: String,
    pub set_version: u32,
    pub set_sha256: String,
    pub chains: usize,
    pub transitions: usize,
    pub distractors: usize,
    /// What the background held, when the run had one.
    pub background: Option<store::BackgroundCounts>,
    pub arms: BTreeMap<&'static str, ArmReport>,
    pub m1: M1,
    /// The keyed arm against the unindexed one: the same store with the older
    /// versions in no index, so they are all that tells the two apart. The
    /// control also differs in the store's history (the superseded rows, the
    /// pairs written against versions still live then, the rows' ages), which
    /// orders hits that score almost the same.
    pub m2: M2,
    /// The keyed arm against the control, kept as a reference.
    pub m2_control: M2,
    /// What showing the key costs, over the keyed arm's queries.
    pub m4: M4,
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
    let keyed = run_of(Arm::Keyed);
    let keys: HashMap<i64, String> =
        keyed.listed.values().filter_map(|row| row.topic_key.clone().map(|key| (row.id, key))).collect();
    let longest_key = input.set.chains.iter().map(|c| c.topic_key.as_str()).max_by_key(|k| k.len()).unwrap_or_default();
    let m4 = cost::m4(&keyed.queries, &keys, longest_key, AGENT, PROJECT, input.counter)?;
    Ok(ChainsReport {
        model_contract: input.embedder.model_id().to_string(),
        set_version: input.set.version,
        set_sha256: input.set.sha256.clone(),
        chains: input.set.chains.len(),
        transitions: input.set.transitions(),
        distractors: input.distractors.len(),
        background: input.background.map(|b| b.counts.clone()),
        arms: runs.iter().map(|(arm, run)| (arm.as_str(), arm_report(run))).collect(),
        m1: m1(&run_of(Arm::NoKey).transitions),
        m2: m2(&views(run_of(Arm::Keyed)), &views(run_of(Arm::KeyedUnindexed))),
        m2_control: m2(&views(run_of(Arm::Keyed)), &views(run_of(Arm::Control))),
        m4,
        positions,
    })
}

/// Saves the distractors and then the arm's versions into a store of its own
/// in `dir`, classifies the no-key arm's transitions, and reads through the
/// router every row and where each version shows for its chain's query.
async fn run_arm<'a>(input: &ChainsInput<'a>, arm: Arm, dir: &Path) -> Result<ArmRun<'a>, RunError> {
    std::fs::create_dir_all(dir).map_err(|source| RunError::Scratch { path: dir.to_path_buf(), source })?;
    let ingest = EvalIngest::open(dir, input.embedder, input.tokenizer_dir).await?;
    let background = input.background.map_or(&[][..], |b| b.saves.as_slice());
    for older in background {
        let topic_key = older.topic_key.as_deref();
        ingest.save(EvalSave { importance: older.importance, ..save(&older.text, &older.memory_type, topic_key) }).await?;
    }
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
    let bare = matches!(arm, Arm::KeyedBare | Arm::KeyedUnindexed);
    let old_vectors_dropped = if bare { drop_superseded_vectors(&runtime).await? } else { 0 };
    let old_texts_dropped = if arm == Arm::KeyedUnindexed { drop_superseded_text(&runtime).await? } else { 0 };

    let session = serve(runtime).await?;
    let read = read_router(&session.client, input.set, &own).await;
    let released = release(session).await;
    let (listed, shown, queries) = read?;
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
    Ok(ArmRun {
        saved,
        own,
        listed,
        shown,
        transitions,
        queries,
        old_vectors_dropped,
        old_texts_dropped,
        background_saves: background.len(),
    })
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
) -> Result<(HashMap<i64, MemoryRow>, Vec<Shown>, Vec<Query>), RunError> {
    let listed = list_rows(client).await?;
    let mut shown = Vec::with_capacity(set.chains.len());
    let mut queries = Vec::with_capacity(set.chains.len());
    for (chain, own) in set.chains.iter().zip(own) {
        let found = client
            .search(SearchArgs {
                query: chain.query.clone(),
                agent_id: AGENT.to_string(),
                project_id: Some(PROJECT.to_string()),
                limit: None,
            })
            .await?;
        let search = format_search(found.clone(), AGENT, Some(PROJECT));
        let prompt = client.context(&chain.query, AGENT, Some(PROJECT), None).await?.prompt;
        let context = format_context(&prompt);
        shown.push(Shown { search: positions(own, &search_ids(&search)), context: positions(own, &context_ids(&context)) });
        queries.push(Query { found, prompt });
    }
    Ok((listed, shown, queries))
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
        old_vectors_dropped: run.old_vectors_dropped,
        old_texts_dropped: run.old_texts_dropped,
        background_saves: run.background_saves,
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

const LABELS_FILE: &str = "labels.jsonl";
const REPLAY_FILE: &str = "replay.json";
const WARNED_FILE: &str = "warned.json";
const REPLAY_STORE: &str = "replay";
const WORK_DIR_PREFIX: &str = "revision-eval-";

/// What the first step of `store` replays with.
pub struct ReplayInput<'a> {
    pub embedder: &'a EvalEmbedder,
    /// Where the chunker looks for `tokenizer.json`, as in [`ChainsInput`].
    pub tokenizer_dir: &'a Path,
    pub counter: &'a EvalTokenCounter,
    pub seed: u64,
    pub fake: bool,
}

/// What the first step reports: counts only, never a stored text.
#[derive(Debug, Serialize)]
pub struct ReplaySummary {
    pub export: store::ExportCounts,
    pub replay: store::ReplayCounts,
    pub labelled_firing: usize,
    pub labelled_sample: usize,
    pub seed: u64,
}

/// The directory `file` sits in: a `revision-eval-*` directory made directly
/// under the system temp dir. Every file of a `store` run lives in it, and it
/// is the one thing removed, so nothing else is ever deleted.
fn work_dir(file: &Path) -> Result<PathBuf, RunError> {
    let temp = std::env::temp_dir();
    let outside = || RunError::OutsideTemp(file.to_path_buf(), temp.clone());
    let dir = file.parent().and_then(|p| p.canonicalize().ok()).ok_or_else(outside)?;
    let root = temp.canonicalize().map_err(|_| outside())?;
    if !dir.starts_with(&root) {
        return Err(outside());
    }
    let named = dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(WORK_DIR_PREFIX));
    if dir.parent() != Some(root.as_path()) || !named {
        return Err(RunError::Refused(format!(
            "{} must sit in a {WORK_DIR_PREFIX}* directory made directly under {}",
            file.display(),
            temp.display()
        )));
    }
    Ok(dir)
}

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> RunError + '_ {
    move |source| RunError::Io { path: path.to_path_buf(), source }
}

fn write_new_file(path: &Path, text: &str) -> Result<(), RunError> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(io_error(path))?;
    file.write_all(text.as_bytes()).map_err(io_error(path))
}

/// Runs every arm on top of the export at `export`, which must sit in a
/// `revision-eval-*` directory made directly under the system temp dir, and
/// then removes that directory and the scratch one, whatever the outcome.
pub async fn run_with_background(
    input: &ChainsInput<'_>,
    scratch: &Path,
    export: &Path,
) -> Result<ChainsReport, RunError> {
    let dir = work_dir(export)?;
    let measured = async {
        let text = std::fs::read_to_string(export).map_err(io_error(export))?;
        let background = store::background(store::read_export(&text)?);
        drop(text);
        run(&ChainsInput { background: Some(&background), ..*input }, scratch).await
    }
    .await;
    remove_scratch(&dir).await;
    measured
}

/// The first step: replays the export beside it, leaves `labels.jsonl` and
/// `replay.json` in its directory and removes the replay store. Any error
/// removes the directory with the export in it.
pub async fn store_replay(export: &Path, input: &ReplayInput<'_>) -> Result<ReplaySummary, RunError> {
    let dir = work_dir(export)?;
    let replayed = replay_in(&dir, export, input).await;
    if replayed.is_err() {
        remove_scratch(&dir).await;
    }
    replayed
}

async fn replay_in(dir: &Path, export: &Path, input: &ReplayInput<'_>) -> Result<ReplaySummary, RunError> {
    let text = std::fs::read_to_string(export).map_err(io_error(export))?;
    let parsed = store::read_export(&text)?;
    drop(text);
    let flow = &parsed.flow;

    let store_dir = dir.join(REPLAY_STORE);
    std::fs::create_dir_all(&store_dir).map_err(|source| RunError::Scratch { path: store_dir.clone(), source })?;
    let ingest = EvalIngest::open(&store_dir, input.embedder, input.tokenizer_dir).await?;
    // The order the saves were made in, each with what it was saved with:
    // `import` would insert the rows without running the detector.
    let mut reports = Vec::with_capacity(flow.len());
    for save in flow {
        reports.push(
            ingest
                .save(EvalSave {
                    agent_id: &save.agent,
                    project_id: save.project.as_deref(),
                    content: &save.text,
                    memory_type: &save.memory_type,
                    importance: save.importance,
                    topic_key: save.topic_key.as_deref(),
                })
                .await?,
        );
    }
    let runtime = ingest.finish().await?;

    let analysis = store::analyse(flow, &reports);
    let mut selected = store::select(&analysis, input.seed);
    for pick in selected.iter_mut().filter(|p| p.earlier.is_none()) {
        pick.earlier = nearest_earlier(&runtime, &analysis, flow, &reports[pick.save], pick.save).await?;
    }
    // Closing is awaited, as in `release`: Windows cannot delete the files
    // of a store whose connection has not closed yet.
    runtime.store.conn.clone().close().await.map_err(|e| RunError::Close(e.to_string()))?;
    drop(runtime);
    remove_scratch(&store_dir).await;

    let labels = store::build_labels(flow, &selected);
    let replay = ReplayFile {
        export_format: parsed.format.clone(),
        export_version: parsed.version,
        model_contract: input.embedder.model_id().to_string(),
        seed: input.seed,
        fake: input.fake,
        export: parsed.counts.clone(),
        replay: analysis.counts.clone(),
        warning_line_tokens: input.counter.count(cost::WARNING_LINE),
        labelled: labels.meta.clone(),
    };
    let lines: String =
        labels.lines.iter().map(|l| serde_json::to_string(l).expect("a label line serializes") + "\n").collect();
    write_new_file(&dir.join(LABELS_FILE), &lines)?;
    let replay_json = serde_json::to_string_pretty(&replay).expect("the replay file serializes");
    write_new_file(&dir.join(REPLAY_FILE), &replay_json)?;
    Ok(ReplaySummary {
        export: parsed.counts.clone(),
        replay: analysis.counts.clone(),
        labelled_firing: labels.meta.iter().filter(|m| m.fired).count(),
        labelled_sample: labels.meta.iter().filter(|m| !m.fired).count(),
        seed: input.seed,
    })
}

/// The earlier save of the same agent and project whose rows lie closest to
/// the rows the save `later` left, among those still live when it came.
async fn nearest_earlier(
    runtime: &MemoryRuntime,
    analysis: &store::Analysis,
    flow: &[store::Save],
    report: &EvalSaved,
    later: usize,
) -> Result<Option<usize>, RunError> {
    let predecessors = analysis.predecessors(flow, later);
    let to: Vec<i64> = predecessors.iter().flat_map(|&j| analysis.rows_of(j).iter().copied()).collect();
    let from = row_ids(report);
    let mut nearest: BTreeMap<usize, f64> = BTreeMap::new();
    for (_, row, distance) in row_distances(runtime, &from, &to).await? {
        let Some(earlier) = analysis.owner_of(row) else { continue };
        let best = nearest.entry(earlier).or_insert(distance);
        *best = best.min(distance);
    }
    Ok(store::closest(&nearest.into_iter().collect::<Vec<_>>()))
}

/// The second step: reads the labels beside `labels` and the replay file,
/// scores them, hands the result to `write` and removes the directory. A
/// labels file the owner has not finished is refused and the directory stays,
/// since the labelling is in it; every other error removes it.
pub async fn store_score(
    labels: &Path,
    check: impl FnOnce(&ReplayFile) -> Result<(), RunError>,
    write: impl FnOnce(&Scored) -> Result<(), RunError>,
) -> Result<Scored, RunError> {
    let dir = work_dir(labels)?;
    let scored = read_and_score(&dir, labels, check);
    settle(&dir, scored, write).await
}

/// What a second step does with its outcome: a result is written and the
/// directory removed, whether or not the writing worked; an error removes the
/// directory too, unless the labelling in it is what needs fixing.
async fn settle<T>(
    dir: &Path,
    scored: Result<T, RunError>,
    write: impl FnOnce(&T) -> Result<(), RunError>,
) -> Result<T, RunError> {
    match &scored {
        Err(e) if e.keeps_work() => {}
        Err(_) => remove_scratch(dir).await,
        Ok(scored) => {
            let written = write(scored);
            remove_scratch(dir).await;
            written?;
        }
    }
    scored
}

fn read_and_score(
    dir: &Path,
    labels: &Path,
    check: impl FnOnce(&ReplayFile) -> Result<(), RunError>,
) -> Result<Scored, RunError> {
    let replay_path = dir.join(REPLAY_FILE);
    let replay = store::parse_replay(&std::fs::read_to_string(&replay_path).map_err(io_error(&replay_path))?)?;
    check(&replay)?;
    let labels = store::parse_labels(&std::fs::read_to_string(labels).map_err(io_error(labels))?)?;
    Ok(store::score(&replay, &labels)?)
}

/// What the first step of `warned` reports: counts only, never a stored text.
#[derive(Debug, Serialize)]
pub struct WarnedSummary {
    pub counts: warned::WindowCounts,
    pub labelled_marked: usize,
    pub labelled_rest: usize,
    pub window_start_ms: i64,
    pub seed: u64,
}

/// The first step of `warned`: reads the window of the export beside it,
/// leaves `labels.jsonl` and `warned.json` in its directory and removes the
/// replay store. Any error removes the directory with the export in it.
pub async fn warned_replay(export: &Path, since_ms: i64, input: &ReplayInput<'_>) -> Result<WarnedSummary, RunError> {
    let dir = work_dir(export)?;
    let replayed = warned_replay_in(&dir, export, since_ms, input).await;
    if replayed.is_err() {
        remove_scratch(&dir).await;
    }
    replayed
}

async fn warned_replay_in(
    dir: &Path,
    export: &Path,
    since_ms: i64,
    input: &ReplayInput<'_>,
) -> Result<WarnedSummary, RunError> {
    let text = std::fs::read_to_string(export).map_err(io_error(export))?;
    let selection = warned::select(&text, since_ms, input.seed)?;
    drop(text);

    let store_dir = dir.join(REPLAY_STORE);
    std::fs::create_dir_all(&store_dir).map_err(|source| RunError::Scratch { path: store_dir.clone(), source })?;
    let ingest = EvalIngest::open(&store_dir, input.embedder, input.tokenizer_dir).await?;
    // Every save with an agent, in the order they were made: the rest is
    // paired by the distances between the rows these leave.
    let mut reports = Vec::with_capacity(selection.flow.len());
    for &i in &selection.flow {
        let save = &selection.saves[i];
        reports.push(
            ingest
                .save(EvalSave {
                    agent_id: save.agent.as_deref().expect("the flow holds the saves that have an agent"),
                    project_id: save.project.as_deref(),
                    content: &save.text,
                    memory_type: &save.memory_type,
                    importance: save.importance,
                    topic_key: save.topic_key.as_deref(),
                })
                .await?,
        );
    }
    let runtime = ingest.finish().await?;

    let rows = ReplayRows::new(&selection.flow, &reports);
    let mut found = Vec::with_capacity(selection.picks.len());
    for pick in &selection.picks {
        found.push(match &pick.kind {
            PickKind::Marked { .. } => None,
            PickKind::Rest { predecessors } => {
                let distances = row_distances(&runtime, &rows.reported(pick.later), &rows.inserted(predecessors)).await?;
                rows.nearest(&distances)
            }
        });
    }
    // Closing is awaited, as in `release`: Windows cannot delete the files
    // of a store whose connection has not closed yet.
    runtime.store.conn.clone().close().await.map_err(|e| RunError::Close(e.to_string()))?;
    drop(runtime);
    remove_scratch(&store_dir).await;

    let labels = warned::build_labels(&selection, &found);
    let mut counts = selection.counts.clone();
    counts.rest_unpaired = labels.unpaired;
    let file = WarnedFile {
        export_format: selection.format.clone(),
        export_version: selection.version,
        model_contract: input.embedder.model_id().to_string(),
        seed: input.seed,
        fake: input.fake,
        window_start_ms: selection.window_start_ms,
        counts: counts.clone(),
        warning_line_tokens: input.counter.count(cost::WARNING_LINE),
        store_sizes: selection.store_sizes.clone(),
        pairs: labels.pairs.clone(),
    };
    let lines: String =
        labels.lines.iter().map(|l| serde_json::to_string(l).expect("a label line serializes") + "\n").collect();
    write_new_file(&dir.join(LABELS_FILE), &lines)?;
    let file_json = serde_json::to_string_pretty(&file).expect("the warned file serializes");
    write_new_file(&dir.join(WARNED_FILE), &file_json)?;
    Ok(WarnedSummary {
        counts,
        labelled_marked: labels.pairs.iter().filter(|p| p.marked).count(),
        labelled_rest: labels.pairs.iter().filter(|p| !p.marked).count(),
        window_start_ms: selection.window_start_ms,
        seed: input.seed,
    })
}

/// The second step of `warned`: reads the labels beside `labels` and the file
/// the first step left, scores them, hands the result to `write` and removes
/// the directory. A labels file the owner has not finished is refused and the
/// directory stays, since the labelling is in it; every other error removes it.
pub async fn warned_score(
    labels: &Path,
    check: impl FnOnce(&WarnedFile) -> Result<(), RunError>,
    write: impl FnOnce(&warned::Scored) -> Result<(), RunError>,
) -> Result<warned::Scored, RunError> {
    let dir = work_dir(labels)?;
    let scored = read_and_score_warned(&dir, labels, check);
    settle(&dir, scored, write).await
}

fn read_and_score_warned(
    dir: &Path,
    labels: &Path,
    check: impl FnOnce(&WarnedFile) -> Result<(), RunError>,
) -> Result<warned::Scored, RunError> {
    let path = dir.join(WARNED_FILE);
    let file = warned::parse_warned(&std::fs::read_to_string(&path).map_err(io_error(&path))?)?;
    check(&file)?;
    let labels = warned::parse_labels(&std::fs::read_to_string(labels).map_err(io_error(labels))?)?;
    Ok(warned::score(&file, &labels)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    use crate::chains::header_id;
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
        let input = ChainsInput { set: &set, distractors: &distractors(), embedder: &embedder, tokenizer_dir: &scratch, counter: &EvalTokenCounter::fake(), background: None };

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
        let input = ChainsInput { set: &set, distractors: &distractors(), embedder: &embedder, tokenizer_dir: &scratch, counter: &EvalTokenCounter::fake(), background: None };

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
        let bare = arm("keyed_bare");
        assert_eq!((bare.saves, bare.live_versions, bare.chains_with_one_live_version), (5, 2, 2));
        assert!(bare.old_vectors_dropped > 0, "the older versions' own rows lose their vectors");
        assert!([&no_key, &keyed, &control].iter().all(|a| a.old_vectors_dropped == 0));
        let unindexed = arm("keyed_unindexed");
        assert_eq!((unindexed.saves, unindexed.live_versions, unindexed.chains_with_one_live_version), (5, 2, 2));
        assert_eq!(unindexed.old_vectors_dropped, bare.old_vectors_dropped);
        assert_eq!(unindexed.old_texts_dropped, bare.old_vectors_dropped, "the same rows leave the text index");
        assert!([&no_key, &keyed, &bare, &control].iter().all(|a| a.old_texts_dropped == 0));
        let m1 = &report.m1.all;
        assert_eq!(m1.transitions, 3);
        assert_eq!(m1.escalated.k + m1.pending.k + m1.no_mark.k, 3);
        assert_eq!(m1.no_mark_reasons.below_floor.k, 0, "a row two versions share is never compared with itself");

        for arm in ["no_key", "keyed", "keyed_bare", "keyed_unindexed", "control"] {
            for chain in &set.chains {
                let shown = &report.positions[arm][&chain.id];
                assert_eq!((shown.search.len(), shown.context.len()), (chain.versions.len(), chain.versions.len()));
            }
        }
        for arm in ["keyed", "keyed_bare", "keyed_unindexed", "control"] {
            let shown = &report.positions[arm]["a"];
            assert!(shown.search[2].is_some() && shown.context[2].is_some(), "{arm}: the last version shows: {shown:?}");
            assert_eq!((shown.search[..2].to_vec(), shown.context[..2].to_vec()), (vec![None, None], vec![None, None]));
        }
        assert!(report.m2.one_live_version.met && report.m2.no_old_version_shown.met, "{:?}", report.m2);
        assert!(report.m2.last_where_reference.met, "the keyed arm places its last versions as the unindexed arm: {:?}", report.m2);
        let m2_control = &report.m2_control;
        assert!(m2_control.one_live_version.met && m2_control.no_old_version_shown.met, "{m2_control:?}");
        let m4 = &report.m4;
        assert_eq!((m4.search.outputs, m4.context.outputs), (2, 2));
        assert!(m4.search.total_byte_delta > 0, "the keyed arm's search shows the keys: {m4:?}");
        assert!(m4.context.total_byte_delta > 0, "the rewritten prompts carry the keys: {m4:?}");
        assert!(m4.context_all_keyed.total_byte_delta >= m4.context.total_byte_delta);
        assert!(m4.fixed.schema_description_tokens > 0 && m4.fixed.protocol_paragraph_tokens > 0 && m4.warning_line_tokens > 0);
    }

    /// The prompts the keyed arm's router returned, and the key of each row.
    async fn keyed_prompts() -> (Vec<String>, HashMap<i64, String>) {
        let set = tiny_set();
        let scratch = scratch_dir();
        let embedder = EvalEmbedder::fake();
        let input = ChainsInput { set: &set, distractors: &distractors(), embedder: &embedder, tokenizer_dir: &scratch, counter: &EvalTokenCounter::fake(), background: None };
        let run = run_arm(&input, Arm::Keyed, &scratch.join(Arm::Keyed.as_str())).await;
        remove_scratch(&scratch).await;
        let run = run.expect("the keyed arm must run");
        let keys = run.listed.values().filter_map(|row| row.topic_key.clone().map(|key| (row.id, key))).collect();
        (run.queries.into_iter().map(|q| q.prompt).collect(), keys)
    }

    /// Fails when the product changes the header it prints for a hit: the
    /// rewrite would then find none and M4 would measure nothing.
    #[tokio::test]
    async fn the_rewrite_finds_every_keyed_header_of_a_real_prompt() {
        let (prompts, keys) = keyed_prompts().await;
        let mut keyed_headers = 0;
        for prompt in &prompts {
            let ids = context_ids(prompt);
            let expected = ids.iter().filter(|id| keys.contains_key(id)).count();
            let rewritten = cost::rewrite_prompt(prompt, &keys, None);
            assert_eq!((rewritten.headers, rewritten.keyed), (ids.len(), expected), "{prompt}");
            for line in rewritten.prompt.lines() {
                if let Some(id) = header_id(line) {
                    assert_eq!(line.contains(" topic="), keys.contains_key(&id), "{line}");
                }
            }
            keyed_headers += expected;
        }
        assert!(keyed_headers > 0, "no prompt held a keyed hit: {prompts:?}");
    }

    #[tokio::test]
    async fn the_production_context_still_omits_the_key() {
        let (prompts, _) = keyed_prompts().await;
        let mut headers = 0;
        for line in prompts.iter().flat_map(|p| p.lines()).filter(|l| header_id(l).is_some()) {
            assert!(!line.contains(" topic="), "{line}");
            headers += 1;
        }
        assert!(headers > 0, "no prompt held a hit: {prompts:?}");
    }

    #[tokio::test]
    async fn a_scratch_dir_outside_the_temp_dir_is_refused_before_anything_is_created() {
        let set = tiny_set();
        let embedder = EvalEmbedder::fake();
        let outside = std::env::temp_dir()
            .parent()
            .expect("the temp dir has a parent")
            .join(format!("revision-eval-must-not-be-created-{}", std::process::id()));
        let input = ChainsInput { set: &set, distractors: &distractors(), embedder: &embedder, tokenizer_dir: &outside, counter: &EvalTokenCounter::fake(), background: None };

        match run(&input, &outside).await {
            Err(RunError::OutsideTemp(path, _)) => assert_eq!(path, outside),
            other => panic!("expected OutsideTemp, got {other:?}"),
        }
        assert!(!outside.exists(), "a refused run must not leave a directory behind");
    }

    use crate::store::fixtures::{small_export, work_with};

    async fn fake_replay(export: &Path, seed: u64) -> Result<ReplaySummary, RunError> {
        let embedder = EvalEmbedder::fake();
        let counter = EvalTokenCounter::fake();
        let no_tokenizer = scratch_dir();
        store_replay(export, &ReplayInput { embedder: &embedder, tokenizer_dir: &no_tokenizer, counter: &counter, seed, fake: true }).await
    }

    #[tokio::test]
    async fn a_fake_store_run_counts_and_labels_a_small_export() {
        let (dir, export) = work_with(&small_export());

        let summary = fake_replay(&export, store::DEFAULT_SEED).await.expect("the replay must succeed");

        let (e, r) = (&summary.export, &summary.replay);
        assert_eq!((e.rows, e.saves, e.without_agent, e.keyed_saves), (6, 6, 1, 1));
        assert_eq!((r.sent, r.keyed, r.unkeyed, r.reinforced), (5, 1, 4, 1), "the repeated sentence only reinforces");
        assert_eq!((r.fires, r.population, r.rest), (0, 2, 2), "the double places nothing near");
        assert_eq!((summary.labelled_firing, summary.labelled_sample), (0, 2));

        let labels = std::fs::read_to_string(dir.join(LABELS_FILE)).unwrap();
        let lines = store::parse_labels(&labels).unwrap();
        let replay = store::parse_replay(&std::fs::read_to_string(dir.join(REPLAY_FILE)).unwrap()).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.proposed.is_none() && l.label.is_none()));
        assert_eq!(replay.labelled.len(), 2);
        assert!(replay.fake && replay.warning_line_tokens > 0 && replay.model_contract == EvalEmbedder::fake().model_id());
        assert!(!dir.join(REPLAY_STORE).exists(), "the replay store is removed at the end of the first step");
        assert!(export.exists());
        remove_scratch(&dir).await;
    }

    #[tokio::test]
    async fn a_fake_chains_run_stores_the_background_in_every_arm_and_removes_its_export() {
        let set = tiny_set();
        let scratch = scratch_dir();
        let embedder = EvalEmbedder::fake();
        let input = ChainsInput { set: &set, distractors: &distractors(), embedder: &embedder, tokenizer_dir: &scratch, counter: &EvalTokenCounter::fake(), background: None };
        let rows = small_export();
        let (dir, export) = work_with(&rows);

        let report = run_with_background(&input, &scratch, &export).await.expect("the run must succeed");

        assert!(!dir.exists() && !scratch.exists(), "the export's directory and the scratch one are removed");
        let background = report.background.as_ref().expect("the report says what the background held");
        assert_eq!((background.saves, background.without_agent, background.gone), (5, 1, 0));
        assert!(report.arms.values().all(|arm| arm.background_saves == 5), "{:?}", report.arms);
        assert!(report.m2.one_live_version.met, "a background key never revises a chain: {:?}", report.m2);
        let json = serde_json::to_string(&report).unwrap();
        assert!(rows.iter().all(|r| !json.contains(&r.content)), "no background text reaches the result");
        assert!(!json.contains("cache/layer"), "nor a background key");
    }

    #[tokio::test]
    async fn a_background_outside_a_work_dir_is_refused_and_left_alone() {
        let set = tiny_set();
        let scratch = scratch_dir();
        let embedder = EvalEmbedder::fake();
        let input = ChainsInput { set: &set, distractors: &distractors(), embedder: &embedder, tokenizer_dir: &scratch, counter: &EvalTokenCounter::fake(), background: None };
        let dir = std::env::temp_dir().join(format!("not-a-work-dir-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let export = dir.join("export.jsonl");
        std::fs::write(&export, "not read").unwrap();

        let error = run_with_background(&input, &scratch, &export).await.unwrap_err();

        assert!(matches!(error, RunError::Refused(_)), "{error}");
        assert!(export.exists() && !scratch.exists(), "nothing is removed and no store is made");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn the_working_files_are_removed_even_when_the_run_fails() {
        // The first step: an export of a version nobody reads.
        let (dir, export) = work_with(&small_export());
        let text = std::fs::read_to_string(&export).unwrap().replacen("\"version\":1", "\"version\":2", 1);
        std::fs::write(&export, text).unwrap();
        let error = fake_replay(&export, 1).await.unwrap_err();
        assert!(matches!(error, RunError::Store(StoreError::Format(_))), "{error}");
        assert!(error.is_bad_input());
        assert!(!dir.exists(), "a failed first step removes the directory with the export in it");

        // The second step, on a replay file that cannot be read.
        let (dir, export) = work_with(&small_export());
        fake_replay(&export, 1).await.unwrap();
        std::fs::write(dir.join(REPLAY_FILE), "{").unwrap();
        let error = store_score(&dir.join(LABELS_FILE), |_| Ok(()), |_| Ok(())).await.unwrap_err();
        assert!(matches!(error, RunError::Store(StoreError::Replay(_))), "{error}");
        assert!(!dir.exists());

        // The second step, when the result cannot be written.
        let (dir, export) = work_with(&small_export());
        fake_replay(&export, 1).await.unwrap();
        let labels = std::fs::read_to_string(dir.join(LABELS_FILE)).unwrap().replace("\"label\":null", "\"label\":\"other\"");
        std::fs::write(dir.join(LABELS_FILE), labels).unwrap();
        let refused = store_score(&dir.join(LABELS_FILE), |_| Ok(()), |_| Err(RunError::Refused("no".into()))).await;
        assert!(refused.is_err());
        assert!(!dir.exists(), "the directory goes whether or not the result is written");
    }

    #[tokio::test]
    async fn a_labels_file_the_owner_has_not_finished_keeps_the_directory() {
        let (dir, export) = work_with(&small_export());
        fake_replay(&export, 1).await.unwrap();

        let error = store_score(&dir.join(LABELS_FILE), |_| Ok(()), |_| Ok(())).await.unwrap_err();

        assert!(matches!(error, RunError::Store(StoreError::EmptyLabel(1))), "{error}");
        assert!(dir.join(LABELS_FILE).exists() && dir.join(REPLAY_FILE).exists());
        let refused = store_score(&dir.join(LABELS_FILE), |_| Err(RunError::Refused("taken".into())), |_| Ok(())).await;
        assert!(matches!(refused, Err(RunError::Refused(_))));
        assert!(dir.exists(), "a refusal before scoring leaves the labelling alone");
        remove_scratch(&dir).await;
    }

    #[tokio::test]
    async fn a_working_directory_outside_the_temp_dir_is_refused_and_left_alone() {
        // The build directory, not the temp dir's parent: on Linux that is `/`,
        // where a test cannot create anything.
        let build = std::env::current_exe().unwrap().parent().expect("the test binary sits in a directory").canonicalize().unwrap();
        assert!(!build.starts_with(std::env::temp_dir().canonicalize().unwrap()), "{} is inside the temp dir", build.display());
        let outside = build.join(format!("revision-eval-outside-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&outside).unwrap();
        let export_path = outside.join("export.jsonl");
        std::fs::write(&export_path, "kept").unwrap();
        let (inside, _) = work_with(&[]);
        let loose = inside.parent().unwrap().join(format!("loose-{}.jsonl", uuid::Uuid::new_v4()));
        std::fs::write(&loose, "kept").unwrap();

        let a = fake_replay(&export_path, 1).await.unwrap_err();
        let b = fake_replay(&loose, 1).await.unwrap_err();

        assert!(matches!(a, RunError::OutsideTemp(..)), "{a}");
        assert!(matches!(b, RunError::Refused(_)), "{b}");
        assert!(export_path.exists() && loose.exists() && inside.exists());
        std::fs::remove_dir_all(&outside).unwrap();
        std::fs::remove_file(&loose).unwrap();
        remove_scratch(&inside).await;
    }

    use crate::store::fixtures::{at, header_filtered, warned_lines, work_with_lines};

    async fn fake_warned(export: &Path, since: i64, seed: u64) -> Result<WarnedSummary, RunError> {
        let embedder = EvalEmbedder::fake();
        let counter = EvalTokenCounter::fake();
        let no_tokenizer = scratch_dir();
        warned_replay(export, since, &ReplayInput { embedder: &embedder, tokenizer_dir: &no_tokenizer, counter: &counter, seed, fake: true }).await
    }

    #[tokio::test]
    async fn a_fake_warned_run_counts_and_labels_the_marked_pairs_and_the_rest_of_a_small_export() {
        let (dir, export) = work_with_lines(&warned_lines());

        let summary = fake_warned(&export, at(10), warned::DEFAULT_SEED).await.expect("the replay must succeed");

        let c = &summary.counts;
        assert_eq!((c.rows, c.saves, c.window_saves, c.window_saves_with_agent), (8, 8, 6, 5));
        assert_eq!((c.marked, c.rest, c.rest_chosen, c.rest_unpaired), (2, 3, 3, 0));
        assert_eq!((summary.labelled_marked, summary.labelled_rest, summary.window_start_ms), (2, 3, at(10)));

        let labels = warned::parse_labels(&std::fs::read_to_string(dir.join(LABELS_FILE)).unwrap()).unwrap();
        let file = warned::parse_warned(&std::fs::read_to_string(dir.join(WARNED_FILE)).unwrap()).unwrap();
        assert_eq!((labels.len(), file.pairs.len()), (5, 5));
        assert!(labels.iter().all(|l| l.a.is_none() && l.b.is_none() && l.c.is_none()));
        assert!(file.fake && file.warning_line_tokens > 0 && file.model_contract == EvalEmbedder::fake().model_id());
        assert_eq!((file.window_start_ms, file.store_sizes.len()), (at(10), 5));
        assert_eq!(file.pairs.iter().filter(|p| p.marked).count(), 2);
        let pair_of = |later: &str| {
            let line = labels.iter().find(|l| l.later.starts_with(later)).expect("the pair is labelled");
            (line.earlier.clone(), file.pairs[line.n - 1].clone())
        };
        let (earlier, thursdays) = pair_of("Deploys now go out on Thursdays");
        assert_eq!(earlier, "Deploys go out on Tuesdays after the freeze.");
        assert!(thursdays.marked && thursdays.actions == Some(warned::Actions::default()));
        let (earlier, fourteen) = pair_of("The nightly backup keeps the last fourteen");
        assert_eq!(earlier, "The nightly backup keeps the last seven snapshots.");
        assert!(fourteen.marked && fourteen.actions.is_some_and(|a| a.followed && !a.hid && !a.reused));
        assert!(!dir.join(REPLAY_STORE).exists(), "the replay store is removed at the end of the first step");
        assert!(export.exists());
        remove_scratch(&dir).await;
    }

    #[tokio::test]
    async fn the_same_seed_labels_the_same_pairs_in_the_same_order() {
        let (dir_a, export_a) = work_with_lines(&warned_lines());
        let (dir_b, export_b) = work_with_lines(&warned_lines());
        fake_warned(&export_a, at(10), 3).await.unwrap();
        fake_warned(&export_b, at(10), 3).await.unwrap();

        let read = |dir: &Path| std::fs::read_to_string(dir.join(LABELS_FILE)).unwrap();

        assert_eq!(read(&dir_a), read(&dir_b));
        remove_scratch(&dir_a).await;
        remove_scratch(&dir_b).await;
    }

    #[tokio::test]
    async fn an_export_the_first_step_refuses_is_removed_with_its_directory() {
        let lines = warned_lines();

        // Made without the deleted rows.
        let (dir, export) = work_with_lines(&lines);
        let text = std::fs::read_to_string(&export).unwrap().replacen("\"include_deleted\":true", "\"include_deleted\":false", 1);
        std::fs::write(&export, text).unwrap();
        let error = fake_warned(&export, at(10), 1).await.unwrap_err();
        assert!(matches!(error, RunError::Warned(WarnedError::WithoutDeleted)), "{error}");
        assert!(error.is_bad_input());
        assert!(!dir.exists(), "a failed first step removes the directory with the export in it");

        // Limited to a project.
        let (dir, export) = work_with_lines(&lines);
        let filtered = crate::store::fixtures::export_with(header_filtered(1, serde_json::json!({"include_deleted": true, "project": "repo-1"})), &lines);
        std::fs::write(&export, filtered).unwrap();
        let error = fake_warned(&export, at(10), 1).await.unwrap_err();
        assert!(matches!(error, RunError::Warned(WarnedError::Filtered(_))), "{error}");
        assert!(!dir.exists());

        // Outside a work directory: refused, and left alone.
        let (inside, _) = work_with_lines(&[]);
        let loose = inside.parent().unwrap().join(format!("loose-{}.jsonl", uuid::Uuid::new_v4()));
        std::fs::write(&loose, "kept").unwrap();
        let error = fake_warned(&loose, at(10), 1).await.unwrap_err();
        assert!(matches!(error, RunError::Refused(_)), "{error}");
        assert!(loose.exists() && inside.exists());
        std::fs::remove_file(&loose).unwrap();
        remove_scratch(&inside).await;
    }

    /// Fills every pass of the labels file with `label`.
    fn fill(dir: &Path, label: &str) {
        let path = dir.join(LABELS_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        let filled = ["a", "b"].iter().fold(text, |text, pass| text.replace(&format!("\"{pass}\":null"), &format!("\"{pass}\":\"{label}\"")));
        std::fs::write(path, filled).unwrap();
    }

    #[tokio::test]
    async fn the_second_step_scores_the_labels_and_removes_the_directory() {
        let (dir, export) = work_with_lines(&warned_lines());
        fake_warned(&export, at(10), 1).await.unwrap();
        fill(&dir, "revision");
        let mut seen: Option<bool> = None;

        let scored = warned_score(
            &dir.join(LABELS_FILE),
            |file: &WarnedFile| {
                seen = Some(file.fake);
                Ok(())
            },
            |_| Ok(()),
        )
        .await
        .expect("the labels score");

        assert_eq!(seen, Some(true));
        assert_eq!((scored.labelled.marked, scored.labelled.rest), (2, 3));
        assert_eq!((scored.precision.k, scored.precision.n), (2, 2));
        assert!(!dir.exists(), "the directory goes once the result is out");
    }

    #[tokio::test]
    async fn a_labels_file_the_owner_has_not_finished_keeps_the_directory_of_the_second_step() {
        let (dir, export) = work_with_lines(&warned_lines());
        fake_warned(&export, at(10), 1).await.unwrap();

        let error = warned_score(&dir.join(LABELS_FILE), |_| Ok(()), |_| Ok(())).await.unwrap_err();

        assert!(matches!(error, RunError::Warned(WarnedError::EmptyLabel { n: 1, field: "a" })), "{error}");
        assert!(error.is_bad_input());
        assert!(dir.join(LABELS_FILE).exists() && dir.join(WARNED_FILE).exists());
        let refused = warned_score(&dir.join(LABELS_FILE), |_| Err(RunError::Refused("taken".into())), |_| Ok(())).await;
        assert!(matches!(refused, Err(RunError::Refused(_))));
        assert!(dir.exists(), "a refusal before scoring leaves the labelling alone");
        remove_scratch(&dir).await;
    }

    #[tokio::test]
    async fn the_second_step_removes_the_directory_when_the_replay_file_or_the_result_fails() {
        let (dir, export) = work_with_lines(&warned_lines());
        fake_warned(&export, at(10), 1).await.unwrap();
        std::fs::write(dir.join(WARNED_FILE), "{").unwrap();
        let error = warned_score(&dir.join(LABELS_FILE), |_| Ok(()), |_| Ok(())).await.unwrap_err();
        assert!(matches!(error, RunError::Warned(WarnedError::Replay(_))), "{error}");
        assert!(!error.is_bad_input(), "the owner did not write that file");
        assert!(!dir.exists());

        let (dir, export) = work_with_lines(&warned_lines());
        fake_warned(&export, at(10), 1).await.unwrap();
        fill(&dir, "other");
        let refused = warned_score(&dir.join(LABELS_FILE), |_| Ok(()), |_| Err(RunError::Refused("no".into()))).await;
        assert!(refused.is_err());
        assert!(!dir.exists(), "the directory goes whether or not the result is written");
    }

    #[test]
    fn the_repository_specs_give_at_least_two_hundred_distractors() {
        let specs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../openspec/specs");
        let distractors = read_distractors(&specs).unwrap();
        assert!(distractors.len() >= 200, "{}", distractors.len());
    }
}
