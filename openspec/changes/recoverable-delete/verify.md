# recoverable-delete — Verificación

**Estado:** verificado en Linux, commit por commit. Falta lo que corre en la máquina de Marcos: la suite en Windows, `rag-eval --real --diff` y la prueba con el perfil dev ([pendiente](#pendiente-máquina-de-marcos)).

## Tests

### Linux

`cargo test --workspace --no-fail-fast` en Linux, en cada commit de la rama, en un worktree aparte.

| Commit | Pasan | Fallan |
|---|---|---|
| Base (`main`, 9aa1012) | 1.848 | 10 |
| `docs(openspec): propose restoring a deleted memory` (b0755fe) | 1.848 | 10 |
| `feat(core): restore a deleted memory with its vector and text index` (6874a24) | 1.861 | 10 |
| `feat(core): list deleted memories newest deletion first` (3fa32e3) | 1.863 | 10 |
| `feat(daemon): restore a deleted memory and list deleted ones` (0490867) | 1.867 | 10 |
| `feat(mcp): let the agent undo its own delete` (63fcd66) | 1.871 | 10 |
| `feat(cli): undelete a memory and list deleted ones` (241f9ed) | 1.876 | 10 |
| `docs: describe restoring a deleted memory` (0248f25) | 1.876 | 10 |
| `test(core): restore over a vector an earlier model wrote` (bdaccf9) | 1.876 | 10 |
| `fix(mcp): name a daemon without the restore route as stale` | 1.877 | 10 |
| `fix(cli): offer undelete only for deleted memory rows` | 1.878 | 10 |

Los commits de docs y el de `Cargo.lock` (que sólo pasa las versiones del workspace a 0.3.0, como ya dice `Cargo.toml` en `main`) no tocan código.

Todos los commits compilan. Las 10 fallas son las mismas en todos, y en `main`, y vienen del entorno:

- 7 de `doctor::tests`: modelan un PATH de Windows con `clawg.exe`.
- 3 de `diff::tests` en `rag-eval`: leen `docs/eval/results/`, que está en `.gitignore`.

Una vez, en una corrida de `cargo test -p clawg-core`, falló `search::tests::disputed_status_does_not_change_score` y pasó en las cinco siguientes. Calcula la recencia con `strftime('now')` en dos inserts separados, así que depende del reloj; no es de este cambio.

**Conteo:** 30 tests nuevos, ninguno borrado. En `admin.rs`, 12 del restore y 2 del listado; 1 en `synthesis.rs`; 3 HTTP en `tests/api.rs` y 1 en `daemon/api.rs`; 4 del puente en `server.rs` y 1 en `tools.rs`; 1 en `cli.rs`, 3 en `memories.rs` y 2 en `undelete.rs`. Cambiaron los conteos de herramientas (de 11 a 12) en `server.rs` y `handshake.rs`, y los literales de `ListOpts` en `cli.rs` y `memories.rs`.

Los warnings de compilación no cambian.

### Qué cubre cada requisito

| Requisito | Tests |
|---|---|
| R1 qué se restaura | `restoring_needs_a_deleted_row_of_the_owner`, `only_memory_rows_are_restored`, `a_live_twin_blocks_the_restore`, `a_row_holding_a_secret_stays_deleted`, `a_failed_embedding_restores_nothing` |
| R2 qué escribe | `a_restored_row_is_found_again`, `delete_restore_delete_keeps_the_text_index_whole`, `a_stale_vector_does_not_stop_a_restore`, `a_restored_row_settles_its_own_dispute`, `restoring_leaves_the_rest_of_the_row_alone` |
| R3 pedazos y conclusiones | `a_restored_piece_is_embedded_behind_its_head_again`, `a_stale_vector_does_not_stop_a_restore`, `a_restored_conclusion_covers_its_sources_again` |
| R4 HTTP | `restore_brings_a_deleted_memory_back_for_its_owner`, `restore_answers_each_refusal_with_its_status`, `listing_deleted_rows_shows_only_them_newest_deletion_first`, `deleted_listing_refuses_a_text_query`; en el core, `deleted_rows_are_listed_only_when_asked` y `deleted_rows_list_newest_deletion_first` |
| R5 MCP | `restore_memory_undoes_an_own_delete`, `an_agent_cannot_restore_another_agents_memory`, `delete_memory_names_the_way_back`, `format_restore_says_when_the_row_is_still_superseded`, `a_daemon_without_the_restore_route_is_named_as_such`, los conteos de herramientas |
| R6 CLI | `undelete_takes_an_id_and_a_profile`, `undelete_restores_through_the_daemon`, `report_names_the_row_and_a_replacement`, `list_deleted_shows_when_each_row_was_deleted`, `list_args_ask_for_deleted_rows_only_with_the_flag`, `deleted_episodes_alone_are_not_offered_an_undelete` |
| R7 datos anteriores | `rows_deleted_by_the_previous_version_can_be_restored` |

### Cómo se comprobó el índice léxico

`text_index_is_whole` (test de `admin.rs`) compara, término por término y con multiplicidad, las entradas de `memory_fts` con un índice FTS5 armado desde cero con las filas vivas, y corre el `integrity-check` de FTS5. El `integrity-check` contra la tabla de contenido no sirve acá: esa tabla guarda las borradas, que el índice no tiene. Se probó que el chequeo detecta el error: quitando la reinserción del restore fallan `a_restored_row_is_found_again`, `delete_restore_delete_keeps_the_text_index_whole` y `rows_deleted_by_the_previous_version_can_be_restored`. Una reinserción doble no se detecta por lectura (FTS5 se queda con la entrada más nueva de un rowid); la evita la estructura del restore, que la hace una sola vez, después del `UPDATE` que pasó la fila de borrada a viva dentro de la transacción.

## rag-eval

No corrido en Linux: necesita el modelo real. El cambio no toca la búsqueda, el puntaje ni el prompt, y el golden set no borra ni restaura filas, así que se espera diff vacío.

## Pendiente (máquina de Marcos)

1. **Suite en Windows:** `cargo test --workspace --no-fail-fast --target-dir target-test` en la punta de la rama. Se esperan 1.864 + 30 = **1.894 pasan, 0 fallan**.
2. **rag-eval:** `rag-eval --real --diff` contra `main`, en el ancla y en `--agent-view`. Se espera diff vacío.
3. **Perfil dev con Claude Code.** Abrir el perfil con este binario no migra nada (no hay migración).
   - Guardar una memoria, borrarla con `delete_memory` y pedir que la deshaga: `restore_memory` la devuelve y `search_memory` la encuentra.
   - `clawg memories list --deleted --profile dev` la lista mientras está borrada; `clawg undelete <id> --profile dev` la devuelve.
   - Con otro agente (`clawg mcp --agent otro`), `restore_memory` sobre ese id falla como si no existiera.
   - Guardar el mismo texto otra vez antes de restaurar: el restore se rechaza y nombra la copia viva.
