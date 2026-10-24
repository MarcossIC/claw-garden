//! The pieces of a long save, linked in `memory_piece`.
//!
//! A text longer than one piece is stored as several ordinary `memory` rows.
//! The link says which save each row is a piece of and in what position, so
//! a search can show one hit per save and the whole text can be read back.
//! A row with no link is a save of one piece, as every row written before the
//! table existed is.

use std::collections::HashMap;

use crate::memory::admin::{get_memory_core, MemoryKind};
use crate::memory::error::MemoryError;
use crate::memory::search::PiecePosition;
use crate::memory::sharing::Reader;
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

/// The save a row belongs to, as much of it as the caller may read.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SaveText {
    pub kind: MemoryKind,
    /// The row that was asked for, and its position (from 1) in the save.
    pub id: i64,
    pub piece: i64,
    pub pieces: i64,
    /// The row at each position, `None` where it is deleted or another
    /// agent's.
    pub ids: Vec<Option<i64>>,
    /// Positions, from 1, with no row to show.
    pub missing: Vec<i64>,
    /// The visible pieces in order, each overlap kept once.
    pub text: String,
    /// The key the row asked for was saved under, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    /// The project the row asked for was saved in, `None` without one; sent
    /// as `null` rather than left out, as a search hit's is.
    pub project_id: Option<String>,
}

/// Shortest repeated text treated as the overlap between two neighbours;
/// anything shorter is as likely a coincidence as a repeat.
const MIN_OVERLAP_CHARS: usize = 16;
/// Farthest back an overlap is looked for. The chunker repeats at least 24
/// tokens between the pieces of a save (64 between chunks), stretched back to
/// where a word starts: well under this at any realistic token length.
const MAX_OVERLAP_BYTES: usize = 2048;

/// Joins neighbouring pieces, keeping once the longest end of each that the
/// next one starts with. Pieces with no such overlap, as on both sides of a
/// missing one, are joined by a line break with nothing cut.
pub(crate) fn join_pieces(pieces: &[&str]) -> String {
    let Some((first, rest)) = pieces.split_first() else {
        return String::new();
    };
    let mut text = first.to_string();
    let mut previous: &str = first;
    for next in rest {
        match overlap(previous, next) {
            Some(len) => text.push_str(&next[len..]),
            None => {
                text.push('\n');
                text.push_str(next);
            }
        }
        previous = next;
    }
    text
}

/// Byte length of the longest end of `a` that `b` starts with.
fn overlap(a: &str, b: &str) -> Option<usize> {
    let earliest = a.len().saturating_sub(MAX_OVERLAP_BYTES);
    a.char_indices()
        .map(|(i, _)| i)
        .filter(|&i| i >= earliest)
        .map(|i| &a[i..])
        .find(|end| end.chars().count() >= MIN_OVERLAP_CHARS && b.starts_with(end))
        .map(str::len)
}

/// One position of a save: its row, and the row's content, author and
/// deletion time, all `None` when the row is gone from `memory`.
type Position = (i64, Option<String>, Option<String>, Option<i64>);

/// Reads the save row `id` of `kind` is a piece of, every position as far as
/// `reader` reaches it. `None` when the row itself is missing, deleted or out
/// of the reader's reach; a row with no link is a save of one piece.
pub async fn read_save_core(
    store: &MemoryStore,
    kind: MemoryKind,
    id: i64,
    reader: Option<&Reader>,
) -> Result<Option<SaveText>, MemoryError> {
    let visible = |author: Option<&str>, deleted_at: Option<i64>| {
        deleted_at.is_none() && reader.is_none_or(|r| r.admits(author))
    };
    let Some(row) = get_memory_core(store, kind, id).await? else {
        return Ok(None);
    };
    if !visible(row.agent_id.as_deref(), row.deleted_at) {
        return Ok(None);
    }
    let topic_key = row.topic_key.clone();
    let project_id = row.project_id.clone();
    let single = |text: String| SaveText {
        kind,
        id,
        piece: 1,
        pieces: 1,
        ids: vec![Some(id)],
        missing: vec![],
        text,
        topic_key: topic_key.clone(),
        project_id: project_id.clone(),
    };
    if kind != MemoryKind::Memory {
        return Ok(Some(single(row.summary)));
    }
    let Some(link) = store.save_links(vec![id]).await?.remove(&id) else {
        return Ok(Some(single(row.summary)));
    };
    let save_id = link.save_id;
    let positions = store
        .conn
        .call(move |c| -> rusqlite::Result<Vec<Position>> {
            let mut stmt = c.prepare(
                "SELECT p.memory_id, m.content, m.agent_id, m.deleted_at \
                 FROM memory_piece p LEFT JOIN memory m ON m.id = p.memory_id \
                 WHERE p.save_id = ?1 ORDER BY p.piece",
            )?;
            let rows = stmt.query_map([save_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect()
        })
        .await
        .map_err(backend("read_save_core"))?;

    let mut ids = Vec::new();
    let mut missing = Vec::new();
    // Pieces on either side of a missing one are not neighbours: each run of
    // visible pieces is joined on its own, and the runs by a line break.
    let mut runs: Vec<Vec<String>> = vec![Vec::new()];
    for (i, (memory_id, content, agent, deleted_at)) in positions.into_iter().enumerate() {
        match content {
            Some(content) if visible(agent.as_deref(), deleted_at) => {
                ids.push(Some(memory_id));
                runs.last_mut().expect("runs starts non-empty").push(content);
            }
            _ => {
                ids.push(None);
                missing.push(i as i64 + 1);
                runs.push(Vec::new());
            }
        }
    }
    let text = runs
        .iter()
        .filter(|run| !run.is_empty())
        .map(|run| join_pieces(&run.iter().map(String::as_str).collect::<Vec<_>>()))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Some(SaveText {
        kind,
        id,
        piece: link.position.piece,
        pieces: link.position.pieces,
        ids,
        missing,
        text,
        topic_key,
        project_id,
    }))
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
    #[cfg(test)]
    pub(crate) async fn link_pieces(&self, pieces: Vec<StoredPiece>) -> Result<i64, MemoryError> {
        self.conn
            .call(move |c| -> rusqlite::Result<i64> {
                let tx = c.transaction()?;
                let save_id = link_in(&tx, &pieces)?;
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
            .call(move |c| read_links(c, &memory_ids))
            .await
            .map_err(backend("save_links"))
    }
}

/// [`MemoryStore::link_pieces`] inside a transaction the caller commits, so the
/// link can land together with whatever else finishes the save.
pub(crate) fn link_in(c: &rusqlite::Connection, pieces: &[StoredPiece]) -> rusqlite::Result<i64> {
    if let Some(id) = existing_save(c, pieces)? {
        return Ok(id);
    }
    let save_id: i64 = c.query_row("SELECT COALESCE(MAX(save_id), 0) + 1 FROM memory_piece", [], |r| r.get(0))?;
    let mut insert =
        c.prepare("INSERT INTO memory_piece (save_id, piece, memory_id, context) VALUES (?1, ?2, ?3, ?4)")?;
    for (i, p) in pieces.iter().enumerate() {
        insert.execute(rusqlite::params![save_id, i as i64 + 1, p.memory_id, p.context])?;
    }
    Ok(save_id)
}

/// [`MemoryStore::save_links`] on a connection already in hand, for a caller
/// that is itself running on the database thread.
pub(crate) fn read_links(c: &rusqlite::Connection, memory_ids: &[i64]) -> rusqlite::Result<HashMap<i64, SaveLink>> {
    if memory_ids.is_empty() {
        return Ok(HashMap::new());
    }
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
    use crate::memory::sharing::Reach;

    /// `agent` as a policy that hides nothing lets it read.
    fn shared(agent: &str) -> Reader {
        Reader::new(agent, Reach::AllBut(Vec::new()))
    }

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

    #[test]
    fn join_pieces_drops_the_overlap_once() {
        let joined = join_pieces(&["alpha beta gamma delta epsilon", "gamma delta epsilon zeta eta"]);
        assert_eq!(joined, "alpha beta gamma delta epsilon zeta eta");
    }

    #[test]
    fn join_pieces_keeps_both_sides_when_nothing_overlaps() {
        assert_eq!(join_pieces(&["one two three", "three four"]), "one two three\nthree four", "under 16 chars");
        assert_eq!(join_pieces(&["first piece of text", "unrelated second one"]), "first piece of text\nunrelated second one");
        assert_eq!(join_pieces(&["only"]), "only");
        assert_eq!(join_pieces(&[]), "");
    }

    #[test]
    fn the_pieces_of_a_split_save_join_back_into_its_text() {
        let text: String = (0..900).map(|i| format!("word{i} ")).collect();
        let split = crate::memory::chunker::ChatChunker::with_fallback().split_save(&text);
        assert!(split.pieces.len() > 2, "the fixture is a long save");
        let pieces: Vec<&str> = split.pieces.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(join_pieces(&pieces), text);
    }

    #[test]
    fn the_pieces_of_a_subword_split_save_join_back_into_its_text() {
        use crate::memory::chunker::token_splitter::tests::{subword_tokenizer, subword_words};
        let text = subword_words(480).join(" ");
        let split = crate::memory::chunker::ChatChunker::with_tokenizer(subword_tokenizer()).split_save(&text);
        assert!(split.pieces.len() > 10, "the fixture is a long save");
        let pieces: Vec<&str> = split.pieces.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(join_pieces(&pieces), text, "no line break where the text has none");
    }

    async fn insert_row(store: &MemoryStore, agent: &str, content: &str) -> i64 {
        let (agent, content) = (agent.to_string(), content.to_string());
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, source, created_at, created_by_type, \
                      created_by_ref, last_modified_at, last_accessed_at, importance, tier, status) \
                     VALUES ('agent', ?1, ?2, 'semantic_fact', 'explicit', 1, 'user', ?1, 1, 1, 5.0, 3, 'active')",
                    rusqlite::params![agent, content],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn read_save_lists_missing_pieces_and_another_agent_reads_it_whole() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let texts = [
            "Notes on clawg-ledger: the retry budget",
            "the retry budget was set to three attempts",
            "a third piece that was deleted later",
            "the fourth and last piece",
        ];
        let mut ids = Vec::new();
        for t in texts {
            ids.push(insert_row(&store, "agent-a", t).await);
        }
        store.link_pieces(ids.iter().map(|&memory_id| piece(memory_id, "")).collect()).await.unwrap();
        let alone = insert_row(&store, "agent-a", "a save of one piece").await;
        let deleted = ids[2];
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET deleted_at = 2 WHERE id = ?1", [deleted]))
            .await
            .unwrap();

        let save = read_save_core(&store, MemoryKind::Memory, ids[1], Some(&shared("agent-a"))).await.unwrap().unwrap();

        assert_eq!(save.id, ids[1]);
        assert_eq!(save.piece, 2);
        assert_eq!(save.pieces, 4);
        assert_eq!(save.ids, vec![Some(ids[0]), Some(ids[1]), None, Some(ids[3])]);
        assert_eq!(save.missing, vec![3]);
        assert_eq!(
            save.text,
            "Notes on clawg-ledger: the retry budget was set to three attempts\nthe fourth and last piece"
        );
        let unscoped = read_save_core(&store, MemoryKind::Memory, ids[1], None).await.unwrap();
        assert_eq!(unscoped, Some(save));

        let by_another = read_save_core(&store, MemoryKind::Memory, ids[1], Some(&shared("agent-b"))).await.unwrap();
        assert_eq!(by_another, unscoped, "another agent reads the save whole");
        let hidden = Reader::new("agent-b", Reach::AllBut(vec!["agent-a".into()]));
        assert_eq!(read_save_core(&store, MemoryKind::Memory, ids[1], Some(&hidden)).await.unwrap(), None);
        assert_eq!(read_save_core(&store, MemoryKind::Memory, deleted, Some(&shared("agent-a"))).await.unwrap(), None);
        assert_eq!(read_save_core(&store, MemoryKind::Memory, 9_999, None).await.unwrap(), None);

        let one = read_save_core(&store, MemoryKind::Memory, alone, Some(&shared("agent-a"))).await.unwrap().unwrap();
        assert_eq!((one.piece, one.pieces, one.ids, one.missing), (1, 1, vec![Some(alone)], vec![]));
        assert_eq!(one.text, "a save of one piece");
    }

    #[tokio::test]
    async fn a_hidden_authors_piece_reads_as_missing() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let first = insert_row(&store, "agent-a", "Notes on clawg-ledger: the retry budget").await;
        let second = insert_row(&store, "agent-c", "the retry budget was set to three attempts").await;
        store.link_pieces(vec![piece(first, ""), piece(second, "")]).await.unwrap();
        let nobodys = store
            .conn
            .call(|c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory (scope, project_id, content, memory_type, source, created_at, created_by_type, \
                     created_by_ref, last_modified_at, last_accessed_at, importance, tier, status) \
                     VALUES ('project', 'p', 'a note no agent saved', 'semantic_fact', 'explicit', 1, 'user', 'x', 1, 1, \
                     5.0, 3, 'active')",
                    [],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap();

        let b = Reader::new("agent-b", Reach::AllBut(vec!["agent-c".into()]));
        let save = read_save_core(&store, MemoryKind::Memory, first, Some(&b)).await.unwrap().unwrap();
        assert_eq!((save.ids, save.missing), (vec![Some(first), None], vec![2]));
        assert_eq!(save.text, "Notes on clawg-ledger: the retry budget");
        assert_eq!(read_save_core(&store, MemoryKind::Memory, second, Some(&b)).await.unwrap(), None);

        let own = Reader::new("agent-b", Reach::Own);
        let note = read_save_core(&store, MemoryKind::Memory, nobodys, Some(&own)).await.unwrap().unwrap();
        assert_eq!(note.text, "a note no agent saved", "every reader reaches a row no agent saved");
    }

    #[tokio::test]
    async fn read_save_carries_the_key_of_the_row_asked_for() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let first = insert_row(&store, "agent-a", "Export format: first piece").await;
        let second = insert_row(&store, "agent-a", "second piece of the export format").await;
        let alone = insert_row(&store, "agent-a", "a save of one piece").await;
        store.link_pieces(vec![piece(first, ""), piece(second, "")]).await.unwrap();
        store
            .conn
            .call(move |c| {
                c.execute("UPDATE memory SET topic_key = 'cli/export-format' WHERE id IN (?1, ?2)", [first, second])?;
                c.execute("UPDATE memory SET topic_key = 'cli/alone' WHERE id = ?1", [alone])
            })
            .await
            .unwrap();

        let reader = shared("agent-a");
        let of = |id| read_save_core(&store, MemoryKind::Memory, id, Some(&reader));
        assert_eq!(of(second).await.unwrap().unwrap().topic_key.as_deref(), Some("cli/export-format"));
        assert_eq!(of(alone).await.unwrap().unwrap().topic_key.as_deref(), Some("cli/alone"));
        let plain = insert_row(&store, "agent-a", "no key here").await;
        assert_eq!(of(plain).await.unwrap().unwrap().topic_key, None);
    }

    fn vector_bytes(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    /// Stores `pieces` as one linked save, each with a vector turned slightly
    /// from the first axis toward `axis`, the way the worker leaves them.
    async fn store_save(store: &MemoryStore, pieces: &[String], head: &str, axis: usize) -> Vec<i64> {
        let mut stored = Vec::new();
        for (i, text) in pieces.iter().enumerate() {
            let id = insert_row(store, "agent-a", text).await;
            let angle = 0.01 * i as f32;
            let mut v = vec![0.0_f32; 384];
            v[0] = angle.cos();
            v[axis] = angle.sin();
            let bytes = vector_bytes(&v);
            store
                .conn
                .call(move |c| {
                    c.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?1, ?2)", rusqlite::params![id, bytes])
                })
                .await
                .unwrap();
            stored.push(piece(id, if i == 0 { "" } else { head }));
        }
        let ids = stored.iter().map(|p| p.memory_id).collect();
        store.link_pieces(stored).await.unwrap();
        ids
    }

    #[tokio::test]
    async fn a_save_split_at_the_old_cap_is_still_found_and_read() {
        use crate::memory::search::{search_all_memories, SearchScope};

        let store = MemoryStore::new_for_test().await.unwrap();
        let chunker = crate::memory::chunker::ChatChunker::with_fallback();
        let old_text: String = (0..500).map(|i| format!("old{i} ")).collect();
        // Cut the way the previous version cut a long save: 424 tokens with
        // the 64 of a chunk.
        let old_pieces: Vec<String> = chunker.chunk(&old_text, 424).into_iter().map(|c| c.text).collect();
        let new_text: String = (0..300).map(|i| format!("new{i} ")).collect();
        let new_split = chunker.split_save(&new_text);
        let new_pieces: Vec<String> = new_split.pieces.into_iter().map(|c| c.text).collect();
        assert!(old_pieces.len() >= 2 && new_pieces.len() > old_pieces.len(), "both fixtures are long saves");

        let old_ids = store_save(&store, &old_pieces, &old_pieces[0][..192], 1).await;
        let new_ids = store_save(&store, &new_pieces, &new_split.head.unwrap(), 2).await;

        let mut query = vec![0.0_f32; 384];
        query[0] = 1.0;
        let hits = search_all_memories(&store, &query, None, "agent-a", SearchScope::all_default(), 10).await.unwrap();

        let found: Vec<(i64, i64)> = hits.iter().map(|h| (h.id, h.piece.expect("a linked row").pieces)).collect();
        assert_eq!(found, vec![(old_ids[0], old_pieces.len() as i64), (new_ids[0], new_pieces.len() as i64)]);
        for (ids, text) in [(&old_ids, &old_text), (&new_ids, &new_text)] {
            let save = read_save_core(&store, MemoryKind::Memory, ids[1], Some(&shared("agent-a"))).await.unwrap().unwrap();
            assert_eq!(&save.text, text, "the save reads back whole, its overlap once");
        }
    }

    /// An embedder that keeps every text it is given and embeds it as the
    /// fake one does.
    struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    #[async_trait::async_trait]
    impl crate::memory::embedder::Embedder for Recorder {
        async fn embed_with_role(
            &self,
            text: &str,
            role: crate::memory::embedder::EmbedRole,
        ) -> Result<Vec<f32>, crate::memory::embedder::EmbedError> {
            self.0.lock().unwrap().push(text.to_string());
            crate::memory::embedder::FakeEmbedder::default().embed_with_role(text, role).await
        }
        fn dimensions(&self) -> usize { 384 }
        fn model_id(&self) -> &str { "recorder" }
        fn query_prefix(&self) -> &str { "" }
        fn passage_prefix(&self) -> &str { "" }
    }

    /// A long save cut the way the splitter cut one before it looked for
    /// word starts: 96 tokens every 72 and a head of the first 48, wherever
    /// they fall.
    fn cut_by_token_count(text: &str, tok: &tokenizers::Tokenizer) -> (Vec<String>, String) {
        let offsets = tok.encode(text, false).expect("the fixture encodes").get_offsets().to_vec();
        let mut pieces = Vec::new();
        let mut start = 0;
        loop {
            let end = (start + 96).min(offsets.len());
            pieces.push(text[offsets[start].0..offsets[end - 1].1].to_string());
            if end == offsets.len() {
                return (pieces, text[..offsets[47].1].to_string());
            }
            start += 72;
        }
    }

    #[tokio::test]
    async fn a_save_cut_inside_words_before_is_still_found_read_and_reembedded() {
        use crate::memory::chunker::embedding_text;
        use crate::memory::chunker::token_splitter::tests::{is_word_boundary, piece_ranges, subword_tokenizer, subword_words};
        use crate::memory::search::{search_all_memories, SearchScope};

        let store = MemoryStore::new_for_test().await.unwrap();
        let tok = subword_tokenizer();
        let words = subword_words(680);
        let old_text = words[..340].join(" ");
        let (old_pieces, old_head) = cut_by_token_count(&old_text, &tok);
        let old_refs: Vec<&str> = old_pieces.iter().map(String::as_str).collect();
        assert!(
            piece_ranges(&old_text, &old_refs).iter().any(|&(s, _)| !is_word_boundary(&old_text, s)),
            "some old piece starts inside a word"
        );
        assert!(!is_word_boundary(&old_text, old_head.len()), "the old head ends inside a word");
        let new_text = words[340..].join(" ");
        let new_split = crate::memory::chunker::ChatChunker::with_tokenizer(tok).split_save(&new_text);
        let new_pieces: Vec<String> = new_split.pieces.into_iter().map(|c| c.text).collect();
        let new_head = new_split.head.expect("a long save has a head");
        assert_eq!((old_pieces.len(), new_pieces.len()), (12, 12), "two saves of 12 pieces");

        let old_ids = store_save(&store, &old_pieces, &old_head, 1).await;
        let new_ids = store_save(&store, &new_pieces, &new_head, 2).await;

        let mut query = vec![0.0_f32; 384];
        query[0] = 1.0;
        let hits = search_all_memories(&store, &query, None, "agent-a", SearchScope::all_default(), 10).await.unwrap();
        let found: Vec<(i64, i64)> = hits.iter().map(|h| (h.id, h.piece.expect("a linked row").pieces)).collect();
        assert_eq!(found, vec![(old_ids[0], 12), (new_ids[0], 12)], "one hit per save");
        for (ids, text) in [(&old_ids, &old_text), (&new_ids, &new_text)] {
            let save = read_save_core(&store, MemoryKind::Memory, ids[5], Some(&shared("agent-a"))).await.unwrap().unwrap();
            assert_eq!(&save.text, text, "the save reads back as it was saved");
        }

        let texts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let plan = crate::memory::maintenance::ReInferPlan::new(std::sync::Arc::new(Recorder(texts.clone())));
        crate::memory::maintenance::reembed_all(&store, crate::memory::maintenance::ReembedMode::ReInfer(plan)).await;
        let mut embedded = texts.lock().unwrap().clone();
        let mut expected: Vec<String> = [(&old_pieces, &old_head), (&new_pieces, &new_head)]
            .into_iter()
            .flat_map(|(pieces, head)| {
                pieces.iter().enumerate().map(move |(i, p)| embedding_text(if i == 0 { "" } else { head }, p).into_owned())
            })
            .collect();
        embedded.sort();
        expected.sort();
        assert_eq!(embedded, expected, "every piece is embedded again behind the head it was saved with");
    }

    /// A store left by the previous version holds a long save as unrelated
    /// rows. Opening it adds the table and links nothing; every row is still
    /// found, listed, read as a save of one piece and re-embedded from its
    /// own text.
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
        let piece_two = hits.iter().find(|h| h.content == pieces[1]).unwrap();
        assert_eq!(piece_two.piece, None);
        let save = read_save_core(&store, MemoryKind::Memory, piece_two.id, Some(&shared("agent-seed"))).await.unwrap().unwrap();
        assert_eq!((save.pieces, save.text.as_str()), (1, pieces[1]), "an unlinked row reads as a save of one piece");

        let texts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let plan = crate::memory::maintenance::ReInferPlan::new(std::sync::Arc::new(Recorder(texts.clone())));
        crate::memory::maintenance::reembed_all(&store, crate::memory::maintenance::ReembedMode::ReInfer(plan)).await;
        let texts = texts.lock().unwrap();
        for p in pieces {
            assert!(texts.iter().any(|t| t == p), "{p} is re-embedded from its own text");
        }
    }
}
