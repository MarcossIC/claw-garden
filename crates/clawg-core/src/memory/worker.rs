//! Memory ingest worker.
//!
//! Spawned once per process (see `MemoryRuntime::ingest_worker`). Drains
//! `MemoryEvent`s from the channel, selects the appropriate chunker, embeds
//! each chunk, and inserts both `memory` and `memory_vectors` rows atomically.
//! Failure isolation: errors are logged and the worker NEVER panics.

use std::collections::HashMap;
use std::sync::Arc;

use once_cell::sync::OnceCell;
use tokio::sync::mpsc::Receiver;
use tokio::sync::{oneshot, watch};
use tokio::time::Instant;

use crate::memory::chunker::{
    embedding_text, select_chunker, ChatChunker, Chunk, ChunkerKind, SaveSplit,
};
use crate::memory::pieces::StoredPiece;
use crate::memory::embedder::{EmbedError, EmbedRole, Embedder};
use crate::memory::events::{MemoryEvent, MemorySource};
use crate::memory::redact::{redact, SecretKind};
use crate::memory::spill::Replay;
use crate::memory::store_insert::NewMemoryRecord;
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
}

impl From<MemoryEvent> for IngestRequest {
    fn from(event: MemoryEvent) -> Self {
        Self { event, reply: None }
    }
}

/// What one event wrote, in chunk order, and the failure that stopped it.
///
/// Both halves at once rather than a `Result`: a failure part-way through a
/// multi-chunk event leaves earlier chunks written, and the caller needs
/// their ids to clean up or retry only the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IngestReport {
    pub memories: Vec<SavedChunk>,
    pub failure: Option<IngestFailure>,
    /// The kinds of secret masked out of the event before it was chunked.
    pub redacted: Vec<SecretKind>,
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
/// `drain` carries the instant a shutdown wants the queue drained by. Once it
/// is set the channel is closed to new events, what is already queued keeps
/// being ingested until [`drain_step`] says to stop, and the events not yet
/// started (unreplayed ones first) are returned in order for the caller to
/// spill. The event in flight
/// always finishes: a half-ingested event replayed later would duplicate the
/// chunks that did land. A dropped `drain` sender never asks for a drain.
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
    loop {
        observe_drain(&mut rx, &mut drain, &mut deadline);
        let IngestRequest { event: evt, reply } = match replay.next() {
            Some(evt) => IngestRequest::from(evt),
            None => {
                replay.finish().await;
                match next_event(&mut rx, &mut drain, &mut deadline).await {
                    Some(request) => request,
                    None => break,
                }
            }
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
            return unstarted(evt, replay, &mut rx);
        }
        let report = process_event(
            &evt,
            &store,
            &embedder_cell,
            &chat_chunker,
            &mut absorbed_duplicates,
        )
        .await;
        if let Some(e) = &report.failure {
            log::warn!(
                "ingest event stopped after {} stored chunks (agent={}, role={:?}): {e}",
                report.memories.len(),
                evt.agent_id,
                evt.role
            );
        }
        if !report.redacted.is_empty() {
            let kinds: Vec<&str> = report.redacted.iter().map(|k| k.as_str()).collect();
            log::info!("ingest masked {} (agent={})", kinds.join(", "), evt.agent_id);
        }
        if let Some(reply) = reply {
            let _ = reply.send(report);
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

/// `evt`, then the events left to replay, then everything still queued, in
/// order. Closes the channel first so nothing can slip in behind the last
/// event read.
fn unstarted(
    evt: MemoryEvent,
    replay: Replay,
    rx: &mut Receiver<IngestRequest>,
) -> Vec<MemoryEvent> {
    rx.close();
    let mut events = vec![evt];
    events.extend(replay.into_remaining());
    while let Ok(next) = rx.try_recv() {
        events.push(next.event);
    }
    log::info!("ingest stopped with {} events not yet ingested", events.len());
    events
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
    content_hash: &str,
    absorbed: &mut u64,
) -> Option<i64> {
    match store
        .reinforce_if_duplicate(
            &evt.agent_id,
            evt.project_id.as_deref(),
            content_hash,
            evt.sources(),
            evt.timestamp_ms,
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

async fn process_event(
    evt: &MemoryEvent,
    store: &Arc<MemoryStore>,
    embedder_cell: &Arc<OnceCell<Arc<dyn Embedder>>>,
    chat_chunker: &Arc<ChatChunker>,
    absorbed: &mut u64,
) -> IngestReport {
    // Masked before anything else reads the text, so the chunker, the dedup
    // hash, the embedding, the full-text index and the stored row all see the
    // same masked content, and the secret reaches none of them.
    let masked = redact(&evt.content);
    let redacted = masked.kinds;
    let content = masked.text;
    // Source-first: Compaction-sourced events bypass select_chunker (and thus
    // the Role::System→Skip rule that ordinarily drops system-authored
    // content from ingestion) entirely — they always chunk via ChatChunker,
    // because a compaction summary is deliberately synthesized content worth
    // keeping regardless of which role produced the text it summarizes. All
    // other sources keep the existing role-based routing.
    let SaveSplit { pieces: chunks, head } = if evt.source == MemorySource::Compaction {
        chat_chunker.split_save(&content)
    } else {
        let kind = select_chunker(evt.role.clone(), evt.is_error, &content);
        match kind {
            ChunkerKind::Chat => chat_chunker.split_save(&content),
            ChunkerKind::Skip => return IngestReport { redacted, ..Default::default() },
        }
    };

    if chunks.is_empty() {
        // Content was all-whitespace or empty after chunker policy. Not an error.
        return IngestReport { redacted, ..Default::default() };
    }

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
        return IngestReport { memories: vec![], failure: Some(IngestFailure::EmbedderUnavailable), redacted };
    };

    // Stage 1 — gate. Every chunk passes the hash gate before anything is
    // embedded; a collapsed entry absorbed by the gate is charged once per
    // copy it stood for, which is what the same copies arriving one at a
    // time would have done.
    // `stored[i]` is what the i-th collapsed chunk became, so the report keeps
    // chunk order even though absorbed chunks are settled before the rest.
    let pending_chunks = collapse_duplicate_chunks(chunks);
    // The head goes into the embedder with every piece after the first; the
    // first already starts with it.
    let context = |position: usize| match &head {
        Some(head) if position > 0 => head.as_str(),
        _ => "",
    };
    let mut stored: Vec<Option<SavedChunk>> = vec![None; pending_chunks.len()];
    let mut survivors: Vec<(usize, PendingChunk)> = Vec::new();
    for (position, pending) in pending_chunks.into_iter().enumerate() {
        if let Some(id) = absorb_duplicate(store, evt, &pending.content_hash, absorbed).await {
            for _ in 1..pending.multiplicity {
                absorb_duplicate(store, evt, &pending.content_hash, absorbed).await;
            }
            stored[position] = Some(SavedChunk { id, outcome: ChunkOutcome::Reinforced });
            continue;
        }
        survivors.push((position, pending));
    }
    let report = |stored: Vec<Option<SavedChunk>>, failure| IngestReport {
        memories: stored.into_iter().flatten().collect(),
        failure,
        redacted: redacted.clone(),
    };
    if survivors.is_empty() {
        link_save(store, &stored, &[]).await;
        return report(stored, None);
    }

    // Stage 2 — one inference call for the whole event. Passage: a stored
    // chunk is the thing being searched THROUGH. Each vector produced here
    // feeds BOTH consumers below — insert and the contradiction KNN probe —
    // with no second inference call, so both sides of `contradiction.rs`'s
    // comparison live in the same Passage space `CONTRADICTION_THRESHOLD`
    // was tuned against.
    let texts: Vec<String> =
        survivors.iter().map(|(position, p)| embedding_text(context(*position), &p.chunk.text).into_owned()).collect();
    let vectors = match embedder.embed_batch(&texts, EmbedRole::Passage).await {
        Ok(v) => v,
        Err(EmbedError::NotReady) | Err(EmbedError::WorkerGone) => {
            return report(stored, Some(IngestFailure::EmbedderUnavailable));
        }
        Err(e) => {
            return report(stored, Some(IngestFailure::Failed(format!("embed failed: {e}"))));
        }
    };
    if vectors.len() != survivors.len() {
        // Zipping a short reply would pair chunks with other chunks'
        // vectors and store the mismatch as fact. Dropping the event is
        // recoverable; a silently mislabeled memory is not.
        return report(
            stored,
            Some(IngestFailure::Failed(format!(
                "embedder returned {} vectors for {} chunks",
                vectors.len(),
                survivors.len()
            ))),
        );
    }

    // Stage 3 — sequential, one chunk at a time: chunk N's contradiction
    // probe is a KNN search over rows already in the store, so chunk N-1 of
    // this same event is already there when it runs. An explicit save
    // carries a claim-bearing type, so a long one reaches the probe once per
    // chunk; the chunks this event already inserted are passed as siblings,
    // because two parts of one save are one claim by one author and must not
    // dispute each other. The rows a conclusion was drawn from are passed the
    // same way: it restates them, it does not contradict them.
    let mut siblings: Vec<i64> = evt.sources().to_vec();
    let mut embedded_with: Vec<(usize, String)> = Vec::new();
    for ((position, pending), vec) in survivors.into_iter().zip(vectors) {
        let PendingChunk { chunk, content_hash, multiplicity } = pending;
        // The contradiction hook runs between embed and insert: it needs the
        // freshly computed embedding to find near-duplicate candidates via
        // KNN, and its result (which existing rows this chunk disputes) must
        // be attached to the record before it is written, not after.
        let mut record = NewMemoryRecord::from_event_and_chunk(evt, &chunk);
        let disputed = crate::memory::contradiction::detect_contradiction(
            store, &vec, &evt.agent_id, evt.project_id.as_deref(), record.memory_type, &siblings,
        ).await;
        if !disputed.is_empty() {
            record.disputed_against = Some(disputed);
        }
        let id = match store.insert_memory_with_vector(record, vec).await {
            Ok(id) => id,
            Err(e) => {
                return report(stored, Some(IngestFailure::Failed(format!("store insert failed: {e}"))));
            }
        };
        stored[position] = Some(SavedChunk { id, outcome: ChunkOutcome::Inserted });
        embedded_with.push((position, context(position).to_string()));
        siblings.push(id);
        // The extra copies now find the row this iteration just wrote and
        // take the same branch a copy from a later event takes.
        for _ in 1..multiplicity {
            absorb_duplicate(store, evt, &content_hash, absorbed).await;
        }
    }
    link_save(store, &stored, &embedded_with).await;
    report(stored, None)
}

/// Links the pieces of a finished save of two or more. `embedded_with` names
/// the positions this event inserted and the head each was embedded behind;
/// a position it absorbed into an existing row keeps that row's vector, made
/// without this head, so its link carries none.
///
/// A failure here leaves the rows unlinked, which is how every save was stored
/// before links existed; the save itself did not fail and is not reported as
/// failed.
async fn link_save(store: &Arc<MemoryStore>, stored: &[Option<SavedChunk>], embedded_with: &[(usize, String)]) {
    if stored.len() < 2 {
        return;
    }
    let pieces: Vec<StoredPiece> = stored
        .iter()
        .enumerate()
        .filter_map(|(position, saved)| {
            let context = embedded_with.iter().find(|(p, _)| *p == position).map(|(_, c)| c.clone());
            saved.map(|s| StoredPiece { memory_id: s.id, context: context.unwrap_or_default() })
        })
        .collect();
    if let Err(e) = store.link_pieces(pieces).await {
        log::warn!("could not link the {} pieces of a save: {e}; they stay unlinked", stored.len());
    }
}

/// Resolves the shared tokenizer exactly once and builds the chat chunker
/// from that SAME value: `tokenizer_cell` and the returned chunker can never
/// disagree about which parse of `tokenizer.json` they got, and the file is
/// never read twice for one process. Pulled out of the worker's setup so this
/// wiring is unit-testable on its own.
pub fn resolve_tokenizer_and_chunker(
    base_dir: &std::path::Path,
    tokenizer_cell: &OnceCell<Arc<tokenizers::Tokenizer>>,
) -> Arc<ChatChunker> {
    let tok = crate::memory::model_download::load_tokenizer(base_dir);
    if let Some(t) = &tok {
        let _ = tokenizer_cell.set(t.clone());
    }
    Arc::new(ChatChunker::from_tokenizer(tok))
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

    /// Helper: insert a memory row with a given embedding directly into the store.
    /// Uses tier=3, status='active' so it is visible to search.
    async fn insert_row_with_embedding(store: &MemoryStore, agent_id: &str, embedding: &[f32]) -> i64 {
        let bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
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
                     VALUES ('agent', ?1, 'pre content', 'chat_message', 'chat', \
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

    #[tokio::test]
    async fn a_chunk_still_disputes_an_older_row() {
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
            assert_eq!(disputes_of(&store, saved.id).await, ("disputed".to_string(), Some(format!("[{older}]"))));
        }
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
        assert_eq!(
            disputes_of(&store, report.memories[0].id).await,
            ("disputed".to_string(), Some(format!("[{other}]")))
        );
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
        build_memory_prompt, QueryTexts, SEP_LONGTERM_TEST, SEP_RECENT_TEST,
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
        let embedder = Arc::new(FakeEmbedder::default());
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

        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let prompt = build_memory_prompt(
            &store, &wm, Some(embedder.clone() as Arc<dyn Embedder>),
            AGENT, None, &crate::memory::sharing::Reach::Own, QueryTexts::single_shot(&fresh.content),
            &crate::memory::workspace::EmittedIds::default(), None,
        )
        .await
        .unwrap();

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
        Chunk { text: text.to_string(), token_count: 0 }
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

        let cell: OnceCell<Arc<tokenizers::Tokenizer>> = OnceCell::new();
        let chunker = resolve_tokenizer_and_chunker(&base_dir, &cell);

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

    #[test]
    fn resolve_tokenizer_and_chunker_leaves_the_cell_empty_when_the_file_is_absent() {
        let base_dir = unique_temp_dir("resolve-tok-absent");
        let cell: OnceCell<Arc<tokenizers::Tokenizer>> = OnceCell::new();
        let chunker = resolve_tokenizer_and_chunker(&base_dir, &cell);

        assert!(cell.get().is_none(), "no tokenizer.json on disk must leave the cell empty");
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
}
