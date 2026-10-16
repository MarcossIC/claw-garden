# ingest-outbox — Verificación

**Estado:** verificado en Linux. Falta la suite, `clippy` y la latencia del save en Windows ([pendiente](#pendiente-máquina-de-marcos)).

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

## Pendiente (máquina de Marcos)

1. `cargo test --workspace --target-dir <otro-dir>`: se esperan 2.010 + 7 = 2.017 tests y 0 fallas.
2. `cargo clippy --workspace --all-targets --target-dir <otro-dir> -- -A clippy::all -D warnings`: código 0.
3. Latencia de `POST /v1/memories` antes y después (ahora hay un fsync por save aceptado): p95 sobre 200 saves; se acepta hasta +20 ms.
4. Al abrir la base real por primera vez se escribe el snapshot `pre-migration` y se aplica `V14`; si queda un `ingest-pending.jsonl` de la versión anterior, se importa y se borra.
