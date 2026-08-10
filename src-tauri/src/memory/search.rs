//! KNN search across memory + episodic + procedural via tokio::try_join!,
//! merged + ranked by the score formula computed in SQL.

use std::time::Instant;

use crate::memory::clock::TimestampMs;
use crate::memory::vector::to_unit;
use crate::memory::{MemoryError, MemoryStore, TableKind};

/// Per-table k_pool selection. `None` means "skip this table entirely".
#[derive(Debug, Clone, Copy)]
pub(crate) struct SearchScope {
    pub memory: Option<usize>,
    pub episodic: Option<usize>,
    pub procedural: Option<usize>,
}

impl SearchScope {
    pub(crate) fn all_default() -> Self {
        Self {
            memory: Some(30),
            episodic: Some(5),
            procedural: Some(5),
        }
    }
    /// Procedural-only scope — (None, None, Some(10)).
    pub(crate) fn procedural_only() -> Self {
        Self {
            memory: None,
            episodic: None,
            procedural: Some(10),
        }
    }
    /// Scope preset for surfacing factual-pivot candidates. Not currently
    /// called from production code — the live contradiction detector in
    /// `contradiction.rs` builds its own narrower scope (memory-only, no
    /// procedural) inline rather than using this constructor.
    pub(crate) fn factual_pivots() -> Self {
        Self {
            memory: Some(5),
            episodic: None,
            procedural: Some(5),
        }
    }
}

/// One result row after the score formula has been applied SQL-side.
#[derive(Debug, Clone)]
pub(crate) struct RankedHit {
    pub id: i64,
    pub kind: TableKind,
    pub content: String,
    pub memory_type: String,
    pub tier: i64,
    pub status: String,
    pub score: f64,
    pub distance: f64,   // raw KNN distance — consumed by contradiction.rs to threshold-filter near-duplicate candidates
    pub created_at: TimestampMs, // for RECENT vs LONG-TERM bucketing in prompt_builder
}

// Compile-time Send + Sync assertions
const _: fn() = || {
    fn _ss<T: Send + Sync>() {}
    _ss::<SearchScope>();
    _ss::<RankedHit>();
};

/// Encode query vector as little-endian bytes for vec0 MATCH operator.
fn serialize_f32_query(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Public test helper — exposes serialize_f32_query for cross-module test use.
#[cfg(test)]
pub(crate) fn serialize_f32_query_pub(v: &[f32]) -> Vec<u8> {
    serialize_f32_query(v)
}

// SQL for memory table KNN
// NOTE: vec0 requires LIMIT directly on the vec0 query. We use a subquery to get
// the top-k by distance from vec0, then join and filter in the outer query.
//
// Stored stamps (`last_accessed_at`) are milliseconds and `exp_decay`'s first
// argument is a whole number of seconds, so both sides of the subtraction are
// lifted to milliseconds and the difference divided back down — `now` and the
// stamp are never rounded apart. The division is integer because that argument
// is read as an integer and a real value is rejected at runtime.
const SEARCH_SQL_MEMORY: &str = "
    SELECT m.id, m.content, m.memory_type, m.tier, m.status, knn.distance, m.created_at,
        cosine_from_l2_unit(knn.distance)
        * MAX(
            CASE m.tier WHEN 1 THEN 0.5 WHEN 2 THEN 0.2 ELSE 0.05 END,
            exp_decay((strftime('%s','now') * 1000 - m.last_accessed_at) / 1000, lambda_for_type(m.memory_type))
          )
        * importance_factor(m.importance)
        * dispute_factor(m.status)
        AS score
    FROM memory_vectors knn
    JOIN memory m ON m.id = knn.rowid
    WHERE knn.embedding MATCH ?1
      AND knn.k = ?2
      AND m.status IN ('active', 'disputed')
      AND m.deleted_at IS NULL
      AND m.agent_id = ?3
      AND m.tier > 1
    ORDER BY score DESC
";

// SQL for episodic table KNN (hardcoded lambda 0.0077)
const SEARCH_SQL_EPISODIC: &str = "
    SELECT epi.id, epi.task, 'episode' AS memory_type, epi.tier, epi.status, knn.distance, epi.created_at,
        cosine_from_l2_unit(knn.distance)
        * MAX(
            CASE epi.tier WHEN 1 THEN 0.5 WHEN 2 THEN 0.2 ELSE 0.05 END,
            exp_decay((strftime('%s','now') * 1000 - epi.last_accessed_at) / 1000, 0.0077)
          )
        * importance_factor(epi.importance)
        * dispute_factor(epi.status)
        AS score
    FROM episodic_vectors knn
    JOIN episodic_trajectory epi ON epi.id = knn.rowid
    WHERE knn.embedding MATCH ?1
      AND knn.k = ?2
      AND epi.status IN ('active', 'disputed')
      AND epi.deleted_at IS NULL
      AND epi.agent_id = ?3
      AND epi.tier > 1
    ORDER BY score DESC
";

// SQL for procedural table KNN (hardcoded lambda 0.0039)
const SEARCH_SQL_PROCEDURAL: &str = "
    SELECT pro.id, pro.skill_name, 'procedure' AS memory_type, pro.tier, pro.status, knn.distance, pro.created_at,
        cosine_from_l2_unit(knn.distance)
        * MAX(
            CASE pro.tier WHEN 1 THEN 0.5 WHEN 2 THEN 0.2 ELSE 0.05 END,
            exp_decay((strftime('%s','now') * 1000 - pro.last_accessed_at) / 1000, 0.0039)
          )
        * importance_factor(pro.importance)
        * dispute_factor(pro.status)
        AS score
    FROM procedural_vectors knn
    JOIN procedural_skill pro ON pro.id = knn.rowid
    WHERE knn.embedding MATCH ?1
      AND knn.k = ?2
      AND pro.status IN ('active', 'disputed')
      AND pro.deleted_at IS NULL
      AND pro.agent_id = ?3
      AND pro.tier > 1
    ORDER BY score DESC
";

async fn knn_memory(
    store: &MemoryStore,
    q_bytes: &[u8],
    agent_id: &str,
    k: usize,
) -> Result<Vec<RankedHit>, MemoryError> {
    let q = q_bytes.to_vec();
    let agent = agent_id.to_string();
    let k_i64 = k as i64;
    store
        .conn
        .call(move |c| {
            let mut stmt = c.prepare(SEARCH_SQL_MEMORY)?;
            let rows = stmt.query_map(
                rusqlite::params![q, k_i64, agent],
                |row| {
                    Ok(RankedHit {
                        id: row.get(0)?,
                        kind: TableKind::Memory,
                        content: row.get(1)?,
                        memory_type: row.get(2)?,
                        tier: row.get(3)?,
                        status: row.get(4)?,
                        distance: row.get(5)?,
                        created_at: row.get(6)?,
                        score: row.get(7)?,
                    })
                },
            )?;
            let mut hits = Vec::new();
            for row in rows {
                hits.push(row?);
            }
            Ok(hits)
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend("connection closed during knn_memory".into())
            }
            other => MemoryError::Backend(other.to_string()),
        })
}

async fn knn_episodic(
    store: &MemoryStore,
    q_bytes: &[u8],
    agent_id: &str,
    k: usize,
) -> Result<Vec<RankedHit>, MemoryError> {
    let q = q_bytes.to_vec();
    let agent = agent_id.to_string();
    let k_i64 = k as i64;
    store
        .conn
        .call(move |c| {
            let mut stmt = c.prepare(SEARCH_SQL_EPISODIC)?;
            let rows = stmt.query_map(
                rusqlite::params![q, k_i64, agent],
                |row| {
                    Ok(RankedHit {
                        id: row.get(0)?,
                        kind: TableKind::Episodic,
                        content: row.get(1)?,
                        memory_type: row.get(2)?,
                        tier: row.get(3)?,
                        status: row.get(4)?,
                        distance: row.get(5)?,
                        created_at: row.get(6)?,
                        score: row.get(7)?,
                    })
                },
            )?;
            let mut hits = Vec::new();
            for row in rows {
                hits.push(row?);
            }
            Ok(hits)
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend("connection closed during knn_episodic".into())
            }
            other => MemoryError::Backend(other.to_string()),
        })
}

async fn knn_procedural(
    store: &MemoryStore,
    q_bytes: &[u8],
    agent_id: &str,
    k: usize,
) -> Result<Vec<RankedHit>, MemoryError> {
    let q = q_bytes.to_vec();
    let agent = agent_id.to_string();
    let k_i64 = k as i64;
    store
        .conn
        .call(move |c| {
            let mut stmt = c.prepare(SEARCH_SQL_PROCEDURAL)?;
            let rows = stmt.query_map(
                rusqlite::params![q, k_i64, agent],
                |row| {
                    Ok(RankedHit {
                        id: row.get(0)?,
                        kind: TableKind::Procedural,
                        content: row.get(1)?,
                        memory_type: row.get(2)?,
                        tier: row.get(3)?,
                        status: row.get(4)?,
                        distance: row.get(5)?,
                        created_at: row.get(6)?,
                        score: row.get(7)?,
                    })
                },
            )?;
            let mut hits = Vec::new();
            for row in rows {
                hits.push(row?);
            }
            Ok(hits)
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend("connection closed during knn_procedural".into())
            }
            other => MemoryError::Backend(other.to_string()),
        })
}

/// Top-level KNN orchestrator. Returns up to `k_final` hits sorted DESC by score.
pub(crate) async fn search_all_memories(
    store: &MemoryStore,
    embedding: &[f32],
    agent_id: &str,
    scope: SearchScope,
    k_final: usize,
) -> Result<Vec<RankedHit>, MemoryError> {
    // Same funnel every caller shares (prompt_builder.rs, contradiction.rs,
    // eval_api.rs): an unusable query vector means "no direction to search
    // with", which is a caller-input problem, not a store failure — so this
    // returns an empty result set rather than propagating `Err` and aborting
    // whatever multi-step flow called it (eval_api.rs in particular turns a
    // hard `Err` here into an aborted eval run).
    let unit = match to_unit(embedding) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("search_all_memories: discarding unusable query vector: {e}");
            return Ok(Vec::new());
        }
    };

    let t0 = Instant::now();
    let q_bytes = serialize_f32_query(&unit);

    let (mem, epi, pro) = tokio::try_join!(
        async {
            match scope.memory {
                Some(k) => knn_memory(store, &q_bytes, agent_id, k).await,
                None => Ok(Vec::new()),
            }
        },
        async {
            match scope.episodic {
                Some(k) => knn_episodic(store, &q_bytes, agent_id, k).await,
                None => Ok(Vec::new()),
            }
        },
        async {
            match scope.procedural {
                Some(k) => knn_procedural(store, &q_bytes, agent_id, k).await,
                None => Ok(Vec::new()),
            }
        },
    )?;

    let (mem_returned, epi_returned, pro_returned) = (mem.len(), epi.len(), pro.len());
    let mut all: Vec<RankedHit> = mem.into_iter().chain(epi).chain(pro).collect();
    all.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all.truncate(k_final);

    // Read once so the per-call number logged below and the sample fed into
    // the latency window describe the same instant — a second read would
    // measure a different span.
    let elapsed = t0.elapsed();
    // Recorded unconditionally, not from inside the log::debug! args below:
    // that macro short-circuits argument evaluation when debug logging is
    // off, which is fine for the percentile snapshot but would leave the
    // window empty while debug logging is off and reporting `warming` for a
    // long time after someone turns it on.
    store.knn_latency().record(elapsed);

    // Tier-per-hit and pool-survival logging — appended to the existing
    // retrieval debug line via pure, unit-tested helpers (t1 is always 0
    // here: the SEARCH_SQL_* `tier > 1` filter means tier-1 rows never reach
    // KNN, not a bug). `snapshot()` sits inside the macro args deliberately:
    // it is the one part of this line that's fine to skip sorting for when
    // debug logging is off.
    log::debug!(
        "retrieval={}ms hits={} {} k_final={} dist={} {}",
        elapsed.as_millis(),
        all.len(),
        crate::commands::memory_admin::format_pool_survival(
            (
                scope.memory.unwrap_or(0),
                scope.episodic.unwrap_or(0),
                scope.procedural.unwrap_or(0),
            ),
            (mem_returned, epi_returned, pro_returned),
        ),
        k_final,
        crate::commands::memory_admin::format_tier_distribution(&all),
        store.knn_latency().snapshot(),
    );

    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::MemoryStore;
    use crate::memory::embedder::Embedder;
    use rusqlite::params;

    fn serialize_embedding(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    /// A unit basis vector `[1.0, 0.0, …]` — the "query == stored, so the
    /// similarity factor is neutral" fixture. An all-zero vector no longer
    /// serves that purpose: `to_unit` rejects zero magnitude, so both sides
    /// of every self-similarity test below use this basis vector instead —
    /// distance stays exactly 0 and `cosine_from_l2_unit(0.0) = 1.0`, the
    /// same neutral factor an unnormalized `(1.0 - 0)` produced before.
    fn unit_basis_vec() -> Vec<f32> {
        let mut v = vec![0.0_f32; 384];
        v[0] = 1.0;
        v
    }

    fn unit_basis_embedding() -> Vec<u8> {
        serialize_embedding(&unit_basis_vec())
    }

    async fn insert_memory_row(
        store: &MemoryStore,
        agent_id: &str,
        content: &str,
        memory_type: &str,
        tier: i64,
        importance: f64,
        status: &str,
        embedding: Vec<u8>,
    ) -> i64 {
        let agent = agent_id.to_string();
        let content = content.to_string();
        let memory_type = memory_type.to_string();
        let status = status.to_string();
        store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                     created_at, created_by_type, created_by_ref,
                     last_modified_at, last_accessed_at, importance, tier, status)
                     VALUES ('agent', ?1, ?2, ?3, 'chat', 0, 'user', ?1, 0, 0, ?4, ?5, ?6)",
                    params![agent, content, memory_type, importance, tier, status],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                    params![id, embedding],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    #[test]
    fn scope_all_default_values() {
        let s = SearchScope::all_default();
        assert_eq!(s.memory, Some(30));
        assert_eq!(s.episodic, Some(5));
        assert_eq!(s.procedural, Some(5));
    }

    #[test]
    fn scope_procedural_only_values() {
        let s = SearchScope::procedural_only();
        assert_eq!(s.memory, None);
        assert_eq!(s.episodic, None);
        assert_eq!(s.procedural, Some(10));
    }

    #[test]
    fn scope_factual_pivots_values() {
        let s = SearchScope::factual_pivots();
        assert_eq!(s.episodic, None);
        assert_eq!(s.memory, Some(5));
        assert_eq!(s.procedural, Some(5));
    }

    #[test]
    fn ranked_hit_is_send() {
        fn _s<T: Send>() {}
        _s::<RankedHit>();
    }

    #[test]
    fn ranked_hit_sort_descending_by_score() {
        let h1 = RankedHit {
            id: 1,
            kind: TableKind::Memory,
            content: "a".into(),
            memory_type: "chat_message".into(),
            tier: 3,
            status: "active".into(),
            score: 0.9,
            distance: 0.1,
            created_at: TimestampMs::from_unix_millis(0),
        };
        let h2 = RankedHit {
            id: 2,
            kind: TableKind::Memory,
            content: "b".into(),
            memory_type: "chat_message".into(),
            tier: 3,
            status: "active".into(),
            score: 0.3,
            distance: 0.7,
            created_at: TimestampMs::from_unix_millis(0),
        };
        let mut hits = vec![h2, h1];
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        assert!((hits[0].score - 0.9).abs() < 1e-9);
    }

    #[test]
    fn serialize_f32_query_produces_le_bytes() {
        let v = vec![1.0_f32, 0.0_f32];
        let bytes = serialize_f32_query(&v);
        assert_eq!(&bytes[0..4], &1.0_f32.to_le_bytes());
    }

    #[test]
    fn serialize_f32_query_length_matches() {
        let v = vec![0.0_f32; 384];
        let bytes = serialize_f32_query(&v);
        assert_eq!(bytes.len(), 384 * 4);
    }

    #[tokio::test]
    async fn tier1_rows_excluded_from_results() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();

        // Insert one tier=1 row and three tier=3 rows
        insert_memory_row(&store, "agent-test", "tier1 content", "chat_message", 1, 5.0, "active", emb.clone()).await;
        insert_memory_row(&store, "agent-test", "tier3 a", "chat_message", 3, 5.0, "active", emb.clone()).await;
        insert_memory_row(&store, "agent-test", "tier3 b", "chat_message", 3, 5.0, "active", emb.clone()).await;
        insert_memory_row(&store, "agent-test", "tier3 c", "chat_message", 3, 5.0, "active", emb.clone()).await;

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, "agent-test", SearchScope::all_default(), 10)
            .await
            .unwrap();

        assert!(hits.iter().all(|h| h.tier > 1), "tier=1 row must not appear in results");
        assert_eq!(hits.len(), 3, "expected 3 tier-3 hits");

        // The assertion above can't distinguish "the query correctly returned
        // 3" from "the query returned something else and a later truncation
        // coincidentally landed on 3". Calling the pool query directly, before
        // any merge or truncation, tests the tier filter in isolation.
        assert_eq!(
            knn_memory(&store, &emb, "agent-test", 30).await.unwrap().len(),
            3,
            "the tier filter must drop the tier-1 row inside the query, before any merge or truncation"
        );
    }

    // A query vector reaching `search_all_memories` at any magnitude must be
    // compared in the same unit space every stored vector lives in — otherwise
    // "distance" mixes an arbitrary caller-side scale with the store's, which
    // is meaningless. Stores a unit vector directly (bypassing the insert
    // path's own normalization, which is under test elsewhere) and queries
    // with the same direction scaled ×10.
    #[tokio::test]
    async fn non_unit_query_is_normalized_by_search() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mut unit = vec![0.0_f32; 384];
        unit[0] = 1.0;
        let stored_bytes = serialize_embedding(&unit);
        let id = insert_memory_row(
            &store, "agent-normalize", "content", "chat_message", 3, 5.0, "active", stored_bytes,
        )
        .await;

        let query: Vec<f32> = unit.iter().map(|x| x * 10.0).collect();
        let hits = search_all_memories(&store, &query, "agent-normalize", SearchScope::all_default(), 10)
            .await
            .unwrap();

        let hit = hits.iter().find(|h| h.id == id).expect("expected the row to be retrievable");
        assert!(
            hit.distance < 1e-3,
            "query vector must be normalized before the KNN match: distance={}",
            hit.distance
        );
    }

    // The query-side counterpart of insertion's zero-vector rejection
    // (store_insert.rs). `to_unit` rejects a zero-magnitude query vector at
    // search.rs:302-308, and that branch fails soft (`warn!` + `Ok(vec![])`)
    // rather than propagating `Err` — `eval_api.rs` turns an `Err` here into
    // an aborted eval run, so an unusable query vector must degrade to "no
    // hits" instead of tearing down the caller. A row is inserted first so
    // the assertion proves the search legitimately found nothing (not that
    // the table was empty to begin with).
    #[tokio::test]
    async fn zero_query_vector_is_discarded_fail_soft_not_propagated_as_error() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        insert_memory_row(&store, "agent-zero-query", "content", "chat_message", 3, 5.0, "active", emb).await;

        let zero_query = vec![0.0_f32; 384];
        let hits = search_all_memories(&store, &zero_query, "agent-zero-query", SearchScope::all_default(), 10)
            .await
            .expect("an unusable query vector must fail soft (Ok(vec![])), not propagate Err");

        assert!(
            hits.is_empty(),
            "a zero query vector carries no direction to search with, so it must return zero hits, \
             not the row above at some fallback distance"
        );
    }

    #[tokio::test]
    async fn superseded_rows_excluded_from_results() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();

        insert_memory_row(&store, "agent-test", "superseded", "chat_message", 3, 5.0, "superseded", emb.clone()).await;
        insert_memory_row(&store, "agent-test", "active", "chat_message", 3, 5.0, "active", emb.clone()).await;

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, "agent-test", SearchScope::all_default(), 10)
            .await
            .unwrap();

        assert!(hits.iter().all(|h| h.status != "superseded"), "superseded rows must not appear");
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn disputed_rows_included_with_degraded_score() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();

        insert_memory_row(&store, "agent-test", "disputed content", "chat_message", 3, 5.0, "disputed", emb.clone()).await;
        insert_memory_row(&store, "agent-test", "active content", "chat_message", 3, 5.0, "active", emb.clone()).await;

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, "agent-test", SearchScope::all_default(), 10)
            .await
            .unwrap();

        let disputed = hits.iter().find(|h| h.status == "disputed");
        assert!(disputed.is_some(), "disputed row must appear in results");
    }

    #[tokio::test]
    async fn score_formula_integration_spot_check() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();

        // Insert row with tier=2, memory_type='chat_message', importance=5.0, status='active'
        // The `last_accessed_at` is set to 0 in insert helper, so age will be large → use a recent value
        let agent = "agent-spot";
        let emb_clone = emb.clone();
        store.conn.call(move |c| -> Result<(), rusqlite::Error> {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', ?1, 'spot check content', 'chat_message', 'chat',
                 0, 'user', ?1, 0, (strftime('%s','now')*1000) - 86400000, 5.0, 2, 'active')",
                params![agent],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                params![id, emb_clone],
            )?;
            tx.commit()?;
            Ok(())
        }).await.unwrap();

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, agent, SearchScope::all_default(), 10)
            .await
            .unwrap();

        assert!(!hits.is_empty(), "expected at least 1 hit");
        let score = hits[0].score;
        // score ≈ (1.0 - distance) * MAX(0.2, exp_decay(86400, 0.023)) * importance_factor(5.0) * dispute_factor('active')
        // distance ≈ 0 (identical vectors), decay ≈ 0.977, imp=0.75, dispute=1.0
        // ≈ 1.0 * 0.977 * 0.75 * 1.0 ≈ 0.73
        assert!(score > 0.0 && score <= 1.0, "score out of range: {score}");
        assert!(score > 0.5, "expected score > 0.5, got {score}");
    }

    #[tokio::test]
    async fn fresh_last_accessed_outranks_stale_last_accessed_when_otherwise_identical() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-recency-discriminates";

        // Both rows share the same embedding, tier, importance, status and
        // memory_type — last_accessed_at is the only input that differs, so any
        // score gap between them is attributable to the decay term alone.
        let emb_fresh = emb.clone();
        let a_fresh = agent.to_string();
        let fresh_id = store.conn.call(move |c| -> Result<i64, rusqlite::Error> {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', ?1, 'fresh row', 'chat_message', 'chat',
                 0, 'user', ?1, 0, strftime('%s','now') * 1000, 5.0, 2, 'active')",
                params![a_fresh],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                params![id, emb_fresh],
            )?;
            tx.commit()?;
            Ok(id)
        }).await.unwrap();

        let emb_stale = emb.clone();
        let a_stale = agent.to_string();
        let stale_id = store.conn.call(move |c| -> Result<i64, rusqlite::Error> {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', ?1, 'stale row', 'chat_message', 'chat',
                 0, 'user', ?1, 0, (strftime('%s','now') * 1000) - (30 * 86400000), 5.0, 2, 'active')",
                params![a_stale],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                params![id, emb_stale],
            )?;
            tx.commit()?;
            Ok(id)
        }).await.unwrap();

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, agent, SearchScope::all_default(), 10)
            .await
            .unwrap();

        let fresh_hit = hits.iter().find(|h| h.id == fresh_id).expect("fresh row must be retrievable");
        let stale_hit = hits.iter().find(|h| h.id == stale_id).expect("stale row must be retrievable");

        // Identical embeddings mean the KNN distance contribution is identical
        // too — isolating last_accessed_at as the only variable left standing
        // between the two scores.
        assert_eq!(
            fresh_hit.distance, stale_hit.distance,
            "both rows share the same embedding, so distance must be identical"
        );
        assert!(
            fresh_hit.score > stale_hit.score,
            "a row accessed moments ago must score strictly higher than an otherwise-identical \
             row accessed 30 days ago: fresh={} stale={}",
            fresh_hit.score,
            stale_hit.score
        );
    }

    #[tokio::test]
    async fn disputed_status_reduces_score_by_factor_0_6() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-dispute-test";

        // Insert active version
        let emb2 = emb.clone();
        let a2 = agent.to_string();
        let active_id = store.conn.call(move |c| -> Result<i64, rusqlite::Error> {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', ?1, 'active version', 'chat_message', 'chat',
                 0, 'user', ?1, 0, (strftime('%s','now')*1000) - 86400000, 5.0, 3, 'active')",
                params![a2],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)", params![id, emb2])?;
            tx.commit()?;
            Ok(id)
        }).await.unwrap();

        let q_vec = unit_basis_vec();
        let active_hits = search_all_memories(&store, &q_vec, agent, SearchScope::all_default(), 10)
            .await.unwrap();
        let active_score = active_hits.iter().find(|h| h.id == active_id).unwrap().score;

        // Insert disputed version
        let emb3 = emb.clone();
        let a3 = agent.to_string();
        let disputed_id = store.conn.call(move |c| -> Result<i64, rusqlite::Error> {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', ?1, 'disputed version', 'chat_message', 'chat',
                 0, 'user', ?1, 0, (strftime('%s','now')*1000) - 86400000, 5.0, 3, 'disputed')",
                params![a3],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)", params![id, emb3])?;
            tx.commit()?;
            Ok(id)
        }).await.unwrap();

        let all_hits = search_all_memories(&store, &q_vec, agent, SearchScope::all_default(), 10)
            .await.unwrap();
        let disputed_score = all_hits.iter().find(|h| h.id == disputed_id).unwrap().score;

        assert!(disputed_score <= active_score * 0.61,
            "disputed score {disputed_score} should be <= 0.6 * active score {active_score}");
    }

    #[tokio::test]
    async fn search_returns_results_sorted_by_score_desc() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-sort";

        for i in 0..3i64 {
            let emb_clone = emb.clone();
            let a = agent.to_string();
            let imp = (i + 1) as f64 * 2.0 + 4.0; // 6.0, 8.0, 10.0
            store.conn.call(move |c| -> Result<(), rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                     created_at, created_by_type, created_by_ref,
                     last_modified_at, last_accessed_at, importance, tier, status)
                     VALUES ('agent', ?1, 'content', 'chat_message', 'chat',
                     0, 'user', ?1, 0, (strftime('%s','now')*1000) - 86400000, ?2, 3, 'active')",
                    params![a, imp],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)", params![id, emb_clone])?;
                tx.commit()?;
                Ok(())
            }).await.unwrap();
        }

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, agent, SearchScope::all_default(), 3)
            .await.unwrap();

        assert_eq!(hits.len(), 3);
        assert!(hits[0].score >= hits[1].score, "must be sorted DESC");
        assert!(hits[1].score >= hits[2].score, "must be sorted DESC");
    }

    #[tokio::test]
    async fn search_respects_scope_none_skips_table() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let q_vec = unit_basis_vec();
        let scope = SearchScope { memory: None, episodic: None, procedural: None };
        let hits = search_all_memories(&store, &q_vec, "agent-x", scope, 10)
            .await.unwrap();
        assert!(hits.is_empty(), "all scopes None → empty results");
    }

    #[tokio::test]
    async fn search_truncates_to_k_final() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-trunc";

        for _ in 0..5 {
            let emb_clone = emb.clone();
            let a = agent.to_string();
            store.conn.call(move |c| -> Result<(), rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                     created_at, created_by_type, created_by_ref,
                     last_modified_at, last_accessed_at, importance, tier, status)
                     VALUES ('agent', ?1, 'content', 'chat_message', 'chat',
                     0, 'user', ?1, 0, 0, 5.0, 3, 'active')",
                    params![a],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)", params![id, emb_clone])?;
                tx.commit()?;
                Ok(())
            }).await.unwrap();
        }

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, agent, SearchScope::all_default(), 2)
            .await.unwrap();

        assert_eq!(hits.len(), 2, "k_final=2 must return exactly 2 hits");

        // The assertion above can't distinguish correct truncation from a
        // pool that was already short before truncation ran. Asserting the
        // pre-truncation pool size directly proves k_final is what cuts the
        // result, not a starved pool.
        assert_eq!(
            knn_memory(&store, &emb, agent, 30).await.unwrap().len(),
            5,
            "all five must survive the query; k_final is what cuts the result, not the pool"
        );
    }

    // ── KNN latency isolation and recording ──

    #[tokio::test]
    async fn two_stores_do_not_share_latency_samples() {
        let store_a = MemoryStore::new_for_test().await.unwrap();
        let store_b = MemoryStore::new_for_test().await.unwrap();
        let q_vec = unit_basis_vec();

        search_all_memories(&store_a, &q_vec, "agent-iso-a", SearchScope::all_default(), 10)
            .await
            .unwrap();

        assert_eq!(
            store_b.knn_latency().snapshot().n,
            0,
            "a separate store must not observe another store's latency samples"
        );
    }

    #[tokio::test]
    async fn cloned_store_shares_latency_samples() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let clone = store.clone();
        let q_vec = unit_basis_vec();

        search_all_memories(&store, &q_vec, "agent-iso-clone", SearchScope::all_default(), 10)
            .await
            .unwrap();

        assert_eq!(
            clone.knn_latency().snapshot().n,
            1,
            "a clone addresses the same database, so it must observe the same recorder"
        );
    }

    #[tokio::test]
    async fn each_search_records_exactly_one_sample() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let q_vec = unit_basis_vec();

        for _ in 0..3 {
            search_all_memories(&store, &q_vec, "agent-iso-n", SearchScope::all_default(), 10)
                .await
                .unwrap();
        }

        assert_eq!(store.knn_latency().snapshot().n, 3);
    }

    #[tokio::test]
    async fn search_empty_db_returns_empty_vec() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, "agent-x", SearchScope::all_default(), 10)
            .await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn integration_5_rows_fake_embedder_top3_sorted() {
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-integration";

        // Use FakeEmbedder to generate an embedding for a fixed text and store
        // that SAME direction so distance will be small/zero (≈ same embedding).
        // `insert_memory_row` bypasses `insert_memory_with_vector` (raw SQL, no
        // `to_unit`), but `search_all_memories` now normalizes the query side —
        // so the stored fixture is normalized here too, or the two would compare
        // FakeEmbedder's raw ~11.3 magnitude against a unit query and never match.
        let embedder = FakeEmbedder::default();
        let q_vec = embedder.embed("test").await.unwrap();
        let emb_bytes = serialize_f32_query(&to_unit(&q_vec).unwrap());

        // Insert 5 rows: 2 tier=3, 1 tier=1, 2 tier=2
        let rows = vec![
            ("tier3-a", "chat_message", 3i64, 5.0f64, "active"),
            ("tier3-b", "user_preference", 3, 7.0, "active"),
            ("tier1-c", "chat_message", 1, 9.0, "active"),  // tier=1 excluded
            ("tier2-d", "chat_message", 2, 5.0, "active"),
            ("tier2-e", "user_preference", 2, 8.0, "active"),
        ];

        for (content, mtype, tier, imp, status) in rows {
            insert_memory_row(&store, agent, content, mtype, tier, imp, status, emb_bytes.clone()).await;
        }

        let hits = search_all_memories(&store, &q_vec, agent, SearchScope::all_default(), 3)
            .await.unwrap();

        assert_eq!(hits.len(), 3, "expected exactly 3 hits");
        assert!(hits.iter().all(|h| h.tier > 1), "no tier=1 in results");
        // When query vec == stored vec, distance ≈ 0, so (1-distance) ≈ 1 → score > 0
        assert!(hits.iter().all(|h| h.score > 0.0), "scores must be positive");
        assert!(hits[0].score >= hits[1].score, "sorted DESC");
        assert!(hits[1].score >= hits[2].score, "sorted DESC");
    }

    // ── episodic_trajectory KNN retrieval ──
    //
    // No production code change here: SEARCH_SQL_EPISODIC's pre-existing
    // `epi.agent_id = ?3 AND epi.tier > 1` filter already supports
    // `episodic_trajectory` rows (tier=3 via compute_tier) without modification.
    // These tests exist to prove that, not to add new behavior.

    async fn insert_episodic_row_for_knn(
        store: &MemoryStore,
        agent_id: &str,
        task: &str,
        embedding: Vec<f32>,
    ) -> i64 {
        use crate::memory::store_insert::{CreatedByType, NewEpisodicTrajectoryRecord};
        let record = NewEpisodicTrajectoryRecord {
            scope: "agent",
            agent_id: Some(agent_id.to_string()),
            project_id: None,
            session_id: format!("sess-knn-{agent_id}"),
            task: task.to_string(),
            tool_calls_json: "[]".to_string(),
            errors_json: "[]".to_string(),
            resolution: None,
            success_score: 0.9,
            created_by_type: CreatedByType::System,
            created_by_ref: agent_id.to_string(),
            importance: 5.0,
            created_at_ms: 0,
        };
        store.insert_episodic_trajectory(record, embedding).await.unwrap()
    }

    // Row with agent_id="agent-a" is KNN-retrievable when the query is
    // searched with the SAME agent_id; surfaced as memory_type='episode'.
    #[tokio::test]
    async fn episodic_trajectory_retrievable_by_same_agent_id() {
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let task = "depurar timeout de red";
        let emb = embedder.embed(task).await.unwrap();
        let id = insert_episodic_row_for_knn(&store, "agent-a", task, emb.clone()).await;

        let hits = search_all_memories(&store, &emb, "agent-a", SearchScope::all_default(), 10)
            .await
            .unwrap();

        let hit = hits
            .iter()
            .find(|h| h.id == id)
            .expect("expected episodic row to be retrievable for the same agent_id");
        assert_eq!(hit.memory_type, "episode", "episodic rows must surface memory_type='episode'");
    }

    // The SAME row is NOT retrievable for a DIFFERENT agent_id — expected
    // behavior (filter exclusion), not a regression.
    #[tokio::test]
    async fn episodic_trajectory_not_retrievable_by_different_agent_id() {
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let task = "depurar timeout de red";
        let emb = embedder.embed(task).await.unwrap();
        let id = insert_episodic_row_for_knn(&store, "agent-a", task, emb.clone()).await;

        let hits = search_all_memories(&store, &emb, "agent-b", SearchScope::all_default(), 10)
            .await
            .unwrap();

        assert!(
            !hits.iter().any(|h| h.id == id),
            "row must NOT be retrievable for a different agent_id — each KNN query filters \
             agent_id = ?3, so cross-agent isolation holds even for episodic rows"
        );
    }
}
