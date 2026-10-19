# outbox-lifecycle — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. Un save que guardó pedazos no se olvida (R2, D2)

- [ ] Tests primero: `a_partial_save_answers_202_and_keeps_its_row` (`tests/api.rs`), los casos nuevos de `save_outcome_text_covers_every_daemon_answer` (`server.rs`).
- [ ] `save_memory` olvida la fila sólo sin pedazos guardados; `202 partially_saved` con `memories` y `detail`; `SaveOutcome::Queued` con `memories` y `detail`; el texto de `save_memory`.
- Commit: `fix(daemon): keep the outbox row of a save that stored some of its pieces`

## 2. Un fallo pasajero se reintenta en la misma ejecución (R1, D1)

- [ ] Tests primero: `retry_delay_doubles_up_to_a_minute`, `a_transient_failure_is_retried_in_the_same_run`, `a_save_behind_a_retried_one_waits_for_it`, `a_drain_during_a_retry_leaves_the_row_pending` (`worker.rs`); `a_save_the_model_cannot_take_answers_202_retrying_and_keeps_its_row` (`tests/api.rs`).
- [ ] El worker retiene el pedido y lo toma otra vez; `202 retrying` y su texto en `save_memory`.
- Commit: `fix(core): retry a save the embedder could not take in the same run`

## 3. El apagado pausa un save entre pedazos (R3, D3)

- [ ] Tests primero: `a_pause_hands_back_its_attempt` (`outbox.rs`), `a_drain_pauses_a_long_save_between_pieces` (`worker.rs`).
- [ ] `pause_due`, `outbox_pause`, `IngestReport::paused`.
- Commit: `fix(core): pause a long save between pieces when the drain deadline passes`

## 4. Reintentar y descartar una fila `failed` (R5, R6, D4)

- [ ] Tests primero: `retrying_a_failed_row_writes_it_again_under_a_new_id`, `only_a_failed_row_is_retried_or_discarded`, `an_operation_id_two_agents_hold_is_ambiguous`, los `pieces` de `the_summary_counts_rows_and_lists_the_latest_causes` (`outbox.rs`); `a_failed_save_is_retried_by_its_operation_id`, `discarding_a_failed_save_deletes_the_pieces_it_inserted` (`tests/api.rs`); `outbox_takes_an_action_an_operation_id_and_an_agent` (`cli.rs`); `outbox_reports_say_what_was_done`; la línea nueva de `format_status_shows_the_ingest_block` (`tools.rs`).
- [ ] `outbox_find_failed`, `outbox_requeue`, `outbox_discard`; `MemoryRuntime::retry_failed` y `discard_failed`; las dos rutas; `DaemonClient::outbox_retry` y `outbox_discard`; `clawg outbox`; `OutboxProblem::pieces`; la línea de `memory_status`.
- Commit: `feat(cli): retry or discard a failed save with clawg outbox`

## 5. Docs

- [ ] `.claude/CLAUDE.md` (la regla de `failed`, el reintento, la pausa, las respuestas `202`, las rutas y `clawg outbox`), el mapa de `openspec/README.md` y `state.yaml`.
- Commit: `docs: describe how a save that fails part-way is retried, paused or discarded`

## 6. Verificación

- [ ] Suite y clippy en Linux sobre el último commit, y cada commit de código por separado.
- [ ] El CI en verde en Linux y Windows.

## 7. Archive

- [ ] `specs/storage`, `specs/ingest` y `specs/pieces` con R1 a R6; `git mv` a `changes/archive/`; `state.yaml` archivado; fila 53 y orden de ejecución del `README.md` de openspec; subir el minor.
