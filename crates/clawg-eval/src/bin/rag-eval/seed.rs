//! Seeds a golden set's corpus into a fresh, run-scoped store.
//!
//! Every corpus row is inserted under one shared agent id — mirroring
//! production, where a single agent's memories all live in one pool — and
//! the resulting autoincrement ids are recorded in a [`RefMap`] so every
//! score and error message downstream can stay in the golden set's own
//! `ref` space instead of leaking a bare database id.

use std::path::PathBuf;

use clawg_core::eval_api::{EmbedRole, EvalEmbedder, EvalError, EvalStore};

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
    /// Where the temp store was opened.
    pub store_path: PathBuf,
    pub mean_norm: f64,
    pub max_norm_deviation: f64,
    /// Every corpus row's own ref paired with the embedding that was inserted
    /// for it, cloned in the same loop that seeds the store. The store only
    /// ever hands a vector back through a distance, never the vector itself,
    /// so a caller that needs the raw corpus vectors — the similarity
    /// measurement block ranking by cosine — has to hold its own copy from
    /// the one place they exist before being moved into the store. Cloning
    /// here costs one extra `Vec<f32>` per row; re-embedding the corpus a
    /// second time to get the same vectors back would cost the whole
    /// inference pass again.
    pub vectors: Vec<(String, Vec<f32>)>,
}

/// Opens a fresh store at `store_path` and inserts every row of `corpus` into it,
/// embedding each row's content with `embedder`. Fails loud: the first
/// embedding or insert error aborts the whole pass rather than returning a
/// partially-seeded store that would make every case scored against it
/// silently wrong.
///
/// `fts_tokenizer`, when given, swaps `memory_fts`'s tokenizer via
/// `EvalStore::recreate_fts_with_tokenizer` immediately after opening and
/// before any row is inserted — so every row lands through the AFTER INSERT
/// trigger under the requested tokenizer, with no separate backfill step.
/// `None` leaves the store on the tokenizer the V8 migration already applied.
pub async fn seed(
    store_path: &std::path::Path,
    corpus: &[CorpusRow],
    embedder: &EvalEmbedder,
    fts_tokenizer: Option<&str>,
) -> Result<SeedResult, EvalError> {
    let store = EvalStore::open(store_path).await?;
    if let Some(tokenizer) = fts_tokenizer {
        store.recreate_fts_with_tokenizer(tokenizer).await?;
    }
    seed_onto(store, store_path.to_path_buf(), corpus, embedder).await
}

/// Inserts every row of `corpus` into `store`, open at `store_path`, which
/// may already hold rows: a background the corpus is measured among.
pub async fn seed_onto(
    store: EvalStore,
    store_path: PathBuf,
    corpus: &[CorpusRow],
    embedder: &EvalEmbedder,
) -> Result<SeedResult, EvalError> {
    let mut refs = RefMap::new();
    let mut norms: Vec<f64> = Vec::with_capacity(corpus.len());
    let mut vectors: Vec<(String, Vec<f32>)> = Vec::with_capacity(corpus.len());

    for row in corpus {
        // Passage: corpus rows are the stored side of the space — this must
        // match production's worker.rs role, or the harness seeds a space
        // production never writes to.
        let embedding = embedder.embed_with_role(&row.content, EmbedRole::Passage).await?;
        norms.push(l2_norm(&embedding));
        vectors.push((row.ref_.clone(), embedding.clone()));
        let id = store
            .insert(AGENT_ID, &row.content, &row.memory_type, row.importance, embedding, age_ms(row.age_days))
            .await?;
        refs.insert(row.ref_.clone(), id);
    }

    let (mean_norm, max_norm_deviation) = norm_stats(&norms);

    Ok(SeedResult { store, refs, store_path, mean_norm, max_norm_deviation, vectors })
}

pub(crate) const DAY_MS: f64 = 86_400_000.0;

/// A row's `age_days` as the whole milliseconds the store backdates it by.
pub(crate) fn age_ms(age_days: f64) -> u64 {
    (age_days * DAY_MS).round() as u64
}

/// A path under the system temp directory, unique per run, for the store the
/// run seeds or copies. The run removes it when it ends; `--keep-store` keeps
/// a seeded one, to open the database behind an unexpected number.
pub(crate) fn unique_store_path() -> PathBuf {
    std::env::temp_dir().join(format!("rag-eval-{}", uuid::Uuid::new_v4()))
}

/// Removal is retried: on Windows another process (a virus scanner, the
/// search indexer) can hold a file it has just seen written for a moment, and
/// a directory holding an open file cannot be deleted. Failing to delete is
/// reported, not fatal: the measurement itself is already complete.
pub(crate) async fn remove_scratch(scratch: &std::path::Path) {
    for _ in 0..40 {
        match std::fs::remove_dir_all(scratch) {
            Ok(()) => return,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
        }
    }
    eprintln!("warning: could not remove the scratch directory {}", scratch.display());
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
    use crate::test_temp::temp_path;

    #[test]
    fn age_days_converts_to_whole_milliseconds() {
        assert_eq!(age_ms(0.0), 0);
        assert_eq!(age_ms(30.0), 2_592_000_000);
        assert_eq!(age_ms(0.5), 43_200_000);
    }

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
        let store = temp_path("seed-map");

        let result = seed(&store, &corpus, &embedder, None).await.expect("seeding must succeed");

        let id1 = result.refs.id_of("c1").expect("c1 must be mapped");
        let id2 = result.refs.id_of("c2").expect("c2 must be mapped");
        let id3 = result.refs.id_of("c3").expect("c3 must be mapped");
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_eq!(result.refs.ref_of(id1), Some("c1"));
        assert_eq!(result.refs.ref_of(id2), Some("c2"));
        assert_eq!(result.refs.ref_of(id3), Some("c3"));
        assert_eq!(result.refs.id_of("does-not-exist"), None);
        result.store.close().await.unwrap();
    }

    #[tokio::test]
    async fn seed_computes_norm_stats_over_the_actual_embeddings() {
        let corpus = vec![row("c1", "some content", "chat_message", 3.0)];
        let embedder = EvalEmbedder::fake();
        let store = temp_path("seed-norms");

        let result = seed(&store, &corpus, &embedder, None).await.expect("seeding must succeed");

        // FakeEmbedder is documented (embedder.rs) as unnormalized with
        // ‖v‖ ≈ 11.3 — far from 1.0. Asserting a wide, generous band rather
        // than an exact value: the norm is a property of the embedder, not
        // of this function, and pinning it exactly here would make this test
        // fail the moment FakeEmbedder's RNG or dimension changes for
        // unrelated reasons.
        assert!(result.mean_norm > 5.0, "expected an unnormalized fake vector, got mean_norm={}", result.mean_norm);
        assert!(result.max_norm_deviation > 4.0);
        result.store.close().await.unwrap();
    }

    #[tokio::test]
    async fn seed_result_carries_every_corpus_vector_at_the_embedders_dimension() {
        let corpus = vec![
            row("c1", "alpha content", "chat_message", 3.0),
            row("c2", "beta content", "code_block", 4.0),
        ];
        let embedder = EvalEmbedder::fake();
        let store = temp_path("seed-vectors");

        let result = seed(&store, &corpus, &embedder, None).await.expect("seeding must succeed");

        assert_eq!(result.vectors.len(), corpus.len());
        for (_ref_, vector) in &result.vectors {
            assert_eq!(vector.len(), embedder.dimensions());
        }
        result.store.close().await.unwrap();
    }

    #[test]
    fn every_run_gets_a_fresh_store_path_under_the_temp_dir() {
        let (a, b) = (unique_store_path(), unique_store_path());
        assert_ne!(a, b, "each run must get its own temp store");
        assert!(a.starts_with(std::env::temp_dir()));
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
        let store = temp_path("seed-unknown-type");

        // `SeedResult` (the Ok side) does not derive `Debug` — it wraps an
        // opaque `EvalStore` handle deliberately, so `expect_err` (which
        // requires the whole `Result` to be `Debug`) is not usable here.
        // Matching directly on the outcome avoids that requirement.
        match seed(&store, &corpus, &embedder, None).await {
            Err(EvalError::UnknownMemoryType(_)) => {}
            Err(other) => panic!("expected UnknownMemoryType, got {other}"),
            Ok(_) => panic!("unknown memory_type must fail the whole seed"),
        }
    }

    #[tokio::test]
    async fn seed_with_a_tokenizer_swaps_it_before_any_row_lands() {
        // Wiring-level smoke test: seeding under a non-default tokenizer must
        // still succeed and map every row normally. The deeper claim — that a
        // row inserted after the swap is immediately lexically findable with
        // no separate backfill — is asserted at the DB level in
        // `eval_api.rs`'s `recreate_fts_with_tokenizer_indexes_rows_inserted_
        // after_the_swap`, where `EvalStore`'s inner connection is reachable;
        // `EvalStore` is opaque from this module, so that assertion cannot be
        // repeated here.
        let corpus = vec![
            row("c1", "trigram needle content", "chat_message", 3.0),
            row("c2", "second row content", "chat_message", 3.0),
        ];
        let embedder = EvalEmbedder::fake();
        let store = temp_path("seed-trigram");

        let result = seed(&store, &corpus, &embedder, Some("trigram")).await.expect("seeding must succeed");
        assert!(result.refs.id_of("c1").is_some());
        assert!(result.refs.id_of("c2").is_some());
        result.store.close().await.unwrap();
    }

    #[tokio::test]
    async fn seed_rejects_an_unlisted_tokenizer_before_seeding_anything() {
        let corpus = vec![row("c1", "content", "chat_message", 3.0)];
        let embedder = EvalEmbedder::fake();
        let store = temp_path("seed-porter");

        match seed(&store, &corpus, &embedder, Some("porter")).await {
            Err(EvalError::UnknownTokenizer(t)) => assert_eq!(t, "porter"),
            Err(other) => panic!("expected UnknownTokenizer, got {other}"),
            Ok(_) => panic!("an unlisted tokenizer must fail before any row is inserted"),
        }
    }
}
