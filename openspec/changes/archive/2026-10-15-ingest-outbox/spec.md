# ingest-outbox — Especificación

Un save aceptado queda en `memory.db` antes de que el daemon conteste, y termina guardado o a la vista como fallido. Al archivar, R1, R6 y R7 van a [`specs/ingest`](../../../specs/ingest/spec.md) (reemplazan los párrafos de «Timeout de confirmación», «Embedder cargando» y «Cola acotada» que hablan del volcado, y suman el `operation_id` y el bloque `ingest`), R2–R5 y R8 a [`specs/storage`](../../../specs/storage/spec.md) (reemplazan «Ningún evento aceptado se pierde en un apagado ordenado», «Reinyección al arrancar» y «Formato del volcado»), R9 a [`specs/secrets`](../../../specs/secrets/spec.md) y R10 a `specs/storage`, «Migraciones».

## Definiciones

- **Outbox:** la tabla `ingest_outbox` de `memory.db` ([diseño, D1](design.md#d1-una-tabla-en-memorydb)).
- **Save aceptado:** uno que respondió `201` o `202`. Desde este cambio, uno con fila confirmada en el outbox.
- **Fila pendiente / fallida:** `state = 'pending'` / `'failed'`.
- **Tope de intentos (`MAX_ATTEMPTS`):** 3.
- **Progreso:** la lista de posiciones de un save ya escritas, con la fila y la cabecera de cada una ([D2](design.md#d2-progreso-por-pieza)).

## R1. Aceptar es durable

`POST /v1/memories` MUST escribir y confirmar la fila del outbox antes de encolar el evento y antes de responder `201` o `202`. La fila MUST guardar el texto ya enmascarado. El commit de la fila MUST hacerse con `synchronous = FULL`, y la conexión MUST volver después a su valor.

Si la cola rechaza el evento (llena o cerrada), el save MUST responder como hoy (`503 ingest_queue_full` o `500`) y MUST borrar la fila: un save rechazado no queda guardado.

#### Escenario: con el modelo cargando
- GIVEN un daemon con el embedder `loading`
- WHEN se guarda un texto y responde `202 embedder_loading`
- THEN el outbox tiene una fila `pending` con ese texto
- AND si el proceso termina sin apagado ordenado y se abre otro runtime sobre el mismo directorio con el embedder listo, la memoria queda en la base y el outbox vacío

#### Escenario: sin confirmación a tiempo
- GIVEN un worker detenido y un plazo de confirmación de 100 ms
- WHEN se guarda un texto y responde `202 unconfirmed`
- THEN el outbox tiene una fila `pending` con ese texto

#### Escenario: cola llena
- GIVEN una cola de capacidad 2, sin worker, y el embedder `loading`
- WHEN se guardan 3 textos
- THEN el tercero responde `503 ingest_queue_full` y el outbox tiene 2 filas

**Tests:** `a_save_accepted_while_the_model_loads_survives_a_process_that_dies` (`crates/clawg-core/tests/shutdown.rs`); `a_save_accepted_while_the_model_loads_is_in_the_outbox_before_the_answer`, `an_unconfirmed_save_is_in_the_outbox` y `a_save_into_a_full_queue_leaves_no_outbox_row` (`crates/clawg-cli/tests/api.rs`); `accept_writes_the_row_before_it_queues_the_event` y `a_refused_event_leaves_no_row` (`crates/clawg-core/src/runtime.rs`); `durably_writes_under_full_and_puts_the_setting_back` y `recording_a_row_leaves_the_store_on_normal` (`outbox.rs`).

## R2. Terminar borra la fila con lo que escribió

Un save que termina MUST NOT dejar su fila. Cada pieza que el worker inserta o refuerza MUST anotar su posición en el progreso en la misma transacción que la escribe. El vínculo `memory_piece` y el borrado de la fila MUST ir en una transacción.

#### Escenario
- GIVEN un daemon con el embedder listo
- WHEN un save de 3 piezas responde `201`
- THEN el outbox está vacío y `memory_piece` tiene las 3 piezas

**Tests:** `a_confirmed_save_leaves_no_outbox_row` (`tests/api.rs`); `each_piece_is_noted_with_the_row_it_wrote` (`worker.rs`).

## R3. Un replay retoma donde quedó

Al tomar una fila con progreso, el worker MUST saltar las posiciones anotadas, MUST NOT reforzar sus filas y MUST vincular cada una con la cabecera anotada. Si el progreso nombra una posición que el troceo de hoy no tiene, MUST descartarlo con un warn y seguir por el dedup.

#### Escenario
- GIVEN un save de 3 piezas cuyo progreso anota las dos primeras, como si el proceso hubiera muerto antes de la tercera
- WHEN el worker toma la fila
- THEN inserta sólo la tercera, el `access_count` de las dos primeras sigue en 1 y `memory_piece` tiene las 3 con la cabecera original en las piezas 2 y 3

**Tests:** `a_replayed_save_resumes_after_its_last_noted_piece` y `progress_that_does_not_fit_the_split_is_dropped` (`worker.rs`); `resume_positions` (puro, `outbox.rs`).

## R4. Una entrega doble se salta

El worker MUST tomar cada fila a lo sumo una vez por ejecución. Una fila que ya no existe, o que está `failed`, MUST saltarse sin escribir nada.

#### Escenario
- GIVEN una fila pendiente que llega por la lectura de arranque y por el canal
- THEN se procesa una vez y la fila del texto tiene `access_count = 1`

**Test:** `a_row_delivered_twice_is_ingested_once` (`worker.rs`).

## R5. Intentos y fallo a la vista

Tomar una fila MUST sumar un intento antes de procesar. Si la fila ya tenía `MAX_ATTEMPTS`, MUST quedar `failed` con una causa que dice cuántas veces se cortó el proceso, sin procesarse, y el worker MUST seguir con la siguiente.

Un fallo del worker MUST anotar la causa en `last_error`. Un fallo transitorio (`EmbedderUnavailable`) MUST NOT contar como intento. Con el tope alcanzado, la fila MUST quedar `failed`; si no, `pending`, y se reintenta en el próximo arranque, no en la misma ejecución.

Si el handler contesta al que guardó con el fallo (`500` o `503`), MUST borrar la fila: el fallo ya llegó a quien lo tenía que saber. Una fila `failed` MUST NOT borrarse sola.

#### Escenario: evento que corta el proceso
- GIVEN una fila con `attempts = 3`
- WHEN el worker la toma
- THEN queda `failed`, no se escribe ninguna memoria y la fila siguiente se procesa

#### Escenario: fallo después de un `202`
- GIVEN un save aceptado con el embedder cargando y un embedder que devuelve NaN
- WHEN el worker lo procesa
- THEN la fila queda `pending` con `attempts = 1` y `last_error` con la causa

#### Escenario: fallo contestado
- GIVEN un daemon con un embedder cuyo batch falla
- WHEN un save responde `500 ingest_failed`
- THEN el outbox está vacío

**Tests:** `claim_step` y `after_failure` (puros, `outbox.rs`); `a_row_that_stopped_the_process_three_times_fails_and_the_next_one_runs` y `a_failure_after_the_answer_stays_in_the_outbox_with_its_cause` (`worker.rs`); `a_failure_the_caller_was_told_leaves_no_outbox_row` (`tests/api.rs`).

## R6. `operation_id`

Toda respuesta `201` y `202` de `POST /v1/memories` MUST llevar `operation_id`. Sin uno en el pedido, el daemon MUST emitir un UUID. Con uno, MUST validarlo (1 a 128 caracteres ASCII visibles, si no `400`) y usarlo. Mientras exista una fila con ese `operation_id` para ese agente, un save con el mismo MUST NOT crear otra: con la fila `pending` responde `202` con `reason: "already_queued"`; con la fila `failed`, `500 ingest_failed` con su causa.

La herramienta `save_memory` MUST NOT ganar un argumento.

**Tests:** `a_save_answers_with_an_operation_id` y `an_operation_id_sent_twice_keeps_one_row` (`tests/api.rs`); `operation_ids_are_checked` (puro, `api.rs`).

## R7. El agente se entera

`/health` MUST publicar `ingest`: `pending`, `failed`, `oldest_pending_s` y hasta 5 `problems` (`operation_id`, `agent_id`, `state`, `attempts`, `error`) de las filas con causa, las más recientes primero. La lectura MUST NOT tardar más de 250 ms: si vence, `ingest` es `null`. El cliente MUST parsear un `/health` sin el campo.

`memory_status` MUST mostrar el bloque. Los textos de un `202` en `save_memory` MUST nombrar el `operation_id` y decir que un fallo aparece en `memory_status`.

**Tests:** `health_reports_a_failed_save_with_its_operation_id` (`tests/api.rs`); `a_health_without_ingest_still_parses` (`client.rs`); `format_status_shows_the_ingest_block` (`tools.rs`); `save_outcome_text_covers_every_daemon_answer` (actualizado, `server.rs`).

## R8. Arranque y apagado

Al arrancar, el worker MUST importar `ingest-pending.jsonl` si existe: cada evento válido pasa a una fila, enmascarado, en una transacción; el archivo MUST borrarse sólo después del commit; las líneas ilegibles MUST ir a `ingest-pending.rejected.jsonl` como hoy. Después MUST procesar las filas pendientes en orden de id antes que cualquier evento del canal.

Un apagado ordenado MUST seguir drenando hasta `DRAIN_DEADLINE` y MUST NOT escribir `ingest-pending.jsonl`: lo que no empezó queda en el outbox.

#### Escenario: volcado viejo
- GIVEN un `ingest-pending.jsonl` con dos eventos válidos (uno con un secreto) y una línea truncada
- WHEN arranca el worker
- THEN las dos memorias quedan en la base, la del secreto enmascarada, el archivo ya no existe y la línea truncada está en `ingest-pending.rejected.jsonl`

#### Escenario: embedder sin cargar en el apagado
- GIVEN el embedder sin cargar y 3 saves aceptados
- WHEN se hace un apagado ordenado
- THEN no existe `ingest-pending.jsonl` y el outbox tiene las 3 filas; al arrancar con el embedder listo se ingieren en orden

**Tests:** en `crates/clawg-core/tests/shutdown.rs`, `shutdown_ingests_every_queued_event_when_the_embedder_is_ready` (sin cambios de sentido), `shutdown_leaves_saves_held_for_the_embedder_in_the_outbox`, `shutdown_leaves_what_the_drain_deadline_left_unstarted_in_the_outbox`, `an_old_spill_is_imported_masked_and_removed` y `pending_rows_are_ingested_first_on_startup` (reemplazan a los tests del volcado).

## R9. `purge` saca el texto del outbox

`clawg purge <ID>` MUST borrar, en la transacción que borra la fila, las filas del outbox cuyo `content` contiene el texto purgado, antes del `VACUUM` y el checkpoint. Los snapshots con una fila así MUST nombrarse como los que conservan la memoria. Un snapshot sin la tabla MUST NOT nombrarse por eso.

**Test:** `purge_removes_the_text_from_the_outbox` (`secrets.rs`).

## R10. La base de la versión anterior

Una base en `V13` MUST abrir con `V14` con cada fila idéntica y el outbox vacío. El checksum de `V14` MUST estar en `PINNED`.

**Test:** `a_database_from_the_previous_version_opens_with_an_empty_outbox` (`db.rs`); `published_migration_checksums_are_pinned` (`migrations.rs`).
