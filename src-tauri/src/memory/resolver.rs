//! Contradiction resolver for Phase 6 sycophancy guard (REQ-07 — REQ-10).
//!
//! Adjudicates disputed memory clusters by consulting an LLM provider and
//! applies the verdict atomically to the SQLite store.
//!
//! # Design constraints (LOCKED)
//! - D2: importance guard — winner `importance = MIN(importance, 6.0)` keeps
//!   disputed rows below `tier.rs:26` fast-track (importance ≥ 9 → tier 1).
//! - D3: `RESOLVER_TIMEOUT = 30 s`.
//! - D5: `state.get_agent_provider(pid)` already falls back to global provider
//!   when `pid` is unknown — use it directly.

use std::sync::Arc;
use std::time::Duration;

use crate::memory::MemoryStore;
use crate::models::{ChatMessage, ChatRequest, ChatStreamResult, Role};
use crate::providers::{AiProvider, StreamSender};

// ── Constants ────────────────────────────────────────────────────────────────

/// Maximum wall-clock time allowed for a single LLM adjudication call (LOCKED D3).
const RESOLVER_TIMEOUT: Duration = Duration::from_secs(30);

/// System prompt for the adjudication LLM call.
const SYS: &str = "You adjudicate memory contradictions. \
    Reply with ONLY one line: \
    `ADD` (statements can both be true), \
    `UPDATE <id>` (they conflict; <id>=the TRUE/current statement), \
    or `NO-OP` (undecidable).";

// ── Types ────────────────────────────────────────────────────────────────────

/// Verdict returned by `adjudicate_cluster`.
///
/// - `Add`    — both the new row and all existing rows stay active.
/// - `Update` — `winner` is the correct row; losers remain disputed.
/// - `NoOp`   — timeout, provider error, or undecidable; zero DB writes.
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum Resolution {
    Add,
    Update { winner: i64 },
    NoOp,
}

/// A group of memory rows that contradict each other.
///
/// Built by `load_disputed_clusters`, consumed by `adjudicate_cluster` and
/// `apply_resolution`.
pub(crate) struct DisputedCluster {
    /// The newly-inserted row flagged as disputed.
    pub new_id: i64,
    /// Agent that owns these memory rows (used to select provider).
    pub agent_id: String,
    /// Content of the new row (displayed to the LLM).
    pub new_content: String,
    /// `(id, content)` pairs of existing rows the new row contradicts.
    pub against: Vec<(i64, String)>,
}

// ── Prompt building ───────────────────────────────────────────────────────────

/// Build the user-turn content for the adjudication request.
///
/// Lists the new row first, then each existing row with their ids, so the LLM
/// can reference them in an `UPDATE <id>` verdict.
fn build_prompt(c: &DisputedCluster) -> String {
    let mut out = format!("NEW [id={}]: {}\n", c.new_id, c.new_content);
    for (id, content) in &c.against {
        out.push_str(&format!("EXISTING [id={}]: {}\n", id, content));
    }
    out
}

// ── Verdict parsing ───────────────────────────────────────────────────────────

/// Parse the first non-empty line of an LLM response into a `Resolution`.
///
/// Matching is case-insensitive and whitespace-tolerant.
///
/// - `"ADD"` → `Resolution::Add`
/// - `"UPDATE <n>"` where `n` is a cluster member id → `Resolution::Update { winner: n }`
/// - anything else (including `UPDATE` with an unknown id) → `Resolution::NoOp`
pub(crate) fn parse_verdict(txt: &str, c: &DisputedCluster) -> Resolution {
    let line = txt
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_uppercase();

    if line == "ADD" {
        return Resolution::Add;
    }
    if let Some(rest) = line.strip_prefix("UPDATE ") {
        if let Ok(n) = rest.trim().parse::<i64>() {
            let valid = n == c.new_id || c.against.iter().any(|(id, _)| *id == n);
            if valid {
                return Resolution::Update { winner: n };
            }
        }
    }
    Resolution::NoOp
}

// ── Adjudication ──────────────────────────────────────────────────────────────

/// Consult an LLM provider and return a `Resolution` for the cluster.
///
/// - Uses a no-op `StreamSender` — the full text is captured via
///   `ChatStreamResult::Text`.
/// - Wrapped in `tokio::time::timeout(t)` — on expiry logs `log::warn!` and
///   returns `NoOp`. `ToolCalls` and provider errors also produce `NoOp`.
pub(crate) async fn adjudicate_cluster(
    provider: &Arc<dyn AiProvider>,
    c: &DisputedCluster,
    t: Duration,
) -> Resolution {
    // No-op sender: we only need the accumulated Text result.
    let sender: StreamSender = Box::new(|_e| Ok(()));

    // `model: String::new()` — the provider already holds the model name
    // (verified as-built: chat.rs:636 uses the same pattern; design §facts).
    let req = ChatRequest {
        model: String::new(),
        messages: vec![ChatMessage {
            id: crate::models::chat::generate_message_id(),
            role: Role::User,
            content: build_prompt(c),
            tool_calls: None,
            timestamp_ms: 0,
            is_error: None,
        }],
        system_prompt: Some(SYS.to_string()),
        custom_instructions: None,
        tools: None,
        extensions: Default::default(),
    };

    match tokio::time::timeout(t, provider.chat_stream(&req, sender)).await {
        Ok(Ok(ChatStreamResult::Text(txt))) => parse_verdict(&txt, c),
        Ok(Ok(ChatStreamResult::ToolCalls(_))) | Ok(Err(_)) => Resolution::NoOp,
        Err(_elapsed) => {
            log::warn!("resolver: LLM timeout for agent={}", c.agent_id);
            Resolution::NoOp
        }
    }
}

// ── Resolution application ────────────────────────────────────────────────────

/// Apply a `Resolution` atomically to the SQLite store.
///
/// All UPDATE statements run in a single `conn.call` + `tx.transaction()` per
/// `Resolution` arm (LOCKED, design §apply_resolution).
///
/// - `Add`:    all cluster rows → `status='active'`, `disputed_against=NULL`.
/// - `Update`: winner → `importance=MIN(importance,6.0)`, `status='active'`,
///             `disputed_against=NULL`; losers → `status='disputed'`,
///             `last_disputed_at=now` (LOCKED D2 importance guard).
/// - `NoOp`:   zero DB writes; rows remain as-is.
pub(crate) async fn apply_resolution(
    store: &MemoryStore,
    c: &DisputedCluster,
    resolution: Resolution,
) -> Result<(), tokio_rusqlite::Error> {
    match resolution {
        Resolution::NoOp => Ok(()),

        Resolution::Add => {
            // All cluster members (new + against) become active.
            let all_ids: Vec<i64> = std::iter::once(c.new_id)
                .chain(c.against.iter().map(|(id, _)| *id))
                .collect();

            store
                .conn
                .call(move |conn| -> Result<(), rusqlite::Error> {
                    let tx = conn.transaction()?;
                    for id in &all_ids {
                        tx.execute(
                            "UPDATE memory \
                             SET status='active', disputed_against=NULL \
                             WHERE id=?1",
                            rusqlite::params![id],
                        )?;
                    }
                    tx.commit()
                })
                .await
        }

        Resolution::Update { winner } => {
            // losers = all cluster members except the winner.
            let losers: Vec<i64> = std::iter::once(c.new_id)
                .chain(c.against.iter().map(|(id, _)| *id))
                .filter(|&id| id != winner)
                .collect();
            let winner_id = winner;
            let now_ms: i64 = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;

            store
                .conn
                .call(move |conn| -> Result<(), rusqlite::Error> {
                    let tx = conn.transaction()?;

                    // Winner: restore to active, cap importance (LOCKED D2),
                    // clear dispute markers, bump modification metadata.
                    tx.execute(
                        "UPDATE memory \
                         SET status='active', \
                             importance=MIN(importance, 6.0), \
                             disputed_against=NULL, \
                             last_modified_at=?1, \
                             update_count=update_count+1 \
                         WHERE id=?2",
                        rusqlite::params![now_ms, winner_id],
                    )?;

                    // Losers: leave disputed, record when the dispute was last seen.
                    for loser_id in &losers {
                        tx.execute(
                            "UPDATE memory \
                             SET status='disputed', last_disputed_at=?1 \
                             WHERE id=?2",
                            rusqlite::params![now_ms, loser_id],
                        )?;
                    }

                    tx.commit()
                })
                .await
        }
    }
}

// ── Cluster loader ────────────────────────────────────────────────────────────

/// Load all disputed clusters that are pending LLM adjudication.
///
/// Queries `memory` for `status='disputed' AND disputed_against IS NOT NULL AND
/// deleted_at IS NULL`, parses each row's JSON id list, then fetches the content
/// of the referenced against-rows. Rows with unparseable JSON or missing
/// against-rows are skipped with `log::warn!`.
pub(crate) async fn load_disputed_clusters(store: &MemoryStore) -> Vec<DisputedCluster> {
    // Step 1: fetch all pending disputed rows.
    let raw: Vec<(i64, String, String, String)> = match store
        .conn
        .call(|conn| -> Result<Vec<(i64, String, String, String)>, rusqlite::Error> {
            let mut stmt = conn.prepare(
                "SELECT id, agent_id, content, disputed_against \
                 FROM memory \
                 WHERE status='disputed' \
                   AND disputed_against IS NOT NULL \
                   AND deleted_at IS NULL",
            )?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();
            Ok(rows)
        })
        .await
    {
        Ok(v) => v,
        Err(e) => {
            log::warn!("load_disputed_clusters: initial query failed: {e}");
            return Vec::new();
        }
    };

    let mut clusters = Vec::new();

    for (new_id, agent_id, new_content, da_json) in raw {
        // Step 2: parse disputed_against JSON → Vec<i64>.
        let against_ids: Vec<i64> = match serde_json::from_str::<Vec<i64>>(&da_json) {
            Ok(ids) => ids,
            Err(e) => {
                log::warn!(
                    "load_disputed_clusters: row {new_id} has invalid \
                     disputed_against JSON `{da_json}`: {e}"
                );
                continue;
            }
        };

        // Step 3: fetch content for each against-id.
        let ids_to_fetch = against_ids.clone();
        let against_rows: Vec<(i64, String)> = match store
            .conn
            .call(move |conn| -> Result<Vec<(i64, String)>, rusqlite::Error> {
                let mut result = Vec::new();
                for id in &ids_to_fetch {
                    match conn.query_row(
                        "SELECT id, content FROM memory WHERE id=?1",
                        rusqlite::params![id],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                    ) {
                        Ok(row) => result.push(row),
                        Err(rusqlite::Error::QueryReturnedNoRows) => {
                            // Row was deleted — skip it silently.
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok(result)
            })
            .await
        {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "load_disputed_clusters: failed to fetch against-rows for \
                     cluster new_id={new_id}: {e}"
                );
                continue;
            }
        };

        if against_rows.is_empty() {
            // All against-rows gone (deleted); nothing to adjudicate.
            continue;
        }

        clusters.push(DisputedCluster {
            new_id,
            agent_id,
            new_content,
            against: against_rows,
        });
    }

    clusters
}

// ── Session entry point ───────────────────────────────────────────────────────

/// Fire-and-forget entry point spawned from `commands/session.rs` on each
/// new or resumed session.
///
/// # Design (LOCKED D5)
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
    let store = state.memory_store.clone();

    // Step 1: load pending clusters.
    let clusters = load_disputed_clusters(&store).await;
    if clusters.is_empty() {
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
    for cluster in clusters {
        let provider = match agent_pid.get(&cluster.agent_id) {
            Some(pid) => state.get_agent_provider(pid),
            None => state.get_provider(),
        };

        let resolution = adjudicate_cluster(&provider, &cluster, RESOLVER_TIMEOUT).await;

        if let Err(e) = apply_resolution(&store, &cluster, resolution).await {
            log::warn!(
                "run_resolver: apply_resolution failed (new_id={}): {e}",
                cluster.new_id
            );
        }
    }
}

// Compile-time proof: every type captured by the future passed to
// `tauri::async_runtime::spawn(run_resolver(app))` must be `Send + 'static`.
const _: fn() = || {
    fn s<T: Send + 'static>() {}
    s::<tauri::AppHandle>();
    s::<Arc<MemoryStore>>();
};

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    use crate::memory::db::MemoryStore;
    use crate::models::{AppError, ChatRequest, ChatStreamResult};
    use crate::providers::{AiProvider, StreamSender};

    // ── Test helpers ─────────────────────────────────────────────────────────

    /// Build a minimal `DisputedCluster` for use in parse_verdict / adjudicate tests.
    /// new_id=1, against=[(2, "existing content")].
    fn make_test_cluster() -> DisputedCluster {
        DisputedCluster {
            new_id: 1,
            agent_id: "test-agent".to_string(),
            new_content: "new content".to_string(),
            against: vec![(2, "existing content".to_string())],
        }
    }

    /// Fake provider that immediately returns a fixed text response.
    struct FakeProvider {
        response: String,
    }

    #[async_trait::async_trait]
    impl AiProvider for FakeProvider {
        fn name(&self) -> &str {
            "fake"
        }
        async fn chat_stream(
            &self,
            _request: &ChatRequest,
            _sender: StreamSender,
        ) -> Result<ChatStreamResult, AppError> {
            Ok(ChatStreamResult::Text(self.response.clone()))
        }
    }

    /// Provider whose `chat_stream` future never resolves — used for T19 timeout test.
    struct NeverProvider;

    #[async_trait::async_trait]
    impl AiProvider for NeverProvider {
        fn name(&self) -> &str {
            "never"
        }
        async fn chat_stream(
            &self,
            _request: &ChatRequest,
            _sender: StreamSender,
        ) -> Result<ChatStreamResult, AppError> {
            // Pending forever — only tokio::time::timeout can break this.
            std::future::pending::<Result<ChatStreamResult, AppError>>().await
        }
    }

    /// Insert an `'active'` memory row and return its id.
    async fn insert_active_row(store: &MemoryStore, agent_id: &str) -> i64 {
        let agent = agent_id.to_string();
        store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, source, created_at, \
                      created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, \
                      importance, tier, status) \
                     VALUES ('agent', ?1, 'active content', 'chat', 0, \
                      'user', ?1, 0, 0, 5.0, 3, 'active')",
                    rusqlite::params![agent],
                )?;
                let id = tx.last_insert_rowid();
                tx.commit()?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    /// Insert a `'disputed'` memory row with a given `disputed_against` JSON and return its id.
    async fn insert_disputed_row(
        store: &MemoryStore,
        agent_id: &str,
        disputed_against_json: &str,
    ) -> i64 {
        let agent = agent_id.to_string();
        let da = disputed_against_json.to_string();
        store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, source, created_at, \
                      created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, \
                      importance, tier, status, \
                      disputed_against, last_disputed_at) \
                     VALUES ('agent', ?1, 'disputed content', 'chat', 0, \
                      'user', ?1, 0, 0, 5.0, 3, 'disputed', ?2, 0)",
                    rusqlite::params![agent, da],
                )?;
                let id = tx.last_insert_rowid();
                tx.commit()?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    /// Fetch `(status, disputed_against)` for a row.
    async fn row_status(store: &MemoryStore, id: i64) -> (String, Option<String>) {
        store
            .conn
            .call(move |c| -> Result<(String, Option<String>), rusqlite::Error> {
                c.query_row(
                    "SELECT status, disputed_against FROM memory WHERE id=?1",
                    rusqlite::params![id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
                )
            })
            .await
            .unwrap()
    }

    /// Fetch `(status, disputed_against, importance)` for a row.
    async fn row_full(store: &MemoryStore, id: i64) -> (String, Option<String>, f64) {
        store
            .conn
            .call(move |c| -> Result<(String, Option<String>, f64), rusqlite::Error> {
                c.query_row(
                    "SELECT status, disputed_against, importance \
                     FROM memory WHERE id=?1",
                    rusqlite::params![id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, f64>(2)?,
                        ))
                    },
                )
            })
            .await
            .unwrap()
    }

    // ── T16 [TEST] parse_verdict ──────────────────────────────────────────────

    /// "ADD" maps to Resolution::Add.
    #[test]
    fn parse_verdict_add_returns_add() {
        let c = make_test_cluster();
        assert_eq!(parse_verdict("ADD", &c), Resolution::Add);
    }

    /// "UPDATE <n>" with a known id maps to Resolution::Update.
    #[test]
    fn parse_verdict_update_known_id_returns_update() {
        let c = make_test_cluster(); // against id = 2
        assert_eq!(
            parse_verdict("UPDATE 2", &c),
            Resolution::Update { winner: 2 }
        );
    }

    /// UPDATE with new_id is also a valid winner.
    #[test]
    fn parse_verdict_update_new_id_is_valid_winner() {
        let c = make_test_cluster(); // new_id = 1
        assert_eq!(
            parse_verdict("UPDATE 1", &c),
            Resolution::Update { winner: 1 }
        );
    }

    /// Garbage text maps to Resolution::NoOp.
    #[test]
    fn parse_verdict_garbage_returns_noop() {
        let c = make_test_cluster();
        assert_eq!(parse_verdict("DUNNO", &c), Resolution::NoOp);
    }

    /// UPDATE with an id not in the cluster maps to NoOp (safety check).
    #[test]
    fn parse_verdict_update_unknown_id_returns_noop() {
        let c = make_test_cluster();
        assert_eq!(parse_verdict("UPDATE 999", &c), Resolution::NoOp);
    }

    /// Empty input maps to NoOp.
    #[test]
    fn parse_verdict_empty_returns_noop() {
        let c = make_test_cluster();
        assert_eq!(parse_verdict("", &c), Resolution::NoOp);
    }

    /// Matching is case-insensitive.
    #[test]
    fn parse_verdict_case_insensitive() {
        let c = make_test_cluster();
        assert_eq!(parse_verdict("add", &c), Resolution::Add);
        assert_eq!(parse_verdict("update 2", &c), Resolution::Update { winner: 2 });
    }

    /// Leading whitespace / multi-line response: first non-empty line wins.
    #[test]
    fn parse_verdict_first_nonempty_line_used() {
        let c = make_test_cluster();
        // Leading blank lines should be skipped; first real line parsed.
        assert_eq!(parse_verdict("\n\nADD\nignored", &c), Resolution::Add);
        assert_eq!(
            parse_verdict("\n\nUPDATE 2\nmore text", &c),
            Resolution::Update { winner: 2 }
        );
    }

    // ── T17 [TEST] adjudicate_cluster — FakeProvider("ADD") ─────────────────

    #[tokio::test]
    async fn adjudicate_cluster_fake_add_returns_add() {
        let provider: Arc<dyn AiProvider> = Arc::new(FakeProvider {
            response: "ADD".to_string(),
        });
        let c = make_test_cluster();
        let result = adjudicate_cluster(&provider, &c, Duration::from_secs(5)).await;
        assert_eq!(result, Resolution::Add, "FakeProvider('ADD') must yield Add");
    }

    // ── T18 [TEST] adjudicate_cluster — FakeProvider("UPDATE n") ────────────

    #[tokio::test]
    async fn adjudicate_cluster_fake_update_returns_update() {
        let provider: Arc<dyn AiProvider> = Arc::new(FakeProvider {
            response: "UPDATE 2".to_string(),
        });
        let c = make_test_cluster(); // against id = 2
        let result = adjudicate_cluster(&provider, &c, Duration::from_secs(5)).await;
        assert_eq!(
            result,
            Resolution::Update { winner: 2 },
            "FakeProvider('UPDATE 2') must yield Update{{winner:2}}"
        );
    }

    // ── T19 [TEST] timeout — ZERO real sleep ─────────────────────────────────

    /// A provider that never completes must produce NoOp after the timeout fires.
    ///
    /// Uses `#[tokio::test(start_paused = true)]` + `tokio::time::advance` to
    /// simulate 31 s without any real wall-clock sleep.
    #[tokio::test(start_paused = true)]
    async fn adjudicate_cluster_timeout_returns_noop() {
        let provider: Arc<dyn AiProvider> = Arc::new(NeverProvider);

        // Spawn the adjudication so we can advance time from the test task.
        let task = tokio::spawn({
            let p = provider.clone();
            async move {
                let cluster = DisputedCluster {
                    new_id: 99,
                    agent_id: "timeout-agent".to_string(),
                    new_content: "will time out".to_string(),
                    against: vec![(100, "other".to_string())],
                };
                adjudicate_cluster(&p, &cluster, Duration::from_secs(30)).await
            }
        });

        // Advance tokio's simulated clock past the 30 s timeout.
        // No real sleep — test-util advances the mock clock atomically.
        tokio::time::advance(Duration::from_secs(31)).await;

        let result = task.await.expect("adjudicate_cluster task must not panic");
        assert_eq!(
            result,
            Resolution::NoOp,
            "LLM timeout must produce NoOp"
        );
    }

    // ── T21 [TEST] apply_resolution Add ──────────────────────────────────────

    /// After Add: both the new row and the against row are `status='active'`
    /// with `disputed_against=NULL`.
    #[tokio::test]
    async fn apply_resolution_add_sets_all_cluster_members_active() {
        let store = MemoryStore::new_for_test().await.unwrap();

        // Insert new (disputed) row and an existing (active) row.
        let against_id = insert_active_row(&store, "agent-add").await;
        let new_id = insert_disputed_row(
            &store,
            "agent-add",
            &format!("[{against_id}]"),
        )
        .await;

        let cluster = DisputedCluster {
            new_id,
            agent_id: "agent-add".to_string(),
            new_content: "new".to_string(),
            against: vec![(against_id, "existing".to_string())],
        };

        apply_resolution(&store, &cluster, Resolution::Add)
            .await
            .unwrap();

        let (new_status, new_da) = row_status(&store, new_id).await;
        let (against_status, against_da) = row_status(&store, against_id).await;

        assert_eq!(new_status, "active", "new row must be active after Add");
        assert!(new_da.is_none(), "new row disputed_against must be NULL after Add");
        assert_eq!(against_status, "active", "against row must be active after Add");
        assert!(
            against_da.is_none(),
            "against row disputed_against must be NULL after Add"
        );
    }

    // ── T22 [TEST] apply_resolution Update ───────────────────────────────────

    /// After Update { winner }: winner is `status='active'`,
    /// `importance = MIN(importance, 6.0)`, `disputed_against=NULL`.
    /// Losers are `status='disputed'` with `last_disputed_at` updated.
    /// Everything happens in one atomic transaction.
    #[tokio::test]
    async fn apply_resolution_update_winner_active_losers_disputed() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let against_id = insert_active_row(&store, "agent-upd").await;
        let new_id = insert_disputed_row(
            &store,
            "agent-upd",
            &format!("[{against_id}]"),
        )
        .await;

        // Raise winner's importance to 8.0 so the MIN(,6.0) cap is observable.
        let winner_cap_id = against_id;
        store
            .conn
            .call(move |c| -> Result<usize, rusqlite::Error> {
                c.execute(
                    "UPDATE memory SET importance=8.0 WHERE id=?1",
                    rusqlite::params![winner_cap_id],
                )
            })
            .await
            .unwrap();

        let cluster = DisputedCluster {
            new_id,
            agent_id: "agent-upd".to_string(),
            new_content: "new".to_string(),
            against: vec![(against_id, "existing".to_string())],
        };

        // against_id wins
        apply_resolution(
            &store,
            &cluster,
            Resolution::Update { winner: against_id },
        )
        .await
        .unwrap();

        // Winner checks
        let (winner_status, winner_da, winner_importance) =
            row_full(&store, against_id).await;
        assert_eq!(winner_status, "active", "winner must be active");
        assert!(winner_da.is_none(), "winner disputed_against must be NULL");
        assert!(
            winner_importance <= 6.0,
            "winner importance must be capped at 6.0; got {winner_importance}"
        );

        // Loser (new_id) checks
        let (loser_status, _, _) = row_full(&store, new_id).await;
        assert_eq!(loser_status, "disputed", "loser must remain disputed");

        // Verify last_disputed_at was set on the loser
        let last_disputed: Option<i64> = store
            .conn
            .call(move |c| -> Result<Option<i64>, rusqlite::Error> {
                c.query_row(
                    "SELECT last_disputed_at FROM memory WHERE id=?1",
                    rusqlite::params![new_id],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert!(
            last_disputed.is_some(),
            "loser last_disputed_at must be set after Update"
        );
    }

    // ── T23 [TEST] apply_resolution NoOp ─────────────────────────────────────

    /// After NoOp: zero DB writes — both rows remain exactly as before.
    #[tokio::test]
    async fn apply_resolution_noop_writes_nothing() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let against_id = insert_active_row(&store, "agent-noop").await;
        let new_id = insert_disputed_row(
            &store,
            "agent-noop",
            &format!("[{against_id}]"),
        )
        .await;

        let cluster = DisputedCluster {
            new_id,
            agent_id: "agent-noop".to_string(),
            new_content: "new".to_string(),
            against: vec![(against_id, "existing".to_string())],
        };

        apply_resolution(&store, &cluster, Resolution::NoOp)
            .await
            .unwrap();

        // Both rows must be unchanged.
        let (new_status, _) = row_status(&store, new_id).await;
        let (against_status, _) = row_status(&store, against_id).await;

        assert_eq!(
            new_status, "disputed",
            "new row must remain disputed after NoOp"
        );
        assert_eq!(
            against_status, "active",
            "against row must remain active after NoOp"
        );
    }
}
