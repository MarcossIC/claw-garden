//! Assembles the 4-block memory prefix for the system prompt.

use std::sync::Arc;

use crate::memory::{
    access::AccessTracker,
    clock::TimestampMs,
    embedder::{EmbedRole, Embedder},
    search::{search_all_memories, RankedHit, SearchScope},
    workspace::{EmittedIds, WorkspaceManager},
    MemoryError, MemoryStore, TableKind,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const SEP_WORKSPACE: &str = "=== WORKSPACE ===";
const SEP_RECENT: &str = "=== RECENT CONTEXT (Last 7 Days) ===";
const SEP_LONGTERM: &str = "=== LONG-TERM KNOWLEDGE ===";
const SEP_CONFLICTS: &str = "=== ACTIVE CONFLICTS (review needed) ===";

/// Exposed for cross-module test assertions only.
#[cfg(any(test, feature = "testing"))]
pub const SEP_WORKSPACE_TEST: &str = SEP_WORKSPACE;
#[cfg(test)]
pub(crate) const SEP_RECENT_TEST: &str = SEP_RECENT;
#[cfg(test)]
pub(crate) const SEP_LONGTERM_TEST: &str = SEP_LONGTERM;

const RECENT_WINDOW_MS: i64 = 7 * 86_400_000;
const K_FINAL: usize = 10;

// ---------------------------------------------------------------------------
// Internal data types
// ---------------------------------------------------------------------------

/// The two texts a retrieval turn needs, kept separate because the query
/// composer (`memory::query_context`) only ever widens the DENSE side: the
/// lexical/FTS5 channel must keep scoring against the raw message the user
/// actually typed, never the composed text with prior turns folded in —
/// composing a lexical query would let BM25 match on words the user never
/// wrote this turn.
///
/// Named fields at the one production construction site make "pass the
/// composed text where the raw one belongs" a compile-visible swap instead
/// of two adjacent `&str` parameters a reviewer has to line up by position.
pub struct QueryTexts<'a> {
    /// Fed to `search_all_memories`'s lexical/FTS5 channel — always the raw
    /// current message.
    pub lexical: &'a str,
    /// Embedded with `EmbedRole::Query` — the composed text when context
    /// composition ran, or the raw message when it did not (empty history,
    /// no tokenizer).
    pub embedded: &'a str,
}

impl<'a> QueryTexts<'a> {
    /// Both fields set to the same text — today's pre-composition behavior,
    /// stated explicitly. Every call site that has no context to offer
    /// (single-shot eval, most tests) uses this rather than repeating `q`
    /// twice at the call site, which is what invited the two fields being
    /// swapped by accident in the first place.
    pub(crate) fn single_shot(q: &'a str) -> Self {
        Self { lexical: q, embedded: q }
    }
}

struct ConflictPair {
    old_id: i64,
    old_created_at: TimestampMs,
    old_content: String,
    old_importance: f64,
    old_tier: i64,
    new_id: i64,
    new_created_at: TimestampMs,
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
/// `emitted` names the rows the workspace file already carries, so the same
/// row is never handed to the model twice. It comes from the regeneration
/// that produced the very file read below — not from a second copy of the
/// workspace's selection rule, which would be free to drift from it.
///
/// `access_tracker` is fired-and-forgotten after retrieval hits are assembled.
/// Production callers pass `Some(&state.access_tracker)`;
/// test callers pass `None` to skip recording.
pub async fn build_memory_prompt(
    store: &MemoryStore,
    workspace: &WorkspaceManager,
    embedder: Option<Arc<dyn Embedder>>,
    agent_id: &str,
    project_id: Option<&str>,
    query: QueryTexts<'_>,
    emitted: &EmittedIds,
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
    //
    // The pool is asked for the workspace's worth of extra candidates, because
    // the rows dropped just below are the high-importance ones most likely to
    // rank near the top. Requesting exactly `K_FINAL` and then subtracting
    // would let a well-stocked workspace starve the query-relevant channel:
    // the better the standing context, the fewer hits the actual question
    // gets. The per-table pools (30/5/5) are the real ceiling, so this only
    // ever spends candidates that were already fetched.
    let k_request = K_FINAL.saturating_add(emitted.len());
    let mut hits: Vec<RankedHit> = match embedder {
        // Query: the composed (or, absent composition, raw) text is the thing
        // being searched FOR on the dense side; the lexical/FTS5 side always
        // gets the raw text — see `QueryTexts`'s doc comment for why the two
        // must never be the same variable here.
        Some(emb) => match emb.embed_with_role(query.embedded, EmbedRole::Query).await {
            Ok(q_vec) => {
                match search_all_memories(store, &q_vec, Some(query.lexical), agent_id, SearchScope::all_default().with_project(project_id), k_request).await {
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
    hits.retain(|h| !emitted.contains(h.kind, h.id));
    hits.truncate(K_FINAL);
    let hits = hits;

    // Fire-and-forget access recording. Runs on the filtered set: a row this
    // channel suppressed was not retrieved by it, and counting it would
    // inflate the access count that drives tier promotion.
    // Called AFTER hits are assembled — non-blocking, never awaits.
    // `record` discards empty vecs internally so no extra guard needed.
    // Only `memory` ids: the flush updates `memory WHERE id = ?`, and ids are
    // per-table, so a skill or trajectory id would land on an unrelated
    // `memory` row that happens to share the number.
    if let Some(tracker) = access_tracker {
        tracker.record(
            hits.iter()
                .filter(|h| h.kind == TableKind::Memory)
                .map(|h| h.id)
                .collect(),
        );
    }

    // Tier-1 coverage logging — the real measure of the "Tier 1 covers 80%"
    // criterion (workspace_content presence alone, or hit count alone, can't
    // answer it — both signals are needed together). Logged unconditionally,
    // BEFORE `bucket_by_recency` consumes `hits` below. `log::info!` (vs
    // search.rs's `log::debug!`) — this is the criterion's capturable signal.
    log::info!(
        "{}",
        crate::memory::admin::format_tier1_coverage(
            !workspace_content.is_empty(),
            hits.len()
        )
    );

    // Skill-hit logging — counts hits whose `kind == TableKind::Procedural`.
    // Borrows `&hits` BEFORE `bucket_by_recency` moves `hits` below.
    log::info!(
        "{}",
        crate::memory::admin::format_skill_hits(&hits)
    );

    // 3. Bucket hits into RECENT vs LONG-TERM.
    let (recent, longterm) = bucket_by_recency(hits, TimestampMs::now());

    // 4. Fetch conflicts (always query; CONFLICTS block emitted only when non-empty).
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

/// Converts a millisecond instant to a calendar date. Divides with
/// `div_euclid` because `unix_secs_to_ymd` already floors explicitly for
/// negative dates and truncation would land one day late; takes the
/// timestamp type so the raw-integer call that rendered year 57,000 cannot
/// be written.
fn unix_millis_to_ymd(ts: TimestampMs) -> (i32, u32, u32) {
    unix_secs_to_ymd(ts.as_millis_i64().div_euclid(1_000))
}

/// Formats a single retrieval hit into the canonical line format.
/// `[YYYY-MM-DD] SOURCE: {kind:?} | TYPE: {memory_type}\nCONTENT: {content}`
fn format_hit_line(hit: &RankedHit) -> String {
    let (y, m, d) = unix_millis_to_ymd(hit.created_at);
    format!(
        "[{y:04}-{m:02}-{d:02}] SOURCE: {:?} | TYPE: {}\nCONTENT: {}",
        hit.kind, hit.memory_type, hit.content
    )
}

/// Formats a conflict pair into a 4-line block.
fn format_conflict_pair(p: &ConflictPair) -> String {
    let (oy, om, od) = unix_millis_to_ymd(p.old_created_at);
    let (ny, nm, nd) = unix_millis_to_ymd(p.new_created_at);
    format!(
        "[{oy:04}-{om:02}-{od:02}] OLD: \"{}\" [memory:{}, importance:{:.0}, tier:{}]\n\
         [{ny:04}-{nm:02}-{nd:02}] NEW: \"{}\" [memory:{}, importance:{:.0}, status:disputed]\n\
         NOTE: estas memorias se contradicen y aún no resolviste cuál vale.\n\
         Si vas a tomar una decisión que dependa de esto, preguntale al usuario.",
        p.old_content, p.old_id, p.old_importance, p.old_tier,
        p.new_content, p.new_id, p.new_importance,
    )
}

/// Partitions hits into (recent, longterm) at `now - RECENT_WINDOW_MS`. A stamp
/// at or after the threshold is recent; the epoch or a negative stamp falls to
/// long-term; a future stamp counts as recent rather than being clamped, so a
/// skewed clock stays visible.
fn bucket_by_recency(hits: Vec<RankedHit>, now: TimestampMs) -> (Vec<RankedHit>, Vec<RankedHit>) {
    let threshold = TimestampMs::from_unix_millis(now.as_millis_i64().saturating_sub(RECENT_WINDOW_MS));
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
    use crate::memory::{clock::TimestampMs, db::MemoryStore, embedder::{EmbedError, Embedder}, TableKind};

    /// Records every `(role, text)` pair handed to `embed_with_role`.
    struct RecordingEmbedder {
        calls: std::sync::Mutex<Vec<(EmbedRole, String)>>,
    }
    #[async_trait::async_trait]
    impl Embedder for RecordingEmbedder {
        async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            self.calls.lock().unwrap().push((role, text.to_string()));
            Ok(vec![0.1_f32; 384])
        }
        fn dimensions(&self) -> usize {
            384
        }
        fn model_id(&self) -> &str {
            "recording"
        }
        fn query_prefix(&self) -> &str {
            ""
        }
        fn passage_prefix(&self) -> &str {
            ""
        }
    }

    /// `build_memory_prompt` must embed the user's query with role `Query` —
    /// the only Query call site in production.
    #[tokio::test]
    async fn build_memory_prompt_embeds_the_user_query_with_role_query() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let recording = Arc::new(RecordingEmbedder { calls: std::sync::Mutex::new(Vec::new()) });
        let embedder: Arc<dyn Embedder> = recording.clone();

        build_memory_prompt(&store, &wm, Some(embedder), "agent-1", None, QueryTexts::single_shot("some query"), &EmittedIds::default(), None)
            .await
            .unwrap();

        let calls = recording.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "expected exactly one embed call, got {calls:?}");
        assert_eq!(calls[0].0, EmbedRole::Query, "the user query must be embedded with role Query");
        assert_eq!(calls[0].1, "some query");
    }

    #[test]
    fn unix_secs_zero_is_1970_01_01() {
        assert_eq!(unix_secs_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn unix_secs_known_epoch_to_date() {
        // 1_700_000_000 seconds = 2023-11-14 (verified known epoch)
        assert_eq!(unix_secs_to_ymd(1_700_000_000), (2023, 11, 14));
    }

    #[test]
    fn unix_millis_zero_is_1970_01_01() {
        assert_eq!(unix_millis_to_ymd(TimestampMs::from_unix_millis(0)), (1970, 1, 1));
    }

    #[test]
    fn unix_millis_known_epoch_to_date() {
        // 1_700_000_000_000 ms = 2023-11-14 (verified known epoch)
        assert_eq!(
            unix_millis_to_ymd(TimestampMs::from_unix_millis(1_700_000_000_000)),
            (2023, 11, 14)
        );
    }

    #[test]
    fn unix_millis_negative_is_correct_pre_1970_date() {
        // -1 ms is the last millisecond of 1969-12-31.
        assert_eq!(unix_millis_to_ymd(TimestampMs::from_unix_millis(-1)), (1969, 12, 31));
    }

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
            created_at: TimestampMs::from_unix_millis(1_700_000_000_000),
        };
        let line = format_hit_line(&hit);
        assert!(
            line.starts_with("[2023-11-14] SOURCE: Memory | TYPE: chat_message"),
            "got: {line}"
        );
        assert!(line.contains("CONTENT: test content"), "got: {line}");
    }

    #[test]
    fn format_conflict_pair_renders_both_dates_in_sane_range() {
        let pair = ConflictPair {
            old_id: 1,
            old_created_at: TimestampMs::from_unix_millis(1_690_000_000_000), // ~2023-07-22
            old_content: "old fact".to_string(),
            old_importance: 8.0,
            old_tier: 2,
            new_id: 2,
            new_created_at: TimestampMs::from_unix_millis(1_700_000_000_000), // 2023-11-14
            new_content: "new fact".to_string(),
            new_importance: 8.0,
        };
        let line = format_conflict_pair(&pair);
        let years: Vec<i32> = line
            .lines()
            .filter_map(|l| l.strip_prefix('['))
            .filter_map(|l| l.split('-').next())
            .filter_map(|y| y.parse::<i32>().ok())
            .collect();
        assert_eq!(years.len(), 2, "expected exactly 2 rendered dates, got: {line}");
        for y in &years {
            assert!(
                (2020..=2100).contains(y),
                "rendered year {y} is not a date anyone lived through: {line}"
            );
        }
    }

    #[test]
    fn bucket_by_recency_splits_at_7_days() {
        let now_ms: i64 = 1_700_000_000_000;
        let recent_ts = now_ms - 86_400_000; // 1 day ago
        let old_ts = now_ms - 30 * 86_400_000; // 30 days ago

        let make_hit = |ts: i64| RankedHit {
            id: 0,
            kind: TableKind::Memory,
            content: String::new(),
            memory_type: String::new(),
            tier: 2,
            status: "active".to_string(),
            score: 0.5,
            distance: 0.5,
            created_at: TimestampMs::from_unix_millis(ts),
        };

        let hits = vec![make_hit(recent_ts), make_hit(recent_ts), make_hit(old_ts)];
        let (recent, longterm) = bucket_by_recency(hits, TimestampMs::from_unix_millis(now_ms));
        assert_eq!(recent.len(), 2, "expected 2 recent hits");
        assert_eq!(longterm.len(), 1, "expected 1 longterm hit");
    }

    #[test]
    fn bucket_by_recency_zero_timestamp_goes_to_longterm() {
        let hit = RankedHit {
            id: 0,
            kind: TableKind::Memory,
            content: String::new(),
            memory_type: String::new(),
            tier: 2,
            status: "active".to_string(),
            score: 0.5,
            distance: 0.5,
            created_at: TimestampMs::from_unix_millis(0),
        };
        let now_ms: i64 = 1_700_000_000_000;
        let (recent, longterm) = bucket_by_recency(vec![hit], TimestampMs::from_unix_millis(now_ms));
        assert_eq!(recent.len(), 0);
        assert_eq!(longterm.len(), 1, "epoch timestamp must land in LONG-TERM");
    }

    #[test]
    fn bucket_by_recency_negative_timestamp_goes_to_longterm() {
        let hit = RankedHit {
            id: 0,
            kind: TableKind::Memory,
            content: String::new(),
            memory_type: String::new(),
            tier: 2,
            status: "active".to_string(),
            score: 0.5,
            distance: 0.5,
            created_at: TimestampMs::from_unix_millis(-86_400_000),
        };
        let now_ms: i64 = 1_700_000_000_000;
        let (recent, longterm) = bucket_by_recency(vec![hit], TimestampMs::from_unix_millis(now_ms));
        assert_eq!(recent.len(), 0);
        assert_eq!(longterm.len(), 1, "negative (pre-epoch) timestamp must land in LONG-TERM");
    }

    #[test]
    fn bucket_by_recency_future_timestamp_goes_to_recent() {
        let now_ms: i64 = 1_700_000_000_000;
        let hit = RankedHit {
            id: 0,
            kind: TableKind::Memory,
            content: String::new(),
            memory_type: String::new(),
            tier: 2,
            status: "active".to_string(),
            score: 0.5,
            distance: 0.5,
            created_at: TimestampMs::from_unix_millis(now_ms + 86_400_000),
        };
        let (recent, longterm) = bucket_by_recency(vec![hit], TimestampMs::from_unix_millis(now_ms));
        assert_eq!(recent.len(), 1, "a future timestamp counts as recent rather than being clamped");
        assert_eq!(longterm.len(), 0);
    }

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

    // build_memory_prompt accepts Option<&AccessTracker> as its last parameter.
    #[tokio::test]
    async fn build_memory_prompt_accepts_none_access_tracker() {
        use crate::memory::access::AccessTracker;
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));

        // 7th argument `None::<&AccessTracker>` — must compile once the parameter exists.
        let result = build_memory_prompt(&store, &wm, None, "agent-1", None, QueryTexts::single_shot("query"), &EmittedIds::default(), None::<&AccessTracker>)
            .await
            .unwrap();
        assert_eq!(result, "", "all empty → empty string");
    }

    #[tokio::test]
    async fn all_empty_returns_empty_string() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));

        let result = build_memory_prompt(&store, &wm, None, "agent-1", None, QueryTexts::single_shot("query"), &EmittedIds::default(), None).await.unwrap();
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

        let result = build_memory_prompt(&store, &wm, None, "agent-1", None, QueryTexts::single_shot("query"), &EmittedIds::default(), None).await.unwrap();
        assert!(result.contains(SEP_WORKSPACE), "should contain WORKSPACE separator");
        assert!(!result.contains(SEP_RECENT), "no embedder → no RECENT");
    }

    #[tokio::test]
    async fn embedder_none_skips_retrieval_no_panic() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));

        let result = build_memory_prompt(&store, &wm, None, "agent-1", None, QueryTexts::single_shot("query"), &EmittedIds::default(), None).await;
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

        let result = build_memory_prompt(&store, &wm, None, "agent-1", None, QueryTexts::single_shot("query"), &EmittedIds::default(), None).await.unwrap();
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

        let result = build_memory_prompt(&store, &wm, None, "agent-x", None, QueryTexts::single_shot("query"), &EmittedIds::default(), None).await.unwrap();
        assert!(result.contains(SEP_CONFLICTS), "should contain CONFLICTS separator");
        assert!(result.contains("ACTIVE CONFLICTS"));
    }

    #[tokio::test]
    async fn no_disputed_rows_omits_conflicts_block() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));

        let result = build_memory_prompt(&store, &wm, None, "agent-1", None, QueryTexts::single_shot("query"), &EmittedIds::default(), None).await.unwrap();
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
        let fake_vec = embedder.embed_with_role("dummy", EmbedRole::Passage).await.unwrap();
        let fake_bytes = crate::memory::search::serialize_f32_query_pub(&fake_vec);

        let now_ms: i64 = TimestampMs::now().as_millis_i64();

        let recent_ts = now_ms - 86_400_000; // 1 day ago
        let old_ts = now_ms - 30 * 86_400_000; // 30 days ago

        // Insert 3 rows into memory with embeddings matching our fake embedder output.
        // Tier value is arbitrary here — SEARCH_SQL_MEMORY has no tier filter, every
        // tier is KNN-retrievable.
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
            &store, &wm, Some(embedder), "agent-split", None, QueryTexts::single_shot("some query"),
            &EmittedIds::default(), None
        ).await.unwrap();

        assert!(result.contains(SEP_RECENT), "should have RECENT block");
        assert!(result.contains(SEP_LONGTERM), "should have LONG-TERM block");
    }

    #[tokio::test]
    async fn recent_longterm_split_by_7day_window() {
        let now_ms: i64 = 1_700_000_000_000;
        let recent = vec![
            RankedHit { id: 1, kind: TableKind::Memory, content: "r".to_string(), memory_type: "chat_message".to_string(), tier: 2, status: "active".to_string(), score: 0.9, distance: 0.1, created_at: TimestampMs::from_unix_millis(now_ms - 86_400_000) },
        ];
        let old = vec![
            RankedHit { id: 2, kind: TableKind::Memory, content: "o".to_string(), memory_type: "chat_message".to_string(), tier: 2, status: "active".to_string(), score: 0.8, distance: 0.2, created_at: TimestampMs::from_unix_millis(now_ms - 8 * 86_400_000) },
        ];
        let (r, l) = bucket_by_recency(recent.into_iter().chain(old).collect(), TimestampMs::from_unix_millis(now_ms));
        assert_eq!(r.len(), 1);
        assert_eq!(l.len(), 1);
    }

    #[tokio::test]
    async fn all_old_hits_go_to_longterm_only() {
        let now_ms: i64 = 1_700_000_000_000;
        let hits = vec![
            RankedHit { id: 1, kind: TableKind::Memory, content: "a".to_string(), memory_type: "chat_message".to_string(), tier: 2, status: "active".to_string(), score: 0.9, distance: 0.1, created_at: TimestampMs::from_unix_millis(now_ms - 30 * 86_400_000) },
            RankedHit { id: 2, kind: TableKind::Memory, content: "b".to_string(), memory_type: "chat_message".to_string(), tier: 2, status: "active".to_string(), score: 0.8, distance: 0.2, created_at: TimestampMs::from_unix_millis(now_ms - 20 * 86_400_000) },
        ];
        let (r, l) = bucket_by_recency(hits, TimestampMs::from_unix_millis(now_ms));
        assert_eq!(r.len(), 0);
        assert_eq!(l.len(), 2);
    }

    #[tokio::test]
    async fn all_recent_hits_go_to_recent_only() {
        let now_ms: i64 = 1_700_000_000_000;
        let hits = vec![
            RankedHit { id: 1, kind: TableKind::Memory, content: "a".to_string(), memory_type: "chat_message".to_string(), tier: 2, status: "active".to_string(), score: 0.9, distance: 0.1, created_at: TimestampMs::from_unix_millis(now_ms - 86_400_000) },
            RankedHit { id: 2, kind: TableKind::Memory, content: "b".to_string(), memory_type: "chat_message".to_string(), tier: 2, status: "active".to_string(), score: 0.8, distance: 0.2, created_at: TimestampMs::from_unix_millis(now_ms - 2 * 86_400_000) },
        ];
        let (r, l) = bucket_by_recency(hits, TimestampMs::from_unix_millis(now_ms));
        assert_eq!(r.len(), 2);
        assert_eq!(l.len(), 0);
    }

    // ── the two delivery channels, and the seam between them ─────────────────

    /// Unit-normalized bytes for a vector, the way every production insert
    /// path stores them. Seeding a raw `FakeEmbedder` vector instead (norm
    /// ≈ 11.3) puts the L2 distance above 2, which the cosine conversion
    /// clamps to 0 — every row then ties at score 0 and the ranking under
    /// test becomes arbitrary.
    fn unit_bytes(v: &[f32]) -> Vec<u8> {
        crate::memory::search::serialize_f32_query_pub(&crate::memory::vector::to_unit(v).unwrap())
    }

    /// Seeds one `memory` row with a vector, at the exact tier/importance the
    /// caller asks for. Raw SQL on purpose: these tests are about which
    /// channel reaches a given (tier, importance) coordinate, so the
    /// coordinate has to be settable directly rather than derived.
    async fn seed_memory(
        store: &MemoryStore,
        agent_id: &str,
        content: &str,
        memory_type: &str,
        tier: i64,
        importance: f64,
        bytes: Vec<u8>,
    ) -> i64 {
        let agent = agent_id.to_string();
        let content = content.to_string();
        let memory_type = memory_type.to_string();
        let ts = TimestampMs::now().as_millis_i64();
        store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                c.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                     created_at, created_by_type, created_by_ref,
                     last_modified_at, last_accessed_at, importance, tier, status)
                     VALUES ('agent', ?1, ?2, ?3, 'chat', ?4, 'user', ?1, ?4, ?4, ?5, ?6, 'active')",
                    rusqlite::params![agent, content, memory_type, ts, importance, tier],
                )?;
                let id = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO memory_vectors (rowid, embedding) VALUES (?1, ?2)",
                    rusqlite::params![id, bytes],
                )?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    /// Seeds one `memory` row at an explicit age, so a fixture can put a row
    /// far enough in the past for its decay term to bottom out at the tier
    /// floor. [`seed_memory`] always stamps "now", which cannot express that.
    #[allow(clippy::too_many_arguments)]
    async fn seed_memory_aged(
        store: &MemoryStore,
        agent_id: &str,
        content: &str,
        memory_type: &str,
        tier: i64,
        importance: f64,
        age_days: i64,
        bytes: Vec<u8>,
    ) -> i64 {
        let agent = agent_id.to_string();
        let content = content.to_string();
        let memory_type = memory_type.to_string();
        let ts = TimestampMs::now().as_millis_i64() - age_days * 86_400_000;
        store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                c.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                     created_at, created_by_type, created_by_ref,
                     last_modified_at, last_accessed_at, importance, tier, status)
                     VALUES ('agent', ?1, ?2, ?3, 'chat', ?4, 'user', ?1, ?4, ?4, ?5, ?6, 'active')",
                    rusqlite::params![agent, content, memory_type, ts, importance, tier],
                )?;
                let id = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO memory_vectors (rowid, embedding) VALUES (?1, ?2)",
                    rusqlite::params![id, bytes],
                )?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    /// Always answers with the same unit vector, whatever it is asked to
    /// embed. Lets a fixture place stored rows at chosen distances from the
    /// query without the query vector depending on the query STRING — which
    /// is what makes the lexical half of this test independent of the dense
    /// half.
    struct FixedVecEmbedder;
    #[async_trait::async_trait]
    impl Embedder for FixedVecEmbedder {
        async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            let mut v = vec![0.0_f32; 384];
            v[0] = 1.0;
            Ok(v)
        }
        fn dimensions(&self) -> usize {
            384
        }
        fn model_id(&self) -> &str {
            "fixed-vec"
        }
        fn query_prefix(&self) -> &str {
            ""
        }
        fn passage_prefix(&self) -> &str {
            ""
        }
    }

    /// The app's ONE retrieval call site must hand the user's own text to the
    /// lexical channel, not just the vector it embedded from that text. The
    /// two are separate arguments, so passing `None` for the second compiles,
    /// runs, and silently turns the whole channel off for every real user
    /// while every helper-level test of the channel keeps passing.
    ///
    /// The fixture is built so ONLY the lexical channel can deliver M: 30
    /// filler rows sit at distance 0 and fill the memory pool exactly, so M —
    /// at cosine 0.95 — is rank 31 and never enters the dense pool. M then has
    /// to survive the k_final cut on score alone, which it does because the
    /// fillers are a year stale (decay bottomed out at the tier floor) while M
    /// is fresh and maximally important.
    #[tokio::test]
    async fn the_production_retrieval_call_site_feeds_the_user_query_to_the_lexical_channel() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let embedder = Arc::new(FixedVecEmbedder) as Arc<dyn Embedder>;
        let agent = "agent-lexical-wiring";

        let mut near = vec![0.0_f32; 384];
        near[0] = 1.0;
        let near_bytes = unit_bytes(&near);
        // cosine 0.95 to the query — comfortably above `cosine_floor`
        // (0.8793), so M's similarity term survives the rescale instead of
        // clamping to zero, yet far enough out to lose the pool race.
        let mut off = vec![0.0_f32; 384];
        off[0] = 0.95;
        off[1] = (1.0_f32 - 0.95 * 0.95).sqrt();
        let off_bytes = unit_bytes(&off);

        for i in 0..30 {
            seed_memory_aged(
                &store,
                agent,
                &format!("filler row {i}"),
                "chat_message",
                3,
                1.0,
                365,
                near_bytes.clone(),
            )
            .await;
        }
        seed_memory_aged(
            &store,
            agent,
            "the deployment runbook lives under GPXSTORE7F3A",
            "chat_message",
            3,
            10.0,
            0,
            off_bytes,
        )
        .await;

        let result = build_memory_prompt(
            &store,
            &wm,
            Some(embedder),
            agent,
            None,
            QueryTexts::single_shot("GPXSTORE7F3A"),
            &EmittedIds::default(),
            None,
        )
        .await
        .unwrap();

        assert!(
            result.contains("GPXSTORE7F3A"),
            "the identifier row is reachable only through the lexical channel, so its absence \
             means the production call site is not feeding the user's query text into it: {result}"
        );
    }

    /// `QueryTexts::lexical` and `QueryTexts::embedded` can legitimately
    /// differ once query-context composition folds prior turns into the
    /// embedded text — the lexical/FTS5 channel must never see that widened
    /// text. Fixture shape mirrors the test above: a row reachable ONLY
    /// through an exact lexical match, with `FixedVecEmbedder` making the
    /// dense channel blind to word content, so the only way this row could
    /// surface is if `search_all_memories` were wrongly handed `embedded`
    /// instead of `lexical`.
    #[tokio::test]
    async fn the_lexical_channel_never_sees_the_composed_context() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let embedder = Arc::new(FixedVecEmbedder) as Arc<dyn Embedder>;
        let agent = "agent-composed-context-lexical";

        let mut near = vec![0.0_f32; 384];
        near[0] = 1.0;
        let near_bytes = unit_bytes(&near);
        let mut off = vec![0.0_f32; 384];
        off[0] = 0.95;
        off[1] = (1.0_f32 - 0.95 * 0.95).sqrt();
        let off_bytes = unit_bytes(&off);

        for i in 0..30 {
            seed_memory_aged(
                &store, agent, &format!("filler row {i}"), "chat_message", 3, 1.0, 365, near_bytes.clone(),
            )
            .await;
        }
        seed_memory_aged(
            &store,
            agent,
            "the deployment runbook lives under RAREIDENT99XY",
            "chat_message",
            3,
            10.0,
            0,
            off_bytes,
        )
        .await;

        // The identifier lives ONLY in `embedded` (as a prior turn folded in
        // by composition), never in `lexical` (the raw current message).
        let query = QueryTexts {
            lexical: "what should I check next",
            embedded: "what should I check next\nRAREIDENT99XY",
        };

        let result = build_memory_prompt(&store, &wm, Some(embedder), agent, None, query, &EmittedIds::default(), None)
            .await
            .unwrap();

        assert!(
            !result.contains("RAREIDENT99XY"),
            "the identifier is only in `embedded`; the lexical channel — which must only ever see \
             `lexical` — has no exact-match term to find this row by, so its presence means \
             `query.embedded` leaked into the lexical search: {result}"
        );
    }

    /// The dense channel must embed exactly `QueryTexts::embedded` — the
    /// composed text — not `QueryTexts::lexical`, the raw current message.
    #[tokio::test]
    async fn the_dense_channel_embeds_the_composed_text() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let recording = Arc::new(RecordingEmbedder { calls: std::sync::Mutex::new(Vec::new()) });
        let embedder: Arc<dyn Embedder> = recording.clone();

        let query = QueryTexts {
            lexical: "raw current message",
            embedded: "raw current message\nprior turn folded in by composition",
        };

        build_memory_prompt(&store, &wm, Some(embedder), "agent-1", None, query, &EmittedIds::default(), None)
            .await
            .unwrap();

        let calls = recording.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "expected exactly one embed call, got {calls:?}");
        assert_eq!(calls[0].0, EmbedRole::Query, "the composed text must be embedded with role Query");
        assert_eq!(
            calls[0].1, "raw current message\nprior turn folded in by composition",
            "the dense channel must embed the COMPOSED text, not the raw lexical text"
        );
    }

    /// A tier-1 row below the workspace's importance bar used to be delivered
    /// by nobody: the workspace only takes importance >= 8, and the pool
    /// skipped tier 1 outright. It must now arrive through the query channel —
    /// and through that one only.
    #[tokio::test]
    async fn a_tier1_row_below_the_workspace_bar_arrives_through_exactly_one_channel() {
        use crate::memory::embedder::FakeEmbedder;

        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let embedder = Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>;
        let content = "prefiere ejecutar los tests antes de commitear";
        let bytes = unit_bytes(&embedder.embed_with_role(content, EmbedRole::Passage).await.unwrap());

        seed_memory(&store, "agent-gap", content, "user_preference", 1, 5.0, bytes).await;

        let emitted = wm.regenerate_auto_section("agent-gap", None, &store).await.unwrap();
        assert_eq!(emitted.len(), 0, "importance 5 is below the workspace bar");

        let result =
            build_memory_prompt(&store, &wm, Some(embedder), "agent-gap", None, QueryTexts::single_shot(content), &emitted, None)
                .await
                .unwrap();

        assert_eq!(
            result.matches(content).count(),
            1,
            "exactly one channel must deliver it:\n{result}"
        );
    }

    /// Across all three backing tables, nothing the workspace rendered may
    /// also arrive as a retrieval hit.
    #[tokio::test]
    async fn no_row_is_delivered_by_both_channels_across_the_three_tables() {
        use crate::memory::embedder::FakeEmbedder;
        use crate::memory::store_insert::{CreatedByType, NewEpisodicTrajectoryRecord, NewProceduralSkillRecord};

        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let embedder = Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>;
        let query = "cómo arreglo un test roto";
        let q_vec = embedder.embed_with_role(query, EmbedRole::Passage).await.unwrap();
        let bytes = unit_bytes(&q_vec);

        // memory: above the workspace bar, so the workspace renders it.
        seed_memory(&store, "agent-3t", "prefiere voseo siempre", "user_preference", 1, 9.0, bytes.clone())
            .await;
        // procedural: skills are always tier 1 and the workspace takes the top 5.
        store
            .promote_to_procedural_skill(
                NewProceduralSkillRecord {
                    agent_id: "agent-3t".to_string(),
                    project_id: None,
                    skill_name: "arreglar-test-roto".to_string(),
                    trigger: "un test falla".to_string(),
                    tool_sequence_json: "[\"read\",\"edit\"]".to_string(),
                    source_trajectory_ids: vec![1],
                    importance: 8.0,
                    now_ms: 1_700_000_000_000,
                },
                Some(q_vec.clone()),
            )
            .await
            .unwrap();
        // episodic: the workspace has no query for this table, so it is the
        // control — it must still arrive, proving the filter is not blanket.
        store
            .insert_episodic_trajectory(
                NewEpisodicTrajectoryRecord {
                    scope: "agent",
                    agent_id: Some("agent-3t".to_string()),
                    project_id: None,
                    session_id: "sess-3t".to_string(),
                    task: "reparé el timeout del cliente".to_string(),
                    tool_calls_json: "[]".to_string(),
                    errors_json: "[]".to_string(),
                    resolution: None,
                    success_score: 0.9,
                    created_by_type: CreatedByType::System,
                    created_by_ref: "agent-3t".to_string(),
                    importance: 5.0,
                    created_at_ms: TimestampMs::now().as_millis_i64() as u64,
                },
                q_vec,
            )
            .await
            .unwrap();

        let emitted = wm.regenerate_auto_section("agent-3t", None, &store).await.unwrap();
        assert_eq!(emitted.len(), 2, "one preference + one skill are rendered by the workspace");

        let result =
            build_memory_prompt(&store, &wm, Some(embedder), "agent-3t", None, QueryTexts::single_shot(query), &emitted, None)
                .await
                .unwrap();

        assert_eq!(result.matches("prefiere voseo siempre").count(), 1, "memory row twice:\n{result}");
        assert_eq!(result.matches("arreglar-test-roto").count(), 1, "skill row twice:\n{result}");
        assert_eq!(
            result.matches("reparé el timeout del cliente").count(),
            1,
            "the episodic row has no workspace channel and must still be delivered once:\n{result}"
        );
    }

    /// Suppressing workspace rows must cost no delivery slots. The workspace
    /// rows are exactly the high-importance ones that rank near the top, so
    /// filtering a fixed-size pool would let a well-stocked workspace starve
    /// the query channel.
    #[tokio::test]
    async fn dedup_does_not_shrink_the_retrieval_budget() {
        use crate::memory::embedder::FakeEmbedder;

        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let embedder = Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>;
        let query = "algo sobre el proyecto";
        let bytes = unit_bytes(&embedder.embed_with_role(query, EmbedRole::Passage).await.unwrap());

        // 5 rows the workspace will render, plus K_FINAL rows it will not.
        for i in 0..5 {
            seed_memory(
                &store,
                "agent-budget",
                &format!("decisión de arquitectura {i}"),
                "architectural_decision",
                1,
                10.0,
                bytes.clone(),
            )
            .await;
        }
        for i in 0..K_FINAL {
            seed_memory(
                &store,
                "agent-budget",
                &format!("mensaje de chat {i}"),
                "chat_message",
                2,
                5.0,
                bytes.clone(),
            )
            .await;
        }

        let emitted = wm.regenerate_auto_section("agent-budget", None, &store).await.unwrap();
        assert_eq!(emitted.len(), 5, "the fixture must actually fill the workspace");

        let result =
            build_memory_prompt(&store, &wm, Some(embedder), "agent-budget", None, QueryTexts::single_shot(query), &emitted, None)
                .await
                .unwrap();

        let delivered = (0..K_FINAL)
            .filter(|i| result.contains(&format!("mensaje de chat {i}")))
            .count();
        assert_eq!(
            delivered, K_FINAL,
            "the retrieval block must still carry K_FINAL hits after the workspace rows are removed"
        );
    }

    /// `format_skill_hits` reported 0 for every query ever issued, because the
    /// pool skipped tier 1 and skills only live there. The workspace carries
    /// the top 5, so the query channel is what reaches the tail beyond them.
    #[tokio::test]
    async fn a_skill_beyond_the_workspace_top_five_is_reachable_by_query() {
        use crate::memory::embedder::FakeEmbedder;
        use crate::memory::record::TableKind;
        use crate::memory::search::{search_all_memories, SearchScope};
        use crate::memory::store_insert::NewProceduralSkillRecord;

        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();

        // Six skills: the workspace renders the top five by access_count, all
        // of which sit at 0 here, so the sixth is only ever reachable by query.
        let mut ids = Vec::new();
        for i in 0..6 {
            let trigger = format!("cuando falla el escenario {i}");
            let emb = embedder.embed_with_role(&trigger, EmbedRole::Passage).await.unwrap();
            let outcome = store
                .promote_to_procedural_skill(
                    NewProceduralSkillRecord {
                        agent_id: "agent-tail".to_string(),
                        project_id: None,
                        skill_name: format!("skill-{i}"),
                        trigger,
                        tool_sequence_json: format!("[\"tool-{i}\"]"),
                        source_trajectory_ids: vec![i as i64 + 1],
                        importance: 8.0,
                        now_ms: 1_700_000_000_000,
                    },
                    Some(emb),
                )
                .await
                .unwrap();
            ids.push(outcome);
        }
        assert_eq!(ids.len(), 6);

        let target = embedder.embed_with_role("cuando falla el escenario 5", EmbedRole::Query).await.unwrap();
        let hits = search_all_memories(&store, &target, None, "agent-tail", SearchScope::all_default(), 10)
            .await
            .unwrap();

        let skill_hits = crate::memory::admin::format_skill_hits(&hits);
        assert_ne!(skill_hits, "skill_hits=0", "the query channel must reach skills");
        assert!(
            hits.iter().any(|h| h.kind == TableKind::Procedural && h.content == "skill-5"),
            "the queried skill must be among the hits"
        );
    }

    /// Reads `(access_count, last_accessed_at)` for one `memory` row.
    async fn memory_access_state(store: &MemoryStore, id: i64) -> (i64, i64) {
        store
            .conn
            .call(move |c| -> Result<(i64, i64), rusqlite::Error> {
                c.query_row(
                    "SELECT access_count, last_accessed_at FROM memory WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap()
    }

    /// Drives the tracker's 30 s timer flush and waits (bounded, real time)
    /// until `witness` shows the flush landed.
    async fn force_flush_until_counted(store: &MemoryStore, witness: i64) {
        tokio::time::advance(std::time::Duration::from_secs(31)).await;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while memory_access_state(store, witness).await.0 == 0 {
            assert!(std::time::Instant::now() < deadline, "the access flush never landed");
            tokio::task::yield_now().await;
        }
    }

    /// The flush only knows how to update `memory`, so a hit from another
    /// table must not reach it: its id would land on whatever `memory` row
    /// happens to share that number.
    #[tokio::test(start_paused = true)]
    async fn a_hit_from_another_table_does_not_touch_the_memory_row_with_the_same_id() {
        use crate::memory::store_insert::NewProceduralSkillRecord;

        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let embedder = Arc::new(FixedVecEmbedder) as Arc<dyn Embedder>;
        let agent = "agent-id-collision";

        let mut like_query = vec![0.0_f32; 384];
        like_query[0] = 1.0;

        // Id 1: another agent's row, so it is invisible to this search.
        let bystander = seed_memory(&store, "agent-someone-else", "bystander", "chat_message", 3, 5.0, unit_bytes(&like_query)).await;
        // Id 2: a real memory hit, the witness that the flush ran.
        let witness = seed_memory(&store, agent, "witness", "chat_message", 3, 5.0, unit_bytes(&like_query)).await;
        // The skill table is empty, so its first row gets the bystander's id.
        store
            .promote_to_procedural_skill(
                NewProceduralSkillRecord {
                    agent_id: agent.to_string(),
                    project_id: None,
                    skill_name: "colliding-skill".to_string(),
                    trigger: "witness".to_string(),
                    tool_sequence_json: "[]".to_string(),
                    source_trajectory_ids: vec![1],
                    importance: 8.0,
                    now_ms: 1_700_000_000_000,
                },
                Some(like_query),
            )
            .await
            .unwrap();
        let before = memory_access_state(&store, bystander).await;

        let tracker = AccessTracker::new(store.conn.clone());
        let result = build_memory_prompt(
            &store,
            &wm,
            Some(embedder),
            agent,
            None,
            QueryTexts::single_shot("witness"),
            &EmittedIds::default(),
            Some(&tracker),
        )
        .await
        .unwrap();
        assert!(result.contains("colliding-skill"), "the skill must be a hit: {result}");
        assert!(!result.contains("bystander"), "the bystander must not be a hit: {result}");

        force_flush_until_counted(&store, witness).await;

        assert_eq!(
            memory_access_state(&store, bystander).await,
            before,
            "a skill hit with the same id must not count as an access to the memory row"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_memory_hit_is_recorded_by_the_access_tracker() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let embedder = Arc::new(FixedVecEmbedder) as Arc<dyn Embedder>;
        let agent = "agent-access-control";

        let mut like_query = vec![0.0_f32; 384];
        like_query[0] = 1.0;
        let hit = seed_memory(&store, agent, "witness", "chat_message", 3, 5.0, unit_bytes(&like_query)).await;

        let tracker = AccessTracker::new(store.conn.clone());
        build_memory_prompt(
            &store,
            &wm,
            Some(embedder),
            agent,
            None,
            QueryTexts::single_shot("witness"),
            &EmittedIds::default(),
            Some(&tracker),
        )
        .await
        .unwrap();

        force_flush_until_counted(&store, hit).await;

        assert_eq!(memory_access_state(&store, hit).await.0, 1);
    }

    #[tokio::test]
    async fn e2e_full_4_block_output() {
        use crate::memory::embedder::FakeEmbedder;

        let store = MemoryStore::new_for_test().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));

        let embedder = Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>;
        let fake_vec = embedder.embed_with_role("dummy", EmbedRole::Passage).await.unwrap();
        let fake_bytes = crate::memory::search::serialize_f32_query_pub(&fake_vec);

        let now_ms: i64 = TimestampMs::now().as_millis_i64();

        let recent_ts = now_ms - 86_400_000;
        let old_ts = now_ms - 30 * 86_400_000;

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
        let emitted = wm.regenerate_auto_section("e2e-agent", None, &store).await.unwrap();

        let result = build_memory_prompt(
            &store, &wm, Some(embedder), "e2e-agent", None, QueryTexts::single_shot("some query"), &emitted, None
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

        // …and nowhere else. The row is now inside the KNN scan set too, so
        // without the emitted-id filter the same preference would be spent
        // twice in the same prompt: once as standing context, once as a
        // retrieval hit.
        assert_eq!(
            result.matches("prefer dark mode").count(),
            1,
            "a row the workspace already carries must not also arrive as a retrieval hit:\n{result}"
        );

        // CONFLICTS block must contain the disputed pair
        let conflicts_block = &result[conf_pos..];
        assert!(conflicts_block.contains("original fact") || conflicts_block.contains("disputed fact"),
            "CONFLICTS must reference the disputed pair");
    }
}
