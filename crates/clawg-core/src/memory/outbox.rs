//! Saves accepted but not yet stored, in `ingest_outbox`.
//!
//! A save is written here, already masked, before the daemon answers it, and
//! its row goes in the transaction that finishes storing it. A process that
//! dies in between leaves the row, and the next start ingests it. Each piece
//! of a long save notes itself in the row in the transaction that writes it,
//! so a replay resumes after the pieces already stored instead of
//! reinforcing them. The decisions about a row are pure functions here; the
//! store methods only read and write.

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::memory::clock::TimestampMs;
use crate::memory::error::MemoryError;
use crate::memory::events::MemoryEvent;
use crate::memory::pieces::{link_in, StoredPiece};
use crate::memory::redact::redact;
use crate::memory::topic::{mark_warned_in, revise_in, Replaced, Revision};
use crate::memory::MemoryStore;

/// How many times a row is taken before it is given up as failed. A save that
/// failed once fails the same way with the same text; the retries are for a
/// process that died while ingesting it, and three deaths in a row on one
/// save say the save is what kills it. A starting value, not measured.
pub const MAX_ATTEMPTS: i64 = 3;

/// How many rows with a cause `/health` shows.
pub const PROBLEMS_SHOWN: usize = 5;

/// Whether a row still waits for the worker or was given up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxState {
    Pending,
    Failed,
}

impl OutboxState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Failed => "failed",
        }
    }

    fn parse(s: &str) -> Self {
        if s == "failed" {
            Self::Failed
        } else {
            Self::Pending
        }
    }
}

/// What taking a row with `attempts` already spent does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClaimStep {
    /// Spend one more attempt and ingest it.
    Process,
    /// Every attempt ended with the process gone: stop replaying it.
    GiveUp,
}

pub(crate) fn claim_step(attempts: i64) -> ClaimStep {
    if attempts < MAX_ATTEMPTS {
        ClaimStep::Process
    } else {
        ClaimStep::GiveUp
    }
}

/// The attempts and state a row keeps after a failure. A transient one (the
/// embedder went away) hands its attempt back: the save itself did nothing
/// wrong.
pub(crate) fn after_failure(attempts: i64, transient: bool) -> (i64, OutboxState) {
    let attempts = if transient { (attempts - 1).max(0) } else { attempts };
    let state = if attempts >= MAX_ATTEMPTS { OutboxState::Failed } else { OutboxState::Pending };
    (attempts, state)
}

/// One piece of a save already stored: where it goes, the hash of its text,
/// the row that holds it, whether this save inserted it, and the head its
/// vector was embedded behind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NotedPiece {
    pub position: usize,
    pub hash: String,
    pub id: i64,
    pub inserted: bool,
    pub context: String,
}

/// The pieces already stored of a save that splits into pieces with
/// `hashes`, by position, or `None` when the progress does not fit that
/// split: the text no longer splits the way it did (the tokenizer changed),
/// and the progress cannot say which row holds which piece.
pub(crate) fn resume_positions(progress: &[NotedPiece], hashes: &[&str]) -> Option<Vec<Option<NotedPiece>>> {
    let mut by_position = vec![None; hashes.len()];
    for noted in progress {
        let slot = by_position.get_mut(noted.position)?;
        if slot.is_some() || hashes[noted.position] != noted.hash {
            return None;
        }
        *slot = Some(noted.clone());
    }
    Some(by_position)
}

/// The progress a store write records in its own transaction.
#[derive(Debug, Clone)]
pub(crate) struct OutboxNote {
    pub row: i64,
    pub progress: Vec<NotedPiece>,
}

/// Writes `note` inside the caller's transaction.
pub(crate) fn apply_note(c: &rusqlite::Connection, note: &OutboxNote) -> rusqlite::Result<()> {
    // Plain owned data; serializing it cannot fail.
    let progress = serde_json::to_string(&note.progress).expect("progress serializes");
    c.execute("UPDATE ingest_outbox SET progress = ?2 WHERE id = ?1", rusqlite::params![note.row, progress])?;
    Ok(())
}

/// What recording a save found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    /// A new row.
    New { row: i64 },
    /// The agent already has a row under this operation id.
    Existing { row: i64, state: OutboxState, last_error: Option<String> },
}

/// What taking a row found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Claimed {
    /// Ingest it, after the pieces already stored.
    Process { progress: Vec<NotedPiece> },
    /// Gone (finished, forgotten) or failed: nothing to do.
    Skip,
    /// Given up just now.
    GaveUp,
}

/// The outbox as `/health` reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxSummary {
    pub pending: u64,
    pub failed: u64,
    /// Age of the oldest pending row, in seconds.
    pub oldest_pending_s: Option<u64>,
    /// The rows with a cause, most recently changed first.
    pub problems: Vec<OutboxProblem>,
}

/// A row that failed at least once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxProblem {
    pub operation_id: String,
    pub agent_id: String,
    pub state: OutboxState,
    pub attempts: i64,
    pub error: String,
    /// The memories the save inserted before it stopped: what discarding it
    /// deletes. A daemon that predates the field sends none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pieces: Vec<i64>,
}

/// The ids a save inserted, by its progress. One that does not parse names
/// none.
fn inserted_pieces(progress: &str) -> Vec<i64> {
    serde_json::from_str::<Vec<NotedPiece>>(progress)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.inserted)
        .map(|p| p.id)
        .collect()
}

/// A row given up as failed, found by its operation id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FailedRow {
    pub row: i64,
    pub agent_id: String,
    pub progress: Vec<NotedPiece>,
}

impl FailedRow {
    /// The memories the save inserted before it stopped.
    pub fn inserted(&self) -> Vec<i64> {
        self.progress.iter().filter(|p| p.inserted).map(|p| p.id).collect()
    }
}

/// Why a failed save could not be retried or discarded.
#[derive(Debug, thiserror::Error)]
pub enum OutboxActionError {
    #[error("no save waits under operation {0}")]
    NotFound(String),
    #[error("operation {0} is still pending: the daemon is storing it")]
    NotFailed(String),
    #[error("operation {operation_id} belongs to more than one agent ({}); name the agent", agents.join(", "))]
    Ambiguous { operation_id: String, agents: Vec<String> },
    #[error("the saved event of operation {0} does not parse, so it cannot be tried again; discard it instead")]
    Unreadable(String),
    #[error(transparent)]
    Store(#[from] MemoryError),
}

fn backend(context: &'static str) -> impl Fn(tokio_rusqlite::Error) -> MemoryError {
    move |e| match e {
        tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
        other => MemoryError::Backend(format!("{context}: {other}")),
    }
}

/// The event without its text, which the row keeps in its own column.
fn event_json(event: &MemoryEvent) -> String {
    let mut rest = event.clone();
    rest.content = String::new();
    // Plain owned data with string keys; serializing it cannot fail.
    serde_json::to_string(&rest).expect("MemoryEvent serializes")
}

/// Runs `write` with `synchronous = FULL`, then puts the connection's own
/// setting back. An accepted save must outlive a power cut, while the rest of
/// the store keeps NORMAL and skips the fsync per commit.
fn durably<T>(
    c: &rusqlite::Connection,
    write: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    let previous: i64 = c.query_row("PRAGMA synchronous", [], |r| r.get(0))?;
    c.pragma_update(None, "synchronous", "FULL")?;
    let written = write(c);
    let restored = c.pragma_update(None, "synchronous", previous);
    let value = written?;
    restored?;
    Ok(value)
}

fn insert_row(c: &rusqlite::Connection, operation_id: &str, event: &MemoryEvent, now: TimestampMs) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO ingest_outbox (operation_id, agent_id, content, event, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        rusqlite::params![operation_id, event.agent_id, event.content, event_json(event), now],
    )?;
    Ok(c.last_insert_rowid())
}

impl MemoryStore {
    /// Records `event`, whose content is already masked, under `operation_id`,
    /// unless the agent already has a row under it.
    pub(crate) async fn outbox_record(&self, operation_id: String, event: MemoryEvent) -> Result<Recorded, MemoryError> {
        let now = TimestampMs::now();
        self.conn
            .call(move |c| -> rusqlite::Result<Recorded> {
                durably(c, |c| {
                    let tx = c.unchecked_transaction()?;
                    let existing = tx
                        .query_row(
                            "SELECT id, state, last_error FROM ingest_outbox WHERE agent_id = ?1 AND operation_id = ?2",
                            rusqlite::params![event.agent_id, operation_id],
                            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)),
                        )
                        .optional()?;
                    if let Some((row, state, last_error)) = existing {
                        return Ok(Recorded::Existing { row, state: OutboxState::parse(&state), last_error });
                    }
                    let row = insert_row(&tx, &operation_id, &event, now)?;
                    tx.commit()?;
                    Ok(Recorded::New { row })
                })
            })
            .await
            .map_err(backend("outbox_record"))
    }

    /// Records events a previous version spilled to a file, each under a new
    /// operation id, in one transaction. Their content must already be masked.
    pub(crate) async fn outbox_import(&self, events: Vec<MemoryEvent>) -> Result<(), MemoryError> {
        let now = TimestampMs::now();
        self.conn
            .call(move |c| -> rusqlite::Result<()> {
                durably(c, |c| {
                    let tx = c.unchecked_transaction()?;
                    for event in &events {
                        insert_row(&tx, &uuid::Uuid::new_v4().to_string(), event, now)?;
                    }
                    tx.commit()
                })
            })
            .await
            .map_err(backend("outbox_import"))
    }

    /// Takes `row` for ingesting: spends an attempt, or gives the row up when
    /// it has none left.
    pub(crate) async fn outbox_claim(&self, row: i64) -> Result<Claimed, MemoryError> {
        let now = TimestampMs::now();
        self.conn
            .call(move |c| -> rusqlite::Result<Claimed> {
                let tx = c.transaction()?;
                let found = tx
                    .query_row(
                        "SELECT state, attempts, progress FROM ingest_outbox WHERE id = ?1",
                        [row],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)),
                    )
                    .optional()?;
                let Some((state, attempts, progress)) = found else { return Ok(Claimed::Skip) };
                if OutboxState::parse(&state) == OutboxState::Failed {
                    return Ok(Claimed::Skip);
                }
                let claimed = match claim_step(attempts) {
                    ClaimStep::GiveUp => {
                        tx.execute(
                            "UPDATE ingest_outbox SET state = 'failed', last_error = ?2, updated_at = ?3 WHERE id = ?1",
                            rusqlite::params![
                                row,
                                format!("the daemon stopped {attempts} times while ingesting this save"),
                                now
                            ],
                        )?;
                        Claimed::GaveUp
                    }
                    ClaimStep::Process => {
                        tx.execute(
                            "UPDATE ingest_outbox SET attempts = attempts + 1, updated_at = ?2 WHERE id = ?1",
                            rusqlite::params![row, now],
                        )?;
                        // Progress the code wrote itself; one that does not
                        // parse is treated as none, and the dedup gate takes
                        // over the pieces it named.
                        Claimed::Process { progress: serde_json::from_str(&progress).unwrap_or_default() }
                    }
                };
                tx.commit()?;
                Ok(claimed)
            })
            .await
            .map_err(backend("outbox_claim"))
    }

    /// Notes why `row` stopped. See [`after_failure`].
    pub(crate) async fn outbox_fail(&self, row: i64, error: String, transient: bool) -> Result<(), MemoryError> {
        let now = TimestampMs::now();
        // The cause is a library's error text and reaches `/health`; nothing
        // stops one from quoting its input.
        let error = redact(&error).text;
        self.conn
            .call(move |c| -> rusqlite::Result<()> {
                let tx = c.transaction()?;
                let attempts: Option<i64> = tx
                    .query_row("SELECT attempts FROM ingest_outbox WHERE id = ?1", [row], |r| r.get(0))
                    .optional()?;
                let Some(attempts) = attempts else { return Ok(()) };
                let (attempts, state) = after_failure(attempts, transient);
                tx.execute(
                    "UPDATE ingest_outbox SET attempts = ?2, state = ?3, last_error = ?4, updated_at = ?5 WHERE id = ?1",
                    rusqlite::params![row, attempts, state.as_str(), error, now],
                )?;
                tx.commit()
            })
            .await
            .map_err(backend("outbox_fail"))
    }

    /// Hands back the attempt taking `row` spent, keeping its cause: a save a
    /// shutdown paused between pieces did nothing wrong.
    pub(crate) async fn outbox_pause(&self, row: i64) -> Result<(), MemoryError> {
        self.conn
            .call(move |c| {
                c.execute("UPDATE ingest_outbox SET attempts = MAX(attempts - 1, 0) WHERE id = ?1", [row]).map(|_| ())
            })
            .await
            .map_err(backend("outbox_pause"))
    }

    /// The failed row of `operation_id`, of `agent` or of the only agent
    /// holding that id.
    pub(crate) async fn outbox_find_failed(&self, operation_id: &str, agent: Option<&str>) -> Result<FailedRow, OutboxActionError> {
        let (op, agent) = (operation_id.to_string(), agent.map(str::to_string));
        let rows: Vec<(i64, String, String, String)> = self
            .conn
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT id, agent_id, state, progress FROM ingest_outbox \
                     WHERE operation_id = ?1 AND (?2 IS NULL OR agent_id = ?2) ORDER BY agent_id",
                )?;
                let rows = stmt.query_map(rusqlite::params![op, agent], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .map_err(backend("outbox_find_failed"))?;
        match rows.as_slice() {
            [] => Err(OutboxActionError::NotFound(operation_id.to_string())),
            [(row, agent_id, state, progress)] => {
                if OutboxState::parse(state) != OutboxState::Failed {
                    return Err(OutboxActionError::NotFailed(operation_id.to_string()));
                }
                Ok(FailedRow {
                    row: *row,
                    agent_id: agent_id.clone(),
                    progress: serde_json::from_str(progress).unwrap_or_default(),
                })
            }
            many => Err(OutboxActionError::Ambiguous {
                operation_id: operation_id.to_string(),
                agents: many.iter().map(|(_, agent, _, _)| agent.clone()).collect(),
            }),
        }
    }

    /// Writes `row` again as a new pending row with its progress and no
    /// attempt spent, and returns its id and event; `None`, changing nothing,
    /// when the event no longer parses. A new id, because the worker knows the
    /// rows its start-up read found by id, and would take the old one for a
    /// second delivery of itself.
    pub(crate) async fn outbox_requeue(&self, row: i64) -> Result<Option<(i64, MemoryEvent)>, MemoryError> {
        let now = TimestampMs::now();
        self.conn
            .call(move |c| -> rusqlite::Result<Option<(i64, MemoryEvent)>> {
                let tx = c.transaction()?;
                let found: Option<(String, String, String, String, String, TimestampMs)> = tx
                    .query_row(
                        "SELECT operation_id, agent_id, content, event, progress, created_at FROM ingest_outbox WHERE id = ?1",
                        [row],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                    )
                    .optional()?;
                let Some((operation_id, agent_id, content, event, progress, created_at)) = found else { return Ok(None) };
                let Ok(mut parsed) = serde_json::from_str::<MemoryEvent>(&event) else { return Ok(None) };
                parsed.content = content.clone();
                tx.execute("DELETE FROM ingest_outbox WHERE id = ?1", [row])?;
                tx.execute(
                    "INSERT INTO ingest_outbox (operation_id, agent_id, content, event, progress, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![operation_id, agent_id, content, event, progress, created_at, now],
                )?;
                let again = tx.last_insert_rowid();
                tx.commit()?;
                Ok(Some((again, parsed)))
            })
            .await
            .map_err(backend("outbox_requeue"))
    }

    /// Drops `row` once whoever saved it has been told how it ended.
    pub async fn outbox_forget(&self, row: i64) -> Result<(), MemoryError> {
        self.conn
            .call(move |c| c.execute("DELETE FROM ingest_outbox WHERE id = ?1", [row]).map(|_| ()))
            .await
            .map_err(backend("outbox_forget"))
    }

    /// Finishes `row`: links the save's pieces when there are two or more,
    /// applies its revision, marks the pair with the row its warning names
    /// and drops the row, in one transaction.
    pub(crate) async fn outbox_finish(
        &self,
        row: i64,
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
                tx.execute("DELETE FROM ingest_outbox WHERE id = ?1", [row])?;
                tx.commit()?;
                Ok(replaced)
            })
            .await
            .map_err(backend("outbox_finish"))
    }

    /// The pending rows, oldest first, with their events. A row whose event no
    /// longer parses is given up with that cause instead of blocking the rest.
    pub(crate) async fn outbox_pending(&self) -> Result<Vec<(i64, MemoryEvent)>, MemoryError> {
        let now = TimestampMs::now();
        self.conn
            .call(move |c| -> rusqlite::Result<Vec<(i64, MemoryEvent)>> {
                let tx = c.transaction()?;
                let rows: Vec<(i64, String, String)> = {
                    let mut stmt = tx.prepare(
                        "SELECT id, content, event FROM ingest_outbox WHERE state = 'pending' ORDER BY id",
                    )?;
                    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                    rows.collect::<Result<_, _>>()?
                };
                let mut pending = Vec::with_capacity(rows.len());
                for (row, content, event) in rows {
                    match serde_json::from_str::<MemoryEvent>(&event) {
                        Ok(mut event) => {
                            event.content = content;
                            pending.push((row, event));
                        }
                        Err(e) => {
                            tx.execute(
                                "UPDATE ingest_outbox SET state = 'failed', last_error = ?2, updated_at = ?3 WHERE id = ?1",
                                rusqlite::params![row, redact(&format!("the saved event does not parse: {e}")).text, now],
                            )?;
                        }
                    }
                }
                tx.commit()?;
                Ok(pending)
            })
            .await
            .map_err(backend("outbox_pending"))
    }

    /// How many rows wait and failed, and the latest causes.
    pub async fn outbox_summary(&self) -> Result<OutboxSummary, MemoryError> {
        let now = TimestampMs::now();
        self.conn
            .call(move |c| -> rusqlite::Result<OutboxSummary> {
                let (pending, failed, oldest): (i64, i64, Option<TimestampMs>) = c.query_row(
                    "SELECT COALESCE(SUM(state = 'pending'), 0), COALESCE(SUM(state = 'failed'), 0), \
                            MIN(CASE WHEN state = 'pending' THEN created_at END) \
                     FROM ingest_outbox",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
                let mut stmt = c.prepare(
                    "SELECT operation_id, agent_id, state, attempts, last_error, progress FROM ingest_outbox \
                     WHERE last_error IS NOT NULL ORDER BY updated_at DESC, id DESC LIMIT ?1",
                )?;
                let problems = stmt
                    .query_map([PROBLEMS_SHOWN as i64], |r| {
                        Ok(OutboxProblem {
                            operation_id: r.get(0)?,
                            agent_id: r.get(1)?,
                            state: OutboxState::parse(&r.get::<_, String>(2)?),
                            attempts: r.get(3)?,
                            error: r.get(4)?,
                            pieces: inserted_pieces(&r.get::<_, String>(5)?),
                        })
                    })?
                    .collect::<Result<_, _>>()?;
                Ok(OutboxSummary {
                    pending: pending.max(0) as u64,
                    failed: failed.max(0) as u64,
                    oldest_pending_s: oldest
                        .map(|t| (now.as_millis_i64() - t.as_millis_i64()).max(0) as u64 / 1000),
                    problems,
                })
            })
            .await
            .map_err(backend("outbox_summary"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::events::MemorySource;
    use crate::models::chat::Role;

    fn event(agent: &str, content: &str) -> MemoryEvent {
        MemoryEvent {
            agent_id: agent.into(),
            project_id: Some("proj".into()),
            role: Role::User,
            content: content.into(),
            is_error: None,
            source: MemorySource::Explicit,
            timestamp_ms: 42,
            memory_type: Some("decision".into()),
            importance: Some(8.0),
            derived_from: vec![3],
            topic_key: None,
            user_stated: false,
            session_id: None,
        }
    }

    fn noted(position: usize, id: i64) -> NotedPiece {
        NotedPiece { position, hash: format!("h{position}"), id, inserted: true, context: String::new() }
    }

    async fn new_row(store: &MemoryStore, op: &str, content: &str) -> i64 {
        match store.outbox_record(op.into(), event("a", content)).await.unwrap() {
            Recorded::New { row } => row,
            other => panic!("expected a new row, got {other:?}"),
        }
    }

    async fn row_state(store: &MemoryStore, row: i64) -> Option<(String, i64, Option<String>, String)> {
        store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT state, attempts, last_error, progress FROM ingest_outbox WHERE id = ?1",
                    [row],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()
            })
            .await
            .unwrap()
    }

    fn synchronous(c: &rusqlite::Connection) -> i64 {
        c.query_row("PRAGMA synchronous", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn durably_writes_under_full_and_puts_the_setting_back() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.pragma_update(None, "synchronous", "NORMAL").unwrap();

        let inside = durably(&c, |c| Ok(synchronous(c))).unwrap();
        assert_eq!(inside, 2, "FULL while the row is written");
        assert_eq!(synchronous(&c), 1);

        let failed: rusqlite::Result<()> = durably(&c, |_| Err(rusqlite::Error::InvalidQuery));
        assert!(failed.is_err());
        assert_eq!(synchronous(&c), 1, "put back after a failure too");
    }

    #[tokio::test]
    async fn recording_a_row_leaves_the_store_on_normal() {
        let dir = crate::test_dir::test_dir();
        let store = MemoryStore::open(dir.path(), crate::memory::store_lock::StoreHolder::Eval).await.unwrap();
        new_row(&store, "op-1", "a save").await;
        store.outbox_import(vec![event("a", "an old one")]).await.unwrap();
        assert_eq!(store.conn.call(|c| Ok::<_, rusqlite::Error>(synchronous(c))).await.unwrap(), 1);
    }

    #[test]
    fn claim_step_processes_until_the_attempts_run_out() {
        assert_eq!(claim_step(0), ClaimStep::Process);
        assert_eq!(claim_step(MAX_ATTEMPTS - 1), ClaimStep::Process);
        assert_eq!(claim_step(MAX_ATTEMPTS), ClaimStep::GiveUp);
        assert_eq!(MAX_ATTEMPTS, 3);
    }

    #[test]
    fn after_failure_fails_a_row_only_at_the_cap_and_hands_back_a_transient_attempt() {
        assert_eq!(after_failure(1, false), (1, OutboxState::Pending));
        assert_eq!(after_failure(MAX_ATTEMPTS, false), (MAX_ATTEMPTS, OutboxState::Failed));
        assert_eq!(after_failure(MAX_ATTEMPTS, true), (MAX_ATTEMPTS - 1, OutboxState::Pending));
        assert_eq!(after_failure(0, true), (0, OutboxState::Pending));
    }

    #[test]
    fn resume_positions_places_each_noted_piece_and_refuses_one_that_does_not_fit() {
        let hashes = ["h0", "h1", "h2"];
        assert_eq!(resume_positions(&[], &hashes[..2]), Some(vec![None, None]));
        assert_eq!(
            resume_positions(&[noted(1, 11), noted(0, 10)], &hashes),
            Some(vec![Some(noted(0, 10)), Some(noted(1, 11)), None])
        );
        assert_eq!(resume_positions(&[noted(3, 13)], &hashes), None, "the split is shorter now");
        assert_eq!(resume_positions(&[noted(0, 10), noted(0, 12)], &hashes), None, "one position twice");
        assert_eq!(resume_positions(&[noted(1, 11)], &["h0", "other", "h2"]), None, "another text there now");
    }

    #[tokio::test]
    async fn a_recorded_event_comes_back_whole_and_the_row_keeps_the_text_apart() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let row = new_row(&store, "op-1", "first line\nsecond line").await;

        let pending = store.outbox_pending().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, row);
        let back = serde_json::to_value(&pending[0].1).unwrap();
        assert_eq!(back, serde_json::to_value(event("a", "first line\nsecond line")).unwrap());
        let event_column: String = store
            .conn
            .call(move |c| c.query_row("SELECT event FROM ingest_outbox WHERE id = ?1", [row], |r| r.get(0)))
            .await
            .unwrap();
        assert!(!event_column.contains("second line"), "{event_column}");
    }

    #[tokio::test]
    async fn an_operation_id_is_one_row_per_agent() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let row = new_row(&store, "op-1", "x").await;

        assert_eq!(
            store.outbox_record("op-1".into(), event("a", "x again")).await.unwrap(),
            Recorded::Existing { row, state: OutboxState::Pending, last_error: None }
        );
        assert!(matches!(
            store.outbox_record("op-1".into(), event("b", "x")).await.unwrap(),
            Recorded::New { .. }
        ));
    }

    #[tokio::test]
    async fn pending_rows_come_back_oldest_first_and_failed_ones_do_not() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = new_row(&store, "op-a", "a").await;
        let b = new_row(&store, "op-b", "b").await;
        let c = new_row(&store, "op-c", "c").await;
        store.outbox_claim(b).await.unwrap();
        store.outbox_fail(b, "broken".into(), false).await.unwrap();
        for _ in 0..MAX_ATTEMPTS {
            store.outbox_claim(c).await.unwrap();
        }
        store.outbox_fail(c, "broken".into(), false).await.unwrap();

        let rows: Vec<i64> = store.outbox_pending().await.unwrap().into_iter().map(|(r, _)| r).collect();
        assert_eq!(rows, vec![a, b]);
    }

    #[tokio::test]
    async fn claiming_spends_an_attempt_and_gives_up_once_none_is_left() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let row = new_row(&store, "op-1", "x").await;

        for spent in 1..=MAX_ATTEMPTS {
            assert_eq!(store.outbox_claim(row).await.unwrap(), Claimed::Process { progress: vec![] });
            assert_eq!(row_state(&store, row).await.unwrap().1, spent);
        }
        assert_eq!(store.outbox_claim(row).await.unwrap(), Claimed::GaveUp);
        let (state, _, error, _) = row_state(&store, row).await.unwrap();
        assert_eq!(state, "failed");
        assert_eq!(error.as_deref(), Some("the daemon stopped 3 times while ingesting this save"));
        assert_eq!(store.outbox_claim(row).await.unwrap(), Claimed::Skip);
        assert_eq!(store.outbox_claim(row + 100).await.unwrap(), Claimed::Skip);
    }

    #[tokio::test]
    async fn a_note_inside_a_transaction_is_the_progress_the_next_claim_reads() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let row = new_row(&store, "op-1", "x").await;
        store.outbox_claim(row).await.unwrap();
        let note = OutboxNote { row, progress: vec![noted(0, 10)] };
        store
            .conn
            .call(move |c| {
                let tx = c.transaction()?;
                apply_note(&tx, &note)?;
                tx.commit()
            })
            .await
            .unwrap();

        assert_eq!(store.outbox_claim(row).await.unwrap(), Claimed::Process { progress: vec![noted(0, 10)] });
    }

    #[tokio::test]
    async fn a_failure_keeps_its_cause_and_a_transient_one_its_attempt() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let row = new_row(&store, "op-1", "x").await;
        store.outbox_claim(row).await.unwrap();
        store.outbox_fail(row, "the embedder is not available".into(), true).await.unwrap();
        assert_eq!(
            row_state(&store, row).await.unwrap(),
            ("pending".into(), 0, Some("the embedder is not available".into()), "[]".into())
        );

        store.outbox_claim(row).await.unwrap();
        store.outbox_fail(row, "embed failed: NaN".into(), false).await.unwrap();
        assert_eq!(row_state(&store, row).await.unwrap().1, 1);
    }

    #[tokio::test]
    async fn a_pause_hands_back_its_attempt() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let row = new_row(&store, "op-1", "x").await;
        store.outbox_claim(row).await.unwrap();
        store.outbox_fail(row, "embed failed: NaN".into(), false).await.unwrap();

        for _ in 0..MAX_ATTEMPTS {
            assert!(matches!(store.outbox_claim(row).await.unwrap(), Claimed::Process { .. }));
            store.outbox_pause(row).await.unwrap();
        }

        assert_eq!(
            row_state(&store, row).await.unwrap(),
            ("pending".into(), 1, Some("embed failed: NaN".into()), "[]".into()),
            "a pause keeps the cause of the last failure"
        );
        assert!(matches!(store.outbox_claim(row).await.unwrap(), Claimed::Process { .. }));
    }

    #[tokio::test]
    async fn finishing_links_the_pieces_and_drops_the_row_together() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let row = new_row(&store, "op-1", "x").await;
        let single = new_row(&store, "op-2", "y").await;

        store
            .outbox_finish(
                row,
                vec![
                    StoredPiece { memory_id: 10, context: String::new() },
                    StoredPiece { memory_id: 11, context: "head".into() },
                ],
                None,
                None,
            )
            .await
            .unwrap();
        store
            .outbox_finish(single, vec![StoredPiece { memory_id: 12, context: String::new() }], None, None)
            .await
            .unwrap();

        assert_eq!(row_state(&store, row).await, None);
        assert_eq!(row_state(&store, single).await, None);
        let links: Vec<(i64, i64, String)> = store
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT piece, memory_id, context FROM memory_piece ORDER BY piece")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect();
                rows
            })
            .await
            .unwrap();
        assert_eq!(links, vec![(1, 10, String::new()), (2, 11, "head".into())]);
    }

    #[tokio::test]
    async fn forgetting_a_row_drops_it() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let row = new_row(&store, "op-1", "x").await;
        store.outbox_forget(row).await.unwrap();
        assert_eq!(row_state(&store, row).await, None);
    }

    #[tokio::test]
    async fn imported_events_get_rows_of_their_own() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store.outbox_import(vec![event("a", "one"), event("a", "two")]).await.unwrap();

        let contents: Vec<String> =
            store.outbox_pending().await.unwrap().into_iter().map(|(_, e)| e.content).collect();
        assert_eq!(contents, vec!["one", "two"]);
    }

    #[tokio::test]
    async fn a_failure_is_stored_masked() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let token: String = "a1B2".chars().cycle().take(36).collect();
        let row = new_row(&store, "op-1", "x").await;
        store.outbox_claim(row).await.unwrap();

        store.outbox_fail(row, format!("embed failed: token ghp_{token}"), false).await.unwrap();

        let error = row_state(&store, row).await.unwrap().2;
        assert_eq!(error.as_deref(), Some("embed failed: token [REDACTED:github_token]"));

        let row = new_row(&store, "op-2", "x").await;
        let event = format!("{{\"importance\": \"ghp_{token}\"}}");
        store
            .conn
            .call(move |c| c.execute("UPDATE ingest_outbox SET event = ?2 WHERE id = ?1", rusqlite::params![row, event]))
            .await
            .unwrap();
        store.outbox_pending().await.unwrap();
        let error = row_state(&store, row).await.unwrap().2.unwrap();
        assert!(error.starts_with("the saved event does not parse"), "{error}");
        assert!(!error.contains(&token), "{error}");
    }

    #[tokio::test]
    async fn an_event_that_does_not_parse_is_given_up_with_its_cause() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let row = new_row(&store, "op-1", "x").await;
        store
            .conn
            .call(move |c| c.execute("UPDATE ingest_outbox SET event = '{\"trunc' WHERE id = ?1", [row]))
            .await
            .unwrap();

        assert!(store.outbox_pending().await.unwrap().is_empty());
        let (state, _, error, _) = row_state(&store, row).await.unwrap();
        assert_eq!(state, "failed");
        assert!(error.unwrap().starts_with("the saved event does not parse"));
    }

    async fn fail_for_good(store: &MemoryStore, row: i64, progress: Vec<NotedPiece>) {
        let progress = serde_json::to_string(&progress).unwrap();
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE ingest_outbox SET state = 'failed', attempts = 3, last_error = 'embed failed: NaN', \
                     progress = ?2 WHERE id = ?1",
                    rusqlite::params![row, progress],
                )
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn retrying_a_failed_row_writes_it_again_under_a_new_id() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let row = new_row(&store, "op-1", "first line\nsecond line").await;
        fail_for_good(&store, row, vec![noted(0, 10)]).await;

        let failed = store.outbox_find_failed("op-1", None).await.unwrap();
        assert_eq!((failed.row, failed.agent_id.as_str(), failed.progress.clone()), (row, "a", vec![noted(0, 10)]));
        let (again, event) = store.outbox_requeue(row).await.unwrap().expect("the event parses");

        assert!(again > row, "a new id: {again}");
        assert_eq!(row_state(&store, row).await, None);
        assert_eq!(
            row_state(&store, again).await.unwrap(),
            ("pending".into(), 0, None, serde_json::to_string(&vec![noted(0, 10)]).unwrap())
        );
        assert_eq!(event.content, "first line\nsecond line");
        assert_eq!(store.outbox_pending().await.unwrap().into_iter().map(|(r, _)| r).collect::<Vec<_>>(), vec![again]);
        assert_eq!(
            store.outbox_record("op-1".into(), event).await.unwrap(),
            Recorded::Existing { row: again, state: OutboxState::Pending, last_error: None },
            "the operation id stays the save's"
        );
    }

    #[tokio::test]
    async fn only_a_failed_row_is_retried_or_discarded() {
        let store = MemoryStore::new_for_test().await.unwrap();
        new_row(&store, "op-1", "x").await;

        assert!(matches!(store.outbox_find_failed("op-1", None).await, Err(OutboxActionError::NotFailed(_))));
        assert!(matches!(store.outbox_find_failed("op-9", None).await, Err(OutboxActionError::NotFound(_))));
        assert!(matches!(store.outbox_find_failed("op-1", Some("b")).await, Err(OutboxActionError::NotFound(_))));
    }

    #[tokio::test]
    async fn an_operation_id_two_agents_hold_is_ambiguous() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = new_row(&store, "op-1", "x").await;
        let Recorded::New { row: b } = store.outbox_record("op-1".into(), event("b", "y")).await.unwrap() else {
            panic!("a new row for another agent")
        };
        fail_for_good(&store, a, vec![]).await;
        fail_for_good(&store, b, vec![]).await;

        let err = store.outbox_find_failed("op-1", None).await.unwrap_err();
        assert!(matches!(&err, OutboxActionError::Ambiguous { agents, .. } if agents == &["a", "b"]), "{err:?}");
        assert_eq!(store.outbox_find_failed("op-1", Some("b")).await.unwrap().row, b);
    }

    #[tokio::test]
    async fn the_summary_counts_rows_and_lists_the_latest_causes() {
        let store = MemoryStore::new_for_test().await.unwrap();
        assert_eq!(store.outbox_summary().await.unwrap(), OutboxSummary::default());

        new_row(&store, "op-ok", "waiting").await;
        let broken = new_row(&store, "op-broken", "broken").await;
        store.outbox_claim(broken).await.unwrap();
        store.outbox_fail(broken, "embed failed: NaN".into(), false).await.unwrap();

        let summary = store.outbox_summary().await.unwrap();
        assert_eq!((summary.pending, summary.failed), (2, 0));
        assert!(summary.oldest_pending_s.is_some());
        assert_eq!(
            summary.problems,
            vec![OutboxProblem {
                operation_id: "op-broken".into(),
                agent_id: "a".into(),
                state: OutboxState::Pending,
                attempts: 1,
                error: "embed failed: NaN".into(),
                pieces: vec![],
            }]
        );

        let reinforced = NotedPiece { inserted: false, ..noted(1, 11) };
        fail_for_good(&store, broken, vec![noted(0, 10), reinforced, noted(2, 12)]).await;
        let summary = store.outbox_summary().await.unwrap();
        assert_eq!(summary.problems[0].pieces, vec![10, 12], "the pieces the save inserted");
    }
}
