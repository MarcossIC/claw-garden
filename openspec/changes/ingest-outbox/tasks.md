# ingest-outbox — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila.

## 1. La tabla (R10, D1)

- [x] Test `a_database_from_the_previous_version_opens_with_an_empty_outbox` (`db.rs`): siembra hasta `V13` y falla porque no existe `ingest_outbox`.
- [x] `crates/clawg-core/migrations/V14__ingest_outbox.sql`, su fila en `PINNED` y el toque de `migrations.rs`.
- Commit: `feat(core): add a table for saves accepted but not yet stored`

## 2. Operaciones y decisiones del outbox (R3, R5, D2, D5)

- [x] Tests puros `claim_step`, `after_failure` y `resume_positions`; tests de store para aceptar, tomar, fallar, olvidar, terminar con vínculo, pendientes en orden y resumen (`outbox.rs`).
- [x] `crates/clawg-core/src/memory/outbox.rs`; el cuerpo de `link_pieces` sobre una transacción abierta (`pieces.rs`); las notas de progreso en el insert y el refuerzo (`store_insert.rs`).
- Commit: `feat(core): record, claim and settle accepted saves in the outbox`

## 3. Aceptar (R1, D3)

- [x] Tests `accept_writes_the_row_before_it_queues_the_event` y `a_refused_event_leaves_no_row` (`runtime.rs`).
- [x] `IngestRequest::outbox`, `MemoryRuntime::accept` con el enmascarado y el borrado de la fila rechazada.
- Commit: `feat(core): accept a save into the outbox before it is queued`

## 4. El worker salda cada fila (R2–R5, D2, D5)

- [x] Tests `each_piece_is_noted_with_the_row_it_wrote`, `a_replayed_save_resumes_after_its_last_noted_piece`, `progress_that_does_not_fit_the_split_is_dropped`, `a_row_delivered_twice_is_ingested_once`, `a_row_that_stopped_the_process_three_times_fails_and_the_next_one_runs` y `a_failure_after_the_answer_stays_in_the_outbox_with_its_cause` (`worker.rs`).
- [x] Tomar, saltar, anotar, terminar y fallar en `memory_worker` y `process_event`.
- Commit: `feat(core): settle each outbox row with the memories it wrote`

## 5. Arranque y apagado (R8, D8)

- [x] Tests de `crates/clawg-core/tests/shutdown.rs`: los del volcado pasan a ser del outbox, más `a_save_accepted_while_the_model_loads_survives_a_process_that_dies` y `an_old_spill_is_imported_masked_and_removed`.
- [x] Importar el volcado viejo y leer las filas pendientes al arrancar; dejar de volcar en el apagado; `spill.rs` queda como lector.
- Commit: `feat(core): replay the outbox on start instead of spilling on shutdown`

## 6. El daemon (R1, R5, R6, D5, D6)

- [x] Tests en `crates/clawg-cli/tests/api.rs`: `a_save_accepted_while_the_model_loads_is_in_the_outbox_before_the_answer`, `an_unconfirmed_save_is_in_the_outbox`, `a_save_into_a_full_queue_leaves_no_outbox_row`, `a_confirmed_save_leaves_no_outbox_row`, `a_failure_the_caller_was_told_leaves_no_outbox_row`, `a_save_answers_with_an_operation_id` y `an_operation_id_sent_twice_keeps_one_row`; el puro `operation_ids_are_checked` en `api.rs`.
- [x] `save_memory` pasa por `accept`; `operation_id` en `SaveRequest` y `SaveBody`; el handler olvida la fila de un fallo contestado.
- Commit: `feat(daemon): accept saves through the outbox with an operation id`

## 7. El agente se entera (R7, D7)

- [x] Tests `health_reports_a_failed_save_with_its_operation_id` (`tests/api.rs`), `a_health_without_ingest_still_parses` (`client.rs`), `format_status_shows_the_ingest_block` (`tools.rs`) y `save_outcome_text_covers_every_daemon_answer` actualizado (`server.rs`).
- [x] `ingest` en `HealthBody` con su tope de 250 ms, `Health::ingest` en el cliente, `operation_id` en `SaveOutcome::Queued` y los textos del `202`.
- Commit: `feat(mcp): report saves that failed after they were accepted`

## 8. `purge` (R9, D9)

- [x] Test `purge_removes_the_text_from_the_outbox` (`secrets.rs`).
- [x] El borrado en la transacción de `purge` y `backups_holding` mirando el outbox.
- Commit: `feat(core): purge a memory's text from the outbox too`

## 9. Docs

- [x] `.claude/CLAUDE.md` (outbox, `operation_id`, `ingest` en `/health`, el volcado que ya no se escribe) y el mapa de `openspec/README.md`.
- Commit: `docs: describe the ingest outbox`

## 10. Verificación

- [x] `cargo test --workspace` una vez sobre el último commit (durante el desarrollo, sólo los tests afectados), clippy con `-A clippy::all -D warnings`, `verify.md` con el conteo contra la base.
- [ ] Pendiente en Windows (máquina de Marcos): la suite, clippy y la latencia del save antes y después.
