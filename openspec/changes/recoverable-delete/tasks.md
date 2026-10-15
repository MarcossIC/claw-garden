# recoverable-delete — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. La suite queda sin fallas nuevas en cada commit.

## 1. Restaurar en el core (D1, D2, D3, R1, R2, R3, R7)

- [x] Tests en `admin.rs`: `restoring_needs_a_deleted_row_of_the_owner`, `only_memory_rows_are_restored`, `a_live_twin_blocks_the_restore`, `a_row_holding_a_secret_stays_deleted`, `a_failed_embedding_restores_nothing`, `a_restored_row_is_found_again`, `delete_restore_delete_keeps_the_text_index_whole`, `a_stale_vector_does_not_stop_a_restore`, `a_restored_row_settles_its_own_dispute`, `restoring_leaves_the_rest_of_the_row_alone`, `a_restored_piece_is_embedded_behind_its_head_again`, `rows_deleted_by_the_previous_version_can_be_restored`.
- [x] Test en `synthesis.rs`: `a_restored_conclusion_covers_its_sources_again`.
- [x] `RestoreError`, `RestoredRow` y `restore_memory_core`; el reposo de una fila (`settle_row` en `dispute.rs`, sobre `settle_sql`).
- Commit: `feat(core): restore a deleted memory with its vector and text index` (6874a24)

## 2. Listar las borradas en el core (D5)

- [x] Tests en `admin.rs`: `deleted_rows_are_listed_only_when_asked`, `deleted_rows_list_newest_deletion_first`.
- [x] `ListParams::deleted` y los tres `query_*_rows`. El `ListParams` del daemon pasa `deleted: false` hasta el commit siguiente.
- Commit: `feat(core): list deleted memories newest deletion first` (3fa32e3)

## 3. HTTP (D4, R4)

- [x] Tests en `crates/clawg-cli/tests/api.rs`: `restore_brings_a_deleted_memory_back_for_its_owner`, `restore_answers_each_refusal_with_its_status`, `listing_deleted_rows_shows_only_them_newest_deletion_first`; en `api.rs`, `deleted_listing_refuses_a_text_query`.
- [x] Ruta `POST /v1/memories/{kind}/{id}/restore`, `deleted` en `ListQuery`; `DaemonClient::restore` y `ListArgs::deleted`.
- Commit: `feat(daemon): restore a deleted memory and list deleted ones` (0490867)

## 4. MCP (R5)

- [x] Tests: `restore_memory_undoes_an_own_delete`, `an_agent_cannot_restore_another_agents_memory` en `server.rs`; `format_restore_says_when_the_row_is_still_superseded` en `tools.rs`; los conteos de herramientas en `server.rs` y `handshake.rs`; la respuesta y la descripción de `delete_memory`.
- [x] Herramienta `restore_memory`; `delete_memory` la nombra.
- Commit: `feat(mcp): let the agent undo its own delete` (63fcd66)

## 5. CLI (D6, R6)

- [x] Tests: `undelete_takes_an_id_and_a_profile` en `cli.rs`; `undelete_restores_through_the_daemon` y `report_names_the_row_and_a_replacement` en `undelete.rs`; `list_deleted_shows_when_each_row_was_deleted` y `list_args_ask_for_deleted_rows_only_with_the_flag` en `memories.rs`.
- [x] `clawg undelete <ID>` (módulo `undelete.rs`) y `clawg memories list --deleted`.
- Commit: `feat(cli): undelete a memory and list deleted ones` (241f9ed)

## 6. Docs

- [x] `.claude/CLAUDE.md` (comando, ruta y herramienta) y `README.md`.
- Commit: `docs: describe restoring a deleted memory` (0248f25)

## 7. El vector viejo es de otro modelo

- [x] `a_stale_vector_does_not_stop_a_restore` deja en la fila un vector distinto del que da el embedder, como lo dejaría un modelo anterior, y prueba que el restore lo reemplaza.
- Commit: `test(core): restore over a vector an earlier model wrote`

## Verify

- [x] `cargo test --workspace` en Linux en cada commit, contra la base: sin fallas nuevas ([`verify.md`](verify.md#linux)).
- [ ] `cargo test --workspace --target-dir <otro-dir>` en Windows (máquina de Marcos).
- [ ] `rag-eval --real --diff` contra el baseline: se espera diff vacío (máquina de Marcos).
- [ ] Prueba manual con el perfil dev: borrar y restaurar desde Claude Code, listar con `clawg memories list --deleted`, restaurar con `clawg undelete`, intentar restaurar una de otro agente.
