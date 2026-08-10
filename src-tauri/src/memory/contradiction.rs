//! Contradiction detection for the RAG sycophancy guard (Phase 6 — REQ-01, REQ-02, REQ-03).
//!
//! Detects existing memory rows that are semantically close (L2 distance < threshold)
//! to a candidate embedding. Called in `worker.rs` between embed and insert so that
//! disputed rows are flagged atomically inside the INSERT transaction.

use crate::memory::search::{search_all_memories, SearchScope};
use crate::memory::record::TableKind;
use crate::memory::MemoryStore;

#[cfg(test)]
use crate::memory::vector::to_unit;

/// L2 distance below which two embeddings are considered contradictory.
///
/// LOCKED decision D1 (design §LOCKED). Value 0.20 is a conservative starting
/// point; **needs empirical tuning post-model-load** since the FakeEmbedder used
/// in tests is non-semantic (SipHash+LCG — NL pairs carry no meaning).
pub(crate) const CONTRADICTION_THRESHOLD: f32 = 0.20;

/// Return the IDs of existing memory rows whose embedding is within
/// `CONTRADICTION_THRESHOLD` L2 distance of `embedding`, scoped to `agent_id`.
///
/// Memory-only scope (LOCKED D4 — episodic and procedural excluded).
/// Errors are swallowed as `log::warn!` — a detection failure must never abort ingest.
/// Self-match is impossible: the new row is not yet in the DB at hook time.
pub(crate) async fn detect_contradiction(
    store: &MemoryStore,
    embedding: &[f32],
    agent_id: &str,
) -> Vec<i64> {
    // Memory-only scope per LOCKED D4; k=5 candidates from vec0.
    let scope = SearchScope {
        memory: Some(5),
        episodic: None,
        procedural: None,
    };
    match search_all_memories(store, embedding, agent_id, scope, 5).await {
        Ok(hits) => hits
            .into_iter()
            .filter(|h| {
                matches!(h.kind, TableKind::Memory)
                    && h.distance < CONTRADICTION_THRESHOLD as f64
            })
            .map(|h| h.id)
            .collect(),
        Err(e) => {
            log::warn!("contradiction detect failed (agent={agent_id}): {e}");
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::MemoryStore;

    // ── shared test helpers ──────────────────────────────────────────────────

    /// Serialize a float slice to little-endian bytes for vec0 storage.
    fn emb_bytes(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    /// Insert one memory row with a known embedding. Returns the new row id.
    ///
    /// Normalizes before writing: this bypasses `insert_memory_with_vector`
    /// (raw SQL, no `to_unit`), but `detect_contradiction`'s queries go through
    /// `search_all_memories`, which normalizes the query side. Storing the raw
    /// fixture here would compare an unnormalized ~11-unit stored vector against
    /// a unit query and never match — every distance blows out to 10+ regardless
    /// of how close the two fixtures are in direction.
    async fn insert_row(store: &MemoryStore, agent_id: &str, embedding: &[f32]) -> i64 {
        let unit = to_unit(embedding).expect("test fixtures are never zero-magnitude");
        let bytes = emb_bytes(&unit);
        let agent = agent_id.to_string();
        store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance, tier, status) \
                     VALUES ('agent', ?1, 'test content', 'chat_message', 'chat', \
                      0, 'user', ?1, 0, 0, 5.0, 3, 'active')",
                    rusqlite::params![agent],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                    rusqlite::params![id, bytes],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    // ── T08 [TEST] REQ-01: no-contradiction → empty Vec ─────────────────────

    /// Far vectors (large L2) must not trigger contradiction detection.
    #[tokio::test]
    async fn detect_contradiction_far_vectors_returns_empty() {
        let store = MemoryStore::new_for_test().await.unwrap();

        // stored = base vector (values ≈ 0.5 in each dim)
        let base = base_vec(1);
        // query = far vector (shifts each dim by +1.0 → L2 ≈ sqrt(384) ≈ 19.6)
        let query = far(&base);

        // Verify far distance is well above threshold (defensive check)
        let dist: f32 = base
            .iter()
            .zip(query.iter())
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!(dist > 0.50, "far() must produce distance > 0.50; got {dist}");

        insert_row(&store, "agent-far", &base).await;

        let ids = detect_contradiction(&store, &query, "agent-far").await;
        assert!(ids.is_empty(), "far vectors must not be flagged; got {ids:?}");
    }

    // ── T09 [TEST] REQ-01,02: near vector → returns stored id ───────────────

    /// Near vectors (L2 < CONTRADICTION_THRESHOLD) must be detected.
    #[tokio::test]
    async fn detect_contradiction_near_vector_returns_stored_id() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let base = base_vec(2);
        // near(base, 0.10) → L2 = 0.10 < 0.20 = CONTRADICTION_THRESHOLD
        let query = near(&base, 0.10);

        // Defensive: verify near distance
        let dist: f32 = base
            .iter()
            .zip(query.iter())
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!(
            dist < CONTRADICTION_THRESHOLD,
            "near() must produce distance < {CONTRADICTION_THRESHOLD}; got {dist}"
        );

        let stored_id = insert_row(&store, "agent-near", &base).await;

        let ids = detect_contradiction(&store, &query, "agent-near").await;
        assert!(
            ids.contains(&stored_id),
            "stored_id={stored_id} must be in contradiction ids={ids:?}"
        );
    }

    #[tokio::test]
    async fn detect_contradiction_empty_db_returns_empty() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let base = base_vec(3);
        let ids = detect_contradiction(&store, &base, "agent-empty").await;
        assert!(ids.is_empty(), "empty DB must yield empty contradiction ids");
    }

    #[tokio::test]
    async fn detect_contradiction_different_agent_id_not_returned() {
        let store = MemoryStore::new_for_test().await.unwrap();
        // Insert row for "agent-a"
        let base = base_vec(4);
        insert_row(&store, "agent-a", &base).await;
        // Query with "agent-b" → different scope → empty
        let query = near(&base, 0.05);
        let ids = detect_contradiction(&store, &query, "agent-b").await;
        assert!(
            ids.is_empty(),
            "row from agent-a must not appear for agent-b; got {ids:?}"
        );
    }

    // ── T11 [TEST] REQ-03: eval harness — recall ≥ 0.85 ────────────────────

    /// Manufactures ≥20 labeled (contradicting, stored, query) pairs.
    /// Contradicting pairs: stored=base_i, query=near(base_i, 0.10) → L2=0.10 < 0.20 ✓
    /// Non-contradicting:   stored=far(base_i), query=base_i → L2≈19.6 > 0.50 ✓
    ///
    /// Each pair uses an isolated agent_id so fixtures don't interfere.
    #[tokio::test]
    async fn eval_harness_contradiction_recall_above_0_85() {
        let store = MemoryStore::new_for_test().await.unwrap();

        const N_PAIRS: usize = 12; // 12 contradicting + 12 non-contradicting = 24 ≥ 20

        // Build labeled fixtures: (contradicting, stored_vec, query_vec)
        let mut fixtures: Vec<(bool, Vec<f32>, Vec<f32>)> = Vec::with_capacity(N_PAIRS * 2);

        for i in 0..N_PAIRS {
            let base = base_vec(i as u64 + 100); // seed offset avoids seed=0 edge case
            let q = near(&base, 0.10);
            fixtures.push((true, base, q));
        }
        for i in 0..N_PAIRS {
            let base = base_vec(i as u64 + 200);
            let stored = far(&base);
            fixtures.push((false, stored, base));
        }

        // Insert each stored vector under its own agent_id
        let mut stored_ids: Vec<Option<i64>> = Vec::with_capacity(fixtures.len());
        for (idx, (_, stored, _)) in fixtures.iter().enumerate() {
            let agent_id = format!("eval-agent-{idx}");
            let id = insert_row(&store, &agent_id, stored).await;
            stored_ids.push(Some(id));
        }

        // Evaluate: for each fixture query with the query vec and check result
        let mut tp = 0usize; // true positive (contradicting correctly detected)
        let mut fp = 0usize; // false positive (non-contradicting incorrectly detected)

        for (idx, (contradicting, _, query)) in fixtures.iter().enumerate() {
            let agent_id = format!("eval-agent-{idx}");
            let ids = detect_contradiction(&store, query, &agent_id).await;
            let expected_id = stored_ids[idx].unwrap();
            let flagged = ids.contains(&expected_id);

            if *contradicting {
                if flagged { tp += 1; }
            } else {
                if flagged { fp += 1; }
            }
        }

        let recall = tp as f32 / N_PAIRS as f32;
        assert!(
            recall >= 0.85,
            "contradiction recall must be ≥0.85; got {recall:.2} (tp={tp}/{N_PAIRS})"
        );
        assert_eq!(
            fp, 0,
            "no non-contradicting fixture should be flagged; got fp={fp}"
        );
    }

    // ── Vector manufacturing helpers (deterministic, metric-agnostic) ────────

    /// Deterministic 384-dim base vector; values in [0, 1].
    fn base_vec(seed: u64) -> Vec<f32> {
        let mut s = seed
            .wrapping_add(1) // avoid all-zero seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (0..384)
            .map(|_| {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                // map to [0, 1]
                (s >> 33) as f32 / u32::MAX as f32
            })
            .collect()
    }

    /// Near vector: v + delta per dimension, ‖delta‖ = eps, BEFORE either side
    /// is normalized. `detect_contradiction` compares these post-normalization
    /// (via `insert_row` and `search_all_memories`), where the threshold check
    /// actually happens — for `base_vec`'s ‖v‖ ≈ 11.3, eps=0.10 becomes an
    /// angular offset of ≈ eps/‖v‖ ≈ 0.009 rad, i.e. unit-space L2 ≈ 0.009,
    /// still comfortably under CONTRADICTION_THRESHOLD (0.20).
    fn near(v: &[f32], eps: f32) -> Vec<f32> {
        // Uniform delta per dim: ‖delta‖ = sqrt(384 * d^2) = sqrt(384)*d = eps → d = eps/sqrt(384)
        let delta = eps / (384.0_f32).sqrt();
        v.iter().map(|x| x + delta).collect()
    }

    /// Far vector: shifts each dim by +1.0 → raw L2 = sqrt(384) ≈ 19.6, BEFORE
    /// normalization. Post-normalization (what `detect_contradiction` actually
    /// compares) the two directions differ enough that cos ≈ 0.947, i.e.
    /// unit-space L2 ≈ 0.33 — still safely over CONTRADICTION_THRESHOLD (0.20).
    fn far(v: &[f32]) -> Vec<f32> {
        v.iter().map(|x| x + 1.0).collect()
    }
}
