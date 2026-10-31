# pipeline-metrics — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## Tramo 1: SQLite y outbox

### 1. La espera y la caché de la conexión (P1, P2, D1, D2)

- [x] Tests primero: `the_probe_waits_behind_a_call_in_progress`, `the_cache_counters_restart_at_each_reading`, `a_closed_connection_is_an_error_not_a_reading` (`crates/clawg-core/src/memory/conn_probe.rs`).
- [x] `MemoryStore::probe_wait` y `MemoryStore::cache_status` (`CacheStatus`).
- Commit: `feat(core): measure the wait on the store connection and its page cache`

### 2. La sonda del daemon (P1 a P4, D1, D2)

- [x] Tests primero: `the_store_files_are_measured_and_a_missing_one_is_left_out`, `a_minute_reading_records_the_cache_the_files_and_the_outbox`, `a_probe_records_its_wait` (`crates/clawg-cli/src/daemon/store_probe.rs`).
- [x] `daemon/store_probe.rs`; `serve` la arranca y la aborta cuando paran los servidores.
- Commit: `feat(daemon): probe the store connection, its cache, its files and the outbox`

### 3. De punta a punta del tramo (P4)

- [x] `a_daemon_probes_its_store_without_being_asked` en `crates/clawg-cli/tests/binary.rs`.
- Commit: `test(cli): check that a real daemon probes its store`

### 4. Docs del tramo

- [x] `.claude/CLAUDE.md`: las series de la sonda.
- Commit: `docs: describe the store probe metrics`

## Tramo 2: búsqueda y contexto (P5, D3, D6)

- [x] El `Recorder` en `MemoryRuntime` (el del store) y el mismo en el daemon; `a_test_runtime_carries_its_own_recorder`.
- Commit: `feat(core): give the memory runtime the recorder the daemon serves`
- [x] Tests primero de `search.duration{arm}`, `search.knn_retry{table}`, `search.hits`, `search.withheld`; luego la búsqueda.
- Commit: `feat(search): record each arm's duration, the widened windows, the hits and the withheld`
- [x] Tests primero de `context.retrieval_state`, `context.prompt_bytes`, `context.cut{block}`; luego el contexto.
- Commit: `feat(context): record its retrieval state, its size and the blocks the budget cut`

## Tramo 3: ingest (P6, D4)

- [x] Partir `ingest_pieces` por etapa (`split_event`, `gate_pieces`, `embed_pieces`, `store_pieces`), un commit por etapa con sus tests, sin cambiar lo que guarda.
- [x] Tests primero de `ingest.stage`, `ingest.pieces`, `ingest.end_to_end`, `ingest.outcome`, `ingest.queue_depth`; luego el worker.
- Commit: `feat(worker): record each stage of a save, how it ended, its pieces and the queue`

## Tramo 4: embedder (P7)

- [x] Tests primero de `embedder.batch`, `embedder.batch_tokens`, `embedder.wait`, `embedder.load`; luego el canal de inferencia y la carga.
- Commit: `feat(embedder): record each batch, its padded tokens, its wait and each load`

## Tramo 5: docs (P8)

- [x] Tests primero de `docs.refresh.duration`, `docs.refresh.files`, `docs.search.inline_wait`; luego el refresco y `search_docs`.
- Commit: `feat(docs): record each refresh of a docs index and how long a search waited for it`

## Tramo 6: eventos (P9, D5, D8)

- [x] Test primero: un `metrics.db` del esquema 1 sembrado sube al 2 con sus filas.
- [x] La tabla `event`, su poda y su escritura con el minuto; `GET /v1/metrics` lista los últimos.
- Commit: `feat(metrics): keep a run's events in metrics.db`
- [x] Un test y un evento por tipo.
- Commits: `feat(metrics): record model loads, unloads, recoveries, maintenance, backups, imports and failed saves as events`, `feat(metrics): record a model download and the shutdown as events`
- [x] `clawg metrics report --events`.
- Commit: `feat(cli): list the events of each run with metrics report --events`

## Tramo 7: cierre

- [x] P10 en `binary.rs`: `a_daemon_measures_its_pipeline_and_keeps_its_events`.
- [x] `.claude/CLAUDE.md` con lo que quede.
- [x] Verificación ([verify](verify.md)): suite y clippy en Linux; el CI en verde en Linux y Windows (pull requests #100 y #101). El sobrecoste con el modelo real no se midió: pide autorización para el perfil dev y un build sin la sonda.
- [x] Archive: `specs/metrics` (R10 a R18; R2, R3, R4, R7, R8 y R9); `git mv` a `changes/archive/`; `state.yaml`; fila 73 del `README.md` de openspec. El minor no sube: 0.67.0 no se publicó y ya trae el código.
