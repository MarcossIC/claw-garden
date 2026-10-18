# conflict-candidates — Verificación

## Linux (2026-10-18)

Sobre `96fa419`, en `target`:

- `cargo test --workspace`: la suite pasa. Cada commit del cambio la corrió entera.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: termina con código 0.
- `cargo build -p clawg-cli`, sin la feature `eval`, da el mismo warning que ya traía `fdd71eb` (`ImportanceFactor::spanning`, `crates/clawg-core/src/memory/score.rs:88`). No viene de este cambio.
- El contrato del agente se bendijo una vez, a propósito. El diff se revisó a mano: la apertura suma una oración que pide juzgar las candidatas de `save_memory` (la apertura pasa a medir 1.948 de 2.048 caracteres); la descripción de `resolve_conflict` es nueva; `keep_both` sale del schema; entran `verdict` (enum de cuatro valores, obligatorio) y `reason`.
- `conflict-eval --fake` corre de punta a punta: 110 pares en cinco clases y un flujo con las oraciones con MUST de `openspec/specs`. Con el doble no se empareja nada, como se esperaba: esa corrida prueba el circuito, no los números.

| Requisito | Cómo quedó |
|---|---|
| C1 | `a_close_fact_is_a_pending_candidate_and_both_stay_active`, `a_close_preference_escalates_and_disputes`, `insert_writes_one_conflict_row_per_candidate` y `pending_candidates_alone_leave_the_row_active` |
| C2 | `needs_user_cases`, `escalate_below_pins_the_shipped_value` e `identifiers_are_shared_only_when_both_texts_hold_one` |
| C3 | `a_long_save_pairs_once_with_an_earlier_save` y `a_chunk_still_pairs_with_an_older_row`. R8 sigue fijado por `sibling_chunks_of_one_save_do_not_dispute_each_other` |
| C4 | Los seis tests de `dispute.rs` que nombra la spec, más `the_old_resolve_records_its_verdict`, `open_pairs_count_and_list_the_live_pending_ones` y `every_verdict_parses_back_to_itself`. R1 a R3 de `specs/disputes` siguen con sus tests de siempre |
| C5 | `judge_records_a_verdict`, `judge_refuses_scoped_without_reason`, `resolve_with_keep_both_still_works`, `disputes_lists_pending_pairs_of_the_agent`, `resolve_conflict_reports_the_outcome`, `resolve_conflict_against_an_older_daemon_asks_for_a_restart` y `memory_status_counts_pending_candidates` |
| C6 | `a_confirmed_save_names_its_candidates`, `save_outcome_text_names_related_memories` y `save_body_leaves_related_out_when_empty` |
| C7 | `the_conflict_note_does_not_claim_a_contradiction` y `the_instructions_carry_the_saving_habits` |
| C8 | `a_restored_row_brings_its_escalated_pair_back`, `purging_drops_the_pairs_of_the_row` y `maintenance_drops_pairs_of_missing_rows`. `a_restored_row_settles_its_own_dispute` cambió a propósito: la disputa con la fila restaurada vuelve (R4) |
| C9 | `a_database_with_disputes_opens_with_escalated_pairs` y el checksum de V15 en `published_migration_checksums_are_pinned` |
| C10 | `wilson_interval_cases`, `labelled_pairs_parse_and_cover_every_class`, `a_fake_run_measures_every_class_and_the_flow`, `the_repository_specs_make_a_flow_of_at_least_two_hundred_facts` y `conflict_pairs_reads_the_table` |

### Tests que cambiaron de sentido

- `a_chunk_still_disputes_an_older_row` pasó a `a_chunk_still_pairs_with_an_older_row`. Antes cada pedazo quedaba `disputed` contra la fila vieja; ahora hay un solo par, `pending`, porque los dos son `semantic_fact` sin identificador en común.
- `a_conclusion_does_not_dispute_its_sources` lee el par pendiente en vez de la marca `disputed`.
- `resolve_rows_without_a_dispute_conflict` usa dos filas sin relación: un par ya juzgado sigue enlazado y se puede juzgar de nuevo (C4).

## Pendiente

- **Windows:** la suite y clippy en `target-test`.
- **Modelo real y `docs/eval/`:**
  - `conflict-eval --real --out docs/eval/results/<fecha>-real-conflict-candidates.json`, contra la compuerta fijada en la propuesta: menos del 5 % de saves del flujo con un par escalado, y la clase contradicción con al menos 0,8 de candidatas. Si `by_class.contradiction.escalated` queda bajo, eso es lo esperado para los `semantic_fact` sin identificador; se lee junto con `candidate`, porque esas candidatas llegan al agente en la respuesta del save.
  - `rag-eval --real --diff` contra el ancla: cero casos movidos, porque el ranking no cambia.
  - `rag-eval --real --agent-view`: el bloque de conflictos sólo puede achicarse, así que el hit rate de `recall_context` (39/46) no debe bajar y los tokens medios (1.086,4) no deben subir.
- **Perfil dev con un cliente MCP:** con autorización, una sesión que guarde dos hechos cercanos y lea la línea `close to earlier memories`, juzgue el par y vea el conteo en `memory_status`.

Este entorno no tenía el modelo real, el golden set ni Windows. Por eso esas corridas quedan sin hacer y el cambio no se archiva hasta tenerlas.
