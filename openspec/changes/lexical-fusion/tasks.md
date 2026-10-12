# lexical-fusion — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. El suite queda sin fallas nuevas en cada commit.

## 1. El score acepta un bono léxico (R2)

- [ ] Tests `a_lexical_match_adds_the_bonus_after_the_floor` y `without_a_match_the_score_is_unchanged` en `score.rs`.
- [ ] `RawHit.lexical_match: bool` y `ScoringConfig.lexical_bonus` (1,0 en `DEFAULT`). `score()` suma el bono a `cos'` sólo si hay match. Todos los constructores de `RawHit` ponen `false`.
- Commit: `feat(core): let an exact lexical match add a bonus to the score`

## 2. La búsqueda marca los identificadores exactos (R1, R3 a R6)

- [ ] Tests `identifier_tokens_*` (R1).
- [ ] Tests de búsqueda: `an_exact_identifier_outranks_its_closer_siblings`, `an_identifier_past_the_sixteenth_token_still_reaches_its_row`, `no_bonus_without_the_lexical_channel_or_query_text`, `an_identifier_shared_by_more_rows_than_the_budget_gives_no_bonus`, `a_query_without_identifiers_scores_exactly_as_without_the_bonus`, `rows_the_caller_cannot_see_neither_match_nor_count_against_the_budget` y `a_failing_identifier_query_leaves_the_search_unboosted`.
- [ ] `identifier_tokens` puro; `identifier_matches` (FTS5 `AND`, `LIMIT presupuesto + 1`, mismos predicados de scope, fail-soft) antes del `try_join!`; `knn_one` y `score_lexical_candidates` marcan `lexical_match`; los matches se suman a los candidatos léxicos.
- Commit: `feat(core): rank rows that carry the query's exact identifier first`

## Verify

- [ ] `cargo test --workspace`: sin fallas nuevas contra la base.
- [ ] `rag-eval --real --diff` sobre el set de la semilla 7 y sobre el ancla, y `--agent-view --diff`, contra el baseline: umbrales de `proposal.md`. Necesita el modelo y `docs/eval/`, que están en la máquina del usuario.
