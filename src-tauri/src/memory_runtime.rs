//! Tauri-facing spawn wrappers for the `clawg_core::memory` background jobs.
//!
//! Every function here is the thin `AppHandle` half of the
//! "AppHandle-wrapper / testable-core split" convention used throughout
//! `clawg_core::memory`: it resolves dependencies out of [`AppState`] and then
//! delegates to a pure core function that takes explicit arguments. The core
//! halves live in `clawg-core` (which does not depend on `tauri` at all) and
//! carry the unit tests; the wrappers below carry only the compile-time
//! `Send + 'static` proofs required by `tauri::async_runtime::spawn`.
//!
//! Names and signatures are unchanged from when these lived inside
//! `memory/{worker,extractor,miner,maintenance,resolver}.rs`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use once_cell::sync::OnceCell;
use tokio::sync::mpsc::UnboundedReceiver;

use clawg_core::memory::chunker::{ChatChunker, CodeChunker};
use clawg_core::memory::embedder::Embedder;
use clawg_core::memory::events::MemoryEvent;
use clawg_core::memory::extractor::{run_extraction_core, EXTRACTOR_TIMEOUT};
use clawg_core::memory::maintenance::{
    run_maintenance_loop, MAINTENANCE_PERIOD, STARTUP_DELAY,
};
use clawg_core::memory::miner::{mine_session_core, NAMER_TIMEOUT};
use clawg_core::memory::resolver::{
    adjudicate_cluster, apply_resolution, load_disputed_clusters, RESOLVER_TIMEOUT,
};
use clawg_core::memory::worker::{memory_worker, resolve_tokenizer_and_chunker};
use clawg_core::memory::MemoryStore;
use clawg_core::models::config::{aux_job_enabled, AuxJob};
use clawg_core::models::ChatMessage;
use clawg_core::providers::AiProvider;


// ── worker ────────────────────────────────────────────────────────────────────

/// Spawn entry point — called from `lib.rs::run()` setup hook.

///

/// Builds the two chunkers once, then hands ownership to the worker future.

/// Returns immediately; the spawned task lives for the app lifetime. This is

/// the ONLY call site that spawns `memory_worker` — the single-sequential-

/// consumer guarantee the dedup gate and the absorbed-duplicates counter both

/// depend on (see `MemoryStore::reinforce_if_duplicate` in `store_insert.rs`)

/// holds only as long as that stays true. A second spawn site would race it.

pub(crate) fn start_memory_worker(

    app_handle: tauri::AppHandle,

    memory_rx: UnboundedReceiver<MemoryEvent>,

) {

    use tauri::Manager;

    let state = app_handle.state::<crate::state::AppState>();

    let store = state.memory_store.clone();

    let embedder_cell = state.embedder.clone();

    let base_dir = state.base_dir.clone();

    let tokenizer_cell = state.tokenizer.clone();



    tauri::async_runtime::spawn(async move {

        let chat_chunker = resolve_tokenizer_and_chunker(&base_dir, &tokenizer_cell);

        let code_chunker = Arc::new(CodeChunker::new());

        memory_worker(memory_rx, store, embedder_cell, chat_chunker, code_chunker).await;

    });

}

// Compile-time Send proof — every captured value MUST be Send + 'static so

// tauri::async_runtime::spawn accepts the future.

const _: fn() = || {

    fn _send<T: Send>() {}

    _send::<UnboundedReceiver<MemoryEvent>>();

    _send::<Arc<MemoryStore>>();

    _send::<Arc<OnceCell<Arc<dyn Embedder>>>>();

    _send::<Arc<OnceCell<Arc<tokenizers::Tokenizer>>>>();

    _send::<Arc<ChatChunker>>();

    _send::<Arc<CodeChunker>>();

};

// ── extractor ───────────────────────────────────────────────────────────

/// Fire-and-forget entry point spawned from `commands/session.rs` on session

/// close (`new_session` / `resume_session`), mirroring `resolver::run_resolver`.

///

/// Resolves dependencies from `AppState` — embedder cell, memory store, and a

/// snapshot of `agent_id -> (provider_id, project_path)` with the `MutexGuard`

/// dropped before any `.await` — then delegates to [`run_extraction_core`].

pub(crate) async fn run_extractor(app: tauri::AppHandle, history: Vec<(String, Vec<ChatMessage>)>) {

    use tauri::Manager;



    if history.iter().all(|(_, m)| m.is_empty()) {

        return;

    }



    let state = app.state::<crate::state::AppState>();



    // Read the flag before any lock, query or LLM call. `info`, not `debug`:

    // a user who disabled this and later wonders why memories stopped

    // appearing needs the trace at the default level. A poisoned lock keeps

    // the job running — the flag exists to save cost, not to gate correctness.

    let enabled = state.config.read()

        .map(|c| aux_job_enabled(&c, AuxJob::Extractor))

        .unwrap_or(true);

    if !enabled {

        log::info!("extractor disabled by config; skipping session close");

        return;

    }



    // Check embedder BEFORE any further work — saves the agents lock + LLM call.

    let Some(embedder) = state.embedder.get().cloned() else {

        log::debug!("extractor: embedder not ready; skipping extraction for this session");

        log::info!("aux_llm job=extractor calls=0");

        return;

    };



    let store = state.memory_store.clone();



    // Snapshot agent_id -> (provider_id, project_path); DROP the MutexGuard

    // before any `.await` (same discipline as resolver::run_resolver).

    let agent_pid: HashMap<String, (String, Option<String>)> = {

        match state.agents.lock() {

            Ok(agents) => agents

                .iter()

                .map(|a| (a.id.clone(), (a.provider_id.clone(), a.project_path.clone())))

                .collect(),

            Err(e) => {

                log::warn!("run_extractor: could not lock agents: {e}");

                HashMap::new()

            }

        }

    }; // MutexGuard dropped here — safe to .await below.



    let mut project_by_agent: HashMap<String, Option<String>> = HashMap::new();

    let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();

    for (agent_id, _) in &history {

        let (provider, project_path) = match agent_pid.get(agent_id) {

            Some((pid, path)) => (state.get_agent_provider(pid), path.clone()),

            None => (state.get_provider(), None),

        };

        project_by_agent.insert(agent_id.clone(), project_path);

        provider_by_agent.insert(agent_id.clone(), provider);

    }



    let calls = run_extraction_core(

        history,

        &store,

        Some(embedder),

        &project_by_agent,

        &provider_by_agent,

        EXTRACTOR_TIMEOUT,

    )

    .await;

    log::info!("aux_llm job=extractor calls={calls}");

}

// Compile-time proof: every type captured by the future passed to

// `tauri::async_runtime::spawn(run_extractor(app, history))` must be `Send + 'static`.

const _: fn() = || {

    fn s<T: Send + 'static>() {}

    s::<tauri::AppHandle>();

    s::<Arc<MemoryStore>>();

    s::<Vec<(String, Vec<ChatMessage>)>>();

};

// ── miner ───────────────────────────────────────────────────────────────

/// Fire-and-forget entry point spawned from `commands/session.rs` on session
/// close (`new_session` / `resume_session`), replacing
/// `episodic_writer::run_episodic_writer` — the ONLY writer of
/// `episodic_trajectory` per session (build + INSERT), and additionally mines
/// Golden candidates. Mirrors `extractor::run_extractor` (needs providers,
/// unlike `run_episodic_writer`'s plain wrapper).
///
/// Resolves dependencies from `AppState` — embedder cell, memory store, and a
/// snapshot of `agent_id -> (provider_id, project_path)` with the
/// `MutexGuard` dropped before any `.await` — then delegates to
/// [`mine_session_core`], which persists recurring candidates via
/// `promote_to_procedural_skill`.
pub(crate) async fn run_miner(
    app: tauri::AppHandle,
    session_id: String,
    history: Vec<(String, Vec<ChatMessage>)>,
) {
    use tauri::Manager;

    if history.iter().all(|(_, m)| m.is_empty()) {
        return;
    }

    let state = app.state::<crate::state::AppState>();

    // Read the flag before any lock, query or LLM call. `info`, not `debug`:
    // a user who disabled this and later wonders why memories stopped
    // appearing needs the trace at the default level. A poisoned lock keeps
    // the job running — the flag exists to save cost, not to gate correctness.
    let enabled = state.config.read()
        .map(|c| aux_job_enabled(&c, AuxJob::Miner))
        .unwrap_or(true);
    if !enabled {
        log::info!("miner disabled by config; skipping session close");
        return;
    }

    // Check embedder BEFORE any further work — saves the agents lock entirely
    // (same branch as the eliminated run_episodic_writer wrapper).
    let Some(embedder) = state.embedder.get().cloned() else {
        log::debug!("miner: embedder not ready; skipping mining for session={session_id}");
        return;
    };

    let store = state.memory_store.clone();

    // Snapshot agent_id -> (provider_id, project_path); DROP the MutexGuard
    // before any `.await` (same discipline as resolver::run_resolver /
    // extractor::run_extractor).
    let agent_pid: HashMap<String, (String, Option<String>)> = {
        match state.agents.lock() {
            Ok(agents) => agents
                .iter()
                .map(|a| (a.id.clone(), (a.provider_id.clone(), a.project_path.clone())))
                .collect(),
            Err(e) => {
                log::warn!("run_miner: could not lock agents: {e}");
                HashMap::new()
            }
        }
    }; // MutexGuard dropped here — safe to .await below.

    let mut project_by_agent: HashMap<String, Option<String>> = HashMap::new();
    let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
    for (agent_id, _) in &history {
        let (provider, project_path) = match agent_pid.get(agent_id) {
            Some((pid, path)) => (state.get_agent_provider(pid), path.clone()),
            None => (state.get_provider(), None),
        };
        project_by_agent.insert(agent_id.clone(), project_path);
        provider_by_agent.insert(agent_id.clone(), provider);
    }

    let now_ms = clawg_core::models::chat::current_time_millis();

    // Candidates are persisted inside the core via promote_to_procedural_skill;
    // the return value here is discarded since the core already logs the outcome.
    let _candidates = mine_session_core(
        session_id,
        history,
        &store,
        Some(embedder),
        &project_by_agent,
        &provider_by_agent,
        NAMER_TIMEOUT,
        now_ms,
    )
    .await;
}

// Compile-time proof: every type captured by the future passed to
// `tauri::async_runtime::spawn(run_miner(app, session_id, history))` must be
// `Send + 'static` — replaces the Send-proof of the eliminated
// `run_episodic_writer` wrapper.
const _: fn() = || {
    fn s<T: Send + 'static>() {}
    s::<tauri::AppHandle>();
    s::<String>();
    s::<Arc<MemoryStore>>();
    s::<Vec<(String, Vec<ChatMessage>)>>();
};

// ── maintenance ─────────────────────────────────────────────────────────

/// Spawn entry point — called from `lib.rs::run()` setup hook.
///
/// AppHandle-thin wrapper: resolves `Arc<MemoryStore>` from `AppState` and
/// hands off to the testable core [`run_maintenance_loop`]. Mirrors
/// `worker::start_memory_worker` — 5th instance of the wrapper/core
/// convention (worker → resolver → extractor → episodic_writer →
/// maintenance). Not directly testable (no `tauri` "test" feature in this
/// codebase); see the compile-time `Send + 'static` proof below instead.
pub(crate) fn start_maintenance_worker(app_handle: tauri::AppHandle) {
    use tauri::Manager;
    let state = app_handle.state::<crate::state::AppState>();
    let store = state.memory_store.clone();

    tauri::async_runtime::spawn(run_maintenance_loop(store, STARTUP_DELAY, MAINTENANCE_PERIOD));
}

// Compile-time Send proof — every captured value MUST be Send + 'static so
// tauri::async_runtime::spawn accepts the future (mirrors worker.rs:165 /
// episodic_writer.rs:307 — 5th instance of this guard).
const _: fn() = || {
    fn s<T: Send + 'static>() {}
    s::<tauri::AppHandle>();
    s::<Arc<MemoryStore>>();
    s::<Duration>();
};

// ── resolver ────────────────────────────────────────────────────────────

/// Fire-and-forget entry point spawned from `commands/session.rs` on each
/// new or resumed session.
///
/// # Design
///
/// 1. Clone the `MemoryStore` handle (cheap — it is a channel clone).
/// 2. Load all pending disputed clusters.
/// 3. Snapshot `agent_id → provider_id` map from `state.agents` and **drop
///    the `MutexGuard` before any `.await`** to satisfy Rust's
///    `Send + 'static` requirement for `tauri::async_runtime::spawn`.
/// 4. Per cluster: resolve provider via `state.get_agent_provider(pid)`
///    (already falls back to global provider when pid is unknown) then
///    adjudicate + apply. Errors are `log::warn!`'d — never propagated.
pub(crate) async fn run_resolver(app: tauri::AppHandle) {
    use tauri::Manager;
    use std::collections::HashMap;

    let state = app.state::<crate::state::AppState>();

    // Read the flag before any lock, query or LLM call. `info`, not `debug`:
    // a user who disabled this and later wonders why memories stopped
    // appearing needs the trace at the default level. A poisoned lock keeps
    // the job running — the flag exists to save cost, not to gate correctness.
    let enabled = state.config.read()
        .map(|c| aux_job_enabled(&c, AuxJob::Resolver))
        .unwrap_or(true);
    if !enabled {
        log::info!("resolver disabled by config; skipping session close");
        return;
    }

    let store = state.memory_store.clone();

    // Step 1: load pending clusters.
    let clusters = load_disputed_clusters(&store).await;
    if clusters.is_empty() {
        log::info!("aux_llm job=resolver calls=0");
        return;
    }

    // Step 2: snapshot agent→provider_id; DROP the MutexGuard before any await.
    let agent_pid: HashMap<String, String> = {
        match state.agents.lock() {
            Ok(agents) => agents
                .iter()
                .map(|a| (a.id.clone(), a.provider_id.clone()))
                .collect(),
            Err(e) => {
                log::warn!("run_resolver: could not lock agents: {e}");
                HashMap::new()
            }
        }
    }; // MutexGuard dropped here — safe to .await below.

    // Step 3: adjudicate + apply per cluster.
    let mut calls: u64 = 0;
    for cluster in clusters {
        let provider = match agent_pid.get(&cluster.agent_id) {
            Some(pid) => state.get_agent_provider(pid),
            None => state.get_provider(),
        };

        calls += 1;
        let resolution = adjudicate_cluster(&provider, &cluster, RESOLVER_TIMEOUT).await;

        if let Err(e) = apply_resolution(&store, &cluster, resolution).await {
            log::warn!(
                "run_resolver: apply_resolution failed (new_id={}): {e}",
                cluster.new_id
            );
        }
    }
    log::info!("aux_llm job=resolver calls={calls}");
}

// Compile-time proof: every type captured by the future passed to
// `tauri::async_runtime::spawn(run_resolver(app))` must be `Send + 'static`.
const _: fn() = || {
    fn s<T: Send + 'static>() {}
    s::<tauri::AppHandle>();
    s::<Arc<MemoryStore>>();
};

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-time Send + 'static proof for the value(s) captured by

    // the future passed to tauri::async_runtime::spawn(run_extractor(app, history)).

    #[test]

    fn run_extractor_captured_types_are_send_static() {

        fn s<T: Send + 'static>() {}

        s::<tauri::AppHandle>();

        s::<Arc<MemoryStore>>();

        s::<Vec<(String, Vec<ChatMessage>)>>();

    }

    // Compile-time Send + 'static proof for the value(s) captured
    // by the future passed to
    // tauri::async_runtime::spawn(run_maintenance_loop(store, delay, period)).
    #[test]
    fn start_maintenance_worker_captured_types_are_send_static() {
        fn s<T: Send + 'static>() {}
        s::<tauri::AppHandle>();
        s::<Arc<MemoryStore>>();
        s::<Duration>();
    }
}
