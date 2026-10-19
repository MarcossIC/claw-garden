# outbox-lifecycle — Especificación

## Definiciones

- **Save con fila:** un evento que llegó al worker con su fila de `ingest_outbox` (`IngestRequest::outbox`). En producción, todos.
- **Fallo pasajero:** `IngestFailure::EmbedderUnavailable` (el modelo no está, o su recarga no pudo hacerse).
- **Toma:** `outbox_claim` de una fila, que gasta un intento.
- **Pedazos guardados de un save:** las posiciones anotadas en su progreso, insertadas o reforzadas.

## R1. Un fallo pasajero se reintenta en la misma ejecución

Tras un fallo pasajero de un save con fila, el worker MUST anotar la causa y devolver el intento (`outbox_fail` con `transient`), MUST contestar al que espera, si lo hay, y MUST tomar la misma fila otra vez tras `retry_delay(n)`: 1 s la primera vez, el doble cada vez siguiente, a lo sumo 60 s (pura). Mientras espera MUST NOT tomar otro evento: el orden de la cola se conserva, como en la espera del modelo. Si en la espera empieza un drenaje, el worker MUST dejar el save en su fila, sin gastar un intento, y terminar como con un evento sin empezar. Un evento sin fila MUST seguir informándose como hoy, sin reintento.

El handler que recibe un fallo pasajero MUST conservar la fila y responder `202` con `reason: "retrying"`, el `operation_id` y la causa en `detail`.

#### Escenario: el modelo vuelve
- GIVEN un save con fila y un embedder que responde `NotReady` una vez y después funciona
- WHEN el worker lo procesa
- THEN el save queda guardado y su fila borrada sin reiniciar el worker

#### Escenario: el orden se conserva
- GIVEN dos saves con la misma `topic_key`, el primero con un fallo pasajero
- WHEN el worker termina los dos
- THEN la versión viva de la clave es la del segundo

#### Escenario: un drenaje durante la espera
- GIVEN un save que espera su reintento
- WHEN empieza un drenaje
- THEN el worker termina y la fila queda `pending` con los intentos que tenía antes de tomarla

#### Escenario: el que espera
- GIVEN un daemon cuyo modelo no puede volver a cargar
- WHEN se guarda un texto
- THEN la respuesta es `202` con `reason: "retrying"` y el outbox tiene la fila

**Tests:** `retry_delay_doubles_up_to_a_minute`, `a_transient_failure_is_retried_in_the_same_run`, `a_save_behind_a_retried_one_waits_for_it`, `a_drain_during_a_retry_leaves_the_row_pending` (`worker.rs`); `a_save_the_model_cannot_take_answers_202_retrying_and_keeps_its_row` (`tests/api.rs`).

## R2. Un save que guardó pedazos no se olvida

Con el que guardó esperando y un fallo no pasajero, el handler MUST borrar la fila sólo si el reporte no trae pedazos guardados, y responder `500 ingest_failed` como hoy. Si trae alguno, MUST conservar la fila y responder `202` con `status: "queued"`, `reason: "partially_saved"`, `memories` con los pedazos guardados en orden, la causa en `detail` y el `operation_id`. La fila conserva su progreso, así que el próximo arranque termina el save sin duplicar ni reforzar lo guardado.

`save_memory` MUST decir que el save quedó a medias, nombrar los ids guardados, la causa y el `operation_id`, y que el resto se reintenta al próximo arranque del daemon. MUST ser un resultado de éxito: el save está aceptado.

#### Escenario: insert parcial
- GIVEN un save de 2 pedazos o más donde el insert del segundo falla
- WHEN se guarda con el embedder listo
- THEN la respuesta es `202 partially_saved` con el id del primero, y el outbox tiene la fila con ese pedazo en su progreso

#### Escenario: nada guardado
- GIVEN un embedder cuyo batch falla
- WHEN se guarda un texto
- THEN la respuesta es `500 ingest_failed` y el outbox está vacío

**Tests:** `a_partial_save_answers_202_and_keeps_its_row` y `a_failure_the_caller_was_told_leaves_no_outbox_row` (`tests/api.rs`); `save_outcome_text_covers_every_daemon_answer` (`server.rs`); el replay que termina el save es `a_replayed_save_resumes_after_its_last_noted_piece` (`worker.rs`).

## R3. El apagado pausa un save entre pedazos

Antes de insertar cada pedazo de un save con fila, el worker MUST mirar el plazo de drenaje. Vencido, MUST detenerse con el progreso ya anotado, MUST devolver el intento sin cambiar `last_error`, MUST NOT contestar al que espera (que ve `202 unconfirmed`) y MUST terminar como con un evento sin empezar, dejando los demás en sus filas.

#### Escenario: un save largo al vencer el plazo
- GIVEN un save de 3 pedazos o más con los dos primeros guardados por una toma anterior, y un drenaje que vence mientras se embebe el resto
- WHEN el worker llega al tercer pedazo
- THEN no lo inserta, la fila conserva su progreso y los intentos de antes de la toma
- AND otra ejecución termina el save con los mismos ids y el vínculo completo

#### Escenario: tres pausas
- GIVEN una fila tomada y pausada tres veces
- THEN la cuarta toma la procesa

**Tests:** `a_drain_pauses_a_long_save_between_pieces` (`worker.rs`); `a_pause_hands_back_its_attempt` (`outbox.rs`).

## R4. Una sola regla de `failed`

Una fila MUST pasar a `failed` sólo cuando gastó `MAX_ATTEMPTS` tomas que terminaron en un fallo no pasajero o en la muerte del proceso (`claim_step`, `after_failure`), o cuando su evento no parsea. Un fallo pasajero y una pausa MUST devolver su intento. `.claude/CLAUDE.md` y `specs/storage` MUST decir esta regla.

**Tests:** `claim_step_processes_until_the_attempts_run_out`, `after_failure_fails_a_row_only_at_the_cap_and_hands_back_a_transient_attempt` y `a_pause_hands_back_its_attempt` (`outbox.rs`).

## R5. Reintentar o descartar una fila `failed`

`POST /v1/outbox/retry` y `POST /v1/outbox/discard` (`{"operation_id", "agent_id"?}`, detrás del token) MUST actuar sobre la fila `failed` de ese `operation_id`; sin `agent_id`, sobre la única que lo tenga. MUST responder `404 not_found` sin fila, `409 not_failed` con una fila `pending` (el worker puede estar en ella) y `409 ambiguous` cuando varios agentes tienen ese `operation_id`, nombrándolos en `detail`.

- **Reintentar** MUST escribir otra vez la fila, en una transacción, con otro `id`, `pending`, `attempts = 0`, sin `last_error` y con su progreso, y encolarla. Si la cola no la admite, la fila queda `pending` para el próximo arranque y la respuesta lo dice (`queued: false`).
- **Descartar** MUST borrar suavemente los pedazos que ese save insertó y siguen vivos, con el borrado de siempre (así `clawg undelete` los trae), y después la fila. La respuesta nombra los ids borrados. Un `operation_id` descartado vuelve a estar libre.

`clawg outbox retry|discard <OPERATION_ID> [--agent NAME] [--profile P]` MUST hacerlo por el daemon en marcha y MUST NOT arrancar uno.

#### Escenario: reintentar
- GIVEN una fila `failed` con `operation_id` `op-1` y un embedder que funciona
- WHEN se reintenta `op-1`
- THEN la memoria queda guardada y el outbox vacío

#### Escenario: descartar
- GIVEN una fila `failed` cuyo save insertó un pedazo
- WHEN se descarta
- THEN el pedazo queda borrado, la respuesta lo nombra, el outbox está vacío y un save con `op-1` vuelve a guardarse

**Tests:** `retrying_a_failed_row_writes_it_again_under_a_new_id`, `only_a_failed_row_is_retried_or_discarded` y `an_operation_id_two_agents_hold_is_ambiguous` (`outbox.rs`); `a_failed_save_is_retried_by_its_operation_id` y `discarding_a_failed_save_deletes_the_pieces_it_inserted` (`tests/api.rs`); `outbox_takes_an_action_an_operation_id_and_an_agent` (`cli.rs`); `outbox_reports_say_what_was_done` (`outbox.rs` de `clawg-cli`).

## R6. Lo que ven el agente y el usuario

Cada fila de `ingest.problems` en `/health` MUST llevar `pieces`, los ids que ese save insertó (omitido si no hay). `memory_status` MUST agregar, con `ingest.failed > 0`, una línea que diga cuántos saves fallaron y que el usuario los reintenta o descarta con `clawg outbox retry|discard <operation_id>`.

**Tests:** `the_summary_counts_rows_and_lists_the_latest_causes` (`outbox.rs`); `format_status_shows_the_ingest_block` (`tools.rs`).
