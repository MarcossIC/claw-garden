# recoverable-delete — Verificación

**Estado:** verificado en Linux commit por commit, en Windows, con `rag-eval --real --diff` y con el puente MCP real sobre el perfil dev. Antes de archivar, un nuevo análisis contra conflict-candidates (24) y topic-revisions (27), que entraron después, encontró un defecto: no se podía restaurar una pieza de un save largo con `topic_key`. Se corrigió ([lo que encontró la verificación](#lo-que-encontró-la-verificación)). Las otras interacciones son decisiones nuevas y pasan a [revision-aware-delete](../../revision-aware-delete/exploration.md) (49).

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

### Windows

- **Con el cambio recién mergeado:** la verificación de long-save-ranking corrió la suite en Windows sobre `04ab3cd`, que trae este cambio: 1.900 pasan y 0 fallan, la base de Windows (1.864) más los 6 tests de aquel cambio y los 30 de éste ([verify de long-save-ranking](../2026-10-15-long-save-ranking/verify.md#windows)). Los 10 que fallan en Linux por el entorno pasan en Windows.
- **Al archivar, con el arreglo:** `cargo test --workspace --no-fail-fast --target-dir target-test` sobre `develop` (4a213aa) más el arreglo, después de `cargo clean --workspace --target-dir target-test`: 2.442 pasan, 0 fallan, 1 ignorado. `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings` sale con 0.

### Qué cubre cada requisito

| Requisito | Tests |
|---|---|
| R1 qué se restaura | `restoring_needs_a_deleted_row_of_the_owner`, `only_memory_rows_are_restored`, `a_live_twin_blocks_the_restore`, `a_row_holding_a_secret_stays_deleted`, `a_failed_embedding_restores_nothing`; la clave de tema (de 27 y del arreglo): `a_row_whose_key_is_live_again_is_not_restored`, `a_live_row_of_another_save_blocks_the_restore_of_a_piece` |
| R2 qué escribe | `a_restored_row_is_found_again`, `delete_restore_delete_keeps_the_text_index_whole`, `a_stale_vector_does_not_stop_a_restore`, `a_restored_row_settles_its_own_dispute`, `restoring_leaves_the_rest_of_the_row_alone`; el par escalado (de 24): `a_restored_row_brings_its_escalated_pair_back` |
| R3 pedazos y conclusiones | `a_restored_piece_is_embedded_behind_its_head_again`, `a_stale_vector_does_not_stop_a_restore`, `a_restored_conclusion_covers_its_sources_again`; con clave (del arreglo): `a_piece_of_a_keyed_save_is_restored_beside_its_live_siblings`, `a_whole_keyed_save_is_restored_piece_by_piece` |
| R4 HTTP | `restore_brings_a_deleted_memory_back_for_its_owner`, `restore_answers_each_refusal_with_its_status`, `listing_deleted_rows_shows_only_them_newest_deletion_first`, `deleted_listing_refuses_a_text_query`; en el core, `deleted_rows_are_listed_only_when_asked` y `deleted_rows_list_newest_deletion_first`; la clave (de 27): `restore_refuses_a_row_whose_topic_is_live_again` |
| R5 MCP | `restore_memory_undoes_an_own_delete`, `an_agent_cannot_restore_another_agents_memory`, `delete_memory_names_the_way_back`, `format_restore_says_when_the_row_is_still_superseded`, `a_daemon_without_the_restore_route_is_named_as_such`, los conteos de herramientas |
| R6 CLI | `undelete_takes_an_id_and_a_profile`, `undelete_restores_through_the_daemon`, `report_names_the_row_and_a_replacement`, `list_deleted_shows_when_each_row_was_deleted`, `list_args_ask_for_deleted_rows_only_with_the_flag`, `deleted_episodes_alone_are_not_offered_an_undelete` |
| R7 datos anteriores | `rows_deleted_by_the_previous_version_can_be_restored` |

### Cómo se comprobó el índice léxico

`text_index_is_whole` (test de `admin.rs`) compara, término por término y con multiplicidad, las entradas de `memory_fts` con un índice FTS5 armado desde cero con las filas vivas, y corre el `integrity-check` de FTS5. El `integrity-check` contra la tabla de contenido no sirve acá: esa tabla guarda las borradas, que el índice no tiene. Se probó que el chequeo detecta el error: quitando la reinserción del restore fallan `a_restored_row_is_found_again`, `delete_restore_delete_keeps_the_text_index_whole` y `rows_deleted_by_the_previous_version_can_be_restored`. Una reinserción doble no se detecta por lectura (FTS5 se queda con la entrada más nueva de un rowid); la evita la estructura del restore, que la hace una sola vez, después del `UPDATE` que pasó la fila de borrada a viva dentro de la transacción.

## rag-eval

Corrido en Windows con el modelo real por la verificación de long-save-ranking: `rag-eval --real` en la base (`9aa1012`) y en la punta de esta rama (`1bc70b5`), sobre el golden set local (`docs/eval/golden.json`, fuera de git), da un `--diff` vacío y 0 diferencias caso por caso en agent-view (`found`, `found_refs`, `rank`, `tokens`) ([verify de long-save-ranking](../2026-10-15-long-save-ranking/verify.md#windows)). Es lo esperado: el cambio no toca la búsqueda, el puntaje ni el prompt, y el golden set no borra ni restaura filas. El arreglo de la clave de tema tampoco toca el ranking.

## Perfil dev (2026-10-18)

Con el binario debug `0.22.0` de `target-test` (con el arreglo), el daemon arrancado con `clawg serve --profile dev` y el modelo real cargado (`embedder` `ready`, `embedder_resident` `true`). Las herramientas se llamaron por el puente MCP real, `clawg mcp --profile dev --no-autostart --agent <A>`, por stdio, con dos agentes, `verify-a` y `verify-b`; no por una sesión de Claude Code. Un proceso del puente por llamada: varias llamadas en un mismo proceso no responden necesariamente en el orden en que se mandaron.

| Escenario | Resultado |
|---|---|
| Guardar, borrar y deshacer con `restore_memory` | `restored #75`; antes, `search_memory` daba `no relevant memories for this query`, después la encuentra |
| `clawg memories list --deleted` y `clawg undelete` | La lista la muestra con el tiempo desde el borrado y termina con `Undo a memory row with 'clawg undelete <ID>'`; `clawg undelete 75 --profile dev` da `restored memory #75` y la búsqueda la encuentra |
| Otro agente | `restore_memory` de `verify-b` sobre #75 da `404: no deleted Memory row with id 75`, y la fila sigue borrada |
| Gemela viva | Con #76 borrada y el mismo texto guardado otra vez como #77: `409: memory #76 was saved again as #77, which is live; #76 stays deleted so the text is not stored twice` |
| Save largo con clave (el arreglo) | Un save de 15 piezas bajo `verify/long-restore` (#78 a #92): borrar y restaurar la pieza del medio (#85) da `restored #85` y `read_memory` lo trae entero; borrar las 15 y restaurarlas en otro orden da `restored #N` en cada una, sin 409 |

Al terminar, las filas de la prueba quedaron borradas y el daemon se paró con `clawg stop --profile dev`.

## Lo que encontró la verificación

El orden de ejecución pedía este cambio después de 24 y 27 («restaurar una fila tiene que conocer las candidatas y las revisiones»). Se implementó antes, y 24 y 27 lo adaptaron al entrar: los pares de `memory_conflict` sobreviven al borrado y el restore devuelve el par escalado (`470c230`), y el restore rechaza una fila cuya clave volvió a estar viva (`efb9db2`). Al archivar se revisó de nuevo contra el código.

- **Defecto, corregido: una pieza de un save largo con clave no se restauraba.** `RESTORE_TOPIC_HOLDER` (`crates/clawg-core/src/memory/admin.rs:412-419`) buscaba otra fila viva con la misma clave excluyendo sólo a la fila misma, y `revise_in` le da la clave a cada pieza del save (`crates/clawg-core/src/memory/topic.rs:145-148`). Restaurar una pieza con sus hermanas vivas daba `TopicTaken` nombrando a una hermana, y restaurar pieza por pieza un save con clave entero fallaba desde la segunda. Ahora la consulta deja afuera las filas que comparten `memory_piece.save_id` con la restaurada. Tests: `a_piece_of_a_keyed_save_is_restored_beside_its_live_siblings`, `a_whole_keyed_save_is_restored_piece_by_piece` y `a_live_row_of_another_save_blocks_the_restore_of_a_piece`; los tres fallaban antes del arreglo con `TopicTaken { id: 2, key: "ledger/notes", live: 1 }`.
- **Comentarios corregidos:** el de `restore_memory_core` decía que las disputas liberadas al borrar seguían liberadas, y el de `RestoredRow` mandaba a resolver una disputa que una revisión por clave no tiene.
- **Pasan a revision-aware-delete (49), porque son decisiones nuevas y no defectos de este cambio:**
  - borrar la versión vigente de un tema deja a la anterior `superseded` por una fila borrada, y el tema desaparece de la búsqueda;
  - restaurar una fila cuyo reemplazante está borrado la deja `superseded`, y la respuesta manda a `resolve_conflict`, que no acepta un lado borrado;
  - el restore no compara la fila con lo que se guardó mientras estuvo borrada (ya decidido: se va a redetectar sólo en esa ventana);
  - la respuesta del restore no muestra `topic=K`.

## Diferencias entre la spec del cambio y el código

La spec de este cambio quedó como estaba al implementarlo. [`specs/restore`](../../../specs/restore/spec.md) describe lo actual. Lo que cambió después:

- **R2 («MUST NOT volver a disputar las filas que se liberaron al borrarla»)** ya no vale: conflict-candidates (24) hizo que el restore llame a `settle_challengers`, que vuelve a disputar a las filas cuyo `disputed_against` nombra a la restaurada ([`specs/disputes`](../../../specs/disputes/spec.md), R4).
- **R1, R4 y D3** no tienen el rechazo `TopicTaken` (409) que sumó topic-revisions (27), ni la excepción de las piezas del mismo save que agregó el arreglo.
- **R4 (503 con el embedder no listo):** desde embedder-idle-unload (46), un modelo soltado por inactividad se vuelve a cargar al restaurar. El 503 queda para un modelo que nunca cargó o cuya recarga falla.
- **R5:** la descripción de `restore_memory` nombra la gemela y el secreto, pero no la clave de tema; ese rechazo le llega al agente sólo por el mensaje del daemon.
- **Citas `ruta:línea`** de la spec y del diseño que se corrieron con los cambios posteriores, por ejemplo `ready_embedder` (`api.rs:756` en el diseño, hoy `:1024`), `embedding_text` (`chunker/mod.rs:48`, hoy `:55`) y `sweep_orphaned_vectors` (`maintenance.rs:170`, hoy `:175`). `specs/restore` las cita de nuevo.
- **Sin test**, y descrito en `specs/restore` desde el código: el 409 por secreto por HTTP, el 503 `embedder_failed`, un restore con el modelo suelto por inactividad, `clawg undelete` sin daemon y la ventana entre el cálculo del vector y la transacción.
