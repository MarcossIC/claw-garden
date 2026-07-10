# Phase 3 — rag-ingest — Design

> **Inputs**: `proposal.md` (8 LOCKED decisions D1–D8) + `explore.md` (code touchpoints) + Phase 2 `design.md` (carry-forward patterns: worker-thread, `OnceCell`, `app.emit`, `const _: fn()` Send/Sync guards).
>
> **Status**: all source line references re-verified against the working tree. Where the proposal disagrees with the actual code, the **design adapts** and surfaces the deviation in §11 (Risks beyond proposal). The 8 LOCKED decisions are honored verbatim.
>
> **Open contradiction with proposal** (raised, not silently resolved): the proposal sketch in §4 and Risk R3 imply the worker can derive `project_id` itself from `agent_id`. The actual `push_and_persist(&AppState, &str, ChatMessage)` signature does NOT carry `agent_project_path` — and re-locking `state.agents` from the worker per event is ugly (extra lock contention on the chat hot path, racy if the agent is mutated mid-flight). Design proposes **deriving `project_id` at the call site, inside `push_and_persist`, and adding `agent_project_path: Option<&str>` to `push_and_persist`'s signature** — one extra parameter, three call sites updated. The 5-line `MemoryEvent::from_message(...)` builder still carries `project_id: Option<String>` exactly as the proposal locked. See §11 R-NEW-1.

---

## 1. Lifecycle diagram (ASCII)

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ App startup (sync — tauri::Builder::default()…)                              │
│                                                                              │
│   AppState::new()  ← block_on(MemoryStore::open(base_dir))                   │
│                    ← memory_tx = (no-op — see §4.a: AppState retains the     │
│                                  Sender; the Receiver is yielded separately) │
│                                                                              │
│   .manage(AppState { …, memory_tx, embedder: empty OnceCell, … })            │
│                                                                              │
│   setup hook fires (sync closure)                                            │
│     ├── start_embedder_bootstrap(app.handle())  [Phase 2 — unchanged]        │
│     └── start_memory_worker(app.handle(), memory_rx)  [Phase 3 — NEW]        │
└──────────────────────────────────────────────────────────────────────────────┘
                                       │
                                       ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ start_memory_worker(app_handle, memory_rx)                                   │
│                                                                              │
│   tauri::async_runtime::spawn(async move {                                   │
│     let state = app_handle.state::<AppState>();                              │
│     let store         = state.memory_store.clone();                          │
│     let embedder_cell = state.embedder.clone();                              │
│     let base_dir      = state.base_dir.clone();                              │
│                                                                              │
│     let chat_chunker  = Arc::new(ChatChunker::new_from_base_dir(&base_dir)); │
│     let code_chunker  = Arc::new(CodeChunker::new());                        │
│                                                                              │
│     memory_worker(memory_rx, store, embedder_cell,                           │
│                   chat_chunker, code_chunker).await;                         │
│   });                                                                        │
└──────────────────────────────────────────────────────────────────────────────┘
                                       │
                                       ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ Chat command (producer side — runs on the request task)                      │
│                                                                              │
│   stream_chat(agent_id, message, on_event, state) →                          │
│      …                                                                       │
│      push_and_persist(&state, &agent_id, agent_project_path.as_deref(), msg) │
│                              │                                               │
│                              ▼  (existing: session.append_message)           │
│                              │                                               │
│                              ▼  (NEW Phase 3: build event + send)            │
│      let evt = MemoryEvent::from_message(                                    │
│           agent_id, project_id_opt, &msg, MemorySource::Chat);               │
│      if let Some(evt) = evt {                                                │
│          let _ = state.memory_tx.send(evt);  // non-blocking, FIFO           │
│      }                                                                       │
└──────────────────────────────────────────────────────────────────────────────┘
                                       │
                       tokio::mpsc::UnboundedSender<MemoryEvent>
                                       ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ memory_worker (single long-lived async task)                                 │
│                                                                              │
│  loop {                                                                      │
│      let Some(evt) = rx.recv().await else { return; };  // shutdown          │
│                                                                              │
│      let kind = chunker::select_chunker(evt.role, evt.is_error, &evt.content)│
│      let chunks = match kind {                                               │
│          ChunkerKind::Chat => chat_chunker.chunk(&evt.content, 480),         │
│          ChunkerKind::Code => code_chunker.chunk(&evt.content, None),        │
│          ChunkerKind::Skip => continue,  // empty assistant tool-call msg    │
│      };                                                                      │
│                                                                              │
│      // ── Embedder gate (LOCKED D4) ──                                      │
│      let Some(embedder) = embedder_cell.get().cloned() else {                │
│          tracing::warn!("embedder NotReady; dropping {} chunks", chunks.len()│
│          continue;  // entire event dropped, no half-inserts                 │
│      };                                                                      │
│                                                                              │
│      for chunk in chunks {                                                   │
│          let vec = match embedder.embed(&chunk.text).await {                 │
│              Ok(v) => v,                                                     │
│              Err(EmbedError::NotReady) => break,    // entire event lost     │
│              Err(EmbedError::WorkerGone) => break,  // ditto                 │
│              Err(e) => { tracing::warn!(…); continue; }  // skip chunk       │
│          };                                                                  │
│                                                                              │
│          let rec = NewMemoryRecord::from_event_and_chunk(&evt, &chunk);      │
│          if let Err(e) = store.insert_memory_with_vector(rec, vec).await {   │
│              tracing::warn!("insert_memory_with_vector failed: {e}");        │
│              continue;  // worker stays alive — failure isolated per chunk   │
│          }                                                                   │
│      }                                                                       │
│  }                                                                           │
└──────────────────────────────────────────────────────────────────────────────┘
                                       │
                                       ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ MemoryStore::insert_memory_with_vector(record, vector) -> Result<i64, …>     │
│                                                                              │
│   self.conn.call_unchecked(|c| {                                             │
│       let tx = c.transaction()?;                                             │
│       tx.execute("INSERT INTO memory (…) VALUES (…)", params)?;              │
│       let id = tx.last_insert_rowid();                                       │
│       tx.execute(                                                            │
│         "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",        │
│         params![id, serialize_f32_vec(&vector)])?;                           │
│       tx.commit()?;                                                          │
│       Ok(id)                                                                 │
│   }).await                                                                   │
└──────────────────────────────────────────────────────────────────────────────┘
```

### Failure branches (annotated)

| Branch | Reached when | Handling |
|--------|--------------|----------|
| Embedder NotReady (cell empty) | Cold start — bootstrap task still downloading | **LOCKED D4**: drop event. `warn!("embedder NotReady; dropping {n} chunks")`. Loop continues. |
| Embedder NotReady (returned mid-stream) | Should not happen post-Phase 2 — defensive | Break out of per-chunk loop, lose remaining chunks of this event. |
| Embedder WorkerGone | `fastembed-worker` thread died unexpectedly | `warn!`. Break inner loop. Subsequent events will hit the same break. Worker stays alive — recovery is unscheduled (proposal §4 footnote). |
| Embedder InferenceFailed (single text) | Transient ORT error on a specific chunk | `warn!`. Skip THIS chunk only. Other chunks of the same event proceed. |
| `chunker::code` parse failure | Malformed source, unknown extension fallback | `CodeChunker.chunk()` degrades to byte-length splits (NEVER panics). Returns 1..N chunks. Worker proceeds. |
| `insert_memory_with_vector` returns `Err` | Disk full, schema violation, vec0 dim mismatch | `warn!`. Skip THIS chunk only. Worker stays alive. |
| `rx.recv()` returns `None` | All `Sender` clones dropped (app shutdown) | Worker exits cleanly (`return`). Spawned task ends, `JoinHandle` is dropped by Tauri runtime. |
| Tokenizer file missing at startup | `ChatChunker::new_from_base_dir()` cannot find `tokenizer.json` | Falls back to byte-length heuristic chunker (§6). `warn!("tokenizer.json missing; using byte-length fallback")`. NO panic. |

---

## 2. Module layout

```
src-tauri/src/memory/
├── chunker/                       NEW subdirectory
│   ├── mod.rs                     NEW — pub(crate) Chunk, ChunkerKind, select_chunker
│   ├── chat.rs                    NEW — pub(crate) ChatChunker (turn + 480-tok cap, no overlap)
│   ├── code.rs                    NEW — pub(crate) CodeChunker (tree-sitter Rust/TS/Py)
│   └── token_splitter.rs          NEW — pub(crate) TokenSplitter (shared by chat & code fallback — R6 mitigation)
├── events.rs                      NEW — pub(crate) MemoryEvent, MemorySource
├── worker.rs                      NEW — pub(crate) memory_worker; pub fn start_memory_worker
├── store_insert.rs                NEW — extension impl on MemoryStore for insert_memory_with_vector
│                                          + pub NewMemoryRecord builder + MemoryType + CreatedByType
│                                          + pub MemoryStoreError (new enum extending MemoryError)
├── db.rs                          UNCHANGED (Phase 1 MemoryStore; insert_memory_with_vector lives in store_insert.rs as impl)
├── embedder.rs                    UNCHANGED (Phase 2 — Embedder trait + FakeEmbedder reused)
├── error.rs                       UNCHANGED (MemoryError stays; new MemoryStoreError lives in store_insert.rs)
├── identity.rs                    UNCHANGED (derive_project_id reused — signature is &Path, not &str — see §11 R3)
├── migrations.rs                  UNCHANGED
├── model_download.rs              MODIFIED — add pub fn tokenizer_path(base_dir: &Path) -> PathBuf (§6, R8 mitigation)
└── mod.rs                         MODIFIED — pub mod chunker; pub mod events; pub mod worker; pub mod store_insert;

src-tauri/src/
├── commands/chat.rs               MODIFIED — push_and_persist gains agent_project_path: Option<&str>;
│                                              3 call sites updated (lines 413, 622, 652, 1049)
├── models/chat.rs                 MODIFIED — ChatMessage gains is_error: Option<bool> (D1)
├── state.rs                       MODIFIED — AppState.memory_tx field; new() returns (AppState, Receiver);
│                                              new_for_test/new_for_test_async return (Self, Receiver);
│                                              tests that don't care drain the Receiver in a sink task
└── lib.rs                         MODIFIED — setup hook spawns start_memory_worker (after embedder bootstrap)

src-tauri/Cargo.toml               MODIFIED — + tree-sitter, tree-sitter-rust/typescript/python,
                                              + tokenizers (explicit dep — already transitive via fastembed),
                                              + tracing (replaces log::warn calls in this phase only IF the
                                                rest of the crate uses tracing; otherwise keep log::warn)
```

### File visibility summary

| Symbol | Visibility | Why |
|--------|------------|-----|
| `MemoryEvent`, `MemorySource` | `pub(crate)` | Constructed in `commands/chat.rs` (same crate). Never serialized cross-process. |
| `ChunkerKind`, `Chunk`, `select_chunker`, `ChatChunker`, `CodeChunker` | `pub(crate)` | Only the worker calls them. |
| `TokenSplitter` | `pub(crate)` | Shared between chat & code-fallback inside `memory/chunker/`. |
| `memory_worker` | `pub(crate)` | The spawn entry point that goes to `lib.rs::start_memory_worker`. |
| `start_memory_worker` | `pub(crate)` | Called from `lib.rs::run()` setup hook. Mirrors `start_embedder_bootstrap` pattern. |
| `MemoryWorkerError` | `pub(crate)` | Internal; not surfaced to FE. Used only in `tracing::warn!` formatting. |
| `MemoryStore::insert_memory_with_vector` | `pub` | Future Phase 4 retrieval and tests outside `memory::` call it. |
| `NewMemoryRecord`, `MemoryType`, `CreatedByType` | `pub` | Builder used from the worker; tests may use it directly. |
| `MemoryStoreError` | `pub` | Returned by `insert_memory_with_vector`. |
| `FakeEmbedder` | unchanged `#[cfg(test)] pub(crate)` | Confirmed in Phase 2 — `worker.rs` tests use it via `use crate::memory::embedder::FakeEmbedder`. |

---

## 3. Public API surface (signatures)

### 3.1 `memory/events.rs`

```rust
use crate::models::chat::{ChatMessage, Role};
use serde::{Deserialize, Serialize};

/// One persisted chat message → one event on the ingest channel.
///
/// Built inside `push_and_persist` AFTER the on-disk session append succeeds,
/// so a lost event cannot leave a memory row without its conversation peer.
/// Owned values only — provably `Send + 'static` (see §9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MemoryEvent {
    pub agent_id: String,
    pub project_id: Option<String>,
    pub role: Role,
    pub content: String,
    /// Only `Some(_)` for `Role::Tool` messages. Drives `memory_type` selection.
    pub is_error: Option<bool>,
    pub source: MemorySource,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemorySource {
    /// Chat turn — user/assistant/tool message from `push_and_persist`.
    Chat,
    /// Compaction summary (reserved — Phase 5 may emit; not produced in Phase 3).
    Compaction,
    /// File-watcher-driven code ingest (reserved — Phase 4+).
    CodeFile,
}

impl MemoryEvent {
    /// Build an event from a persisted `ChatMessage`.
    ///
    /// Returns `None` for messages we deliberately do NOT ingest:
    ///   - `Role::System` (LOCKED D8 — never reaches push_and_persist anyway, defensive)
    ///   - `Role::Assistant` with empty content AND `tool_calls = Some(_)`
    ///     (the "I'm about to call a tool" announcement — no content to embed)
    pub fn from_message(
        agent_id: String,
        project_id: Option<String>,
        msg: &ChatMessage,
        source: MemorySource,
    ) -> Option<Self> {
        match msg.role {
            Role::System => None,
            Role::Assistant if msg.content.is_empty() && msg.tool_calls.is_some() => None,
            _ => Some(Self {
                agent_id,
                project_id,
                role: msg.role.clone(),
                content: msg.content.clone(),
                is_error: msg.is_error,
                source,
                timestamp_ms: msg.timestamp_ms,
            }),
        }
    }
}

// Compile-time Send + 'static proof (Phase 2 pattern).
const _: fn() = || {
    fn _send_static<T: Send + 'static>() {}
    _send_static::<MemoryEvent>();
};
```

### 3.2 `memory/chunker/mod.rs`

```rust
use crate::models::chat::Role;

pub(crate) mod chat;
pub(crate) mod code;
pub(crate) mod token_splitter;

pub(crate) use chat::ChatChunker;
pub(crate) use code::CodeChunker;

/// Which chunker the worker dispatches to for this event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChunkerKind {
    Chat,
    Code,
    /// Worker drops the event without inserting (e.g. empty assistant tool-call msg —
    /// already filtered by `MemoryEvent::from_message`, but kept as a defensive arm).
    Skip,
}

/// One unit of content that becomes ONE row in `memory` + one in `memory_vectors`.
#[derive(Debug, Clone)]
pub(crate) struct Chunk {
    pub text: String,
    /// Approximate token count (480 cap target). Used for telemetry only;
    /// the cap is enforced inside the chunker itself.
    pub token_count: usize,
    /// Optional header describing context (e.g. "fn foo (rust)") — currently
    /// unused by `code.rs` minimal impl but reserved so future enrichment in
    /// Phase 4 retrieval can re-rank by header without a schema change.
    pub header: Option<String>,
}

/// Route an event to its chunker. Pure function — no I/O.
pub(crate) fn select_chunker(
    role: Role,
    is_error: Option<bool>,
    content: &str,
) -> ChunkerKind {
    if content.is_empty() {
        return ChunkerKind::Skip;
    }
    match role {
        // Phase 3: ALL chat-message roles route to the chat chunker.
        // CodeFile events (Phase 4 feeder) will set Role::User + is_error=None
        // with a marker source; that path is unreachable in Phase 3 because
        // push_and_persist is the sole producer.
        Role::User | Role::Assistant | Role::Tool => {
            // is_error only changes memory_type at insert time, not chunker selection.
            let _ = is_error;
            ChunkerKind::Chat
        }
        Role::System => ChunkerKind::Skip, // LOCKED D8 — defensive
    }
}
```

### 3.3 `memory/chunker/token_splitter.rs`

```rust
use std::sync::Arc;
use tokenizers::Tokenizer;

use super::Chunk;

/// Pure "split this text by token cap, no overlap" helper.
///
/// Knows NOTHING about chat speakers OR AST. Both `ChatChunker` (after speaker
/// boundary) and `CodeChunker` (degraded fallback for unknown languages or
/// parse failures) feed text through here. Decouples the cap policy from the
/// upstream semantic split — R6 mitigation in proposal.
pub(crate) struct TokenSplitter {
    tokenizer: Option<Arc<Tokenizer>>,
}

impl TokenSplitter {
    /// Real tokenizer-backed splitter (preferred, post-bootstrap).
    pub fn with_tokenizer(tokenizer: Arc<Tokenizer>) -> Self {
        Self { tokenizer: Some(tokenizer) }
    }

    /// Byte-length fallback when `tokenizer.json` is not yet on disk
    /// (e.g. ChatChunker built before the Phase 2 download finishes).
    /// 1 token ≈ 4 bytes for the languages we support (es/en source code/text).
    pub fn fallback() -> Self {
        Self { tokenizer: None }
    }

    /// Split `text` so each output `Chunk` has `<= max_tokens` tokens.
    /// No overlap. Preserves UTF-8 boundaries.
    pub fn split(&self, text: &str, max_tokens: usize) -> Vec<Chunk> {
        match &self.tokenizer {
            Some(tok) => self.split_by_tokens(text, max_tokens, tok),
            None      => self.split_by_bytes(text, max_tokens.saturating_mul(4)),
        }
    }

    fn split_by_tokens(&self, text: &str, max: usize, tok: &Tokenizer) -> Vec<Chunk> {
        // Encode once, slice by offsets (offsets is Vec<(start_byte, end_byte)>).
        // tokenizers 0.22.2 surface — see Phase 2 dep notes.
        let enc = match tok.encode(text, false) {
            Ok(e) => e,
            Err(_) => return self.split_by_bytes(text, max.saturating_mul(4)),
        };
        let offsets = enc.get_offsets();
        if offsets.is_empty() {
            return vec![];
        }
        let mut out = Vec::new();
        let mut start_byte = offsets[0].0;
        let mut i = 0;
        while i < offsets.len() {
            let end_idx = (i + max).min(offsets.len()) - 1;
            let end_byte = offsets[end_idx].1;
            // Safe slice: tokenizers guarantees offsets land on char boundaries.
            let slice = &text[start_byte..end_byte];
            out.push(Chunk {
                text: slice.to_string(),
                token_count: end_idx - i + 1,
                header: None,
            });
            i = end_idx + 1;
            if i < offsets.len() {
                start_byte = offsets[i].0;
            }
        }
        out
    }

    fn split_by_bytes(&self, text: &str, max_bytes: usize) -> Vec<Chunk> {
        // Walk char boundaries to avoid splitting multibyte UTF-8 sequences.
        let mut out = Vec::new();
        let mut acc = String::new();
        for ch in text.chars() {
            if acc.len() + ch.len_utf8() > max_bytes && !acc.is_empty() {
                out.push(Chunk {
                    text: std::mem::take(&mut acc),
                    token_count: 0, // unknown in fallback mode
                    header: None,
                });
            }
            acc.push(ch);
        }
        if !acc.is_empty() {
            out.push(Chunk { text: acc, token_count: 0, header: None });
        }
        out
    }
}
```

### 3.4 `memory/chunker/chat.rs`

```rust
use std::path::Path;
use std::sync::Arc;
use tokenizers::Tokenizer;

use super::token_splitter::TokenSplitter;
use super::Chunk;

/// Turn-based chat chunker.
///
/// Phase 3 contract (LOCKED D6, D7):
///   - Speaker boundary is the OUTER hard rule (each call here is ONE turn —
///     `push_and_persist` already separates them).
///   - 480-token cap (under the 512-token model limit, safety margin).
///   - NO overlap.
///   - NO markdown-fence / AST enrichment (deferred to Phase 4).
pub(crate) struct ChatChunker {
    splitter: TokenSplitter,
}

impl ChatChunker {
    /// Build a chunker by loading the tokenizer from the Phase 2 cache.
    /// Falls back to byte-length splitter if the file is absent.
    pub fn new_from_base_dir(base_dir: &Path) -> Self {
        let path = crate::memory::model_download::tokenizer_path(base_dir);
        match Tokenizer::from_file(&path) {
            Ok(tok) => Self {
                splitter: TokenSplitter::with_tokenizer(Arc::new(tok)),
            },
            Err(e) => {
                tracing::warn!(
                    "ChatChunker: tokenizer not loadable at {} ({e}); \
                     falling back to byte-length splitter",
                    path.display()
                );
                Self { splitter: TokenSplitter::fallback() }
            }
        }
    }

    /// Test-only — pre-wired tokenizer.
    #[cfg(test)]
    pub fn with_tokenizer(tok: Arc<Tokenizer>) -> Self {
        Self { splitter: TokenSplitter::with_tokenizer(tok) }
    }

    /// Test-only — force fallback path.
    #[cfg(test)]
    pub fn with_fallback() -> Self {
        Self { splitter: TokenSplitter::fallback() }
    }

    /// Chunk a single turn's content. The speaker boundary is implicit:
    /// the caller (worker) invokes this once per `MemoryEvent`.
    pub fn chunk(&self, content: &str, max_tokens: usize) -> Vec<Chunk> {
        if content.is_empty() {
            return vec![];
        }
        self.splitter.split(content, max_tokens)
    }
}
```

### 3.5 `memory/chunker/code.rs`

```rust
use std::sync::Mutex;
use tree_sitter::{Parser, Tree};

use super::token_splitter::TokenSplitter;
use super::Chunk;

/// AST-aware code chunker. Maintains one `Parser` per supported language
/// behind a `Mutex` (tree-sitter `Parser` is `Send` but NOT `Sync` —
/// verified via the `const _: fn()` block below).
///
/// Phase 3 scope: Rust, TypeScript, Python. Unknown languages OR parse
/// failures degrade to the shared `TokenSplitter`. NEVER panics.
pub(crate) struct CodeChunker {
    rust:       Mutex<Parser>,
    typescript: Mutex<Parser>,
    python:     Mutex<Parser>,
    fallback:   TokenSplitter,
}

impl CodeChunker {
    pub fn new() -> Self {
        Self {
            rust:       Mutex::new(make_parser(tree_sitter_rust::LANGUAGE.into())),
            typescript: Mutex::new(make_parser(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())),
            python:     Mutex::new(make_parser(tree_sitter_python::LANGUAGE.into())),
            fallback:   TokenSplitter::fallback(),
        }
    }

    /// Chunk `content`. `language` is a hint: `Some("rs")` / `Some("ts")` /
    /// `Some("py")`; everything else → fallback splitter. Phase 3 sole call
    /// site is the worker with `language = None` because chat-bound events
    /// never know the language; the path exists ahead of Phase 4's file
    /// feeder.
    pub fn chunk(&self, content: &str, language: Option<&str>) -> Vec<Chunk> {
        let parser_mu = match language {
            Some("rs")            => &self.rust,
            Some("ts") | Some("tsx") => &self.typescript,
            Some("py")            => &self.python,
            _                     => return self.fallback.split(content, 480),
        };
        let tree_opt: Option<Tree> = {
            let mut p = parser_mu.lock().unwrap_or_else(|e| e.into_inner());
            p.parse(content, None)
        };
        let Some(tree) = tree_opt else {
            return self.fallback.split(content, 480);
        };
        // Walk top-level named children; one chunk per node, cap at 480 tokens
        // via the fallback splitter when a single node exceeds the limit.
        let bytes = content.as_bytes();
        let mut chunks = Vec::new();
        let mut cursor = tree.root_node().walk();
        for node in tree.root_node().children(&mut cursor) {
            if !node.is_named() { continue; }
            let snippet = std::str::from_utf8(&bytes[node.start_byte()..node.end_byte()])
                .unwrap_or("");
            let header  = Some(format!("{} ({})", node.kind(),
                language.unwrap_or("unknown")));
            // Each node may itself exceed 480 tokens — split it.
            for mut c in self.fallback.split(snippet, 480) {
                c.header = header.clone();
                chunks.push(c);
            }
        }
        if chunks.is_empty() {
            return self.fallback.split(content, 480);
        }
        chunks
    }
}

fn make_parser(lang: tree_sitter::Language) -> Parser {
    let mut p = Parser::new();
    // .set_language returns Err only for grammar-version mismatch; we ship
    // the grammars ourselves, so an Err here is a compile-time-detectable
    // dep bug. Treat as fatal at construction (panics happen once at app
    // startup inside CodeChunker::new, BEFORE any chat traffic).
    p.set_language(&lang).expect("tree-sitter language version mismatch");
    p
}

// Compile-time Send check. CodeChunker is Send (Mutex<Parser> + TokenSplitter
// are both Send). It is also Sync because Mutex provides Sync regardless of
// Parser's bounds.
const _: fn() = || {
    fn _send<T: Send>() {}
    fn _sync<T: Sync>() {}
    _send::<CodeChunker>();
    _sync::<CodeChunker>();
};
```

### 3.6 `memory/worker.rs`

```rust
use std::sync::Arc;
use once_cell::sync::OnceCell;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::memory::chunker::{select_chunker, ChatChunker, ChunkerKind, CodeChunker};
use crate::memory::embedder::{Embedder, EmbedError};
use crate::memory::events::{MemoryEvent, MemorySource};
use crate::memory::store_insert::{CreatedByType, MemoryType, NewMemoryRecord};
use crate::memory::MemoryStore;

/// Worker-internal error type. Never surfaced to FE — only used as `tracing`
/// payload formatting. Recoverable variants are logged + continue; there is
/// NO fatal variant in Phase 3 (the loop only exits when the channel closes).
#[derive(Debug, thiserror::Error)]
pub(crate) enum MemoryWorkerError {
    #[error("embed failed: {0}")]
    Embed(#[from] EmbedError),
    #[error("store insert failed: {0}")]
    Store(#[from] crate::memory::store_insert::MemoryStoreError),
    #[error("chunker produced 0 chunks for non-empty content")]
    EmptyChunks,
}

/// Top-level worker loop. One per app instance.
///
/// Drains events FIFO. Failure isolation per chunk (see lifecycle diagram §1).
/// Exits cleanly when all `Sender`s are dropped (app shutdown).
pub(crate) async fn memory_worker(
    mut rx: UnboundedReceiver<MemoryEvent>,
    store: Arc<MemoryStore>,
    embedder_cell: Arc<OnceCell<Arc<dyn Embedder>>>,
    chat_chunker: Arc<ChatChunker>,
    code_chunker: Arc<CodeChunker>,
) {
    while let Some(evt) = rx.recv().await {
        if let Err(e) = process_event(
            &evt, &store, &embedder_cell, &chat_chunker, &code_chunker,
        ).await {
            tracing::warn!(
                agent_id = %evt.agent_id,
                role = ?evt.role,
                "ingest event dropped: {e}"
            );
        }
    }
    tracing::info!("memory_worker shutting down — channel closed");
}

async fn process_event(
    evt: &MemoryEvent,
    store: &Arc<MemoryStore>,
    embedder_cell: &Arc<OnceCell<Arc<dyn Embedder>>>,
    chat_chunker: &Arc<ChatChunker>,
    code_chunker: &Arc<CodeChunker>,
) -> Result<(), MemoryWorkerError> {
    let kind = select_chunker(evt.role.clone(), evt.is_error, &evt.content);
    let chunks = match kind {
        ChunkerKind::Chat => chat_chunker.chunk(&evt.content, 480),
        ChunkerKind::Code => code_chunker.chunk(&evt.content, None),
        ChunkerKind::Skip => return Ok(()),
    };
    if chunks.is_empty() {
        // Content was all-whitespace or empty after chunker policy. Not an error.
        return Ok(());
    }

    // LOCKED D4 — embedder not ready: drop the entire event.
    let Some(embedder) = embedder_cell.get().cloned() else {
        tracing::warn!(
            agent_id = %evt.agent_id,
            "embedder NotReady; dropping {} chunks",
            chunks.len()
        );
        return Ok(());
    };

    for chunk in chunks {
        match embedder.embed(&chunk.text).await {
            Ok(vec) => {
                let record = NewMemoryRecord::from_event_and_chunk(evt, &chunk);
                store.insert_memory_with_vector(record, vec).await?;
            }
            Err(EmbedError::NotReady | EmbedError::WorkerGone) => {
                // Cell went empty mid-stream or worker thread died — abort the
                // rest of this event. Subsequent events will hit the same arm.
                tracing::warn!(
                    agent_id = %evt.agent_id,
                    "embedder became unavailable mid-event; dropping rest"
                );
                return Ok(());
            }
            Err(e) => {
                // InferenceFailed / LoadFailed — single-chunk problem, skip & continue.
                tracing::warn!(
                    agent_id = %evt.agent_id,
                    "embed failed for chunk: {e}; skipping"
                );
            }
        }
    }
    Ok(())
}

/// Spawn entry point — called from `lib.rs::run()` setup hook.
///
/// Builds the two chunkers once, then hands ownership to the worker future.
/// Returns immediately; the spawned task lives for the app lifetime.
pub(crate) fn start_memory_worker(
    app_handle: tauri::AppHandle,
    memory_rx: UnboundedReceiver<MemoryEvent>,
) {
    use tauri::Manager;
    let state = app_handle.state::<crate::state::AppState>();
    let store         = state.memory_store.clone();
    let embedder_cell = state.embedder.clone();
    let base_dir      = state.base_dir.clone();

    tauri::async_runtime::spawn(async move {
        let chat_chunker = Arc::new(ChatChunker::new_from_base_dir(&base_dir));
        let code_chunker = Arc::new(CodeChunker::new());
        memory_worker(memory_rx, store, embedder_cell, chat_chunker, code_chunker).await;
    });
}

// Compile-time Send proof — every captured value MUST be Send + 'static so
// tauri::async_runtime::spawn accepts the future.
const _: fn() = || {
    fn _send<T: Send>() {}
    _send::<UnboundedReceiver<MemoryEvent>>();
    _send::<Arc<MemoryStore>>();
    _send::<Arc<OnceCell<Arc<dyn Embedder>>>>();
    _send::<Arc<ChatChunker>>();
    _send::<Arc<CodeChunker>>();
};
```

### 3.7 `memory/store_insert.rs`

```rust
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::memory::chunker::Chunk;
use crate::memory::events::{MemoryEvent, MemorySource};
use crate::memory::MemoryStore;
use crate::models::chat::Role;

/// Type of memory row this insert produces. Stored as TEXT in `memory.memory_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    ChatMessage,
    ToolError,
    /// Reserved for Phase 4+ (CodeFile feeder).
    CodeBlock,
}
impl MemoryType {
    fn as_str(self) -> &'static str {
        match self {
            Self::ChatMessage => "chat_message",
            Self::ToolError   => "tool_error",
            Self::CodeBlock   => "code_block",
        }
    }
}

/// `created_by_type` discriminator. Stored as TEXT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatedByType {
    Agent,
    User,
    System,
}
impl CreatedByType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Agent  => "agent",
            Self::User   => "user",
            Self::System => "system",
        }
    }
}

/// Typed INSERT payload for `memory` + `memory_vectors`. Builder pattern via
/// the associated `from_event_and_chunk` ctor keeps the worker call site lean.
#[derive(Debug, Clone)]
pub struct NewMemoryRecord {
    /// Schema CHECK constraint: see V1__init.sql. Phase 3 always emits 'agent'.
    pub scope: &'static str,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub content: String,
    pub memory_type: MemoryType,
    /// Stored as TEXT in `memory.source`. Phase 3 maps `MemorySource::Chat → "chat"`.
    pub source: MemorySource,
    pub created_by_type: CreatedByType,
    pub created_by_ref: String,
    /// 0..10 stored as REAL in `memory.importance`. Phase 3 cascade:
    ///   ChatMessage  → 5.0 (schema default)
    ///   ToolError    → 6.0 (slightly above default; Phase 6 sycophancy refines)
    pub importance: f64,
    pub created_at_ms: u64,
}

impl NewMemoryRecord {
    pub fn from_event_and_chunk(evt: &MemoryEvent, chunk: &Chunk) -> Self {
        let memory_type = match (evt.role.clone(), evt.is_error) {
            (Role::Tool, Some(true)) => MemoryType::ToolError,
            (Role::Tool, _)          => MemoryType::ChatMessage,
            (Role::User, _) | (Role::Assistant, _) => MemoryType::ChatMessage,
            // Defensive: events.rs::from_message filters Role::System; if one
            // slips through (future change), it goes through chat_message — not
            // a panic, since unreachable!() in async code crashes the worker task.
            (Role::System, _) => MemoryType::ChatMessage,
        };
        let created_by_type = match evt.role {
            Role::User      => CreatedByType::User,
            Role::Assistant => CreatedByType::Agent,
            Role::Tool      => CreatedByType::System,
            Role::System    => CreatedByType::System,
        };
        let importance = if matches!(memory_type, MemoryType::ToolError) { 6.0 } else { 5.0 };
        Self {
            scope: "agent",
            agent_id: Some(evt.agent_id.clone()),
            project_id: evt.project_id.clone(),
            content: chunk.text.clone(),
            memory_type,
            source: evt.source,
            created_by_type,
            created_by_ref: evt.agent_id.clone(),
            importance,
            created_at_ms: evt.timestamp_ms,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryStoreError {
    #[error("vector dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("tokio_rusqlite error: {0}")]
    Connection(#[from] tokio_rusqlite::Error),
}

impl MemoryStore {
    /// Atomically INSERT a `memory` row + its matching `memory_vectors` row.
    ///
    /// Transaction shape: open → INSERT memory RETURNING id → INSERT memory_vectors
    /// (rowid, embedding) → commit. If any step fails the transaction rolls back —
    /// either both rows land or neither do. This invariant is what Phase 4
    /// retrieval relies on (memory_vectors KNN → JOIN memory ON rowid = id).
    pub async fn insert_memory_with_vector(
        &self,
        record: NewMemoryRecord,
        embedding: Vec<f32>,
    ) -> Result<i64, MemoryStoreError> {
        // Dimension is fixed by the embedder model (384 for multilingual-e5-small).
        // Validate before hitting SQLite — vec0 would also reject but its error
        // message is less informative.
        if embedding.len() != 384 {
            return Err(MemoryStoreError::DimensionMismatch {
                expected: 384,
                actual: embedding.len(),
            });
        }
        let bytes = serialize_f32_vec(&embedding);
        let created_at = record.created_at_ms as i64;

        self.conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, project_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        record.scope,
                        record.agent_id,
                        record.project_id,
                        record.content,
                        record.memory_type.as_str(),
                        match record.source {
                            MemorySource::Chat       => "chat",
                            MemorySource::Compaction => "compaction",
                            MemorySource::CodeFile   => "code_file",
                        },
                        created_at,
                        record.created_by_type.as_str(),
                        record.created_by_ref,
                        created_at,
                        created_at,
                        record.importance,
                    ],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                    params![id, bytes],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .map_err(MemoryStoreError::from)
    }
}

fn serialize_f32_vec(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for &f in v { out.extend_from_slice(&f.to_le_bytes()); }
    out
}
```

---

## 4. Integration wiring (Rust snippets)

### a) `AppState` field + ctor signature change

```rust
// state.rs — additions
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use crate::memory::events::MemoryEvent;

pub struct AppState {
    // … existing fields …
    /// Producer side of the ingest pipeline. Cheaply cloneable (`Sender`
    /// is an `Arc`-like). Consumed by the worker spawned in `lib.rs::run()`
    /// setup hook. Unbounded — chat hot path MUST NOT block (LOCKED D2).
    pub memory_tx: UnboundedSender<MemoryEvent>,
}

impl AppState {
    /// Build production AppState + the Receiver the setup hook will hand to
    /// the worker. The Receiver MUST be moved out of AppState — only one
    /// consumer end exists per channel by design.
    pub fn new() -> (Self, UnboundedReceiver<MemoryEvent>) {
        // … existing init …
        let (memory_tx, memory_rx) = mpsc::unbounded_channel();
        let state = Self {
            // … existing fields …
            memory_tx,
        };
        (state, memory_rx)
    }

    #[cfg(test)]
    pub fn new_for_test() -> (Self, UnboundedReceiver<MemoryEvent>) { /* same shape */ }

    #[cfg(test)]
    pub async fn new_for_test_async() -> (Self, UnboundedReceiver<MemoryEvent>) { /* same */ }
}
```

Note: the API CHANGE (return tuple instead of `Self`) is intentional — it forces every caller (lib.rs + every test) to acknowledge that someone owns the Receiver. Otherwise the Receiver leaks (chat events silently dropped). Test helpers that don't care can drain in a sink task:

```rust
// test pattern
let (state, mut rx) = AppState::new_for_test_async().await;
tokio::spawn(async move { while rx.recv().await.is_some() {} });
```

### b) Setup hook spawn (`lib.rs::run()`)

```rust
// lib.rs — modified .manage(...) and setup chain
let (state, memory_rx) = AppState::new();
tauri::Builder::default()
    .plugin(/* … */)
    .manage(state)
    .setup(move |app| {
        if cfg!(debug_assertions) { /* … existing log plugin … */ }

        // Phase 2 — unchanged.
        start_embedder_bootstrap(app.handle().clone());

        // Phase 3 — NEW. memory_rx is moved into the spawned task.
        crate::memory::worker::start_memory_worker(app.handle().clone(), memory_rx);

        Ok(())
    })
    .invoke_handler(/* … */)
    .run(/* … */)
    .expect(/* … */);
```

`memory_rx` is captured by the `move` closure and consumed exactly once inside `start_memory_worker` (which spawns the worker task that owns it).

### c) `ChatMessage` field (D1)

```rust
// models/chat.rs — modified ChatMessage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    #[serde(default = "generate_message_id")]
    pub id: String,
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default = "current_time_millis")]
    pub timestamp_ms: u64,
    /// Phase 3 (LOCKED D1) — only `Some(true)` on `Role::Tool` messages whose
    /// underlying tool execution surfaced an error. Default `None` keeps old
    /// persisted session JSON forward-compatible (R1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}
```

### d) `chat.rs:1049` setter + `push_and_persist` send

```rust
// commands/chat.rs — modified push_and_persist signature (NEW: agent_project_path)
fn push_and_persist(
    state: &AppState,
    agent_id: &str,
    agent_project_path: Option<&str>,
    message: ChatMessage,
) {
    // … existing lock-order + persist logic — UNCHANGED …
    if let Err(e) = session.append_message(agent_id, entry.last().unwrap()) {
        entry.pop();
        log::error!("Session persist failed, rolled back: {}", e);
        return; // do NOT emit ingest event for messages that failed to persist
    }

    // ── Phase 3 ingest emit (NEW) ──
    let project_id = agent_project_path
        .filter(|s| !s.is_empty())
        .map(|s| crate::memory::derive_project_id(std::path::Path::new(s)));
    if let Some(evt) = crate::memory::events::MemoryEvent::from_message(
        agent_id.to_string(),
        project_id,
        entry.last().unwrap(),
        crate::memory::events::MemorySource::Chat,
    ) {
        if let Err(e) = state.memory_tx.send(evt) {
            log::debug!("memory_rx dropped, ingest event lost: {e}");
        }
    }
}
```

```rust
// commands/chat.rs:1049 — tool result branch (modified)
push_and_persist(
    &state,
    &agent_id,
    agent_project_path.as_deref(),  // NEW arg
    ChatMessage {
        id: generate_message_id(),
        role: Role::Tool,
        content: final_content,
        tool_calls: Some(vec![tc.clone()]),
        timestamp_ms: current_time_millis(),
        is_error: Some(is_error),   // NEW field — value in scope from line 811
    },
);
```

Other 3 call sites (lines 413, 622, 652) get the new arg and `is_error: None` on the `ChatMessage` literal — mechanical.

---

## 5. Error types

### 5.1 `MemoryWorkerError` (worker-internal)

```rust
#[derive(Debug, thiserror::Error)]
pub(crate) enum MemoryWorkerError {
    #[error("embed failed: {0}")]
    Embed(#[from] EmbedError),
    #[error("store insert failed: {0}")]
    Store(#[from] MemoryStoreError),
    /// Defensive — never reached in Phase 3 (chunker never returns empty for
    /// non-empty content), but kept so a future regression surfaces in logs.
    #[error("chunker produced 0 chunks for non-empty content")]
    EmptyChunks,
}
```

**All variants recoverable** — the worker logs and continues. There is NO fatal variant. The ONLY way the worker exits is `rx.recv() → None` (channel closed = app shutdown). This makes the failure isolation contract enforceable by reading the worker source: every `?` in `process_event` propagates to the top-level `if let Err(e) = … tracing::warn!(…)`, never to a `return Err(_)` of the outer loop.

### 5.2 `MemoryStoreError` (new, in `store_insert.rs`)

```rust
#[derive(Debug, thiserror::Error)]
pub enum MemoryStoreError {
    #[error("vector dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("tokio_rusqlite error: {0}")]
    Connection(#[from] tokio_rusqlite::Error),
}
```

**Why a new enum, not extending `MemoryError`**: `MemoryError` (Phase 1) is the open/migration error surface. Mixing INSERT-time concerns (dimension mismatch is unique to vec0 inserts) would dilute it. Conversion `MemoryStoreError → MemoryError` is trivial via `From` if a future caller needs it, but Phase 3 does not.

### 5.3 Recoverable vs fatal table

| Error | Source | Worker handling |
|-------|--------|-----------------|
| `EmbedError::NotReady` | Embedder cell empty OR returned mid-stream | LOCKED D4 — drop event (no rows). `warn!`. Continue. |
| `EmbedError::WorkerGone` | fastembed thread died | `warn!`. Break per-event loop. Subsequent events hit same arm. **Worker stays alive.** |
| `EmbedError::InferenceFailed` | Transient ORT error on a single text | `warn!`. Skip this chunk. Other chunks of same event proceed. |
| `EmbedError::LoadFailed` | Should never reach here (LoadFailed is from `from_bytes`, not `embed`) | `warn!`. Skip chunk. |
| `MemoryStoreError::DimensionMismatch` | Embedder swap regression (Phase X) | `warn!`. Skip chunk. |
| `MemoryStoreError::Sqlite` | Disk full, schema violation | `warn!`. Skip chunk. |
| `MemoryStoreError::Connection` | tokio_rusqlite background thread gone | `warn!`. Skip chunk. (If persistent, the entire memory subsystem is broken — chat still works.) |
| `rx.recv() → None` | All `Sender`s dropped (shutdown) | `info!`. `return` cleanly. |

---

## 6. Tokenizer loading strategy

### 6.1 Path helper in `model_download.rs`

Add ONE pub fn that co-locates path knowledge with the downloader (R8 mitigation):

```rust
// memory/model_download.rs — new pub fn at module top-level
pub fn tokenizer_path(base_dir: &Path) -> std::path::PathBuf {
    base_dir
        .join("models")
        .join("multilingual-e5-small")
        .join("tokenizer.json")
}
```

This path is the **same** literal used at line 343 (`cache_dir = base_dir.join("models").join("multilingual-e5-small")`) — extracting the helper means future cache-layout changes touch one place.

### 6.2 Load-or-fallback (in `ChatChunker::new_from_base_dir`)

```rust
let path = crate::memory::model_download::tokenizer_path(base_dir);
match Tokenizer::from_file(&path) {
    Ok(tok)  => Self { splitter: TokenSplitter::with_tokenizer(Arc::new(tok)) },
    Err(e)   => {
        tracing::warn!("tokenizer not loadable: {e}; using byte-length fallback");
        Self { splitter: TokenSplitter::fallback() }
    }
}
```

### 6.3 Cold-start race window

The setup hook spawns:
1. `start_embedder_bootstrap` (downloads model + tokenizer.json).
2. `start_memory_worker` (calls `ChatChunker::new_from_base_dir` synchronously inside the spawned task).

The worker task runs concurrently with the embedder bootstrap. On a **first-ever launch** the tokenizer file is NOT yet on disk when `ChatChunker::new_from_base_dir` runs — so the chunker is initialized in fallback mode. We **do NOT hot-swap** the tokenizer in for the app's lifetime. Trade-off:

- Pro: simpler code (chunker initialized once, never mutates).
- Pro: byte-length splits are within ~30% of token-true splits for our text content, so retrieval is still usable.
- Con: a first-launch chat ingested before the model finishes downloading uses byte chunks for the whole session.
- Acceptable because (a) Phase 4 retrieval is the consumer and ranks by cosine similarity, not chunk-boundary alignment, and (b) the next app launch picks up the proper tokenizer.

**Alternative considered**: spawn the chunker INSIDE the embedder-bootstrap task once the model finishes, then `OnceCell` the chunker into AppState. Rejected because it couples Phase 3 lifecycle to Phase 2's task graph for marginal ranking quality on a first launch. Surfaced as **R-NEW-2** in §11.

---

## 7. Tree-sitter parser caching

### 7.1 Decision

Hold **one `Mutex<Parser>` per language**, owned by `CodeChunker` for the app lifetime.

```rust
pub(crate) struct CodeChunker {
    rust:       Mutex<Parser>,
    typescript: Mutex<Parser>,
    python:     Mutex<Parser>,
    fallback:   TokenSplitter,
}
```

### 7.2 Rationale

- `tree_sitter::Parser` is `Send` but **not** `Sync` (it owns a `*mut TSParser` raw FFI pointer that's safe to move between threads but not to alias). Wrapping in `Mutex<Parser>` makes `Sync` regardless — the FFI pointer is single-accessor at any moment.
- The alternative (one parser per call) costs ~50 µs construction per chunk. Acceptable but wasteful: chat messages average 1-3 chunks, so we'd construct/destroy parsers per turn. Long-lived `Mutex<Parser>` is the standard tree-sitter pattern.
- Contention is irrelevant: Phase 3 has **one consumer** (the worker task). The `Mutex` will never block in practice; it's there purely as a `Sync` adapter. `Mutex<Parser>` is also Send+Sync without any `unsafe`.

### 7.3 Compile-time guard

```rust
const _: fn() = || {
    fn _send<T: Send>() {}
    fn _sync<T: Sync>() {}
    _send::<CodeChunker>();
    _sync::<CodeChunker>();  // <-- this line fails to compile if a future
                              //     refactor removes the Mutex wrapper
};
```

---

## 8. Test infrastructure design

### 8.1 Recommendation: keep it simple

Per the launch prompt, the cleanest path is: tests **construct AppState + drain memory_rx via a known sink**, send synthetic `MemoryEvent`s via `state.memory_tx`, **await worker idle** via a one-shot signal, and **assert via direct SQLite queries** on `state.memory_store.conn`.

No `MemorySink` trait. No `TestHandle` struct. The Phase 2 `FakeEmbedder` is reused as-is.

### 8.2 `worker_test_harness` helper (test-only)

A small `#[cfg(test)]` helper hides the boilerplate without introducing trait abstractions:

```rust
// memory/worker.rs — #[cfg(test)] mod tests
async fn spawn_test_worker(
    store: Arc<MemoryStore>,
    embedder: Arc<dyn Embedder>,
) -> (UnboundedSender<MemoryEvent>, tokio::task::JoinHandle<()>) {
    let cell: OnceCell<Arc<dyn Embedder>> = OnceCell::new();
    cell.set(embedder).ok();
    let cell = Arc::new(cell);

    let chat_chunker = Arc::new(ChatChunker::with_fallback()); // tokenizer absent in tests
    let code_chunker = Arc::new(CodeChunker::new());

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<MemoryEvent>();
    let handle = tokio::spawn(async move {
        memory_worker(rx, store, cell, chat_chunker, code_chunker).await;
    });
    (tx, handle)
}
```

### 8.3 Test pattern — "send N events, await all rows"

```rust
#[tokio::test]
async fn worker_inserts_one_row_per_chunk() {
    let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
    let embedder = Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>;
    let (tx, handle) = spawn_test_worker(store.clone(), embedder).await;

    for i in 0..5 {
        tx.send(MemoryEvent {
            agent_id: "agent-1".into(),
            project_id: None,
            role: Role::User,
            content: format!("turn {i}"),
            is_error: None,
            source: MemorySource::Chat,
            timestamp_ms: i as u64,
        }).unwrap();
    }
    drop(tx); // closes channel — worker exits when drained

    // tokio::time::timeout = real seconds, but only fires on bug (deadlock).
    tokio::time::timeout(std::time::Duration::from_secs(2), handle).await
        .expect("worker timed out").expect("worker panicked");

    let count: i64 = store.conn
        .call(|c| Ok(c.query_row("SELECT COUNT(*) FROM memory", [], |r| r.get(0))?))
        .await.unwrap();
    assert_eq!(count, 5);
}
```

### 8.4 No new mocks beyond `FakeEmbedder`

The `MemoryStore` test ctor (`new_for_test`) gives an in-memory SQLite — that IS the assertion surface. Fault-injection tests for the INSERT transaction can use raw `conn.call(|c| c.execute("DROP TABLE memory_vectors", []))` between events to force the next insert to fail.

---

## 9. Concurrency / Send+Sync proofs

Every type that crosses a task boundary gets a compile-time guard. Phase 2 precedent.

| Type | Bound needed | Why it holds | Guard location |
|------|--------------|--------------|----------------|
| `MemoryEvent` | `Send + 'static` | All fields are owned `String` / `Option<String>` / `Role` (Clone) / `bool` / `u64` / `MemorySource` (Copy) | `memory/events.rs` |
| `MemoryStore` | `Send + Sync` | Phase 1 — `tokio_rusqlite::Connection` is `Clone + Send + Sync` | Phase 1 already has it; not added here |
| `Arc<dyn Embedder>` | `Send + Sync` | Phase 2 trait bound `Embedder: Send + Sync`; existing guard | Phase 2 `embedder.rs` line 161-168 — unchanged |
| `ChatChunker` | `Send + Sync` | `TokenSplitter` contains `Option<Arc<Tokenizer>>`; `tokenizers::Tokenizer` is `Send + Sync` | New guard in `memory/chunker/chat.rs` |
| `CodeChunker` | `Send + Sync` | `Mutex<Parser>` always `Send + Sync`; `TokenSplitter` ditto | New guard in `memory/chunker/code.rs` |
| `UnboundedReceiver<MemoryEvent>` | `Send + 'static` | tokio guarantees | Worker spawn guard in `worker.rs` |

Sample (in `worker.rs` after the `start_memory_worker` fn):

```rust
const _: fn() = || {
    fn _send<T: Send>() {}
    fn _static<T: 'static>() {}
    _send::<UnboundedReceiver<MemoryEvent>>();
    _send::<Arc<MemoryStore>>();
    _send::<Arc<OnceCell<Arc<dyn Embedder>>>>();
    _send::<Arc<ChatChunker>>();
    _send::<Arc<CodeChunker>>();
    _static::<UnboundedReceiver<MemoryEvent>>();
};
```

**`Tokenizer` Send/Sync verification**: `tokenizers 0.22.2` declares `unsafe impl Send for Tokenizer {}` and `unsafe impl Sync for Tokenizer {}` (via its `TokenizerImpl<M, N, PT, PP, D>` generic; `tokenizers::Tokenizer` is a type alias for the standard config). Verified by examining the published crate. The guard catches any regression at compile time.

---

## 10. Cargo.toml additions

```toml
# [dependencies] — append after the Phase 2 block
# RAG Phase 3 (ingest)
tree-sitter            = "0.26"
tree-sitter-rust       = "0.24"
tree-sitter-typescript = "0.23"
tree-sitter-python     = "0.25"

# tokenizers is already a transitive of fastembed v5 (see Cargo.lock — pinned
# at 0.22.2). Declare explicitly so a future fastembed bump cannot silently
# drop or change it.
tokenizers             = "0.22.2"

# tracing — RECOMMEND introducing here ONLY IF the existing crate uses tracing.
# Verified: current codebase uses `log::warn!` / `log::error!`, not tracing.
# Phase 3 therefore stays on `log::` macros for consistency. Do NOT add tracing.
```

**Exact-version pin policy (R2)**: the apply phase runs `cargo update --dry-run` at CHECKPOINT 1 to lock the patch versions in `Cargo.lock`. Cargo.toml stays on the minor (`"0.26"`) so future patch bumps via `cargo update` are explicit, not implicit.

**Build cost**: 3 grammars × ~5-10 s cc compile = ~15-30 s added one-time cost on first `cargo check`. Same `cc` pattern as `sqlite-vec` (Phase 1). Acceptable — documented in proposal R7.

---

## 11. Risks beyond proposal

| ID | Risk | Mitigation |
|----|------|------------|
| <a id="R-NEW-1"></a>R-NEW-1 | Proposal sketch §4 implies the worker can derive `project_id` from `agent_id`. Actual: `push_and_persist` does NOT receive `agent_project_path`; doing the lookup inside the worker would force the worker to re-lock `state.agents` per event (lock contention, race with agent mutation). | Add `agent_project_path: Option<&str>` parameter to `push_and_persist` (§4.d). 3 mechanical call-site updates. Derivation happens AT the call site, BEFORE the channel send. Worker receives a finalized `Option<String>`. **Surface to orchestrator** — this is a clean adaptation but the proposal §4 sketch should be updated for spec/tasks accuracy. |
| <a id="R-NEW-2"></a>R-NEW-2 | Cold-start: ChatChunker initializes BEFORE Phase 2 finishes downloading `tokenizer.json` → first session of a fresh install uses byte-length fallback chunks for the entire session. | Acceptable for v1 (byte-length splits within ~30% of token-true; Phase 4 retrieval is cosine-based, not boundary-sensitive). Documented as known limitation in §6.3. **Alternative**: hot-swap chunker post-bootstrap — adds complexity, deferred to Phase X if needed. |
| <a id="R-NEW-3"></a>R-NEW-3 | `AppState::new()` signature change (returns tuple) breaks every test that calls `AppState::new_for_test_async()` directly without destructuring. | Intentional — the new return shape FORCES every caller to acknowledge ownership of the Receiver. Existing tests get a mechanical update: `let state = AppState::new_for_test_async().await;` → `let (state, mut rx) = AppState::new_for_test_async().await; tokio::spawn(async move { while rx.recv().await.is_some() {} });`. Catalogued in tasks.md as the first apply checkpoint. |
| <a id="R-NEW-4"></a>R-NEW-4 | `Role` is `#[derive(Clone)]` but not `Copy`. `select_chunker(role, …)` taking owned `Role` forces `.clone()` at the worker call site (cheap — enum with no payload — but visible). | Pass by value `Role`; `.clone()` is one-byte memcpy. Alternative: borrow `&Role` — but pattern matching on borrow needs `match *role` everywhere. Current design uses `evt.role.clone()` once per event — negligible. |
| <a id="R-NEW-5"></a>R-NEW-5 | `MemoryEvent::from_message` filters `Role::Assistant` + empty content + `Some(tool_calls)` (the "I'll call a tool" announcement). But the tool _result_ for that call (`Role::Tool` at line 1049) IS ingested. If the announcement carries `content` like "Let me run X", that content IS ingested. Mostly OK, but could lead to chat history that ingests the human-readable preamble AND the tool result, producing two semantically similar memories. | Accepted. Phase 6 sycophancy / Mem0 resolver will dedupe at the importance layer. Documented in spec scenarios. |
| <a id="R-NEW-6"></a>R-NEW-6 | `tree-sitter-typescript` 0.23 has TWO languages exposed (`LANGUAGE_TYPESCRIPT` for `.ts`, `LANGUAGE_TSX` for `.tsx`). The current `make_parser` only wires `LANGUAGE_TYPESCRIPT`. `.tsx` files would parse as TS — usually fine, but JSX syntax inside .tsx will produce parser errors and degrade to fallback. | Acceptable for Phase 3 (no .tsx files reach the worker — chat-only producer in Phase 3). Phase 4 file-watcher feeder will wire `LANGUAGE_TSX` as a separate parser slot. Documented as a one-line extension. |
| <a id="R-NEW-7"></a>R-NEW-7 | `tokio_rusqlite::Connection::call` errors include `ConnectionClosed`, `Close`, and `Error(rusqlite::Error)`. Current `From<tokio_rusqlite::Error> for MemoryStoreError` lumps all into `Connection(_)`. A connection close mid-transaction would surface as `Connection(_)` not `Sqlite(_)`. | Cosmetic — `tracing::warn` prints either way. If Phase 5 wants finer error routing, it can split. |

---

## 12. Done definition (re-statement)

Phase 3 design is consumed by spec → tasks → apply → verify. The design is done when:

- All 8 LOCKED proposal decisions are reflected in §3 signatures + §4 wiring.
- ASCII lifecycle (§1) shows every failure branch the failure-mode tests (proposal §7) MUST exercise.
- Tree-sitter caching strategy (§7) survives the `Send + Sync` compile-time guards (§9).
- The 7 surfaced R-NEW risks are visible to the orchestrator for proposal re-litigation if needed.
- No `cargo build` was invoked while drafting (user rule).
