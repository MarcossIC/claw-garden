//! Seeds a golden set's corpus into a fresh, run-scoped store.
//!
//! Every corpus row is inserted under one shared agent id — mirroring
//! production, where a single agent's memories all live in one pool — and
//! the resulting autoincrement ids are recorded in a [`RefMap`] so every
//! score and error message downstream can stay in the golden set's own
//! `ref` space instead of leaking a bare database id.

use std::path::PathBuf;

use agent_office_lib::eval_api::{EvalEmbedder, EvalError, EvalStore};

use crate::golden::{CorpusRow, RefMap};

/// One shared agent id for the whole corpus. Production scopes memories per
/// agent; a golden-set corpus stands in for one agent's whole pool, so every
/// row seeds under the same id rather than inventing a per-row identity that
/// nothing in the schema asks for.
pub const AGENT_ID: &str = "rag-eval-agent";

/// Everything a completed seeding pass produced: the open store (kept open,
/// not closed, so the caller can search it), the ref↔id map, and the
/// embedding-norm statistics needed for the results JSON's `norm_check`
/// (computed here because this is the only place the raw embeddings exist —
/// by the time a row is inserted, its vector has been moved into the store).
pub struct SeedResult {
    pub store: EvalStore,
    pub refs: RefMap,
    /// Where the temp store was opened. Left on disk deliberately — see
    /// [`unique_store_path`].
    pub store_path: PathBuf,
    pub mean_norm: f64,
    pub max_norm_deviation: f64,
}

/// Opens a fresh temp store and inserts every row of `corpus` into it,
/// embedding each row's content with `embedder`. Fails loud: the first
/// embedding or insert error aborts the whole pass rather than returning a
/// partially-seeded store that would make every case scored against it
/// silently wrong.
pub async fn seed(corpus: &[CorpusRow], embedder: &EvalEmbedder) -> Result<SeedResult, EvalError> {
    let store_path = unique_store_path();
    let store = EvalStore::open(&store_path).await?;
    let mut refs = RefMap::new();
    let mut norms: Vec<f64> = Vec::with_capacity(corpus.len());

    for row in corpus {
        let embedding = embedder.embed(&row.content).await?;
        norms.push(l2_norm(&embedding));
        let id = store
            .insert(AGENT_ID, &row.content, &row.memory_type, row.importance, embedding)
            .await?;
        refs.insert(row.ref_.clone(), id);
    }

    let (mean_norm, max_norm_deviation) = norm_stats(&norms);

    Ok(SeedResult { store, refs, store_path, mean_norm, max_norm_deviation })
}

/// A path under the system temp directory, unique per run. Not cleaned up
/// afterward — an unexpected retrieval number is easier to diagnose with the
/// exact database that produced it still on disk than with a clean rerun
/// that may not reproduce it. `tempfile` (which would auto-delete) is a
/// dev-dependency in this crate and does not reach `[[bin]]` targets at all,
/// which makes this a constraint as much as a choice.
fn unique_store_path() -> PathBuf {
    std::env::temp_dir().join(format!("rag-eval-{}", uuid::Uuid::new_v4()))
}

fn l2_norm(v: &[f32]) -> f64 {
    v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt()
}

/// Mean norm and the largest absolute deviation from 1.0 across `norms`.
/// Meaningful as a sanity check only under `--real` (a normalized embedder
/// should sit near `1.0`); computed unconditionally so a `--fake` run's
/// report can show, in the same field, exactly how far off an unnormalized
/// embedder's vectors are instead of leaving the field silently absent.
fn norm_stats(norms: &[f64]) -> (f64, f64) {
    if norms.is_empty() {
        return (0.0, 0.0);
    }
    let mean = norms.iter().sum::<f64>() / norms.len() as f64;
    let max_deviation = norms
        .iter()
        .map(|n| (n - 1.0).abs())
        .fold(0.0_f64, f64::max);
    (mean, max_deviation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(r: &str, content: &str, memory_type: &str, importance: f64) -> CorpusRow {
        // `CorpusRow`'s fields are all `pub` within the crate; constructed
        // directly here rather than through `golden::load`, since seeding
        // tests are about the insert/embed/map loop, not JSON parsing.
        serde_json::from_value(serde_json::json!({
            "ref": r,
            "content": content,
            "memory_type": memory_type,
            "importance": importance,
        }))
        .expect("row fixture must deserialize")
    }

    #[tokio::test]
    async fn seed_inserts_every_row_and_builds_a_bidirectional_map() {
        let corpus = vec![
            row("c1", "alpha content", "chat_message", 3.0),
            row("c2", "beta content", "code_block", 4.0),
            row("c3", "gamma content", "tool_error", 2.5),
        ];
        let embedder = EvalEmbedder::fake();

        // seed() opens its own temp store; nothing to pass in but the corpus
        // and the embedder.
        let result = seed(&corpus, &embedder).await.expect("seeding must succeed");

        let id1 = result.refs.id_of("c1").expect("c1 must be mapped");
        let id2 = result.refs.id_of("c2").expect("c2 must be mapped");
        let id3 = result.refs.id_of("c3").expect("c3 must be mapped");
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_eq!(result.refs.ref_of(id1), Some("c1"));
        assert_eq!(result.refs.ref_of(id2), Some("c2"));
        assert_eq!(result.refs.ref_of(id3), Some("c3"));
        assert_eq!(result.refs.id_of("does-not-exist"), None);

        std::fs::remove_dir_all(&result.store_path).ok();
    }

    #[tokio::test]
    async fn seed_computes_norm_stats_over_the_actual_embeddings() {
        let corpus = vec![row("c1", "some content", "chat_message", 3.0)];
        let embedder = EvalEmbedder::fake();

        let result = seed(&corpus, &embedder).await.expect("seeding must succeed");

        // FakeEmbedder is documented (embedder.rs) as unnormalized with
        // ‖v‖ ≈ 11.3 — far from 1.0. Asserting a wide, generous band rather
        // than an exact value: the norm is a property of the embedder, not
        // of this function, and pinning it exactly here would make this test
        // fail the moment FakeEmbedder's RNG or dimension changes for
        // unrelated reasons.
        assert!(result.mean_norm > 5.0, "expected an unnormalized fake vector, got mean_norm={}", result.mean_norm);
        assert!(result.max_norm_deviation > 4.0);

        std::fs::remove_dir_all(&result.store_path).ok();
    }

    #[tokio::test]
    async fn seed_uses_a_fresh_store_path_on_every_call() {
        let embedder = EvalEmbedder::fake();
        let a = seed(&[], &embedder).await.expect("seeding an empty corpus must still open a store");
        let b = seed(&[], &embedder).await.expect("seeding an empty corpus must still open a store");
        assert_ne!(a.store_path, b.store_path, "each run must get its own temp store");
        std::fs::remove_dir_all(&a.store_path).ok();
        std::fs::remove_dir_all(&b.store_path).ok();
    }

    #[tokio::test]
    async fn seed_fails_loud_on_an_unknown_memory_type() {
        // A typo'd memory_type slipping past validation (or a caller
        // bypassing `golden::validate` entirely, as this test does) must
        // abort the seeding pass rather than silently skip the bad row: a
        // partially-seeded corpus would make every subsequent case's score
        // meaningless without saying so.
        let corpus = vec![row("c1", "content", "not_a_real_type", 3.0)];
        let embedder = EvalEmbedder::fake();

        // `SeedResult` (the Ok side) does not derive `Debug` — it wraps an
        // opaque `EvalStore` handle deliberately, so `expect_err` (which
        // requires the whole `Result` to be `Debug`) is not usable here.
        // Matching directly on the outcome avoids that requirement.
        match seed(&corpus, &embedder).await {
            Err(EvalError::UnknownMemoryType(_)) => {}
            Err(other) => panic!("expected UnknownMemoryType, got {other}"),
            Ok(_) => panic!("unknown memory_type must fail the whole seed"),
        }
    }
}
