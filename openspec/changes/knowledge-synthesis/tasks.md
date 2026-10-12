# knowledge-synthesis — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. El suite queda sin fallas nuevas en cada commit.

## 1. Limpieza (D7, R1)

- [x] R1 ya está cubierta por tests existentes de búsqueda, workspace, listado y re-embed; se corren antes y después del borrado. El de re-embed pasa a fijar el texto literal.
- [x] Borrar `extractor`, `miner`, `resolver`, `episodic_writer`, `providers`, `services`, los lectores sin llamador de `admin.rs` y los tipos de `models` que sólo usaban ellos; mudar `skill_embedding_text` a `maintenance.rs`; dejar los escritores de trayectorias y skills sólo para tests.
- Commit: `refactor(core): remove the unused LLM extraction pipeline` (675f574)

## 2. Migración V10 (D1, D4, R9)

- [x] Fila de V10 en `published_migration_checksums_are_pinned` (falla), y test de que una base en V9 abre con todas sus filas intactas.
- [x] `V10__synthesis.sql` con `memory_derivation` y `synthesis_review`; se toca `migrations.rs`.
- Commit: `feat(core): add tables for memory provenance and synthesis reviews` (8214a7d)

## 3. Procedencia en la ingesta (R2, R4, R9)

- [x] Tests `a_conclusion_is_linked_to_its_sources`, `every_chunk_of_a_conclusion_is_linked`, `a_reinforced_conclusion_gains_the_new_sources`, `a_row_is_never_its_own_source`, `a_conclusion_does_not_dispute_its_sources` en `worker.rs`; `a_spill_without_sources_still_parses` en `spill.rs`.
- [x] `derived_from` en `MemoryEvent` y `NewMemoryRecord`; enlaces en las transacciones de insert y refuerzo; las fuentes se suman a las exclusiones del detector.
- Commit: `feat(core): record which memories a saved memory was derived from` (b1e41d6)

## 4. La procedencia se ve (R5)

- [x] Test `the_listing_names_the_sources_of_a_conclusion` en `admin.rs`.
- [x] `MemoryAdminRow.derived_from` en el listado y en `GET`.
- Commit: `feat(core): show the sources of a derived memory` (71fa3a6)

## 5. Síntesis en el core (R6, R7, R8)

- [x] Tests `synthesis_lists_what_no_conclusion_covers`, `deleting_a_conclusion_makes_its_sources_pending_again`, `synthesis_stays_in_its_project`, `synthesis_records_the_review_and_nothing_else`, `unreviewed_counts_what_arrived_after_the_last_review`, `existing_ids_leave_out_unknown_and_deleted_rows`, `synthesis_stays_with_its_agent`, `existing_ids_leave_out_another_agents_rows` en `synthesis.rs`.
- [x] `memory/synthesis.rs`: `synthesis_core`, `unreviewed_count`, `existing_memory_ids`.
- Commit: `feat(core): list the memories no conclusion covers yet` (757f169)

## 6. HTTP (R3, R7, R8)

- [x] Tests puros de `distinct_sources` y `missing_sources`; tests HTTP `a_save_from_unknown_sources_is_refused`, `synthesis_over_http`, `context_reports_unreviewed_memories`, `an_agent_neither_reviews_nor_cites_another_agents_memories` y `a_misspelt_synthesis_field_is_refused`.
- [x] `derived_from` en `POST /v1/memories` con validación; `POST /v1/synthesis`; `unreviewed` en `POST /v1/context`; `DaemonClient` al día.
- Commit: `feat(daemon): accept memory sources and serve synthesis` (81bec0c)

## 7. MCP (R5, R7, R8)

- [x] Tests `rows_show_where_a_conclusion_came_from`, `the_synthesis_text_says_what_to_save`, `the_nudge_appears_from_the_threshold` en `tools.rs`; conteo de herramientas e instrucciones en `server.rs` y `handshake.rs`.
- [x] `derived_from` en `save_memory`; herramienta `synthesize` con el agente que llama; recordatorio en `recall_context`; instrucciones del servidor.
- Commit: `feat(mcp): let the agent draw conclusions from its memories` (631fccd)

## Después del merge de main

- [x] Test `purging_drops_the_links_to_and_from_the_row` en `secrets.rs`; `purge` borra los enlaces de la fila purgada (R6).
- Commit: `fix(core): drop a purged memory's provenance links` (47d2ce7)

## Verify

- [x] `cargo test --workspace` en Linux, contra la base: sin fallas nuevas, con el conteo justificado ([`verify.md`](verify.md#linux)).
- [ ] `cargo test --workspace --target-dir <otro-dir>` en Windows (máquina de Marcos).
- [ ] `rag-eval --real --diff` y `--agent-view --diff` contra el baseline (máquina de Marcos; el golden set no está en el repo).
- [ ] Prueba manual con el perfil dev en Claude Code: sintetizar, guardar una conclusión con `derived_from`, ver el recordatorio apagarse (máquina de Marcos).

## Archive

Cuando los pendientes de Verify estén cerrados:

- [ ] Crear `specs/synthesis/spec.md` con R2–R9 y sumar R1 a `specs/retrieval/spec.md`.
- [ ] Mover la carpeta a `changes/archive/AAAA-MM-DD-knowledge-synthesis/` y poner `archive: done` en `state.yaml`.
- [ ] Actualizar la fila 11 de `openspec/README.md` con el enlace archivado, y sacar del backlog la línea de `#[allow(dead_code)]` sólo si project-indexing ya la resolvió (este cambio no la toca).
