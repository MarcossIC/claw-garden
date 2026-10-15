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
//! rmcp resolves the calling client from a `RequestContext`, which no test can
//! construct (its `Peer` is crate-private). So each `#[tool]` method does
//! exactly two things — resolve the agent id from the request, then delegate —
//! and all the behaviour lives in an `*_impl` taking that id explicitly, where
//! a test can drive it against an in-process daemon.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use clawg_core::memory::sharing;
use clawg_core::memory::ProjectIdentity;
use clawg_core::paths::Profile;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, DiscoverResult, Implementation, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, RoleServer, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::client::{
    ClientError, DaemonClient, ListArgs, SaveArgs, SaveOutcome, SearchArgs, SynthesisArgs,
};
use crate::mcp::startup::ClientHandle;
use crate::mcp::tools::{
    format_docs, format_recall, format_resolution, format_rows, format_save, format_search, format_sharing,
    format_restore, format_status, format_synthesis, format_update, resolve_project,
};

/// Agent id used when neither `--agent` nor the MCP client named one.
///
/// Memories are scoped per agent, so this is not cosmetic: everything saved
/// under it is retrievable under it, and a client that changes its reported
/// name effectively gets a new memory.
pub const FALLBACK_AGENT_ID: &str = "mcp";

/// Where the agent id of a call came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSource {
    /// `clawg mcp --agent`, set by the user in the host's MCP registration.
    Flag,
    /// The `clientInfo.name` the client declared.
    Client,
    /// Nothing named one.
    Fallback,
}

impl AgentSource {
    /// How `memory_status` explains the origin to the person reading it.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Flag => "from --agent",
            Self::Client => "reported by the client; pin it with --agent",
            Self::Fallback => "the client did not identify itself; pin one with --agent",
        }
    }
}

/// The agent a call reads and writes as: the flag, then the client's name,
/// then [`FALLBACK_AGENT_ID`].
///
/// The client's name is kept exactly as sent. Rows already carry it verbatim,
/// so normalizing it here would hide them.
pub fn resolve_agent(flag: Option<&str>, client: Option<&str>) -> (String, AgentSource) {
    if let Some(flag) = flag {
        return (flag.to_string(), AgentSource::Flag);
    }
    match client.filter(|name| !name.trim().is_empty()) {
        Some(name) => (name.to_string(), AgentSource::Client),
        None => (FALLBACK_AGENT_ID.to_string(), AgentSource::Fallback),
    }
}

/// The client name a request reads and writes under: the one it declares,
/// else the last one a client declared to this bridge. Blank names count as
/// none.
///
/// The second step is for the 2026-07-28 lifecycle, where a client names
/// itself per request and only SHOULD: a stdio bridge serves one client, so a
/// request that leaves the name out still comes from the one that gave it,
/// and its memories must not drop out of sight under the fallback.
fn client_name<'a>(declared: Option<&'a str>, remembered: Option<&'a str>) -> Option<&'a str> {
    [declared, remembered].into_iter().flatten().find(|name| !name.trim().is_empty())
}

/// What the host is told this server is for, at `initialize` or
/// `server/discover`.
const INSTRUCTIONS: &str = "\
Persistent memory for this user across sessions. Use `save_memory` whenever the \
user states a durable fact, preference or decision; use `search_memory` before \
answering anything that depends on earlier sessions, and `recall_context` once \
when a broad task needs background. When it lists ACTIVE CONFLICTS, ask the user \
which memory holds and record the answer with `resolve_conflict`. When \
`recall_context` says memories are waiting for a review, or the user asks what you \
have learned, call `synthesize` and save the conclusions it leads to. \
Memories are scoped to the current project unless you say otherwise. For how \
this project works (its design, conventions, commands or where something is \
implemented), call `search_project` before opening files: it searches the \
project's own documentation and code.";

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
    /// Only for a conclusion: the ids of the memories it generalizes, as
    /// `synthesize` lists them. Omit for an ordinary memory.
    #[serde(default)]
    pub derived_from: Option<Vec<i64>>,
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
pub struct SearchProjectArgs {
    /// What to find out about the project, as a natural-language question or
    /// the name of something in it.
    pub query: String,
    /// How many passages to return, 1 to 20. Defaults to 5.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Absolute path of the project. Defaults to the directory this bridge was
    /// started in.
    #[serde(default)]
    pub project_path: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct SynthesizeArgs {
    /// How many memories to review, 1 to 100. Defaults to 30.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Absolute path of the project to review. Defaults to the directory this
    /// bridge was started in.
    #[serde(default)]
    pub project_path: Option<String>,
    /// Review every project at once, for conclusions about the user rather
    /// than about one codebase.
    #[serde(default)]
    pub all_projects: Option<bool>,
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
pub struct RestoreMemoryArgs {
    /// The id `delete_memory` was given.
    pub id: i64,
    /// The kind `delete_memory` was given. Only memory rows can be restored.
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DeleteMemoryArgs {
    /// The id of the row to delete, as printed by `search_memory`,
    /// `list_memories` or `recall_context`. A `recall_context` hit with no kind
    /// in front of its type is a memory.
    pub id: i64,
    /// Which table the id belongs to: memory, episodic or procedural. Required,
    /// because each table numbers its rows independently — id 7 exists in all
    /// three and means something different in each.
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ReadMemoryArgs {
    /// The id of any piece of the save, as printed by `search_memory`,
    /// `list_memories`, `recall_context` or `save_memory`.
    pub id: i64,
    /// Which table the id belongs to: memory, episodic or procedural.
    /// Defaults to memory, the only kind saved in pieces.
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UpdateMemoryArgs {
    /// The id of the row to change, as printed by `save_memory`,
    /// `search_memory`, `list_memories` or `recall_context`. A `recall_context`
    /// hit with no kind in front of its type is a memory.
    pub id: i64,
    /// Which table the id belongs to: memory, episodic or procedural.
    pub kind: String,
    /// The new importance, 0 to 10. Omit it to leave the importance as it is.
    #[serde(default)]
    pub importance: Option<f64>,
    /// The new type, only for kind memory. One of: semantic_fact,
    /// user_preference, architectural_decision, session_summary, code_block,
    /// chat_message, tool_error. Omit it to leave the type as it is.
    #[serde(default)]
    pub memory_type: Option<String>,
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
    /// `--agent`: wins over whatever the peer declares.
    configured_agent: Option<String>,
    /// The last name a client declared to this bridge. Shared by every clone,
    /// because rmcp clones the server per call.
    declared: Arc<std::sync::Mutex<Option<String>>>,
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
            configured_agent: None,
            declared: Arc::default(),
            default_project,
            registered: Arc::default(),
            profile,
            tool_router: Self::tool_router(),
        }
    }

    /// Pins the agent id every call uses, whatever the peer declares.
    pub fn with_agent(mut self, agent: Option<String>) -> Self {
        self.configured_agent = agent;
        self
    }

    /// Which agent's memory a call reads and writes, and why.
    ///
    /// Resolved per call rather than once at construction: the client names
    /// itself in `initialize` or in each request's `_meta`, both after the
    /// server is built.
    fn agent_for(&self, ctx: Option<&RequestContext<RoleServer>>) -> (String, AgentSource) {
        let declared = ctx.and_then(|ctx| self.remember(ctx));
        let remembered = self.declared.lock().expect("declared lock").clone();
        let client = client_name(declared.as_deref(), remembered.as_deref());
        resolve_agent(self.configured_agent.as_deref(), client)
    }

    fn agent_id_for(&self, ctx: Option<&RequestContext<RoleServer>>) -> String {
        self.agent_for(ctx).0
    }

    /// Records the name `ctx`'s client declares, if any, and returns it.
    fn remember(&self, ctx: &RequestContext<RoleServer>) -> Option<String> {
        let name = ctx.client_info().map(|info| info.name).filter(|n| !n.trim().is_empty())?;
        *self.declared.lock().expect("declared lock") = Some(name.clone());
        Some(name)
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
            Ok(SaveOutcome::Saved { memories, redacted }) => {
                let mut text = match memories.as_slice() {
                    [one] if one.outcome == "reinforced" => format!(
                        "already stored as #{}; reinforced it instead of saving a copy \
                         (update_memory changes its importance or type)",
                        one.id
                    ),
                    [one] => format!("saved #{}", one.id),
                    many => format!(
                        "saved as {} pieces: {}; read_memory with any of them returns the whole text",
                        many.len(),
                        ids(many)
                    ),
                };
                if !redacted.is_empty() {
                    text.push_str(&format!("; masked {} before storing it", redacted.join(", ")));
                }
                Ok(text)
            }
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
                derived_from: args.derived_from.unwrap_or_default(),
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
            Ok(hits) => Self::ok(format_search(hits, agent_id)),
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
            Ok(context) => Self::ok(format_recall(&context)),
            Err(e) => Self::failed(Self::explain(&e)),
        }
    }

    /// Lists the calling agent's rows only, across all of its projects: the
    /// agent is the visibility boundary, the project is not.
    pub async fn list_memories_impl(
        &self,
        agent_id: &str,
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
                agent: Some(agent_id.to_string()),
                ..Default::default()
            })
            .await
        {
            Ok(rows) => Self::ok(format_rows(&rows)),
            Err(e) => Self::failed(Self::explain(&e)),
        }
    }

    /// Another agent's row is refused as missing, like an id that is not there.
    pub async fn delete_memory_impl(
        &self,
        agent_id: &str,
        args: DeleteMemoryArgs,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        match client.delete(&args.kind, args.id, Some(agent_id)).await {
            Ok(n) => Self::ok(format!("deleted {n}; restore_memory with the same id and kind undoes it")),
            Err(e) => Self::failed(Self::explain(&e)),
        }
    }

    /// Another agent's row is refused as missing, as in `delete_memory_impl`.
    pub async fn restore_memory_impl(
        &self,
        agent_id: &str,
        args: RestoreMemoryArgs,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        match client.restore(&args.kind, args.id, Some(agent_id)).await {
            Ok(restored) => Self::ok(format_restore(&restored)),
            Err(e) => Self::failed(Self::route_failure_text(&e, "restore a memory")),
        }
    }

    /// Another agent's save is refused as missing, as in `delete_memory_impl`.
    pub async fn read_memory_impl(
        &self,
        agent_id: &str,
        args: ReadMemoryArgs,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        match client.read_save(args.kind.as_deref().unwrap_or("memory"), args.id, Some(agent_id)).await {
            Ok(save) => Self::ok(format_save(&save)),
            Err(e) => Self::failed(Self::route_failure_text(&e, "read a save")),
        }
    }

    /// A daemon older than the route answers its own bare 404, with no error
    /// text of the daemon's; telling the model the row is missing would be
    /// false.
    fn route_failure_text(err: &ClientError, what: &str) -> String {
        match err {
            ClientError::Daemon { status: 404, message, .. } if message == "404 Not Found" => {
                format!(
                    "the running clawg daemon cannot {what}; restart it with the installed clawg \
                     (`clawg stop`, then call again)"
                )
            }
            other => Self::explain(other),
        }
    }

    /// Another agent's row is refused as missing, as in `delete_memory_impl`.
    pub async fn update_memory_impl(
        &self,
        agent_id: &str,
        args: UpdateMemoryArgs,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        match client
            .update(&args.kind, args.id, args.importance, args.memory_type.as_deref(), Some(agent_id))
            .await
        {
            Ok(updated) => Self::ok(format_update(&updated)),
            Err(e) => Self::failed(Self::explain(&e)),
        }
    }

    /// Both rows must be the calling agent's, as in `delete_memory_impl`.
    pub async fn resolve_conflict_impl(
        &self,
        agent_id: &str,
        args: ResolveConflictArgs,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        match client.resolve(args.keep, args.other, args.keep_both.unwrap_or(false), Some(agent_id)).await {
            Ok(resolution) => Self::ok(format_resolution(&resolution)),
            Err(e) => Self::failed(Self::explain(&e)),
        }
    }

    /// A project's documentation belongs to the project, not to an agent, so
    /// no agent narrows it.
    pub async fn search_project_impl(&self, args: SearchProjectArgs) -> Result<CallToolResult, rmcp::ErrorData> {
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        let Some(project_id) = self.project_for(&client, args.project_path.as_deref(), false).await else {
            return Self::failed("no project to search: pass project_path");
        };
        match client.search_docs(&project_id, &args.query, args.limit).await {
            Ok(found) => Self::ok(format_docs(&found)),
            // The daemon's own 404 names the project; a bare one is a daemon
            // that predates the route.
            Err(ClientError::Daemon { status: 404, message, .. }) if message.starts_with("404") => Self::failed(
                "the running clawg daemon is older than this bridge and cannot search project docs; \
                 restart it with `clawg stop` and call the tool again",
            ),
            Err(e) => Self::failed(Self::explain(&e)),
        }
    }

    /// Only the calling agent's memories, as in `list_memories_impl`.
    pub async fn synthesize_impl(
        &self,
        agent_id: &str,
        args: SynthesizeArgs,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        let project_id = self
            .project_for(&client, args.project_path.as_deref(), args.all_projects.unwrap_or(false))
            .await;
        match client.synthesis(&SynthesisArgs { agent_id: Some(agent_id.into()), project_id, limit: args.limit }).await {
            Ok(synthesis) => Self::ok(format_synthesis(&synthesis)),
            Err(e) => Self::failed(Self::explain(&e)),
        }
    }

    pub async fn memory_status_impl(
        &self,
        (agent_id, source): (String, AgentSource),
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let client = match self.daemon().await {
            Ok(c) => c,
            Err(failure) => return Ok(failure),
        };
        // Read here rather than asked of the daemon: both read the same file
        // in the same data directory, so this is what the next search will use.
        let sharing = format_sharing(&sharing::load(client.base_dir()), &agent_id);
        match client.health().await {
            Ok(health) => Self::ok(format_status(
                &health,
                self.profile,
                client.base_dir(),
                &agent_id,
                source.describe(),
                &sharing,
            )),
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
                       into pieces that search finds one at a time, and read_memory returns it \
                       whole. Write the content so it still makes sense months from now, with no \
                       conversation around it. Returns the id of each stored piece, which \
                       update_memory and delete_memory accept, or why nothing was stored."
    )]
    async fn save_memory(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(args): Parameters<SaveMemoryArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.save_memory_impl(&self.agent_id_for(Some(&ctx)), args).await
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
                       Content longer than 500 characters is truncated. A hit marked piece=K/N \
                       is the best matching piece K of a long text saved as N pieces; \
                       read_memory with its id returns the whole text."
    )]
    async fn search_memory(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(args): Parameters<SearchMemoryArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.search_memory_impl(&self.agent_id_for(Some(&ctx)), args).await
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
        ctx: RequestContext<RoleServer>,
        Parameters(args): Parameters<RecallContextArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.recall_context_impl(&self.agent_id_for(Some(&ctx)), args).await
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
        ctx: RequestContext<RoleServer>,
        Parameters(args): Parameters<ListMemoriesArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.list_memories_impl(&self.agent_id_for(Some(&ctx)), args).await
    }

    #[tool(
        name = "read_memory",
        description = "Read the whole text of a memory by id. A long text is saved as several \
                       pieces, and search shows one of them, marked piece=K/N, cut at 500 \
                       characters; pass its id to get every piece joined in order. Returns which \
                       piece the id is, the ids of the save (a deleted piece shows as missing) \
                       and the text."
    )]
    async fn read_memory(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(args): Parameters<ReadMemoryArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.read_memory_impl(&self.agent_id_for(Some(&ctx)), args).await
    }

    #[tool(
        name = "delete_memory",
        description = "Delete one stored memory by id and kind. Get the id from \
                       `search_memory`, `list_memories` or `recall_context` first — ids are \
                       per-kind, so the same number exists in all three tables. Only your own \
                       memories: one marked `by <agent>` was saved by another agent and is \
                       read-only. `restore_memory` undoes it."
    )]
    async fn delete_memory(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(args): Parameters<DeleteMemoryArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.delete_memory_impl(&self.agent_id_for(Some(&ctx)), args).await
    }

    #[tool(
        name = "restore_memory",
        description = "Undo a `delete_memory`: bring a memory you deleted back, with the id and \
                       kind you deleted it with, when the user says the delete was a mistake. \
                       Refused when the same text was saved again since, or when the memory holds \
                       a secret. Only your own memories: one marked `by <agent>` is read-only."
    )]
    async fn restore_memory(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(args): Parameters<RestoreMemoryArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.restore_memory_impl(&self.agent_id_for(Some(&ctx)), args).await
    }

    #[tool(
        name = "update_memory",
        description = "Correct the importance or the type of one stored memory, by id and kind, \
                       when the user says it matters more or less than it was saved with, or \
                       that it is a different kind of memory. Pass only what changes. To change \
                       what a memory says, save the new text and delete the old one instead. \
                       Returns the memory's importance, type and tier after the change. Only \
                       your own memories: one marked `by <agent>` is read-only."
    )]
    async fn update_memory(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(args): Parameters<UpdateMemoryArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.update_memory_impl(&self.agent_id_for(Some(&ctx)), args).await
    }

    #[tool(
        name = "resolve_conflict",
        description = "Settle a conflict listed under ACTIVE CONFLICTS by `recall_context`, once \
                       the user has said which memory is right. `keep` stays; `other` is \
                       superseded by it and stops being retrieved, but is still listed and comes \
                       back if you resolve the pair the other way. Pass keep_both when the two \
                       do not really contradict each other. Returns the status of both memories. \
                       Only your own memories: one marked `by <agent>` is read-only."
    )]
    async fn resolve_conflict(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(args): Parameters<ResolveConflictArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.resolve_conflict_impl(&self.agent_id_for(Some(&ctx)), args).await
    }

    #[tool(
        name = "synthesize",
        description = "Review stored memories for conclusions: the memories no conclusion covers \
                       yet, newest first, the conclusions already drawn, and how to save new \
                       ones. A conclusion is saved with `save_memory` and `derived_from`; the \
                       memories it rests on stay as they are. Use it when `recall_context` says \
                       memories are waiting, at the end of a long task, or when the user asks \
                       what you have learned. Calling it marks the memories as reviewed, even \
                       if you save nothing."
    )]
    async fn synthesize(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(args): Parameters<SynthesizeArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.synthesize_impl(&self.agent_id_for(Some(&ctx)), args).await
    }

    #[tool(
        name = "search_project",
        description = "Search this project's own documentation (README, docs, CLAUDE.md, design \
                       notes, specs and manifests) and its Rust, Python and TypeScript code for \
                       how it works: its design, conventions, commands, decisions and where \
                       something is implemented. Use it before opening files when the question \
                       is how something in the project works or why it is the way it is. \
                       Matching is by meaning and by exact term, so pass a question or the name \
                       of the thing. Returns numbered passages, each under the file and the \
                       headings or definitions it comes from; the index follows the files on disk."
    )]
    async fn search_project(
        &self,
        Parameters(args): Parameters<SearchProjectArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.search_project_impl(args).await
    }

    #[tool(
        name = "memory_status",
        description = "Report whether the memory daemon is running, which profile and data \
                       directory it is using, and whether its embedding model has finished \
                       loading. Use this when a memory tool fails and you need to tell the \
                       user why."
    )]
    async fn memory_status(&self, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, rmcp::ErrorData> {
        self.memory_status_impl(self.agent_for(Some(&ctx))).await
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

    /// rmcp's own answer, plus remembering who asked: a 2026-07-28 client may
    /// name itself here and leave the name out of later requests.
    async fn discover(&self, context: RequestContext<RoleServer>) -> Result<DiscoverResult, rmcp::ErrorData> {
        self.remember(&context);
        Ok(DiscoverResult::from_server_info(
            self.supported_protocol_versions().into_owned(),
            self.get_info(),
        ))
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
        SaveMemoryArgs { content: content.into(), memory_type: None, importance: None, project_path: None, derived_from: None }
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
                derived_from: Vec::new(),
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
    async fn search_project_lists_each_hit_under_its_breadcrumb() {
        let (_repo, identity) = repo_with_src();
        let root = std::path::PathBuf::from(&identity.root);
        std::fs::write(root.join("README.md"), "# Repo\nHow the repo is laid out.\n## Build\nRun make.\n")
            .expect("readme");
        let (b, _d) = bridge_at(Some(identity), |r| r).await;

        let found = b
            .search_project_impl(SearchProjectArgs { query: "How the repo is laid out".into(), limit: None, project_path: None })
            .await
            .unwrap();

        let out = text(&found);
        assert!(!is_error(&found), "{out}");
        assert!(out.starts_with("[1] README.md > Repo\nHow the repo is laid out."), "{out}");
        assert!(out.contains("[2] README.md > Repo > Build\nRun make."), "{out}");
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

    #[test]
    fn the_name_the_request_declares_wins() {
        assert_eq!(client_name(Some("claude-code"), Some("earlier")), Some("claude-code"));
    }

    #[test]
    fn a_request_that_declares_no_name_keeps_the_one_declared_earlier() {
        assert_eq!(client_name(None, Some("claude-code")), Some("claude-code"));
    }

    #[test]
    fn a_blank_name_counts_as_none() {
        assert_eq!(client_name(Some("  "), Some("claude-code")), Some("claude-code"));
        assert_eq!(client_name(Some(""), None), None);
    }

    #[tokio::test]
    async fn agent_id_falls_back_when_no_client_has_identified_itself() {
        let (b, _d) = bridge().await;
        assert_eq!(b.agent_id_for(None), FALLBACK_AGENT_ID);
    }

    // ── agent identity over the wire ────────────────────────────────────────

    type HostLines = tokio::io::Lines<tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>;

    /// One MCP session against `b`, as a host would open it over stdio.
    struct Session {
        write: tokio::io::WriteHalf<tokio::io::DuplexStream>,
        lines: HostLines,
        next_id: u32,
    }

    impl Session {
        fn open(b: &ClawgMcp) -> Self {
            use rmcp::transport::async_rw::AsyncRwTransport;
            use rmcp::ServiceExt;
            use tokio::io::AsyncBufReadExt;

            let (host, server) = tokio::io::duplex(64 * 1024);
            let (server_read, server_write) = tokio::io::split(server);
            let bridge = b.clone();
            tokio::spawn(async move {
                if let Ok(running) = bridge.serve(AsyncRwTransport::new_server(server_read, server_write)).await {
                    let _ = running.waiting().await;
                }
            });
            let (host_read, write) = tokio::io::split(host);
            Self { write, lines: tokio::io::BufReader::new(host_read).lines(), next_id: 1 }
        }

        async fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
            use tokio::io::AsyncWriteExt;

            let id = self.next_id;
            self.next_id += 1;
            let frame = serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
            self.write.write_all(format!("{frame}\n").as_bytes()).await.expect("host write");
            let line = tokio::time::timeout(Duration::from_secs(10), self.lines.next_line())
                .await
                .expect("the bridge answers within 10 s")
                .expect("readable stream")
                .expect("the bridge is still alive");
            let reply: serde_json::Value = serde_json::from_str(&line).expect("a JSON-RPC line");
            assert_eq!(reply["id"], id, "{reply}");
            reply
        }

        async fn initialize(&mut self, client: &str) {
            let reply = self
                .request(
                    "initialize",
                    serde_json::json!({
                        "protocolVersion": "2025-06-18",
                        "capabilities": {},
                        "clientInfo": { "name": client, "version": "0" }
                    }),
                )
                .await;
            assert!(reply["result"].is_object(), "{reply}");
        }

        /// A tool call; `meta` is the request's `_meta`, absent in a legacy session.
        async fn call(&mut self, meta: Option<serde_json::Value>, tool: &str, arguments: serde_json::Value) -> String {
            let mut params = serde_json::json!({ "name": tool, "arguments": arguments });
            if let Some(meta) = meta {
                params["_meta"] = meta;
            }
            let reply = self.request("tools/call", params).await;
            assert_eq!(reply["result"]["isError"], false, "{reply}");
            reply["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
        }
    }

    /// The `_meta` a 2026-07-28 client puts on each request, naming itself as
    /// `client` or, with `None`, not at all.
    fn modern_meta(client: Option<&str>) -> serde_json::Value {
        let mut meta = serde_json::json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {}
        });
        if let Some(name) = client {
            meta["io.modelcontextprotocol/clientInfo"] = serde_json::json!({ "name": name, "version": "0" });
        }
        meta
    }

    async fn rows_of(b: &ClawgMcp, agent: &str) -> usize {
        let client = b.daemon().await.expect("daemon");
        client
            .list(ListArgs { kind: Some("memory".into()), agent: Some(agent.into()), ..Default::default() })
            .await
            .expect("list")
            .len()
    }

    #[tokio::test]
    async fn a_session_without_initialize_saves_under_the_client_it_declares() {
        let (b, _d) = bridge().await;
        let mut session = Session::open(&b);

        session
            .call(Some(modern_meta(Some("probe-host"))), "save_memory", serde_json::json!({ "content": "deploys happen on tuesdays" }))
            .await;

        assert_eq!(rows_of(&b, "probe-host").await, 1);
        assert_eq!(rows_of(&b, FALLBACK_AGENT_ID).await, 0);
    }

    #[tokio::test]
    async fn a_request_without_client_info_keeps_the_name_declared_on_discover() {
        let (b, _d) = bridge().await;
        let mut session = Session::open(&b);

        let discovered = session.request("server/discover", serde_json::json!({ "_meta": modern_meta(Some("probe-host")) })).await;
        assert!(discovered["result"].is_object(), "{discovered}");
        session
            .call(Some(modern_meta(None)), "save_memory", serde_json::json!({ "content": "deploys happen on tuesdays" }))
            .await;

        assert_eq!(rows_of(&b, "probe-host").await, 1);
    }

    /// Memories a client saved through the `initialize` handshake are found
    /// when the same client comes back without it.
    #[tokio::test]
    async fn memories_saved_through_initialize_stay_visible_without_it() {
        let (b, _d) = bridge().await;
        let content = "the team deploys on tuesdays and never on fridays";
        let mut legacy = Session::open(&b);
        legacy.initialize("probe-host").await;
        legacy.call(None, "save_memory", serde_json::json!({ "content": content })).await;

        let mut modern = Session::open(&b);
        let found = modern
            .call(Some(modern_meta(Some("probe-host"))), "search_memory", serde_json::json!({ "query": content }))
            .await;

        assert!(found.contains(content), "{found}");
    }

    #[test]
    fn the_configured_agent_wins_over_the_client() {
        assert_eq!(resolve_agent(Some("claude-code"), Some("Claude Code")), ("claude-code".into(), AgentSource::Flag));
        assert_eq!(resolve_agent(Some("claude-code"), None), ("claude-code".into(), AgentSource::Flag));
    }

    #[test]
    fn without_a_flag_the_client_name_wins_over_the_fallback() {
        assert_eq!(resolve_agent(None, Some("cursor")), ("cursor".into(), AgentSource::Client));
        assert_eq!(resolve_agent(None, Some(" Cursor ")), (" Cursor ".into(), AgentSource::Client), "kept as sent");
        assert_eq!(resolve_agent(None, Some("  ")), (FALLBACK_AGENT_ID.into(), AgentSource::Fallback));
        assert_eq!(resolve_agent(None, None), (FALLBACK_AGENT_ID.into(), AgentSource::Fallback));
    }

    /// The failure the flag exists for: a client that stops sending
    /// `clientInfo` must not lose sight of what it saved under its old name.
    #[tokio::test]
    async fn a_configured_agent_keeps_its_memories_without_client_info() {
        let (b, _d) = bridge().await;
        let b = b.with_agent(Some("claude-code".into()));
        let content = "the staging database is refreshed every sunday night";
        let client = b.daemon().await.expect("daemon");
        client
            .save(SaveArgs {
                content: content.into(),
                agent_id: "claude-code".into(),
                project_id: Some("proj-test".into()),
                memory_type: None,
                importance: None,
                derived_from: Vec::new(),
            })
            .await
            .expect("save");

        let agent = b.agent_id_for(None);
        assert_eq!(agent, "claude-code");
        let found = b
            .search_memory_impl(
                &agent,
                SearchMemoryArgs { query: content.into(), limit: None, project_path: None, all_projects: None },
            )
            .await
            .expect("search");
        assert!(text(&found).contains(content), "{}", text(&found));
    }

    #[tokio::test]
    async fn memory_status_names_the_agent_and_where_it_came_from() {
        let (b, _d) = bridge().await;
        let pinned = b.clone().with_agent(Some("claude-code".into()));

        let r = pinned.memory_status_impl(pinned.agent_for(None)).await.expect("status");
        assert!(text(&r).contains("agent: claude-code (from --agent)"), "{}", text(&r));

        let r = b.memory_status_impl(b.agent_for(None)).await.expect("status");
        assert!(text(&r).contains(&format!("agent: {FALLBACK_AGENT_ID} (")), "{}", text(&r));
        assert!(text(&r).contains("--agent"), "the fallback must say how to pin one: {}", text(&r));
    }

    /// Rows and a hand-written workspace as the previous version leaves them,
    /// under the name the client used to report: pinning that same name keeps
    /// every one of them reachable.
    #[tokio::test]
    async fn rows_and_workspace_of_a_client_name_survive_pinning_it() {
        let mut workspaces = None;
        let (b, _d) = bridge_with(|runtime| {
            workspaces = Some(runtime.workspaces());
            runtime
        })
        .await;
        let workspaces = workspaces.expect("runtime");
        let b = b.with_agent(Some("claude-code".into()));
        let client = b.daemon().await.expect("daemon");
        client
            .save(SaveArgs {
                content: "the release branch is cut on the first monday".into(),
                agent_id: "claude-code".into(),
                project_id: Some("proj-test".into()),
                memory_type: None,
                importance: None,
                derived_from: Vec::new(),
            })
            .await
            .expect("save");
        let path = workspaces.workspace_path("claude-code", Some("proj-test"));
        assert!(path.ends_with("claude-code/proj-test/workspace.md"), "the path must not move: {}", path.display());
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &path,
            "<!-- AUTO-START -->\n<!-- AUTO-END -->\n<!-- USER-START -->\nask before force-pushing\n<!-- USER-END -->\n",
        )
        .expect("workspace");

        let agent = b.agent_id_for(None);
        let listed = text(&b.list_memories_impl(&agent, list_all()).await.expect("list"));
        assert!(listed.contains("release branch"), "{listed}");
        let context = b
            .recall_context_impl(&agent, RecallContextArgs { query: "release branch".into(), project_path: None })
            .await
            .expect("recall");
        assert!(text(&context).contains("ask before force-pushing"), "{}", text(&context));
        assert!(text(&context).contains("release branch"), "{}", text(&context));
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
        assert!(
            info.instructions.as_deref().is_some_and(|i| i.contains("synthesize")),
            "instructions must say when to look for conclusions"
        );
    }

    #[tokio::test]
    async fn instructions_say_when_to_search_the_project() {
        let (b, _d) = bridge().await;
        let instructions = b.get_info().instructions.expect("instructions");
        assert!(instructions.contains("search_project"), "{instructions}");
        assert!(instructions.contains("how"), "{instructions}");
        assert!(instructions.contains("documentation and code"), "{instructions}");
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
            "update_memory",
            "resolve_conflict",
            "synthesize",
            "search_project",
            "memory_status",
            "read_memory",
            "restore_memory",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}: {names:?}");
        }
        assert_eq!(names.len(), 12, "no extra tools: {names:?}");
    }

    /// The memory viewer (`clawg memories`) is for the person at the terminal.
    /// Handing a model its filters by agent and project, or its text search over
    /// every row, would let it read across agents; so the tool a model sees keeps
    /// exactly its five arguments, and no other tool takes the viewer's filters.
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
                "read_memory",
                "recall_context",
                "resolve_conflict",
                "restore_memory",
                "save_memory",
                "search_memory",
                "search_project",
                "synthesize",
                "update_memory",
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

    /// A conclusion is only a conclusion if its sources reach the store, and
    /// the schema is the only place the model learns the field exists.
    #[tokio::test]
    async fn the_save_memory_schema_offers_sources() {
        let (b, _d) = bridge().await;
        let schema = serde_json::to_string(&tool(&b, "save_memory").input_schema).expect("schema");
        assert!(schema.contains("derived_from"), "{schema}");
        assert!(schema.contains("synthesize"), "the field must say where its ids come from: {schema}");
    }

    #[tokio::test]
    async fn synthesize_lists_what_no_conclusion_covers_and_recall_stops_reminding() {
        let (b, _d) = bridge().await;
        let foreign = b.save_memory_impl("agent-other", save_args("another agent's note about the build")).await.expect("save");
        let foreign = text(&foreign).strip_prefix("saved #").and_then(|n| n.parse::<i64>().ok()).expect("an id");
        let mut ids = Vec::new();
        for i in 0..crate::mcp::tools::SYNTHESIS_NUDGE_THRESHOLD {
            let saved = b.save_memory_impl("agent-mcp", save_args(&format!("fact number {i} about the build"))).await.expect("save");
            ids.push(text(&saved).strip_prefix("saved #").and_then(|n| n.parse::<i64>().ok()).expect("an id"));
        }
        let recall = || b.recall_context_impl("agent-mcp", RecallContextArgs { query: "the build".into(), project_path: None });
        assert!(text(&recall().await.expect("recall")).contains("`synthesize`"), "enough memories wait for a review");

        let listed = text(&b.synthesize_impl("agent-mcp", SynthesizeArgs::default()).await.expect("synthesize"));
        assert!(listed.contains(&format!("#{} ", ids[0])), "{listed}");
        assert!(!listed.contains(&format!("#{foreign} ")), "another agent's memory is not this one's to review: {listed}");
        assert!(!text(&recall().await.expect("recall")).contains("`synthesize`"), "the review turns the reminder off");

        let conclusion = SaveMemoryArgs {
            derived_from: Some(vec![ids[0], ids[1]]),
            memory_type: Some("architectural_decision".into()),
            ..save_args("the build is described by twenty separate facts")
        };
        let saved = b.save_memory_impl("agent-mcp", conclusion).await.expect("save");
        assert!(!is_error(&saved), "{}", text(&saved));
        let listed = text(&b.synthesize_impl("agent-mcp", SynthesizeArgs::default()).await.expect("synthesize"));
        assert!(listed.contains(&format!("from=#{},#{}", ids[0], ids[1])), "{listed}");
        assert!(!listed.contains(&format!("#{} fact", ids[0])), "a covered memory is no longer pending: {listed}");
    }

    #[tokio::test]
    async fn a_save_from_a_missing_source_reports_the_refusal() {
        let (b, _d) = bridge().await;
        let args = SaveMemoryArgs { derived_from: Some(vec![424242]), ..save_args("a conclusion from nothing") };
        let r = b.save_memory_impl("agent-mcp", args).await.expect("save");
        assert!(is_error(&r), "{}", text(&r));
        assert!(text(&r).contains("424242"), "{}", text(&r));
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

    /// `recall_context` prints `#id` before a `memory` hit's type and names
    /// any other kind; the tools that take an id still require the kind.
    #[tokio::test]
    async fn memory_id_docs_name_recall_context_and_the_default_kind() {
        let (b, _d) = bridge().await;
        for name in ["delete_memory", "update_memory"] {
            let schema = serde_json::to_string(&tool(&b, name).input_schema).expect("schema").replace("\\n", " ");
            assert!(schema.contains("recall_context"), "{name} schema: {schema}");
            assert!(schema.contains("no kind in front of its type is a memory"), "{name} schema: {schema}");
        }
        let d = description(&b, "delete_memory");
        assert!(d.contains("recall_context"), "description: {d}");
    }

    /// A long save is split into pieces that are searched one by one; one
    /// fact per call is still what search finds best, and `read_memory` is how
    /// the model gets back a long text whole.
    #[tokio::test]
    async fn save_memory_asks_for_one_fact_per_call() {
        let (b, _d) = bridge().await;
        let d = description(&b, "save_memory");
        assert!(d.contains("one fact"), "description: {d}");
        assert!(d.contains("read_memory"), "description: {d}");
        assert!(!d.contains("never reassembled"), "description: {d}");
    }

    #[tokio::test]
    async fn search_memory_explains_the_piece_of_a_long_save() {
        let (b, _d) = bridge().await;
        let d = description(&b, "search_memory");
        assert!(d.contains("piece=K/N"), "description: {d}");
        assert!(d.contains("read_memory"), "description: {d}");
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
                    derived_from: None,
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
            .list_memories_impl("agent-mcp", ListMemoriesArgs {
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
            .delete_memory_impl("agent-mcp", DeleteMemoryArgs { id, kind: "memory".into() })
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
                    derived_from: None,
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
    fn a_masked_save_tells_the_agent_what_was_masked() {
        let text = |r| ClawgMcp::save_outcome_text(r);
        assert_eq!(
            text(Ok(SaveOutcome::Saved {
                memories: vec![saved(57, "inserted")],
                redacted: vec!["slack_token".into()],
            })),
            Ok("saved #57; masked slack_token before storing it".to_string())
        );
        assert_eq!(
            text(Ok(SaveOutcome::Saved {
                memories: vec![saved(57, "reinforced")],
                redacted: vec!["github_token".into(), "jwt".into()],
            })),
            Ok("already stored as #57; reinforced it instead of saving a copy \
                (update_memory changes its importance or type); \
                masked github_token, jwt before storing it"
                .to_string())
        );
    }

    #[test]
    fn save_outcome_text_covers_every_daemon_answer() {
        let text = |r| ClawgMcp::save_outcome_text(r);
        assert_eq!(
            text(Ok(SaveOutcome::Saved { memories: vec![saved(57, "inserted")], redacted: vec![] })),
            Ok("saved #57".to_string())
        );
        assert_eq!(
            text(Ok(SaveOutcome::Saved {
                memories: vec![saved(57, "inserted"), saved(58, "inserted"), saved(59, "reinforced")],
                redacted: vec![],
            })),
            Ok("saved as 3 pieces: #57, #58, #59; read_memory with any of them returns the whole text".to_string())
        );
        assert_eq!(
            text(Ok(SaveOutcome::Saved { memories: vec![saved(57, "reinforced")], redacted: vec![] })),
            Ok("already stored as #57; reinforced it instead of saving a copy \
                (update_memory changes its importance or type)"
                .to_string())
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
        let r = b.memory_status_impl(b.agent_for(None)).await.expect("status");
        assert!(!is_error(&r), "text was: {}", text(&r));
        let t = text(&r);
        assert!(t.contains("\"status\""), "the health JSON must be there: {t}");
        assert!(t.contains("dev"), "the profile must be named: {t}");
    }

    #[tokio::test]
    async fn deleting_a_missing_row_is_an_error_result_the_model_can_read() {
        let (b, _d) = bridge().await;
        let r = b
            .delete_memory_impl("agent-mcp", DeleteMemoryArgs { id: 999_999, kind: "memory".into() })
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
                    SaveMemoryArgs { content: content.into(), memory_type: None, importance: None, project_path: None, derived_from: None },
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
            .resolve_conflict_impl("agent-mcp", ResolveConflictArgs { keep: new, other: old, keep_both: None })
            .await
            .expect("call");

        assert!(!is_error(&r), "resolve failed: {}", text(&r));
        assert_eq!(text(&r), format!("#{new} active\n#{old} superseded by #{new}"));
    }

    #[tokio::test]
    async fn resolve_conflict_reports_a_refusal() {
        let (b, _d) = bridge().await;
        let r = b
            .resolve_conflict_impl("agent-mcp", ResolveConflictArgs { keep: 4, other: 4, keep_both: Some(true) })
            .await
            .expect("call");
        assert!(is_error(&r));
        assert!(text(&r).contains("itself"), "text was: {}", text(&r));
    }

    #[tokio::test]
    async fn an_unknown_kind_is_reported_as_a_readable_error() {
        let (b, _d) = bridge().await;
        let r = b
            .delete_memory_impl("agent-mcp", DeleteMemoryArgs { id: 1, kind: "bogus".into() })
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
        let r = b.memory_status_impl(b.agent_for(None)).await.expect("call");
        assert!(is_error(&r));
        assert!(
            text(&r).contains("clawg serve"),
            "the message must name the fix: {}",
            text(&r)
        );
    }

    // ── agent scope ─────────────────────────────────────────────────────────

    async fn saved_id(b: &ClawgMcp, agent: &str, content: &str) -> i64 {
        let saved = b.save_memory_impl(agent, save_args(content)).await.expect("save");
        text(&saved)
            .strip_prefix("saved #")
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("expected `saved #<id>`, got: {}", text(&saved)))
    }

    fn list_all() -> ListMemoriesArgs {
        ListMemoriesArgs { kind: None, tier: None, memory_type: None, limit: None, offset: None }
    }

    #[tokio::test]
    async fn an_agent_cannot_delete_another_agents_memory() {
        let (b, _d) = bridge().await;
        let content = "agent a keeps its release notes in the wiki";
        let id = saved_id(&b, "agent-a", content).await;

        let r = b
            .delete_memory_impl("agent-b", DeleteMemoryArgs { id, kind: "memory".into() })
            .await
            .expect("call");
        assert!(is_error(&r), "another agent's memory must not be deletable: {}", text(&r));

        let listed = b.list_memories_impl("agent-a", list_all()).await.expect("list");
        assert!(text(&listed).contains(content), "the row must survive: {}", text(&listed));

        let r = b
            .delete_memory_impl("agent-a", DeleteMemoryArgs { id, kind: "memory".into() })
            .await
            .expect("call");
        assert!(!is_error(&r), "the owner deletes it: {}", text(&r));
    }

    fn restore_args(id: i64) -> RestoreMemoryArgs {
        RestoreMemoryArgs { id, kind: "memory".into() }
    }

    #[tokio::test]
    async fn restore_memory_undoes_an_own_delete() {
        let (b, _d) = bridge().await;
        let content = "agent a keeps its release notes in the wiki";
        let id = saved_id(&b, "agent-a", content).await;

        let deleted = b.delete_memory_impl("agent-a", DeleteMemoryArgs { id, kind: "memory".into() }).await.expect("call");
        assert!(text(&deleted).contains("restore_memory"), "the delete says how to undo it: {}", text(&deleted));
        assert!(!text(&b.list_memories_impl("agent-a", list_all()).await.expect("list")).contains(content));

        let r = b.restore_memory_impl("agent-a", restore_args(id)).await.expect("call");
        assert!(!is_error(&r), "{}", text(&r));
        assert!(text(&r).contains(&format!("#{id}")), "{}", text(&r));
        let listed = b.list_memories_impl("agent-a", list_all()).await.expect("list");
        assert!(text(&listed).contains(content), "it is back: {}", text(&listed));

        let again = b.restore_memory_impl("agent-a", restore_args(id)).await.expect("call");
        assert!(is_error(&again), "a live row has nothing to restore: {}", text(&again));
    }

    #[tokio::test]
    async fn an_agent_cannot_restore_another_agents_memory() {
        let (b, _d) = bridge().await;
        let content = "agent a keeps its release notes in the wiki";
        let id = saved_id(&b, "agent-a", content).await;
        b.delete_memory_impl("agent-a", DeleteMemoryArgs { id, kind: "memory".into() }).await.expect("call");

        let r = b.restore_memory_impl("agent-b", restore_args(id)).await.expect("call");

        assert!(is_error(&r), "another agent's memory must not be restorable: {}", text(&r));
        assert!(!text(&b.list_memories_impl("agent-a", list_all()).await.expect("list")).contains(content));
        let r = b.restore_memory_impl("agent-a", restore_args(id)).await.expect("call");
        assert!(!is_error(&r), "the owner restores it: {}", text(&r));
    }

    #[tokio::test]
    async fn delete_memory_names_the_way_back() {
        let (b, _d) = bridge().await;
        let d = description(&b, "delete_memory");
        assert!(d.contains("restore_memory"), "description: {d}");
    }

    fn read_args(id: i64) -> ReadMemoryArgs {
        ReadMemoryArgs { id, kind: None }
    }

    /// The ids `save_memory` printed for a save of several pieces.
    async fn saved_pieces(b: &ClawgMcp, agent: &str, content: &str) -> Vec<i64> {
        let saved = text(&b.save_memory_impl(agent, save_args(content)).await.expect("save"));
        let list = saved.split_once(": ").map(|(_, rest)| rest).unwrap_or_default();
        let list = list.split(';').next().unwrap_or_default();
        list.split(", ").filter_map(|id| id.trim_start_matches('#').parse().ok()).collect()
    }

    #[tokio::test]
    async fn read_memory_returns_the_whole_save() {
        let (b, _d) = bridge().await;
        let content = (0..600).map(|i| format!("w{i:05}")).collect::<Vec<_>>().join(" ");
        let ids = saved_pieces(&b, "agent-a", &content).await;
        assert_eq!(ids.len(), 15, "the fixture is a save of fifteen pieces");

        let r = b.read_memory_impl("agent-a", read_args(ids[1])).await.expect("call");

        assert!(!is_error(&r), "{}", text(&r));
        let listed = ids.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(", ");
        assert_eq!(text(&r), format!("#{} is piece 2 of 15 of one save: {listed}\n{content}", ids[1]));
    }

    #[tokio::test]
    async fn an_agent_cannot_read_another_agents_save() {
        let (b, _d) = bridge().await;
        let id = saved_id(&b, "agent-a", "agent a keeps its release notes in the wiki").await;

        let other = b.read_memory_impl("agent-b", read_args(id)).await.expect("call");
        let missing = b.read_memory_impl("agent-b", read_args(999_999)).await.expect("call");

        assert!(is_error(&other), "another agent's save must not be readable: {}", text(&other));
        assert_eq!(text(&other).replace(&id.to_string(), "N"), text(&missing).replace("999999", "N"));
        let own = b.read_memory_impl("agent-a", read_args(id)).await.expect("call");
        assert_eq!(text(&own), format!("#{id}\nagent a keeps its release notes in the wiki"));
    }

    #[test]
    fn a_daemon_without_the_restore_route_is_named_as_such() {
        let old = daemon_error(404, "404 Not Found", ErrorDetails::default());
        let text = ClawgMcp::route_failure_text(&old, "restore a memory");
        assert!(text.contains("cannot restore a memory") && text.contains("restart"), "{text}");
        let missing = daemon_error(404, "no Memory row with id 7", ErrorDetails::default());
        assert_eq!(ClawgMcp::route_failure_text(&missing, "restore a memory"), ClawgMcp::explain(&missing));
    }

    #[test]
    fn a_daemon_without_the_save_route_is_named_as_such() {
        let old = daemon_error(404, "404 Not Found", ErrorDetails::default());
        let text = ClawgMcp::route_failure_text(&old, "read a save");
        assert!(text.contains("cannot read a save") && text.contains("restart"), "{text}");
        let missing = daemon_error(404, "no Memory row with id 7", ErrorDetails::default());
        assert_eq!(ClawgMcp::route_failure_text(&missing, "read a save"), ClawgMcp::explain(&missing));
    }

    fn update_args(id: i64, importance: Option<f64>, memory_type: Option<&str>) -> UpdateMemoryArgs {
        UpdateMemoryArgs { id, kind: "memory".into(), importance, memory_type: memory_type.map(Into::into) }
    }

    #[tokio::test]
    async fn update_memory_changes_an_own_memory_and_says_how_it_ended() {
        let (b, _d) = bridge().await;
        let id = saved_id(&b, "agent-a", "the deploy script lives in ops/deploy.sh").await;

        let r = b.update_memory_impl("agent-a", update_args(id, Some(2.0), Some("user_preference"))).await.expect("call");

        assert!(!is_error(&r), "{}", text(&r));
        assert_eq!(text(&r), format!("updated #{id} memory: user_preference, importance 2.0, tier 1"));
        let listed = text(&b.list_memories_impl("agent-a", list_all()).await.expect("list"));
        assert!(listed.contains(&format!("#{id} memory user_preference t1 imp=2.0")), "{listed}");

        let r = b.update_memory_impl("agent-a", update_args(id, Some(11.0), None)).await.expect("call");
        assert!(is_error(&r), "{}", text(&r));
        assert!(text(&r).contains("importance"), "{}", text(&r));
    }

    #[tokio::test]
    async fn an_agent_cannot_update_another_agents_memory() {
        let (b, _d) = bridge().await;
        let id = saved_id(&b, "agent-a", "agent a reviews pull requests on mondays").await;

        let r = b.update_memory_impl("agent-b", update_args(id, Some(9.5), None)).await.expect("call");

        assert!(is_error(&r), "another agent's memory must not change: {}", text(&r));
        let listed = text(&b.list_memories_impl("agent-a", list_all()).await.expect("list"));
        assert!(listed.contains("imp=5.0"), "the row must stay as it was: {listed}");
    }

    #[tokio::test]
    async fn an_agent_lists_only_its_own_memories() {
        let (b, _d) = bridge().await;
        saved_id(&b, "agent-a", "agent a prefers tabs over spaces").await;
        saved_id(&b, "agent-b", "agent b prefers spaces over tabs").await;

        let listed = text(&b.list_memories_impl("agent-b", list_all()).await.expect("list"));

        assert!(listed.contains("agent b prefers"), "{listed}");
        assert!(!listed.contains("agent a prefers"), "another agent's memory leaked: {listed}");
    }

    #[tokio::test]
    async fn an_agent_cannot_resolve_another_agents_conflict() {
        let mut store = None;
        let (b, _d) = bridge_with(|runtime| {
            store = Some(runtime.store.clone());
            runtime
        })
        .await;
        let old = saved_id(&b, "agent-a", "the cache expires after one hour").await;
        let new = saved_id(&b, "agent-a", "the cache expires after one day").await;
        let store = store.expect("the runtime's store");
        store
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
            .resolve_conflict_impl("agent-b", ResolveConflictArgs { keep: new, other: old, keep_both: None })
            .await
            .expect("call");

        assert!(is_error(&r), "another agent's conflict must not be resolvable: {}", text(&r));
        let client = b.daemon().await.expect("daemon");
        let mut rows = client
            .list(ListArgs { kind: Some("memory".into()), ..Default::default() })
            .await
            .expect("list");
        rows.sort_by_key(|r| r.id);
        let statuses: Vec<&str> = rows.iter().map(|r| r.status.as_str()).collect();
        assert_eq!(statuses, ["active", "disputed"], "a refused resolution changes nothing");
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
                derived_from: None,
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

    // ── sharing between agents ──

    fn hide(dir: &tempfile::TempDir, reader: &str, author: &str) {
        let mut policy = sharing::load(dir.path()).expect("policy");
        policy.hide(reader, author);
        sharing::save(dir.path(), &policy).expect("save policy");
    }

    fn search_args(query: &str) -> SearchMemoryArgs {
        SearchMemoryArgs { query: query.into(), limit: None, project_path: None, all_projects: None }
    }

    #[tokio::test]
    async fn search_memory_shows_another_agents_memory_with_its_author() {
        let (b, d) = bridge().await;
        let content = "claude-code learned that the release branch is cut on thursdays";
        let id = saved_id(&b, "claude-code", content).await;

        let shared = text(&b.search_memory_impl("cursor", search_args(content)).await.expect("search"));
        assert!(shared.contains(&format!("#{id} [Memory/")), "{shared}");
        assert!(shared.contains(&format!(" by claude-code] {content}")), "shared without a policy: {shared}");
        assert!(shared.ends_with(clawg_core::memory::prompt_builder::SHARED_NOTE), "{shared}");

        let own = text(&b.search_memory_impl("claude-code", search_args(content)).await.expect("search"));
        assert!(!own.contains(" by "), "the author's own view is unchanged: {own}");

        hide(&d, "cursor", "claude-code");
        let hidden = text(&b.search_memory_impl("cursor", search_args(content)).await.expect("search"));
        assert!(!hidden.contains(content), "hidden from cursor: {hidden}");
    }

    #[tokio::test]
    async fn recall_context_shows_another_agents_memory_with_its_author() {
        let (b, d) = bridge().await;
        let content = "claude-code learned that the staging database is rebuilt every night";
        saved_id(&b, "claude-code", content).await;
        let recall = || {
            let b = &b;
            async move {
                text(&b.recall_context_impl("cursor", RecallContextArgs { query: content.into(), project_path: None }).await.expect("recall"))
            }
        };

        let t = recall().await;
        assert!(t.contains(&format!(" by claude-code] {content}")), "{t}");
        assert!(t.contains("=== SHARED ==="), "{t}");

        hide(&d, "*", "claude-code");
        assert!(!recall().await.contains(content), "no agent reads claude-code");
    }

    #[tokio::test]
    async fn a_shared_memory_is_read_only() {
        let (b, _d) = bridge().await;
        let content = "claude-code keeps the incident runbook in docs/runbook.md";
        let id = saved_id(&b, "claude-code", content).await;
        assert!(text(&b.search_memory_impl("cursor", search_args(content)).await.expect("search")).contains(content));

        let r = b.delete_memory_impl("cursor", DeleteMemoryArgs { id, kind: "memory".into() }).await.expect("call");
        assert!(is_error(&r), "a shared memory must not be deletable by its reader: {}", text(&r));
        let r = b.update_memory_impl("cursor", update_args(id, Some(9.5), None)).await.expect("call");
        assert!(is_error(&r), "nor changed: {}", text(&r));

        let listed = text(&b.list_memories_impl("claude-code", list_all()).await.expect("list"));
        assert!(listed.contains("imp=5.0") && listed.contains(&format!("#{id} ")), "the row stays as it was: {listed}");
        assert!(!text(&b.list_memories_impl("cursor", list_all()).await.expect("list")).contains(content), "and is not listed as cursor's");
    }

    #[tokio::test]
    async fn tool_descriptions_say_shared_memories_are_read_only() {
        let (b, _d) = bridge().await;
        for name in ["delete_memory", "update_memory", "resolve_conflict", "restore_memory"] {
            let tool = b.tool_router.list_all().into_iter().find(|t| t.name == name).expect(name);
            let description = tool.description.as_deref().unwrap_or_default();
            assert!(description.contains("Only your own memories") && description.contains("read-only"), "{name}: {description}");
        }
    }

    #[tokio::test]
    async fn memory_status_reports_the_sharing_of_the_agent() {
        let (b, d) = bridge().await;
        let status = |agent: &'static str| {
            let b = &b;
            async move { text(&b.memory_status_impl((agent.to_string(), AgentSource::Flag)).await.expect("status")) }
        };
        assert!(status("cursor").await.ends_with("shared: reads every agent"));
        hide(&d, "cursor", "claude-code");
        hide(&d, "cursor", "codex");
        assert!(status("cursor").await.ends_with("shared: reads every agent but claude-code, codex"));
        hide(&d, "cursor", "*");
        assert!(status("cursor").await.ends_with("shared: none (reads only its own)"));
        std::fs::write(clawg_core::paths::sharing_file(d.path()), "{").unwrap();
        let t = status("cursor").await;
        assert!(t.contains("shared: none (sharing.json is not valid"), "{t}");
    }

    /// Rows written the way the previous version writes them — `scope =
    /// 'agent'`, the author in `agent_id` — are shared by the policy alone,
    /// and come out of hiding and unhiding exactly as they went in.
    #[tokio::test]
    async fn rows_of_the_previous_version_are_shared_untouched() {
        use clawg_core::memory::embedder::{EmbedRole, Embedder, FakeEmbedder};
        let mut store = None;
        let (b, d) = bridge_with(|runtime| {
            store = Some(runtime.store.clone());
            runtime
        })
        .await;
        let store = store.expect("store");
        let content = "the previous version saved that payments run on stripe";
        let embedding = FakeEmbedder::default().embed_with_role(content, EmbedRole::Passage).await.expect("embed");
        let bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
        let snapshot = |store: Arc<clawg_core::memory::MemoryStore>| async move {
            store
                .conn
                .call(|c| {
                    let mut stmt = c.prepare("SELECT id, scope, agent_id, project_id, status, content FROM memory ORDER BY id")?;
                    let rows = stmt.query_map([], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, Option<String>>(3)?,
                            r.get::<_, String>(4)?,
                            r.get::<_, String>(5)?,
                        ))
                    })?;
                    rows.collect::<Result<Vec<_>, _>>()
                })
                .await
                .expect("snapshot")
        };
        for (agent, row_text) in [("claude-code", content), ("cursor", "cursor keeps its own notes")] {
            let (agent, row_text, bytes) = (agent.to_string(), row_text.to_string(), bytes.clone());
            store
                .conn
                .call(move |c| {
                    c.execute(
                        "INSERT INTO memory (scope, agent_id, content, memory_type, source, created_at, created_by_type, \
                         created_by_ref, last_modified_at, last_accessed_at, importance, tier, status) \
                         VALUES ('agent', ?1, ?2, 'semantic_fact', 'chat', 0, 'user', ?1, 0, 0, 5.0, 3, 'active')",
                        (&agent, &row_text),
                    )
                    .and_then(|_| {
                        c.execute(
                            "INSERT INTO memory_vectors(rowid, embedding) VALUES ((SELECT max(id) FROM memory), ?1)",
                            (&bytes,),
                        )
                    })
                })
                .await
                .expect("seed");
        }
        let before = snapshot(store.clone()).await;

        let finds = || {
            let b = &b;
            async move { text(&b.search_memory_impl("cursor", search_args(content)).await.expect("search")) }
        };
        assert!(finds().await.contains(&format!(" by claude-code] {content}")), "shared with its author without a policy");
        hide(&d, "cursor", "claude-code");
        assert!(!finds().await.contains(content), "hidden");
        let mut policy = sharing::load(d.path()).unwrap();
        policy.unhide("cursor", "claude-code");
        sharing::save(d.path(), &policy).unwrap();
        assert!(finds().await.contains(content), "unhidden");

        assert_eq!(snapshot(store).await, before, "no row was rewritten by hiding or unhiding");
    }
}
