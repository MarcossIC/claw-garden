# context-density — Tareas

Cada tarea es un commit atómico (conventional commits) que compila y deja `cargo test --workspace` sin fallas nuevas. Dentro de cada una, primero el test que falla (RED), después el código (GREEN). En Windows: `cargo test --workspace --target-dir <otro-dir>`.

Los mensajes de commit son sugeridos. No mencionan planes, fases ni specs.

## 1. Hits en una línea con su id — R1, R8

El formato de los hits y el parser del eval van en el mismo commit: el test de punta a punta del agent-view (`runner.rs:317`) arma un prompt real y lo lee con el parser, así que separarlos deja un commit en rojo.

- [x] RED: `hit_line_is_one_header_with_id_and_text` en `prompt_builder.rs`; `a_multi_line_hit_runs_until_the_next_header` y `conflict_text_yields_no_segments` en `parse.rs`; `read_context_finds_rows_in_hit_lines_and_workspace_bullets` en `agent_view/mod.rs`, en lugar de `read_context_finds_rows_in_hit_blocks_and_workspace_bullets`.
- [x] GREEN: `format_hit_line` emite `[YYYY-MM-DD #id tipo] texto` (con `kind/` delante del tipo si no es `memory`). `context_segments` segmenta por sección: hits sólo en RECENT y LONG-TERM, bullets sólo en WORKSPACE. El encabezado de hit reemplaza a `starts_with_date_header`. `format_hit_line_emits_canonical_format` pasa a `format_hit_line_drops_the_old_labels`, y los fixtures de `parse.rs` y `agent_view/mod.rs` pasan al formato nuevo. Se suman `a_hit_of_another_kind_is_one_segment`, `a_hit_may_open_with_a_dash` y `workspace_markers_end_a_bullet`.
- [x] Commit: `feat(core): print each recall hit on one line with its id` (`4ffc8d0`)

## 2. Orden y tope de los conflictos — R5, R6

- [x] RED: `conflict_pairs_rank_by_best_hit_position_workspace_only_last`, `conflict_pair_order_ignores_input_order` y `conflict_block_keeps_five_pairs_and_counts_the_rest` en `prompt_builder.rs`, sin base.
- [x] GREEN: una función pura ordena los pares por `(rank, new_id desc, old_id asc)` con las posiciones de los hits impresos, y se queda con 5. El bloque cierra con `(+N conflictos más sin mostrar)` cuando sobran.
- [x] Commit: `feat(core): cap the conflicts block and order it by what the prompt shows first` (`f92142d`)

## 3. Conflictos por referencia — R2, R3, R4

- [x] RED: `conflict_block_cites_shown_hits_by_id_and_prints_text_only_for_the_rest`, `a_new_row_disputing_several_old_rows_prints_its_text_once` y `conflict_block_has_one_note` en `prompt_builder.rs`.
- [x] GREEN: el bloque agrupa por fila nueva, cita por `#id` el lado que es hit y lleva el texto del resto; la NOTE va una vez. `ConflictPair` y la consulta dejan de leer `importance` y `tier`. `format_conflict_pair` desaparece: `format_conflict_pair_renders_both_dates_in_sane_range` pasa a `conflict_block_renders_both_dates_in_sane_range`, y `the_conflict_note_names_the_tool` y `e2e_full_4_block_output` pasan al formato nuevo (el e2e comprueba que la fila mostrada aparece una sola vez).
- [x] Commit: `feat(core): cite conflict rows the prompt already shows by id` (`88af38f`)

## 4. Workspace sin marcadores — R7

- [x] RED: `workspace_in_prompt_has_no_markers_or_empty_headings`, `workspace_user_section_content_is_kept_without_markers` y `workspace_file_bytes_are_unchanged_by_rendering_the_prompt` en `prompt_builder.rs`.
- [x] GREEN: la función pura que reemplaza a `strip_empty_workspace_markers` saca los marcadores, la sección USER vacía y los encabezados AUTO sin bullets. El comentario del paso 1 describe lo que hace.
- [x] Commit: `feat(core): hand the model the workspace without its markers or empty headings` (`2e970f1`)

## 5. Ids de `recall_context` en `delete_memory` y `update_memory` — R9

- [x] RED: `memory_id_docs_name_recall_context_and_the_default_kind` en `server.rs`.
- [x] GREEN: la doc del `id` de `DeleteMemoryArgs` y `UpdateMemoryArgs`, y la descripción de `delete_memory`.
- [x] Commit: `docs(mcp): say that recall_context ids work in delete_memory and update_memory` (`41a0873`)

## Verificación

- [x] `cargo test --workspace --no-fail-fast` en Linux: conteo contra la base, y cada commit en un worktree limpio. Ver `verify.md`.
- [ ] `cargo test --workspace --target-dir <otro-dir>` en Windows, contra la base de 1.655.
- [ ] `rag-eval --real --agent-view --diff` contra `2026-10-13-real-lexical-fusion-main-agent-view.json`, con los umbrales de R11 (en Windows: el golden set y los resultados no están en el repo).
- [ ] Prueba en el perfil dev con Claude Code: `recall_context` muestra los hits con id, el bloque de conflictos cita por `#id`, y `resolve_conflict` y `delete_memory` aceptan esos ids.

## Archivo

- [ ] Sumar R1 a R11 a `openspec/specs/retrieval/spec.md`, mover la carpeta a `changes/archive/AAAA-MM-DD-context-density/` y marcar la fila 14 del README.
