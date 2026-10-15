# long-save-context — Tareas

Cada tarea es un commit atómico (conventional commits) que compila y deja `cargo test --workspace` sin fallas nuevas. Dentro de cada una, primero el test que falla (RED), después el código (GREEN). En Windows: `cargo test --workspace --target-dir <otro-dir>`.

Los mensajes de commit son sugeridos. No mencionan planes, fases ni specs.

## 0. Artefactos

- [x] `proposal.md`, `spec.md`, `design.md`, `tasks.md`; revisión de la exploración contra `main`; `state.yaml`.
- [x] Commit: `docs(openspec): propose linking the pieces of a long save` (`37813a5`)

## 1. Medición — R5 (parser), R9

- [x] RED: `there_are_twelve_long_cases_at_any_size`, `a_long_row_names_its_key_only_in_its_first_sentence`, `the_answer_appears_once_after_the_first_third`, `a_long_row_splits_into_at_least_three_pieces`, `adding_long_cases_leaves_the_rest_of_the_set_unchanged` (`synth/mod.rs`); `a_long_case_writes_its_subset_and_answer` (`synth/golden.rs`); `an_answer_and_the_long_subset_parse` (`rag-eval/golden.rs`); `the_answer_counts_only_when_its_text_is_visible`, `a_read_save_makes_a_hidden_answer_reachable`, `a_line_repeating_an_earlier_lines_row_is_counted`, `build_results_splits_active_cases_by_subset_with_their_answers` (`agent_view/mod.rs`); `a_case_that_names_its_answer_reports_whether_the_answer_reached_the_agent` (`runner.rs`); `a_hit_line_with_a_piece_yields_its_id_score_and_content`, `a_context_hit_with_a_piece_is_one_segment` (`parse.rs`).
- [x] GREEN: `Subset::Long` y `answer` en los dos esquemas; `long_saves` al final de `generate`; `SearchLine.id`; los parsers aceptan `piece=K/N`; agent-view cuenta respuesta visible, alcanzable y pedazos repetidos, y separa las métricas por subconjunto (`by_subset`); `DaemonClient::read_save`.
- [x] Commit: `feat(eval): measure whether the answer inside a long memory reaches the agent` (`db0c6a1`)

## 2. Pedazos con contexto y su vínculo — R1, R2, R3, R7, R8

- [x] RED: `a_text_that_fits_one_piece_is_split_as_before`, `a_long_text_is_split_below_the_cap_to_leave_room_for_the_head`, `the_head_is_the_first_tokens_of_the_text` (`chat.rs`); `a_piece_without_context_is_embedded_as_stored`, `a_piece_with_context_is_embedded_after_the_head` (`chunker/mod.rs`); `a_single_piece_save_embeds_its_text_and_leaves_no_link`, `later_pieces_are_embedded_with_the_head_and_stored_without_it`, `a_long_save_links_its_pieces_in_order`, `an_absorbed_piece_is_linked_without_context`, `a_save_cut_short_leaves_no_link`, `the_same_long_save_again_links_the_rows_it_left` (`worker.rs`); `link_pieces_numbers_saves_in_order`, `linking_the_same_rows_again_answers_the_save_they_already_form`, `rows_from_v10_survive_the_piece_table` (`pieces.rs`); `reembed_rebuilds_a_pieces_context` (`maintenance.rs`).
- [x] GREEN: `V11__memory_piece.sql`; `link_pieces`; `PIECE_MAX_TOKENS`, `SAVE_HEAD_TOKENS` y su assert; `ChatChunker::split_save`; `embedding_text`; el worker embebe con contexto y vincula al final; `select_reembed_batch` lee el contexto.
- [x] Commit: `feat(core): embed later pieces of a long save with its opening and link them` (`240dc58`). Junta las tareas 2 y 3 del plan: la tabla sola dejaba `link_pieces` sin llamador.

## 3. Un hit por save — R4, R5 (formato)

- [x] RED: `only_the_best_piece_of_a_save_survives`, `rows_without_links_keep_their_place`, `freed_slots_go_to_other_memories`, `a_link_read_failure_leaves_hits_uncollapsed` (`search.rs`); `format_hit_line_names_the_piece_of_a_long_save` (`prompt_builder.rs`); `format_hits_names_the_piece_of_a_long_save` (`tools.rs`).
- [x] GREEN: `PiecePosition` en `RankedHit` y en `Hit`; `MemoryStore::save_links`; `collapse_saves`; `piece=K/N` en los dos formatos.
- [x] Commit: `feat(core): show one hit per save and say which piece it is` (`3d08a5f`)

## 4. El save entero por HTTP y MCP — R6, R8

- [x] RED: `join_pieces_drops_the_overlap_once`, `join_pieces_keeps_both_sides_when_nothing_overlaps`, `the_pieces_of_a_split_save_join_back_into_its_text`, `read_save_lists_missing_pieces_and_skips_other_agents` y el paso de `read_memory` en `rows_from_v10_survive_the_piece_table` (`pieces.rs`); `get_save_returns_the_whole_text`, `get_save_of_another_agent_is_404` (`tests/api.rs`); `read_memory_returns_the_whole_save`, `an_agent_cannot_read_another_agents_save`, `a_daemon_without_the_save_route_is_named_as_such`, `search_memory_explains_the_piece_of_a_long_save`, `save_memory_asks_for_one_fact_per_call` (`server.rs`); `format_save_names_the_piece_and_the_missing_ones` (`tools.rs`); conteos de herramientas en `server.rs` y `handshake.rs`; la respuesta de un save largo en `save_outcome_text_covers_every_daemon_answer`.
- [x] GREEN: `read_save_core` y `join_pieces`; ruta `GET /v1/memories/{kind}/{id}/save`; herramienta `read_memory`; las descripciones de `save_memory` y `search_memory`.
- [x] Commit: `feat(mcp): let the agent read a long save as one text` (`92aea43`)

## 5. Docs

- [x] `.claude/CLAUDE.md` (la herramienta, la ruta y el vínculo) y la lista de herramientas del `README.md`.
- [x] Commit: `docs: list the read_memory tool` (`fab517e`)

## Verify

- [x] `cargo test --workspace` en Linux en cada commit, contra la base (1.674 pasan, 10 fallas de entorno). Detalle en [`verify.md`](verify.md).
- [x] `cargo test --workspace --target-dir <otro-dir>` en Windows: 1.737 pasan, 0 fallan.
- [x] `rag-eval --real --diff` y `--agent-view` sobre el ancla: U1 ✓.
- [x] `golden-mine synth --seed 7` y `rag-eval --real --agent-view` con el binario del commit 1 y con el de la punta: U3 ✓, U4 ✓, **U2 ✗** (0,333 contra 0,75).
- [x] Prueba manual con el perfil dev: guardar un texto largo, buscar por su tema, leerlo con `read_memory`, borrar un pedazo y volver a leerlo, intentar leerlo desde otro agente.

## Archive

- [x] `openspec/specs/pieces/spec.md` con R1–R8, las rutas y líneas del código y la línea **Origen**; R9 a `specs/eval`.
- [x] `specs/mcp` (diez herramientas), `specs/disputes` R8 (los hermanos quedan vinculados), `specs/visibility` R3 y R4 (`read_memory` y su ruta).
- [x] `git mv openspec/changes/long-save-context openspec/changes/archive/2026-10-14-long-save-context`; `state.yaml` archivado; fila 15 del `README.md` de openspec; base de tests de `.claude/CLAUDE.md`.
