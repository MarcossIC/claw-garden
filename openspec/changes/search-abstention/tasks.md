# search-abstention — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El corte no muestra filas de score 0 (R1)

- [x] Tests primero en `relevance.rs`: `a_hit_that_did_not_score_is_never_shown`, `hits_that_did_not_score_do_not_fill_the_minimum`, `an_exact_hit_is_shown_among_hits_that_did_not_score`.
- [x] `shown_hits` descarta la fila inexacta de score 0 antes de contar el mínimo. `rows_of_the_previous_version_are_shared_untouched` (`server.rs`) sembraba un vector sin normalizar, que puntuaba 0: ahora lo guarda de largo unidad, como todo camino de inserción.
- Commit: `feat(cli): never show a memory hit that did not score`

## 2. `recall_context` no rellena (R3)

- [x] Tests primero en `prompt_builder.rs`: `recall_leaves_out_hits_that_did_not_score` (reemplaza a `recall_does_not_count_zero_score_hits_as_accesses`), `a_hit_that_did_not_score_brings_no_conflict`.
- [x] `build_memory_prompt` saca los hits de score 0 antes de `K_FINAL`; `Retrieval::Searched { hits, withheld }`.
- [x] Los fixtures que contaban con el relleno: `three_hits_split_into_recent_and_longterm_blocks` y `e2e_full_4_block_output` guardan el vector de la consulta; `write_fresh_and_stale_then_read_back` (`worker.rs`) pone los dos mensajes sobre un mismo eje; `recall_context_passes_its_budget`, `a_hit_cut_by_the_budget_is_readable_whole` y `rows_and_workspace_of_a_client_name_survive_pinning_it` (`server.rs`) buscan el texto guardado o un identificador que las filas comparten.
- Commit: `feat(core): stop filling the memory prompt with hits that did not score`

## 3. El daemon y el puente dicen la frase en `search_memory` (R2, R5, R6)

- [x] Tests primero: `search_withholds_hits_that_did_not_score` (`tests/api.rs`), `a_search_answer_without_withheld_still_parses` (`client.rs`), `search_says_when_nothing_was_relevant` y `nothing_stored_is_not_called_irrelevant` (`tools.rs`), `search_memory_tells_nothing_relevant_from_nothing_stored` (`server.rs`), `the_abstention_sentence_parses_to_no_lines` (`parse.rs`).
- [x] `SearchBody::withheld`; `DaemonClient::search` devuelve `Found { hits, withheld }`; `format_search` con la frase; el parser de agent-view.
- Commit: `feat(cli): say when search_memory found nothing relevant`

## 4. El daemon y el puente dicen la frase en `recall_context` (R4, R5)

- [x] Tests primero: `context_says_how_many_hits_it_withheld` (`tests/api.rs`), `a_searched_context_without_counts_still_parses` (`client.rs`), `recall_says_when_nothing_was_relevant` y `an_old_daemon_gets_no_invented_notice` con `searched` sin conteos (`tools.rs`).
- [x] `RetrievalState::Searched { hits, withheld }`; `Retrieval::Searched` del cliente con los campos por defecto; la frase en `format_recall`.
- Commit: `feat(cli): say when recall_context found nothing relevant`

## 5. Docs

- [x] `.claude/CLAUDE.md` (API HTTP y herramientas), el mapa de `openspec/README.md` y este cambio.
- Commit: `docs: describe the abstention of search_memory and recall_context`

## 6. Verificación

- [x] Suite y clippy en Linux sobre el último commit ([verify](verify.md)).
- [ ] El CI en verde en Linux y Windows.
- [ ] `rag-eval --real` y `rag-eval --real --agent-view` antes y después (necesita el modelo real y `docs/eval/golden.json`): U1 a U4 de la [propuesta](proposal.md#medición-con-el-modelo-real).
