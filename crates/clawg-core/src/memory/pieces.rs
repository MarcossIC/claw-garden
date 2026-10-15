//! The pieces of a long save, linked in `memory_piece`.
//!
//! A text longer than one piece is stored as several ordinary `memory` rows.
//! The link says which save each row is a piece of and in what position, so
//! a search can show one hit per save and the whole text can be read back.
//! A row with no link is a save of one piece, as every row written before the
//! table existed is.

use std::collections::HashMap;

use crate::memory::error::MemoryError;
use crate::memory::search::PiecePosition;
use crate::memory::MemoryStore;

/// One piece the worker stored: the row that holds it and the head its
/// vector was embedded behind, empty when it was not embedded with one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredPiece {
    pub memory_id: i64,
    pub context: String,
}

/// The save a row is a piece of, and where in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SaveLink {
    pub save_id: i64,
    pub position: PiecePosition,
}

fn backend(context: &'static str) -> impl Fn(tokio_rusqlite::Error) -> MemoryError {
    move |e| match e {
        tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
        other => MemoryError::Backend(format!("{context}: {other}")),
    }
}

impl MemoryStore {
    /// Links `pieces`, in order, as one save and returns its id.
    ///
    /// Saving the same long text again finds every piece already stored and
    /// would link the same rows once more; a save whose rows, in order, are
    /// already linked under one id answers that id and writes nothing.
    pub(crate) async fn link_pieces(&self, pieces: Vec<StoredPiece>) -> Result<i64, MemoryError> {
        self.conn
            .call(move |c| -> rusqlite::Result<i64> {
                let tx = c.transaction()?;
                if let Some(id) = existing_save(&tx, &pieces)? {
                    return Ok(id);
                }
                let save_id: i64 =
                    tx.query_row("SELECT COALESCE(MAX(save_id), 0) + 1 FROM memory_piece", [], |r| r.get(0))?;
                {
                    let mut insert = tx.prepare(
                        "INSERT INTO memory_piece (save_id, piece, memory_id, context) VALUES (?1, ?2, ?3, ?4)",
                    )?;
                    for (i, p) in pieces.iter().enumerate() {
                        insert.execute(rusqlite::params![save_id, i as i64 + 1, p.memory_id, p.context])?;
                    }
                }
                tx.commit()?;
                Ok(save_id)
            })
            .await
            .map_err(backend("link_pieces"))
    }

    /// The save each of `memory_ids` is a piece of. A row linked in several
    /// saves (a piece the dedup absorbed) answers the oldest one.
    pub(crate) async fn save_links(&self, memory_ids: Vec<i64>) -> Result<HashMap<i64, SaveLink>, MemoryError> {
        if memory_ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.conn
            .call(move |c| -> rusqlite::Result<HashMap<i64, SaveLink>> {
                let placeholders = vec!["?"; memory_ids.len()].join(", ");
                let sql = format!(
                    "SELECT p.memory_id, p.save_id, p.piece, \
                            (SELECT COUNT(*) FROM memory_piece q WHERE q.save_id = p.save_id) \
                     FROM memory_piece p WHERE p.memory_id IN ({placeholders}) \
                     ORDER BY p.memory_id, p.save_id, p.piece"
                );
                let mut stmt = c.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params_from_iter(memory_ids.iter()), |r| {
                    Ok((r.get::<_, i64>(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })?;
                let mut links = HashMap::new();
                for row in rows {
                    let (memory_id, save_id, piece, pieces) = row?;
                    links.entry(memory_id).or_insert(SaveLink { save_id, position: PiecePosition { piece, pieces } });
                }
                Ok(links)
            })
            .await
            .map_err(backend("save_links"))
    }
}

/// The save, if any, that already links exactly these rows in this order.
fn existing_save(c: &rusqlite::Connection, pieces: &[StoredPiece]) -> rusqlite::Result<Option<i64>> {
    let Some(first) = pieces.first() else {
        return Ok(None);
    };
    let mut candidates = c.prepare("SELECT save_id FROM memory_piece WHERE memory_id = ?1 AND piece = 1")?;
    let ids: Vec<i64> = candidates.query_map([first.memory_id], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let mut rows = c.prepare("SELECT memory_id FROM memory_piece WHERE save_id = ?1 ORDER BY piece")?;
    for id in ids {
        let linked: Vec<i64> = rows.query_map([id], |r| r.get(0))?.collect::<Result<_, _>>()?;
        if linked.iter().copied().eq(pieces.iter().map(|p| p.memory_id)) {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(memory_id: i64, context: &str) -> StoredPiece {
        StoredPiece { memory_id, context: context.to_string() }
    }

    async fn links(store: &MemoryStore) -> Vec<(i64, i64, i64, String)> {
        store
            .conn
            .call(|c| {
                let mut stmt =
                    c.prepare("SELECT save_id, piece, memory_id, context FROM memory_piece ORDER BY save_id, piece")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
                rows.collect::<Result<Vec<_>, _>>()
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn link_pieces_numbers_saves_in_order() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let first = store.link_pieces(vec![piece(10, ""), piece(11, "head")]).await.unwrap();
        let second = store.link_pieces(vec![piece(20, ""), piece(11, ""), piece(22, "other")]).await.unwrap();

        assert_eq!((first, second), (1, 2));
        assert_eq!(
            links(&store).await,
            vec![
                (1, 1, 10, String::new()),
                (1, 2, 11, "head".to_string()),
                (2, 1, 20, String::new()),
                (2, 2, 11, String::new()),
                (2, 3, 22, "other".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn linking_the_same_rows_again_answers_the_save_they_already_form() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let first = store.link_pieces(vec![piece(10, ""), piece(11, "head")]).await.unwrap();

        let again = store.link_pieces(vec![piece(10, ""), piece(11, "")]).await.unwrap();
        let longer = store.link_pieces(vec![piece(10, ""), piece(11, ""), piece(12, "")]).await.unwrap();

        assert_eq!(again, first);
        assert_eq!(longer, 2, "a different list of rows is a different save");
        assert_eq!(links(&store).await.len(), 5);
    }

    fn vector_bytes(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    /// A store left by the previous version holds a long save as unrelated
    /// rows. Opening it adds the table and links nothing; every row is still
    /// found, listed and re-embedded from its own text.
    #[tokio::test]
    async fn rows_from_v10_survive_the_piece_table() {
        use crate::memory::embedder::{EmbedRole, Embedder, FakeEmbedder};
        let dir = tempfile::tempdir().unwrap();
        crate::memory::db::seed_on_disk(dir.path(), Some(10)).await;
        let pieces = [
            "Notes on clawg-ledger, the crate that settles balances, first part",
            "a second part that never names the crate it belongs to",
            "a third part about the nightly reconciliation job",
        ];
        let fake = FakeEmbedder::default();
        let mut vectors = Vec::new();
        for p in pieces {
            vectors.push(vector_bytes(&fake.embed_with_role(p, EmbedRole::Passage).await.unwrap()));
        }
        {
            let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
            for (p, v) in pieces.iter().zip(&vectors) {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, source, created_at, created_by_type, \
                      created_by_ref, last_modified_at, last_accessed_at, importance, tier, status) \
                     VALUES ('agent', 'agent-seed', ?1, 'semantic_fact', 'explicit', 1, 'user', \
                      'agent-seed', 1, 1, 5.0, 3, 'active')",
                    [p],
                )
                .unwrap();
                c.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (last_insert_rowid(), ?1)",
                    [v],
                )
                .unwrap();
            }
        }

        let store = MemoryStore::open(dir.path(), crate::memory::store_lock::StoreHolder::Eval).await.unwrap();

        assert_eq!(std::fs::read_dir(dir.path().join("backups")).unwrap().count(), 1, "a pre-migration snapshot");
        assert!(links(&store).await.is_empty(), "no row is linked by the migration");
        let listed = crate::memory::admin::list_memories_core(&store, Default::default()).await.unwrap();
        assert_eq!(listed.len(), 4, "the seeded rows are all listed");
        let query = fake.embed_with_role(pieces[1], EmbedRole::Query).await.unwrap();
        let hits = crate::memory::search_all_memories(
            &store,
            &query,
            Some(pieces[1]),
            "agent-seed",
            crate::memory::SearchScope::all_default(),
            10,
        )
        .await
        .unwrap();
        assert!(hits.iter().any(|h| h.content == pieces[1]), "an unlinked piece is still found: {hits:?}");

        let texts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
        #[async_trait::async_trait]
        impl Embedder for Recorder {
            async fn embed_with_role(
                &self,
                text: &str,
                role: EmbedRole,
            ) -> Result<Vec<f32>, crate::memory::embedder::EmbedError> {
                self.0.lock().unwrap().push(text.to_string());
                FakeEmbedder::default().embed_with_role(text, role).await
            }
            fn dimensions(&self) -> usize { 384 }
            fn model_id(&self) -> &str { "recorder" }
            fn query_prefix(&self) -> &str { "" }
            fn passage_prefix(&self) -> &str { "" }
        }
        let plan = crate::memory::maintenance::ReInferPlan::new(std::sync::Arc::new(Recorder(texts.clone())));
        crate::memory::maintenance::reembed_all(&store, crate::memory::maintenance::ReembedMode::ReInfer(plan)).await;
        let texts = texts.lock().unwrap();
        for p in pieces {
            assert!(texts.iter().any(|t| t == p), "{p} is re-embedded from its own text");
        }
    }
}
