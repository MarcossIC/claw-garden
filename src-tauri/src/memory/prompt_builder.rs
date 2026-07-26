//! Assembles the 4-block memory prefix for the system prompt (doc 6:1049-1073).

use std::sync::Arc;
use std::time::UNIX_EPOCH;

use crate::memory::{
    access::AccessTracker,
    embedder::Embedder,
    search::{search_all_memories, RankedHit, SearchScope},
    workspace::WorkspaceManager,
    MemoryError, MemoryStore,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const SEP_WORKSPACE: &str = "=== WORKSPACE ===";
const SEP_RECENT: &str = "=== RECENT CONTEXT (Last 7 Days) ===";
const SEP_LONGTERM: &str = "=== LONG-TERM KNOWLEDGE ===";
const SEP_CONFLICTS: &str = "=== ACTIVE CONFLICTS (review needed) ===";

/// Exposed for cross-module test assertions only.
#[cfg(test)]
pub(crate) const SEP_WORKSPACE_TEST: &str = SEP_WORKSPACE;

const RECENT_WINDOW_SECS: i64 = 7 * 86_400;
const K_FINAL: usize = 10;

// ---------------------------------------------------------------------------
// Internal data types
// ---------------------------------------------------------------------------

struct ConflictPair {
    old_id: i64,
    old_created_at: i64,
    old_content: String,
    old_importance: f64,
    old_tier: i64,
    new_id: i64,
    new_created_at: i64,
    new_content: String,
    new_importance: f64,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Builds the WORKSPACE / RECENT / LONG-TERM / [ACTIVE CONFLICTS] prefix string.
///
/// Returns `Ok(String::new())` when all four blocks would be empty.
/// Each internal failure is degraded to "the matching block is empty".
///
/// `access_tracker` is fired-and-forgotten after retrieval hits are assembled
/// (REQ-TRACK, STATE-3).  Production callers pass `Some(&state.access_tracker)`;
/// test callers pass `None` to skip recording.
pub(crate) async fn build_memory_prompt(
    store: &MemoryStore,
    workspace: &WorkspaceManager,
    embedder: Option<Arc<dyn Embedder>>,
    agent_id: &str,
    project_id: Option<&str>,
    user_query: &str,
    access_tracker: Option<&AccessTracker>,
) -> Result<String, MemoryError> {
    // 1. Read workspace file (may be empty on fresh install or read failure).
    // Strip structural markers — emit only the payload content to the LLM.
    let workspace_content = {
        let raw = workspace
            .read(agent_id, project_id)
            .await
            .unwrap_or_else(|e| {
                log::warn!("workspace read failed: {e}; treating as empty");
                String::new()
            });
        strip_empty_workspace_markers(&raw)
    };

    // 2. Compute query embedding ONLY if embedder is available.
    let hits: Vec<RankedHit> = match embedder {
        Some(emb) => match emb.embed(user_query).await {
            Ok(q_vec) => {
                match search_all_memories(store, &q_vec, agent_id, SearchScope::all_default(), K_FINAL).await {
                    Ok(h) => h,
                    Err(e) => {
                        log::warn!("search_all_memories failed: {e}; no retrieval hits this turn");
                        Vec::new()
                    }
                }
            }
            Err(e) => {
                log::warn!("embedder.embed failed: {e}; no retrieval hits this turn");
                Vec::new()
            }
        },
        None => {
            log::warn!("embedder not ready; retrieval skipped this turn (agent_id={agent_id})");
            Vec::new()
        }
    };

    // Fire-and-forget access recording (REQ-TRACK, STATE-3).
    // Called AFTER hits are assembled — non-blocking, never awaits.
    // `record` discards empty vecs internally (TRACK-2) so no extra guard needed.
    if let Some(tracker) = access_tracker {
        tracker.record(hits.iter().map(|h| h.id).collect());
    }

    // memoria-3-capas-fase-4 (Requirement: Tier-1 coverage logging) — the real
    // measure of doc-7's "Tier 1 covers 80%" criterion (workspace_content
    // presence alone, or hit count alone, can't answer it — both signals are
    // needed together). Logged unconditionally, BEFORE `bucket_by_recency`
    // consumes `hits` below. `log::info!` (vs search.rs's `log::debug!`) —
    // this is the criterion's capturable signal.
    log::info!(
        "{}",
        crate::commands::memory_admin::format_tier1_coverage(
            !workspace_content.is_empty(),
            hits.len()
        )
    );

    // trajectory-miner-fase-4 (Requirement: Skill-hit logging) — counts hits
    // whose `kind == TableKind::Procedural`. Borrows `&hits` BEFORE
    // `bucket_by_recency` moves `hits` below.
    log::info!(
        "{}",
        crate::commands::memory_admin::format_skill_hits(&hits)
    );

    // 3. Bucket hits into RECENT vs LONG-TERM.
    let now_secs = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (recent, longterm) = bucket_by_recency(hits, now_secs);

    // 4. Fetch conflicts (always query; output conditional on D8).
    let conflicts = fetch_active_conflicts(store, agent_id).await.unwrap_or_else(|e| {
        log::warn!("fetch_active_conflicts failed: {e}; CONFLICTS block omitted");
        Vec::new()
    });

    // 5. Assemble in order: WORKSPACE → RECENT → LONG-TERM → CONFLICTS.
    let mut out = String::new();
    if !workspace_content.is_empty() {
        out.push_str(SEP_WORKSPACE);
        out.push('\n');
        out.push_str(&workspace_content);
        if !workspace_content.ends_with('\n') {
            out.push('\n');
        }
    }
    if !recent.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(SEP_RECENT);
        out.push('\n');
        for h in &recent {
            out.push_str(&format_hit_line(h));
            out.push('\n');
        }
    }
    if !longterm.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(SEP_LONGTERM);
        out.push('\n');
        for h in &longterm {
            out.push_str(&format_hit_line(h));
            out.push('\n');
        }
    }
    if !conflicts.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(SEP_CONFLICTS);
        out.push('\n');
        for p in &conflicts {
            out.push_str(&format_conflict_pair(p));
            out.push('\n');
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Howard Hinnant integer-arithmetic algorithm for unix timestamp → (year, month, day).
/// No external dependency. Verified: unix_secs_to_ymd(0) = (1970, 1, 1);
/// unix_secs_to_ymd(1_700_000_000) = (2023, 11, 14).
fn unix_secs_to_ymd(ts: i64) -> (i32, u32, u32) {
    // Days since Unix epoch (truncate toward negative infinity for negative dates)
    let z = if ts >= 0 {
        ts / 86_400
    } else {
        (ts - 86_399) / 86_400
    };
    // Civil date from days — Howard Hinnant's algorithm
    let z = z + 719_468;
    let era: i64 = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

/// Formats a single retrieval hit into the canonical line format.
/// `[YYYY-MM-DD] SOURCE: {kind:?} | TYPE: {memory_type}\nCONTENT: {content}`
fn format_hit_line(hit: &RankedHit) -> String {
    let (y, m, d) = unix_secs_to_ymd(hit.created_at);
    format!(
        "[{y:04}-{m:02}-{d:02}] SOURCE: {:?} | TYPE: {}\nCONTENT: {}",
        hit.kind, hit.memory_type, hit.content
    )
}

/// Formats a conflict pair into the 4-line format from design §3.5.
fn format_conflict_pair(p: &ConflictPair) -> String {
    let (oy, om, od) = unix_secs_to_ymd(p.old_created_at);
    let (ny, nm, nd) = unix_secs_to_ymd(p.new_created_at);
    format!(
        "[{oy:04}-{om:02}-{od:02}] OLD: \"{}\" [memory:{}, importance:{:.0}, tier:{}]\n\
         [{ny:04}-{nm:02}-{nd:02}] NEW: \"{}\" [memory:{}, importance:{:.0}, status:disputed]\n\
         NOTE: estas memorias se contradicen y aún no resolviste cuál vale.\n\
         Si vas a tomar una decisión que dependa de esto, preguntale al usuario.",
        p.old_content, p.old_id, p.old_importance, p.old_tier,
        p.new_content, p.new_id, p.new_importance,
    )
}

/// Partitions hits into (recent, longterm) by `hit.created_at >= now_secs - RECENT_WINDOW_SECS`.
fn bucket_by_recency(hits: Vec<RankedHit>, now_secs: i64) -> (Vec<RankedHit>, Vec<RankedHit>) {
    let threshold = now_secs - RECENT_WINDOW_SECS;
    let mut recent = Vec::new();
    let mut longterm = Vec::new();
    for h in hits {
        if h.created_at >= threshold {
            recent.push(h);
        } else {
            longterm.push(h);
        }
    }
    (recent, longterm)
}

/// Strips known empty structural markers from workspace content.
/// Returns empty string ONLY when the workspace file was auto-generated with
/// no payload content (has both AUTO_START/AUTO_END markers but no bullet items
/// and no USER section content). Content that lacks the structural markers
/// entirely (e.g., plain text files) is considered meaningful.
fn strip_empty_workspace_markers(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    use crate::memory::workspace::{AUTO_START, USER_START, USER_END};

    // If the file doesn't have the AUTO-START marker, it's plain content → keep it.
    if !raw.contains(AUTO_START) {
        return raw.to_string();
    }

    // File has the structured marker framework. Check for actual payload content.
    let has_bullets = raw.contains("\n- ");
    let has_user_content = if let (Some(us), Some(ue)) = (raw.find(USER_START), raw.find(USER_END)) {
        let inner = &raw[us + USER_START.len()..ue];
        inner.chars().any(|c| !c.is_whitespace())
    } else {
        false
    };

    if has_bullets || has_user_content {
        raw.to_string()
    } else {
        String::new()
    }
}

/// Fetches disputed memory pairs from the DB using the CONFLICTS SQL.
/// Returns `Ok(Vec::new())` when there are no disputed rows or on SQL error (caller logs).
async fn fetch_active_conflicts(
    store: &MemoryStore,
    agent_id: &str,
) -> Result<Vec<ConflictPair>, MemoryError> {
    let agent = agent_id.to_string();
    store
        .conn
        .call(move |c| -> Result<Vec<ConflictPair>, rusqlite::Error> {
            let mut stmt = c.prepare(
                "SELECT m_old.id, m_old.created_at, m_old.content, m_old.importance, m_old.tier,
                        m_new.id, m_new.created_at, m_new.content, m_new.importance
                 FROM memory m_new
                 JOIN memory m_old
                   ON m_old.id IN (SELECT value FROM json_each(m_new.disputed_against))
                 WHERE m_new.status = 'disputed'
                   AND m_new.agent_id = ?1
                   AND m_new.deleted_at IS NULL
                   AND m_old.deleted_at IS NULL",
            )?;
            let pairs: Vec<ConflictPair> = stmt
                .query_map(rusqlite::params![agent], |row| {
                    Ok(ConflictPair {
                        old_id: row.get(0)?,
                        old_created_at: row.get(1)?,
                        old_content: row.get(2)?,
                        old_importance: row.get(3)?,
                        old_tier: row.get(4)?,
                        new_id: row.get(5)?,
                        new_created_at: row.get(6)?,
                        new_content: row.get(7)?,
                        new_importance: row.get(8)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok(pairs)
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend("connection closed during fetch_active_conflicts".into())
            }
            other => MemoryError::Backend(other.to_string()),
        })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{db::MemoryStore, embedder::Embedder, TableKind};

    // T21-test: unix_secs_to_ymd
    #[test]
    fn unix_secs_zero_is_1970_01_01() {
        assert_eq!(unix_secs_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn unix_secs_known_epoch_to_date() {
        // 1_700_000_000 seconds = 2023-11-14 (verified known epoch)
        assert_eq!(unix_secs_to_ymd(1_700_000_000), (2023, 11, 14));
    }

    // T21-test: format_hit_line
    #[test]
    fn format_hit_line_emits_canonical_format() {
        let hit = RankedHit {
            id: 1,
            kind: TableKind::Memory,
            content: "test content".to_string(),
            memory_type: "chat_message".to_string(),
            tier: 2,
            status: "active".to_string(),
            score: 0.9,
            distance: 0.1,
            created_at: 1_700_000_000,
        };
        let line = format_hit_line(&hit);
        assert!(
            line.starts_with("[2023-11-14] SOURCE: Memory | TYPE: chat_message"),
            "got: {line}"
        );
        assert!(line.contains("CONTENT: test content"), "got: {line}");
    }

    // T21-test: bucket_by_recency
    #[test]
    fn bucket_by_recency_splits_at_7_days() {
        let now_secs: i64 = 1_700_000_000;
        let recent_ts = now_secs - 86_400; // 1 day ago
        let old_ts = now_secs - 30 * 86_400; // 30 days ago

        let make_hit = |ts: i64| RankedHit {
            id: 0,
            kind: TableKind::Memory,
            content: String::new(),
            memory_type: String::new(),
            tier: 2,
            status: "active".to_string(),
            score: 0.5,
            distance: 0.5,
            created_at: ts,
        };

        let hits = vec![make_hit(recent_ts), make_hit(recent_ts), make_hit(old_ts)];
        let (recent, longterm) = bucket_by_recency(hits, now_secs);
        assert_eq!(recent.len(), 2, "expected 2 recent hits");
        assert_eq!(longterm.len(), 1, "expected 1 longterm hit");
    }

    // T22-test: fetch_active_conflicts
    #[tokio::test]
    async fn conflicts_query_zero_disputed_rows() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let result = fetch_active_conflicts(&store, "agent-1").await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn conflicts_query_one_disputed_pair_renders_correctly() {
        let store = MemoryStore::new_for_test().await.unwrap();

        // Insert old (active) memory
        let old_id: i64 = store.conn.call(|c| -> Result<i64, rusqlite::Error> {
            c.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', 'agent-1', 'old content', 'semantic_fact', 'chat',
                 1000, 'user', 'agent-1', 1000, 1000, 8.0, 2, 'active')",
                [],
            )?;
            Ok(c.last_insert_rowid())
        }).await.unwrap();

        // Insert new (disputed) memory pointing to old
        let _new_id: i64 = store.conn.call(move |c| -> Result<i64, rusqlite::Error> {
            c.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status, disputed_against)
                 VALUES ('agent', 'agent-1', 'new content', 'semantic_fact', 'chat',
                 2000, 'user', 'agent-1', 2000, 2000, 8.0, 2, 'disputed',
                 json('[' || ?1 || ']'))",
                rusqlite::params![old_id],
            )?;
            Ok(c.last_insert_rowid())
        }).await.unwrap();

        let result = fetch_active_conflicts(&store, "agent-1").await.unwrap();
        assert_eq!(result.len(), 1, "expected 1 conflict pair");
        assert_eq!(result[0].old_id, old_id);
        assert_eq!(result[0].old_content, "old content");
        assert_eq!(result[0].new_content, "new content");
    }

    #[tokio::test]
    async fn json_each_null_returns_zero_rows_no_error() {
        let store = MemoryStore::new_for_test().await.unwrap();

        // Insert a disputed row with NULL disputed_against
        store.conn.call(|c| -> Result<(), rusqlite::Error> {
            c.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', 'agent-1', 'disputed but no reference', 'semantic_fact', 'chat',
                 1000, 'user', 'agent-1', 1000, 1000, 8.0, 2, 'disputed')",
                [],
            )?;
            Ok(())
        }).await.unwrap();

        // Should return empty, not panic or error
        let result = fetch_active_conflicts(&store, "agent-1").await.unwrap();
        assert!(result.is_empty(), "json_each(NULL) should return zero rows, not error");
    }

    // T-11-test: build_memory_prompt accepts Option<&AccessTracker> as last parameter.
    // This test will FAIL TO COMPILE until T-11-impl adds the parameter (= RED).
    #[tokio::test]
    async fn t11_build_memory_prompt_accepts_none_access_tracker() {
        use crate::memory::access::AccessTracker;
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));

        // 7th argument `None::<&AccessTracker>` — must compile after T-11-impl.
        let result = build_memory_prompt(&store, &wm, None, "agent-1", None, "query", None::<&AccessTracker>)
            .await
            .unwrap();
        assert_eq!(result, "", "all empty → empty string");
    }

    // T23-test: build_memory_prompt
    #[tokio::test]
    async fn all_empty_returns_empty_string() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));

        let result = build_memory_prompt(&store, &wm, None, "agent-1", None, "query", None).await.unwrap();
        assert_eq!(result, "", "all empty → empty string");
    }

    #[tokio::test]
    async fn workspace_only_returns_workspace_block() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));

        // Pre-write a workspace file
        wm.regenerate_auto_section("agent-1", None, &store).await.unwrap();
        // Also manually write something to the workspace to ensure it's non-empty
        let path = wm.workspace_path("agent-1", None);
        tokio::fs::write(&path, "<!-- AUTO-START -->\n## Notes\n- something\n<!-- AUTO-END -->\n<!-- USER-START -->\n<!-- USER-END -->\n").await.unwrap();

        let result = build_memory_prompt(&store, &wm, None, "agent-1", None, "query", None).await.unwrap();
        assert!(result.contains(SEP_WORKSPACE), "should contain WORKSPACE separator");
        assert!(!result.contains(SEP_RECENT), "no embedder → no RECENT");
    }

    #[tokio::test]
    async fn embedder_none_skips_retrieval_no_panic() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));

        let result = build_memory_prompt(&store, &wm, None, "agent-1", None, "query", None).await;
        assert!(result.is_ok(), "should not panic or error");
        let s = result.unwrap();
        assert!(!s.contains(SEP_RECENT), "no RECENT block without embedder");
    }

    #[tokio::test]
    async fn embedder_none_emits_workspace_only_if_present() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));

        let path = wm.workspace_path("agent-1", None);
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&path, "workspace content here\n").await.unwrap();

        let result = build_memory_prompt(&store, &wm, None, "agent-1", None, "query", None).await.unwrap();
        assert!(result.contains(SEP_WORKSPACE));
        assert!(!result.contains(SEP_RECENT));
        assert!(!result.contains(SEP_LONGTERM));
    }

    #[tokio::test]
    async fn disputed_pair_renders_conflicts_block() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));

        let old_id: i64 = store.conn.call(|c| -> Result<i64, rusqlite::Error> {
            c.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', 'agent-x', 'old fact', 'semantic_fact', 'chat',
                 1000, 'user', 'agent-x', 1000, 1000, 8.0, 2, 'active')",
                [],
            )?;
            Ok(c.last_insert_rowid())
        }).await.unwrap();

        store.conn.call(move |c| -> Result<(), rusqlite::Error> {
            c.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status, disputed_against)
                 VALUES ('agent', 'agent-x', 'new fact', 'semantic_fact', 'chat',
                 2000, 'user', 'agent-x', 2000, 2000, 8.0, 2, 'disputed',
                 json('[' || ?1 || ']'))",
                rusqlite::params![old_id],
            )?;
            Ok(())
        }).await.unwrap();

        let result = build_memory_prompt(&store, &wm, None, "agent-x", None, "query", None).await.unwrap();
        assert!(result.contains(SEP_CONFLICTS), "should contain CONFLICTS separator");
        assert!(result.contains("ACTIVE CONFLICTS"));
    }

    #[tokio::test]
    async fn no_disputed_rows_omits_conflicts_block() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));

        let result = build_memory_prompt(&store, &wm, None, "agent-1", None, "query", None).await.unwrap();
        assert!(!result.contains("ACTIVE CONFLICTS"), "no disputes → CONFLICTS block omitted");
    }

    #[test]
    fn separator_strings_match_exactly() {
        assert_eq!(SEP_RECENT, "=== RECENT CONTEXT (Last 7 Days) ===");
        assert_eq!(SEP_WORKSPACE, "=== WORKSPACE ===");
        assert_eq!(SEP_LONGTERM, "=== LONG-TERM KNOWLEDGE ===");
        assert_eq!(SEP_CONFLICTS, "=== ACTIVE CONFLICTS (review needed) ===");
    }

    #[tokio::test]
    async fn three_hits_split_into_recent_and_longterm_blocks() {
        use crate::memory::embedder::FakeEmbedder;

        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));

        let embedder = Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>;
        let fake_vec = embedder.embed("dummy").await.unwrap();
        let fake_bytes = crate::memory::search::serialize_f32_query_pub(&fake_vec);

        let now_secs: i64 = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let recent_ts = now_secs - 86_400; // 1 day ago
        let old_ts = now_secs - 30 * 86_400; // 30 days ago

        // Insert 3 rows into memory with embeddings matching our fake embedder output
        // Tier 2+ to pass the tier > 1 filter in search
        for (ts, content) in &[(recent_ts, "recent 1"), (recent_ts, "recent 2"), (old_ts, "old 1")] {
            let ts = *ts;
            let content = content.to_string();
            let bytes = fake_bytes.clone();
            store.conn.call(move |c| -> Result<(), rusqlite::Error> {
                let row_id: i64 = {
                    c.execute(
                        "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                         created_at, created_by_type, created_by_ref,
                         last_modified_at, last_accessed_at, importance, tier, status)
                         VALUES ('agent', 'agent-split', ?1, 'chat_message', 'chat',
                         ?2, 'user', 'agent-split', ?2, ?2, 8.0, 2, 'active')",
                        rusqlite::params![content, ts],
                    )?;
                    c.last_insert_rowid()
                };
                c.execute(
                    "INSERT INTO memory_vectors (rowid, embedding) VALUES (?1, ?2)",
                    rusqlite::params![row_id, bytes],
                )?;
                Ok(())
            }).await.unwrap();
        }

        let result = build_memory_prompt(
            &store, &wm, Some(embedder), "agent-split", None, "some query", None
        ).await.unwrap();

        assert!(result.contains(SEP_RECENT), "should have RECENT block");
        assert!(result.contains(SEP_LONGTERM), "should have LONG-TERM block");
    }

    #[tokio::test]
    async fn recent_longterm_split_by_7day_window() {
        let now_secs: i64 = 1_700_000_000;
        let recent = vec![
            RankedHit { id: 1, kind: TableKind::Memory, content: "r".to_string(), memory_type: "chat_message".to_string(), tier: 2, status: "active".to_string(), score: 0.9, distance: 0.1, created_at: now_secs - 86_400 },
        ];
        let old = vec![
            RankedHit { id: 2, kind: TableKind::Memory, content: "o".to_string(), memory_type: "chat_message".to_string(), tier: 2, status: "active".to_string(), score: 0.8, distance: 0.2, created_at: now_secs - 8 * 86_400 },
        ];
        let (r, l) = bucket_by_recency(recent.into_iter().chain(old).collect(), now_secs);
        assert_eq!(r.len(), 1);
        assert_eq!(l.len(), 1);
    }

    #[tokio::test]
    async fn all_old_hits_go_to_longterm_only() {
        let now_secs: i64 = 1_700_000_000;
        let hits = vec![
            RankedHit { id: 1, kind: TableKind::Memory, content: "a".to_string(), memory_type: "chat_message".to_string(), tier: 2, status: "active".to_string(), score: 0.9, distance: 0.1, created_at: now_secs - 30 * 86_400 },
            RankedHit { id: 2, kind: TableKind::Memory, content: "b".to_string(), memory_type: "chat_message".to_string(), tier: 2, status: "active".to_string(), score: 0.8, distance: 0.2, created_at: now_secs - 20 * 86_400 },
        ];
        let (r, l) = bucket_by_recency(hits, now_secs);
        assert_eq!(r.len(), 0);
        assert_eq!(l.len(), 2);
    }

    #[tokio::test]
    async fn all_recent_hits_go_to_recent_only() {
        let now_secs: i64 = 1_700_000_000;
        let hits = vec![
            RankedHit { id: 1, kind: TableKind::Memory, content: "a".to_string(), memory_type: "chat_message".to_string(), tier: 2, status: "active".to_string(), score: 0.9, distance: 0.1, created_at: now_secs - 86_400 },
            RankedHit { id: 2, kind: TableKind::Memory, content: "b".to_string(), memory_type: "chat_message".to_string(), tier: 2, status: "active".to_string(), score: 0.8, distance: 0.2, created_at: now_secs - 2 * 86_400 },
        ];
        let (r, l) = bucket_by_recency(hits, now_secs);
        assert_eq!(r.len(), 2);
        assert_eq!(l.len(), 0);
    }

    #[tokio::test]
    async fn e2e_full_4_block_output() {
        use crate::memory::embedder::FakeEmbedder;

        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));

        let embedder = Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>;
        let fake_vec = embedder.embed("dummy").await.unwrap();
        let fake_bytes = crate::memory::search::serialize_f32_query_pub(&fake_vec);

        let now_secs: i64 = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let recent_ts = now_secs - 86_400;
        let old_ts = now_secs - 30 * 86_400;

        // 3 chat_message rows (2 recent + 1 old)
        for (ts, content) in &[(recent_ts, "chat recent 1"), (recent_ts, "chat recent 2"), (old_ts, "chat old 1")] {
            let ts = *ts;
            let content = content.to_string();
            let bytes = fake_bytes.clone();
            store.conn.call(move |c| -> Result<(), rusqlite::Error> {
                let row_id: i64 = {
                    c.execute(
                        "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                         created_at, created_by_type, created_by_ref,
                         last_modified_at, last_accessed_at, importance, tier, status)
                         VALUES ('agent', 'e2e-agent', ?1, 'chat_message', 'chat',
                         ?2, 'user', 'e2e-agent', ?2, ?2, 8.0, 2, 'active')",
                        rusqlite::params![content, ts],
                    )?;
                    c.last_insert_rowid()
                };
                c.execute(
                    "INSERT INTO memory_vectors (rowid, embedding) VALUES (?1, ?2)",
                    rusqlite::params![row_id, bytes],
                )?;
                Ok(())
            }).await.unwrap();
        }

        // 1 user_preference row with importance=10 and tier=1 (for workspace AUTO section)
        let up_bytes = fake_bytes.clone();
        store.conn.call(move |c| -> Result<(), rusqlite::Error> {
            c.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', 'e2e-agent', 'prefer dark mode', 'user_preference', 'chat',
                 0, 'user', 'e2e-agent', 0, 0, 10.0, 1, 'active')",
                [],
            )?;
            let up_id = c.last_insert_rowid();
            // Also insert into memory_vectors (tier=1 rows can still have vectors but won't be retrieved)
            c.execute(
                "INSERT INTO memory_vectors (rowid, embedding) VALUES (?1, ?2)",
                rusqlite::params![up_id, up_bytes],
            )?;
            Ok(())
        }).await.unwrap();

        // 1 disputed row pointing to one of the chat rows
        let old_id: i64 = store.conn.call(|c| -> Result<i64, rusqlite::Error> {
            c.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', 'e2e-agent', 'original fact', 'semantic_fact', 'chat',
                 500, 'user', 'e2e-agent', 500, 500, 8.0, 2, 'active')",
                [],
            )?;
            Ok(c.last_insert_rowid())
        }).await.unwrap();

        store.conn.call(move |c| -> Result<(), rusqlite::Error> {
            c.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status, disputed_against)
                 VALUES ('agent', 'e2e-agent', 'disputed fact', 'semantic_fact', 'chat',
                 1000, 'user', 'e2e-agent', 1000, 1000, 8.0, 2, 'disputed',
                 json('[' || ?1 || ']'))",
                rusqlite::params![old_id],
            )?;
            Ok(())
        }).await.unwrap();

        // Regenerate workspace AUTO section (captures user_preference)
        wm.regenerate_auto_section("e2e-agent", None, &store).await.unwrap();

        let result = build_memory_prompt(
            &store, &wm, Some(embedder), "e2e-agent", None, "some query", None
        ).await.unwrap();

        // All four separators must appear in order
        let ws_pos = result.find(SEP_WORKSPACE).expect("WORKSPACE separator missing");
        let recent_pos = result.find(SEP_RECENT).expect("RECENT separator missing");
        let lt_pos = result.find(SEP_LONGTERM).expect("LONGTERM separator missing");
        let conf_pos = result.find(SEP_CONFLICTS).expect("CONFLICTS separator missing");

        assert!(ws_pos < recent_pos, "WORKSPACE must come before RECENT");
        assert!(recent_pos < lt_pos, "RECENT must come before LONG-TERM");
        assert!(lt_pos < conf_pos, "LONG-TERM must come before CONFLICTS");

        // user_preference content must be in workspace block (before RECENT)
        let workspace_block = &result[ws_pos..recent_pos];
        assert!(workspace_block.contains("prefer dark mode"), "user_preference must be in WORKSPACE block");

        // CONFLICTS block must contain the disputed pair
        let conflicts_block = &result[conf_pos..];
        assert!(conflicts_block.contains("original fact") || conflicts_block.contains("disputed fact"),
            "CONFLICTS must reference the disputed pair");
    }
}
