//! Memory ingest worker.
//!
//! Spawned once per process (see `MemoryRuntime::ingest_worker`). Drains
//! `MemoryEvent`s from the channel, selects the appropriate chunker, embeds
//! each chunk, and inserts both `memory` and `memory_vectors` rows atomically.
//! Failure isolation: errors are logged and the worker NEVER panics.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use once_cell::sync::OnceCell;
use tokio::sync::mpsc::Receiver;
use tokio::sync::{oneshot, watch};
use tokio::time::Instant;

use crate::memory::chunker::{
    embedding_text, select_chunker, ChatChunker, Chunk, ChunkerKind, SaveSplit,
};
use crate::memory::outbox::{resume_positions, Claimed, NotedPiece, OutboxNote};
use crate::memory::contradiction::{Candidate, SaveKey};
use crate::memory::pieces::StoredPiece;
use crate::memory::embedder::{EmbedError, EmbedRole, Embedder};
use crate::memory::events::{MemoryEvent, MemorySource};
use crate::memory::redact::{redact, SecretKind};
use crate::memory::sharing::Reader;
use crate::memory::store_insert::{NewMemoryRecord, Stored};
use crate::memory::topic::{inherit, unkeyed_revision, Replaced, Revision, TopicScope};
use crate::memory::MemoryStore;

/// What happened to one chunk that reached the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkOutcome {
    /// A new row was written.
    Inserted,
    /// The dedup gate absorbed the chunk into an existing row.
    Reinforced,
}

/// One chunk of an event that is in the store, with the row that holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedChunk {
    pub id: i64,
    pub outcome: ChunkOutcome,
}

/// Why an event stopped before all of its chunks reached the store. Every
/// cause is recoverable: the worker loop never exits on one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IngestFailure {
    #[error("the embedder is not available")]
    EmbedderUnavailable,
    #[error("{0}")]
    Failed(String),
}

/// One event on its way to the worker, with where to send its report.
#[derive(Debug)]
pub struct IngestRequest {
    pub event: MemoryEvent,
    /// `None` for a producer that does not wait for the result. A producer
    /// that stopped waiting leaves the receiver closed, and the report is
    /// dropped.
    pub reply: Option<oneshot::Sender<IngestReport>>,
    /// The `ingest_outbox` row that holds this event until it is stored.
    /// `None` for an event nothing recorded (the in-process paths of tests
    /// and evaluation), which is ingested exactly as before the outbox.
    pub outbox: Option<i64>,
}

impl From<MemoryEvent> for IngestRequest {
    fn from(event: MemoryEvent) -> Self {
        Self { event, reply: None, outbox: None }
    }
}

/// What one event wrote, in chunk order, and the failure that stopped it.
///
/// Both halves at once rather than a `Result`: a failure part-way through a
/// multi-chunk event leaves earlier chunks written, and the caller needs
/// their ids to clean up or retry only the rest.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IngestReport {
    pub memories: Vec<SavedChunk>,
    pub failure: Option<IngestFailure>,
    /// The kinds of secret masked out of the event before it was chunked.
    pub redacted: Vec<SecretKind>,
    /// The earlier rows the detector paired with this save, one per row.
    pub related: Vec<Candidate>,
    /// The rows of the save's topic key it superseded.
    pub replaced: Vec<Replaced>,
    /// For a save without a topic key, the earlier row of its own agent and
    /// project it most likely revises ([`unkeyed_revision`]).
    pub revision_candidate: Option<i64>,
    /// A shutdown stopped the save between two pieces; its outbox row notes
    /// the ones stored, and the next start resumes after them.
    pub paused: bool,
}

/// Backlog size at which the ingest channel is worth reporting. Events are
/// one per chat message or tool result, and draining one costs an embed batch
/// plus a KNN probe plus an insert — tens of milliseconds — so 64 queued
/// events is seconds of backlog: past any normal burst, far above the handful
/// a single agent turn produces. A starting value chosen for that ratio, not
/// a measured optimum; changing it should be deliberate, which is why the
/// number is pinned by a test.
const INGEST_QUEUE_WARN_DEPTH: usize = 64;

/// How often a worker held for the embedder checks whether it has loaded.
const EMBEDDER_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// The longest a save the embedder could not take waits for its next try:
/// `IdleEmbedder` recovers a dead inference worker at most once a minute, so
/// trying more often only finds it not ready again.
const MAX_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(60);

/// How long a save waits before its `retries`-th try after a transient
/// failure: one second, doubling, at most [`MAX_RETRY_DELAY`].
fn retry_delay(retries: u32) -> std::time::Duration {
    std::time::Duration::from_secs(1u64 << retries.min(6)).min(MAX_RETRY_DELAY)
}

/// Which way the backlog just crossed [`INGEST_QUEUE_WARN_DEPTH`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueueDepthEdge {
    Rising,
    Falling,
}

/// Reports only crossings of the threshold, never the standing state.
/// `None` is the common case. A level check would fire once per drained
/// event, so a sustained backlog of 200 would emit 200 identical lines and
/// bury the transition that actually carries information.
fn queue_depth_transition(depth: usize, was_over: bool) -> Option<QueueDepthEdge> {
    match (was_over, depth >= INGEST_QUEUE_WARN_DEPTH) {
        (false, true) => Some(QueueDepthEdge::Rising),
        (true, false) => Some(QueueDepthEdge::Falling),
        _ => None,
    }
}

/// One chunk that survived intra-event collapse, with how many identical
/// copies the event carried.
struct PendingChunk {
    chunk: Chunk,
    content_hash: String,
    /// Copies beyond the first are replayed through the dedup gate rather
    /// than embedded again: on the path where the one surviving chunk
    /// inserts successfully, repetition inside one event reinforces the
    /// same row a repetition across two events would — same counter, same
    /// log branch. That equivalence stops at an insert failure: today a
    /// failed insert of chunk A still lets an identical chunk B later in the
    /// same event embed and insert its own row, but once A and B collapse
    /// into one `PendingChunk`, there is no separate B left to try — A's
    /// insert failure now costs every copy this multiplicity stands for.
    multiplicity: u32,
}

/// Collapses chunks with identical content into one entry each, preserving
/// first-occurrence order.
///
/// Insertion order is load-bearing twice: the embed batch is zipped back
/// against this list positionally, and contradiction detection for a chunk
/// must see the chunks before it already inserted. A `HashMap` alone would
/// keep the counts and lose the order, so the map only indexes into the
/// ordered vector.
fn collapse_duplicate_chunks(chunks: Vec<Chunk>) -> Vec<PendingChunk> {
    let mut out: Vec<PendingChunk> = Vec::with_capacity(chunks.len());
    let mut by_hash: HashMap<String, usize> = HashMap::with_capacity(chunks.len());
    for chunk in chunks {
        let content_hash = crate::memory::dedup::content_hash(&chunk.text);
        match by_hash.get(&content_hash) {
            Some(&idx) => out[idx].multiplicity += 1,
            None => {
                by_hash.insert(content_hash.clone(), out.len());
                out.push(PendingChunk { chunk, content_hash, multiplicity: 1 });
            }
        }
    }
    out
}

/// Outbox rows to ingest on this start, ahead of anything new on the channel.
pub struct Replay {
    requests: VecDeque<IngestRequest>,
}

impl Replay {
    /// Nothing to replay.
    pub fn none() -> Self {
        Self { requests: VecDeque::new() }
    }

    /// The pending outbox rows, in the order given.
    pub(crate) fn rows(rows: Vec<(i64, MemoryEvent)>) -> Self {
        Self {
            requests: rows
                .into_iter()
                .map(|(row, event)| IngestRequest { event, reply: None, outbox: Some(row) })
                .collect(),
        }
    }

    fn next(&mut self) -> Option<IngestRequest> {
        self.requests.pop_front()
    }

    fn into_remaining(self) -> VecDeque<IngestRequest> {
        self.requests
    }
}

/// The outbox rows the start-up read found: the only ones that can reach the
/// worker twice. A save accepted before the read is both in it and on the
/// channel; one accepted after arrives once. The replay runs before anything
/// from the channel, so the copy arrives after its row was taken, and the set
/// never grows past what the read found.
struct StartupRows(HashSet<i64>);

impl StartupRows {
    fn of(replay: &Replay) -> Self {
        Self(replay.requests.iter().filter_map(|r| r.outbox).collect())
    }

    fn is_second_delivery(&self, row: i64, from_replay: bool) -> bool {
        !from_replay && self.0.contains(&row)
    }
}

/// What the worker does with the next event it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrainStep {
    Ingest,
    Spill,
}

/// `deadline` is `None` until a shutdown asks for a drain. A draining worker
/// spills once the deadline passes, and at once without an embedder: the model
/// may still be downloading, and waiting for it would outlast any deadline.
fn drain_step(now: Instant, deadline: Option<Instant>, embedder_ready: bool) -> DrainStep {
    match deadline {
        None => DrainStep::Ingest,
        Some(_) if !embedder_ready => DrainStep::Spill,
        Some(d) if now >= d => DrainStep::Spill,
        Some(_) => DrainStep::Ingest,
    }
}

/// Whether a save stops before its next piece: a drain asked for one by an
/// instant that has passed.
fn pause_due(now: Instant, deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|d| now >= d)
}

/// When a shutdown wants the queue drained by, read again before each piece:
/// the drain can start after the event was taken.
#[derive(Clone, Copy, Default)]
struct DrainWatch<'a> {
    deadline: Option<Instant>,
    signal: Option<&'a watch::Receiver<Option<Instant>>>,
}

impl DrainWatch<'_> {
    fn deadline(&self) -> Option<Instant> {
        self.deadline.or_else(|| self.signal.and_then(|signal| *signal.borrow()))
    }
}

/// Top-level worker loop. One per app instance.
///
/// Drains events FIFO. An event's surviving chunks are embedded in one call,
/// so any embed failure loses that whole event rather than the failing chunk
/// alone: the batch API returns a single result with no per-item outcome to
/// salvage, and the failures it reports (model session, tokenizer) apply to
/// every text in the call anyway. An insert failure stops the event there, so
/// what is stored is always a prefix of the text, never one with a hole.
///
/// `replay` holds the events a previous shutdown spilled. They go first, in
/// file order, before anything from the channel.
///
/// A save with an outbox row that the embedder could not take is answered
/// with that failure, its attempt handed back, and tried again after
/// [`retry_delay`], until it is stored or fails for another reason. Nothing
/// behind it is taken meanwhile: two saves under one topic key must be applied
/// in the order they were accepted, and while the embedder is away the ones
/// behind would fail the same way.
///
/// `drain` carries the instant a shutdown wants the queue drained by. Once it
/// is set the channel is closed to new events, what is already queued keeps
/// being ingested until [`drain_step`] says to stop, and the events not yet
/// started (unreplayed ones first) are returned in order for the caller to
/// spill. A save with an outbox row stops before its next piece once the
/// deadline passes, after the first piece it stores, its attempt handed back:
/// the row notes the pieces stored, and the next start resumes after them. An event without a row always
/// finishes, since replaying it would duplicate the chunks that did land. A
/// save waiting for its retry is left in its row. A dropped `drain` sender
/// never asks for a drain.
/// A spilled request's reply is dropped unanswered, so its caller reports the
/// save as unconfirmed rather than failed.
///
/// Also returns, with nothing spilled, when all `Sender`s are dropped and the
/// queue is empty; events held for an embedder that never loaded are returned
/// then too.
pub async fn memory_worker(
    mut rx: Receiver<IngestRequest>,
    drain: watch::Receiver<Option<Instant>>,
    mut replay: Replay,
    store: Arc<MemoryStore>,
    embedder_cell: Arc<OnceCell<Arc<dyn Embedder>>>,
    chat_chunker: Arc<ChatChunker>,
) -> Vec<MemoryEvent> {
    // Running count of chunks absorbed by the write-border dedup gate.
    // Plain `u64`, no atomic: `memory_worker` is the single sequential
    // consumer of this channel — see `MemoryStore::reinforce_if_duplicate`'s
    // doc comment in `store_insert.rs`, which states that guarantee and its
    // escalation path if the worker is ever parallelized — so nothing else
    // can race an increment.
    let mut absorbed_duplicates: u64 = 0;
    // Tracks which side of INGEST_QUEUE_WARN_DEPTH the backlog was on last
    // time through the loop, so queue_depth_transition only reports the edge
    // crossing rather than the level on every iteration.
    let mut queue_over_depth = false;
    let mut drain = Some(drain);
    let mut deadline: Option<Instant> = None;
    // A second delivery is skipped, and its caller, who may be waiting, gets
    // the report of the first.
    let startup = StartupRows::of(&replay);
    let mut replayed: HashMap<i64, IngestReport> = HashMap::new();
    loop {
        observe_drain(&mut rx, &mut drain, &mut deadline);
        let (IngestRequest { event: evt, reply, outbox }, from_replay) = match replay.next() {
            Some(request) => (request, true),
            None => match next_event(&mut rx, &mut drain, &mut deadline).await {
                Some(request) => (request, false),
                None => break,
            },
        };
        // Read after recv: this is the backlog still waiting, which is what
        // "falling behind" means.
        let depth = rx.len();
        match queue_depth_transition(depth, queue_over_depth) {
            Some(QueueDepthEdge::Rising) => {
                queue_over_depth = true;
                log::warn!("ingest queue depth={depth}; consumer is falling behind producers");
            }
            Some(QueueDepthEdge::Falling) => {
                queue_over_depth = false;
                log::info!("ingest queue depth={depth}; backlog drained");
            }
            None => {}
        }
        hold_for_embedder(&embedder_cell, &mut rx, &mut drain, &mut deadline).await;
        if drain_step(Instant::now(), deadline, embedder_cell.get().is_some()) == DrainStep::Spill {
            return unstarted(Some(IngestRequest { event: evt, reply, outbox }), replay, &mut rx);
        }
        if let Some(row) = outbox.filter(|row| startup.is_second_delivery(*row, from_replay)) {
            log::debug!("outbox row {row} arrived twice; ingested once");
            // A dropped reply reads as unconfirmed.
            if let (Some(reply), Some(report)) = (reply, replayed.remove(&row)) {
                let _ = reply.send(report);
            }
            continue;
        }
        let mut reply = reply;
        let mut retries = 0;
        loop {
            let track = match outbox {
                None => None,
                Some(row) => match take_row(&store, row).await {
                    Some(progress) => Some(Track { row, progress }),
                    None => break,
                },
            };
            let watch = DrainWatch { deadline, signal: drain.as_ref() };
            let report = process_tracked(
                &evt,
                &store,
                &embedder_cell,
                &chat_chunker,
                &mut absorbed_duplicates,
                track,
                watch,
            )
            .await;
            if let Some(row) = outbox.filter(|_| report.paused) {
                if let Err(e) = store.outbox_pause(row).await {
                    log::warn!("could not hand back the attempt of paused outbox row {row}: {e}");
                }
                log::info!("shutdown paused outbox row {row} after {} stored pieces", report.memories.len());
                // The reply goes unanswered, which its caller reads as unconfirmed.
                return unstarted(None, replay, &mut rx);
            }
            let transient = report.failure == Some(IngestFailure::EmbedderUnavailable);
            if let (Some(row), Some(failure)) = (outbox, &report.failure) {
                if let Err(e) = store.outbox_fail(row, failure.to_string(), transient).await {
                    log::warn!("could not note why outbox row {row} stopped: {e}");
                }
            }
            if let Some(e) = &report.failure {
                log::warn!(
                    "ingest event stopped after {} stored chunks (agent={}, role={:?}): {e}",
                    report.memories.len(),
                    evt.agent_id,
                    evt.role
                );
            }
            if !report.redacted.is_empty() && retries == 0 {
                let kinds: Vec<&str> = report.redacted.iter().map(|k| k.as_str()).collect();
                log::info!("ingest masked {} (agent={})", kinds.join(", "), evt.agent_id);
            }
            match (reply.take(), outbox) {
                (Some(reply), _) => {
                    let _ = reply.send(report);
                }
                (None, Some(row)) if from_replay && retries == 0 => {
                    replayed.insert(row, report);
                }
                _ => {}
            }
            let Some(row) = outbox.filter(|_| transient) else { break };
            let delay = retry_delay(retries);
            log::info!("outbox row {row} is tried again in {}s, once the embedder can take it", delay.as_secs());
            if !wait_to_retry(delay, &mut rx, &mut drain, &mut deadline).await {
                return unstarted(None, replay, &mut rx);
            }
            retries += 1;
        }
    }
    log::info!("memory_worker shutting down — channel closed");
    Vec::new()
}

/// Starts the drain the first time `drain` carries a deadline: from then on
/// the channel accepts nothing new, and `recv` yields what is already queued.
fn observe_drain(
    rx: &mut Receiver<IngestRequest>,
    drain: &mut Option<watch::Receiver<Option<Instant>>>,
    deadline: &mut Option<Instant>,
) {
    if deadline.is_some() {
        return;
    }
    if let Some(requested) = drain.as_mut().and_then(|d| *d.borrow_and_update()) {
        *deadline = Some(requested);
        rx.close();
    }
}

/// The next queued event, or `None` once the channel is closed and empty.
async fn next_event(
    rx: &mut Receiver<IngestRequest>,
    drain: &mut Option<watch::Receiver<Option<Instant>>>,
    deadline: &mut Option<Instant>,
) -> Option<IngestRequest> {
    loop {
        observe_drain(rx, drain, deadline);
        let signal = match drain.as_mut() {
            Some(signal) if deadline.is_none() => signal,
            _ => return rx.recv().await,
        };
        let sender_gone = tokio::select! {
            evt = rx.recv() => return evt,
            changed = signal.changed() => changed.is_err(),
        };
        if sender_gone {
            *drain = None;
        }
    }
}

/// Holds the worker until the embedder is available, so an event accepted
/// while the model is still loading is ingested afterwards instead of lost.
///
/// The queue keeps its FIFO order because the worker simply stops draining:
/// everything behind the held event waits with it. `OnceCell` has no async
/// wait, so this polls; the model takes seconds to load, which makes a
/// 100 ms tick free next to it and keeps the cell's shape untouched.
///
/// Gives up when a drain starts, or when every sender is gone and nothing can
/// ask for one; the latter counts as a drain due now. Either way the held
/// event is spilled rather than waited on, which is what keeps a model that
/// never loads from blocking the daemon's shutdown.
async fn hold_for_embedder(
    embedder_cell: &Arc<OnceCell<Arc<dyn Embedder>>>,
    rx: &mut Receiver<IngestRequest>,
    drain: &mut Option<watch::Receiver<Option<Instant>>>,
    deadline: &mut Option<Instant>,
) {
    if embedder_cell.get().is_some() {
        return;
    }
    log::info!("ingest is waiting for the embedder to load");
    while embedder_cell.get().is_none() {
        observe_drain(rx, drain, deadline);
        if deadline.is_some() {
            return;
        }
        if rx.is_closed() {
            *deadline = Some(Instant::now());
            return;
        }
        tokio::time::sleep(EMBEDDER_POLL_INTERVAL).await;
    }
}

/// Waits `delay` before a save is tried again. `false` when a drain started
/// meanwhile, or every sender is gone and nothing can ask for one: the save
/// then stays in its row for the next start.
async fn wait_to_retry(
    delay: std::time::Duration,
    rx: &mut Receiver<IngestRequest>,
    drain: &mut Option<watch::Receiver<Option<Instant>>>,
    deadline: &mut Option<Instant>,
) -> bool {
    let until = Instant::now() + delay;
    loop {
        observe_drain(rx, drain, deadline);
        if deadline.is_some() || rx.is_closed() {
            return false;
        }
        let Some(signal) = drain.as_mut() else {
            tokio::time::sleep_until(until).await;
            return true;
        };
        tokio::select! {
            _ = tokio::time::sleep_until(until) => return true,
            changed = signal.changed() => if changed.is_err() {
                *drain = None;
            },
        }
    }
}

/// Takes outbox `row` for this run: its progress, or `None` when there is
/// nothing to ingest (taken already in this run, finished, failed, given up
/// just now, or unreadable, which leaves it for the next start).
async fn take_row(store: &Arc<MemoryStore>, row: i64) -> Option<Vec<NotedPiece>> {
    match store.outbox_claim(row).await {
        Ok(Claimed::Process { progress }) => Some(progress),
        Ok(Claimed::Skip) => None,
        Ok(Claimed::GaveUp) => {
            log::warn!("gave up outbox row {row}: the daemon stopped every time it ingested it");
            None
        }
        Err(e) => {
            log::warn!("could not take outbox row {row}: {e}; it is ingested on the next start");
            None
        }
    }
}

/// The events nothing recorded among `first`, the requests left to replay and
/// everything still queued, in order. A recorded one stays in the outbox for
/// the next start. Closes the channel first so nothing can slip in behind the
/// last event read.
fn unstarted(
    first: Option<IngestRequest>,
    replay: Replay,
    rx: &mut Receiver<IngestRequest>,
) -> Vec<MemoryEvent> {
    rx.close();
    let mut requests: Vec<IngestRequest> = first.into_iter().collect();
    requests.extend(replay.into_remaining());
    while let Ok(next) = rx.try_recv() {
        requests.push(next);
    }
    log::info!("ingest stopped with {} events not yet ingested", requests.len());
    requests.into_iter().filter(|r| r.outbox.is_none()).map(|r| r.event).collect()
}

/// Runs the write-border dedup gate for one chunk hash. `Some(id)` means the
/// chunk was absorbed into row `id` and must not be inserted.
///
/// A repeated chunk reinforces the existing memory instead of inserting
/// another row: `access_count += 1` promotes the tier through
/// `compute_tier`, so repeating something makes it more retrievable. Chosen
/// over replacing or rejecting the duplicate because in a memory system
/// repetition IS signal. Reverting this reopens the duplicate-dispute path
/// into the resolver, which costs one LLM call per false positive.
///
/// Runs BEFORE embedding: a hash lookup is orders of magnitude cheaper than
/// an inference call, so a duplicate never pays for the vector it does not
/// need.
///
/// This is the only place `absorbed` moves, so a copy arriving inside one
/// event and a copy arriving in a later event increment it through the same
/// branch and keep meaning the same thing.
async fn absorb_duplicate(
    store: &Arc<MemoryStore>,
    evt: &MemoryEvent,
    saver: &Reader,
    content_hash: &str,
    absorbed: &mut u64,
    note: impl FnOnce(i64) -> Option<OutboxNote> + Send + 'static,
) -> Option<i64> {
    match store
        .reinforce_noting(
            saver,
            evt.project_id.as_deref(),
            content_hash,
            evt.sources(),
            evt.timestamp_ms,
            note,
        )
        .await
    {
        Ok(Some(id)) => {
            *absorbed += 1;
            log::debug!(
                "ingest absorbed duplicate (agent={}, memory_id={id}) absorbed_total={absorbed}",
                evt.agent_id
            );
            Some(id)
        }
        Ok(None) => None,
        Err(e) => {
            // Fail open: a lookup failure degrades to embed+insert, the
            // behaviour from before the gate existed, rather than dropping
            // the chunk.
            log::warn!("dedup lookup failed (agent={}): {e}; falling through to embed", evt.agent_id);
            None
        }
    }
}

/// The outbox row an event is ingested for, and the pieces of it already
/// stored by an earlier attempt.
struct Track {
    row: i64,
    progress: Vec<NotedPiece>,
}

/// The note a store write of `position` records for `track`, built from the
/// progress so far plus the row the write produces.
fn note_for(
    track: Option<&Track>,
    noted: &[NotedPiece],
    position: usize,
    hash: &str,
    inserted: bool,
    context: &str,
) -> impl FnOnce(i64) -> Option<OutboxNote> + Send + 'static {
    let row = track.map(|t| t.row);
    let mut progress = noted.to_vec();
    let (hash, context) = (hash.to_string(), context.to_string());
    move |id| {
        row.map(|row| {
            progress.push(NotedPiece { position, hash, id, inserted, context });
            OutboxNote { row, progress }
        })
    }
}

/// Stores `evt` as the worker does, with the fake embedder, for the tests of
/// other modules that need the rows a real save leaves.
#[cfg(test)]
pub(crate) async fn save_for_test(store: &Arc<MemoryStore>, evt: &MemoryEvent) -> IngestReport {
    let cell: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
    cell.set(Arc::new(crate::memory::embedder::FakeEmbedder::default())).ok();
    let mut absorbed = 0;
    process_event(evt, store, &Arc::new(cell), &Arc::new(ChatChunker::with_fallback()), &mut absorbed).await
}

/// [`process_tracked`] for an event no outbox row holds.
#[cfg(test)]
async fn process_event(
    evt: &MemoryEvent,
    store: &Arc<MemoryStore>,
    embedder_cell: &Arc<OnceCell<Arc<dyn Embedder>>>,
    chat_chunker: &Arc<ChatChunker>,
    absorbed: &mut u64,
) -> IngestReport {
    process_tracked(evt, store, embedder_cell, chat_chunker, absorbed, None, DrainWatch::default()).await
}

/// Ingests `evt` and, when it finished, links its pieces and applies its
/// revision; for an event with an outbox row, those and the row's removal go
/// in one transaction. A failure leaves the row, with the pieces stored so far
/// noted in it, and replaces nothing.
async fn process_tracked(
    evt: &MemoryEvent,
    store: &Arc<MemoryStore>,
    embedder_cell: &Arc<OnceCell<Arc<dyn Embedder>>>,
    chat_chunker: &Arc<ChatChunker>,
    absorbed: &mut u64,
    track: Option<Track>,
    drain: DrainWatch<'_>,
) -> IngestReport {
    let row = track.as_ref().map(|t| t.row);
    // A save its outbox held from before its project was merged names the
    // tombstone: it is compared with, and stored in, the project it went into.
    let followed = follow_merge(store, evt).await;
    let evt = followed.as_ref().unwrap_or(evt);
    // Read once per event: the dedup, the detector and the key all compare
    // the save with what its agent reaches in its project.
    let saver = store.reader_for(&evt.agent_id);
    let scope = evt.topic().map(|key| TopicScope {
        saver: saver.clone(),
        project_id: evt.project_id.clone(),
        key: key.to_string(),
    });
    // Read before anything is stored, and again on a replay: the rows a save
    // replaces stay live until it finishes, so both reads find them.
    let earlier = match &scope {
        Some(scope) => store.topic_rows(scope).await.unwrap_or_else(|e| {
            log::warn!("could not read the rows of topic {} (agent={}): {e}", scope.key, evt.agent_id);
            Vec::new()
        }),
        None => Vec::new(),
    };
    let inherited = earlier.first().map(|oldest| inherit(evt, oldest));
    let evt = inherited.as_ref().unwrap_or(evt);
    let earlier: Vec<i64> = earlier.iter().map(|r| r.id).collect();
    let (mut report, pieces) =
        ingest_pieces(evt, &saver, store, embedder_cell, chat_chunker, absorbed, track, &earlier, drain).await;
    if report.failure.is_some() || report.paused {
        return report;
    }
    let revision = scope.filter(|_| !pieces.is_empty()).map(|scope| Revision {
        scope,
        save: pieces.iter().map(|p| p.memory_id).collect(),
        inserted: report.memories.iter().filter(|m| m.outcome == ChunkOutcome::Inserted).map(|m| m.id).collect(),
    });
    report.revision_candidate = unkeyed_revision(evt.topic(), &report.related);
    let warned = report.revision_candidate;
    report.replaced = match row {
        // A failure here leaves the row with every piece noted: the next start
        // only finishes it.
        Some(row) => store.outbox_finish(row, pieces, revision, warned).await.unwrap_or_else(|e| {
            log::warn!("could not finish outbox row {row}: {e}; the next start finishes it");
            Vec::new()
        }),
        None => finish_save(store, pieces, revision, warned).await,
    };
    report
}

/// `evt` under the live project its project leads to, when that is another.
/// A merge waits while a save of its projects is pending, so the project read
/// here is the one every later step of the save reads.
async fn follow_merge(store: &MemoryStore, evt: &MemoryEvent) -> Option<MemoryEvent> {
    let project = evt.project_id.as_deref()?;
    match crate::memory::merge::canonical(store, project).await {
        Ok(live) if live != project => Some(MemoryEvent { project_id: Some(live), ..evt.clone() }),
        Ok(_) => None,
        Err(e) => {
            log::warn!("could not follow the merges of project {project} (agent={}): {e}; the save stays under it", evt.agent_id);
            None
        }
    }
}

/// Masks `evt` and splits it into the pieces a save stores, with the head
/// every piece after the first is embedded behind. `None` when the event
/// stores nothing: a skipped role, or no text left after the chunker's rules.
///
/// Masked before anything else reads the text, so the chunker, the dedup
/// hash, the embedding, the full-text index and the stored row all see the
/// same masked content, and the secret reaches none of them. An event from
/// the outbox was masked before it was recorded, and this pass finds nothing
/// more.
fn split_event(evt: &MemoryEvent, chat_chunker: &ChatChunker) -> (Vec<SecretKind>, Option<SaveSplit>) {
    let masked = redact(&evt.content);
    // Source-first: Compaction-sourced events bypass select_chunker (and thus
    // the Role::System→Skip rule that ordinarily drops system-authored
    // content from ingestion) entirely — they always chunk via ChatChunker,
    // because a compaction summary is deliberately synthesized content worth
    // keeping regardless of which role produced the text it summarizes. All
    // other sources keep the existing role-based routing.
    let kind = match evt.source {
        MemorySource::Compaction => ChunkerKind::Chat,
        _ => select_chunker(evt.role.clone(), evt.is_error, &masked.text),
    };
    let split = match kind {
        ChunkerKind::Chat => Some(chat_chunker.split_save(&masked.text)),
        ChunkerKind::Skip => None,
    };
    // Content that was all-whitespace or empty after chunker policy is not an
    // error.
    (masked.kinds, split.filter(|split| !split.pieces.is_empty()))
}

/// What the dedup gate left of a save: what each position already became,
/// the head each of those was embedded behind, the pieces still to embed and
/// the progress noted so far.
struct Gated {
    stored: Vec<Option<SavedChunk>>,
    contexts: Vec<String>,
    survivors: Vec<(usize, PendingChunk)>,
    noted: Vec<NotedPiece>,
}

/// The head a piece at `position` is embedded behind: none for the first,
/// which already starts with it.
fn head_for(head: Option<&str>, position: usize) -> &str {
    match head {
        Some(head) if position > 0 => head,
        _ => "",
    }
}

/// Runs every piece of a save through the dedup gate before anything is
/// embedded, taking what an earlier attempt of `track` stored as it was.
async fn gate_pieces(
    evt: &MemoryEvent,
    saver: &Reader,
    store: &Arc<MemoryStore>,
    absorbed: &mut u64,
    track: Option<&Track>,
    chunks: Vec<Chunk>,
) -> Gated {
    // A collapsed entry absorbed by the gate is charged once per
    // copy it stood for, which is what the same copies arriving one at a
    // time would have done.
    // `stored[i]` is what the i-th collapsed chunk became, so the report keeps
    // chunk order even though absorbed chunks are settled before the rest.
    let pending_chunks = collapse_duplicate_chunks(chunks);
    // What an earlier attempt already stored is taken as it was: reinforcing
    // it again would count an access nobody made, and its link keeps the head
    // it was embedded behind.
    let hashes: Vec<&str> = pending_chunks.iter().map(|p| p.content_hash.as_str()).collect();
    let mut resumed: Vec<Option<NotedPiece>> = vec![None; pending_chunks.len()];
    let mut noted: Vec<NotedPiece> = Vec::new();
    if let Some(track) = track {
        match resume_positions(&track.progress, &hashes) {
            Some(by_position) => {
                resumed = by_position;
                noted = track.progress.clone();
            }
            None => log::warn!(
                "outbox row {} was noted for a different split of its text; the dedup gate takes over its pieces",
                track.row
            ),
        }
    }
    let mut stored: Vec<Option<SavedChunk>> = vec![None; pending_chunks.len()];
    let mut contexts: Vec<String> = vec![String::new(); pending_chunks.len()];
    let mut survivors: Vec<(usize, PendingChunk)> = Vec::new();
    for (position, pending) in pending_chunks.into_iter().enumerate() {
        if let Some(done) = resumed[position].take() {
            let outcome = if done.inserted { ChunkOutcome::Inserted } else { ChunkOutcome::Reinforced };
            stored[position] = Some(SavedChunk { id: done.id, outcome });
            contexts[position] = done.context;
            continue;
        }
        let note = note_for(track, &noted, position, &pending.content_hash, false, "");
        if let Some(id) = absorb_duplicate(store, evt, saver, &pending.content_hash, absorbed, note).await {
            if track.is_some() {
                noted.push(NotedPiece {
                    position,
                    hash: pending.content_hash.clone(),
                    id,
                    inserted: false,
                    context: String::new(),
                });
            }
            for _ in 1..pending.multiplicity {
                absorb_duplicate(store, evt, saver, &pending.content_hash, absorbed, |_| None).await;
            }
            stored[position] = Some(SavedChunk { id, outcome: ChunkOutcome::Reinforced });
            continue;
        }
        survivors.push((position, pending));
    }
    Gated { stored, contexts, survivors, noted }
}

/// Embeds the pieces the gate left, each behind the head its position takes,
/// in one inference call for the whole save. Passage: a stored piece is the
/// thing being searched THROUGH. Each vector feeds both the insert and the
/// contradiction KNN probe with no second inference call, so both sides of
/// `contradiction.rs`'s comparison live in the same Passage space
/// `CONTRADICTION_THRESHOLD` was tuned against.
async fn embed_pieces(
    embedder: &dyn Embedder,
    survivors: &[(usize, PendingChunk)],
    head: Option<&str>,
) -> Result<Vec<Vec<f32>>, IngestFailure> {
    let texts: Vec<String> = survivors
        .iter()
        .map(|(position, p)| embedding_text(head_for(head, *position), &p.chunk.text).into_owned())
        .collect();
    let vectors = match embedder.embed_batch(&texts, EmbedRole::Passage).await {
        Ok(v) => v,
        Err(EmbedError::NotReady) | Err(EmbedError::WorkerGone) => return Err(IngestFailure::EmbedderUnavailable),
        Err(e) => return Err(IngestFailure::Failed(format!("embed failed: {e}"))),
    };
    if vectors.len() != survivors.len() {
        // Zipping a short reply would pair chunks with other chunks'
        // vectors and store the mismatch as fact. Dropping the event is
        // recoverable; a silently mislabeled memory is not.
        return Err(IngestFailure::Failed(format!(
            "embedder returned {} vectors for {} chunks",
            vectors.len(),
            survivors.len()
        )));
    }
    Ok(vectors)
}

/// Stores the pieces of `evt` and returns its report with the pieces to link,
/// in order, each with the head its vector was embedded behind.
async fn ingest_pieces(
    evt: &MemoryEvent,
    saver: &Reader,
    store: &Arc<MemoryStore>,
    embedder_cell: &Arc<OnceCell<Arc<dyn Embedder>>>,
    chat_chunker: &Arc<ChatChunker>,
    absorbed: &mut u64,
    track: Option<Track>,
    earlier: &[i64],
    drain: DrainWatch<'_>,
) -> (IngestReport, Vec<StoredPiece>) {
    let (redacted, split) = split_event(evt, chat_chunker);
    let Some(SaveSplit { pieces: chunks, head }) = split else {
        return (IngestReport { redacted, ..Default::default() }, Vec::new());
    };

    // `memory_worker` holds events until the embedder is loaded, so this only
    // fires for a caller that skips that wait. Drop the whole event rather
    // than embedding some of its chunks: a half-embedded event could not be
    // told apart from a finished one later.
    let Some(embedder) = embedder_cell.get().cloned() else {
        log::debug!(
            "embedder NotReady; dropping {} chunks for agent={}",
            chunks.len(),
            evt.agent_id
        );
        let report =
            IngestReport { failure: Some(IngestFailure::EmbedderUnavailable), redacted, ..Default::default() };
        return (report, Vec::new());
    };

    let head = head.as_deref();
    let context = |position: usize| head_for(head, position);
    let Gated { mut stored, mut contexts, survivors, mut noted } =
        gate_pieces(evt, saver, store, absorbed, track.as_ref(), chunks).await;
    let report = |stored: &[Option<SavedChunk>], failure| IngestReport {
        memories: stored.iter().flatten().copied().collect(),
        failure,
        redacted: redacted.clone(),
        related: Vec::new(),
        replaced: Vec::new(),
        revision_candidate: None,
        paused: false,
    };
    let pieces = |stored: &[Option<SavedChunk>], contexts: &[String]| -> Vec<StoredPiece> {
        stored
            .iter()
            .zip(contexts)
            .filter_map(|(saved, context)| saved.map(|s| StoredPiece { memory_id: s.id, context: context.clone() }))
            .collect()
    };
    if survivors.is_empty() {
        return (report(&stored, None), pieces(&stored, &contexts));
    }

    let vectors = match embed_pieces(embedder.as_ref(), &survivors, head).await {
        Ok(vectors) => vectors,
        Err(failure) => return (report(&stored, Some(failure)), Vec::new()),
    };

    // Stage 3 — sequential, one chunk at a time: chunk N's contradiction
    // probe is a KNN search over rows already in the store, so chunk N-1 of
    // this same event is already there when it runs. An explicit save
    // carries a claim-bearing type, so a long one reaches the probe once per
    // chunk; the chunks this event already inserted are passed as siblings,
    // because two parts of one save are one claim by one author and must not
    // dispute each other. The rows a conclusion was drawn from are passed the
    // same way: it restates them, it does not contradict them. So are the
    // earlier rows of the save's topic key: it replaces them once it finishes.
    let mut siblings: Vec<i64> = evt.sources().to_vec();
    siblings.extend_from_slice(earlier);
    siblings.extend(stored.iter().flatten().filter(|s| s.outcome == ChunkOutcome::Inserted).map(|s| s.id));
    // A long save probes the same earlier save from several of its chunks;
    // the first chunk that finds it holds the pair, so two saves pair once.
    let mut paired: HashSet<SaveKey> = HashSet::new();
    let mut related: Vec<Candidate> = Vec::new();
    // Every take stores at least one piece before it may pause, so the embed
    // it paid for is not thrown away whole and a save always moves forward.
    for (taken, ((position, pending), vec)) in survivors.into_iter().zip(vectors).enumerate() {
        if taken > 0 && track.is_some() && pause_due(Instant::now(), drain.deadline()) {
            return (IngestReport { paused: true, ..report(&stored, None) }, Vec::new());
        }
        let PendingChunk { chunk, content_hash, multiplicity } = pending;
        // The contradiction hook runs between embed and insert: it needs the
        // freshly computed embedding to find near-duplicate candidates via
        // KNN, and its result (which existing rows this chunk is close to)
        // must be attached to the record before it is written, not after.
        let mut record = NewMemoryRecord::from_event_and_chunk(evt, &chunk);
        record.candidates = crate::memory::contradiction::detect_contradiction(
            store, &vec, &chunk.text, saver, evt.project_id.as_deref(), record.memory_type, &siblings,
        )
        .await
        .into_iter()
        .filter(|(save, _)| paired.insert(*save))
        .map(|(_, candidate)| candidate)
        .collect();
        let candidates = record.candidates.clone();
        let note = note_for(track.as_ref(), &noted, position, &content_hash, true, context(position));
        let absorbed_note = note_for(track.as_ref(), &noted, position, &content_hash, false, "");
        let id = match store.insert_unless_twin(saver, record, vec, note, absorbed_note).await {
            Ok(Stored::Inserted(id)) => id,
            Ok(Stored::Absorbed(id)) => {
                *absorbed += 1;
                log::debug!("ingest absorbed a twin that came back while the save was embedded (memory_id={id})");
                if track.is_some() {
                    noted.push(NotedPiece { position, hash: content_hash.clone(), id, inserted: false, context: String::new() });
                }
                stored[position] = Some(SavedChunk { id, outcome: ChunkOutcome::Reinforced });
                for _ in 1..multiplicity {
                    absorb_duplicate(store, evt, saver, &content_hash, absorbed, |_| None).await;
                }
                continue;
            }
            Err(e) => {
                let failure = IngestFailure::Failed(format!("store insert failed: {e}"));
                return (report(&stored, Some(failure)), Vec::new());
            }
        };
        if track.is_some() {
            noted.push(NotedPiece {
                position,
                hash: content_hash.clone(),
                id,
                inserted: true,
                context: context(position).to_string(),
            });
        }
        stored[position] = Some(SavedChunk { id, outcome: ChunkOutcome::Inserted });
        contexts[position] = context(position).to_string();
        siblings.push(id);
        related.extend(candidates);
        // The extra copies now find the row this iteration just wrote and
        // take the same branch a copy from a later event takes.
        for _ in 1..multiplicity {
            absorb_duplicate(store, evt, saver, &content_hash, absorbed, |_| None).await;
        }
    }
    (IngestReport { related, ..report(&stored, None) }, pieces(&stored, &contexts))
}

/// Finishes a save that no outbox row stands for: links its pieces when there
/// are two or more, applies its revision and marks the pair with the row its
/// warning names. A position absorbed into an existing row keeps that row's
/// vector, made without this save's head, so its link carries none.
///
/// A failure here leaves the rows unlinked and the earlier version live, which
/// is how every save was stored before links and keys existed; the save itself
/// did not fail and is not reported as failed.
async fn finish_save(
    store: &Arc<MemoryStore>,
    pieces: Vec<StoredPiece>,
    revision: Option<Revision>,
    warned: Option<i64>,
) -> Vec<Replaced> {
    if pieces.len() < 2 && revision.is_none() && warned.is_none() {
        return Vec::new();
    }
    let count = pieces.len();
    store.finish_save(pieces, revision, warned).await.unwrap_or_else(|e| {
        log::warn!("could not finish a save of {count} pieces: {e}; they stay unlinked and replace nothing");
        Vec::new()
    })
}

/// Resolves the shared tokenizer exactly once and builds the chat chunker
/// over that SAME cell: `tokenizer_cell` and the returned chunker can never
/// disagree about which parse of `tokenizer.json` they got, and the file is
/// never read twice for one process. A cell this leaves empty is filled by
/// the model load later, and the chunker splits by tokens from then on.
/// Pulled out of the worker's setup so this wiring is unit-testable on its
/// own. The loader is a parameter so the daemon can pass the one that checks
/// the file's hash and tests can pass their own.
pub fn resolve_tokenizer_and_chunker<L>(
    base_dir: &std::path::Path,
    tokenizer_cell: &Arc<OnceCell<Arc<tokenizers::Tokenizer>>>,
    load: L,
) -> Arc<ChatChunker>
where
    L: FnOnce(&std::path::Path) -> Option<Arc<tokenizers::Tokenizer>>,
{
    let _ = tokenizer_cell.get_or_try_init(|| load(base_dir).ok_or(()));
    Arc::new(ChatChunker::from_cell(tokenizer_cell.clone()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use super::*;
    use crate::memory::MemoryStore;
    use crate::memory::embedder::{EmbedError, Embedder, FakeEmbedder};
    use crate::memory::events::MemorySource;
    use crate::models::chat::Role;
    use tokio::sync::mpsc::Sender;
    use tokio::task::JoinHandle;

    // ── Test helpers ────────────────────────────────────────────────────────────

    fn make_event(role: Role, content: &str, is_error: Option<bool>, ts: u64) -> MemoryEvent {
        MemoryEvent {
            agent_id: "agent-test".to_string(),
            project_id: None,
            role,
            content: content.to_string(),
            is_error,
            source: MemorySource::Chat,
            timestamp_ms: ts,
            memory_type: None,
            importance: None,
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
            session_id: None,
        }
    }

    fn make_event_with_project(role: Role, content: &str, project_id: Option<String>, ts: u64) -> MemoryEvent {
        MemoryEvent {
            agent_id: "agent-test".to_string(),
            project_id,
            role,
            content: content.to_string(),
            is_error: None,
            source: MemorySource::Chat,
            timestamp_ms: ts,
            memory_type: None,
            importance: None,
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
            session_id: None,
        }
    }

    async fn count_memory(store: &MemoryStore) -> i64 {
        store.conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM memory", [], |r| r.get(0)))
            .await
            .unwrap()
    }

    async fn drain_worker(tx: Sender<IngestRequest>, handle: JoinHandle<()>) {
        drop(tx);
        tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("worker must exit within timeout after sender dropped")
            .expect("worker must not panic");
    }

    // ── The stages of a save ──────────────────────────────────────────────────

    #[test]
    fn the_split_masks_the_text_and_leaves_out_what_stores_nothing() {
        let chunker = ChatChunker::with_fallback();
        let (redacted, split) = split_event(&make_event(Role::User, "password=hunter2abc and more", None, 1), &chunker);
        assert_eq!(redacted, [SecretKind::Password]);
        let split = split.expect("a user's text is stored");
        assert_eq!(split.pieces.len(), 1);
        assert!(!split.pieces[0].text.contains("hunter2abc"), "{}", split.pieces[0].text);

        assert!(split_event(&make_event(Role::System, "you are a helpful agent", None, 1), &chunker).1.is_none());
        assert!(split_event(&make_event(Role::User, "", None, 1), &chunker).1.is_none());
        let compaction = MemoryEvent { source: MemorySource::Compaction, ..make_event(Role::System, "the summary", None, 1) };
        assert!(split_event(&compaction, &chunker).1.is_some(), "a compaction is kept whoever wrote it");

        let long = "word ".repeat(4_000);
        let split = split_event(&make_event(Role::User, &long, None, 1), &chunker).1.unwrap();
        assert!(split.pieces.len() > 1 && split.head.is_some(), "a long save is pieces behind a head");
    }

    #[tokio::test]
    async fn the_gate_absorbs_a_stored_piece_and_takes_a_resumed_one_as_it_was() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let evt = make_event(Role::User, "alpha", None, 1);
        let first = save_for_test(&store, &evt).await.memories[0].id;
        let saver = store.reader_for(&evt.agent_id);
        let chunks = |texts: &[&str]| texts.iter().map(|t| Chunk { text: t.to_string() }).collect::<Vec<_>>();
        let mut absorbed = 0;

        let gated = gate_pieces(&evt, &saver, &store, &mut absorbed, None, chunks(&["alpha", "beta", "beta"])).await;
        assert_eq!(gated.stored[0], Some(SavedChunk { id: first, outcome: ChunkOutcome::Reinforced }));
        assert_eq!(gated.stored[1], None);
        assert_eq!(gated.survivors.iter().map(|(p, c)| (*p, c.multiplicity)).collect::<Vec<_>>(), [(1, 2)]);
        assert_eq!(absorbed, 1);
        assert!(gated.noted.is_empty(), "nothing is noted without a row");

        let beta = crate::memory::dedup::content_hash("beta");
        let progress = vec![NotedPiece { position: 1, hash: beta, id: 77, inserted: true, context: "head".into() }];
        let track = Track { row: 1, progress };
        let gated = gate_pieces(&evt, &saver, &store, &mut absorbed, Some(&track), chunks(&["alpha", "beta"])).await;
        assert_eq!(gated.stored[1], Some(SavedChunk { id: 77, outcome: ChunkOutcome::Inserted }));
        assert_eq!(gated.contexts[1], "head", "a resumed piece keeps the head it was embedded behind");
        assert!(gated.survivors.is_empty());
        assert_eq!(gated.noted.len(), 2, "the progress so far, and the piece the gate absorbed now");
    }

    #[tokio::test]
    async fn the_embed_puts_the_head_before_every_piece_but_the_first_and_names_its_failure() {
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recording = RecordingEmbedder { calls: calls.clone() };
        let pending = |text: &str| PendingChunk {
            chunk: Chunk { text: text.to_string() },
            content_hash: crate::memory::dedup::content_hash(text),
            multiplicity: 1,
        };
        let survivors = vec![(0, pending("first")), (2, pending("third"))];

        let vectors = embed_pieces(&recording, &survivors, Some("HEAD")).await.unwrap();
        assert_eq!(vectors.len(), 2);
        let texts: Vec<String> = calls.lock().unwrap().iter().map(|(role, text)| {
            assert_eq!(*role, EmbedRole::Passage);
            text.clone()
        }).collect();
        assert_eq!(texts[0], "first");
        assert_eq!(texts[1], embedding_text("HEAD", "third").into_owned());

        struct Answering(Result<usize, EmbedError>);
        #[async_trait::async_trait]
        impl Embedder for Answering {
            async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
                unreachable!()
            }
            async fn embed_batch(&self, _texts: &[String], _role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
                match &self.0 {
                    Ok(n) => Ok(vec![vec![0.1; 384]; *n]),
                    Err(EmbedError::NotReady) => Err(EmbedError::NotReady),
                    Err(_) => Err(EmbedError::WorkerGone),
                }
            }
            fn dimensions(&self) -> usize { 384 }
            fn model_id(&self) -> &str { "answering" }
            fn query_prefix(&self) -> &str { "" }
            fn passage_prefix(&self) -> &str { "" }
        }
        for (answer, failure) in [
            (Err(EmbedError::NotReady), IngestFailure::EmbedderUnavailable),
            (Err(EmbedError::WorkerGone), IngestFailure::EmbedderUnavailable),
            (Ok(1), IngestFailure::Failed("embedder returned 1 vectors for 2 chunks".into())),
        ] {
            assert_eq!(embed_pieces(&Answering(answer), &survivors, None).await.unwrap_err(), failure);
        }
    }

    // ── Worker spawn returns a join handle usable for shutdown/panic detection ──

    #[tokio::test]
    async fn spawn_memory_worker_returns_join_handle() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());

        let (tx, handle) = spawn_test_worker(store, embedder).await;

        // Handle should NOT be done immediately
        assert!(!handle.is_finished());

        drop(tx);
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            handle,
        )
        .await;
        assert!(result.is_ok(), "worker must exit within timeout after sender dropped");
        assert!(result.unwrap().is_ok(), "worker must not panic");
    }

    // ── Graceful shutdown and per-event error isolation ──────────────────────

    #[tokio::test]
    async fn worker_exits_gracefully_when_all_senders_dropped() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(store, embedder).await;
        drain_worker(tx, handle).await;
    }

    #[tokio::test]
    async fn worker_continues_after_per_event_error() {
        // First event: Role::Assistant with empty content + tool_calls (Skip path).
        // Second event: valid user message (should be processed + inserted).
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        // Skip event (empty assistant tool-call)
        let skip_evt = MemoryEvent {
            agent_id: "agent-test".to_string(),
            project_id: None,
            role: Role::Assistant,
            content: String::new(),
            is_error: None,
            source: MemorySource::Chat,
            timestamp_ms: 1,
            memory_type: None,
            importance: None,
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
            session_id: None,
        };
        // Wait: the "tool_calls" filter is in from_message, but the worker just
        // calls select_chunker(Role::Assistant, None, "") → Skip (empty content).
        tx.try_send(IngestRequest::from(skip_evt)).unwrap();

        // Valid user event
        tx.try_send(IngestRequest::from(make_event(Role::User, "hello world", None, 2))).unwrap();

        // Give the worker time to process
        drop(tx);
        tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("worker must exit within timeout")
            .expect("worker must not panic");

        // Worker must have processed the valid event (1 row)
        let rows = count_memory(&store).await;
        assert_eq!(rows, 1, "expected 1 row for valid event; got {rows}");
    }

    // ── Embedder-not-ready gate: events wait for the embedder, in order ─────────

    #[tokio::test]
    async fn events_sent_before_the_embedder_loads_are_ingested_once_it_does() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let cell: Arc<OnceCell<Arc<dyn Embedder>>> = Arc::new(OnceCell::new());
        let (tx, handle) = spawn_test_worker_on_cell(Arc::clone(&store), Arc::clone(&cell)).await;

        tx.try_send(IngestRequest::from(make_event(Role::User, "first fact", None, 1))).unwrap();
        tx.try_send(IngestRequest::from(make_event(Role::User, "second fact", None, 2))).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert_eq!(count_memory(&store).await, 0, "nothing can be embedded before the model loads");

        cell.set(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>).ok();

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        while count_memory(&store).await < 2 {
            assert!(tokio::time::Instant::now() < deadline, "events queued before the embedder loaded were lost");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        drain_worker(tx, handle).await;
    }

    #[tokio::test]
    async fn worker_exits_when_senders_drop_while_the_embedder_never_loads() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let (tx, handle) = spawn_test_worker_opt(Arc::clone(&store), None).await;
        tx.try_send(IngestRequest::from(make_event(Role::User, "hello", None, 1))).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        drain_worker(tx, handle).await;
        assert_eq!(count_memory(&store).await, 0);
    }

    #[tokio::test]
    async fn embedder_not_ready_no_rows_inserted() {
        // FakeEmbedder with dim=0 → always returns Ok([]) → insert will fail dim
        // check → 0 rows. Alternative: use a dim that triggers DimensionMismatch.
        // Actually the dim check is for wrong dimension (not 384). Let's use a
        // test embedder that returns NotReady.
        struct NotReadyEmbedder;
        #[async_trait::async_trait]
        impl Embedder for NotReadyEmbedder {
            async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
                Err(EmbedError::NotReady)
            }
            fn dimensions(&self) -> usize { 384 }
            fn model_id(&self) -> &str { "not-ready" }
            fn query_prefix(&self) -> &str { "" }
            fn passage_prefix(&self) -> &str { "" }
        }

        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(NotReadyEmbedder);
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;
        tx.try_send(IngestRequest::from(make_event(Role::User, "hello", None, 1))).unwrap();
        drain_worker(tx, handle).await;
        let rows = count_memory(&store).await;
        assert_eq!(rows, 0, "NotReady embedder → 0 rows; got {rows}");
    }

    #[tokio::test]
    async fn embedder_worker_gone_skips_chunk_continues() {
        // First event: WorkerGone → 0 rows.
        // Second event: FakeEmbedder (we test worker survival by checking rows = 1).
        // Since we can't hot-swap the embedder in the same worker instance, we
        // verify that the WORKER itself stays alive (doesn't exit) after WorkerGone,
        // and that a subsequent event with a good embedder would still be drainable.
        // Practical test: send 1 event with WorkerGone → worker logs + returns Ok().
        // Worker is still alive → send second valid event, but wait — same embedder.
        // So we use a counting embedder that fails on call 1, succeeds on call 2.
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct CountingEmbedder {
            calls: Arc<AtomicUsize>,
        }
        #[async_trait::async_trait]
        impl Embedder for CountingEmbedder {
            async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
                let n = self.calls.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Err(EmbedError::WorkerGone)
                } else {
                    Ok(vec![0.1_f32; 384])
                }
            }
            fn dimensions(&self) -> usize { 384 }
            fn model_id(&self) -> &str { "counting" }
            fn query_prefix(&self) -> &str { "" }
            fn passage_prefix(&self) -> &str { "" }
        }

        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let embedder: Arc<dyn Embedder> = Arc::new(CountingEmbedder { calls });
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        // First event → WorkerGone → 0 rows for this event
        tx.try_send(IngestRequest::from(make_event(Role::User, "first", None, 1))).unwrap();
        // Second event → Ok → 1 row
        tx.try_send(IngestRequest::from(make_event(Role::User, "second", None, 2))).unwrap();

        drain_worker(tx, handle).await;

        let rows = count_memory(&store).await;
        // First event: WorkerGone breaks inner loop → 0 rows.
        // Second event: Ok → 1 row.
        assert_eq!(rows, 1, "expected 1 row (second event), got {rows}");
    }

    /// An embedder that returns a vector containing NaN never reaches the
    /// vec0 table: `store_insert.rs` routes every embedding through
    /// `memory::vector::to_unit`, which rejects any non-finite component
    /// before the transaction opens. The chunk is dropped, not the worker —
    /// the same `Err` -> `log::warn!` + continue path already exercised by
    /// `store_write_failure_worker_stays_alive` above handles it with no
    /// NaN-specific branch anywhere in the worker loop.
    #[tokio::test]
    async fn nan_embedding_drops_chunk_worker_stays_alive() {
        struct NanEmbedder;
        #[async_trait::async_trait]
        impl Embedder for NanEmbedder {
            async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
                Ok(vec![f32::NAN; 384])
            }
            fn dimensions(&self) -> usize {
                384
            }
            fn model_id(&self) -> &str {
                "nan-embedder"
            }
            fn query_prefix(&self) -> &str {
                ""
            }
            fn passage_prefix(&self) -> &str {
                ""
            }
        }

        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(NanEmbedder);
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.try_send(IngestRequest::from(make_event(Role::User, "hello", None, 1))).unwrap();
        // Second event through the same worker proves the loop kept running
        // past the rejected chunk instead of merely exiting cleanly on drop.
        tx.try_send(IngestRequest::from(make_event(Role::User, "still alive", None, 2))).unwrap();
        drain_worker(tx, handle).await;

        let rows = count_memory(&store).await;
        assert_eq!(
            rows, 0,
            "NaN embedding must be rejected before any row is written; got {rows} rows"
        );
    }

    #[tokio::test]
    async fn store_write_failure_worker_stays_alive() {
        // Drop memory_vectors between events to cause insert failure.
        // Worker must continue processing and not exit.
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        // First event: process before drop (should succeed)
        tx.try_send(IngestRequest::from(make_event(Role::User, "event1", None, 1))).unwrap();

        // Give time for event1 to be processed
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        // Drop memory_vectors to cause subsequent inserts to fail
        store.conn
            .call(|c| c.execute("DROP TABLE IF EXISTS memory_vectors", []))
            .await
            .unwrap();

        // Second event: insert fails but worker stays alive
        tx.try_send(IngestRequest::from(make_event(Role::User, "event2", None, 2))).unwrap();

        drain_worker(tx, handle).await;
        // Worker stayed alive (drain_worker didn't panic or timeout) — test passes.
    }

    // ── memory_type / created_by_type derivation from role and error flag ───

    #[tokio::test]
    async fn user_role_produces_chat_message_type_user_creator() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.try_send(IngestRequest::from(make_event(Role::User, "hello", None, 1))).unwrap();
        drain_worker(tx, handle).await;

        let (mt, cbt): (String, String) = store.conn
            .call(|c| c.query_row(
                "SELECT memory_type, created_by_type FROM memory LIMIT 1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            ))
            .await
            .unwrap();
        assert_eq!(mt, "chat_message");
        assert_eq!(cbt, "user");
    }

    #[tokio::test]
    async fn assistant_role_produces_agent_creator() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.try_send(IngestRequest::from(make_event(Role::Assistant, "I can help you", None, 1))).unwrap();
        drain_worker(tx, handle).await;

        let cbt: String = store.conn
            .call(|c| c.query_row(
                "SELECT created_by_type FROM memory LIMIT 1",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert_eq!(cbt, "agent");
    }

    #[tokio::test]
    async fn tool_error_produces_tool_error_type_system_creator_importance_6() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.try_send(IngestRequest::from(make_event(Role::Tool, "error output", Some(true), 1))).unwrap();
        drain_worker(tx, handle).await;

        let (mt, cbt, imp): (String, String, f64) = store.conn
            .call(|c| c.query_row(
                "SELECT memory_type, created_by_type, importance FROM memory LIMIT 1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?)),
            ))
            .await
            .unwrap();
        assert_eq!(mt, "tool_error");
        assert_eq!(cbt, "system");
        assert!(imp >= 6.0, "expected importance >= 6.0, got {imp}");
    }

    #[tokio::test]
    async fn tool_success_produces_chat_message_type_system_creator() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.try_send(IngestRequest::from(make_event(Role::Tool, "tool output", Some(false), 1))).unwrap();
        drain_worker(tx, handle).await;

        let (mt, cbt): (String, String) = store.conn
            .call(|c| c.query_row(
                "SELECT memory_type, created_by_type FROM memory LIMIT 1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            ))
            .await
            .unwrap();
        assert_eq!(mt, "chat_message");
        assert_eq!(cbt, "system");
    }

    // ── project_id flow-through from event to stored row ─────────────────────

    #[tokio::test]
    async fn project_id_derived_from_path() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        let evt = make_event_with_project(Role::User, "hello", Some("derived-hash".to_string()), 1);
        tx.try_send(IngestRequest::from(evt)).unwrap();
        drain_worker(tx, handle).await;

        let pid: Option<String> = store.conn
            .call(|c| c.query_row(
                "SELECT project_id FROM memory LIMIT 1",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert_eq!(pid.as_deref(), Some("derived-hash"));
    }

    #[tokio::test]
    async fn project_id_none_when_no_path() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        let evt = make_event_with_project(Role::User, "hello", None, 1);
        tx.try_send(IngestRequest::from(evt)).unwrap();
        drain_worker(tx, handle).await;

        let pid: Option<String> = store.conn
            .call(|c| c.query_row(
                "SELECT project_id FROM memory LIMIT 1",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert!(pid.is_none(), "expected NULL project_id, got {pid:?}");
    }

    // ── 5-turn end-to-end integration: a full chat exchange through the worker ──

    #[tokio::test]
    async fn e2e_5_turn_chat_produces_5_rows() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        let roles = [
            (Role::User, "msg1", None),
            (Role::Assistant, "msg2", None),
            (Role::Tool, "msg3", Some(false)),
            (Role::User, "msg4", None),
            (Role::Assistant, "msg5", None),
        ];
        for (i, (role, content, is_error)) in roles.iter().enumerate() {
            tx.try_send(IngestRequest::from(MemoryEvent {
                agent_id: "agent-test".to_string(),
                project_id: None,
                role: role.clone(),
                content: content.to_string(),
                is_error: *is_error,
                source: MemorySource::Chat,
                timestamp_ms: (i as u64) + 1,
                memory_type: None,
                importance: None,
                derived_from: Vec::new(),
                topic_key: None,
                user_stated: false,
                session_id: None,
            })).unwrap();
        }
        drain_worker(tx, handle).await;

        // 5 rows in memory
        let mc = count_memory(&store).await;
        assert_eq!(mc, 5, "expected 5 rows in memory, got {mc}");

        // 5 rows in memory_vectors
        let vc: i64 = store.conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM memory_vectors", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(vc, 5, "expected 5 rows in memory_vectors, got {vc}");

        // All rows have agent_id = "agent-test"
        let agent_count: i64 = store.conn
            .call(|c| c.query_row(
                "SELECT COUNT(*) FROM memory WHERE agent_id = 'agent-test'",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert_eq!(agent_count, 5, "all rows must have agent_id=agent-test");

        // memory_vectors rowids match memory ids
        let mismatch: i64 = store.conn
            .call(|c| c.query_row(
                "SELECT COUNT(*) FROM memory m \
                 LEFT JOIN memory_vectors v ON m.id = v.rowid \
                 WHERE v.rowid IS NULL",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert_eq!(mismatch, 0, "all memory rows must have matching vector rows");

        // FIFO ordering: rows must come back in the same order they were sent.
        // Asserting on (created_at, content) pairs catches a regression where the
        // consumer or the INSERT path scrambles temporal order — the kind of bug
        // that a bare count check would silently let through.
        let ordered: Vec<(i64, String)> = store.conn
            .call(|c| -> Result<Vec<(i64, String)>, rusqlite::Error> {
                let mut stmt = c.prepare(
                    "SELECT created_at, content FROM memory ORDER BY created_at ASC, id ASC",
                )?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                    .collect::<Result<Vec<(i64, String)>, rusqlite::Error>>()?;
                Ok(rows)
            })
            .await
            .unwrap();
        let expected: Vec<(i64, &str)> = vec![
            (1, "msg1"), (2, "msg2"), (3, "msg3"), (4, "msg4"), (5, "msg5"),
        ];
        assert_eq!(
            ordered.iter().map(|(ts, c)| (*ts, c.as_str())).collect::<Vec<_>>(),
            expected,
            "FIFO order broken — rows arrived in {:?}",
            ordered,
        );

        // Repeat with project_id = Some("proj-hash")
        let store2 = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder2: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx2, handle2) = spawn_test_worker(Arc::clone(&store2), embedder2).await;
        for (i, (role, content, is_error)) in roles.iter().enumerate() {
            tx2.try_send(IngestRequest::from(MemoryEvent {
                agent_id: "agent-test".to_string(),
                project_id: Some("proj-hash".to_string()),
                role: role.clone(),
                content: content.to_string(),
                is_error: *is_error,
                source: MemorySource::Chat,
                timestamp_ms: (i as u64) + 1,
                memory_type: None,
                importance: None,
                derived_from: Vec::new(),
                topic_key: None,
                user_stated: false,
                session_id: None,
            })).unwrap();
        }
        drain_worker(tx2, handle2).await;
        let proj_count: i64 = store2.conn
            .call(|c| c.query_row(
                "SELECT COUNT(*) FROM memory WHERE project_id IS NOT NULL",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert_eq!(proj_count, 5, "all rows must have project_id set; got {proj_count}");

        // Repeat with project_id = None
        let store3 = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder3: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx3, handle3) = spawn_test_worker(Arc::clone(&store3), embedder3).await;
        for (i, (role, content, is_error)) in roles.iter().enumerate() {
            tx3.try_send(IngestRequest::from(MemoryEvent {
                agent_id: "agent-test".to_string(),
                project_id: None,
                role: role.clone(),
                content: content.to_string(),
                is_error: *is_error,
                source: MemorySource::Chat,
                timestamp_ms: (i as u64) + 1,
                memory_type: None,
                importance: None,
                derived_from: Vec::new(),
                topic_key: None,
                user_stated: false,
                session_id: None,
            })).unwrap();
        }
        drain_worker(tx3, handle3).await;
        let null_count: i64 = store3.conn
            .call(|c| c.query_row(
                "SELECT COUNT(*) FROM memory WHERE project_id IS NULL",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert_eq!(null_count, 5, "all rows must have NULL project_id; got {null_count}");
    }

    // ── system_role / empty assistant tool-call filtering ────────────────────

    #[tokio::test]
    async fn system_role_never_reaches_worker() {
        // MemoryEvent::from_message returns None for Role::System — this is
        // tested here to confirm the filter contract, even though the worker
        // also has select_chunker(Role::System, ...) → Skip as a defensive arm.
        use crate::models::chat::ChatMessage;
        let msg = ChatMessage {
            id: "t".to_string(),
            role: Role::System,
            content: "system prompt".to_string(),
            tool_calls: None,
            timestamp_ms: 0,
            is_error: None,
        };
        let result = crate::memory::events::MemoryEvent::from_message(
            "agent".to_string(),
            None,
            &msg,
            MemorySource::Chat,
        );
        assert!(result.is_none(), "system role must return None from from_message");
    }

    #[tokio::test]
    async fn empty_assistant_tool_call_not_ingested() {
        use crate::models::chat::ChatMessage;
        let mut msg = ChatMessage {
            id: "t".to_string(),
            role: Role::Assistant,
            content: String::new(),
            tool_calls: Some(vec![]),
            timestamp_ms: 0,
            is_error: None,
        };
        let result = crate::memory::events::MemoryEvent::from_message(
            "agent".to_string(),
            None,
            &msg,
            MemorySource::Chat,
        );
        assert!(result.is_none(), "empty assistant tool-call must return None from from_message");

        // Also verify non-empty assistant content DOES produce an event
        msg.content = "actual response".to_string();
        msg.tool_calls = None;
        let result2 = crate::memory::events::MemoryEvent::from_message(
            "agent".to_string(),
            None,
            &msg,
            MemorySource::Chat,
        );
        assert!(result2.is_some(), "non-empty assistant content must produce an event");
    }

    // ── Compile-time Send + Sync guards ──────────────────────────────────────

    #[test]
    fn memory_event_and_worker_types_are_send_sync() {
        fn _send<T: Send + 'static>() {}
        fn _sync<T: Sync>() {}
        _send::<MemoryEvent>();
        _send::<Arc<OnceCell<Arc<dyn Embedder>>>>();
        _sync::<Arc<OnceCell<Arc<dyn Embedder>>>>();
    }

    // ── Worker contradiction hook: disputed_against set on match, left None otherwise ──

    /// Two identical-content events must REINFORCE, not dispute: the
    /// write-border dedup gate hashes the second chunk, finds the first row
    /// by `(agent_id, project_id, content_hash)`, and increments its
    /// `access_count` instead of inserting a second row and running the
    /// contradiction detector against it. This deliberately inverts the
    /// prior expectation of this test — a resent chunk used to reach
    /// `detect_contradiction` at distance 0 and get marked `disputed`, which
    /// was exactly the structural false positive the dedup gate exists to
    /// prevent.
    #[tokio::test]
    async fn worker_reinforces_identical_content_instead_of_disputing() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        // Event 1: no prior rows → inserted, active.
        tx.try_send(IngestRequest::from(make_event(Role::User, "identical content", None, 1))).unwrap();
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        // Event 2: identical content → hash matches row 1 → REINFORCE, not insert.
        tx.try_send(IngestRequest::from(make_event(Role::User, "identical content", None, 2))).unwrap();

        drain_worker(tx, handle).await;

        // Fetch all rows ordered by id
        let conn = store.conn.clone();
        let rows: Vec<(i64, String, Option<String>, i64)> = conn
            .call(|c| -> Result<Vec<(i64, String, Option<String>, i64)>, rusqlite::Error> {
                let mut stmt = c.prepare(
                    "SELECT id, status, disputed_against, access_count FROM memory ORDER BY id ASC"
                )?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap();

        assert_eq!(rows.len(), 1, "the second identical event must REINFORCE, not insert a second row; got {} rows", rows.len());
        let (_, status, disputed_against, access_count) = &rows[0];
        assert_eq!(status, "active", "the reinforced row must stay active, never disputed");
        assert!(disputed_against.is_none(), "reinforcement must never set disputed_against");
        assert_eq!(*access_count, 2, "access_count must be 2 after one reinforcement");
    }

    /// detect_contradiction returns empty → new row has
    /// disputed_against=None and status='active'.
    #[tokio::test]
    async fn worker_leaves_disputed_against_none_when_no_contradiction() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        // Single event, no prior rows → detect_contradiction returns [] → active
        tx.try_send(IngestRequest::from(make_event(Role::User, "unique content abc123", None, 1))).unwrap();
        drain_worker(tx, handle).await;

        let conn = store.conn.clone();
        let (status, da): (String, Option<String>) = conn
            .call(|c| {
                c.query_row(
                    "SELECT status, disputed_against FROM memory LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap();

        assert_eq!(status, "active", "single event with no prior rows must be active");
        assert!(da.is_none(), "single event with no prior rows must have disputed_against=NULL");
    }

    // ── Role: worker embeds Passage exactly once per chunk, reused by both consumers ──

    /// Records every `(role, text)` pair handed to `embed_with_role`.
    struct RecordingEmbedder {
        calls: Arc<std::sync::Mutex<Vec<(EmbedRole, String)>>>,
    }
    #[async_trait::async_trait]
    impl Embedder for RecordingEmbedder {
        async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            self.calls.lock().unwrap().push((role, text.to_string()));
            Ok(vec![0.1_f32; 384])
        }
        fn dimensions(&self) -> usize {
            384
        }
        fn model_id(&self) -> &str {
            "recording"
        }
        fn query_prefix(&self) -> &str {
            ""
        }
        fn passage_prefix(&self) -> &str {
            ""
        }
    }

    /// A one-chunk ingest event must produce exactly ONE `embed_with_role`
    /// call, role `Passage` — the same vector feeds both
    /// `insert_memory_with_vector` and `detect_contradiction`'s KNN probe, so
    /// there must be no second inference for the contradiction check.
    #[tokio::test]
    async fn worker_embeds_passage_exactly_once_per_chunk() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let embedder: Arc<dyn Embedder> = Arc::new(RecordingEmbedder { calls: calls.clone() });
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.try_send(IngestRequest::from(make_event(Role::User, "single chunk of content", None, 1))).unwrap();
        drain_worker(tx, handle).await;

        let recorded = calls.lock().unwrap().clone();
        assert_eq!(
            recorded.len(),
            1,
            "expected exactly one embed call for a one-chunk event, got {recorded:?}"
        );
        assert_eq!(recorded[0].0, EmbedRole::Passage, "worker must embed with role Passage");
        assert_eq!(recorded[0].1, "single chunk of content");
    }

    /// The write-border dedup gate runs BEFORE `embed_with_role`: a duplicate
    /// chunk must never reach the embedder at all. Sending the identical text
    /// twice through one worker must produce exactly ONE recorded embed call
    /// — asserting `== 1`, not `== 0`, is the point: it proves the second
    /// send was absorbed by the gate, not that the embedder was never wired.
    #[tokio::test]
    async fn worker_embeds_duplicate_chunk_zero_times() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let embedder: Arc<dyn Embedder> = Arc::new(RecordingEmbedder { calls: calls.clone() });
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.try_send(IngestRequest::from(make_event(Role::User, "repeated content", None, 1))).unwrap();
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        tx.try_send(IngestRequest::from(make_event(Role::User, "repeated content", None, 2))).unwrap();
        drain_worker(tx, handle).await;

        let recorded = calls.lock().unwrap().clone();
        assert_eq!(
            recorded.len(),
            1,
            "the duplicate send must be absorbed by the dedup gate before embedding, got {recorded:?}"
        );
    }

    // ── Absorbed-duplicates counter: pinned against deletion ─────────────────

    /// `absorbed` is a plain counter incremented once per REINFORCE hit.
    /// Every existing test in this module observes duplicate absorption only
    /// indirectly — via row count or embed-call count — so deleting the
    /// increment itself (or the whole `log::debug!` line reading it) would
    /// leave the rest of the suite green. This test calls `process_event`
    /// directly, twice, with identical content through the same store and
    /// embedder, and asserts the counter's value after each call.
    #[tokio::test]
    async fn absorbed_duplicates_counter_increments_on_reinforce_hit() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let cell: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
        cell.set(embedder).ok();
        let cell = Arc::new(cell);
        let chat_chunker = Arc::new(ChatChunker::with_fallback());

        let mut absorbed: u64 = 0;
        let evt = make_event(Role::User, "counter fixture content", None, 1);

        process_event(&evt, &store, &cell, &chat_chunker, &mut absorbed)
            .await;
        assert_eq!(absorbed, 0, "the first send has no prior row to reinforce");

        process_event(&evt, &store, &cell, &chat_chunker, &mut absorbed)
            .await;
        assert_eq!(
            absorbed, 1,
            "the second identical send must be absorbed by the dedup gate, incrementing the counter"
        );
    }

    // ── The report of what an event wrote and where it stopped ──────────────

    /// Embeds like `FakeEmbedder` except at one batch position, which comes
    /// back NaN: the store rejects a non-finite vector, so the insert of
    /// exactly that chunk fails while the chunks around it are valid.
    struct NanAtEmbedder {
        position: usize,
    }
    #[async_trait::async_trait]
    impl Embedder for NanAtEmbedder {
        async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            FakeEmbedder::default().embed_with_role(text, role).await
        }
        async fn embed_batch(&self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
            let mut out = FakeEmbedder::default().embed_batch(texts, role).await?;
            if let Some(v) = out.get_mut(self.position) {
                *v = vec![f32::NAN; 384];
            }
            Ok(out)
        }
        fn dimensions(&self) -> usize { 384 }
        fn model_id(&self) -> &str { "nan-at" }
        fn query_prefix(&self) -> &str { "" }
        fn passage_prefix(&self) -> &str { "" }
    }

    /// Distinct words, so no two windows of the fallback chunker collapse.
    fn distinct_text(words: usize) -> String {
        (0..words).map(|i| format!("w{i:05}")).collect::<Vec<_>>().join(" ")
    }

    async fn process_with(embedder: Arc<dyn Embedder>, store: &Arc<MemoryStore>, evt: &MemoryEvent) -> IngestReport {
        let cell: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
        cell.set(embedder).ok();
        let chat_chunker = Arc::new(ChatChunker::with_fallback());
        let mut absorbed = 0;
        process_event(evt, store, &Arc::new(cell), &chat_chunker, &mut absorbed).await
    }

    fn chunk_count(content: &str) -> usize {
        ChatChunker::with_fallback().split_save(content).pieces.len()
    }

    #[tokio::test]
    async fn a_code_block_save_is_chunked_like_any_other() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let code = "fn main() {\n    println!(\"hi\");\n}";
        let mut evt = make_event(Role::User, code, None, 1);
        evt.source = MemorySource::Explicit;
        evt.memory_type = Some("code_block".to_string());

        let report = process_with(Arc::new(FakeEmbedder::default()), &store, &evt).await;

        assert_eq!(report.failure, None);
        assert_eq!(report.memories.len(), 1);
        let (content, memory_type): (String, String) = store
            .conn
            .call(|c| -> rusqlite::Result<_> {
                c.query_row("SELECT content, memory_type FROM memory", [], |r| Ok((r.get(0)?, r.get(1)?)))
            })
            .await
            .unwrap();
        assert_eq!(content, code);
        assert_eq!(memory_type, "code_block");
    }

    /// Hands the i-th text of each batch the i-th vector, so a test decides
    /// how close each chunk of one event lands to the others.
    struct PlacedEmbedder {
        vectors: Vec<Vec<f32>>,
    }
    #[async_trait::async_trait]
    impl Embedder for PlacedEmbedder {
        async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            Ok(self.vectors[0].clone())
        }
        async fn embed_batch(&self, texts: &[String], _role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
            Ok(self.vectors[..texts.len()].to_vec())
        }
        fn dimensions(&self) -> usize { 384 }
        fn model_id(&self) -> &str { "placed" }
        fn query_prefix(&self) -> &str { "" }
        fn passage_prefix(&self) -> &str { "" }
    }

    /// `e0` turned `angle` radians toward `e{toward}`: two of these at
    /// angles a and b sit `2·sin(|a−b|/2)` apart.
    fn turned(toward: usize, angle: f32) -> Vec<f32> {
        let mut v = vec![0.0_f32; 384];
        v[0] = angle.cos();
        v[toward] = angle.sin();
        v
    }

    fn explicit_fact(content: &str) -> MemoryEvent {
        MemoryEvent {
            source: MemorySource::Explicit,
            memory_type: Some("semantic_fact".into()),
            ..make_event(Role::User, content, None, 1)
        }
    }

    async fn disputes_of(store: &MemoryStore, id: i64) -> (String, Option<String>) {
        store
            .conn
            .call(move |c| {
                c.query_row("SELECT status, disputed_against FROM memory WHERE id = ?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn sibling_chunks_of_one_save_do_not_dispute_each_other() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let content = distinct_text(400);
        let chunks = chunk_count(&content);
        assert!(chunks >= 2, "fixture must split into several chunks, got {chunks}");
        // Every other sibling sits in the dispute band of its neighbours.
        let in_band = PlacedEmbedder { vectors: (0..chunks).map(|i| turned(1, 0.2 * (i % 2) as f32)).collect() };

        let report = process_with(Arc::new(in_band), &store, &explicit_fact(&content)).await;

        assert_eq!(report.failure, None);
        for saved in &report.memories {
            assert_eq!(disputes_of(&store, saved.id).await, ("active".to_string(), None));
        }
    }

    async fn pairs(store: &MemoryStore) -> Vec<(i64, i64, String)> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT new_id, old_id, state FROM memory_conflict ORDER BY new_id, old_id")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect();
                rows
            })
            .await
            .unwrap()
    }

    fn explicit(content: &str, memory_type: &str) -> MemoryEvent {
        MemoryEvent { memory_type: Some(memory_type.into()), ..explicit_fact(content) }
    }

    #[tokio::test]
    async fn a_close_fact_is_a_pending_candidate_and_both_stay_active() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let older = PlacedEmbedder { vectors: vec![turned(2, 0.2)] };
        let older = process_with(Arc::new(older), &store, &explicit_fact("the build uses cargo")).await.memories[0].id;
        let newer = PlacedEmbedder { vectors: vec![turned(1, 0.0)] };
        let newer = process_with(Arc::new(newer), &store, &explicit_fact("the build uses make")).await.memories[0].id;

        assert_eq!(disputes_of(&store, older).await, ("active".to_string(), None));
        assert_eq!(disputes_of(&store, newer).await, ("active".to_string(), None));
        assert_eq!(pairs(&store).await, vec![(newer, older, "pending".to_string())]);
    }

    #[tokio::test]
    async fn a_close_preference_escalates_and_disputes() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let older = PlacedEmbedder { vectors: vec![turned(2, 0.2)] };
        let older = process_with(Arc::new(older), &store, &explicit("answer in English", "user_preference"))
            .await
            .memories[0]
            .id;
        let newer = PlacedEmbedder { vectors: vec![turned(1, 0.0)] };
        let newer = process_with(Arc::new(newer), &store, &explicit("answer in Spanish", "user_preference"))
            .await
            .memories[0]
            .id;

        assert_eq!(disputes_of(&store, newer).await, ("disputed".to_string(), Some(format!("[{older}]"))));
        assert_eq!(pairs(&store).await, vec![(newer, older, "escalated".to_string())]);
    }

    #[tokio::test]
    async fn a_confirmed_save_names_its_candidates() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let older = PlacedEmbedder { vectors: vec![turned(2, 0.2)] };
        let older = process_with(Arc::new(older), &store, &explicit_fact("Build tool\nthe build uses cargo"))
            .await
            .memories[0]
            .id;
        let far = PlacedEmbedder { vectors: vec![turned(3, 1.5)] };
        let lonely = process_with(Arc::new(far), &store, &explicit_fact("the logo is green")).await;
        assert!(lonely.related.is_empty(), "{:?}", lonely.related);

        let newer = PlacedEmbedder { vectors: vec![turned(1, 0.0)] };
        let report = process_with(Arc::new(newer), &store, &explicit_fact("the build uses make")).await;

        assert_eq!(report.related.len(), 1, "{:?}", report.related);
        let related = &report.related[0];
        assert_eq!((related.id, related.escalated, related.excerpt.as_str()), (older, false, "Build tool"));
    }

    fn fact_in(content: &str, project: Option<&str>) -> MemoryEvent {
        MemoryEvent { project_id: project.map(str::to_owned), ..explicit_fact(content) }
    }

    async fn save_placed(store: &Arc<MemoryStore>, vector: Vec<f32>, evt: &MemoryEvent) -> IngestReport {
        let report = process_with(Arc::new(PlacedEmbedder { vectors: vec![vector] }), store, evt).await;
        assert_eq!(report.failure, None);
        report
    }

    #[tokio::test]
    async fn an_unkeyed_save_names_the_closest_candidate_its_key_could_revise() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let far = only_id(&save_placed(&store, turned(2, 0.3), &fact_in("the build uses cargo", Some("p-1"))).await);
        let unscoped = only_id(&save_placed(&store, turned(3, 0.15), &fact_in("builds take a minute", None)).await);
        let near = only_id(&save_placed(&store, turned(4, 0.2), &fact_in("the build uses cargo 1.90", Some("p-1"))).await);

        let report = save_placed(&store, turned(1, 0.0), &fact_in("the build uses make", Some("p-1"))).await;

        let scope: Vec<(i64, bool)> = report.related.iter().map(|c| (c.id, c.same_scope)).collect();
        assert!(scope.contains(&(far, true)) && scope.contains(&(near, true)), "{scope:?}");
        assert!(scope.contains(&(unscoped, false)), "a row without a project is no key's: {scope:?}");
        assert_eq!(report.revision_candidate, Some(near), "the closest of its own project, not the closest of all");
    }

    #[tokio::test]
    async fn a_save_under_a_key_or_without_a_candidate_of_its_scope_names_none() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let older = only_id(&save_placed(&store, turned(2, 0.2), &fact_in("the build uses cargo", Some("p-1"))).await);

        let keyed = MemoryEvent { topic_key: Some("build/tool".into()), ..fact_in("the build uses make", Some("p-1")) };
        let keyed = save_placed(&store, turned(1, 0.0), &keyed).await;
        assert_eq!(keyed.related.iter().map(|c| c.id).collect::<Vec<_>>(), vec![older]);
        assert_eq!(keyed.revision_candidate, None, "a key already revises");

        let unprojected = save_placed(&store, turned(3, 0.05), &fact_in("the build uses ninja", None)).await;
        assert!(unprojected.related.iter().all(|c| !c.same_scope), "{:?}", unprojected.related);
        assert_eq!(unprojected.revision_candidate, None, "rows of a project are out of a save without one");
    }

    async fn warned_pairs(store: &MemoryStore) -> Vec<(i64, i64)> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT new_id, old_id FROM memory_conflict WHERE topic_key_warned ORDER BY new_id, old_id",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect();
                rows
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn an_unkeyed_save_marks_the_pair_with_its_revision_candidate() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let far = only_id(&save_placed(&store, turned(2, 0.3), &fact_in("the build uses cargo", Some("p-1"))).await);
        let near = only_id(&save_placed(&store, turned(3, 0.2), &fact_in("the build uses cargo 1.90", Some("p-1"))).await);

        let report = save_placed(&store, turned(1, 0.0), &fact_in("the build uses make", Some("p-1"))).await;
        let new = only_id(&report);

        assert_eq!(report.revision_candidate, Some(near));
        let written: Vec<(i64, i64)> = pairs(&store).await.into_iter().map(|(n, o, _)| (n, o)).collect();
        assert!(written.contains(&(new, far)) && written.contains(&(new, near)), "{written:?}");
        let marked: Vec<(i64, i64)> = warned_pairs(&store).await.into_iter().filter(|(n, _)| *n == new).collect();
        assert_eq!(marked, vec![(new, near)], "only the pair with the row the warning names");
    }

    #[tokio::test]
    async fn a_save_under_a_key_marks_no_pair() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let older = only_id(&save_placed(&store, turned(2, 0.2), &fact_in("the build uses cargo", Some("p-1"))).await);

        let keyed = MemoryEvent { topic_key: Some("build/tool".into()), ..fact_in("the build uses make", Some("p-1")) };
        let new = only_id(&save_placed(&store, turned(1, 0.0), &keyed).await);

        assert!(pairs(&store).await.iter().any(|(n, o, _)| (*n, *o) == (new, older)), "the pair is still written");
        assert!(warned_pairs(&store).await.is_empty());
    }

    #[tokio::test]
    async fn following_the_warning_keeps_the_pair_marked() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let older = only_id(&save_placed(&store, turned(2, 0.2), &fact_in("the build uses cargo", Some("p-1"))).await);
        let warned = save_placed(&store, turned(1, 0.0), &fact_in("the build uses make", Some("p-1"))).await;
        let new = only_id(&warned);
        assert_eq!(warned.revision_candidate, Some(older));

        let keyed = MemoryEvent { topic_key: Some("build/tool".into()), ..fact_in("the build uses make", Some("p-1")) };
        let again = save_placed(&store, turned(1, 0.0), &keyed).await;

        assert_eq!(again.memories, vec![SavedChunk { id: new, outcome: ChunkOutcome::Reinforced }]);
        assert_eq!(live_with_key(&store, "build/tool").await, vec![new], "the warned row took the key");
        assert_eq!(warned_pairs(&store).await, vec![(new, older)], "and its pair keeps the mark");
    }

    #[tokio::test]
    async fn a_verdict_or_a_delete_leaves_the_mark() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let older = only_id(&save_placed(&store, turned(2, 0.2), &fact_in("the build uses cargo", Some("p-1"))).await);
        let new = only_id(&save_placed(&store, turned(1, 0.0), &fact_in("the build uses make", Some("p-1"))).await);

        crate::memory::dispute::judge_pair(&store, new, older, crate::memory::dispute::Verdict::Supersedes, None, false, None)
            .await
            .unwrap();
        assert_eq!(warned_pairs(&store).await, vec![(new, older)], "after a verdict");

        crate::memory::admin::delete_memory_core(&store, new, crate::memory::admin::MemoryKind::Memory, None)
            .await
            .unwrap();
        assert_eq!(warned_pairs(&store).await, vec![(new, older)], "after a delete");
    }

    #[tokio::test]
    async fn a_chunk_still_pairs_with_an_older_row() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let older = PlacedEmbedder { vectors: vec![turned(2, 0.2)] };
        let older = process_with(Arc::new(older), &store, &explicit_fact("the build uses cargo")).await;
        let older = older.memories[0].id;

        // Every sibling sits closer to the last chunk than the older row
        // does, so without room for them they fill the whole candidate pool.
        let content = distinct_text(400);
        let chunks = chunk_count(&content);
        assert!(chunks >= 7, "fixture must split into at least seven chunks, got {chunks}");
        let siblings = PlacedEmbedder { vectors: (0..chunks).map(|i| turned(1, 0.01 * i as f32)).collect() };

        let report = process_with(Arc::new(siblings), &store, &explicit_fact(&content)).await;

        assert_eq!(report.failure, None);
        assert_eq!(report.memories.len(), chunks);
        for saved in &report.memories {
            assert_eq!(disputes_of(&store, saved.id).await.0, "active");
        }
        let first = report.memories[0].id;
        assert_eq!(pairs(&store).await, vec![(first, older, "pending".to_string())], "one pair per pair of saves");
    }

    #[tokio::test]
    async fn a_long_save_pairs_once_with_an_earlier_save() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let content = distinct_text(400);
        let chunks = chunk_count(&content);
        assert!(chunks >= 3, "fixture must split into several chunks, got {chunks}");
        let earlier = PlacedEmbedder { vectors: (0..chunks).map(|i| turned(1, 0.15 + 0.01 * i as f32)).collect() };
        let earlier = process_with(Arc::new(earlier), &store, &explicit_fact(&content)).await;
        let pieces = earlier
            .memories
            .iter()
            .map(|m| StoredPiece { memory_id: m.id, context: String::new() })
            .collect();
        store.link_pieces(pieces).await.unwrap();

        let rewrite = distinct_text(400).replace('w', "v");
        let later = PlacedEmbedder { vectors: (0..chunks).map(|i| turned(2, 0.15 + 0.01 * i as f32)).collect() };
        let later = process_with(Arc::new(later), &store, &explicit_fact(&rewrite)).await;

        assert_eq!(later.failure, None);
        let found = pairs(&store).await;
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, later.memories[0].id);
        assert!(earlier.memories.iter().any(|m| m.id == found[0].1), "{found:?}");
        assert_eq!(later.related.len(), 1, "{:?}", later.related);
    }

    // ── The pieces of a long save ───────────────────────────────────────────

    /// `FakeEmbedder`, keeping every text it was handed.
    struct TextRecorder {
        texts: Arc<std::sync::Mutex<Vec<String>>>,
    }
    #[async_trait::async_trait]
    impl Embedder for TextRecorder {
        async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            self.texts.lock().unwrap().push(text.to_string());
            FakeEmbedder::default().embed_with_role(text, role).await
        }
        fn dimensions(&self) -> usize { 384 }
        fn model_id(&self) -> &str { "text-recorder" }
        fn query_prefix(&self) -> &str { "" }
        fn passage_prefix(&self) -> &str { "" }
    }

    fn recorder() -> (Arc<dyn Embedder>, Arc<std::sync::Mutex<Vec<String>>>) {
        let texts = Arc::new(std::sync::Mutex::new(Vec::new()));
        (Arc::new(TextRecorder { texts: texts.clone() }), texts)
    }

    async fn piece_links(store: &MemoryStore) -> Vec<(i64, i64, i64, String)> {
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

    async fn content_of(store: &MemoryStore, id: i64) -> String {
        store
            .conn
            .call(move |c| c.query_row("SELECT content FROM memory WHERE id = ?1", [id], |r| r.get(0)))
            .await
            .unwrap()
    }

    fn long_save() -> String {
        format!("Notes on clawg-ledger, the crate that settles balances. {}", distinct_text(1000))
    }

    #[tokio::test]
    async fn a_single_piece_save_embeds_its_text_and_leaves_no_link() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let (embedder, texts) = recorder();

        let report = process_with(embedder, &store, &explicit_fact("the build uses cargo")).await;

        assert_eq!(report.memories.len(), 1);
        assert_eq!(*texts.lock().unwrap(), vec!["the build uses cargo".to_string()]);
        assert!(piece_links(&store).await.is_empty());
    }

    #[tokio::test]
    async fn later_pieces_are_embedded_with_the_head_and_stored_without_it() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let content = long_save();
        let SaveSplit { pieces, head } = ChatChunker::with_fallback().split_save(&content);
        let head = head.expect("the fixture is long");
        assert!(pieces.len() >= 3, "fixture must split into at least three pieces");
        let (embedder, texts) = recorder();

        let report = process_with(embedder, &store, &explicit_fact(&content)).await;

        assert_eq!(report.failure, None);
        let mut expected = vec![pieces[0].text.clone()];
        expected.extend(pieces[1..].iter().map(|p| format!("{head}\n…\n{}", p.text)));
        assert_eq!(*texts.lock().unwrap(), expected);
        for (saved, piece) in report.memories.iter().zip(&pieces) {
            assert_eq!(content_of(&store, saved.id).await, piece.text, "a stored row keeps only its piece");
        }
    }

    #[tokio::test]
    async fn a_long_save_links_its_pieces_in_order() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let content = long_save();
        let head = ChatChunker::with_fallback().split_save(&content).head.unwrap();

        let report = process_with(Arc::new(FakeEmbedder::default()), &store, &explicit_fact(&content)).await;

        let expected: Vec<(i64, i64, i64, String)> = report
            .memories
            .iter()
            .enumerate()
            .map(|(i, m)| (1, i as i64 + 1, m.id, if i == 0 { String::new() } else { head.clone() }))
            .collect();
        assert_eq!(piece_links(&store).await, expected);
    }

    #[tokio::test]
    async fn a_save_writes_its_session_on_every_row() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let long = MemoryEvent { session_id: Some("sess-1".into()), ..explicit_fact(&long_save()) };
        let report = process_with(Arc::new(FakeEmbedder::default()), &store, &long).await;
        assert!(report.memories.len() > 1, "the fixture splits into pieces");
        process_with(Arc::new(FakeEmbedder::default()), &store, &explicit_fact("no session behind this one")).await;

        let sessions: Vec<Option<String>> = store
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT source_session_id FROM memory ORDER BY id")?;
                let rows = stmt.query_map([], |r| r.get(0))?.collect();
                rows
            })
            .await
            .unwrap();
        let mut want = vec![Some("sess-1".to_string()); report.memories.len()];
        want.push(None);
        assert_eq!(sessions, want);
    }

    #[tokio::test]
    async fn an_absorbed_piece_is_linked_without_context() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let content = long_save();
        let SaveSplit { pieces, head } = ChatChunker::with_fallback().split_save(&content);
        let held = saved(&store, &explicit_fact(&pieces[1].text)).await[0].id;

        let report = process_with(Arc::new(FakeEmbedder::default()), &store, &explicit_fact(&content)).await;

        assert_eq!(report.memories[1], SavedChunk { id: held, outcome: ChunkOutcome::Reinforced });
        let links = piece_links(&store).await;
        assert_eq!(links[1], (1, 2, held, String::new()));
        assert_eq!(links[2].3, head.unwrap(), "an inserted piece still records its head");
    }

    #[tokio::test]
    async fn a_save_cut_short_leaves_no_link() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());

        let report = process_with(Arc::new(NanAtEmbedder { position: 1 }), &store, &explicit_fact(&long_save())).await;

        assert!(report.failure.is_some());
        assert!(piece_links(&store).await.is_empty());
    }

    #[tokio::test]
    async fn the_same_long_save_again_links_the_rows_it_left() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let content = long_save();
        let head = ChatChunker::with_fallback().split_save(&content).head.unwrap();
        let cut = process_with(Arc::new(NanAtEmbedder { position: 2 }), &store, &explicit_fact(&content)).await;
        assert_eq!(cut.memories.len(), 2, "the first two pieces were stored before the failure");

        let report = process_with(Arc::new(FakeEmbedder::default()), &store, &explicit_fact(&content)).await;
        process_with(Arc::new(FakeEmbedder::default()), &store, &explicit_fact(&content)).await;

        assert_eq!(report.failure, None);
        let links = piece_links(&store).await;
        assert_eq!(links.len(), report.memories.len(), "saving it a third time links nothing new");
        assert_eq!((links[0].2, links[1].2), (cut.memories[0].id, cut.memories[1].id));
        assert_eq!((links[1].3.as_str(), links[2].3.as_str()), ("", head.as_str()));
    }

    // ── Conclusions and their sources ───────────────────────────────────────

    async fn sources_of(store: &MemoryStore, id: i64) -> Vec<i64> {
        store
            .conn
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT source_id FROM memory_derivation WHERE memory_id = ?1 ORDER BY source_id",
                )?;
                let ids = stmt.query_map([id], |r| r.get(0))?.collect();
                ids
            })
            .await
            .unwrap()
    }

    fn conclusion(content: &str, sources: &[i64]) -> MemoryEvent {
        MemoryEvent { derived_from: sources.to_vec(), ..explicit_fact(content) }
    }

    async fn saved(store: &Arc<MemoryStore>, evt: &MemoryEvent) -> Vec<SavedChunk> {
        let report = process_with(Arc::new(FakeEmbedder::default()), store, evt).await;
        assert_eq!(report.failure, None);
        report.memories
    }

    #[tokio::test]
    async fn a_conclusion_is_linked_to_its_sources() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let a = saved(&store, &explicit_fact("the go service is indented with tabs")).await[0].id;
        let b = saved(&store, &explicit_fact("the rust crate is indented with tabs")).await[0].id;

        let c = saved(&store, &conclusion("the user indents with tabs everywhere", &[b, a, b])).await[0].id;

        assert_eq!(sources_of(&store, c).await, vec![a, b]);
        assert!(sources_of(&store, a).await.is_empty(), "a source gains no sources of its own");
    }

    #[tokio::test]
    async fn every_chunk_of_a_conclusion_is_linked() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let a = saved(&store, &explicit_fact("one thing the user said")).await[0].id;
        let b = saved(&store, &explicit_fact("another thing the user said")).await[0].id;
        let content = distinct_text(400);
        let pieces = chunk_count(&content);
        assert!(pieces >= 2, "fixture must split into several chunks, got {pieces}");

        let chunks = saved(&store, &conclusion(&content, &[a, b])).await;

        assert_eq!(chunks.len(), pieces);
        for chunk in chunks {
            assert_eq!(sources_of(&store, chunk.id).await, vec![a, b]);
        }
    }

    #[tokio::test]
    async fn a_reinforced_conclusion_gains_the_new_sources() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let a = saved(&store, &explicit_fact("one thing the user said")).await[0].id;
        let b = saved(&store, &explicit_fact("another thing the user said")).await[0].id;
        let c = saved(&store, &conclusion("what both of them mean", &[a])).await[0].id;

        let again = saved(&store, &conclusion("what both of them mean", &[b])).await;

        assert_eq!(again, vec![SavedChunk { id: c, outcome: ChunkOutcome::Reinforced }]);
        assert_eq!(sources_of(&store, c).await, vec![a, b]);
    }

    #[tokio::test]
    async fn a_row_is_never_its_own_source() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let a = saved(&store, &explicit_fact("the same words twice")).await[0].id;
        let b = saved(&store, &explicit_fact("something else entirely")).await[0].id;

        let again = saved(&store, &conclusion("the same words twice", &[a, b])).await;

        assert_eq!(again, vec![SavedChunk { id: a, outcome: ChunkOutcome::Reinforced }]);
        assert_eq!(sources_of(&store, a).await, vec![b]);
    }

    #[tokio::test]
    async fn a_conclusion_does_not_dispute_its_sources() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let place = |v: Vec<f32>| -> Arc<dyn Embedder> { Arc::new(PlacedEmbedder { vectors: vec![v] }) };
        let source = process_with(place(turned(1, 0.2)), &store, &explicit_fact("tabs in the go service")).await;
        let source = source.memories[0].id;
        let other = process_with(place(turned(2, 0.2)), &store, &explicit_fact("spaces in the python app")).await;
        let other = other.memories[0].id;

        let report = process_with(
            place(turned(1, 0.0)),
            &store,
            &conclusion("the user indents with tabs", &[source]),
        )
        .await;

        assert_eq!(report.failure, None);
        let conclusion = report.memories[0].id;
        let of_conclusion: Vec<_> = pairs(&store).await.into_iter().filter(|p| p.0 == conclusion).collect();
        assert_eq!(of_conclusion, vec![(conclusion, other, "pending".to_string())]);
    }

    #[tokio::test]
    async fn process_event_reports_the_rows_written_before_an_insert_failed() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let content = distinct_text(400);
        assert!(chunk_count(&content) >= 2, "fixture must split into several chunks");
        let evt = make_event(Role::User, &content, None, 1);

        let report = process_with(Arc::new(NanAtEmbedder { position: 1 }), &store, &evt).await;

        assert!(matches!(report.failure, Some(IngestFailure::Failed(_))), "got {:?}", report.failure);
        let first_id: i64 = store.conn
            .call(|c| c.query_row("SELECT id FROM memory", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(report.memories, vec![SavedChunk { id: first_id, outcome: ChunkOutcome::Inserted }]);
    }

    #[tokio::test]
    async fn process_event_stops_at_the_first_failed_insert() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let content = distinct_text(400);
        assert!(chunk_count(&content) >= 3, "fixture must leave a chunk after the failed one");
        let evt = make_event(Role::User, &content, None, 1);

        let report = process_with(Arc::new(NanAtEmbedder { position: 1 }), &store, &evt).await;

        assert!(report.failure.is_some());
        assert_eq!(report.memories.len(), 1);
        assert_eq!(count_memory(&store).await, 1, "the chunk after the failed one must not be inserted");
    }

    #[tokio::test]
    async fn process_event_reports_reinforced_with_the_existing_id() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let evt = make_event(Role::User, "the train leaves on tuesdays", None, 1);

        let first = process_with(Arc::new(FakeEmbedder::default()), &store, &evt).await;
        let second = process_with(Arc::new(FakeEmbedder::default()), &store, &evt).await;

        assert_eq!(first.failure, None);
        assert_eq!(first.memories.len(), 1);
        assert_eq!(first.memories[0].outcome, ChunkOutcome::Inserted);
        assert_eq!(
            second,
            IngestReport {
                memories: vec![SavedChunk { id: first.memories[0].id, outcome: ChunkOutcome::Reinforced }],
                failure: None,
                ..Default::default()
            }
        );
    }

    /// The dedup check runs before the embedding and the contradiction probe,
    /// and a restore of the same text can land while the save waits for the
    /// model.
    #[tokio::test]
    async fn a_twin_restored_while_the_save_is_embedded_is_reinforced_not_copied() {
        use std::sync::Mutex;
        use tokio::sync::oneshot;
        struct GatedEmbedder {
            reached: Mutex<Option<oneshot::Sender<()>>>,
            release: Mutex<Option<oneshot::Receiver<()>>>,
        }
        #[async_trait::async_trait]
        impl Embedder for GatedEmbedder {
            async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
                let reached = self.reached.lock().unwrap().take();
                if let Some(reached) = reached {
                    let _ = reached.send(());
                }
                let release = self.release.lock().unwrap().take();
                if let Some(release) = release {
                    let _ = release.await;
                }
                Ok(vec![0.1_f32; 384])
            }
            fn dimensions(&self) -> usize { 384 }
            fn model_id(&self) -> &str { "gated" }
            fn query_prefix(&self) -> &str { "" }
            fn passage_prefix(&self) -> &str { "" }
        }

        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let evt = make_event(Role::User, "the train leaves on tuesdays", None, 1);
        let twin = process_with(Arc::new(FakeEmbedder::default()), &store, &evt).await.memories[0].id;
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET deleted_at = 1 WHERE id = ?1", [twin]))
            .await
            .unwrap();

        let (reached_tx, reached_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let gated: Arc<dyn Embedder> = Arc::new(GatedEmbedder {
            reached: Mutex::new(Some(reached_tx)),
            release: Mutex::new(Some(release_rx)),
        });
        let saving = {
            let (store, evt) = (store.clone(), evt.clone());
            tokio::spawn(async move { process_with(gated, &store, &evt).await })
        };
        reached_rx.await.expect("the save reached the embedding");
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET deleted_at = NULL WHERE id = ?1", [twin]))
            .await
            .unwrap();
        release_tx.send(()).unwrap();
        let report = saving.await.unwrap();

        assert_eq!(report.memories, vec![SavedChunk { id: twin, outcome: ChunkOutcome::Reinforced }]);
        let live: i64 = store
            .conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM memory WHERE deleted_at IS NULL", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(live, 1, "one text, one live row");
    }

    #[tokio::test]
    async fn a_failed_embed_batch_reports_the_failure_and_no_rows() {
        struct FailingBatch;
        #[async_trait::async_trait]
        impl Embedder for FailingBatch {
            async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
                Err(EmbedError::InferenceFailed("boom".into()))
            }
            fn dimensions(&self) -> usize { 384 }
            fn model_id(&self) -> &str { "failing" }
            fn query_prefix(&self) -> &str { "" }
            fn passage_prefix(&self) -> &str { "" }
        }
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let evt = make_event(Role::User, "hello", None, 1);

        let report = process_with(Arc::new(FailingBatch), &store, &evt).await;

        assert!(report.memories.is_empty());
        assert!(matches!(report.failure, Some(IngestFailure::Failed(ref d)) if d.contains("boom")), "got {:?}", report.failure);
    }

    #[tokio::test]
    async fn an_unavailable_embedder_reports_embedder_unavailable() {
        struct Gone;
        #[async_trait::async_trait]
        impl Embedder for Gone {
            async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
                Err(EmbedError::WorkerGone)
            }
            fn dimensions(&self) -> usize { 384 }
            fn model_id(&self) -> &str { "gone" }
            fn query_prefix(&self) -> &str { "" }
            fn passage_prefix(&self) -> &str { "" }
        }
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let evt = make_event(Role::User, "hello", None, 1);

        let report = process_with(Arc::new(Gone), &store, &evt).await;

        assert_eq!(report, IngestReport { memories: vec![], failure: Some(IngestFailure::EmbedderUnavailable), ..Default::default() });
    }

    // Built at run time so no literal in the repository looks like a live
    // credential to a secret scanner.
    fn fake_key(prefix: &str, unit: &str, len: usize) -> String {
        format!("{prefix}{}", unit.chars().cycle().take(len).collect::<String>())
    }

    async fn stored_content(store: &MemoryStore, id: i64) -> String {
        store
            .conn
            .call(move |c| c.query_row("SELECT content FROM memory WHERE id = ?1", [id], |r| r.get(0)))
            .await
            .unwrap()
    }

    async fn fts_hits(store: &MemoryStore, term: &str) -> i64 {
        let phrase = format!("\"{term}\"");
        store
            .conn
            .call(move |c| {
                c.query_row("SELECT count(*) FROM memory_fts WHERE memory_fts MATCH ?1", [phrase], |r| r.get(0))
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_saved_secret_is_stored_masked_and_not_searchable() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let body = fake_key("", "a1B2", 36);
        let evt = explicit_fact(&format!("deploy uses ghp_{body}"));

        let report = process_with(Arc::new(FakeEmbedder::default()), &store, &evt).await;

        assert_eq!(report.failure, None);
        assert_eq!(report.redacted, vec![SecretKind::GithubToken]);
        assert_eq!(stored_content(&store, report.memories[0].id).await, "deploy uses [REDACTED:github_token]");
        assert_eq!(fts_hits(&store, &body).await, 0);
        assert_eq!(fts_hits(&store, "deploy").await, 1);
    }

    #[tokio::test]
    async fn saves_that_differ_only_in_a_masked_secret_are_one_memory() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let first = explicit_fact(&format!("the staging key is {}", fake_key("AKIA", "Q7ZX", 16)));
        let second = explicit_fact(&format!("the staging key is {}", fake_key("AKIA", "M4NP", 16)));

        let a = process_with(Arc::new(FakeEmbedder::default()), &store, &first).await;
        let b = process_with(Arc::new(FakeEmbedder::default()), &store, &second).await;

        assert_eq!(b.memories, vec![SavedChunk { id: a.memories[0].id, outcome: ChunkOutcome::Reinforced }]);
        let rows: i64 = store
            .conn
            .call(|c| c.query_row("SELECT count(*) FROM memory", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[tokio::test]
    async fn the_report_names_what_was_masked_even_when_the_save_fails() {
        struct Gone;
        #[async_trait::async_trait]
        impl Embedder for Gone {
            async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
                Err(EmbedError::WorkerGone)
            }
            fn dimensions(&self) -> usize { 384 }
            fn model_id(&self) -> &str { "gone" }
            fn query_prefix(&self) -> &str { "" }
            fn passage_prefix(&self) -> &str { "" }
        }
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let evt = explicit_fact(&format!("slack bot {}", fake_key("xoxb-", "12345-", 24)));

        let report = process_with(Arc::new(Gone), &store, &evt).await;

        assert_eq!(report.failure, Some(IngestFailure::EmbedderUnavailable));
        assert_eq!(report.redacted, vec![SecretKind::SlackToken]);
    }

    // ── Regression: the System→Skip guard still applies to non-Compaction events ──

    #[tokio::test]
    async fn system_role_with_chat_source_still_yields_zero_rows() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.try_send(IngestRequest::from(MemoryEvent {
            agent_id: "agent-test".to_string(),
            project_id: None,
            role: Role::System,
            content: "system prompt".to_string(),
            is_error: None,
            source: MemorySource::Chat,
            timestamp_ms: 1,
            memory_type: None,
            importance: None,
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
            session_id: None,
        })).unwrap();
        drain_worker(tx, handle).await;

        let rows = count_memory(&store).await;
        assert_eq!(rows, 0, "Role::System + source=Chat must still Skip; got {rows}");
    }

    // ── Compaction events bypass the System→Skip guard, producing 1 session_summary row ──

    #[tokio::test]
    async fn compaction_event_produces_one_session_summary_row() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        tx.try_send(IngestRequest::from(MemoryEvent {
            agent_id: "agent-test".to_string(),
            project_id: None,
            role: Role::System,
            content: "<summary>\nConversation summary:\n- Scope: 3 messages compacted.\n</summary>".to_string(),
            is_error: None,
            source: MemorySource::Compaction,
            timestamp_ms: 1,
            memory_type: None,
            importance: None,
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
            session_id: None,
        })).unwrap();
        drain_worker(tx, handle).await;

        let rows = count_memory(&store).await;
        assert_eq!(rows, 1, "Compaction source must bypass the System→Skip guard and insert exactly 1 row; got {rows}");

        let (mt, vec_count): (String, i64) = store.conn
            .call(|c| -> Result<(String, i64), rusqlite::Error> {
                let mt: String = c.query_row(
                    "SELECT memory_type FROM memory LIMIT 1",
                    [],
                    |r| r.get(0),
                )?;
                let vc: i64 = c.query_row(
                    "SELECT COUNT(*) FROM memory_vectors",
                    [],
                    |r| r.get(0),
                )?;
                Ok((mt, vc))
            })
            .await
            .unwrap();
        assert_eq!(mt, "session_summary", "expected memory_type='session_summary'; got {mt}");
        assert_eq!(vec_count, 1, "expected 1 matching memory_vectors row; got {vec_count}");
    }

    // ── Shared fixture: write two real messages, read them back ────────────

    use crate::memory::prompt_builder::{
        build_memory_prompt, QueryTexts, DEFAULT_BUDGET, SEP_LONGTERM_TEST, SEP_RECENT_TEST,
    };
    use crate::memory::search::{search_all_memories, RankedHit, SearchScope};
    use crate::memory::workspace::WorkspaceManager;
    use crate::models::chat::{current_time_millis, ChatMessage};

    /// State shared by the two gates below: one fresh message and one a month
    /// old, written through the real worker and read back through both
    /// `search_all_memories` and `build_memory_prompt`. Not a test itself and
    /// not gated behind either test's ignore reason — it exists so the write
    /// path runs once instead of once per assertion group.
    struct WrittenMemoryFixture {
        hits: Vec<RankedHit>,
        prompt: String,
        fresh: ChatMessage,
        stale: ChatMessage,
    }

    async fn write_fresh_and_stale_then_read_back() -> WrittenMemoryFixture {
        const AGENT: &str = "agent-invariant";
        const DAY_MS: u64 = 86_400_000;

        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        // Every text on one axis: the month-old message is as close to the
        // query as the fresh one, so both score and both reach the prompt.
        let embedder = Arc::new(PlacedEmbedder { vectors: vec![turned(1, 0.0); 2] });
        let (tx, handle) =
            spawn_test_worker(Arc::clone(&store), embedder.clone() as Arc<dyn Embedder>).await;

        let now_ms = current_time_millis();
        let fresh = ChatMessage {
            id: "msg-fresh".to_string(),
            role: Role::User,
            content: "el proyecto usa pnpm y no npm".to_string(),
            tool_calls: None,
            timestamp_ms: now_ms,
            is_error: None,
        };
        let stale = ChatMessage {
            id: "msg-stale".to_string(),
            role: Role::User,
            content: "el monorepo entero se instala con pnpm".to_string(),
            tool_calls: None,
            timestamp_ms: now_ms - 30 * DAY_MS,
            is_error: None,
        };

        for msg in [&fresh, &stale] {
            let evt = MemoryEvent::from_message(
                AGENT.to_string(), None, msg, MemorySource::Chat,
            )
            .expect("a user message with content must produce an ingest event");
            tx.try_send(IngestRequest::from(evt)).unwrap();
        }

        // Dropping the sender closes the channel; the worker's recv loop awaits each
        // insert inline, so it can only observe the closed channel after the last
        // write committed. Awaiting the handle is the barrier — no sleeping.
        drain_worker(tx, handle).await;

        let q_vec = embedder.embed_with_role(&fresh.content, EmbedRole::Query).await.unwrap();
        let hits = search_all_memories(
            &store, &q_vec, None, AGENT, SearchScope::all_default(), 10,
        )
        .await
        .unwrap();
        assert_eq!(
            hits.len(), 2,
            "both messages must come back as one row each; a different count means \
             the chunker split them, which changes what the rest of this test measures"
        );

        let tmp = crate::test_dir::test_dir();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let prompt = build_memory_prompt(
            crate::preferences::Lang::En,
            &store, &wm, Some(embedder.clone() as Arc<dyn Embedder>),
            AGENT, None, &crate::memory::sharing::Reach::Own, QueryTexts::single_shot(&fresh.content),
            &crate::memory::workspace::EmittedIds::default(), None, DEFAULT_BUDGET,
        )
        .await
        .unwrap().text;

        WrittenMemoryFixture { hits, prompt, fresh, stale }
    }

    // A written memory has to render with a date a human could read and land in
    // the right recency block. Nothing here names an instant: the two messages
    // are stamped by the same clock production uses, one of them a month back,
    // so the assertions cannot be satisfied by a fixture that happens to agree
    // with a broken unit.
    #[tokio::test]
    async fn e2e_real_write_path_produces_sane_date_and_buckets() {
        let fixture = write_fresh_and_stale_then_read_back().await;
        let mut violations: Vec<String> = Vec::new();

        // Every rendered hit opens with `[YYYY-MM-DD]`.
        let years: Vec<i32> = fixture
            .prompt
            .lines()
            .filter_map(|l| l.strip_prefix('['))
            .filter_map(|l| l.split('-').next())
            .filter_map(|y| y.parse::<i32>().ok())
            .collect();
        if years.len() != 2 {
            violations.push(format!("expected one rendered date per hit, got {years:?}"));
        }
        for y in &years {
            if !(2020..=2100).contains(y) {
                violations.push(format!("rendered year {y} is not a date anyone lived through"));
            }
        }

        match (
            fixture.prompt.find(SEP_RECENT_TEST),
            fixture.prompt.find(SEP_LONGTERM_TEST),
        ) {
            (Some(r), Some(l)) => {
                if !fixture.prompt[r..l].contains(&fixture.fresh.content) {
                    violations.push("the just-written message did not bucket as recent".into());
                }
                if !fixture.prompt[l..].contains(&fixture.stale.content) {
                    violations.push("the month-old message did not bucket as long-term".into());
                }
            }
            (Some(_), None) => violations.push(
                "every hit bucketed as recent — the month-old message never reached long-term".into(),
            ),
            (None, _) => violations.push("no recent block was rendered at all".into()),
        }

        assert!(
            violations.is_empty(),
            "unit invariants violated:\n  - {}",
            violations.join("\n  - ")
        );
    }

    // A written memory's score has to land inside its own contract: never
    // negative, never above one. The similarity factor is
    // `cosine_from_l2_unit(distance)` (score.rs), which converts the raw L2
    // distance between two UNIT vectors into exact cosine similarity and
    // clamps it to `MAX(0.0, 1.0 - distance^2/2)` — both edges (insert and
    // query) route through `to_unit` (vector.rs) before a distance is ever
    // measured, so the similarity factor is structurally bounded in [0,1]
    // rather than merely bounded for embeddings that happen to be close
    // together. The default config additionally re-scales that similarity
    // factor against a measured floor
    // (`(cos-floor)/(1-floor)`, clamped to `[0,1]`), and a candidate whose
    // raw similarity falls below that floor clamps all the way down to
    // exactly `0.0` by design — that is the floor doing its job, not a
    // formula error, so the lower bound this test can actually promise is
    // `0.0` inclusive, not a strictly-positive one. The remaining three
    // multiplicative factors (decay, importance, dispute) are each already
    // clamped to `[0,1]` on their own, so the product never exceeds `1.0`
    // regardless. This end-to-end test exercises a real write followed by a
    // real read to prove the whole pipeline honors the bound, not just the
    // formula in isolation.
    #[tokio::test]
    async fn e2e_real_write_path_produces_bounded_score() {
        let fixture = write_fresh_and_stale_then_read_back().await;
        let mut violations: Vec<String> = Vec::new();

        for h in &fixture.hits {
            if !(h.score >= 0.0 && h.score <= 1.0) {
                violations.push(format!("score {} is outside [0.0, 1.0]", h.score));
            }
        }

        assert!(
            violations.is_empty(),
            "score contract violated:\n  - {}",
            violations.join("\n  - ")
        );
    }

    // ── Ingest queue depth: pinned constant + pure edge-detection helper ────

    #[test]
    fn ingest_queue_warn_depth_is_64() {
        assert_eq!(INGEST_QUEUE_WARN_DEPTH, 64);
    }

    #[test]
    fn queue_depth_transition_below_threshold_not_over_is_none() {
        assert_eq!(queue_depth_transition(63, false), None);
    }

    #[test]
    fn queue_depth_transition_at_threshold_not_over_is_rising() {
        assert_eq!(queue_depth_transition(64, false), Some(QueueDepthEdge::Rising));
    }

    #[test]
    fn queue_depth_transition_at_threshold_already_over_is_none() {
        assert_eq!(queue_depth_transition(64, true), None);
    }

    #[test]
    fn queue_depth_transition_above_threshold_already_over_is_none() {
        assert_eq!(queue_depth_transition(65, true), None);
    }

    #[test]
    fn queue_depth_transition_below_threshold_already_over_is_falling() {
        assert_eq!(queue_depth_transition(63, true), Some(QueueDepthEdge::Falling));
    }

    // ── collapse_duplicate_chunks (pure) ─────────────────────────────────────

    fn make_chunk(text: &str) -> Chunk {
        Chunk { text: text.to_string() }
    }

    #[test]
    fn collapse_duplicate_chunks_keeps_first_occurrence_order() {
        let chunks = vec![make_chunk("a"), make_chunk("b"), make_chunk("a")];
        let out = collapse_duplicate_chunks(chunks);
        assert_eq!(out.len(), 2, "expected 2 collapsed entries");
        assert_eq!(out[0].chunk.text, "a");
        assert_eq!(out[1].chunk.text, "b");
    }

    #[test]
    fn collapse_duplicate_chunks_counts_repeat_copies() {
        let chunks = vec![make_chunk("a"), make_chunk("a"), make_chunk("a")];
        let out = collapse_duplicate_chunks(chunks);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].multiplicity, 3, "three identical chunks must collapse to multiplicity 3");
    }

    #[test]
    fn collapse_duplicate_chunks_of_all_distinct_is_identity() {
        let chunks = vec![make_chunk("a"), make_chunk("b"), make_chunk("c")];
        let out = collapse_duplicate_chunks(chunks);
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|p| p.multiplicity == 1), "all-distinct input must leave every multiplicity at 1");
    }

    // ── process_event: one embed_batch call per event ────────────────────────

    /// `process_event` must route a multi-chunk event through exactly one
    /// `embed_batch` call, never per-chunk `embed_with_role` calls.
    #[tokio::test]
    async fn multi_chunk_event_issues_one_embed_batch_call() {
        struct BatchRecordingEmbedder {
            batch_calls: Arc<std::sync::Mutex<Vec<usize>>>,
        }
        #[async_trait::async_trait]
        impl Embedder for BatchRecordingEmbedder {
            async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
                unreachable!("process_event must call embed_batch, not embed_with_role, for chunked events");
            }
            async fn embed_batch(&self, texts: &[String], _role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
                self.batch_calls.lock().unwrap().push(texts.len());
                Ok(texts.iter().map(|_| vec![0.1_f32; 384]).collect())
            }
            fn dimensions(&self) -> usize {
                384
            }
            fn model_id(&self) -> &str {
                "batch-recording"
            }
            fn query_prefix(&self) -> &str {
                ""
            }
            fn passage_prefix(&self) -> &str {
                ""
            }
        }

        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let batch_calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let embedder: Arc<dyn Embedder> =
            Arc::new(BatchRecordingEmbedder { batch_calls: batch_calls.clone() });
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        // Every word differs, so no chunk absorbs another and all of them
        // survive to the embed stage.
        let content = distinct_text(400);
        let chunks = chunk_count(&content);
        tx.try_send(IngestRequest::from(make_event(Role::User, &content, None, 1))).unwrap();
        drain_worker(tx, handle).await;

        let calls = batch_calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "expected exactly one embed_batch call for the whole event, got {calls:?}");
        assert_eq!(calls[0], chunks, "expected the single call to carry all {chunks} chunks, got {calls:?}");
    }

    /// Event `[A, …, A]`, not present in the store: stage 1 collapses to one
    /// survivor with multiplicity n, stage 3 inserts it once then replays the
    /// reinforce path n-1 times — 1 row, access_count=n, absorbed=n-1. Same
    /// externally observable outcome as two identical chunks arriving in
    /// separate events.
    #[tokio::test]
    async fn intra_event_duplicate_chunks_insert_one_row_and_reinforce() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let cell: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
        cell.set(embedder).ok();
        let cell = Arc::new(cell);
        let chat_chunker = Arc::new(ChatChunker::with_fallback());

        let mut absorbed: u64 = 0;
        // Too long for one piece, so it is cut at the piece cap: 2112 bytes =
        // cap(384) + 6 strides(288) gives exactly seven windows, all 384
        // bytes of the same repeated char — i.e. seven byte-identical chunks,
        // no partial one.
        let content = "x".repeat(2112);
        assert_eq!(chunk_count(&content), 7);
        let evt = make_event(Role::User, &content, None, 1);

        process_event(&evt, &store, &cell, &chat_chunker, &mut absorbed)
            .await;

        let rows = count_memory(&store).await;
        assert_eq!(rows, 1, "identical chunks in one event must insert exactly one row; got {rows}");
        assert_eq!(absorbed, 6, "every other copy absorbed within the event");

        let access_count: i64 = store.conn
            .call(|c| c.query_row("SELECT access_count FROM memory LIMIT 1", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(access_count, 7, "access_count must be insert(1) + intra-event reinforce replays(6) = 7");
    }

    /// Event `[A, …, A]` where `A` is ALREADY in the store (from a prior event):
    /// collapsing first and gating once per survivor would under-count —
    /// today's per-chunk loop hits the gate twice for this input. Both copies
    /// must be replayed through `absorb_duplicate`, so `absorbed` must move
    /// by the number of copies, not 1.
    #[tokio::test]
    async fn existing_duplicate_chunks_absorb_once_per_copy_not_once_per_event() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let cell: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
        cell.set(embedder).ok();
        let cell = Arc::new(cell);
        let chat_chunker = Arc::new(ChatChunker::with_fallback());

        // Seed the store with ONE row for chunk A ("x" * 384), which fits
        // in one piece and is stored whole.
        let mut absorbed: u64 = 0;
        let seed_evt = make_event(Role::User, &"x".repeat(384), None, 1);
        process_event(&seed_evt, &store, &cell, &chat_chunker, &mut absorbed)
            .await;
        assert_eq!(absorbed, 0, "seed insert has no prior row to reinforce");
        assert_eq!(count_memory(&store).await, 1);

        // Now send an event carrying seven copies of the already-stored chunk
        // A. 2112 bytes (piece cap + 6 strides), not a byte more: with
        // overlap a longer run would produce a shorter window distinct from
        // A — a brand-new chunk this test isn't about. 2112 keeps the fixture
        // to exactly seven 384-byte windows, all == A.
        absorbed = 0;
        let dup_evt = make_event(Role::User, &"x".repeat(2112), None, 2);
        process_event(&dup_evt, &store, &cell, &chat_chunker, &mut absorbed)
            .await;

        assert_eq!(
            absorbed, 7,
            "every copy of an already-stored chunk must be absorbed individually, not collapsed into one absorb"
        );
        assert_eq!(count_memory(&store).await, 1, "no new row inserted for an already-present chunk");

        let access_count: i64 = store.conn
            .call(|c| c.query_row("SELECT access_count FROM memory LIMIT 1", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(access_count, 8, "seed insert(1) + seven reinforce hits(+7) = 8");
    }

    /// Integration-level counterpart of the unit-level boundary test in
    /// `token_splitter.rs`: a marker straddling the old (pre-overlap)
    /// 0-overlap cut point must come out whole in one persisted `memory`
    /// row, not just inside one in-memory `Chunk`.
    #[tokio::test]
    async fn boundary_straddling_span_survives_whole_in_one_persisted_row() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        // The old splitter cut at multiples of 1920 bytes with zero overlap;
        // this marker straddles the first such cut point.
        let mut content = "a".repeat(1900);
        content.push_str("MARKER_SPANNING_BOUNDARY");
        content.push_str(&"b".repeat(2000));
        tx.try_send(IngestRequest::from(make_event(Role::User, &content, None, 1))).unwrap();
        drain_worker(tx, handle).await;

        let hits: i64 = store
            .conn
            .call(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM memory WHERE content LIKE '%MARKER_SPANNING_BOUNDARY%'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(
            hits, 1,
            "marker straddling the old cut point must survive whole in exactly one persisted row"
        );
    }

    /// `InferenceFailed` from `embed_batch` must drop the WHOLE event, not
    /// skip one chunk and continue — a deliberate behavior change from the
    /// old per-chunk skip semantics (which would have left 4-of-5 chunks
    /// inserted here).
    #[tokio::test]
    async fn batch_embed_failure_drops_the_whole_event() {
        struct FailingBatchEmbedder;
        #[async_trait::async_trait]
        impl Embedder for FailingBatchEmbedder {
            async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
                Err(EmbedError::InferenceFailed("boom".to_string()))
            }
            async fn embed_batch(&self, _texts: &[String], _role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
                Err(EmbedError::InferenceFailed("boom".to_string()))
            }
            fn dimensions(&self) -> usize {
                384
            }
            fn model_id(&self) -> &str {
                "failing-batch"
            }
            fn query_prefix(&self) -> &str {
                ""
            }
            fn passage_prefix(&self) -> &str {
                ""
            }
        }

        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(FailingBatchEmbedder);
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        let content = format!(
            "{}{}{}{}{}",
            "a".repeat(1920), "b".repeat(1920), "c".repeat(1920), "d".repeat(1920), "e".repeat(1920)
        );
        tx.try_send(IngestRequest::from(make_event(Role::User, &content, None, 1))).unwrap();
        drain_worker(tx, handle).await;

        let rows = count_memory(&store).await;
        assert_eq!(
            rows, 0,
            "InferenceFailed from embed_batch must drop the whole event, not insert 4-of-5 chunks; got {rows}"
        );
    }

    /// An `embed_batch` override that returns one FEWER vector than requested
    /// — the shape a real batch backend produces on a partial reply. This
    /// must never reach the zip: `process_event`'s own length check is the
    /// only guard standing between a short reply and a chunk silently paired
    /// with another chunk's vector.
    #[tokio::test]
    async fn embed_batch_returning_too_few_vectors_drops_the_event_not_mispairs_it() {
        struct ShortReplyEmbedder;
        #[async_trait::async_trait]
        impl Embedder for ShortReplyEmbedder {
            async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
                unreachable!("process_event must call embed_batch for chunked events");
            }
            async fn embed_batch(&self, texts: &[String], _role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
                // One short: a real backend that dropped or merged one item
                // mid-batch would look exactly like this.
                Ok(texts.iter().skip(1).map(|_| vec![0.1_f32; 384]).collect())
            }
            fn dimensions(&self) -> usize {
                384
            }
            fn model_id(&self) -> &str {
                "short-reply"
            }
            fn query_prefix(&self) -> &str {
                ""
            }
            fn passage_prefix(&self) -> &str {
                ""
            }
        }

        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(ShortReplyEmbedder);
        let (tx, handle) = spawn_test_worker(Arc::clone(&store), embedder).await;

        // Three DISTINCT chunks so all three survive collapse and reach the
        // embed stage; the mock then replies with only 2 vectors.
        let content = format!("{}{}{}", "a".repeat(1920), "b".repeat(1920), "c".repeat(1920));
        tx.try_send(IngestRequest::from(make_event(Role::User, &content, None, 1))).unwrap();
        drain_worker(tx, handle).await;

        let rows = count_memory(&store).await;
        assert_eq!(
            rows, 0,
            "a short embed_batch reply must drop the whole event, never zip a chunk to another chunk's vector; got {rows} rows"
        );
    }

    // ── Shared tokenizer lifecycle: cell + chunker come from ONE load ────────

    /// Writes a real, minimal `tokenizer.json` under `base_dir` — a
    /// `WordLevel` model with a tiny vocab and a whitespace pre-tokenizer, so
    /// `Tokenizer::from_file` genuinely parses it and `encode` genuinely
    /// splits on words (unlike the byte-length fallback this proves the code
    /// is NOT taking).
    fn write_minimal_tokenizer_fixture(base_dir: &std::path::Path) {
        use tokenizers::models::wordlevel::WordLevel;
        use tokenizers::pre_tokenizers::whitespace::Whitespace;

        let target = crate::memory::model_download::tokenizer_path(base_dir);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let vocab_path = target.parent().unwrap().join("vocab.json");
        std::fs::write(&vocab_path, r#"{"[UNK]":0,"hello":1,"world":2}"#).unwrap();

        let model = WordLevel::from_file(vocab_path.to_str().unwrap(), "[UNK]".to_string())
            .expect("fixture vocab must build a WordLevel model");
        let mut tok = tokenizers::Tokenizer::new(model);
        tok.with_pre_tokenizer(Some(Whitespace));
        tok.save(&target, false).expect("fixture tokenizer must save to tokenizer_path");
    }

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "clawg-worker-test-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_tokenizer_and_chunker_fills_the_cell_and_shares_the_identical_arc() {
        let base_dir = unique_temp_dir("resolve-tok");
        write_minimal_tokenizer_fixture(&base_dir);

        let cell: Arc<OnceCell<Arc<tokenizers::Tokenizer>>> = Arc::new(OnceCell::new());
        let chunker = resolve_tokenizer_and_chunker(&base_dir, &cell, crate::memory::model_download::load_tokenizer);

        let cell_tok = cell.get().expect("cell must be filled when tokenizer.json is present");
        let cell_ptr: *const tokenizers::Tokenizer = Arc::as_ptr(cell_tok);
        assert_eq!(
            chunker.tokenizer_ptr(),
            Some(cell_ptr),
            "the cell and the chunker must share the exact same Arc — one load, not two independent parses"
        );

        // Behavioral confirmation this is really the tokenizer path, not the
        // byte-length fallback: "hello world" is 2 words in the fixture
        // vocab, so a max_tokens=1 cap must split it into 2 chunks.
        let chunks = chunker.chunk("hello world", 1);
        assert_eq!(chunks.len(), 2, "a real word-level tokenizer must split 2 words at max_tokens=1");

        std::fs::remove_dir_all(&base_dir).ok();
    }

    /// A new install starts the worker before the first model download lands,
    /// and the process can then run for days.
    #[test]
    fn the_chunker_splits_by_tokens_once_the_tokenizer_loads_after_it() {
        let base_dir = unique_temp_dir("resolve-tok-late");
        let cell: Arc<OnceCell<Arc<tokenizers::Tokenizer>>> = Arc::new(OnceCell::new());
        let chunker = resolve_tokenizer_and_chunker(&base_dir, &cell, |_: &std::path::Path| None);
        assert!(cell.get().is_none(), "nothing on disk yet");

        write_minimal_tokenizer_fixture(&base_dir);
        let tokenizer = crate::memory::model_download::load_tokenizer(&base_dir).expect("the fixture loads");
        cell.set(tokenizer).ok();

        let chunks = chunker.chunk("hello world", 1);
        assert_eq!(chunks.len(), 2, "once the model loaded, the same chunker must split by tokens");

        std::fs::remove_dir_all(&base_dir).ok();
    }

    #[test]
    fn resolve_tokenizer_and_chunker_leaves_the_cell_empty_when_the_file_is_absent() {
        let base_dir = unique_temp_dir("resolve-tok-absent");
        let cell: Arc<OnceCell<Arc<tokenizers::Tokenizer>>> = Arc::new(OnceCell::new());
        let chunker = resolve_tokenizer_and_chunker(&base_dir, &cell, crate::memory::model_download::load_tokenizer);

        assert!(cell.get().is_none(), "no tokenizer.json on disk must leave the cell empty");
        assert!(chunker.tokenizer_ptr().is_none(), "the chunker must be running the byte fallback");

        std::fs::remove_dir_all(&base_dir).ok();
    }

    #[test]
    fn a_tokenizer_file_with_another_sha256_is_not_parsed() {
        let base_dir = unique_temp_dir("resolve-tok-other-sha");
        write_minimal_tokenizer_fixture(&base_dir);

        let parses = std::cell::Cell::new(0u32);
        let cell: Arc<OnceCell<Arc<tokenizers::Tokenizer>>> = Arc::new(OnceCell::new());
        let chunker = resolve_tokenizer_and_chunker(&base_dir, &cell, |dir| {
            crate::memory::model_download::load_verified_tokenizer_with(dir, |bytes| {
                parses.set(parses.get() + 1);
                tokenizers::Tokenizer::from_bytes(bytes).ok()
            })
        });

        assert_eq!(parses.get(), 0, "a file that is not the pinned one must never reach the parser");
        assert!(cell.get().is_none(), "the cell must stay empty");
        assert!(chunker.tokenizer_ptr().is_none(), "the chunker must be running the byte fallback");

        std::fs::remove_dir_all(&base_dir).ok();
    }

    // ── Drain decision ───────────────────────────────────────────────────────

    #[test]
    fn without_a_drain_the_worker_keeps_ingesting() {
        let now = Instant::now();
        assert_eq!(drain_step(now, None, true), DrainStep::Ingest);
        assert_eq!(drain_step(now, None, false), DrainStep::Ingest);
    }

    #[test]
    fn a_drain_keeps_ingesting_until_its_deadline() {
        let now = Instant::now();
        let later = now + std::time::Duration::from_secs(1);
        assert_eq!(drain_step(now, Some(later), true), DrainStep::Ingest);
        assert_eq!(drain_step(later, Some(later), true), DrainStep::Spill);
        assert_eq!(drain_step(later + std::time::Duration::from_millis(1), Some(later), true), DrainStep::Spill);
    }

    #[test]
    fn a_drain_without_an_embedder_spills_at_once() {
        let now = Instant::now();
        let later = now + std::time::Duration::from_secs(10);
        assert_eq!(drain_step(now, Some(later), false), DrainStep::Spill);
    }

    // ── Saves recorded in the outbox ─────────────────────────────────────────

    async fn record(store: &MemoryStore, content: &str) -> i64 {
        let evt = make_event(Role::User, content, None, 1);
        match store.outbox_record(uuid::Uuid::new_v4().to_string(), evt).await.unwrap() {
            crate::memory::outbox::Recorded::New { row } => row,
            other => panic!("expected a new row, got {other:?}"),
        }
    }

    /// Runs the worker over `replay` and `requests` until both are drained.
    async fn run_worker(
        store: &Arc<MemoryStore>,
        embedder: Arc<dyn Embedder>,
        replay: Replay,
        requests: Vec<IngestRequest>,
    ) {
        let cell: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
        cell.set(embedder).ok();
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        for request in requests {
            tx.try_send(request).unwrap();
        }
        drop(tx);
        let chat_chunker = Arc::new(ChatChunker::with_fallback());
        memory_worker(rx, no_drain(), replay, store.clone(), Arc::new(cell), chat_chunker).await;
    }

    async fn replay_pending(store: &Arc<MemoryStore>, embedder: Arc<dyn Embedder>) {
        let rows = store.outbox_pending().await.unwrap();
        run_worker(store, embedder, Replay::rows(rows), Vec::new()).await;
    }

    async fn outbox_row(store: &MemoryStore, row: i64) -> Option<(String, i64, Option<String>, Vec<NotedPiece>)> {
        use rusqlite::OptionalExtension;
        store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT state, attempts, last_error, progress FROM ingest_outbox WHERE id = ?1",
                    [row],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, String>(3)?)),
                )
                .optional()
            })
            .await
            .unwrap()
            .map(|(state, attempts, error, progress)| {
                (state, attempts, error, serde_json::from_str(&progress).unwrap())
            })
    }

    async fn access_counts(store: &MemoryStore) -> Vec<i64> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT access_count FROM memory ORDER BY id")?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                rows.collect::<Result<Vec<i64>, _>>()
            })
            .await
            .unwrap()
    }

    async fn memory_ids(store: &MemoryStore) -> Vec<i64> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT id FROM memory ORDER BY id")?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                rows.collect::<Result<Vec<i64>, _>>()
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn each_piece_is_noted_with_the_row_it_wrote() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let content = long_save();
        assert!(chunk_count(&content) >= 3, "fixture must split into three pieces or more");
        let row = record(&store, &content).await;

        replay_pending(&store, Arc::new(NanAtEmbedder { position: 2 })).await;

        let (state, attempts, error, progress) = outbox_row(&store, row).await.expect("the row stays");
        assert_eq!((state.as_str(), attempts), ("pending", 1));
        assert!(error.unwrap().starts_with("store insert failed"));
        let ids = memory_ids(&store).await;
        assert_eq!(progress.iter().map(|p| (p.position, p.id, p.inserted)).collect::<Vec<_>>(), vec![
            (0, ids[0], true),
            (1, ids[1], true),
        ]);
        assert_eq!(progress[0].context, "");
        assert!(!progress[1].context.is_empty(), "the second piece is embedded behind the head");
        assert!(piece_links(&store).await.is_empty(), "an unfinished save is not linked");
    }

    #[tokio::test]
    async fn a_replayed_save_resumes_after_its_last_noted_piece() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let content = long_save();
        let pieces = chunk_count(&content);
        let row = record(&store, &content).await;
        replay_pending(&store, Arc::new(NanAtEmbedder { position: 2 })).await;
        let head = outbox_row(&store, row).await.unwrap().3[1].context.clone();

        replay_pending(&store, Arc::new(FakeEmbedder::default())).await;

        assert_eq!(outbox_row(&store, row).await, None, "a finished save leaves no row");
        assert_eq!(access_counts(&store).await, vec![1; pieces], "no piece is reinforced by the replay");
        let links = piece_links(&store).await;
        assert_eq!(links.len(), pieces);
        let ids = memory_ids(&store).await;
        for (i, (save_id, piece, memory_id, context)) in links.into_iter().enumerate() {
            assert_eq!((save_id, piece, memory_id), (1, i as i64 + 1, ids[i]));
            assert_eq!(context, if i == 0 { String::new() } else { head.clone() });
        }
    }

    #[tokio::test]
    async fn progress_that_does_not_fit_the_split_is_dropped() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let row = record(&store, "a short save of one piece").await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE ingest_outbox SET progress = \
                     '[{\"position\":4,\"hash\":\"x\",\"id\":999,\"inserted\":true,\"context\":\"\"}]' \
                     WHERE id = ?1",
                    [row],
                )
            })
            .await
            .unwrap();

        replay_pending(&store, Arc::new(FakeEmbedder::default())).await;

        assert_eq!(count_memory(&store).await, 1);
        assert_eq!(outbox_row(&store, row).await, None);
    }

    #[tokio::test]
    async fn a_row_delivered_twice_is_ingested_once() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let row = record(&store, "delivered by the start-up read and the channel").await;
        let rows = store.outbox_pending().await.unwrap();
        let again = IngestRequest { event: rows[0].1.clone(), reply: None, outbox: Some(row) };

        run_worker(&store, Arc::new(FakeEmbedder::default()), Replay::rows(rows), vec![again]).await;

        assert_eq!(access_counts(&store).await, vec![1]);
        assert_eq!(outbox_row(&store, row).await, None);
    }

    #[test]
    fn only_the_channel_copy_of_a_start_up_row_is_a_second_delivery() {
        let startup = StartupRows::of(&Replay::rows(vec![(7, make_event(Role::User, "read at start", None, 1))]));

        assert!(!startup.is_second_delivery(7, true), "the replay's own delivery is the first");
        assert!(startup.is_second_delivery(7, false), "its copy on the channel is the second");
        assert!(!startup.is_second_delivery(8, false), "a save accepted after the read arrives once");
    }

    #[tokio::test]
    async fn a_row_that_stopped_the_process_three_times_fails_and_the_next_one_runs() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let poison = record(&store, "the save that kills the process").await;
        let next = record(&store, "the save after it").await;
        store
            .conn
            .call(move |c| c.execute("UPDATE ingest_outbox SET attempts = 3 WHERE id = ?1", [poison]))
            .await
            .unwrap();

        replay_pending(&store, Arc::new(FakeEmbedder::default())).await;

        let (state, _, error, _) = outbox_row(&store, poison).await.expect("a failed row stays");
        assert_eq!(state, "failed");
        assert_eq!(error.as_deref(), Some("the daemon stopped 3 times while ingesting this save"));
        assert_eq!(outbox_row(&store, next).await, None);
        assert_eq!(count_memory(&store).await, 1);
    }

    #[tokio::test]
    async fn a_failure_after_the_answer_stays_in_the_outbox_with_its_cause() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let row = record(&store, "accepted while the model loaded").await;

        replay_pending(&store, Arc::new(NanAtEmbedder { position: 0 })).await;

        let (state, attempts, error, progress) = outbox_row(&store, row).await.expect("the row stays");
        assert_eq!((state.as_str(), attempts, progress.len()), ("pending", 1, 0));
        assert!(error.unwrap().starts_with("store insert failed"));
        assert_eq!(count_memory(&store).await, 0);
    }

    // ── A save the embedder could not take is retried ────────────────────────

    #[test]
    fn retry_delay_doubles_up_to_a_minute() {
        let secs = |n| retry_delay(n).as_secs();
        assert_eq!((secs(0), secs(1), secs(2), secs(5)), (1, 2, 4, 32));
        assert_eq!((secs(6), secs(7), secs(40)), (60, 60, 60));
    }

    /// Not ready for the first `failures` batches, then the fake model.
    struct FlakyEmbedder {
        failures: std::sync::atomic::AtomicUsize,
    }
    impl FlakyEmbedder {
        fn failing(times: usize) -> Arc<Self> {
            Arc::new(Self { failures: std::sync::atomic::AtomicUsize::new(times) })
        }
    }
    #[async_trait::async_trait]
    impl Embedder for FlakyEmbedder {
        async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            FakeEmbedder::default().embed_with_role(text, role).await
        }
        async fn embed_batch(&self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
            use std::sync::atomic::Ordering;
            if self.failures.load(Ordering::SeqCst) > 0 {
                self.failures.fetch_sub(1, Ordering::SeqCst);
                return Err(EmbedError::NotReady);
            }
            FakeEmbedder::default().embed_batch(texts, role).await
        }
        fn dimensions(&self) -> usize { 384 }
        fn model_id(&self) -> &str { "flaky" }
        fn query_prefix(&self) -> &str { "" }
        fn passage_prefix(&self) -> &str { "" }
    }

    async fn record_event(store: &MemoryStore, evt: MemoryEvent) -> i64 {
        match store.outbox_record(uuid::Uuid::new_v4().to_string(), evt).await.unwrap() {
            crate::memory::outbox::Recorded::New { row } => row,
            other => panic!("expected a new row, got {other:?}"),
        }
    }

    /// A worker over the pending rows whose channel stays open, as the
    /// daemon's does, with the sender of its drain signal.
    fn spawn_replaying(
        store: &Arc<MemoryStore>,
        embedder: Arc<dyn Embedder>,
        rows: Vec<(i64, MemoryEvent)>,
    ) -> (Sender<IngestRequest>, watch::Sender<Option<Instant>>, JoinHandle<Vec<MemoryEvent>>) {
        let cell: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
        cell.set(embedder).ok();
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let (drain_tx, drain_rx) = watch::channel(None);
        let worker = tokio::spawn(memory_worker(
            rx,
            drain_rx,
            Replay::rows(rows),
            store.clone(),
            Arc::new(cell),
            Arc::new(ChatChunker::with_fallback()),
        ));
        (tx, drain_tx, worker)
    }

    async fn wait_until_gone(store: &MemoryStore, row: i64) {
        for _ in 0..100 {
            if outbox_row(store, row).await.is_none() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("outbox row {row} was not finished within 5 s: {:?}", outbox_row(store, row).await);
    }

    #[tokio::test]
    async fn a_transient_failure_is_retried_in_the_same_run() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let row = record(&store, "saved while the model could not load").await;
        let rows = store.outbox_pending().await.unwrap();
        let (tx, _drain, worker) = spawn_replaying(&store, FlakyEmbedder::failing(1), rows);

        wait_until_gone(&store, row).await;

        assert_eq!(count_memory(&store).await, 1);
        drop(tx);
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn a_save_behind_a_retried_one_waits_for_it() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let first = record_event(&store, keyed("Storage engine is SQLite\nOne local file.", "storage/engine")).await;
        let second = record_event(&store, keyed("Storage engine is Postgres\nConcurrent writers.", "storage/engine")).await;
        let rows = store.outbox_pending().await.unwrap();
        let (tx, _drain, worker) = spawn_replaying(&store, FlakyEmbedder::failing(1), rows);

        wait_until_gone(&store, first).await;
        wait_until_gone(&store, second).await;

        let live = live_with_key(&store, "storage/engine").await;
        assert_eq!(live.len(), 1, "{live:?}");
        assert!(content_of(&store, live[0]).await.starts_with("Storage engine is Postgres"), "the later save is the live one");
        drop(tx);
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn a_drain_during_a_retry_leaves_the_row_pending() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let row = record(&store, "saved while the model never came back").await;
        let rows = store.outbox_pending().await.unwrap();
        let (_tx, drain, worker) = spawn_replaying(&store, FlakyEmbedder::failing(usize::MAX), rows);
        for _ in 0..100 {
            if outbox_row(&store, row).await.is_some_and(|r| r.2.is_some()) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        drain.send(Some(Instant::now() + std::time::Duration::from_secs(10))).unwrap();
        tokio::time::timeout(std::time::Duration::from_millis(500), worker)
            .await
            .expect("a drain ends the wait for the retry")
            .unwrap();

        let (state, attempts, error, _) = outbox_row(&store, row).await.expect("the row stays");
        assert_eq!((state.as_str(), attempts), ("pending", 0));
        assert_eq!(error.as_deref(), Some("the embedder is not available"));
    }

    // ── A drain pauses a long save between pieces ───────────────────────────

    #[test]
    fn a_pause_is_due_once_the_drain_deadline_passes() {
        let now = Instant::now();
        assert!(!pause_due(now, None));
        assert!(!pause_due(now, Some(now + std::time::Duration::from_secs(1))));
        assert!(pause_due(now, Some(now)));
    }

    /// Embeds like the fake model once released, and says when it was asked.
    struct GatedBatch {
        asked: watch::Sender<bool>,
        release: tokio::sync::Semaphore,
    }
    #[async_trait::async_trait]
    impl Embedder for GatedBatch {
        async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            FakeEmbedder::default().embed_with_role(text, role).await
        }
        async fn embed_batch(&self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
            self.asked.send_replace(true);
            self.release.acquire().await.expect("the gate stays open").forget();
            FakeEmbedder::default().embed_batch(texts, role).await
        }
        fn dimensions(&self) -> usize { 384 }
        fn model_id(&self) -> &str { "gated-batch" }
        fn query_prefix(&self) -> &str { "" }
        fn passage_prefix(&self) -> &str { "" }
    }

    #[tokio::test]
    async fn a_drain_pauses_a_long_save_between_pieces() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let content = long_save();
        let pieces = chunk_count(&content);
        assert!(pieces >= 3, "fixture must split into three pieces or more");
        let row = record(&store, &content).await;
        replay_pending(&store, Arc::new(NanAtEmbedder { position: 2 })).await;
        let before = outbox_row(&store, row).await.unwrap();
        assert_eq!((before.1, before.3.len()), (1, 2), "two pieces stored by the first take");

        let gate = Arc::new(GatedBatch { asked: watch::Sender::new(false), release: tokio::sync::Semaphore::new(0) });
        let mut asked = gate.asked.subscribe();
        let rows = store.outbox_pending().await.unwrap();
        let (_tx, drain, worker) = spawn_replaying(&store, gate.clone(), rows);
        asked.wait_for(|asked| *asked).await.unwrap();
        drain.send(Some(Instant::now())).unwrap();
        gate.release.add_permits(1);
        tokio::time::timeout(std::time::Duration::from_secs(2), worker)
            .await
            .expect("the drain ends the save before its next piece")
            .unwrap();

        let paused = outbox_row(&store, row).await.expect("the row stays");
        assert_eq!((&paused.0, paused.1, &paused.2), (&before.0, before.1, &before.2), "the attempts of before the take");
        assert_eq!(paused.3.len(), 3, "the take stores one piece, then pauses");
        assert_eq!(count_memory(&store).await, 3);

        replay_pending(&store, Arc::new(FakeEmbedder::default())).await;

        assert_eq!(outbox_row(&store, row).await, None);
        assert_eq!(access_counts(&store).await, vec![1; pieces], "nothing stored twice or reinforced");
        assert_eq!(piece_links(&store).await.len(), pieces);
    }

    // ── Test harnesses ───────────────────────────────────────────────────────

    /// A drain signal whose sender is already gone, so it never asks for one.
    pub(crate) fn no_drain() -> watch::Receiver<Option<Instant>> {
        watch::channel(None).1
    }

    /// Test harness — spawns `memory_worker` with a seeded embedder cell.
    pub(crate) async fn spawn_test_worker(
        store: Arc<MemoryStore>,
        embedder: Arc<dyn Embedder>,
    ) -> (Sender<IngestRequest>, tokio::task::JoinHandle<()>) {
        let cell: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
        cell.set(embedder).ok();
        let cell = Arc::new(cell);

        let chat_chunker = Arc::new(ChatChunker::with_fallback());

        let (tx, rx) = tokio::sync::mpsc::channel(crate::runtime::INGEST_QUEUE_CAPACITY);
        let handle = tokio::spawn(async move {
            memory_worker(rx, no_drain(), Replay::none(), store, cell, chat_chunker).await;
        });
        (tx, handle)
    }

    /// Variant with optional embedder — None = empty cell, for tests that
    /// exercise the embedder-not-ready gate.
    pub(crate) async fn spawn_test_worker_opt(
        store: Arc<MemoryStore>,
        embedder: Option<Arc<dyn Embedder>>,
    ) -> (Sender<IngestRequest>, tokio::task::JoinHandle<()>) {
        let cell: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
        if let Some(e) = embedder {
            cell.set(e).ok();
        }
        spawn_test_worker_on_cell(store, Arc::new(cell)).await
    }

    /// Variant that shares the embedder cell with the caller, so a test can
    /// fill it after the worker is already running.
    pub(crate) async fn spawn_test_worker_on_cell(
        store: Arc<MemoryStore>,
        cell: Arc<OnceCell<Arc<dyn Embedder>>>,
    ) -> (Sender<IngestRequest>, tokio::task::JoinHandle<()>) {
        let chat_chunker = Arc::new(ChatChunker::with_fallback());

        let (tx, rx) = tokio::sync::mpsc::channel(crate::runtime::INGEST_QUEUE_CAPACITY);
        let handle = tokio::spawn(async move {
            memory_worker(rx, no_drain(), Replay::none(), store, cell, chat_chunker).await;
        });
        (tx, handle)
    }

    /// An event the outbox held from before its project was merged names the
    /// tombstone: it is stored in the project it went into, and revises the
    /// key there.
    #[tokio::test]
    async fn a_save_retried_after_a_merge_lands_in_its_target() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        store
            .conn
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO project (id, name, born, created_at) VALUES ('web-bbbb', 'web', 'root', 0);
                     INSERT INTO project (id, name, born, created_at, merged_into, merged_at) \
                       VALUES ('app-aaaa', 'app', 'root', 0, 'web-bbbb', 1);",
                )
            })
            .await
            .unwrap();
        let under = |project: &str, content: &str, at: u64| MemoryEvent {
            project_id: Some(project.into()),
            timestamp_ms: at,
            ..keyed(content, "storage/engine")
        };
        let old = save_for_test(&store, &under("web-bbbb", "Storage engine is SQLite", 10)).await.memories[0].id;

        let report = save_for_test(&store, &under("app-aaaa", "Storage engine is SQLite with WAL", 20)).await;

        assert_eq!(report.failure, None);
        let new = report.memories[0].id;
        let project: Option<String> = store
            .conn
            .call(move |c| c.query_row("SELECT project_id FROM memory WHERE id = ?1", [new], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(project.as_deref(), Some("web-bbbb"));
        assert_eq!(revision_state(&store, old).await, ("superseded".into(), Some(new), Some("storage/engine".into())));
    }

    // ── Revisions by topic key ──────────────────────────────────────────────

    fn keyed(content: &str, key: &str) -> MemoryEvent {
        MemoryEvent { topic_key: Some(key.into()), ..explicit(content, "architectural_decision") }
    }

    /// `(status, superseded_by, topic_key)` of row `id`.
    async fn revision_state(store: &MemoryStore, id: i64) -> (String, Option<i64>, Option<String>) {
        store
            .conn
            .call(move |c| {
                c.query_row("SELECT status, superseded_by, topic_key FROM memory WHERE id = ?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
            })
            .await
            .unwrap()
    }

    async fn live_with_key(store: &MemoryStore, key: &str) -> Vec<i64> {
        let key = key.to_string();
        store
            .conn
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT id FROM memory WHERE topic_key = ?1 AND deleted_at IS NULL \
                     AND status IN ('active', 'disputed') ORDER BY id",
                )?;
                let ids = stmt.query_map([key], |r| r.get(0))?.collect();
                ids
            })
            .await
            .unwrap()
    }

    async fn save(store: &Arc<MemoryStore>, evt: &MemoryEvent) -> IngestReport {
        let report = process_with(Arc::new(FakeEmbedder::default()), store, evt).await;
        assert_eq!(report.failure, None);
        report
    }

    fn only_id(report: &IngestReport) -> i64 {
        assert_eq!(report.memories.len(), 1, "{report:?}");
        report.memories[0].id
    }

    #[tokio::test]
    async fn a_keyed_save_replaces_the_live_row_of_its_key() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let old_text = "Storage engine is SQLite\nChosen for a single local file.";
        let old = only_id(&save(&store, &keyed(old_text, "storage/engine")).await);

        let report = save(&store, &keyed("Storage engine moved to Postgres\nFor concurrent writers.", "storage/engine")).await;
        let new = only_id(&report);

        assert_eq!(report.memories[0].outcome, ChunkOutcome::Inserted);
        assert_eq!(report.replaced, vec![Replaced { id: old, excerpt: "Storage engine is SQLite".into() }]);
        assert_eq!(revision_state(&store, old).await, ("superseded".into(), Some(new), Some("storage/engine".into())));
        assert_eq!(revision_state(&store, new).await, ("active".into(), None, Some("storage/engine".into())));
        assert_eq!(live_with_key(&store, "storage/engine").await, vec![new]);
        let vector = FakeEmbedder::default().embed_with_role(old_text, EmbedRole::Passage).await.unwrap();
        let hits = crate::memory::search::search_all_memories(
            &store,
            &vector,
            Some("SQLite"),
            "agent-test",
            crate::memory::search::SearchScope::all_default(),
            10,
        )
        .await
        .unwrap();
        assert!(hits.iter().all(|h| h.id != old), "{hits:?}");
    }

    #[tokio::test]
    async fn a_revision_records_why_it_replaced() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let old = only_id(&save(&store, &keyed("Storage engine is SQLite", "storage/engine")).await);
        let new = only_id(&save(&store, &keyed("Storage engine moved to Postgres", "storage/engine")).await);

        let causes: Vec<Option<String>> = store
            .conn
            .call(move |c| {
                let mut stmt = c.prepare("SELECT supersede_cause FROM memory WHERE id IN (?1, ?2) ORDER BY id")?;
                let rows = stmt.query_map([old, new], |r| r.get(0))?.collect();
                rows
            })
            .await
            .unwrap();
        assert_eq!(causes, vec![Some("revision".to_string()), None]);
    }

    #[tokio::test]
    async fn a_search_hit_carries_its_topic_key() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let text = "Storage engine is SQLite\nChosen for a single local file.";
        let keyed_id = only_id(&save(&store, &keyed(text, "storage/engine")).await);
        let plain_id = only_id(&save(&store, &explicit("Deploys happen on fridays", "architectural_decision")).await);

        let vector = FakeEmbedder::default().embed_with_role(text, EmbedRole::Passage).await.unwrap();
        let hits = crate::memory::search::search_all_memories(
            &store,
            &vector,
            Some("SQLite"),
            "agent-test",
            crate::memory::search::SearchScope::all_default(),
            10,
        )
        .await
        .unwrap();

        let key_of = |id: i64| hits.iter().find(|h| h.id == id).unwrap_or_else(|| panic!("#{id} not found: {hits:?}")).topic_key.clone();
        assert_eq!(key_of(keyed_id).as_deref(), Some("storage/engine"));
        assert_eq!(key_of(plain_id), None);
        let json = serde_json::to_value(hits.iter().find(|h| h.id == plain_id).unwrap()).unwrap();
        assert!(json.get("topic_key").is_none(), "{json}");
    }

    #[tokio::test]
    async fn a_save_without_a_key_replaces_nothing() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let old = only_id(&save(&store, &keyed("Storage engine is SQLite", "storage/engine")).await);

        let report = save(&store, &explicit("Storage engine moved to Postgres", "architectural_decision")).await;

        assert!(report.replaced.is_empty());
        assert_eq!(revision_state(&store, old).await.0, "active");
        assert_eq!(revision_state(&store, only_id(&report)).await.2, None);
    }

    #[tokio::test]
    async fn the_same_text_under_its_key_only_reinforces() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let first = only_id(&save(&store, &keyed("Storage engine is SQLite", "storage/engine")).await);

        let report = save(&store, &keyed("Storage engine is SQLite", "storage/engine")).await;

        assert_eq!(report.memories, vec![SavedChunk { id: first, outcome: ChunkOutcome::Reinforced }]);
        assert!(report.replaced.is_empty());
        assert_eq!(revision_state(&store, first).await.0, "active");
    }

    #[tokio::test]
    async fn saving_an_earlier_text_again_is_another_revision() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let v1 = only_id(&save(&store, &keyed("Storage engine is SQLite", "storage/engine")).await);
        let v2 = only_id(&save(&store, &keyed("Storage engine moved to Postgres", "storage/engine")).await);

        let report = save(&store, &keyed("Storage engine is SQLite", "storage/engine")).await;
        let v3 = only_id(&report);

        assert!(v3 != v1 && v3 != v2);
        assert_eq!(report.memories[0].outcome, ChunkOutcome::Inserted);
        assert_eq!(revision_state(&store, v1).await.1, Some(v2));
        assert_eq!(revision_state(&store, v2).await.1, Some(v3));
        assert_eq!(live_with_key(&store, "storage/engine").await, vec![v3]);
    }

    #[tokio::test]
    async fn a_revision_inherits_tier_and_accesses() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let old = only_id(&save(&store, &keyed("Storage engine is SQLite", "storage/engine")).await);
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET tier = 1, access_count = 7 WHERE id = ?1", [old]))
            .await
            .unwrap();

        let new = only_id(&save(&store, &keyed("Storage engine moved to Postgres", "storage/engine")).await);

        let (tier, accesses): (i64, i64) = store
            .conn
            .call(move |c| c.query_row("SELECT tier, access_count FROM memory WHERE id = ?1", [new], |r| Ok((r.get(0)?, r.get(1)?))))
            .await
            .unwrap();
        assert_eq!((tier, accesses), (1, 7));
    }

    #[tokio::test]
    async fn a_revision_keeps_the_type_and_importance_it_does_not_name() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let first = MemoryEvent { importance: Some(8.0), ..keyed("Indent with tabs", "style/indent") };
        let first = MemoryEvent { memory_type: Some("user_preference".into()), ..first };
        save(&store, &first).await;

        let revised = MemoryEvent { memory_type: None, importance: None, ..keyed("Indent with four spaces", "style/indent") };
        let new = only_id(&save(&store, &revised).await);

        let (memory_type, importance): (String, f64) = store
            .conn
            .call(move |c| c.query_row("SELECT memory_type, importance FROM memory WHERE id = ?1", [new], |r| Ok((r.get(0)?, r.get(1)?))))
            .await
            .unwrap();
        assert_eq!((memory_type.as_str(), importance), ("user_preference", 8.0));
    }

    #[tokio::test]
    async fn a_revision_releases_the_rows_disputing_the_old_one() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let old = only_id(&save(&store, &keyed("Storage engine is SQLite", "storage/engine")).await);
        let challenger = only_id(&save(&store, &explicit("Storage engine is DuckDB", "architectural_decision")).await);
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE memory SET status = 'disputed', disputed_against = json_array(?2) WHERE id = ?1",
                    [challenger, old],
                )
            })
            .await
            .unwrap();

        save(&store, &keyed("Storage engine moved to Postgres", "storage/engine")).await;

        assert_eq!(revision_state(&store, challenger).await.0, "active");
    }

    #[tokio::test]
    async fn a_failed_revision_replaces_nothing() {
        struct FailingBatch;
        #[async_trait::async_trait]
        impl Embedder for FailingBatch {
            async fn embed_with_role(&self, _text: &str, _role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
                Err(EmbedError::InferenceFailed("boom".into()))
            }
            fn dimensions(&self) -> usize { 384 }
            fn model_id(&self) -> &str { "failing" }
            fn query_prefix(&self) -> &str { "" }
            fn passage_prefix(&self) -> &str { "" }
        }
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let old = only_id(&save(&store, &keyed("Storage engine is SQLite", "storage/engine")).await);

        let report =
            process_with(Arc::new(FailingBatch), &store, &keyed("Storage engine moved to Postgres", "storage/engine")).await;

        assert!(report.failure.is_some());
        assert!(report.replaced.is_empty());
        assert_eq!(live_with_key(&store, "storage/engine").await, vec![old]);
    }

    #[tokio::test]
    async fn a_reinforced_row_without_a_key_takes_it() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let unkeyed = only_id(&save(&store, &explicit("Storage engine moved to Postgres", "architectural_decision")).await);
        let old = only_id(&save(&store, &keyed("Storage engine is SQLite", "storage/engine")).await);

        let report = save(&store, &keyed("Storage engine moved to Postgres", "storage/engine")).await;

        assert_eq!(report.memories, vec![SavedChunk { id: unkeyed, outcome: ChunkOutcome::Reinforced }]);
        assert_eq!(revision_state(&store, old).await.1, Some(unkeyed));
        assert_eq!(live_with_key(&store, "storage/engine").await, vec![unkeyed]);
    }

    #[tokio::test]
    async fn a_revision_does_not_pair_with_the_row_it_replaces() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let placed = |angle| Arc::new(PlacedEmbedder { vectors: vec![turned(1, angle)] }) as Arc<dyn Embedder>;
        let other = process_with(placed(0.30), &store, &explicit_fact("Storage engine notes")).await.memories[0].id;
        let old = process_with(placed(0.0), &store, &MemoryEvent { memory_type: Some("semantic_fact".into()), ..keyed("Storage engine is SQLite", "storage/engine") })
            .await
            .memories[0]
            .id;

        let report = process_with(placed(0.15), &store, &MemoryEvent { memory_type: Some("semantic_fact".into()), ..keyed("Storage engine moved to Postgres", "storage/engine") }).await;
        let new = only_id(&report);

        let found = pairs(&store).await;
        assert!(found.iter().all(|(n, o, _)| !(*n == new && *o == old)), "{found:?}");
        assert!(found.iter().any(|(n, o, _)| *n == new && *o == other), "{found:?}");
    }

    #[tokio::test]
    async fn a_key_is_scoped_to_its_project() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let old = only_id(&save(&store, &keyed("Storage engine is SQLite", "storage/engine")).await);

        let other_project = MemoryEvent { project_id: Some("p-2".into()), ..keyed("Storage engine moved to Oracle", "storage/engine") };
        assert!(save(&store, &other_project).await.replaced.is_empty());
        assert_eq!(revision_state(&store, old).await.0, "active");

        let other_agent = MemoryEvent { agent_id: "agent-other".into(), ..keyed("Storage engine moved to Postgres", "storage/engine") };
        let report = save(&store, &other_agent).await;
        let new = only_id(&report);
        assert_eq!(report.replaced.iter().map(|r| r.id).collect::<Vec<_>>(), vec![old], "a key names a topic of the project");
        assert_eq!(revision_state(&store, old).await, ("superseded".into(), Some(new), Some("storage/engine".into())));
    }

    /// What agent-scoped keys let a store hold: two live holders of one key in
    /// a project, from two agents. The next save under the key replaces both.
    #[tokio::test]
    async fn a_keyed_save_replaces_every_holder_of_its_project() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let first = only_id(&save(&store, &keyed("Storage engine is SQLite", "storage/engine")).await);
        let second = store
            .conn
            .call(|c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source, created_at, created_by_type, \
                     created_by_ref, last_modified_at, last_accessed_at, importance, tier, status, topic_key) \
                     VALUES ('agent', 'agent-other', 'Storage engine is DuckDB', 'architectural_decision', 'explicit', \
                     1, 'agent', 'agent-other', 1, 1, 5.0, 3, 'active', 'storage/engine')",
                    [],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap();

        let report = save(&store, &keyed("Storage engine moved to Postgres", "storage/engine")).await;

        let mut replaced: Vec<i64> = report.replaced.iter().map(|r| r.id).collect();
        replaced.sort_unstable();
        assert_eq!(replaced, vec![first, second]);
        assert_eq!(live_with_key(&store, "storage/engine").await, vec![only_id(&report)]);
    }

    #[tokio::test]
    async fn a_key_leaves_a_hidden_authors_holder() {
        let dir = crate::test_dir::test_dir();
        let mut policy = crate::memory::sharing::SharingPolicy::default();
        policy.hide("agent-other", "agent-test");
        crate::memory::sharing::save(dir.path(), &policy).unwrap();
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap().with_sharing_dir(dir.path().to_path_buf()));
        let old = only_id(&save(&store, &keyed("Storage engine is SQLite", "storage/engine")).await);

        let hidden = MemoryEvent { agent_id: "agent-other".into(), ..keyed("Storage engine moved to Postgres", "storage/engine") };
        assert!(save(&store, &hidden).await.replaced.is_empty(), "a save never replaces what its agent cannot read");
        assert_eq!(revision_state(&store, old).await.0, "active");
    }

    #[tokio::test]
    async fn the_same_text_from_another_agent_reinforces_the_row_already_there() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let first = only_id(&save(&store, &explicit("The deploy runs on fridays", "semantic_fact")).await);

        let other = MemoryEvent { agent_id: "agent-other".into(), ..explicit("The deploy runs on fridays", "semantic_fact") };
        let report = save(&store, &other).await;

        assert_eq!(report.memories.len(), 1, "{report:?}");
        assert_eq!((report.memories[0].id, report.memories[0].outcome), (first, ChunkOutcome::Reinforced));
    }

    fn other_text(words: usize) -> String {
        (0..words).map(|i| format!("x{i:05}")).collect::<Vec<_>>().join(" ")
    }

    #[tokio::test]
    async fn a_long_revision_replaces_every_piece_of_the_old_save() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let old: Vec<i64> = save(&store, &keyed(&distinct_text(400), "storage/engine")).await.memories.iter().map(|m| m.id).collect();
        assert!(old.len() >= 2);

        let report = save(&store, &keyed(&other_text(400), "storage/engine")).await;
        let head = report.memories[0].id;

        let mut replaced: Vec<i64> = report.replaced.iter().map(|r| r.id).collect();
        replaced.sort_unstable();
        assert_eq!(replaced, old);
        for id in old {
            assert_eq!(revision_state(&store, id).await.1, Some(head));
        }
        let new: Vec<i64> = report.memories.iter().map(|m| m.id).collect();
        assert_eq!(live_with_key(&store, "storage/engine").await, new);
    }

    #[tokio::test]
    async fn a_piece_shared_with_another_save_is_detached() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let old: Vec<i64> = save(&store, &keyed(&distinct_text(400), "storage/engine")).await.memories.iter().map(|m| m.id).collect();
        let unkeyed: Vec<i64> =
            save(&store, &explicit(&other_text(400), "semantic_fact")).await.memories.iter().map(|m| m.id).collect();
        // The dedup absorbed a piece of the keyed save into another save.
        let (shared, first) = (old[1], unkeyed[0]);
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory_piece (save_id, piece, memory_id, context) \
                     SELECT MAX(save_id) + 1, 1, ?1, '' FROM memory_piece \
                     UNION ALL SELECT MAX(save_id) + 1, 2, ?2, '' FROM memory_piece",
                    [first, shared],
                )
            })
            .await
            .unwrap();

        save(&store, &keyed("Storage engine moved to Postgres", "storage/engine")).await;

        assert_eq!(revision_state(&store, shared).await, ("active".into(), None, None));
        assert_eq!(revision_state(&store, old[0]).await.0, "superseded");
    }

    #[tokio::test]
    async fn two_queued_revisions_apply_in_order() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let (tx, handle) = spawn_test_worker(store.clone(), Arc::new(FakeEmbedder::default())).await;
        for text in ["Storage engine is SQLite", "Storage engine moved to Postgres", "Storage engine moved to DuckDB"] {
            tx.send(keyed(text, "storage/engine").into()).await.unwrap();
        }
        drain_worker(tx, handle).await;

        let live = live_with_key(&store, "storage/engine").await;
        assert_eq!(live.len(), 1);
        assert_eq!(stored_content(&store, live[0]).await, "Storage engine moved to DuckDB");
    }

    #[tokio::test]
    async fn a_revision_is_finished_with_its_outbox_row() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let old = only_id(&save(&store, &keyed("Storage engine is SQLite", "storage/engine")).await);
        let row = match store
            .outbox_record("op-revision".into(), keyed("Storage engine moved to Postgres", "storage/engine"))
            .await
            .unwrap()
        {
            crate::memory::outbox::Recorded::New { row } => row,
            other => panic!("expected a new row, got {other:?}"),
        };

        replay_pending(&store, Arc::new(FakeEmbedder::default())).await;

        assert_eq!(outbox_row(&store, row).await, None);
        let live = live_with_key(&store, "storage/engine").await;
        assert_eq!(live.len(), 1);
        assert_eq!(revision_state(&store, old).await, ("superseded".into(), Some(live[0]), Some("storage/engine".into())));
    }
}
