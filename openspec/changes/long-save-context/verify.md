# long-save-context — Verificación

**Código:** rama `claude/project-thread-0dd4jr`, un commit por tarea, sobre `main` en `9c3ff19`.
**Modo:** Strict TDD (`openspec/config.yaml`: `strict_tdd: true`). Los tests de cada tarea se escribieron antes que su código y fallaron por no compilar (tipos, funciones o la tabla que todavía no existían) o por la aserción.
**Estado:** R1 a R8 y la parte de R9 que se prueba sin modelo pasan en Linux, commit por commit. Faltan la suite en Windows, las mediciones U1 a U4 y la prueba con el perfil dev, que necesitan la máquina de Marcos ([pendiente](#pendiente-máquina-de-marcos)).

## Suite

`cargo test --workspace --no-fail-fast` en Linux, cada commit en un worktree limpio:

| Commit | Pasan | Fallan |
|---|---|---|
| `9c3ff19` (base, `main`) | 1674 | 10 |
| `37813a5` docs(openspec): propose linking the pieces of a long save | 1674 | 10 |
| `db0c6a1` feat(eval): measure whether the answer inside a long memory reaches the agent | COUNT_1 | 10 |
| `240dc58` feat(core): embed later pieces of a long save with its opening and link them | COUNT_2 | 10 |
| `3d08a5f` feat(core): show one hit per save and say which piece it is | COUNT_3 | 10 |
| `92aea43` feat(mcp): let the agent read a long save as one text | COUNT_4 | 10 |
| `fab517e` docs: list the read_memory tool | COUNT_5 | 10 |

- **+NEW tests.** El conteo nunca baja: los fixtures que dependían de las ventanas de 480 tokens se ajustaron en el mismo commit que baja el tope a 424.
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

## Pendiente (máquina de Marcos)

1. **Suite en Windows:** `cargo test --workspace --target-dir <otro-dir>`. Espero la base de Windows más los tests nuevos (1.675 + NEW = WIN_TOTAL si la base sigue en 1.675) y 0 fallas.
2. **U1, el ancla:** `rag-eval --real --diff <último resultado de main>` y `rag-eval --real --agent-view`, con `docs/eval/golden.json`. Diff vacío en retrieval y similarity, y el bloque `agent_view` igual campo por campo: el ancla no tiene filas de más de un pedazo.
3. **U2, U3 y U4:** `golden-mine synth --base docs/eval/golden.json --out <set> --seed 7` y `rag-eval --real --agent-view --golden <set>` dos veces: con el binario de `db0c6a1` (la base: entiende `long` pero guarda y busca como `main`) y con el de la punta. Umbrales, fijados en la propuesta y sin mover:

   | # | Condición |
   |---|---|
   | U2 | `long`: `by_subset.long.answers.search_reachable` ≥ 0,75 y al menos 0,25 por encima de la base |
   | U3 | `long`: `by_subset.long.repeated_rows` = 0 (ningún `search_memory` muestra dos pedazos del mismo save) |
   | U4 | los otros cinco subconjuntos: ninguna métrica de agent-view baja más de 0,02 |

   Si una condición no se cumple, se registra tal cual.
4. **Perfil dev con Claude Code:** guardar un texto largo (varios párrafos que nombran el tema sólo al principio), buscar por el tema y ver un solo hit con `piece=K/N`, leerlo con `read_memory`, borrar un pedazo con `delete_memory` y volver a leerlo (aparece `(missing)`), y leerlo con otro `--agent` (error, como un id que no existe). Abrir el perfil dev con este binario aplica V11 y deja un snapshot `pre-migration` en `~/.clawg-dev/backups`.

Cuando estén, se archiva con los pasos de `tasks.md`.
