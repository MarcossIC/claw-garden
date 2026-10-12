# dispute-resolution — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. El suite queda sin fallas nuevas en cada commit.

## 1. Resolver una disputa (R1, R2, R3)

- [x] Tests puros de la validación del par en `dispute.rs`: misma fila, no enlazadas, reemplazada por una tercera, enlazadas por disputa y por `superseded_by`.
- [x] Tests con base: `the_kept_row_wins_and_the_other_is_superseded`, `the_old_row_can_win`, `a_row_stays_disputed_while_another_dispute_is_open`, `a_superseded_row_leaves_search_but_stays_listed`, `keeping_both_closes_the_dispute_without_superseding`, `resolving_backwards_undoes_a_resolution`, `resolving_twice_is_the_same_as_once`, `a_row_cannot_be_resolved_against_itself`, `a_missing_or_deleted_row_is_not_found`, `rows_without_a_dispute_are_refused`.
- [x] `memory/dispute.rs`: `ResolveError`, la validación pura, la expresión del estado de reposo y `resolve_dispute_core`.
- Commit: `feat(core): resolve a dispute between two memories` (`628a9fc`)

## 2. Borrar libera las disputas (R4)

- [x] Tests `deleting_the_disputed_row_releases_its_challenger` y `deleting_one_of_two_keeps_the_dispute` en `admin.rs`.
- [x] `delete_memory_core` libera, en su transacción, las disputas sobre la fila de `memory` que borra.
- Commit: `fix(core): release the disputes on a deleted memory` (`4f4a7ad`)

## 3. El mantenimiento libera las huérfanas (R5, datos de la versión anterior)

- [x] Tests `maintenance_releases_disputes_left_by_an_earlier_delete` y `maintenance_release_is_idempotent` en `maintenance.rs`; `disputes_written_by_the_previous_version_resolve_and_release` en `dispute.rs`.
- [x] La pasada de mantenimiento lleva a su estado de reposo las `disputed` sin contraparte viva.
- Commit: `fix(core): release orphaned disputes during maintenance` (`88a6e78`)

## 4. Workspace sin reemplazadas (R7)

- [x] Test `a_superseded_row_is_not_in_the_workspace` en `workspace.rs`.
- [x] Las consultas a `memory` del workspace excluyen `superseded`.
- Commit: `fix(core): keep superseded memories out of the workspace` (`d8a4e46`)

## 5. Pedazos hermanos (R8)

- [x] Tests `sibling_chunks_of_one_save_do_not_dispute_each_other` y `a_chunk_still_disputes_an_older_row` en `worker.rs`.
- [x] `detect_contradiction` recibe los ids hermanos, los excluye y pide tantos candidatos extra. Se corrige el comentario de `worker.rs` sobre qué pedazos cargan afirmaciones.
- Commit: `fix(core): stop chunks of one save from disputing each other` (`b1f87fa`)

## 6. `POST /v1/disputes/resolve` (R6)

- [x] Tests HTTP `resolve_supersedes_the_other_row`, `resolve_rejects_the_same_row`, `resolve_unknown_row_is_not_found` y `resolve_rows_without_a_dispute_conflict` en `crates/clawg-cli/tests/api.rs`.
- [x] Ruta, mapeo de `ResolveError` a 400/404/409 y `DaemonClient::resolve`.
- Commit: `feat(daemon): resolve a dispute over HTTP` (`ccb724c`)

## 7. `resolve_conflict` en MCP (R6)

- [x] Tests `resolve_conflict_reports_the_outcome`, el conteo de herramientas en `server.rs` y `the_conflict_note_names_the_tool` en `prompt_builder.rs`.
- [x] Herramienta `resolve_conflict`; el bloque de conflictos y las instrucciones del servidor la nombran.
- Commit: `feat(mcp): let the agent resolve a conflict` (`3cc85ac`)

## Verify

- [x] `cargo test --workspace` en Linux: sin fallas nuevas contra la base (ver `verify.md`). En Windows: pendiente.
- [ ] `rag-eval --real --diff` y `--agent-view --diff` contra el baseline: pendiente, el golden set no está en el repo (ver `verify.md`).
- [x] Hipótesis de los pedazos hermanos con el modelo real: confirmada (ver `verify.md`).
