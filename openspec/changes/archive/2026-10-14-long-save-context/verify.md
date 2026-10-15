# long-save-context — Verificación

**Código:** rama `claude/project-thread-0dd4jr`, un commit por tarea, sobre `main` en `9c3ff19`.
**Modo:** Strict TDD (`openspec/config.yaml`: `strict_tdd: true`). Los tests de cada tarea se escribieron antes que su código y fallaron por no compilar (tipos, funciones o la tabla que todavía no existían) o por la aserción.
**Estado:** R1 a R8 y la parte de R9 que se prueba sin modelo pasan en Linux, commit por commit, y en Windows sobre `main` en `f060593` (1.737 pasan, 0 fallan). U1, U3 y U4 se cumplen. **U2 no se cumple:** la respuesta de un save `long` es alcanzable en 0,333 de los casos (base 0,000; el umbral es 0,75). La prueba con el perfil dev pasa. Detalle en [Windows](#windows).

## Suite

`cargo test --workspace --no-fail-fast` en Linux, cada commit en un worktree limpio:

| Commit | Pasan | Fallan |
|---|---|---|
| `9c3ff19` (base, `main`) | 1674 | 10 |
| `37813a5` docs(openspec): propose linking the pieces of a long save | 1674 | 10 |
| `db0c6a1` feat(eval): measure whether the answer inside a long memory reaches the agent | 1688 | 10 |
| `240dc58` feat(core): embed later pieces of a long save with its opening and link them | 1703 | 10 |
| `3d08a5f` feat(core): show one hit per save and say which piece it is | 1709 | 10 |
| `92aea43` feat(mcp): let the agent read a long save as one text | 1720 | 10 |
| `fab517e` docs: list the read_memory tool | 1720 | 10 |
| merge de `main` en `3c84cc9` (trae el arreglo de context-density en `prompt_builder.rs`) | 1721 | 10 |

- **+46 tests.** El conteo nunca baja: los fixtures que dependían de las ventanas de 480 tokens se ajustaron en el mismo commit que baja el tope a 424.
- **Los 10 que fallan son los mismos 10 de `main`** en todos los commits (comparados por nombre): `doctor::tests::*` (7, sólo pasan en Windows) y `rag-eval` `diff::tests::*` (3, leen `docs/eval/results/`, que `.gitignore` deja afuera).
- **Warnings y clippy:** los warnings del build son los de `main` (`insert_row_with_embedding`, `search_all_memories_with_scoring`). `cargo clippy -p clawg-core -p clawg-cli -p clawg-eval --all-targets` no da avisos nuevos.

## Requisitos

| Req | Test(s) | Resultado |
|---|---|---|
| R1. Un pedazo no cambia | `a_text_that_fits_one_piece_is_split_as_before`, `a_single_piece_save_embeds_its_text_and_leaves_no_link` | ✓ |
| R2. Tope y cabecera | `a_long_text_is_split_below_the_cap_to_leave_room_for_the_head`, `the_head_is_the_first_tokens_of_the_text`, `a_piece_without_context_is_embedded_as_stored`, `a_piece_with_context_is_embedded_after_the_head`, `later_pieces_are_embedded_with_the_head_and_stored_without_it`; el assert de compilación en `chunker/mod.rs` | ✓ |
| R3. Vínculo | `a_long_save_links_its_pieces_in_order`, `an_absorbed_piece_is_linked_without_context`, `a_save_cut_short_leaves_no_link`, `the_same_long_save_again_links_the_rows_it_left`, `link_pieces_numbers_saves_in_order`, `linking_the_same_rows_again_answers_the_save_they_already_form` | ✓ |
| R4. Un hit por save | `only_the_best_piece_of_a_save_survives`, `rows_without_links_keep_their_place`, `freed_slots_go_to_other_memories`, `a_link_read_failure_leaves_hits_uncollapsed` | ✓ |
| R5. `piece=K/N` | `format_hits_names_the_piece_of_a_long_save`, `format_hit_line_names_the_piece_of_a_long_save`, `a_hit_line_with_a_piece_yields_its_id_score_and_content`, `a_context_hit_with_a_piece_is_one_segment` | ✓ |
| R6. El save entero | `join_pieces_drops_the_overlap_once`, `join_pieces_keeps_both_sides_when_nothing_overlaps`, `the_pieces_of_a_split_save_join_back_into_its_text`, `read_save_lists_missing_pieces_and_skips_other_agents`, `get_save_returns_the_whole_text`, `get_save_of_another_agent_is_404`, `read_memory_returns_the_whole_save`, `an_agent_cannot_read_another_agents_save`, `a_daemon_without_the_save_route_is_named_as_such`, `format_save_names_the_piece_and_the_missing_ones`, `save_memory_asks_for_one_fact_per_call`, `search_memory_explains_the_piece_of_a_long_save`, `save_outcome_text_covers_every_daemon_answer`, los conteos de herramientas (diez) en `server.rs` y `handshake.rs` | ✓ |
| R7. Re-embed con contexto | `reembed_rebuilds_a_pieces_context` | ✓ |
| R8. Datos de V10 | `rows_from_v10_survive_the_piece_table`: siembra en V10 con SQL directo, abre en V11 con snapshot `pre-migration`, ningún vínculo, y las filas se buscan, se listan, se leen con `read_save_core` como saves de un pedazo y se re-embeben con su propio texto | ✓ |
| R9. Medición | `there_are_twelve_long_cases_at_any_size`, `a_long_row_names_its_key_only_in_its_first_sentence`, `the_answer_appears_once_after_the_first_third`, `a_long_row_splits_into_at_least_three_pieces`, `adding_long_cases_leaves_the_rest_of_the_set_unchanged`, `a_long_case_writes_its_subset_and_answer`, `an_answer_and_the_long_subset_parse`, `the_answer_counts_only_when_its_text_is_visible`, `a_read_save_makes_a_hidden_answer_reachable`, `a_line_repeating_an_earlier_lines_row_is_counted`, `build_results_splits_active_cases_by_subset_with_their_answers`, `a_case_that_names_its_answer_reports_whether_the_answer_reached_the_agent` | ✓ en tests; U1 a U4 pendientes |

## Cómo lo ve el agente

Un save largo de cuatro pedazos, con el 3 borrado. `search_memory` muestra un solo pedazo:

```
1. #58 [Memory/semantic_fact/t3 piece=2/4 score=0.81] the retry budget was set to …
2. #12 [Memory/semantic_fact/t3 score=0.64] …
```

`read_memory` con `id: 58`:

```
#58 is piece 2 of 4 of one save: #57, #58, (missing), #60
Notes on clawg-ledger: … the retry budget was set to three attempts …
… the fourth and last piece
```

`save_memory` de un texto largo responde `saved as 4 pieces: #57, #58, #59, #60; read_memory with any of them returns the whole text`.

## Decisiones tomadas al implementar

- **Commits.** La tabla y `link_pieces` se juntaron con el worker (sin llamador eran código muerto), y `read_save_core` y `join_pieces` pasaron al commit de la ruta. Quedaron cinco commits de código en vez de seis.
- **`link_pieces` es idempotente.** Guardar otra vez el mismo texto absorbe todos los pedazos; si esas filas, en ese orden, ya forman un save, se devuelve ese `save_id` y no se escribe nada. La spec (R3) y el diseño (D4) lo dicen ahora.
- **`SaveText.piece`.** La respuesta de `/save` dice la posición de la fila pedida. Con un pedazo repetido dentro del mismo save, el id solo no la determina.
- **`join_pieces` por tramos.** Los pedazos a cada lado de uno que falta no se comparan entre sí: cada tramo visible se une por solapamiento y los tramos con un salto de línea.
- **Daemon viejo.** Su 404 no trae el error JSON del daemon; `read_memory` lo reconoce y pide reiniciar el daemon en vez de decir que la fila no existe.
- **R9, relleno.** El relleno de una fila `long` no repite 6 palabras seguidas de ninguna fila de la base. Entre filas `long` lo que se exige es que no compartan ningún pedazo, porque salen de las mismas plantillas.
- **El `kind` en `search_memory`** sale como lo serializa el daemon (`Memory`), no en minúscula como decía el ejemplo de la spec. Los parsers del eval aceptan los dos.

## Windows

Todo sobre `main` en `f060593` (el merge de la rama), Windows 11, binarios debug.

### Suite

`cargo test --workspace --no-fail-fast --target-dir target-test`: **1.737 pasan, 0 fallan.** Es la base de Windows (1.691) más los 46 tests nuevos. Los 10 que fallan en Linux por el entorno pasan acá.

### U1, el ancla

La base es la última corrida de `main`: `2026-10-15-real-context-density-fix-anchor.json` y `-agent-view.json`, de `d175348`. Entre `d175348` y `3c84cc9` (el padre del merge en `main`) sólo cambiaron docs, y `golden.json` sigue en `d2d9d732…ed792a`. La punta corrió `rag-eval --real --tag long-save-context-tip-anchor` y `rag-eval --real --agent-view --tag long-save-context-tip`.

- `rag-eval --diff`: 0 mejores y 0 peores en las cinco métricas de retrieval y de similarity. El ancla queda en R@1 0,4348, R@5 0,7826, R@10 0,8913, MRR 0,5979 y nDCG@10 0,6693.
- Bloque `agent_view`, campo por campo y sin contar latencias: 0 diferencias en los campos que ya existían. Los únicos campos nuevos son los de este change: `by_subset`, `per_case[].subset` y `per_case[].search.repeated_rows`. Se mantienen 41 de 46 encontrados, `search_memory` con media de 851,5 tokens (máx. 1.239) y `recall_context` con 1.405,0 (máx. 1.738).

**✓**

### U2, U3 y U4

El set es `golden-mine synth --base docs/eval/golden.json --seed 7`, generado con el binario de la punta (1.040 filas, 124 casos, 12 `long`). El `golden-mine` de `db0c6a1` genera otras filas `long`, porque el generador cambió después de ese commit: difieren 12 filas y 11 casos, todos `long`, y el resto es idéntico. Las dos corridas usan el set de la punta. `rag-eval --real --agent-view --golden <set>`: la base con el binario compilado en `db0c6a1` (`--tag long-save-context-base-seed7`) y la punta con el de `f060593` (`--tag long-save-context-tip-seed7`).

El `git_sha` del archivo de la base dice `f060593`, porque `rag-eval` lee el `HEAD` del directorio desde donde corre, no el commit del binario. Que el binario es el de `db0c6a1` se ve en dos cosas: escribió sus resultados en el `docs/eval/results` del worktree donde se compiló, y se comporta distinto (3 filas repetidas en `natural`, contra 0 de la punta).

| Subconjunto | Métrica | Base | Punta | Δ |
|---|---|---|---|---|
| `long` | `answers.search_reachable` | 0,000 | 0,333 | +0,333 |
| `long` | `answers.context_visible` | 0,000 | 0,250 | +0,250 |
| `long` | `answers.search_visible` | 0,000 | 0,000 | 0 |
| `long` | `search.hit_rate` | 0,083 | 0,333 | +0,250 |
| `long` | `context.hit_rate` | 0,083 | 0,500 | +0,417 |
| `long` | `repeated_rows` | 0 | 0 | 0 |
| `natural` | `search.hit_rate` / `context.hit_rate` | 0,4348 / 0,4348 | 0,4348 / 0,4348 | 0 |
| `natural` | `search.mean_tokens` / `context.mean_tokens` | 873,1 / 3.397,9 | 845,5 / 3.227,3 | −27,6 / −170,5 |
| `natural` | `repeated_rows` | 3 | 0 | −3 |
| `high_importance`, `low_importance`, `lexical`, `multi_ref` | todas | — | — | 0 |

| # | Condición | Resultado |
|---|---|---|
| U2 | `long`: `search_reachable` ≥ 0,75 y al menos 0,25 por encima de la base | **✗** 0,333: sube 0,333 sobre la base, pero no llega a 0,75 |
| U3 | `long`: `repeated_rows` = 0 | ✓ 0 (y `natural` baja de 3 a 0) |
| U4 | los otros cinco: ninguna métrica de agent-view baja más de 0,02 | ✓ los hit rates no cambian; los tokens quedan iguales o bajan |

**Por qué falla U2.** En los 4 casos donde el save aparece en `search_memory`, su respuesta es alcanzable con `read_memory`: el vínculo y la lectura hacen su parte. El límite está antes, en el ranking. Cuando el save aparece, lo hace en el puesto 7, 8 o 9 de 10, con un score pegado al último de la lista (por ejemplo 0,24 contra un tope de 0,32). En los otros 8 casos queda fuera de los 10. Las filas cortas del set, densas en las palabras del tema (`retry budget`, `timeout`, `cache`), le ganan a un pedazo de hasta 424 tokens donde la respuesta es una frase entre muchas. La cabecera hace que un pedazo posterior se encuentre por su clave (de 1 a 4 casos en búsqueda y de 1 a 6 en contexto), pero no lo sube lo suficiente. Se registra tal cual; el umbral no se mueve.

### Perfil dev

Con autorización para tocar `~/.clawg-dev`. Antes, `clawg backup --profile dev` (`memory-2026-10-15T01-05-23.413Z-manual.db`) y `clawg stop --profile dev`. Después se recompiló `target\debug\clawg.exe`, cuya imagen vieja quedó renombrada a `clawg.exe.old-2026-10-14` porque la tenía abierta el bridge del editor. Un bridge nuevo (`clawg mcp --profile dev --agent claude-code`, por JSON-RPC) levantó el daemon. Ese daemon aplicó V11 (`memory_piece`) y dejó `memory-2026-10-15T01-06-48.866Z-pre-migration.db`. El bridge lista diez herramientas, entre ellas `read_memory`.

1. `save_memory` de un texto de 12 párrafos (3.645 caracteres) que nombra `clawg-ledger` sólo en la primera oración. El daemon todavía cargaba el modelo y respondió `accepted; it will be stored once the embedding model finishes loading`. Quedó en tres pedazos, `#39`, `#40` y `#41`, vinculados en `memory_piece` como el save 1, y los pedazos 2 y 3 con la cabecera como contexto.
2. `search_memory` por el tema: un solo hit por save, `#39 [Memory/semantic_fact/t3 piece=1/3 score=0.48]` en una consulta y `#41 … piece=3/3 …` en otra. `list_memories` sigue mostrando las tres filas.
3. `read_memory` con `id: 40`: `#40 is piece 2 of 3 of one save: #39, #40, #41`, y el texto unido es idéntico al guardado.
4. `delete_memory` de `#40` y `read_memory` con `id: 41`: `#41 is piece 3 of 3 of one save: #39, (missing), #41`, con los dos tramos visibles separados por un salto de línea.
5. Con `--agent otro`, `read_memory` de `#39` y de `#41` responde `daemon returned 404: no Memory row with id 39` (y `… 41`), lo mismo que un id que no existe (`999999`).

Al final se borraron `#39` y `#41`. **✓**

**Para el editor:** el bridge `clawg-dev` que ya estaba abierto tiene el token del daemon anterior y no responde hasta reconectarlo (`/mcp`) o reiniciar Claude Code. Cuando se cierre, se puede borrar `target\debug\clawg.exe.old-2026-10-14`.
