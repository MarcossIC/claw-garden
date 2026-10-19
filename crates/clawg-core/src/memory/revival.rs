//! What becomes of the rows a deleted row had superseded.
//!
//! Deleting the current version of a topic brings back the version its
//! revision replaced, so the topic does not vanish from every read; the row
//! brought back records the delete in `revived_from`, and restoring the
//! deleted row supersedes it again. A row a `supersedes` verdict replaced
//! stays replaced: undoing a judged decision is not a delete's to do.

use rusqlite::{Connection, OptionalExtension};

use crate::memory::clock::TimestampMs;
use crate::memory::contradiction::excerpt;
use crate::memory::dispute::{settle_challengers, settle_row, REVIVE_SQL};
use crate::memory::topic::Replaced;

/// A live row superseded by the deleted one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Replacement {
    pub id: i64,
    /// Replaced by a revision of the deleted row's own topic, in its scope.
    pub revision: bool,
    /// The `memory_piece` save it belongs to, if any.
    pub save: Option<i64>,
}

/// What [`plan_revival`] decides.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Revival {
    /// Brought back, in id order.
    pub revive: Vec<i64>,
    /// Pointed at the first row of `revive` instead of the deleted row.
    pub repoint: Vec<i64>,
    /// Left superseded by the deleted row: a verdict replaced them.
    pub hidden: Vec<i64>,
}

/// Decides, for a deleted row and the rows it superseded, which come back.
/// Nothing does unless the deleted row was the current one (`was_current`)
/// and no other row holds its topic now (`key_held`). Then the save of the
/// newest revision comes back whole, the pieces of a long save together,
/// and the other revisions point at it, so the topic has one current save.
pub(crate) fn plan_revival(was_current: bool, key_held: bool, replaced: &[Replacement]) -> Revival {
    let hidden: Vec<i64> = replaced.iter().filter(|r| !r.revision).map(|r| r.id).collect();
    let revisions: Vec<&Replacement> = replaced.iter().filter(|r| r.revision).collect();
    let top = match revisions.iter().max_by_key(|r| r.id) {
        Some(top) if was_current && !key_held => *top,
        _ => return Revival { hidden, ..Revival::default() },
    };
    let together = |r: &Replacement| r.id == top.id || (top.save.is_some() && r.save == top.save);
    let mut revive: Vec<i64> = revisions.iter().filter(|r| together(r)).map(|r| r.id).collect();
    let mut repoint: Vec<i64> = revisions.iter().filter(|r| !together(r)).map(|r| r.id).collect();
    revive.sort_unstable();
    repoint.sort_unstable();
    Revival { revive, repoint, hidden }
}

/// The rows a deleted row brought back and the ones it leaves hidden.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Revived {
    pub revived: Vec<Replaced>,
    pub hidden: Vec<i64>,
}

/// The live rows `?1` superseded, each marked a revision when it was
/// replaced by a save of the deleted row's agent and project under its key.
const REPLACED_SQL: &str = "SELECT r.id, \
       r.supersede_cause = 'revision' AND r.agent_id IS d.agent_id \
         AND COALESCE(r.project_id, '') = COALESCE(d.project_id, '') AND r.topic_key IS NOT NULL, \
       (SELECT MIN(p.save_id) FROM memory_piece p WHERE p.memory_id = r.id), \
       r.content \
     FROM memory r JOIN memory d ON d.id = ?1 \
     WHERE r.superseded_by = ?1 AND r.status = 'superseded' AND r.deleted_at IS NULL \
     ORDER BY r.id";

/// Whether a live row other than the deleted `?1` holds the topic key `?2`
/// in `?1`'s scope.
const KEY_HELD_SQL: &str = "SELECT EXISTS (SELECT 1 FROM memory d JOIN memory t \
       ON t.agent_id IS d.agent_id AND COALESCE(t.project_id, '') = COALESCE(d.project_id, '') \
     WHERE d.id = ?1 AND t.id <> d.id AND t.topic_key = ?2 \
       AND t.deleted_at IS NULL AND t.status IN ('active', 'disputed'))";

/// Applies [`plan_revival`] to the deleted row `deleted` inside the caller's
/// transaction. A revived row is settled against what is live now, and the
/// rows that disputed it before it was replaced dispute it again.
pub(crate) fn revive_after_delete(c: &Connection, deleted: i64, now: TimestampMs) -> rusqlite::Result<Revived> {
    let Some(status) = c
        .query_row("SELECT status FROM memory WHERE id = ?1 AND deleted_at IS NOT NULL", [deleted], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
    else {
        return Ok(Revived::default());
    };
    let rows: Vec<(Replacement, String)> = {
        let mut stmt = c.prepare_cached(REPLACED_SQL)?;
        let rows = stmt.query_map([deleted], |r| {
            Ok((Replacement { id: r.get(0)?, revision: r.get(1)?, save: r.get(2)? }, r.get(3)?))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let replaced: Vec<Replacement> = rows.iter().map(|(r, _)| r.clone()).collect();
    let key_held = match replaced.iter().filter(|r| r.revision).max_by_key(|r| r.id) {
        Some(top) => {
            let key: String = c.query_row("SELECT topic_key FROM memory WHERE id = ?1", [top.id], |r| r.get(0))?;
            c.query_row(KEY_HELD_SQL, rusqlite::params![deleted, key], |r| r.get(0))?
        }
        None => false,
    };
    let plan = plan_revival(matches!(status.as_str(), "active" | "disputed"), key_held, &replaced);

    let mut revived = Vec::new();
    for &id in &plan.revive {
        c.execute(REVIVE_SQL, rusqlite::params![id, deleted, now])?;
        c.execute("UPDATE memory SET revived_from = ?2 WHERE id = ?1", [id, deleted])?;
        let content = rows.iter().find(|(r, _)| r.id == id).map(|(_, text)| text.as_str()).unwrap_or("");
        revived.push(Replaced { id, excerpt: excerpt(content) });
    }
    if let Some(&head) = plan.revive.first() {
        for &id in &plan.repoint {
            c.execute(
                "UPDATE memory SET superseded_by = ?2, last_modified_at = ?3 WHERE id = ?1",
                rusqlite::params![id, head, now],
            )?;
        }
    }
    for &id in &plan.revive {
        settle_row(c, id, now)?;
        settle_challengers(c, id, now)?;
    }
    Ok(Revived { revived, hidden: plan.hidden })
}

/// Deleted rows still current when deleted that a live revision of theirs
/// still points at: what a delete that predates revivals left behind.
const HIDDEN_SQL: &str = "SELECT DISTINCT d.id FROM memory d JOIN memory r ON r.superseded_by = d.id \
     WHERE d.deleted_at IS NOT NULL AND d.status IN ('active', 'disputed') \
       AND r.deleted_at IS NULL AND r.status = 'superseded' AND r.supersede_cause = 'revision' \
     ORDER BY d.id";

/// Applies [`revive_after_delete`] to every row [`HIDDEN_SQL`] finds, in one
/// transaction, and returns how many rows came back. A second pass finds
/// nothing: what is left points at a row whose topic is held, or is a
/// verdict's.
pub(crate) fn revive_hidden(c: &mut Connection, now: TimestampMs) -> rusqlite::Result<usize> {
    let tx = c.transaction()?;
    let deleted: Vec<i64> = {
        let mut stmt = tx.prepare(HIDDEN_SQL)?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut revived = 0;
    for id in deleted {
        revived += revive_after_delete(&tx, id, now)?.revived.len();
    }
    tx.commit()?;
    Ok(revived)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn revision(id: i64, save: Option<i64>) -> Replacement {
        Replacement { id, revision: true, save }
    }

    fn verdict(id: i64) -> Replacement {
        Replacement { id, revision: false, save: None }
    }

    #[test]
    fn revival_cases() {
        let plan = |was_current, key_held, rows: &[Replacement]| plan_revival(was_current, key_held, rows);
        let revival = |revive: &[i64], repoint: &[i64], hidden: &[i64]| Revival {
            revive: revive.to_vec(),
            repoint: repoint.to_vec(),
            hidden: hidden.to_vec(),
        };

        assert_eq!(plan(true, false, &[]), Revival::default(), "nothing replaced, nothing to do");
        assert_eq!(plan(true, false, &[revision(4, None)]), revival(&[4], &[], &[]), "the version before comes back");
        assert_eq!(plan(false, false, &[revision(4, None)]), revival(&[], &[], &[]), "a replaced row was not current");
        assert_eq!(plan(true, true, &[revision(4, None)]), revival(&[], &[], &[]), "another row holds the topic");
        assert_eq!(plan(true, false, &[verdict(4)]), revival(&[], &[], &[4]), "a verdict stands");
        assert_eq!(plan(false, true, &[verdict(4)]), revival(&[], &[], &[4]), "and is named whatever else holds");
        assert_eq!(
            plan(true, false, &[revision(2, None), revision(5, None), verdict(3)]),
            revival(&[5], &[2], &[3]),
            "the newest comes back and the older one points at it"
        );
        assert_eq!(
            plan(true, false, &[revision(2, None), revision(6, Some(1)), revision(7, Some(1)), revision(8, Some(1))]),
            revival(&[6, 7, 8], &[2], &[]),
            "a long save comes back whole"
        );
        assert_eq!(
            plan(true, false, &[revision(6, Some(1)), revision(7, Some(1)), revision(9, Some(2))]),
            revival(&[9], &[6, 7], &[]),
            "the newest save wins over an older long one"
        );
    }
}
