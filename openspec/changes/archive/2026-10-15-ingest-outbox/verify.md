# ingest-outbox — Verificación

**Estado:** verificado en Linux y Windows, con la latencia del save medida en el perfil dev. La base real aplica `V14` con la primera instalación posterior ([base real](#base-real)).

## Tests

### Linux

`cargo test --workspace --no-fail-fast`, una sola vez sobre el último commit de la rama; durante el desarrollo corrieron sólo los tests afectados.

| | Pasan | Fallan |
|---|---|---|
| Base (`main`, 210c692) | 1.971 | 7 |
| Último commit de la rama | 2.010 | 7 |

Las 7 fallas son las mismas en `main` y vienen del entorno: `doctor::tests` modela un PATH de Windows con `clawg.exe`.

Por binario: `clawg-core` unitarios 1.040 → 1.067, `clawg-cli` unitarios 341 → 345, `clawg-cli/tests/api.rs` 83 → 91; `clawg-core/tests/shutdown.rs` sigue en 7 (los tests del volcado se reemplazaron por los del outbox). El resto no cambia.

`cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` termina con código 0.

Cambiaron a propósito los tests que contaban migraciones (de 13 a 14) y el que fija los checksums.

### Windows

`cargo test --workspace --no-fail-fast --target-dir target-test`, en un worktree limpio de `main`:

| Commit | Pasan | Fallan | Ignorados | Warnings del workspace |
|---|---|---|---|---|
| `f7ce973` (este cambio y redaction-precision ya mergeados) | 2.034 | 0 | 0 | 0 |

2.034 es la base de 1.984 (`8312e42`) más los 39 de este cambio (2.010 − 1.971 en Linux) y los 11 de redaction-precision. Los 7 de `doctor::tests` que fallan en Linux pasan en Windows. Por binario: `clawg-core` unitarios 1.078, `clawg-cli` unitarios 357, `clawg-cli/tests/api.rs` 91 y `clawg-core/tests/shutdown.rs` 7.

`cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings` sobre `f7ce973`: **código 0**, sin ningún warning.

### Qué cubre cada requisito

| Requisito | Tests |
|---|---|
| R1. La fila antes de responder, enmascarada y con FULL | `a_save_accepted_while_the_model_loads_survives_a_process_that_dies`, `a_save_accepted_while_the_model_loads_is_in_the_outbox_before_the_answer`, `an_unconfirmed_save_is_in_the_outbox`, `a_save_into_a_full_queue_leaves_no_outbox_row`, `accept_writes_the_row_before_it_queues_the_event`, `a_refused_event_leaves_no_row`, `durably_writes_under_full_and_puts_the_setting_back`, `recording_a_row_leaves_the_store_on_normal` |
| R2. Se borra al terminar | `a_confirmed_save_leaves_no_outbox_row`, `each_piece_is_noted_with_the_row_it_wrote`, `finishing_links_the_pieces_and_drops_the_row_together` |
| R3. Retomar por pieza | `a_replayed_save_resumes_after_its_last_noted_piece`, `progress_that_does_not_fit_the_split_is_dropped`, `resume_positions_places_each_noted_piece_and_refuses_one_that_does_not_fit`, `a_note_inside_a_transaction_is_the_progress_the_next_claim_reads` |
| R4. Entrega doble | `a_row_delivered_twice_is_ingested_once` |
| R5. Intentos y fallas | `claim_step_processes_until_the_attempts_run_out`, `after_failure_fails_a_row_only_at_the_cap_and_hands_back_a_transient_attempt`, `claiming_spends_an_attempt_and_gives_up_once_none_is_left`, `a_failure_keeps_its_cause_and_a_transient_one_its_attempt`, `a_row_that_stopped_the_process_three_times_fails_and_the_next_one_runs`, `a_failure_after_the_answer_stays_in_the_outbox_with_its_cause`, `a_failure_the_caller_was_told_leaves_no_outbox_row`, `an_event_that_does_not_parse_is_given_up_with_its_cause` |
| R6. `operation_id` | `a_save_answers_with_an_operation_id`, `an_operation_id_sent_twice_keeps_one_row`, `operation_ids_are_checked`, `an_operation_id_already_recorded_is_not_queued_again`, `an_operation_id_is_one_row_per_agent`, `a_confirmed_save_names_its_operation_and_what_was_masked_before_it_was_recorded` |
| R7. `/health` y `memory_status` | `health_reports_a_failed_save_with_its_operation_id`, `a_health_without_ingest_still_parses`, `format_status_shows_the_ingest_block`, `the_summary_counts_rows_and_lists_the_latest_causes`, `save_outcome_text_covers_every_daemon_answer` |
| R8. Arranque y apagado | `shutdown_ingests_every_queued_event_when_the_embedder_is_ready`, `shutdown_leaves_saves_held_for_the_embedder_in_the_outbox`, `shutdown_leaves_what_the_drain_deadline_left_unstarted_in_the_outbox`, `pending_rows_are_ingested_first_on_startup`, `an_old_spill_is_imported_masked_and_removed`, `imported_events_get_rows_of_their_own`, `pending_rows_come_back_oldest_first_and_failed_ones_do_not` |
| R9. `purge` | `purge_removes_the_text_from_the_outbox`, `a_backup_from_before_the_outbox_is_not_named_for_it` |
| R10. La base de la versión anterior | `a_database_from_the_previous_version_opens_with_an_empty_outbox` |

## Contrato del agente

`agent_contract.json` no cambia: el `202` suma `operation_id` en el cuerpo y el texto de `save_memory`, pero no la descripción ni el esquema de ninguna herramienta.

## Commits

Las tareas 3, 4 y 5 quedaron en un solo commit (`feat(core): ingest accepted saves through the outbox and replay it on start`): aceptar sin que el worker salde la fila, o saldarla sin leerla al arrancar, dejaba pasos intermedios con código muerto o con filas que nadie borraba. La durabilidad FULL va en su propio commit, después de la respuesta a la tarjeta.

## rag-eval

No aplica: el cambio no toca ranking ni recuperación.

## Latencia del save

Es el procedimiento de [ingest-durability](../2026-10-12-ingest-durability/verify.md#latencia-del-save-tareas-02-y-102):

- **Entorno:** perfil dev con el modelo real, binario debug.
- **Carga:** saves secuenciales a `POST /v1/memories`, 100 cortos (25 palabras) y 100 largos (700), con los mismos textos en cada corrida. Se descartan 5 de calentamiento.
- **Qué se mide:** la respuesta, desde el `POST` hasta el `201`.
- **Binarios:** el antes es `2b1b08a` (`main` con todo menos este cambio) y el después `f7ce973`. Entre los dos no cambia ninguna dependencia.
- **Estado de partida:** antes de cada corrida se restauró la misma copia del store dev (V12, 55 memorias). Cada binario la migra: el antes a V13, el después a V13 y V14.
- **Control:** las dos últimas corridas suman 50 búsquedas (`POST /v1/search`), que embeben la consulta y no tocan el outbox.

Las corridas, en el orden en que se hicieron (ms):

| Corrida | Binario | Corto p50 | Corto p95 | Corto p99 | Largo p50 | Largo p95 | Búsqueda p50 |
|---|---|---|---|---|---|---|---|
| 1 | antes | 17,6 | 38,1 | 41,4 | 440 | 528 | — |
| 2 | antes | 18,2 | 41,8 | 44,2 | 448 | 535 | — |
| 3 | después | 29,1 | 47,8 | 52,3 | 599 | 707 | — |
| 4 | antes | 20,1 | 43,9 | 48,0 | 525 | 612 | 32,2 |
| 5 | después | 24,2 | 48,3 | 52,3 | 546 | 615 | 35,0 |

Todos los saves respondieron `201`. Al terminar las corridas del después, `/health` mostró `ingest` con 0 pendientes y 0 fallidas.

- **El umbral se cumple.** El p95 corto crece entre 4,4 ms (4 → 5) y 9,7 ms (1 → 3), contra un tope de 20 ms. La mediana pareada por save es +3,8 ms en 4 → 5 (79 de 100 más lentos) y +7,5 ms en 1 → 3. Es el fsync de la fila con `synchronous = FULL`.
- **En el save largo, el efecto no se distingue de la deriva de la máquina.** El mismo binario antes pasó de 440 a 525 ms de p50 entre las corridas 1 y 4. El par 1 → 3 daba +152 ms de mediana pareada, pero el par contiguo 4 → 5 da +12 ms (68 de 100 más lentos) y +3 ms de p95, y el control de búsqueda +0,4 ms (26 de 50 más lentos). Con estas corridas no se ve lo que el outbox suma a un save largo: un `UPDATE` de progreso por pieza, dentro de su transacción y en `NORMAL`.
- **La corrida 2 repite la 1 por error.** El primer después se compiló con el mismo `--target-dir` que el antes. Cargo no mira la ruta de los crates del workspace y decide por mtime, así que dio por frescas las fuentes del otro worktree y dejó el mismo binario (mismo sha256; sólo aplicó V13). Esa corrida se usó como medida del ruido, y el después se compiló aparte. El binario bueno contiene `ingest_outbox` y aplica V14.
- **El build es debug.** En release el embebido y la tokenización son más rápidos; el fsync no.
- **Efecto colateral.** Cada `clawg restore` deja un snapshot `pre-restore` y cada migración uno `pre-migration`, y sólo se guardan los 5 automáticos más nuevos (`AUTO_KEEP`, `crates/clawg-core/src/memory/backup.rs:231`). Por eso las corridas rotaron los cinco automáticos que había en `~/.clawg-dev/backups` (del 12 al 15/10). Los `manual` quedaron, y el store dev terminó como estaba (V12, 55 memorias).

## Base real

Al archivar, el daemon `prod` corre 0.7.0 (`memory_status`: `"version": "0.7.0"`, perfil `prod`, embedder `ready`), una versión anterior al merge de este cambio, así que la base real todavía no tiene `V14`. No se instaló un release ni se tocó `~/.clawg`.

La primera instalación posterior a 0.9.0 escribe el snapshot `pre-migration` y aplica `V13` y `V14`. Ahí hay que comprobar dos cosas:

- que `memory_status` muestre el bloque `ingest` con 0 pendientes;
- que, si quedó un `ingest-pending.jsonl` del daemon anterior, se importe y desaparezca.

## Diferencias entre el spec y el código

Al consolidar en [`specs/ingest`](../../../specs/ingest/spec.md), [`specs/storage`](../../../specs/storage/spec.md), [`specs/secrets`](../../../specs/secrets/spec.md), [`specs/mcp`](../../../specs/mcp/spec.md) y, por las citas que este cambio movió, [`specs/pieces`](../../../specs/pieces/spec.md) y [`specs/disputes`](../../../specs/disputes/spec.md), se revisó contra `main` cada `ruta:línea` y cada test citado. Todos los tests existen, y lo que afirman R1 a R10 coincide con el código. Las diferencias:

1. **Al backlog.** Una fila `failed` no tiene salida. Lo único que borra filas del outbox es el handler (un save rechazado o contestado con su fallo, `api.rs:846`), el cierre de un save (`outbox_finish`, `outbox.rs:329`) y `purge` por coincidencia de texto (`secrets.rs:111`), y un save que falló no dejó ninguna fila en `memory` que purgar. Así que la fila queda para siempre: `/health` cuenta `failed` sin que nada lo baje, `problems` muestra sólo las 5 últimas, y reenviar el mismo `operation_id` responde `500 ingest_failed` mientras exista. El spec dice que una fila `failed` MUST NOT borrarse sola (R5) y lo cumple, pero no define cómo la saca una persona ni cómo se reintenta.
2. R7 y D7 dicen que, si la lectura del outbox vence, `ingest` es `null`. El código omite la clave (`skip_serializing_if`, `api.rs:446`, y lo mismo en `Health` del cliente, `client.rs:134`), de modo que `memory_status` tampoco la muestra y no se distingue «sin filas» (que sí imprime `pending: 0`) de «no contestó». Los specs consolidados dicen que se omite.
3. D2 describe el progreso como `{position, id, inserted, context}`. `NotedPiece` también guarda `hash` (`outbox.rs:84`), y `resume_positions` descarta el progreso no sólo cuando nombra una posición inexistente, como dice R3, sino también cuando repite una posición o el hash anotado no es el del texto de esa posición (`outbox.rs:96-106`). Los specs consolidados describen el código.
4. R4 pide saltar una fila que ya no existe o que está `failed`; sólo se prueba a nivel del store (`claiming_spends_an_attempt_and_gives_up_once_none_is_left`), no a través del worker. Tampoco tiene test que el que espera una segunda entrega reciba el reporte de la primera (`replayed`, `worker.rs:257`, `:291` y `:330`): `a_row_delivered_twice_is_ingested_once` manda la segunda sin canal de respuesta. El spec no menciona esa regla; el consolidado sí.
5. R5 pide que un fallo transitorio no cuente intento y que una fila con fallo no se reintente en la misma ejecución. Lo primero se prueba en la función pura y en el store (`after_failure_fails_a_row_only_at_the_cap_and_hands_back_a_transient_attempt`, `a_failure_keeps_its_cause_and_a_transient_one_its_attempt`), no a través del worker; lo segundo no tiene test. El escenario «fallo después de un `202`» usa un embedder que devuelve NaN, que el worker informa como `store insert failed: …`, no como un fallo del embedder.
6. R1, escenario «cola llena»: el test que lo cubre, `a_save_into_a_full_queue_leaves_no_outbox_row`, sólo comprueba las 2 filas del outbox, no el `503` del tercero; ese lo comprueba el test anterior, `a_save_into_a_full_queue_is_503_queue_full`.
7. R2, escenario «3 piezas»: `a_confirmed_save_leaves_no_outbox_row` comprueba el outbox vacío, no que `memory_piece` tenga las piezas, y su texto de 600 palabras no fija cuántas son. La transacción única del vínculo y el borrado de la fila la prueba `finishing_links_the_pieces_and_drops_the_row_together`, que la tabla de requisitos de este `verify.md` pone bajo R2 pero `spec.md` no lista.
8. R6: la respuesta `500 ingest_failed` a un `operation_id` con fila `failed` no tiene test; sólo se prueba el `already_queued` de una fila `pending`. El `operation_id` identifica el save y no su texto: un reenvío con otro contenido y el mismo id no guarda el contenido nuevo mientras la fila exista. D6 lo implica; los specs consolidados lo dicen.
9. R8 y D8 dicen que el volcado se borra sólo después del commit. El código tampoco lo borra cuando no pudo apartar sus líneas ilegibles en `ingest-pending.rejected.jsonl` (`spill.rs:76`), así que las líneas válidas, ya en el outbox, se importan otra vez en cada arranque hasta que eso se arregla, y el dedup las absorbe como refuerzos. Es una rama de falla de disco, sin test; D8 sólo menciona el fallo del borrado.
10. R10: `a_database_from_the_previous_version_opens_with_an_empty_outbox` llama a `run_migrations` directo, así que no prueba el snapshot `pre-migration` de la apertura; lo cubre la regla general de [`specs/storage`](../../../specs/storage/spec.md), «Snapshot antes de migrar». D10 (un `restore` reemplaza las filas pendientes por las del snapshot) no tiene test propio: es lo que resulta de reemplazar `memory.db`.
11. Las citas de `design.md` se movieron: `memory_worker` (`worker.rs:198`, hoy `:232`), `EvalIngest` (`eval_api.rs:496`, hoy `:468`), `link_pieces` (`pieces.rs:180`, cuyo cuerpo hoy es `link_in`, `pieces.rs:207`), el enmascarado del worker (`worker.rs:431`, hoy `:591`, dentro de `ingest_pieces`), `ingest_worker_future` (`runtime.rs:363`, hoy `:401`), `shutdown` (`runtime.rs:219`, hoy `:267`), `purge` (`secrets.rs:81`, hoy `:82`) y `backups_holding` (`secrets.rs:134`, hoy `:139`). `process_event` ya no existe fuera de los tests (`worker.rs:537`, `#[cfg(test)]`): el camino de producción es `process_tracked` (`worker.rs:550`). Las citas de `db.rs:197`, `contradiction.rs:137`, `tools.rs:352` y `server.rs:495` siguen bien.
12. Quedan nombres y comentarios de cuando el apagado volcaba a un archivo: `DrainStep::Spill` y la doc de `drain_step` y de `memory_worker` (`worker.rs:190-232`), el test `a_drain_without_an_embedder_spills_at_once` (`worker.rs:2903`), el comentario de `serve.rs:151` y el de `stop.rs:60`. Nada vuelca; sólo el nombre.

Citas viejas de otros specs que dependían de este cambio y se corrigieron al consolidar: `process_event` y los números de línea de `worker.rs` en R2, R3, R5 y R6 de `specs/secrets` (las que listó el `verify.md` de redaction-precision), los de `specs/pieces` (R3 y las Definiciones), `specs/disputes` y las del apartado de backup y restore de `specs/storage`. En `specs/sharing` y `specs/mcp` se corrigieron además `format_sharing` y `memory_status_impl` (`tools.rs:338`, `server.rs:889`), que no vienen de este cambio.
