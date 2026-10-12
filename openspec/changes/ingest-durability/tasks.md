# ingest-durability — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código que lo hace pasar, y el commit compila y deja `cargo test --workspace` en verde. Los requisitos (R*) son los de `spec.md` y las decisiones (D*) las de `design.md`.

El orden va de adentro hacia afuera: primero el worker y el runtime (`clawg-core`), después el daemon, el cliente y la herramienta MCP (`clawg-cli`). Hasta la tarea 7 el contrato visible del save no cambia, así que cada commit intermedio deja el producto como está hoy.

## 0. Antes de empezar

- [x] 0.1 Correr `cargo test --workspace` en la rama y anotar el conteo (base: 1.304, 0 fallos). En Windows, con `--target-dir` aparte.
- [ ] 0.2 Medir el p50 y el p99 de un save con el embedder real, en el perfil dev, de un texto corto y de uno de varios chunks. **Necesita autorización** para usar `~/.clawg-dev`. Con eso se confirma o se ajusta el timeout de 10 s antes de la tarea 7.

## 1. El worker informa qué escribió y qué falló (R3, D1)

- [x] 1.1 RED: en `worker.rs` (mod tests), `process_event_reports_the_rows_written_before_an_insert_failed` (2 chunks, falla el insert del segundo) y `process_event_stops_at_the_first_failed_insert` (3 chunks, falla el segundo: el tercero no se intenta). Un tercer test, `process_event_reports_reinforced_with_the_existing_id`, cubre el camino del dedup.
- [x] 1.2 `absorb_duplicate` devuelve `Option<i64>` (el id reforzado) en vez de `bool`.
- [x] 1.3 `process_event` devuelve `IngestReport { memories, failure }`. Los descartes por embed (`worker.rs:322-331`), por vectores de menos (`:333-342`) y por insert (`:369-372`) cargan `failure`. El insert corta en el primer fallo.
- [x] 1.4 El bucle del worker loguea `failure` como hoy. Nada más lo lee todavía.
- Commit: `refactor(core): report written rows and the first failure from ingest`

## 2. Cola acotada con canal de respuesta (R9 en el core, D1, D4)

- [x] 2.1 RED: en `runtime.rs` (mod tests), `a_request_with_a_reply_channel_receives_its_ingest_report` y `the_ingest_queue_refuses_a_send_past_its_capacity` (capacidad 2 con `try_send`: el tercero es `Full`).
- [x] 2.2 `IngestRequest { event, reply: Option<oneshot::Sender<IngestReport>> }`. `memory_tx` pasa a `Sender<IngestRequest>` con capacidad 1.024, e `ingest_worker` recibe un `Receiver<IngestRequest>`. Un `send` sobre un `oneshot` cerrado se ignora.
- [x] 2.3 `MemoryRuntime::new_for_test` acepta la capacidad. Producción usa la constante.
- [x] 2.4 Actualizar el comentario de `runtime.rs:47-49`: el productor sigue sin bloquearse porque usa `try_send`.
- [x] 2.5 Adaptar los productores y los tests que usan la cola: `api.rs:456` (con `try_send` y `reply: None`, mismo `202` de hoy), `crates/clawg-core/tests/public_api.rs:155`, `:195`, `:237`, `crates/clawg-cli/tests/api.rs:78-87` y los tests de `runtime.rs`.
- Commit: `feat(core): bound the ingest queue and let a producer await its result`

## 3. Estado del embedder con `failed` (R7 en el core, D2)

- [x] 3.1 RED: en `bootstrap.rs` (mod tests), `a_failed_download_publishes_failed_and_retryable`, `a_contract_mismatch_publishes_failed_and_not_retryable` y `a_loaded_embedder_fills_the_cell_before_publishing_ready`. Test puro `classify_retries_everything_but_a_contract_mismatch`.
- [x] 3.2 `EmbedderStatus { Loading, Ready, Failed { error, retryable } }` y la función pura `classify`.
- [x] 3.3 `run_embedder_bootstrap` recibe el `watch::Sender<EmbedderStatus>` y devuelve el resultado del intento. Llena el cell antes de publicar `Ready`.
- [x] 3.4 `MemoryRuntime` expone `embedder_status()`. `new_for_test` arranca en `Ready`, porque el cell viene lleno.
- [x] 3.5 `serve.rs:84-91` pasa el `Sender`. Sigue habiendo un solo intento.
- Commit: `feat(core): publish whether the embedder is loading, ready or failed`

## 4. Reintento del bootstrap (R8, D3)

- [x] 4.1 RED: en `bootstrap.rs` (mod tests), `retry_delay_doubles_from_30s_and_caps_at_30min` (puro, incluye un intento muy alto sin desbordar), `a_failed_download_is_retried_until_it_succeeds` y `a_contract_mismatch_is_never_retried`, con esperas inyectadas en cero.
- [x] 4.2 `retry_delay(attempt)` y `run_embedder_bootstrap_with_retry`. Durante la espera el estado es `Failed { retryable: true }`, y durante el intento `Loading`.
- [x] 4.3 `serve.rs` lanza el bucle en lugar de un intento.
- Commit: `feat(core): retry a failed embedder bootstrap with capped backoff`

## 5. `/health` y `clawg status` muestran el fallo (R7)

- [x] 5.1 RED: `health_reports_failed_with_the_cause_after_a_failed_bootstrap` en `crates/clawg-cli/tests/api.rs`, y `a_failed_embedder_shows_in_the_status_report` en `status.rs`.
- [x] 5.2 El `Harness` de `tests/api.rs` suma un estado `Failed` que publica el estado sin lanzar un bootstrap.
- [x] 5.3 `HealthBody` (`api.rs:290-302`) suma `embedder_error` y `embedder_retryable`, `null` salvo con `failed`. El `Health` del cliente (`client.rs:83-90`) suma los dos campos con `#[serde(default)]`.
- Commit: `feat(cli): show a failed embedder and its cause in health and status`

## 6. El save rechaza y avisa según el estado (R5, R6, R9)

- [x] 6.1 RED en `tests/api.rs`: `a_save_after_the_bootstrap_failed_is_503_with_the_cause`, `search_after_the_bootstrap_failed_says_failed_not_loading`, `a_save_while_the_model_loads_answers_202_and_lands_once_it_loads` y `a_save_into_a_full_queue_is_503_queue_full`.
- [x] 6.2 El handler del save lee el estado: con `Failed` responde `503 embedder_failed` sin encolar, con `Loading` responde `202` con `reason: "embedder_loading"`, y con la cola llena responde `503 ingest_queue_full`. Con `Ready` sigue respondiendo `202 queued` hasta la tarea 7.
- [x] 6.3 `search` responde `embedder_failed` con el mismo cuerpo cuando el estado es `Failed`. `context` queda igual a propósito: nunca respondió `embedder_not_ready` y sin embedder cae a la recuperación léxica.
- Commit: `feat(cli): refuse saves while the embedder is failed or the queue is full`

## 7. Save confirmado (R1, R2, R3, R4, D5)

- [x] 7.1 RED en `tests/api.rs`: `a_save_answers_201_with_the_id_of_the_row_it_wrote`, `a_multi_chunk_save_returns_one_id_per_chunk`, `saving_the_same_text_twice_returns_the_same_id_as_reinforced`, `a_failed_embed_reaches_the_caller_as_an_error` y `a_save_the_worker_does_not_confirm_in_time_answers_202_unconfirmed`. Test puro de `save_response` en `api.rs` (mod tests) que recorre cada estado y cada resultado.
- [x] 7.2 `AppCtx` suma el timeout de confirmación (10 s por defecto, o el valor que salga de 0.2). El `Harness` lo baja a 100 ms.
- [x] 7.3 Con `Ready`, el handler encola con `reply`, espera con `tokio::time::timeout` y arma la respuesta con `save_response`. Un `oneshot` cerrado sin respuesta se trata como el timeout (test `a_save_whose_reply_is_dropped_answers_202_unconfirmed`).
- [x] 7.4 Reescribir `a_saved_memory_is_queued_then_becomes_a_listable_row` (`tests/api.rs:252`): ya no hace falta el polling.
- [x] 7.5 Actualizar la doc del handler (`api.rs:418-431`): la razón del consumidor único sigue valiendo, pero el save ya no es "accepted".
- Commit: `feat(cli): answer a save with the ids it wrote or the reason it failed`

## 8. El cliente devuelve el resultado (R11)

- [x] 8.1 RED: `save_returns_the_ids_the_daemon_reported` en `client.rs` (mod tests), y un test de que un error del save conserva `memories`, `detail` y `retryable`.
- [x] 8.2 `DaemonClient::save` devuelve `SaveOutcome::{Saved { memories }, Queued { reason }}`. El cuerpo de error se parsea sin perder esos campos.
- [x] 8.3 `save_memory_impl` (`server.rs:246-281`) compila contra el tipo nuevo y sigue devolviendo `queued` hasta la tarea 9.
- Commit: `feat(cli): return the save outcome from the daemon client`

## 9. La herramienta MCP dice qué pasó (R10)

- [x] 9.1 RED en `server.rs` (mod tests): `save_outcome_text_covers_every_daemon_answer` (puro), `save_memory_returns_the_id_that_delete_memory_accepts`, `save_memory_is_an_error_when_the_embedder_failed` y `explain_does_not_promise_search_when_the_embedder_failed`.
- [x] 9.2 La función pura que arma el texto, y `save_memory_impl` la usa. El comentario de `server.rs:276-278` se actualiza.
- [x] 9.3 `explain` (`server.rs:229-242`) distingue `embedder_failed` de `embedder_not_ready`.
- [x] 9.4 Reescribir `save_then_list_then_search_then_delete_through_the_tools` (`server.rs:757`): usa el id devuelto y deja el polling.
- [x] 9.5 Revisar la descripción de la herramienta `save_memory` para que no prometa un resultado asíncrono.
- Commit: `feat(mcp): tell the model the saved ids or why the save failed`

## 10. Verificación

- [x] 10.1 `cargo test --workspace`: reportar el conteo contra la base de 1.304, con 0 fallos.
- [ ] 10.2 Repetir la medición de 0.2 con el código nuevo y reportar el p50 y el p99 antes y después. **Necesita autorización**, igual que 0.2.
- [ ] 10.3 Prueba manual en el perfil dev, **con autorización**:
  - un `save_memory` devuelve `saved #N`;
  - `delete_memory` con ese id funciona;
  - sin red en el primer arranque, `clawg status` muestra `failed` con la causa, y al volver la red el embedder pasa a `ready` sin reiniciar.
- [x] 10.4 Mover el estado a `verify` en `state.yaml` y actualizar la fila del mapa en `openspec/README.md`.

No se toca ningún archivo de data-preservation. Si ese cambio mergea antes, el conflicto esperable está en `serve.rs` (tareas 3.5 y 4.3).
