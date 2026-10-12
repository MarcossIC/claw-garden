//! The `clawg mcp` stdio server.
//!
//! A bridge, not a memory system: every tool below turns its arguments into one
//! HTTP call against a running `clawg serve` and renders the answer as text.
//! It owns no database, no embedder and no workers, which is why two editors
//! can each run one against the same profile without corrupting anything.
//!
//! # How failures are reported
//!
//! Every failure comes back as `CallToolResult::error` with a sentence, and
//! never as `Err(ErrorData)`. The caller here is a language model: a protocol
//! error reaches it as an opaque fault it cannot act on, whereas "the embedder
//! is still loading, try again shortly" tells it exactly what to do next. The
//! `is_error` flag still marks the call as failed, so nothing is being hidden.
//!
//! # Why `*_impl` exists beside each tool
//!
//! rmcp resolves the calling client from a `Peer`, which no test can construct
//! (`Peer::new` is crate-private). So each `#[tool]` method does exactly two
//! things — resolve the agent id from the peer, then delegate — and all the
//! behaviour lives in an `*_impl` taking that id explicitly, where a test can
//! drive it against an in-process daemon.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use clawg_core::memory::ProjectIdentity;
use clawg_core::paths::Profile;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{tool, tool_handler, tool_router, Peer, RoleServer, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::client::{ClientError, DaemonClient, ListArgs, SaveArgs, SaveOutcome, SearchArgs};
use crate::mcp::startup::ClientHandle;
use crate::mcp::tools::{
    format_context, format_resolution, format_rows, format_search, format_status, resolve_project,
};

/// Agent id used when the MCP client did not identify itself.
///
/// Memories are scoped per agent, so this is not cosmetic: everything saved
/// under it is retrievable under it, and a client that changes its reported
/// name effectively gets a new memory.
pub const FALLBACK_AGENT_ID: &str = "mcp";

/// What the host is told this server is for, at `initialize` or
/// `server/discover`.
const INSTRUCTIONS: &str = "\
Persistent memory for this user across sessions. Use `save_memory` whenever the \
user states a durable fact, preference or decision; use `search_memory` before \
answering anything that depends on earlier sessions, and `recall_context` once \
when a broad task needs background. When it lists ACTIVE CONFLICTS, ask the user \
which memory holds and record the answer with `resolve_conflict`. \
Memories are scoped to the current project unless you say otherwise.";

// ── Tool parameters ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SaveMemoryArgs {
    /// The memory to store, written as a self-contained statement — it will be
    /// read back in a future session with none of this conversation around it.
    pub content: String,
    /// What kind of memory this is. One of: semantic_fact, user_preference,
    /// architectural_decision, session_summary, code_block, chat_message,
    /// tool_error.
    /// Defaults to semantic_fact when omitted.
    #[serde(default)]
    pub memory_type: Option<String>,
    /// How much this matters, 0 to 10. Higher values survive longer and are
    /// retrieved sooner. Defaults to 5.
    #[serde(default)]
    pub importance: Option<f64>,
    /// Absolute path of the project this memory belongs to. Defaults to the
    /// directory this bridge was started in.
    #[serde(default)]
    pub project_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SearchMemoryArgs {
    /// What to look for, in natural language. Full sentences retrieve better
    /// than keywords.
    pub query: String,
    /// How many memories to return, 1 to 50. Defaults to 10.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Absolute path of the project to search in. Defaults to the directory
    /// this bridge was started in.
    #[serde(default)]
    pub project_path: Option<String>,
    /// Search every project instead of just one. Use when the user asks about
    /// something that is not tied to the code in front of them.
    #[serde(default)]
    pub all_projects: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RecallContextArgs {
    /// What the user is asking about. The returned block is meant to be read
    /// as background before answering, not quoted back to them.
    pub query: String,
    /// Absolute path of the project. Defaults to the directory this bridge was
    /// started in.
    #[serde(default)]
    pub project_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ListMemoriesArgs {
    /// Which table to list: memory, episodic or procedural. Lists all three
    /// when omitted.
    #[serde(default)]
    pub kind: Option<String>,
    /// Only rows in this tier: 1 (hot), 2 (warm) or 3 (cold).
    #[serde(default)]
    pub tier: Option<i64>,
    /// Only rows of this memory type. Applies to `memory` rows only.
    #[serde(default)]
    pub memory_type: Option<String>,
    /// How many rows to return, 1 to 100. Defaults to 50.
    #[serde(default)]
    pub limit: Option<u32>,
    /// How many rows to skip, for paging through a long list. Defaults to 0.
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DeleteMemoryArgs {
    /// The id of the row to delete, as printed by `search_memory` or
    /// `list_memories`.
    pub id: i64,
    /// Which table the id belongs to: memory, episodic or procedural. Required,
    /// because each table numbers its rows independently — id 7 exists in all
    /// three and means something different in each.
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ResolveConflictArgs {
    /// The memory id that stays, as shown in the ACTIVE CONFLICTS block.
    pub keep: i64,
    /// The other memory id of the same conflict. It is superseded by `keep`:
    /// no longer retrieved, still listed, and restored by resolving the pair
    /// the other way round.
    pub other: i64,
    /// Set to true when the two memories do not actually contradict each
    /// other: both stay and the conflict is closed. Defaults to false.
    #[serde(default)]
    pub keep_both: Option<bool>,
}

// ── The server ──────────────────────────────────────────────────────────────

/// The MCP server: one daemon client, one identity, one tool router.
#[derive(Clone)]
pub struct ClawgMcp {
    client: ClientHandle,
    /// Used when the peer has not identified itself.
    agent_id: String,
    /// The project the bridge process itself is sitting in.
    default_project: Option<ProjectIdentity>,
    /// Roots (with the legacy id they were reached through) already sent to
    /// `POST /v1/projects` by this process. Shared by every clone, because rmcp
    /// clones the server per call.
    registered: Arc<tokio::sync::Mutex<HashSet<(PathBuf, String)>>>,
    profile: Profile,
    tool_router: ToolRouter<Self>,
}

impl ClawgMcp {
    /// Builds a server whose default project is the bridge's working directory.
    ///
    /// The working directory is the right default because an MCP host launches
    /// the bridge from the project the user has open — so "where am I?" and
    /// "which project do these memories belong to?" have the same answer
    /// without anyone configuring it.
    pub fn new(client: DaemonClient, profile: Profile) -> Self {
        Self::pending(ClientHandle::ready(client), profile)
    }

    /// [`ClawgMcp::new`] for a daemon that may still be starting: each tool
    /// call waits on the handle instead of assuming a live client.
    pub fn pending(client: ClientHandle, profile: Profile) -> Self {
        let default_project = std::env::current_dir()
            .ok()
            .map(|cwd| ProjectIdentity::resolve(&cwd));
        Self::with_handle(client, profile, default_project)
    }

    /// [`ClawgMcp::new`] with the default project stated explicitly.
    pub fn with_project(
        client: DaemonClient,
        profile: Profile,
        default_project: Option<ProjectIdentity>,
    ) -> Self {
        Self::with_handle(ClientHandle::ready(client), profile, default_project)
    }

    fn with_handle(
        client: ClientHandle,
        profile: Profile,
        default_project: Option<ProjectIdentity>,
    ) -> Self {
        Self {
            client,
            agent_id: FALLBACK_AGENT_ID.to_string(),
            default_project,
            registered: Arc::default(),
            profile,
            tool_router: Self::tool_router(),
        }
    }

    /// Which agent's memory a call reads and writes.
    ///
    /// Resolved per call rather than once at construction because rmcp only
    /// knows the peer after `initialize`, and a server built before that would
    /// pin the fallback forever.
    fn agent_id_for(&self, peer: Option<&Peer<RoleServer>>) -> String {
        peer.and_then(|p| p.peer_info().map(|info| info.client_info.name.clone()))
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| self.agent_id.clone())
    }

    fn ok(text: impl Into<String>) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(CallToolResult::success(vec![ContentBlock::text(text.into())]))
    }

    fn failed(text: impl Into<String>) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(CallToolResult::error(vec![ContentBlock::text(text.into())]))
    }

    /// The daemon client, once the background start has produced one.
    ///
    /// The failure is already the sentence the model should read, so each tool
    /// returns it as its own result instead of hanging or dropping the bridge.
    async fn daemon(&self) -> Result<DaemonClient, CallToolResult> {
        self.client.get().await.map_err(|e| {
            CallToolResult::error(vec![ContentBlock::text(Self::explain(&e))])
        })
    }

    /// Turns a client failure into the sentence a model should act on.
    ///
    /// The cases worth distinguishing are the ones a caller can do something
    /// about: a daemon that is not up (tell the user to start one), an
    /// embedder that is not ready yet (wait and retry) and one that failed to
    /// load (search is out until it does). Everything else is one message,
    /// because the model's next move is the same either way.
    fn explain(err: &ClientError) -> String {
        match err {
            ClientError::NotRunning => {
                "no clawg daemon is running; start one with `clawg serve` and try again"
                    .to_string()
            }
            // Starting it was tried, so `clawg serve` is not the advice: the
            // reason it gave up (a store from a newer clawg, say) is what
            // `clawg status` reports.
            ClientError::StartTimeout(_) => {
                "clawg tried to start its daemon and it is not up; run `clawg status` to see \
                 why, or try again if it is still starting"
                    .to_string()
            }
            ClientError::Daemon { status: 503, message, .. } if message == "embedder_not_ready" => {
                "the memory index is still loading its embedding model; \
                 saving works now, and search will work in a minute"
                    .to_string()
            }
            ClientError::Daemon { message, details, .. } if message == "embedder_failed" => {
                format!(
                    "the memory index could not load its embedding model ({}); search and \
                     saving are unavailable {}",
                    details.detail.as_deref().unwrap_or("no cause given"),
                    if details.retryable == Some(true) {
                        "until clawg's retry succeeds"
                    } else {
                        "until the cause is fixed"
                    }
                )
            }
            other => other.to_string(),
        }
    }

    /// What `save_memory` tells the model: `Ok` is a success result, `Err` an
    /// error result. Every id it names is one `delete_memory` takes with
    /// `kind: "memory"`.
    fn save_outcome_text(result: Result<SaveOutcome, ClientError>) -> Result<String, String> {
        let ids = |memories: &[crate::client::SavedMemory]| {
            memories.iter().map(|m| format!("#{}", m.id)).collect::<Vec<_>>().join(", ")
        };
        match result {
            Ok(SaveOutcome::Saved { memories }) => Ok(match memories.as_slice() {
                [one] if one.outcome == "reinforced" => {
                    format!("already stored as #{}; reinforced it instead of saving a copy", one.id)
                }
                [one] => format!("saved #{}", one.id),
                many => format!("saved as {} pieces: {}", many.len(), ids(many)),
            }),
            Ok(SaveOutcome::Queued { reason }) if reason.as_deref() == Some("embedder_loading") => {
                Ok("accepted; it will be stored once the embedding model finishes loading".into())
            }
            Ok(SaveOutcome::Queued { .. }) => Ok(
                "accepted but not confirmed yet; check with list_memories before relying on it".into(),
            ),
            Err(ClientError::Daemon { message, details, .. }) if message == "embedder_failed" => {
                let mut text = format!(
                    "not saved: the embedding model failed to load ({})",
                    details.detail.as_deref().unwrap_or("no cause given")
                );
                if details.retryable == Some(true) {
                    text.push_str("; clawg will retry");
                }
                Err(text)
            }
            Err(ClientError::Daemon { message, .. }) if message == "ingest_queue_full" => {
                Err("not saved: the ingest queue is full; try again shortly".into())
            }
            Err(ClientError::Daemon { message, details, .. })
                if message == "ingest_failed" || message == "embedder_unavailable" =>
            {
                let mut text = format!("not saved: {}", details.detail.unwrap_or(message));
                if !details.memories.is_empty() {
                    text.push_str(&format!("; partially saved as {}", ids(&details.memories)));
                }
                Err(text)
            }
            Err(other) => Err(Self::explain(&other)),
        }
    }

    /// Which project a call is scoped to, registered with the daemon first so
    /// rows saved under its legacy id are already under its id when the call
    /// reads or writes.
    ///
    /// Each root and legacy id is sent once per process, even when the call
    /// fails: a daemon that cannot register (an older version, say) must not
    /// cost every tool call a round trip, and the scope is the same either way.
    async fn project_for(
        &self,
        client: &DaemonClient,
        project_path: Option<&str>,
        all_projects: bool,
    ) -> Option<String> {
        let project = resolve_project(
            self.default_project.as_ref(),
            project_path,
            all_projects,
            ProjectIdentity::resolve,
        )?;
        let mut registered = self.registered.lock().await;
        if registered.insert((project.root.clone(), project.legacy_id.clone())) {
            let legacy: Vec<String> =
                Some(project.legacy_id.clone()).filter(|l| *l != project.id).into_iter().collect();
            if let Err(e) = client.register_project(&project.root, &legacy).await {
                log::warn!("could not register project {}: {e}", project.root.display());
            }
        }
        Some(project.id)
    }

    // ── Implementations, testable without a Peer ────────────────────────────

    pub async fn save_memory_impl(
        &self,
        agent_id: &str,
        args: SaveMemoryArgs,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if args.content.trim().is_empty() {
            return Self::failed("content must not be empty");
        }
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        // A save is never "all projects": a memory belongs somewhere, and an
        // unscoped write would be invisible to every project-scoped search.
        let project_id = self.project_for(&client, args.project_path.as_deref(), false).await;
        let result = client
            .save(SaveArgs {
                content: args.content,
                agent_id: agent_id.to_string(),
                project_id,
                memory_type: args.memory_type,
                importance: args.importance,
            })
            .await;
        // "saved" only when the daemon confirmed the rows exist: telling a
        // model otherwise invites it to search for something still in a
        // channel.
        match Self::save_outcome_text(result) {
            Ok(text) => Self::ok(text),
            Err(text) => Self::failed(text),
        }
    }

    pub async fn search_memory_impl(
        &self,
        agent_id: &str,
        args: SearchMemoryArgs,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        let project_id = self
            .project_for(&client, args.project_path.as_deref(), args.all_projects.unwrap_or(false))
            .await;
        match client
            .search(SearchArgs {
                query: args.query,
                agent_id: agent_id.to_string(),
                project_id,
                limit: args.limit,
            })
            .await
        {
            Ok(hits) => Self::ok(format_search(hits)),
            Err(e) => Self::failed(Self::explain(&e)),
        }
    }

    pub async fn recall_context_impl(
        &self,
        agent_id: &str,
        args: RecallContextArgs,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        let project_id = self.project_for(&client, args.project_path.as_deref(), false).await;
        match client
            .context(&args.query, agent_id, project_id.as_deref())
            .await
        {
            Ok(prompt) => Self::ok(format_context(&prompt)),
            Err(e) => Self::failed(Self::explain(&e)),
        }
    }

    pub async fn list_memories_impl(
        &self,
        args: ListMemoriesArgs,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        match client
            .list(ListArgs {
                kind: args.kind,
                tier: args.tier,
                memory_type: args.memory_type,
                limit: args.limit,
                offset: args.offset,
                ..Default::default()
            })
            .await
        {
            Ok(rows) => Self::ok(format_rows(&rows)),
            Err(e) => Self::failed(Self::explain(&e)),
        }
    }

    pub async fn delete_memory_impl(
        &self,
        args: DeleteMemoryArgs,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        match client.delete(&args.kind, args.id).await {
            Ok(n) => Self::ok(format!("deleted {n}")),
            Err(e) => Self::failed(Self::explain(&e)),
        }
    }

    pub async fn resolve_conflict_impl(
        &self,
        args: ResolveConflictArgs,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        match client.resolve(args.keep, args.other, args.keep_both.unwrap_or(false)).await {
            Ok(resolution) => Self::ok(format_resolution(&resolution)),
            Err(e) => Self::failed(Self::explain(&e)),
        }
    }

    pub async fn memory_status_impl(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        match client.health().await {
            Ok(health) => Self::ok(format_status(&health, self.profile, client.base_dir())),
            Err(e) => Self::failed(Self::explain(&e)),
        }
    }
}

// ── Tools ───────────────────────────────────────────────────────────────────

#[tool_router]
impl ClawgMcp {
    #[tool(
        name = "save_memory",
        description = "Store something worth remembering across sessions: a fact about the \
                       user or their project, a preference they stated, or a decision that was \
                       made. Save one fact per call, in a few sentences: a long text is split \
                       into separate pieces that are found independently and never reassembled. \
                       Write the content so it still makes sense months from now, with no \
                       conversation around it. Returns the id of each stored piece, which \
                       delete_memory accepts, or why nothing was stored."
    )]
    async fn save_memory(
        &self,
        peer: Peer<RoleServer>,
        Parameters(args): Parameters<SaveMemoryArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.save_memory_impl(&self.agent_id_for(Some(&peer)), args).await
    }

    #[tool(
        name = "search_memory",
        description = "Find stored memories related to a query. Use this first, before \
                       answering anything that depends on an earlier session. Matching is by \
                       meaning, so pass a full natural-language question, not keywords; it \
                       finds memories written in other words or in another language. Returns a \
                       numbered list of the memories close in relevance to the best match (at \
                       least two), with their ids, kinds and relevance scores; a short list \
                       means the rest scored far lower, not that nothing else is stored. \
                       Content longer than 500 characters is truncated."
    )]
    async fn search_memory(
        &self,
        peer: Peer<RoleServer>,
        Parameters(args): Parameters<SearchMemoryArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.search_memory_impl(&self.agent_id_for(Some(&peer)), args).await
    }

    #[tool(
        name = "recall_context",
        description = "Get a ready-to-read background block for a query: the user's workspace \
                       notes, recent memories and long-term memories, already assembled and \
                       deduplicated, with full content. It costs several times the tokens of \
                       `search_memory`, so use it once when starting a broad task that needs \
                       background, and `search_memory` for specific questions."
    )]
    async fn recall_context(
        &self,
        peer: Peer<RoleServer>,
        Parameters(args): Parameters<RecallContextArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.recall_context_impl(&self.agent_id_for(Some(&peer)), args).await
    }

    #[tool(
        name = "list_memories",
        description = "Browse what is stored, newest first, without searching. Use this to \
                       show the user their memory inventory or to find an id to delete. \
                       Returns one line per row: id, kind, type, tier, importance, access \
                       count, status and a truncated summary."
    )]
    async fn list_memories(
        &self,
        Parameters(args): Parameters<ListMemoriesArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.list_memories_impl(args).await
    }

    #[tool(
        name = "delete_memory",
        description = "Delete one stored memory by id and kind. Get the id from \
                       `search_memory` or `list_memories` first — ids are per-kind, so the \
                       same number exists in all three tables."
    )]
    async fn delete_memory(
        &self,
        Parameters(args): Parameters<DeleteMemoryArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.delete_memory_impl(args).await
    }

    #[tool(
        name = "resolve_conflict",
        description = "Settle a conflict listed under ACTIVE CONFLICTS by `recall_context`, once \
                       the user has said which memory is right. `keep` stays; `other` is \
                       superseded by it and stops being retrieved, but is still listed and comes \
                       back if you resolve the pair the other way. Pass keep_both when the two \
                       do not really contradict each other. Returns the status of both memories."
    )]
    async fn resolve_conflict(
        &self,
        Parameters(args): Parameters<ResolveConflictArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.resolve_conflict_impl(args).await
    }

    #[tool(
        name = "memory_status",
        description = "Report whether the memory daemon is running, which profile and data \
                       directory it is using, and whether its embedding model has finished \
                       loading. Use this when a memory tool fails and you need to tell the \
                       user why."
    )]
    async fn memory_status(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        self.memory_status_impl().await
    }
}

// The router built once in `with_handle`, not one per call, which is what
// the macro does by default.
#[tool_handler(router = self.tool_router)]
impl ServerHandler for ClawgMcp {
    fn get_info(&self) -> ServerConfig {
        // Built through the constructor, not a struct literal: `ServerInfo` is
        // `#[non_exhaustive]`, so a literal would break on any rmcp release
        // that adds a field. Assignment afterwards is unaffected.
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        // The profile's own name, so a dev bridge and a prod bridge are two
        // distinguishable servers in the host's list rather than one shadowing
        // the other.
        info.server_info =
            Implementation::new(self.profile.mcp_server_name(), env!("CARGO_PKG_VERSION"));
        info.instructions = Some(INSTRUCTIONS.to_string());
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use clawg_core::memory::bootstrap::EmbedderStatus;
    use clawg_core::runtime::MemoryRuntime;

    use crate::client::{ErrorDetails, ListArgs, SaveArgs, SaveOutcome, SavedMemory};
    use crate::daemon::api::{router, AppCtx, SAVE_CONFIRM_TIMEOUT};
    use crate::mcp::tools::CONTENT_PREVIEW_LIMIT;

    /// Brings up an in-process daemon and returns a bridge pointed at it.
    async fn bridge() -> (ClawgMcp, tempfile::TempDir) {
        bridge_with(|runtime| runtime).await
    }

    /// [`bridge`] over a runtime that `adjust` changed first.
    async fn bridge_with(adjust: impl FnOnce(MemoryRuntime) -> MemoryRuntime) -> (ClawgMcp, tempfile::TempDir) {
        bridge_at(Some(test_project()), adjust).await
    }

    /// [`bridge_with`] for a bridge whose own project is `project`.
    async fn bridge_at(
        project: Option<ProjectIdentity>,
        adjust: impl FnOnce(MemoryRuntime) -> MemoryRuntime,
    ) -> (ClawgMcp, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let (runtime, rx) = MemoryRuntime::new_for_test(dir.path().to_path_buf()).await;
        let runtime = adjust(runtime);
        tokio::spawn(runtime.ingest_worker(rx, tokio::sync::watch::channel(None).1));

        let token = "mcp-test-token".to_string();
        let (shutdown, _rx) = tokio::sync::watch::channel(false);
        let ctx = Arc::new(AppCtx {
            runtime,
            token: token.clone(),
            profile: Profile::Dev,
            started_at: Instant::now(),
            shutdown,
            save_timeout: SAVE_CONFIRM_TIMEOUT,
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr: SocketAddr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(ctx)).await;
        });

        let client = DaemonClient::new(
            format!("http://127.0.0.1:{}", addr.port()),
            token,
            dir.path().to_path_buf(),
        );
        (
            ClawgMcp::with_project(client, Profile::Dev, project),
            dir,
        )
    }

    /// The default project of [`bridge`]: a root that is already canonical, so
    /// there is no legacy id to move.
    fn test_project() -> ProjectIdentity {
        ProjectIdentity { id: "proj-test".into(), root: "/proj-test".into(), legacy_id: "proj-test".into() }
    }

    /// A bridge pointed at an address where nothing is listening.
    fn dead_bridge() -> ClawgMcp {
        ClawgMcp::with_project(
            DaemonClient::new("http://127.0.0.1:1", "token", std::env::temp_dir()),
            Profile::Dev,
            None,
        )
    }

    fn text(result: &CallToolResult) -> String {
        result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default()
    }

    fn is_error(result: &CallToolResult) -> bool {
        result.is_error.unwrap_or(false)
    }

    // ── project identity ────────────────────────────────────────────────────

    /// A repository with a `src` directory, by canonical name, and the
    /// identity a bridge started in `src` resolves to.
    fn repo_with_src() -> (tempfile::TempDir, ProjectIdentity) {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = std::fs::canonicalize(dir.path()).expect("canonicalize").join("repo");
        std::fs::create_dir_all(repo.join(".git")).expect(".git");
        std::fs::create_dir_all(repo.join("src")).expect("src");
        let identity = ProjectIdentity::resolve(&repo.join("src"));
        assert_ne!(identity.legacy_id, identity.id, "test setup: src must have its own legacy id");
        (dir, identity)
    }

    async fn rows_in(b: &ClawgMcp, project: &str) -> usize {
        let client = b.daemon().await.expect("daemon");
        client
            .list(ListArgs { kind: Some("memory".into()), project: Some(project.into()), ..Default::default() })
            .await
            .expect("list")
            .len()
    }

    fn save_args(content: &str) -> SaveMemoryArgs {
        SaveMemoryArgs { content: content.into(), memory_type: None, importance: None, project_path: None }
    }

    #[tokio::test]
    async fn a_bridge_in_a_subdirectory_brings_what_was_saved_there_to_the_root() {
        let (_repo, identity) = repo_with_src();
        let (b, _d) = bridge_at(Some(identity.clone()), |r| r).await;
        let client = b.daemon().await.expect("daemon");
        client
            .save(SaveArgs {
                content: "saved before paths were resolved".into(),
                agent_id: "a".into(),
                project_id: Some(identity.legacy_id.clone()),
                memory_type: None,
                importance: None,
            })
            .await
            .expect("seed a legacy row");

        let saved = b.save_memory_impl("a", save_args("saved after")).await.unwrap();
        assert!(!is_error(&saved), "{}", text(&saved));

        assert_eq!(rows_in(&b, &identity.id).await, 2);
        assert_eq!(rows_in(&b, &identity.legacy_id).await, 0);
    }

    #[tokio::test]
    async fn a_root_is_registered_once_per_bridge() {
        let (_repo, identity) = repo_with_src();
        let (b, _d) = bridge_at(Some(identity), |r| r).await;

        for content in ["one", "two"] {
            b.save_memory_impl("a", save_args(content)).await.unwrap();
        }
        b.search_memory_impl(
            "a",
            SearchMemoryArgs { query: "one".into(), limit: None, project_path: None, all_projects: None },
        )
        .await
        .unwrap();

        assert_eq!(b.registered.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn a_refused_registration_does_not_block_the_save() {
        let (_repo, identity) = repo_with_src();
        let store = std::sync::Mutex::new(None);
        let (b, _d) = bridge_at(Some(identity.clone()), |runtime| {
            *store.lock().unwrap() = Some(runtime.store.clone());
            runtime
        })
        .await;
        let store = store.into_inner().unwrap().expect("store");
        // Another root already owns this id, so the daemon answers 409.
        let id = identity.id.clone();
        store
            .conn
            .call(move |c| {
                c.execute("INSERT INTO project (id, root_path, created_at) VALUES (?1, '/elsewhere', 0)", [id])
            })
            .await
            .expect("seed a colliding project");

        let saved = b.save_memory_impl("a", save_args("still saved")).await.unwrap();

        assert!(!is_error(&saved), "{}", text(&saved));
        assert_eq!(rows_in(&b, &identity.id).await, 1);
    }

    // ── agent identity ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn agent_id_falls_back_when_no_peer_has_identified_itself() {
        let (b, _d) = bridge().await;
        assert_eq!(b.agent_id_for(None), FALLBACK_AGENT_ID);
    }

    // ── get_info ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn get_info_advertises_tools_and_the_profiles_server_name() {
        let (b, _d) = bridge().await;
        let info = b.get_info();
        assert!(info.capabilities.tools.is_some(), "the host must be told there are tools");
        assert_eq!(info.server_info.name, Profile::Dev.mcp_server_name());
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
        assert!(
            info.instructions.as_deref().is_some_and(|i| i.contains("save_memory")),
            "instructions must tell the model when to reach for the tools"
        );
    }

    #[tokio::test]
    async fn every_tool_is_registered_under_its_wire_name() {
        let (b, _d) = bridge().await;
        let names: Vec<String> = b
            .tool_router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        for expected in [
            "save_memory",
            "search_memory",
            "recall_context",
            "list_memories",
            "delete_memory",
            "resolve_conflict",
            "memory_status",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}: {names:?}");
        }
        assert_eq!(names.len(), 7, "no extra tools: {names:?}");
    }

    /// The memory viewer (`clawg memories`) is for the person at the terminal.
    /// Handing a model its filters by agent and project, or its text search over
    /// every row, would let it read across agents; so the tool a model sees keeps
    /// exactly its five arguments, and the set of tools does not grow.
    #[tokio::test]
    async fn the_viewer_is_not_exposed_to_the_model() {
        let (b, _d) = bridge().await;
        let tools = b.tool_router.list_all();

        let mut names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "delete_memory",
                "list_memories",
                "memory_status",
                "recall_context",
                "resolve_conflict",
                "save_memory",
                "search_memory",
            ]
        );

        let list = tools.iter().find(|t| t.name == "list_memories").expect("list_memories");
        let mut properties: Vec<&str> = list
            .input_schema
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("the schema has properties")
            .keys()
            .map(String::as_str)
            .collect();
        properties.sort();
        assert_eq!(properties, ["kind", "limit", "memory_type", "offset", "tier"]);
    }

    /// The model has to pick `memory_type` from the set the store recognises;
    /// an invented string is stored verbatim and then never matches a filter.
    /// The doc comments on `SaveMemoryArgs` are the only place that set reaches
    /// it, so the generated schema has to actually carry them.
    #[tokio::test]
    async fn the_save_memory_schema_names_the_accepted_memory_types() {
        let (b, _d) = bridge().await;
        let tool = b
            .tool_router
            .list_all()
            .into_iter()
            .find(|t| t.name == "save_memory")
            .expect("save_memory must be registered");
        let schema = serde_json::to_string(&tool.input_schema).expect("schema");
        for value in [
            "semantic_fact",
            "user_preference",
            "architectural_decision",
            "session_summary",
            "code_block",
            "chat_message",
            "tool_error",
        ] {
            assert!(schema.contains(value), "schema does not mention {value}: {schema}");
        }
    }

    /// A tool with no description is a tool the model will not reach for.
    #[tokio::test]
    async fn every_tool_carries_a_description() {
        let (b, _d) = bridge().await;
        for t in b.tool_router.list_all() {
            let d = t.description.clone().unwrap_or_default();
            assert!(d.len() > 40, "{} has a thin description: {d:?}", t.name);
        }
    }

    fn tool(b: &ClawgMcp, name: &str) -> rmcp::model::Tool {
        b.tool_router
            .list_all()
            .into_iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} must be registered"))
    }

    fn description(b: &ClawgMcp, name: &str) -> String {
        tool(b, name).description.map(|d| d.to_string()).unwrap_or_default()
    }

    /// An explicit save with no type is stored as `semantic_fact`
    /// (`NewMemoryRecord::from_event_and_chunk`). A schema that names another
    /// default teaches the model to pass a type it did not mean.
    #[tokio::test]
    async fn the_save_memory_schema_states_the_default_type_the_store_applies() {
        let (b, _d) = bridge().await;
        let schema = serde_json::to_string(&tool(&b, "save_memory").input_schema).expect("schema");
        assert!(schema.contains("Defaults to semantic_fact"), "schema: {schema}");
        assert!(!schema.contains("Defaults to a plain chat message"), "schema: {schema}");
    }

    /// A long save is split into chunks stored as unrelated rows, and no tool
    /// can put them back together; one fact per call is the only way the model
    /// gets back what it saved.
    #[tokio::test]
    async fn save_memory_asks_for_one_fact_per_call() {
        let (b, _d) = bridge().await;
        let d = description(&b, "save_memory");
        assert!(d.contains("one fact"), "description: {d}");
    }

    /// Retrieval is semantic: measured on the golden set, keyword queries find
    /// fewer expected memories than the full question does.
    #[tokio::test]
    async fn search_memory_asks_for_a_question_rather_than_keywords() {
        let (b, _d) = bridge().await;
        let d = description(&b, "search_memory");
        assert!(d.contains("natural-language question"), "description: {d}");
        assert!(d.contains("not keywords"), "description: {d}");
    }

    /// `recall_context` returns several times the tokens of `search_memory` for
    /// the same query, so it must not be presented as the default lookup.
    #[tokio::test]
    async fn recall_context_is_not_offered_as_the_default_lookup() {
        let (b, _d) = bridge().await;
        let d = description(&b, "recall_context");
        assert!(!d.contains("Prefer this over"), "description: {d}");
        assert!(d.contains("costs several times"), "description: {d}");
    }

    /// A short list is not a sign that memory is thin; the model has to know
    /// the cut is by relevance or it reads a two-line answer as "nothing more".
    #[tokio::test]
    async fn search_memory_says_it_shows_only_the_hits_close_to_the_best() {
        let (b, _d) = bridge().await;
        let d = description(&b, "search_memory");
        assert!(d.contains("close in relevance to the best match"), "description: {d}");
    }

    /// The model never sees more than a preview of each hit, and there is no
    /// tool that fetches a memory by id; promising otherwise sends it looking.
    #[tokio::test]
    async fn search_memory_states_the_preview_limit_it_applies() {
        let (b, _d) = bridge().await;
        let d = description(&b, "search_memory");
        assert!(d.contains(&format!("{CONTENT_PREVIEW_LIMIT} characters")), "description: {d}");
    }

    // ── end to end against a real daemon ────────────────────────────────────

    #[tokio::test]
    async fn save_memory_returns_the_id_that_delete_memory_accepts() {
        let (b, _d) = bridge().await;
        let content = "the team deploys on tuesdays and never on fridays";

        let saved = b
            .save_memory_impl(
                "agent-mcp",
                SaveMemoryArgs {
                    content: content.into(),
                    memory_type: Some("semantic_fact".into()),
                    importance: Some(8.0),
                    project_path: None,
                },
            )
            .await
            .expect("save");
        assert!(!is_error(&saved), "save failed: {}", text(&saved));
        let id: i64 = text(&saved)
            .strip_prefix("saved #")
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("expected `saved #<id>`, got: {}", text(&saved)));

        let listed = b
            .list_memories_impl(ListMemoriesArgs {
                kind: Some("memory".into()),
                tier: None,
                memory_type: None,
                limit: None,
                offset: None,
            })
            .await
            .expect("list");
        let listed = text(&listed);
        assert!(listed.contains(content), "the saved memory must be listable at once: {listed}");
        assert!(listed.contains(&format!("#{id}")), "the listing must show the saved id: {listed}");

        let found = b
            .search_memory_impl(
                "agent-mcp",
                SearchMemoryArgs {
                    query: content.into(),
                    limit: Some(5),
                    project_path: None,
                    all_projects: Some(true),
                },
            )
            .await
            .expect("search");
        assert!(!is_error(&found), "search failed: {}", text(&found));
        assert!(text(&found).contains(content), "text was: {}", text(&found));

        let deleted = b
            .delete_memory_impl(DeleteMemoryArgs { id, kind: "memory".into() })
            .await
            .expect("delete");
        assert!(!is_error(&deleted), "delete failed: {}", text(&deleted));
        assert!(text(&deleted).contains("deleted"), "text was: {}", text(&deleted));
    }

    #[tokio::test]
    async fn save_memory_is_an_error_when_the_embedder_failed() {
        let (b, _d) = bridge_with(|runtime| MemoryRuntime {
            embedder: Arc::new(once_cell::sync::OnceCell::new()),
            embedder_status: tokio::sync::watch::Sender::new(EmbedderStatus::Failed {
                error: "download failed: connection refused".into(),
                retryable: true,
            }),
            ..runtime
        })
        .await;

        let r = b
            .save_memory_impl(
                "agent-mcp",
                SaveMemoryArgs {
                    content: "not stored".into(),
                    memory_type: None,
                    importance: None,
                    project_path: None,
                },
            )
            .await
            .expect("save");

        assert!(is_error(&r), "text was: {}", text(&r));
        assert!(text(&r).contains("connection refused"), "text was: {}", text(&r));
    }

    fn daemon_error(status: u16, message: &str, details: ErrorDetails) -> ClientError {
        ClientError::Daemon { status, message: message.into(), details }
    }

    fn saved(id: i64, outcome: &str) -> SavedMemory {
        SavedMemory { kind: "memory".into(), id, outcome: outcome.into() }
    }

    #[test]
    fn save_outcome_text_covers_every_daemon_answer() {
        let text = |r| ClawgMcp::save_outcome_text(r);
        assert_eq!(
            text(Ok(SaveOutcome::Saved { memories: vec![saved(57, "inserted")] })),
            Ok("saved #57".to_string())
        );
        assert_eq!(
            text(Ok(SaveOutcome::Saved {
                memories: vec![saved(57, "inserted"), saved(58, "inserted"), saved(59, "reinforced")]
            })),
            Ok("saved as 3 pieces: #57, #58, #59".to_string())
        );
        assert_eq!(
            text(Ok(SaveOutcome::Saved { memories: vec![saved(57, "reinforced")] })),
            Ok("already stored as #57; reinforced it instead of saving a copy".to_string())
        );
        assert_eq!(
            text(Ok(SaveOutcome::Queued { reason: Some("unconfirmed".into()) })),
            Ok("accepted but not confirmed yet; check with list_memories before relying on it".to_string())
        );
        assert_eq!(
            text(Ok(SaveOutcome::Queued { reason: Some("embedder_loading".into()) })),
            Ok("accepted; it will be stored once the embedding model finishes loading".to_string())
        );
        assert_eq!(
            text(Err(daemon_error(
                503,
                "embedder_failed",
                ErrorDetails { detail: Some("connection refused".into()), retryable: Some(true), memories: vec![] },
            ))),
            Err("not saved: the embedding model failed to load (connection refused); clawg will retry".to_string())
        );
        assert_eq!(
            text(Err(daemon_error(
                503,
                "embedder_failed",
                ErrorDetails { detail: Some("contract mismatch".into()), retryable: Some(false), memories: vec![] },
            ))),
            Err("not saved: the embedding model failed to load (contract mismatch)".to_string())
        );
        assert_eq!(
            text(Err(daemon_error(
                500,
                "ingest_failed",
                ErrorDetails {
                    detail: Some("store insert failed: disk full".into()),
                    retryable: None,
                    memories: vec![saved(57, "inserted")],
                },
            ))),
            Err("not saved: store insert failed: disk full; partially saved as #57".to_string())
        );
        assert_eq!(
            text(Err(daemon_error(503, "embedder_unavailable", ErrorDetails::default()))),
            Err("not saved: embedder_unavailable".to_string())
        );
        assert_eq!(
            text(Err(daemon_error(503, "ingest_queue_full", ErrorDetails::default()))),
            Err("not saved: the ingest queue is full; try again shortly".to_string())
        );
        for answer in [
            text(Ok(SaveOutcome::Queued { reason: None })),
            text(Ok(SaveOutcome::Queued { reason: Some("unconfirmed".into()) })),
        ] {
            let t = answer.unwrap();
            assert!(!t.split(|c: char| !c.is_alphanumeric()).any(|w| w == "queued"), "{t}");
        }
    }

    #[test]
    fn a_daemon_that_did_not_start_points_at_status() {
        let t = ClawgMcp::explain(&ClientError::StartTimeout(std::time::Duration::from_secs(30)));
        assert!(t.contains("clawg status"), "{t}");
        assert!(!t.contains("clawg serve"), "{t}");

        let t = ClawgMcp::explain(&ClientError::NotRunning);
        assert!(t.contains("clawg serve"), "{t}");
    }

    #[test]
    fn explain_does_not_promise_search_when_the_embedder_failed() {
        let failed = daemon_error(
            503,
            "embedder_failed",
            ErrorDetails { detail: Some("connection refused".into()), retryable: Some(true), memories: vec![] },
        );
        let t = ClawgMcp::explain(&failed);
        assert!(!t.contains("in a minute"), "{t}");
        assert!(t.contains("connection refused"), "{t}");

        let loading = daemon_error(503, "embedder_not_ready", ErrorDetails::default());
        assert!(ClawgMcp::explain(&loading).contains("loading"));
    }

    #[tokio::test]
    async fn recall_context_answers_even_with_nothing_stored() {
        let (b, _d) = bridge().await;
        let r = b
            .recall_context_impl(
                "agent-mcp",
                RecallContextArgs { query: "anything".into(), project_path: None },
            )
            .await
            .expect("recall");
        assert!(!is_error(&r), "text was: {}", text(&r));
        assert!(!text(&r).is_empty(), "an empty string is never a useful tool answer");
    }

    #[tokio::test]
    async fn search_with_nothing_stored_says_so_without_failing() {
        let (b, _d) = bridge().await;
        let r = b
            .search_memory_impl(
                "agent-mcp",
                SearchMemoryArgs {
                    query: "nothing here".into(),
                    limit: None,
                    project_path: None,
                    all_projects: Some(true),
                },
            )
            .await
            .expect("search");
        assert!(!is_error(&r), "an empty corpus is not an error");
        assert_eq!(text(&r), "no memories found");
    }

    #[tokio::test]
    async fn memory_status_reports_the_daemon_and_the_profile() {
        let (b, _d) = bridge().await;
        let r = b.memory_status_impl().await.expect("status");
        assert!(!is_error(&r), "text was: {}", text(&r));
        let t = text(&r);
        assert!(t.contains("\"status\""), "the health JSON must be there: {t}");
        assert!(t.contains("dev"), "the profile must be named: {t}");
    }

    #[tokio::test]
    async fn deleting_a_missing_row_is_an_error_result_the_model_can_read() {
        let (b, _d) = bridge().await;
        let r = b
            .delete_memory_impl(DeleteMemoryArgs { id: 999_999, kind: "memory".into() })
            .await
            .expect("the call itself must succeed");
        assert!(is_error(&r), "a failed delete must be marked as an error");
        assert!(!text(&r).is_empty(), "an error result must explain itself");
    }

    #[tokio::test]
    async fn resolve_conflict_reports_the_outcome() {
        let mut store = None;
        let (b, _d) = bridge_with(|runtime| {
            store = Some(runtime.store.clone());
            runtime
        })
        .await;
        let mut ids = Vec::new();
        for content in ["the deploy runs on fridays", "the deploy runs on mondays"] {
            let saved = b
                .save_memory_impl(
                    "agent-mcp",
                    SaveMemoryArgs { content: content.into(), memory_type: None, importance: None, project_path: None },
                )
                .await
                .expect("save");
            ids.push(text(&saved).strip_prefix("saved #").and_then(|n| n.parse::<i64>().ok()).expect("an id"));
        }
        let (old, new) = (ids[0], ids[1]);
        store
            .expect("the runtime's store")
            .conn
            .call(move |c| {
                c.execute(
                    &format!("UPDATE memory SET status = 'disputed', disputed_against = '[{old}]' WHERE id = {new}"),
                    [],
                )
            })
            .await
            .expect("mark the dispute");

        let r = b
            .resolve_conflict_impl(ResolveConflictArgs { keep: new, other: old, keep_both: None })
            .await
            .expect("call");

        assert!(!is_error(&r), "resolve failed: {}", text(&r));
        assert_eq!(text(&r), format!("#{new} active\n#{old} superseded by #{new}"));
    }

    #[tokio::test]
    async fn resolve_conflict_reports_a_refusal() {
        let (b, _d) = bridge().await;
        let r = b
            .resolve_conflict_impl(ResolveConflictArgs { keep: 4, other: 4, keep_both: Some(true) })
            .await
            .expect("call");
        assert!(is_error(&r));
        assert!(text(&r).contains("itself"), "text was: {}", text(&r));
    }

    #[tokio::test]
    async fn an_unknown_kind_is_reported_as_a_readable_error() {
        let (b, _d) = bridge().await;
        let r = b
            .delete_memory_impl(DeleteMemoryArgs { id: 1, kind: "bogus".into() })
            .await
            .expect("call");
        assert!(is_error(&r));
        assert!(text(&r).contains("bogus"), "text was: {}", text(&r));
    }

    /// The failure a user actually hits: the daemon died, or never started.
    /// It must arrive as advice, not as a transport stack trace.
    #[tokio::test]
    async fn a_dead_daemon_is_explained_rather_than_dumped() {
        let b = dead_bridge();
        let r = b.memory_status_impl().await.expect("call");
        assert!(is_error(&r));
        assert!(
            text(&r).contains("clawg serve"),
            "the message must name the fix: {}",
            text(&r)
        );
    }

    // ── project scoping ─────────────────────────────────────────────────────

    /// `all_projects` is the only way to reach memories saved elsewhere, so it
    /// has to actually clear the scope rather than be quietly ignored.
    #[tokio::test]
    async fn all_projects_widens_a_search_beyond_the_bridges_own_project() {
        let (b, _d) = bridge().await;
        let content = "a memory saved under a completely different project";

        b.save_memory_impl(
            "agent-scope",
            SaveMemoryArgs {
                content: content.into(),
                memory_type: None,
                importance: None,
                project_path: Some("/some/elsewhere".into()),
            },
        )
        .await
        .expect("save");

        let mut found = false;
        for _ in 0..40 {
            let r = b
                .search_memory_impl(
                    "agent-scope",
                    SearchMemoryArgs {
                        query: content.into(),
                        limit: None,
                        project_path: None,
                        all_projects: Some(true),
                    },
                )
                .await
                .expect("search");
            if text(&r).contains(content) {
                found = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(found, "all_projects must see a memory saved under another project");

        // The same query scoped to this bridge's own project must not see it.
        let scoped = b
            .search_memory_impl(
                "agent-scope",
                SearchMemoryArgs {
                    query: content.into(),
                    limit: None,
                    project_path: None,
                    all_projects: Some(false),
                },
            )
            .await
            .expect("search");
        assert!(
            !text(&scoped).contains(content),
            "a project-scoped search must not leak another project's memory: {}",
            text(&scoped)
        );
    }
}
