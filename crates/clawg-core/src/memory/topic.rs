//! The topic key a deliberate save is a version of.
//!
//! A save that names a key replaces the live rows under the same key in the
//! same project, whoever saved them, as far as its agent reaches them; the
//! replaced rows stay, `superseded` by the new save, as its history.

use rusqlite::Connection;
use serde::Serialize;

use crate::memory::clock::TimestampMs;
use crate::memory::contradiction::{excerpt, Candidate};
use crate::memory::dispute::{release_challengers, SUPERSEDE_SQL};
use crate::memory::error::MemoryError;
use crate::memory::events::MemoryEvent;
use crate::memory::pieces::{link_in, StoredPiece};
use crate::memory::search::{reach_predicate, reader_bind};
use crate::memory::sharing::Reader;
use crate::memory::MemoryStore;

/// Longest key accepted, in characters.
pub const MAX_TOPIC_KEY_LEN: usize = 80;

/// Why a `topic_key` is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TopicKeyError {
    #[error("topic_key must be 1 to {MAX_TOPIC_KEY_LEN} characters")]
    Length,
    #[error("topic_key is segments of a-z, 0-9, '.', '_' or '-' joined by '/', with no empty segment")]
    Shape,
}

/// Checks a key: lowercase segments of `[a-z0-9._-]` joined by `/`, none
/// empty, 1 to [`MAX_TOPIC_KEY_LEN`] characters in all. Lowercase only, so
/// one topic written twice in another case is still one key.
pub fn validate_topic_key(key: &str) -> Result<(), TopicKeyError> {
    if key.is_empty() || key.chars().count() > MAX_TOPIC_KEY_LEN {
        return Err(TopicKeyError::Length);
    }
    let segment_ok = |s: &str| {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
    };
    if key.split('/').all(segment_ok) {
        Ok(())
    } else {
        Err(TopicKeyError::Shape)
    }
}

/// The rows a key reaches: in one project (or in none), under one key, of
/// any author the saver reaches. A key names a topic of the project, so two
/// agents that save under it revise one topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TopicScope {
    pub saver: Reader,
    pub project_id: Option<String>,
    pub key: String,
}

/// A live row of a scope, as a revision reads it before saving.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TopicRow {
    pub id: i64,
    pub memory_type: String,
    pub importance: f64,
}

/// A row a revision superseded, with the first line of its text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Replaced {
    pub id: i64,
    pub excerpt: String,
}

/// A finished save that names a key: `save` is every row it holds, in order,
/// the first being the row the replaced ones point at, and `inserted` the
/// rows it wrote.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Revision {
    pub scope: TopicScope,
    pub save: Vec<i64>,
    pub inserted: Vec<i64>,
}

/// The live rows of a scope; `?2` is the project, matched as `idx_memory_topic`
/// indexes it, and `?1` the saver's reach (`reach_bind`).
const SCOPE_ROWS_SQL: &str = concat!(
    "SELECT id, memory_type, importance, content, tier, access_count FROM memory \
     WHERE COALESCE(project_id, '') = COALESCE(?2, '') AND topic_key = ?3 \
       AND deleted_at IS NULL AND status IN ('active', 'disputed') AND ",
    reach_predicate!("agent_id", "?1"),
    " ORDER BY id"
);

/// Whether row `?1` is a piece of a save that belongs to something else: one
/// whose first piece is live without key `?2`. Superseding it would cut a hole
/// in that save, so it is left live and leaves the topic instead.
const SHARED_PIECE_SQL: &str = "SELECT EXISTS (SELECT 1 FROM memory_piece p \
     JOIN memory_piece f ON f.save_id = p.save_id AND f.piece = 1 \
     JOIN memory o ON o.id = f.memory_id \
     WHERE p.memory_id = ?1 AND o.deleted_at IS NULL AND o.status IN ('active', 'disputed') \
       AND o.topic_key IS NOT ?2)";

/// Reads the live rows of `scope`, oldest first.
fn scope_rows(c: &Connection, scope: &TopicScope) -> rusqlite::Result<Vec<(TopicRow, String, i64, i64)>> {
    let mut stmt = c.prepare_cached(SCOPE_ROWS_SQL)?;
    let reach = reader_bind(Some(&scope.saver));
    let rows = stmt.query_map(rusqlite::params![reach, scope.project_id, scope.key], |r| {
        Ok((TopicRow { id: r.get(0)?, memory_type: r.get(1)?, importance: r.get(2)? }, r.get(3)?, r.get(4)?, r.get(5)?))
    })?;
    rows.collect()
}

/// Applies `revision` inside the caller's transaction: every live row of its
/// scope outside the save is superseded by the save's first row, except a
/// piece another save still needs, which only loses the key. The rows the save
/// inserted take the best tier and the most accesses of the ones replaced, and
/// the save's rows without a key take this one.
pub(crate) fn revise_in(c: &Connection, revision: &Revision, now: TimestampMs) -> rusqlite::Result<Vec<Replaced>> {
    let Some(&head) = revision.save.first() else {
        return Ok(Vec::new());
    };
    let key = &revision.scope.key;
    let mut replaced = Vec::new();
    let mut best_tier: Option<i64> = None;
    let mut most_accesses: Option<i64> = None;
    for (row, content, tier, accesses) in scope_rows(c, &revision.scope)? {
        if revision.save.contains(&row.id) {
            continue;
        }
        let shared: bool = c.query_row(SHARED_PIECE_SQL, rusqlite::params![row.id, key], |r| r.get(0))?;
        if shared {
            c.execute(
                "UPDATE memory SET topic_key = NULL, last_modified_at = ?2 WHERE id = ?1",
                rusqlite::params![row.id, now],
            )?;
            continue;
        }
        c.execute(SUPERSEDE_SQL, rusqlite::params![row.id, head, now, "revision"])?;
        release_challengers(c, row.id, now)?;
        best_tier = Some(best_tier.map_or(tier, |t| t.min(tier)));
        most_accesses = Some(most_accesses.map_or(accesses, |a| a.max(accesses)));
        replaced.push(Replaced { id: row.id, excerpt: excerpt(&content) });
    }
    if let (Some(tier), Some(accesses)) = (best_tier, most_accesses) {
        let mut inherit = c.prepare_cached(
            "UPDATE memory SET tier = MIN(tier, ?2), access_count = MAX(access_count, ?3) WHERE id = ?1",
        )?;
        for id in &revision.inserted {
            inherit.execute(rusqlite::params![id, tier, accesses])?;
        }
    }
    let mut adopt = c.prepare_cached("UPDATE memory SET topic_key = ?2 WHERE id = ?1 AND topic_key IS NULL")?;
    for id in &revision.save {
        adopt.execute(rusqlite::params![id, key])?;
    }
    Ok(replaced)
}

/// `evt` with the type and importance it does not name taken from `earlier`,
/// the oldest live row of its scope.
pub(crate) fn inherit(evt: &MemoryEvent, earlier: &TopicRow) -> MemoryEvent {
    MemoryEvent {
        memory_type: evt.memory_type.clone().or_else(|| Some(earlier.memory_type.clone())),
        importance: evt.importance.or(Some(earlier.importance)),
        ..evt.clone()
    }
}

/// Marks, inside the caller's transaction, the pair that joins the save of
/// `pieces` to `candidate`, the row its warning names. Taking the key the
/// warning asks for keys the save's own row, which from then on reads like a
/// row saved under a key from the start, so only the pair says it was warned.
pub(crate) fn mark_warned_in(c: &Connection, pieces: &[StoredPiece], candidate: i64) -> rusqlite::Result<()> {
    let mut mark =
        c.prepare_cached("UPDATE memory_conflict SET topic_key_warned = 1 WHERE new_id = ?1 AND old_id = ?2")?;
    for piece in pieces {
        mark.execute([piece.memory_id, candidate])?;
    }
    Ok(())
}

/// The earlier row a save without a key most likely revises: the closest of
/// the detector's candidates in the save's own project, the only rows a key
/// could revise, whoever saved them, and of two as close the first found. A save under
/// a key has none: the key already revises.
pub(crate) fn unkeyed_revision(topic: Option<&str>, related: &[Candidate]) -> Option<i64> {
    if topic.is_some() {
        return None;
    }
    related.iter().filter(|c| c.same_scope).min_by(|a, b| a.distance.total_cmp(&b.distance)).map(|c| c.id)
}

impl MemoryStore {
    /// The live rows of `scope`, oldest first.
    pub(crate) async fn topic_rows(&self, scope: &TopicScope) -> Result<Vec<TopicRow>, MemoryError> {
        let scope = scope.clone();
        self.conn
            .call(move |c| scope_rows(c, &scope).map(|rows| rows.into_iter().map(|(row, ..)| row).collect()))
            .await
            .map_err(backend("topic_rows"))
    }

    /// Finishes a save no outbox row holds: links its pieces when there are
    /// two or more, applies its revision and marks the pair with the row its
    /// warning names, in one transaction.
    pub(crate) async fn finish_save(
        &self,
        pieces: Vec<StoredPiece>,
        revision: Option<Revision>,
        warned: Option<i64>,
    ) -> Result<Vec<Replaced>, MemoryError> {
        let now = TimestampMs::now();
        self.conn
            .call(move |c| -> rusqlite::Result<Vec<Replaced>> {
                let tx = c.transaction()?;
                if pieces.len() >= 2 {
                    link_in(&tx, &pieces)?;
                }
                let replaced = match &revision {
                    Some(revision) => revise_in(&tx, revision, now)?,
                    None => Vec::new(),
                };
                if let Some(candidate) = warned {
                    mark_warned_in(&tx, &pieces, candidate)?;
                }
                tx.commit()?;
                Ok(replaced)
            })
            .await
            .map_err(backend("finish_save"))
    }
}

fn backend(context: &'static str) -> impl Fn(tokio_rusqlite::Error) -> MemoryError {
    move |e| match e {
        tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
        tokio_rusqlite::Error::ConnectionClosed => MemoryError::Backend(format!("connection closed during {context}")),
        other => MemoryError::Backend(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: i64, distance: f64, same_scope: bool) -> Candidate {
        Candidate { id, distance, escalated: false, excerpt: String::new(), same_scope }
    }

    #[test]
    fn an_unkeyed_save_revises_its_closest_candidate_of_its_own_scope() {
        let related = [candidate(1, 0.30, true), candidate(2, 0.12, false), candidate(3, 0.20, true)];
        assert_eq!(unkeyed_revision(None, &related), Some(3));
        assert_eq!(unkeyed_revision(Some("build/tool"), &related), None, "a key already revises");
        assert_eq!(unkeyed_revision(None, &related[1..2]), None, "a row of another scope is no key's");
        assert_eq!(unkeyed_revision(None, &[]), None);
        let tied = [candidate(4, 0.2, true), candidate(5, 0.2, true)];
        assert_eq!(unkeyed_revision(None, &tied), Some(4), "of two as close, the first found");
    }

    #[test]
    fn topic_key_cases() {
        let long = "a".repeat(MAX_TOPIC_KEY_LEN);
        let too_long = "a".repeat(MAX_TOPIC_KEY_LEN + 1);
        let cases: &[(&str, Result<(), TopicKeyError>)] = &[
            ("storage", Ok(())),
            ("storage/engine", Ok(())),
            ("clawg-core/ingest.outbox_v2", Ok(())),
            ("0.18/release", Ok(())),
            (&long, Ok(())),
            ("", Err(TopicKeyError::Length)),
            (&too_long, Err(TopicKeyError::Length)),
            ("Storage", Err(TopicKeyError::Shape)),
            ("storage engine", Err(TopicKeyError::Shape)),
            ("/storage", Err(TopicKeyError::Shape)),
            ("storage/", Err(TopicKeyError::Shape)),
            ("storage//engine", Err(TopicKeyError::Shape)),
            ("almacenamiento/motor-ñ", Err(TopicKeyError::Shape)),
            ("storage\\engine", Err(TopicKeyError::Shape)),
        ];
        for (key, expected) in cases {
            assert_eq!(&validate_topic_key(key), expected, "{key:?}");
        }
    }

    #[test]
    fn the_error_names_the_field() {
        for e in [TopicKeyError::Length, TopicKeyError::Shape] {
            assert!(e.to_string().starts_with("topic_key "), "{e}");
        }
    }

    #[tokio::test]
    async fn the_scope_lookup_uses_the_topic_index() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let plan: Vec<String> = store
            .conn
            .call(|c| -> rusqlite::Result<Vec<String>> {
                let saver = Reader::new("a", crate::memory::sharing::Reach::AllBut(Vec::new()));
                let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {SCOPE_ROWS_SQL}"))?;
                let rows = stmt
                    .query_map(rusqlite::params![reader_bind(Some(&saver)), Option::<String>::None, "k"], |r| r.get(3))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
            .unwrap();
        let plan = plan.join(" | ");
        assert!(plan.contains("idx_memory_topic"), "{plan}");
    }
}
