# search-abstention — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El corte no muestra filas de score 0 (R1)

- [ ] Tests primero en `relevance.rs`: `a_hit_that_did_not_score_is_never_shown`, `hits_that_did_not_score_do_not_fill_the_minimum`, `an_exact_hit_is_shown_among_hits_that_did_not_score`.
- [ ] `shown_hits` descarta la fila inexacta de score 0 antes de contar el mínimo.
- Commit: `feat(cli): never show a memory hit that did not score`

## 2. El daemon y el puente dicen la frase en `search_memory` (R2, R5)

- [ ] Tests primero: `search_withholds_hits_that_did_not_score` (`tests/api.rs`), `a_search_answer_without_withheld_still_parses` (`client.rs`), `search_says_when_nothing_was_relevant` y la parte de búsqueda de `an_old_daemon_gets_no_abstention` (`tools.rs`), `search_memory_tells_nothing_relevant_from_nothing_stored` (`server.rs`), `the_abstention_sentence_parses_to_no_lines` (`parse.rs`).
- [ ] `SearchBody::withheld`; `DaemonClient::search` devuelve `Found { hits, withheld }`; `format_search` con la frase; el parser de agent-view.
- Commit: `feat(cli): say when search_memory found nothing relevant`

## 3. `recall_context` no rellena (R3)

- [ ] Tests primero en `prompt_builder.rs`: `recall_leaves_out_hits_that_did_not_score`, `a_hit_that_did_not_score_brings_no_conflict`.
- [ ] `build_memory_prompt` saca los hits de score 0 antes de `K_FINAL`; `MemoryPrompt { hits, withheld }`.
- Commit: `feat(core): stop filling the memory prompt with hits that did not score`

## 4. El daemon y el puente dicen la frase en `recall_context` (R4, R5)

- [ ] Tests primero: `context_says_how_many_hits_it_withheld` (`tests/api.rs`), `a_searched_context_without_counts_still_parses` (`client.rs`), `recall_says_when_nothing_was_relevant` y la parte de contexto de `an_old_daemon_gets_no_abstention` (`tools.rs`).
- [ ] `RetrievalState::Searched { hits, withheld }`; `Retrieval::Searched` del cliente con los campos por defecto; la frase en `format_recall`.
- Commit: `feat(cli): say when recall_context found nothing relevant`

## 5. Docs

- [ ] `.claude/CLAUDE.md` (API HTTP y herramientas), el mapa de `openspec/README.md` y este cambio.
- Commit: `docs: describe the abstention of search_memory and recall_context`

## 6. Verificación

- [ ] Suite y clippy en Linux sobre el último commit ([verify](verify.md)).
- [ ] El CI en verde en Linux y Windows.
- [ ] `rag-eval --real` y `rag-eval --real --agent-view` antes y después (necesita el modelo real y `docs/eval/golden.json`): U1 a U4 de la [propuesta](proposal.md#medición-con-el-modelo-real).
