//! HTTP client for a running `clawg serve`.
//!
//! Used by `clawg mcp`, `clawg status` and `clawg stop`. It owns the only two
//! pieces of knowledge a client needs: where the daemon is (`daemon.json`) and
//! how to prove it may talk to it (`token`).
//!
//! # Autostart
//!
//! `clawg mcp` is registered with an MCP host that launches it on demand, and a
//! user who just opened their editor has no reason to have started a daemon
//! first. So [`ensure_running`] starts one when none answers. Spawning is
//! behind [`DaemonSpawner`] rather than called directly, because the real
//! implementation launches a process that downloads a 470 MB model — a test
//! substitutes an in-process daemon instead.
//!
//! Autostart is skipped, with [`ClientError::StoreHeld`], when the memory store
//! is owned by something that is not a daemon: such a daemon could never start,
//! and finding that out by polling costs the whole [`AUTOSTART_TIMEOUT`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use clawg_core::memory::store_lock::{StoreHolder, StoreLock, StoreOwner};
use clawg_core::paths::{data_dir, Profile};
use serde::{Deserialize, Serialize};

use crate::daemon::lifecycle::{read_state, read_token};
use crate::relevance::RELEVANCE_RATIO;

/// How often [`ensure_running`] re-checks a daemon it just asked for.
pub const AUTOSTART_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// How long [`ensure_running`] waits for a spawned daemon to answer `/health`.
///
/// Generous because a first run migrates the database and opens SQLite before
/// it binds — but it does NOT cover the model download: `/health` answers
/// `embedder: "loading"` long before the model exists, so this is a bound on
/// process startup, not on readiness.
pub const AUTOSTART_TIMEOUT: Duration = Duration::from_secs(60);
/// Timeout for a single liveness probe. Short on purpose: the question is
/// "is something listening on loopback?", which is answered in microseconds or
/// not at all.
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(1);

/// Everything that can go wrong talking to a daemon.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("no clawg daemon is running for this profile")]
    NotRunning,
    #[error("the daemon rejected the token; delete the token file and restart `clawg serve`")]
    Unauthorized,
    /// The daemon answered, and said no. `message` is its `{"error": …}` text.
    #[error("daemon returned {status}: {message}")]
    Daemon { status: u16, message: String },
    #[error("transport error talking to the daemon: {0}")]
    Http(String),
    /// `--here` needs the current directory to name the project, and the
    /// operating system would not give it.
    #[error("cannot read the current directory for --here: {0}")]
    WorkingDir(String),
    #[error("could not start a daemon: {0}")]
    Spawn(String),
    #[error("timed out after {0:?} waiting for the daemon to start")]
    StartTimeout(Duration),
    #[error("the daemon (pid {pid}) accepted the shutdown but is still running after {timeout:?}")]
    StopTimeout { pid: u32, timeout: Duration },
    /// The memory store is owned by something that is not a daemon (an
    /// evaluation run), so no daemon can start and waiting for one is pointless.
    /// Raised at once instead of after [`AUTOSTART_TIMEOUT`].
    #[error(
        "the memory store is held by {} (pid {}), not by a clawg daemon; close it and reconnect",
        .0.holder, .0.pid
    )]
    StoreHeld(StoreOwner),
}

// ── Wire types the client reads back ────────────────────────────────────────
//
// Deliberately separate from the daemon's `Serialize`-only response structs.
// Those mirror core types (`RankedHit`, `MemoryAdminRow`) that carry fields no
// client needs and that deliberately do NOT implement `Deserialize`; mirroring
// them here would force a derive onto core for the sake of a consumer.

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Health {
    pub status: String,
    pub version: String,
    pub profile: String,
    pub pid: u32,
    pub embedder: String,
    pub uptime_s: u64,
}

impl Health {
    /// Whether retrieval will work right now — i.e. whether `/v1/search` will
    /// answer instead of `503`.
    pub fn embedder_ready(&self) -> bool {
        self.embedder == "ready"
    }
}

/// One retrieval hit, as much of it as a caller formats.
#[derive(Debug, Clone, Deserialize)]
pub struct Hit {
    pub id: i64,
    /// `"Memory"` | `"Episodic"` | `"Procedural"` — the core serializes
    /// `TableKind` in PascalCase, and this type does not re-spell it.
    pub kind: String,
    pub content: String,
    pub memory_type: String,
    pub tier: i64,
    pub score: f64,
}

/// One inventory row from `GET /v1/memories`.
///
/// Every field past `access_count` is optional and defaulted: a daemon that
/// predates them omits the keys, and the MCP bridge must keep reading it.
/// Serialized under the API's own field names, which is what `--json` prints.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct MemoryRow {
    pub id: i64,
    /// `"memory"` | `"episodic"` | `"procedural"` (lowercase — `MemoryKind`,
    /// not `TableKind`; the two spellings are a known wart in core).
    pub kind: String,
    pub memory_type: Option<String>,
    /// The row's full content, not a preview.
    pub summary: String,
    pub tier: i64,
    pub importance: f64,
    pub status: String,
    pub access_count: i64,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub last_accessed_at: Option<i64>,
    #[serde(default)]
    pub superseded_by: Option<i64>,
    #[serde(default)]
    pub deleted_at: Option<i64>,
    #[serde(default)]
    pub success_count: Option<i64>,
    #[serde(default)]
    pub trigger: Option<String>,
    #[serde(default)]
    pub source_trajectory_ids: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub source_session_id: Option<String>,
    #[serde(default)]
    pub created_by_type: Option<String>,
    #[serde(default)]
    pub created_by_ref: Option<String>,
    #[serde(default)]
    pub last_modified_at: Option<i64>,
    #[serde(default)]
    pub update_count: Option<i64>,
    #[serde(default)]
    pub dispute_count: Option<i64>,
    /// Raw JSON text of the ids this row disputes.
    #[serde(default)]
    pub disputed_against: Option<String>,
    #[serde(default)]
    pub last_disputed_at: Option<i64>,
}

/// One bucket of a grouping from `GET /v1/stats`. `key: None` is the NULL
/// bucket (rows with no project, say).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Count {
    pub key: Option<String>,
    pub count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct StatsTotals {
    pub memory: u64,
    pub episodic: u64,
    pub procedural: u64,
    /// Soft-deleted rows across the three tables; not part of the live counts.
    pub deleted: u64,
}

/// The body of `GET /v1/stats`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct MemoryStats {
    pub totals: StatsTotals,
    pub by_type: Vec<Count>,
    pub by_tier: Vec<Count>,
    pub by_status: Vec<Count>,
    pub by_agent: Vec<Count>,
    pub by_project: Vec<Count>,
    pub newest_created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SaveArgs {
    pub content: String,
    pub agent_id: String,
    pub project_id: Option<String>,
    pub memory_type: Option<String>,
    pub importance: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchArgs {
    pub query: String,
    pub agent_id: String,
    pub project_id: Option<String>,
    pub limit: Option<usize>,
}

/// The body of `POST /v1/search`: the caller's arguments plus the bridge's
/// relevance cut.
///
/// The cut rides on every search so the daemon records as accessed only the
/// hits this client goes on to show; asking for the full list would count
/// rows no agent ever read.
#[derive(Serialize)]
struct SearchRequest<'a> {
    #[serde(flatten)]
    args: &'a SearchArgs,
    min_relative_score: f64,
}

fn search_request(args: &SearchArgs) -> SearchRequest<'_> {
    SearchRequest { args, min_relative_score: RELEVANCE_RATIO }
}

#[derive(Debug, Clone, Default)]
pub struct ListArgs {
    pub kind: Option<String>,
    pub tier: Option<i64>,
    pub memory_type: Option<String>,
    pub agent: Option<String>,
    pub project: Option<String>,
    pub status: Option<String>,
    /// Free text over memory content; the daemon then lists `memory` rows only.
    pub q: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

// ── The client ──────────────────────────────────────────────────────────────

/// A handle on one daemon: its address, its token, and a reusable HTTP client.
#[derive(Debug, Clone)]
pub struct DaemonClient {
    base_url: String,
    token: String,
    http: reqwest::Client,
    /// The data directory this client was resolved from, so callers can report
    /// it without re-deriving the profile.
    base_dir: PathBuf,
}

impl DaemonClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>, base_dir: PathBuf) -> Self {
        Self {
            base_url: base_url.into(),
            token: token.into(),
            http: reqwest::Client::new(),
            base_dir,
        }
    }

    /// Builds a client from `<base_dir>/daemon.json` + `<base_dir>/token`.
    ///
    /// `None` when either file is missing — which is the same answer as "no
    /// daemon", since a daemon that is up has written both.
    pub fn from_data_dir(base_dir: &Path) -> Option<Self> {
        let state = read_state(base_dir)?;
        let token = read_token(base_dir)?;
        Some(Self::new(
            // Always loopback, never the recorded host: the daemon binds
            // 127.0.0.1 and a state file that claimed otherwise would be a
            // tampered file pointing this client's bearer token elsewhere.
            format!("http://127.0.0.1:{}", state.port),
            token,
            base_dir.to_path_buf(),
        ))
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Turns a finished response into either its parsed body or the right
    /// [`ClientError`].
    ///
    /// One funnel for every call so the mapping cannot drift per endpoint: 401
    /// is always `Unauthorized` (a distinct, actionable failure — the token
    /// file is stale), and every other non-2xx keeps the daemon's own status
    /// and `{"error": …}` message rather than being flattened into a string.
    async fn read_response<T: serde::de::DeserializeOwned>(
        resp: reqwest::Response,
    ) -> Result<T, ClientError> {
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ClientError::Unauthorized);
        }
        if !status.is_success() {
            let message = resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned))
                // A body that is not the daemon's error shape (a proxy page, a
                // truncated response) still has to produce something a user can
                // act on.
                .unwrap_or_else(|| status.to_string());
            return Err(ClientError::Daemon { status: status.as_u16(), message });
        }
        resp.json::<T>().await.map_err(|e| ClientError::Http(e.to_string()))
    }

    /// A connection that was refused, reset or timed out means "no daemon" —
    /// the same answer as a missing state file. Anything else is a genuine
    /// transport fault worth reporting as itself.
    fn map_transport(e: reqwest::Error) -> ClientError {
        if e.is_connect() || e.is_timeout() {
            ClientError::NotRunning
        } else {
            ClientError::Http(e.to_string())
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    pub async fn health(&self) -> Result<Health, ClientError> {
        let resp = self
            .http
            .get(self.url("/health"))
            .timeout(HEALTH_TIMEOUT)
            .send()
            .await
            .map_err(Self::map_transport)?;
        Self::read_response(resp).await
    }

    /// Queues a memory. Returns as soon as the daemon has accepted it — the row
    /// does not exist yet (see `daemon::api::save_memory`).
    pub async fn save(&self, args: SaveArgs) -> Result<(), ClientError> {
        let resp = self
            .http
            .post(self.url("/v1/memories"))
            .bearer_auth(&self.token)
            .json(&args)
            .send()
            .await
            .map_err(Self::map_transport)?;
        let _: serde_json::Value = Self::read_response(resp).await?;
        Ok(())
    }

    pub async fn search(&self, args: SearchArgs) -> Result<Vec<Hit>, ClientError> {
        #[derive(Deserialize)]
        struct Body {
            hits: Vec<Hit>,
        }
        let resp = self
            .http
            .post(self.url("/v1/search"))
            .bearer_auth(&self.token)
            .json(&search_request(&args))
            .send()
            .await
            .map_err(Self::map_transport)?;
        Ok(Self::read_response::<Body>(resp).await?.hits)
    }

    pub async fn context(
        &self,
        query: &str,
        agent_id: &str,
        project_id: Option<&str>,
    ) -> Result<String, ClientError> {
        #[derive(Deserialize)]
        struct Body {
            prompt: String,
        }
        let resp = self
            .http
            .post(self.url("/v1/context"))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({
                "query": query,
                "agent_id": agent_id,
                "project_id": project_id,
            }))
            .send()
            .await
            .map_err(Self::map_transport)?;
        Ok(Self::read_response::<Body>(resp).await?.prompt)
    }

    pub async fn list(&self, args: ListArgs) -> Result<Vec<MemoryRow>, ClientError> {
        #[derive(Deserialize)]
        struct Body {
            items: Vec<MemoryRow>,
        }
        // Built as pairs rather than a struct so an absent filter contributes
        // no query parameter at all — `?kind=` would reach the daemon as an
        // empty string and fail the kind parse.
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(k) = &args.kind {
            query.push(("kind", k.clone()));
        }
        if let Some(t) = args.tier {
            query.push(("tier", t.to_string()));
        }
        if let Some(mt) = &args.memory_type {
            query.push(("memory_type", mt.clone()));
        }
        if let Some(a) = &args.agent {
            query.push(("agent", a.clone()));
        }
        if let Some(p) = &args.project {
            query.push(("project", p.clone()));
        }
        if let Some(s) = &args.status {
            query.push(("status", s.clone()));
        }
        if let Some(q) = &args.q {
            query.push(("q", q.clone()));
        }
        if let Some(l) = args.limit {
            query.push(("limit", l.to_string()));
        }
        if let Some(o) = args.offset {
            query.push(("offset", o.to_string()));
        }

        let resp = self
            .http
            .get(self.url("/v1/memories"))
            .bearer_auth(&self.token)
            .query(&query)
            .send()
            .await
            .map_err(Self::map_transport)?;
        Ok(Self::read_response::<Body>(resp).await?.items)
    }

    /// One row with every column, soft-deleted rows included (they carry
    /// `deleted_at`). A missing row is the daemon's 404.
    pub async fn get(&self, kind: &str, id: i64) -> Result<MemoryRow, ClientError> {
        #[derive(Deserialize)]
        struct Body {
            item: MemoryRow,
        }
        let resp = self
            .http
            .get(self.url(&format!("/v1/memories/{kind}/{id}")))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(Self::map_transport)?;
        Ok(Self::read_response::<Body>(resp).await?.item)
    }

    /// Counts of what is stored, optionally narrowed to an agent and/or project.
    pub async fn stats(
        &self,
        agent: Option<&str>,
        project: Option<&str>,
    ) -> Result<MemoryStats, ClientError> {
        let mut query: Vec<(&str, &str)> = Vec::new();
        if let Some(a) = agent {
            query.push(("agent", a));
        }
        if let Some(p) = project {
            query.push(("project", p));
        }
        let resp = self
            .http
            .get(self.url("/v1/stats"))
            .bearer_auth(&self.token)
            .query(&query)
            .send()
            .await
            .map_err(Self::map_transport)?;
        Self::read_response(resp).await
    }

    pub async fn set_importance(
        &self,
        kind: &str,
        id: i64,
        importance: f64,
    ) -> Result<usize, ClientError> {
        #[derive(Deserialize)]
        struct Body {
            updated: usize,
        }
        let resp = self
            .http
            .patch(self.url(&format!("/v1/memories/{kind}/{id}")))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({ "importance": importance }))
            .send()
            .await
            .map_err(Self::map_transport)?;
        Ok(Self::read_response::<Body>(resp).await?.updated)
    }

    pub async fn delete(&self, kind: &str, id: i64) -> Result<usize, ClientError> {
        #[derive(Deserialize)]
        struct Body {
            deleted: usize,
        }
        let resp = self
            .http
            .delete(self.url(&format!("/v1/memories/{kind}/{id}")))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(Self::map_transport)?;
        Ok(Self::read_response::<Body>(resp).await?.deleted)
    }

    pub async fn shutdown(&self) -> Result<(), ClientError> {
        let resp = self
            .http
            .post(self.url("/v1/shutdown"))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(Self::map_transport)?;
        let _: serde_json::Value = Self::read_response(resp).await?;
        Ok(())
    }
}

// ── Spawning ────────────────────────────────────────────────────────────────

/// Starts a detached `clawg serve`.
///
/// A trait with exactly one production implementation, for one reason: the
/// production one launches a process that downloads a 470 MB model, so no test
/// may ever call it. Everything around autostart — the poll loop, the timeout,
/// the not-running branch — is testable only because this seam exists.
pub trait DaemonSpawner: Send + Sync {
    fn spawn_serve(&self, profile: Profile) -> Result<(), ClientError>;
}

/// Launches the current executable as `clawg serve --profile <p>`, detached.
pub struct ProcessSpawner {
    /// Where `logs/serve.log` goes. Passed in rather than derived so it always
    /// matches the data directory the caller resolved.
    pub base_dir: PathBuf,
}

impl DaemonSpawner for ProcessSpawner {
    fn spawn_serve(&self, profile: Profile) -> Result<(), ClientError> {
        use std::process::{Command, Stdio};

        let exe = std::env::current_exe()
            .map_err(|e| ClientError::Spawn(format!("cannot locate the clawg executable: {e}")))?;

        // Both streams go to one appended log file. Appending, not truncating:
        // a daemon that died is exactly when the previous run's tail matters.
        let log_dir = self.base_dir.join("logs");
        std::fs::create_dir_all(&log_dir)
            .map_err(|e| ClientError::Spawn(format!("cannot create {}: {e}", log_dir.display())))?;
        let log_path = log_dir.join("serve.log");
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| ClientError::Spawn(format!("cannot open {}: {e}", log_path.display())))?;
        let log_err = log
            .try_clone()
            .map_err(|e| ClientError::Spawn(format!("cannot duplicate the log handle: {e}")))?;

        let mut cmd = Command::new(exe);
        cmd.arg("serve")
            .arg("--profile")
            .arg(profile.as_str())
            // Null stdin, never inherited: the caller is usually `clawg mcp`,
            // whose stdin IS the MCP transport. A child sharing it would eat
            // protocol frames.
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err));

        // Detach so the daemon outlives whatever started it — and, on Windows,
        // so a Ctrl-C in the parent's console does not travel to it.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            // Detaching the console is not detaching the handles: without
            // this the daemon inherits the host's stdio pipes and the host
            // never sees this bridge exit. See `detach.rs`.
            crate::detach::disinherit_std_handles();
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Its own process group, so a SIGINT delivered to the parent's
            // group stops the bridge without taking the daemon with it.
            cmd.process_group(0);
        }

        // The handle is dropped without a `wait`: this process does not
        // supervise the daemon, and blocking to reap it would defeat the point
        // of detaching. On Unix that leaves one zombie entry until the parent
        // exits — bounded by one, since autostart happens at most once per
        // bridge process.
        cmd.spawn()
            .map(|_child| ())
            .map_err(|e| ClientError::Spawn(format!("could not spawn `clawg serve`: {e}")))
    }
}

/// Returns a client for a live daemon, starting one if allowed.
///
/// Thin wrapper over [`ensure_running_in`] that resolves the profile's real
/// data directory — the same wrapper/core split the memory admin commands use,
/// and for the same reason: the core is testable against a tempdir, the
/// wrapper's only job is `data_dir`.
pub async fn ensure_running(
    profile: Profile,
    autostart: bool,
    spawner: &dyn DaemonSpawner,
) -> Result<DaemonClient, ClientError> {
    ensure_running_in(&data_dir(profile), profile, autostart, spawner, AUTOSTART_TIMEOUT).await
}

/// [`ensure_running`] against an explicit data directory and timeout.
pub async fn ensure_running_in(
    base_dir: &Path,
    profile: Profile,
    autostart: bool,
    spawner: &dyn DaemonSpawner,
    timeout: Duration,
) -> Result<DaemonClient, ClientError> {
    match probe_running(base_dir, autostart).await? {
        Probe::Running(client) => Ok(client),
        Probe::NeedsStart => start_daemon(base_dir, profile, spawner, timeout).await,
    }
}

/// What the quick half of [`ensure_running_in`] found.
pub enum Probe {
    /// A daemon answered `/health`.
    Running(DaemonClient),
    /// None answered and one may be started: the slow half, [`start_daemon`].
    NeedsStart,
}

/// The half of [`ensure_running_in`] that is bounded by one short `/health`
/// probe and two file reads, so a caller can run it before it must answer
/// anyone. Everything that can wait on a cold start is in [`start_daemon`].
pub async fn probe_running(base_dir: &Path, autostart: bool) -> Result<Probe, ClientError> {
    // A state file alone proves nothing: it survives a crash, and its pid may
    // belong to something else entirely by now. Only a `/health` answer counts.
    if let Some(client) = DaemonClient::from_data_dir(base_dir) {
        if client.health().await.is_ok() {
            return Ok(Probe::Running(client));
        }
    }

    // Before anything that waits. A store owned by a NON-daemon (an evaluation
    // run) means the `clawg serve` we would spawn dies on the store lock at
    // once, and the poll below would burn the whole timeout on a daemon that
    // cannot come up. A daemon holder — possibly still starting, so not yet
    // publishing `daemon.json` — or an unknown holder falls through to the
    // normal path: only a positively-identified non-daemon short-circuits.
    // Ahead of the `autostart` check because "something else has the store" is
    // the truthful answer even with `--no-autostart`, where "not running" would
    // send the user to start a daemon that cannot start.
    if let Some(owner) = StoreLock::current_owner(base_dir) {
        if owner.holder != StoreHolder::Daemon {
            return Err(ClientError::StoreHeld(owner));
        }
    }

    if !autostart {
        return Err(ClientError::NotRunning);
    }

    Ok(Probe::NeedsStart)
}

/// Spawns a daemon and polls until it answers `/health` or `timeout` passes.
pub async fn start_daemon(
    base_dir: &Path,
    profile: Profile,
    spawner: &dyn DaemonSpawner,
    timeout: Duration,
) -> Result<DaemonClient, ClientError> {
    // The data directory has to exist before the daemon can publish into it —
    // on a first ever run it does not.
    std::fs::create_dir_all(base_dir)
        .map_err(|e| ClientError::Spawn(format!("cannot create {}: {e}", base_dir.display())))?;

    spawner.spawn_serve(profile)?;

    // Poll rather than wait on the child: the daemon is detached, so there is
    // no handle to await, and the only fact worth having is that it answers.
    // The state file appearing is NOT enough — it is written before the first
    // request can be served.
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(client) = DaemonClient::from_data_dir(base_dir) {
            if client.health().await.is_ok() {
                return Ok(client);
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(ClientError::StartTimeout(timeout));
        }
        tokio::time::sleep(AUTOSTART_POLL_INTERVAL).await;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    use clawg_core::memory::store_lock::{StoreHolder, StoreLock, StoreOwner};
    use clawg_core::paths::{store_lock_file, store_owner_file};
    use clawg_core::runtime::MemoryRuntime;

    use crate::daemon::api::{router, AppCtx};
    use crate::daemon::lifecycle::{write_state, write_token, DaemonState};

    /// An in-process daemon plus the tempdir holding its lifecycle files.
    pub(crate) struct TestDaemon {
        pub(crate) dir: tempfile::TempDir,
        addr: SocketAddr,
        token: String,
    }

    pub(crate) async fn start_daemon() -> TestDaemon {
        let dir = tempfile::tempdir().expect("tempdir");
        let (runtime, rx) = MemoryRuntime::new_for_test(dir.path().to_path_buf()).await;
        tokio::spawn(runtime.ingest_worker(rx));

        let token = "0123456789abcdef0123456789abcdef".to_string();
        let (shutdown, _rx) = tokio::sync::watch::channel(false);
        let ctx = Arc::new(AppCtx {
            runtime,
            token: token.clone(),
            profile: Profile::Dev,
            started_at: Instant::now(),
            shutdown,
        });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(ctx)).await;
        });

        TestDaemon { dir, addr, token }
    }

    /// Publishes the daemon's lifecycle files so `from_data_dir` can find it.
    pub(crate) fn publish(d: &TestDaemon) {
        write_state(
            d.dir.path(),
            &DaemonState {
                pid: std::process::id(),
                port: d.addr.port(),
                profile: "dev".into(),
                version: "0.1.0".into(),
                started_at_ms: 0,
            },
        )
        .expect("write state");
        write_token(d.dir.path(), &d.token).expect("write token");
    }

    /// Stands in for `ProcessSpawner`: brings up an in-process daemon and
    /// publishes its files, exactly as a real `clawg serve` would.
    struct FakeSpawner {
        daemon: TestDaemon,
        calls: AtomicUsize,
    }

    impl DaemonSpawner for FakeSpawner {
        fn spawn_serve(&self, _profile: Profile) -> Result<(), ClientError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            publish(&self.daemon);
            Ok(())
        }
    }

    /// Never brings anything up — the "the daemon failed to start" path.
    struct DeadSpawner;
    impl DaemonSpawner for DeadSpawner {
        fn spawn_serve(&self, _profile: Profile) -> Result<(), ClientError> {
            Ok(())
        }
    }

    /// [`DeadSpawner`] that counts its calls, for the tests that must prove a
    /// spawn did or did not happen.
    struct CountingDeadSpawner {
        calls: AtomicUsize,
    }
    impl CountingDeadSpawner {
        fn new() -> Self {
            Self { calls: AtomicUsize::new(0) }
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }
    impl DaemonSpawner for CountingDeadSpawner {
        fn spawn_serve(&self, _profile: Profile) -> Result<(), ClientError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    // ── from_data_dir ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn from_data_dir_is_none_without_lifecycle_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(DaemonClient::from_data_dir(dir.path()).is_none());
    }

    #[tokio::test]
    async fn from_data_dir_needs_both_the_state_and_the_token() {
        let d = start_daemon().await;
        write_state(
            d.dir.path(),
            &DaemonState {
                pid: 1,
                port: d.addr.port(),
                profile: "dev".into(),
                version: "0.1.0".into(),
                started_at_ms: 0,
            },
        )
        .expect("write state");
        assert!(
            DaemonClient::from_data_dir(d.dir.path()).is_none(),
            "a state file without a token cannot authenticate, so it is not a usable client"
        );

        write_token(d.dir.path(), &d.token).expect("write token");
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("now both files exist");
        assert!(c.base_url().contains("127.0.0.1"));
        assert!(c.base_url().contains(&d.addr.port().to_string()));
    }

    // ── Happy paths ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn health_reports_the_daemons_profile_and_embedder_state() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");

        let h = c.health().await.expect("health");
        assert_eq!(h.status, "ok");
        assert_eq!(h.profile, "dev");
        assert!(h.embedder_ready(), "new_for_test pre-seeds the embedder");
    }

    #[tokio::test]
    async fn save_then_list_then_search_then_patch_then_delete() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");

        let content = "the release train leaves every second tuesday";
        c.save(SaveArgs {
            content: content.into(),
            agent_id: "agent-client".into(),
            project_id: None,
            memory_type: Some("semantic_fact".into()),
            importance: Some(7.0),
        })
        .await
        .expect("save");

        // Ingest is asynchronous by contract; poll rather than sleep.
        let mut rows = Vec::new();
        for _ in 0..40 {
            rows = c
                .list(ListArgs { kind: Some("memory".into()), ..Default::default() })
                .await
                .expect("list");
            if !rows.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(rows.len(), 1, "the saved memory must become exactly one row");
        assert_eq!(rows[0].summary, content);
        assert_eq!(rows[0].memory_type.as_deref(), Some("semantic_fact"));
        let id = rows[0].id;

        let hits = c
            .search(SearchArgs {
                query: content.into(),
                agent_id: "agent-client".into(),
                project_id: None,
                limit: Some(5),
            })
            .await
            .expect("search");
        assert!(hits.iter().any(|h| h.id == id), "the saved row must be retrievable");

        let prompt = c.context(content, "agent-client", None).await.expect("context");
        let _ = prompt; // an empty prompt is a valid answer

        assert_eq!(c.set_importance("memory", id, 9.5).await.expect("patch"), 1);
        assert_eq!(c.delete("memory", id).await.expect("delete"), 1);
    }

    #[test]
    fn search_request_carries_the_relevance_ratio() {
        let args = SearchArgs {
            query: "q".into(),
            agent_id: "a".into(),
            project_id: None,
            limit: Some(5),
        };
        let body = serde_json::to_value(search_request(&args)).expect("serializes");
        assert_eq!(body["min_relative_score"], serde_json::json!(crate::relevance::RELEVANCE_RATIO));
        assert_eq!(body["query"], "q");
        assert_eq!(body["limit"], 5);
    }

    // ── Error mapping ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_wrong_token_maps_to_unauthorized() {
        let d = start_daemon().await;
        publish(&d);
        write_token(d.dir.path(), "not-the-real-token").expect("write token");
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");

        let err = c
            .list(ListArgs::default())
            .await
            .expect_err("a bad token must not read rows");
        assert!(
            matches!(err, ClientError::Unauthorized),
            "401 must not be flattened into a generic daemon error: {err:?}"
        );
    }

    #[tokio::test]
    async fn a_daemon_side_error_keeps_its_status_and_message() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");

        let err = c
            .delete("memory", 999_999)
            .await
            .expect_err("deleting a missing row must fail");
        match err {
            ClientError::Daemon { status, message } => {
                assert_eq!(status, 404);
                assert!(message.contains("999999"), "message was: {message}");
            }
            other => panic!("expected a daemon error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unreachable_address_maps_to_not_running() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_state(
            dir.path(),
            &DaemonState {
                // Port 1 on loopback: nothing binds it, so connecting fails
                // immediately rather than hanging on a timeout.
                pid: 1,
                port: 1,
                profile: "dev".into(),
                version: "0.1.0".into(),
                started_at_ms: 0,
            },
        )
        .expect("write state");
        write_token(dir.path(), "token").expect("write token");
        let c = DaemonClient::from_data_dir(dir.path()).expect("client");

        let err = c.health().await.expect_err("nothing is listening");
        assert!(
            matches!(err, ClientError::NotRunning),
            "a refused connection is 'not running', not a transport bug: {err:?}"
        );
    }

    // ── ensure_running ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn ensure_running_returns_the_live_daemon_without_spawning() {
        let d = start_daemon().await;
        publish(&d);
        let base = d.dir.path().to_path_buf();
        let spawner = FakeSpawner { daemon: start_daemon().await, calls: AtomicUsize::new(0) };

        let c = ensure_running_in(&base, Profile::Dev, true, &spawner, Duration::from_secs(2))
            .await
            .expect("the running daemon must be used");
        assert_eq!(c.health().await.expect("health").status, "ok");
        assert_eq!(
            spawner.calls.load(Ordering::SeqCst),
            0,
            "a daemon that already answers must never be started a second time"
        );
    }

    #[tokio::test]
    async fn ensure_running_without_autostart_reports_not_running() {
        let dir = tempfile::tempdir().expect("tempdir");
        let spawner = FakeSpawner { daemon: start_daemon().await, calls: AtomicUsize::new(0) };

        let err = ensure_running_in(
            dir.path(),
            Profile::Dev,
            false,
            &spawner,
            Duration::from_secs(2),
        )
        .await
        .expect_err("no daemon and no autostart");
        assert!(matches!(err, ClientError::NotRunning), "{err:?}");
        assert_eq!(spawner.calls.load(Ordering::SeqCst), 0, "--no-autostart must not spawn");
    }

    #[tokio::test]
    async fn ensure_running_starts_a_daemon_and_then_connects_to_it() {
        // The spawner's daemon publishes into ITS OWN tempdir, which is the
        // directory `ensure_running_in` is pointed at — exactly how a real
        // `clawg serve` publishes into the profile's data directory.
        let daemon = start_daemon().await;
        let base = daemon.dir.path().to_path_buf();
        let spawner = FakeSpawner { daemon, calls: AtomicUsize::new(0) };

        let c = ensure_running_in(&base, Profile::Dev, true, &spawner, Duration::from_secs(5))
            .await
            .expect("autostart must produce a usable client");
        assert_eq!(spawner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(c.health().await.expect("health").status, "ok");
    }

    /// A spawner that silently fails must not hang the bridge forever; the
    /// timeout has to be reported as such, not as "not running".
    #[tokio::test]
    async fn ensure_running_times_out_when_the_daemon_never_appears() {
        let dir = tempfile::tempdir().expect("tempdir");
        let started = Instant::now();
        let err = ensure_running_in(
            dir.path(),
            Profile::Dev,
            true,
            &DeadSpawner,
            Duration::from_millis(300),
        )
        .await
        .expect_err("nothing ever came up");
        assert!(matches!(err, ClientError::StartTimeout(_)), "{err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the timeout must bound the wait, not merely decorate it"
        );
    }

    // ── ensure_running: who holds the store ─────────────────────────────────

    /// A holder that is not a daemon can never answer `/health`, so spawning
    /// one and polling for a minute is a guaranteed wait for nothing: the
    /// spawned `clawg serve` dies on the store lock at once. Fail at once too,
    /// and name the holder so the user knows what to close.
    #[tokio::test]
    async fn a_store_held_by_a_non_daemon_fails_fast_without_spawning() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _holder = StoreLock::acquire(dir.path(), StoreHolder::Eval).expect("holder has it");
        let spawner = CountingDeadSpawner::new();

        let started = Instant::now();
        let err = ensure_running_in(dir.path(), Profile::Dev, true, &spawner, Duration::from_secs(5))
            .await
            .expect_err("the store is held by a non-daemon");

        match err {
            ClientError::StoreHeld(owner) => assert_eq!(owner.holder, StoreHolder::Eval),
            other => panic!("expected StoreHeld, got {other:?}"),
        }
        assert_eq!(spawner.calls(), 0, "a daemon cannot start while a non-daemon holds the store");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "failing fast means not waiting on the timeout: took {:?}",
            started.elapsed()
        );
    }

    /// A daemon that holds the store but has not published `daemon.json` yet is
    /// just a slow start — the exact case the poll loop exists for.
    #[tokio::test]
    async fn a_store_held_by_a_daemon_still_takes_the_autostart_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _daemon = StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("daemon holds it");
        let spawner = CountingDeadSpawner::new();

        let err = ensure_running_in(dir.path(), Profile::Dev, true, &spawner, Duration::from_millis(300))
            .await
            .expect_err("no daemon.json ever appears");
        assert!(matches!(err, ClientError::StartTimeout(_)), "{err:?}");
        assert_eq!(spawner.calls(), 1, "a daemon holder must not short-circuit the spawn");
    }

    /// A crashed holder leaves `memory.owner` behind with the lock already gone;
    /// believing it would refuse the bridge forever.
    #[tokio::test]
    async fn a_stale_owner_record_does_not_block_autostart() {
        let daemon = start_daemon().await;
        let base = daemon.dir.path().to_path_buf();
        let stale = StoreOwner { holder: StoreHolder::Eval, pid: 4242, started_at_ms: 1 };
        std::fs::write(store_owner_file(&base), serde_json::to_string(&stale).expect("json"))
            .expect("write stale record");
        std::fs::write(store_lock_file(&base), b"").expect("unlocked lock file");
        let spawner = FakeSpawner { daemon, calls: AtomicUsize::new(0) };

        let c = ensure_running_in(&base, Profile::Dev, true, &spawner, Duration::from_secs(5))
            .await
            .expect("a dead holder's leftovers must not stop the daemon from starting");
        assert_eq!(spawner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(c.health().await.expect("health").status, "ok");
    }

    /// With `--no-autostart` the useful answer is still "something else has
    /// it", not "no daemon is running", which would send the user to start a
    /// daemon that cannot start.
    #[tokio::test]
    async fn a_held_store_is_reported_even_without_autostart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _holder = StoreLock::acquire(dir.path(), StoreHolder::Eval).expect("holder has it");
        let spawner = CountingDeadSpawner::new();

        let err = ensure_running_in(dir.path(), Profile::Dev, false, &spawner, Duration::from_secs(5))
            .await
            .expect_err("held by a non-daemon");
        assert!(matches!(err, ClientError::StoreHeld(_)), "{err:?}");
        assert_eq!(spawner.calls(), 0);
    }

    #[test]
    fn store_held_names_the_holder_and_says_it_is_not_a_daemon() {
        let err = ClientError::StoreHeld(StoreOwner {
            holder: StoreHolder::Eval,
            pid: 4242,
            started_at_ms: 1,
        });
        assert_eq!(
            err.to_string(),
            "the memory store is held by an evaluation run (pid 4242), not by a clawg daemon; close it and reconnect"
        );
    }

    #[tokio::test]
    async fn shutdown_asks_the_daemon_to_stop() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        c.shutdown().await.expect("shutdown must be accepted");
    }

    // ── Viewer reads ────────────────────────────────────────────────────────

    /// Queues one memory and waits until the ingest worker has stored it.
    pub(crate) async fn save_and_wait(
        c: &DaemonClient,
        content: &str,
        agent: &str,
        project: Option<&str>,
        want_rows: usize,
    ) {
        c.save(SaveArgs {
            content: content.into(),
            agent_id: agent.into(),
            project_id: project.map(str::to_owned),
            memory_type: Some("semantic_fact".into()),
            importance: Some(6.0),
        })
        .await
        .expect("save");
        for _ in 0..40 {
            let rows = c.list(ListArgs::default()).await.expect("list");
            if rows.len() >= want_rows {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("ingest never produced {want_rows} row(s) within 2 s");
    }

    #[tokio::test]
    async fn get_reads_one_row() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        save_and_wait(&c, "the staging database lives in eu-west", "agent-get", Some("proj-x"), 1)
            .await;
        let id = c.list(ListArgs::default()).await.expect("list")[0].id;

        let row = c.get("memory", id).await.expect("get");
        assert_eq!(row.id, id);
        assert_eq!(row.kind, "memory");
        assert_eq!(row.summary, "the staging database lives in eu-west");
        assert_eq!(row.agent_id.as_deref(), Some("agent-get"));
        assert_eq!(row.project_id.as_deref(), Some("proj-x"));
        assert_eq!(row.source.as_deref(), Some("explicit"));
        assert_eq!(row.update_count, Some(0));
        assert!(row.created_at.is_some_and(|t| t > 0), "{row:?}");
        assert_eq!(row.deleted_at, None);
    }

    #[tokio::test]
    async fn get_missing_maps_to_daemon_404() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");

        match c.get("memory", 999_999).await.expect_err("no such row") {
            ClientError::Daemon { status, message } => {
                assert_eq!(status, 404);
                assert!(message.contains("999999"), "message was: {message}");
            }
            other => panic!("expected a daemon error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stats_reads_counts() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        save_and_wait(&c, "first note about queues", "agent-a", None, 1).await;
        save_and_wait(&c, "second note about indexes", "agent-b", Some("proj-1"), 2).await;

        let all = c.stats(None, None).await.expect("stats");
        assert_eq!(all.totals.memory, 2);
        assert_eq!(all.totals.deleted, 0);
        assert_eq!(all.by_agent.len(), 2, "{all:?}");
        assert!(
            all.by_project.iter().any(|b| b.key.is_none() && b.count == 1),
            "the row with no project is its own bucket: {all:?}"
        );
        assert!(all.newest_created_at.is_some());

        let one = c.stats(Some("agent-a"), None).await.expect("stats");
        assert_eq!(one.totals.memory, 1);
    }

    #[tokio::test]
    async fn list_forwards_new_filters() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        save_and_wait(&c, "alpha note about caching layers", "agent-a", Some("proj-1"), 1).await;
        save_and_wait(&c, "beta note about retry budgets", "agent-b", Some("proj-1"), 2).await;
        save_and_wait(&c, "gamma note about schema drift", "agent-a", Some("proj-2"), 3).await;

        let by_agent = c
            .list(ListArgs { agent: Some("agent-a".into()), ..Default::default() })
            .await
            .expect("list");
        assert_eq!(by_agent.len(), 2);

        let by_both = c
            .list(ListArgs {
                agent: Some("agent-a".into()),
                project: Some("proj-1".into()),
                ..Default::default()
            })
            .await
            .expect("list");
        assert_eq!(by_both.len(), 1);
        assert_eq!(by_both[0].summary, "alpha note about caching layers");

        let by_text = c
            .list(ListArgs { q: Some("retry".into()), ..Default::default() })
            .await
            .expect("list");
        assert_eq!(by_text.len(), 1);
        assert_eq!(by_text[0].summary, "beta note about retry budgets");

        let by_status = c
            .list(ListArgs { status: Some("disputed".into()), ..Default::default() })
            .await
            .expect("list");
        assert!(by_status.is_empty(), "nothing is disputed: {by_status:?}");
    }

    /// A daemon older than the viewer answers without the new columns; the
    /// bridge and the viewer must both keep reading it.
    #[test]
    fn memory_row_tolerates_missing_provenance_fields() {
        let old = serde_json::json!({
            "id": 7,
            "kind": "memory",
            "memory_type": "semantic_fact",
            "summary": "kept",
            "tier": 2,
            "importance": 5.0,
            "status": "active",
            "access_count": 1
        });
        let row: MemoryRow = serde_json::from_value(old).expect("old shape still parses");
        assert_eq!(row.id, 7);
        assert_eq!(row.source, None);
        assert_eq!(row.created_at, None);
        assert_eq!(row.update_count, None);
        assert_eq!(row.agent_id, None);
    }
}
