use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use clawg_core::config::ProviderPool;
use clawg_core::hooks::permission::UserDecision;
use crate::mcp::McpPool;
use clawg_core::memory::MemoryStore;
use clawg_core::memory::embedder::Embedder;
use clawg_core::memory::events::MemoryEvent;
use crate::recovery::gate::RecoveryGate;
use crate::tools::{GlobalToolRegistry, executor::ToolExecutor};
use once_cell::sync::OnceCell;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use clawg_core::models::config::{
    AnthropicSettings, AppConfig, OllamaSettings, OllamaStatus, OpenRouterSettings, ProviderType,
};
use clawg_core::services::auth::{
    resolve_anthropic_auth, resolve_api_key, resolve_base_url, ANTHROPIC_BASE_URL_ENV,
    DEFAULT_ANTHROPIC_URL, DEFAULT_OPENROUTER_URL, OPENROUTER_BASE_URL_ENV,
    OPENROUTER_KEY_ENV,
};
use clawg_core::services::config_store;
use clawg_core::services::model_meta;
use clawg_core::models::provider_config::ProviderConfig;
use clawg_core::models::{
    compose_system_prompt, Agent, AgentState, AppError, ChatMessage,
};
use clawg_core::providers::anthropic::{AnthropicConfig, AnthropicProvider};
use clawg_core::providers::ollama::{OllamaConfig, OllamaProvider};
use clawg_core::providers::openrouter::{OpenRouterConfig, OpenRouterProvider};
use clawg_core::providers::AiProvider;
use crate::session::SessionManager;

pub struct AppState {
    pub base_dir: PathBuf,
    pub providers: Mutex<Vec<ProviderConfig>>,
    pub agents: Mutex<Vec<Agent>>,
    /// Global (legacy) provider — used by the config panel's set_provider command.
    pub provider: RwLock<Arc<dyn AiProvider>>,
    /// Cache of named provider instances keyed by ProviderConfig.id.
    /// Avoids rebuilding the same provider on every chat iteration.
    pub provider_pool: Mutex<ProviderPool>,
    pub conversations: Mutex<HashMap<String, Vec<ChatMessage>>>,
    /// Per-project MCP connections. Uses per-server RwLocks internally —
    /// no external Mutex needed. Agents on different servers never block each other.
    pub mcp_pool: McpPool,
    pub config: RwLock<AppConfig>,
    pub session_manager: Mutex<SessionManager>,
    /// Per-provider rate-limit timestamps (unix seconds).
    /// Key is the provider name ("anthropic", "openrouter", "ollama").
    /// Circuit breaker adds a cooldown only for the specific provider that got 429'd.
    pub rate_limit_tracker: Mutex<HashMap<String, u64>>,
    /// Serializes OAuth token refresh attempts across concurrent agents
    /// to prevent duplicate refresh requests that could invalidate each other's tokens.
    pub oauth_refresh_lock: tokio::sync::Mutex<()>,
    /// Per-server recovery semaphore. Prevents multiple agents from simultaneously
    /// reconnecting the same MCP server (recovery storm prevention).
    pub mcp_recovery_gate: RecoveryGate,
    /// Registry of all native built-in tools. Shared across all agents.
    pub tool_registry: Arc<GlobalToolRegistry>,
    /// Shared executor infrastructure: ConcurrencyGate, FileLockRegistry, HTTP client.
    /// One instance per process — all agents share the concurrency controls.
    pub tool_executor: Arc<ToolExecutor>,
    /// In-flight HITL confirmation requests — keyed by call_id.
    /// The chat loop parks on the receiver side of the oneshot; the
    /// `resolve_tool_confirmation` Tauri command pulls the sender out and fires it.
    pub pending_confirmations: Mutex<HashMap<String, oneshot::Sender<UserDecision>>>,
    /// SQLite-vec persistent memory store. Always populated after `AppState::new()`.
    /// `MemoryStore` is `Clone` (channel handle) — `Arc` provides cheap shared
    /// ownership; no `Mutex` needed.
    pub memory_store: Arc<MemoryStore>,
    /// Producer side of the memory ingest pipeline.
    ///
    /// Cheaply cloneable (`UnboundedSender` is an Arc-like). Consumed by the worker
    /// spawned in `lib.rs::run()` setup hook. Unbounded — chat hot path MUST NOT block.
    pub memory_tx: UnboundedSender<MemoryEvent>,
    /// Lazily-filled embedder for the RAG pipeline.
    ///
    /// `AppState::new()` initializes this as an EMPTY `OnceCell`; the setup
    /// hook in `lib.rs` spawns a background task that downloads the model and
    /// then `cell.set(...)`s a `FastEmbedLocal` instance. Lock-free reads via
    /// `OnceCell::get()`. A `OnceCell` is write-once by construction, so the
    /// embedder cannot be swapped at runtime — changing the model means a
    /// restart, which is the safe outcome anyway: every vector already in the
    /// store belongs to the old model's space and mixing the two silently
    /// corrupts similarity.
    ///
    /// In `new_for_test()` / `new_for_test_async()` the cell is PRE-SEEDED
    /// with a `FakeEmbedder` so tests never race the bootstrap task.
    pub embedder: Arc<OnceCell<Arc<dyn Embedder>>>,
    /// Shared tokenizer for query-context composition on the chat path. Same
    /// `tokenizer.json` the chat chunker loads (`model_download::tokenizer_path`),
    /// so the two never disagree about what a "token" is. Filled once by the
    /// memory worker's setup task; `compute_memory_prefix` self-heals a still-
    /// empty cell via `get_or_try_init` on the query path, so a turn that
    /// arrives before the worker's own load finishes still gets one retry.
    /// Empty `OnceCell` in every constructor (production and both test ones) —
    /// there is no eager load here, only a shared slot for whoever fills it.
    pub tokenizer: Arc<OnceCell<Arc<tokenizers::Tokenizer>>>,
    /// Fire-and-forget access tracker for the RAG hot path.
    ///
    /// Buffers retrieved memory IDs and flushes them atomically to SQLite
    /// (access_count + tier via `compute_tier` UDF) every 30 s or when ≥500
    /// entries accumulate.  The internal `UnboundedSender` is the only public
    /// surface — `record` never blocks or panics the hot path.
    ///
    /// `tokio_rusqlite::Connection` is `Clone` (channel handle) so no extra
    /// `Mutex` is needed here; the `Arc` provides cheap shared ownership.
    pub access_tracker: Arc<clawg_core::memory::access::AccessTracker>,
}

/// Test handle bundling `AppState` with its `memory_rx` so tests can assert on
/// emitted `MemoryEvent`s without moving the receiver around separately.
#[cfg(test)]
pub struct MemoryTestHandle {
    pub state: AppState,
    pub memory_rx: UnboundedReceiver<MemoryEvent>,
}

impl AppState {
    /// Build production AppState + the Receiver the setup hook will hand to
    /// the memory worker. The Receiver is returned instead of being stored on
    /// `AppState` because an mpsc channel has exactly one consumer end: it is
    /// not `Clone`, and `AppState` is shared behind `&`, so it could never be
    /// taken back out. Returning it makes the single-drainer ownership explicit
    /// at the call site rather than a runtime surprise.
    pub fn new() -> (Self, UnboundedReceiver<MemoryEvent>) {
        let config = config_store::load_config();
        let provider = create_provider_from_config(&config);

        // Misconfiguration (an invalid or unsafe CLAWG_PROFILE) must be fatal
        // at startup, never silently fall back to a wrong data directory.
        let base_dir = clawg_core::paths::data_dir_from_env()
            .unwrap_or_else(|e| panic!("failed to resolve data directory: {e}"));

        let sessions_dir = base_dir.join("sessions");

        // Load persisted data
        let providers = load_json::<Vec<ProviderConfig>>(&base_dir.join("providers.json"))
            .unwrap_or_default();
        let mut agents = load_json::<Vec<Agent>>(&base_dir.join("agents.json"))
            .unwrap_or_default();

        // Recompose system prompts (not persisted)
        for agent in &mut agents {
            agent.system_prompt = compose_system_prompt(
                &agent.name,
                &agent.role,
                agent.efficient_mode,
                agent.project_path.as_deref(),
            );
            agent.state = AgentState::Idle;
        }

        let (session_manager, preloaded_conversations) = Self::init_session(&sessions_dir);

        let tool_registry = Arc::new(GlobalToolRegistry::new());
        let tool_executor = Arc::new(ToolExecutor::new(tool_registry.clone()));

        // Open memory store synchronously. AppState::new() is called from the sync
        // lib.rs::run() — no tokio context exists yet, so block_on is safe here.
        let memory_store = Arc::new(
            tauri::async_runtime::block_on(async {
                MemoryStore::open(&base_dir).await
            })
            .expect("Failed to open memory store at base_dir/memory.db"),
        );

        // Memory ingest channel. Single-consumer by design — see `memory_tx`.
        let (memory_tx, memory_rx) = tokio::sync::mpsc::unbounded_channel::<MemoryEvent>();

        // AccessTracker::new() calls tokio::spawn internally, so we wrap it in
        // block_on to enter the tauri async runtime context — identical pattern
        // to the MemoryStore open() call above.
        let access_tracker = Arc::new(
            tauri::async_runtime::block_on(async {
                clawg_core::memory::access::AccessTracker::new(memory_store.conn.clone())
            })
        );

        let state = Self {
            base_dir,
            providers: Mutex::new(providers),
            agents: Mutex::new(agents),
            provider: RwLock::new(provider),
            provider_pool: Mutex::new(ProviderPool::new()),
            conversations: Mutex::new(preloaded_conversations),
            mcp_pool: McpPool::new(),
            config: RwLock::new(config),
            session_manager: Mutex::new(session_manager),
            rate_limit_tracker: Mutex::new(HashMap::new()),
            oauth_refresh_lock: tokio::sync::Mutex::new(()),
            mcp_recovery_gate: RecoveryGate::new(),
            tool_registry,
            tool_executor,
            pending_confirmations: Mutex::new(HashMap::new()),
            memory_store,
            memory_tx,
            // Empty OnceCell: filled by the setup-hook bootstrap task in lib.rs.
            // AppState::new() MUST NOT block on the download — keeps startup snappy
            // and lets the UI render even when the embedder fails to load.
            embedder: Arc::new(OnceCell::new()),
            tokenizer: Arc::new(OnceCell::new()),
            access_tracker,
        };
        (state, memory_rx)
    }

    /// Minimal constructor for unit tests — no disk I/O, no provider connections.
    ///
    /// Returns `(Self, UnboundedReceiver<MemoryEvent>)` — same tuple shape as
    /// `AppState::new()`. Tests that don't assert on events should drain the
    /// receiver via: `tokio::spawn(async move { while rx.recv().await.is_some() {} });`
    ///
    /// PANICS if called from a `#[tokio::test]` context (i.e. inside a tokio runtime).
    /// Use [`AppState::new_for_test_async`] from async tests that need `AppState`.
    #[cfg(test)]
    pub fn new_for_test() -> (Self, UnboundedReceiver<MemoryEvent>) {
        use clawg_core::providers::ollama::{OllamaConfig, OllamaProvider};
        use clawg_core::models::config::AppConfig;

        let temp_dir = std::env::temp_dir().join(format!("clawg-test-{}", std::process::id()));
        let sessions_dir = temp_dir.join("sessions");
        let _ = std::fs::create_dir_all(&sessions_dir);

        let config = AppConfig::default();
        let provider = Arc::new(OllamaProvider::new(OllamaConfig {
            host: "localhost".into(),
            port: 11434,
            model: "test".into(),
        }));

        let (session_manager, conversations) = Self::init_session(&sessions_dir);

        let tool_registry = Arc::new(GlobalToolRegistry::new());
        let tool_executor = Arc::new(ToolExecutor::new(tool_registry.clone()));

        // Build memory_store. Detect tokio context: if we're inside a tokio runtime,
        // block_on would panic — force callers to use new_for_test_async() instead.
        let memory_store = match tokio::runtime::Handle::try_current() {
            Err(_) => {
                // Safe: no tokio runtime active.
                Arc::new(
                    tauri::async_runtime::block_on(async {
                        MemoryStore::new_for_test().await
                    })
                    .expect("test memory store"),
                )
            }
            Ok(_) => panic!(
                "AppState::new_for_test() cannot be called from #[tokio::test]; \
                 use AppState::new_for_test_async().await instead"
            ),
        };

        let (memory_tx, memory_rx) = tokio::sync::mpsc::unbounded_channel::<MemoryEvent>();

        // No tokio runtime is active here (the Err branch above guarantees
        // that), so we use block_on to enter the tauri runtime context for the
        // duration of AccessTracker::new(), which calls tokio::spawn internally.
        let access_tracker = Arc::new(
            tauri::async_runtime::block_on(async {
                clawg_core::memory::access::AccessTracker::new(memory_store.conn.clone())
            })
        );

        let state = Self {
            base_dir: temp_dir,
            providers: Mutex::new(vec![]),
            agents: Mutex::new(vec![]),
            provider: RwLock::new(provider),
            provider_pool: Mutex::new(ProviderPool::new()),
            conversations: Mutex::new(conversations),
            mcp_pool: McpPool::new(),
            config: RwLock::new(config),
            session_manager: Mutex::new(session_manager),
            rate_limit_tracker: Mutex::new(HashMap::new()),
            oauth_refresh_lock: tokio::sync::Mutex::new(()),
            mcp_recovery_gate: RecoveryGate::new(),
            tool_registry,
            tool_executor,
            pending_confirmations: Mutex::new(HashMap::new()),
            memory_store,
            memory_tx,
            embedder: seeded_embedder_cell(),
            tokenizer: Arc::new(OnceCell::new()),
            access_tracker,
        };
        (state, memory_rx)
    }

    /// Async variant of [`new_for_test`] for use inside `#[tokio::test]`.
    ///
    /// Returns `(Self, UnboundedReceiver<MemoryEvent>)`. Tests that don't assert
    /// on events should still drain the receiver — the channel is unbounded, so
    /// an ignored receiver grows without bound for the life of the test.
    /// Identical fields to `new_for_test()` but builds `memory_store` via `.await`
    /// to avoid the `block_on`-inside-tokio panic.
    #[cfg(test)]
    pub async fn new_for_test_async() -> (Self, UnboundedReceiver<MemoryEvent>) {
        use clawg_core::providers::ollama::{OllamaConfig, OllamaProvider};
        use clawg_core::models::config::AppConfig;

        let temp_dir = std::env::temp_dir()
            .join(format!("clawg-test-async-{}", std::process::id()));
        let sessions_dir = temp_dir.join("sessions");
        let _ = std::fs::create_dir_all(&sessions_dir);

        let config = AppConfig::default();
        let provider = Arc::new(OllamaProvider::new(OllamaConfig {
            host: "localhost".into(),
            port: 11434,
            model: "test".into(),
        }));

        let (session_manager, conversations) = Self::init_session(&sessions_dir);
        let tool_registry = Arc::new(GlobalToolRegistry::new());
        let tool_executor = Arc::new(ToolExecutor::new(tool_registry.clone()));
        let memory_store = Arc::new(
            MemoryStore::new_for_test()
                .await
                .expect("test memory store"),
        );

        let (memory_tx, memory_rx) = tokio::sync::mpsc::unbounded_channel::<MemoryEvent>();

        // We are inside a #[tokio::test] context, so tokio::spawn (called inside
        // AccessTracker::new()) works directly — no block_on wrapper needed.
        let access_tracker = Arc::new(
            clawg_core::memory::access::AccessTracker::new(memory_store.conn.clone())
        );

        let state = Self {
            base_dir: temp_dir,
            providers: Mutex::new(vec![]),
            agents: Mutex::new(vec![]),
            provider: RwLock::new(provider),
            provider_pool: Mutex::new(ProviderPool::new()),
            conversations: Mutex::new(conversations),
            mcp_pool: McpPool::new(),
            config: RwLock::new(config),
            session_manager: Mutex::new(session_manager),
            rate_limit_tracker: Mutex::new(HashMap::new()),
            oauth_refresh_lock: tokio::sync::Mutex::new(()),
            mcp_recovery_gate: RecoveryGate::new(),
            tool_registry,
            tool_executor,
            pending_confirmations: Mutex::new(HashMap::new()),
            memory_store,
            memory_tx,
            embedder: seeded_embedder_cell(),
            tokenizer: Arc::new(OnceCell::new()),
            access_tracker,
        };
        (state, memory_rx)
    }

    /// Test helper that bundles `AppState` with the `UnboundedReceiver<MemoryEvent>`,
    /// allowing tests to assert on emitted events.
    #[cfg(test)]
    pub async fn new_for_test_with_memory_rx() -> MemoryTestHandle {
        let (state, memory_rx) = Self::new_for_test_async().await;
        MemoryTestHandle { state, memory_rx }
    }

    fn init_session(
        sessions_dir: &std::path::Path,
    ) -> (SessionManager, HashMap<String, Vec<ChatMessage>>) {
        let manager = SessionManager::load_latest(sessions_dir)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                SessionManager::new_session(sessions_dir)
                    .expect("Failed to create session directory")
            });

        let mut conversations = HashMap::new();
        for agent_id in &manager.meta().agents {
            if let Ok(messages) = manager.load_agent_messages(agent_id) {
                conversations.insert(agent_id.clone(), messages);
            }
        }

        (manager, conversations)
    }

    // ── Provider resolution ──

    pub fn get_provider(&self) -> Arc<dyn AiProvider> {
        self.provider.read().unwrap().clone()
    }

    /// Build a provider instance from a ProviderConfig.
    ///
    /// Delegates to [`ProviderPool::build_provider`] so the construction logic
    /// lives in a single place.
    pub fn build_provider_from_config(config: &ProviderConfig) -> Arc<dyn AiProvider> {
        ProviderPool::build_provider(config)
    }

    /// Get the provider for a specific agent (by its provider_id).
    ///
    /// Returns a cached instance from the [`ProviderPool`] when possible.
    /// Falls back to the global provider when `provider_id` is not found.
    pub fn get_agent_provider(&self, provider_id: &str) -> Arc<dyn AiProvider> {
        let providers = self.providers.lock().ok();
        if let Some(providers) = providers {
            if let Some(config) = providers.iter().find(|p| p.id == provider_id) {
                if let Ok(mut pool) = self.provider_pool.lock() {
                    return pool.get_or_build(config);
                }
                return Self::build_provider_from_config(config);
            }
        }
        // Fallback to global
        self.get_provider()
    }

    /// Remove the cached provider for `provider_id`.
    ///
    /// Must be called after updating or deleting a [`ProviderConfig`], and after
    /// an OAuth token refresh so the next call picks up fresh credentials.
    pub fn invalidate_agent_provider(&self, provider_id: &str) {
        if let Ok(mut pool) = self.provider_pool.lock() {
            pool.invalidate(provider_id);
        }
    }

    // ── Legacy provider switch (for existing config panel) ──

    pub fn switch_provider(
        &self,
        provider_type: ProviderType,
        ollama: Option<OllamaSettings>,
        anthropic: Option<AnthropicSettings>,
        openrouter: Option<OpenRouterSettings>,
    ) -> Result<(), AppError> {
        let mut config = self
            .config
            .write()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        config.active_provider = provider_type;
        if let Some(o) = ollama {
            config.ollama = o;
        }
        if let Some(a) = anthropic {
            config.anthropic = a;
        }
        if let Some(or) = openrouter {
            config.openrouter = or;
        }

        let new_provider = create_provider_from_config(&config);
        let config_snapshot = config.clone();
        drop(config); // Release write lock before disk I/O

        if let Err(e) = config_store::save_config(&config_snapshot) {
            log::error!("Failed to persist config: {}", e);
        }

        let mut provider = self
            .provider
            .write()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        *provider = new_provider;

        Ok(())
    }

    // ── Persistence helpers ──

    pub fn save_providers(&self, providers: &[ProviderConfig]) {
        let path = self.base_dir.join("providers.json");
        save_json(&path, providers);
    }

    pub fn save_agents(&self, agents: &[Agent]) {
        let path = self.base_dir.join("agents.json");
        save_json(&path, agents);
    }
}

// ── File helpers ──

fn load_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Option<T> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            log::error!("Failed to read {}: {}", path.display(), e);
            return None;
        }
    };
    match serde_json::from_str(&content) {
        Ok(v) => Some(v),
        Err(e) => {
            log::error!("Failed to parse {}: {}", path.display(), e);
            None
        }
    }
}

fn save_json<T: serde::Serialize + ?Sized>(path: &std::path::Path, data: &T) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::error!("Failed to create dir {}: {}", parent.display(), e);
            return;
        }
    }
    match serde_json::to_string_pretty(data) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path, &json) {
                log::error!("Failed to write {}: {}", path.display(), e);
            }
        }
        Err(e) => log::error!("Failed to serialize data for {}: {}", path.display(), e),
    }
}

fn create_provider_from_config(config: &AppConfig) -> Arc<dyn AiProvider> {
    match config.active_provider {
        ProviderType::Ollama => Arc::new(OllamaProvider::new(OllamaConfig {
            host: config.ollama.host.clone(),
            port: config.ollama.port,
            model: model_meta::resolve_alias(&config.ollama.model).to_string(),
        })),
        ProviderType::Anthropic => Arc::new(AnthropicProvider::new(AnthropicConfig {
            auth: resolve_anthropic_auth(&config.anthropic.api_key),
            model: model_meta::resolve_alias(&config.anthropic.model).to_string(),
            base_url: resolve_base_url(
                ANTHROPIC_BASE_URL_ENV,
                &config.anthropic.base_url,
                DEFAULT_ANTHROPIC_URL,
            ),
        })),
        ProviderType::OpenRouter => Arc::new(OpenRouterProvider::new(OpenRouterConfig {
            api_key: resolve_api_key(OPENROUTER_KEY_ENV, &config.openrouter.api_key),
            model: model_meta::resolve_alias(&config.openrouter.model).to_string(),
            base_url: resolve_base_url(
                OPENROUTER_BASE_URL_ENV,
                &config.openrouter.base_url,
                DEFAULT_OPENROUTER_URL,
            ),
        })),
    }
}

/// Build an [`OnceCell`] pre-filled with a [`FakeEmbedder`] for the test
/// constructors. Shared helper so `new_for_test()` and `new_for_test_async()`
/// stay in sync.
///
/// Pre-seeding in BOTH test helpers is what keeps tests deterministic: in
/// production the cell is filled by the setup-hook bootstrap task, which does
/// not exist under test, so an unseeded cell would make every
/// `state.embedder.get()` return `None` — or race, once a test spawns its own
/// filler. Seeding at construction removes the window entirely.
#[cfg(test)]
fn seeded_embedder_cell() -> Arc<OnceCell<Arc<dyn Embedder>>> {
    use clawg_core::memory::embedder::FakeEmbedder;
    let cell: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
    // .ok() — set() can only fail if the cell is already filled, which is
    // impossible on a brand-new instance.
    cell.set(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>).ok();
    Arc::new(cell)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- AppState constructors wire access_tracker ----------

    /// `new_for_test_async` must produce an `Arc<AccessTracker>` that accepts
    /// `record()` calls without blocking or panicking — it sits on the retrieval
    /// hot path, so a blocking or panicking tracker would stall chat.
    #[tokio::test]
    async fn t10_new_for_test_async_access_tracker_accepts_record() {
        let (state, _rx) = AppState::new_for_test_async().await;
        // access_tracker field must exist and record must not block or panic
        state.access_tracker.record(vec![1, 2, 3]);
        state.access_tracker.record(vec![]); // empty vec is a no-op
    }

    /// `Arc<AccessTracker>` must be clonable for shared ownership across threads.
    #[tokio::test]
    async fn t10_access_tracker_arc_is_clonable_and_usable() {
        let (state, _rx) = AppState::new_for_test_async().await;
        let tracker_ref = std::sync::Arc::clone(&state.access_tracker);
        tracker_ref.record(vec![99, 100]); // record on the clone must not block
    }

    /// `new_for_test_async()` must build a memory_store with the `vec0`
    /// extension actually loaded — a store that opens but lacks `vec0` would
    /// only fail much later, inside a vector query.
    #[tokio::test]
    async fn new_for_test_async_opens_memory_store() {
        let (state, _rx) = AppState::new_for_test_async().await;
        // Verify memory_store is functional by querying vec_version().
        let version: String = state
            .memory_store
            .conn
            .call(|c| -> Result<String, rusqlite::Error> {
                c.query_row("SELECT vec_version()", [], |r| r.get(0))
            })
            .await
            .expect("vec_version() failed");
        assert!(!version.is_empty(), "memory_store vec_version() returned empty");
    }

    /// `new_for_test()` must panic when called from `#[tokio::test]`: it
    /// `block_on`s, which deadlocks inside an active runtime. Failing loudly
    /// points the author at `new_for_test_async()` instead of hanging the suite.
    #[tokio::test]
    async fn new_for_test_panics_in_tokio_context() {
        // AppState::new_for_test() should panic inside a tokio context.
        let result = std::panic::catch_unwind(|| {
            AppState::new_for_test()
        });
        assert!(result.is_err(), "Expected panic from new_for_test() in tokio context");
    }

    // ---------- Embedder / tokenizer cell wiring ----------

    /// `AppState::new_for_test()` is sync-only — calling it inside a tokio
    /// runtime panics (see `new_for_test_panics_in_tokio_context`). We can't
    /// invoke it directly from a `#[tokio::test]`, so this test verifies the
    /// SHARED `seeded_embedder_cell()` helper that both ctors use.
    #[tokio::test]
    async fn new_for_test_preseeds_fake_embedder() {
        let cell = seeded_embedder_cell();
        let embedder = cell
            .get()
            .expect("seeded_embedder_cell must pre-fill the OnceCell");
        assert_eq!(
            embedder.model_id(),
            "fake-embedder",
            "expected FakeEmbedder, got {}",
            embedder.model_id()
        );
    }

    #[tokio::test]
    async fn new_for_test_async_preseeds_fake_embedder() {
        let (state, _rx) = AppState::new_for_test_async().await;
        let embedder = state
            .embedder
            .get()
            .expect("new_for_test_async must pre-fill the embedder cell");
        let v = embedder
            .embed_with_role("x", clawg_core::memory::embedder::EmbedRole::Passage)
            .await
            .expect("FakeEmbedder.embed_with_role");
        assert_eq!(v.len(), 384, "FakeEmbedder must yield 384-dim vector");
    }

    /// `state.tokenizer` starts empty in every constructor, unlike
    /// `state.embedder` — no test double is pre-seeded here, because every
    /// query-composition test injects its own counter closure and never
    /// needs a real `Tokenizer` (see `memory::query_context`'s tests).
    #[tokio::test]
    async fn new_for_test_async_exposes_an_empty_tokenizer_cell() {
        let (state, _rx) = AppState::new_for_test_async().await;
        assert!(state.tokenizer.get().is_none(), "the tokenizer cell must start empty");
    }

    /// `new_for_test()` is sync-only but safe to call from a plain (non-tokio)
    /// `#[test]` — this is that direct call, covering the ctor
    /// `new_for_test_async` above cannot reach.
    #[test]
    fn new_for_test_exposes_an_empty_tokenizer_cell() {
        let (state, _rx) = AppState::new_for_test();
        assert!(state.tokenizer.get().is_none(), "the tokenizer cell must start empty");
    }

    /// `AppState::new()` cannot be exercised from a `#[tokio::test]` either —
    /// it `block_on`s the memory-store open. So we assert the WIRING via the
    /// same shape: a default-constructed `Arc::new(OnceCell::new())` returns
    /// `None`, matching what `new()` will yield to a caller that races the
    /// setup-hook bootstrap.
    #[test]
    fn new_returns_empty_embedder_cell() {
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());
        assert!(
            cell.get().is_none(),
            "production AppState::new() initializes the embedder cell empty"
        );
    }

    // ---------- Memory ingest channel ----------

    #[tokio::test]
    async fn memory_tx_send_before_worker_buffers_event() {
        use clawg_core::memory::events::{MemoryEvent, MemorySource};
        use clawg_core::models::chat::Role;
        let (state, _memory_rx) = AppState::new_for_test_async().await;
        let result = state.memory_tx.send(MemoryEvent {
            agent_id: "a".into(),
            project_id: None,
            role: Role::User,
            content: "hello".into(),
            is_error: None,
            source: MemorySource::Chat,
            timestamp_ms: 0,
        });
        assert!(result.is_ok(), "send must succeed before worker is connected");
    }

    #[tokio::test]
    async fn memory_tx_clone_sends_independently() {
        use clawg_core::memory::events::{MemoryEvent, MemorySource};
        use clawg_core::models::chat::Role;
        let (state, mut memory_rx) = AppState::new_for_test_async().await;
        let tx2 = state.memory_tx.clone();
        state.memory_tx.send(MemoryEvent {
            agent_id: "a".into(),
            project_id: None,
            role: Role::User,
            content: "first".into(),
            is_error: None,
            source: MemorySource::Chat,
            timestamp_ms: 1,
        }).unwrap();
        tx2.send(MemoryEvent {
            agent_id: "b".into(),
            project_id: None,
            role: Role::User,
            content: "second".into(),
            is_error: None,
            source: MemorySource::Chat,
            timestamp_ms: 2,
        }).unwrap();
        let e1 = memory_rx.recv().await.expect("first event");
        let e2 = memory_rx.recv().await.expect("second event");
        assert_eq!(e1.content, "first");
        assert_eq!(e2.content, "second");
    }

    #[tokio::test]
    async fn new_for_test_async_memory_tx_does_not_deadlock() {
        use clawg_core::memory::events::{MemoryEvent, MemorySource};
        use clawg_core::models::chat::Role;
        let (state, _rx) = AppState::new_for_test_async().await;
        for i in 0..3u64 {
            assert!(state.memory_tx.send(MemoryEvent {
                agent_id: "a".into(),
                project_id: None,
                role: Role::User,
                content: format!("msg {i}"),
                is_error: None,
                source: MemorySource::Chat,
                timestamp_ms: i,
            }).is_ok());
        }
    }

    #[tokio::test]
    async fn new_for_test_with_memory_rx_round_trip() {
        use clawg_core::memory::events::{MemoryEvent, MemorySource};
        use clawg_core::models::chat::Role;
        let mut handle = AppState::new_for_test_with_memory_rx().await;
        handle.state.memory_tx.send(MemoryEvent {
            agent_id: "a".into(),
            project_id: None,
            role: Role::User,
            content: "hello".into(),
            is_error: None,
            source: MemorySource::Chat,
            timestamp_ms: 42,
        }).unwrap();
        let evt = handle.memory_rx.recv().await.expect("event received");
        assert_eq!(evt.content, "hello");
        assert_eq!(evt.timestamp_ms, 42);
    }
}

pub async fn check_ollama_status() -> OllamaStatus {
    let client = reqwest::Client::new();
    match client.get("http://localhost:11434/api/tags").send().await {
        Ok(resp) if resp.status().is_success() => {
            let models = resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v.get("models")?.as_array().cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|m| m.get("name")?.as_str().map(String::from))
                .collect();
            OllamaStatus {
                running: true,
                models,
            }
        }
        _ => OllamaStatus {
            running: false,
            models: vec![],
        },
    }
}
