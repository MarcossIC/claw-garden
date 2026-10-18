//! Contradiction detection for the RAG sycophancy guard.
//!
//! Finds existing memory rows that are semantically close (L2 distance under
//! the threshold) to a new row's embedding. Closeness is not contradiction: a
//! rewording, a complement or an update sits in the same band as a negation.
//! So every row found is a candidate, recorded as a `memory_conflict` pair,
//! and only the pairs [`needs_user`] picks are escalated to the user. Called
//! in `worker.rs` between embed and insert so the pairs are written inside the
//! INSERT transaction.

use std::collections::HashMap;

use crate::memory::search::{identifier_tokens, search_all_memories, SearchScope};
use crate::memory::record::TableKind;
use crate::memory::store_insert::MemoryType;
use crate::memory::MemoryStore;

#[cfg(test)]
use crate::memory::vector::to_unit;

/// L2 distance below which two embeddings are close enough to be a candidate
/// contradiction.
///
/// Ten contradicting and ten compatible pairs of project facts
/// (`El proyecto usa PostgreSQL` / `… SQLite`, against `… SQLite` /
/// `Las migraciones se aplican con refinery`) separate at (0.350, 0.469]
/// under mean pooling. 0.42 keeps 0.07 above the last contradiction and 0.05
/// below the first compatible pair. On the golden set with mean vectors no
/// compatible pair falls under 0.50, and 0.42 sits between recall 0.8 at 0.40
/// and 0.9 at 0.45; going higher would trade the margin the hand-written
/// compatible pair at 0.469 asks for.
///
/// The labelled pairs leave out identical texts on purpose: the write-border
/// dedup gate intercepts an identical resend by content hash before it ever
/// reaches this detector, so a pair the detector never sees must not move the
/// threshold that governs the pairs it does see.
pub(crate) const CONTRADICTION_THRESHOLD: f32 = 0.42;

/// Below this L2 distance the two texts are the same claim, not two claims
/// that disagree — a near-duplicate the content hash could not catch because
/// the difference is casing, interior whitespace, or a trivial edit rather
/// than the surrounding whitespace and NFC form the hash normalizes away.
/// Twice its CLS-pooled 0.05: the pair that sat at 0.047 under CLS (a
/// sentence and itself with a doubled space) sits at 0.097 under mean.
pub(crate) const NEAR_DUPLICATE_FLOOR: f32 = 0.10;

// If a future threshold sweep ever lands a value below the floor, the
// contradiction band [floor, threshold) becomes empty or inverted — fail the
// build instead of the detector silently returning nothing at runtime.
const _: () = assert!(NEAR_DUPLICATE_FLOOR < CONTRADICTION_THRESHOLD);

// The calibration above re-embeds each labelled pair's raw text and measures
// L2 directly on the two vectors; it never calls `detect_contradiction`, so it
// says nothing about the type gate, the floor or the dedup path. Read it as a
// calibration of the distance alone; `conflict-eval` measures the detector.

/// Candidates the detector weighs per probe, before room for siblings.
const CANDIDATE_POOL: usize = 5;

/// Below this L2 distance a candidate pair of two preferences or decisions is
/// put to the user. It sits between the last measured contradiction (0.350)
/// and the first compatible pair (0.469) of the calibration above.
pub(crate) const ESCALATE_BELOW: f64 = 0.40;

/// Below this L2 distance a pair of plain facts that share an identifier is
/// put to the user. Facts of one project name the same identifiers all the
/// time: in conflict-eval's flow of this repository's requirements, such
/// pairs sat between 0.34 and 0.40, and escalating them at [`ESCALATE_BELOW`]
/// put one save in four to the user.
pub(crate) const IDENTIFIER_ESCALATE_BELOW: f64 = 0.30;

/// How many characters of a candidate's first line a save answer shows.
pub(crate) const EXCERPT_CHARS: usize = 80;

/// One existing row the detector found close to a new one.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Candidate {
    pub id: i64,
    pub distance: f64,
    /// Whether the pair is put to the user ([`needs_user`]).
    pub escalated: bool,
    /// The first line of the candidate, cut to [`EXCERPT_CHARS`].
    pub excerpt: String,
}

/// The save a candidate belongs to: the `memory_piece` save of a piece, the
/// row itself otherwise. A new save pairs once with each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SaveKey {
    Save(i64),
    Row(i64),
}

/// Whether a candidate pair goes to the user rather than only to the agent.
/// A distance nobody recorded (a pair from before the pair table) always
/// does; otherwise only a close pair of two preferences or decisions, or a
/// closer one whose two texts name the same identifier.
///
/// A fact close to a decision stays with the agent: in conflict-eval's replay
/// of a real store, most of those were notes of what happened saved as
/// decisions. The agent still cannot supersede the decision on its own word
/// (`dispute::judge_pair`), so it asks the user through `conflicts`.
pub(crate) fn needs_user(new: MemoryType, old: MemoryType, distance: Option<f64>, shares_identifier: bool) -> bool {
    let protected = |t: MemoryType| matches!(t, MemoryType::UserPreference | MemoryType::ArchitecturalDecision);
    match distance {
        None => true,
        Some(d) => {
            (d < ESCALATE_BELOW && protected(new) && protected(old))
                || (d < IDENTIFIER_ESCALATE_BELOW && shares_identifier)
        }
    }
}

/// Whether `a` and `b` hold a common identifier (`identifier_tokens`).
pub(crate) fn shares_identifier(a: &str, b: &str) -> bool {
    let theirs: Vec<String> = identifier_tokens(b).iter().map(|t| t.to_lowercase()).collect();
    identifier_tokens(a).iter().any(|t| theirs.contains(&t.to_lowercase()))
}

/// The first non-blank line of `content`, cut to [`EXCERPT_CHARS`] characters.
pub(crate) fn excerpt(content: &str) -> String {
    let line = content.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if line.chars().count() <= EXCERPT_CHARS {
        return line.to_string();
    }
    let mut cut: String = line.chars().take(EXCERPT_CHARS).collect();
    cut.push('…');
    cut
}

/// Return the existing memory rows whose embedding is within the dispute
/// band `[NEAR_DUPLICATE_FLOOR, CONTRADICTION_THRESHOLD)` of `embedding`,
/// scoped to `agent_id` and, when given, `project_id`, leaving out
/// `siblings`: the rows the same event already inserted. Each comes with the
/// save it belongs to; the search already keeps the closest piece of each.
///
/// Deliberately searches `memory` only. An episodic trajectory records what
/// happened and a procedural skill records how something is done — neither
/// asserts a claim, so neither can contradict one; widening the scope would
/// only add near-neighbours that can never legitimately be disputed.
/// Errors are swallowed as `log::warn!` — a detection failure must never abort ingest.
/// Self-match is impossible: the new row is not yet in the DB at hook time.
///
/// `candidate_type` gates BEFORE the KNN runs: an ineligible candidate
/// returns `[]` without building a `SearchScope` at all, saving a search for
/// every ingest that cannot possibly assert a claim (the overwhelming
/// majority — every chat role maps to `ChatMessage`).
// Only the memory pool is requested; `pool` is a candidate budget, not a result
// count — the threshold in `detect_contradiction` decides what actually
// disputes.
//
// `lexical: None`, deliberately: this detector's distance band is what
// decides whether a candidate is a near-duplicate, and a lexically promoted
// candidate can share vocabulary with the query while sitting nowhere near
// it in embedding space — exactly the pair this detector must not treat as
// a knowledge contradiction. Widening this pool with a channel that finds
// text matches, not distance matches, would feed the threshold below rows
// it was never calibrated against.
//
// Extracted from `detect_contradiction` so `lexical: None` is a directly
// assertable value rather than a field inside a function-local literal:
// `query_text: None` at this function's only call site already blocks the
// lexical arm regardless of this field (`search_all_memories_inner` fires
// lexical only when BOTH `scope.lexical` and `query_text` are `Some`), so a
// black-box test of `detect_contradiction`'s output cannot distinguish this
// field's value from its opposite. A direct assertion on the returned
// struct is the only way to pin it.
fn contradiction_search_scope(project_id: Option<&str>, pool: usize) -> SearchScope {
    SearchScope {
        memory: Some(pool),
        episodic: None,
        procedural: None,
        project_id: project_id.map(str::to_owned),
        lexical: None,
        // A save disputes only the saver's own rows: sharing lets an agent
        // read another's memories, never mark them disputed.
        reach: crate::memory::sharing::Reach::Own,
    }
}

pub(crate) async fn detect_contradiction(
    store: &MemoryStore,
    embedding: &[f32],
    text: &str,
    agent_id: &str,
    project_id: Option<&str>,
    candidate_type: MemoryType,
    siblings: &[i64],
) -> Vec<(SaveKey, Candidate)> {
    if !candidate_type.is_claim_bearing() {
        return Vec::new();
    }

    // Siblings sit closest of all to the chunk being probed, so the pool
    // grows by their count: otherwise a long save fills all five slots with
    // its own chunks and a real dispute with an older row is never seen.
    let pool = CANDIDATE_POOL + siblings.len();
    let scope = contradiction_search_scope(project_id, pool);
    let hits = match search_all_memories(store, embedding, None, agent_id, scope, pool).await {
        Ok(hits) => hits,
        Err(e) => {
            log::warn!("contradiction detect failed (agent={agent_id}): {e}");
            return Vec::new();
        }
    };
    // Two similar chat messages are not a knowledge contradiction. Only
    // types that assert something can contradict something, so the
    // detector is gated to those and skips the KNN entirely for the rest.
    let found: Vec<Candidate> = hits
        .into_iter()
        .filter(|h| {
            matches!(h.kind, TableKind::Memory)
                && !siblings.contains(&h.id)
                && h.distance >= NEAR_DUPLICATE_FLOOR as f64
                && h.distance < CONTRADICTION_THRESHOLD as f64
        })
        .filter_map(|h| {
            let old_type = MemoryType::from_db_str(&h.memory_type).filter(|t| t.is_claim_bearing())?;
            let escalated = needs_user(candidate_type, old_type, Some(h.distance), shares_identifier(text, &h.content));
            Some(Candidate { id: h.id, distance: h.distance, escalated, excerpt: excerpt(&h.content) })
        })
        .collect();
    if found.is_empty() {
        return Vec::new();
    }
    let ids: Vec<i64> = found.iter().map(|c| c.id).collect();
    let saves = match store.save_links(ids).await {
        Ok(saves) => saves,
        Err(e) => {
            log::warn!("contradiction detect could not read the saves of its candidates (agent={agent_id}): {e}");
            HashMap::new()
        }
    };
    found
        .into_iter()
        .map(|c| {
            let key = saves.get(&c.id).map_or(SaveKey::Row(c.id), |link| SaveKey::Save(link.save_id));
            (key, c)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::MemoryStore;

    // ── shared test helpers ──────────────────────────────────────────────────

    /// The ids `detect_contradiction` finds for a probe with no text.
    async fn detected(
        store: &MemoryStore,
        embedding: &[f32],
        agent_id: &str,
        project_id: Option<&str>,
        candidate_type: MemoryType,
        siblings: &[i64],
    ) -> Vec<i64> {
        detect_contradiction(store, embedding, "", agent_id, project_id, candidate_type, siblings)
            .await
            .into_iter()
            .map(|(_, c)| c.id)
            .collect()
    }

    /// Serialize a float slice to little-endian bytes for vec0 storage.
    fn emb_bytes(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    /// Insert one memory row with a known embedding and `memory_type`. Returns
    /// the new row id.
    ///
    /// Normalizes before writing: this bypasses `insert_memory_with_vector`
    /// (raw SQL, no `to_unit`), but `detect_contradiction`'s queries go through
    /// `search_all_memories`, which normalizes the query side. Storing the raw
    /// fixture here would compare an unnormalized ~11-unit stored vector against
    /// a unit query and never match — every distance blows out to 10+ regardless
    /// of how close the two fixtures are in direction.
    async fn insert_row_typed(store: &MemoryStore, agent_id: &str, memory_type: &str, embedding: &[f32]) -> i64 {
        let unit = to_unit(embedding).expect("test fixtures are never zero-magnitude");
        let bytes = emb_bytes(&unit);
        let agent = agent_id.to_string();
        let mt = memory_type.to_string();
        store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance, tier, status) \
                     VALUES ('agent', ?1, 'test content', ?2, 'chat', \
                      0, 'user', ?1, 0, 0, 5.0, 3, 'active')",
                    rusqlite::params![agent, mt],
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

    /// Distance-only fixtures use `user_preference` — a claim-bearing type —
    /// so the hit-side type gate never interferes with what these tests
    /// actually measure (distance banding, not eligibility).
    async fn insert_row(store: &MemoryStore, agent_id: &str, embedding: &[f32]) -> i64 {
        insert_row_typed(store, agent_id, "user_preference", embedding).await
    }

    // ── Pinned constant: CONTRADICTION_THRESHOLD ────────────────────────────

    /// Pins `CONTRADICTION_THRESHOLD` as a bare literal. Every band test in
    /// this module (`near`/`far`-based fixtures, the near-duplicate-floor
    /// test, the eval-harness recall test) measures its own fixture's
    /// distance against the constant itself, so all of them pass for ANY
    /// value the constant holds — none of them can catch an edit to it. This
    /// is the one test that dies if the shipped value changes.
    #[test]
    fn contradiction_threshold_pins_the_shipped_value() {
        assert_eq!(
            CONTRADICTION_THRESHOLD, 0.42,
            "CONTRADICTION_THRESHOLD changed from its shipped value. Under mean pooling the \
             measured inter-class gap is (0.350, 0.469]. If this changed on purpose, re-derive \
             it from a fresh sweep before re-pinning here — do not just update this literal."
        );
    }

    /// Pins `NEAR_DUPLICATE_FLOOR` as a bare literal, for the same reason the
    /// test above pins the threshold: every other test that names this
    /// constant measures its own fixture against the constant itself. The
    /// floor test's fixture sits at unit-space L2 ≈ 0.009 and the near-band
    /// test's at ≈ 0.26, so the pair of them only bounds the floor to the open
    /// range (0.009, 0.26] — mutating the floor to 0.01 left every test green.
    /// A 10x window is not a pin.
    #[test]
    fn near_duplicate_floor_pins_the_shipped_value() {
        assert_eq!(
            NEAR_DUPLICATE_FLOOR, 0.10,
            "NEAR_DUPLICATE_FLOOR changed from its shipped value. This floor is what keeps a \
             near-exact duplicate the content hash could not catch (casing, interior whitespace, \
             a trivial edit) from being filed as a contradiction and burning an LLM adjudication \
             — lowering it reopens exactly the false-positive path the write-border dedup work \
             exists to close. If this changed on purpose, justify the new value against the \
             near-duplicate distance distribution before re-pinning here."
        );
    }

    /// Far vectors (large L2) must not trigger contradiction detection.
    #[tokio::test]
    async fn detect_contradiction_far_vectors_returns_empty() {
        let store = MemoryStore::new_for_test().await.unwrap();

        // stored = base vector (values ≈ 0.5 in each dim)
        let base = base_vec(1);
        // query = far vector (antipodal after normalization, see far()'s doc)
        let query = far(&base);

        // Defensive: verify the distance `detect_contradiction` actually
        // compares — post-normalization, unit space — not a raw pre-norm
        // distance, which lives in a different space than the threshold
        // comparison and can clear or miss it independently of the real one.
        let ub = to_unit(&base).unwrap();
        let uq = to_unit(&query).unwrap();
        let dist: f32 = ub.iter().zip(uq.iter()).map(|(a, b)| (a - b).powi(2)).sum::<f32>().sqrt();
        assert!(
            dist > CONTRADICTION_THRESHOLD,
            "far() must produce a unit-space distance above CONTRADICTION_THRESHOLD; got {dist}"
        );

        insert_row(&store, "agent-far", &base).await;

        let ids = detected(&store, &query, "agent-far", None, MemoryType::UserPreference, &[]).await;
        assert!(ids.is_empty(), "far vectors must not be flagged; got {ids:?}");
    }

    /// Near vectors, in the dispute band `[NEAR_DUPLICATE_FLOOR,
    /// CONTRADICTION_THRESHOLD)`, must be detected.
    #[tokio::test]
    async fn detect_contradiction_near_vector_returns_stored_id() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let base = base_vec(2);
        // eps=1.5 measured (not assumed) to land unit-space L2 inside the
        // dispute band — see the eps→unit-space-distance table in this
        // module's doc comment on `near()`.
        let query = near(&base, 3.0);

        // Defensive: verify the distance `detect_contradiction` actually
        // compares (post-normalization, unit space), not the raw pre-norm one.
        let ub = to_unit(&base).unwrap();
        let uq = to_unit(&query).unwrap();
        let dist: f32 = ub.iter().zip(uq.iter()).map(|(a, b)| (a - b).powi(2)).sum::<f32>().sqrt();
        assert!(
            dist >= NEAR_DUPLICATE_FLOOR && dist < CONTRADICTION_THRESHOLD,
            "near() must produce unit-space distance in [{NEAR_DUPLICATE_FLOOR}, {CONTRADICTION_THRESHOLD}); got {dist}"
        );

        let stored_id = insert_row(&store, "agent-near", &base).await;

        let ids = detected(&store, &query, "agent-near", None, MemoryType::UserPreference, &[]).await;
        assert!(
            ids.contains(&stored_id),
            "stored_id={stored_id} must be in contradiction ids={ids:?}"
        );
    }

    #[tokio::test]
    async fn detect_contradiction_empty_db_returns_empty() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let base = base_vec(3);
        let ids = detected(&store, &base, "agent-empty", None, MemoryType::UserPreference, &[]).await;
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
        let ids = detected(&store, &query, "agent-b", None, MemoryType::UserPreference, &[]).await;
        assert!(
            ids.is_empty(),
            "row from agent-a must not appear for agent-b; got {ids:?}"
        );
    }

    /// Manufactures ≥20 labeled (contradicting, stored, query) pairs.
    /// Contradicting pairs: stored=base_i, query=near(base_i, 3.0) → unit-space
    /// L2 ≈ 0.26, inside `[NEAR_DUPLICATE_FLOOR, CONTRADICTION_THRESHOLD)`.
    /// Non-contradicting:   stored=far(base_i), query=base_i → antipodal after
    /// normalization, unit-space L2 = 2.0, far outside the dispute band.
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
            let q = near(&base, 3.0);
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
            let ids = detected(&store, query, &agent_id, None, MemoryType::UserPreference, &[]).await;
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

    // ── Escalation ──────────────────────────────────────────────────────────

    #[test]
    fn needs_user_cases() {
        use MemoryType::{ArchitecturalDecision as Decision, SemanticFact as Fact, UserPreference as Pref};
        let close = Some(0.30);
        let edge = Some(ESCALATE_BELOW);
        let nearer = Some(0.25);
        let between = Some(0.35);
        let cases: &[(MemoryType, MemoryType, Option<f64>, bool, bool)] = &[
            (Fact, Fact, nearer, false, false),
            (Fact, Fact, nearer, true, true),
            (Fact, Fact, Some(IDENTIFIER_ESCALATE_BELOW), true, false),
            (Fact, Fact, between, true, false),
            (Fact, Fact, edge, true, false),
            (Pref, Fact, between, false, false),
            (Pref, Fact, close, false, false),
            (Fact, Decision, close, false, false),
            (Fact, Pref, nearer, true, true),
            (Pref, Decision, between, false, true),
            (Decision, Decision, close, false, true),
            (Pref, Pref, edge, false, false),
            (Pref, Pref, Some(0.41), true, false),
            (Fact, Fact, None, false, true),
        ];
        for &(new, old, distance, shares, expected) in cases {
            assert_eq!(
                needs_user(new, old, distance, shares),
                expected,
                "needs_user({new:?}, {old:?}, {distance:?}, {shares})"
            );
        }
    }

    #[test]
    fn escalate_below_pins_the_shipped_value() {
        assert_eq!(
            ESCALATE_BELOW, 0.40,
            "ESCALATE_BELOW sits between the last measured contradiction (0.350) and the first \
             compatible pair (0.469); move it only from a conflict-eval run"
        );
    }

    #[test]
    fn identifier_escalate_below_pins_the_shipped_value() {
        assert_eq!(
            IDENTIFIER_ESCALATE_BELOW, 0.30,
            "facts of one project that share an identifier sit between 0.34 and 0.40 in \
             conflict-eval's flow; move it only from a conflict-eval run"
        );
    }

    #[test]
    fn identifiers_are_shared_only_when_both_texts_hold_one() {
        assert!(shares_identifier("the cache lives in load_index", "load_index reads the WAL"));
        assert!(!shares_identifier("the project uses PostgreSQL", "the project uses SQLite"));
        assert!(!shares_identifier("parseLedger7731 is slow", "parseLedger is slow"));
    }

    #[test]
    fn an_excerpt_is_the_first_line_cut_to_its_limit() {
        assert_eq!(excerpt("\n  Ports\nthe api listens on 8080"), "Ports");
        let long = "é".repeat(EXCERPT_CHARS + 5);
        let cut = excerpt(&long);
        assert_eq!(cut.chars().count(), EXCERPT_CHARS + 1);
        assert!(cut.ends_with('…'));
    }

    // ── Pinned field: contradiction_search_scope.lexical ────────────────────

    /// `contradiction_search_scope`'s `lexical: None` cannot be pinned through
    /// `detect_contradiction`'s output: its call site also passes
    /// `query_text: None`, and `search_all_memories_inner` only spends the
    /// lexical channel when BOTH are `Some` — so flipping this field alone,
    /// with `query_text` left as `None`, changes no observable behavior at
    /// all. This test asserts the constructed value directly, the only way
    /// to catch that mutation.
    #[test]
    fn contradiction_search_scope_disables_the_lexical_channel() {
        let scope = contradiction_search_scope(None, CANDIDATE_POOL);
        assert_eq!(
            scope.lexical, None,
            "the near-duplicate detector's distance band must never be fed lexically \
             promoted candidates — see the doc comment on contradiction_search_scope"
        );
    }

    /// The detector reaches the saver's own rows whatever the sharing policy
    /// says: a save may dispute those, never rows it is only allowed to read.
    #[test]
    fn contradiction_probe_ignores_shared_authors() {
        let scope = contradiction_search_scope(Some("proj"), CANDIDATE_POOL);
        assert_eq!(scope.reach, crate::memory::sharing::Reach::Own);
    }

    // ── Type gate: candidate side ───────────────────────────────────────────

    /// An ineligible candidate type must return `[]` without ever building a
    /// `SearchScope` or hitting the KNN.
    ///
    /// The stored row sits at a distance measured to land INSIDE the dispute
    /// band, and its type is claim-bearing, so the hit-side filter cannot
    /// explain an empty result — only the candidate-side gate can. The
    /// original fixture probed at distance 0 and claimed that "would certainly
    /// flag if the KNN ran at all"; `NEAR_DUPLICATE_FLOOR` made that false —
    /// distance 0 is precisely what the floor drops. With that fixture,
    /// deleting the candidate-side gate outright left all 801 tests green
    /// (mutation-verified). An in-band distance restores the test's teeth.
    #[tokio::test]
    async fn ineligible_candidate_type_returns_empty_without_knn() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let base = base_vec(10);
        let query = near(&base, 3.0);

        // Defensive: without this, a fixture drifting out of the band would
        // turn this back into the vacuous test it just stopped being.
        let ub = to_unit(&base).unwrap();
        let uq = to_unit(&query).unwrap();
        let dist: f32 = ub.iter().zip(uq.iter()).map(|(a, b)| (a - b).powi(2)).sum::<f32>().sqrt();
        assert!(
            dist >= NEAR_DUPLICATE_FLOOR && dist < CONTRADICTION_THRESHOLD,
            "fixture must land inside [{NEAR_DUPLICATE_FLOOR}, {CONTRADICTION_THRESHOLD}) so only \
             the candidate gate can produce an empty result; got {dist}"
        );

        insert_row_typed(&store, "agent-gate", "user_preference", &base).await;

        let ids = detected(&store, &query, "agent-gate", None, MemoryType::ChatMessage, &[]).await;
        assert!(
            ids.is_empty(),
            "an ineligible candidate type must short-circuit to [], got {ids:?}"
        );
    }

    // ── Type gate: hit side ──────────────────────────────────────────────────

    /// An eligible candidate against an ineligible hit type must drop that
    /// hit even though it is well inside the dispute band.
    #[tokio::test]
    async fn eligible_candidate_ineligible_hit_is_dropped() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let base = base_vec(11);
        let query = near(&base, 3.0); // unit-space L2 ≈ 0.26, inside the band
        insert_row_typed(&store, "agent-hitgate", "chat_message", &base).await;

        let ids = detected(&store, &query, "agent-hitgate", None, MemoryType::UserPreference, &[]).await;
        assert!(
            ids.is_empty(),
            "a chat_message hit must be dropped even for an eligible candidate, got {ids:?}"
        );
    }

    // ── Similarity band: near-duplicate floor ───────────────────────────────

    /// A hit inside `NEAR_DUPLICATE_FLOOR` (a near-exact duplicate the hash
    /// gate didn't catch — e.g. a whitespace/casing variant) must be skipped,
    /// not disputed.
    #[tokio::test]
    async fn hit_below_near_duplicate_floor_is_excluded() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let base = base_vec(12);
        // eps=0.10 measured to land unit-space L2 well under NEAR_DUPLICATE_FLOOR.
        let query = near(&base, 0.10);
        let ub = to_unit(&base).unwrap();
        let uq = to_unit(&query).unwrap();
        let dist: f32 = ub.iter().zip(uq.iter()).map(|(a, b)| (a - b).powi(2)).sum::<f32>().sqrt();
        assert!(
            dist < NEAR_DUPLICATE_FLOOR,
            "fixture must land below the floor to exercise it; got {dist}"
        );

        insert_row_typed(&store, "agent-floor", "user_preference", &base).await;

        let ids = detected(&store, &query, "agent-floor", None, MemoryType::UserPreference, &[]).await;
        assert!(
            ids.is_empty(),
            "a hit below NEAR_DUPLICATE_FLOOR must be skipped, not disputed; got {ids:?}"
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
    /// far below either `NEAR_DUPLICATE_FLOOR` or `CONTRADICTION_THRESHOLD`
    /// regardless of which specific value either constant currently holds.
    /// Callers that need a fixture landing INSIDE the dispute band use a
    /// larger `eps` and measure the resulting unit-space distance directly
    /// rather than relying on a number in this comment.
    fn near(v: &[f32], eps: f32) -> Vec<f32> {
        // Uniform delta per dim: ‖delta‖ = sqrt(384 * d^2) = sqrt(384)*d = eps → d = eps/sqrt(384)
        let delta = eps / (384.0_f32).sqrt();
        v.iter().map(|x| x + delta).collect()
    }

    /// Far vector: the negation of `v`. After normalization the two
    /// directions are exactly antipodal (cos = -1), giving a unit-space L2
    /// distance of exactly 2.0 — deterministic, and independent of `v`'s
    /// magnitude, mean, or dimensionality.
    ///
    /// An earlier version shifted every dimension by a fixed +1.0 instead.
    /// For this module's `base_vec` (values in [0, 1], mean ≈ 0.5) that
    /// produced cos ≈ 0.947, i.e. unit-space L2 ≈ 0.33 — a margin over
    /// `CONTRADICTION_THRESHOLD` that shrank every time the threshold moved
    /// up (0.13 of margin at 0.20, 0.03 at 0.30) and would have gone
    /// negative at 0.35, putting the "far" fixture back inside the dispute
    /// band it exists to sit outside of. Worse, an additive shift's
    /// post-normalization distance approaches a fixed ceiling as the shift
    /// grows (≈0.52 for this `base_vec`, the angle between `v` and the
    /// all-ones direction) — no larger additive constant would have fixed
    /// this for good. Negation has no such ceiling.
    fn far(v: &[f32]) -> Vec<f32> {
        v.iter().map(|x| -x).collect()
    }

    // ── user_preference regression ──────────────────────────────────────────
    //
    // Probing with an identical embedding (distance 0) would isolate "can the
    // pool see it" from "is the threshold calibrated", but
    // NEAR_DUPLICATE_FLOOR makes distance 0 a skip, not a dispute. So this
    // uses a distance measured (not assumed) to land inside the dispute band,
    // through the real `insert_memory_with_vector` write path.

    /// A contradictory `user_preference` — the anti-sycophancy guard's
    /// reason to exist — must be detected once its embedding lands inside
    /// `[NEAR_DUPLICATE_FLOOR, CONTRADICTION_THRESHOLD)`.
    #[tokio::test]
    async fn user_preference_contradiction_detected_at_in_band_distance() {
        use crate::memory::embedder::{EmbedRole, Embedder, FakeEmbedder};
        use crate::memory::events::MemorySource;
        use crate::memory::store_insert::{CreatedByType, NewMemoryRecord};

        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let text = "Siempre usá voseo en las respuestas";
        let emb = embedder.embed_with_role(text, EmbedRole::Passage).await.unwrap();
        let record = NewMemoryRecord {
            scope: "agent",
            agent_id: Some("agent-contra-pref".to_string()),
            project_id: None,
            content: text.to_string(),
            memory_type: MemoryType::UserPreference,
            source: MemorySource::Chat,
            created_by_type: CreatedByType::System,
            created_by_ref: "agent-contra-pref".to_string(),
            importance: 8.0,
            created_at_ms: 0,
            candidates: Vec::new(),
            derived_from: Vec::new(),
        };
        let stored_id = store.insert_memory_with_vector(record, emb.clone()).await.unwrap();

        // eps=2.0 measured (not assumed) against this exact FakeEmbedder
        // output to land unit-space L2 inside the dispute band — the
        // eps→distance relationship is embedder- and input-specific, so it
        // is re-measured here rather than reused from the synthetic
        // base_vec() fixtures above.
        let query = near(&emb, 2.0);
        let ub = to_unit(&emb).unwrap();
        let uq = to_unit(&query).unwrap();
        let dist: f32 = ub.iter().zip(uq.iter()).map(|(a, b)| (a - b).powi(2)).sum::<f32>().sqrt();
        assert!(
            dist >= NEAR_DUPLICATE_FLOOR && dist < CONTRADICTION_THRESHOLD,
            "fixture must land inside [{NEAR_DUPLICATE_FLOOR}, {CONTRADICTION_THRESHOLD}); got {dist}"
        );

        let candidates = detected(&store, &query, "agent-contra-pref", None, MemoryType::UserPreference, &[]).await;
        assert!(
            candidates.contains(&stored_id),
            "a contradictory user_preference at an in-band distance must be detected"
        );
    }
}
