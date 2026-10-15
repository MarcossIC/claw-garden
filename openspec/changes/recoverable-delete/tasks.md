# recoverable-delete — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. La suite queda sin fallas nuevas en cada commit.

## 1. Restaurar en el core (D1, D2, D3, R1, R2, R3, R7)

- [ ] Tests en `admin.rs`: `restoring_needs_a_deleted_row_of_the_owner`, `only_memory_rows_are_restored`, `a_live_twin_blocks_the_restore`, `a_row_holding_a_secret_stays_deleted`, `a_failed_embedding_restores_nothing`, `a_restored_row_is_found_again`, `delete_restore_delete_keeps_the_text_index_whole`, `a_stale_vector_does_not_stop_a_restore`, `a_restored_row_settles_its_own_dispute`, `restoring_leaves_the_rest_of_the_row_alone`, `a_restored_piece_is_embedded_behind_its_head_again`, `rows_deleted_by_the_previous_version_can_be_restored`.
- [ ] Test en `synthesis.rs`: `a_restored_conclusion_covers_its_sources_again`.
- [ ] `RestoreError`, `RestoredRow` y `restore_memory_core`; el reposo de una fila (`settle_row` en `dispute.rs`, sobre `settle_sql`).
- Commit: `feat(core): restore a deleted memory with its vector and text index`

## 2. Listar las borradas en el core (D5)

- [ ] Tests en `admin.rs`: `deleted_rows_are_listed_only_when_asked`, `deleted_rows_list_newest_deletion_first`.
- [ ] `ListParams::deleted` y los tres `query_*_rows`.
- Commit: `feat(core): list deleted memories newest deletion first`

## 3. HTTP (D4, R4)

- [ ] Tests en `crates/clawg-cli/tests/api.rs`: `restore_brings_a_deleted_memory_back_for_its_owner`, `restore_answers_each_refusal_with_its_status`, `listing_deleted_rows_shows_only_them_newest_deletion_first`; en `api.rs`, `deleted_listing_refuses_a_text_query`.
- [ ] Ruta `POST /v1/memories/{kind}/{id}/restore`, `deleted` en `ListQuery`; `DaemonClient::restore` y `ListArgs::deleted`.
- Commit: `feat(daemon): restore a deleted memory and list deleted ones`

## 4. MCP (R5)

- [ ] Tests: `restore_memory_undoes_an_own_delete`, `an_agent_cannot_restore_another_agents_memory` en `server.rs`; `format_restore_says_when_the_row_is_still_superseded` en `tools.rs`; los conteos de herramientas en `server.rs` y `handshake.rs`; la respuesta y la descripción de `delete_memory`.
- [ ] Herramienta `restore_memory`; `delete_memory` la nombra.
- Commit: `feat(mcp): let the agent undo its own delete`

## 5. CLI (D6, R6)

- [ ] Tests: `undelete_takes_an_id_and_a_profile` en `cli.rs`; `undelete_restores_through_the_daemon`, `list_deleted_shows_when_each_row_was_deleted`.
- [ ] `clawg undelete <ID>` y `clawg memories list --deleted`.
- Commit: `feat(cli): undelete a memory and list deleted ones`

## 6. Docs

- [ ] `.claude/CLAUDE.md` (comando, ruta y herramienta) y `README.md`.
- Commit: `docs: describe restoring a deleted memory`

## Verify

- [ ] `cargo test --workspace` en Linux en cada commit, contra la base: sin fallas nuevas.
- [ ] `cargo test --workspace --target-dir <otro-dir>` en Windows (máquina de Marcos).
- [ ] `rag-eval --real --diff` contra el baseline: se espera diff vacío (máquina de Marcos).
- [ ] Prueba manual con el perfil dev: borrar y restaurar desde Claude Code, listar con `clawg memories list --deleted`, restaurar con `clawg undelete`, intentar restaurar una de otro agente.
